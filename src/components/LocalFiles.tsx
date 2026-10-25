import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AppWindow, ArrowUp, ChevronDown, ChevronUp, Copy, CornerDownLeft, ExternalLink, File, FilePen, Folder, FolderOpen, FolderPlus, House, Link2, Loader2, Monitor, RotateCw, Search, Upload } from "lucide-react";
import { api, errorText, type Editor, type FileEntry, type LocalFile } from "../lib/api";
import { EditorChooser, openIn } from "./Editors";
import { copyText, shortcutKey, useStore } from "../lib/store";
import { tr, useT } from "../i18n";
import { Button, ContextMenu, IconButton, type MenuItem } from "./ui";
import { HostIcon } from "./common";
import { NameInput, formatBytes, formatModified, sortEntries, type SortBy, type StartDrag } from "./FilesView";

/* ---------- paths on this computer: C:\Users\me on Windows, /home/me elsewhere ---------- */

const isWin = (p: string) => /^[A-Za-z]:\\/.test(p);

export const localJoin = (dir: string, name: string) => {
  if (!dir) return name;
  const sep = isWin(dir) ? "\\" : "/";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
};

/** The folder above; "" is the list of drives on Windows. */
const localParent = (p: string) => {
  if (isWin(p)) {
    const t = p.replace(/\\+$/, "");
    const i = t.lastIndexOf("\\");
    return i < 0 ? "" : i === 2 ? t.slice(0, 3) : t.slice(0, i);
  }
  const i = p.replace(/\/+$/, "").lastIndexOf("/");
  return i <= 0 ? "/" : p.slice(0, i);
};

const canGoUp = (p: string) => p !== "" && p !== "/";

/** Where `dir` is, for a button: its last part. */
export const lastPart = (dir: string) => dir.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || dir;

/**
 * This computer's side of a file browser: browse, make folders, and send what
 * is picked to the server's folder (`target`, null while there is none).
 */
