// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Dock layout: left, right, top and bottom panels around a center, with
//! minimum sizes and draggable splitters.
//!
//! Left and right panels span the full height; top and bottom span the
//! space between them. A splitter sits on each visible panel's inner edge,
//! overlaying it (it takes no space); its hit area extends `hit_slop` px to
//! both sides.

use crate::geometry::{Point, Rect, Size};

/// A dock edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DockEdge {
    /// Left panel.
    Left,
    /// Right panel.
    Right,
    /// Top panel.
    Top,
    /// Bottom panel.
    Bottom,
}

impl DockEdge {
    /// All edges in layout order.
    pub const ALL: [DockEdge; 4] = [
        DockEdge::Left,
        DockEdge::Right,
        DockEdge::Top,
        DockEdge::Bottom,
    ];

    /// Whether the panel's size is a width.
    #[must_use]
    pub fn is_horizontal(self) -> bool {
        matches!(self, DockEdge::Left | DockEdge::Right)
    }
}

/// One side panel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DockSide {
    /// Width (left/right) or height (top/bottom).
    pub size: f32,
    /// Lower bound for `size`.
    pub min: f32,
    /// Hidden panels take no space and have no splitter.
    pub visible: bool,
}

impl DockSide {
    /// A visible side.
    #[must_use]
    pub const fn new(size: f32, min: f32) -> Self {
        DockSide {
            size,
            min,
            visible: true,
        }
    }
}

/// A splitter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Splitter {
    /// Which panel it resizes.
    pub edge: DockEdge,
    /// The visible line.
    pub rect: Rect,
    /// The pointer hit area.
    pub hit_rect: Rect,
}

/// The result of [`Dock::layout`].
#[derive(Clone, Debug, PartialEq)]
pub struct DockLayout {
    /// The center area.
    pub center: Rect,
    /// Visible panels.
    pub panels: Vec<(DockEdge, Rect)>,
    /// Splitters of visible panels.
    pub splitters: Vec<Splitter>,
}

impl DockLayout {
    /// The rect of a visible panel.
    #[must_use]
    pub fn panel(&self, edge: DockEdge) -> Option<Rect> {
        self.panels
            .iter()
            .find(|(e, _)| *e == edge)
            .map(|(_, r)| *r)
    }
}

/// A splitter drag in progress: sizes follow the pointer's total travel
/// from where the drag started, so overshooting a limit and coming back
/// re-engages exactly where the pointer is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitterDrag {
    /// The panel being resized.
    pub edge: DockEdge,
    start_size: f32,
}

/// The dock.
#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    /// Left panel.
    pub left: Option<DockSide>,
    /// Right panel.
    pub right: Option<DockSide>,
    /// Top panel.
    pub top: Option<DockSide>,
    /// Bottom panel.
    pub bottom: Option<DockSide>,
    /// Minimum center size.
    pub center_min: Size,
    /// Visible splitter thickness.
    pub splitter: f32,
    /// Extra hit area on each side of a splitter.
    pub hit_slop: f32,
}

impl Default for Dock {
    fn default() -> Self {
        Dock {
            left: None,
            right: None,
            top: None,
            bottom: None,
            center_min: Size::new(120.0, 80.0),
            splitter: 1.0,
            hit_slop: 4.0,
        }
    }
}

impl Dock {
    /// The side at `edge`.
    #[must_use]
    pub fn side(&self, edge: DockEdge) -> Option<&DockSide> {
        match edge {
            DockEdge::Left => self.left.as_ref(),
            DockEdge::Right => self.right.as_ref(),
            DockEdge::Top => self.top.as_ref(),
            DockEdge::Bottom => self.bottom.as_ref(),
        }
    }

    /// The side at `edge`, mutably.
    pub fn side_mut(&mut self, edge: DockEdge) -> Option<&mut DockSide> {
        match edge {
            DockEdge::Left => self.left.as_mut(),
            DockEdge::Right => self.right.as_mut(),
            DockEdge::Top => self.top.as_mut(),
            DockEdge::Bottom => self.bottom.as_mut(),
        }
    }

    fn visible_size(&self, edge: DockEdge) -> f32 {
        self.side(edge)
            .filter(|s| s.visible)
            .map_or(0.0, |s| s.size)
    }

