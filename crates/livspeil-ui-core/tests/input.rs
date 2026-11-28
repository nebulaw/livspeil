// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Hit testing, event propagation, focus, keymaps, text/IME, drags.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use livspeil_testkit::assert_golden;
use livspeil_ui_core::{
    App, DragPhase, Element, ImeEvent, Key, Keymap, Keystroke, Modifiers, Phase, PointerFilter,
    Render, TestHost, ViewContext, actions, div,
};
use livspeil_ui_layout::{Canvas, Point, Rect, Size, Transform2D, Vec2};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// A view built from a closure (rebuilt on each render).
struct Fn0(Box<dyn Fn() -> Element>);

impl Render for Fn0 {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        (self.0)()
    }
}

fn host(build: impl Fn() -> Element + 'static) -> TestHost {
    let mut app = App::new();
    let v = app.new_view(Fn0(Box::new(build)));
    app.set_root(v);
    let mut host = TestHost::new(app, Size::new(400.0, 400.0));
    host.run_until_idle(20);
    host
}

fn name_at(h: &TestHost, x: f32, y: f32) -> Option<String> {
    h.app
        .hit_test(Point::new(x, y))
        .and_then(|id| h.app.label(id))
}

#[test]
fn hit_testing_with_transforms_and_clips() {
    let h = host(|| {
        div()
            .name("root")
            .size(400.0, 400.0)
            .child(
                // Clips its child, which overflows to the right.
                div()
                    .name("clipper")
                    .absolute(Rect::new(0.0, 0.0, 100.0, 100.0))
                    .clip()
                    .child(
                        div()
                            .name("wide")
                            .absolute(Rect::new(50.0, 10.0, 200.0, 20.0)),
                    ),
            )
            .child(
                // Scaled 2x about its center (150,250): covers 125..175 → 100..200.
                div()
                    .name("scaled")
                    .absolute(Rect::new(125.0, 225.0, 50.0, 50.0))
                    .scale(2.0),
            )
            .child(
                // Translated right by 100: layout 0..50 at y 300 shows at 100..150.
                div()
                    .name("moved")
                    .absolute(Rect::new(0.0, 300.0, 50.0, 50.0))
                    .translate(Vec2::new(100.0, 0.0)),
            )
            .child(
                // Rotated 45° about its center (330,80).
                div()
                    .name("rotated")
                    .absolute(Rect::new(300.0, 50.0, 60.0, 60.0))
                    .transform(Transform2D::rotate(std::f32::consts::FRAC_PI_4)),
            )
            .child(
                // Pass-through overlay.
                div()
                    .name("overlay")
                    .absolute(Rect::new(0.0, 380.0, 400.0, 20.0))
                    .hit_test(false),
            )
    });
    let at = |x, y| name_at(&h, x, y);
    assert_eq!(at(60.0, 20.0).as_deref(), Some("wide"));
    assert_eq!(at(150.0, 20.0).as_deref(), Some("root"), "clipped away");
    assert_eq!(at(105.0, 205.0).as_deref(), Some("scaled"));
    assert_eq!(at(195.0, 295.0).as_deref(), Some("scaled"));
    assert_eq!(at(98.0, 200.0).as_deref(), Some("root"));
    assert_eq!(at(120.0, 320.0).as_deref(), Some("moved"));
    assert_eq!(at(20.0, 320.0).as_deref(), Some("root"));
    // Rotated: the corner of the unrotated box is outside, the diamond tip inside.
    assert_eq!(at(302.0, 52.0).as_deref(), Some("root"));
    assert_eq!(at(330.0, 40.0).as_deref(), Some("rotated"));
    assert_eq!(
        at(10.0, 390.0).as_deref(),
        Some("root"),
        "hit_test(false) passes through"
    );
    assert_eq!(at(500.0, 500.0), None);
}

/// A canvas: a clip over a culling layer element panned and zoomed.
struct Panned;

impl Render for Panned {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let canvas = Canvas {
            pan: Vec2::new(-100.0, 0.0),
            zoom: 2.0,
            ..Canvas::default()
        };
        let world = cx.layer("world");
        world.set_transform(cx, canvas.transform());
        div().name("canvas").size(400.0, 400.0).clip().child(
            div()
                .name("world")
                .w_full()
                .h_full()
                .hit_test(false)
                .layer(world)
                .cull_offscreen()
                .children((0..10).map(|i| {
                    div()
                        .name(format!("n{i}"))
                        .absolute(Rect::new(i as f32 * 100.0, 10.0, 40.0, 40.0))
                        .bg(livspeil_ui_theme::Color::WHITE)
                })),
        )
    }
}