export function LocalPane({ dir, onDir, refreshKey, target, onUpload, onDragRows }: { dir: string | null; onDir: (d: string) => void; refreshKey: number; target: string | null; onUpload: (items: LocalFile[]) => void; onDragRows: StartDrag }) {
  const t = useT();
  const { toast, settings } = useStore();
  const [entries, setEntries] = useState<FileEntry[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sel, setSel] = useState<string[]>([]);
  const [cursor, setCursor] = useState<string | null>(null);
  const anchor = useRef<string | null>(null);
  const [sort, setSort] = useState<{ by: SortBy; desc: boolean }>({ by: "name", desc: false });
  const [creating, setCreating] = useState(false);
  const [editingPath, setEditingPath] = useState<string | null>(null);
  const [home, setHome] = useState<string | null>(null);
  const [menuAt, setMenuAt] = useState<{ x: number; y: number; entry: FileEntry } | null>(null);
  const [choosing, setChoosing] = useState<FileEntry | null>(null);
  const closeMenu = useCallback(() => setMenuAt(null), []);
  const listRef = useRef<HTMLDivElement>(null);
  const dirRef = useRef(dir);
  dirRef.current = dir;

  const sorted = useMemo(() => (entries ? sortEntries(entries, sort.by, sort.desc) : null), [entries, sort]);
  const picked = sorted?.filter((e) => sel.includes(e.name)) ?? [];

  const selectOnly = (name: string | null) => {
    setSel(name ? [name] : []);
    setCursor(name);
    anchor.current = name;
  };

  const selectRange = (name: string) => {
    if (!sorted) return;
    const names = sorted.map((e) => e.name);
    const a = names.indexOf(anchor.current ?? name);
    const b = names.indexOf(name);
    setSel(names.slice(Math.min(a, b), Math.max(a, b) + 1));
    setCursor(name);
  };

  /** Shows a folder; `select` picks an entry in it. */
  const go = async (to: string, select?: string | null) => {
    setLoading(true);
    try {
      const list = await api.localList(to);
      setEntries(list);
      setError(null);
      onDir(to);
      selectOnly(select !== undefined ? select : sortEntries(list, sort.by, sort.desc)[0]?.name ?? null);
      setCreating(false);
    } catch (e) {
      if (entries) toast({ text: errorText(e), tone: "error" });
      else setError(errorText(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void api.localHome().then((h) => {
      setHome(h);
      void go(dir ?? h);
    });
    // Starts once; later folders come from go().
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Something was downloaded here: show it.
  useEffect(() => {
    if (refreshKey && dirRef.current !== null) void go(dirRef.current, cursor);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshKey]);

  const here = dir ?? "";
  const up = () => canGoUp(here) && void go(localParent(here), lastPart(here));
  const enter = (e: FileEntry) => {
    if (e.kind === "dir" || e.kind === "link") return void go(localJoin(here, e.name));
    if (target) upload([e]);
  };
  const upload = (items: FileEntry[]) => {
    if (!target || !here || items.length === 0) return;
    onUpload(items.map((e) => ({ path: localJoin(here, e.name), name: e.name })));
  };

  const openWith = async (e: FileEntry, editor: Editor | null) => {
    try {
      await openIn(localJoin(here, e.name), editor);
    } catch (err) {
      toast({ text: errorText(err), tone: "error" });
    }
  };

  const copyPath = async (e: FileEntry) => {
    const p = localJoin(here, e.name);
    toast((await copyText(p)) ? { text: tr().files.pathCopied(p), tone: "success" } : { text: tr().common.copyFailed, tone: "error" });
  };

  /** The menu of a row; for a row in a selection of several, what applies to all of them. */
  const rowMenu = (e: FileEntry): MenuItem[] => {
    const many = picked.length > 1 && sel.includes(e.name);
    const send = target && here ? [{ label: t.files.local.uploadTo(target), icon: Upload, onSelect: () => upload(many ? picked : [e]) }] : [];
    if (many) return send;
    const isDir = e.kind === "dir" || !here;
    return [
      { label: t.files.menu.open, icon: isDir ? FolderOpen : ExternalLink, onSelect: () => (isDir ? void go(localJoin(here, e.name)) : void openWith(e, null)) },
      ...(!isDir && settings.editor ? [{ label: `${t.files.menu.edit} · ${settings.editor.name}`, icon: FilePen, onSelect: () => void openWith(e, settings.editor) }] : []),
      ...(!isDir ? [{ label: t.files.menu.editIn, icon: AppWindow, onSelect: () => setChoosing(e) }] : []),
      ...send,
      ...(here ? [{ label: t.files.menu.copyPath, icon: Copy, onSelect: () => void copyPath(e) }] : []),
    ];
  };

  const mkdir = async (name: string) => {
    setCreating(false);
    requestAnimationFrame(() => listRef.current?.focus());
    if (!name.trim() || !here) return;
    try {
      await api.localMkdir(localJoin(here, name.trim()));
      void go(here, name.trim());
    } catch (e) {
      toast({ text: errorText(e), tone: "error" });
    }
  };

  const toggleSort = (by: SortBy) => setSort((s) => (s.by === by ? { by, desc: !s.desc } : { by, desc: by !== "name" }));
  const SortMark = ({ by }: { by: SortBy }) => (sort.by === by ? sort.desc ? <ChevronDown size={12} /> : <ChevronUp size={12} /> : null);

  const onKey = (ev: React.KeyboardEvent) => {
    if (!sorted || creating || (ev.target as HTMLElement).tagName === "INPUT") return;
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
    else if (ev.key === "Enter" && cur) picked.length > 1 ? upload(picked) : enter(cur);
    else if (ev.key === "Backspace") up();
    else if (ev.key === "Escape" && sel.length > 1) selectOnly(cursor);
    else if (ev.key === "F5") {
      ev.preventDefault();
      void go(here, cursor);
    } else if (ctrl && shortcutKey(ev) === "a") {
      ev.preventDefault();
      setSel(sorted.map((e) => e.name));
    } else if (ctrl && shortcutKey(ev) === "l") {
      ev.preventDefault();
      setEditingPath(here);
    }
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

  const canUpload = !!target && !!here && picked.length > 0;
  return (
    <div className="files-pane local">
      <div className="files-bar">
        <span className="files-side" title={t.files.local.title}><Monitor size={15} /></span>
        <IconButton icon={ArrowUp} label={t.files.bar.up} disabled={!canGoUp(here)} onClick={up} />
        <IconButton icon={House} label={t.files.bar.home} disabled={!home || here === home} onClick={() => home && void go(home)} />
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
                void go(to);
                listRef.current?.focus();
              }
            }}
          />
        ) : (
          <LocalCrumbs path={here} onGo={(p) => void go(p)} onEdit={() => setEditingPath(here)} />
        )}
        {loading && <Loader2 size={14} className="spin muted" />}
        <IconButton icon={RotateCw} label={t.files.bar.refresh} onClick={() => void go(here, cursor)} />
        {here && <IconButton icon={FolderPlus} label={t.files.bar.newFolder} onClick={() => setCreating(true)} />}
        <Button size="sm" variant="primary" icon={Upload} disabled={!canUpload} title={target ? t.files.local.uploadTo(target) : undefined} onClick={() => upload(picked)}>
          {t.files.local.upload}
        </Button>
      </div>

      {error && !sorted ? (
        <div className="files-center"><span className="err-text">{error}</span></div>
      ) : (
        <div className="files-list" ref={listRef} tabIndex={0} onKeyDown={onKey} role="grid" aria-label={here || t.files.local.drives} aria-multiselectable>
          <div className="files-row local files-head" role="row">
            <span />
            <button className="files-sort" onClick={() => toggleSort("name")}>{t.files.columns.name} <SortMark by="name" /></button>
            <button className="files-sort files-size" onClick={() => toggleSort("size")}><SortMark by="size" /> {t.files.columns.size}</button>
            <button className="files-sort files-date" onClick={() => toggleSort("modified")}>{t.files.columns.modified} <SortMark by="modified" /></button>
            <span />
          </div>
          {creating && (
            <div className="files-row local selected" role="row">
              <Folder size={15} className="files-icon dir" />
              <NameInput initial={t.files.newFolderName} onDone={(v) => void mkdir(v ?? "")} />
              <span />
              <span />
              <span />
            </div>
          )}
          {sorted?.map((e) => {
            const Icon = e.kind === "dir" ? Folder : e.kind === "link" ? Link2 : File;
            const isSel = sel.includes(e.name);
            return (
              <div
                key={e.name}
                data-name={e.name}
                role="row"
                aria-selected={isSel}
                className={`files-row local ${isSel ? "selected" : ""} ${cursor === e.name ? "cursor" : ""}`}
                onClick={(ev) => clickRow(ev, e.name)}
                onDoubleClick={() => enter(e)}
                onMouseDown={(ev) => {
                  if (!here) return;
                  const items = isSel ? picked : [e];
                  onDragRows(ev, items.length === 1 ? e.name : t.files.items(items.length), () => upload(items));
                }}
                onContextMenu={(ev) => {
                  ev.preventDefault();
                  if (!isSel) selectOnly(e.name);
                  setMenuAt({ x: ev.clientX, y: ev.clientY, entry: e });
                }}
              >
                <Icon size={15} className={`files-icon ${e.kind}`} />
                <span className="files-name">{e.name}</span>
                <span className="files-size">{e.kind === "dir" ? "" : formatBytes(e.size)}</span>
                <span className="files-date">{formatModified(e.modified)}</span>
                <span className="row-actions">
                  {target && here && <IconButton icon={Upload} label={t.files.local.uploadTo(target)} tipSide="left" onClick={(ev) => (ev.stopPropagation(), upload([e]))} />}
                </span>
              </div>
            );
          })}
          {sorted?.length === 0 && !creating && <div className="files-empty"><span>{t.files.empty}</span></div>}
        </div>
      )}
      {menuAt && <ContextMenu at={menuAt} items={rowMenu(menuAt.entry)} onClose={closeMenu} />}
      {choosing && <EditorChooser onPick={(ed) => void openWith(choosing, ed)} onClose={() => setChoosing(null)} />}
    </div>
  );
}

