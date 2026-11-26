// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Frame timing statistics for the frame HUD and the frame log.
//!
//! Everything here is plain data and arithmetic (no window, no GPU), so the
//! statistics are unit-tested headlessly. The host feeds one
//! [`FrameSample`] per presented frame to a [`FrameStats`]; the HUD shows
//! [`Summary::hud_lines`] of the last [`HUD_WINDOW`] frames, and the frame
//! log gets [`FrameStats::csv`] and a summary of the whole run on demand.
//!
//! The phases of one frame, in the order the host runs them:
//!
//! | phase     | what                                                        |
//! |-----------|-------------------------------------------------------------|
//! | `build`   | `App::draw_frame`: render, diff, layout, core paint list    |
//! | `acquire` | `Surface::get_current_texture` (blocks on vsync under FIFO) |
//! | `paint`   | scene to display list, glyph atlas uploads                  |
//! | `encode`  | display list to GPU instance data                           |
//! | `submit`  | recording and submitting the GPU commands                   |
//! | `present` | `Queue::present`                                            |
//!
//! The *interval* is the time between two presents. Gaps longer than
//! [`IDLE_GAP`] (the window was idle: the UI draws nothing when nothing
//! changes) are not intervals and are left out of the statistics.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Frames the HUD summarises.
pub const HUD_WINDOW: usize = 120;

/// A gap between two presents above this is idle time, not a slow frame.
pub const IDLE_GAP: Duration = Duration::from_millis(250);

/// Frames kept for the log (about a minute at 240 Hz).
pub const DEFAULT_CAPACITY: usize = 16_384;

/// A frame is late when its interval exceeds this multiple of the median.
pub const LATE_FACTOR: f64 = 1.5;

/// What the host measured for one presented frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameSample {
    /// Time since the host started, at the end of `present`.
    pub at: Duration,
    /// Whether the app built a new scene (false: an expose or similar
    /// redraw re-rendered the last scene).
    pub rebuilt: bool,
    /// `App::draw_frame`.
    pub build: Duration,
    /// Acquiring the surface texture.
    pub acquire: Duration,
    /// Renderer: scene to display list.
    pub paint: Duration,
    /// Renderer: display list to instances.
    pub encode: Duration,
    /// Renderer: GPU recording and submit.
    pub submit: Duration,
    /// `Queue::present`.
    pub present: Duration,
    /// Display list items (0 when the renderer does not report).
    pub items: u32,
    /// GPU instances.
    pub instances: u32,
    /// Draw calls.
    pub draw_calls: u32,
    /// Bytes of instance data uploaded.
    pub upload_bytes: u32,
}

/// A stored frame: the sample plus its interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameRecord {
    /// Frame number since the host started (counts every presented frame).
    pub seq: u64,
    /// Time since the previous present; `None` for the first frame and
    /// after an idle gap.
    pub interval: Option<Duration>,
    /// The measurements.
    pub sample: FrameSample,
}

/// Average, 95th percentile and maximum of a series.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Dist {
    /// Mean.
    pub avg: Duration,
    /// 95th percentile (nearest rank).
    pub p95: Duration,
    /// Maximum.
    pub max: Duration,
}

impl Dist {
    /// Statistics of `values` (all zero when empty).
    #[must_use]
    pub fn of(values: &[Duration]) -> Dist {
        if values.is_empty() {
            return Dist::default();
        }
        let mut v = values.to_vec();
        v.sort_unstable();
        let sum: Duration = v.iter().sum();
        // Nearest rank: ceil(0.95 * n), 1-based.
        let rank = (v.len() * 95).div_ceil(100).clamp(1, v.len());
        Dist {
            avg: sum / u32::try_from(v.len()).unwrap_or(u32::MAX),
            p95: v[rank - 1],
            max: v[v.len() - 1],
        }
    }
}

/// Statistics over a run of frames.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    /// Frames summarised.
    pub frames: usize,
    /// Of those, frames with an interval.
    pub intervals: usize,
    /// Frames per second while drawing (reciprocal of the mean interval).
    pub fps: f64,
    /// Median interval.
    pub median: Duration,
    /// Intervals.
    pub interval: Dist,
    /// Frames whose interval exceeds [`LATE_FACTOR`] times the median.
    pub late: usize,
    /// Frames that re-rendered the last scene without rebuilding.
    pub rerenders: usize,
    /// Build phase.
    pub build: Dist,
    /// Acquire phase.
    pub acquire: Dist,
    /// Paint phase.
    pub paint: Dist,
    /// Encode phase.
    pub encode: Dist,
    /// Submit phase.
    pub submit: Dist,
    /// Present phase.
    pub present: Dist,
    /// The last frame's display list items, instances, draw calls, bytes.
    pub last_items: (u32, u32, u32, u32),
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

