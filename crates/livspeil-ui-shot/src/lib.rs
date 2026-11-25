// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Headless screenshots and golden-image tests for Livspeil UI.
//!
//! - [`scenes`]: named display lists at a fixed size and scale.
//! - [`render_scene`]: renders one with the CPU reference or the GPU.
//! - [`compare`]: per-channel delta plus a maximum fraction of differing
//!   pixels, with a diff image.
//! - [`check_golden`]: compares against `goldens/<name>.png`, writes
//!   `actual` and `diff` images on failure, and accepts the new image with
//!   `UPDATE_GOLDEN=1`.
//!
//! Golden strategy: goldens are rendered by the deterministic CPU reference
//! rasterizer (no GPU needed, identical on every machine up to libm
//! rounding). GPU output is compared against the same goldens with a looser
//! tolerance: [`Tolerance::GPU_SOFTWARE`] for llvmpipe/lavapipe (CI Linux),
//! [`Tolerance::GPU_HARDWARE`] for real GPUs (macOS Metal).

#![forbid(unsafe_code)]

pub mod containers;
pub mod gallery;
pub mod scenes;
pub mod ui;

use std::fmt;
use std::path::{Path, PathBuf};

use livspeil_ui_render::{
    AtlasStore, FrameOptions, GpuContext, RenderError, Renderer, RgbaImage, TARGET_FORMAT, encode,
    render_cpu_with,
};

pub use scenes::{Scene, Shot, find, scenes};

/// Errors from rendering, PNG I/O or golden checks.
#[derive(Debug)]
pub enum ShotError {
    /// Rendering failed.
    Render(RenderError),
    /// Reading or writing a file failed.
    Io(PathBuf, std::io::Error),
    /// PNG encoding failed.
    PngEncode(String),
    /// PNG decoding failed or the PNG is not 8-bit RGBA.
    PngDecode(String),
    /// Building a UI scene (core views, paint bridge) failed.
    Scene(String),
}

impl fmt::Display for ShotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShotError::Render(e) => write!(f, "{e}"),
            ShotError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            ShotError::PngEncode(e) => write!(f, "PNG encoding failed: {e}"),
            ShotError::PngDecode(e) => write!(f, "PNG decoding failed: {e}"),
            ShotError::Scene(e) => write!(f, "building the scene failed: {e}"),
        }
    }
}

impl std::error::Error for ShotError {}

impl From<RenderError> for ShotError {
    fn from(e: RenderError) -> Self {
        ShotError::Render(e)
    }
}

/// Frame options for a scene.
pub fn frame_options(scene: &Scene) -> FrameOptions {
    FrameOptions {
        width: (scene.width as f32 * scene.scale).round() as u32,
        height: (scene.height as f32 * scene.scale).round() as u32,
        scale: scene.scale,
        clear: (scene.clear)(),
    }
}

/// Renders a scene with the CPU reference rasterizer.
pub fn render_scene_cpu(scene: &Scene) -> Result<RgbaImage, ShotError> {
    let shot = (scene.build)(scene.scale)?;
    let frame = encode(&shot.list, frame_options(scene))?;
    let mut atlases = AtlasStore::new();
    atlases.apply_all(&shot.atlas)?;
    Ok(render_cpu_with(&frame, &atlases))
}

/// A GPU device with a renderer, reusable across scenes.
#[derive(Debug)]
pub struct GpuShooter {
    /// Device and queue.
    pub context: GpuContext,
    renderer: Renderer,
}

impl GpuShooter {
    /// Opens a headless device. [`RenderError::NoAdapter`] when there is no
    /// adapter at all.
    pub fn new() -> Result<GpuShooter, ShotError> {
        let context = GpuContext::headless()?;
        let renderer = Renderer::new(&context.device, TARGET_FORMAT);
        Ok(GpuShooter { context, renderer })
    }

    /// Renders a scene on the GPU and reads it back.
    pub fn render(&mut self, scene: &Scene) -> Result<RgbaImage, ShotError> {
        let shot = (scene.build)(scene.scale)?;
        let frame = encode(&shot.list, frame_options(scene))?;
        self.renderer
            .update_atlases(&self.context.device, &self.context.queue, &shot.atlas)?;
        Ok(self.context.render_to_image(&mut self.renderer, &frame)?)
    }

    /// The tolerance against CPU-rendered goldens for this adapter.
    pub fn tolerance(&self) -> Tolerance {
        if self.context.is_software() {
            Tolerance::GPU_SOFTWARE
        } else {
            Tolerance::GPU_HARDWARE
        }
    }

