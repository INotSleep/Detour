// The only place the UI talks to the backend.
//
// In the Tauri window everything goes to detour-core. In a plain browser
// (vite dev) everything is demo data, so the UI can be developed without the
// Rust side.

import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { readText, writeText } from "@tauri-apps/plugin-clipboard-manager";
import { open, save } from "@tauri-apps/plugin-dialog";
import { fakeKeyMaterial, parsePrivateKey } from "./keys";
import { mockSecrets } from "./mockData";
import { mockFiles } from "./mockFiles";
import type { ActivityItem, Coll, Conflict, Host, Level, OwnLogin, Problem, Role, Snippet, SshKey, Tunnel } from "./types";
import { demoSync } from "./mockSync";

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
  | { kind: "notASyncVault" | "invalidCode" | "codeExpired" | "syncHasTeams" }
  | { kind: "git"; message: string }
  | { kind: "portBusy" | "forwardRefused"; port: number }
  | { kind: "unavailable"; path: string }
  | { kind: "sftp"; message: string }
  | { kind: "cancelled" };

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
    case "notASyncVault": return "Здесь нет синхронизированного vault";
    case "invalidCode": return "Неверный код";
    case "codeExpired": return "Код истёк, создайте новый на другом устройстве";
    case "syncHasTeams": return "На этом устройстве есть команды: сначала выйдите из них";
    case "unavailable": return `Хранилище недоступно: ${e.path}`;
    case "git": return gitReason(e.message);
    case "portBusy": return `Порт ${e.port} занят другой программой`;
    case "forwardRefused": return `Сервер не разрешает открыть порт ${e.port}`;
    case "sftp": return e.message.endsWith("already exists") ? "Уже есть файл с таким именем" : e.message;
    case "cancelled": return "Отменено";
    case "connect": return e.message === "connection lost" ? "Соединение с сервером потеряно" : e.message;
    default: return e.message;
  }
}

function gitReason(message: string) {
  if (message.includes("repository address")) return "Неверный адрес репозитория";
  if (message.includes("key or ssh-agent")) return "Выберите ключ или ssh-agent";
  if (message.includes("keeps changing")) return "Репозиторий всё время меняется, повторите позже";
  if (message.includes("token rejected")) return "Репозиторий не принял токен";
  if (message.includes("use https")) return "Токен передаётся только по https";
  if (message.includes("not a smart Git server")) return "По этому адресу нет Git-репозитория";
  if (message.includes("repository not found")) return "Репозиторий не найден";
  if (message.includes("unsafe")) return "В репозитории посторонние файлы";
  if (/not found|does not appear|not a git repository/i.test(message)) return "Репозиторий не найден";
  return `Репозиторий: ${message}`;
}

function keyReason(reason: string) {
  if (reason.startsWith("this is a public key")) return "Это публичный ключ, нужен приватный";
  if (reason.startsWith("PuTTY")) return "Формат PuTTY: экспортируйте ключ в OpenSSH";
  return "Не похоже на приватный ключ";
}

/* ---------- shapes exchanged with detour-core ---------- */

type HostView = Omit<Host, "vaultId" | "os" | "online" | "access">;
type KeyView = Omit<SshKey, "vaultId" | "access">;
type SnippetView = Omit<Snippet, "vaultId" | "access">;
type TunnelView = Omit<Tunnel, "vaultId" | "state" | "error" | "access">;

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
    hosts: d.hosts.map(({ vaultId: _v, os: _o, online: _on, access: _a, own: _w, ...h }) => h),
    keys: d.keys.map(({ vaultId: _v, access: _a, ...k }) => k),
    snippets: d.snippets.map(({ vaultId: _v, access: _a, ...s }) => s),
    tunnels: d.tunnels.map(({ vaultId: _v, state: _s, error: _e, access: _a, connections: _c, ...t }) => t),
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
  members: { id: string; name: string; role: Role; recovery: boolean; fingerprint: string; isYou: boolean }[];
  data: VaultView;
  /** Who has which access to the records you can see. */
  access: { coll: Coll; record: string; member: string; level: Level }[];
  conflicts: Conflict[];
  activity: Omit<ActivityItem, "vaultId">[];
  problems: Problem[];
  /** Team kept in Git: how mirroring it goes. */
  remote: { url: string; pending: boolean; error: CoreError | null; syncedAt: number | null } | null;
}

