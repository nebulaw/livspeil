// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Binary and exclusive choices: [`Toggle`] (switch), [`Checkbox`]
//! (with a mixed state), [`Radio`] and [`RadioGroup`].

use std::rc::Rc;

use livspeil_ui_core::{Element, EventCx, SharedString, actions as core_actions, div, text};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Color, MotionClass, Theme};

use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics, Toggled};
use crate::style::{self, metrics};

type ChangeFn<T> = Rc<dyn Fn(T, &mut EventCx<'_>)>;
type ActionFn = Rc<dyn Fn(&mut EventCx<'_>)>;

/// A row: indicator then optional label; clickable and focusable as a whole.
fn labeled_row(
    theme: &Theme,
    indicator: Element,
    label: Option<&SharedString>,
    disabled: bool,
) -> Element {
    let m = metrics(theme);
    let mut row = div().flex_row().items_center().gap(m.gap).child(indicator);
    if let Some(l) = label {
        row = row.child(
            text(l.clone())
                .name("label")
                .font_size(m.font)
                .text_color(theme.colors().text_primary)
                .hit_test(false),
        );
    }
    if !disabled {
        row = row.focusable();
    }
    style::disabled(row, disabled)
}

/// An on/off switch. The thumb (child `thumb`) springs between positions
/// with the micro spring and keeps its velocity when retargeted mid-flight.
/// Click, Space or Enter flip it.
#[derive(Clone)]
pub struct Toggle {
    on: bool,
    label: Option<SharedString>,
    disabled: bool,
    on_change: Option<ChangeFn<bool>>,
}

impl std::fmt::Debug for Toggle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toggle")
            .field("on", &self.on)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl Toggle {
    /// A switch showing `on`.
    #[must_use]
    pub fn new(on: bool) -> Self {
        Toggle {
            on,
            label: None,
            disabled: false,
            on_change: None,
        }
    }

    /// A label to the right.
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the new value when flipped.
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(bool, &mut EventCx<'_>) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// Semantics: `Switch`, toggled.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Switch)
            .label_opt(self.label.as_deref())
            .toggled(if self.on {
                Toggled::True
            } else {
                Toggled::False
            })
            .interactive(self.disabled, &[SemanticAction::Click, SemanticAction::Focus])
    }

    /// Build the element (name `toggle`, children `track` > `thumb`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let sem = self.semantics();
        let c = theme.colors();
        let (w, h, thumb) = metrics(theme).switch;
        let inset = (h - thumb) / 2.0;
        let travel = w - thumb - 2.0 * inset;
        let fill = if self.on { theme.accent() } else { c.surface[3] };
        let mut track = div()
            .key("track")
            .size(w, h)
            .rounded(h / 2.0)
            .bg(fill)
            .transition(MotionClass::Micro)
            .child(
                div()
                    .key("thumb")
                    .absolute(Rect::new(inset, inset, thumb, thumb))
                    .rounded(thumb / 2.0)
                    .bg(Color::WHITE)
                    .translate(Vec2::new(if self.on { travel } else { 0.0 }, 0.0))
                    .transition(MotionClass::Micro)
                    .hit_test(false),
            );
        if !self.disabled {
            track = track
                .hover(|r| r.bg(style::emphasize(theme, fill, 0.04)))
                .active(|r| r.bg(style::emphasize(theme, fill, 0.08)));
        }
        let mut row = labeled_row(theme, track, self.label.as_ref(), self.disabled).name("toggle");
        if let (false, Some(f)) = (self.disabled, self.on_change) {
            let next = !self.on;
            row = row.on_click(move |_, cx| f(next, cx));
        }
        sem.apply(row)
    }
}

/// Check box state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum CheckState {
    /// Not checked.
    #[default]
    Unchecked,
    /// Checked.
    Checked,
    /// Indeterminate (some children checked).
    Mixed,
}

impl CheckState {
    /// The state after a click: mixed and unchecked become checked,
    /// checked becomes unchecked.
    #[must_use]
    pub fn next(self) -> CheckState {
        match self {
            CheckState::Checked => CheckState::Unchecked,
            CheckState::Unchecked | CheckState::Mixed => CheckState::Checked,
        }
    }

    /// From a bool.
    #[must_use]
    pub fn from_bool(on: bool) -> CheckState {
        if on {
            CheckState::Checked
        } else {
            CheckState::Unchecked
        }
    }

    fn toggled(self) -> Toggled {
        match self {
            CheckState::Unchecked => Toggled::False,
            CheckState::Checked => Toggled::True,
            CheckState::Mixed => Toggled::Mixed,
        }
    }
}

/// A check box with an optional label and a mixed state. The mark (child
/// `mark`) fades and scales in with the micro spring. Click, Space or Enter
/// move to [`CheckState::next`].
#[derive(Clone)]
pub struct Checkbox {
    state: CheckState,
    label: Option<SharedString>,
    disabled: bool,
    on_change: Option<ChangeFn<CheckState>>,
}

