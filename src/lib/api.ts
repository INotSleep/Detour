// The only place the UI talks to the backend.
//
// In the Tauri window the personal vault, keys and SSH sessions go to
// detour-core. Team vaults are still demo data (team sync is the next core
// stage), and in a plain browser (vite dev) everything is demo data, so the UI
// can be developed without the Rust side.

import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { fakeKeyMaterial, parsePrivateKey } from "./keys";
import { mockSecrets } from "./mockData";
import type { ActivityItem, Conflict, Host, Role, Snippet, SshKey, Tunnel } from "./types";

export interface CoreInfo {
  coreVersion: string;
  os: string;
}

/** Errors from detour-core, see `detour_core::Error`. */
export type CoreError =
  | { kind: "locked" | "alreadyInitialized" | "notInitialized" | "wrongMasterPassword" | "passphraseRequired" | "wrongPassphrase" | "agentUnavailable" }
  | { kind: "notFound"; what: string }
  | { kind: "invalidKey"; reason: string }
  | { kind: "needSecret"; secret: SecretKind; prompt: string }
  | { kind: "authFailed"; secret: SecretKind | null }
  | { kind: "hostKeyUnknown"; fingerprint: string }
  | { kind: "hostKeyChanged"; expected: string; actual: string }
  | { kind: "unreachable"; endpoint: string }
  | { kind: "connect" | "io" | "internal"; message: string }
  | { kind: "invalidId"; reason: string }
  | { kind: "forbidden" | "notATeamVault" | "folderNotEmpty" | "rootChanged" | "alreadyMember" }
  | { kind: "unavailable"; path: string };

export type SecretKind = "password" | "passphrase";

export function isCoreError(e: unknown): e is CoreError {
  return typeof e === "object" && e !== null && "kind" in e;
}

/** Human text for an error, for toasts and inline messages. */
export function errorText(e: unknown): string {
  if (!isCoreError(e)) return e instanceof Error ? e.message : String(e);
  switch (e.kind) {
    case "locked": return "Vault заблокирован";
    case "alreadyInitialized": return "Vault уже создан";
    case "notInitialized": return "Vault ещё не создан";
    case "wrongMasterPassword": return "Неверный мастер-пароль";
    case "notFound": return `Не найдено: ${e.what}`;
    case "invalidKey": return keyReason(e.reason);
    case "passphraseRequired": return "Ключ защищён паролем";
    case "wrongPassphrase": return "Неверный пароль ключа";
    case "needSecret": return e.prompt;
    case "authFailed": return e.secret === "password" ? "Неверный пароль" : e.secret === "passphrase" ? "Неверный пароль ключа" : "Сервер не принял ключ";
    case "hostKeyUnknown": return "Неизвестный ключ хоста";
    case "hostKeyChanged": return "Ключ хоста изменился";
    case "unreachable": return `${e.endpoint} не отвечает`;
    case "agentUnavailable": return "ssh-agent не запущен";
    case "invalidId": return e.reason.includes("recovery") ? "Неверный ключ восстановления" : "Неверный Detour ID";
    case "forbidden": return "Недостаточно прав";
    case "notATeamVault": return "В этой папке нет хранилища команды";
    case "folderNotEmpty": return "Папка не пустая";
    case "rootChanged": return "Создатель хранилища изменился";
    case "alreadyMember": return "Уже в команде";
    case "unavailable": return `Хранилище недоступно: ${e.path}`;
    default: return e.message;
  }
}

function keyReason(reason: string) {
  if (reason.startsWith("this is a public key")) return "Это публичный ключ, нужен приватный";
  if (reason.startsWith("PuTTY")) return "Формат PuTTY: экспортируйте ключ в OpenSSH";
  return "Не похоже на приватный ключ";
}

/* ---------- shapes exchanged with detour-core ---------- */

type HostView = Omit<Host, "vaultId" | "os" | "online">;
type KeyView = Omit<SshKey, "vaultId">;
type SnippetView = Omit<Snippet, "vaultId">;
type TunnelView = Omit<Tunnel, "vaultId" | "state" | "error">;

