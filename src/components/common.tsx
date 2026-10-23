import { FolderSync, GitBranch, HardDrive, Server } from "lucide-react";
import type { Host, Member, Role, SyncBackend, SyncStatus, Vault } from "../lib/types";

const palette = ["#7a8cff", "#4cc38a", "#e5a54b", "#c08bff", "#ef6461", "#3fb7c9", "#e58bb8"];

function hashColor(s: string) {
  return palette[[...s].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7) % palette.length];
}

export function HostIcon({ host, size = 30 }: { host: Host; size?: number }) {
  return (
    <span className="host-icon" style={{ width: size, height: size }}>
      <Server size={size * 0.5} />
      {host.online !== undefined && <span className={`host-status ${host.online ? "on" : "off"}`} />}
    </span>
  );
}

export function VaultAvatar({ vault, size = 22 }: { vault: Vault; size?: number }) {
  const color = vault.kind === "personal" ? "var(--text-2)" : hashColor(vault.id);
  return (
    <span className="vault-avatar" style={{ width: size, height: size, fontSize: size * 0.48, color, background: `color-mix(in srgb, ${color} 18%, transparent)` }}>
      {vault.name.slice(0, 1).toUpperCase()}
    </span>
  );
}

export function Avatar({ member, size = 28 }: { member: Member; size?: number }) {
  const invited = member.recovery;
  const color = hashColor(member.id + member.name);
  const initials = member.name
    .split(/[\s@.]/)
    .filter(Boolean)
    .slice(0, 2)
    .map((p) => p[0]!.toUpperCase())
    .join("");
  return (
    <span
      className={`avatar ${invited ? "invited" : ""}`}
      style={{ width: size, height: size, fontSize: size * 0.38, ...(invited ? {} : { color, background: `color-mix(in srgb, ${color} 20%, transparent)` }) }}
    >
      {initials}
    </span>
  );
}

export function SyncIcon({ sync, size = 14 }: { sync: SyncBackend; size?: number }) {
  if (sync.type === "git") return <GitBranch size={size} />;
  if (sync.type === "folder") return <FolderSync size={size} />;
  return <HardDrive size={size} />;
}

export function syncTarget(sync: SyncBackend): string {
  if (sync.type === "git") return sync.remote;
  if (sync.type === "folder") return sync.path;
  return "";
}

export const syncStatusText: Record<SyncStatus, string> = {
  synced: "Синхронизировано",
  syncing: "Синхронизация…",
  offline: "Хранилище недоступно",
  conflict: "Конфликт",
};

export const roleLabel: Record<Role, string> = {
  owner: "Владелец",
  admin: "Админ",
  member: "Участник",
};

export function authLabel(host: Host, keyName?: string): string {
  if (host.auth.type === "key") return keyName ?? "Ключ";
  if (host.auth.type === "agent") return "ssh-agent";
  return "Пароль";
}
