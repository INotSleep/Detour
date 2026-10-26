// A real SSH session from detour-core, driven from an xterm.js terminal.
// Prompts for passwords / passphrases happen inside the terminal like ssh does;
// host key decisions are delegated to the UI (an overlay with buttons).

import type { Terminal } from "@xterm/xterm";
import { api, errorText, isCoreError, trustMore, type SecretKind } from "./api";
import type { SessionStatus } from "./fakeShell";
import type { Host } from "./types";
import { highlightedOutput } from "./highlight";
import { tr } from "../i18n";

const C = { reset: "\x1b[0m", dim: "\x1b[2m", red: "\x1b[31m" };

/** `via`: the id of the host on the way whose key it is. */
export type HostKeyQuestion = ({ kind: "unknown"; fingerprint: string } | { kind: "changed"; expected: string; actual: string }) & { via?: string };
/** `trust` for an unknown key, `replace` for a changed one, `cancel` otherwise. */
export type HostKeyAnswer = "trust" | "replace" | "cancel";

interface Options {
  initialCommand?: string;
  onStatus: (s: SessionStatus) => void;
  onExit: () => void;
  /** A secret the user typed in worked; the UI may offer to save it. */
  onSecret: (kind: SecretKind, value: string) => void;
  askHostKey: (q: HostKeyQuestion) => Promise<HostKeyAnswer>;
  /** Highlight the output (read on every write, so it follows the setting). */
  highlight: () => boolean;
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
  const output = highlightedOutput(term, opts.highlight);

  const fail = (message: string) => {
    term.write(`${C.red}${message}${C.reset}\r\n${C.dim}${tr().terminal.session.retry}${C.reset}\r\n`);
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
          onData: (bytes) => output.write(bytes),
          onClosed: (exitStatus) => {
            sessionId = null;
            if (disposed) return;
            output.flush();
            if (exitStatus === 0) {
              opts.onStatus("closed");
              opts.onExit();
              return;
            }
            term.write(`\r\n${C.dim}${tr().terminal.session.closed}${C.reset}\r\n`);
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
        void send(`${opts.initialCommand}\r`);
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
        const answer = await opts.askHostKey({ kind: "unknown", fingerprint: e.fingerprint, via: e.via });
        if (disposed) return;
        if (answer !== "trust") return fail(tr().terminal.session.cancelled);
        trust = trustMore(trust, e.fingerprint);
        return open();
      }
      case "hostKeyChanged": {
        const answer = await opts.askHostKey({ kind: "changed", expected: e.expected, actual: e.actual, via: e.via });
        if (disposed) return;
        if (answer !== "replace") return fail(tr().terminal.session.cancelledKeyChanged);
        await api.hostForgetKey(host.vaultId, e.via ?? host.id);
        // The user just saw the new key: trust it without asking again.
        trust = trustMore(trust, e.actual);
        return open();
      }
      case "needSecret":
        return askSecret(e.secret, e.prompt);
      case "authFailed":
        if (e.secret) {
          attemptsLeft -= 1;
          secret = null;
          if (attemptsLeft > 0) {
            term.write(e.secret === "password" ? "Permission denied, please try again.\r\n" : `${tr().common.errors.wrongPassphrase}.\r\n`);
            return askSecret(e.secret, secretPrompt || (e.secret === "password" ? `${host.user}@${host.address}'s password: ` : tr().terminal.session.passphrasePrompt));
          }
          attemptsLeft = MAX_SECRET_ATTEMPTS;
          return fail(`${host.user}@${host.address}: Permission denied`);
        }
        return fail(tr().common.errors.keyRejected);
      default:
        return fail(errorText(e));
    }
  };

  // Keystrokes go out one call at a time, in order: separate calls could overtake each other.
  // What is typed while a call is on its way goes with the next one.
  let pending = "";
  let sending = false;
  async function send(d: string) {
    pending += d;
    if (sending) return;
    sending = true;
    while (pending && sessionId) {
      const chunk = pending;
      pending = "";
      await api.sessionInput(sessionId, chunk).catch(() => {});
    }
    pending = "";
    sending = false;
  }

  const data = term.onData((d) => {
    if (mode === "live" && sessionId) {
      void send(d);
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
          return fail(tr().terminal.session.cancelled);
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
    output.flush();
    data.dispose();
    resize.dispose();
    if (sessionId) void api.sessionClose(sessionId);
  };
}
