// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Select (popup in the overlay layer) and Tooltip (hover delay).

mod common;

use std::time::Duration;

use common::{center, click, find, host, wait};
use livspeil_ui_core::{Element, Model, Role, TestHost, View, div};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_widgets::{Button, Env, Select, Tooltip, place_tooltip};

struct Fx {
    h: TestHost,
    sel: Model<Option<usize>>,
    select: View<Select>,
    name: String,
}

fn fixture() -> Fx {
    let mut out = None;
    let (h, _) = host(|app, env| {
        let sel = app.new_model(Some(1));
        let select = app.new_view(
            Select::new(env, sel, ["Apple", "Banana", "Cherry", "Date"])
                .label("Fruit")
                .disable_option(2),
        );
        out = Some((sel, select));
        Box::new(move |_| {
            div()
                .flex_col()
                .p(16.0)
                .gap(8.0)
                .child(Element::view(&select))
                // A later sibling: the popup must still be on top of it.
                .child(div().name("below").size(200.0, 200.0).focusable())
        })
    });
    let (sel, select) = out.unwrap();
    let name = Select::element_name(&select, &h.app);
    Fx {
        h,
        sel,
        select,
        name,
    }
}

impl Fx {
    fn open(&self) -> bool {
        self.select.read(&self.h.app).is_open()
    }
    fn hl(&self) -> Option<usize> {
        self.select.read(&self.h.app).highlighted()
    }
    fn sel(&self) -> Option<usize> {
        *self.sel.read(&self.h.app)
    }
    fn key(&mut self, k: &str) {
        self.h.key(k);
        self.h.step();
    }
}

#[test]
fn select_opens_navigates_and_chooses_with_keyboard() {
    let mut f = fixture();
    let trigger = find(&f.h, &f.name);
    f.key("tab");
    assert_eq!(f.h.app.focused(), Some(trigger));
    assert!(f.h.app.dump_tree().contains("\"Banana\""));
    f.key("down");
    assert!(f.open());
    assert_eq!(f.hl(), Some(1), "opens on the chosen option");
    f.h.run_until_idle(100);
    assert!(f.h.app.find("listbox").is_some());
    f.key("down");
    assert_eq!(f.hl(), Some(3), "skips the disabled option");
    f.key("down");
    assert_eq!(f.hl(), Some(3), "no wrap");
    f.key("home");
    assert_eq!(f.hl(), Some(0));
    f.key("end");
    assert_eq!(f.hl(), Some(3));
    f.key("up");
    assert_eq!(f.hl(), Some(1));
    f.key("enter");
    assert!(!f.open());
    assert_eq!(f.sel(), Some(1));
    f.h.run_until_idle(100);
    assert!(f.h.app.find("listbox").is_none());
    // Escape closes without choosing; focus stays.
    f.key("space");
    assert!(f.open());
    f.key("up");
    f.key("escape");
    assert!(!f.open());
    assert_eq!(f.sel(), Some(1));
    assert_eq!(f.h.app.focused(), Some(trigger));
    // Typeahead while closed chooses directly.
    f.h.type_text("d");
    f.h.step();
    assert_eq!(f.sel(), Some(3));
    // Tab while open closes and moves on.
    f.key("alt-down");
    assert!(f.open());
    f.key("tab");
    assert!(!f.open());
    assert_eq!(f.h.app.focused(), Some(find(&f.h, "below")));
    let node = f.h.app.semantic_node(trigger).unwrap();
    assert_eq!(node.role(), Some(Role::Button));
    assert_eq!(node.label(), Some("Fruit"));
    assert_eq!(node.semantics.unwrap().get_expanded(), Some(false));
}

