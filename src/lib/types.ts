// Mirrors crates/detour-core/src/model.rs and team/*. Fields marked "mock"
// exist only for demo data.
import type { LangSetting } from "../i18n";

export type VaultKind = "personal" | "team";

export type SyncBackend =
  | { type: "local" }
  | { type: "git"; remote: string }
  | { type: "folder"; path: string };

export type SyncStatus = "synced" | "syncing" | "offline" | "conflict";

export type Role = "owner" | "admin" | "member";

/** Access to one team record; each level includes the ones before it (detour_core::team::state::Level). */
export type Level = "none" | "view" | "secret" | "edit";

export interface Vault {
  id: string;
  name: string;
  kind: VaultKind;
  sync: SyncBackend;
  syncStatus: SyncStatus;
  /** Unix ms of the last successful read of the storage. */
  lastSync?: number;
  conflicts: Conflict[];
  /** Team only: you are in the member list. */
  joined?: boolean;
  /** Team only: you were removed from the member list. */
  removed?: boolean;
  myRole?: Role;
  /** Team only: your member id. */
  me?: string;
  /** Team only: what is wrong with the storage or changes in it. */
  problems?: Problem[];
  /** Git team: local changes the repository doesn't have yet. */
  pending?: boolean;
  /** Team only: digest of the entries read, compared with other members. */
  stateFingerprint?: string;
  /** Git team: why the repository could not be reached last time. */
  remoteError?: import("./api").CoreError;
}

/** detour_core::team::ProblemView */
export type Problem = { authorName: string } & (
  | { kind: "rejected"; device: string; author: string; seq: number; reason: string }
  | { kind: "missing" | "forked" | "rolledBack"; device: string; author: string; seq: number }
  | { kind: "unknownFile"; path: string }
);

/** One member's access to one team record. */
export interface AccessEntry {
  vaultId: string;
  coll: Coll;
  record: string;
  member: string;
  level: Level;
}

export type Coll = "host" | "key" | "snippet" | "tunnel";

/** A team edit that could not be merged automatically (detour_core::team::ConflictView). */
export type Conflict =
  | { kind: "field"; coll: Coll; record: string; field: string; values: { value: unknown; author: string; authorName: string; at: number }[] }
  | { kind: "deleted"; coll: Coll; record: string; by: string; byName: string; at: number };

export type Auth = { type: "key"; keyId: string } | { type: "password" } | { type: "agent" };

export type Os = "ubuntu" | "debian" | "fedora" | "alpine" | "freebsd" | "macos" | "windows";

export interface Host {
  id: string;
  vaultId: string;
  group: string;
  label: string;
  address: string;
  port: number;
  user: string;
  auth: Auth;
  tags: string[];
  /** A password is saved in the vault (the value never reaches the UI). */
  hasPassword: boolean;
  /** Transient: a password just typed in, sent to the vault with the next save. */
  newPassword?: string;
  /** Unix ms of the last successful login. */
  lastConnected?: number;
  /** Team only: your access. */
  access?: Level;
  /** Team only: your own login, used instead of the shared one. */
  own?: OwnLogin;
  /** How the host is reached when not directly. */
  proxy?: Proxy | null;
  /** A proxy password is saved in the vault. */
  hasProxyPassword?: boolean;
  /** Transient: a proxy password just typed in, sent with the next save. */
  newProxyPassword?: string;
  /** mock: demo hosts only */
  os?: Os;
  /** mock: demo hosts only; real reachability is known only when connecting */
  online?: boolean;
}

/** detour_core::model::Proxy */
export type Proxy =
  | { type: "jump"; hostId: string }
  | { type: "socks5" | "http"; address: string; port: number; user: string };

/** A person's own login for a team host, kept in their personal vault. */
export interface OwnLogin {
  user: string;
  /** A key here is a personal key. */
  auth: Auth;
  hasPassword: boolean;
  /** Transient: a password just typed in. */
  newPassword?: string;
}

export interface Member {
  /** Signing key id */
  id: string;
  vaultId: string;
  name: string;
  role: Role;
  /** The team's recovery key, listed as an owner. */
  recovery: boolean;
  fingerprint: string;
  isYou: boolean;
}

export interface SshKey {
  id: string;
  vaultId: string;
  name: string;
  algorithm: "ed25519" | "rsa" | "ecdsa";
  bits?: number;
  fingerprint: string;
  publicKey: string;
  /** The private key is protected with a passphrase. */
  hasPassphrase: boolean;
  /** The passphrase is saved in the vault (otherwise asked when the key is used). */
  passphraseSaved: boolean;
  /** Unix ms */
  createdAt: number;
  /** Team only: your access. */
  access?: Level;
}

export interface Snippet {
  id: string;
  vaultId: string;
  name: string;
  command: string;
  /** Team only: your access. */
  access?: Level;
}

/**
 * local:   localhost:bindPort on this computer → via host → target (host:port as seen from the via host)
 * remote:  via host's bindPort → target on this computer (host:port as seen from here)
 * dynamic: SOCKS5 proxy on localhost:bindPort, traffic leaves through the via host
 */
export interface Tunnel {
  id: string;
  vaultId: string;
  name: string;
  kind: "local" | "remote" | "dynamic";
  bindPort: number;
  viaHostId: string;
  target?: string;
  /** mock */
  state: "off" | "starting" | "on" | "error";
  /** Why it failed or stopped. */
  error?: string;
  /** Connections going through it now. */
  connections?: number;
  /** Team only: your access. */
  access?: Level;
}

/** detour_core::team::ActivityView */
export type ActivityItem = { vaultId: string; authorName: string; isYou: boolean; at: number } & (
  | { kind: "created" }
  | { kind: "renamed"; name: string }
  | { kind: "added" | "changed" | "secretChanged" | "deleted"; coll: Coll; id: string; name: string }
  | { kind: "accessChanged"; coll: Coll; id: string; name: string; member: string; level: Level }
  | { kind: "memberAdded" | "memberChanged" | "memberRemoved"; member: string }
);

export interface Workspace {
  vaults: Vault[];
  hosts: Host[];
  members: Member[];
  keys: SshKey[];
  snippets: Snippet[];
  tunnels: Tunnel[];
  activity: ActivityItem[];
  access: AccessEntry[];
}

export interface Settings {
  theme: "dark" | "light" | "system";
  language: LangSetting;
  fontSize: number;
  scrollback: number;
  autoLockMinutes: number;
  /** Look for a new version at start. */
  autoUpdate: boolean;
  /**
   * Copy and paste in a terminal. `ssh`: as in a system terminal running ssh (right click
   * copies a selection or pastes; Ctrl+C copies a selection, otherwise interrupts; Ctrl+V pastes).
   * `termius`: Ctrl+Shift+C and Ctrl+Shift+V, Ctrl+C and Ctrl+V go to the server, right click opens a menu.
   */
  clipboard: "ssh" | "termius";
}
