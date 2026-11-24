// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The single-line editing core shared by text and number fields: an
//! [`Editor`] plus its [`TextLayout`], horizontal scroll, pointer
//! selection, action handling and the caret/selection/preedit elements.

use std::time::Duration;

use livspeil_ui_core::{Element, ImeEvent, div, text};
use livspeil_ui_layout::{Point, Rect};
use livspeil_ui_text::{
    Direction, Editor, FontFeature, LayoutOptions, TextCursor, TextLayout, TextStyle,
};
use livspeil_ui_theme::{Color, Theme};

use crate::env::Env;
use crate::keymap::actions;
use crate::style;

/// Two presses within this time and distance count as a multi-click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(400);
const MULTI_CLICK_DIST: f32 = 4.0;
/// Caret width.
pub(crate) const CARET_WIDTH: f32 = 1.0;

/// What an operation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// Not handled.
    Ignored,
    /// Selection or caret moved (or clipboard written).
    Moved,
    /// The committed text changed.
    Edited,
}

struct Cache {
    text: String,
    size: f32,
    layout: TextLayout,
}

/// Editing state of a single-line text box.
pub(crate) struct TextBox {
    pub(crate) editor: Editor,
    /// Horizontal scroll (px) keeping the caret visible.
    pub(crate) scroll: f32,
    tnum: bool,
    cache: Cache,
    last_press: Option<(Duration, Point, u8)>,
}

impl std::fmt::Debug for TextBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextBox")
            .field("text", &self.editor.text())
            .field("selection", &self.editor.selection())
            .field("scroll", &self.scroll)
            .finish_non_exhaustive()
    }
}

fn style_for(size: f32, tnum: bool) -> TextStyle {
    let s = TextStyle::sized(size);
    if tnum {
        s.with_feature(FontFeature::TNUM)
    } else {
        s
    }
}

