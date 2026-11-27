// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! NodeCanvas: pan/zoom math, selection rules, moving, wire snapping and
//! type rejection, culling and level of detail.

mod common;

use common::{down, find, host_sized, moved, pointer, scene, up};
use livspeil_ui_core::{
    Element, InputEvent, Key, Model, Modifiers, MouseButton, NamedKey, PointerKind, ScrollDelta, TestHost,
    View, div,
};
use livspeil_ui_layout::{Canvas, Point, Size, Vec2};
use livspeil_ui_theme::MotionConfig;
use livspeil_ui_widgets::{
    CardMetrics, DemoGraph, GraphCache, Link, Lod, NodeCanvas, NodeGraphModel, ParamValue, PortRef,
    find_snap, rubber_select,
};

type DemoCanvas = NodeCanvas<DemoGraph>;

struct Fx {
    h: TestHost,
    graph: Model<DemoGraph>,
    view: View<DemoCanvas>,
}

fn fixture_with(graph: DemoGraph, size: Size) -> Fx {
    let mut out = None;
    let (h, _) = host_sized(size, |app, env| {
        let graph = app.new_model(graph);
        let view = app.new_view(NodeCanvas::new(env, graph));
        out = Some((graph, view));
        Box::new(move |_| div().w_full().h_full().child(Element::view(&view)))
    });
    let (graph, view) = out.unwrap();
    let mut f = Fx { h, graph, view };
    // Let the canvas learn its frame.
    f.h.pointer_move(1.0, 1.0);
    f.h.run_until_idle(100);
    f
}

fn fixture() -> Fx {
    fixture_with(DemoGraph::sample(), Size::new(900.0, 600.0))
}

impl Fx {
    fn s(&self) -> &DemoCanvas {
        self.view.read(&self.h.app)
    }
    fn canvas(&self) -> Canvas {
        self.s().canvas()
    }
    fn sel(&self) -> Vec<u64> {
        self.s().selection().iter().copied().collect()
    }
    fn click_mod(&mut self, p: Point, m: Modifiers) {
        moved(&mut self.h, p, m);
        down(&mut self.h, p, m);
        up(&mut self.h, p, m);
        self.h.run_until_idle(100);
    }
    fn node(&self, title: &str) -> Point {
        let b = self
            .h
            .app
            .bounds(find(&self.h, &format!("node:{title}")))
            .unwrap();
        // Bounds are untransformed layout bounds = canvas space here; map
        // to the window through the canvas (origin at 0,0).
        self.canvas().to_view(Point::new(b.x + 100.0, b.y + 12.0))
    }
    fn port(&self, name: &str) -> Point {
        let b = self.h.app.bounds(find(&self.h, name)).unwrap();
        self.canvas().to_view(b.center())
    }
}

const SHIFT: Modifiers = Modifiers::SHIFT;
const CTRL: Modifiers = Modifiers {
    shift: false,
    ctrl: true,
    alt: false,
    cmd: false,
};

