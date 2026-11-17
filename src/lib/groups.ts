// Groups of hosts in their order, and moving hosts and groups around.

import { isNoGroup } from "../components/common";
import { uid } from "./store";
import type { Group, GroupPrefs, Host, Layout, MyGroup, Workspace } from "./types";

/** A group as the host list shows it: its record if it has one, its hosts in order. */
export interface ListedGroup {
  /** Tells groups apart, including a group of mine and a shared one of the same name. */
  key: string;
  /** The group's name; "" for hosts without one. */
  name: string;
  record?: Group;
  /** A group of this person's own grouping. */
  mine?: MyGroup;
  hosts: Host[];
}

export const sharedKey = (name: string) => `s:${name}`;
export const mineKey = (id: string) => `m:${id}`;

const nameOf = (h: Host) => (isNoGroup(h.group) ? "" : h.group);

/** Hosts by their order; those without one keep the order they come in, after the rest. */
export function sortHosts(hosts: Host[]): Host[] {
  return hosts
    .map((h, i) => ({ h, i }))
    .sort((a, b) => (a.h.order ?? Infinity) - (b.h.order ?? Infinity) || a.i - b.i)
    .map((x) => x.h);
}

/**
 * The groups of a vault in order: those with a record by their order (empty ones too),
 * then groups only hosts name, by name, then hosts without a group.
 */
export function listGroups(hosts: Host[], groups: Group[], withEmpty = true): ListedGroup[] {
  const byName = new Map<string, Host[]>();
  for (const h of hosts) byName.set(nameOf(h), [...(byName.get(nameOf(h)) ?? []), h]);
  const recorded = [...groups].sort((a, b) => a.order - b.order);
  const out: ListedGroup[] = recorded.filter((g) => withEmpty || byName.has(g.name)).map((g) => ({ key: sharedKey(g.name), name: g.name, record: g, hosts: sortHosts(byName.get(g.name) ?? []) }));
  const known = new Set(recorded.map((g) => g.name));
  const rest = [...byName.keys()].filter((n) => n && !known.has(n)).sort((a, b) => a.localeCompare(b));
  out.push(...rest.map((name) => ({ key: sharedKey(name), name, hosts: sortHosts(byName.get(name)!) })));
  if (byName.has("")) out.push({ key: sharedKey(""), name: "", hosts: sortHosts(byName.get("")!) });
  return out;
}

/** A number between two places (either may be missing). */
function between(a: number | undefined, b: number | undefined): number {
  if (a === undefined && b === undefined) return 1;
  if (a === undefined) return b! - 1;
  if (b === undefined) return a + 1;
  return (a + b) / 2;
}

/**
 * Moves a host into `group` before `before` (or last). Hosts of that group without a place get
 * one first, in the order they are shown, so the move lands where it was dropped.
 */
export function moveHost(ws: Workspace, vaultId: string, hostId: string, group: string, before: string | null): Workspace {
  const moving = ws.hosts.find((h) => h.id === hostId);
  if (!moving) return ws;
  const target = sortHosts(ws.hosts.filter((h) => h.vaultId === vaultId && h.id !== hostId && nameOf(h) === group));
  const places = new Map<string, number>();
  if (target.some((h) => h.order == null)) target.forEach((h, i) => places.set(h.id, i + 1));
  const placeOf = (h: Host) => places.get(h.id) ?? h.order ?? undefined;
  const at = before ? target.findIndex((h) => h.id === before) : target.length;
  const i = at < 0 ? target.length : at;
  const order = between(i > 0 ? placeOf(target[i - 1]!) : undefined, i < target.length ? placeOf(target[i]!) : undefined);
  return {
    ...ws,
    hosts: ws.hosts.map((h) => (h.id === hostId ? { ...h, group, order } : places.has(h.id) ? { ...h, order: places.get(h.id)! } : h)),
  };
}

/** Records for every group shown, so each has a place to keep. */
function withRecords(ws: Workspace, vaultId: string): Workspace {
  const listed = listGroups(ws.hosts.filter((h) => h.vaultId === vaultId), ws.groups.filter((g) => g.vaultId === vaultId)).filter((g) => g.name);
  const missing = listed.filter((g) => !g.record);
  if (!missing.length) return ws;
  const last = Math.max(0, ...ws.groups.filter((g) => g.vaultId === vaultId).map((g) => g.order));
  const added: Group[] = missing.map((g, i) => ({ id: uid("g"), vaultId, name: g.name, order: last + i + 1 }));
  return { ...ws, groups: [...ws.groups, ...added] };
}

