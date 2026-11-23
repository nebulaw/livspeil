// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Frame loop abstraction and a deterministic headless host for tests.

use std::time::Duration;

use livspeil_ui_layout::{Point, Size, Vec2};

use crate::app::{App, Frame, InputEvent};
use crate::input::{
    ImeEvent, Key, KeyEvent, Keystroke, Modifiers, MouseButton, PointerEvent, PointerKind,
    ScrollDelta,
};
use crate::paint::{Painter, Scene};

/// Decides when to draw and how much time each frame advances.
///
/// A host (winit, Wayland, Bevy) calls [`FrameScheduler::should_draw`]
/// after handling events; if true it requests a redraw, and on redraw calls
/// [`FrameScheduler::begin`] → [`App::draw_frame`] → [`FrameScheduler::end`].
/// When nothing is dirty or moving, nothing is drawn.
#[derive(Clone, Copy, Debug)]
pub struct FrameScheduler {
    nominal: Duration,
    max_dt: Duration,
    last: Option<Duration>,
    running: bool,
}

impl FrameScheduler {
    /// A scheduler for a display refreshing every `interval`.
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        FrameScheduler {
            nominal: interval,
            max_dt: Duration::from_millis(50),
            last: None,
            running: false,
        }
    }

    /// Whether the app wants a frame.
    #[must_use]
    pub fn should_draw(&self, app: &App) -> bool {
        app.needs_frame()
    }

    /// Start a frame at host time `now`; returns `dt` in seconds. The first
    /// frame after idle advances by one nominal interval (not by the idle
    /// time), and long stalls are clamped to 50 ms.
    pub fn begin(&mut self, now: Duration) -> f32 {
        let dt = match (self.running, self.last) {
            (true, Some(last)) => now.saturating_sub(last).min(self.max_dt),
            _ => self.nominal,
        };
        self.last = Some(now);
        dt.as_secs_f32()
    }

    /// End a frame; `needs_next` is [`Frame::needs_next_frame`].
    pub fn end(&mut self, needs_next: bool) {
        self.running = needs_next;
    }

    /// Whether the previous frame asked for another.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running
    }
}

/// A headless host: a virtual clock, deterministic frame steps and input
/// helpers. Frames are only drawn when the app needs one.
#[derive(Debug)]
pub struct TestHost {
    /// The app.
    pub app: App,
    scheduler: FrameScheduler,
    interval: Duration,
    now: Duration,
    frames: usize,
    last: Option<Frame>,
}

impl TestHost {
    /// A host at 60 Hz with `viewport`.
    #[must_use]
    pub fn new(mut app: App, viewport: Size) -> Self {
        app.set_viewport(viewport);
        let interval = Duration::from_micros(16_667);
        TestHost {
            app,
            scheduler: FrameScheduler::new(interval),
            interval,
            now: Duration::ZERO,
            frames: 0,
            last: None,
        }
    }

    /// Current virtual time.
    #[must_use]
    pub fn now(&self) -> Duration {
        self.now
    }

    /// Frames drawn so far.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.frames
    }

    /// The last frame drawn.
    #[must_use]
    pub fn last_frame(&self) -> Option<&Frame> {
        self.last.as_ref()
    }

    /// The last scene drawn.
    #[must_use]
    pub fn scene(&self) -> Option<&Scene> {
        self.last.as_ref().map(|f| &f.scene)
    }

    /// Advance one interval; draw if needed. Returns the frame, or `None`
    /// when idle.
    pub fn step(&mut self) -> Option<&Frame> {
        self.now += self.interval;
        if !self.scheduler.should_draw(&self.app) {
            self.scheduler.end(false);
            return None;
        }
        let dt = self.scheduler.begin(self.now);
        let frame = self.app.draw_frame(dt);
        self.scheduler.end(frame.needs_next_frame);
        self.frames += 1;
        self.last = Some(frame);
        self.last.as_ref()
    }

    /// Step and hand the scene to `painter`.
    pub fn step_with(&mut self, painter: &mut dyn Painter) -> bool {
        match self.step() {
            Some(f) => {
                painter.paint(&f.scene);
                true
            }
            None => false,
        }
    }

    /// Step until idle (or `max` steps); returns frames drawn.
    pub fn run_until_idle(&mut self, max: usize) -> usize {
        let mut drawn = 0;
        for _ in 0..max {
            if self.step().is_none() {
                break;
            }
            drawn += 1;
        }
        drawn
    }

    /// Step for `duration` of virtual time; returns frames drawn.
    pub fn advance(&mut self, duration: Duration) -> usize {
        let end = self.now + duration;
        let mut drawn = 0;
        while self.now + self.interval <= end {
            if self.step().is_some() {
                drawn += 1;
            }
        }
        drawn
    }

    fn pointer(&mut self, kind: PointerKind, p: Point) {
        self.app.dispatch(InputEvent::Pointer(PointerEvent {
            kind,
            position: p,
            modifiers: Modifiers::NONE,
            time: self.now,
        }));
    }

    /// Move the pointer.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.pointer(PointerKind::Move, Point::new(x, y));
    }

    /// Press the primary button.
    pub fn pointer_down(&mut self, x: f32, y: f32) {
        self.pointer(PointerKind::Down(MouseButton::Left), Point::new(x, y));
    }

    /// Release the primary button.
    pub fn pointer_up(&mut self, x: f32, y: f32) {
        self.pointer(PointerKind::Up(MouseButton::Left), Point::new(x, y));
    }

    /// Move, press and release at a point.
    pub fn click(&mut self, x: f32, y: f32) {
        self.pointer_move(x, y);
        self.pointer_down(x, y);
        self.pointer_up(x, y);
    }

    /// Scroll by precise pixel deltas.
    pub fn scroll(&mut self, x: f32, y: f32, delta: Vec2) {
        self.pointer(
            PointerKind::Scroll(ScrollDelta::Pixels(delta)),
            Point::new(x, y),
        );
    }

    /// Drag from `from` to `to` in `steps` moves, one frame interval apart
    /// (frames are stepped in between), then release.
    pub fn drag(&mut self, from: Point, to: Point, steps: usize) {
        self.pointer_move(from.x, from.y);
        self.pointer_down(from.x, from.y);
        let steps = steps.max(1);
        for i in 1..=steps {
            #[allow(clippy::cast_precision_loss)]
            let t = i as f32 / steps as f32;
            let p = from + (to - from) * t;
            self.step();
            self.pointer_move(p.x, p.y);
        }
        self.pointer_up(to.x, to.y);
    }

    /// Press and release a keystroke such as `shift-tab`.
    ///
    /// # Panics
    /// If the keystroke does not parse (test helper).
    pub fn key(&mut self, keystroke: &str) {
        let ks = match Keystroke::parse(keystroke) {
            Ok(k) => k,
            Err(e) => panic!("{e}"),
        };
        self.key_event(ks.key, ks.modifiers, true);
        self.key_event(ks.key, ks.modifiers, false);
    }

    /// A single key event.
    pub fn key_event(&mut self, key: Key, modifiers: Modifiers, pressed: bool) {
        self.app.dispatch(InputEvent::Key(KeyEvent {
            key,
            modifiers,
            pressed,
            repeat: false,
        }));
    }

    /// Committed text.
    pub fn type_text(&mut self, text: &str) {
        self.app.dispatch(InputEvent::Text(text.to_owned()));
    }

    /// An IME event.
    pub fn ime(&mut self, event: ImeEvent) {
        self.app.dispatch(InputEvent::Ime(event));
    }
}
