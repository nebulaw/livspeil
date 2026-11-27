// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Scripted interactions for measuring frame costs. The app runs them in a
//! real window (`--scenario=NAME`, timed on hardware) and the tests run the
//! same steps headlessly (counted, so CI can hold budgets).
//!
//! Each step is a pure function of the frame number: no clocks, no input.

use livspeil_ui_core::App;
use livspeil_ui_layout::{Point, Vec2};
use livspeil_ui_widgets::{NodeCanvas, NodeGraphModel};

use crate::{Mounted, Page};

/// Frames a direction is kept before turning back.
const LEG: u32 = 60;

/// A scripted interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// Pan the sample graph (6 nodes).
    PanSample,
    /// Pan the stress graph (500 nodes) at its initial zoom.
    PanStress,
    /// Pan the stress graph zoomed out to fit.
    PanStressFit,
    /// Zoom the stress graph in and out about the canvas centre.
    ZoomStress,
    /// Drag one node of the stress graph.
    DragStress,
    /// Scroll the 100,000 row list.
    ScrollList,
}

impl Scenario {
    /// Every scenario.
    pub const ALL: [Scenario; 6] = [
        Scenario::PanSample,
        Scenario::PanStress,
        Scenario::PanStressFit,
        Scenario::ZoomStress,
        Scenario::DragStress,
        Scenario::ScrollList,
    ];

    /// The first frame that interacts; earlier ones set the scene up.
    pub const WARMUP: u32 = 10;

    /// The name used on the command line and in reports.
    #[must_use]
    pub fn slug(self) -> &'static str {
        match self {
            Scenario::PanSample => "pan-sample",
            Scenario::PanStress => "pan-stress",
            Scenario::PanStressFit => "pan-stress-fit",
            Scenario::ZoomStress => "zoom-stress",
            Scenario::DragStress => "drag-stress",
            Scenario::ScrollList => "scroll-list",
        }
    }

    /// The scenario named `slug`.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Scenario> {
        Scenario::ALL.into_iter().find(|s| s.slug() == slug)
    }

    /// How many of 88 consecutive interacting frames at least draw
    /// something new (so a scenario that stops working is noticed).
    #[must_use]
    pub fn changing_frames(self) -> u32 {
        80
    }

    /// The gallery page to mount.
    #[must_use]
    pub fn page(self) -> Page {
        match self {
            Scenario::ScrollList => Page::Scroll,
            _ => Page::NodeCanvas,
        }
    }

    /// Apply frame `frame` to the mounted gallery.
    pub fn step(self, app: &mut App, mounted: &Mounted, frame: u32) {
        let which = usize::from(!matches!(self, Scenario::PanSample));
        let canvas = mounted.canvases[which];
        if frame < Scenario::WARMUP {
            if frame == 0 && self.page() == Page::NodeCanvas {
                mounted.canvas_choice.update(app, |c, cx| {
                    *c = which;
                    cx.notify();
                });
            }
            // The canvas knows its size after a layout.
            if frame == Scenario::WARMUP / 2 && self == Scenario::PanStressFit {
                NodeCanvas::fit(canvas, app);
            }
            return;
        }
        let t = frame - Scenario::WARMUP;
        let forward = (t / LEG).is_multiple_of(2);
        let sign = if forward { 1.0 } else { -1.0 };
        match self {
            Scenario::PanSample | Scenario::PanStress | Scenario::PanStressFit => {
                NodeCanvas::pan_by(canvas, app, Vec2::new(3.0 * sign, 0.0));
            }
            Scenario::ZoomStress => {
                let mut c = canvas.read(app).canvas();
                let factor = if forward { 1.01 } else { 1.0 / 1.01 };
                c.zoom_at(Point::new(400.0, 400.0), factor);
                NodeCanvas::set_canvas(canvas, app, c);
            }
            Scenario::DragStress => {
                let graph = canvas.read(app).model();
                graph.update(app, |g, cx| {
                    if let Some(&node) = g.node_ids().first() {
                        let p = g.position(node);
                        g.set_position(node, Point::new(p.x + 2.0 * sign, p.y + sign));
                        cx.notify();
                    }
                });
            }
            Scenario::ScrollList => mounted.list.update(app, |s, cx| {
                // Down for a leg, then back: never past the start.
                let leg = t % (2 * LEG);
                let steps = leg.min(2 * LEG - leg);
                let offset = f32::from(u16::try_from(steps).unwrap_or(u16::MAX)) * 7.0;
                s.scroller_mut().jump_to(Vec2::new(0.0, offset));
                cx.notify();
            }),
        }
    }
}
