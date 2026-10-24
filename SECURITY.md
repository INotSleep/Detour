# Security policy

## Reporting a vulnerability

Please don't open a public issue. Report it privately through [GitHub's security advisories](https://github.com/INotSleep/Detour/security/advisories/new) instead.

Include what an attacker can do, the steps to reproduce, and the Detour version (**Settings → About**). The report stays private until a fixed release is out, and you're credited in the release notes unless you'd rather not be.

## What counts

Detour assumes the storage, the network and other members' computers are hostile, so these are in scope:

- reading or changing team data without the access for it, including with a modified client;
- the storage forging, reordering, hiding or rolling back changes without members noticing;
- recovering vault contents without the master password;
- connecting to a server without the host key check stopping it;
- getting the app to install an update not signed by the project.

The limits listed under "What cryptography can't do" in [docs/teams.md](docs/teams.md) are known and not vulnerabilities, for example a member copying a secret they're allowed to read.

## Supported versions

Fixes go into the latest release. Detour updates itself, so please update before reporting.