/** Moves a group before `before` (or last). */
export function moveGroup(ws: Workspace, vaultId: string, name: string, before: string | null): Workspace {
  const w = withRecords(ws, vaultId);
  const list = w.groups.filter((g) => g.vaultId === vaultId && g.name !== name).sort((a, b) => a.order - b.order);
  const at = before ? list.findIndex((g) => g.name === before) : list.length;
  const i = at < 0 ? list.length : at;
  const order = between(list[i - 1]?.order, list[i]?.order);
  return { ...w, groups: w.groups.map((g) => (g.vaultId === vaultId && g.name === name ? { ...g, order } : g)) };
}

/** A new empty group, last. */
export function addGroup(ws: Workspace, vaultId: string, name: string): Workspace {
  const w = withRecords(ws, vaultId);
  const last = Math.max(0, ...w.groups.filter((g) => g.vaultId === vaultId).map((g) => g.order));
  return { ...w, groups: [...w.groups, { id: uid("g"), vaultId, name, order: last + 1 }] };
}

/** Sets the proxy of a group for everyone in its vault. */
export function setGroupProxy(ws: Workspace, vaultId: string, name: string, proxyId: string | null): Workspace {
  const w = ws.groups.some((g) => g.vaultId === vaultId && g.name === name) ? ws : addGroup(ws, vaultId, name);
  return { ...w, groups: w.groups.map((g) => (g.vaultId === vaultId && g.name === name ? { ...g, proxyId: proxyId ?? undefined } : g)) };
}

/** Renames a group and the group of its hosts. */
export function renameGroup(ws: Workspace, vaultId: string, from: string, to: string): Workspace {
  return {
    ...ws,
    groups: ws.groups.map((g) => (g.vaultId === vaultId && g.name === from ? { ...g, name: to } : g)),
    hosts: ws.hosts.map((h) => (h.vaultId === vaultId && nameOf(h) === from ? { ...h, group: to } : h)),
  };
}

/** Removes a group; its hosts go to no group. */
export function removeGroup(ws: Workspace, vaultId: string, name: string): Workspace {
  return {
    ...ws,
    groups: ws.groups.filter((g) => !(g.vaultId === vaultId && g.name === name)),
    hosts: ws.hosts.map((h) => (h.vaultId === vaultId && nameOf(h) === name ? { ...h, group: "" } : h)),
  };
}

/** What a host takes from its groups, as connecting does: the proxy and the login, and from which group. */
export interface FromGroups {
  proxy?: { vault: string; id: string; group: string };
  login?: { prefs: GroupPrefs; group: string };
}

/**
 * This person's settings of the group of mine a host is in come first, then theirs of its shared
 * group (each setting on its own), then the host's own; the shared group's proxy is the last resort.
 */
export function fromGroups(ws: Workspace, host: Host): FromGroups {
  const v = host.vaultId;
  const mineG = ws.layouts[v]?.groups.find((g) => g.hosts.includes(host.id));
  const minePrefs = mineG && ws.groupPrefs.find((p) => p.key === `${v}/mine:${mineG.id}`);
  const teamPrefs = nameOf(host) ? ws.groupPrefs.find((p) => p.key === `${v}/team:${host.group}`) : undefined;
  const levels = [
    ...(minePrefs ? [{ prefs: minePrefs, group: mineG!.name }] : []),
    ...(teamPrefs ? [{ prefs: teamPrefs, group: host.group }] : []),
  ];
  const out: FromGroups = {};
  const proxied = levels.find((l) => l.prefs.proxy);
  if (proxied) out.proxy = { ...proxied.prefs.proxy!, group: proxied.group };
  else if (!host.proxyId && !host.proxy) {
    const shared = ws.groups.find((g) => g.vaultId === v && g.name === host.group)?.proxyId;
    if (shared) out.proxy = { vault: v, id: shared, group: host.group };
  }
  const login = levels.find((l) => l.prefs.auth || l.prefs.user);
  if (login) out.login = login;
  return out;
}