#[test]
fn zoom_keeps_the_point_under_the_cursor() {
    // The math.
    let mut c = Canvas {
        pan: Vec2::new(30.0, -20.0),
        zoom: 0.8,
        ..Canvas::default()
    };
    let anchor = Point::new(300.0, 200.0);
    let before = c.to_canvas(anchor);
    c.zoom_at(anchor, 1.7);
    assert!((c.to_canvas(anchor) - before).length() < 1e-3);

    // The widget, through ctrl+scroll (pinch), animated.
    let mut f = fixture();
    let p = Point::new(420.0, 260.0);
    let under = f.canvas().to_canvas(p);
    pointer(
        &mut f.h,
        PointerKind::Scroll(ScrollDelta::Pixels(Vec2::new(0.0, 40.0))),
        p,
        CTRL,
    );
    f.h.step();
    f.h.step();
    assert!(f.s().is_zooming());
    let mid = f.canvas();
    assert!(mid.zoom > 1.0 && mid.zoom < 0.4f32.exp());
    assert!(
        (mid.to_canvas(p) - under).length() < 1e-2,
        "fixed mid-flight"
    );
    f.h.run_until_idle(200);
    let c = f.canvas();
    assert!((c.zoom - 0.4f32.exp()).abs() < 1e-4, "{}", c.zoom);
    assert!((c.to_canvas(p) - under).length() < 1e-2);
    // Wheel lines zoom too.
    pointer(
        &mut f.h,
        PointerKind::Scroll(ScrollDelta::Lines(Vec2::new(0.0, -2.0))),
        p,
        Modifiers::NONE,
    );
    f.h.run_until_idle(200);
    assert!((f.canvas().zoom - 0.4f32.exp() / 1.15f32.powi(2)).abs() < 1e-3);
    assert!((f.canvas().to_canvas(p) - under).length() < 1e-2);
    // Reduce motion: instant.
    f.h.app.set_motion(MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    });
    pointer(
        &mut f.h,
        PointerKind::Scroll(ScrollDelta::Lines(Vec2::new(0.0, 1.0))),
        p,
        Modifiers::NONE,
    );
    assert!(!f.s().is_zooming());
    assert!((f.canvas().to_canvas(p) - under).length() < 1e-2);
}

#[test]
fn pan_with_trackpad_middle_drag_and_space() {
    let mut f = fixture();
    f.h.scroll(400.0, 400.0, Vec2::new(-30.0, 10.0));
    f.h.run_until_idle(50);
    assert_eq!(f.canvas().pan, Vec2::new(-30.0, 10.0));
    // Middle drag, even over a node.
    let n = f.node("Mix");
    pointer(&mut f.h, PointerKind::Move, n, Modifiers::NONE);
    pointer(
        &mut f.h,
        PointerKind::Down(MouseButton::Middle),
        n,
        Modifiers::NONE,
    );
    for i in 1..=4 {
        f.h.step();
        pointer(
            &mut f.h,
            PointerKind::Move,
            n + Vec2::new(10.0 * i as f32, 0.0),
            Modifiers::NONE,
        );
    }
    pointer(
        &mut f.h,
        PointerKind::Up(MouseButton::Middle),
        n + Vec2::new(40.0, 0.0),
        Modifiers::NONE,
    );
    f.h.run_until_idle(50);
    assert_eq!(f.canvas().pan, Vec2::new(10.0, 10.0));
    assert_eq!(
        f.graph.read(&f.h.app).nodes[2].position,
        Point::new(280.0, 70.0),
        "the node did not move"
    );
    // Space + drag on the background.
    let canvas_el = find(&f.h, "node-canvas");
    f.h.app.focus(canvas_el);
    f.h.key_event(Key::Named(NamedKey::Space), Modifiers::NONE, true);
    f.h.drag(Point::new(850.0, 30.0), Point::new(850.0, 80.0), 5);
    f.h.key_event(Key::Named(NamedKey::Space), Modifiers::NONE, false);
    f.h.run_until_idle(50);
    assert_eq!(f.canvas().pan, Vec2::new(10.0, 60.0));
    assert!(f.sel().is_empty(), "no rubber band while panning");
}

