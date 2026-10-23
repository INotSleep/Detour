// Mirrors crates/detour-core/src/model.rs and team/*. Fields marked "mock"
// exist only for demo data.

export type VaultKind = "personal" | "team";

export type SyncBackend =
  | { type: "local" }
  | { type: "git"; remote: string; branch: string }
  | { type: "folder"; path: string };

export type SyncStatus = "synced" | "syncing" | "offline" | "conflict";

export type Role = "owner" | "admin" | "member" | "readOnly";

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
  /** Team only: you may view and export secrets. */
  canReveal?: boolean;
  /** Team only: log entries that failed verification. */
  problems?: number;
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
  /** mock: demo hosts only */
  os?: Os;
  /** mock: demo hosts only; real reachability is known only when connecting */
  online?: boolean;
}

export interface Member {
  /** Signing key id */
  id: string;
  vaultId: string;
  name: string;
  role: Role;
  canReveal: boolean;
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
}

export interface Snippet {
  id: string;
  vaultId: string;
  name: string;
  command: string;
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
  /** mock */
  error?: string;
}

/** detour_core::team::ActivityView */
export type ActivityItem = { vaultId: string; authorName: string; isYou: boolean; at: number } & (
  | { kind: "created" }
  | { kind: "renamed"; name: string }
  | { kind: "added" | "changed" | "deleted"; coll: Coll; id: string }
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
}

export interface Settings {
  theme: "dark" | "light" | "system";
  fontSize: number;
  scrollback: number;
  autoLockMinutes: number;
}
