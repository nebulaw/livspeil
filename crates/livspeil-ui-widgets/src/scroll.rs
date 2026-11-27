// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`ScrollView`] (clipping scroll container with overlay scrollbars,
//! momentum and rubber-band overscroll), its physics ([`Scroller`],
//! [`ScrollPhysics`]) and [`VirtualList`] for long uniform lists.

use std::cell::Cell;
use std::fmt;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use livspeil_ui_core::{
    Animated, App, DragPhase, Element, ElementId, EventCx, Layer, PointerKind, Render,
    Role as CoreRole, SCROLL_INTO_VIEW, ScrollDelta, SharedString, VelocityTracker, View,
    ViewContext, div, spring_step,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto, Position};
use livspeil_ui_layout::{Rect, Size, Transform2D, Vec2};
use livspeil_ui_theme::{MotionClass, MotionConfig, MotionSpec, PropertyKind, Theme};

use crate::env::Env;
use crate::semantics::{Role, Semantics};
use crate::style;

/// Motion scope of scrolling (`[overrides.scroll]` in `motion.toml`).
pub const SCROLL_SCOPE: &str = "scroll";

/// Scroll actions (key context `ScrollView`).
pub mod scroll_actions {
    /// One line up.
    pub const LINE_UP: &str = "scroll.line_up";
    /// One line down.
    pub const LINE_DOWN: &str = "scroll.line_down";
    /// One line left.
    pub const LINE_LEFT: &str = "scroll.line_left";
    /// One line right.
    pub const LINE_RIGHT: &str = "scroll.line_right";
    /// One page up.
    pub const PAGE_UP: &str = "scroll.page_up";
    /// One page down.
    pub const PAGE_DOWN: &str = "scroll.page_down";
    /// To the start.
    pub const TOP: &str = "scroll.top";
    /// To the end.
    pub const BOTTOM: &str = "scroll.bottom";
}

/// Which directions scroll.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollAxis {
    /// Vertical only (content as wide as the view).
    #[default]
    Vertical,
    /// Horizontal only.
    Horizontal,
    /// Both.
    Both,
}

impl ScrollAxis {
    fn mask(self) -> Vec2 {
        match self {
            ScrollAxis::Vertical => Vec2::new(0.0, 1.0),
            ScrollAxis::Horizontal => Vec2::new(1.0, 0.0),
            ScrollAxis::Both => Vec2::new(1.0, 1.0),
        }
    }
}

/// Scrolling feel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollPhysics {
    /// Pixels per wheel line.
    pub line: f32,
    /// Keep gliding after a trackpad gesture ends.
    pub momentum: bool,
    /// Momentum decay time constant in seconds (velocity × e^(−t/decay)).
    pub decay: f32,
    /// Momentum stops below this speed (px/s).
    pub min_velocity: f32,
    /// Trackpad gestures may pull past the ends (resisted) and spring back;
    /// momentum bounces at the ends.
    pub rubber_band: bool,
    /// Rubber-band stiffness (UIKit's 0.55: lower is stiffer).
    pub rubber_coefficient: f32,
    /// A gesture counts as released after this long without a pixel delta.
    pub release_delay: f32,
    /// Most a gesture can pull past an end, as a fraction of the viewport
    /// extent (unresisted distance). Further outward deltas (for example the
    /// operating system's momentum events after a hard flick) start the
    /// spring-back and are dropped until the stream pauses or reverses.
    pub overscroll_limit: f32,
    /// Scrollbars stay this long after the last scroll or hover (seconds).
    pub bar_linger: f32,
}

impl Default for ScrollPhysics {
    fn default() -> Self {
        ScrollPhysics {
            line: 40.0,
            momentum: true,
            decay: 0.325,
            min_velocity: 20.0,
            rubber_band: true,
            rubber_coefficient: 0.55,
            release_delay: 0.06,
            overscroll_limit: 0.6,
            bar_linger: 0.9,
        }
    }
}

impl ScrollPhysics {
    /// The physics for a motion configuration: Reduce Motion (in the
    /// `scroll` scope) turns off overscroll and bounce; speed 0 also turns
    /// off momentum.
    #[must_use]
    pub fn from_motion(motion: &MotionConfig) -> ScrollPhysics {
        let reduce = motion.reduce_motion(Some(SCROLL_SCOPE));
        let speed = motion.speed(Some(SCROLL_SCOPE));
        ScrollPhysics {
            rubber_band: !reduce,
            momentum: speed > 0.0,
            ..ScrollPhysics::default()
        }
    }
}

/// Resisted overscroll: `x` px pulled past an end shows as this many (iOS
/// rubber band; approaches `d`, the viewport extent).
#[must_use]
pub fn rubber_band(x: f32, d: f32, c: f32) -> f32 {
    if d <= 0.0 {
        return 0.0;
    }
    let a = x.abs();
    (1.0 - 1.0 / (a * c / d + 1.0)) * d * x.signum()
}