/** C: › Users › me on Windows (Up from C: lists the drives), / › home › me elsewhere. */
function LocalCrumbs({ path, onGo, onEdit }: { path: string; onGo: (p: string) => void; onEdit: () => void }) {
  const t = useT().files.local;
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (ref.current) ref.current.scrollLeft = ref.current.scrollWidth;
  }, [path]);
  const win = isWin(path);
  const names = win ? path.slice(3).split("\\").filter(Boolean) : path.split("/").filter(Boolean);
  // Each crumb and the path it opens.
  const crumbs: { label: string; to: string }[] = win
    ? [{ label: path.slice(0, 2), to: path.slice(0, 3) }, ...names.map((n, i) => ({ label: n, to: path.slice(0, 3) + names.slice(0, i + 1).join("\\") }))]
    : path === ""
      ? [{ label: t.drives, to: "" }]
      : [{ label: "/", to: "/" }, ...names.map((n, i) => ({ label: n, to: "/" + names.slice(0, i + 1).join("/") }))];
  return (
    <div className="files-crumbs" ref={ref} onClick={(e) => e.target === e.currentTarget && onEdit()} title="Ctrl L">
      {crumbs.map((c, i) => (
        <span key={i} className="crumb-part">
          {/* On Windows › between all; elsewhere the root "/" is itself the first separator. */}
          {(win ? i > 0 : i > 1) && <span className="crumb-sep">{win ? "›" : "/"}</span>}
          <button className={`crumb ${i === crumbs.length - 1 ? "current" : ""}`} onClick={() => onGo(c.to)}>{c.label}</button>
        </span>
      ))}
    </div>
  );
}

