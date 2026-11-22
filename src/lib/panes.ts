// How a terminal tab is split: a tree whose leaves are panes and whose nodes lay their
// children side by side ("row") or one above another ("col"), each child with its share.
// Panes are drawn in one flat list placed by `place`, so a pane whose place in the tree
// changes (its neighbour split, a sibling closed) keeps its terminal and its session.

export type SplitDir = "row" | "col";
export type Side = "left" | "right" | "top" | "bottom";

export interface Pane {
  id: string;
  hostId: string;
  command?: string;
  /** A session the core already holds, shown instead of opening one. */
  attach?: string;
}

export type Layout = { pane: Pane } | { dir: SplitDir; children: Layout[]; sizes: number[] };

/** Most panes a tab splits into. */
export const MAX_PANES = 8;

export const isPane = (l: Layout): l is { pane: Pane } => "pane" in l;

export function panes(l: Layout): Pane[] {
  return isPane(l) ? [l.pane] : l.children.flatMap(panes);
}

/** Every pane kept, `fn` applied to the one with this id. */
export function mapPane(l: Layout, id: string, fn: (p: Pane) => Pane): Layout {
  if (isPane(l)) return l.pane.id === id ? { pane: fn(l.pane) } : l;
  return { ...l, children: l.children.map((c) => mapPane(c, id, fn)) };
}

/** A node with one child is that child; a child split the same way as its node joins it. */
function tidy(l: Layout): Layout {
  if (isPane(l)) return l;
  const children: Layout[] = [];
  const sizes: number[] = [];
  l.children.forEach((c0, i) => {
    const c = tidy(c0);
    const share = l.sizes[i] ?? 1 / l.children.length;
    if (!isPane(c) && c.dir === l.dir) {
      c.children.forEach((cc, j) => {
        children.push(cc);
        sizes.push(share * (c.sizes[j] ?? 1 / c.children.length));
      });
    } else {
      children.push(c);
      sizes.push(share);
    }
  });
  if (children.length === 1) return children[0]!;
  const sum = sizes.reduce((a, b) => a + b, 0) || 1;
  return { dir: l.dir, children, sizes: sizes.map((s) => s / sum) };
}

/** `add` put beside the pane `target`, on `side` of it; the two share what the pane had. */
export function insert(l: Layout, target: string, side: Side, add: Layout): Layout {
  const dir: SplitDir = side === "left" || side === "right" ? "row" : "col";
  const before = side === "left" || side === "top";
  const walk = (n: Layout): Layout => {
    if (isPane(n)) {
      if (n.pane.id !== target) return n;
      return { dir, children: before ? [add, n] : [n, add], sizes: [0.5, 0.5] };
    }
    // Splitting a pane the way its node already goes adds a pane to that node.
    const i = n.children.findIndex((c) => isPane(c) && c.pane.id === target);
    if (i >= 0 && n.dir === dir) {
      const half = n.sizes[i]! / 2;
      const children = [...n.children];
      const sizes = [...n.sizes];
      children.splice(before ? i : i + 1, 0, add);
      sizes.splice(i, 1, half, half);
      return { ...n, children, sizes };
    }
    return { ...n, children: n.children.map(walk) };
  };
  return tidy(walk(l));
}