    /// The largest size `edge` may have in `bounds` given the opposite
    /// panel and the minimum center size.
    #[must_use]
    pub fn max_size(&self, edge: DockEdge, bounds: Size) -> f32 {
        let (extent, opposite, center_min) = match edge {
            DockEdge::Left => (
                bounds.width,
                self.visible_size(DockEdge::Right),
                self.center_min.width,
            ),
            DockEdge::Right => (
                bounds.width,
                self.visible_size(DockEdge::Left),
                self.center_min.width,
            ),
            DockEdge::Top => (
                bounds.height,
                self.visible_size(DockEdge::Bottom),
                self.center_min.height,
            ),
            DockEdge::Bottom => (
                bounds.height,
                self.visible_size(DockEdge::Top),
                self.center_min.height,
            ),
        };
        (extent - opposite - center_min).max(0.0)
    }

    /// Lay out panels, center and splitters in `bounds`. Sizes too large
    /// for `bounds` are clamped (stored sizes are not changed).
    #[must_use]
    pub fn layout(&self, bounds: Rect) -> DockLayout {
        let clamp = |edge: DockEdge, first: f32| -> f32 {
            let Some(side) = self.side(edge).filter(|s| s.visible) else {
                return 0.0;
            };
            let extent = if edge.is_horizontal() {
                bounds.width
            } else {
                bounds.height
            };
            let center_min = if edge.is_horizontal() {
                self.center_min.width
            } else {
                self.center_min.height
            };
            side.size.min(extent - first - center_min).max(0.0)
        };
        let left = clamp(DockEdge::Left, 0.0);
        let right = clamp(DockEdge::Right, left);
        let top = clamp(DockEdge::Top, 0.0);
        let bottom = clamp(DockEdge::Bottom, top);

        let inner_x = bounds.x + left;
        let inner_w = (bounds.width - left - right).max(0.0);
        let mut panels = Vec::new();
        let mut splitters = Vec::new();
        let t = self.splitter;
        let slop = self.hit_slop;
        let mut push = |edge: DockEdge, rect: Rect, line: Rect| {
            panels.push((edge, rect));
            let hit = if edge.is_horizontal() {
                Rect::new(line.x - slop, line.y, line.width + 2.0 * slop, line.height)
            } else {
                Rect::new(line.x, line.y - slop, line.width, line.height + 2.0 * slop)
            };
            splitters.push(Splitter {
                edge,
                rect: line,
                hit_rect: hit,
            });
        };
        if self.side(DockEdge::Left).is_some_and(|s| s.visible) {
            let r = Rect::new(bounds.x, bounds.y, left, bounds.height);
            push(
                DockEdge::Left,
                r,
                Rect::new(r.right() - t, r.y, t, r.height),
            );
        }
        if self.side(DockEdge::Right).is_some_and(|s| s.visible) {
            let r = Rect::new(bounds.right() - right, bounds.y, right, bounds.height);
            push(DockEdge::Right, r, Rect::new(r.x, r.y, t, r.height));
        }
        if self.side(DockEdge::Top).is_some_and(|s| s.visible) {
            let r = Rect::new(inner_x, bounds.y, inner_w, top);
            push(DockEdge::Top, r, Rect::new(r.x, r.bottom() - t, r.width, t));
        }
        if self.side(DockEdge::Bottom).is_some_and(|s| s.visible) {
            let r = Rect::new(inner_x, bounds.bottom() - bottom, inner_w, bottom);
            push(DockEdge::Bottom, r, Rect::new(r.x, r.y, r.width, t));
        }
        let center = Rect::new(
            inner_x,
            bounds.y + top,
            inner_w,
            (bounds.height - top - bottom).max(0.0),
        );
        DockLayout {
            center,
            panels,
            splitters,
        }
    }

    /// The splitter under `point`, if any (later splitters win ties).
    #[must_use]
    pub fn splitter_at(&self, bounds: Rect, point: Point) -> Option<DockEdge> {
        self.layout(bounds)
            .splitters
            .iter()
            .rev()
            .find(|s| s.hit_rect.contains(point))
            .map(|s| s.edge)
    }

    /// Start dragging the splitter of `edge`.
    #[must_use]
    pub fn begin_drag(&self, edge: DockEdge) -> Option<SplitterDrag> {
        self.side(edge).filter(|s| s.visible).map(|s| SplitterDrag {
            edge,
            start_size: s.size,
        })
    }

    /// Update a drag with the pointer's total travel since it began
    /// (`dx` for left/right, `dy` for top/bottom). Returns the new size.
    pub fn drag(&mut self, drag: SplitterDrag, travel: f32, bounds: Size) -> f32 {
        let grow = match drag.edge {
            DockEdge::Left | DockEdge::Top => travel,
            DockEdge::Right | DockEdge::Bottom => -travel,
        };
        let max = self.max_size(drag.edge, bounds);
        let Some(side) = self.side_mut(drag.edge) else {
            return 0.0;
        };
        side.size = (drag.start_size + grow).min(max).max(side.min);
        side.size
    }
}
