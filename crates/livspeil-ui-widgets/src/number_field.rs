// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`NumberField`]: a numeric field with Houdini-style drag-scrub.

use livspeil_ui_core::{
    App, DRAG_THRESHOLD, Element, EventCx, Model, Modifiers, Render, SharedString, ValueRequest,
    View, ViewContext, actions as core, div, text,
};
use livspeil_ui_layout::{Point, Rect};
use livspeil_ui_theme::MotionClass;

use crate::env::Env;
use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics, step_decimals};
use crate::text_box::{Outcome, TextBox};

/// Precision multiplier for scrubbing: Shift ×10 (coarse), Alt ×0.1
/// (fine), both or neither ×1.
#[must_use]
pub fn scrub_precision(mods: Modifiers) -> f64 {
    match (mods.shift, mods.alt) {
        (true, false) => 10.0,
        (false, true) => 0.1,
        _ => 1.0,
    }
}

/// Remove float noise (`0.1 + 0.2`): round to 9 decimals.
fn clean(v: f64) -> f64 {
    (v * 1e9).round() / 1e9
}

/// Scrub math, independent of the widget.
///
/// - Moving the pointer by `dx` pixels changes the raw (unrounded) value by
///   `dx × sensitivity × precision(modifiers)`; the raw value is clamped to
///   `min..=max`, so reversing direction responds at once at the ends.
/// - The shown value is the raw value rounded to the step grid (multiples
///   of `step` from zero; `step × 0.1` while Alt is held) and clamped.
/// - Modifiers are read on every move, so precision can change mid-drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scrub {
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
    /// Step.
    pub step: f64,
    /// Value change per pixel at ×1.
    pub sensitivity: f64,
}

impl Scrub {
    /// The raw value after moving `dx` pixels from `raw` with `mods`.
    #[must_use]
    pub fn advance(&self, raw: f64, dx: f32, mods: Modifiers) -> f64 {
        (raw + f64::from(dx) * self.sensitivity * scrub_precision(mods)).clamp(self.min, self.max)
    }

    /// The value shown for `raw` with `mods`.
    #[must_use]
    pub fn quantize(&self, raw: f64, mods: Modifiers) -> f64 {
        let q = if mods.alt && !mods.shift {
            self.step * 0.1
        } else {
            self.step
        };
        if q <= 0.0 {
            return clean(raw).clamp(self.min, self.max);
        }
        clean((raw / q).round() * q).clamp(self.min, self.max)
    }
}

#[derive(Clone, Copy, Debug)]
struct Press {
    start: Point,
    last_x: f32,
    raw: f64,
    scrubbing: bool,
}

/// A numeric field.
///
/// - **Scrub**: press and drag horizontally (past the 3 px threshold) to
///   change the value: one pixel = `sensitivity` (default `step`), Shift
///   ×10, Alt ×0.1 (and rounding to `step × 0.1`), clamped to `min..=max`
///   (see [`Scrub`]). Dragging back reverses immediately.
/// - **Type**: click without dragging (or Enter/Space, or type a digit) to
///   edit the text; Enter commits (clamped, not snapped), Escape reverts,
///   Tab commits and moves focus, losing focus commits. Invalid text
///   reverts.
/// - **Keys** (context `NumberField`): Up/Right +step, Down/Left −step,
///   Shift ×10, Alt ×0.1, Page Up/Down ×10, Home/End to min/max.
/// - Digits use tabular figures (`tnum`) in the editor layout. With a
///   finite range a fill bar shows the value's position.
///
/// The focusable element is named [`NumberField::element_name`].
pub struct NumberField {
    /// The value.
    pub value: Model<f64>,
    /// Minimum (may be `-inf`).
    pub min: f64,
    /// Maximum (may be `inf`).
    pub max: f64,
    /// Step (> 0).
    pub step: f64,
    /// Value change per scrubbed pixel at ×1.
    pub sensitivity: f64,
    /// Width.
    pub width: f32,
    env: Env,
    label: Option<SharedString>,
    decimals: Option<usize>,
    disabled: bool,
    name: Option<SharedString>,
    press: Option<Press>,
    edit: Option<TextBox>,
}

impl std::fmt::Debug for NumberField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NumberField")
            .field("value", &self.value)
            .field("min", &self.min)
            .field("max", &self.max)
            .field("step", &self.step)
            .field("editing", &self.edit)
            .finish_non_exhaustive()
    }
}

