// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Slider`]: a horizontal slider bound to a `Model<f32>`.

use livspeil_ui_core::{
    App, DragPhase, Element, EventCx, Model, Render, SharedString, Transition, ValueRequest, View,
    ViewContext, actions, div, text,
};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Color, Elevation, MotionClass};

use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics};

/// Slider thumb size.
pub const SLIDER_THUMB: f32 = 16.0;

/// A horizontal slider bound to a `Model<f32>`.
///
/// Pointer down jumps to the position; dragging follows the pointer 1:1
/// (no spring while dragging); on release the value snaps to `step` and the
/// thumb springs there carrying the release velocity. Arrow keys step,
/// Shift+arrows and Page Up/Down step ×10, Home/End go to the ends (key
/// context `Slider`). The thumb grows on hover and press. With
/// [`Slider::show_value`] the formatted value is shown to the right.
#[derive(Debug)]
pub struct Slider {
    /// The value.
    pub value: Model<f32>,
    /// Minimum.
    pub min: f32,
    /// Maximum.
    pub max: f32,
    /// Step (> 0).
    pub step: f32,
    /// Track width.
    pub width: f32,
    /// Accessible name.
    pub label: Option<SharedString>,
    /// Show the value label.
    pub show_value: bool,
    /// Disabled.
    pub disabled: bool,
    dragging: bool,
}

impl Slider {
    /// A slider over `min..=max` in steps of `step`, 200 px wide.
    #[must_use]
    pub fn new(value: Model<f32>, min: f32, max: f32, step: f32) -> Self {
        Slider {
            value,
            min,
            max: max.max(min),
            step: if step > 0.0 { step } else { 1.0 },
            width: 200.0,
            label: None,
            show_value: false,
            disabled: false,
            dragging: false,
        }
    }

    /// Set the track width.
    #[must_use]
    pub fn width(mut self, w: f32) -> Self {
        self.width = w.max(SLIDER_THUMB);
        self
    }

    /// Show the value to the right of the track.
    #[must_use]
    pub fn show_value(mut self) -> Self {
        self.show_value = true;
        self
    }

    /// Accessible name.
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

    /// Whether a drag is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Snap to the step grid (from `min`) and clamp.
    #[must_use]
    pub fn snap(&self, v: f32) -> f32 {
        let steps = ((v - self.min) / self.step).round();
        (self.min + steps * self.step).clamp(self.min, self.max)
    }

    fn fraction(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        if span <= 0.0 {
            0.0
        } else {
            ((v - self.min) / span).clamp(0.0, 1.0)
        }
    }

    fn value_at(&self, x: f32, track: Rect) -> f32 {
        let usable = (track.width - SLIDER_THUMB).max(1.0);
        let f = ((x - track.x - SLIDER_THUMB / 2.0) / usable).clamp(0.0, 1.0);
        self.min + f * (self.max - self.min)
    }

    /// Thumb x offset for value `v`.
    #[must_use]
    pub fn thumb_offset(&self, v: f32) -> f32 {
        self.fraction(v) * (self.width - SLIDER_THUMB)
    }

    /// The value as shown: with as many decimals as the step has.
    #[must_use]
    pub fn format(&self, v: f32) -> String {
        let decimals = style::step_decimals(f64::from(self.step));
        format!("{v:.decimals$}")
    }

