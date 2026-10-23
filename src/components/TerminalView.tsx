import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import "@xterm/xterm/css/xterm.css";
import { ChevronDown, ChevronUp, ClipboardPaste, Columns2, Copy, Rows2, Search, ShieldAlert, ShieldQuestion, X } from "lucide-react";
import type { Host } from "../lib/types";
import { runFakeSession, type SessionSecret, type SessionStatus } from "../lib/fakeShell";
import { runRealSession, type HostKeyAnswer, type HostKeyQuestion } from "../lib/realSession";
import { api, errorText, type SecretKind } from "../lib/api";
import { mockSecrets } from "../lib/mockData";
import { useStore } from "../lib/store";
import { Button, IconButton } from "./ui";

export type SplitDir = "row" | "col";

interface Props {
  host: Host;
  active: boolean;
  initialCommand?: string;
  onStatus: (s: SessionStatus) => void;
  onExit: () => void;
  /** Splits the tab, opening another session to this host beside or below. */
  onSplit?: (dir: SplitDir) => void;
  /** The ways this pane may split: a tab already split one way only adds panes that way. */
  splitDirs?: SplitDir[];
  /** Keys typed here go to this pane. */
  onFocus?: () => void;
}

type Question = HostKeyQuestion & { answer: (a: HostKeyAnswer) => void };

const SEARCH_COLORS = {
  matchBackground: "#7a8cff55",
  activeMatchBackground: "#e5a54b",
  matchOverviewRuler: "#7a8cff",
  activeMatchColorOverviewRuler: "#e5a54b",
};