#[test]
fn selection_rules() {
    let mut f = fixture();
    let (time, noise, mix) = (f.node("Time"), f.node("Noise"), f.node("Mix"));
    f.click_mod(time, Modifiers::NONE);
    assert_eq!(f.sel(), vec![1]);
    f.click_mod(mix, SHIFT);
    assert_eq!(f.sel(), vec![1, 3]);
    f.click_mod(time, SHIFT);
    assert_eq!(f.sel(), vec![3], "shift toggles");
    f.click_mod(noise, SHIFT);
    f.click_mod(time, SHIFT);
    assert_eq!(f.sel(), vec![1, 2, 3]);
    f.click_mod(mix, Modifiers::NONE);
    assert_eq!(f.sel(), vec![3], "a plain click selects only it");
    f.click_mod(Point::new(850.0, 20.0), Modifiers::NONE);
    assert!(f.sel().is_empty(), "empty space clears");
    // Rubber band over Time and Noise.
    f.h.drag(Point::new(20.0, 20.0), Point::new(120.0, 200.0), 6);
    f.h.run_until_idle(50);
    assert_eq!(f.sel(), vec![1, 2]);
    assert!(f.s().rubber_band().is_none());
    // Shift adds.
    let m = f.node("Mix");
    f.h.pointer_move(m.x, 20.0);
    down(&mut f.h, Point::new(m.x, 20.0), SHIFT);
    for i in 1..=4 {
        f.h.step();
        moved(&mut f.h, Point::new(m.x, 20.0 + 15.0 * i as f32), SHIFT);
    }
    assert!(f.s().rubber_band().is_some());
    f.h.run_until_idle(10);
    assert!(f.h.app.find("rubber-band").is_some());
    up(&mut f.h, Point::new(m.x, 80.0), SHIFT);
    f.h.run_until_idle(50);
    assert_eq!(f.sel(), vec![1, 2, 3]);
    // Select all, Escape clears.
    let canvas_el = find(&f.h, "node-canvas");
    f.h.app.focus(canvas_el);
    f.h.key("ctrl-a");
    f.h.run_until_idle(50);
    assert_eq!(f.sel().len(), 6);
    f.h.key("escape");
    f.h.run_until_idle(50);
    assert!(f.sel().is_empty());
}

#[test]
fn dragging_moves_the_selection() {
    let mut f = fixture();
    let (time, noise) = (f.node("Time"), f.node("Noise"));
    f.click_mod(time, Modifiers::NONE);
    f.click_mod(noise, SHIFT);
    f.h.drag(time, time + Vec2::new(50.0, 20.0), 5);
    f.h.run_until_idle(50);
    let g = f.graph.read(&f.h.app);
    assert_eq!(g.nodes[0].position, Point::new(90.0, 60.0));
    assert_eq!(g.nodes[1].position, Point::new(90.0, 170.0));
    assert_eq!(f.sel(), vec![1, 2], "dragging keeps the selection");
    // At zoom 2 a drag moves half as far in canvas space.
    DemoCanvas::set_canvas(f.view, &mut f.h.app, Canvas {
            zoom: 2.0,
            ..Canvas::default()
        });
    f.h.run_until_idle(50);
    let mix = f.node("Mix");
    f.h.drag(mix, mix + Vec2::new(40.0, 0.0), 4);
    f.h.run_until_idle(50);
    assert_eq!(
        f.graph.read(&f.h.app).nodes[2].position,
        Point::new(300.0, 70.0)
    );
    assert_eq!(f.sel(), vec![3]);
}

#[test]
fn wires_snap_to_compatible_ports_only() {
    let mut f = fixture();
    let blur_radius = f.port("port:5:in:1"); // float
    let blur_image = f.port("port:5:in:0"); // image
    let noise_out = f.port("port:2:out:0"); // float
    // Toward the image input: no snap.
    f.h.pointer_move(noise_out.x, noise_out.y);
    f.h.pointer_down(noise_out.x, noise_out.y);
    for i in 1..=6 {
        f.h.step();
        let t = i as f32 / 6.0;
        let p = noise_out + (blur_image + Vec2::new(4.0, 3.0) - noise_out) * t;
        f.h.pointer_move(p.x, p.y);
    }
    f.h.run_until_idle(10);
    let (from, snap) = f.s().pending_wire().unwrap();
    assert_eq!(from, PortRef::output(2, 0));
    assert_eq!(snap, None, "image input rejected for a float output");
    assert!(f.h.app.find("pending-wire").is_some());
    // Close to the float input: snaps.
    let p = blur_radius + Vec2::new(10.0, -6.0);
    f.h.pointer_move(p.x, p.y);
    f.h.run_until_idle(10);
    assert_eq!(f.s().pending_wire().unwrap().1, Some(PortRef::input(5, 1)));
    f.h.pointer_up(p.x, p.y);
    f.h.run_until_idle(50);
    assert!(f.s().pending_wire().is_none());
    assert!(f.graph.read(&f.h.app).links.contains(&Link {
        from: PortRef::output(2, 0),
        to: PortRef::input(5, 1),
    }));
    // Released without a snap: nothing is connected.
    let before = f.graph.read(&f.h.app).links.len();
    f.h.drag(noise_out, blur_image, 6);
    f.h.run_until_idle(50);
    assert_eq!(f.graph.read(&f.h.app).links.len(), before);
    // From an input backward to an output.
    let ramp_in = f.port("port:4:in:0");
    let time_out = f.port("port:1:out:0");
    f.h.drag(ramp_in, time_out + Vec2::new(-5.0, 5.0), 6);
    f.h.run_until_idle(50);
    assert!(f.graph.read(&f.h.app).links.contains(&Link {
        from: PortRef::output(1, 0),
        to: PortRef::input(4, 0),
    }));
}

