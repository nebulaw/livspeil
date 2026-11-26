// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`PaintRenderer`]: the standard [`SceneRenderer`] (feature `widgets`).
//! Scenes go through the `livspeil-ui-paint` bridge (display list plus
//! glyph atlas uploads) into a `livspeil-ui-render` GPU renderer.

use livspeil_ui_core::{App, Scene};
use livspeil_ui_paint::PaintBridge;
use livspeil_ui_render as render;
use livspeil_ui_theme::Theme;

use crate::{HostError, RenderTarget, SceneRenderer};

/// Observer of blur path changes.
type BlurObserver = Box<dyn FnMut(&render::BlurPath)>;

/// Draws scenes with the paint bridge: each scene becomes a display list
/// plus glyph atlas uploads, applied to the GPU renderer before the frame.
///
/// - The GPU renderer is (re)built for the surface format; a new renderer
///   has no atlas texture, so the whole glyph atlas is uploaded again.
/// - Backdrop blur samples what was drawn so far. A surface that can be
///   sampled (Vulkan) is drawn into directly; a render-only one (Metal
///   drawables) goes through an internal texture and a blit
///   ([`BlurPath`]). When blur cannot run at all (the device rejected the
///   blur pipelines, or the surface is not renderable), every backdrop
///   becomes an opaque quad of its own colour, one warning is logged with
///   the reason, and [`PaintRenderer::on_blur_path`] observers are told
///   (decision D63).
///
/// Give the app the bridge's text measurer
/// (`app.set_text_measurer(renderer.bridge().text.measurer())`) so labels
/// are shaped once for layout and painting.
pub struct PaintRenderer {
    bridge: PaintBridge,
    clear: render::Color,
    renderer: Option<(wgpu::TextureFormat, render::Renderer)>,
    blur_path: Option<render::BlurPath>,
    on_blur_path: Option<BlurObserver>,
}

impl std::fmt::Debug for PaintRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaintRenderer")
            .field("clear", &self.clear)
            .field("format", &self.renderer.as_ref().map(|(f, _)| *f))
            .field("blur_path", &self.blur_path)
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
            blur_path: None,
            on_blur_path: None,
        }
    }

    /// Call `f` with the blur path whenever it is first known or changes
    /// (the first frame, a new surface format). The gallery uses this to
    /// show a note on its Materials page when blur is in fallback.
    pub fn on_blur_path(&mut self, f: impl FnMut(&render::BlurPath) + 'static) {
        self.on_blur_path = Some(Box::new(f));
    }

    /// The blur path chosen for the last frame (`None` before the first).
    #[must_use]
    pub fn blur_path(&self) -> Option<&render::BlurPath> {
        self.blur_path.as_ref()
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
/// (paragraphs) still wraps where it should. [`PaintRenderer`] applies
/// this to every scene.
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

/// Replace every backdrop with an opaque quad of its own colour (blur
/// unavailable; what T0 and "Reduce transparency" look like).
pub fn without_backdrops(list: &mut render::DisplayList) {
    list.without_backdrops();
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
        let mut list = render::DisplayList::new();
        let scene = widen_text_runs(scene);
        let updates = self
            .bridge
            .paint_scene(&scene, target.scale, &mut list)
            .map_err(renderer_error)?;
        let path = renderer.blur_path(target.usage);
        if self.blur_path.as_ref() != Some(&path) {
            match &path {
                render::BlurPath::Direct => {
                    log::info!("backdrop blur: direct (surface is sampled)")
                }
                render::BlurPath::Offscreen => log::info!(
                    "backdrop blur: through an offscreen texture (the surface cannot be sampled, usage {:?})",
                    target.usage
                ),
                render::BlurPath::Fallback(why) => log::warn!(
                    "backdrop blur disabled, drawing plain opaque colours instead: {why}"
                ),
            }
            if let Some(f) = &mut self.on_blur_path {
                f(&path);
            }
            self.blur_path = Some(path.clone());
        }
        if matches!(path, render::BlurPath::Fallback(_)) {
            list.without_backdrops();
        }
        renderer.update_atlases(&gpu.device, &gpu.queue, &updates)?;
        let frame = render::encode(
            &list,
            render::FrameOptions {
                width: target.size_px.0,
                height: target.size_px.1,
                scale: target.scale,
                clear: self.clear,
            },
        )?;
        renderer.render(&gpu.device, &gpu.queue, &frame, target.texture)?;
        Ok(())
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
