# Teams: design

Status: stage 3a is implemented (`crates/detour-core/src/team`, `identity.rs`); 3b to 3e are planned.

## Goals

- No Detour server. A team vault lives in storage the team already has: a Git repository or a synced folder (Syncthing, Dropbox, OneDrive, SMB); S3/WebDAV later.
- End-to-end encrypted: the storage, the Git host and outsiders see no names, addresses, logins or secrets.
- Works offline; edits made at the same time by different people merge.
- Every change is attributable and cannot be forged or reordered.
- As few runtime dependencies as possible (no system `git`).

## Identities: Detour ID

Every member has one identity, kept in their personal vault:

- X25519 key (age) for encryption,
- Ed25519 key for signatures.

The public half is the member's **Detour ID**, a single copyable string. Joining a team:

1. The newcomer copies their Detour ID and sends it to an admin over any channel.
2. The admin pastes it into "Add member" and picks a role and permissions.
3. The newcomer adds the storage (repository URL or folder) and finds themselves in the member list.

Both sides see a short fingerprint (a few words) to compare out of band, so a swapped ID in transit is detectable. Access to the Git repository itself is granted on the Git host as usual.

Several devices of one person share the identity through personal vault sync (see the last section).

## Storage layout

```
<vault>/
  vault.json                     format version, vault id (no names)
  members/                       membership log, signed by owners/admins
  keys/epoch-0003.age            vault key, age-encrypted to every member and the recovery key
  ops/<device-id>/000184.age     this device's changes, append-only
  snapshots/0012.age             periodic compacted state
```

**Why per-device logs:** two devices never write the same file, so neither Git nor Syncthing ever sees a conflict at the storage level (a single encrypted file would conflict on every concurrent edit and can't be merged). All conflicts become semantic and are resolved inside Detour. The logs also give history and the activity feed for free.

## Encryption

- The vault has a symmetric **epoch key**, stored in `keys/` encrypted with age to every member (and to the recovery key). Operations are encrypted with the epoch key, so their size does not grow with the team.
- Removing a member starts a new epoch: the new key is given only to the remaining members. Old data stays readable to the removed person (they already had it), hence the rotation flow below.
- New members receive the current epoch key; older epoch keys are chained (each epoch key also encrypts the previous one) so history stays readable to current members.

## Integrity and ordering

- Every operation is signed by its author and carries a per-device sequence number and the hash of that device's previous operation. Storage, the Git host or another member cannot forge, alter or reorder it.
- Files can still be deleted by whoever controls the storage. Gaps in a device's chain are detected and shown ("missing changes from Max").
- Timestamps are hybrid logical clocks, so ordering survives skewed device clocks.

## Membership, roles and permissions

The membership log starts with a genesis entry signed by the creator (the first owner). Every client validates each operation against the author's rights at that point in the log; invalid operations are ignored and reported.

Roles control **changes**:

| Role | Edit hosts, keys, snippets | Manage members | Manage admins |
|---|---|---|---|
| Owner | yes | yes | yes |
| Admin | yes | yes | no |
| Member | yes | no | no |
| Read-only | no | no | no |

A separate **"Can reveal secrets"** permission controls **seeing** secrets, independent of the role:

- With it: view and copy passwords, export private keys.
- Without it: the member *uses* hosts, keys and passwords to connect, but Detour never displays, copies or exports them, and the core refuses to hand them to the UI.

Honest limit: without a server this is enforced by the Detour client. A member who can connect necessarily decrypts the secret on their machine, so a modified client could extract it. Two ways to give a real guarantee:

- **SSH certificates (planned, preferred for keys).** The team has a CA key held only by owners/admins. Members log in with their *own* key plus a short-lived certificate signed by the team CA; the shared private key never reaches them. Detour sets `TrustedUserCAKeys` on hosts once (from an admin's machine) and renews certificates whenever an admin's Detour is online. Removing a member = no renewal + a revocation list pushed to hosts.
- **Collections (planned).** A subset of entries (e.g. "Production") encrypted with its own key, given only to some members.

Shared passwords can only ever be client-protected; the UI should nudge teams towards keys/certificates.

## Conflicts

- Different fields of the same record edited concurrently merge automatically.
- The same field edited concurrently: the later change (by HLC) wins provisionally, and Detour shows the conflict ("yours / Dima's") until someone picks a side; the choice is written as a new operation.
- Delete vs. edit: delete wins if the edit happened before it, otherwise it is a conflict.

## Removing a member and rotating secrets

When a member is removed, Detour lists what they could access (hosts, keys, passwords) and offers to rotate it:

1. generate a new key,
2. over SSH, replace it in `authorized_keys` on every affected host,
3. remove the old key,
4. list passwords that must be changed by hand.

## Recovery key (mandatory)

Creating a team generates a recovery key. The team is created only after the owner confirms they saved it (download/print, then re-enter part of it). The recovery key is an extra recipient of every epoch key and can act as an owner to restore membership if all owners lose their devices. Detour also nudges teams to have a second owner or admin.

## Transports

- **Folder:** plain file reads/writes plus a file watcher.
- **Git:** embedded, no system `git` required. Evaluate `gix` (pure Rust) for fetch/push first; fall back to `git2` with a vendored, statically linked libgit2. Git auth uses keys from the Detour vault or ssh-agent (SSH) or a token (HTTPS).
- **Later:** S3, WebDAV.

## What the storage can see

| Visible | Hidden |
|---|---|
| number of writing devices | host names, addresses, logins |
| when and how many changes | passwords, keys, snippets |
| members' public keys | member names and roles |

## Personal vault sync

The personal vault becomes a team of one person with several devices, using the same format and transports. One mechanism covers both Teams and PC ↔ laptop sync.

## Stage 3 plan

- **3a.** Format, Detour ID, epoch keys, signed per-device logs, merge and conflicts, folder transport, recovery key, "can reveal secrets" (client-enforced).
- **3b.** Embedded Git transport.
- **3c.** Member removal with automatic key rotation on hosts.
- **3d.** Collections.
- **3e.** SSH certificate authority.

## Stage 3a: what exists and its limits

Implemented: Detour ID, recovery key (mandatory at creation), signed per-device logs with hash chains, epoch keys (new epoch on member removal), replay with role checks, field-level merge with conflicts (field and edit-vs-delete), "can reveal secrets" (client-enforced), folder transport, the Tauri commands and the UI.

Known limits, to address later:

- **No snapshots yet.** Every open replays all logs; fine for thousands of changes, not for years of history.
- **Backdated entries.** A removed member could still sign an entry with a forged earlier timestamp using an old epoch key, and replay would accept it as made before the removal. Fix: require each entry to reference (via `seen`) the latest membership change its author could know about, and reject entries from members whose removal they had seen.
- **Polling, not watching.** The app re-reads team folders every 20 seconds and on "Sync".
- **Root of trust is first use.** A joining member confirms the creator fingerprint once; later owners are trusted through the signed member log.