/** The layout without the pane `id`; null when it was the last. Its share goes to its neighbours. */
export function remove(l: Layout, id: string): Layout | null {
  if (isPane(l)) return l.pane.id === id ? null : l;
  const children: Layout[] = [];
  const sizes: number[] = [];
  l.children.forEach((c, i) => {
    const r = remove(c, id);
    if (r) {
      children.push(r);
      sizes.push(l.sizes[i]!);
    }
  });
  if (!children.length) return null;
  return tidy({ dir: l.dir, children, sizes });
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Divider extends Rect {
  dir: SplitDir;
  /** Where the node is in the tree, and the child the divider follows. */
  path: number[];
  index: number;
  /** The node's own size in the tab, to turn a drag into shares. */
  node: Rect;
}

/** Where each pane and divider is, as shares of the tab. */
export function place(l: Layout): { panes: { pane: Pane; rect: Rect }[]; dividers: Divider[] } {
  const out = { panes: [] as { pane: Pane; rect: Rect }[], dividers: [] as Divider[] };
  const walk = (n: Layout, r: Rect, path: number[]) => {
    if (isPane(n)) return void out.panes.push({ pane: n.pane, rect: r });
    let at = 0;
    n.children.forEach((c, i) => {
      const s = n.sizes[i]!;
      const cr = n.dir === "row" ? { x: r.x + at * r.w, y: r.y, w: s * r.w, h: r.h } : { x: r.x, y: r.y + at * r.h, w: r.w, h: s * r.h };
      walk(c, cr, [...path, i]);
      at += s;
      if (i < n.children.length - 1) {
        out.dividers.push(n.dir === "row" ? { dir: "row", path, index: i, node: r, x: r.x + at * r.w, y: r.y, w: 0, h: r.h } : { dir: "col", path, index: i, node: r, x: r.x, y: r.y + at * r.h, w: r.w, h: 0 });
      }
    });
  };
  walk(l, { x: 0, y: 0, w: 1, h: 1 }, []);
  return out;
}

/** The node at `path` with the divider after child `index` moved: its two neighbours get `first` and the rest of their shares. */
export function resize(l: Layout, path: number[], index: number, first: number): Layout {
  if (isPane(l)) return l;
  if (path.length) return { ...l, children: l.children.map((c, i) => (i === path[0] ? resize(c, path.slice(1), index, first) : c)) };
  const both = l.sizes[index]! + l.sizes[index + 1]!;
  const sizes = [...l.sizes];
  sizes[index] = first;
  sizes[index + 1] = both - first;
  return { ...l, sizes };
}

export const sharesAt = (l: Layout, path: number[]): number[] => {
  let n = l;
  for (const i of path) if (!isPane(n)) n = n.children[i]!;
  return isPane(n) ? [] : n.sizes;
};

/** The pane next to `id` in a direction: the nearest one that touches it on that side. */
export function neighbour(l: Layout, id: string, side: Side): string | null {
  const all = place(l).panes;
  const from = all.find((p) => p.pane.id === id)?.rect;
  if (!from) return null;
  const eps = 1e-6;
  const overlap = (a0: number, a1: number, b0: number, b1: number) => Math.min(a1, b1) - Math.max(a0, b0);
  let best: { id: string; score: number } | null = null;
  for (const { pane, rect } of all) {
    if (pane.id === id) continue;
    let touches = false;
    let shared = 0;
    if (side === "left" || side === "right") {
      touches = side === "left" ? Math.abs(rect.x + rect.w - from.x) < eps : Math.abs(from.x + from.w - rect.x) < eps;
      shared = overlap(from.y, from.y + from.h, rect.y, rect.y + rect.h);
    } else {
      touches = side === "top" ? Math.abs(rect.y + rect.h - from.y) < eps : Math.abs(from.y + from.h - rect.y) < eps;
      shared = overlap(from.x, from.x + from.w, rect.x, rect.x + rect.w);
    }
    if (touches && shared > eps && (!best || shared > best.score)) best = { id: pane.id, score: shared };
  }
  return best?.id ?? null;
}

/** The core session each pane shows, so a pane moved to another place shows it again. */
export const paneSession = new Map<string, string>();
/** Panes on their way to another place: their terminal goes without ending the session. */
export const movingPanes = new Set<string>();

/** The layout `l` ready to be shown elsewhere: each pane shows the session it had. */
export function carried(l: Layout): Layout {
  if (isPane(l)) {
    movingPanes.add(l.pane.id);
    const attach = paneSession.get(l.pane.id) ?? l.pane.attach;
    return { pane: { ...l.pane, attach, command: attach ? undefined : l.pane.command } };
  }
  return { ...l, children: l.children.map(carried) };
}
