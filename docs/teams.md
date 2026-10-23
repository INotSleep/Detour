# Teams: design

Status: format 2 (per-record access enforced by encryption, storage integrity checks), the folder and Git transports and personal vault sync are implemented in `crates/detour-core/src/team` and `personal.rs`; rotation on hosts, snapshots and SSH certificates are planned.

## Goals

- No Detour server. A team vault lives in storage the team already has: a Git repository or a synced folder (Syncthing, Dropbox, OneDrive, SMB); S3/WebDAV later.
- End-to-end encrypted: the storage, the Git host and outsiders see no names, addresses, logins or secrets.
- **Every permission is enforced by cryptography, assuming the member's Detour is already compromised** and does whatever its owner wants. A member never receives a key or plaintext they have no right to. Nothing relies on "the UI hides it" or "the core refuses".
- The storage is untrusted too: it can't forge, reorder, hide or roll back changes without every client noticing.
- Works offline; edits made at the same time by different people merge.
- As few runtime dependencies as possible (no system `git`).

What cryptography can't do, and Detour doesn't pretend to:

- Anyone allowed to read something can copy it and pass it on.
- Connecting with a secret means having it: "may connect with the shared password" and "may see the shared password" are one right.
- Someone removed keeps what they already had. Detour lists it so it can be changed (see "Removing access").

## Identities: Detour ID

Every member has one identity, kept in their personal vault and shared by all of their devices (see "Personal vault sync"):

- X25519 key (age) for encryption,
- Ed25519 key for signatures.

The public half is the member's **Detour ID**, a single copyable string (`detour1…`). Joining a team:

1. The newcomer copies their Detour ID and sends it to an admin over any channel.
2. The admin pastes it into "Add member", picks a role and the default access.
3. The newcomer adds the storage (folder or repository) and confirms the creator's fingerprint once.

Both sides see a short fingerprint to compare out of band, so a swapped ID in transit is detectable.

## Roles and access

**Team roles** decide who manages the team:

| Role | Manage members | Manage admins and owners | Grant and revoke access |
|---|---|---|---|
| Owner | yes | yes | yes, up to their own access |
| Admin | yes | no | yes, up to their own access |
| Member | no | no | no |

**Access** is set per record (host, key, snippet, tunnel) and per member. Levels are ordered:

| Level | What the member physically receives | What they can do |
|---|---|---|
| None | nothing; they don't learn the record exists | |
| View | the record's metadata key | see name, address, port, login; connect **with their own credentials** (a personal key, password or agent the member adds themselves) |
| Secret | plus the record's secret key | connect with the shared password or key, view and copy it, export the private key |
| Edit | the same keys as Secret | change the record, its shared secret, delete it |

Snippets and tunnels have no secret part: View or Edit.

Why Edit includes Secret: whoever can change a host's address could point it at their own server and collect the shared password from the next person who connects. Making every editor a secret holder removes that attack instead of patching it.

A **matrix** (records × members) shows and sets every member's level on every record the viewer can see. Granting a host's Secret also grants Secret on the team key it uses; the UI says when that key opens other hosts too.

Limits that follow from the cryptography rather than from rules:

- You can only grant or revoke up to the level you hold yourself: granting means handing over keys you have.
- A record's access list is readable by everyone who can view the record, because every reader must check that an edit's author was allowed to make it.

Detour never changes accounts on servers (no user creation, no `authorized_keys` edits): commands differ per system and the result can't be predicted. "Own credentials" are whatever the member already has on that server.

## Encryption

Two kinds of keys:

- **Team key** (per epoch, symmetric): team name, member list and roles. Every member has it. A new epoch starts when a member is removed.
- **Record keys** (per record, per part, per generation, symmetric): every record has a *meta* key (View) and, for hosts and keys, a *secret* key (Secret). A new generation starts when someone loses access.

Keys reach members as **key drops**: age-encrypted to the recipient's Detour ID, carrying any number of team and record keys. age doesn't reveal recipients, so the storage can't tell who got what. A client tries every drop once and caches the result.

Each log entry is signed and holds encrypted **parts**; a part names its scope (team, or a random record id with meta/secret) and the key generation, and is encrypted with that key. A member without the key can't decrypt the part, and can't tell what kind of record it is.

