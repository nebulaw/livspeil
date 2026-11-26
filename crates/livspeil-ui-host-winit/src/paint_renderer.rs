// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`PaintRenderer`]: the standard [`SceneRenderer`] (feature `widgets`).
//! Scenes go through the `livspeil-ui-paint` bridge (display list plus
//! glyph atlas uploads) into a `livspeil-ui-render` GPU renderer.

use livspeil_ui_core::{App, Scene};
use livspeil_ui_paint::PaintBridge;
use livspeil_ui_render as render;
use livspeil_ui_theme::Theme;

use std::time::Instant;

use crate::{HostError, RenderTarget, RenderTimings, SceneRenderer};

/// Draws scenes with the paint bridge: each scene becomes a display list
/// plus glyph atlas uploads, applied to the GPU renderer before the frame.
///
/// - The GPU renderer is (re)built for the surface format; a new renderer
///   has no atlas texture, so the whole glyph atlas is uploaded again.
/// - Backdrop blur samples the surface. When the surface cannot be sampled
///   (no `TEXTURE_BINDING` usage), every backdrop becomes an opaque quad of
///   its tint: the materials' T0 / "Reduce transparency" look.
///
/// Give the app the bridge's text measurer
/// (`app.set_text_measurer(renderer.bridge().text.measurer())`) so labels
/// are shaped once for layout and painting.
pub struct PaintRenderer {
    bridge: PaintBridge,
    clear: render::Color,
    renderer: Option<(wgpu::TextureFormat, render::Renderer)>,
    list: render::DisplayList,
    frame: render::Frame,
    timings: RenderTimings,
}

impl std::fmt::Debug for PaintRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaintRenderer")
            .field("clear", &self.clear)
            .field("format", &self.renderer.as_ref().map(|(f, _)| *f))
            .finish_non_exhaustive()
    }
}

impl PaintRenderer {
    /// A renderer drawing through `bridge`, clearing each frame to `clear`.
    #[must_use]
    pub fn new(bridge: PaintBridge, clear: render::Color) -> Self {
        PaintRenderer {
            bridge,
            clear,
            renderer: None,
            list: render::DisplayList::new(),
            frame: render::Frame {
                options: render::FrameOptions {
                    width: 1,
                    height: 1,
                    scale: 1.0,
                    clear,
                },
                clear: [0.0; 4],
                instances: Vec::new(),
                ops: Vec::new(),
            },
            timings: RenderTimings::default(),
        }
    }

    /// A renderer clearing to the theme's `surface.0`.
    #[must_use]
    pub fn for_theme(bridge: PaintBridge, theme: &Theme) -> Self {
        Self::new(bridge, clear_color(theme))
    }

    /// Install the bridge's text measurer into `app` (shared shaping cache).
    pub fn install_measurer(&self, app: &mut App) {
        app.set_text_measurer(self.bridge.text.measurer());
    }

    /// The paint bridge.
    #[must_use]
    pub fn bridge(&self) -> &PaintBridge {
        &self.bridge
    }

    /// The clear color.
    #[must_use]
    pub fn clear(&self) -> render::Color {
        self.clear
    }

    /// Change the clear color.
    pub fn set_clear(&mut self, clear: render::Color) {
        self.clear = clear;
    }
}

/// The theme's `surface.0` as a render color.
#[must_use]
pub fn clear_color(theme: &Theme) -> render::Color {
    let bg = theme.colors().surface[0];
    render::Color::oklab(bg.l, bg.a, bg.b).with_alpha(bg.alpha)
}

/// Give every text run one more logical pixel of width.
///
/// Layout rounds element sizes to whole pixels, so a label measured at
/// 65.8 px can get a 65 px box; the paint bridge wraps text whose natural
/// width exceeds its box by more than half a pixel, which would break such
/// a label (text fields lay out their text at its measured width) onto two
/// lines. One extra pixel absorbs the rounding; text given a real width
/// (paragraphs) still wraps where it should. [`PaintRenderer`] gets the
/// same effect without copying the scene from
/// `TextContext::width_slack`; this function is for other renderers.
pub fn widen_text_runs(scene: &Scene) -> std::borrow::Cow<'_, Scene> {
    use livspeil_ui_core::PaintCmd;
    if !scene
        .commands
        .iter()
        .any(|c| matches!(c, PaintCmd::Text(t) if t.rect.width > 0.0))
    {
        return std::borrow::Cow::Borrowed(scene);
    }
    let mut s = scene.clone();
    for c in &mut s.commands {
        if let PaintCmd::Text(t) = c
            && t.rect.width > 0.0
        {
            t.rect.width += 1.0;
        }
    }
    std::borrow::Cow::Owned(s)
}