#[test]
fn culling_and_hit_testing_through_pan_zoom() {
    let mut app = App::new();
    let v = app.new_view(Panned);
    app.set_root(v);
    let mut h = TestHost::new(app, Size::new(400.0, 400.0));
    h.run_until_idle(20);
    // Visible canvas x range: (0..400 + 100) / 2 = 50..250, so n0 (0..40) is culled.
    let stats = h.last_frame().unwrap().stats.paint;
    assert_eq!(stats.culled, 8, "only n1 and n2 are visible");
    // n1 at canvas 100..140 → view (100*2-100)=100..180.
    assert_eq!(name_at(&h, 120.0, 40.0).as_deref(), Some("n1"));
    assert_eq!(name_at(&h, 190.0, 40.0).as_deref(), Some("canvas"));
}

#[test]
fn propagation_capture_bubble_and_stop() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = log.clone();
    let mut h = host(move || {
        let rec = |tag: &'static str, l: Rc<RefCell<Vec<String>>>| {
            move |_: &livspeil_ui_core::PointerEvent, cx: &mut livspeil_ui_core::EventCx<'_>| {
                l.borrow_mut().push(format!("{tag} {:?}", cx.phase()));
            }
        };
        let stopper = l.clone();
        div()
            .name("outer")
            .size(300.0, 300.0)
            .on_pointer(PointerFilter::Down, Phase::Capture, rec("outer", l.clone()))
            .on_pointer(PointerFilter::Down, Phase::Bubble, rec("outer", l.clone()))
            .child(
                div()
                    .name("middle")
                    .size(200.0, 200.0)
                    .on_pointer(
                        PointerFilter::Down,
                        Phase::Capture,
                        rec("middle", l.clone()),
                    )
                    .on_pointer(PointerFilter::Down, Phase::Bubble, rec("middle", l.clone()))
                    .on_pointer(
                        PointerFilter::Up,
                        Phase::Bubble,
                        rec("middle-up", l.clone()),
                    )
                    .child(
                        div()
                            .name("inner")
                            .size(100.0, 100.0)
                            .on_pointer(PointerFilter::Down, Phase::Bubble, rec("inner", l.clone()))
                            .on_pointer(PointerFilter::Up, Phase::Bubble, move |_, cx| {
                                stopper.borrow_mut().push("inner-up stop".into());
                                cx.stop_propagation();
                            }),
                    ),
            )
    });
    h.pointer_down(50.0, 50.0);
    h.pointer_up(50.0, 50.0);
    h.pointer_down(150.0, 150.0);
    assert_eq!(
        log.borrow().as_slice(),
        [
            "outer Capture",
            "middle Capture",
            "inner Bubble",
            "middle Bubble",
            "outer Bubble",
            "inner-up stop",
            "outer Capture",
            "middle Capture",
            "middle Bubble",
            "outer Bubble",
        ]
    );
}

#[test]
fn click_requires_release_on_target_and_no_drag() {
    let clicks: Rc<RefCell<Vec<&'static str>>> = Rc::default();
    let c = clicks.clone();
    let mut h = host(move || {
        let (c1, c2) = (c.clone(), c.clone());
        div()
            .size(400.0, 400.0)
            .child(
                div()
                    .name("a")
                    .size(100.0, 100.0)
                    .on_click(move |_, _| c1.borrow_mut().push("a")),
            )
            .child(
                div()
                    .name("b")
                    .size(100.0, 100.0)
                    .on_click(move |_, _| c2.borrow_mut().push("b")),
            )
    });
    h.click(50.0, 50.0);
    h.pointer_down(50.0, 50.0);
    h.pointer_up(150.0, 50.0); // released elsewhere: no click
    h.click(150.0, 50.0);
    assert_eq!(clicks.borrow().as_slice(), ["a", "b"]);
}

fn focus_ui() -> Element {
    div()
        .name("root")
        .size(400.0, 400.0)
        .flex_col()
        .child(div().name("f1").size(10.0, 10.0).focusable())
        .child(div().name("skip").size(10.0, 10.0).tab_index(-1))
        .child(div().name("first").size(10.0, 10.0).tab_index(1))
        .child(
            div()
                .name("dialog")
                .flex_col()
                .focus_scope()
                .child(div().name("d1").size(10.0, 10.0).focusable())
                .child(div().name("d2").size(10.0, 10.0).focusable()),
        )
        .child(div().name("f2").size(10.0, 10.0).focusable())
}

