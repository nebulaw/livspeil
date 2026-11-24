// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! `Scene` (core paint commands) to `DisplayList` (render primitives).

use livspeil_ui_core::{Border, Fill, PaintCmd, Quad, Scene, TextRun};
use livspeil_ui_layout::{Corners, Rect as LRect, Transform2D};
use livspeil_ui_render::{
    Affine, Backdrop, BorderWidth, Color, CornerStyle, DisplayList, Fill as RFill, GradientStop,
    Highlight, HighlightKind, Quad as RQuad, Radii, Rect, Shadow as RShadow,
};
use livspeil_ui_text::{TextError, TextStyle};
use livspeil_ui_theme::{MaterialRecipe, Shadow, ShadowLayer};

use crate::PaintError;
use crate::text::{Placement, TextContext};

/// Grain on blurred materials (breaks banding on large blurs).
const MATERIAL_GRAIN: f32 = 0.012;

pub(crate) fn color(c: livspeil_ui_theme::Color) -> Color {
    Color {
        l: c.l,
        a: c.a,
        b: c.b,
        alpha: c.alpha,
    }
}

fn rect(r: LRect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

fn radii(c: Corners) -> Radii {
    Radii {
        tl: c.top_left,
        tr: c.top_right,
        br: c.bottom_right,
        bl: c.bottom_left,
    }
}

fn affine(t: Transform2D) -> Affine {
    Affine {
        m: [t.a, t.b, t.c, t.d, t.e, t.f],
    }
}

fn shadow_layer(l: ShadowLayer) -> RShadow {
    RShadow {
        offset: [l.x, l.y],
        blur: l.blur,
        spread: l.spread,
        color: Color::BLACK.with_alpha(l.alpha),
    }
}

/// Two-layer elevation shadow: key layer below, contact layer above.
pub(crate) fn shadows(s: Option<Shadow>) -> Vec<RShadow> {
    let Some(s) = s else { return Vec::new() };
    let mut v = vec![shadow_layer(s.key)];
    v.extend(s.contact.map(shadow_layer));
    v
}

/// The core's borders are inner borders; a width of one logical pixel or
/// less is the design system's hairline and stays one physical pixel at
/// every scale.
fn border_width(w: f32) -> BorderWidth {
    if w <= 1.0 {
        BorderWidth::Hairline
    } else {
        BorderWidth::Logical(w)
    }
}

fn fill(f: Fill, r: LRect) -> RFill {
    match f {
        Fill::Solid(c) => RFill::Solid(color(c)),
        Fill::LinearGradient {
            start,
            end,
            from,
            to,
        } => {
            let unit = |x: f32, o: f32, len: f32| if len > 0.0 { (x - o) / len } else { 0.0 };
            RFill::Linear {
                start: [unit(start.x, r.x, r.width), unit(start.y, r.y, r.height)],
                end: [unit(end.x, r.x, r.width), unit(end.y, r.y, r.height)],
                stops: vec![
                    GradientStop {
                        offset: 0.0,
                        color: color(from),
                    },
                    GradientStop {
                        offset: 1.0,
                        color: color(to),
                    },
                ],
            }
        }
    }
}

/// Current transform, opacity and clip while walking a scene.
#[derive(Debug, Clone, Copy)]
pub(crate) struct State {
    pub transform: Affine,
    pub opacity: f32,
    pub clip: Option<Rect>,
}

impl Default for State {
    fn default() -> Self {
        State {
            transform: Affine::IDENTITY,
            opacity: 1.0,
            clip: None,
        }
    }
}

fn intersect(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        None => b,
        Some(a) => {
            let x0 = a.x.max(b.x);
            let y0 = a.y.max(b.y);
            let x1 = a.right().min(b.right());
            let y1 = a.bottom().min(b.bottom());
            Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
        }
    }
}

