// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Widgets on layers: scrolling renders nothing until virtualized content
//! runs out, and pointers, popups and thumbs agree with what is drawn.

mod common;

use common::{find, host_sized, wait};
use livspeil_ui_core::{Element, Model, TestHost, View, div, text};
use livspeil_ui_layout::{Point, Rect, Size, Vec2};
use livspeil_ui_widgets::{ScrollAxis, ScrollView, Select, Slider, VirtualList};

fn mounted(
    size: Size,
    build: impl FnOnce(&mut livspeil_ui_core::App, &livspeil_ui_widgets::Env) -> View<ScrollView>,
) -> (TestHost, View<ScrollView>) {
    let mut out = None;
    let (h, _) = host_sized(Size::new(500.0, 500.0), |app, env| {
        let view = build(app, env);
        out = Some(view);
        Box::new(move |_| {
            div()
                .size(size.width, size.height)
                .child(Element::view(&view))
        })
    });
    (h, out.unwrap())
}

fn visual(h: &TestHost, name: &str) -> Rect {
    h.app.visual_bounds(find(h, name)).unwrap()
}

fn list(h_px: f32) -> (TestHost, View<ScrollView>) {
    mounted(Size::new(300.0, h_px), |app, env| {
        app.new_view(
            VirtualList::build(env, 10_000, 20.0, |_, i| {
                text(format!("Row {i}")).name(format!("text-{i}"))
            })
            .name("list"),
        )
    })
}

#[test]
fn a_virtual_list_renders_only_when_it_runs_out_of_rows() {
    let (mut h, view) = list(400.0);
    let layouts = h.app.layout_stats();
    let renders = |h: &TestHost| h.app.render_count(view.id());
    let start = renders(&h);
    // The margin is a quarter of the 400 px view: five rows.
    for step in 1..=10_u8 {
        ScrollView::jump_to(view, &mut h.app, Vec2::new(0.0, f32::from(step) * 10.0));
        let frame = h.step().unwrap();
        assert_eq!(frame.stats.views_rendered, 0, "step {step}");
        assert!(!frame.stats.layout);
        assert_eq!(visual(&h, "row-7").y, 140.0 - f32::from(step) * 10.0);
    }
    assert_eq!(h.app.layout_stats(), layouts);
    ScrollView::jump_to(view, &mut h.app, Vec2::new(0.0, 101.0));
    let frame = h.step().unwrap();
    assert_eq!(frame.stats.views_rendered, 1);
    assert_eq!(renders(&h), start + 1);
    // New rows are laid out inside the list only.
    assert_eq!(h.app.layout_stats().computes, layouts.computes);
    // Back and forth over the old edge does not render again.
    for offset in [90.0, 110.0, 95.0, 105.0, 80.0, 120.0] {
        ScrollView::jump_to(view, &mut h.app, Vec2::new(0.0, offset));
        h.run_until_idle(10);
    }
    assert_eq!(renders(&h), start + 1);
    // A jump far away shows the right rows in the same frame.
    ScrollView::jump_to(view, &mut h.app, Vec2::new(0.0, 100_000.0));
    h.step().unwrap();
    assert_eq!(visual(&h, "row-5000").y, 0.0);
    assert!(h.app.find("row-0").is_none());
}

#[test]
fn a_view_that_grows_builds_more_rows() {
    let (mut h, _) = host_sized(Size::new(300.0, 200.0), |app, env| {
        let view = app.new_view(VirtualList::build(env, 10_000, 20.0, |_, i| {
            text(format!("Row {i}"))
        }));
        Box::new(move |_| div().w_full().h_full().child(Element::view(&view)))
    });
    let rows = |h: &TestHost| h.app.children_of(find(h, "virtual-list")).len();
    let short = rows(&h);
    h.app.set_viewport(Size::new(300.0, 600.0));
    h.run_until_idle(100);
    assert!(rows(&h) >= short + 18, "{short} then {}", rows(&h));
}