#[test]
fn focus_traversal_tab_order_and_scopes() {
    let mut h = host(focus_ui);
    let name = |h: &TestHost| h.app.focused().and_then(|f| h.app.label(f));
    let order: Vec<String> = h
        .app
        .tab_order()
        .into_iter()
        .filter_map(|id| h.app.label(id))
        .collect();
    assert_eq!(order, ["first", "f1", "d1", "d2", "f2"]);

    h.key("tab");
    assert_eq!(name(&h).as_deref(), Some("first"));
    assert!(h.app.focus_visible());
    h.key("tab");
    h.key("tab");
    assert_eq!(name(&h).as_deref(), Some("d1"));
    // Inside the scope, Tab cycles d1 <-> d2.
    h.key("tab");
    assert_eq!(name(&h).as_deref(), Some("d2"));
    h.key("tab");
    assert_eq!(name(&h).as_deref(), Some("d1"));
    h.key("shift-tab");
    assert_eq!(name(&h).as_deref(), Some("d2"));

    // Pointer focus: no ring. The tab_index(-1) element is focusable by pointer.
    let skip = h.app.find("skip").unwrap();
    let p = h.app.bounds(skip).unwrap().center();
    h.click(p.x, p.y);
    assert_eq!(name(&h).as_deref(), Some("skip"));
    assert!(!h.app.focus_visible());
    h.key("tab");
    assert_eq!(name(&h).as_deref(), Some("first"));
    h.key("shift-tab");
    assert_eq!(name(&h).as_deref(), Some("f2"), "wraps backwards");

    // Clicking empty space blurs.
    h.click(300.0, 300.0);
    assert_eq!(h.app.focused(), None);
}

#[test]
fn focus_ring_is_painted_only_for_keyboard_focus() {
    let mut h = host(focus_ui);
    h.key("tab");
    h.run_until_idle(20);
    let scene = h.scene().unwrap().clone();
    let dump = h.app.dump_scene(&scene);
    assert!(
        dump.contains("quad first -2,18 14x14 r=2 border=2"),
        "{dump}"
    );
    let f1 = h.app.find("f1").unwrap();
    let p = h.app.bounds(f1).unwrap().center();
    h.click(p.x, p.y);
    h.run_until_idle(20);
    let scene = h.scene().unwrap().clone();
    assert!(!h.app.dump_scene(&scene).contains("border=2"));
}

#[test]
fn keymap_actions_contexts_and_text() {
    let log: Rc<RefCell<Vec<String>>> = Rc::default();
    let l = log.clone();
    let mut h = host(move || {
        let (a, b, c, d, e) = (l.clone(), l.clone(), l.clone(), l.clone(), l.clone());
        div()
            .name("root")
            .size(400.0, 400.0)
            .on_action("app.save", move |_| a.borrow_mut().push("root save".into()))
            .child(
                div()
                    .name("editor")
                    .size(100.0, 100.0)
                    .key_context("Editor")
                    .focusable()
                    .on_action("editor.undo", move |_| {
                        b.borrow_mut().push("editor undo".into())
                    })
                    .on_text_input(move |t, _| c.borrow_mut().push(format!("text {t}")))
                    .on_ime(move |ev, _| d.borrow_mut().push(format!("ime {ev:?}")))
                    .on_key(Phase::Capture, move |ev, cx| {
                        if ev.pressed && ev.keystroke() == Keystroke::parse("ctrl-q").unwrap() {
                            e.borrow_mut().push("swallowed ctrl-q".into());
                            cx.stop_propagation();
                        }
                    }),
            )
    });
    let mut keymap = Keymap::defaults();
    keymap.extend(
        &Keymap::parse(
            r#"
[bindings]
"ctrl-s" = "app.save"
"ctrl-z" = "global.undo"
"ctrl-q" = "app.quit"
[contexts.Editor]
"ctrl-z" = "editor.undo"
"#,
        )
        .unwrap(),
    );
    h.app.set_keymap(keymap);
    let editor = h.app.find("editor").unwrap();
    h.app.focus(editor);
    h.key("ctrl-z"); // context binding wins
    h.key("ctrl-s"); // bubbles to root
    h.key("ctrl-q"); // swallowed by a key handler before the keymap
    h.type_text("hé");
    h.ime(ImeEvent::Preedit {
        text: "に".into(),
        cursor: Some((0, 3)),
    });
    h.ime(ImeEvent::Commit("日本".into()));
    h.app.blur();
    h.key("ctrl-z"); // global binding, nobody handles it
    assert!(!h.app.dispatch_action("global.undo"));
    assert_eq!(
        log.borrow().as_slice(),
        [
            "editor undo",
            "root save",
            "swallowed ctrl-q",
            "text hé",
            "ime Preedit { text: \"に\", cursor: Some((0, 3)) }",
            "ime Commit(\"日本\")",
        ]
    );
}

