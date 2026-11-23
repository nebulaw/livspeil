// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The display list: plain data describing one frame, in logical pixels.
//! Build it once per dirty frame (or keep and edit it: every item is a
//! value, so a retained tree can patch items in place).

use crate::color::Color;
use crate::geom::{Affine, Rect};

/// Corner radii in logical pixels, clockwise from the top left.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Radii {
    /// Top left.
    pub tl: f32,
    /// Top right.
    pub tr: f32,
    /// Bottom right.
    pub br: f32,
    /// Bottom left.
    pub bl: f32,
}

impl Radii {
    /// The same radius on every corner.
    pub const fn all(r: f32) -> Radii {
        Radii {
            tl: r,
            tr: r,
            br: r,
            bl: r,
        }
    }

    /// `[tl, tr, br, bl]`.
    pub fn to_array(self) -> [f32; 4] {
        [self.tl, self.tr, self.br, self.bl]
    }
}

/// How corners are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CornerStyle {
    /// Circular arcs (CSS `border-radius`).
    Circular,
    /// Continuous corners: a superellipse whose curvature blends into the
    /// straight edges (no curvature jump where the arc meets the edge). For
    /// the same radius it has the same 45-degree inset as a circular corner
    /// but starts further along the edge (about 1.7x the radius), like the
    /// "squircle" corners of macOS and iOS.
    #[default]
    Continuous,
}

/// One gradient stop. Gradients are interpolated in OKLab with
/// premultiplied alpha, then converted to sRGB per pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    /// Position along the gradient, 0..1.
    pub offset: f32,
    /// Color at this position.
    pub color: Color,
}

/// Maximum number of stops in one gradient.
pub const MAX_GRADIENT_STOPS: usize = 4;

/// How a quad is filled.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    /// A single color.
    Solid(Color),
    /// A linear gradient from `start` to `end`. Points are in unit
    /// coordinates of the quad's rect (`(0, 0)` top left, `(1, 1)` bottom
    /// right).
    Linear {
        /// Where offset 0 lies.
        start: [f32; 2],
        /// Where offset 1 lies.
        end: [f32; 2],
        /// 1 to [`MAX_GRADIENT_STOPS`] stops, sorted by offset.
        stops: Vec<GradientStop>,
    },
    /// An elliptical radial gradient. `center` and `radius` are in unit
    /// coordinates of the quad's rect.
    Radial {
        /// Where offset 0 lies.
        center: [f32; 2],
        /// Distance to offset 1 along x and y.
        radius: [f32; 2],
        /// 1 to [`MAX_GRADIENT_STOPS`] stops, sorted by offset.
        stops: Vec<GradientStop>,
    },
}

impl Fill {
    /// A vertical linear gradient between two colors.
    pub fn vertical(top: Color, bottom: Color) -> Fill {
        Fill::Linear {
            start: [0.0, 0.0],
            end: [0.0, 1.0],
            stops: vec![
                GradientStop {
                    offset: 0.0,
                    color: top,
                },
                GradientStop {
                    offset: 1.0,
                    color: bottom,
                },
            ],
        }
    }
}

/// Border width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BorderWidth {
    /// In logical pixels (scaled by the frame scale and the transform).
    Logical(f32),
    /// Exactly one physical pixel at any scale.
    Hairline,
}

/// An inner border: drawn inside the shape's edge, over the fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Border {
    /// Width.
    pub width: BorderWidth,
    /// Color.
    pub color: Color,
}

impl Border {
    /// A one-physical-pixel border.
    pub fn hairline(color: Color) -> Border {
        Border {
            width: BorderWidth::Hairline,
            color,
        }
    }
}

/// One drop shadow (CSS `box-shadow` semantics: `blur` is the blur radius,
/// the Gaussian's sigma is `blur / 2`). Shadows are not drawn under the
/// shape itself, so translucent fills do not show them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shadow {
    /// Offset `[x, y]` in logical pixels.
    pub offset: [f32; 2],
    /// Blur radius in logical pixels.
    pub blur: f32,
    /// Grows (or shrinks) the shadow shape.
    pub spread: f32,
    /// Color.
    pub color: Color,
}

/// A rectangle with rounded corners, fill, inner border and shadows.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    /// The shape, in the quad's local coordinates.
    pub rect: Rect,
    /// Corner radii.
    pub radii: Radii,
    /// Corner style.
    pub corners: CornerStyle,
    /// Fill; `None` draws only border and shadows.
    pub fill: Option<Fill>,
    /// Inner border.
    pub border: Option<Border>,
    /// Shadow stack, drawn in order (first is lowest), all below the fill.
    /// Two layers (ambient + contact) is typical, see the elevation tokens.
    pub shadows: Vec<Shadow>,
    /// Clip rectangle in logical screen coordinates (after the transform).
    pub clip: Option<Rect>,
    /// Multiplies the alpha of everything the quad draws.
    pub opacity: f32,
    /// Local-to-screen transform (logical pixels).
    pub transform: Affine,
}

impl Quad {
    /// An unrounded quad filled with one color.
    pub fn new(rect: Rect, color: Color) -> Quad {
        Quad {
            rect,
            radii: Radii::default(),
            corners: CornerStyle::default(),
            fill: Some(Fill::Solid(color)),
            border: None,
            shadows: Vec::new(),
            clip: None,
            opacity: 1.0,
            transform: Affine::IDENTITY,
        }
    }

