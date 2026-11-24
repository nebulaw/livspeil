// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! TextField: editing, selection, clipboard, undo, IME, focus, scrolling,
//! and a property test over random edit/IME sequences.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{center, find, host};
use livspeil_ui_core::{Element, ImeEvent, Role, TestHost, View, div};
use livspeil_ui_layout::Point;
use livspeil_ui_widgets::{Env, TextField};
use proptest::prelude::*;

struct Fx {
    h: TestHost,
    env: Env,
    field: View<TextField>,
    name: String,
    changes: Rc<RefCell<Vec<String>>>,
    submitted: Rc<RefCell<Vec<String>>>,
}

fn fixture(initial: &str, width: f32) -> Fx {
    let changes = Rc::new(RefCell::new(Vec::new()));
    let submitted = Rc::new(RefCell::new(Vec::new()));
    let (c2, s2) = (changes.clone(), submitted.clone());
    let mut out = None;
    let (h, env) = host(|app, env| {
        let field = app.new_view(
            TextField::new(env, initial)
                .placeholder("Name")
                .label("Name")
                .width(width)
                .on_change(move |t, _| c2.borrow_mut().push(t.to_owned()))
                .on_submit(move |t, _| s2.borrow_mut().push(t.to_owned())),
        );
        out = Some(field);
        Box::new(move |_| {
            div()
                .flex_col()
                .p(16.0)
                .gap(8.0)
                .child(Element::view(&field))
                .child(div().name("other").size(40.0, 20.0).focusable())
        })
    });
    let field = out.unwrap();
    let name = TextField::element_name(&field, &h.app);
    Fx {
        h,
        env,
        field,
        name,
        changes,
        submitted,
    }
}

impl Fx {
    fn text(&self) -> String {
        self.field.read(&self.h.app).text().to_owned()
    }
    fn sel(&self) -> (usize, usize) {
        let s = self.field.read(&self.h.app).editor().selection();
        (s.anchor, s.focus)
    }
    fn focus(&mut self) {
        let c = center(&self.h, &self.name);
        self.h.click(c.x, c.y);
        self.h.run_until_idle(50);
    }
    fn key(&mut self, k: &str) {
        self.h.key(k);
        self.h.step();
    }
    fn has(&self, name: &str) -> bool {
        self.h.app.find(name).is_some()
    }
}

#[test]
fn typing_moves_deletes_undo_and_submit() {
    let mut f = fixture("", 200.0);
    assert!(f.has("placeholder"));
    f.focus();
    assert!(f.has("caret"), "caret when focused");
    f.h.type_text("hello");
    f.h.type_text(" ");
    f.h.type_text("world");
    f.h.step();
    assert_eq!(f.text(), "hello world");
    assert!(!f.has("placeholder"));
    // Space is text, not activation.
    f.key("space");
    assert_eq!(f.text(), "hello world");
    f.key("ctrl-left");
    assert_eq!(f.sel(), (6, 6));
    f.key("ctrl-shift-right");
    assert_eq!(f.sel(), (6, 11));
    assert!(f.has("selection-0"));
    assert!(!f.has("caret"), "no caret over a selection");
    f.h.type_text("there");
    f.h.step();
    assert_eq!(f.text(), "hello there");
    f.key("home");
    f.key("right");
    f.key("shift-right");
    f.key("delete");
    assert_eq!(f.text(), "hllo there");
    f.key("end");
    f.key("ctrl-backspace");
    assert_eq!(f.text(), "hllo ");
    f.key("backspace");
    assert_eq!(f.text(), "hllo");
    // Undo restores step by step, redo replays.
    f.key("ctrl-z");
    assert_eq!(f.text(), "hllo ");
    f.key("ctrl-z");
    assert_eq!(f.text(), "hllo there");
    f.key("ctrl-z");
    assert_eq!(f.text(), "hello there");
    f.key("ctrl-shift-z");
    assert_eq!(f.text(), "hllo there");
    f.key("ctrl-y");
    assert_eq!(f.text(), "hllo ");
    f.key("enter");
    assert_eq!(f.submitted.borrow().as_slice(), ["hllo "]);
    assert_eq!(f.changes.borrow().last().map(String::as_str), Some("hllo "));
    // Line breaks in input become spaces.
    f.h.type_text("a\nb");
    assert_eq!(f.text(), "hllo a b");
}

#[test]
fn clipboard_select_all_cut_paste() {
    let mut f = fixture("copy me", 200.0);
    f.focus();
    f.key("ctrl-a");
    assert_eq!(f.sel(), (0, 7));
    f.key("ctrl-c");
    assert_eq!(f.env.clipboard_text().as_deref(), Some("copy me"));
    f.key("end");
    f.key("ctrl-v");
    assert_eq!(f.text(), "copy mecopy me");
    f.key("ctrl-a");
    f.key("cmd-x");
    assert_eq!(f.text(), "");
    assert_eq!(f.env.clipboard_text().as_deref(), Some("copy mecopy me"));
    f.env.set_clipboard_text("multi\nline");
    f.key("cmd-v");
    assert_eq!(f.text(), "multi line");
    f.key("ctrl-z");
    assert_eq!(f.text(), "");
}

