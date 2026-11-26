// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI renderer.
//!
//! Build a [`DisplayList`] (quads with continuous corners, gradients,
//! hairline borders and shadow stacks; backdrops for the Frost and Glass
//! materials), [`encode`] it for a target size, then draw the [`Frame`] with
//! the wgpu [`Renderer`] or with the CPU reference [`render_cpu`].
//!
//! Design notes:
//! - **Corners**: per-corner superellipse `|x|^n + |y|^n = R^n` in a box of
//!   size `R`; circular corners are `n = 2`. Continuous corners use
//!   `R = 1.7 r` and pick `n` so the 45-degree point matches a circle of
//!   radius `r` (n ~ 3.7); when the radii must shrink to fit (pills) `n`
//!   falls back toward 2. The distance is the superellipse norm corrected
//!   by its gradient length, so edges get a one-pixel AA ramp everywhere.
//! - **Shadows**: analytic. A Gaussian-blurred rounded box: closed form
//!   (erf) along x, four samples along y (the approach popularised by Evan
//!   Wallace), with a circular corner of the same 45-degree inset.
//!   Constant cost per pixel at any blur radius, so no separable blur pass
//!   is needed. Shadows are clipped out of their shape.
//! - **Backdrops**: dual-Kawase blur (downsample from full to 1/2^L
//!   resolution, upsample back to 1/2), scissored to the backdrop's region
//!   grown by the blur's reach; the composite samples the half-resolution
//!   level, applies CSS-style saturation, tint, inner highlight and grain.
//! - **Color**: colors are OKLab/OKLCH; gradients interpolate in OKLab
//!   (premultiplied) and convert per pixel. Compositing is premultiplied
//!   alpha in gamma-encoded sRGB (UI spec, section 3) into a non-sRGB 8-bit
//!   target, so the bytes read back are sRGB. Gradients, shadows and
//!   backdrops add a 4x4 ordered dither of +-0.5 LSB against banding.
//! - **Text**: [`Glyphs`] are instanced quads (the same instance layout and
//!   pipeline as quads, kind 3, so text and boxes on one layer batch into
//!   one draw call) that read an R8 coverage atlas (uploaded through
//!   [`AtlasUpdate`]s: dirty rectangles, or a full upload when the
//!   producer's generation changes). Sampling is a manual bilinear filter
//!   with `textureLoad` that treats texels outside the glyph's own rect as
//!   empty, so neighbours never bleed and the CPU reference matches the GPU
//!   exactly; with an identity transform texels map 1:1 to pixels.
//!   Blending: grayscale AA (no LCD). Compositing stays in gamma sRGB like
//!   the rest of the chrome, which on its own makes light-on-dark text thin
//!   and dark-on-light text heavy. To compensate, the coverage `a` is
//!   remapped from the text color's OKLab lightness `L`:
//!   `a' = mix(1 - (1 - a)^(1/g), a^(1/g), L)` with `g = TEXT_GAMMA` (1.8).
//!   With `g = 2.2` this would be exact linear-light blending against a
//!   black (light text) or white (dark text) background; 1.8 keeps light
//!   text on dark surfaces from looking bold, like macOS-style smoothing.

#![forbid(unsafe_code)]

mod atlas;
mod color;
mod cpu;
mod encode;
mod geom;
mod gpu;
mod image;
mod list;
mod shade;
mod support;

pub use atlas::{AtlasStore, AtlasUpdate, MAX_ATLAS_SIZE};
pub use color::{Color, linear_to_srgb, oklab_to_srgb, srgb_to_linear};
pub use cpu::{render_cpu, render_cpu_with};
pub use encode::{
    BlurPlan, Frame, FrameOptions, INSTANCE_FLOATS, Instance, MAX_BLUR_LEVELS, Op, TEXT_GAMMA,
    corner_params, encode, kawase_params, level_size,
};
pub use geom::{Affine, Rect};
pub use gpu::{GpuContext, Renderer, TARGET_FORMAT};
pub use image::RgbaImage;
pub use list::{
    Backdrop, Border, BorderWidth, CornerStyle, DisplayList, Fill, GlyphSprite, Glyphs,
    GradientStop, Highlight, HighlightKind, Image, Item, MAX_GRADIENT_STOPS, Primitive, Quad,
    Radii, Shadow,
};
pub use support::{BlurPath, BlurSupport, choose_blur_path};

/// Errors from encoding or rendering.
#[derive(Debug, Clone, PartialEq)]
pub enum RenderError {
    /// The target has zero width or height.
    EmptyTarget,
    /// The scale factor is not a positive finite number.
    InvalidScale(f32),
    /// A display list item has invalid values.
    InvalidPrimitive {
        /// Item index in the display list.
        index: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// A gradient has no stops or more than [`MAX_GRADIENT_STOPS`].
    GradientStops {
        /// Item index in the display list.
        index: usize,
        /// Number of stops given.
        count: usize,
    },
    /// No GPU adapter (not even a software one) is available.
    NoAdapter(String),
    /// The device could not be created or reported a validation error.
    Device(String),
    /// The target texture does not match the frame or renderer.
    Target(&'static str),
    /// Backdrop blur is unavailable on this device or target (see
    /// [`BlurSupport`]); the text says why.
    BlurUnavailable(String),
    /// Reading the target back failed.
    Readback(String),
    /// An atlas update is invalid.
    Atlas(&'static str),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenderError::EmptyTarget => write!(f, "target has zero size"),
            RenderError::InvalidScale(s) => write!(f, "invalid scale factor {s}"),
            RenderError::InvalidPrimitive { index, reason } => {
                write!(f, "display list item {index}: {reason}")
            }
            RenderError::GradientStops { index, count } => write!(
                f,
                "display list item {index}: gradient has {count} stops (1..={MAX_GRADIENT_STOPS} allowed)"
            ),
            RenderError::NoAdapter(e) => write!(f, "no GPU adapter available: {e}"),
            RenderError::Device(e) => write!(f, "GPU device error: {e}"),
            RenderError::Target(e) => write!(f, "target texture: {e}"),
            RenderError::BlurUnavailable(e) => write!(f, "backdrop blur unavailable: {e}"),
            RenderError::Readback(e) => write!(f, "reading back the target failed: {e}"),
            RenderError::Atlas(e) => write!(f, "atlas update: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}
