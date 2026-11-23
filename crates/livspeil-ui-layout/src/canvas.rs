// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Infinite canvas: a pan/zoom transform from canvas space to view space,
//! and culling of items by the visible viewport.

use std::collections::HashMap;

use crate::geometry::{Point, Rect, Size, Transform2D, Vec2};

/// Pan and zoom of an infinite canvas.
///
/// `view = canvas × zoom + pan`: `pan` is where the canvas origin appears
/// in the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Canvas {
    /// View-space position of the canvas origin.
    pub pan: Vec2,
    /// Scale factor.
    pub zoom: f32,
    /// Smallest zoom.
    pub min_zoom: f32,
    /// Largest zoom.
    pub max_zoom: f32,
}

impl Default for Canvas {
    fn default() -> Self {
        Canvas {
            pan: Vec2::ZERO,
            zoom: 1.0,
            min_zoom: 0.05,
            max_zoom: 16.0,
        }
    }
}

impl Canvas {
    /// Canvas space to view space.
    #[must_use]
    pub fn transform(&self) -> Transform2D {
        Transform2D::scale(self.zoom, self.zoom).then(Transform2D::translate(self.pan))
    }

    /// A view point in canvas space.
    #[must_use]
    pub fn to_canvas(&self, view: Point) -> Point {
        (view - self.pan) * (1.0 / self.zoom)
    }

    /// A canvas point in view space.
    #[must_use]
    pub fn to_view(&self, canvas: Point) -> Point {
        canvas * self.zoom + self.pan
    }

    /// Pan by a view-space delta.
    pub fn pan_by(&mut self, delta: Vec2) {
        self.pan += delta;
    }

    /// Multiply the zoom by `factor` (clamped), keeping the canvas point
    /// under `anchor` (view space) fixed.
    pub fn zoom_at(&mut self, anchor: Point, factor: f32) {
        let before = self.to_canvas(anchor);
        self.zoom = (self.zoom * factor).clamp(self.min_zoom, self.max_zoom);
        self.pan = anchor - before * self.zoom;
    }

    /// The canvas-space rect visible in a view of `viewport` size.
    #[must_use]
    pub fn visible_rect(&self, viewport: Size) -> Rect {
        let tl = self.to_canvas(Vec2::ZERO);
        Rect::new(
            tl.x,
            tl.y,
            viewport.width / self.zoom,
            viewport.height / self.zoom,
        )
    }

    /// Indices of `items` (canvas-space bounds) that intersect the viewport
    /// grown by `margin` view pixels. Linear; use [`CanvasIndex`] for many items.
    #[must_use]
    pub fn cull(&self, viewport: Size, margin: f32, items: &[Rect]) -> Vec<usize> {
        let visible = self.visible_rect(viewport).inflate(margin / self.zoom);
        items
            .iter()
            .enumerate()
            .filter(|(_, r)| visible.intersects(r))
            .map(|(i, _)| i)
            .collect()
    }
}

/// A uniform-grid spatial index of canvas items for fast viewport queries.
#[derive(Clone, Debug)]
pub struct CanvasIndex {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<u32>>,
    items: Vec<Rect>,
}

impl CanvasIndex {
    /// An index with square cells of `cell` canvas units (> 0).
    #[must_use]
    pub fn new(cell: f32) -> Self {
        CanvasIndex {
            cell: if cell > 0.0 { cell } else { 256.0 },
            cells: HashMap::new(),
            items: Vec::new(),
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn cell_range(&self, r: &Rect) -> (i32, i32, i32, i32) {
        let f = |v: f32| (v / self.cell).floor().clamp(-1e6, 1e6) as i32;
        (f(r.x), f(r.y), f(r.right()), f(r.bottom()))
    }

    /// Add an item; returns its index.
    pub fn insert(&mut self, bounds: Rect) -> u32 {
        let id = u32::try_from(self.items.len()).unwrap_or(u32::MAX);
        self.items.push(bounds);
        let (x0, y0, x1, y1) = self.cell_range(&bounds);
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                self.cells.entry((cx, cy)).or_default().push(id);
            }
        }
        id
    }

    /// Number of items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Items intersecting `area` (canvas space), sorted by index.
    #[must_use]
    pub fn query(&self, area: &Rect) -> Vec<u32> {
        let (x0, y0, x1, y1) = self.cell_range(area);
        let mut out = Vec::new();
        for cy in y0..=y1 {
            for cx in x0..=x1 {
                if let Some(ids) = self.cells.get(&(cx, cy)) {
                    out.extend(ids.iter().copied().filter(|&i| {
                        self.items
                            .get(i as usize)
                            .is_some_and(|r| r.intersects(area))
                    }));
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Items visible through `canvas` in a `viewport`.
    #[must_use]
    pub fn visible(&self, canvas: &Canvas, viewport: Size, margin: f32) -> Vec<u32> {
        self.query(&canvas.visible_rect(viewport).inflate(margin / canvas.zoom))
    }
}