/// Converts `scene` into `list` at `scale` physical pixels per logical
/// pixel (glyphs are rasterized for it). Items go on `layer`.
pub(crate) fn convert(
    scene: &Scene,
    text: &mut TextContext,
    scale: f32,
    layer: i32,
    list: &mut DisplayList,
) -> Result<(), PaintError> {
    let mut stack: Vec<(Kind, State)> = Vec::new();
    let mut st = State::default();
    for cmd in &scene.commands {
        match cmd {
            PaintCmd::PushTransform(t) => {
                stack.push((Kind::Transform, st));
                st.transform = st.transform.then_after(affine(*t));
            }
            PaintCmd::PushOpacity(o) => {
                stack.push((Kind::Opacity, st));
                st.opacity *= o.clamp(0.0, 1.0);
            }
            PaintCmd::PushClip { rect: r, .. } => {
                // Rounded clips are clipped to their bounding rect for now.
                stack.push((Kind::Clip, st));
                st.clip = Some(intersect(st.clip, st.transform.bounds(rect(*r))));
            }
            PaintCmd::PopTransform | PaintCmd::PopOpacity | PaintCmd::PopClip => {
                let want = match cmd {
                    PaintCmd::PopTransform => Kind::Transform,
                    PaintCmd::PopOpacity => Kind::Opacity,
                    _ => Kind::Clip,
                };
                match stack.pop() {
                    Some((k, prev)) if k == want => st = prev,
                    _ => return Err(PaintError::UnbalancedStack),
                }
            }
            PaintCmd::Quad(q) => quad(q, &st, layer, list),
            PaintCmd::Text(t) => text_run(t, &st, text, scale, layer, list)?,
        }
    }
    if stack.is_empty() {
        Ok(())
    } else {
        Err(PaintError::UnbalancedStack)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Transform,
    Opacity,
    Clip,
}

fn quad(q: &Quad, st: &State, layer: i32, list: &mut DisplayList) {
    let r = rect(q.rect);
    let shadows = shadows(q.shadow.map(|(_, s)| s));
    let border = q
        .border
        .map(|Border { width, color: c }| livspeil_ui_render::Border {
            width: border_width(width),
            color: color(c),
        });
    if let Some((_, recipe)) = q.material
        && recipe.blur > 0.0
    {
        list.push(layer, backdrop(r, q.radii, &recipe, shadows, st));
        if let Some(b) = border {
            list.push(
                layer,
                RQuad {
                    fill: None,
                    border: Some(b),
                    clip: st.clip,
                    opacity: st.opacity,
                    transform: st.transform,
                    radii: radii(q.radii),
                    ..RQuad::new(r, Color::TRANSPARENT)
                },
            );
        }
        return;
    }
    // Solid, Ink and opaque (T0 / reduce transparency) materials: the core
    // already put the opaque tint in the fill.
    let fill = q.fill.map(|f| fill(f, q.rect));
    if fill.is_none() && border.is_none() && shadows.is_empty() {
        return;
    }
    list.push(
        layer,
        RQuad {
            rect: r,
            radii: radii(q.radii),
            corners: CornerStyle::Continuous,
            fill,
            border,
            shadows,
            clip: st.clip,
            opacity: st.opacity,
            transform: st.transform,
        },
    );
}

fn backdrop(
    r: Rect,
    c: Corners,
    recipe: &MaterialRecipe,
    shadows: Vec<RShadow>,
    st: &State,
) -> Backdrop {
    let highlight = if recipe.rim > 0.0 {
        Some(Highlight {
            width: 1.0,
            color: Color::WHITE.with_alpha(recipe.rim),
            kind: HighlightKind::TopRim,
        })
    } else if recipe.highlight > 0.0 {
        Some(Highlight {
            width: 1.0,
            color: Color::WHITE.with_alpha(recipe.highlight),
            kind: HighlightKind::Full,
        })
    } else {
        None
    };
    Backdrop {
        rect: r,
        radii: radii(c),
        corners: CornerStyle::Continuous,
        blur: recipe.blur,
        saturation: recipe.saturate,
        tint: color(recipe.tint),
        highlight,
        shadows,
        grain: MATERIAL_GRAIN,
        clip: st.clip,
        opacity: st.opacity,
        transform: st.transform,
    }
}

/// Physical pixels per local pixel under a transform.
fn local_scale(scale: f32, t: Affine) -> f32 {
    let s = scale * t.det().abs().sqrt();
    if s.is_finite() && s > 0.0 { s } else { scale }
}

fn text_run(
    t: &TextRun,
    st: &State,
    text: &mut TextContext,
    scale: f32,
    layer: i32,
    list: &mut DisplayList,
) -> Result<(), PaintError> {
    let style = TextStyle::sized(t.size);
    // Labels are laid out at their measured width; only wrap when the box
    // is narrower than the text (a paragraph given a width by its view).
    let natural = text.natural_width(&t.text, &style);
    let max_width = (t.rect.width > 0.0 && natural > t.rect.width + 0.5).then_some(t.rect.width);
    draw_text(
        text,
        &t.text,
        &style,
        TextDraw {
            origin: (t.rect.x, t.rect.y),
            max_width,
            color: color(t.color),
            scale,
            layer,
        },
        st,
        list,
    )
}

/// Inputs of [`draw_text`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct TextDraw {
    pub origin: (f32, f32),
    pub max_width: Option<f32>,
    pub color: Color,
    pub scale: f32,
    pub layer: i32,
}

pub(crate) fn draw_text(
    text: &mut TextContext,
    s: &str,
    style: &TextStyle,
    d: TextDraw,
    st: &State,
    list: &mut DisplayList,
) -> Result<(), PaintError> {
    if s.trim().is_empty() || d.color.alpha <= 0.0 || st.opacity <= 0.0 {
        return Ok(());
    }
    // Rasterize for the transform's scale (zoomed canvases stay crisp),
    // except under rotation or skew where the sprite is resampled anyway.
    let [a, b, c, dd, _, _] = st.transform.m;
    let axis_aligned = b == 0.0 && c == 0.0 && a > 0.0 && dd > 0.0 && (a - dd).abs() < 1e-6;
    let raster_scale = if axis_aligned {
        local_scale(d.scale, st.transform)
    } else {
        d.scale
    };
    let glyphs = text
        .glyphs(
            s,
            style,
            Placement {
                origin: d.origin,
                max_width: d.max_width,
                scale: raster_scale,
            },
        )
        .map_err(PaintError::Text)?;
    if glyphs.is_empty() {
        return Ok(());
    }
    list.push(
        d.layer,
        livspeil_ui_render::Glyphs {
            atlas: text.atlas_id(),
            glyphs,
            color: d.color,
            clip: st.clip,
            opacity: st.opacity,
            transform: st.transform,
        },
    );
    Ok(())
}

/// Whether an error means "clear the atlas and paint again".
pub(crate) fn is_atlas_full(e: &PaintError) -> bool {
    matches!(e, PaintError::Text(TextError::AtlasFull))
}