/** The server's side of a new SFTP tab before it has one: which host to open. */
export function HostChooser({ onPick }: { onPick: (hostId: string) => void }) {
  const t = useT();
  const { ws, vault } = useStore();
  const [q, setQ] = useState("");
  const [index, setIndex] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const hosts = useMemo(() => {
    const query = q.trim().toLowerCase();
    return [...ws.hosts]
      .sort((a, b) => Number(b.vaultId === vault.id) - Number(a.vaultId === vault.id))
      .filter((h) => !query || [h.label, h.address, h.user, ...h.tags].some((x) => x.toLowerCase().includes(query)));
  }, [q, ws.hosts, vault.id]);
  useEffect(() => setIndex(0), [q]);
  useEffect(() => {
    listRef.current?.querySelector(`[data-index="${index}"]`)?.scrollIntoView({ block: "nearest" });
  }, [index]);
  const vaultName = (id: string) => (id === vault.id ? null : ws.vaults.find((v) => v.id === id)?.name ?? null);

  return (
    <div className="files-center">
      <div className="host-chooser">
        <div className="host-chooser-title">{t.files.pick.title}</div>
        <div className="palette-input">
          <Search size={16} />
          <input
            autoFocus
            value={q}
            placeholder={t.files.pick.search}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) => Math.min(hosts.length - 1, i + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(0, i - 1));
              } else if (e.key === "Enter" && hosts[index]) {
                onPick(hosts[index].id);
              }
            }}
          />
        </div>
        <div className="palette-list" ref={listRef}>
          {hosts.length === 0 && <div className="palette-empty">{ws.hosts.length ? t.files.pick.nothingFound : t.files.pick.noHosts}</div>}
          {hosts.map((h, i) => (
            <button key={h.id} data-index={i} className={`palette-item ${i === index ? "active" : ""}`} onMouseMove={() => setIndex(i)} onClick={() => onPick(h.id)}>
              <HostIcon host={h} size={24} />
              <span className="strong">{h.label}</span>
              <span className="muted mono small">{h.user}@{h.address}</span>
              <span className="grow" />
              {vaultName(h.vaultId) && <span className="chip">{vaultName(h.vaultId)}</span>}
              {i === index && <CornerDownLeft size={13} className="muted" />}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