#[test]
fn plain_content_never_renders_for_scrolling_and_thumbs_follow() {
    let (mut h, view) = mounted(Size::new(200.0, 200.0), |app, env| {
        app.new_view(
            ScrollView::new(env, |_, _| div().h(1000.0).name("tall"))
                .name("sv")
                .focusable(true),
        )
    });
    let renders = h.app.render_count(view.id());
    let thumb = |h: &TestHost| visual(h, "vbar");
    // Thumbs are not shown before any scroll.
    assert_eq!(h.app.opacity_of(find(&h, "vbar")), Some(0.0));
    h.scroll(100.0, 100.0, Vec2::new(0.0, -100.0));
    h.step().unwrap();
    let first = thumb(&h);
    h.scroll(100.0, 100.0, Vec2::new(0.0, -200.0));
    h.step().unwrap();
    assert!(thumb(&h).y > first.y);
    assert_eq!(visual(&h, "tall").y, -300.0);
    wait(&mut h, 100);
    assert!(h.app.opacity_of(find(&h, "vbar")).unwrap() > 0.9);
    // Pulled past the end the thumb shrinks, still without rendering.
    ScrollView::jump_to(view, &mut h.app, Vec2::ZERO);
    h.run_until_idle(300);
    h.scroll(100.0, 100.0, Vec2::new(0.0, 60.0));
    h.step().unwrap();
    assert!(visual(&h, "tall").y > 0.0, "rubber band");
    assert!(thumb(&h).height < first.height);
    wait(&mut h, 3000);
    assert!(!h.app.needs_frame(), "at rest");
    assert_eq!(h.app.opacity_of(find(&h, "vbar")), Some(0.0));
    // Dragging the thumb scrolls, five content pixels per pixel here.
    h.pointer_move(100.0, 100.0);
    wait(&mut h, 200);
    let t = thumb(&h);
    let from = t.center();
    h.drag(from, from + Vec2::new(0.0, 20.0), 4);
    h.step();
    let off = view.read(&h.app).offset().y;
    assert!((off - 20.0 * 1000.0 / 196.0).abs() < 1e-2, "{off}");
    assert_eq!(h.app.render_count(view.id()), renders);
}

#[test]
fn pointers_and_popups_see_scrolled_content_where_it_is() {
    let mut models: Option<Model<f32>> = None;
    let (mut h, view) = mounted(Size::new(300.0, 300.0), |app, env| {
        let value = app.new_model(0.0_f32);
        let selected = app.new_model(None);
        models = Some(value);
        let slider = app.new_view(Slider::new(value, 0.0, 100.0, 1.0).label("Level"));
        let select =
            app.new_view(Select::new(env, selected, ["One", "Two", "Three"]).name("select"));
        app.new_view(
            ScrollView::new(env, move |_, _| {
                div()
                    .w(900.0)
                    .h(900.0)
                    .child(
                        div()
                            .absolute(Rect::new(250.0, 100.0, 200.0, 24.0))
                            .name("slider-box")
                            .child(Element::view(&slider)),
                    )
                    .child(
                        div()
                            .absolute(Rect::new(180.0, 200.0, 120.0, 28.0))
                            .child(Element::view(&select)),
                    )
            })
            .axis(ScrollAxis::Both),
        )
    });
    let value = models.unwrap();
    // Measured by the first event.
    h.pointer_move(5.0, 5.0);
    ScrollView::jump_to(view, &mut h.app, Vec2::new(150.0, 50.0));
    h.run_until_idle(100);
    let slider = visual(&h, "slider-box");
    assert_eq!(slider.origin(), Point::new(100.0, 50.0));
    // Three quarters along the track where it is shown.
    h.click(slider.x + 0.75 * slider.width, slider.center().y);
    h.run_until_idle(100);
    let v = *value.read(&h.app);
    assert!((70.0..=80.0).contains(&v), "{v}");

    let select = visual(&h, "select");
    assert_eq!(select.origin(), Point::new(30.0, 150.0));
    h.click(select.center().x, select.center().y);
    h.run_until_idle(300);
    let option = visual(&h, "option:One");
    assert!(
        (option.x - select.x).abs() < 20.0 && (option.y - select.y).abs() < 120.0,
        "{option} by {select}"
    );
}