fn rubber_band_inverse(y: f32, d: f32, c: f32) -> f32 {
    let a = y.abs().min(d * 0.999);
    if d <= 0.0 {
        return 0.0;
    }
    d * a / (c * (d - a)) * y.signum()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ScrollPhase {
    Idle,
    /// A trackpad gesture: pixel deltas arriving; seconds since the last.
    Tracking(f32),
    /// Gliding with the release velocity.
    Momentum,
    /// Springing to a target (animated scrolls, rubber-band return).
    Settling(Vec2),
}

/// Scroll physics of one scroll area, independent of the UI (pure state,
/// advanced with [`Scroller::tick`]).
#[derive(Clone, Copy, Debug)]
pub struct Scroller {
    axis: ScrollAxis,
    offset: Vec2,
    velocity: Vec2,
    /// Unresisted offset of the current gesture.
    raw: Vec2,
    viewport: Size,
    content: Size,
    phase: ScrollPhase,
    tracker: VelocityTracker,
    /// While `Some(idle)`: a hard pull hit the overscroll limit and is
    /// springing back; outward deltas are dropped; `idle` is the time since
    /// the last dropped delta.
    drain: Option<f32>,
}

impl Scroller {
    /// A scroller at offset zero.
    #[must_use]
    pub fn new(axis: ScrollAxis) -> Scroller {
        Scroller {
            axis,
            offset: Vec2::ZERO,
            velocity: Vec2::ZERO,
            raw: Vec2::ZERO,
            viewport: Size::new(0.0, 0.0),
            content: Size::new(0.0, 0.0),
            phase: ScrollPhase::Idle,
            tracker: VelocityTracker::default(),
            drain: None,
        }
    }

    /// Current offset (content pixels scrolled past the top left); outside
    /// `0..=max` while overscrolled.
    #[must_use]
    pub fn offset(&self) -> Vec2 {
        self.offset
    }

    /// Current velocity (px/s).
    #[must_use]
    pub fn velocity(&self) -> Vec2 {
        self.velocity
    }

    /// The viewport and content sizes.
    #[must_use]
    pub fn sizes(&self) -> (Size, Size) {
        (self.viewport, self.content)
    }

    /// Set the viewport and content sizes (measured after layout).
    pub fn set_sizes(&mut self, viewport: Size, content: Size) {
        self.viewport = viewport;
        self.content = content;
        if matches!(self.phase, ScrollPhase::Idle) {
            self.offset = self.clamp(self.offset);
        }
    }

    /// Largest offset.
    #[must_use]
    pub fn max(&self) -> Vec2 {
        let m = self.axis.mask();
        Vec2::new(
            (self.content.width - self.viewport.width).max(0.0) * m.x,
            (self.content.height - self.viewport.height).max(0.0) * m.y,
        )
    }

    /// `o` clamped to `0..=max`.
    #[must_use]
    pub fn clamp(&self, o: Vec2) -> Vec2 {
        let m = self.max();
        Vec2::new(o.x.clamp(0.0, m.x), o.y.clamp(0.0, m.y))
    }

    /// Whether anything is moving.
    #[must_use]
    pub fn is_moving(&self) -> bool {
        !matches!(self.phase, ScrollPhase::Idle)
    }

    /// Whether a trackpad gesture is in progress.
    #[must_use]
    pub fn is_tracking(&self) -> bool {
        matches!(self.phase, ScrollPhase::Tracking(_))
    }

    /// Whether gliding with momentum.
    #[must_use]
    pub fn is_gliding(&self) -> bool {
        matches!(self.phase, ScrollPhase::Momentum)
    }

    fn overscroll_view(&self, raw: f32, max: f32, extent: f32, p: &ScrollPhysics) -> f32 {
        if raw < 0.0 {
            if p.rubber_band {
                rubber_band(raw, extent, p.rubber_coefficient)
            } else {
                0.0
            }
        } else if raw > max {
            if p.rubber_band {
                max + rubber_band(raw - max, extent, p.rubber_coefficient)
            } else {
                max
            }
        } else {
            raw
        }
    }

    fn raw_from_offset(&self, o: f32, max: f32, extent: f32, p: &ScrollPhysics) -> f32 {
        if o < 0.0 {
            rubber_band_inverse(o, extent, p.rubber_coefficient)
        } else if o > max {
            max + rubber_band_inverse(o - max, extent, p.rubber_coefficient)
        } else {
            o
        }
    }

    /// A trackpad (pixel) delta at host time `time`. Positive deltas move
    /// the content down/right (scroll toward the start), as winit reports.
    pub fn pixel_scroll(&mut self, delta: Vec2, time: Duration, p: &ScrollPhysics) {
        let m = self.axis.mask();
        let max = self.max();
        let step = Vec2::new(delta.x * m.x, delta.y * m.y);
        if let Some(idle) = self.drain {
            // Springing back from the limit: more of the same stream (OS
            // momentum) is dropped; a pause or a reversal is a new gesture.
            let before = self.offset;
            let outward = (before.x < 0.0 && step.x > 0.0)
                || (before.x > max.x && step.x < 0.0)
                || (before.y < 0.0 && step.y > 0.0)
                || (before.y > max.y && step.y < 0.0);
            let same_stream = idle < p.release_delay && (outward || step == Vec2::ZERO);
            if same_stream {
                self.drain = Some(0.0);
                return;
            }
            self.drain = None;
        }
        if !self.is_tracking() {
            self.raw = Vec2::new(
                self.raw_from_offset(self.offset.x, max.x, self.viewport.width, p),
                self.raw_from_offset(self.offset.y, max.y, self.viewport.height, p),
            );
            self.tracker.reset();
            self.tracker.push(time, self.raw);
        }
        self.raw -= step;
        // Without rubber band the gesture cannot build up overscroll.
        if !p.rubber_band {
            self.raw = self.clamp(self.raw);
        }
        // Bounded overscroll: an unbounded raw offset would make a reversal
        // wait until the whole excess had been undone.
        let limit_x = self.viewport.width * p.overscroll_limit;
        let limit_y = self.viewport.height * p.overscroll_limit;
        let limited = Vec2::new(
            self.raw.x.clamp(-limit_x, max.x + limit_x),
            self.raw.y.clamp(-limit_y, max.y + limit_y),
        );
        let hit_limit = p.rubber_band && limited != self.raw;
        self.raw = limited;
        self.tracker.push(time, self.raw);
        self.offset = Vec2::new(
            self.overscroll_view(self.raw.x, max.x, self.viewport.width, p),
            self.overscroll_view(self.raw.y, max.y, self.viewport.height, p),
        );
        self.velocity = Vec2::ZERO;
        if hit_limit {
            self.phase = ScrollPhase::Settling(self.clamp(self.offset));
            self.drain = Some(0.0);
        } else {
            self.phase = ScrollPhase::Tracking(0.0);
        }
    }

    /// A wheel delta in lines: an animated scroll (no overscroll).
    pub fn line_scroll(&mut self, lines: Vec2, p: &ScrollPhysics) {
        let m = self.axis.mask();
        let d = Vec2::new(lines.x * m.x, lines.y * m.y) * p.line;
        self.scroll_by(-d);
    }

    /// Scroll by `delta` from the current target (animated).
    pub fn scroll_by(&mut self, delta: Vec2) {
        let from = match self.phase {
            ScrollPhase::Settling(t) => t,
            _ => self.clamp(self.offset),
        };
        self.scroll_to(from + delta);
    }

    /// Spring to `target` (clamped), keeping the current velocity.
    pub fn scroll_to(&mut self, target: Vec2) {
        let target = self.clamp(target);
        if target == self.offset && self.velocity == Vec2::ZERO {
            self.phase = ScrollPhase::Idle;
        } else {
            self.phase = ScrollPhase::Settling(target);
        }
    }

    /// Jump to `target` (clamped) and stop.
    pub fn jump_to(&mut self, target: Vec2) {
        self.offset = self.clamp(target);
        self.velocity = Vec2::ZERO;
        self.phase = ScrollPhase::Idle;
    }

    /// Advance by `dt` seconds with springs from `spring` (an `Instant`
    /// spec jumps). Returns whether still moving.
    pub fn tick(&mut self, dt: f32, p: &ScrollPhysics, spring: MotionSpec) -> bool {
        if let Some(idle) = self.drain {
            self.drain = Some(idle + dt);
        }
        match self.phase {
            ScrollPhase::Idle => return false,
            ScrollPhase::Tracking(idle) => {
                let idle = idle + dt;
                if idle < p.release_delay {
                    self.phase = ScrollPhase::Tracking(idle);
                    return true;
                }
                self.release(p);
            }
            ScrollPhase::Momentum => {
                self.velocity = self.velocity * (-dt / p.decay.max(1e-3)).exp();
                self.offset += self.velocity * dt;
                let clamped = self.clamp(self.offset);
                if clamped != self.offset {
                    if p.rubber_band {
                        // Bounce: spring back to the end, keeping velocity.
                        self.phase = ScrollPhase::Settling(clamped);
                    } else {
                        self.offset = clamped;
                        self.velocity = Vec2::ZERO;
                        self.phase = ScrollPhase::Idle;
                    }
                } else if self.velocity.length() < p.min_velocity {
                    self.velocity = Vec2::ZERO;
                    self.phase = ScrollPhase::Idle;
                }
            }
            ScrollPhase::Settling(target) => match spring {
                MotionSpec::Instant => self.jump_to(target),
                MotionSpec::Spring(s) => {
                    let omega = std::f32::consts::TAU / s.response.max(1e-3);
                    // Scrolling never overshoots its target.
                    let zeta = s.damping.max(1.0);
                    let (x, vx) =
                        spring_step(self.offset.x - target.x, self.velocity.x, dt, omega, zeta);
                    let (y, vy) =
                        spring_step(self.offset.y - target.y, self.velocity.y, dt, omega, zeta);
                    self.offset = target + Vec2::new(x, y);
                    self.velocity = Vec2::new(vx, vy);
                    if x.abs() < 0.25 && y.abs() < 0.25 && self.velocity.length() < 5.0 {
                        self.jump_to(target);
                    }
                }
            },
        }
        self.is_moving()
    }

    fn release(&mut self, p: &ScrollPhysics) {
        let v = self.tracker.velocity();
        let clamped = self.clamp(self.offset);
        if clamped != self.offset {
            self.velocity = Vec2::ZERO;
            self.phase = ScrollPhase::Settling(clamped);
        } else if p.momentum && v.length() >= p.min_velocity {
            self.velocity = v;
            self.phase = ScrollPhase::Momentum;
        } else {
            self.velocity = Vec2::ZERO;
            self.phase = ScrollPhase::Idle;
        }
    }

    /// The offset that brings `rect` (relative to the content origin) into
    /// view with `margin`, moving as little as possible.
    #[must_use]
    pub fn reveal(&self, rect: Rect, margin: f32) -> Vec2 {
        let o = self.clamp(self.offset);
        let axis = |o: f32, start: f32, len: f32, view: f32| {
            if len + 2.0 * margin > view || start - margin < o {
                start - margin
            } else if start + len + margin > o + view {
                start + len + margin - view
            } else {
                o
            }
        };
        let m = self.axis.mask();
        let x = if m.x > 0.0 {
            axis(o.x, rect.x, rect.width, self.viewport.width)
        } else {
            o.x
        };
        let y = if m.y > 0.0 {
            axis(o.y, rect.y, rect.height, self.viewport.height)
        } else {
            o.y
        };
        self.clamp(Vec2::new(x, y))
    }
}

/// What a scroll view's content builder gets.
///
/// Scrolling moves the content without rendering the scroll view. A builder
/// that mounts only part of its content asks for [`ScrollInfo::region`];
/// the view then renders again when scrolling leaves that region.
#[derive(Debug)]
pub struct ScrollInfo {
    viewport: Size,
    region: Rect,
    virtualized: Cell<bool>,
}

impl ScrollInfo {
    /// Viewport size (zero until the view was laid out).
    #[must_use]
    pub fn viewport(&self) -> Size {
        self.viewport
    }

    /// The part of the content to build, in content coordinates: the view
    /// and a margin around it. Asking for it makes the content depend on
    /// the scroll position.
    #[must_use]
    pub fn region(&self) -> Rect {
        self.virtualized.set(true);
        self.region
    }
}

/// Builds a scroll view's content.
pub type ScrollContent = Rc<dyn Fn(&App, &ScrollInfo) -> Element>;

/// The built region reaches past the view on each side by this fraction of
/// the view, and at least by the view's [`ScrollView::overscan`].
const MARGIN: f32 = 0.25;
/// Default least margin of the built region.
const MIN_MARGIN: f32 = 96.0;
/// Key of the scroll view's frame task.
const SCROLL_TASK: &str = "scroll";

/// What virtualized content was built for.
#[derive(Clone, Copy, Debug)]
struct Built {
    area: Rect,
    viewport: Size,
}

/// Scrollbar thickness (thumb, at rest).
const BAR: f32 = 6.0;
/// Inset of scrollbars from the edges.
const BAR_INSET: f32 = 2.0;
/// Smallest thumb length.
const MIN_THUMB: f32 = 24.0;
/// Margin kept around elements scrolled into view.
const REVEAL_MARGIN: f32 = 8.0;

/// A scroll container (UI spec §9 ScrollView).
///
/// - Trackpad pixel deltas move the content 1:1, with rubber-band
///   resistance past the ends; after the gesture ends (no delta for
///   [`ScrollPhysics::release_delay`]) the content glides with the release
///   velocity and decays, bouncing at the ends, or springs back from
///   overscroll. Wheel lines animate by [`ScrollPhysics::line`] px.
///   Ctrl+scroll is left to ancestors (zoom).
/// - Reduce Motion: no overscroll or bounce, scrolls to targets jump
///   (momentum is kept: it follows the gesture's own velocity).
/// - Overlay scrollbars fade in while scrolling or hovered and out after
///   [`ScrollPhysics::bar_linger`]; thumbs can be dragged.
/// - Keyboard while focused (context `ScrollView`): arrows by a line,
///   Page Up/Down by a page, Home/End to the ends.
/// - Handles [`SCROLL_INTO_VIEW`] from descendants (assistive technology,
///   and Tab focus moves via [`crate::WidgetRoot`]).
/// - Layers: the content moves, and the thumbs move, resize and fade,
///   without rendering the view; it renders only when virtualized content
///   ([`ScrollInfo::region`]) must change.
pub struct ScrollView {
    axis: ScrollAxis,
    scroller: Scroller,
    physics: Option<ScrollPhysics>,
    content: ScrollContent,
    name: Option<SharedString>,
    label: Option<SharedString>,
    width: Option<f32>,
    height: Option<f32>,
    focusable: bool,
    /// Seconds since the last scroll or hover.
    bar_idle: f32,
    /// Fade of the scrollbars.
    bar_alpha: Animated<f32>,
    overscan: f32,
    /// `Some` while the content depends on the scroll position.
    built: Option<Built>,
    thumb_grab: Option<Vec2>,
}

impl fmt::Debug for ScrollView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScrollView")
            .field("axis", &self.axis)
            .field("scroller", &self.scroller)
            .finish_non_exhaustive()
    }
}