#[test]
fn ime_preedit_is_shown_underlined_and_commits_as_one_step() {
    let mut f = fixture("ab", 200.0);
    f.focus();
    f.key("left");
    f.h.ime(ImeEvent::Enabled);
    f.h.ime(ImeEvent::Preedit {
        text: "にほ".to_owned(),
        cursor: Some((3, 3)),
    });
    f.h.run_until_idle(50);
    let ed = f.field.read(&f.h.app).editor().clone();
    assert_eq!(ed.text(), "ab", "preedit is not committed text");
    assert_eq!(ed.display_text(), "aにほb");
    assert!(f.has("preedit-0"), "thin underline under the preedit");
    assert!(
        !f.has("preedit-target-0"),
        "collapsed IME cursor: no thick underline"
    );
    assert!(f.has("caret"));
    assert!(f.h.app.dump_tree().contains("\"aにほb\""));
    // A target range gets the thick underline.
    f.h.ime(ImeEvent::Preedit {
        text: "日本".to_owned(),
        cursor: Some((0, 6)),
    });
    f.h.run_until_idle(50);
    assert!(f.has("preedit-target-0"));
    // Moves are ignored while composing.
    f.key("home");
    assert_eq!(f.sel(), (1, 1));
    f.h.ime(ImeEvent::Commit("日本".to_owned()));
    f.h.ime(ImeEvent::Disabled);
    f.h.run_until_idle(50);
    assert_eq!(f.text(), "a日本b");
    assert!(!f.has("preedit-0"));
    assert_eq!(f.sel(), (7, 7));
    f.key("ctrl-z");
    assert_eq!(f.text(), "ab", "commit is one undo step");
    // Escape cancels a composition.
    f.h.ime(ImeEvent::Preedit {
        text: "x".to_owned(),
        cursor: None,
    });
    f.h.step();
    f.key("escape");
    assert!(!f.field.read(&f.h.app).editor().is_composing());
    assert_eq!(f.text(), "ab");
}

#[test]
fn mouse_selection_by_drag_double_and_shift_click() {
    let mut f = fixture("alpha beta gamma", 300.0);
    f.focus();
    let text = f.h.app.bounds(find(&f.h, "text")).unwrap();
    let y = text.center().y;
    let x_at = |frac: f32| text.x + text.width * frac;
    // Drag from the start to past the end selects everything.
    f.h.drag(
        Point::new(text.x + 0.5, y),
        Point::new(text.right() + 20.0, y),
        6,
    );
    f.h.run_until_idle(50);
    assert_eq!(f.sel(), (0, 16));
    // Drag backwards from the end to the middle.
    f.h.step();
    f.h.drag(
        Point::new(text.right() - 0.5, y),
        Point::new(x_at(0.5), y),
        4,
    );
    let (a, b) = f.sel();
    assert_eq!(a, 16);
    assert!(b > 4 && b < 12, "focus in the middle: {b}");
    // Click places the caret; shift-click extends.
    f.h.advance(std::time::Duration::from_secs(1));
    f.h.click(text.x + 0.5, y);
    assert_eq!(f.sel(), (0, 0));
    f.h.advance(std::time::Duration::from_secs(1));
    let p = Point::new(text.right() - 0.5, y);
    common::down(&mut f.h, p, livspeil_ui_core::Modifiers::SHIFT);
    common::up(&mut f.h, p, livspeil_ui_core::Modifiers::SHIFT);
    assert_eq!(f.sel(), (0, 16));
    // Double-click selects the word, triple-click all.
    f.h.advance(std::time::Duration::from_secs(1));
    let mid = Point::new(x_at(8.0 / 16.0), y);
    f.h.click(mid.x, mid.y);
    f.h.click(mid.x, mid.y);
    let s = f.field.read(&f.h.app).editor().clone();
    assert_eq!(s.selected_text(), "beta");
    f.h.click(mid.x, mid.y);
    assert_eq!(f.sel(), (0, 16));
}