export function TerminalView({ host, active, initialCommand, onStatus, onExit, onSplit, splitDirs = ["row", "col"], onFocus }: Props) {
  const { settings, ws, update, toast, applyTeamView } = useStore();
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const searchRef = useRef<SearchAddon | null>(null);
  const [question, setQuestion] = useState<Question | null>(null);
  const [searching, setSearching] = useState(false);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const cb = useRef({ onStatus, onExit, onSplit, splitDirs, clipboard: settings.clipboard });
  cb.current = { onStatus, onExit, onSplit, splitDirs, clipboard: settings.clipboard };

  const copySelection = async () => {
    const term = termRef.current;
    const text = term?.getSelection();
    if (!term || !text) return false;
    await api.clipboardWrite(text);
    term.clearSelection();
    return true;
  };

  const paste = async () => {
    const text = await api.clipboardRead();
    if (text) termRef.current?.paste(text);
    termRef.current?.focus();
  };

  useEffect(() => {
    const term = new Terminal({
      fontFamily: '"JetBrains Mono", "Cascadia Mono", "SF Mono", Menlo, Consolas, monospace',
      fontSize: settings.fontSize,
      lineHeight: 1.25,
      cursorBlink: true,
      scrollback: settings.scrollback,
      // Search highlights use decorations.
      allowProposedApi: true,
      theme: {
        background: "#0b0c0e",
        foreground: "#d9dbe1",
        cursor: "#8d9cff",
        cursorAccent: "#0b0c0e",
        selectionBackground: "rgba(122, 140, 255, 0.32)",
        black: "#1b1d22",
        brightBlack: "#5d616c",
        red: "#ef6461",
        green: "#4cc38a",
        brightGreen: "#6fd8a4",
        yellow: "#e5a54b",
        blue: "#7a8cff",
        brightBlue: "#9eabff",
      },
    });
    const fit = new FitAddon();
    const search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.open(hostRef.current!);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;
    searchRef.current = search;

    // Ctrl+Shift+C and Ctrl+Shift+V copy and paste either way. As in a system terminal, Ctrl+C
    // also copies a selection (and interrupts without one) and Ctrl+V pastes; as in Termius,
    // those two go to the server.
    term.attachCustomKeyEventHandler((e) => {
      if (e.type !== "keydown" || !(e.ctrlKey || e.metaKey) || e.altKey) return true;
      const key = e.key.toLowerCase();
      const ssh = cb.current.clipboard === "ssh";
      if (key === "c" && (e.shiftKey || (ssh && term.hasSelection()))) {
        e.preventDefault();
        void copySelection();
        return false;
      }
      if (key === "v" && (e.shiftKey || ssh)) {
        // Stop the browser's own paste, or the text would arrive twice.
        e.preventDefault();
        void paste();
        return false;
      }
      if (e.shiftKey && key === "f") {
        e.preventDefault();
        setSearching(true);
        return false;
      }
      const splitDir: SplitDir = key === "d" ? "row" : "col";
      if (e.shiftKey && (key === "d" || key === "e") && cb.current.onSplit && cb.current.splitDirs.includes(splitDir)) {
        e.preventDefault();
        cb.current.onSplit(splitDir);
        return false;
      }
      return true;
    });

    const key = host.auth.type === "key" ? ws.keys.find((k) => k.id === (host.auth as { keyId: string }).keyId) : undefined;

    // Offer to keep a secret the user typed in, once it proved correct.
    const offerSave = (kind: SecretKind, value: string) =>
      toast({
        text: kind === "password" ? `Сохранить пароль для «${host.label}»?` : `Сохранить пароль ключа «${key?.name}»?`,
        duration: 15000,
        action: {
          label: "Сохранить",
          run: async () => {
            try {
              await api.secretSave(host, kind, value);
              // In a team the secret may have gone to your own login rather than the shared one: ask the core.
              if (host.access !== undefined && api.real) applyTeamView(await api.teamSync(host.vaultId));
              else update((w) =>
                kind === "password"
                  ? { ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, hasPassword: true } : h)) }
                  : { ...w, keys: w.keys.map((k) => (k.id === key?.id ? { ...k, passphraseSaved: true } : k)) },
              );
              toast({ text: "Сохранено в vault", tone: "success" });
              termRef.current?.focus();
            } catch (e) {
              toast({ text: errorText(e), tone: "error" });
            }
          },
        },
      });

    let stop: () => void;
    if (api.usesCore(host.vaultId)) {
      stop = runRealSession(term, host, {
        initialCommand,
        onStatus: (s) => {
          cb.current.onStatus(s);
          if (s === "connected") update((w) => ({ ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, lastConnected: Date.now() } : h)) }));
        },
        onExit: () => cb.current.onExit(),
        onSecret: offerSave,
        askHostKey: (q) => new Promise<HostKeyAnswer>((resolve) => setQuestion({ ...q, answer: (a) => { setQuestion(null); term.focus(); resolve(a); } })),
      });
    } else {
      const secret: SessionSecret | undefined =
        host.auth.type === "password"
          ? { kind: "password", prompt: `${host.user}@${host.address}'s password: `, saved: mockSecrets.hostPasswords.get(host.id) }
          : key?.hasPassphrase
            ? { kind: "passphrase", prompt: `Enter passphrase for key '${key.name}': `, saved: mockSecrets.keyPassphrases.get(key.id) }
            : undefined;
      stop = runFakeSession(term, host, {
        initialCommand,
        secret,
        onStatus: (s) => cb.current.onStatus(s),
        onExit: () => cb.current.onExit(),
        onSecret: (value) => secret && offerSave(secret.kind, value),
      });
    }

    const ro = new ResizeObserver(() => hostRef.current?.offsetParent && fit.fit());
    ro.observe(hostRef.current!);

    return () => {
      stop();
      ro.disconnect();
      term.dispose();
    };
    // A session is bound to the host it was opened with.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.fontSize = settings.fontSize;
    term.options.scrollback = settings.scrollback;
    fitRef.current?.fit();
  }, [settings.fontSize, settings.scrollback]);

  useEffect(() => {
    if (!active) return;
    requestAnimationFrame(() => {
      fitRef.current?.fit();
      termRef.current?.focus();
    });
  }, [active]);

  const onContextMenu = async (e: React.MouseEvent) => {
    e.preventDefault();
    if (cb.current.clipboard === "ssh") {
      // As in Windows Terminal and PuTTY: copy a selection, otherwise paste.
      if (!(await copySelection())) await paste();
      return;
    }
    setMenu({ x: e.clientX, y: e.clientY });
  };

  return (
    <div className="terminal" onMouseDown={() => { termRef.current?.focus(); onFocus?.(); }} onFocusCapture={onFocus} onContextMenu={(e) => void onContextMenu(e)}>
      <div className="terminal-inner" ref={hostRef} />
      {searching && searchRef.current && (
        <SearchBar
          search={searchRef.current}
          onClose={() => {
            searchRef.current?.clearDecorations();
            setSearching(false);
            termRef.current?.focus();
          }}
        />
      )}
      {menu && (
        <TerminalMenu
          at={menu}
          canCopy={!!termRef.current?.hasSelection()}
          splitDirs={onSplit ? splitDirs : []}
          shortcuts={settings.clipboard === "ssh" ? ["Ctrl C", "Ctrl V"] : ["Ctrl Shift C", "Ctrl Shift V"]}
          onClose={() => {
            setMenu(null);
            termRef.current?.focus();
          }}
          onCopy={() => void copySelection()}
          onPaste={() => void paste()}
          onSearch={() => setSearching(true)}
          onSplit={(dir) => onSplit?.(dir)}
        />
      )}
      {question && <HostKeyCard host={ws.hosts.find((h) => h.id === question.via) ?? host} question={question} />}
    </div>
  );
}

