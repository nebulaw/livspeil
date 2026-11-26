<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Repo Conventions and Setup (as built, Rev 3, 2026-11-26)

Not legal advice. The complete license and CLA discussion lives in `licensing-and-contributions.md` (not in this repository; see `licensing.md`); this doc keeps the repo-level conventions. Have a lawyer review the CLA text and license boundaries before the first public release.

## 1. License matrix (summary)
| Area | License | Notes |
|---|---|---|
| Apps: Floor, Shell, World, Dream Mode, Settings, Launcher | GPL-3.0-or-later | D21 |
| Kernel crates (graph, params, doc, cmd, nodes-core, script, wasm-host, testkit, cli) | GPL-3.0-or-later | |
| SDK: `livspeil-node-api`, Python SDK (`sdk/python`), WIT interface files | MPL-2.0 | D33 |
| Livspeil UI: every `crates/livspeil-ui-*` | MPL-2.0 | D33; must not depend on GPL kernel crates |
| Fonts: Inter, Noto Sans Georgian/Arabic | SIL OFL 1.1 | vendored in `assets/fonts/` with licenses |
| Icons: Boxicons Free | CC BY 4.0 | Pro files must never be vendored |
| Docs, specs, `.claude/` setup | CC BY 4.0 | |
| Examples' assets, config files | CC0-1.0 | via `REUSE.toml` |
| Third-party Rust deps | MIT, Apache-2.0, BSD, ISC, Zlib, MPL-2.0, Unicode, CC0, BSL-1.0 | enforced by `cargo-deny` (BSL-1.0 per D53) |

## 2. Contribution policy
CLA only (D26); DCO dropped (D42). Nobody adds `Signed-off-by`. AI tools are welcome; `Co-Authored-By` trailers are kept. The CLA check stays off in CI until the reviewed CLA text exists; outside PRs wait until then.

## 3. License hygiene tooling
- REUSE: SPDX headers or `REUSE.toml` annotations, `LICENSES/`, `reuse lint` in CI.
- cargo-deny: license allow-list, sources, advisories, bans. Ignored advisories are listed by id in `deny.toml` and explained in `known-issues.md`.
- Asset provenance: `assets.lock` per room.

## 4. Repository layout
```
livspeil/
  crates/        kernel (livspeil-*), SDK (node-api), UI (livspeil-ui-*), gallery
  sdk/python/    Python SDK for scripts
  wit/           WASM node interface
  examples/      *.room packages (sun-and-lamp, charm-demo, out-demo, blink, wasm)
  assets/fonts/  Inter, Noto
  specs/         kernel and UI specs (as built)
  docs/          component docs, benchmarks, known issues, decisions, playbook
  xtask/         cargo xtask check | node-specs | shot
  .claude/       settings.json, agents/, skills/ (Claude Code setup, D55)
  LICENSES/  CLAUDE.md  CONTRIBUTING.md  CLA.md (draft)  TRADEMARKS.md  README.md
  .github/workflows/  ci, licenses, cla (gated)
```

## 5. CI
- `fmt, clippy, test` on ubuntu-latest and macos-latest with latest stable Rust; Linux installs `mesa-vulkan-drivers libfontconfig1-dev pkg-config` (lavapipe for GPU tests).
- `MSRV (Rust 1.95)` job; `cargo-deny`; `REUSE compliance`; `cla` (skipped until enabled).
- Golden images render on the CPU reference rasterizer, so they are identical across machines; GPU-vs-CPU tests compare within tolerance and run serialized.

## 6. Agent workflow
Full guide: `claude-code-playbook.md` (repo: `docs/claude-code-playbook.md`). Essentials:
- Lead session plans rounds and lands branches; `implementer` agents build one milestone each in `../wt/<track>` worktrees; optional `reviewer` agent for high-stakes work.
- Spec first; one milestone per PR; tests for every behaviour; look at rendered output; say what was not verified.
- `cargo xtask check` before finishing; agents commit but never push; the owner merges.
- PRs are opened with `gh api repos/nebulaw/livspeil/pulls`; never resolve `Cargo.lock` in the GitHub web editor.

## 7. Dev environment
- Owner develops on Linux (Arch) since D51; macOS remains CI-tested.
- Targets: the owner's Linux machine, an Intel-iGPU Arch laptop for baselines, a VM for install tests; NVIDIA later.