export interface VaultView {
  hosts: HostView[];
  keys: KeyView[];
  snippets: SnippetView[];
  tunnels: TunnelView[];
}

export interface PersonalData {
  hosts: Host[];
  keys: SshKey[];
  snippets: Snippet[];
  tunnels: Tunnel[];
}

const PERSONAL = "personal";

export function fromView(v: VaultView, vaultId = PERSONAL): PersonalData {
  return {
    hosts: v.hosts.map((h) => ({ ...h, vaultId })),
    keys: v.keys.map((k) => ({ ...k, vaultId })),
    snippets: v.snippets.map((s) => ({ ...s, vaultId })),
    tunnels: v.tunnels.map((t) => ({ ...t, vaultId, state: "off" })),
  };
}

export function toView(d: PersonalData): VaultView {
  return {
    hosts: d.hosts.map(({ vaultId: _v, os: _o, online: _on, ...h }) => h),
    keys: d.keys.map(({ vaultId: _v, ...k }) => k),
    snippets: d.snippets.map(({ vaultId: _v, ...s }) => s),
    tunnels: d.tunnels.map(({ vaultId: _v, state: _s, error: _e, ...t }) => t),
  };
}

/* ---------- teams ---------- */

/** detour_core::team::TeamView */
export interface TeamView {
  id: string;
  name: string;
  path: string;
  joined: boolean;
  removed: boolean;
  myRole: Role | null;
  canReveal: boolean;
  members: { id: string; name: string; role: Role; canReveal: boolean; recovery: boolean; fingerprint: string; isYou: boolean }[];
  data: VaultView;
  conflicts: Conflict[];
  activity: Omit<ActivityItem, "vaultId">[];
  problems: { device: string; reason: string }[];
}

export interface TeamEntry {
  id: string;
  path: string;
  view: TeamView | null;
  /** JSON of a CoreError */
  error: string | null;
}

export interface VaultInfo {
  id: string;
  root: string;
  rootFingerprint: string;
}

export interface MyId {
  detourId: string;
  fingerprint: string;
}

/* ---------- sessions ---------- */

export interface SessionCallbacks {
  onData: (bytes: Uint8Array) => void;
  onClosed: (exitStatus: number | null) => void;
}

export interface OpenedSession {
  sessionId: string;
  pinnedFingerprint: string | null;
}

/* ---------- demo backend (browser, team vaults) ---------- */

const demoUnlocked = () => !/[?&](setup|locked)\b/.test(location.search);

function demoKey(vaultId: string, name: string, algorithm: SshKey["algorithm"], privateKey: string, passphrase: string | null, remember: boolean): SshKey {
  const m = fakeKeyMaterial(algorithm === "rsa" ? "rsa" : "ed25519", name);
  const id = `k${Date.now().toString(36)}`;
  mockSecrets.privateKeys.set(id, privateKey || m.privateKey);
  if (passphrase && remember) mockSecrets.keyPassphrases.set(id, passphrase);
  return {
    id,
    vaultId,
    name,
    algorithm,
    bits: algorithm === "rsa" ? 4096 : undefined,
    fingerprint: m.fingerprint,
    publicKey: m.publicKey,
    hasPassphrase: !!passphrase,
    passphraseSaved: !!passphrase && remember,
    createdAt: Date.now(),
  };
}

/* ---------- api ---------- */

const real = isTauri();

