// A real SSH session from detour-core, driven from an xterm.js terminal.
// Prompts for passwords / passphrases happen inside the terminal like ssh does;
// host key decisions are delegated to the UI (an overlay with buttons).

import type { Terminal } from "@xterm/xterm";
import { api, errorText, isCoreError, type SecretKind } from "./api";
import type { SessionStatus } from "./fakeShell";
import type { Host } from "./types";

const C = { reset: "\x1b[0m", dim: "\x1b[2m", red: "\x1b[31m" };

export type HostKeyQuestion = { kind: "unknown"; fingerprint: string } | { kind: "changed"; expected: string; actual: string };
/** `trust` for an unknown key, `replace` for a changed one, `cancel` otherwise. */
export type HostKeyAnswer = "trust" | "replace" | "cancel";

interface Options {
  initialCommand?: string;
  onStatus: (s: SessionStatus) => void;
  onExit: () => void;
  /** A secret the user typed in worked; the UI may offer to save it. */
  onSecret: (kind: SecretKind, value: string) => void;
  askHostKey: (q: HostKeyQuestion) => Promise<HostKeyAnswer>;
}

const MAX_SECRET_ATTEMPTS = 3;

export function runRealSession(term: Terminal, host: Host, opts: Options): () => void {
  let disposed = false;
  let mode: "busy" | "live" | "secret" | "failed" = "busy";
  let sessionId: string | null = null;
  let secret: string | null = null;
  let typed: { kind: SecretKind; value: string } | null = null;
  let trust: string | null = null;
  let secretKind: SecretKind = "password";
  let secretPrompt = "";
  let secretBuf = "";
  let attemptsLeft = MAX_SECRET_ATTEMPTS;
  let commandSent = false;

  const fail = (message: string) => {
    term.write(`${C.red}${message}${C.reset}\r\n${C.dim}Enter: повторить${C.reset}\r\n`);
    mode = "failed";
    opts.onStatus("failed");
  };

  const askSecret = (kind: SecretKind, prompt: string) => {
    secretKind = kind;
    secretPrompt = prompt;
    secretBuf = "";
    mode = "secret";
    term.write(prompt);
  };

  const open = async () => {
    mode = "busy";
    opts.onStatus("connecting");
    try {
      const opened = await api.sessionOpen(
        { vaultId: host.vaultId, hostId: host.id, secret, trustFingerprint: trust, cols: term.cols, rows: term.rows },
        {
          onData: (bytes) => term.write(bytes),
          onClosed: (exitStatus) => {
            sessionId = null;
            if (disposed) return;
            if (exitStatus === 0) {
              opts.onStatus("closed");
              opts.onExit();
              return;
            }
            term.write(`\r\n${C.dim}Соединение закрыто. Enter: переподключиться${C.reset}\r\n`);
            mode = "failed";
            opts.onStatus("closed");
          },
        },
      );
      if (disposed) {
        void api.sessionClose(opened.sessionId);
        return;
      }
      sessionId = opened.sessionId;
      mode = "live";
      attemptsLeft = MAX_SECRET_ATTEMPTS;
      opts.onStatus("connected");
      if (typed) {
        opts.onSecret(typed.kind, typed.value);
        typed = null;
      }
      if (opts.initialCommand && !commandSent) {
        commandSent = true;
        void api.sessionInput(sessionId, `${opts.initialCommand}\r`);
      }
    } catch (e) {
      if (disposed) return;
      await handleError(e);
    }
  };

  const handleError = async (e: unknown) => {
    if (!isCoreError(e)) return fail(errorText(e));
    switch (e.kind) {
      case "hostKeyUnknown": {
        const answer = await opts.askHostKey({ kind: "unknown", fingerprint: e.fingerprint });
        if (disposed) return;
        if (answer !== "trust") return fail("Подключение отменено");
        trust = e.fingerprint;
        return open();
      }
      case "hostKeyChanged": {
        const answer = await opts.askHostKey({ kind: "changed", expected: e.expected, actual: e.actual });
        if (disposed) return;
        if (answer !== "replace") return fail("Подключение отменено: ключ хоста изменился");
        await api.hostForgetKey(host.vaultId, host.id);
        trust = null;
        return open();
      }
      case "needSecret":
        return askSecret(e.secret, e.prompt);
      case "authFailed":
        if (e.secret) {
          attemptsLeft -= 1;
          secret = null;
          if (attemptsLeft > 0) {
            term.write(e.secret === "password" ? "Permission denied, please try again.\r\n" : "Неверный пароль ключа.\r\n");
            return askSecret(e.secret, secretPrompt || (e.secret === "password" ? `${host.user}@${host.address}'s password: ` : "Пароль ключа: "));
          }
          attemptsLeft = MAX_SECRET_ATTEMPTS;
          return fail(`${host.user}@${host.address}: Permission denied`);
        }
        return fail("Сервер не принял ключ");
      default:
        return fail(errorText(e));
    }
  };

  const data = term.onData((d) => {
    if (mode === "live" && sessionId) {
      void api.sessionInput(sessionId, d);
      return;
    }
    if (mode === "secret") {
      for (const ch of d) {
        if (ch === "\r") {
          term.write("\r\n");
          secret = secretBuf;
          typed = { kind: secretKind, value: secretBuf };
          secretBuf = "";
          void open();
          return;
        }
        if (ch === "\x7f") secretBuf = secretBuf.slice(0, -1);
        else if (ch === "\x03") {
          term.write("^C\r\n");
          secretBuf = "";
          return fail("Подключение отменено");
        } else if (ch >= " ") secretBuf += ch;
      }
      return;
    }
    if (mode === "failed" && d === "\r") {
      term.write("\r\n");
      void open();
    }
  });

  const resize = term.onResize(({ cols, rows }) => {
    if (mode === "live" && sessionId) void api.sessionResize(sessionId, cols, rows);
  });

  term.write(`${C.dim}${host.user}@${host.address}:${host.port}${C.reset}\r\n`);
  void open();

  return () => {
    disposed = true;
    data.dispose();
    resize.dispose();
    if (sessionId) void api.sessionClose(sessionId);
  };
}