    /// With the same radius on every corner.
    #[must_use]
    pub fn radius(mut self, r: f32) -> Quad {
        self.radii = Radii::all(r);
        self
    }

    /// With another fill.
    #[must_use]
    pub fn fill(mut self, fill: Fill) -> Quad {
        self.fill = Some(fill);
        self
    }

    /// With a border.
    #[must_use]
    pub fn border(mut self, border: Border) -> Quad {
        self.border = Some(border);
        self
    }

    /// With shadows.
    #[must_use]
    pub fn shadows(mut self, shadows: &[Shadow]) -> Quad {
        self.shadows = shadows.to_vec();
        self
    }
}

/// How the inner highlight of a [`Backdrop`] is distributed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightKind {
    /// Along the whole edge.
    Full,
    /// Along the top edge, fading out below the top corners.
    TopRim,
}

/// A light inner rim on a material.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Highlight {
    /// Rim width in logical pixels (1 is typical).
    pub width: f32,
    /// Rim color (white at 8-12% alpha is typical).
    pub color: Color,
    /// Distribution.
    pub kind: HighlightKind,
}

/// A region that shows a blurred, tinted copy of what was drawn before it
/// (Frost and Glass materials).
#[derive(Debug, Clone, PartialEq)]
pub struct Backdrop {
    /// Shape, in local coordinates.
    pub rect: Rect,
    /// Corner radii.
    pub radii: Radii,
    /// Corner style.
    pub corners: CornerStyle,
    /// Blur radius in logical pixels (roughly the standard deviation of the
    /// equivalent Gaussian). 0 disables the blur: the region shows only the
    /// tint over what is behind it (use an opaque tint for T0 and "Reduce
    /// transparency").
    pub blur: f32,
    /// Saturation multiplier applied to the blurred content (1 = unchanged).
    pub saturation: f32,
    /// Tint laid over the blurred content; its alpha is the tint strength.
    pub tint: Color,
    /// Inner highlight.
    pub highlight: Option<Highlight>,
    /// Shadow stack, drawn outside the shape.
    pub shadows: Vec<Shadow>,
    /// Grain amplitude (0..~0.03) added to break banding on large blurs.
    pub grain: f32,
    /// Clip rectangle in logical screen coordinates.
    pub clip: Option<Rect>,
    /// Multiplies the alpha of everything the backdrop draws.
    pub opacity: f32,
    /// Local-to-screen transform.
    pub transform: Affine,
}

impl Backdrop {
    /// A backdrop with blur and tint and no highlight or shadows.
    pub fn new(rect: Rect, blur: f32, tint: Color) -> Backdrop {
        Backdrop {
            rect,
            radii: Radii::default(),
            corners: CornerStyle::default(),
            blur,
            saturation: 1.0,
            tint,
            highlight: None,
            shadows: Vec::new(),
            grain: 0.0,
            clip: None,
            opacity: 1.0,
            transform: Affine::IDENTITY,
        }
    }
}

/// A run of glyphs from the glyph atlas. Placeholder: text arrives in M1.2
/// (`livspeil-ui-text`); this primitive draws nothing yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Glyphs {
    /// Atlas the glyphs come from.
    pub atlas: u32,
    /// Glyph ids and their positions in local coordinates.
    pub glyphs: Vec<(u32, [f32; 2])>,
    /// Text color.
    pub color: Color,
    /// Local-to-screen transform.
    pub transform: Affine,
}

/// An image or icon from a texture atlas. Placeholder: draws nothing yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// Atlas texture id.
    pub atlas: u32,
    /// Destination rectangle in local coordinates.
    pub rect: Rect,
    /// Source rectangle in atlas texels.
    pub source: Rect,
    /// Tint (icons are alpha masks tinted at draw time).
    pub tint: Option<Color>,
    /// Local-to-screen transform.
    pub transform: Affine,
}

/// One display list item.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    /// A rounded rectangle.
    Quad(Quad),
    /// A blurred region.
    Backdrop(Backdrop),
    /// Text (draws nothing yet).
    Glyphs(Glyphs),
    /// An image (draws nothing yet).
    Image(Image),
}

/// An item with its layer.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// Layer; lower layers are drawn first. Items in one layer are drawn in
    /// insertion order.
    pub layer: i32,
    /// The primitive.
    pub primitive: Primitive,
}

/// An ordered list of primitives.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    /// The items, in insertion order.
    pub items: Vec<Item>,
}

impl DisplayList {
    /// An empty list.
    pub fn new() -> DisplayList {
        DisplayList::default()
    }

    /// Appends a primitive on a layer.
    pub fn push(&mut self, layer: i32, primitive: impl Into<Primitive>) {
        self.items.push(Item {
            layer,
            primitive: primitive.into(),
        });
    }

    /// Appends a primitive on layer 0.
    pub fn add(&mut self, primitive: impl Into<Primitive>) {
        self.push(0, primitive);
    }

    /// Number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl From<Quad> for Primitive {
    fn from(q: Quad) -> Self {
        Primitive::Quad(q)
    }
}

impl From<Backdrop> for Primitive {
    fn from(b: Backdrop) -> Self {
        Primitive::Backdrop(b)
    }
}

impl From<Glyphs> for Primitive {
    fn from(g: Glyphs) -> Self {
        Primitive::Glyphs(g)
    }
}

impl From<Image> for Primitive {
    fn from(i: Image) -> Self {
        Primitive::Image(i)
    }
}
