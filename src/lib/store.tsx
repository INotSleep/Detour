import { createContext, useContext } from "react";
import type { Settings, Vault, Workspace } from "./types";
import type { CoreInfo, TeamView } from "./api";

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
  coreInfo: CoreInfo | null;
  toast: (t: ToastInput) => void;
  connect: (hostId: string, command?: string) => void;
  openPalette: () => void;
  /** Opens the conflict resolver for a vault (defaults to the current one). */
  resolveConflicts: (vaultId?: string) => void;
  /** Locks the vault (detour-core forgets decrypted secrets). */
  lock: () => Promise<void>;
  /** Puts a team's state returned by detour-core into the workspace. */
  applyTeamView: (view: TeamView) => void;
}

export const StoreContext = createContext<Store | null>(null);

export function useStore(): Store {
  const s = useContext(StoreContext);
  if (!s) throw new Error("useStore outside StoreContext");
  return s;
}

export const uid = (p: string) => `${p}${Date.now().toString(36)}${Math.random().toString(36).slice(2, 6)}`;

export const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

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

/** "5 мин назад", "вчера", "3 дня назад", "14 мар 2026". */
export function formatAgo(ms?: number): string {
  if (!ms) return "";
  const min = Math.floor((Date.now() - ms) / 60_000);
  if (min < 1) return "только что";
  if (min < 60) return `${min} мин назад`;
  const h = Math.floor(min / 60);
  if (h < 24) return `${h} ч назад`;
  const d = Math.floor(h / 24);
  if (d === 1) return "вчера";
  if (d < 7) return `${d} ${plural(d, "день", "дня", "дней")} назад`;
  return formatDate(ms);
}

export function formatDate(ms: number): string {
  return new Date(ms).toLocaleDateString("ru-RU", { day: "numeric", month: "short", year: "numeric" }).replace(" г.", "");
}

export function plural(n: number, one: string, few: string, many: string) {
  const m10 = n % 10;
  const m100 = n % 100;
  if (m10 === 1 && m100 !== 11) return one;
  if (m10 >= 2 && m10 <= 4 && (m100 < 12 || m100 > 14)) return few;
  return many;
}
