// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Models, observation, re-render minimality, element ids, the frame loop.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use livspeil_testkit::assert_golden;
use livspeil_ui_core::{
    App, Element, FrameScheduler, Model, Render, TestHost, View, ViewContext, div, text,
};
use livspeil_ui_layout::Size;
use livspeil_ui_theme::{Color, Elevation, Material};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

struct Label {
    model: Model<String>,
}

impl Render for Label {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let s = self.model.read(cx).clone();
        div().p(4.0).child(text(s))
    }
}

struct Parent {
    title: Model<u32>,
    left: View<Label>,
    right: View<Label>,
}

impl Render for Parent {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let n = *self.title.read(cx);
        div()
            .flex_col()
            .w(400.0)
            .child(text(format!("title {n}")))
            .child(Element::view(&self.left))
            .child(Element::view(&self.right))
    }
}

struct Fixture {
    host: TestHost,
    title: Model<u32>,
    a: Model<String>,
    b: Model<String>,
    parent: View<Parent>,
    left: View<Label>,
    right: View<Label>,
}

fn fixture() -> Fixture {
    let mut app = App::new();
    let title = app.new_model(0_u32);
    let a = app.new_model("a".to_owned());
    let b = app.new_model("b".to_owned());
    let left = app.new_view(Label { model: a });
    let right = app.new_view(Label { model: b });
    let parent = app.new_view(Parent { title, left, right });
    app.set_root(parent);
    let mut host = TestHost::new(app, Size::new(400.0, 300.0));
    host.run_until_idle(10);
    Fixture {
        host,
        title,
        a,
        b,
        parent,
        left,
        right,
    }
}

fn counts(f: &Fixture) -> (usize, usize, usize) {
    let app = &f.host.app;
    (
        app.render_count(f.parent.id()),
        app.render_count(f.left.id()),
        app.render_count(f.right.id()),
    )
}

#[test]
fn notify_rerenders_only_observers() {
    let mut f = fixture();
    assert_eq!(counts(&f), (1, 1, 1));

    // Changing model A re-renders only the left label.
    f.a.update(&mut f.host.app, |s, cx| {
        s.push('!');
        cx.notify();
    });
    f.host.run_until_idle(10);
    assert_eq!(counts(&f), (1, 2, 1));
    assert!(f.host.app.dump_tree().contains("\"a!\""));

    // Updating without notify does nothing.
    f.b.update(&mut f.host.app, |s, _| s.push('?'));
    assert!(!f.host.app.needs_frame());
    assert_eq!(f.host.run_until_idle(10), 0);
    assert_eq!(counts(&f), (1, 2, 1));

    // The parent re-renders on its own model; child views stay cached.
    f.title.update(&mut f.host.app, |t, cx| {
        *t += 1;
        cx.notify();
    });
    f.host.run_until_idle(10);
    assert_eq!(counts(&f), (2, 2, 1));

    // Notifying both children in one batch renders each once.
    let (a, b) = (f.a, f.b);
    a.update(&mut f.host.app, |_, cx| {
        cx.notify();
        b.update(cx, |_, cx| cx.notify());
        b.update(cx, |_, cx| cx.notify());
    });
    let frame = f.host.step().unwrap();
    assert_eq!(frame.stats.views_rendered, 2);
    assert_eq!(counts(&f), (2, 3, 2));
}

#[test]
fn observers_run_after_outermost_update() {
    let mut app = App::new();
    let m = app.new_model(0);
    let seen = Rc::new(Cell::new(-1));
    let s = seen.clone();
    let sub = app.observe(m.id(), move |app| s.set(*m.read(app)));
    m.update(&mut app, |v, cx| {
        *v = 5;
        cx.notify();
        // Still inside the update: the observer has not run yet.
    });
    assert_eq!(seen.get(), 5);
    app.unsubscribe(sub);
    m.update(&mut app, |v, cx| {
        *v = 6;
        cx.notify();
    });
    assert_eq!(seen.get(), 5);
}

#[test]
fn idle_produces_no_frames() {
    let mut f = fixture();
    assert!(!f.host.app.needs_frame());
    let before = f.host.frames();
    assert_eq!(f.host.advance(std::time::Duration::from_secs(2)), 0);
    // Pointer movement over content without hover styles draws nothing.
    f.host.pointer_move(10.0, 10.0);
    f.host.pointer_move(20.0, 12.0);
    assert_eq!(f.host.run_until_idle(10), 0);
    assert_eq!(f.host.frames(), before);
}

#[test]
fn scheduler_dt_after_idle_is_nominal() {
    let mut s = FrameScheduler::new(std::time::Duration::from_millis(10));
    let ms = std::time::Duration::from_millis;
    assert!((s.begin(ms(1000)) - 0.010).abs() < 1e-6);
    s.end(true);
    assert!((s.begin(ms(1016)) - 0.016).abs() < 1e-6);
    s.end(true);
    // A long stall is clamped.
    assert!((s.begin(ms(2000)) - 0.050).abs() < 1e-6);
    s.end(false);
    // After idle, the first frame advances by the nominal interval.
    assert!((s.begin(ms(9000)) - 0.010).abs() < 1e-6);
}

