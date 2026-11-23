// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The paint boundary: a renderer-agnostic list of primitives.
//!
//! `livspeil-ui-render` implements [`Painter`] and turns a [`Scene`] into
//! GPU work. Coordinates are logical pixels in window space before the
//! enclosing `PushTransform`s are applied.

use std::fmt::Write as _;

use livspeil_ui_layout::geometry::num;
use livspeil_ui_layout::{Corners, Rect, Transform2D, Vec2};
use livspeil_ui_theme::{Color, Elevation, Material, MaterialRecipe, Shadow, Theme};

use crate::element::{ElementId, SharedString};
use crate::tree::{Tree, canvas_visible};

/// A fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fill {
    /// Solid color.
    Solid(Color),
    /// Linear gradient between two points (interpolated in OKLab).
    LinearGradient {
        /// Start point.
        start: Vec2,
        /// End point.
        end: Vec2,
        /// Color at start.
        from: Color,
        /// Color at end.
        to: Color,
    },
}

/// An inner border (hairline when width is 1 physical pixel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    /// Width in logical pixels.
    pub width: f32,
    /// Color.
    pub color: Color,
}

/// A rounded rectangle (continuous corners).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    /// The element that painted it.
    pub element: ElementId,
    /// Bounds.
    pub rect: Rect,
    /// Corner radii.
    pub radii: Corners,
    /// Fill (`None`: no fill, for example a focus ring).
    pub fill: Option<Fill>,
    /// Inner border.
    pub border: Option<Border>,
    /// Drop shadow (two layers).
    pub shadow: Option<(Elevation, Shadow)>,
    /// Backdrop material recipe; the fill is the material's tint.
    pub material: Option<(Material, MaterialRecipe)>,
}

/// A text run. Placeholder until `livspeil-ui-text` provides shaped glyphs.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// The element that painted it.
    pub element: ElementId,
    /// Layout box.
    pub rect: Rect,
    /// Text.
    pub text: SharedString,
    /// Font size.
    pub size: f32,
    /// Color.
    pub color: Color,
}

/// One paint command. Push/Pop pairs nest.
#[derive(Clone, Debug, PartialEq)]
pub enum PaintCmd {
    /// Multiply the current transform by this one (applied first).
    PushTransform(Transform2D),
    /// Undo the last `PushTransform`.
    PopTransform,
    /// Multiply opacity for subsequent commands.
    PushOpacity(f32),
    /// Undo the last `PushOpacity`.
    PopOpacity,
    /// Intersect the clip with a rounded rect.
    PushClip {
        /// Clip rect.
        rect: Rect,
        /// Radii.
        radii: Corners,
    },
    /// Undo the last `PushClip`.
    PopClip,
    /// Draw a quad.
    Quad(Quad),
    /// Draw text.
    Text(TextRun),
}

/// A frame's display list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Commands in paint order.
    pub commands: Vec<PaintCmd>,
}

/// Implemented by renderers.
pub trait Painter {
    /// Draw a scene.
    fn paint(&mut self, scene: &Scene);
}

/// A painter that keeps the last scene (tests, screenshots).
#[derive(Debug, Default)]
pub struct RecordingPainter {
    /// Scenes painted so far.
    pub scenes: Vec<Scene>,
}

impl Painter for RecordingPainter {
    fn paint(&mut self, scene: &Scene) {
        self.scenes.push(scene.clone());
    }
}

fn color(c: Color) -> String {
    let [l, ch, h] = c.to_oklch();
    format!(
        "oklch({} {} {} / {})",
        num(l),
        num(ch),
        num(h.round()),
        num(c.alpha)
    )
}

impl Scene {
    /// A readable dump for golden tests; `name` maps element ids to labels.
    pub fn dump(&self, name: impl Fn(ElementId) -> String) -> String {
        let mut out = String::new();
        let mut depth = 0usize;
        for cmd in &self.commands {
            if matches!(
                cmd,
                PaintCmd::PopTransform | PaintCmd::PopOpacity | PaintCmd::PopClip
            ) {
                depth = depth.saturating_sub(1);
            }
            out.push_str(&"  ".repeat(depth));
            match cmd {
                PaintCmd::PushTransform(t) => {
                    let _ = write!(out, "transform {t}");
                    depth += 1;
                }
                PaintCmd::PushOpacity(o) => {
                    let _ = write!(out, "opacity {}", num(*o));
                    depth += 1;
                }
                PaintCmd::PushClip { rect, radii } => {
                    let _ = write!(out, "clip {rect} r={}", num(radii.top_left));
                    depth += 1;
                }
                PaintCmd::PopTransform | PaintCmd::PopOpacity | PaintCmd::PopClip => {
                    out.push_str("pop");
                }
                PaintCmd::Quad(q) => {
                    let _ = write!(out, "quad {} {}", name(q.element), q.rect);
                    if !q.radii.is_zero() {
                        let _ = write!(out, " r={}", num(q.radii.top_left));
                    }
                    match q.fill {
                        Some(Fill::Solid(c)) => {
                            let _ = write!(out, " fill={}", color(c));
                        }
                        Some(Fill::LinearGradient { from, to, .. }) => {
                            let _ = write!(out, " gradient={}..{}", color(from), color(to));
                        }
                        None => {}
                    }
                    if let Some(b) = q.border {
                        let _ = write!(out, " border={} {}", num(b.width), color(b.color));
                    }
                    if let Some((e, _)) = q.shadow {
                        let _ = write!(out, " shadow={e:?}");
                    }
                    if let Some((m, r)) = q.material {
                        let _ = write!(out, " material={m:?} blur={}", num(r.blur));
                    }
                }
                PaintCmd::Text(t) => {
                    let _ = write!(
                        out,
                        "text {} {} {:?} size={} color={}",
                        name(t.element),
                        t.rect,
                        &*t.text,
                        num(t.size),
                        color(t.color)
                    );
                }
            }
            out.push('\n');
        }
        out
    }
}

