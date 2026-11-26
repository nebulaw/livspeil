// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Tabs: selection, keyboard, the sliding pill, close buttons, overflow
//! scrolling and accessibility.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{center, find, golden_dir, host, host_sized, scene, wait};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{Element, Model, Role, TestHost, Toggled, View, div};
use livspeil_ui_layout::{Size, Vec2};
use livspeil_ui_theme::{MotionConfig, Scheme, Theme};
use livspeil_ui_widgets::{TabItem, Tabs};

struct Fx {
    h: TestHost,
    sel: Model<usize>,
    tabs: View<Tabs>,
    closed: Rc<RefCell<Vec<usize>>>,
}

fn fixture(width: f32, titles: &[&str], closable: bool) -> Fx {
    let closed = Rc::new(RefCell::new(Vec::new()));
    let mut out = None;
    let c2 = closed.clone();
    let titles: Vec<String> = titles.iter().map(|s| (*s).to_owned()).collect();
    let (h, _) = host_sized(Size::new(480.0, 200.0), |app, env| {
        let sel = app.new_model(0usize);
        let mut t = Tabs::new(
            env,
            sel,
            titles.iter().enumerate().map(|(i, s)| {
                // The third tab of the standard fixture is disabled.
                TabItem::new(s.clone()).disabled(s == "Disabled" && i > 0)
            }),
        )
        .label("Views")
        .closable(closable)
        .on_close(move |i, _| c2.borrow_mut().push(i));
        t = t.name("tabs");
        let tabs = app.new_view(t);
        out = Some((sel, tabs));
        Box::new(move |_| {
            div()
                .flex_col()
                .p(16.0)
                .child(div().w(width).child(Element::view(&tabs)))
                .child(div().name("below").size(100.0, 40.0).focusable())
        })
    });
    let (sel, tabs) = out.unwrap();
    Fx {
        h,
        sel,
        tabs,
        closed,
    }
}

impl Fx {
    fn sel(&self) -> usize {
        *self.sel.read(&self.h.app)
    }
    fn key(&mut self, k: &str) {
        self.h.key(k);
        self.h.run_until_idle(200);
    }
    fn pill_x(&self) -> f32 {
        self.h
            .app
            .bounds(find(&self.h, "tab-pill"))
            .map_or(f32::NAN, |b| b.x)
    }
}

/// The tab's rectangle on screen (layout bounds plus the strip's scroll
/// translation).
fn on_screen(h: &TestHost, name: &str) -> livspeil_ui_layout::Rect {
    let mut b = h.app.bounds(find(h, name)).unwrap();
    b.x += h.app.translation(find(h, "strip")).unwrap().x;
    b
}

/// A point on the tab's left part (the centre may be a close button).
fn left_of(h: &TestHost, name: &str) -> livspeil_ui_layout::Point {
    let b = on_screen(h, name);
    livspeil_ui_layout::Point::new(b.x + 6.0, b.y + b.height / 2.0)
}

const FOUR: [&str; 4] = ["Scene", "Disabled", "Charms", "Timeline"];

#[test]
fn click_and_keyboard_choose_tabs_and_skip_disabled() {
    let mut f = fixture(400.0, &FOUR, false);
    assert_eq!(f.sel(), 0);
    f.h.key("tab");
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "tabs")));
    f.key("right");
    assert_eq!(f.sel(), 2, "skips the disabled tab");
    f.key("end");
    assert_eq!(f.sel(), 3);
    f.key("right");
    assert_eq!(f.sel(), 3, "no wrap");
    f.key("left");
    assert_eq!(f.sel(), 2);
    f.key("home");
    assert_eq!(f.sel(), 0);
    // Delete does nothing when tabs cannot be closed.
    f.key("delete");
    assert_eq!(f.tabs.read(&f.h.app).items().len(), 4);
    // Clicks.
    let p = left_of(&f.h, "tab:Timeline");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert_eq!(f.sel(), 3);
    let p = left_of(&f.h, "tab:Disabled");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert_eq!(f.sel(), 3, "disabled tab ignores clicks");
}