## Storage layout

```
<vault>/
  vault.json                     format, vault id, creator's Detour ID
  ops/<device-id>/00000001.json  this device's entries, append-only, signed
```

Per-device logs: two devices never write the same file, so neither Git nor Syncthing ever sees a conflict at the storage level. All conflicts are semantic and resolved inside Detour.

## Integrity: nothing can be forged, reordered, hidden or rolled back silently

Every client checks the whole storage on every read:

1. **Every file is accounted for.** It is `vault.json`, an entry at its exact path, or known sync-tool debris (temp files, `.stfolder`, conflict copies). Anything else is reported. A conflict copy that is a valid entry with different content is reported as two versions of one entry.
2. **Signatures and chains.** Each entry is signed by its author and carries its device's sequence number and the hash of that device's previous entry. Gaps, foreign entries in a device's directory and changed bytes are reported.
3. **Cross references.** Each entry lists the head (sequence number and hash) of every device its author had read. A referenced entry that is missing means the storage hides it; a different hash means two versions of one entry exist.
4. **No rollback.** The client remembers, in the personal vault, the head of every device it has read. Storage showing an older head or a different entry at a known position is reported.
5. **Acknowledgements.** A client that has read new entries from others and wrote nothing for a day writes an empty, signed acknowledgement, so others' entries get referenced and can't be dropped unnoticed.

Honest limit: with a single untrusted storage and no other channel, the storage can show a member a consistent *old* state and withhold only the newest entries nobody has referenced yet (a proven bound for systems without a trusted server). What it can't do is hide anything someone has acknowledged, roll a client back, or show two members different histories without that showing up as soon as one of them reads an entry from the other. Comparing a short state fingerprint between members, or a second storage, closes the rest (planned).

## Authorization on replay

Entries are replayed in a total order (hybrid logical clock, author, device, sequence). Every reader checks each change against its author's role and access at that point; invalid changes are ignored and reported.

A removal or downgrade records how far the remover had read each of the target's devices (a *cutoff*). Any entry by the target beyond the cutoff is judged with the reduced rights, wherever its timestamp puts it. A removed member can't backdate changes, and a new device of theirs gets nothing.

## Removing access

Removing a member or lowering their access starts a new team epoch or record key generation, given only to those who keep access. The admin rotates the keys they hold; records they can't see are rotated automatically by the next client that holds them, before it writes anything new there.

Planned: list what the person could see (hosts, keys, passwords) and offer to change it: generate a new key, replace it in `authorized_keys` on affected hosts over SSH, list passwords to change by hand.

## Conflicts

- Different fields of the same record edited concurrently merge automatically.
- The same field edited concurrently: the later change (by HLC) wins provisionally, and Detour shows the conflict until someone picks a side; the choice is a new entry.
- Delete vs. edit: delete wins if the edit happened before it, otherwise it is a conflict.

## Recovery key (mandatory)

Creating a team generates a recovery key. The team is created only after the owner confirms they saved it. The recovery key is an owner with Edit on every record created by an honest client, and can make a new owner if all owners lose their devices.

## Transports

- **Folder:** plain file reads and writes; the app re-reads every 20 seconds and on "Sync".
- **Git (SSH or HTTPS):** Detour's own Git client, no system `git` and no C libraries (see below).
- **Later:** S3, WebDAV.

## Git transport

Why not a library: `gitoxide` can't push yet, and `git2` brings libgit2, libssh2 and OpenSSL as C code. A team vault needs very little of Git, so Detour speaks it itself (`team/git`):

- **Objects:** blobs, trees, one commit per push with a neutral author (`Detour <detour@invalid>`), so the host learns nothing about who wrote what.
- **Packs:** reads packs with deltas, writes packs of whole objects.
- **Protocol:** smart protocol v0 to `git-upload-pack` / `git-receive-pack`, split into a request and a response so one implementation serves both transports.
- **SSH:** an exec channel of Detour's own SSH client. The server key is pinned like any host's; the login is a personal key or ssh-agent. One connection per server and login is kept open and reused for every command (a new channel each time), closed after 5 minutes unused and when the vault locks, so a sync costs a channel, not a handshake.
- **HTTPS:** smart HTTP (`GET info/refs`, `POST git-upload-pack` / `git-receive-pack`) with a personal access token as the password. TLS is rustls with ring, certificates are checked against the system's store (company proxies with their own CA work), `HTTPS_PROXY` is honoured. Plain `http://` is refused except to this machine. Each member's token is theirs and lives only in their personal vault; a rejected token can be replaced from the team page.

