import { tr } from "../i18n";
import { createContext, useContext } from "react";
import type { Settings, Vault, Workspace } from "./types";
import type { CoreInfo, SyncInfo, TeamView } from "./api";
import type { UpdateInfo } from "./updates";
import type { Theme } from "./themes";

/** Where looking for and installing a new version stands. */
export type UpdateState =
  | { kind: "idle" | "checking" | "latest" }
  | { kind: "available"; info: UpdateInfo }
  | { kind: "installing"; info: UpdateInfo; share: number | null }
  | { kind: "error"; text: string };

export type Section = "hosts" | "keys" | "snippets" | "tunnels" | "team" | "settings";

export interface ToastInput {
  text: string;
  tone?: "default" | "success" | "error";
  action?: { label: string; run: () => void };
  /** ms; defaults depend on whether there is an action */
  duration?: number;
}

export interface Store {
  ws: Workspace;
  update: (fn: (ws: Workspace) => Workspace) => void;
  vault: Vault;
  setVault: (id: string) => void;
  section: Section;
  navigate: (s: Section) => void;
  settings: Settings;
  setSettings: (patch: Partial<Settings>) => void;
  /** The themes in use, "system" and "same as the app" resolved. */
  themes: { app: Theme; terminal: Theme };
  coreInfo: CoreInfo | null;
  toast: (t: ToastInput) => void;
  connect: (hostId: string, command?: string) => void;
  /** Opens a file browser tab on a host. */
  openFiles: (hostId: string) => void;
  openPalette: () => void;
  /** Opens the conflict resolver for a vault (defaults to the current one). */
  resolveConflicts: (vaultId?: string) => void;
  /** Locks the vault (detour-core forgets decrypted secrets). */
  lock: () => Promise<void>;
  /** Puts a team's state returned by detour-core into the workspace. */
  applyTeamView: (view: TeamView) => void;
  /** How the personal vault syncs with this person's other devices; null until known. */
  sync: SyncInfo | null;
  setSync: (s: SyncInfo) => void;
  /** Reads the personal vault again, after sync brought changes. */
  refreshPersonal: () => Promise<void>;
  update_: UpdateState;
  checkUpdate: () => Promise<void>;
  installUpdate: () => Promise<void>;
}

export const StoreContext = createContext<Store | null>(null);

export function useStore(): Store {
  const s = useContext(StoreContext);
  if (!s) throw new Error("useStore outside StoreContext");
  return s;
}

export const uid = (p: string) => `${p}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

/** The letter of a shortcut whatever the keyboard layout: Ctrl+K stays Ctrl+K on a Cyrillic layout. */
export const shortcutKey = (e: { code: string; key: string }) => (/^Key[A-Z]$/.test(e.code) ? e.code.slice(3).toLowerCase() : e.key.toLowerCase());

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

export function sshCommand(h: { user: string; address: string; port: number }) {
  return `ssh ${h.port !== 22 ? `-p ${h.port} ` : ""}${h.user}@${h.address}`;
}

/** "5 min ago", "yesterday", "3 days ago", "Mar 14, 2026". */
export function formatAgo(ms?: number): string {
  if (!ms) return "";
  return tr().common.ago(Math.floor((Date.now() - ms) / 60_000)) ?? formatDate(ms);
}

export function formatDate(ms: number): string {
  return tr().common.date(new Date(ms));
}
