import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AppWindow, ArrowDown, ArrowUp, Check, ChevronDown, ChevronUp, Copy, Download, Ellipsis, File, FilePen, FolderOpen, FolderPlus, Folder, House, KeyRound, Link2, Loader2, Pencil, RotateCw, Server, ShieldAlert, ShieldQuestion, SquareTerminal, Trash2, Upload, X,
} from "lucide-react";
import { api, errorText, isCoreError, trustMore, type Editor, type FileEntry, type LocalFile, type SecretKind } from "../lib/api";
import { copyText, shortcutKey, uid, useStore } from "../lib/store";
import { locale, tr, useT } from "../i18n";
import type { Host } from "../lib/types";
import type { SessionStatus } from "../lib/fakeShell";
import { Button, Checkbox, ConfirmDialog, ContextMenu, Field, IconButton, Menu, Modal, PasswordInput, type MenuItem } from "./ui";
import { HostChooser, LocalPane, lastPart } from "./LocalFiles";
import { EditorChooser, openIn } from "./Editors";

interface Props {
  /** Null until a host is chosen for a new SFTP tab. */
  host: Host | null;
  active: boolean;
  onStatus: (s: SessionStatus) => void;
  onClose: () => void;
  onPickHost: (hostId: string) => void;
}

/** What this computer's side can ask of the server's side. */
interface RemoteHandle {
  upload: (items: LocalFile[]) => void;
  download: (items: FileEntry[]) => void;
}

/** Rows dragged from one side; `drop` runs when they land on the other. */
export type StartDrag = (ev: React.MouseEvent, label: string, drop: () => void) => void;

type Side = "local" | "remote";
/** The other side under the pointer, when it takes the drop. */
type Over = { side: Side; rect: DOMRect; label: string } | null;
interface Layer {
  show: (d: { from: Side; label: string; x: number; y: number } | null) => void;
  over: (o: Over) => void;
  move: (x: number, y: number) => void;
}

/**
 * Dragging rows between the two sides, with the pointer: the webview's own
 * drag and drop is taken by drops from the system. Only the drag layer
 * redraws while the pointer moves; the lists don't.
 */
function usePaneDrag(accepts: (to: Side) => string | null) {
  const layer = useRef<Layer | null>(null);
  const acceptsRef = useRef(accepts);
  acceptsRef.current = accepts;

  const start = (from: Side): StartDrag => (ev, label, drop) => {
    if (ev.button !== 0 || (ev.target as HTMLElement).closest("button, input")) return;
    const x0 = ev.clientX;
    const y0 = ev.clientY;
    let dragging = false;
    let over: Over = null;
    let frame = 0;
    let last = { x: x0, y: y0 };
    // The other side of this tab, measured once: hit tests by coordinates cost nothing per move.
    const other = (ev.currentTarget as HTMLElement).closest(".files")?.querySelector<HTMLElement>(`.files-pane.${from === "local" ? "remote" : "local"}`);
    const rect = other?.getBoundingClientRect();
    const to: Side = from === "local" ? "remote" : "local";
    const sideAt = (x: number, y: number): Over => {
      if (!rect || x < rect.left || x > rect.right || y < rect.top || y > rect.bottom) return null;
      const into = acceptsRef.current(to);
      return into === null ? null : { side: to, rect, label: into };
    };
    const paint = () => {
      frame = 0;
      layer.current?.move(last.x, last.y);
      const now = sideAt(last.x, last.y);
      if (now?.side !== over?.side || now?.label !== over?.label) layer.current?.over(now);
      over = now;
    };
    const move = (e: MouseEvent) => {
      e.preventDefault();
      last = { x: e.clientX, y: e.clientY };
      if (!dragging) {
        if (Math.hypot(e.clientX - x0, e.clientY - y0) < 6) return;
        dragging = true;
        layer.current?.show({ from, label, x: e.clientX, y: e.clientY });
      }
      if (!frame) frame = requestAnimationFrame(paint);
    };
    const up = (e: MouseEvent) => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      cancelAnimationFrame(frame);
      if (dragging) {
        layer.current?.show(null);
        if (sideAt(e.clientX, e.clientY)) drop();
      }
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  };

  return { start, view: <DragLayer handle={layer} /> };
}

