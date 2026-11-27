// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The links of the canvas, in a view of their own: moving a node redraws
//! them without rendering the canvas.

use livspeil_ui_core::{App, Element, Render, View, ViewContext, Wire, div};
use livspeil_ui_layout::Rect;

/// Wires drawn under the cards, in canvas coordinates.
#[derive(Debug, Default)]
pub(super) struct WireLayer {
    /// What the element covers: whole units, so layout does not round it.
    area: Rect,
    /// Relative to the area's origin.
    wires: Vec<Wire>,
}

/// `area` widened to whole units.
fn whole(area: Rect) -> Rect {
    let (x, y) = (area.x.floor(), area.y.floor());
    Rect::new(x, y, area.right().ceil() - x, area.bottom().ceil() - y)
}

impl WireLayer {
    /// Show `wires` (canvas coordinates) over `area`, rendering again only
    /// if they differ from what is shown.
    pub(super) fn set(view: View<Self>, app: &mut App, area: Rect, mut wires: Vec<Wire>) {
        let area = whole(area);
        let origin = area.origin() - livspeil_ui_layout::Point::ZERO;
        for w in &mut wires {
            w.from -= origin;
            w.c1 -= origin;
            w.c2 -= origin;
            w.to -= origin;
        }
        let same = app.untracked(|app| {
            let shown = view.read(app);
            shown.area == area && shown.wires == wires
        });
        if !same {
            view.update(app, |s, cx| {
                s.area = area;
                s.wires = wires;
                cx.notify();
            });
        }
    }
}

impl Render for WireLayer {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        div()
            .name("wires")
            .absolute(self.area)
            .hit_test(false)
            .cull_offscreen()
            .wires(self.wires.clone())
    }
}
