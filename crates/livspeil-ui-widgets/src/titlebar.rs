// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Titlebar`]: a macOS-style titlebar for windows whose content extends
//! under a hidden or transparent native titlebar.

use std::fmt;

use livspeil_ui_core::{Element, SharedString, div, text};
use livspeil_ui_layout::style::{LengthPercentageAuto, Position};
use livspeil_ui_theme::{Material, Theme};

use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};

/// Height of the titlebar: the macOS titlebar area a full-size-content
/// window leaves free (`livspeil-ui-host-winit`'s `Titlebar::MACOS_INSET`).
pub const TITLEBAR_HEIGHT: f32 = 28.0;
/// Height with toolbar items (a compact unified toolbar).
pub const TOOLBAR_HEIGHT: f32 = 38.0;
/// Width kept free on the left for the traffic lights (close, minimize,
/// zoom: three 12 px buttons 8 px apart, 20 px from the edge, plus a gap).
pub const TRAFFIC_LIGHT_INSET: f32 = 78.0;

/// A titlebar: the traffic-light inset, optional leading and trailing
/// toolbar items and the title centered in the window.
///
/// The whole bar is a window drag region
/// ([`Element::window_drag_region`]): pressing empty space (or the title)
/// requests a window move, a double press a zoom; items that handle the
/// pointer (buttons) do not.
pub struct Titlebar {
    title: SharedString,
    leading: Vec<Element>,
    trailing: Vec<Element>,
    inset: f32,
    material: Option<Material>,
}

impl fmt::Debug for Titlebar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Titlebar")
            .field("title", &self.title)
            .field("inset", &self.inset)
            .finish_non_exhaustive()
    }
}

impl Titlebar {
    /// A titlebar with no surface (the content shows through).
    #[must_use]
    pub fn new(title: impl Into<SharedString>) -> Self {
        Titlebar {
            title: title.into(),
            leading: Vec::new(),
            trailing: Vec::new(),
            inset: TRAFFIC_LIGHT_INSET,
            material: None,
        }
    }

    /// Item after the traffic lights (sidebar toggle, back/forward).
    #[must_use]
    pub fn leading(mut self, el: Element) -> Self {
        self.leading.push(el);
        self
    }

    /// Item at the right end (share, search, menus).
    #[must_use]
    pub fn trailing(mut self, el: Element) -> Self {
        self.trailing.push(el);
        self
    }

    /// Left inset (default [`TRAFFIC_LIGHT_INSET`]; 0 where the platform
    /// draws no traffic lights).
    #[must_use]
    pub fn inset(mut self, px: f32) -> Self {
        self.inset = px;
        self
    }

    /// Put the bar on a Frost surface.
    #[must_use]
    pub fn frost(mut self) -> Self {
        self.material = Some(Material::Frost);
        self
    }

    /// The bar's height: [`TOOLBAR_HEIGHT`] with items, else
    /// [`TITLEBAR_HEIGHT`].
    #[must_use]
    pub fn height(&self) -> f32 {
        if self.leading.is_empty() && self.trailing.is_empty() {
            TITLEBAR_HEIGHT
        } else {
            TOOLBAR_HEIGHT
        }
    }

    /// Semantics: a `Titlebar` (toolbar) named by the title.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Titlebar).label(&*self.title)
    }

    /// Build the element (full width).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let m = metrics(theme);
        let c = theme.colors();
        let h = self.height();
        let sem = self.semantics();
        let title = div()
            .key("title")
            .hit_test(false)
            .flex_row()
            .items_center()
            .justify_center()
            .style(|s| {
                s.position = Position::Absolute;
                let z = LengthPercentageAuto::length(0.0);
                s.inset.left = z;
                s.inset.right = z;
                s.inset.top = z;
                s.inset.bottom = z;
            })
            .child(
                text(self.title.clone())
                    .font_size(m.font)
                    .font_weight(600.0)
                    .text_color(c.text_secondary)
                    .hit_test(false),
            );
        let mut bar = div()
            .name("titlebar")
            .w_full()
            .h(h)
            .flex_row()
            .items_center()
            .gap(theme.tokens.spacing.step(1))
            .style(|s| s.flex_shrink = 0.0)
            .child(title)
            .child(
                div()
                    .key("traffic-lights")
                    .w(self.inset)
                    .h(h)
                    .hit_test(false),
            )
            .children(self.leading)
            .child(div().key("spacer").grow().hit_test(false))
            .children(self.trailing)
            .child(
                div()
                    .key("end")
                    .w(theme.tokens.spacing.step(1))
                    .hit_test(false),
            )
            .child(
                div()
                    .key("rule")
                    .hit_test(false)
                    .bg(style::hairline(theme))
                    .style(|s| {
                        s.position = Position::Absolute;
                        s.inset.left = LengthPercentageAuto::length(0.0);
                        s.inset.right = LengthPercentageAuto::length(0.0);
                        s.inset.bottom = LengthPercentageAuto::length(0.0);
                        s.size.height = livspeil_ui_layout::style::Dimension::length(1.0);
                    }),
            )
            .window_drag_region();
        if let Some(mat) = self.material {
            bar = bar.material(mat);
        }
        sem.apply(bar)
    }
}
