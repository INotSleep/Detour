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

export type Coll = "host" | "key" | "snippet" | "tunnel" | "proxy" | "group";

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
  /** A saved proxy of the same vault the host is reached through; used before `proxy`. */
  proxyId?: string | null;
  /** Its place in its group; hosts without one come after, as they are. */
  order?: number | null;
  /** The server may use your keys while you're connected (`ssh -A`). */
  forwardAgent?: boolean;
  /** mock: demo hosts only */
  os?: Os;
  /** mock: demo hosts only; real reachability is known only when connecting */
  online?: boolean;
}

/** detour_core::model::Proxy */
export type Proxy =
  | { type: "jump"; hostId: string }
  | { type: "socks5" | "http"; address: string; port: number; user: string };

/** detour_core::model::ProxyKind */
export type ProxyKind =
  | { type: "socks5" | "http"; address: string; port: number; user: string }
  | {
      type: "wireguard";
      /** The peer's host:port. */
      endpoint: string;
      publicKey: string;
      addresses: string[];
      dns: string[];
      mtu?: number | null;
      allowedIps: string[];
      keepalive?: number | null;
    }
  | {
      type: "vless";
      address: string;
      port: number;
      security: "none" | "tls" | "reality";
      sni: string;
      alpn: string[];
      publicKey: string;
      shortId: string;
      pinnedCert: string;
      transport: "tcp" | "xhttp";
      flow: string;
      /** VLESS Encryption (mlkem768x25519plus...); empty for none. */
      encryption?: string;
      path: string;
      host: string;
      mode: string;
      extra: string;
      params: Record<string, string>;
    };

export type ProxyType = ProxyKind["type"];
export type ProxySecret = "password" | "privateKey" | "presharedKey" | "uuid";

/** A saved proxy hosts of the same vault go through (detour_core::model::ProxyView). */
export interface SavedProxy {
  id: string;
  vaultId: string;
  name: string;
  kind: ProxyKind;
  /** Secrets saved in the vault (their values never reach the UI). */
  saved: ProxySecret[];
  /** Transient: secrets just typed in, sent with the next save; empty removes one. */
  newSecrets?: Partial<Record<ProxySecret, string>>;
  /** Team only: your access. */
  access?: Level;
}

/** A group of hosts with its place among the others (detour_core::model::Group).
 *  Hosts name their group; one without a record comes after those with one. */
export interface Group {
  id: string;
  vaultId: string;
  name: string;
  order: number;
  /** A saved proxy of the same vault for its hosts that have none of their own. */
  proxyId?: string | null;
  /** Team only: your access. */
  access?: Level;
}

/** A person's own grouping of a team's hosts ("Mine"), kept in their personal vault. */
export interface MyGroup {
  id: string;
  name: string;
  /** Host ids, in order. */
  hosts: string[];
}
export interface Layout {
  groups: MyGroup[];
}

/** A saved proxy of some vault: this person's own or a team's. */
export interface ProxyRef {
  vault: string;
  id: string;
}

/** This person's own settings of a group (`<vault>/team:<name>` or `<vault>/mine:<id>`), used before the host's. */
export interface GroupPrefs {
  key: string;
  proxy?: ProxyRef | null;
  user?: string | null;
  /** A key here is a personal key. */
  auth?: Auth | null;
  hasPassword: boolean;
  /** Transient: a password just typed in; empty removes the saved one. */
  newPassword?: string;
}

/** A person's own login for a team host, kept in their personal vault. */
export interface OwnLogin {
  user: string;
  /** A key here is a personal key. */
  auth: Auth;
  hasPassword: boolean;
  /** Used to log in; off, it is kept while the shared login is used instead. */
  active: boolean;
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
  proxies: SavedProxy[];
  groups: Group[];
  /** This person's own grouping of each team's hosts, by team id. */
  layouts: Record<string, Layout>;
  groupPrefs: GroupPrefs[];
  activity: ActivityItem[];
  access: AccessEntry[];
}

export interface Settings {
  /** The app's theme: a theme id, or "system" for Detour Dark or Light as the system is. */
  theme: string;
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
  /** Colour the prompt, addresses, paths, sizes and status words in output that has no colours of its own. */
  highlight: boolean;
  /** What "Edit" opens a file in; null: the app the system uses for it. */
  editor: { name: string; path: string } | null;
  /** The sidebar shows only icons. */
  sidebarCollapsed: boolean;
  /** The terminals' theme: a theme id, or "app" for the app's. */
  terminalTheme: string;
  /** Themes made or imported on this device. */
  customThemes: import("./themes").Theme[];
}
