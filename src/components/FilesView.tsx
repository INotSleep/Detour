import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDown, ArrowUp, Check, ChevronDown, ChevronUp, Copy, Download, Ellipsis, File, FolderOpen, FolderPlus, FolderUp, Folder, House, KeyRound, Link2, Loader2, Pencil, RotateCw, ShieldAlert, ShieldQuestion, SquareTerminal, Trash2, Upload, X,
} from "lucide-react";
import { api, errorText, isCoreError, type FileEntry, type LocalFile, type SecretKind } from "../lib/api";
import { copyText, plural, uid, useStore } from "../lib/store";
import type { Host } from "../lib/types";
import type { SessionStatus } from "../lib/fakeShell";
import { Button, Checkbox, ConfirmDialog, Field, IconButton, Menu, Modal, PasswordInput } from "./ui";

interface Props {
  host: Host;
  active: boolean;
  onStatus: (s: SessionStatus) => void;
  onClose: () => void;
}

/** Where opening the browser stands. */
type Phase =
  | { kind: "connecting" }
  | { kind: "secret"; secret: SecretKind; trust?: string; error?: string }
  | { kind: "trust"; fingerprint: string }
  | { kind: "changed"; expected: string; actual: string }
  | { kind: "failed"; error: string }
  | { kind: "ready"; id: string; home: string };

interface Transfer {
  id: string;
  up: boolean;
  name: string;
  /** The remote folder it goes to or comes from. */
  dir: string;
  done: number;
  total: number;
  started: number;
  state: "active" | "done" | "error";
  error?: string;
}

type SortBy = "name" | "size" | "modified";

/** `dir` joined with `name`, in the server's syntax. */
const join = (dir: string, name: string) => (dir.endsWith("/") ? dir + name : `${dir}/${name}`);

function parentOf(path: string) {
  const i = path.replace(/\/+$/, "").lastIndexOf("/");
  return i <= 0 ? "/" : path.slice(0, i);
}

/** "812 Б", "8,4 МБ", "177 МБ", "1,2 ГБ". */
export function formatBytes(n: number) {
  const units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = i === 0 || v >= 100 ? 0 : 1;
  return `${v.toLocaleString("ru-RU", { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${units[i]}`;
}

/** "14:32" today, "21 окт 14:32" this year, "21 окт 2024" before. */
function formatModified(ms: number | null) {
  if (!ms) return "";
  const d = new Date(ms);
  const now = new Date();
  const time = d.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return time;
  const date = d.toLocaleDateString("ru-RU", { day: "numeric", month: "short", ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }) }).replace(".", "").replace(" г", "");
  return d.getFullYear() === now.getFullYear() ? `${date} ${time}` : date;
}

/** `0o754` -> "rwxr-xr--". */
function formatMode(mode: number | null) {
  if (mode == null) return "";
  return [6, 3, 0].map((s) => ((mode >> s) & 4 ? "r" : "-") + ((mode >> s) & 2 ? "w" : "-") + ((mode >> s) & 1 ? "x" : "-")).join("");
}

/** Folders first, then by the chosen column. */
function sortEntries(list: FileEntry[], by: SortBy, desc: boolean) {
  const key = (e: FileEntry) => (by === "size" ? e.size : by === "modified" ? (e.modified ?? 0) : 0);
  return [...list].sort((a, b) => {
    const dirs = Number(b.kind === "dir") - Number(a.kind === "dir");
    if (dirs) return dirs;
    const k = by === "name" ? a.name.localeCompare(b.name, "ru", { numeric: true, sensitivity: "base" }) : key(a) - key(b) || a.name.localeCompare(b.name, "ru", { numeric: true });
    return desc ? -k : k;
  });
}

