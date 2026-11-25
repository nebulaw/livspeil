// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Window and surface options, and the surface configuration they imply.

use std::time::Duration;

use livspeil_ui_theme::ConfigWatcher;

use crate::HostError;

/// The titlebar on macOS. Elsewhere the window keeps the system's
/// decorations (X11 window manager frames; on Wayland server-side
/// decorations, or winit's client-side ones where the compositor offers
/// none), whatever this says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Titlebar {
    /// The standard titlebar above the content.
    Standard,
    /// A full-size content view under a transparent native titlebar: the
    /// traffic lights and title stay, the UI draws behind them. Leave
    /// [`Titlebar::MACOS_INSET`] logical pixels free at the top.
    #[default]
    FullSizeContent,
    /// Like `FullSizeContent` with the title hidden, for a custom
    /// titlebar widget (`livspeil_ui_widgets::Titlebar`) that draws the
    /// title and is the window's drag region.
    Hidden,
}

impl Titlebar {
    /// Height of the macOS titlebar area in logical pixels.
    pub const MACOS_INSET: f32 = 28.0;
}

/// Window options.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowOptions {
    /// Title.
    pub title: String,
    /// Initial inner size in logical pixels.
    pub size: (f64, f64),
    /// Minimum inner size in logical pixels.
    pub min_size: Option<(f64, f64)>,
    /// Resizable.
    pub resizable: bool,
    /// A transparent window (premultiplied alpha composition when the
    /// surface supports it). A hook for later; the example is opaque.
    pub transparent: bool,
    /// macOS titlebar style.
    pub titlebar: Titlebar,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: "Livspeil".to_owned(),
            size: (960.0, 640.0),
            min_size: Some((320.0, 240.0)),
            resizable: true,
            transparent: false,
            titlebar: Titlebar::default(),
        }
    }
}

/// Presentation options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceOptions {
    /// Present mode; FIFO (vsync, always supported) by default. Modes the
    /// surface does not support fall back to FIFO.
    pub present_mode: wgpu::PresentMode,
    /// Frames the presentation engine may queue (1 = lowest latency; UI spec
    /// target latency is one frame).
    pub max_frame_latency: u32,
}

impl Default for SurfaceOptions {
    fn default() -> Self {
        SurfaceOptions {
            present_mode: wgpu::PresentMode::Fifo,
            max_frame_latency: 1,
        }
    }
}

/// Called with the app after an accessibility action was performed (for
/// example to let `livspeil-ui-widgets`' `Env` see focus changes:
/// `move |app| env.sync_focus(app)`).
pub struct ActionHook(pub Box<dyn FnMut(&mut livspeil_ui_core::App)>);

impl std::fmt::Debug for ActionHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ActionHook")
    }
}

/// Everything [`run`](crate::run) needs besides the app and renderer.
#[derive(Debug)]
pub struct HostOptions {
    /// The window.
    pub window: WindowOptions,
    /// The surface.
    pub surface: SurfaceOptions,
    /// Config hot reload: a watcher with files registered, or `None` (then
    /// the host sets no timer at all).
    pub config: Option<ConfigWatcher>,
    /// How often the config files are polled.
    pub config_poll_interval: Duration,
    /// Derive cursor shapes from the role of the element under the pointer
    /// when no handler asks for one (pointing hand for buttons, I-beam for
    /// text fields, east-west resize for sliders and scrub fields). On by
    /// default.
    pub role_cursors: bool,
    /// Blink the caret of the focused text input. On by default.
    pub caret_blink: bool,
    /// See [`ActionHook`].
    pub after_accessibility_action: Option<ActionHook>,
}

impl Default for HostOptions {
    fn default() -> Self {
        HostOptions {
            window: WindowOptions::default(),
            surface: SurfaceOptions::default(),
            config: None,
            config_poll_interval: Duration::from_millis(500),
            role_cursors: true,
            caret_blink: true,
            after_accessibility_action: None,
        }
    }
}

impl HostOptions {
    /// Set the title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.window.title = title.into();
        self
    }

    /// Set the initial logical size.
    #[must_use]
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.window.size = (width, height);
        self
    }

    /// Set the present mode.
    #[must_use]
    pub fn present_mode(mut self, mode: wgpu::PresentMode) -> Self {
        self.surface.present_mode = mode;
        self
    }

    /// Set the minimum logical size.
    #[must_use]
    pub fn min_size(mut self, width: f64, height: f64) -> Self {
        self.window.min_size = Some((width, height));
        self
    }

    /// Set the macOS titlebar style (see [`Titlebar`]).
    #[must_use]
    pub fn titlebar(mut self, titlebar: Titlebar) -> Self {
        self.window.titlebar = titlebar;
        self
    }

    /// Run `f` after each accessibility action (see [`ActionHook`]).
    #[must_use]
    pub fn after_accessibility_action(
        mut self,
        f: impl FnMut(&mut livspeil_ui_core::App) + 'static,
    ) -> Self {
        self.after_accessibility_action = Some(ActionHook(Box::new(f)));
        self
    }

    /// Watch config files with `watcher`.
    #[must_use]
    pub fn config(mut self, watcher: ConfigWatcher) -> Self {
        self.config = Some(watcher);
        self
    }
}