#[test]
fn snapping_math() {
    let g = DemoGraph::sample();
    let theme = livspeil_ui_theme::Theme::default();
    let cache = GraphCache::build(&g, CardMetrics::from_theme(&theme));
    let radius_in = cache.port_position(PortRef::input(5, 1)).unwrap();
    let ok = |o, i| g.can_connect(o, i);
    let snap = find_snap(
        &cache,
        PortRef::output(2, 0),
        radius_in + Vec2::new(8.0, 0.0),
        24.0,
        ok,
    );
    assert_eq!(snap, Some(PortRef::input(5, 1)));
    assert_eq!(
        find_snap(
            &cache,
            PortRef::output(2, 0),
            radius_in + Vec2::new(30.0, 0.0),
            24.0,
            ok
        ),
        None,
        "out of range"
    );
    // Never to the same node.
    let mix_in = cache.port_position(PortRef::input(3, 0)).unwrap();
    assert_eq!(
        find_snap(&cache, PortRef::output(3, 0), mix_in, 24.0, ok),
        None
    );
    assert!(!g.can_connect(PortRef::output(3, 0), PortRef::input(3, 0)));
    // Rubber band.
    let mut sel = rubber_select(&cache, Point::new(0.0, 0.0), Point::new(60.0, 60.0));
    sel.sort_unstable();
    assert_eq!(sel, vec![1]);
}

#[test]
fn culling_and_level_of_detail_with_2k_nodes() {
    let mut f = fixture_with(DemoGraph::grid(2000, 3000), Size::new(1200.0, 800.0));
    let st = f.s().stats();
    assert_eq!((st.nodes, st.links), (2000, 3000));
    assert!(st.built_nodes > 10 && st.built_nodes < 60, "{st:?}");
    assert!(st.built_wires < 300, "{st:?}");
    assert_eq!(st.lod, Lod::Full);
    let frame = f.h.last_frame().unwrap().stats;
    assert!(frame.nodes < 3000, "{} retained nodes", frame.nodes);
    // Zoomed out to fit: blocks, everything visible, still one element
    // per node plus wires.
    NodeCanvas::fit(f.view, &mut f.h.app);
    f.h.run_until_idle(50);
    let st = f.s().stats();
    assert_eq!(st.lod, Lod::Block);
    assert_eq!(st.built_nodes, 2000);
    assert_eq!(st.built_wires, 3000);
    let frame = f.h.last_frame().unwrap().stats;
    assert!(
        frame.nodes < 2000 + 2000 + 200,
        "{} retained nodes",
        frame.nodes
    );
    // Panning far away culls everything.
    DemoCanvas::set_canvas(f.view, &mut f.h.app, Canvas {
            pan: Vec2::new(-1e6, -1e6),
            ..Canvas::default()
        });
    f.h.run_until_idle(50);
    assert_eq!(f.s().stats().built_nodes, 0);
}

#[test]
fn parameters_edit_the_model() {
    let mut f = fixture();
    // The Clamp toggle of Color Ramp.
    assert!(f.h.app.find("node:Color Ramp").is_some());
    let toggle = f.h.app.find("toggle").expect("a toggle row in a card");
    let b = f.h.app.bounds(toggle).unwrap();
    let p = f.canvas().to_view(b.center());
    f.h.click(p.x, p.y);
    f.h.run_until_idle(100);
    assert_eq!(
        f.graph.read(&f.h.app).nodes[3].params[0].value,
        ParamValue::Toggle(false)
    );
    // A number field: Up steps the Mix factor.
    let field = find(&f.h, "param:3:0");
    f.h.app.focus(field);
    f.h.key("up");
    f.h.run_until_idle(100);
    let ParamValue::Number { value, .. } = f.graph.read(&f.h.app).nodes[2].params[0].value else {
        panic!()
    };
    assert!((value - 0.51).abs() < 1e-9, "{value}");
}

