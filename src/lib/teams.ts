// Mapping detour-core team views into the UI workspace.

import { fromView, isCoreError, type PersonalData, type TeamEntry, type TeamView } from "./api";
import type { Level, Role, Vault, Workspace } from "./types";

export const canManage = (role?: Role | null) => role === "owner" || role === "admin";

const rank: Record<Level, number> = { none: 0, view: 1, secret: 2, edit: 3 };

/** Does an access level reach `min`? Personal records (no level) allow everything. */
export function atLeast(level: Level | undefined, min: Level) {
  return level === undefined || rank[level] >= rank[min];
}

export const maxLevel = (a: Level, b: Level) => (rank[a] >= rank[b] ? a : b);

/** May the user add records to this vault? */
export function writable(v: Vault) {
  return v.kind === "personal" || !!v.joined;
}

export function dataOf(ws: Workspace, vaultId: string): PersonalData {
  const mine = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId === vaultId);
  return { hosts: mine(ws.hosts), keys: mine(ws.keys), snippets: mine(ws.snippets), tunnels: mine(ws.tunnels) };
}

const folderName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;

function teamVault(view: TeamView): Vault {
  return {
    id: view.id,
    name: view.name || folderName(view.path),
    kind: "team",
    sync: { type: "folder", path: view.path },
    syncStatus: view.conflicts.length ? "conflict" : "synced",
    lastSync: Date.now(),
    conflicts: view.conflicts,
    joined: view.joined,
    removed: view.removed,
    myRole: view.myRole ?? undefined,
    me: view.members.find((m) => m.isYou)?.id,
    problems: view.problems,
  };
}

/**
 * Puts a team's state from detour-core into the workspace. With `withData`
 * false only membership, access, conflicts and activity are replaced, so edits
 * still on their way to the core are not overwritten.
 */
export function applyTeamView(ws: Workspace, view: TeamView, withData = true): Workspace {
  const id = view.id;
  const others = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId !== id);
  const exists = ws.vaults.some((v) => v.id === id);
  const vaults = exists ? ws.vaults.map((v) => (v.id === id ? teamVault(view) : v)) : [...ws.vaults, teamVault(view)];
  const me = view.members.find((m) => m.isYou)?.id;
  const mine = new Map(view.access.filter((a) => a.member === me).map((a) => [a.record, a.level]));
  const next: Workspace = {
    ...ws,
    vaults,
    members: [...others(ws.members), ...view.members.map((m) => ({ ...m, vaultId: id }))],
    activity: [...others(ws.activity), ...view.activity.map((a) => ({ ...a, vaultId: id }) as Workspace["activity"][number])],
    access: [...others(ws.access), ...view.access.map((a) => ({ ...a, vaultId: id }))],
  };
  if (!withData) return next;
  const data = fromView(view.data, id);
  const level = (record: string): Level => mine.get(record) ?? "none";
  const tunnelState = new Map(ws.tunnels.filter((t) => t.vaultId === id).map((t) => [t.id, t]));
  return {
    ...next,
    hosts: [...others(ws.hosts), ...data.hosts.map((h) => ({ ...h, access: level(h.id) }))],
    keys: [...others(ws.keys), ...data.keys.map((k) => ({ ...k, access: level(k.id) }))],
    snippets: [...others(ws.snippets), ...data.snippets.map((s) => ({ ...s, access: level(s.id) }))],
    tunnels: [...others(ws.tunnels), ...data.tunnels.map((t) => ({ ...t, access: level(t.id), state: tunnelState.get(t.id)?.state ?? "off", error: tunnelState.get(t.id)?.error }))],
  };
}

/** A joined team whose storage can't be read right now. */
export function markUnavailable(ws: Workspace, entry: TeamEntry): Workspace {
  const known = ws.vaults.find((v) => v.id === entry.id);
  const vault: Vault = {
    ...(known ?? { id: entry.id, name: folderName(entry.path), kind: "team" as const, conflicts: [] }),
    sync: { type: "folder", path: entry.path },
    syncStatus: "offline",
  };
  return { ...ws, vaults: known ? ws.vaults.map((v) => (v.id === entry.id ? vault : v)) : [...ws.vaults, vault] };
}

export function applyTeamEntries(ws: Workspace, entries: TeamEntry[]): Workspace {
  return entries.reduce((w, e) => (e.view ? applyTeamView(w, e.view) : markUnavailable(w, e)), ws);
}

export function removeVault(ws: Workspace, id: string): Workspace {
  const others = <T extends { vaultId: string }>(xs: T[]) => xs.filter((x) => x.vaultId !== id);
  return {
    vaults: ws.vaults.filter((v) => v.id !== id),
    hosts: others(ws.hosts),
    members: others(ws.members),
    keys: others(ws.keys),
    snippets: others(ws.snippets),
    tunnels: others(ws.tunnels),
    activity: others(ws.activity),
    access: others(ws.access),
  };
}

export function isUnavailable(e: unknown) {
  return isCoreError(e) && (e.kind === "unavailable" || e.kind === "io");
}
