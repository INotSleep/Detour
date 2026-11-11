// Groups of hosts in their order, and moving hosts and groups around.

import { isNoGroup } from "../components/common";
import { uid } from "./store";
import type { Group, Host, Workspace } from "./types";

/** A group as the host list shows it: its record if it has one, its hosts in order. */
export interface ListedGroup {
  /** The group's name; "" for hosts without one. */
  name: string;
  record?: Group;
  hosts: Host[];
}

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
  const out: ListedGroup[] = recorded.filter((g) => withEmpty || byName.has(g.name)).map((g) => ({ name: g.name, record: g, hosts: sortHosts(byName.get(g.name) ?? []) }));
  const known = new Set(recorded.map((g) => g.name));
  const rest = [...byName.keys()].filter((n) => n && !known.has(n)).sort((a, b) => a.localeCompare(b));
  out.push(...rest.map((name) => ({ name, hosts: sortHosts(byName.get(name)!) })));
  if (byName.has("")) out.push({ name: "", hosts: sortHosts(byName.get("")!) });
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
