import type { Workspace } from "./types";
import { fakePrivateKey } from "./keys";

// Fake workspace for the mockup. Replaced piece by piece as features move into detour-core.
export const mockWorkspace: Workspace = {
  vaults: [
    { id: "personal", name: "Личное", kind: "personal", sync: { type: "local" }, syncStatus: "synced", lastSync: "только что", conflicts: [] },
    {
      id: "acme",
      name: "Acme Infra",
      kind: "team",
      sync: { type: "git", remote: "git@github.com:acme/detour-vault.git", branch: "main" },
      syncStatus: "synced",
      lastSync: "2 мин назад",
      conflicts: [],
    },
    {
      id: "homelab",
      name: "Homelab",
      kind: "team",
      sync: { type: "folder", path: "~/Syncthing/detour-homelab" },
      syncStatus: "conflict",
      lastSync: "1 ч назад",
      conflicts: [{ id: "c1", hostId: "h21", theirsBy: "m7", when: "1 ч назад", theirs: { address: "10.10.0.54", port: 2222 } }],
    },
  ],

  hosts: [
    { id: "h1", vaultId: "personal", group: "Дом", label: "laptop", address: "192.168.1.30", port: 22, user: "me", auth: { type: "key", keyId: "k1" }, tags: ["ubuntu"], os: "ubuntu", online: true, lastConnected: "сегодня" },
    { id: "h2", vaultId: "personal", group: "Дом", label: "nas", address: "192.168.1.10", port: 22, user: "admin", auth: { type: "password" }, tags: ["storage"], password: "nas-admin-2026", os: "freebsd", online: true, lastConnected: "5 дней назад" },
    { id: "h3", vaultId: "personal", group: "VPS", label: "vpn-fra", address: "203.0.113.17", port: 2222, user: "root", auth: { type: "key", keyId: "k1" }, tags: ["wireguard"], os: "debian", online: false, lastConnected: "3 дня назад" },

    { id: "h10", vaultId: "acme", group: "Production", label: "api-1", address: "10.0.1.11", port: 22, user: "deploy", auth: { type: "key", keyId: "k2" }, tags: ["api", "prod"], os: "ubuntu", online: true, addedBy: "m2", lastConnected: "вчера" },
    { id: "h11", vaultId: "acme", group: "Production", label: "api-2", address: "10.0.1.12", port: 22, user: "deploy", auth: { type: "key", keyId: "k2" }, tags: ["api", "prod"], os: "ubuntu", online: true, addedBy: "m2", lastConnected: "вчера" },
    { id: "h12", vaultId: "acme", group: "Production", label: "db-primary", address: "10.0.2.5", port: 22, user: "dba", auth: { type: "key", keyId: "k2" }, tags: ["postgres", "prod"], os: "debian", online: true, addedBy: "m3", lastConnected: "2 дня назад" },
    { id: "h13", vaultId: "acme", group: "Staging", label: "staging", address: "staging.acme.internal", port: 22, user: "deploy", auth: { type: "agent" }, tags: ["staging"], os: "alpine", online: true, addedBy: "m1", lastConnected: "сегодня" },
    { id: "h14", vaultId: "acme", group: "Staging", label: "ci-runner", address: "10.0.9.3", port: 22, user: "runner", auth: { type: "key", keyId: "k2" }, tags: ["ci"], os: "fedora", online: false, addedBy: "m4" },

    { id: "h20", vaultId: "homelab", group: "Rack", label: "proxmox", address: "10.10.0.2", port: 22, user: "root", auth: { type: "key", keyId: "k3" }, tags: ["hypervisor"], os: "debian", online: true, addedBy: "m6", lastConnected: "сегодня" },
    { id: "h21", vaultId: "homelab", group: "Rack", label: "pi-hole", address: "10.10.0.53", port: 22, user: "pi", auth: { type: "password" }, tags: ["dns"], os: "debian", online: true, addedBy: "m7" },
  ],

  members: [
    { id: "m1", vaultId: "acme", name: "Владислав", email: "vlad@acme.dev", role: "owner", fingerprint: "age1q8m…r4xz", status: "active", isYou: true },
    { id: "m2", vaultId: "acme", name: "Anna Kovalenko", email: "anna@acme.dev", role: "admin", fingerprint: "age1zt3…9wkd", status: "active" },
    { id: "m3", vaultId: "acme", name: "Max Petrenko", email: "max@acme.dev", role: "member", fingerprint: "age1v0p…c2ln", status: "active" },
    { id: "m4", vaultId: "acme", name: "Oleh Shevchuk", email: "oleh@acme.dev", role: "readOnly", fingerprint: "age1hh7…u8fa", status: "active" },
    { id: "m5", vaultId: "acme", name: "sam@acme.dev", email: "sam@acme.dev", role: "member", fingerprint: "", status: "invited" },
    { id: "m6", vaultId: "homelab", name: "Владислав", email: "vlad@acme.dev", role: "admin", fingerprint: "age1q8m…r4xz", status: "active", isYou: true },
    { id: "m7", vaultId: "homelab", name: "Dima", email: "dima@mail.example", role: "owner", fingerprint: "age1k2j…p0qe", status: "active" },
  ],

  keys: [
    { id: "k1", vaultId: "personal", name: "id_ed25519", algorithm: "ed25519", fingerprint: "SHA256:Xq3eT0n…v9Lk", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK9x3rQm0vJ2c8fQe1Yt7Zp4WbN5hL2sD6uG8kR1aXo me@laptop", privateKey: fakePrivateKey("id_ed25519"), hasPassphrase: true, createdAt: "14 мар 2026" },
    { id: "k2", vaultId: "acme", name: "acme-deploy", algorithm: "ed25519", fingerprint: "SHA256:b7RtWq1…Qm2c", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIM2pT6vH9cX1bR4nK7wQ3eY8uL5jF0aS2dG6hZ9oNiVt acme-deploy", privateKey: fakePrivateKey("acme-deploy"), hasPassphrase: false, createdAt: "9 янв 2026" },
    { id: "k3", vaultId: "homelab", name: "homelab-root", algorithm: "rsa-4096", fingerprint: "SHA256:Pz0wLk8…aH5s", publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQC7… homelab-root", privateKey: fakePrivateKey("homelab-root"), hasPassphrase: true, passphrase: "rack-key", createdAt: "30 ноя 2025" },
  ],

  snippets: [
    { id: "s1", vaultId: "acme", name: "Перезапуск API", command: "sudo systemctl restart acme-api && journalctl -u acme-api -n 50 -f" },
    { id: "s2", vaultId: "acme", name: "Место на дисках", command: "df -h" },
    { id: "s3", vaultId: "acme", name: "Нагрузка", command: "uptime" },
    { id: "s4", vaultId: "personal", name: "Обновить систему", command: "sudo apt update && sudo apt upgrade -y" },
  ],

  tunnels: [
    { id: "t1", vaultId: "acme", name: "Postgres prod", kind: "local", bindPort: 5432, viaHostId: "h10", target: "db-primary:5432", state: "off" },
    { id: "t2", vaultId: "acme", name: "Прокси через staging", kind: "dynamic", bindPort: 1080, viaHostId: "h13", state: "off" },
    { id: "t4", vaultId: "acme", name: "Демо фронтенда", kind: "remote", bindPort: 8080, viaHostId: "h13", target: "localhost:3000", state: "off" },
    { id: "t5", vaultId: "acme", name: "Метрики CI", kind: "local", bindPort: 9090, viaHostId: "h14", target: "localhost:9090", state: "off" },
    { id: "t3", vaultId: "personal", name: "Proxmox", kind: "local", bindPort: 8006, viaHostId: "h1", target: "10.10.0.2:8006", state: "off" },
  ],

  activity: [
    { id: "a1", vaultId: "acme", memberId: "m2", action: "добавила", target: "api-2", when: "2 мин назад" },
    { id: "a2", vaultId: "acme", memberId: "m3", action: "изменил", target: "Перезапуск API", when: "1 ч назад" },
    { id: "a3", vaultId: "acme", memberId: "m1", action: "пригласили", target: "sam@acme.dev", when: "вчера" },
    { id: "a4", vaultId: "acme", memberId: "m2", action: "обновила ключ", target: "acme-deploy", when: "3 дня назад" },
    { id: "a5", vaultId: "homelab", memberId: "m7", action: "изменил", target: "pi-hole", when: "1 ч назад" },
  ],
};