/** A shell command that goes to `path`. */
const cdTo = (path: string) => `cd '${path.replace(/'/g, `'\\''`)}'`;

export function FilesView({ host, active, onStatus, onClose }: Props) {
  const { update, toast, applyTeamView, connect: openTerminal } = useStore();
  const [phase, setPhase] = useState<Phase>({ kind: "connecting" });
  const [path, setPath] = useState("");
  const [entries, setEntries] = useState<FileEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  // Selected names; the cursor is the one keys move from.
  const [sel, setSel] = useState<string[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const anchor = useRef<string | null>(null);
  const [sort, setSort] = useState<{ by: SortBy; desc: boolean }>({ by: "name", desc: false });
  const [renaming, setRenaming] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [removing, setRemoving] = useState<FileEntry[] | null>(null);
  const [replacing, setReplacing] = useState<{ items: LocalFile[]; existing: string[] } | null>(null);
  const [chmodding, setChmodding] = useState<FileEntry | null>(null);
  const [transfers, setTransfers] = useState<Transfer[]>([]);
  const [dragOver, setDragOver] = useState(false);
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  // Async results must see the latest folder, not the one of the render that started them.
  const pathRef = useRef(path);
  pathRef.current = path;
  const conn = phase.kind === "ready" ? phase : null;
  const cb = useRef(onStatus);
  cb.current = onStatus;

  const sorted = useMemo(() => (entries ? sortEntries(entries, sort.by, sort.desc) : null), [entries, sort]);
  const picked = sorted?.filter((e) => sel.includes(e.name)) ?? [];

  useEffect(() => {
    cb.current(phase.kind === "ready" ? "connected" : phase.kind === "failed" ? "failed" : "connecting");
  }, [phase.kind]);

  /* ---------- connecting ---------- */

  const connect = useCallback(
    async (answer: { secret?: string; secretKind?: SecretKind; remember?: boolean; trust?: string } = {}) => {
      setPhase({ kind: "connecting" });
      try {
        const opened = await api.filesOpen(host, answer.secret ?? null, answer.trust ?? null);
        setPhase({ kind: "ready", ...opened });
        update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, lastConnected: Date.now() } : h)) }));
        if (answer.secret && answer.remember && answer.secretKind) {
          try {
            await api.secretSave(host, answer.secretKind, answer.secret);
            // In a team the secret may have gone to your own login rather than the shared one: ask the core.
            if (host.access !== undefined && api.real) applyTeamView(await api.teamSync(host.vaultId));
            else if (answer.secretKind === "password") update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, hasPassword: true } : h)) }));
          } catch (e) {
            toast({ text: errorText(e), tone: "error" });
          }
        }
        void open(opened.id, opened.home);
      } catch (e) {
        if (isCoreError(e) && e.kind === "needSecret") return setPhase({ kind: "secret", secret: e.secret, trust: answer.trust });
        if (isCoreError(e) && e.kind === "authFailed" && answer.secret) return setPhase({ kind: "secret", secret: e.secret ?? "password", trust: answer.trust, error: errorText(e) });
        if (isCoreError(e) && e.kind === "hostKeyUnknown") return setPhase({ kind: "trust", fingerprint: e.fingerprint });
        if (isCoreError(e) && e.kind === "hostKeyChanged") return setPhase({ kind: "changed", expected: e.expected, actual: e.actual });
        setPhase({ kind: "failed", error: errorText(e) });
      }
    },
    // `open` only uses refs and setters.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [host, update, toast, applyTeamView],
  );

  useEffect(() => {
    void connect();
    // A browser is bound to the host it was opened with.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Close the connection with the tab.
  const idRef = useRef<string | null>(null);
  idRef.current = conn?.id ?? null;
  useEffect(() => () => void (idRef.current && api.filesClose(idRef.current)), []);

  /* ---------- selection ---------- */

  const selectOnly = (name: string | null) => {
    setSel(name ? [name] : []);
    setCursor(name);
    anchor.current = name;
  };

  /** Selects from the anchor to `name`, as Shift does. */
  const selectRange = (name: string) => {
    if (!sorted) return;
    const names = sorted.map((e) => e.name);
    const a = names.indexOf(anchor.current ?? name);
    const b = names.indexOf(name);
    setSel(names.slice(Math.min(a, b), Math.max(a, b) + 1));
    setCursor(name);
  };

  const clickRow = (ev: React.MouseEvent, name: string) => {
    if (ev.shiftKey && anchor.current) return selectRange(name);
    if (ev.ctrlKey || ev.metaKey) {
      setSel((s) => (s.includes(name) ? s.filter((n) => n !== name) : [...s, name]));
      setCursor(name);
      anchor.current = name;
      return;
    }
    selectOnly(name);
  };

  /* ---------- browsing ---------- */

  /** Shows a folder; `select` picks an entry in it. */
  const open = async (id: string, dir: string, select?: string | null) => {
    setLoading(true);
    try {
      const list = await api.filesList(id, dir);
      setPath(dir);
      setEntries(list);
      setListError(null);
      selectOnly(select !== undefined ? select : sortEntries(list, sort.by, sort.desc)[0]?.name ?? null);
      setRenaming(null);
      setCreating(false);
    } catch (e) {
      if (pathRef.current) toast({ text: errorText(e), tone: "error" });
      else setListError(errorText(e));
    } finally {
      setLoading(false);
    }
  };

  const go = (dir: string, select?: string | null) => conn && open(conn.id, dir, select);
  const refresh = (select?: string | null) => go(pathRef.current, select === undefined ? cursor : select);

  const up = async () => {
    if (!conn || path === "/") return;
    const from = path.replace(/\/+$/, "").split("/").pop() ?? null;
    let target = parentOf(path);
    // A path that came through a link: ask the server where its parent really is.
    try {
      target = await api.filesResolve(conn.id, join(path, ".."));
    } catch {
      /* the plain parent will do */
    }
    void go(target, from);
  };

  const enter = async (e: FileEntry) => {
    if (!conn) return;
    if (e.kind === "dir") return go(join(path, e.name));
    if (e.kind === "link") {
      // A link to a folder opens it; a link to a file downloads.
      try {
        await api.filesList(conn.id, join(path, e.name));
        return go(join(path, e.name));
      } catch {
        /* not a folder */
      }
    }
    void download([e]);
  };

  const toggleSort = (by: SortBy) => setSort((s) => (s.by === by ? { by, desc: !s.desc } : { by, desc: by !== "name" }));

  /* ---------- transfers ---------- */

  const patchTransfer = (id: string, p: Partial<Transfer>) => setTransfers((ts) => ts.map((t) => (t.id === id ? { ...t, ...p } : t)));
  const dropTransfer = (id: string) => setTransfers((ts) => ts.filter((t) => t.id !== id));

  const run = async (t: Transfer, work: (onProgress: (done: number, total: number) => void) => Promise<unknown>) => {
    setTransfers((ts) => [...ts, t]);
    try {
      await work((done, total) => patchTransfer(t.id, { done, total }));
      patchTransfer(t.id, { state: "done" });
      setTimeout(() => dropTransfer(t.id), 4000);
      return true;
    } catch (e) {
      if (isCoreError(e) && e.kind === "cancelled") dropTransfer(t.id);
      else patchTransfer(t.id, { state: "error", error: errorText(e) });
      return false;
    }
  };

  const transfer = (up: boolean, name: string, total = 0): Transfer => ({ id: uid("t"), up, name, dir: path, done: 0, total, started: Date.now(), state: "active" });

  /** One file: saved where the user says. A folder or several things: into a folder the user picks. */
  const download = async (items: FileEntry[]) => {
    if (!conn || items.length === 0) return;
    const [only] = items;
    if (items.length === 1 && only && only.kind !== "dir") {
      const local = await api.pickDownloadTarget(only.name);
      if (!local) return;
      const t = transfer(false, only.name, only.size);
      await run(t, (p) => api.filesDownload(conn.id, join(t.dir, only.name), local, t.id, p));
      return;
    }
    const into = await api.pickDownloadFolder();
    if (!into) return;
    for (const e of items) {
      const t = transfer(false, e.name, e.kind === "dir" ? 0 : e.size);
      void run(t, (p) => api.filesDownloadInto(conn.id, join(t.dir, e.name), into, t.id, p));
    }
  };

  const upload = (items: LocalFile[], force = false) => {
    if (!conn || items.length === 0) return;
    const existing = items.filter((f) => entries?.some((e) => e.name === f.name)).map((f) => f.name);
    if (existing.length && !force) return setReplacing({ items, existing });
    const dir = path;
    for (const f of items) {
      const t = transfer(true, f.name);
      void run(t, (p) => api.filesUpload(conn.id, f, join(dir, f.name), existing.includes(f.name), t.id, p)).then((ok) => void (ok && pathRef.current === dir && refresh(f.name)));
    }
  };

  const pickUploads = async () => upload(await api.pickUploads());
  const pickFolders = async () => upload(await api.pickUploadFolders());

  // Files and folders dragged in from the system.
  const uploadRef = useRef(upload);
  uploadRef.current = upload;
  const ready = !!conn && entries !== null;
  useEffect(() => {
    if (!active || !ready) return;
    let stop = () => {};
    let gone = false;
    void api
      .onFileDrop((e) => {
        if (e.type === "drop") {
          setDragOver(false);
          uploadRef.current(e.files);
        } else setDragOver(e.type !== "leave");
      })
      .then((un) => (gone ? un() : (stop = un)));
    return () => {
      gone = true;
      stop();
      setDragOver(false);
    };
  }, [active, ready]);

  /* ---------- changes ---------- */

  // Keys go to the list again once an inline field or a dialog is gone.
  const focusList = () => requestAnimationFrame(() => listRef.current?.focus());

  const mkdir = async (name: string) => {
    setCreating(false);
    focusList();
    if (!conn || !name.trim()) return;
    try {
      await api.filesMkdir(conn.id, join(path, name.trim()));
      void refresh(name.trim());
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const rename = async (from: string, to: string) => {
    setRenaming(null);
    focusList();
    if (!conn || !to.trim() || to.trim() === from) return;
    try {
      await api.filesRename(conn.id, join(path, from), join(path, to.trim()));
      void refresh(to.trim());
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const remove = async (items: FileEntry[]) => {
    if (!conn || !sorted) return;
    const idx = sorted.findIndex((x) => x.name === items[0]?.name);
    let gone = 0;
    for (const e of items) {
      try {
        await api.filesRemove(conn.id, join(path, e.name));
        gone++;
      } catch (err) {
        toast({ text: `${e.name}: ${errorText(err)}`, tone: "error" });
      }
    }
    if (gone) toast({ text: items.length === 1 ? `«${items[0]!.name}» удалён` : `Удалено: ${gone}`, tone: "success" });
    const left = sorted.filter((x) => !items.some((i) => i.name === x.name));
    void refresh(left[Math.min(Math.max(idx, 0), left.length - 1)]?.name ?? null);
  };

  const chmod = async (e: FileEntry, mode: number) => {
    if (!conn) return;
    try {
      await api.filesChmod(conn.id, join(path, e.name), mode);
      toast({ text: `Права «${e.name}»: ${formatMode(mode)}`, tone: "success" });
      void refresh(e.name);
    } catch (err) {
      toast({ text: errorText(err), tone: "error" });
    }
  };

  const copyPath = async (e: FileEntry) => {
    const p = join(path, e.name);
    toast((await copyText(p)) ? { text: `Скопировано: ${p}`, tone: "success" } : { text: "Не удалось скопировать", tone: "error" });
  };

  /* ---------- keyboard ---------- */

  useEffect(() => {
    if (active && ready) requestAnimationFrame(() => listRef.current?.focus());
  }, [active, ready]);

  const onListKey = (ev: React.KeyboardEvent) => {
    if (!sorted || renaming || creating || removing || replacing || chmodding || (ev.target as HTMLElement).tagName === "INPUT") return;
    const i = sorted.findIndex((e) => e.name === cursor);
    const cur = sorted[i];
    const move = (to: number) => {
      ev.preventDefault();
      const e = sorted[Math.max(0, Math.min(sorted.length - 1, to))];
      if (!e) return;
      if (ev.shiftKey) selectRange(e.name);
      else selectOnly(e.name);
      listRef.current?.querySelector(`[data-name="${CSS.escape(e.name)}"]`)?.scrollIntoView({ block: "nearest" });
    };
    const ctrl = ev.ctrlKey || ev.metaKey;
    if (ev.key === "ArrowDown") move(i + 1);
    else if (ev.key === "ArrowUp") move(i - 1);
    else if (ev.key === "Home") move(0);
    else if (ev.key === "End") move(sorted.length - 1);
    else if (ev.key === "Enter" && cur) void (picked.length > 1 ? download(picked) : enter(cur));
    else if (ev.key === "Backspace") void up();
    else if (ev.key === "Delete" && picked.length) setRemoving(picked);
    else if (ev.key === "F2" && cur) setRenaming(cur.name);
    else if (ev.key === "Escape" && sel.length > 1) selectOnly(cursor);
    else if (ev.key === "F5") {
      ev.preventDefault();
      void refresh();
    } else if (ctrl && ev.key.toLowerCase() === "a") {
      ev.preventDefault();
      setSel(sorted.map((e) => e.name));
    } else if (ctrl && ev.key.toLowerCase() === "l") {
      ev.preventDefault();
      setEditingPath(path);
    }
  };

  /* ---------- view ---------- */

  if (!conn || sorted === null) {
    return (
      <div className="files">
        <div className="files-center">
          <Gate host={host} phase={phase} listError={listError} connect={connect} retryList={() => conn && open(conn.id, conn.home)} onClose={onClose} />
        </div>
      </div>
    );
  }

  const where = path === "/" ? "/" : path.split("/").pop();
  const SortMark = ({ by }: { by: SortBy }) => (sort.by === by ? sort.desc ? <ChevronDown size={12} /> : <ChevronUp size={12} /> : null);

  return (
    <div
      className="files"
      onDragOver={(e) => !api.real && (e.preventDefault(), setDragOver(true))}
      onDragLeave={(e) => !api.real && e.currentTarget === e.target && setDragOver(false)}
      onDrop={(e) => {
        if (api.real) return;
        e.preventDefault();
        setDragOver(false);
        upload(api.droppedFiles(e.dataTransfer.files));
      }}
    >
      <div className="files-bar">
        <IconButton icon={ArrowUp} label="Вверх · Backspace" disabled={path === "/"} onClick={() => void up()} />
        <IconButton icon={House} label="Домашняя папка" disabled={path === conn.home} onClick={() => void go(conn.home)} />
        {editingPath !== null ? (
          <input
            className="input mono files-path-input"
            autoFocus
            value={editingPath}
            onChange={(e) => setEditingPath(e.target.value)}
            onFocus={(e) => e.target.select()}
            onBlur={() => setEditingPath(null)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                setEditingPath(null);
                listRef.current?.focus();
              }
              if (e.key === "Enter" && editingPath.trim()) {
                const to = editingPath.trim();
                setEditingPath(null);
                void go(to.startsWith("/") ? to : join(path, to));
                listRef.current?.focus();
              }
            }}
          />
        ) : (
          <Crumbs path={path} onGo={(p) => void go(p)} onEdit={() => setEditingPath(path)} />
        )}
        {loading && <Loader2 size={14} className="spin muted" />}
        <IconButton icon={RotateCw} label="Обновить · F5" onClick={() => void refresh()} />
        <IconButton icon={SquareTerminal} label="Терминал в этой папке" onClick={() => openTerminal(host.id, cdTo(path))} />
        <Button size="sm" icon={FolderPlus} onClick={() => setCreating(true)}>Папка</Button>
        {api.real && <IconButton icon={FolderUp} label="Загрузить папку" onClick={() => void pickFolders()} />}
        <Button size="sm" variant="primary" icon={Upload} onClick={() => void pickUploads()}>Загрузить</Button>
      </div>

      {picked.length > 1 && (
        <div className="files-selbar">
          <span>Выбрано: {picked.length}</span>
          <Button size="sm" icon={Download} onClick={() => void download(picked)}>Скачать</Button>
          <Button size="sm" variant="ghost" icon={Trash2} className="danger-text" onClick={() => setRemoving(picked)}>Удалить</Button>
          <span className="grow" />
          <IconButton icon={X} label="Снять выделение · Esc" onClick={() => selectOnly(cursor)} />
        </div>
      )}

      <div className="files-list" ref={listRef} tabIndex={0} onKeyDown={onListKey} role="grid" aria-label={path} aria-multiselectable>
        <div className="files-row files-head" role="row">
          <span />
          <button className="files-sort" onClick={() => toggleSort("name")}>Имя <SortMark by="name" /></button>
          <button className="files-sort files-size" onClick={() => toggleSort("size")}><SortMark by="size" /> Размер</button>
          <button className="files-sort" onClick={() => toggleSort("modified")}>Изменён <SortMark by="modified" /></button>
          <span>Права</span>
          <span />
        </div>
        {creating && (
          <div className="files-row selected" role="row">
            <Folder size={15} className="files-icon dir" />
            <NameInput initial="Новая папка" onDone={(v) => void mkdir(v ?? "")} />
            <span />
            <span />
            <span />
            <span />
          </div>
        )}
        {sorted.map((e) => {
          const Icon = e.kind === "dir" ? Folder : e.kind === "link" ? Link2 : File;
          const isSel = sel.includes(e.name);
          return (
            <div
              key={e.name}
              data-name={e.name}
              role="row"
              aria-selected={isSel}
              className={`files-row ${isSel ? "selected" : ""} ${cursor === e.name ? "cursor" : ""}`}
              onClick={(ev) => clickRow(ev, e.name)}
              onDoubleClick={() => void enter(e)}
            >
              <Icon size={15} className={`files-icon ${e.kind}`} />
              {renaming === e.name ? (
                <NameInput initial={e.name} onDone={(v) => void rename(e.name, v ?? e.name)} />
              ) : (
                <span className="files-name">{e.name}</span>
              )}
              <span className="files-size">{e.kind === "dir" ? "" : formatBytes(e.size)}</span>
              <span className="files-date">{formatModified(e.modified)}</span>
              <span className="files-mode mono">{formatMode(e.permissions)}</span>
              <span className="row-actions">
                <IconButton icon={Download} label="Скачать" onClick={(ev) => (ev.stopPropagation(), void download([e]))} />
                <Menu
                  trigger={(toggle, isOpen) => <IconButton icon={Ellipsis} label="Ещё" className={isOpen ? "pressed" : ""} onClick={toggle} />}
                  items={[
                    ...(e.kind === "dir" ? [{ label: "Открыть", icon: FolderOpen, onSelect: () => void enter(e) }] : []),
                    { label: "Скачать", icon: Download, onSelect: () => void download([e]) },
                    { label: "Копировать путь", icon: Copy, onSelect: () => void copyPath(e) },
                    ...(e.kind === "dir" ? [{ label: "Терминал в этой папке", icon: SquareTerminal, onSelect: () => openTerminal(host.id, cdTo(join(path, e.name))) }] : []),
                    { label: "Права доступа", icon: KeyRound, onSelect: () => setChmodding(e) },
                    { label: "Переименовать", icon: Pencil, hint: "F2", onSelect: () => setRenaming(e.name) },
                    { label: "Удалить", icon: Trash2, danger: true, hint: "Del", onSelect: () => setRemoving([e]) },
                  ]}
                />
              </span>
            </div>
          );
        })}
        {sorted.length === 0 && !creating && (
          <div className="files-empty">
            <span>Папка пуста</span>
            <Button size="sm" icon={Upload} onClick={() => void pickUploads()}>Загрузить файлы</Button>
          </div>
        )}
      </div>

      {transfers.length > 0 && (
        <div className="files-transfers">
          {transfers.map((t) => (
            <TransferRow key={t.id} t={t} onCancel={() => void api.filesCancel(t.id)} onDismiss={() => dropTransfer(t.id)} />
          ))}
        </div>
      )}

      {dragOver && (
        <div className="files-drop">
          <Upload size={22} />
          <span>Загрузить в {where}</span>
        </div>
      )}

      {removing && (
        <ConfirmDialog
          title={removing.length === 1 ? `Удалить «${removing[0]!.name}»?` : `Удалить ${removing.length} ${plural(removing.length, "элемент", "элемента", "элементов")}?`}
          body={removing.some((e) => e.kind === "dir") ? "Папки вместе со всем содержимым" : undefined}
          confirm="Удалить"
          onConfirm={() => void remove(removing)}
          onClose={() => {
            setRemoving(null);
            focusList();
          }}
        />
      )}
      {replacing && (
        <ConfirmDialog
          title={replacing.existing.length === 1 ? `Заменить «${replacing.existing[0]}»?` : `Заменить ${replacing.existing.length} ${plural(replacing.existing.length, "элемент", "элемента", "элементов")}?`}
          confirm="Заменить"
          onConfirm={() => upload(replacing.items, true)}
          onClose={() => {
            setReplacing(null);
            focusList();
          }}
        />
      )}
      {chmodding && (
        <PermissionsDialog
          entry={chmodding}
          onClose={() => {
            setChmodding(null);
            focusList();
          }}
          onSave={(mode) => void chmod(chmodding, mode)}
        />
      )}
    </div>
  );
}

/* ---------- pieces ---------- */

const WHO = [
  { label: "Владелец", shift: 6 },
  { label: "Группа", shift: 3 },
  { label: "Остальные", shift: 0 },
];
const WHAT = [
  { label: "Чтение", bit: 4 },
  { label: "Запись", bit: 2 },
  { label: "Выполнение", bit: 1 },
];

/** Unix permission bits as checkboxes and as a number, kept in step. */
function PermissionsDialog({ entry, onClose, onSave }: { entry: FileEntry; onClose: () => void; onSave: (mode: number) => void }) {
  const [mode, setMode] = useState(entry.permissions ?? (entry.kind === "dir" ? 0o755 : 0o644));
  const [octal, setOctal] = useState((mode & 0o777).toString(8).padStart(3, "0"));
  const set = (m: number) => {
    setMode(m);
    setOctal((m & 0o777).toString(8).padStart(3, "0"));
  };
  return (
    <Modal
      title={`Права «${entry.name}»`}
      width={420}
      onClose={onClose}
      onSubmit={() => {
        onSave(mode);
        onClose();
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary">Сохранить</Button>
        </>
      }
    >
      <div className="perm-grid">
        <span />
        {WHAT.map((w) => <span key={w.bit} className="perm-head">{w.label}</span>)}
        {WHO.map((who) => (
          <Fragment key={who.label}>
            <span className="perm-who">{who.label}</span>
            {WHAT.map((w) => {
              const bit = w.bit << who.shift;
              return (
                <span key={`${who.shift}-${w.bit}`} className="perm-cell">
                  <Checkbox checked={!!(mode & bit)} onChange={(on) => set(on ? mode | bit : mode & ~bit)}>{""}</Checkbox>
                </span>
              );
            })}
          </Fragment>
        ))}
      </div>
      <Field label="Числом">
        <input
          className="input mono perm-octal"
          value={octal}
          maxLength={4}
          onChange={(e) => {
            const v = e.target.value.replace(/[^0-7]/g, "");
            setOctal(v);
            if (v.length >= 3) setMode((mode & ~0o7777) | parseInt(v, 8));
          }}
        />
      </Field>
    </Modal>
  );
}

function Crumbs({ path, onGo, onEdit }: { path: string; onGo: (p: string) => void; onEdit: () => void }) {
  const parts = path.split("/").filter(Boolean);
  const ref = useRef<HTMLDivElement>(null);
  // A long path shows its end: the folder you are in.
  useEffect(() => {
    if (ref.current) ref.current.scrollLeft = ref.current.scrollWidth;
  }, [path]);
  return (
    <div className="files-crumbs" ref={ref} onClick={(e) => e.target === e.currentTarget && onEdit()} title="Ctrl L">
      <button className="crumb" onClick={() => onGo("/")}>/</button>
      {parts.map((p, i) => (
        <span key={i} className="crumb-part">
          {i > 0 && <span className="crumb-sep">/</span>}
          <button className={`crumb ${i === parts.length - 1 ? "current" : ""}`} onClick={() => onGo("/" + parts.slice(0, i + 1).join("/"))}>
            {p}
          </button>
        </span>
      ))}
    </div>
  );
}

/** An inline name field: Enter or leaving it keeps the name, Esc gives null. */
function NameInput({ initial, onDone }: { initial: string; onDone: (v: string | null) => void }) {
  const [v, setV] = useState(initial);
  const done = useRef(false);
  const finish = (value: string | null) => {
    if (done.current) return;
    done.current = true;
    onDone(value);
  };
  return (
    <input
      className="input files-name-input"
      autoFocus
      value={v}
      onChange={(e) => setV(e.target.value)}
      onFocus={(e) => {
        // Select the name without its extension.
        const dot = initial.lastIndexOf(".");
        e.target.setSelectionRange(0, dot > 0 ? dot : initial.length);
      }}
      onClick={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onBlur={() => finish(v)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(v);
        if (e.key === "Escape") finish(null);
      }}
    />
  );
}

function TransferRow({ t, onCancel, onDismiss }: { t: Transfer; onCancel: () => void; onDismiss: () => void }) {
  const pct = t.total ? Math.round((t.done / t.total) * 100) : 0;
  const secs = (Date.now() - t.started) / 1000;
  const speed = secs >= 1 && t.done ? ` · ${formatBytes(t.done / secs)}/с` : "";
  return (
    <div className={`transfer ${t.state}`}>
      {t.state === "done" ? <Check size={14} className="ok-text" /> : t.up ? <ArrowUp size={14} /> : <ArrowDown size={14} />}
      <span className="transfer-name">{t.name}</span>
      {t.state === "active" && (
        <>
          <span className="transfer-bar"><span style={{ width: `${pct}%` }} /></span>
          <span className="transfer-amount">{t.total ? `${formatBytes(t.done)} из ${formatBytes(t.total)}${speed}` : ""}</span>
        </>
      )}
      {t.state === "done" && <span className="transfer-amount ok-text">{t.up ? "Загружен" : "Скачан"}</span>}
      {t.state === "error" && <span className="transfer-amount err-text">{t.error}</span>}
      <IconButton icon={X} label={t.state === "active" ? "Отменить" : "Убрать"} tipSide="left" onClick={t.state === "active" ? onCancel : onDismiss} />
    </div>
  );
}

/** Logging in: progress, the questions a login asks, and failures. */
function Gate({
  host,
  phase,
  listError,
  connect,
  retryList,
  onClose,
}: {
  host: Host;
  phase: Phase;
  listError: string | null;
  connect: (a?: { secret?: string; secretKind?: SecretKind; remember?: boolean; trust?: string }) => void;
  retryList: () => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState("");
  const [remember, setRemember] = useState(true);
  const { toast } = useStore();
  // A rejected password is typed again from scratch.
  useEffect(() => setValue(""), [phase]);
  const where = `${host.address}:${host.port}`;

  if (phase.kind === "connecting" || (phase.kind === "ready" && !listError)) {
    return (
      <div className="files-wait">
        <Loader2 size={18} className="spin" />
        <span>{host.user}@{host.address}</span>
      </div>
    );
  }

  if (phase.kind === "ready" || phase.kind === "failed") {
    return (
      <div className="hostkey-card danger">
        <div className="hostkey-head">
          <ShieldAlert size={20} />
          <h2>{phase.kind === "failed" ? phase.error : listError}</h2>
        </div>
        <div className="hostkey-host mono">{host.user}@{where}</div>
        <div className="hostkey-actions">
          <Button variant="ghost" onClick={onClose}>Закрыть</Button>
          <Button variant="primary" icon={RotateCw} autoFocus onClick={() => (phase.kind === "ready" ? retryList() : connect())}>Повторить</Button>
        </div>
      </div>
    );
  }

  if (phase.kind === "secret") {
    const label = phase.secret === "passphrase" ? "Пароль ключа" : `Пароль ${host.user}@${host.address}`;
    return (
      <form
        className="hostkey-card"
        onSubmit={(e) => {
          e.preventDefault();
          if (value) connect({ secret: value, secretKind: phase.secret, remember, trust: phase.trust });
        }}
      >
        <div className="hostkey-head">
          <FolderOpen size={20} />
          <h2>{host.label}</h2>
        </div>
        <Field label={label} error={phase.error}>
          <PasswordInput value={value} onChange={setValue} invalid={!!phase.error} autoFocus />
        </Field>
        <Checkbox checked={remember} onChange={setRemember}>Сохранить в vault</Checkbox>
        <div className="hostkey-actions">
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" disabled={!value}>Подключиться</Button>
        </div>
      </form>
    );
  }

  if (phase.kind === "trust") {
    return (
      <form
        className="hostkey-card"
        onSubmit={(e) => {
          e.preventDefault();
          connect({ trust: phase.fingerprint });
        }}
      >
        <div className="hostkey-head">
          <ShieldQuestion size={20} />
          <h2>Новый хост</h2>
        </div>
        <div className="hostkey-host mono">{where}</div>
        <dl className="hostkey-fp">
          <dt>Отпечаток</dt>
          <dd className="mono">{phase.fingerprint}</dd>
        </dl>
        <div className="hostkey-actions">
          <Button type="button" variant="ghost" onClick={onClose}>Отмена</Button>
          <Button type="submit" variant="primary" autoFocus>Доверять и подключиться</Button>
        </div>
      </form>
    );
  }

  return (
    <div className="hostkey-card danger">
      <div className="hostkey-head">
        <ShieldAlert size={20} />
        <h2>Ключ хоста изменился</h2>
      </div>
      <div className="hostkey-host mono">{where}</div>
      <dl className="hostkey-fp">
        <dt>Был</dt>
        <dd className="mono">{phase.expected}</dd>
        <dt>Сейчас</dt>
        <dd className="mono">{phase.actual}</dd>
      </dl>
      <div className="hostkey-actions">
        <Button
          variant="ghost"
          className="danger-text"
          onClick={async () => {
            try {
              await api.hostForgetKey(host.vaultId, host.id);
              connect({ trust: phase.actual });
            } catch (e) {
              toast({ text: errorText(e), tone: "error" });
            }
          }}
        >
          Заменить ключ
        </Button>
        <Button variant="primary" autoFocus onClick={onClose}>Отменить</Button>
      </div>
    </div>
  );
}