#[test]
fn the_pill_springs_to_the_selection_and_retargets() {
    let mut f = fixture(400.0, &FOUR, false);
    let x0 = f.pill_x();
    let tab0 = f.h.app.bounds(find(&f.h, "tab:Scene")).unwrap();
    assert!((x0 - tab0.x).abs() < 0.6, "starts under the first tab");
    f.h.key("tab");
    f.h.key("end");
    f.h.step();
    f.h.step();
    let mid = f.pill_x();
    let end = f.h.app.bounds(find(&f.h, "tab:Timeline")).unwrap().x;
    assert!(mid > x0 && mid < end, "in flight: {x0} < {mid} < {end}");
    // Retarget in flight: the pill heads back without a jump.
    f.h.key("home");
    f.h.step();
    let back = f.pill_x();
    assert!(
        back >= mid - 40.0 && back <= end,
        "keeps its velocity: {mid} -> {back}"
    );
    f.h.run_until_idle(300);
    assert!((f.pill_x() - x0).abs() < 0.6, "settles on the first tab");
    assert!(!f.h.app.needs_frame(), "idle after settling");
    // Width follows the tab.
    f.key("end");
    let w = f.h.app.bounds(find(&f.h, "tab-pill")).unwrap().width;
    let tw = f.h.app.bounds(find(&f.h, "tab:Timeline")).unwrap().width;
    assert!((w - tw).abs() < 0.6);
}

#[test]
fn reduce_motion_jumps() {
    let mut f = fixture(400.0, &FOUR, false);
    f.h.app.set_motion(MotionConfig {
        reduce_motion: true,
        ..MotionConfig::default()
    });
    f.h.key("tab");
    f.h.key("end");
    f.h.step();
    f.h.step();
    let end = f.h.app.bounds(find(&f.h, "tab:Timeline")).unwrap().x;
    assert!(
        (f.pill_x() - end).abs() < 0.6,
        "no sliding under Reduce Motion"
    );
}

#[test]
fn close_buttons_remove_tabs_and_pick_the_neighbour() {
    let mut f = fixture(460.0, &["A", "B", "C"], true);
    // Choose B, close it: C takes its place and is chosen.
    let p = left_of(&f.h, "tab:B");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert_eq!(f.sel(), 1);
    let p = center(&f.h, "close:B");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert_eq!(*f.closed.borrow(), [1]);
    assert!(f.h.app.find("tab:B").is_none());
    assert_eq!(f.sel(), 1);
    let node = f.h.app.semantic_node(find(&f.h, "tab:C")).unwrap();
    assert_eq!(node.semantics.unwrap().get_selected(), Some(true));
    // Closing a tab before the selection shifts it.
    let p = center(&f.h, "close:A");
    f.h.click(p.x, p.y);
    f.h.run_until_idle(200);
    assert_eq!(f.sel(), 0);
    assert_eq!(*f.closed.borrow(), [1, 0]);
    // Delete closes the selected tab from the keyboard; the last one goes
    // too and the selection stays valid.
    let bar = find(&f.h, "tabs");
    f.h.app.focus(bar);
    f.key("delete");
    assert_eq!(*f.closed.borrow(), [1, 0, 0]);
    assert!(f.tabs.read(&f.h.app).items().is_empty());
    assert_eq!(f.sel(), 0);
    // Appending chooses the new tab.
    Tabs::push(f.tabs, &mut f.h.app, "New");
    f.h.run_until_idle(200);
    assert!(f.h.app.find("tab:New").is_some());
    assert_eq!(f.sel(), 0);
}