/** How to reach a team's Git repository: over SSH a personal key or ssh-agent, over HTTPS a token. */
export type GitAuth = { type: "key"; keyId: string } | { type: "agent" } | { type: "password" };

/** A repository address and how to log in to it. */
export interface GitLogin {
  url: string;
  auth: GitAuth;
  /** HTTPS only. */
  token?: string;
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

/** A file or folder on a server, see `detour_core::sftp::FileEntry`. */
export interface FileEntry {
  name: string;
  kind: "file" | "dir" | "link";
  size: number;
  /** Unix ms */
  modified: number | null;
  permissions: number | null;
}

/** Where the personal vault syncs and how that goes (detour_core::store::SyncStatus). */
export interface SyncInfo {
  enabled: boolean;
  /** The folder, or the repository's URL. */
  location: string | null;
  git: boolean;
  pending: boolean;
  error: CoreError | null;
  syncedAt: number | null;
  problems: Problem[];
}

/** A one-time code another device of this person joins with. */
export interface Pairing {
  code: string;
  expiresAt: number;
}

/** Where to sync: a folder, or a repository. */
export type SyncPlace = { path: string } | GitLogin;

/** A local file picked to upload. */
export interface LocalFile {
  /** Path on this computer; in a plain browser just the name. */
  path: string;
  name: string;
}

type OnProgress = (done: number, total: number) => void;

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

/* ---------- files helpers ---------- */

function progressChannel(onProgress: OnProgress) {
  const ch = new Channel<{ done: number; total: number }>();
  ch.onmessage = (m) => onProgress(m.done, m.total);
  return ch;
}

function baseName(path: string) {
  return path.split(/[\\/]/).pop() || path;
}

const demoDelay = (ms: number) => new Promise((r) => setTimeout(r, ms));
const demoUsers = new Map<string, string>();
const demoUser = (id: string) => demoUsers.get(id) ?? "demo";
const demoCancelled = new Set<string>();
const demoPicked = new Map<string, number>();

function demoLocal(files: File[]): LocalFile[] {
  return files.map((f) => {
    demoPicked.set(f.name, f.size);
    return { path: f.name, name: f.name };
  });
}

/** A transfer at about 40 MB/s, at least a second long. */
async function demoTransfer(transfer: string, total: number, onProgress: OnProgress) {
  const ms = Math.max(1000, (total / 40_000_000) * 1000);
  const start = Date.now();
  for (;;) {
    if (demoCancelled.delete(transfer)) throw { kind: "cancelled" } satisfies CoreError;
    const t = Math.min(1, (Date.now() - start) / ms);
    onProgress(Math.round(total * t), total);
    if (t >= 1) return;
    await demoDelay(100);
  }
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

  /** A saved password to show or copy: the shared one, or with `own` your own login for a team host. */
  async hostPassword(host: Host, own = false): Promise<string | null> {
    if (api.usesCore(host.vaultId)) return invoke("host_password", { vaultId: host.vaultId, hostId: host.id, own });
    return mockSecrets.hostPasswords.get(host.id) ?? null;
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
  teamSync: (id: string, trustFingerprint?: string): Promise<TeamView> => invoke("team_sync", { id, trustFingerprint: trustFingerprint ?? null }),
  teamCreateGit: (g: GitLogin, name: string, myName: string, recoveryKey: string, trustFingerprint?: string): Promise<TeamView> =>
    invoke("team_create_git", { url: g.url, auth: g.auth, token: g.token ?? null, name, myName, recoveryKey, trustFingerprint: trustFingerprint ?? null }),
  /** Clones a repository to see whose team it is; `token` hands the clone to teamJoinGit. */
  teamInspectGit: (g: GitLogin, trustFingerprint?: string): Promise<{ info: VaultInfo; clone: string }> =>
    invoke("team_inspect_git", { url: g.url, auth: g.auth, token: g.token ?? null, trustFingerprint: trustFingerprint ?? null }),
  /** Trusts a repository server's new key in place of the pinned one. */
  teamReplaceServerKey: (id: string, fingerprint: string): Promise<TeamView> => invoke("team_replace_server_key", { id, fingerprint }),
  teamJoinGit: (clone: string, root: string, g: GitLogin): Promise<TeamView> => invoke("team_join_git", { clone, root, url: g.url, auth: g.auth, token: g.token ?? null }),
  /** Replaces the HTTPS token of a team's repository. */
  teamSetGitToken: (id: string, token: string): Promise<TeamView> => invoke("team_set_git_token", { id, token }),
  teamRecoverGit: (g: GitLogin, recoveryKey: string, myName: string, trustFingerprint?: string): Promise<TeamView> =>
    invoke("team_recover_git", { url: g.url, auth: g.auth, token: g.token ?? null, recoveryKey, myName, trustFingerprint: trustFingerprint ?? null }),
  teamPut: (id: string, data: PersonalData): Promise<TeamView> => invoke("team_put", { id, update: toView(data) }),
  teamRename: (id: string, name: string): Promise<TeamView> => invoke("team_rename", { id, name }),
  teamMemberAdd: (id: string, detourId: string, name: string, role: Role, access: Level): Promise<TeamView> => invoke("team_member_add", { id, detourId, name, role, access }),
  teamMemberUpdate: (id: string, member: string, role: Role): Promise<TeamView> => invoke("team_member_update", { id, member, role }),
  teamAccessSet: (id: string, changes: { record: string; member: string; level: Level }[]): Promise<TeamView> => invoke("team_access_set", { id, changes }),
  /** Sets or (with null) removes your own login for a team host. */
  teamOwnLoginSet: (id: string, hostId: string, login: Omit<OwnLogin, "hasPassword"> | null): Promise<TeamView> => invoke("team_own_login_set", { id, hostId, login }),
  teamMemberRemove: (id: string, member: string): Promise<TeamView> => invoke("team_member_remove", { id, member }),
  teamResolveField: (id: string, coll: string, record: string, field: string, pick: number): Promise<TeamView> => invoke("team_resolve_field", { id, coll, record, field, pick }),
  teamResolveDeleted: (id: string, coll: string, record: string, keep: boolean): Promise<TeamView> => invoke("team_resolve_deleted", { id, coll, record, keep }),

  /** Native folder picker; null if cancelled or not in the app window. */
  async pickFolder(): Promise<string | null> {
    if (!real) return null;
    const r = await open({ directory: true, multiple: false });
    return typeof r === "string" ? r : null;
  },

  /* tunnels (detour-core only) */

  /** Opens a tunnel; `secret` and `trustFingerprint` answer a login's prompts. */
  tunnelStart: (t: Tunnel, secret: string | null, trustFingerprint: string | null): Promise<void> =>
    invoke("tunnel_start", { vaultId: t.vaultId, tunnelId: t.id, secret, trustFingerprint }),
  tunnelStop: (t: Tunnel): Promise<void> => invoke("tunnel_stop", { vaultId: t.vaultId, tunnelId: t.id }),
  /** Running and failed tunnels, keyed `<vault>/<tunnel>`. */
  tunnelsStatus: (): Promise<{ key: string; state: "on" | "error"; connections: number; error: CoreError | null }[]> => invoke("tunnels_status"),

  /* clipboard: the system one in the app, the browser's in the demo */

  async clipboardRead(): Promise<string> {
    try {
      return real ? await readText() : await navigator.clipboard.readText();
    } catch {
      return "";
    }
  },
  async clipboardWrite(text: string): Promise<boolean> {
    try {
      if (real) await writeText(text);
      else await navigator.clipboard.writeText(text);
      return true;
    } catch {
      return false;
    }
  },

  /* personal vault sync */

  syncStatus: (): Promise<SyncInfo> => (real ? invoke("sync_status") : demoSync.status()),
  /** Syncs now; `changed` means the personal vault should be read again. */
  syncNow: (trustFingerprint?: string): Promise<{ changed: boolean; status: SyncInfo }> =>
    real ? invoke("sync_now", { trustFingerprint: trustFingerprint ?? null }) : demoSync.now(),
  syncEnable: (place: SyncPlace, trustFingerprint?: string): Promise<SyncInfo> => {
    if (!real) return demoSync.enable(place);
    if ("path" in place) return invoke("sync_enable_folder", { path: place.path });
    return invoke("sync_enable_git", { url: place.url, auth: place.auth, token: place.token ?? null, trustFingerprint: trustFingerprint ?? null });
  },
  /** Joins the vault this person syncs on other devices, with a pairing code or the recovery key. */
  syncJoin: (place: SyncPlace, code: string, trustFingerprint?: string): Promise<SyncInfo> => {
    if (!real) return demoSync.join(place, code);
    if ("path" in place) return invoke("sync_join_folder", { path: place.path, code });
    return invoke("sync_join_git", { url: place.url, auth: place.auth, token: place.token ?? null, code, trustFingerprint: trustFingerprint ?? null });
  },
  syncAddDevice: (): Promise<Pairing> => (real ? invoke("sync_add_device") : demoSync.addDevice()),
  syncRecoveryKey: (): Promise<string> => (real ? invoke("sync_recovery_key") : demoSync.recoveryKey()),
  syncDisable: (): Promise<void> => (real ? invoke("sync_disable") : demoSync.disable()),
  syncReplaceServerKey: (fingerprint: string): Promise<{ changed: boolean; status: SyncInfo }> => invoke("sync_replace_server_key", { fingerprint }),
  syncSetToken: (token: string): Promise<{ changed: boolean; status: SyncInfo }> => invoke("sync_set_token", { token }),

  /* files (SFTP) */

  /** Opens a file browser on a host; `secret` and `trustFingerprint` answer a login's prompts. */
  async filesOpen(host: Host, secret: string | null, trustFingerprint: string | null): Promise<{ id: string; home: string }> {
    if (api.usesCore(host.vaultId)) return invoke("files_open", { vaultId: host.vaultId, hostId: host.id, secret, trustFingerprint });
    await demoDelay(500);
    if (host.online === false) throw { kind: "unreachable", endpoint: `${host.address}:${host.port}` } satisfies CoreError;
    demoUsers.set(host.id, host.user);
    return { id: host.id, home: mockFiles.home(host.user) };
  },
  async filesList(id: string, path: string): Promise<FileEntry[]> {
    if (real) return invoke("files_list", { id, path });
    await demoDelay(120);
    return mockFiles.list(demoUser(id), path);
  },
  async filesResolve(id: string, path: string): Promise<string> {
    return real ? invoke("files_resolve", { id, path }) : mockFiles.resolve(path);
  },
  async filesMkdir(id: string, path: string): Promise<void> {
    if (real) return invoke("files_mkdir", { id, path });
    mockFiles.mkdir(demoUser(id), path);
  },
  async filesRename(id: string, from: string, to: string): Promise<void> {
    if (real) return invoke("files_rename", { id, from, to });
    mockFiles.rename(demoUser(id), from, to);
  },
  /** Removes a file, or a folder with everything in it. */
  async filesRemove(id: string, path: string): Promise<void> {
    if (real) return invoke("files_remove", { id, path });
    await demoDelay(200);
    mockFiles.remove(demoUser(id), path);
  },
  /** Downloads a file to the place the user picked. */
  async filesDownload(id: string, remote: string, local: string, transfer: string, onProgress: OnProgress): Promise<void> {
    if (real) return invoke("files_download", { id, remote, local, transfer, onProgress: progressChannel(onProgress) });
    await demoTransfer(transfer, mockFiles.size(demoUser(id), remote), onProgress);
  },
  /** Downloads a file or a whole folder into a local folder, under a name not taken there. Returns where it went. */
  async filesDownloadInto(id: string, remote: string, localDir: string, transfer: string, onProgress: OnProgress): Promise<string> {
    if (real) return invoke("files_download_into", { id, remote, localDir, transfer, onProgress: progressChannel(onProgress) });
    await demoTransfer(transfer, mockFiles.size(demoUser(id), remote), onProgress);
    return `${localDir}\\${baseName(remote)}`;
  },
  /** Uploads a file or a whole folder; with `replace` what is there is replaced once the upload is complete. */
  async filesUpload(id: string, local: LocalFile, remote: string, replace: boolean, transfer: string, onProgress: OnProgress): Promise<void> {
    if (real) return invoke("files_upload", { id, local: local.path, remote, replace, transfer, onProgress: progressChannel(onProgress) });
    const size = demoPicked.get(local.path) ?? 0;
    await demoTransfer(transfer, size, onProgress);
    mockFiles.upload(demoUser(id), remote, size);
  },
  /** Sets Unix permission bits (`0o755`). */
  async filesChmod(id: string, path: string, mode: number): Promise<void> {
    if (real) return invoke("files_chmod", { id, path, mode });
    mockFiles.chmod(demoUser(id), path, mode);
  },
  async filesCancel(transfer: string): Promise<void> {
    if (real) return invoke("files_cancel", { transfer });
    demoCancelled.add(transfer);
  },
  async filesClose(id: string): Promise<void> {
    if (real) await invoke("files_close", { id });
  },

  /** Asks which files to upload; empty if cancelled. */
  async pickUploads(): Promise<LocalFile[]> {
    if (real) {
      const r = await open({ multiple: true, directory: false });
      return (r ?? []).map((path) => ({ path, name: baseName(path) }));
    }
    return new Promise((resolve) => {
      const input = Object.assign(document.createElement("input"), { type: "file", multiple: true });
      input.onchange = () => resolve(demoLocal([...(input.files ?? [])]));
      input.oncancel = () => resolve([]);
      input.click();
    });
  },

  /** Asks which folders to upload; empty if cancelled. Only in the app. */
  async pickUploadFolders(): Promise<LocalFile[]> {
    if (!real) return [];
    const r = await open({ multiple: true, directory: true });
    return (r ?? []).map((path) => ({ path, name: baseName(path) }));
  },

  /** Files dropped from a browser drag (the demo); in the app drops come from onFileDrop. */
  droppedFiles: (list: FileList): LocalFile[] => demoLocal([...list]),

  /** Asks where to save a download; null if cancelled. */
  async pickDownloadTarget(name: string): Promise<string | null> {
    if (real) return save({ defaultPath: name });
    return name;
  },

  /** Asks for a folder to download several things into; null if cancelled. */
  async pickDownloadFolder(): Promise<string | null> {
    if (real) {
      const r = await open({ directory: true, multiple: false });
      return typeof r === "string" ? r : null;
    }
    return "Загрузки";
  },

  /** Files dragged onto the window from the system, in the app. Returns how to stop listening. */
  async onFileDrop(cb: (e: { type: "enter" | "over" | "leave" } | { type: "drop"; files: LocalFile[] }) => void): Promise<() => void> {
    if (!real) return () => {};
    return getCurrentWebview().onDragDropEvent(({ payload: p }) => {
      if (p.type === "drop") cb({ type: "drop", files: p.paths.map((path) => ({ path, name: baseName(path) })) });
      else cb({ type: p.type });
    });
  },

  async secretSave(host: Host, kind: SecretKind, value: string): Promise<void> {
    if (api.usesCore(host.vaultId)) return invoke("secret_save", { vaultId: host.vaultId, hostId: host.id, kind, value });
    if (kind === "password") mockSecrets.hostPasswords.set(host.id, value);
    else if (host.auth.type === "key") mockSecrets.keyPassphrases.set(host.auth.keyId, value);
  },
};
