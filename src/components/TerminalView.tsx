import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import type { Host } from "../lib/types";
import { runFakeSession, type SessionSecret, type SessionStatus } from "../lib/fakeShell";
import { useStore } from "../lib/store";

interface Props {
  host: Host;
  active: boolean;
  initialCommand?: string;
  onStatus: (s: SessionStatus) => void;
  onExit: () => void;
}

export function TerminalView({ host, active, initialCommand, onStatus, onExit }: Props) {
  const { settings, ws, update, toast } = useStore();
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
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
    const secret: SessionSecret | undefined =
      host.auth.type === "password"
        ? { kind: "password", prompt: `${host.user}@${host.address}'s password: `, saved: host.password }
        : key?.hasPassphrase
          ? { kind: "passphrase", prompt: `Enter passphrase for key '${key.name}': `, saved: key.passphrase }
          : undefined;

    const stop = runFakeSession(term, host, {
      initialCommand,
      secret,
      onStatus: (s) => cb.current.onStatus(s),
      onExit: () => cb.current.onExit(),
      onSecret: (value) =>
        toast({
          text: secret?.kind === "password" ? `Сохранить пароль для «${host.label}»?` : `Сохранить пароль ключа «${key?.name}»?`,
          duration: 15000,
          action: {
            label: "Сохранить",
            run: () => {
              update((w) =>
                secret?.kind === "password"
                  ? { ...w, hosts: w.hosts.map((h) => (h.id === host.id ? { ...h, password: value } : h)) }
                  : { ...w, keys: w.keys.map((k) => (k.id === key?.id ? { ...k, passphrase: value } : k)) },
              );
              toast({ text: "Сохранено в vault", tone: "success" });
            },
          },
        }),
    });
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
    </div>
  );
}