/// Pick the surface format: a non-sRGB 8-bit format (the renderer writes
/// gamma-encoded sRGB and blends in sRGB space, so an `*Srgb` format would
/// encode twice). Prefers the surface's own order.
///
/// # Errors
/// [`HostError::SurfaceConfig`] when the surface offers no such format.
pub fn pick_format(formats: &[wgpu::TextureFormat]) -> Result<wgpu::TextureFormat, HostError> {
    formats
        .iter()
        .copied()
        .find(|f| {
            matches!(
                f,
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
            )
        })
        .ok_or(HostError::SurfaceConfig(
            "the surface offers no Bgra8Unorm or Rgba8Unorm format",
        ))
}

/// The surface configuration for `caps`, a physical `size` and the options.
///
/// # Errors
/// See [`pick_format`].
pub fn surface_config(
    caps: &wgpu::SurfaceCapabilities,
    size: (u32, u32),
    options: SurfaceOptions,
    transparent: bool,
) -> Result<wgpu::SurfaceConfiguration, HostError> {
    let format = pick_format(&caps.formats)?;
    let present_mode = match options.present_mode {
        m @ (wgpu::PresentMode::AutoVsync | wgpu::PresentMode::AutoNoVsync) => m,
        m if caps.present_modes.contains(&m) => m,
        _ => wgpu::PresentMode::Fifo,
    };
    let wanted = if transparent {
        [
            wgpu::CompositeAlphaMode::PreMultiplied,
            wgpu::CompositeAlphaMode::Inherit,
        ]
    } else {
        [
            wgpu::CompositeAlphaMode::Opaque,
            wgpu::CompositeAlphaMode::Auto,
        ]
    };
    let alpha_mode = wanted
        .into_iter()
        .find(|m| caps.alpha_modes.contains(m))
        .or_else(|| caps.alpha_modes.first().copied())
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);
    let usage = wgpu::TextureUsages::RENDER_ATTACHMENT
        | (caps.usages & wgpu::TextureUsages::TEXTURE_BINDING);
    Ok(wgpu::SurfaceConfiguration {
        usage,
        format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: size.0.max(1),
        height: size.1.max(1),
        present_mode,
        desired_maximum_frame_latency: options.max_frame_latency.max(1),
        alpha_mode,
        view_formats: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{CompositeAlphaMode as A, PresentMode as P, TextureFormat as F};

    fn caps(formats: Vec<F>) -> wgpu::SurfaceCapabilities {
        wgpu::SurfaceCapabilities {
            formats,
            format_capabilities: vec![],
            present_modes: vec![P::Fifo, P::Mailbox],
            alpha_modes: vec![A::Opaque, A::PostMultiplied],
            usages: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        }
    }

    #[test]
    fn non_srgb_format_preferred() {
        // Metal lists the sRGB variant first.
        let c = caps(vec![F::Bgra8UnormSrgb, F::Bgra8Unorm, F::Rgba16Float]);
        let cfg = surface_config(&c, (800, 600), SurfaceOptions::default(), false).unwrap();
        assert_eq!(cfg.format, F::Bgra8Unorm);
        assert_eq!(cfg.present_mode, P::Fifo);
        assert_eq!(cfg.desired_maximum_frame_latency, 1);
        assert_eq!(cfg.alpha_mode, A::Opaque);
        assert_eq!(
            cfg.usage,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        );
        assert!(pick_format(&[F::Bgra8UnormSrgb]).is_err());
    }

    #[test]
    fn present_mode_fallback_and_zero_size() {
        let c = caps(vec![F::Rgba8Unorm]);
        let o = SurfaceOptions {
            present_mode: P::Mailbox,
            max_frame_latency: 0,
        };
        let cfg = surface_config(&c, (0, 0), o, true).unwrap();
        assert_eq!(cfg.present_mode, P::Mailbox);
        assert_eq!((cfg.width, cfg.height), (1, 1));
        assert_eq!(cfg.desired_maximum_frame_latency, 1);
        // No premultiplied mode offered: the surface's first.
        assert_eq!(cfg.alpha_mode, A::Opaque);
        let o = SurfaceOptions {
            present_mode: P::Immediate,
            max_frame_latency: 2,
        };
        assert_eq!(
            surface_config(&c, (1, 1), o, false).unwrap().present_mode,
            P::Fifo
        );
    }
}