function DragLayer({ handle }: { handle: React.MutableRefObject<Layer | null> }) {
  const [drag, setDrag] = useState<{ from: Side; label: string; x: number; y: number } | null>(null);
  const [over, setOver] = useState<Over>(null);
  const ghost = useRef<HTMLDivElement>(null);
  handle.current = {
    show: (d) => {
      setDrag(d);
      setOver(null);
    },
    over: setOver,
    move: (x, y) => {
      if (ghost.current) ghost.current.style.transform = `translate(${x + 14}px, ${y + 14}px)`;
    },
  };
  if (!drag) return null;
  return (
    <>
      {/* Takes the pointer while dragging, so nothing below reacts to it or has to be restyled. */}
      <div className="files-drag-capture" />
      {over && (
        <div className="files-drop-zone" style={{ top: over.rect.top + 6, left: over.rect.left + 6, width: over.rect.width - 12, height: over.rect.height - 12 }}>
          {over.side === "remote" ? <Upload size={22} /> : <Download size={22} />}
          <span>{over.label}</span>
        </div>
      )}
      <div className="files-drag-ghost" ref={ghost} style={{ transform: `translate(${drag.x + 14}px, ${drag.y + 14}px)` }}>
        {drag.from === "local" ? <Upload size={13} /> : <Download size={13} />}
        <span>{drag.label}</span>
      </div>
    </>
  );
}

interface RemoteProps {
  host: Host;
  active: boolean;
  onStatus: (s: SessionStatus) => void;
  onClose: () => void;
  /** The folder on this computer that downloads go to. */
  localDir: string | null;
  onDownloaded: () => void;
  /** The server's folder once it shows, null before. */
  onFolder: (path: string | null) => void;
  handle: React.MutableRefObject<RemoteHandle | null>;
  onDragRows: StartDrag;
}

/** Where opening the browser stands. */
type Phase =
  | { kind: "connecting" }
  | { kind: "secret"; secret: SecretKind; trust?: string; error?: string }
  | { kind: "trust"; fingerprint: string; via?: string; trusted?: string }
  | { kind: "changed"; expected: string; actual: string; via?: string; trusted?: string }
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

export type SortBy = "name" | "size" | "modified";

/** `dir` joined with `name`, in the server's syntax. */
const join = (dir: string, name: string) => (dir.endsWith("/") ? dir + name : `${dir}/${name}`);

function parentOf(path: string) {
  const i = path.replace(/\/+$/, "").lastIndexOf("/");
  return i <= 0 ? "/" : path.slice(0, i);
}

/** "812 B", "8.4 MB", "177 MB", "1.2 GB". */
export function formatBytes(n: number) {
  const units = tr().files.units;
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = i === 0 || v >= 100 ? 0 : 1;
  return `${v.toLocaleString(locale(), { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${units[i]}`;
}

/** "14:32" today, "Oct 21 14:32" this year, "Oct 21, 2024" before. */
export function formatModified(ms: number | null) {
  if (!ms) return "";
  const d = new Date(ms);
  const now = new Date();
  const time = d.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" });
  if (d.toDateString() === now.toDateString()) return time;
  // Russian adds a dot after the month and a suffix after the year: both go.
  const date = d.toLocaleDateString(locale(), { day: "numeric", month: "short", ...(d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }) }).replace(".", "").replace(" \u0433", "");
  return d.getFullYear() === now.getFullYear() ? `${date} ${time}` : date;
}

/** `0o754` -> "rwxr-xr--". */
function formatMode(mode: number | null) {
  if (mode == null) return "";
  return [6, 3, 0].map((s) => ((mode >> s) & 4 ? "r" : "-") + ((mode >> s) & 2 ? "w" : "-") + ((mode >> s) & 1 ? "x" : "-")).join("");
}

/** Folders first, then by the chosen column. */
export function sortEntries(list: FileEntry[], by: SortBy, desc: boolean) {
  const key = (e: FileEntry) => (by === "size" ? e.size : by === "modified" ? (e.modified ?? 0) : 0);
  return [...list].sort((a, b) => {
    const dirs = Number(b.kind === "dir") - Number(a.kind === "dir");
    if (dirs) return dirs;
    const k = by === "name" ? a.name.localeCompare(b.name, locale(), { numeric: true, sensitivity: "base" }) : key(a) - key(b) || a.name.localeCompare(b.name, locale(), { numeric: true });
    return desc ? -k : k;
  });
}

