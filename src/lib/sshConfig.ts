import type { Auth, Host, SshKey } from "./types";

/** A name ssh takes after `Host`: no spaces, commas or patterns. */
const alias = (label: string) => label.trim().replace(/[\s,*?!"]+/g, "-") || "host";

/** A file name for a key, the same on every system. */
export const keyFileName = (name: string) => name.trim().replace(/[\s\\/:*?"<>|]+/g, "-") || "key";

const arg = (s: string) => (/\s/.test(s) ? `"${s}"` : s);

/** Who logs in: a team host may have your own login instead of the shared one. */
function login(h: Host): { user: string; auth: Auth } {
  return h.own ? { user: h.own.user, auth: h.own.auth } : { user: h.user, auth: h.auth };
}

/** The keys these hosts log in with. */
export function keysOf(hosts: Host[], keys: SshKey[]): SshKey[] {
  const ids = new Set(hosts.map((h) => login(h).auth).flatMap((a) => (a.type === "key" ? [a.keyId] : [])));
  return keys.filter((k) => ids.has(k.id));
}

/** The hosts with the jump hosts they go through, which their entries name. */
export function withJumps(hosts: Host[], all: Host[]): Host[] {
  const out = [...hosts];
  for (let i = 0; i < out.length; i++) {
    const p = out[i].proxy;
    const jump = p?.type === "jump" ? all.find((h) => h.id === p.hostId) : undefined;
    if (jump && !out.includes(jump)) out.push(jump);
  }
  return out;
}

/**
 * The hosts as `~/.ssh/config` entries. A key file is where `keyPaths` says (by key id),
 * otherwise `~/.ssh/<key name>`. Passwords can't go into an ssh config.
 */
export function sshConfig(hosts: Host[], all: Host[], keys: SshKey[], keyPaths: Record<string, string> = {}): string {
  // Hosts without a group first, then group by group, so a group's comment heads only its hosts.
  const list = withJumps(hosts, all).sort((a, b) => (a.group === b.group ? 0 : !a.group ? -1 : !b.group ? 1 : a.group.localeCompare(b.group)));
  const names = new Map<string, string>();
  const taken = new Set<string>();
  for (const h of list) {
    const base = alias(h.label);
    let name = base;
    for (let n = 2; taken.has(name.toLowerCase()); n++) name = `${base}-${n}`;
    taken.add(name.toLowerCase());
    names.set(h.id, name);
  }

  const blocks: string[] = [];
  let group: string | undefined;
  for (const h of list) {
    const lines: string[] = [];
    if (h.group && h.group !== group) lines.push(`# ${h.group}`);
    group = h.group;
    const { user, auth } = login(h);
    lines.push(`Host ${names.get(h.id)}`, `  HostName ${h.address}`, `  User ${user}`);
    if (h.port !== 22) lines.push(`  Port ${h.port}`);
    if (auth.type === "key") {
      const key = keys.find((k) => k.id === auth.keyId);
      if (key) lines.push(`  IdentityFile ${arg(keyPaths[key.id] ?? `~/.ssh/${keyFileName(key.name)}`)}`, "  IdentitiesOnly yes");
    }
    const p = h.proxy;
    if (p?.type === "jump") lines.push(`  ProxyJump ${names.get(p.hostId)}`);
    if (p?.type === "socks5") lines.push(`  ProxyCommand nc -X 5 -x ${p.address}:${p.port} %h %p`);
    if (p?.type === "http") lines.push(`  ProxyCommand nc -X connect -x ${p.address}:${p.port}${p.user ? ` -P ${p.user}` : ""} %h %p`);
    if (h.forwardAgent) lines.push("  ForwardAgent yes");
    blocks.push(lines.join("\n"));
  }
  return blocks.join("\n\n") + "\n";
}

/** One host as an `ssh` command. */
export function sshCommand(h: Host, all: Host[]): string {
  const { user } = login(h);
  const parts = ["ssh"];
  if (h.forwardAgent) parts.push("-A");
  // -J a,b: the first hop first.
  const hops: string[] = [];
  for (let at = h, n = 0; at.proxy?.type === "jump" && n < 8; n++) {
    const id = at.proxy.hostId;
    const jump = all.find((x) => x.id === id);
    if (!jump) break;
    hops.unshift(`${login(jump).user}@${jump.address}${jump.port !== 22 ? `:${jump.port}` : ""}`);
    at = jump;
  }
  if (hops.length) parts.push("-J", hops.join(","));
  if (h.port !== 22) parts.push("-p", String(h.port));
  parts.push(`${user}@${h.address}`);
  return parts.join(" ");
}