impl std::fmt::Debug for Checkbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Checkbox")
            .field("state", &self.state)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl Checkbox {
    /// A check box in `state`.
    #[must_use]
    pub fn new(state: CheckState) -> Self {
        Checkbox {
            state,
            label: None,
            disabled: false,
            on_change: None,
        }
    }

    /// A label to the right.
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called with the next state when clicked.
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(CheckState, &mut EventCx<'_>) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// Semantics: `CheckBox`, toggled (true/false/mixed).
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::CheckBox)
            .label_opt(self.label.as_deref())
            .toggled(self.state.toggled())
            .interactive(self.disabled, &[SemanticAction::Click, SemanticAction::Focus])
    }

    /// Build the element (name `checkbox`, children `box` > `mark`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let sem = self.semantics();
        let m = metrics(theme);
        let s = m.indicator;
        let filled = self.state != CheckState::Unchecked;
        let accent = theme.accent();
        let (bg, border) = if filled {
            (accent, accent)
        } else {
            (theme.colors().surface[1], style::indicator_border(theme))
        };
        let glyph = if self.state == CheckState::Mixed {
            "–"
        } else {
            "✓"
        };
        let mut b = div()
            .key("box")
            .size(s, s)
            .rounded(theme.tokens.radii.xs)
            .items_center()
            .justify_center()
            .bg(bg)
            .border(1.0, border)
            .transition(MotionClass::Micro)
            .child(
                text(glyph)
                    .key("mark")
                    .font_size(s * 0.8)
                    .text_color(style::on_color(accent))
                    .opacity(if filled { 1.0 } else { 0.0 })
                    .scale(if filled { 1.0 } else { 0.6 })
                    .transition(MotionClass::Micro)
                    .hit_test(false),
            );
        if !self.disabled {
            b = b
                .hover(|r| r.bg(style::emphasize(theme, bg, 0.04)))
                .active(|r| r.scale(0.92));
        }
        let mut row = labeled_row(theme, b, self.label.as_ref(), self.disabled).name("checkbox");
        if let (false, Some(f)) = (self.disabled, self.on_change) {
            let next = self.state.next();
            row = row.on_click(move |_, cx| f(next, cx));
        }
        sem.apply(row)
    }
}

/// The circular indicator of a radio button.
fn radio_indicator(theme: &Theme, selected: bool, disabled: bool) -> Element {
    let s = metrics(theme).indicator;
    let accent = theme.accent();
    let (bg, border) = if selected {
        (accent, accent)
    } else {
        (theme.colors().surface[1], style::indicator_border(theme))
    };
    let dot = s * 0.4;
    let mut ind = div()
        .key("circle")
        .size(s, s)
        .rounded(s / 2.0)
        .bg(bg)
        .border(1.0, border)
        .transition(MotionClass::Micro)
        .child(
            div()
                .key("dot")
                .absolute(Rect::new((s - dot) / 2.0, (s - dot) / 2.0, dot, dot))
                .rounded(dot / 2.0)
                .bg(Color::WHITE)
                .opacity(if selected { 1.0 } else { 0.0 })
                .scale(if selected { 1.0 } else { 0.4 })
                .transition(MotionClass::Micro)
                .hit_test(false),
        );
    if !disabled {
        ind = ind
            .hover(|r| r.bg(style::emphasize(theme, bg, 0.04)))
            .active(|r| r.scale(0.92));
    }
    ind
}

/// A single radio button. Usually used through [`RadioGroup`], which owns
/// keyboard navigation; a standalone radio is focusable and selects on
/// click, Space or Enter.
#[derive(Clone)]
pub struct Radio {
    label: SharedString,
    selected: bool,
    disabled: bool,
    on_select: Option<ActionFn>,
}

