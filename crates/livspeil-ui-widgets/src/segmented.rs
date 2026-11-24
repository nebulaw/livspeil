// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`SegmentedControl`]: exclusive segments with a sliding selection pill.

use std::rc::Rc;

use livspeil_ui_core::{Element, EventCx, SharedString, div, text};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Elevation, MotionClass, Theme};

use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics};

/// Inset of the pill and segments inside the track.
pub const SEGMENT_INSET: f32 = 2.0;

type ChangeFn = Rc<dyn Fn(usize, &mut EventCx<'_>)>;

/// Equal-width segments; the selected one sits on a pill (child `pill`)
/// that slides to a new selection with the standard spring (keeping its
/// velocity when retargeted). One tab stop (key context
/// `SegmentedControl`): Left/Right move the selection (no wrap),
/// Home/End go to the ends. Segments are children keyed `segment-<i>`.
#[derive(Clone)]
pub struct SegmentedControl {
    options: Vec<SharedString>,
    selected: usize,
    segment_width: Option<f32>,
    label: Option<SharedString>,
    disabled: bool,
    on_change: Option<ChangeFn>,
}

impl std::fmt::Debug for SegmentedControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentedControl")
            .field("options", &self.options)
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl SegmentedControl {
    /// Segments `options` with `selected` chosen.
    #[must_use]
    pub fn new<S: Into<SharedString>>(
        options: impl IntoIterator<Item = S>,
        selected: usize,
    ) -> Self {
        SegmentedControl {
            options: options.into_iter().map(Into::into).collect(),
            selected,
            segment_width: None,
            label: None,
            disabled: false,
            on_change: None,
        }
    }

    /// Fixed segment width (default 80 in Simple density, 64 in Pro).
    #[must_use]
    pub fn segment_width(mut self, w: f32) -> Self {
        self.segment_width = Some(w);
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

    /// Called with the chosen index.
    #[must_use]
    pub fn on_change(mut self, f: impl Fn(usize, &mut EventCx<'_>) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    /// The segment width for `theme`.
    #[must_use]
    pub fn width_for(&self, theme: &Theme) -> f32 {
        self.segment_width.unwrap_or(match theme.density {
            livspeil_ui_theme::Density::Simple => 80.0,
            livspeil_ui_theme::Density::Pro => 64.0,
        })
    }

    /// Semantics: `SegmentedControl` with one `Segment` per option.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let mut s = Semantics::new(Role::SegmentedControl)
            .label_opt(self.label.as_deref())
            .interactive(self.disabled, &[SemanticAction::Focus]);
        if let Some(o) = self.options.get(self.selected) {
            s = s.value(&**o);
        }
        for (i, o) in self.options.iter().enumerate() {
            s = s.child(
                Semantics::new(Role::Segment)
                    .label(&**o)
                    .selected(i == self.selected)
                    .interactive(self.disabled, &[SemanticAction::Click]),
            );
        }
        s
    }

    /// Build the element (name `segmented`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let sem = self.semantics();
        let m = metrics(theme);
        let c = theme.colors();
        let w = self.width_for(theme);
        let n = self.options.len();
        let h = m.height;
        let inner_h = h - 2.0 * SEGMENT_INSET;
        #[allow(clippy::cast_precision_loss)]
        let total = w * n as f32 + 2.0 * SEGMENT_INSET;
        let sel = self.selected.min(n.saturating_sub(1));
        #[allow(clippy::cast_precision_loss)]
        let pill_x = w * sel as f32;
        let mut el = div()
            .name("segmented")
            .key_context("SegmentedControl")
            .size(total, h)
            .flex_row()
            .p(SEGMENT_INSET)
            .rounded(m.radius)
            .bg(c.surface[1])
            .border(1.0, style::hairline(theme))
            .child(
                div()
                    .key("pill")
                    .absolute(Rect::new(SEGMENT_INSET, SEGMENT_INSET, w, inner_h))
                    .rounded((m.radius - SEGMENT_INSET).max(2.0))
                    .bg(c.surface[3])
                    .elevation(Elevation::E1)
                    .opacity(if n == 0 { 0.0 } else { 1.0 })
                    .translate(Vec2::new(pill_x, 0.0))
                    .transition(MotionClass::Standard)
                    .hit_test(false),
            );
        for (i, label) in self.options.iter().enumerate() {
            let color = if i == sel {
                c.text_primary
            } else {
                c.text_secondary
            };
            let mut seg = div()
                .key(format!("segment-{i}"))
                .name(format!("segment:{label}"))
                .size(w, inner_h)
                .items_center()
                .justify_center()
                .rounded((m.radius - SEGMENT_INSET).max(2.0))
                .transition(MotionClass::Micro)
                .child(
                    text(label.clone())
                        .name("label")
                        .font_size(m.font)
                        .text_color(color)
                        .hit_test(false),
                );
            if !self.disabled {
                if i != sel {
                    seg = seg.hover(|r| r.bg(c.surface[2].with_alpha(0.6)));
                }
                if let Some(f) = self.on_change.clone() {
                    seg = seg.on_click(move |_, cx| f(i, cx));
                }
            }
            if let Some(part) = sem.children.get(i) {
                seg = part.apply(seg);
            }
            el = el.child(seg);
        }
        el = sem.apply(el);
        if self.disabled {
            return style::disabled(el, true);
        }
        el = el
            .focusable()
            .focus_visual(|r| r.border_color(c.focus_ring));
        if let Some(f) = self.on_change {
            let enabled = Rc::new(vec![true; n]);
            let nav = |dir: isize, edge: bool| {
                let f = f.clone();
                let enabled = enabled.clone();
                move |cx: &mut EventCx<'_>| {
                    let next = if edge {
                        style::edge_enabled(&enabled, dir)
                    } else {
                        style::step_enabled(&enabled, Some(sel), dir, false)
                    };
                    if let Some(i) = next.filter(|i| *i != sel) {
                        f(i, cx);
                    }
                }
            };
            el = el
                .on_action(actions::NEXT, nav(1, false))
                .on_action(actions::PREV, nav(-1, false))
                .on_action(actions::FIRST, nav(1, true))
                .on_action(actions::LAST, nav(-1, true));
        }
        el
    }
}
