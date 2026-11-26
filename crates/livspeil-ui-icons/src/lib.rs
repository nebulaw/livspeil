// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI icons (UI spec §3, decision D22).
//!
//! - [`IconId`]: the registry of the embedded Boxicons free set, looked up
//!   by Boxicons name (`bx-copy`).
//! - [`IconRasterizer`]: an icon as an 8-bit coverage mask at a physical
//!   pixel size. No colour and no GPU types: `livspeil-ui-paint` caches the
//!   masks in the glyph atlas and tints them at draw time.
//! - [`parse_svg`]: a strict parser of the SVG subset the set uses. It
//!   rejects anything else instead of drawing it wrong.
//!
//! The icons are CC BY 4.0, Copyright 2025 Boxicons
//! (<https://boxicons.com>); see `assets/icons/boxicons/README.md`.

#![forbid(unsafe_code)]

mod path;
mod raster;
mod registry;
mod svg;

pub use raster::{IconMask, IconRasterizer, MAX_ICON_PX};
pub use registry::IconId;
pub use svg::{SvgError, VIEW_BOX, parse_svg};
pub use zeno::Command;