impl ScrollView {
    /// A vertical scroll view filling its parent, over `content`. (`env`
    /// keeps the constructor uniform with the other stateful widgets.)
    #[must_use]
    pub fn new(_env: &Env, content: impl Fn(&App, &ScrollInfo) -> Element + 'static) -> Self {
        ScrollView {
            axis: ScrollAxis::Vertical,
            scroller: Scroller::new(ScrollAxis::Vertical),
            physics: None,
            content: Rc::new(content),
            name: None,
            label: None,
            width: None,
            height: None,
            focusable: true,
            bar_idle: f32::INFINITY,
            bar_alpha: Animated::new(0.0),
            overscan: MIN_MARGIN,
            built: None,
            thumb_grab: None,
        }
    }

    /// Scroll directions.
    #[must_use]
    pub fn axis(mut self, axis: ScrollAxis) -> Self {
        self.axis = axis;
        self.scroller.axis = axis;
        self
    }

    /// Fixed physics (default: [`ScrollPhysics::from_motion`] of the app's
    /// motion configuration, read every frame).
    #[must_use]
    pub fn physics(mut self, p: ScrollPhysics) -> Self {
        self.physics = Some(p);
        self
    }

    /// Fixed size (default: fill the parent).
    #[must_use]
    pub fn size(mut self, w: f32, h: f32) -> Self {
        self.width = Some(w);
        self.height = Some(h);
        self
    }

