# Licensing summary

Short version for contributors and packagers. The full discussion with alternatives and sources is kept in the project's design docs ("Licensing and Contributions: Complete Summary"). Not legal advice; the CLA text and the Charm exception still need lawyer review.

## Who gets which license

| Area | License |
|---|---|
| Floor, Shell, World, Dream Mode, Settings, Launcher, kernel crates | GPL-3.0-or-later |
| `livspeil-node-api`, Charm SDK (Python module, IPC/WASM interface files) | LGPL-3.0-or-later (a switch to MPL-2.0 for Rust crates is under evaluation because of Rust static linking) |
| Livkit (UI toolkit) | GPL-3.0-or-later for now; may loosen to MPL-2.0 later via the CLA |
| Future server components (store, sync) | AGPL-3.0 (not yet used) |
| Docs and specs | CC BY 4.0 |
| Inter font | SIL OFL 1.1 |
| Boxicons Free icons | CC BY 4.0 (keep in a separate directory with its license) |
| Sample assets | CC0 preferred; provenance recorded in `assets.lock` |
| Names and logos | Not licensed; see `TRADEMARKS.md` |

## Why the Charm boundary matters
Charms run out of process (Python, TypeScript, Lua) or as WASM components and talk to the core through the LGPL SDK/protocol. Authors may therefore choose any license for their Charms. A GPLv3 section 7 "Charm exception" is planned to make this explicit for the in-process WASM case.

## Rust and the LGPL
LGPL assumes a replaceable shared library. Rust usually links statically, so LGPL behaves much like GPL for Rust crates. Weak-copyleft Rust crates meant for third parties are better served by MPL-2.0 or LGPL plus a linking exception.

## Contributions
- DCO sign-off on every commit (required now).
- CLA with an open-source lock (required once the reviewed text exists; CI check is off until then).
- Details in [CONTRIBUTING.md](../CONTRIBUTING.md).

## Tooling
`reuse lint` (SPDX compliance), `cargo deny check` (dependency licenses and advisories), both run in CI and in `cargo xtask check` when installed.

## Open decisions
1. CLA relicensing scope (recommended: open-source lock).
2. Livkit: GPL now and loosen later (recommended) or MPL-2.0 now.
3. `livspeil-node-api`: MPL-2.0 or LGPL plus linking exception.
4. Add the GPLv3 section 7 Charm exception.
5. Names: Livkit vs LiveKit, Charms; run a trademark search.
6. CLA counterparty and any future legal entity.