#[test]
fn overflow_scrolls_to_the_selection_and_with_the_wheel() {
    let titles = [
        "Scene", "Charms", "Timeline", "Assets", "Settings", "History",
    ];
    let mut f = fixture(220.0, &titles, false);
    let bar = f.h.app.bounds(find(&f.h, "tabs")).unwrap();
    assert!((bar.width - 220.0).abs() < 0.6, "bar is clamped to its box");
    let last = on_screen(&f.h, "tab:History");
    assert!(last.x > bar.right(), "last tab starts out of view");
    f.h.key("tab");
    f.key("end");
    let last = on_screen(&f.h, "tab:History");
    assert!(
        last.right() <= bar.right() + 0.6 && last.x >= bar.x,
        "scrolled into view: {last:?} in {bar:?}"
    );
    // Wheel back to the start.
    let p = center(&f.h, "tabs");
    for _ in 0..20 {
        f.h.scroll(p.x, p.y, Vec2::new(-20.0, 0.0));
        wait(&mut f.h, 16);
    }
    f.h.run_until_idle(200);
    let first = on_screen(&f.h, "tab:Scene");
    assert!(first.x >= bar.x - 0.6, "scrolled back: {first:?}");
    // Vertical wheels scroll the bar as well.
    for _ in 0..10 {
        f.h.scroll(p.x, p.y, Vec2::new(0.0, 20.0));
        wait(&mut f.h, 16);
    }
    f.h.run_until_idle(200);
    let first = on_screen(&f.h, "tab:Scene");
    assert!(first.x < bar.x, "vertical wheel moves the strip: {first:?}");
}

#[test]
fn semantics_are_a_tab_list_of_tabs() {
    let f = fixture(400.0, &FOUR, true);
    let bar = f.h.app.semantic_node(find(&f.h, "tabs")).unwrap();
    assert_eq!(bar.role(), Some(Role::TabList));
    assert_eq!(bar.label(), Some("Views"));
    let t = f.h.app.semantic_node(find(&f.h, "tab:Scene")).unwrap();
    assert_eq!(t.role(), Some(Role::Tab));
    assert_eq!(t.semantics.as_ref().unwrap().get_selected(), Some(true));
    let d = f.h.app.semantic_node(find(&f.h, "tab:Disabled")).unwrap();
    assert!(d.is_disabled());
    let s = f.tabs.read(&f.h.app).semantics(&f.h.app);
    assert_eq!(
        s.to_string(),
        "TabList label=\"Views\" value=\"Scene\" focusable actions=[Focus]\n  \
         Tab label=\"Scene\" selected=true focusable actions=[Click]\n  \
         Tab label=\"Disabled\" selected=false disabled\n  \
         Tab label=\"Charms\" selected=false focusable actions=[Click]\n  \
         Tab label=\"Timeline\" selected=false focusable actions=[Click]\n"
    );
    let _ = Toggled::On;
}

#[test]
fn tabs_scenes_in_both_themes() {
    for (scheme, name) in [(Scheme::Dark, "tabs-dark"), (Scheme::Light, "tabs-light")] {
        let (mut h, _) = host(|app, env| {
            app.set_theme(Theme {
                scheme,
                ..Theme::default()
            });
            let a = app.new_model(1usize);
            let b = app.new_model(0usize);
            let plain = app.new_view(
                Tabs::new(env, a, ["Scene", "Charms", "Timeline"])
                    .name("plain")
                    .label("Plain"),
            );
            let closable = app.new_view(
                Tabs::new(
                    env,
                    b,
                    [
                        TabItem::new("Living room"),
                        TabItem::new("Kitchen"),
                        TabItem::new("Off").disabled(true),
                    ],
                )
                .closable(true)
                .name("closable")
                .label("Rooms"),
            );
            let c = app.new_model(4usize);
            let overflow = app.new_view(
                Tabs::new(
                    env,
                    c,
                    ["Scene", "Charms", "Timeline", "Assets", "Settings"],
                )
                .name("overflow")
                .label("Overflow"),
            );
            Box::new(move |_| {
                div()
                    .flex_col()
                    .p(16.0)
                    .gap(12.0)
                    .child(Element::view(&plain))
                    .child(Element::view(&closable))
                    .child(div().w(190.0).child(Element::view(&overflow)))
            })
        });
        h.run_until_idle(100);
        assert_golden(&golden_dir(), name, &scene(&h));
    }
}
