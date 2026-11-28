// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Picker: narrowing by typing, keys, pointer and focus.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{click, find, golden_dir, host_sized, scene};
use livspeil_testkit::assert_golden;
use livspeil_ui_core::{TestHost, div};
use livspeil_ui_layout::Size;
use livspeil_ui_widgets::{Picker, PickerItem, searching};

struct Fx {
    h: TestHost,
    picker: Picker,
    picked: Rc<RefCell<Vec<usize>>>,
}

/// A window with a focusable `below` and a picker of 20 numbered items.
fn fixture() -> Fx {
    let picked = Rc::new(RefCell::new(Vec::new()));
    let log = picked.clone();
    let mut out = None;
    let (mut h, env) = host_sized(Size::new(640.0, 520.0), |app, env| {
        let items = (0..20)
            .map(|i| PickerItem::new(format!("item{i}"), format!("number {i}")))
            .collect::<Vec<_>>();
        out = Some(Picker::new(
            app,
            env,
            "pick",
            "Choose",
            searching(items),
            move |item, _| {
                let n = item.label.trim_start_matches("item").parse().unwrap();
                log.borrow_mut().push(n);
            },
        ));
        Box::new(|_| {
            div()
                .w_full()
                .h_full()
                .child(div().name("below").size(40.0, 40.0).focusable())
        })
    });
    let below = find(&h, "below");
    env.focus(&mut h.app, below);
    h.run_until_idle(100);
    Fx {
        h,
        picker: out.unwrap(),
        picked,
    }
}

impl Fx {
    fn open(&mut self) {
        self.picker.open(&mut self.h.app);
        self.h.run_until_idle(300);
    }

    fn key(&mut self, key: &str) {
        self.h.key(key);
        self.h.run_until_idle(300);
    }

    fn type_text(&mut self, text: &str) {
        self.h.type_text(text);
        self.h.run_until_idle(300);
    }

    fn shows(&self, name: &str) -> bool {
        self.h.app.find(name).is_some()
    }

    fn open_now(&self) -> bool {
        self.picker.is_open(&self.h.app)
    }

    fn focused(&self) -> Option<std::rc::Rc<str>> {
        self.h
            .app
            .focused()
            .and_then(|f| self.h.app.label(f))
            .map(|l| l.to_string().into())
    }
}

#[test]
fn typing_narrows_and_enter_chooses_the_highlighted_item() {
    let mut f = fixture();
    f.open();
    assert_eq!(f.focused().as_deref(), Some("pick-query"));
    assert!(f.shows("pick:item0"));
    f.type_text("item1");
    // item1 itself, then item10..item19.
    assert!(f.shows("pick:item19") && !f.shows("pick:item2"));
    f.key("down");
    f.key("down");
    f.key("up");
    f.key("enter");
    assert_eq!(*f.picked.borrow(), [10]);
    assert!(!f.open_now());
    assert_eq!(f.focused().as_deref(), Some("below"));

    // Opening again starts over.
    f.open();
    assert!(f.shows("pick:item0"));
    f.key("up");
    f.key("enter");
    assert_eq!(*f.picked.borrow(), [10, 0]);
}

#[test]
fn escape_and_an_empty_result_choose_nothing() {
    let mut f = fixture();
    f.open();
    f.type_text("zzz");
    assert!(f.shows("pick-none"));
    f.key("enter");
    assert!(f.open_now(), "there is nothing to choose");
    f.key("down");
    f.key("escape");
    assert!(!f.open_now());
    assert!(f.picked.borrow().is_empty());
    assert_eq!(f.focused().as_deref(), Some("below"));
}

#[test]
fn the_list_follows_the_highlight_and_takes_clicks() {
    let mut f = fixture();
    f.open();
    let top = |f: &Fx, name: &str| f.h.app.visual_bounds(find(&f.h, name)).unwrap().y;
    let list_top = top(&f, "pick-list");
    f.key("pagedown");
    f.key("pagedown");
    f.key("down");
    // item19 is highlighted and inside the list.
    let y = top(&f, "pick:item19");
    assert!(y >= list_top && y < list_top + 360.0, "{y} {list_top}");
    f.key("pageup");
    f.key("pageup");
    f.key("pageup");
    assert!((top(&f, "pick:item0") - list_top).abs() < 1.0);
    click(&mut f.h, "pick:item3");
    f.h.run_until_idle(300);
    assert_eq!(*f.picked.borrow(), [3]);
    assert!(!f.open_now());
}

#[test]
fn it_looks_like_this() {
    let mut f = fixture();
    f.open();
    f.type_text("item1");
    f.key("down");
    assert_golden(&golden_dir(), "picker", &scene(&f.h));
}

#[test]
fn the_source_is_asked_at_every_query_and_every_opening() {
    let asked = Rc::new(RefCell::new(Vec::new()));
    let picked = Rc::new(RefCell::new(Vec::new()));
    let (queries, chosen) = (asked.clone(), picked.clone());
    let mut out = None;
    let (mut h, _) = host_sized(Size::new(640.0, 520.0), |app, env| {
        // Offers back what was typed.
        let source = move |query: &str| {
            queries.borrow_mut().push(query.to_owned());
            if query.is_empty() {
                Vec::new()
            } else {
                vec![PickerItem::new(query.to_owned(), "as typed")]
            }
        };
        let picker = Picker::new(app, env, "echo", "Echo", source, move |item, _| {
            chosen.borrow_mut().push(item.label.to_string());
        });
        picker.set_hints(app, "Say something", "Nothing said");
        out = Some(picker);
        Box::new(|_| div().w_full().h_full())
    });
    let picker = out.unwrap();
    picker.open(&mut h.app);
    h.run_until_idle(300);
    assert!(scene(&h).contains("Nothing said"));
    assert!(scene(&h).contains("Say something"));
    h.type_text("a/b");
    h.run_until_idle(300);
    h.key("enter");
    h.run_until_idle(300);
    assert_eq!(*picked.borrow(), ["a/b"]);
    picker.open(&mut h.app);
    h.run_until_idle(300);
    assert_eq!(*asked.borrow(), ["", "a/b", ""]);
}
