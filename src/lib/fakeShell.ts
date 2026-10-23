// Simulated SSH session: a tiny line-editing shell on top of xterm.js.
// Replaced by a real russh channel from detour-core later.

import type { Terminal } from "@xterm/xterm";
import type { Host } from "./types";

export type SessionStatus = "connecting" | "connected" | "failed" | "closed";

const C = {
  reset: "\x1b[0m",
  dim: "\x1b[2m",
  green: "\x1b[1;32m",
  blue: "\x1b[1;34m",
  yellow: "\x1b[33m",
  red: "\x1b[31m",
};

const osPretty: Record<Host["os"], string> = {
  ubuntu: "Ubuntu 26.04 LTS",
  debian: "Debian GNU/Linux 13",
  fedora: "Fedora Linux 44",
  alpine: "Alpine Linux v3.23",
  freebsd: "FreeBSD 15.0-RELEASE",
  macos: "macOS 26",
  windows: "Windows Server 2025",
};

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** A password or key passphrase the login needs. `saved` comes from the vault; absent means ask. */
export interface SessionSecret {
  kind: "password" | "passphrase";
  prompt: string;
  saved?: string;
}

interface Options {
  initialCommand?: string;
  secret?: SessionSecret;
  onStatus: (s: SessionStatus) => void;
  /** Called with a secret the user typed in, after it worked. */
  onSecret?: (value: string) => void;
  onExit: () => void;
}

export function runFakeSession(term: Terminal, host: Host, { initialCommand, secret, onStatus, onSecret, onExit }: Options): () => void {
  let disposed = false;
  let mode: "busy" | "ready" | "failed" | "secret" = "busy";
  let line = "";
  const history: string[] = [];
  let historyIndex = 0;

  const prompt = () => term.write(`\r\n${C.green}${host.user}@${host.label}${C.reset}:${C.blue}~${C.reset}$ `);
  const println = (s = "") => term.write(`\r\n${s}`);

  const replaceLine = (next: string) => {
    term.write("\b \b".repeat(line.length));
    line = next;
    term.write(line);
  };

  const commands: Record<string, (args: string[]) => void> = {
    help: () => println("help ls whoami hostname uname uptime date df echo clear exit"),
    ls: () => println(`${C.blue}deploy${C.reset}  ${C.blue}logs${C.reset}  README.md  docker-compose.yml`),
    whoami: () => println(host.user),
    hostname: () => println(host.label),
    uname: (args) => println(args.includes("-a") ? `Linux ${host.label} 6.17.0-12-generic #12 SMP PREEMPT_DYNAMIC x86_64 GNU/Linux` : "Linux"),
    uptime: () => println(" 12:41:07 up 23 days,  4:12,  1 user,  load average: 0.08, 0.12, 0.09"),
    date: () => println(new Date().toString()),
    df: () => {
      println("Filesystem      Size  Used Avail Use% Mounted on");
      println("/dev/nvme0n1p2  468G  121G  324G  28% /");
      println("/dev/nvme0n1p1  1.1G  6.2M  1.1G   1% /boot/efi");
    },
    echo: (args) => println(args.join(" ")),
    clear: () => term.clear(),
    exit: () => {
      println("logout");
      mode = "busy";
      onStatus("closed");
      setTimeout(onExit, 400);
    },
  };

  const run = (input: string) => {
    const [cmd, ...args] = input.trim().split(/\s+/);
    if (!cmd) return;
    history.push(input);
    historyIndex = history.length;
    const handler = commands[cmd];
    if (handler) handler(args);
    else println(`${cmd}: command not found`);
  };

  const connect = async () => {
    mode = "busy";
    onStatus("connecting");
    term.write(`${C.dim}${host.user}@${host.address}:${host.port}${C.reset}`);
    await sleep(host.online ? 700 : 1600);
    if (disposed) return;
    if (!host.online) {
      println(`${C.red}Не удалось подключиться: ${host.address}:${host.port} не отвечает${C.reset}`);
      println(`${C.dim}Enter: повторить${C.reset}`);
      mode = "failed";
      onStatus("failed");
      return;
    }
    if (secret && secret.saved === undefined) {
      askSecret(3);
      return;
    }
    await login();
  };

  // Hidden input like ssh's own password / passphrase prompt.
  let secretBuf = "";
  let attemptsLeft = 0;
  const askSecret = (attempts: number) => {
    attemptsLeft = attempts;
    secretBuf = "";
    mode = "secret";
    onStatus("connecting");
    println(secret!.prompt);
  };
  const submitSecret = async () => {
    const value = secretBuf;
    secretBuf = "";
    if (!value) {
      attemptsLeft -= 1;
      if (attemptsLeft > 0) {
        println(secret!.kind === "password" ? "Permission denied, please try again." : "Неверный пароль ключа.");
        println(secret!.prompt);
        return;
      }
      println(`${C.red}${host.user}@${host.address}: Permission denied${C.reset}`);
      println(`${C.dim}Enter: повторить${C.reset}`);
      mode = "failed";
      onStatus("failed");
      return;
    }
    mode = "busy";
    await login();
    if (!disposed) onSecret?.(value);
  };

  const login = async () => {
    await sleep(300);
    if (disposed) return;
    println();
    println(`Welcome to ${osPretty[host.os]}`);
    println(`${C.dim}Last login: Thu Oct 22 18:04:51 2026 from 10.0.0.42${C.reset}`);
    mode = "ready";
    onStatus("connected");
    prompt();
    if (initialCommand) {
      await sleep(250);
      if (disposed) return;
      term.write(initialCommand);
      run(initialCommand);
      prompt();
    }
  };

  const sub = term.onData((data) => {
    if (mode === "secret") {
      for (const ch of data) {
        if (ch === "\r") void submitSecret();
        else if (ch === "\x7f") secretBuf = secretBuf.slice(0, -1);
        else if (ch === "\x03") {
          term.write("^C");
          println(`${C.dim}Enter: повторить${C.reset}`);
          mode = "failed";
          onStatus("failed");
          return;
        } else if (ch >= " ") secretBuf += ch;
      }
      return;
    }
    if (mode === "failed") {
      if (data === "\r") {
        term.write("\r\n\r\n");
        void connect();
      }
      return;
    }
    if (mode !== "ready") return;
    if (data === "\x1b[A" || data === "\x1b[B") {
      if (!history.length) return;
      historyIndex = Math.max(0, Math.min(history.length, historyIndex + (data === "\x1b[A" ? -1 : 1)));
      replaceLine(history[historyIndex] ?? "");
      return;
    }
    if (data.startsWith("\x1b")) return;
    for (const ch of data) {
      if (ch === "\r") {
        const input = line;
        line = "";
        run(input);
        if (mode === "ready") prompt();
      } else if (ch === "\x7f") {
        if (line.length) {
          line = line.slice(0, -1);
          term.write("\b \b");
        }
      } else if (ch === "\x03") {
        line = "";
        term.write("^C");
        prompt();
      } else if (ch === "\x0c") {
        term.clear();
      } else if (ch >= " ") {
        line += ch;
        term.write(ch);
      }
    }
  });

  void connect();

  return () => {
    disposed = true;
    sub.dispose();
  };
}
