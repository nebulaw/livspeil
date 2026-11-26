// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Whether backdrop blur can run, and how (decision D63).
//!
//! The decision is a pure function of what the device check found and of
//! the target's format and usage, so it is tested without a GPU. The
//! reason strings are for logs, the gallery's Materials page and
//! `--gpu-info`; nothing parses them.

/// Whether the renderer can blur backdrops on this device.
///
/// Found once, when the [`Renderer`](crate::Renderer) is created: the blur
/// pipelines and a probe of the blur level format are built inside a
/// validation error scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlurSupport {
    /// The blur pipelines were created and the level format works.
    Supported,
    /// Blur cannot be used; backdrops must be drawn as plain opaque quads
    /// (see [`DisplayList::without_backdrops`](crate::DisplayList::without_backdrops)).
    /// The text says why.
    Fallback(String),
}

impl BlurSupport {
    /// Whether blur works.
    pub fn is_supported(&self) -> bool {
        matches!(self, BlurSupport::Supported)
    }

    /// The reason blur is off, if it is.
    pub fn fallback_reason(&self) -> Option<&str> {
        match self {
            BlurSupport::Supported => None,
            BlurSupport::Fallback(r) => Some(r),
        }
    }
}

/// How a frame with backdrops reaches a given target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlurPath {
    /// Draw straight into the target and sample it (the target has
    /// `TEXTURE_BINDING`). Vulkan swapchains.
    Direct,
    /// Draw into an internal texture that can be sampled, then copy it to
    /// the target with a blit pass (the target is render-only). Metal
    /// drawables: wgpu's Metal surfaces do not offer `TEXTURE_BINDING`.
    Offscreen,
    /// No blur: the text says why.
    Fallback(String),
}

/// Chooses the blur path for a target of `format` and `usage`.
///
/// - Device check failed: `Fallback` with that reason.
/// - Format not `Bgra8Unorm` / `Rgba8Unorm` (the renderer writes
///   gamma-encoded sRGB; an sRGB format would encode twice): `Fallback`.
/// - Target not a render attachment: `Fallback` (not even a blit works).
/// - Target samplable: `Direct`; otherwise `Offscreen`.
pub fn choose_blur_path(
    device: &BlurSupport,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> BlurPath {
    if let BlurSupport::Fallback(reason) = device {
        return BlurPath::Fallback(reason.clone());
    }
    if !matches!(
        format,
        wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm
    ) {
        return BlurPath::Fallback(format!(
            "target format {format:?} is not Bgra8Unorm or Rgba8Unorm"
        ));
    }
    if !usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT) {
        return BlurPath::Fallback("target is not a render attachment".to_owned());
    }
    if usage.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
        BlurPath::Direct
    } else {
        BlurPath::Offscreen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{TextureFormat as F, TextureUsages as U};

    #[test]
    fn samplable_target_is_direct() {
        let u = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        assert_eq!(
            choose_blur_path(&BlurSupport::Supported, F::Bgra8Unorm, u),
            BlurPath::Direct
        );
    }

    #[test]
    fn render_only_target_goes_through_the_offscreen_copy() {
        // What a Metal drawable offers (wgpu-hal 30 reports no RESOURCE).
        let u = U::RENDER_ATTACHMENT | U::COPY_SRC | U::COPY_DST;
        assert_eq!(
            choose_blur_path(&BlurSupport::Supported, F::Bgra8Unorm, u),
            BlurPath::Offscreen
        );
    }

    #[test]
    fn unusable_targets_fall_back_with_a_reason() {
        let ok = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        for (format, usage) in [
            (F::Bgra8UnormSrgb, ok),
            (F::Rgba16Float, ok),
            (F::Bgra8Unorm, U::COPY_DST),
        ] {
            let p = choose_blur_path(&BlurSupport::Supported, format, usage);
            assert!(
                matches!(&p, BlurPath::Fallback(r) if !r.is_empty()),
                "{p:?}"
            );
        }
    }

    #[test]
    fn a_failed_device_check_wins_over_a_good_target() {
        let bad = BlurSupport::Fallback("pipeline failed".to_owned());
        let u = U::RENDER_ATTACHMENT | U::TEXTURE_BINDING;
        assert_eq!(
            choose_blur_path(&bad, F::Bgra8Unorm, u),
            BlurPath::Fallback("pipeline failed".to_owned())
        );
        assert_eq!(bad.fallback_reason(), Some("pipeline failed"));
        assert!(!bad.is_supported());
        assert!(BlurSupport::Supported.is_supported());
    }
}