impl Summary {
    /// Statistics of `records`.
    #[must_use]
    pub fn of<'a>(records: impl IntoIterator<Item = &'a FrameRecord>) -> Summary {
        let rs: Vec<&FrameRecord> = records.into_iter().collect();
        let intervals: Vec<Duration> = rs.iter().filter_map(|r| r.interval).collect();
        let phase = |f: fn(&FrameSample) -> Duration| {
            Dist::of(&rs.iter().map(|r| f(&r.sample)).collect::<Vec<_>>())
        };
        let mut sorted = intervals.clone();
        sorted.sort_unstable();
        let median = sorted.get(sorted.len() / 2).copied().unwrap_or_default();
        let limit = median.mul_f64(LATE_FACTOR);
        let interval = Dist::of(&intervals);
        let fps = if interval.avg > Duration::ZERO {
            1.0 / interval.avg.as_secs_f64()
        } else {
            0.0
        };
        let last = rs.last().map(|r| r.sample).unwrap_or_default();
        Summary {
            frames: rs.len(),
            intervals: intervals.len(),
            fps,
            median,
            interval,
            late: intervals.iter().filter(|i| **i > limit).count(),
            rerenders: rs.iter().filter(|r| !r.sample.rebuilt).count(),
            build: phase(|s| s.build),
            acquire: phase(|s| s.acquire),
            paint: phase(|s| s.paint),
            encode: phase(|s| s.encode),
            submit: phase(|s| s.submit),
            present: phase(|s| s.present),
            last_items: (
                last.items,
                last.instances,
                last.draw_calls,
                last.upload_bytes,
            ),
        }
    }

    /// The CPU time of the phases that run on this thread before present
    /// (build, paint, encode, submit), summed over their means.
    #[must_use]
    pub fn cpu_mean(&self) -> Duration {
        self.build.avg + self.paint.avg + self.encode.avg + self.submit.avg
    }

    /// The summary as lines of text (the log and the HUD's first lines).
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        out.push(format!(
            "{} frames ({} with an interval), {:.1} fps while drawing",
            self.frames, self.intervals, self.fps
        ));
        out.push(format!(
            "interval ms: avg {:.2}  p95 {:.2}  max {:.2}  median {:.2}",
            ms(self.interval.avg),
            ms(self.interval.p95),
            ms(self.interval.max),
            ms(self.median)
        ));
        out.push(format!(
            "late (> {LATE_FACTOR}x median): {} of {}   re-render only: {}",
            self.late, self.intervals, self.rerenders
        ));
        let row = |name: &str, d: Dist| {
            format!(
                "{name:<8} avg {:>6.2}  p95 {:>6.2}  max {:>6.2} ms",
                ms(d.avg),
                ms(d.p95),
                ms(d.max)
            )
        };
        out.push(row("build", self.build));
        out.push(row("paint", self.paint));
        out.push(row("encode", self.encode));
        out.push(row("submit", self.submit));
        out.push(row("acquire", self.acquire));
        out.push(row("present", self.present));
        let (items, instances, draws, bytes) = self.last_items;
        out.push(format!(
            "last frame: {items} items, {instances} instances, {draws} draws, {:.2} MB uploaded",
            f64::from(bytes) / 1e6
        ));
        out
    }

    /// The HUD's lines for this window: short, with every phase on one
    /// row.
    #[must_use]
    pub fn hud_lines(&self) -> Vec<String> {
        let d = |d: Dist| format!("{:.2}/{:.2}", ms(d.avg), ms(d.max));
        let (items, instances, draws, bytes) = self.last_items;
        vec![
            format!(
                "{:.1} fps   interval ms avg {:.2}  p95 {:.2}  max {:.2}",
                self.fps,
                ms(self.interval.avg),
                ms(self.interval.p95),
                ms(self.interval.max)
            ),
            format!(
                "late (>{LATE_FACTOR}x median {:.1} ms): {} of {}   re-render only {}",
                ms(self.median),
                self.late,
                self.intervals,
                self.rerenders
            ),
            format!(
                "ms avg/max  build {}  paint {}  encode {}",
                d(self.build),
                d(self.paint),
                d(self.encode)
            ),
            format!(
                "ms avg/max  submit {}  acquire {}  present {}",
                d(self.submit),
                d(self.acquire),
                d(self.present)
            ),
            format!(
                "{items} items  {instances} instances  {draws} draws  {:.2} MB",
                f64::from(bytes) / 1e6
            ),
        ]
    }
}