/// Text input sanitized for a single line: line breaks become spaces,
/// other control characters are dropped.
pub(crate) fn single_line(s: &str) -> String {
    s.chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

impl TextBox {
    pub(crate) fn new(env: &Env, text: &str, size: f32, tnum: bool) -> Self {
        let layout = TextLayout::new(
            &mut env.fonts(),
            text,
            &style_for(size, tnum),
            LayoutOptions::default(),
        );
        TextBox {
            editor: Editor::new(text),
            scroll: 0.0,
            tnum,
            cache: Cache {
                text: text.to_owned(),
                size,
                layout,
            },
            last_press: None,
        }
    }

    /// Rebuild the layout if the display text or size changed.
    pub(crate) fn refresh(&mut self, env: &Env, size: f32) {
        let display = self.editor.display_text();
        if self.cache.text != *display || (self.cache.size - size).abs() > f32::EPSILON {
            let display = display.into_owned();
            let layout = TextLayout::new(
                &mut env.fonts(),
                &display,
                &style_for(size, self.tnum),
                LayoutOptions::default(),
            );
            self.cache = Cache {
                text: display,
                size,
                layout,
            };
        }
    }

    /// The layout of the display text (call [`TextBox::refresh`] first).
    #[allow(dead_code)]
    pub(crate) fn layout(&self) -> &TextLayout {
        &self.cache.layout
    }

    /// Caret x in layout coordinates.
    pub(crate) fn caret_x(&self) -> Option<f32> {
        self.editor
            .display_caret()
            .map(|c| self.cache.layout.cursor_rect(TextCursor::new(c), CARET_WIDTH).x)
    }

    /// Scroll so the caret is inside `inner_width`.
    pub(crate) fn keep_caret_visible(&mut self, inner_width: f32) {
        let content = self.cache.layout.width();
        let max = (content + CARET_WIDTH - inner_width).max(0.0);
        if let Some(x) = self.caret_x() {
            if x - self.scroll > inner_width - CARET_WIDTH {
                self.scroll = x - inner_width + CARET_WIDTH;
            }
            if x < self.scroll {
                self.scroll = x;
            }
        }
        self.scroll = self.scroll.clamp(0.0, max);
    }

    /// Apply an action. Returns what it did.
    pub(crate) fn action(&mut self, env: &Env, size: f32, action: &str) -> Outcome {
        self.refresh(env, size);
        let e = &mut self.editor;
        let layout = &self.cache.layout;
        let before = e.text().to_owned();
        let sel_before = e.selection();
        match action {
            actions::MOVE_LEFT => e.move_visual(layout, false, false, false),
            actions::MOVE_RIGHT => e.move_visual(layout, true, false, false),
            actions::SELECT_LEFT => e.move_visual(layout, false, false, true),
            actions::SELECT_RIGHT => e.move_visual(layout, true, false, true),
            actions::MOVE_WORD_LEFT => e.move_word(Direction::Backward, false),
            actions::MOVE_WORD_RIGHT => e.move_word(Direction::Forward, false),
            actions::SELECT_WORD_LEFT => e.move_word(Direction::Backward, true),
            actions::SELECT_WORD_RIGHT => e.move_word(Direction::Forward, true),
            actions::MOVE_HOME => e.move_to_edge(Direction::Backward, false),
            actions::MOVE_END => e.move_to_edge(Direction::Forward, false),
            actions::SELECT_HOME => e.move_to_edge(Direction::Backward, true),
            actions::SELECT_END => e.move_to_edge(Direction::Forward, true),
            actions::DELETE_BACKWARD => e.delete(Direction::Backward),
            actions::DELETE_FORWARD => e.delete(Direction::Forward),
            actions::DELETE_WORD_BACKWARD => e.delete_word(Direction::Backward),
            actions::DELETE_WORD_FORWARD => e.delete_word(Direction::Forward),
            actions::SELECT_ALL => e.select_all(),
            actions::UNDO => {
                e.undo();
            }
            actions::REDO => {
                e.redo();
            }
            actions::COPY | actions::CUT => {
                if e.selection().is_collapsed() {
                    return Outcome::Moved;
                }
                env.set_clipboard_text(e.selected_text());
                if action == actions::CUT {
                    e.delete(Direction::Backward);
                }
            }
            actions::PASTE => {
                let Some(t) = env.clipboard_text() else {
                    return Outcome::Moved;
                };
                e.insert(&single_line(&t));
            }
            actions::TEXT_KEY => return Outcome::Moved,
            _ => return Outcome::Ignored,
        }
        if e.text() != before {
            Outcome::Edited
        } else {
            let _ = sel_before;
            Outcome::Moved
        }
    }

    /// Typed or committed text.
    pub(crate) fn insert_text(&mut self, s: &str) -> Outcome {
        let s = single_line(s);
        if s.is_empty() {
            return Outcome::Ignored;
        }
        self.editor.insert(&s);
        Outcome::Edited
    }

    /// An IME event.
    pub(crate) fn ime(&mut self, ev: &ImeEvent) -> Outcome {
        let before = self.editor.text().to_owned();
        match ev {
            ImeEvent::Enabled => return Outcome::Ignored,
            ImeEvent::Preedit { text, cursor } => {
                self.editor
                    .set_preedit(&single_line(text), cursor.map(|(a, b)| a..b));
            }
            ImeEvent::Commit(t) => self.editor.commit(&single_line(t)),
            ImeEvent::Disabled => self.editor.cancel_preedit(),
        }
        if self.editor.text() == before {
            Outcome::Moved
        } else {
            Outcome::Edited
        }
    }

    /// Pointer press at `local` (layout coordinates, scroll applied) at
    /// host time `time`. Shift extends; double-click selects a word,
    /// triple-click all.
    pub(crate) fn press(&mut self, local: Point, window: Point, time: Duration, extend: bool) {
        let count = match self.last_press {
            Some((t, p, n))
                if time.saturating_sub(t) <= MULTI_CLICK_TIME
                    && (window - p).length() <= MULTI_CLICK_DIST =>
            {
                n % 3 + 1
            }
            _ => 1,
        };
        self.last_press = Some((time, window, count));
        let e = &mut self.editor;
        e.click(&self.cache.layout, local.x, local.y, extend && count == 1);
        match count {
            2 => {
                let at = e.selection().focus;
                e.move_word(Direction::Forward, false);
                let end = e.selection().focus;
                e.move_word(Direction::Backward, false);
                let start = e.selection().focus;
                if start <= at && at <= end {
                    e.set_selection(start, end);
                } else {
                    e.set_selection(at, at);
                }
            }
            3 => e.select_all(),
            _ => {}
        }
    }

    /// Pointer drag to `local`: extend the selection.
    pub(crate) fn drag(&mut self, local: Point) {
        self.editor
            .click(&self.cache.layout, local.x, local.y, true);
    }

    /// Elements for the text area: selection, text (or placeholder),
    /// preedit underlines and caret, absolutely positioned in a field of
    /// `height` with the text starting at `pad_x`. Call after `refresh` and
    /// `keep_caret_visible`.
    pub(crate) fn elements(
        &self,
        theme: &Theme,
        height: f32,
        pad_x: f32,
        focused: bool,
        text_color: Color,
        placeholder: Option<&str>,
    ) -> Vec<Element> {
        let layout = &self.cache.layout;
        let size = self.cache.size;
        let lh = layout.height().max(size * 1.2);
        let y = ((height - lh) / 2.0).round();
        let x0 = pad_x - self.scroll;
        let mut out = Vec::new();
        let sel = self.editor.display_selection();
        if !sel.is_empty() {
            for (i, r) in layout.selection_rects(sel.clone()).iter().enumerate() {
                out.push(
                    div()
                        .key(format!("selection-{i}"))
                        .absolute(Rect::new(x0 + r.x, y + r.y, r.width, r.height))
                        .bg(style::selection(theme, focused))
                        .hit_test(false),
                );
            }
        }
        let display = self.cache.text.clone();
        if display.is_empty() {
            if let Some(p) = placeholder {
                out.push(
                    text(p)
                        .key("placeholder")
                        .absolute(Rect::new(pad_x, y, 1.0, lh))
                        .style(|s| s.size.width = livspeil_ui_layout::style::Dimension::auto())
                        .font_size(size)
                        .text_color(theme.colors().text_tertiary)
                        .hit_test(false),
                );
            }
        } else {
            out.push(
                text(display)
                    .key("text")
                    .absolute(Rect::new(x0, y, layout.width().max(1.0), lh))
                    .font_size(size)
                    .text_color(text_color)
                    .hit_test(false),
            );
        }
        let underline = |key: String, r: &livspeil_ui_text::Rect, thick: f32| {
            div()
                .key(key)
                .absolute(Rect::new(x0 + r.x, y + r.y + r.height - thick, r.width, thick))
                .bg(text_color)
                .hit_test(false)
        };
        if let Some(range) = self.editor.preedit_range() {
            for (i, r) in layout.selection_rects(range).iter().enumerate() {
                out.push(underline(format!("preedit-{i}"), r, 1.0));
            }
        }
        if let Some(range) = self.editor.preedit_target().filter(|r| !r.is_empty()) {
            for (i, r) in layout.selection_rects(range).iter().enumerate() {
                out.push(underline(format!("preedit-target-{i}"), r, 2.0));
            }
        }
        if focused
            && sel.is_empty()
            && let Some(c) = self.editor.display_caret()
        {
            let r = layout.cursor_rect(TextCursor::new(c), CARET_WIDTH);
            out.push(
                div()
                    .key("caret")
                    .absolute(Rect::new(x0 + r.x, y + r.y, CARET_WIDTH, r.height.max(size)))
                    .bg(theme.accent())
                    .hit_test(false),
            );
        }
        out
    }

    /// Text y offset in a field of `height`.
    pub(crate) fn text_y(&self, height: f32) -> f32 {
        let lh = self.cache.layout.height().max(self.cache.size * 1.2);
        ((height - lh) / 2.0).round()
    }
}