    /// Semantics: `Slider` with value and range.
    #[must_use]
    pub fn semantics(&self, app: &App) -> Semantics {
        let v = *self.value.read(app);
        Semantics::new(Role::Slider)
            .label_opt(self.label.as_deref())
            .value(self.format(v))
            .numeric(
                f64::from(v),
                f64::from(self.min),
                f64::from(self.max),
                f64::from(self.step),
            )
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
}

fn set_value(slider: &View<Slider>, cx: &mut App, f: impl FnOnce(&Slider, f32) -> f32) {
    let (model, new) = {
        let s = slider.read(cx);
        let current = *s.value.read(cx);
        (s.value, f(s, current))
    };
    model.update(cx, |v, mcx| {
        if (*v - new).abs() > f32::EPSILON {
            *v = new;
            mcx.notify();
        }
    });
}

impl Render for Slider {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let value = *self.value.read(cx);
        let view = cx.view();
        let theme = cx.theme().clone();
        let c = theme.colors();
        let accent = theme.accent();
        let offset = self.thumb_offset(value);
        let h = SLIDER_THUMB + 4.0;
        let track_y = h / 2.0 - 2.0;
        let mut slider = div()
            .name("slider")
            .size(self.width, h)
            .child(
                div()
                    .key("track")
                    .absolute(Rect::new(0.0, track_y, self.width, 4.0))
                    .rounded(2.0)
                    .bg(c.surface[3])
                    .hit_test(false),
            )
            .child(
                div()
                    .key("fill")
                    .absolute(Rect::new(0.0, track_y, offset + SLIDER_THUMB / 2.0, 4.0))
                    .rounded(2.0)
                    .bg(accent.with_alpha(0.7))
                    .hit_test(false),
            );
        let mut thumb = div()
            .key("thumb")
            .absolute(Rect::new(0.0, 2.0, SLIDER_THUMB, SLIDER_THUMB))
            .rounded(SLIDER_THUMB / 2.0)
            .bg(Color::WHITE)
            .border(1.0, accent)
            .elevation(Elevation::E1)
            .translate(Vec2::new(offset, 0.0))
            .transition_with((!self.dragging).then_some(Transition::Motion(MotionClass::Micro)));
        if self.disabled {
            thumb = thumb.hit_test(false);
        } else {
            thumb = thumb.hover(|r| r.scale(1.15)).active(|r| r.scale(1.25));
            let step_action = |delta: f32| {
                move |cx: &mut EventCx<'_>| {
                    set_value(&view, cx, |s, v| s.snap(v + delta * s.step));
                }
            };
            slider = slider
                .key_context("Slider")
                .focusable()
                .on_pointer_down(move |ev, cx| {
                    let track = cx.bounds();
                    set_value(&view, cx, |s, _| s.snap(s.value_at(ev.position.x, track)));
                })
                .on_drag(move |ev, cx| {
                    let track = cx.bounds();
                    let thumb = cx.element_id().child("thumb");
                    match ev.phase {
                        DragPhase::Start | DragPhase::Move => {
                            view.update(cx, |s, vcx| {
                                if !s.dragging {
                                    s.dragging = true;
                                    vcx.notify();
                                }
                            });
                            set_value(&view, cx, |s, _| s.value_at(ev.position.x, track));
                        }
                        DragPhase::End => {
                            view.update(cx, |s, vcx| {
                                s.dragging = false;
                                vcx.notify();
                            });
                            set_value(&view, cx, |s, v| s.snap(v));
                            cx.set_translate_velocity(thumb, Vec2::new(ev.velocity.x, 0.0));
                        }
                    }
                })
                .on_action(actions::INCREMENT, step_action(1.0))
                .on_action(actions::DECREMENT, step_action(-1.0))
                .on_action(actions::INCREMENT_LARGE, step_action(10.0))
                .on_action(actions::DECREMENT_LARGE, step_action(-10.0))
                .on_action(actions::MIN, move |cx| set_value(&view, cx, |s, _| s.min))
                .on_action(actions::MAX, move |cx| set_value(&view, cx, |s, _| s.max));
        }
        slider = self.semantics(cx).apply(slider);
        if !self.disabled {
            slider = slider.on_set_value(move |req, cx| {
                if let ValueRequest::Number(v) = req {
                    #[allow(clippy::cast_possible_truncation)]
                    let v = *v as f32;
                    set_value(&view, cx, |s, _| s.snap(v));
                }
            });
        }
        slider = slider.child(thumb);
        let el = if self.show_value {
            let m = metrics(&theme);
            div()
                .name("slider-row")
                .flex_row()
                .items_center()
                .gap(m.gap)
                .child(slider)
                .child(
                    text(self.format(value))
                        .key("value")
                        .font_size(m.font)
                        .text_color(c.text_secondary)
                        .hit_test(false),
                )
        } else {
            slider
        };
        style::disabled(el, self.disabled)
    }
}