export const api = {
  /** True inside the Tauri window. */
  real,

  /** Does this vault live in detour-core (as opposed to demo data)? */
  usesCore(_vaultId: string) {
    return real;
  },

  async coreInfo(): Promise<CoreInfo | null> {
    return real ? invoke<CoreInfo>("core_info") : null;
  },

  /** Asks where to save and writes the file. Returns where it went, or null if cancelled. */
  async saveTextFile(defaultName: string, contents: string): Promise<string | null> {
    if (real) {
      const path = await save({ defaultPath: defaultName });
      if (!path) return null;
      await invoke("write_text_file", { path, contents });
      return path;
    }
    const url = URL.createObjectURL(new Blob([contents], { type: "text/plain" }));
    const a = Object.assign(document.createElement("a"), { href: url, download: defaultName });
    a.click();
    URL.revokeObjectURL(url);
    return defaultName;
  },

  /* vault */

  async vaultStatus(): Promise<{ initialized: boolean; unlocked: boolean }> {
    if (real) return invoke("vault_status");
    return { initialized: !/[?&]setup\b/.test(location.search), unlocked: demoUnlocked() };
  },

  async vaultCreate(masterPassword: string): Promise<VaultView | null> {
    if (real) return invoke("vault_create", { masterPassword });
    return null;
  },

  async vaultUnlock(masterPassword: string): Promise<VaultView | null> {
    if (real) return invoke("vault_unlock", { masterPassword });
    if (masterPassword === "wrong") throw { kind: "wrongMasterPassword" } satisfies CoreError;
    return null;
  },

  async vaultGet(): Promise<VaultView> {
    return invoke("vault_get");
  },

  async vaultLock(): Promise<void> {
    if (real) await invoke("vault_lock");
  },

  async vaultChangeMasterPassword(current: string, next: string): Promise<void> {
    if (real) await invoke("vault_change_master_password", { current, new: next });
  },

  async vaultPut(data: PersonalData): Promise<VaultView> {
    return invoke("vault_put", { update: toView(data) });
  },

  /* hosts */

  async hostSetPassword(host: Host, password: string | null): Promise<void> {
    if (api.usesCore(host.vaultId)) return invoke("host_set_password", { vaultId: host.vaultId, hostId: host.id, password });
    if (password) mockSecrets.hostPasswords.set(host.id, password);
    else mockSecrets.hostPasswords.delete(host.id);
  },

  async hostForgetKey(vaultId: string, hostId: string): Promise<void> {
    await invoke("host_forget_key", { vaultId, hostId });
  },

  /* keys */

  async keyGenerate(vaultId: string, name: string, algorithm: "ed25519" | "rsa", passphrase: string | null, remember: boolean): Promise<SshKey> {
    if (api.usesCore(vaultId)) return { ...(await invoke<KeyView>("key_generate", { vaultId, name, algorithm, passphrase, remember })), vaultId };
    await new Promise((r) => setTimeout(r, algorithm === "rsa" ? 1200 : 300));
    return demoKey(vaultId, name, algorithm, "", passphrase, remember);
  },

  async keyImport(vaultId: string, name: string, text: string, passphrase: string | null, remember: boolean): Promise<SshKey> {
    if (api.usesCore(vaultId)) return { ...(await invoke<KeyView>("key_import", { vaultId, name, text, passphrase, remember })), vaultId };
    const parsed = parsePrivateKey(text);
    if (!parsed.ok) throw { kind: "invalidKey", reason: parsed.error } satisfies CoreError;
    if (/ENCRYPTED|aes256/.test(text) && !passphrase) throw { kind: "passphraseRequired" } satisfies CoreError;
    return demoKey(vaultId, name, parsed.algorithm, text.trim(), passphrase, remember);
  },

  async keyExport(key: SshKey, passphrase: string | null): Promise<string> {
    if (api.usesCore(key.vaultId)) return invoke("key_export", { vaultId: key.vaultId, keyId: key.id, passphrase });
    return mockSecrets.privateKeys.get(key.id) ?? "";
  },

  async keySetPassphrase(key: SshKey, current: string | null, next: string | null, remember: boolean): Promise<SshKey> {
    if (api.usesCore(key.vaultId)) return { ...(await invoke<KeyView>("key_set_passphrase", { vaultId: key.vaultId, keyId: key.id, current, new: next, remember })), vaultId: key.vaultId };
    const saved = mockSecrets.keyPassphrases.get(key.id);
    if (key.hasPassphrase && !saved && !current) throw { kind: "passphraseRequired" } satisfies CoreError;
    if (next && remember) mockSecrets.keyPassphrases.set(key.id, next);
    else mockSecrets.keyPassphrases.delete(key.id);
    return { ...key, hasPassphrase: !!next, passphraseSaved: !!next && remember };
  },

  /* sessions (detour-core only) */

  async sessionOpen(
    args: { vaultId: string; hostId: string; secret: string | null; trustFingerprint: string | null; cols: number; rows: number },
    cb: SessionCallbacks,
  ): Promise<OpenedSession> {
    const onEvent = new Channel<ArrayBuffer | { closed: true; exitStatus: number | null }>();
    onEvent.onmessage = (m) => {
      if (m instanceof ArrayBuffer) cb.onData(new Uint8Array(m));
      else if (m && typeof m === "object" && "closed" in m) cb.onClosed(m.exitStatus);
    };
    return invoke("session_open", { args, onEvent });
  },

  sessionInput(sessionId: string, data: string) {
    return invoke("session_input", { sessionId, data });
  },

  sessionResize(sessionId: string, cols: number, rows: number) {
    return invoke("session_resize", { sessionId, cols, rows });
  },

  sessionClose(sessionId: string) {
    return invoke("session_close", { sessionId });
  },

  /* teams (detour-core only) */

  myId: (): Promise<MyId> => invoke("my_id"),
  idInspect: (detourId: string): Promise<string> => invoke("id_inspect", { detourId }),
  recoveryKeyNew: (): Promise<string> => invoke("recovery_key_new"),
  teamsList: (): Promise<TeamEntry[]> => invoke("teams_list"),
  teamCreate: (path: string, name: string, myName: string, recoveryKey: string): Promise<TeamView> => invoke("team_create", { path, name, myName, recoveryKey }),
  teamInspect: (path: string): Promise<VaultInfo> => invoke("team_inspect", { path }),
  teamJoin: (path: string, root: string): Promise<TeamView> => invoke("team_join", { path, root }),
  teamRecover: (path: string, recoveryKey: string, myName: string): Promise<TeamView> => invoke("team_recover", { path, recoveryKey, myName }),
  teamLeave: (id: string): Promise<void> => invoke("team_leave", { id }),
  teamSync: (id: string): Promise<TeamView> => invoke("team_sync", { id }),
  teamPut: (id: string, data: PersonalData): Promise<TeamView> => invoke("team_put", { id, update: toView(data) }),
  teamRename: (id: string, name: string): Promise<TeamView> => invoke("team_rename", { id, name }),
  teamMemberAdd: (id: string, detourId: string, name: string, role: Role, canReveal: boolean): Promise<TeamView> => invoke("team_member_add", { id, detourId, name, role, canReveal }),
  teamMemberUpdate: (id: string, member: string, role: Role, canReveal: boolean): Promise<TeamView> => invoke("team_member_update", { id, member, role, canReveal }),
  teamMemberRemove: (id: string, member: string): Promise<TeamView> => invoke("team_member_remove", { id, member }),
  teamResolveField: (id: string, coll: string, record: string, field: string, pick: number): Promise<TeamView> => invoke("team_resolve_field", { id, coll, record, field, pick }),
  teamResolveDeleted: (id: string, coll: string, record: string, keep: boolean): Promise<TeamView> => invoke("team_resolve_deleted", { id, coll, record, keep }),

  /** Native folder picker; null if cancelled or not in the app window. */
  async pickFolder(): Promise<string | null> {
    if (!real) return null;
    const r = await open({ directory: true, multiple: false });
    return typeof r === "string" ? r : null;
  },

  async secretSave(host: Host, kind: SecretKind, value: string): Promise<void> {
    if (api.usesCore(host.vaultId)) return invoke("secret_save", { vaultId: host.vaultId, hostId: host.id, kind, value });
    if (kind === "password") mockSecrets.hostPasswords.set(host.id, value);
    else if (host.auth.type === "key") mockSecrets.keyPassphrases.set(host.auth.keyId, value);
  },
};
