// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Tooltip`]: a delayed Glass bubble shown in the overlay layer while the
//! pointer rests on an anchor element.

use std::time::Duration;

use livspeil_ui_core::{App, Element, SharedString, div, text};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_text::{LayoutOptions, TextLayout, TextStyle};
use livspeil_ui_theme::{Material, MotionClass};

use crate::env::{Env, OverlayKey, OverlayKind};
use crate::semantics::{Role, Semantics};
use crate::style::metrics;

/// Default hover delay.
pub const TOOLTIP_DELAY: Duration = Duration::from_millis(500);

/// Gap between the anchor and the bubble.
const GAP: f32 = 6.0;
/// Margin kept from the viewport edges.
const MARGIN: f32 = 4.0;

/// A tooltip waiting or shown in the overlay layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TooltipEntry {
    pub(crate) text: SharedString,
    pub(crate) anchor: Rect,
    /// Delay in seconds.
    pub(crate) delay: f32,
}

/// Where a bubble of `size` goes for `anchor` in `viewport`: centered below
/// the anchor, above it if it would leave the viewport, clamped
/// horizontally.
#[must_use]
pub fn place(anchor: Rect, size: Vec2, viewport: Vec2) -> Rect {
    let mut y = anchor.bottom() + GAP;
    if y + size.y > viewport.y - MARGIN && anchor.y - GAP - size.y >= MARGIN {
        y = anchor.y - GAP - size.y;
    }
    let x = (anchor.center().x - size.x / 2.0)
        .min(viewport.x - MARGIN - size.x)
        .max(MARGIN);
    Rect::new(x, y, size.x, size.y)
}

/// The bubble element for an entry (in window coordinates).
pub(crate) fn bubble(
    env: &Env,
    app: &App,
    key: OverlayKey,
    t: &TooltipEntry,
    appearing: bool,
) -> Element {
    let theme = app.theme();
    let m = metrics(theme);
    let size = (m.font - 1.0).max(11.0);
    let (w, h) = {
        let mut fonts = env.fonts();
        let l = TextLayout::new(
            &mut fonts,
            &t.text,
            &TextStyle::sized(size),
            LayoutOptions::default(),
        );
        (l.width().ceil(), l.height().ceil())
    };
    let (px, py) = (theme.tokens.spacing.step(2), theme.tokens.spacing.step(1));
    let vp = app.viewport();
    let r = place(
        t.anchor,
        Vec2::new(w + 2.0 * px, h + 2.0 * py),
        Vec2::new(vp.width, vp.height),
    );
    let id = match key {
        OverlayKey::Tooltip(e) => format!("tooltip-{:016x}", e.0),
        OverlayKey::Popup(p) => format!("tooltip-{p:?}"),
    };
    div()
        .key(id)
        .name("tooltip")
        .absolute(r)
        .rounded(theme.tokens.radii.sm)
        .material(Material::Glass)
        .items_center()
        .justify_center()
        .opacity(if appearing { 0.0 } else { 1.0 })
        .translate(Vec2::new(0.0, if appearing { -2.0 } else { 0.0 }))
        .transition(MotionClass::Micro)
        .hit_test(false)
        .semantics(|s| s.role(livspeil_ui_core::Role::Label).label(t.text.clone()))
        .child(
            text(t.text.clone())
                .name("tooltip-text")
                .font_size(size)
                .text_color(theme.colors().text_primary)
                .hit_test(false),
        )
}

/// A tooltip: attach it to any element. After the pointer has rested on
/// the element for the delay (default 500 ms), a Glass bubble appears below
/// it (above if there is no room) and fades in; it hides when the pointer
/// leaves the element or any button is pressed. Needs [`Env::root`].
#[derive(Clone, Debug)]
pub struct Tooltip {
    text: SharedString,
    delay: Duration,
}

impl Tooltip {
    /// A tooltip with `text`.
    #[must_use]
    pub fn new(text: impl Into<SharedString>) -> Self {
        Tooltip {
            text: text.into(),
            delay: TOOLTIP_DELAY,
        }
    }

    /// Hover delay (zero shows at once).
    #[must_use]
    pub fn delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// Semantics: `Tooltip` with the text (the anchor is described by it).
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Tooltip).label(&*self.text)
    }

    /// Attach to `anchor`.
    #[must_use]
    pub fn attach(self, env: &Env, anchor: Element) -> Element {
        let env = env.clone();
        let anchor = anchor.semantics({
            let d = self.text.clone();
            move |s| s.description(d)
        });
        let entry = TooltipEntry {
            text: self.text,
            anchor: Rect::ZERO,
            delay: self.delay.as_secs_f32(),
        };
        anchor.on_pointer_move(move |_, cx| {
            let id = cx.element_id();
            let key = OverlayKey::Tooltip(id);
            if cx.is_hovered(id) && !env.has_overlay(cx, key) {
                let mut e = entry.clone();
                e.anchor = cx.bounds();
                env.show_overlay(cx, key, OverlayKind::Tooltip(e));
            }
        })
    }
}
