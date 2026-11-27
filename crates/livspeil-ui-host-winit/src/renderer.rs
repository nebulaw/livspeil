// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The rendering boundary: the host owns the window, surface and frame
//! pacing; a [`SceneRenderer`] turns a [`Scene`] into GPU work.

use std::time::Duration;

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

/// Where a renderer spent the last frame (all zero when it does not
/// measure). Shown by the frame HUD ([`crate::frame_stats`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderTimings {
    /// Scene to display list and glyph atlas uploads (CPU).
    pub paint: Duration,
    /// Display list to GPU instance data (CPU).
    pub encode: Duration,
    /// Recording and submitting the GPU commands, including the instance
    /// upload (CPU time; the GPU runs asynchronously).
    pub submit: Duration,
    /// Display list items.
    pub items: usize,
    /// GPU instances.
    pub instances: usize,
    /// Draw calls (backdrop composites included).
    pub draw_calls: usize,
    /// Bytes of instance data uploaded.
    pub upload_bytes: usize,
    /// GPU time of a recent frame, when timing is on and supported.
    pub gpu: Option<Duration>,
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

    /// Turn GPU timing on or off (it costs a little, so the host asks only
    /// while measuring). The default ignores it and reports no GPU time.
    fn set_gpu_timing(&mut self, on: bool) {
        let _ = on;
    }

    /// The timings of the last [`SceneRenderer::render`], if the renderer
    /// measures them (the default reports zeros).
    fn take_timings(&mut self) -> RenderTimings {
        RenderTimings::default()
    }
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

    fn take_timings(&mut self) -> RenderTimings {
        (**self).take_timings()
    }
}
