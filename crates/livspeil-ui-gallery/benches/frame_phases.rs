// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Where a NodeCanvas frame spends its CPU time: 2000 nodes and 3000
//! links in a 1600x1000 window, one frame after a pan at zoom 1 (Full),
//! 0.4 (Compact) and fit-all (Block), split into the phases the gallery's
//! frame HUD shows: build (`App::draw_frame`: render, diff, layout, core
//! paint list), paint (`PaintBridge::paint_scene`: scene to display list,
//! wires flattened into quads) and encode (`render::encode`: display list
//! to instance data). GPU work is not measured here.
//!
//! Prints the median and the minimum of `ITERATIONS` frames per phase, in
//! microseconds, plus the size of the scene and display list. Run with
//! `cargo bench -p livspeil-ui-gallery --bench frame_phases`.

use std::time::{Duration, Instant};

use livspeil_ui_core::{App, Element, Render, View, ViewContext, div};
use livspeil_ui_layout::{Canvas, Size, Vec2};
use livspeil_ui_paint::{FontSystem, PaintBridge};
use livspeil_ui_render::{Color, DisplayList, FrameOptions, encode};
use livspeil_ui_widgets::{DemoGraph, Env, NodeCanvas};

/// Frames measured per configuration (override with `LIVSPEIL_BENCH_ITERS`).
const ITERATIONS: usize = 200;

fn iterations() -> usize {
    std::env::var("LIVSPEIL_BENCH_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ITERATIONS)
}

struct Root {
    canvas: View<NodeCanvas<DemoGraph>>,
}

impl Render for Root {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        div().w_full().h_full().child(Element::view(&self.canvas))
    }
}

fn setup(zoom: Option<f32>) -> (App, View<NodeCanvas<DemoGraph>>) {
    let mut app = App::new();
    app.set_viewport(Size::new(1600.0, 1000.0));
    let env = Env::headless(&mut app);
    let graph = app.new_model(DemoGraph::grid(2000, 3000));
    let canvas = app.new_view(NodeCanvas::new(&env, graph));
    let root = app.new_view(Root { canvas });
    let root = env.root(&mut app, root);
    app.set_root(root);
    app.draw_frame(1.0 / 60.0);
    match zoom {
        Some(z) => canvas.update(&mut app, |s, cx| {
            s.set_canvas(Canvas {
                zoom: z,
                pan: Vec2::new(-200.0, -100.0),
                ..Canvas::default()
            });
            cx.notify();
        }),
        None => NodeCanvas::fit(canvas, &mut app),
    }
    app.draw_frame(1.0 / 60.0);
    (app, canvas)
}

fn median(v: &mut [Duration]) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn main() {
    let mut bridge = PaintBridge::new(FontSystem::test());
    let iters = iterations();
    for (name, zoom) in [
        ("zoom 1 (Full)", Some(1.0)),
        ("zoom 0.4 (Compact)", Some(0.4)),
        ("fit-all (Block)", None),
    ] {
        let (mut app, canvas) = setup(zoom);
        let st = canvas.read(&app).stats();
        let (mut build, mut paint, mut enc) = (Vec::new(), Vec::new(), Vec::new());
        let (mut cmds, mut items) = (0, 0);
        let mut sign = 1.0;
        for i in 0..iters + 20 {
            sign = -sign;
            let t0 = Instant::now();
            NodeCanvas::pan_by(canvas, &mut app, Vec2::new(sign * 3.0, 0.0));
            let frame = app.draw_frame(1.0 / 60.0);
            let t1 = Instant::now();
            let mut list = DisplayList::new();
            let _updates = bridge.paint_scene(&frame.scene, 1.0, &mut list);
            let t2 = Instant::now();
            let _ = encode(
                &list,
                FrameOptions {
                    width: 1600,
                    height: 1000,
                    scale: 1.0,
                    clear: Color::BLACK,
                },
            );
            let t3 = Instant::now();
            cmds = frame.scene.commands.len();
            items = list.items.len();
            if i >= 20 {
                build.push(t1 - t0);
                paint.push(t2 - t1);
                enc.push(t3 - t2);
            }
        }
        let (b, p, e) = (median(&mut build), median(&mut paint), median(&mut enc));
        println!(
            "{name}: {} nodes, {} wires built; {cmds} scene commands, {items} display items",
            st.visible_nodes, st.visible_wires
        );
        println!(
            "  build {:.0} us, paint {:.0} us, encode {:.0} us, total {:.0} us (medians of {iters})",
            us(b),
            us(p),
            us(e),
            us(b + p + e)
        );
    }
}
