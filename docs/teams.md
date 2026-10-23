# Teams: design

Status: format 2 (per-record access enforced by encryption, storage integrity checks) is implemented in `crates/detour-core/src/team`; Git transport, rotation on hosts, snapshots and SSH certificates are planned.

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

Every member has one identity, kept in their personal vault:

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
- **Git:** embedded, no system `git` required (`gix` first, vendored `git2` as a fallback).
- **Later:** S3, WebDAV.

## What the storage can see

| Visible | Hidden |
|---|---|
| number of writing devices, when and how much they write | team and member names, roles, Detour IDs |
| which entries touch the same (random) record id | record types, names, addresses, logins |
| | passwords, keys, snippets, who has access to what |

## Personal vault sync

The personal vault becomes a team of one person with several devices, using the same format and transports.

## Plan

- Done: format 2 with per-record access, key drops, cutoffs, integrity checks, folder transport, recovery key.
- Embedded Git transport.
- Rotation on hosts after removal.
- Snapshots (every open replays all entries now: fine for thousands of changes, not for years).
- State fingerprint comparison between members.
- SSH certificate authority.
