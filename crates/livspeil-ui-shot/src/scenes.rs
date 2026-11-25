// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Named scenes. The renderer scenes use local token constants (from the
//! UI spec, section 7); the UI scenes ([`crate::ui`]) are real core views
//! painted through `livspeil-ui-paint` with the theme's tokens.

use livspeil_ui_render::{
    Affine, AtlasUpdate, Backdrop, Border, BorderWidth, Color, CornerStyle, DisplayList, Fill,
    GradientStop, Highlight, HighlightKind, Quad, Radii, Rect, Shadow,
};

/// What a scene builds: a display list and the glyph atlas uploads it
/// needs (empty for scenes without text).
#[derive(Debug, Clone, Default)]
pub struct Shot {
    /// The display list.
    pub list: DisplayList,
    /// Atlas uploads to apply before rendering.
    pub atlas: Vec<AtlasUpdate>,
}

impl From<DisplayList> for Shot {
    fn from(list: DisplayList) -> Self {
        Shot {
            list,
            atlas: Vec::new(),
        }
    }
}

/// A named scene: a display list at a fixed logical size and scale.
#[derive(Debug, Clone, Copy)]
pub struct Scene {
    /// Name (file stem of its screenshot and golden).
    pub name: &'static str,
    /// What it shows.
    pub description: &'static str,
    /// Logical width.
    pub width: u32,
    /// Logical height.
    pub height: u32,
    /// Physical pixels per logical pixel.
    pub scale: f32,
    /// Builds the display list for the scene's scale (text is rasterized
    /// for it).
    pub build: fn(f32) -> Result<Shot, crate::ShotError>,
    /// Clear color.
    pub clear: fn() -> Color,
}

/// Radius tokens.
pub mod radius {
    /// xs
    pub const XS: f32 = 4.0;
    /// sm
    pub const SM: f32 = 6.0;
    /// md
    pub const MD: f32 = 10.0;
    /// lg
    pub const LG: f32 = 14.0;
}

/// `surface.0..3` (dark theme): OKLCH lightness 0.14, 0.17, 0.20, 0.24
/// with a faint cool hue.
pub fn surface(level: usize) -> Color {
    let l = [0.14, 0.17, 0.20, 0.24];
    Color::oklch(l[level.min(3)], 0.008, 265.0)
}

fn shadow(y: f32, blur: f32, alpha: f32) -> Shadow {
    Shadow {
        offset: [0.0, y],
        blur,
        spread: 0.0,
        color: Color::BLACK.with_alpha(alpha),
    }
}

/// Elevation tokens: ambient layer + contact layer. Dark themes need
/// fairly opaque shadows to read at all.
pub fn elevation(level: u8) -> Vec<Shadow> {
    match level {
        0 => Vec::new(),
        1 => vec![shadow(1.0, 2.0, 0.40)],
        2 => vec![shadow(4.0, 12.0, 0.36), shadow(1.0, 2.0, 0.30)],
        3 => vec![shadow(12.0, 32.0, 0.42), shadow(2.0, 6.0, 0.30)],
        _ => vec![shadow(24.0, 64.0, 0.50), shadow(4.0, 12.0, 0.30)],
    }
}

/// The Solid material's hairline: white at 6%.
pub fn hairline() -> Border {
    Border::hairline(Color::WHITE.with_alpha(0.06))
}

fn stop(offset: f32, color: Color) -> GradientStop {
    GradientStop { offset, color }
}

fn card_e2() -> DisplayList {
    let mut list = DisplayList::new();
    list.add(
        Quad::new(Rect::new(40.0, 36.0, 240.0, 128.0), surface(2))
            .radius(radius::LG)
            .border(hairline())
            .shadows(&elevation(2)),
    );
    // A smaller card on top, e1, to compare elevations.
    list.add(
        Quad::new(Rect::new(64.0, 60.0, 96.0, 36.0), surface(3))
            .radius(radius::SM)
            .border(hairline())
            .shadows(&elevation(1)),
    );
    list
}

