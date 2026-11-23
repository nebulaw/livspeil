// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Text editing model: committed text, selection, IME preedit, undo/redo
//! and an edit-op stream.
//!
//! Offsets are UTF-8 byte offsets into the committed text and always lie
//! on extended grapheme cluster boundaries. Logical movement (graphemes,
//! words, text edges) needs no layout. Visual movement (bidi-aware
//! left/right, up/down, line edges, clicks) takes the [`TextLayout`] built
//! from [`Editor::display_text`].
//!
//! IME: while a preedit (composition) string is active it is shown at the
//! caret but not part of the committed text; `commit` inserts the final
//! string as one undoable edit and `cancel_preedit` drops it. Starting a
//! composition over a selection deletes the selection first.

use std::borrow::Cow;
use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

use crate::layout::{TextCursor, TextLayout};

/// Logical direction in the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    Backward,
    Forward,
}

/// A selection; collapsed (`anchor == focus`) is a caret. `focus` is the
/// end that moves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Selection {
    pub anchor: usize,
    pub focus: usize,
}

impl Selection {
    #[must_use]
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    #[must_use]
    pub fn is_collapsed(&self) -> bool {
        self.anchor == self.focus
    }

    /// The selected byte range, ordered.
    #[must_use]
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }
}

/// An active IME composition.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Preedit {
    pub text: String,
    /// Cursor or highlighted target range inside `text` (bytes), as
    /// reported by the input method; `None` hides the caret.
    pub cursor: Option<Range<usize>>,
}

/// Why an edit happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditKind {
    Insert,
    Delete,
    ImeCommit,
    Undo,
    Redo,
}

/// One text replacement: `range` of the text before the edit was replaced
/// by `inserted` (and contained `deleted`). Applying the op stream in
/// order to the initial text reproduces the current text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditOp {
    pub kind: EditKind,
    pub range: Range<usize>,
    pub deleted: String,
    pub inserted: String,
    pub selection_before: Selection,
    pub selection_after: Selection,
}

impl EditOp {
    fn inverse(&self, kind: EditKind) -> EditOp {
        EditOp {
            kind,
            range: self.range.start..self.range.start + self.inserted.len(),
            deleted: self.inserted.clone(),
            inserted: self.deleted.clone(),
            selection_before: self.selection_after,
            selection_after: self.selection_before,
        }
    }
}

/// The editing model behind a text field.
#[derive(Clone, Debug, Default)]
pub struct Editor {
    text: String,
    selection: Selection,
    preedit: Option<Preedit>,
    undo: Vec<Vec<EditOp>>,
    redo: Vec<Vec<EditOp>>,
    ops: Vec<EditOp>,
    /// Whether the next typed insert may join the last undo group.
    coalesce: bool,
}

fn next_grapheme(text: &str, at: usize) -> usize {
    let mut c = GraphemeCursor::new(at, text.len(), true);
    match c.next_boundary(text, 0) {
        Ok(Some(i)) => i,
        _ => text.len(),
    }
}

fn prev_grapheme(text: &str, at: usize) -> usize {
    let mut c = GraphemeCursor::new(at, text.len(), true);
    match c.prev_boundary(text, 0) {
        Ok(Some(i)) => i,
        _ => 0,
    }
}

fn is_grapheme_boundary(text: &str, at: usize) -> bool {
    if at > text.len() || !text.is_char_boundary(at) {
        return false;
    }
    let mut c = GraphemeCursor::new(at, text.len(), true);
    c.is_boundary(text, 0).unwrap_or(true)
}

/// Clamp into the text and move back onto a grapheme boundary.
fn snap(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    if is_grapheme_boundary(text, at) {
        at
    } else {
        prev_grapheme(text, at)
    }
}

/// Move forward onto a grapheme boundary (used after inserts, where the
/// caret must stay after the inserted text).
fn snap_forward(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at += 1;
    }
    if is_grapheme_boundary(text, at) {
        at
    } else {
        next_grapheme(text, at)
    }
}

fn is_word(segment: &str) -> bool {
    segment.chars().any(char::is_alphanumeric)
}

fn next_word_end(text: &str, at: usize) -> usize {
    text.split_word_bound_indices()
        .find(|(start, s)| start + s.len() > at && is_word(s))
        .map_or(text.len(), |(start, s)| start + s.len())
}

fn prev_word_start(text: &str, at: usize) -> usize {
    text.split_word_bound_indices()
        .rev()
        .find(|(start, s)| *start < at && is_word(s))
        .map_or(0, |(start, _)| start)
}

