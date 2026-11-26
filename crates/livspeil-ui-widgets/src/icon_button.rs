// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`IconButton`]: a square button showing a Boxicons icon.

use std::rc::Rc;

use livspeil_ui_core::{ClickEvent, Element, EventCx, SharedString, div};
use livspeil_ui_theme::{MotionClass, Scheme, Theme};

use crate::button::{ButtonVariant, chrome, palette};
use crate::env::Env;
use crate::semantics::{Role, SemanticAction, Semantics, Toggled};
use crate::style::{self, ClickFn, metrics};
use crate::tooltip::Tooltip;

/// Icon sizes of the design system (UI spec §7), in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconSize {
    /// 14 px.
    S14,
    /// 16 px.
    S16,
    /// 20 px.
    S20,
    /// 24 px.
    S24,
}

impl IconSize {
    /// Every size, ascending.
    pub const ALL: [IconSize; 4] = [IconSize::S14, IconSize::S16, IconSize::S20, IconSize::S24];

    /// The icon's edge length.
    #[must_use]
    pub fn px(self) -> f32 {
        match self {
            IconSize::S14 => 14.0,
            IconSize::S16 => 16.0,
            IconSize::S20 => 20.0,
            IconSize::S24 => 24.0,
        }
    }

    /// The button's edge length: the icon plus 5 px on every side.
    #[must_use]
    pub fn button(self) -> f32 {
        self.px() + 10.0
    }
}

/// How an icon button looks and behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum IconButtonStyle {
    /// No surface until hovered (default).
    #[default]
    Plain,
    /// Filled with the accent (or the [`ButtonVariant`] color).
    Filled,
    /// Plain, but holds an on/off state: while on it sits on a raised pill
    /// like the selected segment of a [`crate::SegmentedControl`], and is
    /// exposed to assistive technology as a toggle button.
    Toggle,
}

/// A square button showing an icon.
///
/// The icon slot is a sized, outlined placeholder box named
/// `icon:<name>` (Boxicons name, e.g. `bx-trash`) until the icon atlas
/// lands in the renderer (UI spec §3); sizes are 14, 16, 20 and 24 px
/// ([`IconSize`]). The label is required: it is the accessible name, and
/// with [`IconButton::tooltip`] also the tooltip text.
///
/// Styles ([`IconButtonStyle`]): plain (default), filled, and toggle
/// (`toggled(bool)`; the caller owns the state and flips it in
/// `on_click`).
#[derive(Clone)]
pub struct IconButton {
    icon: SharedString,
    label: SharedString,
    variant: ButtonVariant,
    style: IconButtonStyle,
    size: Option<IconSize>,
    toggled: bool,
    disabled: bool,
    tooltip: Option<Env>,
    on_click: Option<ClickFn>,
}

impl std::fmt::Debug for IconButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IconButton")
            .field("icon", &self.icon)
            .field("label", &self.label)
            .field("style", &self.style)
            .field("size", &self.size)
            .field("toggled", &self.toggled)
            .finish_non_exhaustive()
    }
}

impl IconButton {
    /// A plain icon button with icon `icon` (Boxicons name) and accessible
    /// `label`.
    #[must_use]
    pub fn new(icon: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        IconButton {
            icon: icon.into(),
            label: label.into(),
            variant: ButtonVariant::Ghost,
            style: IconButtonStyle::Plain,
            size: None,
            toggled: false,
            disabled: false,
            tooltip: None,
            on_click: None,
        }
    }

    /// Set the color variant (ghost by default; primary and destructive
    /// are filled).
    #[must_use]
    pub fn variant(mut self, v: ButtonVariant) -> Self {
        self.variant = v;
        self
    }

    /// Set the style.
    #[must_use]
    pub fn style(mut self, s: IconButtonStyle) -> Self {
        self.style = s;
        self
    }

    /// Filled with the accent.
    #[must_use]
    pub fn filled(mut self) -> Self {
        self.style = IconButtonStyle::Filled;
        self
    }

    /// A toggle button in state `on`.
    #[must_use]
    pub fn toggle(mut self, on: bool) -> Self {
        self.style = IconButtonStyle::Toggle;
        self.toggled = on;
        self
    }

    /// Icon size (the button grows with it). Without it the icon size and
    /// the button size follow the theme density.
    #[must_use]
    pub fn size(mut self, size: IconSize) -> Self {
        self.size = Some(size);
        self
    }

    /// Show the label as a [`Tooltip`] (needs [`Env::root`]).
    #[must_use]
    pub fn tooltip(mut self, env: &Env) -> Self {
        self.tooltip = Some(env.clone());
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

    /// Semantics: `Button` named by the label; a toggle also reports its
    /// state.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let s = Semantics::new(Role::Button)
            .label(&*self.label)
            .interactive(
                self.disabled,
                &[SemanticAction::Click, SemanticAction::Focus],
            );
        if self.style == IconButtonStyle::Toggle {
            s.toggled(if self.toggled {
                Toggled::True
            } else {
                Toggled::False
            })
        } else {
            s
        }
    }

    /// Build the element (name `icon-button:<label>`, icon `icon:<name>`).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let m = metrics(theme);
        let c = theme.colors();
        let sem = self.semantics();
        let variant = match self.style {
            IconButtonStyle::Filled if self.variant == ButtonVariant::Ghost => {
                ButtonVariant::Primary
            }
            _ => self.variant,
        };
        let (.., mut fg) = palette(theme, variant);
        let (side, icon) = match self.size {
            Some(s) => (s.button(), s.px()),
            None => (m.height, m.icon),
        };
        let mut el = chrome(theme, variant, self.disabled, self.on_click);
        if self.style == IconButtonStyle::Toggle && !self.disabled {
            // The segmented control's pill: lighter in dark, bright on a
            // darker surface in light.
            let (pill, hover) = match theme.scheme {
                Scheme::Dark => (c.surface[3], style::shade(c.surface[3], 0.03)),
                Scheme::Light => (c.surface[0], c.surface[0]),
            };
            if self.toggled {
                el = el
                    .bg(pill)
                    .border(1.0, style::hairline(theme))
                    .hover(|r| r.bg(hover));
                fg = theme.accent();
            }
        }
        el = el
            .name(format!("icon-button:{}", self.label))
            .size(side, side)
            .rounded(if self.size.is_some() {
                m.radius.min(side / 3.0)
            } else {
                m.radius
            })
            .transition(MotionClass::Micro)
            .child(
                div()
                    .name(format!("icon:{}", self.icon))
                    .size(icon, icon)
                    .rounded(theme.tokens.radii.xs)
                    .border(1.5, fg.with_alpha(fg.alpha * 0.8))
                    .hit_test(false),
            );
        let el = sem.apply(el);
        match self.tooltip {
            Some(env) => Tooltip::new(self.label).attach(&env, el),
            None => el,
        }
    }
}
