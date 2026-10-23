// Mirrors crates/detour-core/src/model.rs. Fields marked "mock" don't exist in
// the core yet and only drive the mockup.

export type VaultKind = "personal" | "team";

export type SyncBackend =
  | { type: "local" }
  | { type: "git"; remote: string; branch: string }
  | { type: "folder"; path: string };

export type SyncStatus = "synced" | "syncing" | "offline" | "conflict";

export interface Vault {
  id: string;
  name: string;
  kind: VaultKind;
  sync: SyncBackend;
  /** mock */
  syncStatus: SyncStatus;
  /** mock */
  lastSync: string;
  /** mock */
  conflicts: SyncConflict[];
}

/** Fields a teammate's synced version of a host may differ in. */
export type ConflictField = "label" | "address" | "port" | "user" | "group";

/** The same host was changed here and by a teammate before either synced. */
export interface SyncConflict {
  id: string;
  hostId: string;
  /** member whose version is in the storage */
  theirsBy: string;
  when: string;
  theirs: Partial<Pick<Host, ConflictField>>;
}

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
  /** Saved password for password auth; absent = ask on every connect. Stored encrypted in the vault. */
  password?: string;
  /** mock */
  os: Os;
  /** mock */
  online: boolean;
  /** mock: member id of whoever added it to a team vault */
  addedBy?: string;
  /** mock */
  lastConnected?: string;
}

export type Role = "owner" | "admin" | "member" | "readOnly";

export interface Member {
  id: string;
  vaultId: string;
  name: string;
  role: Role;
  fingerprint: string;
  /** mock */
  email: string;
  /** mock */
  status: "active" | "invited";
  /** mock */
  isYou?: boolean;
}

export interface SshKey {
  id: string;
  vaultId: string;
  name: string;
  algorithm: "ed25519" | "rsa-4096";
  fingerprint: string;
  publicKey: string;
  /** mock: stands in for the encrypted private key file */
  privateKey: string;
  /** The private key is protected with a passphrase. */
  hasPassphrase: boolean;
  /** Saved passphrase; absent = ask when the key is used. Stored encrypted in the vault. */
  passphrase?: string;
  createdAt: string;
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

export interface ActivityItem {
  id: string;
  vaultId: string;
  memberId: string;
  action: string;
  target: string;
  when: string;
}

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
