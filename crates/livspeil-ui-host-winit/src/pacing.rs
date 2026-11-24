// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Frame pacing: when to ask the window for a redraw and when to wake up.
//!
//! The rules (UI spec sections 3 and 12: render on demand, idle = zero
//! frames):
//! - After each batch of events (`about_to_wait`) the host asks
//!   [`Pacer::want_redraw`] with [`FrameScheduler::should_draw`]. A redraw is
//!   requested only when the app needs a frame, the window is visible and no
//!   redraw is already pending.
//! - After a frame, if [`Frame::needs_next_frame`] (something is still
//!   animating) the next redraw is requested right away; winit delivers it
//!   at the display's pace (FIFO presentation blocks on vsync).
//! - Occluded or minimized (zero-size) windows draw nothing; when they come
//!   back the next `about_to_wait` requests a frame if one is needed.
//! - The only timer is the config hot-reload poll, and only when config
//!   files are watched. Otherwise the event loop waits without a deadline.
//!
//! [`FrameScheduler::should_draw`]: livspeil_ui_core::FrameScheduler::should_draw
//! [`Frame::needs_next_frame`]: livspeil_ui_core::Frame::needs_next_frame

use std::time::{Duration, Instant};

/// When the event loop should wake up next if no event arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wake {
    /// Sleep until an event (idle).
    Never,
    /// Wake at this time (config poll).
    At(Instant),
}

/// The pacing state machine (no window needed; see the module docs).
#[derive(Clone, Debug)]
pub struct Pacer {
    occluded: bool,
    zero_size: bool,
    redraw_pending: bool,
    config_interval: Option<Duration>,
    next_config_poll: Option<Instant>,
}

impl Pacer {
    /// A pacer; `config_interval` is the hot-reload poll interval, `None`
    /// when no config files are watched.
    #[must_use]
    pub fn new(config_interval: Option<Duration>) -> Self {
        Pacer {
            occluded: false,
            zero_size: false,
            redraw_pending: false,
            config_interval,
            next_config_poll: None,
        }
    }

    /// Whether frames can be drawn (visible, non-zero size).
    #[must_use]
    pub fn can_draw(&self) -> bool {
        !self.occluded && !self.zero_size
    }

    /// Whether a redraw has been requested and not yet delivered.
    #[must_use]
    pub fn redraw_pending(&self) -> bool {
        self.redraw_pending
    }

    /// `WindowEvent::Occluded` (also used for minimize on macOS).
    pub fn set_occluded(&mut self, occluded: bool) {
        self.occluded = occluded;
    }

    /// The surface size in physical pixels (zero when minimized on some
    /// platforms).
    pub fn set_size(&mut self, width: u32, height: u32) {
        self.zero_size = width == 0 || height == 0;
    }

    /// After handling events: whether to call `Window::request_redraw`.
    pub fn want_redraw(&mut self, needs_frame: bool) -> bool {
        if needs_frame && self.can_draw() && !self.redraw_pending {
            self.redraw_pending = true;
            true
        } else {
            false
        }
    }

    /// `WindowEvent::RedrawRequested` arrived (requested by us or by the
    /// system, for example on expose or resize): whether to draw now.
    pub fn redraw_started(&mut self) -> bool {
        self.redraw_pending = false;
        self.can_draw()
    }

    /// After drawing a frame: whether to request the next redraw right away.
    pub fn frame_done(&mut self, needs_next_frame: bool) -> bool {
        self.want_redraw(needs_next_frame)
    }

    /// Whether the config poll is due at `now`; if so the next poll is
    /// scheduled one interval later. The first call is always due, so the
    /// files present at startup apply before the first frame.
    pub fn config_due(&mut self, now: Instant) -> bool {
        let Some(interval) = self.config_interval else {
            return false;
        };
        match self.next_config_poll {
            Some(t) if now < t => false,
            _ => {
                self.next_config_poll = Some(now + interval);
                true
            }
        }
    }

    /// When to wake if nothing happens.
    #[must_use]
    pub fn wake(&self) -> Wake {
        match (self.config_interval, self.next_config_poll) {
            (Some(_), Some(t)) => Wake::At(t),
            _ => Wake::Never,
        }
    }
}

/// Frame time counters (CPU time from the start of a redraw to present).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameTimes {
    /// Frames drawn.
    pub frames: u64,
    /// Redraws that drew nothing (occluded, surface unavailable).
    pub skipped: u64,
    /// The last frame's time.
    pub last: Duration,
    /// Exponential moving average (alpha 1/16).
    pub average: Duration,
    /// The longest frame so far.
    pub max: Duration,
}

impl FrameTimes {
    /// Record one frame.
    pub fn record(&mut self, time: Duration) {
        self.average = if self.frames == 0 {
            time
        } else {
            (self.average * 15 + time) / 16
        };
        self.frames += 1;
        self.last = time;
        self.max = self.max.max(time);
    }

