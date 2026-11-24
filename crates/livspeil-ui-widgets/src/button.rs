// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Button`] and [`IconButton`].

use std::rc::Rc;

use livspeil_ui_core::{ClickEvent, Element, EventCx, SharedString, div, text};
use livspeil_ui_theme::{Color, MotionClass, Theme};

use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, ClickFn, metrics};

/// Visual emphasis of a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ButtonVariant {
    /// Filled with the accent: the main action.
    Primary,
    /// Raised surface with a hairline (default).
    #[default]
    Secondary,
    /// No surface until hovered.
    Ghost,
    /// Filled with the danger color.
    Destructive,
}

/// Fill colors of a variant: (rest, hover, pressed, text).
fn palette(theme: &Theme, variant: ButtonVariant) -> (Color, Color, Color, Color) {
    let c = theme.colors();
    match variant {
        ButtonVariant::Primary => {
            let a = theme.accent();
            (
                a,
                style::shade(a, 0.05),
                style::shade(a, -0.05),
                style::on_color(a),
            )
        }
        ButtonVariant::Destructive => {
            let d = c.danger;
            (
                d,
                style::shade(d, 0.05),
                style::shade(d, -0.05),
                style::on_color(d),
            )
        }
        ButtonVariant::Secondary => (c.surface[2], c.surface[3], c.surface[3], c.text_primary),
        ButtonVariant::Ghost => (
            c.surface[2].with_alpha(0.0),
            c.surface[2],
            c.surface[3],
            c.text_primary,
        ),
    }
}

/// Shared chrome of buttons: fill, hover/press/focus refinements animated
/// with the micro spring, focusability and the click handler.
fn chrome(
    theme: &Theme,
    variant: ButtonVariant,
    disabled: bool,
    on_click: Option<ClickFn>,
) -> Element {
    let (rest, hover, pressed, _) = palette(theme, variant);
    let m = metrics(theme);
    let mut el = div()
        .h(m.height)
        .items_center()
        .justify_center()
        .rounded(m.radius)
        .bg(rest)
        .transition(MotionClass::Micro);
    if variant == ButtonVariant::Secondary {
        el = el.border(1.0, style::hairline(theme));
    } else {
        el = el.border(1.0, theme.colors().focus_ring.with_alpha(0.0));
    }
    if disabled {
        return style::disabled(el, true);
    }
    el = el
        .hover(|r| r.bg(hover))
        .active(|r| r.bg(pressed).scale(0.97))
        .focus_visual(|r| r.border_color(theme.colors().focus_ring))
        .focusable();
    if let Some(f) = on_click {
        el = el.on_click(move |e, cx| f(e, cx));
    }
    el
}

/// A push button with a text label.
///
/// Hover, press (scale 0.97) and keyboard focus animate with the micro
/// spring; Space/Enter activate it when focused (`ui.activate`). Disabled
/// buttons are faded, not focusable and ignore input.
#[derive(Clone)]
pub struct Button {
    label: SharedString,
    variant: ButtonVariant,
    disabled: bool,
    on_click: Option<ClickFn>,
}

impl std::fmt::Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("label", &self.label)
            .field("variant", &self.variant)
            .field("disabled", &self.disabled)
            .finish_non_exhaustive()
    }
}

impl Button {
    /// A secondary button.
    #[must_use]
    pub fn new(label: impl Into<SharedString>) -> Self {
        Button {
            label: label.into(),
            variant: ButtonVariant::Secondary,
            disabled: false,
            on_click: None,
        }
    }

    /// Set the variant.
    #[must_use]
    pub fn variant(mut self, v: ButtonVariant) -> Self {
        self.variant = v;
        self
    }

    /// Primary variant.
    #[must_use]
    pub fn primary(self) -> Self {
        self.variant(ButtonVariant::Primary)
    }

    /// Ghost variant.
    #[must_use]
    pub fn ghost(self) -> Self {
        self.variant(ButtonVariant::Ghost)
    }

    /// Destructive variant.
    #[must_use]
    pub fn destructive(self) -> Self {
        self.variant(ButtonVariant::Destructive)
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Click (pointer or keyboard) handler.
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// Semantics: `Button` with the label.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Button)
            .label(&*self.label)
            .interactive(
                self.disabled,
                &[SemanticAction::Click, SemanticAction::Focus],
            )
    }

    /// Build the element (name `button:<label>`, label child `label`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let m = metrics(theme);
        let (.., fg) = palette(theme, self.variant);
        let sem = self.semantics();
        let el = chrome(theme, self.variant, self.disabled, self.on_click)
            .name(format!("button:{}", self.label))
            .px(m.pad_x)
            .child(
                text(self.label)
                    .name("label")
                    .font_size(m.font)
                    .text_color(fg)
                    .hit_test(false),
            );
        sem.apply(el)
    }
}

/// A square button showing an icon. The icon is a placeholder slot (a
/// sized, outlined box named `icon:<name>`) until the icon atlas lands.
/// The label is required: it is the accessible name (and the tooltip text
/// callers usually attach).
#[derive(Clone)]
pub struct IconButton {
    icon: SharedString,
    label: SharedString,
    variant: ButtonVariant,
    disabled: bool,
    on_click: Option<ClickFn>,
}

impl std::fmt::Debug for IconButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconButton")
            .field("icon", &self.icon)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl IconButton {
    /// A ghost icon button with icon `icon` (Boxicons name) and accessible
    /// `label`.
    #[must_use]
    pub fn new(icon: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        IconButton {
            icon: icon.into(),
            label: label.into(),
            variant: ButtonVariant::Ghost,
            disabled: false,
            on_click: None,
        }
    }

    /// Set the variant.
    #[must_use]
    pub fn variant(mut self, v: ButtonVariant) -> Self {
        self.variant = v;
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Click handler.
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }

    /// Semantics: `Button` named by the label.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Button)
            .label(&*self.label)
            .interactive(
                self.disabled,
                &[SemanticAction::Click, SemanticAction::Focus],
            )
    }

    /// Build the element (name `icon-button:<label>`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let m = metrics(theme);
        let (.., fg) = palette(theme, self.variant);
        let sem = self.semantics();
        let el = chrome(theme, self.variant, self.disabled, self.on_click)
            .name(format!("icon-button:{}", self.label))
            .w(m.height)
            .child(
                div()
                    .name(format!("icon:{}", self.icon))
                    .size(m.icon, m.icon)
                    .rounded(theme.tokens.radii.xs)
                    .border(1.5, fg.with_alpha(fg.alpha * 0.8))
                    .hit_test(false),
            );
        sem.apply(el)
    }
}
