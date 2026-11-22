# Contributing to Livspeil

Thank you for helping. Once it is enabled, every contributor signs the **CLA** once; no per-commit sign-off is required.

## 1. Contributor License Agreement (CLA)

The CLA ([CLA.md](CLA.md)) lets the project keep its licensing flexible (for example loosening the UI or SDK licenses later) while you keep the copyright to your work. The current text is a **draft pending legal review**; the CLA check is switched off until it is final. Until then, outside contributions are accepted only on the understanding that you will sign the final CLA before merge.

## 2. Licensing of your contribution

Your contribution is licensed under the license of the part you modify: GPL-3.0-or-later by default, MPL-2.0 for `crates/livspeil-node-api`, CC BY 4.0 for docs and specs. Every file needs SPDX headers (`SPDX-FileCopyrightText` and `SPDX-License-Identifier`) or an entry in `REUSE.toml`. Dependencies must pass `cargo deny check`. Details: [docs/licensing.md](docs/licensing.md).

## 3. AI-assisted contributions

Using AI tools is welcome. You remain responsible: you must understand and have reviewed what you submit, and by submitting you confirm you have the right to submit it under the project's licenses. Say so in the pull request when a substantial part was generated. Tool-added `Co-Authored-By` trailers are kept for transparency.

## 4. Workflow

1. Open an issue or discussion for anything non-trivial.
2. Branch from `main`; keep pull requests small and focused (one node, one feature).
3. Run `cargo xtask check` locally.
4. New nodes need a spec, an implementation and a golden test (see the kernel spec).
5. Golden file changes are reviewed by hand; never accept them blindly.
