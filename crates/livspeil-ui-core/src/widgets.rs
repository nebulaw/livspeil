// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Proof widgets for the core: [`button`], [`toggle`] and [`Slider`].
//!
//! The real widget set is `livspeil-ui-widgets` (Button, Toggle, Slider and
//! more, with densities, states and semantics); use that in applications.
//! These stay as minimal fixtures for the core's and `livspeil-ui-a11y`'s
//! tests, which cannot depend on the widget crate.

use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{MotionClass, Theme};

use crate::element::{Element, SharedString, div, text};
use crate::entity::{Model, Render, ViewContext};
use crate::keymap::actions;
use crate::semantics::{Role, SemanticValue, ValueRequest};

/// A push button: hover, press and keyboard focus animate with the
/// micro spring; Space/Enter activate it when focused. Add `.on_click`.
#[must_use]
pub fn button(theme: &Theme, label: impl Into<SharedString>) -> Element {
    let c = theme.colors();
    let label: SharedString = label.into();
    div()
        .name(format!("button:{label}"))
        .h(theme.metrics().row_height)
        .px(theme.tokens.spacing.step(3))
        .items_center()
        .justify_center()
        .rounded(theme.tokens.radii.sm)
        .bg(c.surface[2])
        .border(1.0, livspeil_ui_theme::Color::WHITE.with_alpha(0.06))
        .hover(|r| r.bg(c.surface[3]))
        .active(|r| r.bg(c.surface[3]).scale(0.97))
        .focus_visual(|r| r.border_color(c.focus_ring))
        .transition(MotionClass::Micro)
        .focusable()
        // a11y (M1.7)
        .role(Role::Button)
        .semantics(|s| s.label(label.clone()))
        .child(text(label).name("label").hit_test(false))
}

/// Toggle geometry.
const TRACK_W: f32 = 36.0;
const TRACK_H: f32 = 20.0;
const THUMB: f32 = 16.0;

/// A switch. The thumb (child key `thumb`) springs between positions with
/// the micro spring. Add `.on_click` to flip the value.
#[must_use]
pub fn toggle(theme: &Theme, on: bool) -> Element {
    let c = theme.colors();
    let travel = TRACK_W - THUMB - 4.0;
    div()
        .name("toggle")
        .size(TRACK_W, TRACK_H)
        .rounded(TRACK_H / 2.0)
        .bg(if on { theme.accent() } else { c.surface[3] })
        .transition(MotionClass::Micro)
        .focusable()
        // a11y (M1.7); add a name with `.semantics(|s| s.label(..))`
        .role(Role::Switch)
        .semantics(|s| s.checked(on))
        .child(
            div()
                .key("thumb")
                .absolute(Rect::new(2.0, 2.0, THUMB, THUMB))
                .rounded(THUMB / 2.0)
                .bg(livspeil_ui_theme::Color::WHITE)
                .translate(Vec2::new(if on { travel } else { 0.0 }, 0.0))
                .transition(MotionClass::Micro)
                .hit_test(false),
        )
}

/// A horizontal slider bound to a `Model<f32>`.
///
/// Pointer down jumps to the position; dragging follows the pointer 1:1
/// (no spring while dragging); on release the value snaps to `step` and the
/// thumb springs there carrying the release velocity. Arrow keys step,
/// Shift+arrows step ×10, Home/End go to the ends (key context `Slider`).
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
    /// Accessible name (a11y, M1.7).
    pub label: Option<SharedString>,
    dragging: bool,
}

/// Slider thumb size.
pub const SLIDER_THUMB: f32 = 16.0;

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
            dragging: false,
        }
    }

    /// Whether a drag is in progress.
    #[must_use]
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    fn snap(&self, v: f32) -> f32 {
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
}

fn set_value(
    slider: &crate::entity::View<Slider>,
    cx: &mut crate::app::App,
    f: impl FnOnce(&Slider, f32) -> f32,
) {
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
        let theme = cx.theme();
        let c = theme.colors();
        let accent = theme.accent();
        let offset = self.thumb_offset(value);
        let step_action = |delta: f32| {
            move |cx: &mut crate::app::EventCx<'_>| {
                set_value(&view, cx, |s, v| s.snap(v + delta * s.step));
            }
        };
        div()
            .name("slider")
            .size(self.width, SLIDER_THUMB + 4.0)
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
                    crate::input::DragPhase::Start | crate::input::DragPhase::Move => {
                        view.update(cx, |s, vcx| {
                            if !s.dragging {
                                s.dragging = true;
                                vcx.notify();
                            }
                        });
                        set_value(&view, cx, |s, _| s.value_at(ev.position.x, track));
                    }
                    crate::input::DragPhase::End => {
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
            .on_action(actions::MAX, move |cx| set_value(&view, cx, |s, _| s.max))
            // a11y (M1.7)
            .role(Role::Slider)
            .semantics(|s| {
                let s = s.value(SemanticValue::number(
                    f64::from(value),
                    f64::from(self.min),
                    f64::from(self.max),
                    f64::from(self.step),
                ));
                match &self.label {
                    Some(l) => s.label(l.clone()),
                    None => s,
                }
            })
            .on_set_value(move |req, cx| {
                if let ValueRequest::Number(v) = req {
                    #[allow(clippy::cast_possible_truncation)]
                    let v = *v as f32;
                    set_value(&view, cx, |s, _| s.snap(v));
                }
            })
            .child(
                div()
                    .key("track")
                    .absolute(Rect::new(
                        0.0,
                        (SLIDER_THUMB + 4.0) / 2.0 - 2.0,
                        self.width,
                        4.0,
                    ))
                    .rounded(2.0)
                    .bg(c.surface[3])
                    .hit_test(false),
            )
            .child(
                div()
                    .key("thumb")
                    .absolute(Rect::new(0.0, 2.0, SLIDER_THUMB, SLIDER_THUMB))
                    .rounded(SLIDER_THUMB / 2.0)
                    .bg(accent)
                    .translate(Vec2::new(offset, 0.0))
                    .transition_with(
                        (!self.dragging)
                            .then_some(crate::element::Transition::Motion(MotionClass::Micro)),
                    )
                    .hit_test(false),
            )
    }
}
