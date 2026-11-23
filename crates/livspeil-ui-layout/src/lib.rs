// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI layout.
//!
//! - [`geometry`]: vectors, rects, corner radii, affine transforms.
//! - [`LayoutTree`]: Taffy (flexbox, grid, block) over a retained tree, with
//!   change detection so only dirty subtrees are laid out again.
//! - [`Dock`]: left/right/top/bottom panels around a center with minimum
//!   sizes and draggable splitters.
//! - [`Canvas`] and [`CanvasIndex`]: infinite canvas pan/zoom and viewport
//!   culling.

pub mod canvas;
pub mod dock;
pub mod geometry;
mod tree;

pub use canvas::{Canvas, CanvasIndex};
pub use dock::{Dock, DockEdge, DockLayout, DockSide, Splitter, SplitterDrag};
pub use geometry::{Corners, Point, Rect, Size, Transform2D, Vec2};
pub use taffy;
pub use tree::{LayoutError, LayoutNode, LayoutStats, LayoutTree, Measure, Style, style};
