import type { Workspace } from "./types";
import { fakePrivateKey } from "./keys";

const ago = (min: number) => Date.now() - min * 60_000;
const H = 60;
const D = 24 * H;

// Fake workspace for the mockup. Replaced piece by piece as features move into detour-core.
export const mockWorkspace: Workspace = {
  vaults: [
    { id: "personal", name: "Личное", kind: "personal", sync: { type: "local" }, syncStatus: "synced", lastSync: ago(0), conflicts: [] },
    {
      id: "acme",
      name: "Acme Infra",
      kind: "team",
      sync: { type: "folder", path: "D:\\Sync\\acme-vault" },
      syncStatus: "synced",
      lastSync: ago(2),
      conflicts: [],
      joined: true,
      myRole: "owner",
      canReveal: true,
      problems: 0,
    },
    {
      id: "homelab",
      name: "Homelab",
      kind: "team",
      sync: { type: "folder", path: "~/Syncthing/detour-homelab" },
      syncStatus: "conflict",
      lastSync: ago(60),
      joined: true,
      myRole: "admin",
      canReveal: true,
      problems: 0,
      conflicts: [
        {
          kind: "field",
          coll: "host",
          record: "h21",
          field: "address",
          values: [
            { value: "10.10.0.53", author: "m6", authorName: "Владислав", at: ago(70) },
            { value: "10.10.0.54", author: "m7", authorName: "Dima", at: ago(60) },
          ],
        },
        {
          kind: "field",
          coll: "host",
          record: "h21",
          field: "port",
          values: [
            { value: 22, author: "m6", authorName: "Владислав", at: ago(70) },
            { value: 2222, author: "m7", authorName: "Dima", at: ago(60) },
          ],
        },
      ],
    },
  ],

  hosts: [
    { id: "h1", vaultId: "personal", group: "Дом", label: "laptop", address: "192.168.1.30", port: 22, user: "me", auth: { type: "key", keyId: "k1" }, tags: ["ubuntu"], hasPassword: false, os: "ubuntu", online: true, lastConnected: ago(3 * H) },
    { id: "h2", vaultId: "personal", group: "Дом", label: "nas", address: "192.168.1.10", port: 22, user: "admin", auth: { type: "password" }, tags: ["storage"], hasPassword: true, os: "freebsd", online: true, lastConnected: ago(5 * D) },
    { id: "h3", vaultId: "personal", group: "VPS", label: "vpn-fra", address: "203.0.113.17", port: 2222, user: "root", auth: { type: "key", keyId: "k1" }, tags: ["wireguard"], hasPassword: false, os: "debian", online: false, lastConnected: ago(3 * D) },

    { id: "h10", vaultId: "acme", group: "Production", label: "api-1", address: "10.0.1.11", port: 22, user: "deploy", auth: { type: "key", keyId: "k2" }, tags: ["api", "prod"], hasPassword: false, os: "ubuntu", online: true, lastConnected: ago(D + 2 * H) },
    { id: "h11", vaultId: "acme", group: "Production", label: "api-2", address: "10.0.1.12", port: 22, user: "deploy", auth: { type: "key", keyId: "k2" }, tags: ["api", "prod"], hasPassword: false, os: "ubuntu", online: true, lastConnected: ago(D + 2 * H) },
    { id: "h12", vaultId: "acme", group: "Production", label: "db-primary", address: "10.0.2.5", port: 22, user: "dba", auth: { type: "key", keyId: "k2" }, tags: ["postgres", "prod"], hasPassword: false, os: "debian", online: true, lastConnected: ago(2 * D) },
    { id: "h13", vaultId: "acme", group: "Staging", label: "staging", address: "staging.acme.internal", port: 22, user: "deploy", auth: { type: "agent" }, tags: ["staging"], hasPassword: false, os: "alpine", online: true, lastConnected: ago(3 * H) },
    { id: "h14", vaultId: "acme", group: "Staging", label: "ci-runner", address: "10.0.9.3", port: 22, user: "runner", auth: { type: "key", keyId: "k2" }, tags: ["ci"], hasPassword: false, os: "fedora", online: false },

    { id: "h20", vaultId: "homelab", group: "Rack", label: "proxmox", address: "10.10.0.2", port: 22, user: "root", auth: { type: "key", keyId: "k3" }, tags: ["hypervisor"], hasPassword: false, os: "debian", online: true, lastConnected: ago(3 * H) },
    { id: "h21", vaultId: "homelab", group: "Rack", label: "pi-hole", address: "10.10.0.53", port: 22, user: "pi", auth: { type: "password" }, tags: ["dns"], hasPassword: false, os: "debian", online: true },
  ],

  members: [
    { id: "m1", vaultId: "acme", name: "Владислав", role: "owner", canReveal: true, recovery: false, fingerprint: "K7QM-2XPA-9DTE-WN4F", isYou: true },
    { id: "m2", vaultId: "acme", name: "Anna Kovalenko", role: "admin", canReveal: true, recovery: false, fingerprint: "R2TV-8HCE-QM4P-LX7A", isYou: false },
    { id: "m3", vaultId: "acme", name: "Max Petrenko", role: "member", canReveal: false, recovery: false, fingerprint: "WD3N-6KQY-T2MB-9PEC", isYou: false },
    { id: "m4", vaultId: "acme", name: "Oleh Shevchuk", role: "readOnly", canReveal: false, recovery: false, fingerprint: "HF8C-4ZRW-M7QN-2TLD", isYou: false },
    { id: "m5", vaultId: "acme", name: "Recovery key", role: "owner", canReveal: true, recovery: true, fingerprint: "J9PL-3QWE-X6RT-B4NM", isYou: false },
    { id: "m6", vaultId: "homelab", name: "Владислав", role: "admin", canReveal: true, recovery: false, fingerprint: "K7QM-2XPA-9DTE-WN4F", isYou: true },
    { id: "m7", vaultId: "homelab", name: "Dima", role: "owner", canReveal: true, recovery: false, fingerprint: "P4XC-7NMQ-2LWA-DE8K", isYou: false },
  ],

  keys: [
    { id: "k1", vaultId: "personal", name: "id_ed25519", algorithm: "ed25519", fingerprint: "SHA256:Xq3eT0n…v9Lk", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK9x3rQm0vJ2c8fQe1Yt7Zp4WbN5hL2sD6uG8kR1aXo me@laptop", hasPassphrase: true, passphraseSaved: false, createdAt: Date.UTC(2026, 2, 14) },
    { id: "k2", vaultId: "acme", name: "acme-deploy", algorithm: "ed25519", fingerprint: "SHA256:b7RtWq1…Qm2c", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIM2pT6vH9cX1bR4nK7wQ3eY8uL5jF0aS2dG6hZ9oNiVt acme-deploy", hasPassphrase: false, passphraseSaved: false, createdAt: Date.UTC(2026, 0, 9) },
    { id: "k3", vaultId: "homelab", name: "homelab-root", algorithm: "rsa", bits: 4096, fingerprint: "SHA256:Pz0wLk8…aH5s", publicKey: "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAACAQC7… homelab-root", hasPassphrase: true, passphraseSaved: true, createdAt: Date.UTC(2025, 10, 30) },
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
    { vaultId: "acme", authorName: "Anna Kovalenko", isYou: false, at: ago(2), kind: "added", coll: "host", id: "h11" },
    { vaultId: "acme", authorName: "Max Petrenko", isYou: false, at: ago(60), kind: "changed", coll: "snippet", id: "s1" },
    { vaultId: "acme", authorName: "Владислав", isYou: true, at: ago(D), kind: "memberAdded", member: "Oleh Shevchuk" },
    { vaultId: "acme", authorName: "Anna Kovalenko", isYou: false, at: ago(3 * D), kind: "changed", coll: "key", id: "k2" },
    { vaultId: "homelab", authorName: "Dima", isYou: false, at: ago(60), kind: "changed", coll: "host", id: "h21" },
  ],
};

/** Secrets of the demo data; the real ones live only in detour-core. */
export const mockSecrets = {
  hostPasswords: new Map<string, string>([["h2", "nas-admin-2026"]]),
  keyPassphrases: new Map<string, string>([["k3", "rack-key"]]),
  privateKeys: new Map<string, string>([
    ["k1", fakePrivateKey("id_ed25519")],
    ["k2", fakePrivateKey("acme-deploy")],
    ["k3", fakePrivateKey("homelab-root")],
  ]),
};