// ---- element ids and keyed reordering --------------------------------

struct List {
    items: Model<Vec<&'static str>>,
    keyed: bool,
}

impl Render for List {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let keyed = self.keyed;
        let items = self.items.read(cx).clone();
        div()
            .flex_col()
            .children(items.into_iter().map(move |name| {
                let e = div().name(name).h(20.0).focusable();
                if keyed { e.key(name) } else { e }
            }))
    }
}

#[derive(Default)]
struct Scratch(u32);

fn list_host(keyed: bool) -> (TestHost, Model<Vec<&'static str>>) {
    let mut app = App::new();
    let items = app.new_model(vec!["a", "b", "c"]);
    let view = app.new_view(List { items, keyed });
    app.set_root(view);
    let mut host = TestHost::new(app, Size::new(200.0, 200.0));
    host.run_until_idle(10);
    (host, items)
}

#[test]
fn keyed_ids_survive_reorder() {
    let (mut host, items) = list_host(true);
    let b = host.app.find("b").unwrap();
    host.app.element_state_mut::<Scratch>(b).0 = 42;
    host.app.focus(b);
    let b_y = host.app.bounds(b).unwrap().y;

    items.update(&mut host.app, |v, cx| {
        v.reverse();
        v.insert(0, "z");
        cx.notify();
    });
    host.run_until_idle(10);
    let b2 = host.app.find("b").unwrap();
    assert_eq!(b, b2, "keyed id is stable across reorder");
    assert_eq!(host.app.element_state::<Scratch>(b).map(|s| s.0), Some(42));
    assert_eq!(host.app.focused(), Some(b));
    assert_ne!(host.app.bounds(b).unwrap().y, b_y, "but it moved");

    // Removing it drops its state and focus.
    items.update(&mut host.app, |v, cx| {
        v.retain(|n| *n != "b");
        cx.notify();
    });
    host.run_until_idle(10);
    assert!(host.app.find("b").is_none());
    assert!(host.app.element_state::<Scratch>(b).is_none());
    assert_eq!(host.app.focused(), None);
}

#[test]
fn removing_the_focused_element_focuses_what_surrounds_it() {
    let mut app = App::new();
    let model = app.new_model(vec!["a", "b"]);
    let list = app.new_view(Panel { items: model });
    app.set_root(list);
    let mut host = TestHost::new(app, Size::new(200.0, 200.0));
    host.run_until_idle(10);
    let (panel, b) = (host.app.find("panel").unwrap(), host.app.find("b").unwrap());
    host.app.focus(b);
    host.run_until_idle(10);

    model.update(&mut host.app, |v, cx| {
        v.retain(|n| *n != "b");
        cx.notify();
    });
    host.run_until_idle(10);
    assert_eq!(host.app.focused(), Some(panel));

    // Removed before a frame saw it focused: there is nothing to go by.
    let a = host.app.find("a").unwrap();
    host.app.focus(a);
    model.update(&mut host.app, |v, cx| {
        v.clear();
        cx.notify();
    });
    host.run_until_idle(10);
    assert_eq!(host.app.focused(), None);
}

/// A focusable list, itself named `panel`.
struct Panel {
    items: Model<Vec<&'static str>>,
}

impl Render for Panel {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let items = self.items.read(cx).clone();
        div().name("panel").focusable().flex_col().children(
            items
                .into_iter()
                .map(|name| div().name(name).key(name).h(20.0).focusable()),
        )
    }
}

#[test]
fn positional_ids_follow_position() {
    let (mut host, items) = list_host(false);
    let first = host.app.find("a").unwrap();
    items.update(&mut host.app, |v, cx| {
        v.reverse();
        cx.notify();
    });
    host.run_until_idle(10);
    // Without keys, the first slot keeps its id and now shows "c".
    assert_eq!(host.app.find("c"), Some(first));
}

#[test]
fn duplicate_keys_are_disambiguated() {
    let mut app = App::new();
    let items = app.new_model(vec!["x", "x", "y"]);
    let view = app.new_view(List { items, keyed: true });
    app.set_root(view);
    let mut host = TestHost::new(app, Size::new(200.0, 200.0));
    host.run_until_idle(10);
    let list = host.app.children_of(host.app.find("window").unwrap())[0];
    let ids = host.app.children_of(list);
    assert_eq!(ids.len(), 3);
    assert_ne!(ids[0], ids[1]);
}

// ---- layout and paint snapshots --------------------------------------

struct Window;

