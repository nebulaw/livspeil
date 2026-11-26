// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! `--gpu-info`: what the GPU stack offers and which blur path the
//! renderer picks, as text a user can paste into a bug report.
//!
//! [`gpu_info`] opens a small window (a surface needs one), collects a
//! [`GpuReport`] and prints nothing itself; [`GpuReport::to_text`] is pure
//! and tested.

use std::fmt::Write as _;
use std::sync::Arc;

use livspeil_ui_render::{BlurPath, BlurSupport, Renderer, choose_blur_path};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use crate::options::{SurfaceOptions, surface_config};
use crate::{HostError, pick_format};

/// What the adapter says about one texture format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatReport {
    /// The format.
    pub format: wgpu::TextureFormat,
    /// Usages the adapter allows.
    pub usages: wgpu::TextureUsages,
    /// Whether it can be sampled with a filtering sampler.
    pub filterable: bool,
}

/// Everything `--gpu-info` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuReport {
    /// Adapter info (backend, name, driver).
    pub adapter: wgpu::AdapterInfo,
    /// Surface formats, in the surface's order.
    pub surface_formats: Vec<wgpu::TextureFormat>,
    /// Usages the surface offers.
    pub surface_usages: wgpu::TextureUsages,
    /// Present modes.
    pub present_modes: Vec<wgpu::PresentMode>,
    /// Alpha modes.
    pub alpha_modes: Vec<wgpu::CompositeAlphaMode>,
    /// The format the host would configure, or why none.
    pub chosen_format: Result<wgpu::TextureFormat, String>,
    /// The usage the host would configure.
    pub chosen_usage: Option<wgpu::TextureUsages>,
    /// Formats the blur depends on.
    pub formats: Vec<FormatReport>,
    /// The renderer's own device check.
    pub blur_support: BlurSupport,
    /// The path the renderer takes for the configured surface.
    pub blur_path: BlurPath,
}

