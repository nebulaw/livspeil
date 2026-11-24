// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Host errors.

use std::fmt;

use livspeil_ui_render::RenderError;

/// Errors from the desktop host.
#[derive(Debug)]
pub enum HostError {
    /// The event loop could not be created or failed while running.
    EventLoop(String),
    /// The window could not be created.
    Window(String),
    /// A wgpu surface could not be created for the window.
    Surface(String),
    /// No GPU adapter can present to the window.
    NoAdapter(String),
    /// The device could not be created.
    Device(String),
    /// The surface cannot be configured (no supported format).
    SurfaceConfig(&'static str),
    /// The renderer failed.
    Render(RenderError),
    /// A [`SceneRenderer`](crate::SceneRenderer) from another crate failed.
    Renderer(Box<dyn std::error::Error + Send + Sync>),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::EventLoop(e) => write!(f, "event loop: {e}"),
            HostError::Window(e) => write!(f, "creating the window: {e}"),
            HostError::Surface(e) => write!(f, "creating the surface: {e}"),
            HostError::NoAdapter(e) => write!(f, "no GPU adapter can present to the window: {e}"),
            HostError::Device(e) => write!(f, "GPU device: {e}"),
            HostError::SurfaceConfig(e) => write!(f, "configuring the surface: {e}"),
            HostError::Render(e) => write!(f, "rendering: {e}"),
            HostError::Renderer(e) => write!(f, "scene renderer: {e}"),
        }
    }
}

impl std::error::Error for HostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            HostError::Render(e) => Some(e),
            HostError::Renderer(e) => Some(e.as_ref()),
            _ => None,
        }
    }
}

impl From<RenderError> for HostError {
    fn from(e: RenderError) -> Self {
        HostError::Render(e)
    }
}
