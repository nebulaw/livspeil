// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! NodeCanvas with 2000 nodes and 3000 links in a 1600x1000 window: one
//! frame after a pan (render, diff, layout and the core paint list) at
//! zoom 1 (full cards), 0.4 (compact) and fit-all (blocks), and building
//! the per-revision geometry cache.

use criterion::{Criterion, criterion_group, criterion_main};
use livspeil_ui_core::{App, Element, Render, View, ViewContext, div};
use livspeil_ui_layout::{Canvas, Size, Vec2};
use livspeil_ui_widgets::{CardMetrics, DemoGraph, Env, GraphCache, NodeCanvas};

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
        Some(z) => NodeCanvas::set_canvas(
            canvas,
            &mut app,
            Canvas {
                zoom: z,
                pan: Vec2::new(-200.0, -100.0),
                ..Canvas::default()
            },
        ),
        None => NodeCanvas::fit(canvas, &mut app),
    }
    app.draw_frame(1.0 / 60.0);
    (app, canvas)
}

fn bench(c: &mut Criterion) {
    for (name, zoom) in [
        ("pan_frame_zoom_1", Some(1.0)),
        ("pan_frame_zoom_0.4", Some(0.4)),
        ("pan_frame_fit_all", None),
    ] {
        let (mut app, canvas) = setup(zoom);
        let st = canvas.read(&app).stats();
        eprintln!("{name}: {st:?}");
        let mut sign = 1.0;
        c.bench_function(name, |b| {
            b.iter(|| {
                sign = -sign;
                NodeCanvas::pan_by(canvas, &mut app, Vec2::new(sign * 3.0, 0.0));
                app.draw_frame(1.0 / 60.0)
            });
        });
    }
    let graph = DemoGraph::grid(2000, 3000);
    let metrics = CardMetrics::from_theme(&livspeil_ui_theme::Theme::default());
    c.bench_function("graph_cache_build_2k", |b| {
        b.iter(|| GraphCache::build(&graph, metrics));
    });
}

criterion_group!(benches, bench);
criterion_main!(benches);
