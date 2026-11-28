// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The Dream Mode app (`specs/livspeil-dream-spec-v0.md`), first slice:
//! open a room, see its graph and watch it run.
//!
//! - [`session`]: the open room, its graph and its frame clock. No UI.
//! - [`shown`]: what a node of the root network looks like from outside
//!   (ports, links, input sources), for a plain node, a subnet or a Charm
//!   instance alike. No UI.
//! - [`arrange`]: positions for nodes that have none. No UI.
//! - [`canvas`]: the session as a node canvas model.
//! - [`app`]: the window's views.

#![forbid(unsafe_code)]

pub mod app;
pub mod arrange;
pub mod canvas;
pub mod inspect;
pub mod session;
pub mod shown;

pub use session::{OpenError, Session};