/// A ring buffer of the most recent frames, with run totals.
#[derive(Clone, Debug)]
pub struct FrameStats {
    records: VecDeque<FrameRecord>,
    capacity: usize,
    last_at: Option<Duration>,
    seq: u64,
    skipped: u64,
}

impl Default for FrameStats {
    fn default() -> Self {
        FrameStats::new(DEFAULT_CAPACITY)
    }
}

impl FrameStats {
    /// A buffer keeping the last `capacity` frames (at least 2).
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        FrameStats {
            records: VecDeque::with_capacity(capacity.clamp(2, DEFAULT_CAPACITY)),
            capacity: capacity.max(2),
            last_at: None,
            seq: 0,
            skipped: 0,
        }
    }

    /// Add a presented frame.
    pub fn record(&mut self, sample: FrameSample) {
        let interval = self
            .last_at
            .and_then(|t| sample.at.checked_sub(t))
            .filter(|d| *d <= IDLE_GAP);
        self.last_at = Some(sample.at);
        if self.records.len() == self.capacity {
            self.records.pop_front();
        }
        self.records.push_back(FrameRecord {
            seq: self.seq,
            interval,
            sample,
        });
        self.seq += 1;
    }

    /// Count a redraw that drew nothing (occluded, surface unavailable).
    pub fn skip(&mut self) {
        self.skipped += 1;
    }

    /// Redraws that drew nothing.
    #[must_use]
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Frames presented since the start.
    #[must_use]
    pub fn total_frames(&self) -> u64 {
        self.seq
    }

    /// Frames currently stored.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no frame is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The stored frames, oldest first.
    pub fn records(&self) -> impl Iterator<Item = &FrameRecord> {
        self.records.iter()
    }

    /// Summary of the last `n` frames.
    #[must_use]
    pub fn summary(&self, n: usize) -> Summary {
        Summary::of(
            self.records
                .iter()
                .skip(self.records.len().saturating_sub(n)),
        )
    }

    /// Summary of every stored frame.
    #[must_use]
    pub fn summary_all(&self) -> Summary {
        self.summary(usize::MAX)
    }

    /// Per-frame CSV lines (with a header) for the frames of the last
    /// `seconds` of the run.
    #[must_use]
    pub fn csv(&self, seconds: f64) -> String {
        let mut out = String::from(CSV_HEADER);
        out.push('\n');
        let Some(end) = self.records.back().map(|r| r.sample.at) else {
            return out;
        };
        let start = end.saturating_sub(Duration::from_secs_f64(seconds.max(0.0)));
        for r in self.records.iter().filter(|r| r.sample.at >= start) {
            let s = &r.sample;
            let us = |d: Duration| d.as_micros();
            let _ = writeln!(
                out,
                "{},{:.3},{},{},{},{},{},{},{},{},{},{},{},{}",
                r.seq,
                s.at.as_secs_f64(),
                r.interval.map_or(String::new(), |i| us(i).to_string()),
                u8::from(s.rebuilt),
                us(s.build),
                us(s.paint),
                us(s.encode),
                us(s.submit),
                us(s.acquire),
                us(s.present),
                s.items,
                s.instances,
                s.draw_calls,
                s.upload_bytes
            );
        }
        out
    }
}

/// The header line of [`FrameStats::csv`] (times in microseconds).
pub const CSV_HEADER: &str = "seq,t_s,interval_us,rebuilt,build_us,paint_us,encode_us,submit_us,acquire_us,present_us,items,instances,draws,upload_bytes";