    /// Element name (default `scroll:<entity>`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Accessible name.
    #[must_use]
    pub fn label(mut self, l: impl Into<SharedString>) -> Self {
        self.label = Some(l.into());
        self
    }

    /// The least distance past each end of the view that virtualized
    /// content is built for (default 96 px).
    #[must_use]
    pub fn overscan(mut self, px: f32) -> Self {
        self.overscan = px.max(0.0);
        self
    }

    /// Whether the view itself takes focus for keyboard scrolling (default
    /// true).
    #[must_use]
    pub fn focusable(mut self, on: bool) -> Self {
        self.focusable = on;
        self
    }

    /// Replace the content builder.
    pub fn set_content(&mut self, content: impl Fn(&App, &ScrollInfo) -> Element + 'static) {
        self.content = Rc::new(content);
    }

    /// The physics state.
    #[must_use]
    pub fn scroller(&self) -> &Scroller {
        &self.scroller
    }

    /// Jump to `offset` (clamped), stopping any motion. Renders the view
    /// only if virtualized content must change.
    pub fn jump_to(view: View<Self>, app: &mut App, offset: Vec2) {
        measure_view(view, app);
        view.update(app, |s, cx| {
            s.scroller.jump_to(offset);
            s.sync(cx);
        });
    }

    /// Spring to `offset` (clamped); instant under Reduce Motion.
    pub fn scroll_to(view: View<Self>, app: &mut App, offset: Vec2) {
        measure_view(view, app);
        poke(view, app, |s, _| s.scroller.scroll_to(offset));
    }