/// Colorful background shared by the material scenes.
fn colorful_background(list: &mut DisplayList, w: f32, h: f32) {
    list.add(
        Quad::new(Rect::new(0.0, 0.0, w, h), Color::BLACK).fill(Fill::Linear {
            start: [0.0, 0.0],
            end: [1.0, 1.0],
            stops: vec![
                stop(0.0, Color::oklch(0.45, 0.17, 285.0)),
                stop(0.4, Color::oklch(0.62, 0.20, 340.0)),
                stop(0.75, Color::oklch(0.78, 0.16, 60.0)),
                stop(1.0, Color::oklch(0.70, 0.14, 160.0)),
            ],
        }),
    );
    let blobs = [
        (90.0, 80.0, 70.0, Color::oklch(0.85, 0.17, 95.0)),
        (330.0, 220.0, 90.0, Color::oklch(0.55, 0.20, 250.0)),
        (230.0, 70.0, 50.0, Color::oklch(0.70, 0.22, 25.0)),
    ];
    for (cx, cy, r, c) in blobs {
        list.add(
            Quad::new(Rect::new(cx - r, cy - r, 2.0 * r, 2.0 * r), c)
                .radius(r)
                .fill(Fill::Radial {
                    center: [0.5, 0.5],
                    radius: [0.5, 0.5],
                    stops: vec![
                        stop(0.0, c),
                        stop(0.6, c.with_alpha(0.7)),
                        stop(1.0, c.with_alpha(0.0)),
                    ],
                }),
        );
    }
    // Sharp stripes, to make the blur obvious.
    for i in 0..10 {
        let x = 20.0 + i as f32 * 44.0;
        list.add(Quad::new(
            Rect::new(x, 0.0, 6.0, h),
            Color::WHITE.with_alpha(0.55),
        ));
    }
}

fn frost_panel() -> DisplayList {
    let mut list = DisplayList::new();
    colorful_background(&mut list, 480.0, 320.0);
    // Frost (T2 recipe): blur 24, tint surface.1 at 0.72, saturate x1.3,
    // inner highlight white 8%, e2.
    list.add(Backdrop {
        radii: Radii::all(radius::LG),
        saturation: 1.3,
        highlight: Some(Highlight {
            width: 1.0,
            color: Color::WHITE.with_alpha(0.08),
            kind: HighlightKind::Full,
        }),
        shadows: elevation(2),
        grain: 0.012,
        ..Backdrop::new(
            Rect::new(60.0, 60.0, 240.0, 200.0),
            24.0,
            surface(1).with_alpha(0.72),
        )
    });
    // Glass (tooltip/menu recipe): blur 16, tint 0.5, top rim 12%, e3.
    list.add(Backdrop {
        radii: Radii::all(radius::MD),
        saturation: 1.3,
        highlight: Some(Highlight {
            width: 1.0,
            color: Color::WHITE.with_alpha(0.12),
            kind: HighlightKind::TopRim,
        }),
        shadows: elevation(3),
        grain: 0.012,
        ..Backdrop::new(
            Rect::new(320.0, 40.0, 130.0, 90.0),
            16.0,
            surface(1).with_alpha(0.5),
        )
    });
    // Content on the Frost panel.
    for i in 0..3 {
        list.add(
            Quad::new(
                Rect::new(80.0, 84.0 + i as f32 * 40.0, 200.0, 28.0),
                Color::WHITE.with_alpha(0.06),
            )
            .radius(radius::SM)
            .border(Border::hairline(Color::WHITE.with_alpha(0.08))),
        );
    }
    list
}

fn radii_row() -> DisplayList {
    let mut list = DisplayList::new();
    let radii = [radius::XS, radius::SM, radius::MD, radius::LG];
    for (row, style) in [CornerStyle::Continuous, CornerStyle::Circular]
        .into_iter()
        .enumerate()
    {
        for (i, r) in radii.iter().enumerate() {
            let x = 16.0 + i as f32 * 64.0;
            let y = 16.0 + row as f32 * 64.0;
            let mut q = Quad::new(Rect::new(x, y, 48.0, 48.0), surface(3))
                .radius(*r)
                .border(Border::hairline(Color::WHITE.with_alpha(0.10)));
            q.corners = style;
            list.add(q);
        }
        // A pill: continuous corners must degrade to a semicircle.
        let mut pill = Quad::new(
            Rect::new(272.0, 28.0 + row as f32 * 64.0, 72.0, 24.0),
            Color::oklch(0.62, 0.17, 255.0),
        )
        .radius(12.0);
        pill.corners = style;
        list.add(pill);
    }
    list
}

