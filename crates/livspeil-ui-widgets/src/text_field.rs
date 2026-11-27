// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`TextField`]: single-line text input on `livspeil-ui-text`'s `Editor`.

use std::rc::Rc;

use livspeil_ui_core::{
    App, DragPhase, Element, EventCx, Render, SharedString, ValueRequest, View, ViewContext,
    actions as core, div,
};
use livspeil_ui_layout::Point;
use livspeil_ui_theme::MotionClass;

use crate::env::Env;
use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics};
use crate::text_box::{Outcome, TextBox};

/// Called with the new text after an edit, or the text on submit.
pub type TextFn = Rc<dyn Fn(&str, &mut App)>;

/// Every action a text field handles.
pub(crate) const TEXT_ACTIONS: &[&str] = &[
    actions::MOVE_LEFT,
    actions::MOVE_RIGHT,
    actions::SELECT_LEFT,
    actions::SELECT_RIGHT,
    actions::MOVE_WORD_LEFT,
    actions::MOVE_WORD_RIGHT,
    actions::SELECT_WORD_LEFT,
    actions::SELECT_WORD_RIGHT,
    actions::MOVE_HOME,
    actions::MOVE_END,
    actions::SELECT_HOME,
    actions::SELECT_END,
    actions::DELETE_BACKWARD,
    actions::DELETE_FORWARD,
    actions::DELETE_WORD_BACKWARD,
    actions::DELETE_WORD_FORWARD,
    actions::SELECT_ALL,
    actions::COPY,
    actions::CUT,
    actions::PASTE,
    actions::UNDO,
    actions::REDO,
    actions::TEXT_KEY,
];

/// A single-line text input.
///
/// - Caret and selection from `livspeil-ui-text` layout hit testing; click
///   places the caret, Shift+click and drag extend, double-click selects a
///   word, triple-click all. The field scrolls horizontally to keep the
///   caret visible.
/// - Keyboard (key context `TextField`): arrows (visual, bidi-aware),
///   Ctrl/Alt+arrows by word, Home/End, Shift extends, Backspace/Delete
///   (Ctrl/Alt by word), select all, copy/cut/paste through
///   [`Env`]'s clipboard, undo/redo (typing is grouped). Enter submits.
///   All are keymap actions (`text.*`, `edit.*`).
/// - IME: the preedit is shown at the caret with a thin underline (thick
///   under the IME's target range); commit inserts it as one undo step.
/// - Placeholder when empty; focus shows the caret and an accent border
///   (plus the core focus ring when focused from the keyboard).
///
/// The focusable element is named [`TextField::element_name`].
pub struct TextField {
    env: Env,
    tb: TextBox,
    placeholder: Option<SharedString>,
    label: Option<SharedString>,
    width: f32,
    disabled: bool,
    on_change: Option<TextFn>,
    on_submit: Option<TextFn>,
    name: Option<SharedString>,
}

impl std::fmt::Debug for TextField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextField")
            .field("text_box", &self.tb)
            .field("placeholder", &self.placeholder)
            .finish_non_exhaustive()
    }
}

impl TextField {
    /// A 200 px wide field with initial `text`.
    #[must_use]
    pub fn new(env: &Env, text: &str) -> Self {
        TextField {
            env: env.clone(),
            tb: TextBox::new(env, &crate::text_box::single_line(text), 13.0, false),
            placeholder: None,
            label: None,
            width: 200.0,
            disabled: false,
            on_change: None,
            on_submit: None,
            name: None,
        }
    }

    /// Placeholder shown when empty.
    #[must_use]
    pub fn placeholder(mut self, p: impl Into<SharedString>) -> Self {
        self.placeholder = Some(p.into());
        self
    }

    /// Accessible name.
    #[must_use]
    pub fn label(mut self, l: impl Into<SharedString>) -> Self {
        self.label = Some(l.into());
        self
    }

    /// Width.
    #[must_use]
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    /// Element name (must be unique; default `textfield:<entity>`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Called after every edit with the new text.
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(&str, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// Called on Enter with the text.
    #[must_use]
    pub fn on_submit(mut self, f: impl Fn(&str, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }

    /// The committed text.
    #[must_use]
    pub fn text(&self) -> &str {
        self.tb.editor.text()
    }

    /// The editor (selection, preedit, undo state).
    #[must_use]
    pub fn editor(&self) -> &livspeil_ui_text::Editor {
        &self.tb.editor
    }

    /// Horizontal scroll offset.
    #[must_use]
    pub fn scroll(&self) -> f32 {
        self.tb.scroll
    }

    /// Replace the text (not undoable).
    pub fn set_text(&mut self, text: &str) {
        self.tb.editor.set_text(&crate::text_box::single_line(text));
    }

    /// The focusable element's name for `view`.
    #[must_use]
    pub fn element_name(view: &View<TextField>, app: &App) -> String {
        view.read(app)
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("textfield", view.id()), str::to_owned)
    }

    /// Semantics: `TextInput` with value, placeholder and selection.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let sel = self.tb.editor.selection();
        let mut s = Semantics::new(Role::TextInput)
            .label_opt(self.label.as_deref())
            .value(self.text())
            .placeholder(self.placeholder.as_deref())
            .interactive(
                self.disabled,
                &[
                    SemanticAction::Focus,
                    SemanticAction::SetValue,
                    SemanticAction::SetTextSelection,
                    SemanticAction::Clipboard,
                ],
            );
        s.text_selection = Some((sel.anchor, sel.focus));
        s
    }
}