impl NumberField {
    /// A field over `min..=max` in steps of `step`, 120 px wide.
    #[must_use]
    pub fn new(env: &Env, value: Model<f64>, min: f64, max: f64, step: f64) -> Self {
        let step = if step > 0.0 && step.is_finite() {
            step
        } else {
            1.0
        };
        NumberField {
            value,
            min,
            max: max.max(min),
            step,
            sensitivity: step,
            width: 120.0,
            env: env.clone(),
            label: None,
            decimals: None,
            disabled: false,
            name: None,
            press: None,
            edit: None,
        }
    }

    /// Inline label (left, secondary) and accessible name.
    #[must_use]
    pub fn label(mut self, l: impl Into<SharedString>) -> Self {
        self.label = Some(l.into());
        self
    }

    /// Value change per pixel at ×1 (default: the step).
    #[must_use]
    pub fn sensitivity(mut self, s: f64) -> Self {
        self.sensitivity = s;
        self
    }

    /// Fixed display decimals (default: from the step).
    #[must_use]
    pub fn decimals(mut self, d: usize) -> Self {
        self.decimals = Some(d);
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

    /// Element name (unique; default `numberfield:<entity>`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// The scrub parameters.
    #[must_use]
    pub fn scrub(&self) -> Scrub {
        Scrub {
            min: self.min,
            max: self.max,
            step: self.step,
            sensitivity: self.sensitivity,
        }
    }

    /// Whether the text is being edited.
    #[must_use]
    pub fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    /// Whether a scrub is in progress.
    #[must_use]
    pub fn is_scrubbing(&self) -> bool {
        self.press.is_some_and(|p| p.scrubbing)
    }

    /// The edit text, while editing.
    #[must_use]
    pub fn edit_text(&self) -> Option<&str> {
        self.edit.as_ref().map(|t| t.editor.text())
    }

    /// Format `v`: the step's decimals (or [`NumberField::decimals`]),
    /// with up to three more when `v` is finer than that (Alt scrubbing).
    #[must_use]
    pub fn format(&self, v: f64) -> String {
        let base = self.decimals.unwrap_or_else(|| step_decimals(self.step));
        let d = (base..=base + 3)
            .find(|&d| {
                let p = 10f64.powi(i32::try_from(d).unwrap_or(0));
                ((v * p).round() / p - v).abs() < 1e-9
            })
            .unwrap_or(base + 3);
        let s = format!("{v:.d$}");
        if s.starts_with('-') && s.trim_start_matches(['-', '0', '.']).is_empty() {
            s[1..].to_owned()
        } else {
            s
        }
    }

    /// Parse typed text (`,` accepted as decimal point); clamps.
    #[must_use]
    pub fn parse(&self, s: &str) -> Option<f64> {
        let v: f64 = s.trim().replace(',', ".").parse().ok()?;
        v.is_finite().then(|| v.clamp(self.min, self.max))
    }

    /// The focusable element's name for `view`.
    #[must_use]
    pub fn element_name(view: &View<NumberField>, app: &App) -> String {
        view.read(app).name.as_deref().map_or_else(
            || style::entity_name("numberfield", view.id()),
            str::to_owned,
        )
    }

    /// Semantics: `SpinButton` with value and range (core: `Slider` with a
    /// numeric value and `SetValue`).
    #[must_use]
    pub fn semantics(&self, app: &App) -> Semantics {
        let v = *self.value.read(app);
        Semantics::new(Role::SpinButton)
            .label_opt(self.label.as_deref())
            .value(self.format(v))
            .numeric(v, self.min, self.max, self.step)
            .interactive(
                self.disabled,
                &[
                    SemanticAction::Increment,
                    SemanticAction::Decrement,
                    SemanticAction::SetValue,
                    SemanticAction::Focus,
                ],
            )
    }

    fn start_edit(&mut self, initial: Option<&str>, app: &App) {
        let size = metrics(app.theme()).font;
        let current = self.format(*self.value.read(app));
        let mut tb = TextBox::new(&self.env, initial.unwrap_or(&current), size, true);
        if initial.is_none() {
            tb.editor.select_all();
        }
        self.edit = Some(tb);
    }

    /// Commit the edit text: returns the new value (if valid).
    fn finish_edit(&mut self, commit: bool) -> Option<f64> {
        let tb = self.edit.take()?;
        if commit {
            self.parse(tb.editor.text())
        } else {
            None
        }
    }
}

fn set_value(model: Model<f64>, cx: &mut App, v: f64) {
    model.update(cx, |x, mcx| {
        if (*x - v).abs() > f64::EPSILON || x.to_bits() != v.to_bits() {
            *x = v;
            mcx.notify();
        }
    });
}

/// Update the field; a returned value is written to the model afterwards.
fn with(
    view: View<NumberField>,
    cx: &mut App,
    f: impl FnOnce(&mut NumberField, &mut App) -> Option<f64>,
) {
    let (model, v) = view.update(cx, |nf, vcx| {
        let v = f(nf, vcx);
        vcx.notify();
        (nf.value, v)
    });
    if let Some(v) = v {
        set_value(model, cx, v);
    }
}

fn step_by(view: View<NumberField>, factor: f64) -> impl Fn(&mut EventCx<'_>) {
    move |cx| {
        with(view, cx, |nf, app| {
            let v = *nf.value.read(app);
            let q = nf.step * factor.abs().min(1.0);
            let next = clean(((v + nf.step * factor) / q).round() * q);
            Some(next.clamp(nf.min, nf.max))
        });
    }
}

fn local(nf: &NumberField, bounds: Rect, p: Point, height: f32, pad: f32) -> Point {
    let tb = nf.edit.as_ref();
    let scroll = tb.map_or(0.0, |t| t.scroll);
    let ty = tb.map_or(0.0, |t| t.text_y(height));
    Point::new(p.x - bounds.x - pad + scroll, p.y - bounds.y - ty)
}

impl Render for NumberField {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let name = self.name.as_deref().map_or_else(
            || style::entity_name("numberfield", view.id()),
            str::to_owned,
        );
        let focused = !self.disabled && self.env.is_focused(cx, &name);
        // Losing focus commits the edit.
        if self.edit.is_some()
            && !focused
            && let Some(v) = self.finish_edit(true)
        {
            set_value(self.value, cx, v);
        }
        let value = *self.value.read(cx);
        let (height, pad) = (m.height, m.pad_x);
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
            .transition(MotionClass::Micro);
        let editing = self.edit.is_some();
        if let Some(tb) = &mut self.edit {
            tb.refresh(&self.env, m.font);
            tb.keep_caret_visible(self.width - 2.0 * pad);
            el = el.key_context("TextField").children(tb.elements(
                &theme,
                height,
                pad,
                focused,
                c.text_primary,
                None,
            ));
        } else {
            if self.min.is_finite() && self.max.is_finite() && self.max > self.min {
                #[allow(clippy::cast_possible_truncation)]
                let f = ((value - self.min) / (self.max - self.min)).clamp(0.0, 1.0) as f32;
                el = el.child(
                    div()
                        .key("fill")
                        .absolute(Rect::new(0.0, 0.0, self.width * f, height))
                        .bg(theme.accent().with_alpha(0.18))
                        .hit_test(false),
                );
            }
            let mut row = div()
                .key("content")
                .absolute(Rect::new(0.0, 0.0, self.width, height))
                .flex_row()
                .items_center()
                .justify_center()
                .gap(m.gap)
                .hit_test(false);
            if let Some(l) = &self.label {
                row = row.child(
                    text(l.clone())
                        .key("label")
                        .font_size(m.font)
                        .text_color(c.text_secondary)
                        .hit_test(false),
                );
            }
            row = row.child(
                text(self.format(value))
                    .key("value")
                    .font_size(m.font)
                    .text_color(c.text_primary)
                    .hit_test(false),
            );
            el = el.key_context("NumberField").child(row);
        }
        el = self.semantics(cx).apply(el);
        if self.disabled {
            return style::disabled(el, true);
        }
        let env = self.env.clone();
        el = el
            .focusable()
            .hover(|r| r.bg(c.surface[2]))
            .on_set_value(move |req, cx| {
                if let ValueRequest::Number(v) = *req {
                    with(view, cx, |nf, _| Some(v.clamp(nf.min, nf.max)));
                }
            })
            .on_pointer_down(move |ev, cx| {
                env.sync_focus(cx);
                let bounds = cx.bounds();
                let (p, time, shift) = (ev.position, ev.time, ev.modifiers.shift);
                with(view, cx, |nf, app| {
                    let raw = *nf.value.read(app);
                    nf.press = Some(Press {
                        start: p,
                        last_x: p.x,
                        raw,
                        scrubbing: false,
                    });
                    let l = local(nf, bounds, p, height, pad);
                    if let Some(tb) = &mut nf.edit {
                        tb.press(l, p, time, shift);
                    }
                    None
                });
            })
            .on_pointer_move(move |ev, cx| {
                let pressed = view.read(cx).press.is_some();
                if !pressed {
                    return;
                }
                let bounds = cx.bounds();
                let (p, mods) = (ev.position, ev.modifiers);
                with(view, cx, |nf, _| {
                    let l = local(nf, bounds, p, height, pad);
                    let scrub = nf.scrub();
                    let mut press = nf.press?;
                    if let Some(tb) = &mut nf.edit {
                        tb.drag(l);
                        return None;
                    }
                    if !press.scrubbing && (p - press.start).length() >= DRAG_THRESHOLD {
                        press.scrubbing = true;
                    }
                    let out = if press.scrubbing {
                        press.raw = scrub.advance(press.raw, p.x - press.last_x, mods);
                        press.last_x = p.x;
                        Some(scrub.quantize(press.raw, mods))
                    } else {
                        None
                    };
                    nf.press = Some(press);
                    out
                });
            })
            .on_pointer_up(move |_, cx| {
                with(view, cx, |nf, app| {
                    let press = nf.press.take()?;
                    if !press.scrubbing && nf.edit.is_none() {
                        nf.start_edit(None, app);
                    }
                    None
                });
            })
            .on_text_input(move |s, cx| {
                with(view, cx, |nf, app| {
                    if let Some(tb) = &mut nf.edit {
                        tb.insert_text(s);
                    } else if s.starts_with(|c: char| c.is_ascii_digit() || "-+.,".contains(c)) {
                        nf.start_edit(Some(s), app);
                    }
                    None
                });
            })
            .on_ime(move |ev, cx| {
                with(view, cx, |nf, _| {
                    if let Some(tb) = &mut nf.edit {
                        tb.ime(ev);
                    }
                    None
                });
            });
        if editing {
            let env = self.env.clone();
            let (env2, env3) = (env.clone(), env.clone());
            el = el
                .on_action(actions::SUBMIT, move |cx| {
                    with(view, cx, |nf, _| nf.finish_edit(true));
                })
                .on_action(core::CANCEL, move |cx| {
                    with(view, cx, |nf, _| nf.finish_edit(false));
                })
                .on_action(core::FOCUS_NEXT, move |cx| {
                    with(view, cx, |nf, _| nf.finish_edit(true));
                    cx.focus_next();
                    env2.sync_focus(cx);
                })
                .on_action(core::FOCUS_PREV, move |cx| {
                    with(view, cx, |nf, _| nf.finish_edit(true));
                    cx.focus_prev();
                    env3.sync_focus(cx);
                });
            for &a in crate::text_field::TEXT_ACTIONS {
                let env = env.clone();
                el = el.on_action(a, move |cx| {
                    let size = metrics(cx.theme()).font;
                    with(view, cx, |nf, _| {
                        if let Some(tb) = &mut nf.edit {
                            let _: Outcome = tb.action(&env, size, a);
                        }
                        None
                    });
                });
            }
        } else {
            el = el
                .on_action(core::ACTIVATE, move |cx| {
                    with(view, cx, |nf, app| {
                        nf.start_edit(None, app);
                        None
                    });
                })
                .on_action(core::INCREMENT, step_by(view, 1.0))
                .on_action(core::DECREMENT, step_by(view, -1.0))
                .on_action(core::INCREMENT_LARGE, step_by(view, 10.0))
                .on_action(core::DECREMENT_LARGE, step_by(view, -10.0))
                .on_action(actions::INCREMENT_SMALL, step_by(view, 0.1))
                .on_action(actions::DECREMENT_SMALL, step_by(view, -0.1))
                .on_action(core::MIN, move |cx| {
                    with(view, cx, |nf, _| nf.min.is_finite().then_some(nf.min));
                })
                .on_action(core::MAX, move |cx| {
                    with(view, cx, |nf, _| nf.max.is_finite().then_some(nf.max));
                });
        }
        el
    }
}