    /// Current offset.
    #[must_use]
    pub fn offset(&self) -> Vec2 {
        self.scroller.offset
    }

    /// Whether the scrollbars are shown.
    #[must_use]
    pub fn bars_visible(&self) -> bool {
        self.bar_idle < self.effective_physics_default().bar_linger
    }

    fn effective_physics_default(&self) -> ScrollPhysics {
        self.physics.unwrap_or_default()
    }

    fn physics_for(&self, motion: &MotionConfig) -> ScrollPhysics {
        self.physics
            .unwrap_or_else(|| ScrollPhysics::from_motion(motion))
    }

    /// The element name for `view`.
    #[must_use]
    pub fn element_name(view: &View<ScrollView>, app: &App) -> String {
        view.read(app)
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("scroll", view.id()), str::to_owned)
    }

    /// Semantics: `ScrollView` with its label.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::ScrollView).label_opt(self.label.as_deref())
    }
}

/// Measure viewport and content from the retained tree.
fn measure(view: View<ScrollView>, cx: &mut App, sv: ElementId) {
    let Some(vp) = cx.bounds(sv) else { return };
    let content = cx.bounds(sv.child("content")).unwrap_or(vp);
    view.update(cx, |s, _| {
        s.scroller.set_sizes(
            Size::new(vp.width, vp.height),
            Size::new(content.width, content.height),
        );
    });
}

/// [`measure`] for callers that only have the view.
fn measure_view(view: View<ScrollView>, app: &mut App) {
    if let Some(sv) = app.view_element(view.id()) {
        measure(view, app, sv);
    }
}

/// Run the view's frame task until the content and bars are at rest.
fn run(view: View<ScrollView>, app: &mut App) {
    if !app.is_animating(view.id(), SCROLL_TASK) {
        app.animate(view.id(), SCROLL_TASK, move |app, dt| {
            let motion = app.motion().clone();
            view.update(app, |s, cx| s.step(dt, &motion, cx))
        });
    }
}

