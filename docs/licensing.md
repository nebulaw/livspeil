# Licensing summary

Short version for contributors and packagers. The full discussion with alternatives and sources is kept in the project's design docs ("Licensing and Contributions: Complete Summary"). Not legal advice; the CLA text and the Charm exception still need lawyer review.

## Principle
Livspeil is free software in the sense of the four freedoms (run, study and change, redistribute, share changes). The core stays under strong copyleft so it cannot be closed; the pieces that other people build on (SDK, UI toolkit) use weak copyleft so scenes, Rooms and Charms can be published under whatever free license their authors choose. See [free-software-pledge.md](free-software-pledge.md).

## Who gets which license

| Area | License |
|---|---|
| Floor, Shell, World, Dream Mode, Settings, Launcher, kernel crates | GPL-3.0-or-later |
| `livspeil-node-api`, Charm SDK (Python module, IPC/WASM interface files) | MPL-2.0 |
| Livspeil UI (toolkit and design system, formerly Livkit; crates `livspeil-ui-*`, npm scope `@livspeil/ui`) | MPL-2.0 |
| Future server components (store, sync) | AGPL-3.0 (not yet used) |
| Docs and specs | CC BY 4.0 |
| Inter font | SIL OFL 1.1 |
| Boxicons Free icons | CC BY 4.0 (keep in a separate directory with its license) |
| Sample assets | CC0 preferred; provenance recorded in `assets.lock` |
| Names and logos | Not licensed; see `TRADEMARKS.md` |

MPL-2.0 is file-level copyleft: changes to MPL files stay open, but larger works that use them may carry any license. It is FSF-approved and GPL-compatible, and it avoids the Rust static-linking problems of the LGPL. If a stronger or looser choice proves necessary later, the CLA's open-source lock allows relicensing within free licenses.

## Scenes, Rooms and Charms (published by others)
A Room package (`*.room`) can ship its own Charms, assets and scripts, and authors choose their licenses. Rules:
- Charms talk to the core through the MPL-2.0 SDK or protocol, out of process or as WASM components, so any license is possible, including proprietary for private use.
- A GPLv3 section 7 "Charm exception" (to be drafted by a lawyer) states that Charms using only the documented SDK/protocol are not derivative works of Livspeil.
- Every asset in a Room declares source and license in `assets.lock`.
- The official store accepts free licenses only (OSI/FSF-approved for code, CC BY / CC BY-SA / CC0 and similar for content). Recommended defaults: CC BY-SA 4.0 for Rooms, MIT or MPL-2.0 for Charms.

## `assets.lock`
Each Room package records its assets in `assets.lock` (KDL, next to `room.kdl`): per file the path (`assets/<sha256>.<ext>`), SHA-256, size, source (`"original"` or a URL), SPDX license expression and author/attribution. `livspeil assets add` writes entries and `livspeil validate` / `livspeil assets check` verify them: files must match their hashes and sizes, every file in `assets/` must be listed, and the license must be a well-formed SPDX expression. Licenses outside the allow-list (CC0-1.0, CC-BY-4.0, CC-BY-SA-4.0, OFL-1.1, MIT, Apache-2.0, MPL-2.0, GPL-3.0-or-later, LGPL-3.0-or-later) are warnings, since the official store will not accept them; a missing author for a license that requires attribution is a warning too. In this repository asset files are additionally annotated in `REUSE.toml`.

## Contributions
- CLA with an open-source lock (required once the reviewed text exists; the CI check is off until then).
- Details in [CONTRIBUTING.md](../CONTRIBUTING.md).

## Tooling
`reuse lint` (SPDX compliance) and `cargo deny check` (dependency licenses and advisories) run in CI and in `cargo xtask check` when installed.

## Open items
1. Lawyer review of the CLA and the Charm exception.
2. CLA counterparty (you personally or a future legal entity).
3. Trademark search for Livspeil, Livspeil UI and Charms.