/// Painting counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaintStats {
    /// Nodes painted.
    pub painted: usize,
    /// Canvas children skipped by viewport culling.
    pub culled: usize,
}

pub(crate) struct PaintInput<'a> {
    pub(crate) tree: &'a Tree,
    pub(crate) theme: &'a Theme,
    pub(crate) focus: Option<ElementId>,
    pub(crate) focus_visible: bool,
}

/// Paint the whole tree.
pub(crate) fn paint(input: &PaintInput<'_>) -> (Scene, PaintStats) {
    let mut scene = Scene::default();
    let mut stats = PaintStats::default();
    if let Some(root) = input.tree.root {
        paint_node(input, root, &mut scene, &mut stats);
    }
    (scene, stats)
}

fn paint_node(input: &PaintInput<'_>, id: ElementId, scene: &mut Scene, stats: &mut PaintStats) {
    let Some(node) = input.tree.nodes.get(&id) else {
        return;
    };
    let opacity = node.opacity();
    if opacity <= 0.001 {
        return;
    }
    stats.painted += 1;
    let cmds = &mut scene.commands;
    let transform = node.transform();
    let transformed = !transform.is_identity();
    if transformed {
        cmds.push(PaintCmd::PushTransform(transform));
    }
    let faded = opacity < 0.999;
    if faded {
        cmds.push(PaintCmd::PushOpacity(opacity));
    }

    let v = &node.visual;
    let bg = node.anims.bg.value();
    let border_color = node.anims.border.value();
    let material = v.material.map(|m| (m, input.theme.material(m)));
    let shadow = v
        .elevation
        .or(material.and_then(|(_, r)| r.elevation))
        .map(|e| (e, input.theme.shadow(e)));
    let fill = match material {
        Some((_, r)) if r.tint.alpha > 0.0 => Some(Fill::Solid(r.tint)),
        _ if bg.alpha > 0.0 => Some(Fill::Solid(bg)),
        _ => None,
    };
    let border = v
        .border
        .map(|(w, _)| Border {
            width: w,
            color: border_color,
        })
        .filter(|b| b.color.alpha > 0.0)
        .or_else(|| {
            material
                .filter(|(_, r)| r.hairline > 0.0)
                .map(|(_, r)| Border {
                    width: 1.0,
                    color: Color::WHITE.with_alpha(r.hairline),
                })
        });
    if fill.is_some() || border.is_some() || shadow.is_some() || material.is_some() {
        cmds.push(PaintCmd::Quad(Quad {
            element: id,
            rect: node.bounds,
            radii: v.radii,
            fill,
            border,
            shadow,
            material,
        }));
    }
    if let Some(t) = &node.text {
        cmds.push(PaintCmd::Text(TextRun {
            element: id,
            rect: node.bounds,
            text: t.text.clone(),
            size: t.size.unwrap_or_else(|| input.theme.metrics().body),
            color: t.color.unwrap_or(input.theme.colors().text_primary),
        }));
    }

    if !node.children.is_empty() {
        if v.clip {
            scene.commands.push(PaintCmd::PushClip {
                rect: node.bounds,
                radii: v.radii,
            });
        }
        let child_t = node.child_transform();
        if let Some(t) = child_t {
            scene.commands.push(PaintCmd::PushTransform(t));
        }
        for c in &node.children {
            if node.canvas.is_some()
                && let Some(child) = input.tree.nodes.get(c)
                && !canvas_visible(node, child)
            {
                stats.culled += 1;
                continue;
            }
            paint_node(input, *c, scene, stats);
        }
        if child_t.is_some() {
            scene.commands.push(PaintCmd::PopTransform);
        }
        if v.clip {
            scene.commands.push(PaintCmd::PopClip);
        }
    }

    if input.focus == Some(id) && input.focus_visible {
        scene.commands.push(PaintCmd::Quad(Quad {
            element: id,
            rect: node.bounds.inflate(2.0),
            radii: v.radii.grow(2.0),
            fill: None,
            border: Some(Border {
                width: 2.0,
                color: input.theme.colors().focus_ring,
            }),
            shadow: None,
            material: None,
        }));
    }
    if faded {
        scene.commands.push(PaintCmd::PopOpacity);
    }
    if transformed {
        scene.commands.push(PaintCmd::PopTransform);
    }
}
