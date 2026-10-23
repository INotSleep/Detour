# Detour

Open-source SSH client for macOS, Windows and Linux with **serverless Teams**:
a team shares hosts, keys and snippets through storage it already owns
(a private Git repo or a shared folder). Everything is end-to-end encrypted
to each member's key, and every change is signed. No accounts, no Detour cloud.

> Status: UI mockup. SSH sessions are simulated; nothing touches the network yet.

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
cargo test -p detour-core
```

## Teams without a server (planned design)

1. A team vault lives in storage the team chooses: a private Git repo first, shared folder (Syncthing, SMB) next.
2. Each member has a keypair. Joining = exchanging public keys; there is no sign-up.
3. Vault entries are encrypted with [age](https://age-encryption.org) to all members' keys and signed by the author.
4. Removing a member re-encrypts the vault without their key and flags shared secrets they could read for rotation.

## Roadmap

- [x] UI mockup: hosts, terminal tabs, keys, snippets, teams
- [ ] Real SSH sessions in detour-core (russh) streamed to xterm.js
- [ ] Local encrypted vault (personal)
- [ ] Known hosts / host key pinning
- [ ] Team vaults over Git
- [ ] Port forwarding, SFTP
- [ ] CI builds for macOS, Windows, Linux
