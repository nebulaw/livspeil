// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Parameter model, animation channels and the expression language.
//!
//! - [`expr`]: the expression language (M0.5a).
//! - [`anim`]: keyframed animation channels (M0.5b).
//!
//! How a graph input picks its value (link, expression, animation channel,
//! static value, default) lives in `livspeil-graph`. See
//! `specs/livspeil-kernel-spec-v0.md`, section 6.
//!
//! Cargo feature `serde` (off by default) derives `Serialize` and
//! `Deserialize` for the animation types; deserialized channels are
//! validated like [`anim::AnimChannel::new`].

pub mod anim;
pub mod expr;

pub use anim::{AnimChannel, AnimError, Extrapolation, Interpolation, Keyframe, Tangent};
