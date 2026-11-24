// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Shared visual helpers: per-density metrics and per-scheme colors.

use std::rc::Rc;

use livspeil_ui_core::{ClickEvent, Element, EventCx};
use livspeil_ui_theme::{Color, Density, Scheme, Theme};

/// Opacity of disabled widgets.
pub const DISABLED_OPACITY: f32 = 0.4;

/// A click callback.
pub type ClickFn = Rc<dyn Fn(&ClickEvent, &mut EventCx<'_>)>;

/// Sizes that depend on density.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Metrics {
    /// Control height (row height).
    pub(crate) height: f32,
    /// Body text size.
    pub(crate) font: f32,
    /// Horizontal padding of text controls.
    pub(crate) pad_x: f32,
    /// Check box / radio indicator size.
    pub(crate) indicator: f32,
    /// Switch track width and height, thumb size.
    pub(crate) switch: (f32, f32, f32),
    /// Gap between an indicator and its label.
    pub(crate) gap: f32,
    /// Icon size.
    pub(crate) icon: f32,
    /// Corner radius of controls.
    pub(crate) radius: f32,
}

pub(crate) fn metrics(theme: &Theme) -> Metrics {
    let m = theme.metrics();
    let t = &theme.tokens;
    match theme.density {
        Density::Simple => Metrics {
            height: m.row_height,
            font: m.body,
            pad_x: t.spacing.step(3),
            indicator: 16.0,
            switch: (36.0, 20.0, 16.0),
            gap: t.spacing.step(2),
            icon: t.icons.0[1],
            radius: t.radii.sm,
        },
        Density::Pro => Metrics {
            height: m.row_height,
            font: m.body,
            pad_x: t.spacing.step(2),
            indicator: 14.0,
            switch: (28.0, 16.0, 12.0),
            gap: t.spacing.step(1) + 2.0,
            icon: t.icons.0[0],
            radius: t.radii.xs,
        },
    }
}

/// Lightness offset that reads as "more prominent" in the scheme.
pub(crate) fn emphasize(theme: &Theme, c: Color, amount: f32) -> Color {
    let d = match theme.scheme {
        Scheme::Dark => amount,
        Scheme::Light => -amount,
    };
    Color {
        l: (c.l + d).clamp(0.0, 1.0),
        ..c
    }
}

/// Change lightness by `d` regardless of scheme.
pub(crate) fn shade(c: Color, d: f32) -> Color {
    Color {
        l: (c.l + d).clamp(0.0, 1.0),
        ..c
    }
}

/// Hairline border color of controls.
pub(crate) fn hairline(theme: &Theme) -> Color {
    match theme.scheme {
        Scheme::Dark => Color::WHITE.with_alpha(0.06),
        Scheme::Light => Color::BLACK.with_alpha(0.10),
    }
}

/// Border of unchecked indicators (check box, radio).
pub(crate) fn indicator_border(theme: &Theme) -> Color {
    theme.colors().text_tertiary
}

/// Readable text on a filled color.
pub(crate) fn on_color(c: Color) -> Color {
    if c.l > 0.72 {
        Color::oklch(0.16, 0.0, 0.0, 0.92)
    } else {
        Color::WHITE.with_alpha(0.96)
    }
}

/// Selection highlight behind text.
pub(crate) fn selection(theme: &Theme, focused: bool) -> Color {
    if focused {
        theme.accent().with_alpha(0.35)
    } else {
        theme.colors().text_tertiary.with_alpha(0.25)
    }
}

/// Fade a disabled widget.
pub(crate) fn disabled(el: Element, disabled: bool) -> Element {
    if disabled {
        el.opacity(DISABLED_OPACITY)
    } else {
        el
    }
}

/// A stable element name derived from an entity (`kind:EntityId(n)`).
pub(crate) fn entity_name(kind: &str, id: livspeil_ui_core::EntityId) -> String {
    format!("{kind}:{id:?}")
}

/// The next enabled index from `current` in direction `dir` (+1/-1), or
/// `None` if there is none. With `wrap` the search wraps around. Without a
/// current index, forward starts at the first and backward at the last.
pub(crate) fn step_enabled(
    enabled: &[bool],
    current: Option<usize>,
    dir: isize,
    wrap: bool,
) -> Option<usize> {
    let n = enabled.len();
    if n == 0 {
        return None;
    }
    let len = n.cast_signed();
    let mut i = match current {
        Some(c) => c.min(n - 1).cast_signed(),
        None if dir > 0 => -1,
        None => len,
    };
    for _ in 0..n {
        i += dir;
        if wrap {
            i = i.rem_euclid(len);
        } else if i < 0 || i >= len {
            return None;
        }
        let u = i.cast_unsigned();
        if enabled[u] {
            return Some(u);
        }
    }
    None
}

/// First (`dir > 0`) or last enabled index.
pub(crate) fn edge_enabled(enabled: &[bool], dir: isize) -> Option<usize> {
    step_enabled(enabled, None, dir, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_skips_disabled_and_wraps() {
        let e = [true, false, true, true];
        assert_eq!(step_enabled(&e, Some(0), 1, false), Some(2));
        assert_eq!(step_enabled(&e, Some(3), 1, false), None);
        assert_eq!(step_enabled(&e, Some(3), 1, true), Some(0));
        assert_eq!(step_enabled(&e, Some(0), -1, true), Some(3));
        assert_eq!(step_enabled(&e, None, 1, false), Some(0));
        assert_eq!(step_enabled(&e, None, -1, false), Some(3));
        assert_eq!(edge_enabled(&[false, true, false], 1), Some(1));
        assert_eq!(edge_enabled(&[false, false], -1), None);
    }
}
