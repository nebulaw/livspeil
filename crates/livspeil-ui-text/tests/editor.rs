// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Editor model: property tests over random edit sequences, IME flows and
//! layout-driven (visual) editing in Georgian and Arabic.

use livspeil_ui_text::{
    Direction, EditKind, Editor, FontSystem, LayoutOptions, Selection, TextLayout, TextStyle,
};
use proptest::prelude::*;

/// Pieces that stress grapheme handling: combining marks, ZWJ emoji,
/// regional indicators, Georgian, Arabic, CRLF.
const PIECES: &[&str] = &[
    "a",
    "Z",
    " ",
    "\u{301}",
    "e\u{301}",
    "👨\u{200d}👩\u{200d}👧",
    "🇬🇪",
    "\u{1f1ec}",
    "ა",
    "თბილისი",
    "مرحبا",
    "\u{64e}",
    "\r",
    "\n",
    "\r\n",
    "word ",
    "日本",
];

#[derive(Clone, Debug)]
enum Action {
    Insert(usize),
    Delete(bool),
    DeleteWord(bool),
    MoveGrapheme(bool, bool),
    MoveWord(bool, bool),
    Edge(bool, bool),
    Select(usize, usize),
    SelectAll,
    Preedit(usize, Option<(usize, usize)>),
    Commit(usize),
    Cancel,
    Undo,
    Redo,
    Break,
}

fn action() -> impl Strategy<Value = Action> {
    let piece = 0..PIECES.len();
    prop_oneof![
        4 => piece.clone().prop_map(Action::Insert),
        2 => any::<bool>().prop_map(Action::Delete),
        1 => any::<bool>().prop_map(Action::DeleteWord),
        2 => (any::<bool>(), any::<bool>()).prop_map(|(f, e)| Action::MoveGrapheme(f, e)),
        1 => (any::<bool>(), any::<bool>()).prop_map(|(f, e)| Action::MoveWord(f, e)),
        1 => (any::<bool>(), any::<bool>()).prop_map(|(f, e)| Action::Edge(f, e)),
        1 => (0usize..80, 0usize..80).prop_map(|(a, b)| Action::Select(a, b)),
        1 => Just(Action::SelectAll),
        1 => (piece.clone(), proptest::option::of((0usize..12, 0usize..12)))
            .prop_map(|(p, c)| Action::Preedit(p, c)),
        1 => piece.prop_map(Action::Commit),
        1 => Just(Action::Cancel),
        2 => Just(Action::Undo),
        1 => Just(Action::Redo),
        1 => Just(Action::Break),
    ]
}

fn dir(forward: bool) -> Direction {
    if forward {
        Direction::Forward
    } else {
        Direction::Backward
    }
}

fn apply(e: &mut Editor, a: &Action) {
    match a {
        Action::Insert(p) => e.insert(PIECES[*p]),
        Action::Delete(f) => e.delete(dir(*f)),
        Action::DeleteWord(f) => e.delete_word(dir(*f)),
        Action::MoveGrapheme(f, x) => e.move_grapheme(dir(*f), *x),
        Action::MoveWord(f, x) => e.move_word(dir(*f), *x),
        Action::Edge(f, x) => e.move_to_edge(dir(*f), *x),
        Action::Select(a, b) => e.set_selection(*a, *b),
        Action::SelectAll => e.select_all(),
        Action::Preedit(p, c) => e.set_preedit(PIECES[*p], c.map(|(s, t)| s..t)),
        Action::Commit(p) => e.commit(PIECES[*p]),
        Action::Cancel => e.cancel_preedit(),
        Action::Undo => {
            e.undo();
        }
        Action::Redo => {
            e.redo();
        }
        Action::Break => e.break_undo_group(),
    }
}

fn check_invariants(e: &Editor) {
    let sel = e.selection();
    assert!(
        e.is_boundary(sel.anchor),
        "anchor {sel:?} in {:?}",
        e.text()
    );
    assert!(e.is_boundary(sel.focus), "focus {sel:?} in {:?}", e.text());
    let display = e.display_text();
    assert!(std::str::from_utf8(display.as_bytes()).is_ok());
    if let Some(r) = e.preedit_range() {
        assert!(display.is_char_boundary(r.start) && display.is_char_boundary(r.end));
        assert!(r.end <= display.len());
    }
    if let Some(c) = e.display_caret() {
        assert!(display.is_char_boundary(c));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn random_edits_keep_invariants_and_undo_restores(
        initial in 0usize..PIECES.len(),
        actions in proptest::collection::vec(action(), 1..60),
    ) {
        let start = PIECES[initial].repeat(2);
        let mut e = Editor::new(&start);
        check_invariants(&e);
        for a in &actions {
            apply(&mut e, a);
            check_invariants(&e);
        }

        // The op stream replays the committed text.
        let mut replay = start.clone();
        for op in e.take_ops() {
            prop_assert_eq!(&replay[op.range.clone()], op.deleted.as_str());
            replay.replace_range(op.range.clone(), &op.inserted);
        }
        prop_assert_eq!(&replay, e.text());

        // Redo whatever the sequence left undone, then: undo everything
        // gives the initial text, redo everything gives that text again.
        while e.redo() {
            check_invariants(&e);
        }
        let end = e.text().to_owned();
        while e.undo() {
            check_invariants(&e);
        }
        prop_assert_eq!(e.text(), start.as_str());
        while e.redo() {
            check_invariants(&e);
        }
        prop_assert_eq!(e.text(), end.as_str());
    }
}

#[test]
fn ime_compose_and_commit() {
    // Japanese-style composition: preedit updates, then commit.
    let mut e = Editor::new("ab");
    e.set_selection(1, 1);
    e.set_preedit("に", Some(3..3));
    assert_eq!(e.display_text(), "aにb");
    assert_eq!(e.text(), "ab", "preedit is not committed");
    assert_eq!(e.preedit_range(), Some(1..4));
    assert_eq!(e.display_caret(), Some(4));
    e.set_preedit("にほ", Some(0..6));
    assert_eq!(e.preedit_target(), Some(1..7));
    // Movement is owned by the IME while composing.
    e.move_grapheme(Direction::Forward, false);
    assert_eq!(e.selection(), Selection::caret(1));
    e.commit("日本");
    assert!(!e.is_composing());
    assert_eq!(e.text(), "a日本b");
    assert_eq!(e.selection(), Selection::caret(1 + "日本".len()));
    let ops = e.take_ops();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].kind, EditKind::ImeCommit);
    assert!(e.undo());
    assert_eq!(e.text(), "ab");
}

