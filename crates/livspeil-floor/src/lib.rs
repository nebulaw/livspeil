// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Livspeil Floor: a Wayland compositor (spike).
//!
//! Linux only. The platform-independent math lives in [`geometry`] and
//! builds everywhere; the compositor itself is behind
//! `cfg(target_os = "linux")` and its dependencies are target-gated, so
//! `cargo build --workspace` on macOS never compiles Smithay.
//! See `docs/decisions/floor-renderer.md` and `docs/floor-spike-checklist.md`.

pub mod geometry;