    /// Adapter name and backend, for logs.
    pub fn describe(&self) -> String {
        let i = &self.context.adapter_info;
        format!("{} ({:?}, {:?})", i.name, i.backend, i.device_type)
    }
}

/// Opens a GPU shooter, or prints why GPU checks are skipped and returns
/// `None` when no adapter exists. Other errors are returned.
pub fn gpu_or_skip() -> Result<Option<GpuShooter>, ShotError> {
    match GpuShooter::new() {
        Ok(s) => Ok(Some(s)),
        Err(ShotError::Render(RenderError::NoAdapter(e))) => {
            eprintln!(
                "SKIPPING GPU rendering: no wgpu adapter ({e}). On Linux install a software \
                 driver (mesa-vulkan-drivers for lavapipe, or Mesa EGL for llvmpipe)."
            );
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Encodes an image as an sRGB-tagged 8-bit RGBA PNG.
pub fn encode_png(img: &RgbaImage) -> Result<Vec<u8>, ShotError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        enc.set_compression(png::Compression::High);
        let mut writer = enc
            .write_header()
            .map_err(|e| ShotError::PngEncode(e.to_string()))?;
        writer
            .write_image_data(&img.data)
            .map_err(|e| ShotError::PngEncode(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| ShotError::PngEncode(e.to_string()))?;
    }
    Ok(out)
}

/// Decodes an 8-bit RGBA PNG.
pub fn decode_png(bytes: &[u8]) -> Result<RgbaImage, ShotError> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder
        .read_info()
        .map_err(|e| ShotError::PngDecode(e.to_string()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| ShotError::PngDecode("image too large".into()))?;
    let mut buf = vec![0; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| ShotError::PngDecode(e.to_string()))?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
        return Err(ShotError::PngDecode(format!(
            "expected 8-bit RGBA, found {:?} {:?}",
            info.bit_depth, info.color_type
        )));
    }
    buf.truncate(info.buffer_size());
    Ok(RgbaImage {
        width: info.width,
        height: info.height,
        data: buf,
    })
}

/// Writes a PNG, creating parent directories.
pub fn write_png(path: &Path, img: &RgbaImage) -> Result<(), ShotError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ShotError::Io(dir.to_path_buf(), e))?;
    }
    let bytes = encode_png(img)?;
    std::fs::write(path, bytes).map_err(|e| ShotError::Io(path.to_path_buf(), e))
}

/// Reads a PNG.
pub fn read_png(path: &Path) -> Result<RgbaImage, ShotError> {
    let bytes = std::fs::read(path).map_err(|e| ShotError::Io(path.to_path_buf(), e))?;
    decode_png(&bytes)
}

/// How different two images may be.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// A pixel differs when any channel differs by more than this.
    pub max_channel_delta: u8,
    /// At most this fraction of pixels may differ.
    pub max_differing_fraction: f64,
}

impl Tolerance {
    /// CPU reference vs CPU-rendered golden: allows libm rounding
    /// differences between platforms (macOS vs Linux) and nothing else.
    pub const CPU: Tolerance = Tolerance {
        max_channel_delta: 2,
        max_differing_fraction: 0.001,
    };
    /// Software GPU (llvmpipe, lavapipe) vs CPU-rendered golden: texture
    /// filtering precision, f16 blur levels, fast transcendental functions.
    pub const GPU_SOFTWARE: Tolerance = Tolerance {
        max_channel_delta: 4,
        max_differing_fraction: 0.002,
    };
    /// Hardware GPU (Metal on macOS) vs CPU-rendered golden.
    pub const GPU_HARDWARE: Tolerance = Tolerance {
        max_channel_delta: 8,
        max_differing_fraction: 0.01,
    };
}

/// Result of [`compare`].
#[derive(Debug, Clone)]
pub struct Comparison {
    /// Largest channel difference anywhere.
    pub max_delta: u8,
    /// Pixels whose difference exceeds the tolerance's channel delta.
    pub differing: usize,
    /// All pixels.
    pub total: usize,
    /// Whether the images have the same size.
    pub same_size: bool,
    /// Whether the comparison passes the tolerance.
    pub passed: bool,
    /// Diff image: the expected image dimmed to 25% grey, differing pixels
    /// in red (brighter = larger delta), sub-threshold differences in blue.
    pub diff: RgbaImage,
}

impl Comparison {
    /// Fraction of differing pixels.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.differing as f64 / self.total as f64
        }
    }
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.same_size {
            return write!(f, "image sizes differ");
        }
        write!(
            f,
            "max channel delta {}, {} of {} pixels differ ({:.4}%)",
            self.max_delta,
            self.differing,
            self.total,
            self.fraction() * 100.0
        )
    }
}