A leaked Git credential gives access to the storage, not to the team's data: the storage is untrusted anyway, and every change to it is verified.

A Git team is a local copy (`<data>/teams/<id>/files`) in the folder layout, plus what the remote branch held at the last sync. The folder transport's checks run on it unchanged. Mirroring follows from "the log is the truth, Git is only a carrier":

- **Pull** adds files the client doesn't have. It never replaces or deletes one: a different version from the remote is kept beside the original (`*.git-conflict-<id>.json`) and reported as a second version of that entry; a file the remote dropped stays.
- **Push** commits exactly the local files on top of the remote head. Whatever the remote dropped or changed is put back for everyone. A push rejected because someone else pushed first pulls and tries again; per-device files never conflict.
- **Refused:** file names outside `[A-Za-z0-9._-]`, deeper than the vault layout, links and submodules.

The app pulls and pushes on "Sync", every 20 seconds, and right after every edit. Network work never holds the vault lock.

Not yet: signing in with GitHub (device flow through a registered GitHub App) instead of pasting a token.

## What the storage can see

| Visible | Hidden |
|---|---|
| number of writing devices, when and how much they write | team and member names, roles, Detour IDs |
| which entries touch the same (random) record id | record types, names, addresses, logins |
| | passwords, keys, snippets, who has access to what |

## Personal vault sync

One person's devices share their personal vault through a folder or a Git repository, in the same format, with the same transports and the same integrity checks as a team. The synced vault has a single member: the person's identity, which every one of their devices holds. `vault.json` marks it as personal, so it can't be joined as a team and a team can't be joined as one.

What syncs: hosts with their passwords, keys with private parts and passphrases, snippets, tunnels, pinned server keys, own logins for team hosts, and the list of teams. What stays on each device: login times, where a team sits on this device, team read positions, the sync settings and the device's master password.

**Teams on other devices.** Each joined team is a record with its id, name, creator and, for a team in Git, its repository, login and token (secret). Another device of the same person joins a team in Git by itself (it is the same member, and the creator was confirmed when the team was first joined); a team in a folder waits for its folder on that device, which must hold that very team. Leaving a team on one device leaves it on all of them; its storage is untouched. Pinned server keys and own logins have no ids of their own; their record ids are keyed with the identity, so the storage can't guess endpoints from them.

Each device keeps its own vault file. A sync is a three-way merge against the records as of the last sync: what this device changed since then is written to the synced vault field by field, then the synced records become this device's records. Concurrent edits of different fields both survive; the same field resolves to the later change.

A folder is synced on every save and every 20 seconds; a repository is pulled and pushed every 20 seconds and pushed after every edit.

**Adding a device.** An existing device writes the identity to `pair/<locator>-<expiry>.age`, sealed with age's scrypt recipient under a one-time code `XXXX-XXXX-XXXX-XXXX` (Crockford base32: 20 bits name the file, 60 bits are the secret). The new device opens it with the code, checks that the identity is the vault's creator, and from then on is that person. The storage never sees the code; it can't open the offer without brute-forcing 60 bits through scrypt, and it can't plant an offer of its own: it doesn't know the code. Offers expire after an hour and any device deletes expired ones. Codes are typed between one's own devices, so no other channel is involved.

**Recovery.** The identity's recovery key, shown when sync is turned on and any time later, works in place of a code, so the vault opens on a new device when no other device is left.

A device that is already in teams keeps its own identity and can't join another person's synced vault: its teams know it by that identity.

## Plan

- Done: format 2 with per-record access, key drops, cutoffs, integrity checks, folder transport, recovery key.
- Done: Git transport over SSH (one kept connection) and HTTPS (token).
- Done: personal vault sync between one person's devices, pairing codes, recovery key.
- Signing in with GitHub (device flow).
- Rotation on hosts after removal.
- Snapshots (every open replays all entries now: fine for thousands of changes, not for years).
- State fingerprint comparison between members.
- SSH certificate authority.