impl std::fmt::Debug for Radio {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Radio")
            .field("label", &self.label)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl Radio {
    /// A radio button.
    #[must_use]
    pub fn new(label: impl Into<SharedString>, selected: bool) -> Self {
        Radio {
            label: label.into(),
            selected,
            disabled: false,
            on_select: None,
        }
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Called when chosen.
    #[must_use]
    pub fn on_select(mut self, f: impl Fn(&mut EventCx<'_>) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }

    /// Semantics: `RadioButton`, selected.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::RadioButton)
            .label(&*self.label)
            .selected(self.selected)
            .interactive(self.disabled, &[SemanticAction::Click, SemanticAction::Focus])
    }

    /// Build the element (name `radio:<label>`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let sem = self.semantics();
        let ind = radio_indicator(theme, self.selected, self.disabled);
        let mut row = labeled_row(theme, ind, Some(&self.label), self.disabled)
            .name(format!("radio:{}", self.label));
        if let (false, Some(f)) = (self.disabled, self.on_select) {
            row = row.on_click(move |_, cx| f(cx));
        }
        sem.apply(row)
    }
}

/// Exclusive choice among labeled options.
///
/// The group is one tab stop (key context `RadioGroup`); arrow keys move the
/// selection to the next/previous enabled option and wrap, Home/End go to
/// the first/last. Space/Enter select the first option when none is
/// selected. Clicking an option selects it. Options are children keyed
/// `radio-<i>`.
#[derive(Clone)]
pub struct RadioGroup {
    options: Vec<SharedString>,
    enabled: Vec<bool>,
    selected: Option<usize>,
    label: Option<SharedString>,
    horizontal: bool,
    disabled: bool,
    on_change: Option<ChangeFn<usize>>,
}

impl std::fmt::Debug for RadioGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RadioGroup")
            .field("options", &self.options)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl RadioGroup {
    /// A vertical group with `options`, `selected` chosen.
    #[must_use]
    pub fn new<S: Into<SharedString>>(
        options: impl IntoIterator<Item = S>,
        selected: Option<usize>,
    ) -> Self {
        let options: Vec<SharedString> = options.into_iter().map(Into::into).collect();
        let enabled = vec![true; options.len()];
        RadioGroup {
            options,
            enabled,
            selected,
            label: None,
            horizontal: false,
            disabled: false,
            on_change: None,
        }
    }

    /// Accessible name of the group.
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Lay options out in a row.
    #[must_use]
    pub fn horizontal(mut self) -> Self {
        self.horizontal = true;
        self
    }

    /// Disable the whole group.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Disable one option (skipped by arrow keys).
    #[must_use]
    pub fn disable_option(mut self, index: usize) -> Self {
        if let Some(e) = self.enabled.get_mut(index) {
            *e = false;
        }
        self
    }

    /// Called with the chosen index.
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(usize, &mut EventCx<'_>) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// Semantics: `RadioGroup` with one `RadioButton` per option.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(Role::RadioGroup)
            .label_opt(self.label.as_deref())
            .interactive(self.disabled, &[SemanticAction::Focus]);
        if let Some(i) = self.selected.and_then(|i| self.options.get(i)) {
            s = s.value(&**i);
        }
        for (i, o) in self.options.iter().enumerate() {
            s = s.child(
                Semantics::new(Role::RadioButton)
                    .label(&**o)
                    .selected(self.selected == Some(i))
                    .interactive(self.disabled || !self.enabled[i], &[SemanticAction::Click]),
            );
        }
        s
    }

    /// Build the element (name `radio-group`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let sem = self.semantics();
        let m = metrics(theme);
        let mut group = div().name("radio-group").gap(m.gap).key_context("RadioGroup");
        group = if self.horizontal {
            group.flex_row().items_center()
        } else {
            group.flex_col()
        };
        for (i, label) in self.options.iter().enumerate() {
            let enabled = !self.disabled && self.enabled[i];
            let selected = self.selected == Some(i);
            let ind = radio_indicator(theme, selected, !enabled);
            let mut row = div()
                .key(format!("radio-{i}"))
                .name(format!("radio:{label}"))
                .flex_row()
                .items_center()
                .gap(m.gap)
                .h(m.height)
                .child(ind)
                .child(
                    text(label.clone())
                        .name("label")
                        .font_size(m.font)
                        .text_color(theme.colors().text_primary)
                        .hit_test(false),
                );
            if !self.disabled && !self.enabled[i] {
                row = row.opacity(style::DISABLED_OPACITY);
            }
            if let (true, Some(f)) = (enabled, self.on_change.clone()) {
                row = row.on_click(move |_, cx| f(i, cx));
            }
            if let Some(part) = sem.children.get(i) {
                row = part.apply(row);
            }
            group = group.child(row);
        }
        group = sem.apply(group);
        if self.disabled {
            return style::disabled(group, true);
        }
        group = group.focusable();
        if let Some(f) = self.on_change {
            let enabled = Rc::new(self.enabled);
            let current = self.selected;
            let nav = |dir: isize, edge: bool| {
                let f = f.clone();
                let enabled = enabled.clone();
                move |cx: &mut EventCx<'_>| {
                    let next = if edge {
                        style::edge_enabled(&enabled, dir)
                    } else {
                        style::step_enabled(&enabled, current, dir, true)
                    };
                    if let Some(n) = next.filter(|n| Some(*n) != current) {
                        f(n, cx);
                    }
                }
            };
            group = group
                .on_action(actions::NEXT, nav(1, false))
                .on_action(actions::PREV, nav(-1, false))
                .on_action(actions::FIRST, nav(1, true))
                .on_action(actions::LAST, nav(-1, true));
            let first = nav(1, true);
            group = group.on_action(core_actions::ACTIVATE, move |cx| {
                if current.is_none() {
                    first(cx);
                }
            });
        }
        group
    }
}