#[test]
fn selection_ring_paints_above_the_header_and_frame_comes_from_layout() {
    let mut out = None;
    let (mut h, _) = host_sized(Size::new(900.0, 600.0), |app, env| {
        let graph = app.new_model(DemoGraph::sample());
        let view = app.new_view(NodeCanvas::new(env, graph));
        out = Some(view);
        Box::new(move |_| div().w_full().h_full().p(20.0).child(Element::view(&view)))
    });
    let view = out.unwrap();
    h.run_until_idle(100);
    // No pointer event yet: the frame is the canvas's laid-out box.
    let (origin, size) = view.read(&h.app).frame();
    assert_eq!(
        (origin, size),
        (Point::new(20.0, 20.0), Size::new(860.0, 560.0))
    );
    view.update(&mut h.app, |v, cx| {
        v.set_selection([3]);
        cx.notify();
    });
    h.run_until_idle(100);
    let card = find(&h, "node:Mix");
    let kids: Vec<String> = h
        .app
        .children_of(card)
        .into_iter()
        .filter_map(|c| h.app.label(c))
        .collect();
    let header = kids.iter().position(|k| k == "header").unwrap();
    let ring = kids.iter().position(|k| k == "selection-ring").unwrap();
    let first_port = kids.iter().position(|k| k.starts_with("port:")).unwrap();
    assert!(header < ring && ring < first_port, "{kids:?}");
    // The parameter field is named for assistive technology.
    let field = find(&h, "param:3:0");
    assert_eq!(
        h.app
            .semantic_node(field)
            .and_then(|n| n.label().map(str::to_owned)),
        Some("Factor".to_owned())
    );
}

/// One frame after `change`: views rendered, and whether layout ran.
fn frame_after(f: &mut Fx, change: impl FnOnce(&mut Fx)) -> (usize, bool) {
    change(f);
    let frame = f.h.step().expect("a frame").stats;
    (frame.views_rendered, frame.layout)
}

#[test]
fn panning_and_zooming_inside_the_built_region_render_nothing() {
    let mut f = fixture_with(DemoGraph::grid(500, 700), Size::new(1200.0, 800.0));
    let card = find(&f.h, "node:Node 1");
    let (before, built) = (f.h.app.visual_bounds(card).unwrap(), f.s().stats());
    let layouts = f.h.app.layout_stats();
    let view = f.view;
    for _ in 0..20 {
        let stats = frame_after(&mut f, |f| {
            DemoCanvas::pan_by(view, &mut f.h.app, Vec2::new(-5.0, -3.0));
        });
        assert_eq!(stats, (0, false));
    }
    assert_eq!(f.h.app.layout_stats(), layouts);
    assert_eq!(f.s().stats(), built);
    let after = f.h.app.visual_bounds(card).unwrap();
    assert_eq!(after.origin(), before.origin() + Vec2::new(-100.0, -60.0));
    // The minimap's viewport frame followed.
    let mark = find(&f.h, "minimap-viewport");
    let x = f.h.app.bounds(mark).unwrap().x;
    DemoCanvas::pan_by(view, &mut f.h.app, Vec2::new(-150.0, 0.0));
    f.h.run_until_idle(10);
    assert!(f.h.app.bounds(mark).unwrap().x > x);
    // A small zoom of the same level of detail.
    let mut canvas = f.canvas();
    canvas.zoom_at(Point::new(600.0, 400.0), 1.1);
    let stats = frame_after(&mut f, |f| DemoCanvas::set_canvas(view, &mut f.h.app, canvas));
    assert_eq!(stats, (0, false));
}