fn hairlines() -> DisplayList {
    let mut list = DisplayList::new();
    // Hairlines (one physical pixel) at integer and fractional positions.
    for (i, x) in [12.0, 52.5, 92.25].into_iter().enumerate() {
        list.add(
            Quad::new(Rect::new(x, 12.0, 32.0, 32.0), surface(2))
                .radius(if i == 0 { 0.0 } else { radius::SM })
                .border(Border::hairline(Color::WHITE.with_alpha(0.5))),
        );
    }
    // One logical pixel for comparison (two physical pixels at 2x).
    list.add(
        Quad::new(Rect::new(132.0, 12.0, 32.0, 32.0), surface(2))
            .radius(radius::SM)
            .border(Border {
                width: BorderWidth::Logical(1.0),
                color: Color::WHITE.with_alpha(0.5),
            }),
    );
    // A border-only quad and a rotated one.
    list.add(Quad {
        fill: None,
        ..Quad::new(Rect::new(12.0, 56.0, 72.0, 24.0), Color::BLACK)
            .radius(12.0)
            .border(Border::hairline(Color::oklch(0.8, 0.12, 200.0)))
    });
    list.add(Quad {
        transform: Affine::rotate_about(0.3, 124.0, 68.0),
        ..Quad::new(Rect::new(100.0, 56.0, 48.0, 24.0), surface(3))
            .radius(radius::XS)
            .border(Border::hairline(Color::WHITE.with_alpha(0.5)))
    });
    list
}

fn gradients() -> DisplayList {
    let mut list = DisplayList::new();
    // A long, dark, low-contrast gradient: bands without dithering.
    list.add(
        Quad::new(Rect::new(8.0, 8.0, 304.0, 40.0), Color::BLACK)
            .radius(radius::MD)
            .fill(Fill::Linear {
                start: [0.0, 0.0],
                end: [1.0, 0.0],
                stops: vec![stop(0.0, surface(0)), stop(1.0, surface(3))],
            }),
    );
    // OKLab interpolation between complementary hues stays vivid-ish (no
    // grey midpoint as in sRGB).
    list.add(
        Quad::new(Rect::new(8.0, 56.0, 304.0, 40.0), Color::BLACK)
            .radius(radius::MD)
            .fill(Fill::Linear {
                start: [0.0, 0.0],
                end: [1.0, 0.0],
                stops: vec![
                    stop(0.0, Color::oklch(0.7, 0.15, 250.0)),
                    stop(1.0, Color::oklch(0.8, 0.15, 80.0)),
                ],
            }),
    );
    list.add(
        Quad::new(Rect::new(8.0, 104.0, 148.0, 72.0), Color::BLACK)
            .radius(radius::LG)
            .fill(Fill::Radial {
                center: [0.3, 0.3],
                radius: [0.9, 1.2],
                stops: vec![
                    stop(0.0, Color::oklch(0.9, 0.08, 150.0)),
                    stop(0.5, Color::oklch(0.6, 0.15, 200.0)),
                    stop(1.0, Color::oklch(0.3, 0.08, 270.0)),
                ],
            }),
    );
    // Clip and opacity.
    list.add(Quad {
        clip: Some(Rect::new(164.0, 104.0, 100.0, 72.0)),
        opacity: 0.6,
        ..Quad::new(
            Rect::new(164.0, 104.0, 148.0, 72.0),
            Color::oklch(0.7, 0.18, 30.0),
        )
        .radius(36.0)
    });
    list
}

fn dark() -> Color {
    surface(0)
}

/// All scenes, in a stable order.
pub fn scenes() -> Vec<Scene> {
    let mut v = vec![
        Scene {
            name: "card-e2",
            description: "a Solid card with e2 shadow and hairline, an e1 chip on it",
            width: 320,
            height: 200,
            scale: 2.0,
            build: |_| Ok(card_e2().into()),
            clear: dark,
        },
        Scene {
            name: "frost-panel",
            description: "a Frost panel and a Glass tooltip over a colorful gradient",
            width: 480,
            height: 320,
            scale: 1.0,
            build: |_| Ok(frost_panel().into()),
            clear: dark,
        },
        Scene {
            name: "radii-row",
            description: "radii xs, sm, md, lg and a pill; continuous (top) vs circular (bottom)",
            width: 360,
            height: 144,
            scale: 2.0,
            build: |_| Ok(radii_row().into()),
            clear: dark,
        },
        Scene {
            name: "hairlines-1x",
            description: "hairline borders at 1x; one logical-pixel border for comparison",
            width: 176,
            height: 92,
            scale: 1.0,
            build: |_| Ok(hairlines().into()),
            clear: dark,
        },
        Scene {
            name: "hairlines-2x",
            description: "the same at 2x: hairlines stay one physical pixel",
            width: 176,
            height: 92,
            scale: 2.0,
            build: |_| Ok(hairlines().into()),
            clear: dark,
        },
        Scene {
            name: "gradients",
            description: "dithered OKLab gradients (linear, radial), clip and opacity",
            width: 320,
            height: 184,
            scale: 2.0,
            build: |_| Ok(gradients().into()),
            clear: dark,
        },
    ];
    v.extend(crate::ui::scenes());
    v.extend(crate::containers::scenes());
    v
}

/// The scene with this name.
pub fn find(name: &str) -> Option<Scene> {
    scenes().into_iter().find(|s| s.name == name)
}
