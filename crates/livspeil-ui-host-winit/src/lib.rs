// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI desktop host on winit (macOS first) with wgpu surfaces.
//!
//! [`run`] opens a window, translates winit events into
//! `livspeil-ui-core` input ([`input`]), paces frames on demand
//! ([`pacing`]: idle = no frames and no timers), and hands each frame's
//! [`Scene`](livspeil_ui_core::Scene) to a [`SceneRenderer`], which owns
//! all GPU work. The host also applies the app's platform requests: cursor
//! shape, IME enablement and candidate window position, clipboard
//! copy/paste and window requests (move, zoom, minimize from a custom
//! titlebar), and polls config files for hot reload when asked to.
//!
//! ```no_run
//! use livspeil_ui_core::{App, Scene};
//! use livspeil_ui_host_winit::{HostError, HostOptions, RenderTarget, SceneRenderer, run};
//! use livspeil_ui_render::GpuContext;
//!
//! struct Nothing;
//! impl SceneRenderer for Nothing {
//!     fn render(&mut self, _: &GpuContext, _: &RenderTarget<'_>, _: &Scene) -> Result<(), HostError> {
//!         Ok(())
//!     }
//! }
//!
//! run(App::new(), Nothing, HostOptions::default().title("Hello"))?;
//! # Ok::<(), HostError>(())
//! ```
//!
//! Design notes: `docs/ui-host.md`.

mod accessibility;
mod error;
pub mod frame_stats;
mod gpu_info;
mod host;
pub mod hud;
pub mod input;
mod options;
pub mod pacing;
#[cfg(feature = "widgets")]
mod paint_renderer;
mod platform;
mod renderer;
pub mod widget_platform;

pub use accessibility::Accessibility;
pub use error::HostError;
pub use gpu_info::{FormatReport, GpuReport, gpu_info};
pub use host::{Host, HostEvent, run};
pub use hud::HudOptions;
pub use livspeil_ui_render::{BlurPath, BlurSupport};
pub use options::{
    ActionHook, FRAME_LATENCY_ENV, HostOptions, PRESENT_MODE_ENV, SurfaceOptions, Titlebar,
    WindowOptions, parse_present_mode, pick_format, surface_config,
};
#[cfg(feature = "widgets")]
pub use paint_renderer::{PaintRenderer, clear_color, widen_text_runs, without_backdrops};
pub use platform::{
    Clipboard, ImeState, MemoryClipboard, SystemClipboard, WindowAction, cursor_icon,
    service_clipboard, window_action,
};
pub use renderer::{RenderTarget, RenderTimings, SceneRenderer};
#[cfg(feature = "widgets")]
pub use widget_platform::WidgetClipboard;

pub use accesskit_winit;

pub use winit;