fn renderer_error(e: impl std::error::Error + Send + Sync + 'static) -> HostError {
    HostError::Renderer(Box::new(e))
}

/// Replace every backdrop with an opaque quad of its tint (surfaces that
/// cannot be sampled; what T0 and "Reduce transparency" look like).
pub fn without_backdrops(list: &mut render::DisplayList) {
    for item in &mut list.items {
        if let render::Primitive::Backdrop(b) = &item.primitive {
            let mut q = render::Quad::new(b.rect, b.tint.with_alpha(1.0));
            q.radii = b.radii;
            q.corners = b.corners;
            q.shadows = b.shadows.clone();
            q.clip = b.clip;
            q.opacity = b.opacity;
            q.transform = b.transform;
            item.primitive = render::Primitive::Quad(q);
        }
    }
}

impl SceneRenderer for PaintRenderer {
    fn render(
        &mut self,
        gpu: &render::GpuContext,
        target: &RenderTarget<'_>,
        scene: &Scene,
    ) -> Result<(), HostError> {
        if self.renderer.as_ref().map(|(f, _)| *f) != Some(target.format) {
            self.renderer = Some((
                target.format,
                render::Renderer::new(&gpu.device, target.format),
            ));
            // A new renderer has no atlas texture: upload everything again.
            self.bridge.text.clear_atlas();
        }
        let Some((_, renderer)) = &mut self.renderer else {
            return Ok(());
        };
        // Buffers are reused frame to frame (the list and the encoded
        // instances are megabytes in a busy scene).
        let t0 = Instant::now();
        self.list.items.clear();
        self.bridge.text.width_slack = 1.0;
        let updates = self
            .bridge
            .paint_scene(scene, target.scale, &mut self.list)
            .map_err(renderer_error)?;
        if !target.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
            without_backdrops(&mut self.list);
        }
        renderer.update_atlases(&gpu.device, &gpu.queue, &updates)?;
        let t1 = Instant::now();
        render::encode_into(
            &self.list,
            render::FrameOptions {
                width: target.size_px.0,
                height: target.size_px.1,
                scale: target.scale,
                clear: self.clear,
            },
            &mut self.frame,
        )?;
        let t2 = Instant::now();
        renderer.render(&gpu.device, &gpu.queue, &self.frame, target.texture)?;
        self.timings = RenderTimings {
            paint: t1 - t0,
            encode: t2 - t1,
            submit: t2.elapsed(),
            items: self.list.items.len(),
            instances: self.frame.instances.len(),
            draw_calls: self.frame.draw_calls(),
            upload_bytes: self.frame.instance_byte_len(),
        };
        Ok(())
    }

    fn take_timings(&mut self) -> RenderTimings {
        std::mem::take(&mut self.timings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrops_become_opaque_quads() {
        let mut list = render::DisplayList::new();
        let tint = render::Color::oklab(0.5, 0.0, 0.0).with_alpha(0.5);
        let rect = render::Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        };
        list.push(
            0,
            render::Primitive::Backdrop(render::Backdrop::new(rect, 8.0, tint)),
        );
        without_backdrops(&mut list);
        match &list.items[0].primitive {
            render::Primitive::Quad(q) => {
                assert_eq!(q.fill, render::Quad::new(rect, tint.with_alpha(1.0)).fill);
            }
            other => panic!("expected a quad, got {other:?}"),
        }
    }

    #[test]
    fn clear_follows_theme() {
        let theme = Theme::default();
        let c = clear_color(&theme);
        assert_eq!(
            c,
            PaintRenderer::for_theme(
                PaintBridge::new(livspeil_ui_paint::FontSystem::test()),
                &theme
            )
            .clear()
        );
    }
}