    /// Record a skipped redraw.
    pub fn skip(&mut self) {
        self.skipped += 1;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use livspeil_ui_core::{App, Element, FrameScheduler, Render, ViewContext, div, widgets};
    use livspeil_ui_layout::Size;

    use super::*;

    /// A simulated host loop: events → `about_to_wait` → redraws, driven
    /// by the real core scheduler, counting redraw requests.
    struct Sim {
        app: App,
        scheduler: FrameScheduler,
        pacer: Pacer,
        now: Duration,
        requests: usize,
        frames: usize,
        queued_redraw: bool,
    }

    impl Sim {
        fn new(app: App) -> Self {
            Sim {
                app,
                scheduler: FrameScheduler::new(Duration::from_micros(16_667)),
                pacer: Pacer::new(None),
                now: Duration::ZERO,
                requests: 0,
                frames: 0,
                queued_redraw: false,
            }
        }

        fn about_to_wait(&mut self) {
            if self
                .pacer
                .want_redraw(self.scheduler.should_draw(&self.app))
            {
                self.requests += 1;
                self.queued_redraw = true;
            }
        }

        /// Deliver a pending redraw (one vsync later). Returns whether a
        /// frame was drawn.
        fn vsync(&mut self) -> bool {
            self.now += Duration::from_micros(16_667);
            if !std::mem::take(&mut self.queued_redraw) {
                return false;
            }
            if !self.pacer.redraw_started() || !self.scheduler.should_draw(&self.app) {
                return false;
            }
            let dt = self.scheduler.begin(self.now);
            let frame = self.app.draw_frame(dt);
            self.scheduler.end(frame.needs_next_frame);
            self.frames += 1;
            if self.pacer.frame_done(frame.needs_next_frame) {
                self.requests += 1;
                self.queued_redraw = true;
            }
            true
        }

        fn run(&mut self, vsyncs: usize) {
            for _ in 0..vsyncs {
                self.about_to_wait();
                self.vsync();
            }
        }
    }

    struct Root;
    impl Render for Root {
        fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
            div()
                .size(300.0, 200.0)
                .child(widgets::button(cx.theme(), "OK").key("ok"))
        }
    }

    fn sim() -> Sim {
        let mut app = App::new();
        let v = app.new_view(Root);
        app.set_root(v);
        app.set_viewport(Size::new(300.0, 200.0));
        Sim::new(app)
    }

    #[test]
    fn idle_requests_no_redraws() {
        let mut s = sim();
        s.run(30);
        let (frames, requests) = (s.frames, s.requests);
        assert!(frames >= 1, "the first frame mounts the root");
        // Settled: many more vsyncs, nothing happens.
        s.run(600);
        assert_eq!(s.frames, frames);
        assert_eq!(s.requests, requests);
        assert!(!s.pacer.redraw_pending());
        assert_eq!(s.pacer.wake(), Wake::Never);
    }

    #[test]
    fn animation_runs_then_stops() {
        let mut s = sim();
        s.run(30);
        let before = s.frames;
        // Hover the button: a micro spring animates its background.
        let b = s.app.bounds(s.app.find("ok").unwrap()).unwrap().center();
        s.app.dispatch(livspeil_ui_core::InputEvent::Pointer(
            livspeil_ui_core::pointer(livspeil_ui_core::PointerKind::Move, b.x, b.y, s.now),
        ));
        s.run(120);
        let animated = s.frames - before;
        assert!(
            animated > 1,
            "hover animates over several frames: {animated}"
        );
        let settled = s.frames;
        s.run(300);
        assert_eq!(s.frames, settled, "back to idle after the spring settles");
        // One request per frame at most: no duplicate requests.
        assert!(s.requests <= s.frames + 1);
    }

    #[test]
    fn occluded_windows_draw_nothing() {
        let mut s = sim();
        s.pacer.set_occluded(true);
        s.run(30);
        assert_eq!(s.frames, 0);
        assert_eq!(s.requests, 0);
        s.pacer.set_occluded(false);
        s.run(30);
        assert!(s.frames >= 1);
        s.pacer.set_size(0, 0);
        s.app.request_frame();
        s.run(5);
        let f = s.frames;
        s.run(5);
        assert_eq!(s.frames, f);
        s.pacer.set_size(10, 10);
        s.run(5);
        assert_eq!(s.frames, f + 1);
    }

    #[test]
    fn requests_are_not_duplicated() {
        let mut p = Pacer::new(None);
        assert!(p.want_redraw(true));
        assert!(!p.want_redraw(true));
        assert!(p.redraw_started());
        assert!(!p.want_redraw(false));
        assert!(p.frame_done(true));
    }

    #[test]
    fn config_timer_only_when_watching() {
        let t0 = Instant::now();
        let mut p = Pacer::new(None);
        assert!(!p.config_due(t0));
        assert_eq!(p.wake(), Wake::Never);
        let mut p = Pacer::new(Some(Duration::from_millis(500)));
        assert!(p.config_due(t0), "first poll right away");
        assert_eq!(p.wake(), Wake::At(t0 + Duration::from_millis(500)));
        assert!(!p.config_due(t0 + Duration::from_millis(100)));
        assert!(p.config_due(t0 + Duration::from_millis(600)));
        assert_eq!(p.wake(), Wake::At(t0 + Duration::from_millis(1100)));
    }

    #[test]
    fn frame_times() {
        let mut t = FrameTimes::default();
        t.record(Duration::from_millis(4));
        t.record(Duration::from_millis(20));
        t.skip();
        assert_eq!(t.frames, 2);
        assert_eq!(t.skipped, 1);
        assert_eq!(t.max, Duration::from_millis(20));
        assert_eq!(t.last, Duration::from_millis(20));
        assert_eq!(t.average, Duration::from_millis(5));
    }
}
