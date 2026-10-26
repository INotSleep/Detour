// Simulated SSH session: a tiny line-editing shell on top of xterm.js.
// Replaced by a real russh channel from detour-core later.

import { tr } from "../i18n";
import type { Terminal } from "@xterm/xterm";
import type { Host } from "./types";
import { highlightedOutput } from "./highlight";

export type SessionStatus = "connecting" | "connected" | "failed" | "closed";

const C = {
  reset: "\x1b[0m",
  dim: "\x1b[2m",
  green: "\x1b[1;32m",
  blue: "\x1b[1;34m",
  yellow: "\x1b[33m",
  red: "\x1b[31m",
};

const osPretty: Record<NonNullable<Host["os"]>, string> = {
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
  highlight: () => boolean;
}

export function runFakeSession(term: Terminal, host: Host, { initialCommand, secret, onStatus, onSecret, onExit, highlight }: Options): () => void {
  let disposed = false;
  let mode: "busy" | "ready" | "failed" | "secret" = "busy";
  let line = "";
  const history: string[] = [];
  let historyIndex = 0;

  const output = highlightedOutput(term, highlight);
  const write = (s: string) => output.write(s);
  // Plain, as bash prints it for root on a fresh server.
  const prompt = () => write(`\r\n${host.user}@${host.label}:~${host.user === "root" ? "#" : "$"} `);
  const println = (s = "") => write(`\r\n${s}`);

  const replaceLine = (next: string) => {
    write("\b \b".repeat(line.length));
    line = next;
    write(line);
  };

  const commands: Record<string, (args: string[]) => void> = {
    help: () => println("help ls whoami hostname uname uptime date df free ip systemctl docker tail echo clear exit"),
    ls: () => println(`${C.blue}deploy${C.reset}  ${C.blue}logs${C.reset}  README.md  docker-compose.yml`),
    whoami: () => println(host.user),
    hostname: () => println(host.label),
    uname: (args) => println(args.includes("-a") ? `Linux ${host.label} 6.17.0-12-generic #12 SMP PREEMPT_DYNAMIC x86_64 GNU/Linux` : "Linux"),
    uptime: () => println(" 12:41:07 up 23 days,  4:12,  1 user,  load average: 0.08, 0.12, 0.09"),
    date: () => println(new Date().toString()),
    df: () => {
      println("Filesystem      Size  Used Avail Use% Mounted on");
      println("/dev/vda1        77G   56G   21G  73% /");
      println("tmpfs           1.9G     0  1.9G   0% /dev/shm");
      println("/dev/vda15      105M  6.1M   99M   6% /boot/efi");
      println("/dev/sdb1       916G  842G   28G  97% /mnt/backup");
    },
    free: () => {
      println("               total        used        free      shared  buff/cache   available");
      println("Mem:           3.8Gi       1.2Gi       412Mi        12Mi       2.3Gi       2.4Gi");
      println("Swap:          2.0Gi          0B       2.0Gi");
    },
    ip: () => {
      println("1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN group default qlen 1000");
      println("    inet 127.0.0.1/8 scope host lo");
      println("2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq_codel state UP group default qlen 1000");
      println("    link/ether 52:54:00:12:34:56 brd ff:ff:ff:ff:ff:ff");
      println("    inet 10.0.0.5/24 metric 100 brd 10.0.0.255 scope global dynamic eth0");
      println("    inet6 fe80::5054:ff:fe12:3456/64 scope link");
    },
    systemctl: () => {
      println("nginx.service - A high performance web server and a reverse proxy server");
      println("     Loaded: loaded (/usr/lib/systemd/system/nginx.service; enabled; preset: enabled)");
      println("     Active: active (running) since Mon 2026-10-26 13:51:33 UTC; 2h 5min ago");
      println("   Main PID: 1234 (nginx)");
      println("     Memory: 4.2M (peak: 6.0M)");
      println();
      println("Oct 26 13:51:33 web-01 systemd[1]: Starting nginx.service...");
      println('Oct 26 13:51:33 web-01 nginx[1231]: nginx: [warn] conflicting server name "_" on 0.0.0.0:80, ignored');
      println("Oct 26 13:51:33 web-01 systemd[1]: Started nginx.service.");
    },
    docker: () => {
      println("CONTAINER ID   IMAGE         STATUS                    PORTS                  NAMES");
      println("3f2a1b9c8d7e   nginx:1.27    Up 3 days (healthy)       0.0.0.0:80->80/tcp     web");
      println("9c8d7e3f2a1b   redis:8       Up 3 days                 127.0.0.1:6379->6379   cache");
      println("a1b2c3d4e5f6   postgres:17   Exited (1) 2 hours ago                           db");
    },
    tail: () => {
      println('45.10.12.7 - - [26/Oct/2026:13:51:33 +0000] "GET /api/stats HTTP/1.1" 200 612 "-" "curl/8.5.0"');
      println('45.10.12.7 - - [26/Oct/2026:13:51:34 +0000] "GET /wp-login.php HTTP/1.1" 404 162 "-" "Mozilla/5.0"');
      println('10.0.0.9 - - [26/Oct/2026:13:51:35 +0000] "POST /upload HTTP/1.1" 502 157 "-" "-"');
      println("2026/10/26 13:51:35 [error] 1234#1234: *88 connect() failed (111: Connection refused) while connecting to upstream");
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
    write(`${C.dim}${host.user}@${host.address}:${host.port}${C.reset}`);
    await sleep(host.online !== false ? 700 : 1600);
    if (disposed) return;
    if (host.online === false) {
      println(`${C.red}${tr().demo.connectFailed(`${host.address}:${host.port}`)}${C.reset}`);
      println(`${C.dim}${tr().demo.enterRetry}${C.reset}`);
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
        println(secret!.kind === "password" ? "Permission denied, please try again." : `${tr().common.errors.wrongPassphrase}.`);
        println(secret!.prompt);
        return;
      }
      println(`${C.red}${host.user}@${host.address}: Permission denied${C.reset}`);
      println(`${C.dim}${tr().demo.enterRetry}${C.reset}`);
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
    println(`Welcome to ${osPretty[host.os ?? "ubuntu"]}`);
    println(`${C.dim}Last login: Thu Oct 22 18:04:51 2026 from 10.0.0.42${C.reset}`);
    mode = "ready";
    onStatus("connected");
    prompt();
    if (initialCommand) {
      await sleep(250);
      if (disposed) return;
      write(initialCommand);
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
          write("^C");
          println(`${C.dim}${tr().demo.enterRetry}${C.reset}`);
          mode = "failed";
          onStatus("failed");
          return;
        } else if (ch >= " ") secretBuf += ch;
      }
      return;
    }
    if (mode === "failed") {
      if (data === "\r") {
        write("\r\n\r\n");
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
          write("\b \b");
        }
      } else if (ch === "\x03") {
        line = "";
        write("^C");
        prompt();
      } else if (ch === "\x0c") {
        term.clear();
      } else if (ch >= " ") {
        line += ch;
        write(ch);
      }
    }
  });

  void connect();

  return () => {
    disposed = true;
    output.flush();
    sub.dispose();
  };
}