/// Run `f` on the field, then fire `on_change` if the text changed.
fn edit(view: View<TextField>, cx: &mut App, f: impl FnOnce(&mut TextField, f32) -> Outcome) {
    let size = metrics(cx.theme()).font;
    let changed = view.update(cx, |tf, vcx| {
        let out = f(tf, size);
        if out != Outcome::Ignored {
            vcx.notify();
        }
        (out == Outcome::Edited)
            .then(|| tf.on_change.clone().map(|cb| (cb, tf.text().to_owned())))
            .flatten()
    });
    if let Some((cb, text)) = changed {
        cb(&text, cx);
    }
}

/// A pointer position in text coordinates.
fn local(cx: &EventCx<'_>, tf: &TextField, p: Point, height: f32, pad: f32) -> Point {
    let b = cx.bounds();
    let p = cx.layout_position(p);
    Point::new(
        p.x - b.x - pad + tf.tb.scroll,
        p.y - b.y - tf.tb.text_y(height),
    )
}

impl Render for TextField {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let name = self
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("textfield", view.id()), str::to_owned);
        let focused = !self.disabled && self.env.is_focused(cx, &name);
        if !focused {
            self.tb.editor.break_undo_group();
        }
        let (height, pad) = (m.height, m.pad_x);
        self.tb.refresh(&self.env, m.font);
        self.tb.keep_caret_visible(self.width - 2.0 * pad);
        let border = if focused {
            theme.accent()
        } else {
            style::hairline(&theme)
        };
        let mut el = div()
            .name(name)
            .size(self.width, height)
            .rounded(m.radius)
            .bg(c.surface[1])
            .border(1.0, border)
            .clip()
            .transition(MotionClass::Micro)
            .children(self.tb.elements(
                &theme,
                height,
                pad,
                focused,
                c.text_primary,
                self.placeholder.as_deref(),
            ));
        el = self.semantics().apply(el);
        if self.disabled {
            return style::disabled(el, true);
        }
        let env = self.env.clone();
        el = el
            .key_context("TextField")
            .focusable()
            .hover(|r| r.bg(c.surface[2]))
            .on_pointer_down(move |ev, cx| {
                env.sync_focus(cx);
                let p = ev.position;
                let (shift, time) = (ev.modifiers.shift, ev.time);
                let l = local(cx, view.read(cx), p, height, pad);
                edit(view, cx, |tf, size| {
                    tf.tb.refresh(&tf.env, size);
                    tf.tb.press(l, p, time, shift);
                    Outcome::Moved
                });
            })
            .on_drag(move |ev, cx| {
                if ev.phase == DragPhase::End {
                    return;
                }
                let l = local(cx, view.read(cx), ev.position, height, pad);
                edit(view, cx, |tf, size| {
                    tf.tb.refresh(&tf.env, size);
                    tf.tb.drag(l);
                    Outcome::Moved
                });
            })
            .on_text_input(move |s, cx| edit(view, cx, |tf, _| tf.tb.insert_text(s)))
            .on_ime(move |ev, cx| edit(view, cx, |tf, _| tf.tb.ime(ev)))
            .on_set_value(move |req, cx| {
                if let ValueRequest::Text(t) = req {
                    edit(view, cx, |tf, _| {
                        tf.tb.editor.select_all();
                        tf.tb.editor.insert(&crate::text_box::single_line(t));
                        Outcome::Edited
                    });
                }
            })
            .on_action(actions::SUBMIT, move |cx| {
                let cb = view.read(cx).on_submit.clone();
                let text = view.read(cx).text().to_owned();
                if let Some(cb) = cb {
                    cb(&text, cx);
                }
            })
            .on_action(core::CANCEL, move |cx| {
                edit(view, cx, |tf, _| {
                    if tf.tb.editor.is_composing() {
                        tf.tb.editor.cancel_preedit();
                        Outcome::Moved
                    } else {
                        Outcome::Ignored
                    }
                });
            });
        for &a in TEXT_ACTIONS {
            el = el.on_action(a, move |cx| {
                let env = view.read(cx).env.clone();
                edit(view, cx, |tf, size| tf.tb.action(&env, size, a));
            });
        }
        el
    }
}
