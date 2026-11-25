// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! NodeCanvas: pan/zoom math, selection rules, moving, wire snapping and
//! type rejection, culling and level of detail.

mod common;

use common::{down, find, host_sized, moved, pointer, up};
use livspeil_ui_core::{
    Element, Key, Model, Modifiers, MouseButton, NamedKey, PointerKind, ScrollDelta, TestHost,
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
    f.view.update(&mut f.h.app, |s, cx| {
        s.set_canvas(Canvas {
            zoom: 2.0,
            ..Canvas::default()
        });
        cx.notify();
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
    assert!(st.visible_nodes > 10 && st.visible_nodes < 60, "{st:?}");
    assert!(st.visible_wires < 300, "{st:?}");
    assert_eq!(st.lod, Lod::Full);
    let frame = f.h.last_frame().unwrap().stats;
    assert!(frame.nodes < 3000, "{} retained nodes", frame.nodes);
    // Zoomed out to fit: blocks, everything visible, still one element
    // per node plus wires.
    NodeCanvas::fit(f.view, &mut f.h.app);
    f.h.run_until_idle(50);
    let st = f.s().stats();
    assert_eq!(st.lod, Lod::Block);
    assert_eq!(st.visible_nodes, 2000);
    assert_eq!(st.visible_wires, 3000);
    let frame = f.h.last_frame().unwrap().stats;
    assert!(
        frame.nodes < 2000 + 2000 + 200,
        "{} retained nodes",
        frame.nodes
    );
    // Panning far away culls everything.
    f.view.update(&mut f.h.app, |s, cx| {
        s.set_canvas(Canvas {
            pan: Vec2::new(-1e6, -1e6),
            ..Canvas::default()
        });
        cx.notify();
    });
    f.h.run_until_idle(50);
    assert_eq!(f.s().stats().visible_nodes, 0);
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