/// A scroll or hover: change the scroller, show the bars and keep going.
fn poke(view: View<ScrollView>, cx: &mut App, f: impl FnOnce(&mut ScrollView, &ScrollPhysics)) {
    let motion = cx.motion().clone();
    view.update(cx, |s, vcx| {
        let p = s.physics_for(&motion);
        f(s, &p);
        s.bar_idle = 0.0;
        s.sync(vcx);
    });
    run(view, cx);
}

/// A scroll event on the scroll element.
fn on_scroll(view: View<ScrollView>, ev: &livspeil_ui_core::PointerEvent, cx: &mut EventCx<'_>) {
    let PointerKind::Scroll(delta) = ev.kind else {
        return;
    };
    if ev.modifiers.ctrl || ev.modifiers.cmd {
        return; // zoom gestures belong to ancestors
    }
    let sv = cx.element_id();
    measure(view, cx, sv);
    poke(view, cx, |s, p| match delta {
        ScrollDelta::Pixels(d) => s.scroller.pixel_scroll(d, ev.time, p),
        ScrollDelta::Lines(l) => s.scroller.line_scroll(l, p),
    });
    cx.stop_propagation();
}

fn keyboard(view: View<ScrollView>, cx: &mut EventCx<'_>, f: impl Fn(&Scroller) -> Vec2) {
    let sv = cx.element_id();
    measure(view, cx, sv);
    poke(view, cx, |s, _| {
        let target = f(&s.scroller);
        s.scroller.scroll_to(target);
    });
}

fn reveal(view: View<ScrollView>, cx: &mut EventCx<'_>) {
    let sv = cx.element_id();
    let target = cx.target();
    if target == sv {
        return;
    }
    measure(view, cx, sv);
    let content = sv.child("content");
    let (Some(t), Some(origin)) = (cx.bounds_within(target, content), App::bounds(cx, content))
    else {
        return;
    };
    let rel = Rect::new(t.x - origin.x, t.y - origin.y, t.width, t.height);
    poke(view, cx, |s, _| {
        let to = s.scroller.reveal(rel, REVEAL_MARGIN);
        s.scroller.scroll_to(to);
    });
}

/// Where a thumb is, if the content is longer than the view.
fn thumb_rect(vertical: bool, vp: Size, content: Size, offset: Vec2) -> Option<Rect> {
    let (len, total, off) = if vertical {
        (vp.height, content.height, offset.y)
    } else {
        (vp.width, content.width, offset.x)
    };
    if total <= len + 0.5 || len <= 0.0 {
        return None;
    }
    let track = len - 2.0 * BAR_INSET;
    let max = total - len;
    // Overscroll shrinks the thumb.
    let over = if off < 0.0 {
        -off
    } else {
        (off - max).max(0.0)
    };
    let size = (track * len / total - over).max(MIN_THUMB).min(track);
    let pos = BAR_INSET + (off.clamp(0.0, max) / max) * (track - size);
    Some(if vertical {
        Rect::new(vp.width - BAR - BAR_INSET, pos, BAR, size)
    } else {
        Rect::new(pos, vp.height - BAR - BAR_INSET, size, BAR)
    })
}

/// Content pixels per pixel a thumb is dragged.
fn drag_ratio(vertical: bool, vp: Size, content: Size) -> f32 {
    let (len, total) = if vertical {
        (vp.height, content.height)
    } else {
        (vp.width, content.width)
    };
    total / (len - 2.0 * BAR_INSET).max(1.0)
}

fn bar_layer_key(vertical: bool) -> &'static str {
    if vertical { "vbar" } else { "hbar" }
}

/// A thumb element; its box and fade are its layer's.
fn thumb(view: View<ScrollView>, vertical: bool, theme: &Theme, layer: Layer) -> Element {
    let color = theme.colors().text_secondary.with_alpha(0.45);
    div()
        .key(bar_layer_key(vertical))
        .absolute(Rect::ZERO)
        .layer(layer)
        .rounded(BAR / 2.0)
        .bg(color)
        .hover(|r| r.bg(color.with_alpha(0.7)))
        .transition(MotionClass::Micro)
        .on_pointer_move(move |_, cx| {
            view.update(cx, |s, _| s.bar_idle = 0.0);
            run(view, cx);
        })
        .on_drag(move |ev, cx| {
            poke(view, cx, |s, _| {
                if ev.phase == DragPhase::Start {
                    s.thumb_grab = Some(s.scroller.clamp(s.scroller.offset));
                }
                if let Some(start) = s.thumb_grab {
                    let (vp, content) = s.scroller.sizes();
                    let d = ev.total() * drag_ratio(vertical, vp, content);
                    let d = if vertical {
                        Vec2::new(0.0, d.y)
                    } else {
                        Vec2::new(d.x, 0.0)
                    };
                    s.scroller.jump_to(start + d);
                }
                if ev.phase == DragPhase::End {
                    s.thumb_grab = None;
                }
            });
        })
}

impl ScrollView {
    fn axes(&self) -> impl Iterator<Item = bool> {
        let axis = self.axis;
        [true, false]
            .into_iter()
            .filter(move |&vertical| match axis {
                ScrollAxis::Vertical => vertical,
                ScrollAxis::Horizontal => !vertical,
                ScrollAxis::Both => true,
            })
    }

