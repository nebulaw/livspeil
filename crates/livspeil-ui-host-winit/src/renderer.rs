// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The rendering boundary: the host owns the window, surface and frame
//! pacing; a [`SceneRenderer`] turns a [`Scene`] into GPU work.

use livspeil_ui_core::Scene;
use livspeil_ui_render::GpuContext;

use crate::HostError;

/// The surface texture of one frame.
#[derive(Debug)]
pub struct RenderTarget<'a> {
    /// The texture (usage: [`RenderTarget::usage`]).
    pub texture: &'a wgpu::Texture,
    /// A default view of it.
    pub view: &'a wgpu::TextureView,
    /// Its format: a non-sRGB 8-bit format. Write gamma-encoded sRGB
    /// (the UI composites in sRGB space, UI spec section 3).
    pub format: wgpu::TextureFormat,
    /// Its usage: always `RENDER_ATTACHMENT`, plus `TEXTURE_BINDING` when
    /// the surface supports it (backdrop blur reads the target).
    pub usage: wgpu::TextureUsages,
    /// Size in physical pixels.
    pub size_px: (u32, u32),
    /// Physical pixels per logical pixel (the window's scale factor).
    pub scale: f32,
}

/// Draws a [`Scene`] into a surface texture.
///
/// The host never inspects the scene: it decides *when* to draw and hands
/// the scene and the target to the renderer, then presents. Implementations
/// keep their pipelines and atlases between frames and rebuild them when
/// `target.format` changes. Submit the work on `gpu.queue`; the host
/// presents after `render` returns.
pub trait SceneRenderer {
    /// Draw `scene` (logical pixels) into `target`.
    ///
    /// # Errors
    /// Rendering failures; the host logs them and skips the frame.
    fn render(
        &mut self,
        gpu: &GpuContext,
        target: &RenderTarget<'_>,
        scene: &Scene,
    ) -> Result<(), HostError>;
}

impl<R: SceneRenderer + ?Sized> SceneRenderer for Box<R> {
    fn render(
        &mut self,
        gpu: &GpuContext,
        target: &RenderTarget<'_>,
        scene: &Scene,
    ) -> Result<(), HostError> {
        (**self).render(gpu, target, scene)
    }
}
