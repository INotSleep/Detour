import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { ShieldAlert, ShieldQuestion } from "lucide-react";
import type { Host } from "../lib/types";
import { runFakeSession, type SessionSecret, type SessionStatus } from "../lib/fakeShell";
import { runRealSession, type HostKeyAnswer, type HostKeyQuestion } from "../lib/realSession";
import { api, errorText, type SecretKind } from "../lib/api";
import { mockSecrets } from "../lib/mockData";
import { useStore } from "../lib/store";
import { Button } from "./ui";

interface Props {
  host: Host;
  active: boolean;
  initialCommand?: string;
  onStatus: (s: SessionStatus) => void;
  onExit: () => void;
}

type Question = HostKeyQuestion & { answer: (a: HostKeyAnswer) => void };

export function TerminalView({ host, active, initialCommand, onStatus, onExit }: Props) {
  const { settings, ws, update, toast } = useStore();
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const [question, setQuestion] = useState<Question | null>(null);
  const cb = useRef({ onStatus, onExit });
  cb.current = { onStatus, onExit };

  useEffect(() => {
    const term = new Terminal({
      fontFamily: '"JetBrains Mono", "Cascadia Mono", "SF Mono", Menlo, Consolas, monospace',
      fontSize: settings.fontSize,
      lineHeight: 1.25,
      cursorBlink: true,
      scrollback: settings.scrollback,
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
    term.loadAddon(fit);
    term.open(hostRef.current!);
    fit.fit();
    termRef.current = term;
    fitRef.current = fit;

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
              update((w) =>
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

  return (
    <div className="terminal" onMouseDown={() => termRef.current?.focus()}>
      <div className="terminal-inner" ref={hostRef} />
      {question && <HostKeyCard host={host} question={question} />}
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
        <div className="hostkey-host mono">{host.address}:{host.port}</div>
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