/** A shell command that goes to `path`. */
const cdTo = (path: string) => `cd '${path.replace(/'/g, `'\\''`)}'`;

/** An SFTP tab: this computer on the left, the server on the right (or which host to open). */
export function FilesView({ host, active, onStatus, onClose, onPickHost }: Props) {
  const [localDir, setLocalDir] = useState<string | null>(null);
  const [localRefresh, setLocalRefresh] = useState(0);
  const [remoteDir, setRemoteDir] = useState<string | null>(null);
  const remote = useRef<RemoteHandle | null>(null);
  const target = host ? remoteDir : null;
  const t = useT().files;
  const drag = usePaneDrag((to) => (to === "remote" ? (target ? t.local.uploadTo(target) : null) : localDir ? t.downloadTo(localDir) : null));
  return (
    <div className="files">
      {drag.view}
      <div className="files-panes">
        <LocalPane dir={localDir} onDir={setLocalDir} refreshKey={localRefresh} target={target} onUpload={(items) => remote.current?.upload(items)} onDragRows={drag.start("local")} />
        {host ? (
          <RemotePane
            key={host.id}
            host={host}
            active={active}
            onStatus={onStatus}
            onClose={onClose}
            localDir={localDir}
            onDownloaded={() => setLocalRefresh((n) => n + 1)}
            onFolder={setRemoteDir}
            handle={remote}
            onDragRows={drag.start("remote")}
          />
        ) : (
          <div className="files-pane remote">
            <HostChooser onPick={onPickHost} />
          </div>
        )}
      </div>
    </div>
  );
}

function RemotePane({ host, active, onStatus, onClose, localDir, onDownloaded, onFolder, handle, onDragRows }: RemoteProps) {
  const t = useT();
  const { update, toast, applyTeamView, connect: openTerminal, settings } = useStore();
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
  const [menuAt, setMenuAt] = useState<{ x: number; y: number; entry: FileEntry } | null>(null);
  const [choosing, setChoosing] = useState<FileEntry | null>(null);
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
        if (isCoreError(e) && e.kind === "hostKeyUnknown") return setPhase({ kind: "trust", fingerprint: e.fingerprint, via: e.via, trusted: answer.trust });
        if (isCoreError(e) && e.kind === "hostKeyChanged") return setPhase({ kind: "changed", expected: e.expected, actual: e.actual, via: e.via, trusted: answer.trust });
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

  /** Into the folder open on this computer's side; without one, one file where the user says and more into a folder they pick. */
  const download = async (items: FileEntry[]) => {
    if (!conn || items.length === 0) return;
    if (localDir) {
      for (const e of items) {
        const t = transfer(false, e.name, e.kind === "dir" ? 0 : e.size);
        void run(t, (p) => api.filesDownloadInto(conn.id, join(t.dir, e.name), localDir, t.id, p)).then((ok) => void (ok && onDownloaded()));
      }
      return;
    }
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

  // Files and folders dragged in from the system, or sent from this computer's side.
  const uploadRef = useRef(upload);
  uploadRef.current = upload;
  const downloadRef = useRef<(items: FileEntry[]) => void>(() => {});
  downloadRef.current = (items) => void download(items);
  useEffect(() => {
    handle.current = { upload: (items) => uploadRef.current(items), download: (items) => downloadRef.current(items) };
    return () => void (handle.current = null);
  }, [handle]);

  /* ---------- editing in another app: a copy here, sent back each time it is saved ---------- */

  const edits = useRef(new Map<string, { remote: string; dir: string; name: string; modified: number | null; sending: boolean }>());
  const [editCount, setEditCount] = useState(0);

  const edit = async (e: FileEntry, editor: Editor | null) => {
    if (!conn) return;
    const remote = join(path, e.name);
    try {
      const local = await api.editCopyPath(e.name);
      const t = transfer(false, e.name, e.size);
      if (!(await run(t, (p) => api.filesDownload(conn.id, remote, local, t.id, p)))) return;
      edits.current.set(local, { remote, dir: path, name: e.name, modified: await api.localModified(local), sending: false });
      setEditCount(edits.current.size);
      await openIn(local, editor);
    } catch (err) {
      toast({ text: errorText(err), tone: "error" });
    }
  };

  // Each save in the editor goes to the server.
  useEffect(() => {
    if (!conn || editCount === 0) return;
    const timer = setInterval(() => {
      for (const [local, ed] of edits.current) {
        if (ed.sending) continue;
        void api.localModified(local).then((m) => {
          if (m === null) return void edits.current.delete(local);
          if (m === ed.modified) return;
          ed.modified = m;
          ed.sending = true;
          const t: Transfer = { ...transfer(true, ed.name), dir: ed.dir };
          void run(t, (p) => api.filesUpload(conn.id, { path: local, name: ed.name }, ed.remote, true, t.id, p)).then((ok) => {
            ed.sending = false;
            if (ok && pathRef.current === ed.dir) void refresh(ed.name);
          });
        });
      }
    }, 1500);
    return () => clearInterval(timer);
    // `run`, `transfer` and `refresh` only use setters and refs.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn?.id, editCount]);
  const shown = conn && entries ? path : null;
  useEffect(() => onFolder(shown), [shown]);
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
    if (gone) toast({ text: items.length === 1 ? tr().files.removedOne(items[0]!.name) : tr().files.removedMany(gone), tone: "success" });
    const left = sorted.filter((x) => !items.some((i) => i.name === x.name));
    void refresh(left[Math.min(Math.max(idx, 0), left.length - 1)]?.name ?? null);
  };

  const chmod = async (e: FileEntry, mode: number) => {
    if (!conn) return;
    try {
      await api.filesChmod(conn.id, join(path, e.name), mode);
      toast({ text: tr().files.modeSet(e.name, formatMode(mode)), tone: "success" });
      void refresh(e.name);
    } catch (err) {
      toast({ text: errorText(err), tone: "error" });
    }
  };

  const closeMenu = useCallback(() => setMenuAt(null), []);

  const copyPath = async (e: FileEntry) => {
    const p = join(path, e.name);
    toast((await copyText(p)) ? { text: tr().files.pathCopied(p), tone: "success" } : { text: tr().common.copyFailed, tone: "error" });
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
    } else if (ctrl && shortcutKey(ev) === "a") {
      ev.preventDefault();
      setSel(sorted.map((e) => e.name));
    } else if (ctrl && shortcutKey(ev) === "l") {
      ev.preventDefault();
      setEditingPath(path);
    }
  };

  /** The menu of a row; for a row in a selection of several, what applies to all of them. */
  const rowMenu = (e: FileEntry): MenuItem[] => {
    const many = picked.length > 1 && sel.includes(e.name);
    if (many) {
      return [
        { label: t.files.download, icon: Download, onSelect: () => void download(picked) },
        { label: t.common.delete, icon: Trash2, danger: true, hint: "Del", onSelect: () => setRemoving(picked) },
      ];
    }
    const isFile = e.kind !== "dir";
    return [
      ...(e.kind === "dir" ? [{ label: t.files.menu.open, icon: FolderOpen, onSelect: () => void enter(e) }] : []),
      ...(isFile ? [{ label: settings.editor ? `${t.files.menu.edit} · ${settings.editor.name}` : t.files.menu.edit, icon: FilePen, onSelect: () => void edit(e, settings.editor) }] : []),
      ...(isFile ? [{ label: t.files.menu.editIn, icon: AppWindow, onSelect: () => setChoosing(e) }] : []),
      { label: localDir ? t.files.downloadTo(lastPart(localDir)) : t.files.download, icon: Download, onSelect: () => void download([e]) },
      { label: t.files.menu.copyPath, icon: Copy, onSelect: () => void copyPath(e) },
      ...(e.kind === "dir" ? [{ label: t.files.bar.terminalHere, icon: SquareTerminal, onSelect: () => openTerminal(host.id, cdTo(join(path, e.name))) }] : []),
      { label: t.files.menu.permissions, icon: KeyRound, onSelect: () => setChmodding(e) },
      { label: t.common.rename, icon: Pencil, hint: "F2", onSelect: () => setRenaming(e.name) },
      { label: t.common.delete, icon: Trash2, danger: true, hint: "Del", onSelect: () => setRemoving([e]) },
    ];
  };

  /* ---------- view ---------- */

  if (!conn || sorted === null) {
    return (
      <div className="files-pane remote">
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
      className="files-pane remote"
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
        <span className="files-side" title={host.label}><Server size={15} /></span>
        <IconButton icon={ArrowUp} label={t.files.bar.up} disabled={path === "/"} onClick={() => void up()} />
        <IconButton icon={House} label={t.files.bar.home} disabled={path === conn.home} onClick={() => void go(conn.home)} />
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
        <IconButton icon={RotateCw} label={t.files.bar.refresh} onClick={() => void refresh()} />
        <IconButton icon={SquareTerminal} label={t.files.bar.terminalHere} onClick={() => openTerminal(host.id, cdTo(path))} />
        <IconButton icon={FolderPlus} label={t.files.bar.newFolder} onClick={() => setCreating(true)} />
        <Button size="sm" variant="primary" icon={Download} disabled={picked.length === 0} title={localDir ? t.files.downloadTo(localDir) : undefined} onClick={() => void download(picked)}>
          {t.files.download}
        </Button>
      </div>

      {picked.length > 1 && (
        <div className="files-selbar">
          <span>{t.files.selected(picked.length)}</span>
          <Button size="sm" icon={Download} onClick={() => void download(picked)}>{t.files.download}</Button>
          <Button size="sm" variant="ghost" icon={Trash2} className="danger-text" onClick={() => setRemoving(picked)}>{t.common.delete}</Button>
          <span className="grow" />
          <IconButton icon={X} label={t.files.clearSelection} onClick={() => selectOnly(cursor)} />
        </div>
      )}

      <div className="files-list" ref={listRef} tabIndex={0} onKeyDown={onListKey} role="grid" aria-label={path} aria-multiselectable>
        <div className="files-row files-head" role="row">
          <span />
          <button className="files-sort" onClick={() => toggleSort("name")}>{t.files.columns.name} <SortMark by="name" /></button>
          <button className="files-sort files-size" onClick={() => toggleSort("size")}><SortMark by="size" /> {t.files.columns.size}</button>
          <button className="files-sort" onClick={() => toggleSort("modified")}>{t.files.columns.modified} <SortMark by="modified" /></button>
          <span>{t.files.columns.permissions}</span>
          <span />
        </div>
        {creating && (
          <div className="files-row selected" role="row">
            <Folder size={15} className="files-icon dir" />
            <NameInput initial={t.files.newFolderName} onDone={(v) => void mkdir(v ?? "")} />
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
              onMouseDown={(ev) => {
                const items = isSel ? picked : [e];
                onDragRows(ev, items.length === 1 ? e.name : t.files.items(items.length), () => void download(items));
              }}
              onContextMenu={(ev) => {
                ev.preventDefault();
                if (!isSel) selectOnly(e.name);
                setMenuAt({ x: ev.clientX, y: ev.clientY, entry: e });
              }}
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
                <IconButton icon={Download} label={localDir ? t.files.downloadTo(lastPart(localDir)) : t.files.download} onClick={(ev) => (ev.stopPropagation(), void download([e]))} />
                <Menu
                  trigger={(toggle, isOpen) => <IconButton icon={Ellipsis} label={t.files.more} className={isOpen ? "pressed" : ""} onClick={toggle} />}
                  items={rowMenu(e)}
                />
              </span>
            </div>
          );
        })}
        {sorted.length === 0 && !creating && (
          <div className="files-empty">
            <span>{t.files.empty}</span>
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
          <span>{t.files.dropTo(where ?? "/")}</span>
        </div>
      )}

      {menuAt && <ContextMenu at={menuAt} items={rowMenu(menuAt.entry)} onClose={closeMenu} />}
      {choosing && <EditorChooser onPick={(ed) => void edit(choosing, ed)} onClose={() => setChoosing(null)} />}
      {removing && (
        <ConfirmDialog
          title={removing.length === 1 ? t.files.confirm.removeOne(removing[0]!.name) : t.files.confirm.removeMany(removing.length)}
          body={removing.some((e) => e.kind === "dir") ? t.files.confirm.withContents : undefined}
          confirm={t.common.delete}
          onConfirm={() => void remove(removing)}
          onClose={() => {
            setRemoving(null);
            focusList();
          }}
        />
      )}
      {replacing && (
        <ConfirmDialog
          title={replacing.existing.length === 1 ? t.files.confirm.replaceOne(replacing.existing[0]!) : t.files.confirm.replaceMany(replacing.existing.length)}
          confirm={t.files.confirm.replace}
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


/** Unix permission bits as checkboxes and as a number, kept in step. */
function PermissionsDialog({ entry, onClose, onSave }: { entry: FileEntry; onClose: () => void; onSave: (mode: number) => void }) {
  const t = useT();
  const p = t.files.perms;
  const WHO = [
    { label: p.owner, shift: 6 },
    { label: p.group, shift: 3 },
    { label: p.others, shift: 0 },
  ];
  const WHAT = [
    { label: p.read, bit: 4 },
    { label: p.write, bit: 2 },
    { label: p.execute, bit: 1 },
  ];
  const [mode, setMode] = useState(entry.permissions ?? (entry.kind === "dir" ? 0o755 : 0o644));
  const [octal, setOctal] = useState((mode & 0o777).toString(8).padStart(3, "0"));
  const set = (m: number) => {
    setMode(m);
    setOctal((m & 0o777).toString(8).padStart(3, "0"));
  };
  return (
    <Modal
      title={p.title(entry.name)}
      width={420}
      onClose={onClose}
      onSubmit={() => {
        onSave(mode);
        onClose();
      }}
      footer={
        <>
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary">{t.common.save}</Button>
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
      <Field label={p.octal}>
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
export function NameInput({ initial, onDone }: { initial: string; onDone: (v: string | null) => void }) {
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
  const x = useT().files;
  const pct = t.total ? Math.round((t.done / t.total) * 100) : 0;
  const secs = (Date.now() - t.started) / 1000;
  const speed = secs >= 1 && t.done ? ` · ${formatBytes(t.done / secs)}${x.perSecond}` : "";
  return (
    <div className={`transfer ${t.state}`}>
      {t.state === "done" ? <Check size={14} className="ok-text" /> : t.up ? <ArrowUp size={14} /> : <ArrowDown size={14} />}
      <span className="transfer-name">{t.name}</span>
      {t.state === "active" && (
        <>
          <span className="transfer-bar"><span style={{ width: `${pct}%` }} /></span>
          <span className="transfer-amount">{t.total ? `${x.transfer.amount(formatBytes(t.done), formatBytes(t.total))}${speed}` : ""}</span>
        </>
      )}
      {t.state === "done" && <span className="transfer-amount ok-text">{t.up ? x.transfer.uploaded : x.transfer.downloaded}</span>}
      {t.state === "error" && <span className="transfer-amount err-text">{t.error}</span>}
      <IconButton icon={X} label={t.state === "active" ? x.transfer.cancel : x.transfer.dismiss} tipSide="left" onClick={t.state === "active" ? onCancel : onDismiss} />
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
  const t = useT();
  const k = t.terminal.hostKey;
  const { toast, ws } = useStore();
  // A key on the way belongs to the jump host, not to this one.
  const keyHost = (phase.kind === "trust" || phase.kind === "changed") && phase.via ? ws.hosts.find((h) => h.id === phase.via) : undefined;
  const keyWhere = keyHost ? `${keyHost.label} · ${keyHost.address}:${keyHost.port}` : `${host.address}:${host.port}`;
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
          <Button variant="ghost" onClick={onClose}>{t.common.close}</Button>
          <Button variant="primary" icon={RotateCw} autoFocus onClick={() => (phase.kind === "ready" ? retryList() : connect())}>{t.common.retry}</Button>
        </div>
      </div>
    );
  }

  if (phase.kind === "secret") {
    const label = phase.secret === "passphrase" ? t.files.gate.passphrase : t.files.gate.password(`${host.user}@${host.address}`);
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
        <Checkbox checked={remember} onChange={setRemember}>{t.files.gate.remember}</Checkbox>
        <div className="hostkey-actions">
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" disabled={!value}>{t.common.connect}</Button>
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
          connect({ trust: trustMore(phase.trusted, phase.fingerprint) });
        }}
      >
        <div className="hostkey-head">
          <ShieldQuestion size={20} />
          <h2>{k.unknown}</h2>
        </div>
        <div className="hostkey-host mono">{keyWhere}</div>
        <dl className="hostkey-fp">
          <dt>{k.fingerprint}</dt>
          <dd className="mono">{phase.fingerprint}</dd>
        </dl>
        <div className="hostkey-actions">
          <Button type="button" variant="ghost" onClick={onClose}>{t.common.cancel}</Button>
          <Button type="submit" variant="primary" autoFocus>{k.trust}</Button>
        </div>
      </form>
    );
  }

  return (
    <div className="hostkey-card danger">
      <div className="hostkey-head">
        <ShieldAlert size={20} />
        <h2>{k.changed}</h2>
      </div>
      <div className="hostkey-host mono">{keyWhere}</div>
      <dl className="hostkey-fp">
        <dt>{k.was}</dt>
        <dd className="mono">{phase.expected}</dd>
        <dt>{k.now}</dt>
        <dd className="mono">{phase.actual}</dd>
      </dl>
      <div className="hostkey-actions">
        <Button
          variant="ghost"
          className="danger-text"
          onClick={async () => {
            try {
              await api.hostForgetKey(host.vaultId, phase.via ?? host.id);
              connect({ trust: trustMore(phase.trusted, phase.actual) });
            } catch (e) {
              toast({ text: errorText(e), tone: "error" });
            }
          }}
        >
          {k.replace}
        </Button>
        <Button variant="primary" autoFocus onClick={onClose}>{k.stop}</Button>
      </div>
    </div>
  );
}