/** Find in the terminal's output: Enter for the next match, Shift+Enter for the previous one. */
function SearchBar({ search, onClose }: { search: SearchAddon; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<{ index: number; count: number } | null>(null);

  useEffect(() => {
    const sub = search.onDidChangeResults((r) => setResult(r.resultCount ? { index: r.resultIndex, count: r.resultCount } : null));
    return () => sub.dispose();
  }, [search]);

  const find = (forward: boolean, q = query) => {
    if (!q) {
      search.clearDecorations();
      setResult(null);
      return;
    }
    const opts = { decorations: SEARCH_COLORS };
    if (forward) search.findNext(q, opts);
    else search.findPrevious(q, opts);
  };

  return (
    <div className="term-search" onMouseDown={(e) => e.stopPropagation()}>
      <Search size={14} className="muted" />
      <input
        className="term-search-input"
        autoFocus
        value={query}
        placeholder="Найти"
        spellCheck={false}
        onChange={(e) => {
          setQuery(e.target.value);
          find(true, e.target.value);
        }}
        onKeyDown={(e) => {
          e.stopPropagation();
          if (e.key === "Escape") onClose();
          if (e.key === "Enter") find(!e.shiftKey);
        }}
      />
      <span className={`term-search-count ${query && !result ? "err-text" : ""}`}>{query ? (result ? `${result.index + 1} из ${result.count}` : "Нет") : ""}</span>
      <IconButton icon={ChevronUp} label="Предыдущее · Shift+Enter" disabled={!result} onClick={() => find(false)} />
      <IconButton icon={ChevronDown} label="Следующее · Enter" disabled={!result} onClick={() => find(true)} />
      <IconButton icon={X} label="Закрыть · Esc" onClick={onClose} />
    </div>
  );
}

function TerminalMenu({
  at,
  canCopy,
  splitDirs,
  shortcuts,
  onClose,
  onCopy,
  onPaste,
  onSearch,
  onSplit,
}: {
  at: { x: number; y: number };
  canCopy: boolean;
  splitDirs: SplitDir[];
  /** How copying and pasting are typed, to show beside them. */
  shortcuts: [string, string];
  onClose: () => void;
  onCopy: () => void;
  onPaste: () => void;
  onSearch: () => void;
  onSplit: (dir: SplitDir) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);
  useEffect(() => {
    const m = ref.current?.getBoundingClientRect();
    if (m) setPos({ x: Math.min(at.x, window.innerWidth - m.width - 8), y: Math.min(at.y, window.innerHeight - m.height - 8) });
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && onClose();
    const key = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", key);
    };
  }, [at, onClose]);

  const item = (label: string, Icon: typeof Copy, hint: string, run: () => void, disabled = false) => (
    <button
      role="menuitem"
      disabled={disabled}
      onClick={() => {
        onClose();
        run();
      }}
    >
      <Icon size={14} />
      <span>{label}</span>
      <span className="menu-hint">{hint}</span>
    </button>
  );

  return (
    <div ref={ref} className="menu term-menu" role="menu" style={{ top: pos.y, left: pos.x }} onMouseDown={(e) => e.stopPropagation()}>
      {item("Копировать", Copy, shortcuts[0], onCopy, !canCopy)}
      {item("Вставить", ClipboardPaste, shortcuts[1], onPaste)}
      {item("Найти", Search, "Ctrl Shift F", onSearch)}
      {splitDirs.includes("row") && item("Разделить вправо", Columns2, "Ctrl Shift D", () => onSplit("row"))}
      {splitDirs.includes("col") && item("Разделить вниз", Rows2, "Ctrl Shift E", () => onSplit("col"))}
    </div>
  );
}

function HostKeyCard({ host, question }: { host: Host; question: Question }) {
  const changed = question.kind === "changed";
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") question.answer("cancel");
      if (e.key === "Enter" && !changed) question.answer("trust");
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [question, changed]);

  return (
    <div className="hostkey-backdrop" onMouseDown={(e) => e.stopPropagation()}>
      <div className={`hostkey-card ${changed ? "danger" : ""}`} role="alertdialog" aria-label="Ключ хоста">
        <div className="hostkey-head">
          {changed ? <ShieldAlert size={20} /> : <ShieldQuestion size={20} />}
          <h2>{changed ? "Ключ хоста изменился" : "Новый хост"}</h2>
        </div>
        <div className="hostkey-host mono">{question.via ? `${host.label} · ` : ""}{host.address}:{host.port}</div>
        {question.kind === "unknown" ? (
          <dl className="hostkey-fp">
            <dt>Отпечаток</dt>
            <dd className="mono">{question.fingerprint}</dd>
          </dl>
        ) : (
          <dl className="hostkey-fp">
            <dt>Был</dt>
            <dd className="mono">{question.expected}</dd>
            <dt>Сейчас</dt>
            <dd className="mono">{question.actual}</dd>
          </dl>
        )}
        <div className="hostkey-actions">
          {changed ? (
            <>
              <Button variant="ghost" className="danger-text" onClick={() => question.answer("replace")}>Заменить ключ</Button>
              <Button variant="primary" autoFocus onClick={() => question.answer("cancel")}>Отменить</Button>
            </>
          ) : (
            <>
              <Button variant="ghost" onClick={() => question.answer("cancel")}>Отмена</Button>
              <Button variant="primary" autoFocus onClick={() => question.answer("trust")}>Доверять и подключиться</Button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
