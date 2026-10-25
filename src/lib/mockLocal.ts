// Demo of this computer's files, for the side beside a server in a plain browser (no detour-core).
import type { FileEntry } from "./api";

interface Node {
  kind: "file" | "dir";
  size: number;
  modified: number;
  children?: Map<string, Node>;
}

const day = 86_400_000;
const now = Date.now();
const dir = (entries: Record<string, Node>, age = 5): Node => ({ kind: "dir", size: 0, modified: now - age * day, children: new Map(Object.entries(entries)) });
const file = (size: number, age: number): Node => ({ kind: "file", size, modified: now - age * day });

const drives = new Map<string, Node>([
  [
    "C:\\",
    dir({
      Users: dir({
        demo: dir({
          Desktop: dir({ "deploy-checklist.md": file(3_412, 1) }, 1),
          Documents: dir({ "invoice-2026-10.pdf": file(184_220, 4), "network-plan.drawio": file(52_110, 12), contracts: dir({ "acme-msa.pdf": file(412_881, 40) }, 40) }, 2),
          Downloads: dir({ "ubuntu-24.04-server.iso": file(2_754_981_888, 6), "nginx.conf": file(1_490, 0) }, 0),
          Projects: dir({ "site-backup.tar.gz": file(48_211_002, 3), scripts: dir({ "rotate-logs.sh": file(812, 9), "backup.sh": file(1_204, 9) }, 9) }, 3),
          "notes.txt": file(2_048, 1),
        }, 0),
      }, 30),
      "Program Files": dir({}, 60),
    }, 30),
  ],
  ["D:\\", dir({ Media: dir({}, 20), "archive-2025.zip": file(912_004_112, 120) }, 20)],
]);

const parts = (path: string): [string, string[]] => {
  const drive = path.slice(0, 3);
  return [drive, path.slice(3).split("\\").filter(Boolean)];
};

function find(path: string): Node | null {
  const [drive, names] = parts(path);
  let n = drives.get(drive) ?? null;
  for (const name of names) n = n?.children?.get(name) ?? null;
  return n;
}

const notFound = (path: string) => ({ kind: "io", message: `${path}: not found` });

export const mockLocal = {
  home: "C:\\Users\\demo",

  list(path: string): FileEntry[] {
    if (!path) return [...drives.keys()].map((name) => ({ name, kind: "dir" as const, size: 0, modified: null, permissions: null }));
    const n = find(path);
    if (!n?.children) throw notFound(path);
    return [...n.children.entries()].map(([name, c]) => ({ name, kind: c.kind, size: c.size, modified: c.modified, permissions: null }));
  },

  /** A file's size, or everything in a folder. */
  size(path: string): number {
    const total = (n: Node | null): number => (!n ? 0 : n.children ? [...n.children.values()].reduce((s, c) => s + total(c), 0) : n.size);
    return total(find(path));
  },

  mkdir(path: string) {
    const i = path.lastIndexOf("\\");
    const parent = find(path.slice(0, i + 1 === 3 ? 3 : i));
    if (!parent?.children) throw notFound(path);
    parent.children.set(path.slice(i + 1), dir({}, 0));
  },

  /** Something downloaded into `dirPath`, under a name not taken there. Returns its path. */
  add(dirPath: string, name: string, size: number): string {
    const parent = find(dirPath);
    if (!parent?.children) throw notFound(dirPath);
    const dot = name.lastIndexOf(".");
    const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ""];
    let free = name;
    for (let n = 2; parent.children.has(free); n++) free = `${stem} (${n})${ext}`;
    parent.children.set(free, file(size, 0));
    return `${dirPath.replace(/\\$/, "")}\\${free}`;
  },
};
