// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Names of animation settings in `room.kdl`.

use livspeil_params::{Extrapolation, Interpolation};

/// `pre=` / `post=` names.
pub(crate) const EXTRAPOLATIONS: [(&str, Extrapolation); 3] = [
    ("hold", Extrapolation::Hold),
    ("loop", Extrapolation::Loop),
    ("ping-pong", Extrapolation::PingPong),
];

/// `interp=` names. `spring` additionally needs `frequency=` and `damping=`.
pub(crate) const INTERPOLATIONS: [&str; 4] = ["step", "linear", "bezier", "spring"];

/// `tangent=` names. `explicit` additionally needs `in=` and `out=`.
pub(crate) const TANGENTS: [&str; 3] = ["auto", "flat", "explicit"];

pub(crate) fn extrapolation_name(e: Extrapolation) -> &'static str {
    match e {
        Extrapolation::Hold => "hold",
        Extrapolation::Loop => "loop",
        Extrapolation::PingPong => "ping-pong",
    }
}

pub(crate) fn extrapolation(name: &str) -> Option<Extrapolation> {
    EXTRAPOLATIONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| *e)
}

pub(crate) fn interpolation_name(i: &Interpolation) -> &'static str {
    match i {
        Interpolation::Step => "step",
        Interpolation::Linear => "linear",
        Interpolation::Bezier => "bezier",
        Interpolation::Spring { .. } => "spring",
    }
}

/// Quoted, comma-separated list of names for error messages.
pub(crate) fn list(names: impl IntoIterator<Item = &'static str>) -> String {
    names
        .into_iter()
        .map(|n| format!("\"{n}\""))
        .collect::<Vec<_>>()
        .join(", ")
}