/** The host as it logs in: user and auth from a group's login of this person's, else their own
 *  login while it is on, else the host's (what the core's `login_in` picks). */
export function asLoggedIn(ws: Workspace, host: Host): Host {
  const g = fromGroups(ws, host).login?.prefs;
  const own = host.own?.active ? host.own : undefined;
  return { ...host, user: g?.user || own?.user || host.user, auth: g?.auth ?? own?.auth ?? host.auth, own: undefined };
}

/** The user a host is logged in as. */
export function loginUser(ws: Workspace, host: Host): string {
  return asLoggedIn(ws, host).user;
}

/* ---------- this person's own grouping of a team's hosts ---------- */

/** Groups of mine in their order, then the hosts in none of them under their shared groups. */
export function listMine(hosts: Host[], layout: Layout, groups: Group[], withEmpty = true): ListedGroup[] {
  const byId = new Map(hosts.map((h) => [h.id, h]));
  const placed = new Set<string>();
  const out: ListedGroup[] = [];
  for (const g of layout.groups) {
    const list = g.hosts.map((id) => byId.get(id)).filter((h): h is Host => !!h);
    list.forEach((h) => placed.add(h.id));
    if (withEmpty || list.length) out.push({ key: mineKey(g.id), name: g.name, mine: g, hosts: list });
  }
  return [...out, ...listGroups(hosts.filter((h) => !placed.has(h.id)), groups, false)];
}

/** A grouping of mine that starts as the shared one. */
export function layoutFrom(hosts: Host[], groups: Group[]): Layout {
  return { groups: listGroups(hosts, groups, true).filter((g) => g.name).map((g) => ({ id: uid("m"), name: g.name, hosts: g.hosts.map((h) => h.id) })) };
}

/** Shared groups shown below mine become mine, so they can be ordered among them. */
function materialized(layout: Layout, listed: ListedGroup[]): Layout {
  const extra = listed.filter((g) => !g.mine && g.name).map((g) => ({ id: uid("m"), name: g.name, hosts: g.hosts.map((h) => h.id) }));
  return extra.length ? { groups: [...layout.groups, ...extra] } : layout;
}

/** Moves a host into a group of mine (or a shared one shown below, which becomes mine), before `before`. */
export function mineMoveHost(layout: Layout, listed: ListedGroup[], hostId: string, key: string, before: string | null): Layout {
  let groups: MyGroup[] = layout.groups.map((g) => ({ ...g, hosts: g.hosts.filter((id) => id !== hostId) }));
  let target = groups.find((g) => mineKey(g.id) === key);
  if (!target) {
    const shared = listed.find((g) => g.key === key);
    // Into the hosts without a group of mine: out of mine.
    if (!shared || !shared.name) return { groups };
    target = { id: uid("m"), name: shared.name, hosts: shared.hosts.map((h) => h.id).filter((id) => id !== hostId) };
    groups = [...groups, target];
  }
  const at = before ? target.hosts.indexOf(before) : -1;
  target.hosts.splice(at < 0 ? target.hosts.length : at, 0, hostId);
  return { groups };
}

/** Moves a group of mine (or a shared one shown below) before another, or last of mine. */
export function mineMoveGroup(layout: Layout, listed: ListedGroup[], key: string, before: string | null): Layout {
  const all = materialized(layout, listed).groups;
  // Shared groups just made mine are found by name.
  const find = (k: string) => all.find((g) => mineKey(g.id) === k) ?? all.find((g) => sharedKey(g.name) === k && !layout.groups.includes(g));
  const moving = find(key);
  if (!moving) return layout;
  const rest = all.filter((g) => g !== moving);
  const target = before ? find(before) : undefined;
  const at = target ? rest.indexOf(target) : rest.length;
  rest.splice(at < 0 ? rest.length : at, 0, moving);
  return { groups: rest };
}

export const mineAdd = (layout: Layout, name: string): Layout => ({ groups: [...layout.groups, { id: uid("m"), name, hosts: [] }] });
export const mineRename = (layout: Layout, id: string, name: string): Layout => ({ groups: layout.groups.map((g) => (g.id === id ? { ...g, name } : g)) });
export const mineRemove = (layout: Layout, id: string): Layout => ({ groups: layout.groups.filter((g) => g.id !== id) });
