// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Frame-time bookkeeping for the spike's measurements.

use std::time::Duration;

/// One rendered frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameSample {
    /// CPU time to build and submit the frame (draw calls recorded, buffers
    /// swapped), in milliseconds.
    pub cpu_ms: f64,
    /// CPU time until the GPU finished the frame (fence wait or 1x1
    /// read-back), when sampled; includes `cpu_ms`.
    pub gpu_ms: Option<f64>,
}

/// Collected frame samples.
#[derive(Debug, Default, Clone)]
pub struct FrameStats {
    samples: Vec<FrameSample>,
}

/// Percentile summary of one series (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub count: usize,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "n={} mean={:.2} p50={:.2} p95={:.2} max={:.2} ms",
            self.count, self.mean, self.p50, self.p95, self.max
        )
    }
}

/// Nearest-rank percentile of a sorted, non-empty slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

fn summarize(mut v: Vec<f64>) -> Option<Summary> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(Summary {
        count: v.len(),
        mean: v.iter().sum::<f64>() / v.len() as f64,
        p50: percentile(&v, 50.0),
        p95: percentile(&v, 95.0),
        max: v[v.len() - 1],
    })
}

impl FrameStats {
    pub fn push(&mut self, cpu: Duration, gpu: Option<Duration>) {
        self.samples.push(FrameSample {
            cpu_ms: cpu.as_secs_f64() * 1e3,
            gpu_ms: gpu.map(|d| d.as_secs_f64() * 1e3),
        });
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn cpu(&self) -> Option<Summary> {
        summarize(self.samples.iter().map(|s| s.cpu_ms).collect())
    }

    pub fn gpu(&self) -> Option<Summary> {
        summarize(self.samples.iter().filter_map(|s| s.gpu_ms).collect())
    }

    /// Drop the first `n` samples (shader compilation, texture upload).
    pub fn skip_warmup(&mut self, n: usize) {
        let n = n.min(self.samples.len());
        self.samples.drain(..n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_nearest_rank() {
        let mut s = FrameStats::default();
        for i in 1..=100 {
            s.push(Duration::from_micros(i * 1000), None);
        }
        let c = s.cpu().expect("samples");
        assert_eq!(c.count, 100);
        assert!((c.p50 - 50.0).abs() < 1e-9);
        assert!((c.p95 - 95.0).abs() < 1e-9);
        assert!((c.max - 100.0).abs() < 1e-9);
        assert!(s.gpu().is_none());
    }

    #[test]
    fn warmup_is_skipped() {
        let mut s = FrameStats::default();
        s.push(Duration::from_millis(100), Some(Duration::from_millis(120)));
        s.push(Duration::from_millis(2), Some(Duration::from_millis(3)));
        s.skip_warmup(1);
        assert_eq!(s.len(), 1);
        assert!((s.gpu().expect("gpu").mean - 3.0).abs() < 1e-9);
    }

    #[test]
    fn empty_has_no_summary() {
        assert!(FrameStats::default().cpu().is_none());
    }
}