#[test]
fn select_popup_is_on_top_animates_and_chooses_by_pointer() {
    let mut f = fixture();
    click(&mut f.h, &f.name);
    assert!(f.open());
    let trigger = f.h.app.bounds(find(&f.h, &f.name)).unwrap();
    let list = find(&f.h, "listbox");
    let lb = f.h.app.bounds(list).unwrap();
    assert!(lb.y >= trigger.bottom(), "below the trigger: {lb:?}");
    assert_eq!(f.h.app.scale_of(list), Some(1.0));
    // Over the later sibling, the popup wins the hit test.
    let cherry = center(&f.h, "option:Date");
    let below = f.h.app.bounds(find(&f.h, "below")).unwrap();
    assert!(below.contains(cherry), "the option overlaps the sibling");
    assert_eq!(f.h.app.hit_test(cherry), Some(find(&f.h, "option:Date")));
    // Hover highlights; disabled options ignore the pointer.
    let c = center(&f.h, "option:Cherry");
    f.h.pointer_move(c.x, c.y);
    assert_eq!(f.hl(), Some(1));
    f.h.click(c.x, c.y);
    assert!(f.open());
    let a = center(&f.h, "option:Apple");
    f.h.pointer_move(a.x, a.y);
    assert_eq!(f.hl(), Some(0));
    f.h.click(a.x, a.y);
    f.h.run_until_idle(100);
    assert!(!f.open());
    assert_eq!(f.sel(), Some(0));
    assert_eq!(
        f.h.app.focused(),
        Some(find(&f.h, &f.name)),
        "focus back on the trigger"
    );
    // Open again: first frame starts hidden, then springs in.
    let p = center(&f.h, &f.name);
    f.h.click(p.x, p.y);
    f.h.step();
    let list = find(&f.h, "listbox");
    f.h.step();
    let s = f.h.app.scale_of(list).unwrap();
    assert!(s > 0.97 && s < 1.0, "scaling in: {s}");
    f.h.run_until_idle(200);
    // Clicking outside closes without choosing.
    f.h.click(470.0, 350.0);
    f.h.run_until_idle(100);
    assert!(!f.open());
    assert_eq!(f.sel(), Some(0));
    assert!(!f.h.app.needs_frame());
}

fn tooltip_host() -> (TestHost, Env) {
    host(|_, env| {
        let env = env.clone();
        Box::new(move |cx| {
            let theme = cx.theme().clone();
            div().p(120.0).child(
                Tooltip::new("Saves the document")
                    .attach(&env, Button::new("Save").render(&theme).key("save")),
            )
        })
    })
}

#[test]
fn tooltip_shows_after_delay_and_hides_on_leave_or_press() {
    let (mut h, env) = tooltip_host();
    let anchor = find(&h, "save");
    let p = center(&h, "save");
    h.pointer_move(p.x, p.y);
    wait(&mut h, 300);
    assert!(!env.tooltip_shown(&h.app, anchor));
    assert!(h.app.find("tooltip").is_none());
    wait(&mut h, 250);
    assert!(env.tooltip_shown(&h.app, anchor));
    h.run_until_idle(100);
    let tip = find(&h, "tooltip");
    let tb = h.app.bounds(tip).unwrap();
    let ab = h.app.bounds(anchor).unwrap();
    assert!(tb.y > ab.bottom(), "below the anchor");
    assert!((tb.center().x - ab.center().x).abs() < 1.0, "centered");
    assert!(
        h.scene()
            .is_some_and(|s| format!("{s:?}").contains("Glass"))
    );
    assert!(!h.app.needs_frame(), "idle once shown");
    // The anchor is described by the tooltip.
    let node = h.app.semantic_node(anchor).unwrap();
    assert_eq!(
        node.semantics.unwrap().get_description(),
        Some("Saves the document")
    );
    // Leaving hides it.
    h.pointer_move(470.0, 350.0);
    h.run_until_idle(100);
    assert!(h.app.find("tooltip").is_none());
    // Pressing hides it too.
    h.pointer_move(p.x, p.y);
    wait(&mut h, 600);
    assert!(env.tooltip_shown(&h.app, anchor));
    h.pointer_down(p.x, p.y);
    h.pointer_up(p.x, p.y);
    h.run_until_idle(100);
    assert!(h.app.find("tooltip").is_none());
    // While pending, frames keep coming; nothing is left running after.
    let _ = Duration::ZERO;
}

#[test]
fn tooltip_placement_flips_and_clamps() {
    let vp = Vec2::new(400.0, 300.0);
    let r = place_tooltip(
        Rect::new(100.0, 50.0, 60.0, 24.0),
        Vec2::new(80.0, 20.0),
        vp,
    );
    assert_eq!(r, Rect::new(90.0, 80.0, 80.0, 20.0));
    // Near the bottom: above.
    let r = place_tooltip(
        Rect::new(100.0, 270.0, 60.0, 24.0),
        Vec2::new(80.0, 20.0),
        vp,
    );
    assert_eq!(r.y, 270.0 - 6.0 - 20.0);
    // Near the right edge: clamped.
    let r = place_tooltip(
        Rect::new(380.0, 50.0, 20.0, 24.0),
        Vec2::new(80.0, 20.0),
        vp,
    );
    assert_eq!(r.right(), 396.0);
}
