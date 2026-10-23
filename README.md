# Detour

Open-source SSH client for macOS, Windows and Linux with **serverless Teams**:
a team shares hosts, keys and snippets through storage it already owns
(a private Git repo or a shared folder). Everything is end-to-end encrypted
to each member's key, and every change is signed. No accounts, no Detour cloud.

> Status: early. The personal vault (synced between your devices), keys, SSH sessions, tunnels, SFTP file browsing and team vaults over a shared folder or a Git repository (SSH or HTTPS) work. CI tests every change and builds installers on macOS, Windows and Linux.

## Layout

```
crates/detour-core/   SSH, vaults, team sync, crypto — no UI dependencies
src-tauri/            Tauri 2 shell: thin commands that call detour-core
src/                  React + TypeScript UI (xterm.js terminal)
  lib/api.ts          the only UI ↔ backend boundary (falls back to mocks in a browser)
```

`detour-core` must never depend on Tauri, so the frontend can be swapped without
touching SSH, crypto or sync code.

## Development

Requirements: Node 20+, Rust (stable), and the platform's Tauri prerequisites
(Windows: Visual Studio Build Tools with the C++ workload, WebView2).

```bash
npm install
npm run dev          # UI only, in a browser at http://localhost:1420 (mock data)
npm run tauri dev    # desktop app with detour-core
cargo test --workspace
```

## Teams without a server

A team vault is a folder of signed, per-device, append-only logs, encrypted end to end. Access to every host, key and snippet is set per member and enforced by encryption, not by the client: a member never receives keys for what they may not see. Every client also checks that the storage hides, rolls back or forges nothing. Members join by exchanging Detour IDs; a recovery key is created with every team. Design and current limits: [docs/teams.md](docs/teams.md).

## Roadmap

- [x] UI mockup: hosts, terminal tabs, keys, snippets, teams
- [x] Real SSH sessions in detour-core (russh) streamed to xterm.js
- [x] Local encrypted vault (personal)
- [x] Known hosts / host key pinning
- [x] Team vaults over a shared folder (Detour ID, roles, conflicts, recovery key)
- [x] Per-record access enforced by encryption, own logins for team hosts, storage integrity checks
- [x] Team vaults over Git (SSH or HTTPS with a token), with Detour's own Git client
- [x] Tunnels: local (`-L`), remote (`-R`) and SOCKS5 (`-D`) port forwarding over a kept SSH connection
- [x] SFTP: browse and sort, upload and download files and whole folders with progress, speed and cancel, select several, rename, delete, permissions, a terminal in the current folder; nothing half-written or replaced on failure
- [x] Personal vault sync between your devices through a folder or a Git repository, new devices added with a one-time code
- [x] CI: tests on macOS, Windows and Linux on every push; installers for each from `v*` tags (draft release) or a manual run