#[test]
fn keymap_parsing_errors_have_locations() {
    let cases = [
        ("unknown-modifier", "[bindings]\n\"hyper-a\" = \"x.y\"\n"),
        ("unknown-key", "[bindings]\n\"ctrl-banana\" = \"x.y\"\n"),
        (
            "bad-action",
            "[contexts.Editor]\n\"ctrl-a\" = \"Select All\"\n",
        ),
        ("unknown-section", "[bind]\n\"a\" = \"b\"\n"),
        ("not-a-string", "[bindings]\n\"a\" = 3\n"),
    ];
    let mut out = String::new();
    for (name, src) in cases {
        let err = Keymap::parse(src).unwrap_err();
        assert!(err.line > 0, "{name}: {err}");
        out.push_str(&format!("{name}: {err}\n"));
    }
    assert_golden(&golden_dir(), "keymap-errors", &out);
    let k = Keymap::defaults();
    let tab = Keystroke::parse("tab").unwrap();
    assert_eq!(
        k.action_for(&tab, &[]).as_deref(),
        Some(actions::FOCUS_NEXT)
    );
    let left = Keystroke::parse("left").unwrap();
    assert_eq!(k.action_for(&left, &[]), None);
    assert_eq!(
        k.action_for(&left, &["Slider".into()]).as_deref(),
        Some(actions::DECREMENT)
    );
}

#[test]
fn drag_gesture_with_threshold_and_velocity() {
    let log: Rc<RefCell<Vec<(DragPhase, Vec2)>>> = Rc::default();
    let l = log.clone();
    let mut h = host(move || {
        let l = l.clone();
        div().size(400.0, 400.0).child(
            div()
                .name("handle")
                .size(50.0, 50.0)
                .on_drag(move |ev, _| l.borrow_mut().push((ev.phase, ev.velocity))),
        )
    });
    // Below the threshold: no drag.
    h.pointer_down(10.0, 10.0);
    h.pointer_move(11.0, 11.0);
    h.pointer_up(11.0, 11.0);
    assert!(log.borrow().is_empty());

    // 10 moves of 5 px, one 60 Hz frame apart: ~300 px/s.
    h.drag(Point::new(10.0, 10.0), Point::new(60.0, 10.0), 10);
    let log = log.borrow();
    assert_eq!(log.first().map(|e| e.0), Some(DragPhase::Start));
    assert_eq!(log.last().map(|e| e.0), Some(DragPhase::End));
    assert!(log.iter().filter(|e| e.0 == DragPhase::Move).count() >= 8);
    let v = log.last().unwrap().1;
    assert!((v.x - 300.0).abs() < 15.0, "release velocity {v:?}");
    assert!(v.y.abs() < 1e-3);
}

#[test]
fn scroll_events_carry_precise_deltas() {
    let got: Rc<RefCell<Vec<Vec2>>> = Rc::default();
    let g = got.clone();
    let mut h = host(move || {
        let g = g.clone();
        div().size(400.0, 400.0).on_scroll(move |ev, _| {
            if let livspeil_ui_core::PointerKind::Scroll(d) = ev.kind {
                g.borrow_mut().push(d.pixels(16.0));
            }
        })
    });
    h.scroll(10.0, 10.0, Vec2::new(0.0, -3.5));
    assert_eq!(got.borrow().as_slice(), [Vec2::new(0.0, -3.5)]);
}

#[test]
fn a_shortcut_that_moves_focus_does_not_type_into_the_new_target() {
    let typed = Rc::new(RefCell::new(String::new()));
    let seen = typed.clone();
    let mut h = host(move || {
        let seen = seen.clone();
        div()
            .name("root")
            .size(400.0, 400.0)
            .child(
                div()
                    .name("from")
                    .size(10.0, 10.0)
                    .focusable()
                    .on_action("open", |cx| {
                        if let Some(field) = cx.find("field") {
                            cx.focus(field);
                        }
                    }),
            )
            .child(
                div()
                    .name("field")
                    .size(10.0, 10.0)
                    .focusable()
                    .on_text_input(move |t, _| seen.borrow_mut().push_str(t)),
            )
    });
    let mut keymap = Keymap::default();
    keymap.bind(Keystroke::parse("shift-a").unwrap(), "open", None);
    h.app.set_keymap(keymap);
    let from = h.app.find("from").unwrap();
    h.app.focus(from);
    // As a window delivers it: the key, its text, then the release.
    let key = Keystroke::parse("shift-a").unwrap();
    h.key_event(key.key, key.modifiers, true);
    h.type_text("A");
    h.key_event(key.key, key.modifiers, false);
    assert_eq!(h.app.focused(), h.app.find("field"));
    assert_eq!(*typed.borrow(), "");
    // The next keystroke types as usual.
    h.key_event(Key::Char('b'), Modifiers::NONE, true);
    h.type_text("b");
    assert_eq!(*typed.borrow(), "b");
}
