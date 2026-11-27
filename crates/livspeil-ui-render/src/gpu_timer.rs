// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! GPU time of a frame through timestamp queries.
//!
//! Timestamps are written at the start and end of the renderer's frame
//! command buffer, so they cover its draw and blur passes and nothing the CPU
//! does (buffer uploads, which run before the buffer, are not included).
//! Results are read back without ever waiting: they arrive a few frames late,
//! and a frame whose readback slots are all busy is simply not timed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

/// Device features the timer needs.
pub const FEATURES: wgpu::Features =
    wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Readbacks that may be in flight at once.
const SLOTS: usize = 4;
/// Two 64-bit timestamps.
const BYTES: u64 = 16;

const FREE: u8 = 0;
const PENDING: u8 = 1;
const READY: u8 = 2;
const FAILED: u8 = 3;

#[derive(Debug)]
struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

/// Times one command buffer per frame: [`GpuTimer::start`] and
/// [`GpuTimer::stop`] while recording it, [`GpuTimer::submitted`] after it
/// was submitted.
#[derive(Debug)]
pub struct GpuTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    slots: Vec<Slot>,
    /// Nanoseconds per timestamp tick.
    period: f32,
    /// The slot the frame being recorded will resolve into.
    open: Option<usize>,
    latest: Option<Duration>,
}

/// The time between two timestamps `period` nanoseconds a tick apart;
/// `None` when the clock went backwards (it was reset).
fn elapsed(start: u64, end: u64, period: f32) -> Option<Duration> {
    let ticks = end.checked_sub(start)?;
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let nanos = (ticks as f64 * f64::from(period)) as u64;
    Some(Duration::from_nanos(nanos))
}

impl GpuTimer {
    /// A timer, or `None` when `device` lacks [`FEATURES`].
    #[must_use]
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(FEATURES) {
            return None;
        }
        let buffer = |label, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: BYTES,
                usage,
                mapped_at_creation: false,
            })
        };
        Some(GpuTimer {
            queries: device.create_query_set(&wgpu::QuerySetDescriptor {
                label: Some("frame time"),
                ty: wgpu::QueryType::Timestamp,
                count: 2,
            }),
            resolve: buffer(
                "frame time resolve",
                wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            ),
            slots: (0..SLOTS)
                .map(|_| Slot {
                    buffer: buffer(
                        "frame time readback",
                        wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    ),
                    state: Arc::new(AtomicU8::new(FREE)),
                })
                .collect(),
            period: queue.get_timestamp_period(),
            open: None,
            latest: None,
        })
    }

    /// At the start of the frame's command buffer.
    pub fn start(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder) {
        self.collect(device);
        self.open = self
            .slots
            .iter()
            .position(|s| s.state.load(Ordering::Acquire) == FREE);
        if self.open.is_some() {
            enc.write_timestamp(&self.queries, 0);
        }
    }

    /// At the end of the same command buffer.
    pub fn stop(&mut self, enc: &mut wgpu::CommandEncoder) {
        if let Some(slot) = self.open.and_then(|i| self.slots.get(i)) {
            enc.write_timestamp(&self.queries, 1);
            enc.resolve_query_set(&self.queries, 0..2, &self.resolve, 0);
            enc.copy_buffer_to_buffer(&self.resolve, 0, &slot.buffer, 0, BYTES);
        }
    }

    /// After that command buffer was submitted: start the readback.
    pub fn submitted(&mut self) {
        let Some(slot) = self.open.take().and_then(|i| self.slots.get(i)) else {
            return;
        };
        slot.state.store(PENDING, Ordering::Release);
        let state = Arc::clone(&slot.state);
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |r| {
                state.store(if r.is_ok() { READY } else { FAILED }, Ordering::Release);
            });
    }

    /// Take finished readbacks, without waiting for the others.
    fn collect(&mut self, device: &wgpu::Device) {
        // A failed poll only delays the numbers.
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &self.slots {
            match slot.state.load(Ordering::Acquire) {
                READY => {
                    if let Ok(view) = slot.buffer.slice(..).get_mapped_range() {
                        let word = |at: usize| {
                            view.get(at..at + 8)
                                .and_then(|b| b.try_into().ok())
                                .map(u64::from_le_bytes)
                        };
                        if let (Some(start), Some(end)) = (word(0), word(8)) {
                            self.latest = elapsed(start, end, self.period).or(self.latest);
                        }
                    }
                    slot.buffer.unmap();
                    slot.state.store(FREE, Ordering::Release);
                }
                FAILED => slot.state.store(FREE, Ordering::Release),
                _ => {}
            }
        }
    }

    /// The most recently read frame time.
    #[must_use]
    pub fn latest(&self) -> Option<Duration> {
        self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_convert_with_the_period() {
        assert_eq!(elapsed(10, 1010, 1.0), Some(Duration::from_micros(1)));
        assert_eq!(elapsed(0, 1000, 2.5), Some(Duration::from_nanos(2500)));
        assert_eq!(elapsed(10, 5, 1.0), None);
    }

    /// On a real device: a time arrives after a few frames. Skips without an
    /// adapter or timestamp support.
    #[test]
    fn a_device_reports_a_frame_time() {
        let instance = wgpu::Instance::default();
        let Ok(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
        else {
            return;
        };
        if !adapter.features().contains(FEATURES) {
            return;
        }
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_features: FEATURES,
                required_limits: wgpu::Limits::downlevel_defaults(),
                ..Default::default()
            }))
        else {
            return;
        };
        let Some(mut timer) = GpuTimer::new(&device, &queue) else {
            panic!("the device has the features");
        };
        for _ in 0..20 {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            timer.start(&device, &mut enc);
            timer.stop(&mut enc);
            queue.submit([enc.finish()]);
            timer.submitted();
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        }
        assert!(timer.latest().is_some_and(|d| d < Duration::from_secs(1)));
    }
}
