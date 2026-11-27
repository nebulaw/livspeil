// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The minimap's content: one small rect per node, each placed by a layer
//! frame, so a node that moves is not rendered.

use livspeil_ui_core::{App, Element, Layer, Render, View, ViewContext, div};
use livspeil_ui_layout::style::{Dimension, Position};
use livspeil_ui_layout::{Point, Rect, Size, Vec2};
use livspeil_ui_theme::Color;

use super::geom::{GraphCache, graph_bounds};

/// Minimap size.
pub const MINIMAP: Size = Size {
    width: 200.0,
    height: 130.0,
};
/// Space between the mapped graph and the minimap's edges.
const PAD: f32 = 8.0;

/// What the minimap shows: the graph with a margin around it (so the
/// viewport frame stays readable).
fn wanted_bounds(cache: &GraphCache) -> Rect {
    let b = graph_bounds(cache);
    b.inflate(b.width.max(b.height) * 0.15)
}

/// Maps graph bounds into the minimap: `(scale, offset)` with
/// `minimap = canvas × scale + offset`.
#[must_use]
pub fn minimap_mapping(bounds: Rect, size: Size, pad: f32) -> (f32, Vec2) {
    let w = (size.width - 2.0 * pad).max(1.0);
    let h = (size.height - 2.0 * pad).max(1.0);
    let s = (w / bounds.width.max(1.0)).min(h / bounds.height.max(1.0));
    let off = Vec2::new(
        pad + (w - bounds.width * s) / 2.0 - bounds.x * s,
        pad + (h - bounds.height * s) / 2.0 - bounds.y * s,
    );
    (s, off)
}

/// The part of canvas space the minimap shows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Shown(Rect);

impl Shown {
    pub(super) fn all(cache: &GraphCache) -> Shown {
        Shown(wanted_bounds(cache))
    }

    /// Adjust to the graph as it is now. Returns whether the mapping
    /// changed (a node inside the graph's bounds moving does not).
    pub(super) fn follow(&mut self, cache: &GraphCache) -> bool {
        let now = Shown::all(cache);
        std::mem::replace(self, now) != now
    }

    /// `(scale, offset)` from canvas to minimap coordinates.
    pub(super) fn mapping(self) -> (f32, Vec2) {
        minimap_mapping(self.0, MINIMAP, PAD)
    }

    /// A canvas rect in minimap coordinates, at least a pixel large.
    pub(super) fn map(self, r: Rect) -> Rect {
        let (k, off) = self.mapping();
        Rect::new(
            r.x * k + off.x,
            r.y * k + off.y,
            (r.width * k).max(1.0),
            (r.height * k).max(1.0),
        )
    }

    /// The canvas point shown at `local` minimap coordinates.
    pub(super) fn to_canvas(self, local: Point) -> Point {
        let (k, off) = self.mapping();
        (local - off) * (1.0 / k)
    }
}

/// One rect per node, in paint order.
#[derive(Debug)]
pub(super) struct MinimapNodes {
    selected: Vec<bool>,
    color: Color,
    accent: Color,
}

impl MinimapNodes {
    pub(super) fn new(color: Color, accent: Color) -> Self {
        MinimapNodes {
            selected: Vec::new(),
            color,
            accent,
        }
    }

    /// The layer placing node `index`.
    fn layer(view: View<Self>, app: &mut App, index: usize) -> Layer {
        app.layer(view.id(), &index.to_string())
    }

    /// Put node `index` at `rect` (minimap coordinates).
    pub(super) fn place(view: View<Self>, app: &mut App, index: usize, rect: Rect) {
        Self::layer(view, app, index).set_frame(app, Some(rect));
    }

    /// Which nodes are selected, one flag per node. Renders again only if
    /// the flags or colors differ.
    pub(super) fn set(
        view: View<Self>,
        app: &mut App,
        selected: Vec<bool>,
        color: Color,
        accent: Color,
    ) {
        let same = app.untracked(|app| {
            let s = view.read(app);
            s.selected == selected && s.color == color && s.accent == accent
        });
        if !same {
            view.update(app, |s, cx| {
                s.selected = selected;
                s.color = color;
                s.accent = accent;
                cx.notify();
            });
        }
    }
}

impl Render for MinimapNodes {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let mut el = div().name("minimap-nodes").hit_test(false).style(|s| {
            s.position = Position::Absolute;
            s.size.width = Dimension::percent(1.0);
            s.size.height = Dimension::percent(1.0);
        });
        for (i, selected) in self.selected.iter().enumerate() {
            let key = i.to_string();
            el = el.child(
                div()
                    .key(key.clone())
                    .absolute(Rect::ZERO)
                    .layer(cx.layer(&key))
                    .rounded(1.0)
                    .bg(if *selected { self.accent } else { self.color })
                    .hit_test(false),
            );
        }
        el
    }
}