/// Compares two images.
pub fn compare(actual: &RgbaImage, expected: &RgbaImage, tol: Tolerance) -> Comparison {
    let same_size = actual.width == expected.width && actual.height == expected.height;
    let mut diff = RgbaImage::new(expected.width, expected.height);
    if !same_size {
        return Comparison {
            max_delta: 255,
            differing: expected.data.len() / 4,
            total: expected.data.len() / 4,
            same_size,
            passed: false,
            diff,
        };
    }
    let mut max_delta = 0u8;
    let mut differing = 0;
    for ((a, e), d) in actual
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.data.as_chunks::<4>().0)
        .zip(diff.data.as_chunks_mut::<4>().0)
    {
        let delta = a
            .iter()
            .zip(e)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap_or(0);
        max_delta = max_delta.max(delta);
        let grey = ((u16::from(e[0]) + u16::from(e[1]) + u16::from(e[2])) / 12) as u8;
        if delta > tol.max_channel_delta {
            differing += 1;
            d.copy_from_slice(&[128u8.saturating_add(delta / 2), 0, 0, 255]);
        } else if delta > 0 {
            d.copy_from_slice(&[grey, grey, 160, 255]);
        } else {
            d.copy_from_slice(&[grey, grey, grey, 255]);
        }
    }
    let total = actual.data.len() / 4;
    let passed = (differing as f64) <= tol.max_differing_fraction * total as f64;
    Comparison {
        max_delta,
        differing,
        total,
        same_size,
        passed,
        diff,
    }
}

/// Outcome of [`check_golden`].
#[derive(Debug)]
pub enum GoldenOutcome {
    /// Matches within tolerance.
    Matched(Comparison),
    /// `UPDATE_GOLDEN=1`: the golden was (re)written.
    Updated,
    /// Does not match; `actual` and `diff` images were written.
    Mismatch {
        /// The comparison.
        comparison: Comparison,
        /// Where the actual image was written.
        actual: PathBuf,
        /// Where the diff image was written.
        diff: PathBuf,
    },
    /// No golden exists (run with `UPDATE_GOLDEN=1` to create it).
    Missing(PathBuf),
}

/// Directory of the committed goldens.
pub fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("goldens")
}

/// The workspace's `target/shots` directory (or `$CARGO_TARGET_DIR/shots`).
pub fn shots_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(t) => PathBuf::from(t).join("shots"),
        None => {
            let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
            let root = manifest.parent().and_then(Path::parent).unwrap_or(manifest);
            root.join("target").join("shots")
        }
    }
}

/// Whether `UPDATE_GOLDEN` is set to something other than `0`.
pub fn update_requested() -> bool {
    std::env::var("UPDATE_GOLDEN").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Compares `img` with `goldens/<name>.png`. With `update` the golden is
/// written instead. On mismatch, writes `<out>/<name>.<tag>.actual.png`
/// and `.diff.png`.
pub fn check_golden(
    name: &str,
    tag: &str,
    img: &RgbaImage,
    tol: Tolerance,
    update: bool,
    out: &Path,
) -> Result<GoldenOutcome, ShotError> {
    let path = golden_dir().join(format!("{name}.png"));
    if update {
        write_png(&path, img)?;
        return Ok(GoldenOutcome::Updated);
    }
    if !path.exists() {
        return Ok(GoldenOutcome::Missing(path));
    }
    let expected = read_png(&path)?;
    let comparison = compare(img, &expected, tol);
    if comparison.passed {
        return Ok(GoldenOutcome::Matched(comparison));
    }
    let actual = out.join(format!("{name}.{tag}.actual.png"));
    let diff = out.join(format!("{name}.{tag}.diff.png"));
    write_png(&actual, img)?;
    write_png(&diff, &comparison.diff)?;
    Ok(GoldenOutcome::Mismatch {
        comparison,
        actual,
        diff,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let mut img = RgbaImage::new(3, 2);
        img.data[0..4].copy_from_slice(&[1, 2, 3, 4]);
        let back = decode_png(&encode_png(&img).unwrap()).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn compare_counts_pixels_over_threshold() {
        let a = RgbaImage::new(10, 10);
        let mut b = a.clone();
        b.data[0] = 3;
        b.data[4] = 10;
        let c = compare(
            &a,
            &b,
            Tolerance {
                max_channel_delta: 4,
                max_differing_fraction: 0.0,
            },
        );
        assert_eq!((c.max_delta, c.differing, c.passed), (10, 1, false));
        let c = compare(
            &a,
            &b,
            Tolerance {
                max_channel_delta: 4,
                max_differing_fraction: 0.01,
            },
        );
        assert!(c.passed);
    }
}
