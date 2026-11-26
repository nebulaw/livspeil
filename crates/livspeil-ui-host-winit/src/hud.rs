// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The frame HUD: options and the paint commands of its overlay.
//!
//! The host appends the overlay to the scene it renders (after the app's
//! own commands, so it is on top) and removes it again afterwards; the
//! app never sees it. See [`crate::frame_stats`] for what it shows.

use std::path::PathBuf;

use livspeil_ui_core::{Border, ElementId, Fill, FontFeature, FontSpec, PaintCmd, Quad, TextRun};
use livspeil_ui_layout::{Corners, Rect, Size};
use livspeil_ui_theme::Theme;

/// Hotkey: show or hide the HUD.
pub const HUD_KEY: &str = "F3";
/// Hotkey: write a summary and the per-frame CSV to the frame log.
pub const DUMP_KEY: &str = "F4";

/// Frame HUD options (see [`crate::HostOptions::frame_hud`]).
#[derive(Clone, Debug, PartialEq)]
pub struct HudOptions {
    /// Start with the HUD visible.
    pub visible: bool,
    /// Frame log file; `None` for no log (the HUD still works).
    pub log_path: Option<PathBuf>,
    /// Seconds of per-frame CSV a dump writes.
    pub dump_seconds: f64,
}

impl Default for HudOptions {
    fn default() -> Self {
        HudOptions {
            visible: false,
            log_path: None,
            dump_seconds: 10.0,
        }
    }
}

/// Characters per HUD line before it is cut with an ellipsis.
pub const MAX_LINE_CHARS: usize = 64;

const FONT_SIZE: f32 = 11.5;
const LINE_HEIGHT: f32 = 15.0;
const PAD: f32 = 8.0;
const MARGIN: f32 = 10.0;
/// Panel width in logical pixels (lines are at most [`MAX_LINE_CHARS`]).
const WIDTH: f32 = 400.0;

/// Cut `line` to [`MAX_LINE_CHARS`] characters.
#[must_use]
pub fn clip_line(line: &str) -> String {
    if line.chars().count() <= MAX_LINE_CHARS {
        return line.to_owned();
    }
    let mut s: String = line.chars().take(MAX_LINE_CHARS - 1).collect();
    s.push('\u{2026}');
    s
}

/// The paint commands of the HUD panel with `lines`, in the bottom-right
/// corner of a `viewport`.
#[must_use]
pub fn hud_commands(lines: &[String], viewport: Size, theme: &Theme) -> Vec<PaintCmd> {
    let colors = theme.colors();
    #[allow(clippy::cast_precision_loss)]
    let height = lines.len() as f32 * LINE_HEIGHT + 2.0 * PAD;
    let x = (viewport.width - WIDTH - MARGIN).max(MARGIN);
    let y = (viewport.height - height - MARGIN).max(MARGIN);
    let mut out = Vec::with_capacity(lines.len() + 1);
    out.push(PaintCmd::Quad(Quad {
        element: ElementId::ROOT,
        rect: Rect::new(x, y, WIDTH, height),
        radii: Corners::all(8.0),
        fill: Some(Fill::Solid(colors.surface[0].with_alpha(0.88))),
        border: Some(Border {
            width: 1.0,
            color: colors.text_secondary.with_alpha(0.3),
        }),
        shadow: None,
        material: None,
    }));
    let font = FontSpec {
        features: vec![FontFeature::TNUM],
        ..FontSpec::sized(FONT_SIZE)
    };
    for (i, line) in lines.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let top = y + PAD + i as f32 * LINE_HEIGHT;
        out.push(PaintCmd::Text(TextRun {
            element: ElementId::ROOT,
            rect: Rect::new(x + PAD, top, 0.0, LINE_HEIGHT),
            text: clip_line(line).into(),
            font: font.clone(),
            color: colors.text_primary,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_are_clipped() {
        assert_eq!(clip_line("short"), "short");
        let long = "x".repeat(200);
        let c = clip_line(&long);
        assert_eq!(c.chars().count(), MAX_LINE_CHARS);
        assert!(c.ends_with('\u{2026}'));
    }

    #[test]
    fn panel_sits_in_the_bottom_right_corner() {
        let lines = vec!["a".to_owned(), "b".to_owned()];
        let cmds = hud_commands(&lines, Size::new(1000.0, 700.0), &Theme::default());
        assert_eq!(cmds.len(), 3);
        let PaintCmd::Quad(q) = &cmds[0] else {
            panic!("the panel comes first");
        };
        assert!((q.rect.right() - 990.0).abs() < 1e-3);
        assert!((q.rect.bottom() - 690.0).abs() < 1e-3);
        let PaintCmd::Text(t) = &cmds[1] else {
            panic!("then the lines");
        };
        assert!(t.rect.y > q.rect.y && t.rect.x > q.rect.x);
        // A viewport smaller than the panel keeps it on screen.
        let small = hud_commands(&lines, Size::new(100.0, 20.0), &Theme::default());
        let PaintCmd::Quad(q) = &small[0] else {
            panic!("the panel comes first");
        };
        assert!(q.rect.x >= 0.0 && q.rect.y >= 0.0);
    }
}
