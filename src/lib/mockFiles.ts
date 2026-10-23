// Demo file system for the file browser in a plain browser (no detour-core).
import type { FileEntry } from "./api";

interface Node {
  kind: "file" | "dir";
  size: number;
  modified: number;
  mode?: number;
  children?: Map<string, Node>;
}

const day = 86_400_000;
const now = Date.now();

function dir(entries: Record<string, Node>, age = 3): Node {
  return { kind: "dir", size: 4096, modified: now - age * day, children: new Map(Object.entries(entries)) };
}

function file(size: number, age: number): Node {
  return { kind: "file", size, modified: now - age * day };
}

const roots = new Map<string, Node>();

function fsFor(user: string): Node {
  let root = roots.get(user);
  if (!root) {
    root = dir({
      home: dir({
        [user]: dir({
          app: dir({ "docker-compose.yml": file(1_284, 2), ".env": file(412, 2), logs: dir({ "app.log": file(8_402_113, 0), "app.log.1": file(52_428_800, 1) }, 0) }, 1),
          backups: dir({ "db-2026-10-20.sql.gz": file(184_320_512, 3), "db-2026-10-21.sql.gz": file(186_011_904, 2) }, 2),
          ".bashrc": file(3_771, 90),
          ".profile": file(807, 90),
          "notes.md": file(2_150, 5),
        }, 1),
      }),
      etc: dir({ hostname: file(12, 200), hosts: file(221, 40), "nginx": dir({ "nginx.conf": file(1_490, 30) }, 30) }, 30),
      var: dir({ log: dir({ syslog: file(2_341_120, 0) }, 0) }, 10),
    });
    roots.set(user, root);
  }
  return root;
}

const split = (path: string) => path.split("/").filter(Boolean);

function find(user: string, path: string): Node | null {
  let n: Node | undefined = fsFor(user);
  for (const part of split(path)) n = n?.children?.get(part);
  return n ?? null;
}

const notFound = (path: string) => ({ kind: "notFound", what: path });

export const mockFiles = {
  home: (user: string) => `/home/${user}`,

  list(user: string, path: string): FileEntry[] {
    const n = find(user, path);
    if (!n?.children) throw notFound(path);
    return [...n.children.entries()]
      .map(([name, c]) => ({ name, kind: c.kind, size: c.size, modified: c.modified, permissions: c.mode ?? (c.kind === "dir" ? 0o755 : 0o644) }))
      .sort((a, b) => Number(a.kind !== "dir") - Number(b.kind !== "dir") || a.name.toLowerCase().localeCompare(b.name.toLowerCase()));
  },

  resolve(path: string): string {
    const out: string[] = [];
    for (const p of split(path)) {
      if (p === "..") out.pop();
      else if (p !== ".") out.push(p);
    }
    return "/" + out.join("/");
  },

  /** A file's size, or everything in a folder. */
  size(user: string, path: string): number {
    const total = (n: Node | null): number => (!n ? 0 : n.children ? [...n.children.values()].reduce((s, c) => s + total(c), 0) : n.size);
    return total(find(user, path));
  },

  put(user: string, path: string, node: Node) {
    const parts = split(path);
    const name = parts.pop()!;
    const parent = find(user, "/" + parts.join("/"));
    if (!parent?.children) throw notFound(path);
    if (node.kind === "dir" && parent.children.has(name)) throw { kind: "sftp", message: `${path} already exists` };
    parent.children.set(name, node);
  },

  mkdir(user: string, path: string) {
    mockFiles.put(user, path, dir({}, 0));
  },

  upload(user: string, path: string, size: number) {
    mockFiles.put(user, path, file(size, 0));
  },

  rename(user: string, from: string, to: string) {
    if (find(user, to)) throw { kind: "sftp", message: `${to} already exists` };
    const node = find(user, from);
    if (!node) throw notFound(from);
    mockFiles.remove(user, from);
    mockFiles.put(user, to, node);
  },

  chmod(user: string, path: string, mode: number) {
    const n = find(user, path);
    if (!n) throw notFound(path);
    n.mode = mode;
  },

  remove(user: string, path: string) {
    const parts = split(path);
    const name = parts.pop()!;
    find(user, "/" + parts.join("/"))?.children?.delete(name);
  },
};
