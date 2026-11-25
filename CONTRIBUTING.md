# Contributing to Livspeil

Thank you for helping. Once it is enabled, every contributor signs the **CLA** once; no per-commit sign-off is required.

## 1. Contributor License Agreement (CLA)

The CLA ([CLA.md](CLA.md)) lets the project keep its licensing flexible (for example loosening the UI or SDK licenses later) while you keep the copyright to your work. The current text is a **draft pending legal review**; the CLA check is switched off until it is final. Until then, outside contributions are accepted only on the understanding that you will sign the final CLA before merge.

## 2. Licensing of your contribution

Your contribution is licensed under the license of the part you modify: GPL-3.0-or-later by default, MPL-2.0 for `crates/livspeil-node-api`, every `crates/livspeil-ui-*` crate and `sdk/python`, CC BY 4.0 for docs and specs. Every file needs SPDX headers (`SPDX-FileCopyrightText` and `SPDX-License-Identifier`) or an entry in `REUSE.toml`. Dependencies must pass `cargo deny check`. Details: [docs/licensing.md](docs/licensing.md).

## 3. AI-assisted contributions

Using AI tools is welcome. You remain responsible: you must understand and have reviewed what you submit, and by submitting you confirm you have the right to submit it under the project's licenses. Say so in the pull request when a substantial part was generated. Tool-added `Co-Authored-By` trailers are kept for transparency. The repository ships a Claude Code setup (`.claude/`: shared permissions, `implementer` and `reviewer` agents, `/start-round`, `/land-branch` and `/fix-ci`); how it is used is in [docs/claude-code-playbook.md](docs/claude-code-playbook.md).

## 4. Workflow

1. Open an issue or discussion for anything non-trivial.
2. Branch from `main`; keep pull requests small and focused (one node, one feature).
3. Run `cargo xtask check` locally.
4. New nodes need a spec file, an implementation and a golden test (see [Adding a node](#adding-a-node)).
5. Golden file changes are reviewed by hand; never accept them blindly.

## Adding a node

A node is a spec file, a Rust implementation and tests. Keep it to one node per pull request. For a core node:

1. **Implement** the node in `crates/livspeil-nodes-core/src/` (`NodeSpec` built in `new()`, a pure, deterministic `cook`; typed `NodeError`s, no panics) and **register** it in `register_all` in `src/lib.rs`.
2. **Generate its spec file** with `cargo xtask node-specs`. This writes `crates/livspeil-nodes-core/specs/<id>.node.toml` from the Rust `NodeSpec` (id, version, summary, flags, ports, defaults) and refreshes `docs/nodes.md`.
3. **Document and test it in the spec file**: add a `description` and `[[tests]]` cases (inputs, expected outputs or `expect_error`; see [docs/nodes.md](docs/nodes.md#spec-file-format) for the format). Cover the defaults, typical values and every error and edge case (`NaN`, infinities, empty tables). Run `cargo xtask node-specs` again so `docs/nodes.md` picks up the description.
4. **Add golden cases** for the node in `crates/livspeil-nodes-core/tests/golden*.rs`, refresh them with `UPDATE_GOLDEN=1` and review the diff.
5. Run `cargo xtask check`.

`crates/livspeil-nodes-core/tests/node_specs.rs` fails when a registered node has no spec file (or a file has no node), when a spec file's generated part differs from the Rust `NodeSpec`, when a `[[tests]]` case fails or when `docs/nodes.md` is stale; its message says which file and how to fix it. Changing ports or behavior incompatibly means bumping the node's `version`.