/// What the host knows about the graphics stack: shown by the HUD and
/// written at the top of the frame log.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GpuReport {
    /// wgpu backend (`Vulkan`, `Metal`, `Gl`, ...).
    pub backend: String,
    /// Adapter name.
    pub adapter: String,
    /// Device type (`DiscreteGpu`, ...).
    pub device_type: String,
    /// Driver name.
    pub driver: String,
    /// Driver version string.
    pub driver_info: String,
    /// The present mode that was asked for.
    pub present_mode_requested: String,
    /// The present mode in use (the request falls back to FIFO when the
    /// surface does not support it).
    pub present_mode: String,
    /// The modes the surface supports.
    pub present_modes_supported: Vec<String>,
    /// Surface format.
    pub format: String,
    /// Composite alpha mode.
    pub alpha_mode: String,
    /// `desired_maximum_frame_latency`.
    pub frame_latency: u32,
    /// Window scale factor.
    pub scale_factor: f64,
    /// Refresh rate of the window's monitor in millihertz.
    pub refresh_mhz: Option<u32>,
    /// Surface size in physical pixels.
    pub size_px: (u32, u32),
}

impl GpuReport {
    /// The report as lines of text.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        let refresh = self.refresh_mhz.map_or("unknown".to_owned(), |m| {
            format!("{:.2} Hz", f64::from(m) / 1000.0)
        });
        vec![
            format!(
                "backend {}  adapter {} ({})",
                self.backend, self.adapter, self.device_type
            ),
            format!("driver {} {}", self.driver, self.driver_info),
            format!(
                "present mode {} (requested {}; supported {})",
                self.present_mode,
                self.present_mode_requested,
                self.present_modes_supported.join(", ")
            ),
            format!(
                "format {}  alpha {}  frame latency {}",
                self.format, self.alpha_mode, self.frame_latency
            ),
            format!(
                "scale {}  monitor {}  surface {}x{} px",
                self.scale_factor, refresh, self.size_px.0, self.size_px.1
            ),
        ]
    }

    /// Shorter lines for the HUD.
    #[must_use]
    pub fn hud_lines(&self) -> Vec<String> {
        let refresh = self.refresh_mhz.map_or("? Hz".to_owned(), |m| {
            format!("{:.0} Hz", f64::from(m) / 1000.0)
        });
        vec![
            format!("{} | {}", self.backend, self.adapter),
            format!("driver {} {}", self.driver, self.driver_info),
            format!(
                "present {} (asked {})  latency {}",
                self.present_mode, self.present_mode_requested, self.frame_latency
            ),
            format!(
                "{}  scale {}  {}  {}x{}",
                self.format, self.scale_factor, refresh, self.size_px.0, self.size_px.1
            ),
        ]
    }
}

/// Which operating system's conventions [`log_path`] follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatePlatform {
    /// `$XDG_STATE_HOME` or `~/.local/state`.
    Unix,
    /// `~/Library/Logs`.
    MacOs,
    /// `%LOCALAPPDATA%`.
    Windows,
}

impl StatePlatform {
    /// The platform this build runs on.
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            StatePlatform::MacOs
        } else if cfg!(windows) {
            StatePlatform::Windows
        } else {
            StatePlatform::Unix
        }
    }
}

/// Where the frame log named `file` goes: `$XDG_STATE_HOME/livspeil/<file>`
/// (default `~/.local/state/livspeil/<file>`) on Linux and other Unix,
/// `~/Library/Logs/livspeil/<file>` on macOS, `%LOCALAPPDATA%\livspeil\<file>`
/// on Windows. `env` looks up an environment variable. `None` when the
/// home directory is unknown.
#[must_use]
pub fn log_path(
    platform: StatePlatform,
    env: impl Fn(&str) -> Option<String>,
    file: &str,
) -> Option<PathBuf> {
    let non_empty = |k: &str| env(k).filter(|v| !v.is_empty());
    let dir = match platform {
        StatePlatform::Unix => match non_empty("XDG_STATE_HOME") {
            // The XDG spec: a relative path is invalid and ignored.
            Some(p) if Path::new(&p).is_absolute() => PathBuf::from(p),
            _ => PathBuf::from(non_empty("HOME")?).join(".local/state"),
        },
        StatePlatform::MacOs => PathBuf::from(non_empty("HOME")?).join("Library/Logs"),
        StatePlatform::Windows => PathBuf::from(non_empty("LOCALAPPDATA")?),
    };
    Some(dir.join("livspeil").join(file))
}

/// [`log_path`] for this platform and the process environment.
#[must_use]
pub fn default_log_path(file: &str) -> Option<PathBuf> {
    log_path(StatePlatform::current(), |k| std::env::var(k).ok(), file)
}