impl Editor {
    #[must_use]
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            selection: Selection::caret(text.len()),
            ..Self::default()
        }
    }

    /// The committed text (without preedit).
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn selection(&self) -> Selection {
        self.selection
    }

    #[must_use]
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection.range()]
    }

    #[must_use]
    pub fn preedit(&self) -> Option<&Preedit> {
        self.preedit.as_ref()
    }

    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.preedit.is_some()
    }

    /// Set the selection; offsets are clamped and snapped back onto
    /// grapheme boundaries.
    pub fn set_selection(&mut self, anchor: usize, focus: usize) {
        self.selection = Selection {
            anchor: snap(&self.text, anchor),
            focus: snap(&self.text, focus),
        };
        self.coalesce = false;
    }

    pub fn select_all(&mut self) {
        self.selection = Selection {
            anchor: 0,
            focus: self.text.len(),
        };
        self.coalesce = false;
    }

    /// Replace the whole text (not undoable; clears history and preedit).
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.selection = Selection::caret(text.len());
        self.preedit = None;
        self.undo.clear();
        self.redo.clear();
        self.coalesce = false;
    }

    // ------------------------------------------------------------ edits

    fn replace(&mut self, range: Range<usize>, inserted: &str, kind: EditKind) {
        let before = self.selection;
        let deleted = self.text[range.clone()].to_owned();
        self.text.replace_range(range.clone(), inserted);
        let caret = snap_forward(&self.text, range.start + inserted.len());
        self.selection = Selection::caret(caret);
        let op = EditOp {
            kind,
            range,
            deleted,
            inserted: inserted.to_owned(),
            selection_before: before,
            selection_after: self.selection,
        };
        self.record(op);
    }

    fn record(&mut self, op: EditOp) {
        self.redo.clear();
        let joins = self.coalesce
            && op.kind == EditKind::Insert
            && op.deleted.is_empty()
            && !op.inserted.starts_with(char::is_whitespace)
            && self.undo.last().and_then(|g| g.last()).is_some_and(|last| {
                last.kind == EditKind::Insert
                    && last.deleted.is_empty()
                    && last.range.start + last.inserted.len() == op.range.start
            });
        self.coalesce = op.kind == EditKind::Insert;
        self.ops.push(op.clone());
        match self.undo.last_mut() {
            Some(group) if joins => group.push(op),
            _ => self.undo.push(vec![op]),
        }
    }

    /// Type or paste: replaces the selection with `s`. Consecutive typing
    /// is grouped into one undo step (broken at whitespace and by moves).
    pub fn insert(&mut self, s: &str) {
        self.cancel_preedit();
        if s.is_empty() && self.selection.is_collapsed() {
            return;
        }
        self.replace(self.selection.range(), s, EditKind::Insert);
    }

    /// Delete the selection, or one grapheme in `direction`.
    pub fn delete(&mut self, direction: Direction) {
        self.delete_by(direction, false);
    }

    /// Delete the selection, or to the previous word start / next word end.
    pub fn delete_word(&mut self, direction: Direction) {
        self.delete_by(direction, true);
    }

    fn delete_by(&mut self, direction: Direction, word: bool) {
        self.cancel_preedit();
        let range = if self.selection.is_collapsed() {
            let at = self.selection.focus;
            match (direction, word) {
                (Direction::Backward, false) => prev_grapheme(&self.text, at)..at,
                (Direction::Forward, false) => at..next_grapheme(&self.text, at),
                (Direction::Backward, true) => prev_word_start(&self.text, at)..at,
                (Direction::Forward, true) => at..next_word_end(&self.text, at),
            }
        } else {
            self.selection.range()
        };
        if range.is_empty() {
            return;
        }
        self.coalesce = false;
        self.replace(range, "", EditKind::Delete);
        self.coalesce = false;
    }

    // --------------------------------------------------------- movement

    fn move_focus(&mut self, to: usize, extend: bool) {
        let to = snap(&self.text, to);
        self.selection = if extend {
            Selection {
                anchor: self.selection.anchor,
                focus: to,
            }
        } else {
            Selection::caret(to)
        };
        self.coalesce = false;
    }

    /// Move one grapheme. Without `extend`, a non-empty selection collapses
    /// to its start/end instead.
    pub fn move_grapheme(&mut self, direction: Direction, extend: bool) {
        if self.is_composing() {
            return;
        }
        let sel = self.selection;
        let to = match (direction, extend, sel.is_collapsed()) {
            (Direction::Backward, false, false) => sel.range().start,
            (Direction::Forward, false, false) => sel.range().end,
            (Direction::Backward, ..) => prev_grapheme(&self.text, sel.focus),
            (Direction::Forward, ..) => next_grapheme(&self.text, sel.focus),
        };
        self.move_focus(to, extend);
    }

    /// Move to the previous word start or the next word end.
    pub fn move_word(&mut self, direction: Direction, extend: bool) {
        if self.is_composing() {
            return;
        }
        let at = self.selection.focus;
        let to = match direction {
            Direction::Backward => prev_word_start(&self.text, at),
            Direction::Forward => next_word_end(&self.text, at),
        };
        self.move_focus(to, extend);
    }

    /// Move to the start or end of the text.
    pub fn move_to_edge(&mut self, direction: Direction, extend: bool) {
        if self.is_composing() {
            return;
        }
        let to = match direction {
            Direction::Backward => 0,
            Direction::Forward => self.text.len(),
        };
        self.move_focus(to, extend);
    }

    fn layout_cursor(&self) -> TextCursor {
        TextCursor::new(self.selection.focus)
    }

    /// Visual left/right by grapheme or word (bidi-aware). `layout` must be
    /// built from [`Editor::display_text`].
    pub fn move_visual(&mut self, layout: &TextLayout, right: bool, word: bool, extend: bool) {
        if self.is_composing() {
            return;
        }
        let to = layout.move_visual(self.layout_cursor(), right, word).index;
        self.move_focus(to, extend);
    }

    /// Up/down by visual lines.
    pub fn move_lines(&mut self, layout: &TextLayout, delta: isize, extend: bool) {
        if self.is_composing() {
            return;
        }
        let to = layout.move_lines(self.layout_cursor(), delta).index;
        self.move_focus(to, extend);
    }

    /// Home/End of the visual line.
    pub fn move_to_line_edge(&mut self, layout: &TextLayout, end: bool, extend: bool) {
        if self.is_composing() {
            return;
        }
        let to = layout.line_edge(self.layout_cursor(), end).index;
        self.move_focus(to, extend);
    }

    /// Click (or shift-click / drag with `extend`) at a layout point.
    pub fn click(&mut self, layout: &TextLayout, x: f32, y: f32, extend: bool) {
        if self.is_composing() {
            return;
        }
        let to = layout.hit_test(x, y).index;
        self.move_focus(to, extend);
    }

    // -------------------------------------------------------------- IME

    /// Update the composition string. An empty `text` ends composition
    /// (like a cancel). Starting a composition over a selection deletes the
    /// selection.
    pub fn set_preedit(&mut self, text: &str, cursor: Option<Range<usize>>) {
        if text.is_empty() {
            self.preedit = None;
            return;
        }
        if self.preedit.is_none() && !self.selection.is_collapsed() {
            self.replace(self.selection.range(), "", EditKind::Delete);
            self.coalesce = false;
        }
        let cursor = cursor.map(|r| {
            let s = r.start.min(text.len());
            let e = r.end.clamp(s, text.len());
            if text.is_char_boundary(s) && text.is_char_boundary(e) {
                s..e
            } else {
                text.len()..text.len()
            }
        });
        self.preedit = Some(Preedit {
            text: text.to_owned(),
            cursor,
        });
    }

    /// Finish composition, inserting `text` as one undoable edit.
    pub fn commit(&mut self, text: &str) {
        self.preedit = None;
        if text.is_empty() && self.selection.is_collapsed() {
            return;
        }
        self.coalesce = false;
        self.replace(self.selection.range(), text, EditKind::ImeCommit);
        self.coalesce = false;
    }

    /// Drop the composition without changing the text.
    pub fn cancel_preedit(&mut self) {
        self.preedit = None;
    }

    /// Text to lay out: committed text with the preedit at the caret.
    #[must_use]
    pub fn display_text(&self) -> Cow<'_, str> {
        match &self.preedit {
            None => Cow::Borrowed(&self.text),
            Some(p) => {
                let at = self.selection.focus;
                let mut s = String::with_capacity(self.text.len() + p.text.len());
                s.push_str(&self.text[..at]);
                s.push_str(&p.text);
                s.push_str(&self.text[at..]);
                Cow::Owned(s)
            }
        }
    }

    /// Range of the preedit in the display text (draw it underlined).
    #[must_use]
    pub fn preedit_range(&self) -> Option<Range<usize>> {
        let p = self.preedit.as_ref()?;
        let at = self.selection.focus;
        Some(at..at + p.text.len())
    }

    /// The IME's target/cursor range in display text (draw a thick
    /// underline when non-empty).
    #[must_use]
    pub fn preedit_target(&self) -> Option<Range<usize>> {
        let p = self.preedit.as_ref()?;
        let at = self.selection.focus;
        p.cursor.as_ref().map(|r| at + r.start..at + r.end)
    }

    /// Caret in display text coordinates (`None` when the IME hides it).
    #[must_use]
    pub fn display_caret(&self) -> Option<usize> {
        match &self.preedit {
            None => Some(self.selection.focus),
            Some(p) => p.cursor.as_ref().map(|r| self.selection.focus + r.end),
        }
    }

    /// Selection in display text coordinates (empty while composing).
    #[must_use]
    pub fn display_selection(&self) -> Range<usize> {
        match self.display_caret() {
            Some(c) if self.preedit.is_some() => c..c,
            _ => self.selection.range(),
        }
    }

    // ------------------------------------------------------- undo/redo

    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Stop the current typing group, so the next insert starts a new undo
    /// step (call on focus loss, after a pause, etc.).
    pub fn break_undo_group(&mut self) {
        self.coalesce = false;
    }

    fn apply_raw(&mut self, op: &EditOp) {
        self.text.replace_range(op.range.clone(), &op.inserted);
        self.selection = op.selection_after;
        self.ops.push(op.clone());
    }

    /// Undo the last step. Returns whether anything changed.
    pub fn undo(&mut self) -> bool {
        self.preedit = None;
        self.coalesce = false;
        let Some(group) = self.undo.pop() else {
            return false;
        };
        for op in group.iter().rev() {
            self.apply_raw(&op.inverse(EditKind::Undo));
        }
        self.redo.push(group);
        true
    }

    /// Redo the last undone step. Returns whether anything changed.
    pub fn redo(&mut self) -> bool {
        self.preedit = None;
        self.coalesce = false;
        let Some(group) = self.redo.pop() else {
            return false;
        };
        for op in &group {
            let mut op = op.clone();
            op.kind = EditKind::Redo;
            self.apply_raw(&op);
        }
        self.undo.push(group);
        true
    }

    /// Drain the edit-op stream (every text change since the last call,
    /// including undo/redo) for persistence, sync or accessibility.
    pub fn take_ops(&mut self) -> Vec<EditOp> {
        std::mem::take(&mut self.ops)
    }

    /// Whether `at` is a valid caret position (grapheme boundary).
    #[must_use]
    pub fn is_boundary(&self, at: usize) -> bool {
        is_grapheme_boundary(&self.text, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grapheme_moves_skip_clusters() {
        // e + combining acute, family emoji (ZWJ sequence), Georgian.
        let mut e = Editor::new("e\u{301}👨‍👩‍👧ა");
        e.move_grapheme(Direction::Backward, false);
        assert_eq!(e.selection().focus, e.text().len() - "ა".len());
        e.move_grapheme(Direction::Backward, false);
        assert_eq!(e.selection().focus, "e\u{301}".len());
        e.move_grapheme(Direction::Backward, false);
        assert_eq!(e.selection().focus, 0);
    }

    #[test]
    fn insert_before_combining_mark_snaps_forward() {
        let mut e = Editor::new("\u{301}");
        e.set_selection(0, 0);
        e.insert("a");
        assert_eq!(e.text(), "a\u{301}");
        assert_eq!(e.selection(), Selection::caret(e.text().len()));
    }

    #[test]
    fn word_moves_and_deletes() {
        let mut e = Editor::new("hello, brave new world");
        e.move_word(Direction::Backward, false);
        assert_eq!(e.selection().focus, 17);
        e.move_word(Direction::Backward, false);
        assert_eq!(e.selection().focus, 13);
        e.delete_word(Direction::Backward);
        assert_eq!(e.text(), "hello, new world");
        e.move_to_edge(Direction::Backward, false);
        e.move_word(Direction::Forward, true);
        assert_eq!(e.selected_text(), "hello");
    }

    #[test]
    fn typing_coalesces_until_whitespace() {
        let mut e = Editor::new("");
        for c in ["a", "b", "c", " ", "d", "e"] {
            e.insert(c);
        }
        assert_eq!(e.text(), "abc de");
        assert!(e.undo());
        assert_eq!(e.text(), "abc");
        assert!(e.undo());
        assert_eq!(e.text(), "");
        assert!(!e.undo());
        assert!(e.redo());
        assert!(e.redo());
        assert_eq!(e.text(), "abc de");
    }

    #[test]
    fn op_stream_replays() {
        let mut e = Editor::new("abc");
        e.insert("d");
        e.set_selection(0, 1);
        e.delete(Direction::Forward);
        e.undo();
        e.redo();
        let mut replay = String::from("abc");
        for op in e.take_ops() {
            assert_eq!(&replay[op.range.clone()], op.deleted);
            replay.replace_range(op.range, &op.inserted);
        }
        assert_eq!(replay, e.text());
    }
}