#[test]
fn the_built_region_follows_with_hysteresis() {
    let mut f = fixture_with(DemoGraph::grid(500, 700), Size::new(1200.0, 800.0));
    let view = f.view;
    let renders = |f: &Fx| f.h.app.render_count(view.id());
    let start = renders(&f);
    // 25% of 1200 px: the first rebuild comes after 300 px.
    let mut frames = 0;
    while renders(&f) == start {
        DemoCanvas::pan_by(view, &mut f.h.app, Vec2::new(-10.0, 0.0));
        f.h.run_until_idle(10);
        frames += 1;
        assert!(frames < 100);
    }
    assert_eq!(frames, 31);
    // Going back and forth over that point does not rebuild again.
    let rebuilt = renders(&f);
    for _ in 0..10 {
        for dx in [40.0, -40.0] {
            DemoCanvas::pan_by(view, &mut f.h.app, Vec2::new(dx, 0.0));
            f.h.run_until_idle(10);
        }
    }
    assert_eq!(renders(&f), rebuilt);
    // A long pan rebuilds now and then, not every frame.
    for _ in 0..200 {
        DemoCanvas::pan_by(view, &mut f.h.app, Vec2::new(-10.0, 0.0));
        f.h.run_until_idle(10);
    }
    let long = renders(&f) - rebuilt;
    assert!((5..=8).contains(&long), "{long} renders for 2000 px");

    // A level of detail change renders at once.
    let mut canvas = f.canvas();
    canvas.zoom_at(Point::new(600.0, 400.0), 0.5);
    let before = renders(&f);
    DemoCanvas::set_canvas(view, &mut f.h.app, canvas);
    f.h.run_until_idle(10);
    assert_eq!(f.s().stats().lod, Lod::Compact);
    assert_eq!(renders(&f), before + 1);
    // So does zooming in far enough that most of what is built is unused.
    canvas.zoom_at(Point::new(600.0, 400.0), 2.0);
    DemoCanvas::set_canvas(view, &mut f.h.app, canvas);
    f.h.run_until_idle(10);
    let wide = f.s().stats().built_nodes;
    canvas.zoom_at(Point::new(600.0, 400.0), 4.0);
    DemoCanvas::set_canvas(view, &mut f.h.app, canvas);
    f.h.run_until_idle(10);
    assert!(f.s().stats().built_nodes < wide / 4);
}

#[test]
fn an_animated_zoom_renders_only_when_the_detail_changes() {
    let mut f = fixture_with(DemoGraph::grid(200, 300), Size::new(1200.0, 800.0));
    let view = f.view;
    DemoCanvas::zoom_about(view, &mut f.h.app, Point::new(600.0, 400.0), 1.3);
    let mut renders = 0;
    let mut frames = 0;
    while let Some(frame) = f.h.step() {
        renders += frame.stats.views_rendered;
        assert!(!frame.stats.layout);
        frames += 1;
        assert!(frames < 200);
    }
    assert!(frames > 5, "{frames}");
    assert_eq!(renders, 0);
    assert!(!f.s().is_zooming());
    assert_eq!(f.canvas().zoom, 1.3);
}

#[test]
fn a_canvas_inside_scaled_and_moved_content_maps_the_pointer() {
    let mut out = None;
    let (mut h, _) = host_sized(Size::new(900.0, 600.0), |app, env| {
        let graph = app.new_model(DemoGraph::sample());
        let view = app.new_view(NodeCanvas::new(env, graph));
        out = Some(view);
        Box::new(move |_| {
            div().w_full().h_full().child(
                div()
                    .w_full()
                    .h_full()
                    .translate(Vec2::new(50.0, 30.0))
                    .child(Element::view(&view)),
            )
        })
    });
    let view = out.unwrap();
    // The canvas point under the pointer stays: pointer positions are read
    // through the canvas's own transforms.
    let pointer = Point::new(350.0, 230.0);
    let under = view.read(&h.app).canvas().to_canvas(Point::new(300.0, 200.0));
    pointer_scroll(&mut h, pointer);
    h.run_until_idle(100);
    let canvas = view.read(&h.app).canvas();
    assert!(canvas.zoom > 1.0);
    assert!((canvas.to_canvas(Point::new(300.0, 200.0)) - under).length() < 1e-2);
}