#[test]
fn ime_cancel_and_empty_preedit() {
    let mut e = Editor::new("abc");
    e.set_preedit("ㅎ", None);
    assert_eq!(e.display_caret(), None, "IME hid the caret");
    e.cancel_preedit();
    assert_eq!(e.display_text(), "abc");
    e.set_preedit("ㅎ", Some(0..3));
    e.set_preedit("", None); // winit sends an empty preedit when done
    assert!(!e.is_composing());
    assert!(e.take_ops().is_empty());
}

#[test]
fn ime_over_selection_deletes_it_first() {
    let mut e = Editor::new("hello world");
    e.set_selection(6, 11);
    e.set_preedit("w", Some(1..1));
    assert_eq!(e.text(), "hello ");
    assert_eq!(e.display_text(), "hello w");
    e.commit("wörld");
    assert_eq!(e.text(), "hello wörld");
    assert!(e.undo());
    assert_eq!(e.text(), "hello ");
    assert!(e.undo());
    assert_eq!(e.text(), "hello world");
    assert_eq!(
        e.selection(),
        Selection {
            anchor: 6,
            focus: 11
        }
    );
}

#[test]
fn ime_invalid_cursor_is_clamped() {
    let mut e = Editor::new("");
    e.set_preedit("ა", Some(1..2)); // inside a UTF-8 sequence
    assert_eq!(e.preedit_target(), Some(3..3));
    e.set_preedit("ა", Some(10..20));
    assert_eq!(e.preedit_target(), Some(3..3));
}

#[test]
fn undo_restores_selection() {
    let mut e = Editor::new("გამარჯობა");
    e.set_selection(0, "გამა".len());
    e.delete(Direction::Forward);
    assert_eq!(e.text(), "რჯობა");
    e.undo();
    assert_eq!(e.text(), "გამარჯობა");
    assert_eq!(
        e.selection(),
        Selection {
            anchor: 0,
            focus: "გამა".len()
        }
    );
}

#[test]
fn georgian_editing_with_layout() {
    let mut fs = FontSystem::test();
    let mut e = Editor::new("თბილისი ქალაქი");
    let style = TextStyle::sized(13.0);
    let l = TextLayout::new(&mut fs, &e.display_text(), &style, LayoutOptions::default());
    e.move_to_edge(Direction::Backward, false);
    e.move_visual(&l, true, true, false);
    assert_eq!(e.selection().focus, "თბილისი".len());
    e.move_visual(&l, true, false, true);
    assert_eq!(e.selected_text(), " ");
    e.move_to_line_edge(&l, true, false);
    assert_eq!(e.selection().focus, e.text().len());
    // Click in the middle of the second word.
    let r = l.cursor_rect(livspeil_ui_text::TextCursor::new("თბილისი ქა".len()), 1.0);
    e.click(&l, r.x + 0.5, r.y + 4.0, false);
    assert_eq!(e.selection().focus, "თბილისი ქა".len());
}

#[test]
fn rtl_visual_moves_are_mirrored() {
    let mut fs = FontSystem::test();
    let text = "مرحبا بالعالم";
    let mut e = Editor::new(text);
    let l = TextLayout::new(
        &mut fs,
        text,
        &TextStyle::sized(13.0),
        LayoutOptions::default(),
    );
    assert!(l.is_rtl());
    e.move_to_edge(Direction::Backward, false); // logical start = right edge
    let x0 = l.cursor_rect(livspeil_ui_text::TextCursor::new(0), 1.0).x;
    // In RTL text, moving visually left advances logically.
    e.move_visual(&l, false, false, false);
    let i = e.selection().focus;
    assert!(i > 0, "left moved forward in the text");
    let x1 = l.cursor_rect(livspeil_ui_text::TextCursor::new(i), 1.0).x;
    assert!(x1 < x0, "and the caret moved left on screen ({x0} -> {x1})");
    e.move_visual(&l, true, false, false);
    assert_eq!(e.selection().focus, 0);
}

#[test]
fn multiline_up_down() {
    let mut fs = FontSystem::test();
    let text = "first line\nsecond line\nthird";
    let mut e = Editor::new(text);
    let l = TextLayout::new(
        &mut fs,
        text,
        &TextStyle::sized(13.0),
        LayoutOptions::default(),
    );
    e.set_selection(3, 3);
    e.move_lines(&l, 1, false);
    let line = l
        .lines()
        .iter()
        .position(|li| li.text_range.contains(&e.selection().focus));
    assert_eq!(line, Some(1));
    e.move_lines(&l, 1, true);
    assert!(e.selected_text().contains('\n'));
}