    /// The region to build for a view of `viewport` at the current offset.
    fn region_around(&self, viewport: Size) -> Rect {
        let o = self.scroller.clamp(self.scroller.offset);
        let m = self.axis.mask();
        let margin = |len: f32, on: f32| (len * MARGIN).max(self.overscan) * on;
        let (x, y) = (margin(viewport.width, m.x), margin(viewport.height, m.y));
        Rect::new(
            o.x - x,
            o.y - y,
            viewport.width + 2.0 * x,
            viewport.height + 2.0 * y,
        )
    }

    /// Whether what is built still covers the view.
    fn covers(&self, built: Built, viewport: Size) -> bool {
        // Past the ends there is nothing to build.
        let o = self.scroller.clamp(self.scroller.offset);
        let a = built.area;
        built.viewport == viewport
            && a.x <= o.x
            && a.y <= o.y
            && a.right() >= o.x + viewport.width
            && a.bottom() >= o.y + viewport.height
    }

    /// The viewport size, or the window's before the first layout.
    fn viewport_or_window(&self, app: &App) -> Size {
        let vp = self.scroller.viewport;
        if vp.height > 0.0 { vp } else { app.viewport() }
    }

    /// Set the layers from the scroller, and render only if virtualized
    /// content no longer covers the view.
    fn sync(&mut self, cx: &mut ViewContext<'_, Self>) {
        let offset = self.scroller.offset;
        let content_layer = cx.layer("content");
        content_layer.set_transform(cx, Transform2D::translate(-offset));
        let (vp, content) = self.scroller.sizes();
        for vertical in self.axes() {
            let rect = thumb_rect(vertical, vp, content, offset);
            let layer = cx.layer(bar_layer_key(vertical));
            layer.set_frame(cx, rect);
            let alpha = if rect.is_some() {
                self.bar_alpha.value()
            } else {
                0.0
            };
            layer.set_opacity(cx, alpha);
        }
        if let Some(built) = self.built
            && !self.covers(built, self.viewport_or_window(cx))
        {
            cx.notify();
        }
    }

    /// One frame of momentum, springs and the scrollbar fade. Returns
    /// whether there is more to do.
    fn step(&mut self, dt: f32, motion: &MotionConfig, cx: &mut ViewContext<'_, Self>) -> bool {
        let p = self.physics_for(motion);
        let spring = motion.resolve(
            Some(SCROLL_SCOPE),
            MotionClass::Standard,
            PropertyKind::Movement,
        );
        let moving = self.scroller.tick(dt, &p, spring);
        if moving || self.thumb_grab.is_some() {
            self.bar_idle = 0.0;
        } else if self.bar_idle.is_finite() {
            self.bar_idle += dt;
        }
        let bars = self.bar_idle < p.bar_linger;
        let alpha = if bars { 1.0 } else { 0.0 };
        if self.bar_alpha.target() != alpha {
            let fade = motion.resolve(None, MotionClass::Standard, PropertyKind::Appearance);
            self.bar_alpha.animate_to(alpha, fade);
        }
        let fading = self.bar_alpha.step(dt);
        self.sync(cx);
        if !bars {
            self.bar_idle = f32::INFINITY;
        }
        moving || fading || bars
    }
}

impl Render for ScrollView {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let p = self.physics_for(cx.motion());

