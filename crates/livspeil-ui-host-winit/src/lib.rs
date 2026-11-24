// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI desktop host on winit (macOS first) with wgpu surfaces.
//!
//! [`run`] opens a window, translates winit events into
//! `livspeil-ui-core` input ([`input`]), paces frames on demand
//! ([`pacing`]: idle = no frames and no timers), and hands each frame's
//! [`Scene`](livspeil_ui_core::Scene) to a [`SceneRenderer`], which owns
//! all GPU work. The host also applies the app's platform requests: cursor
//! shape, IME enablement and candidate window position, and clipboard
//! copy/paste, and polls config files for hot reload when asked to.
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

mod error;
mod host;
pub mod input;
mod options;
pub mod pacing;
mod platform;
mod renderer;

pub use error::HostError;
pub use host::{Host, run};
pub use options::{
    HostOptions, SurfaceOptions, Titlebar, WindowOptions, pick_format, surface_config,
};
pub use platform::{
    Clipboard, ImeState, MemoryClipboard, SystemClipboard, cursor_icon, service_clipboard,
};
pub use renderer::{RenderTarget, SceneRenderer};

pub use winit;