impl Render for Window {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let c = theme.colors();
        div()
            .name("root")
            .w_full()
            .h_full()
            .bg(c.surface[0])
            .child(
                div()
                    .name("sidebar")
                    .w(180.0)
                    .h_full()
                    .flex_col()
                    .p(8.0)
                    .gap(4.0)
                    .material(Material::Frost)
                    .children((0..3).map(|i| {
                        div()
                            .key(format!("row{i}"))
                            .h(theme.metrics().row_height)
                            .px(8.0)
                            .items_center()
                            .rounded(theme.tokens.radii.sm)
                            .child(text(format!("Item {i}")).name(format!("item{i}")))
                    })),
            )
            .child(
                div()
                    .name("content")
                    .grow()
                    .p(16.0)
                    .style(|s| {
                        use livspeil_ui_layout::style::{fr, length};
                        use livspeil_ui_layout::taffy::{Display, GridTemplateComponent};
                        s.display = Display::Grid;
                        s.grid_template_columns = vec![
                            GridTemplateComponent::Single(fr(1.0)),
                            GridTemplateComponent::Single(fr(1.0)),
                        ];
                        s.gap = livspeil_ui_layout::taffy::Size {
                            width: length(12.0),
                            height: length(12.0),
                        };
                    })
                    .children((0..3).map(move |i| {
                        div()
                            .key(format!("card{i}"))
                            .h(80.0)
                            .rounded(theme.tokens.radii.md)
                            .bg(c.surface[2])
                            .border(1.0, Color::WHITE.with_alpha(0.06))
                            .elevation(Elevation::E2)
                    })),
            )
    }
}

#[test]
fn layout_and_scene_snapshots() {
    let mut app = App::new();
    let v = app.new_view(Window);
    app.set_root(v);
    let mut host = TestHost::new(app, Size::new(640.0, 400.0));
    host.run_until_idle(10);
    assert_golden(&golden_dir(), "window-layout", &host.app.dump_tree());
    let scene = host.scene().unwrap().clone();
    assert_golden(&golden_dir(), "window-scene", &host.app.dump_scene(&scene));

    // Resizing relayouts.
    host.app.set_viewport(Size::new(800.0, 400.0));
    let frame = host.step().unwrap();
    assert!(frame.stats.layout);
    assert_eq!(frame.stats.views_rendered, 0);
    let card = host.app.find("card1").unwrap();
    // (800 - 180 sidebar - 2 * 16 padding - 12 gap) / 2
    assert_eq!(host.app.bounds(card).unwrap().width, 288.0);
}

#[test]
fn rerender_with_same_output_does_not_relayout() {
    let mut f = fixture();
    let before = f.host.app.layout_stats();
    f.host.app.notify(f.right.id());
    let frame = f.host.step().unwrap();
    assert_eq!(frame.stats.views_rendered, 1);
    assert!(
        !frame.stats.layout,
        "identical output must not dirty layout"
    );
    assert_eq!(f.host.app.layout_stats().style_writes, before.style_writes);

    // A change in the left label relayouts without measuring the right one.
    let measures = f.host.app.layout_stats().measure_calls;
    f.a.update(&mut f.host.app, |s, cx| {
        *s = "a much longer label".to_owned();
        cx.notify();
    });
    let frame = f.host.step().unwrap();
    assert!(frame.stats.layout);
    let delta = f.host.app.layout_stats().measure_calls - measures;
    // The untouched title and right label are served from Taffy's cache.
    let first_layout = measures;
    assert!(
        delta >= 1 && delta * 2 < first_layout,
        "measured {delta} of {first_layout}"
    );
}

#[test]
fn layout_bounds_reach_the_view_without_events() {
    use livspeil_ui_layout::Rect;

    struct Sized {
        seen: Vec<Option<Rect>>,
    }
    impl Render for Sized {
        fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
            self.seen.push(cx.layout_bounds());
            div().name("sized").w_full().h_full()
        }
    }
    struct Outer(View<Sized>);
    impl Render for Outer {
        fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
            div()
                .w_full()
                .h_full()
                .p(10.0)
                .child(div().size(300.0, 200.0).child(Element::view(&self.0)))
        }
    }
    let mut app = App::new();
    let inner = app.new_view(Sized { seen: Vec::new() });
    let outer = app.new_view(Outer(inner));
    app.set_root(outer);
    let mut h = TestHost::new(app, Size::new(400.0, 300.0));
    h.run_until_idle(10);
    let seen = inner.read(&h.app).seen.clone();
    assert_eq!(seen, vec![None, Some(Rect::new(10.0, 10.0, 300.0, 200.0))]);
    assert!(!h.app.needs_frame());
    // A new window size re-renders it once more; nothing else does.
    h.app.set_viewport(Size::new(500.0, 300.0));
    h.run_until_idle(10);
    assert_eq!(inner.read(&h.app).seen.len(), 2);
}