impl GpuReport {
    /// The report as plain text.
    #[must_use]
    pub fn to_text(&self) -> String {
        let a = &self.adapter;
        let mut s = String::new();
        let _ = writeln!(s, "livspeil gpu-info");
        let _ = writeln!(s, "backend:        {:?}", a.backend);
        let _ = writeln!(s, "adapter:        {}", a.name);
        let _ = writeln!(s, "device type:    {:?}", a.device_type);
        let _ = writeln!(s, "driver:         {} {}", a.driver, a.driver_info);
        let _ = writeln!(s, "vendor/device:  {:#06x} / {:#06x}", a.vendor, a.device);
        let _ = writeln!(s, "surface formats:  {:?}", self.surface_formats);
        let _ = writeln!(s, "surface usages:   {:?}", self.surface_usages);
        let _ = writeln!(
            s,
            "  TEXTURE_BINDING: {}",
            yes(self
                .surface_usages
                .contains(wgpu::TextureUsages::TEXTURE_BINDING))
        );
        let _ = writeln!(s, "present modes:    {:?}", self.present_modes);
        let _ = writeln!(s, "alpha modes:      {:?}", self.alpha_modes);
        match &self.chosen_format {
            Ok(f) => {
                let _ = writeln!(s, "chosen format:    {f:?}");
            }
            Err(e) => {
                let _ = writeln!(s, "chosen format:    none ({e})");
            }
        }
        if let Some(u) = self.chosen_usage {
            let _ = writeln!(s, "chosen usage:     {u:?}");
        }
        for f in &self.formats {
            let _ = writeln!(
                s,
                "format {:?}: usages {:?}, filterable {}",
                f.format,
                f.usages,
                yes(f.filterable)
            );
        }
        match &self.blur_support {
            BlurSupport::Supported => {
                let _ = writeln!(s, "blur device check: supported");
            }
            BlurSupport::Fallback(r) => {
                let _ = writeln!(s, "blur device check: FALLBACK ({r})");
            }
        }
        match &self.blur_path {
            BlurPath::Direct => {
                let _ = writeln!(s, "blur path:        direct (surface is sampled)");
            }
            BlurPath::Offscreen => {
                let _ = writeln!(s, "blur path:        offscreen texture + blit");
            }
            BlurPath::Fallback(r) => {
                let _ = writeln!(s, "blur path:        FALLBACK, plain colours ({r})");
            }
        }
        s
    }
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// Opens a window, inspects the GPU stack as the host would use it, and
/// returns the report. Must run on the main thread (macOS).
///
/// # Errors
/// Event loop, window, surface, adapter and device failures.
pub fn gpu_info() -> Result<GpuReport, HostError> {
    let event_loop = EventLoop::new().map_err(|e| HostError::EventLoop(e.to_string()))?;
    let mut app = InfoApp { result: None };
    event_loop
        .run_app(&mut app)
        .map_err(|e| HostError::EventLoop(e.to_string()))?;
    app.result
        .unwrap_or_else(|| Err(HostError::EventLoop("no window event arrived".to_owned())))
}

struct InfoApp {
    result: Option<Result<GpuReport, HostError>>,
}

impl ApplicationHandler for InfoApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.result.is_none() {
            self.result = Some(collect(event_loop));
            event_loop.exit();
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn collect(event_loop: &ActiveEventLoop) -> Result<GpuReport, HostError> {
    let window = Arc::new(
        event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("livspeil gpu-info")
                    .with_inner_size(winit::dpi::LogicalSize::new(320.0, 200.0)),
            )
            .map_err(|e| HostError::Window(e.to_string()))?,
    );
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
        Box::new(event_loop.owned_display_handle()),
    ));
    let surface = instance
        .create_surface(window.clone())
        .map_err(|e| HostError::Surface(e.to_string()))?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface: Some(&surface),
        ..Default::default()
    }))
    .map_err(|e| HostError::NoAdapter(e.to_string()))?;
    let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("livspeil gpu-info"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .map_err(|e| HostError::Device(e.to_string()))?;
    let caps = surface.get_capabilities(&adapter);
    let size = window.inner_size();
    let config = surface_config(
        &caps,
        (size.width, size.height),
        SurfaceOptions::default(),
        false,
    );
    let (chosen_format, chosen_usage) = match &config {
        Ok(c) => (Ok(c.format), Some(c.usage)),
        Err(e) => (Err(e.to_string()), None),
    };
    let format = pick_format(&caps.formats).unwrap_or(wgpu::TextureFormat::Bgra8Unorm);
    let renderer = Renderer::new(&device, format);
    let blur_support = renderer.blur_support().clone();
    let blur_path = choose_blur_path(
        &blur_support,
        format,
        chosen_usage.unwrap_or(wgpu::TextureUsages::empty()),
    );
    let formats = [
        wgpu::TextureFormat::Rgba16Float,
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::R8Unorm,
    ]
    .into_iter()
    .map(|f| {
        let t = adapter.get_texture_format_features(f);
        FormatReport {
            format: f,
            usages: t.allowed_usages,
            filterable: t
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE),
        }
    })
    .collect();
    Ok(GpuReport {
        adapter: adapter.get_info(),
        surface_formats: caps.formats.clone(),
        surface_usages: caps.usages,
        present_modes: caps.present_modes.clone(),
        alpha_modes: caps.alpha_modes.clone(),
        chosen_format,
        chosen_usage,
        formats,
        blur_support,
        blur_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> wgpu::AdapterInfo {
        wgpu::AdapterInfo {
            name: "Test GPU".to_owned(),
            vendor: 0x106b,
            device: 1,
            device_type: wgpu::DeviceType::IntegratedGpu,
            device_pci_bus_id: String::new(),
            driver: "drv".to_owned(),
            driver_info: "1.0".to_owned(),
            backend: wgpu::Backend::Metal,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            limit_bucket: Default::default(),
            transient_saves_memory: None,
        }
    }

    #[test]
    fn report_text_names_the_blur_path_and_surface_usages() {
        let r = GpuReport {
            adapter: info(),
            surface_formats: vec![wgpu::TextureFormat::Bgra8Unorm],
            surface_usages: wgpu::TextureUsages::RENDER_ATTACHMENT,
            present_modes: vec![wgpu::PresentMode::Fifo],
            alpha_modes: vec![wgpu::CompositeAlphaMode::Opaque],
            chosen_format: Ok(wgpu::TextureFormat::Bgra8Unorm),
            chosen_usage: Some(wgpu::TextureUsages::RENDER_ATTACHMENT),
            formats: vec![],
            blur_support: BlurSupport::Supported,
            blur_path: BlurPath::Offscreen,
        };
        let t = r.to_text();
        assert!(t.contains("backend:        Metal"), "{t}");
        assert!(t.contains("TEXTURE_BINDING: no"), "{t}");
        assert!(t.contains("blur path:        offscreen"), "{t}");
    }

    #[test]
    fn fallback_is_loud() {
        let r = GpuReport {
            adapter: info(),
            surface_formats: vec![],
            surface_usages: wgpu::TextureUsages::empty(),
            present_modes: vec![],
            alpha_modes: vec![],
            chosen_format: Err("no format".to_owned()),
            chosen_usage: None,
            formats: vec![],
            blur_support: BlurSupport::Fallback("why".to_owned()),
            blur_path: BlurPath::Fallback("why".to_owned()),
        };
        let t = r.to_text();
        assert!(t.contains("FALLBACK (why)"), "{t}");
        assert!(t.contains("chosen format:    none"), "{t}");
    }
}