fn pointer_scroll(h: &mut TestHost, p: Point) {
    pointer(
        h,
        PointerKind::Scroll(ScrollDelta::Lines(Vec2::new(0.0, 1.0))),
        p,
        Modifiers::NONE,
    );
}

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % n
    }

    fn between(&mut self, min: i32, max: i32) -> f32 {
        let n = self.below(u64::try_from(max - min).unwrap());
        f32::from(u16::try_from(n).unwrap()) + f32::from(i16::try_from(min).unwrap())
    }
}

/// After random pans, zooms, node moves and selections, the canvas that was
/// maintained incrementally (layers, virtualization) paints exactly what a
/// new one paints.
#[test]
fn retained_equals_fresh() {
    const SIZE: Size = Size::new(900.0, 600.0);
    for seed in 0..4 {
        let mut rng = Rng(seed);
        let mut f = fixture_with(DemoGraph::grid(60, 80), SIZE);
        // The fixture's pointer move must not leave a hover behind.
        f.h.app.dispatch(InputEvent::PointerLeft);
        f.h.run_until_idle(100);
        let view = f.view;
        let mut checked = 0;
        for step in 0..120 {
            match rng.below(8) {
                0..=2 => {
                    let by = Vec2::new(rng.between(-250, 250), rng.between(-200, 200));
                    DemoCanvas::pan_by(view, &mut f.h.app, by);
                }
                3 | 4 => {
                    let mut c = f.canvas();
                    let at = Point::new(rng.between(0, 900), rng.between(0, 600));
                    c.zoom_at(at, [0.6, 0.85, 0.97, 1.03, 1.2, 1.7][rng.below(6) as usize]);
                    c.zoom = c.zoom.clamp(0.15, 2.5);
                    DemoCanvas::set_canvas(view, &mut f.h.app, c);
                }
                5 => {
                    let node = 1 + rng.below(60);
                    let to = Point::new(rng.between(0, 1500), rng.between(0, 900));
                    f.graph.update(&mut f.h.app, |g, cx| {
                        g.set_position(node, to);
                        cx.notify();
                    });
                }
                6 => {
                    let ids = [1 + rng.below(60), 1 + rng.below(60)];
                    view.update(&mut f.h.app, |s, cx| {
                        s.set_selection(ids[..rng.below(3) as usize].to_vec());
                        cx.notify();
                    });
                }
                _ => {
                    DemoCanvas::fit(view, &mut f.h.app);
                }
            }
            f.h.run_until_idle(200);
            if step % 3 != 0 {
                continue;
            }
            let graph = f.graph.read(&f.h.app).clone();
            let (canvas, selection) = (f.canvas(), f.s().selection().clone());
            let mut fresh = fixture_with(graph, SIZE);
            fresh.h.app.dispatch(InputEvent::PointerLeft);
            DemoCanvas::set_canvas(fresh.view, &mut fresh.h.app, canvas);
            fresh.view.update(&mut fresh.h.app, |s, cx| {
                s.set_selection(selection);
                cx.notify();
            });
            fresh.h.run_until_idle(200);
            f.h.app.request_frame();
            fresh.h.app.request_frame();
            f.h.step();
            fresh.h.step();
            let (a, b) = (scene(&f.h), scene(&fresh.h));
            assert!(a == b, "seed {seed} step {step}: {}", first_difference(&a, &b));
            checked += 1;
        }
        assert!(checked >= 40);
    }
}

fn first_difference(a: &str, b: &str) -> String {
    let line = a.lines().zip(b.lines()).position(|(x, y)| x != y);
    let at = line.unwrap_or_else(|| a.lines().count().min(b.lines().count()));
    format!(
        "line {at} of {}/{}:\n  retained {:?}\n  fresh    {:?}",
        a.lines().count(),
        b.lines().count(),
        a.lines().nth(at),
        b.lines().nth(at)
    )
}