/// The frame log: a text file the owner can send back.
#[derive(Clone, Debug)]
pub struct FrameLog {
    path: PathBuf,
}

impl FrameLog {
    /// A log at `path` (nothing is written yet).
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        FrameLog { path }
    }

    /// The file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replace the file with `text` (creating the directory).
    ///
    /// # Errors
    /// I/O errors.
    pub fn start(&self, text: &str) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.path, text)
    }

    /// Append `text`.
    ///
    /// # Errors
    /// I/O errors.
    pub fn append(&self, text: &str) -> std::io::Result<()> {
        use std::io::Write as _;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(text.as_bytes())
    }
}

/// The text of a dump: the graphics report, the summary of the whole run
/// and the per-frame CSV of the last `seconds`.
#[must_use]
pub fn dump_text(label: &str, report: &GpuReport, stats: &FrameStats, seconds: f64) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "\n== {label} ==");
    for l in report.lines() {
        let _ = writeln!(out, "{l}");
    }
    let _ = writeln!(
        out,
        "presented {} frames, {} redraws drew nothing; summary of the last {} stored:",
        stats.total_frames(),
        stats.skipped(),
        stats.len()
    );
    for l in stats.summary_all().lines() {
        let _ = writeln!(out, "{l}");
    }
    let _ = writeln!(out, "-- frames of the last {seconds} s (times in us) --");
    out.push_str(&stats.csv(seconds));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn sample(at_ms: u64, build_us: u64) -> FrameSample {
        FrameSample {
            at: d(at_ms),
            rebuilt: true,
            build: Duration::from_micros(build_us),
            ..FrameSample::default()
        }
    }

    #[test]
    fn dist_uses_nearest_rank_for_p95() {
        let v: Vec<Duration> = (1..=20).map(d).collect();
        let dist = Dist::of(&v);
        assert_eq!(dist.max, d(20));
        // ceil(0.95 * 20) = 19th value.
        assert_eq!(dist.p95, d(19));
        assert_eq!(dist.avg, Duration::from_micros(10_500));
        assert_eq!(Dist::of(&[]), Dist::default());
        assert_eq!(Dist::of(&[d(7)]).p95, d(7));
    }

    #[test]
    fn intervals_skip_idle_gaps() {
        let mut s = FrameStats::new(100);
        s.record(sample(0, 0));
        s.record(sample(16, 0));
        s.record(sample(33, 0));
        // The window sat idle for two seconds.
        s.record(sample(2_033, 0));
        s.record(sample(2_050, 0));
        let recs: Vec<_> = s.records().map(|r| r.interval).collect();
        assert_eq!(recs, [None, Some(d(16)), Some(d(17)), None, Some(d(17))]);
        let sum = s.summary_all();
        assert_eq!((sum.frames, sum.intervals), (5, 3));
        assert_eq!(sum.interval.max, d(17));
    }

    #[test]
    fn late_frames_are_counted_against_the_median() {
        let mut s = FrameStats::new(100);
        let mut t = 0;
        for i in 0..40 {
            // Every tenth frame takes 30 ms instead of 10.
            t += if i % 10 == 9 { 30 } else { 10 };
            s.record(sample(t, 0));
        }
        let sum = s.summary_all();
        assert_eq!(sum.median, d(10));
        assert_eq!(sum.late, 4);
        assert_eq!(sum.interval.max, d(30));
        // 1.5x the median is not late (strictly greater is).
        let mut s = FrameStats::new(10);
        for t in [0, 10, 25, 35] {
            s.record(sample(t, 0));
        }
        assert_eq!(s.summary_all().late, 0);
    }

    #[test]
    fn ring_keeps_the_last_frames() {
        let mut s = FrameStats::new(3);
        for i in 0..5 {
            s.record(sample(i * 10, 0));
        }
        assert_eq!(s.len(), 3);
        assert_eq!(s.total_frames(), 5);
        let seqs: Vec<u64> = s.records().map(|r| r.seq).collect();
        assert_eq!(seqs, [2, 3, 4]);
        assert_eq!(s.summary(2).frames, 2);
        assert_eq!(s.summary(10).frames, 3);
    }

    #[test]
    fn rerenders_and_skips_are_counted() {
        let mut s = FrameStats::new(10);
        s.record(sample(0, 0));
        s.record(FrameSample {
            rebuilt: false,
            ..sample(10, 0)
        });
        s.skip();
        s.skip();
        assert_eq!(s.summary_all().rerenders, 1);
        assert_eq!(s.skipped(), 2);
    }

    #[test]
    fn csv_covers_the_requested_window() {
        let mut s = FrameStats::new(100);
        for i in 0..10 {
            s.record(sample(i * 1000, 1500));
        }
        let all = s.csv(100.0);
        assert_eq!(all.lines().count(), 11);
        assert_eq!(all.lines().next(), Some(CSV_HEADER));
        // 2.5 s back from t = 9 s: the frames at 7, 8 and 9 s.
        let last = s.csv(2.5);
        assert_eq!(last.lines().count(), 4);
        let line = last.lines().nth(1).unwrap_or_default();
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields.len(), CSV_HEADER.split(',').count());
        assert_eq!(fields[0], "7");
        assert_eq!(fields[1], "7.000");
        assert_eq!(fields[2], "", "no interval across an idle gap");
        assert_eq!(fields[4], "1500");
        assert_eq!(FrameStats::new(4).csv(1.0), format!("{CSV_HEADER}\n"));
    }

    #[test]
    fn summary_text_mentions_every_phase() {
        let mut s = FrameStats::new(10);
        s.record(sample(0, 1000));
        s.record(sample(16, 3000));
        let text = s.summary_all().lines().join("\n");
        for word in [
            "interval", "late", "build", "paint", "encode", "submit", "acquire", "present",
        ] {
            assert!(text.contains(word), "{word} missing from\n{text}");
        }
        let hud = s.summary(HUD_WINDOW).hud_lines();
        assert!(hud[0].contains("fps"));
        assert!(hud.iter().any(|l| l.contains("build 2.00/3.00")), "{hud:?}");
    }

    #[test]
    fn log_path_follows_platform_conventions() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let f = "gallery-frames.log";
        assert_eq!(
            log_path(
                StatePlatform::Unix,
                env(&[("XDG_STATE_HOME", "/s"), ("HOME", "/h")]),
                f
            ),
            Some(PathBuf::from("/s/livspeil/gallery-frames.log"))
        );
        assert_eq!(
            log_path(StatePlatform::Unix, env(&[("HOME", "/h")]), f),
            Some(PathBuf::from("/h/.local/state/livspeil/gallery-frames.log"))
        );
        // A relative XDG_STATE_HOME is ignored.
        assert_eq!(
            log_path(
                StatePlatform::Unix,
                env(&[("XDG_STATE_HOME", "rel"), ("HOME", "/h")]),
                f
            ),
            Some(PathBuf::from("/h/.local/state/livspeil/gallery-frames.log"))
        );
        assert_eq!(
            log_path(StatePlatform::MacOs, env(&[("HOME", "/Users/a")]), f),
            Some(PathBuf::from(
                "/Users/a/Library/Logs/livspeil/gallery-frames.log"
            ))
        );
        assert_eq!(
            log_path(StatePlatform::Windows, env(&[("LOCALAPPDATA", "C:/L")]), f),
            Some(PathBuf::from("C:/L/livspeil/gallery-frames.log"))
        );
        assert_eq!(log_path(StatePlatform::Unix, env(&[]), f), None);
    }

    #[test]
    fn log_writes_and_appends() {
        let dir = std::env::temp_dir().join(format!("livspeil-frame-log-{}", std::process::id()));
        let log = FrameLog::new(dir.join("nested/gallery-frames.log"));
        assert!(log.start("first\n").is_ok());
        assert!(log.append("second\n").is_ok());
        let text = std::fs::read_to_string(log.path()).unwrap_or_default();
        assert_eq!(text, "first\nsecond\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dump_has_report_summary_and_csv() {
        let mut s = FrameStats::new(10);
        s.record(sample(0, 100));
        s.record(sample(16, 100));
        let report = GpuReport {
            backend: "Vulkan".into(),
            adapter: "Test GPU".into(),
            present_mode: "Fifo".into(),
            present_mode_requested: "Mailbox".into(),
            ..GpuReport::default()
        };
        let text = dump_text("dump 1", &report, &s, 5.0);
        assert!(text.contains("== dump 1 =="));
        assert!(text.contains("Test GPU"));
        assert!(text.contains("present mode Fifo (requested Mailbox"));
        assert!(text.contains(CSV_HEADER));
    }
}