        let name = self
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("scroll", view.id()), str::to_owned);
        // The viewport from the last layout (rendered again when it
        // changes), so virtualized content is right before any event.
        if let Some(b) = cx.layout_bounds().filter(|b| b.height > 0.0) {
            let content = self.scroller.content;
            self.scroller.set_sizes(b.size(), content);
        }
        let vp = self.viewport_or_window(cx);
        let area = match self.built {
            Some(b) if self.covers(b, vp) => b.area,
            _ => self.region_around(vp),
        };
        let info = ScrollInfo {
            viewport: self.scroller.viewport,
            region: area,
            virtualized: Cell::new(false),
        };
        let axis = self.axis;
        let content = (self.content)(cx, &info)
            .key("content")
            .layer(cx.layer("content"))
            .style(move |s| {
                s.position = Position::Absolute;
                s.inset.left = LengthPercentageAuto::length(0.0);
                s.inset.top = LengthPercentageAuto::length(0.0);
                match axis {
                    ScrollAxis::Vertical => s.size.width = Dimension::percent(1.0),
                    ScrollAxis::Horizontal => s.size.height = Dimension::percent(1.0),
                    ScrollAxis::Both => {
                        s.min_size.width = LengthPercentageAuto::percent(1.0);
                        s.min_size.height = LengthPercentageAuto::percent(1.0);
                    }
                }
            });
        self.built = info
            .virtualized
            .get()
            .then_some(Built { area, viewport: vp });
        self.sync(cx);

        let mut el = div()
            .name(name)
            .clip()
            .key_context("ScrollView")
            .child(content);
        el = match self.width {
            Some(w) => el.w(w),
            None => el.w_full(),
        };
        el = match self.height {
            Some(h) => el.h(h),
            None => el.h_full(),
        };
        if self.focusable {
            el = el.focusable();
        }
        for vertical in self.axes() {
            let layer = cx.layer(bar_layer_key(vertical));
            el = el.child(thumb(view, vertical, &theme, layer));
        }
        el = self.semantics().apply(el);
        let el = el
            .on_scroll(move |ev, cx| on_scroll(view, ev, cx))
            .on_pointer_move(move |_, cx| {
                let sv = cx.element_id();
                let hidden = !view.read(cx).bars_visible() || view.read(cx).bar_idle > 0.2;
                if hidden {
                    measure(view, cx, sv);
                    poke(view, cx, |_, _| {});
                }
            })
            .on_action(SCROLL_INTO_VIEW, move |cx| reveal(view, cx));
        let p_line = p.line;
        let line = move |d: Vec2| move |s: &Scroller| s.clamp(s.offset) + d * p_line;
        el.on_action(scroll_actions::LINE_UP, move |cx| {
            keyboard(view, cx, line(Vec2::new(0.0, -1.0)));
        })
        .on_action(scroll_actions::LINE_DOWN, move |cx| {
            keyboard(view, cx, line(Vec2::new(0.0, 1.0)));
        })
        .on_action(scroll_actions::LINE_LEFT, move |cx| {
            keyboard(view, cx, line(Vec2::new(-1.0, 0.0)));
        })
        .on_action(scroll_actions::LINE_RIGHT, move |cx| {
            keyboard(view, cx, line(Vec2::new(1.0, 0.0)));
        })
        .on_action(scroll_actions::PAGE_UP, move |cx| {
            keyboard(view, cx, |s| {
                let page = (s.viewport.height - p_line).max(s.viewport.height * 0.5);
                s.clamp(s.offset) - Vec2::new(0.0, page)
            });
        })
        .on_action(scroll_actions::PAGE_DOWN, move |cx| {
            keyboard(view, cx, |s| {
                let page = (s.viewport.height - p_line).max(s.viewport.height * 0.5);
                s.clamp(s.offset) + Vec2::new(0.0, page)
            });
        })
        .on_action(scroll_actions::TOP, move |cx| {
            keyboard(view, cx, |s| Vec2::new(s.offset.x, 0.0));
        })
        .on_action(scroll_actions::BOTTOM, move |cx| {
            keyboard(view, cx, |s| Vec2::new(s.offset.x, s.max().y));
        })
        .semantics(|s| s.role(CoreRole::ScrollView))
    }
}

/// The rows of a uniform list visible at `offset` in a `viewport` of that
/// height, plus `overscan` rows on each side.
#[must_use]
pub fn visible_range(
    offset: f32,
    viewport: f32,
    row_height: f32,
    count: usize,
    overscan: usize,
) -> Range<usize> {
    if count == 0 || row_height <= 0.0 {
        return 0..0;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let first = (offset.max(0.0) / row_height).floor() as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let last = ((offset.max(0.0) + viewport.max(0.0)) / row_height).ceil() as usize;
    let start = first.saturating_sub(overscan).min(count);
    let end = (last + overscan).min(count).max(start);
    start..end
}

/// A virtualized list of `count` rows of one height: a [`ScrollView`] whose
/// content is a spacer of the full height with only the rows of the built
/// region (the view and a margin of at least [`OVERSCAN`] rows) as
/// children, keyed by index (`row-<i>`).
#[derive(Debug, Clone, Copy)]
pub struct VirtualList;

/// Rows built at least beyond each edge of the viewport.
pub const OVERSCAN: usize = 4;

impl VirtualList {
    /// A scroll view over `count` rows built by `row` (called only for
    /// rows near the view; the row is placed and sized by the list).
    #[must_use]
    pub fn build(
        env: &Env,
        count: usize,
        row_height: f32,
        row: impl Fn(&App, usize) -> Element + 'static,
    ) -> ScrollView {
        #[allow(clippy::cast_precision_loss)]
        let total = count as f32 * row_height;
        #[allow(clippy::cast_precision_loss)]
        let overscan = OVERSCAN as f32 * row_height;
        ScrollView::new(env, move |app, info| {
            let region = info.region();
            // The region already reaches past the view.
            let top = region.y.max(0.0);
            let range = visible_range(top, region.bottom() - top, row_height, count, 0);
            let mut spacer = div().name("virtual-list").h(total).cull_offscreen();
            for i in range {
                #[allow(clippy::cast_precision_loss)]
                let y = i as f32 * row_height;
                spacer = spacer.child(
                    div()
                        .key(format!("row-{i}"))
                        .style(move |s| {
                            s.position = Position::Absolute;
                            s.inset.left = LengthPercentageAuto::length(0.0);
                            s.inset.top = LengthPercentageAuto::length(y);
                            s.size.width = Dimension::percent(1.0);
                            s.size.height = Dimension::length(row_height);
                        })
                        .child(row(app, i)),
                );
            }
            spacer.semantics(|s| s.role(CoreRole::List))
        })
        .overscan(overscan)
    }
}