#[test]
fn focus_caret_scroll_disabled_and_semantics() {
    let mut f = fixture("", 80.0);
    assert!(!f.has("caret"));
    // Tab in (from the window) shows the caret after the focus sync.
    f.h.app.focus(find(&f.h, "other"));
    f.key("shift-tab");
    f.h.run_until_idle(50);
    assert_eq!(f.h.app.focused(), Some(find(&f.h, &f.name)));
    assert!(f.has("caret"));
    // Long text scrolls to keep the caret visible.
    f.h.type_text("a fairly long line of text that overflows");
    f.h.run_until_idle(50);
    let field = f.h.app.bounds(find(&f.h, &f.name)).unwrap();
    let caret = f.h.app.bounds(find(&f.h, "caret")).unwrap();
    assert!(f.field.read(&f.h.app).scroll() > 0.0);
    assert!(
        caret.right() <= field.right() && caret.x >= field.x,
        "{caret:?} in {field:?}"
    );
    f.key("home");
    f.h.run_until_idle(50);
    assert_eq!(f.field.read(&f.h.app).scroll(), 0.0);
    // Clicking elsewhere hides the caret.
    let o = center(&f.h, "other");
    f.h.click(o.x, o.y);
    f.h.run_until_idle(50);
    assert!(!f.has("caret"));

    let id = find(&f.h, &f.name);
    let node = f.h.app.semantic_node(id).unwrap();
    assert_eq!(node.role(), Some(Role::TextInput));
    assert_eq!(node.label(), Some("Name"));
    f.h.app
        .perform_semantic_action(
            id,
            livspeil_ui_core::SemanticRequest::SetValue(livspeil_ui_core::ValueRequest::Text(
                "set".to_owned(),
            )),
        )
        .unwrap();
    assert_eq!(f.text(), "set");

    // Disabled: not focusable, ignores typing.
    f.field.update(&mut f.h.app, |tf, cx| {
        *tf = std::mem::replace(tf, TextField::new(&f.env, "x")).disabled(true);
        cx.notify();
    });
    f.h.run_until_idle(50);
    let c = center(&f.h, &f.name);
    f.h.click(c.x, c.y);
    f.h.type_text("zzz");
    assert_eq!(f.text(), "set");
}

#[derive(Clone, Debug)]
enum Op {
    Type(String),
    Key(&'static str),
    Preedit(String, Option<(usize, usize)>),
    Commit(String),
    CancelIme,
    Click(f32),
}

fn op() -> impl Strategy<Value = Op> {
    let keys = [
        "left",
        "right",
        "shift-left",
        "shift-right",
        "ctrl-left",
        "ctrl-shift-right",
        "home",
        "end",
        "backspace",
        "delete",
        "ctrl-backspace",
        "ctrl-a",
        "ctrl-c",
        "ctrl-x",
        "ctrl-v",
        "ctrl-z",
        "ctrl-shift-z",
        "space",
        "escape",
    ];
    let text = || proptest::string::string_regex("[a-z é日ა\u{301}👍]{0,4}").unwrap();
    prop_oneof![
        3 => text().prop_map(Op::Type),
        4 => proptest::sample::select(keys.to_vec()).prop_map(Op::Key),
        2 => (text(), proptest::option::of((0usize..12, 0usize..12)))
            .prop_map(|(t, c)| Op::Preedit(t, c)),
        1 => text().prop_map(Op::Commit),
        1 => Just(Op::CancelIme),
        1 => (0.0f32..1.2).prop_map(Op::Click),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// Random edits and IME sequences keep the editor consistent: valid
    /// UTF-8 boundaries, selection in range, the display text is the text
    /// plus the preedit at the caret, the caret stays inside the field and
    /// undoing everything restores the start.
    #[test]
    fn random_edit_and_ime_sequences(ops in proptest::collection::vec(op(), 1..24)) {
        let mut f = fixture("seed", 120.0);
        f.focus();
        for op in &ops {
            match op {
                Op::Type(t) => f.h.type_text(t),
                Op::Key(k) => f.h.key(k),
                Op::Preedit(t, c) => f.h.ime(ImeEvent::Preedit { text: t.clone(), cursor: *c }),
                Op::Commit(t) => f.h.ime(ImeEvent::Commit(t.clone())),
                Op::CancelIme => f.h.ime(ImeEvent::Disabled),
                Op::Click(frac) => {
                    let b = f.h.app.bounds(find(&f.h, &f.name)).unwrap();
                    f.h.advance(std::time::Duration::from_millis(500));
                    f.h.click(b.x + b.width * frac, b.center().y);
                }
            }
            f.h.step();
            let ed = f.field.read(&f.h.app).editor().clone();
            let t = ed.text();
            let s = ed.selection();
            prop_assert!(s.anchor <= t.len() && s.focus <= t.len());
            prop_assert!(t.is_char_boundary(s.anchor) && t.is_char_boundary(s.focus));
            prop_assert!(!t.contains('\n'));
            let display = ed.display_text().into_owned();
            match ed.preedit() {
                None => prop_assert_eq!(&display, t),
                Some(p) => {
                    let mut want = t.to_owned();
                    want.insert_str(s.focus, &p.text);
                    prop_assert_eq!(&display, &want);
                }
            }
            if let Some(c) = f.h.app.find("caret") {
                let field = f.h.app.bounds(find(&f.h, &f.name)).unwrap();
                let caret = f.h.app.bounds(c).unwrap();
                prop_assert!(caret.x >= field.x - 0.5 && caret.right() <= field.right() + 0.5,
                    "caret {:?} outside {:?}", caret, field);
            }
        }
        f.h.ime(ImeEvent::Disabled);
        f.h.advance(std::time::Duration::from_millis(500));
        f.focus();
        for _ in 0..200 {
            f.h.key("ctrl-z");
        }
        f.h.step();
        prop_assert_eq!(f.text(), "seed");
    }
}
