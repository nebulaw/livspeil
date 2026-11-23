// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Spring and tween math, the animation driver, motion config, budgets.

use std::time::Duration;

use livspeil_ui_core::{
    Animated, App, Easing, Element, Model, Render, TestHost, TransformParts, ViewContext, div,
    spring_step,
};
use livspeil_ui_layout::{Rect, Size, Vec2};
use livspeil_ui_theme::{Color, MotionClass, MotionConfig, MotionSpec, SpringSpec};

const DT: f32 = 1.0 / 120.0;

fn settle_time(spring: SpringSpec, from: f32, to: f32) -> (f32, f32) {
    let mut a = Animated::new(from);
    a.spring_to(to, spring);
    let mut t = 0.0;
    let mut peak = from;
    while a.step(DT) {
        t += DT;
        peak = if to > from {
            peak.max(a.value())
        } else {
            peak.min(a.value())
        };
        assert!(t < 10.0, "did not settle");
    }
    assert_eq!(a.value(), to);
    (t, peak)
}

#[test]
fn critically_damped_settles_without_overshoot() {
    let (t, peak) = settle_time(SpringSpec::new(0.3, 1.0), 0.0, 100.0);
    assert!(peak <= 100.0 + 1e-3, "overshoot {peak}");
    assert!(t > 0.2 && t < 1.2, "settle {t}");
}

#[test]
fn underdamped_overshoots_then_settles() {
    let (t, peak) = settle_time(SpringSpec::new(0.3, 0.5), 0.0, 100.0);
    assert!(peak > 110.0, "peak {peak}");
    assert!(t < 3.0);
    let (_, peak) = settle_time(SpringSpec::new(0.3, 2.0), 0.0, 100.0);
    assert!(peak <= 100.0 + 1e-3, "overdamped overshoot {peak}");
}

#[test]
fn spring_step_is_frame_rate_independent() {
    let omega = std::f32::consts::TAU / 0.25;
    for zeta in [0.6, 1.0, 1.5] {
        let (mut x1, mut v1) = (50.0_f32, -20.0_f32);
        for _ in 0..120 {
            (x1, v1) = spring_step(x1, v1, 1.0 / 120.0, omega, zeta);
        }
        let (x2, v2) = spring_step(50.0, -20.0, 1.0, omega, zeta);
        assert!(
            (x1 - x2).abs() < 1e-3 && (v1 - v2).abs() < 1e-2,
            "zeta {zeta}: {x1} {x2}"
        );
    }
}

#[test]
fn retarget_preserves_velocity() {
    let spring = SpringSpec::new(0.2, 0.9);
    let mut a = Animated::new(0.0_f32);
    a.spring_to(100.0, spring);
    for _ in 0..6 {
        a.step(DT);
    }
    let (value, velocity) = (a.value(), a.velocity());
    assert!(velocity > 100.0);
    a.spring_to(-50.0, spring);
    assert_eq!(a.value(), value, "no jump");
    assert_eq!(a.velocity(), velocity, "velocity kept");
    a.step(DT);
    assert!(a.value() > value, "keeps moving forward before turning");

    // Gesture hand-off: a resting value given a velocity moves that way.
    let mut b = Animated::new(Vec2::ZERO);
    b.set_velocity(Vec2::new(800.0, 0.0));
    b.spring_to(Vec2::ZERO, spring);
    b.step(DT);
    assert!(b.value().x > 0.0);
    while b.step(DT) {}
    assert_eq!(b.value(), Vec2::ZERO);
}

#[test]
fn speed_zero_and_off_are_instant() {
    let zero = MotionConfig::parse("speed = 0").unwrap();
    let spec = zero.resolve(
        None,
        MotionClass::Large,
        livspeil_ui_theme::PropertyKind::Movement,
    );
    assert_eq!(spec, MotionSpec::Instant);
    let mut a = Animated::new(0.0_f32);
    assert!(!a.animate_to(10.0, spec));
    assert_eq!(a.value(), 10.0);
    // Zero response is instant too.
    assert!(!a.spring_to(3.0, SpringSpec::new(0.0, 1.0)));
    assert_eq!(a.value(), 3.0);
    // Speed 2 halves the response.
    let fast = MotionConfig::parse("speed = 2").unwrap();
    let spec = fast.resolve(
        None,
        MotionClass::Micro,
        livspeil_ui_theme::PropertyKind::Movement,
    );
    assert_eq!(spec, MotionSpec::Spring(SpringSpec::new(0.05, 0.9)));
}

#[test]
fn colors_animate_in_oklab_and_transforms_by_parts() {
    let red = Color::oklch(0.6, 0.2, 25.0, 1.0);
    let blue = Color::oklch(0.6, 0.2, 260.0, 1.0);
    let mut c = Animated::new(red);
    c.tween_to(blue, Duration::from_millis(100), Easing::Linear);
    for _ in 0..6 {
        c.step(1.0 / 120.0);
    }
    let mid = c.value();
    let lerp = red.lerp(blue, 0.5);
    assert!((mid.l - lerp.l).abs() < 1e-3 && (mid.a - lerp.a).abs() < 1e-3);
    while c.step(DT) {}
    assert_eq!(c.value(), blue);

    let mut t = Animated::new(TransformParts::default());
    let target = TransformParts {
        rotation: std::f32::consts::PI,
        ..TransformParts::default()
    };
    t.spring_to(target, SpringSpec::new(0.2, 1.0));
    for _ in 0..10 {
        t.step(DT);
    }
    // Rotating by parts keeps the scale (a matrix lerp would shrink it).
    let m = t.value().to_transform();
    let det = m.a * m.d - m.b * m.c;
    assert!((det - 1.0).abs() < 1e-4);
}

#[test]
fn easings() {
    for e in [
        Easing::Linear,
        Easing::EaseIn,
        Easing::EaseOut,
        Easing::EaseInOut,
        Easing::CubicBezier(0.25, 0.1, 0.25, 1.0),
    ] {
        assert!(e.apply(0.0).abs() < 1e-4, "{e:?}");
        assert!((e.apply(1.0) - 1.0).abs() < 1e-4, "{e:?}");
    }
    assert!(Easing::EaseIn.apply(0.5) < 0.5);
    assert!(Easing::EaseOut.apply(0.5) > 0.5);
    // CSS `ease` at x = 0.5 is ~0.8024.
    assert!((Easing::CubicBezier(0.25, 0.1, 0.25, 1.0).apply(0.5) - 0.8024).abs() < 2e-3);
}

// ---- driver ------------------------------------------------------------

struct Boxes {
    targets: Model<Vec<f32>>,
    class: MotionClass,
    hover: bool,
}

impl Render for Boxes {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let class = self.class;
        let hover = self.hover;
        let targets = self.targets.read(cx).clone();
        div()
            .size(400.0, 400.0)
            .children(targets.into_iter().enumerate().map(move |(i, x)| {
                let e = div()
                    .key(format!("b{i}"))
                    .absolute(Rect::new(0.0, i as f32 * 20.0, 20.0, 20.0))
                    .translate(Vec2::new(x, 0.0))
                    .transition(class);
                if hover {
                    e.bg(Color::BLACK).hover(|r| r.bg(Color::WHITE).scale(1.1))
                } else {
                    e
                }
            }))
    }
}

fn boxes(
    n: usize,
    class: MotionClass,
    hover: bool,
    motion: MotionConfig,
) -> (TestHost, Model<Vec<f32>>) {
    let mut app = App::new();
    app.set_motion(motion);
    let targets = app.new_model(vec![0.0; n]);
    let v = app.new_view(Boxes {
        targets,
        class,
        hover,
    });
    app.set_root(v);
    let mut host = TestHost::new(app, Size::new(400.0, 400.0));
    host.run_until_idle(10);
    (host, targets)
}

fn set_all(host: &mut TestHost, m: Model<Vec<f32>>, x: f32) {
    m.update(&mut host.app, |v, cx| {
        v.iter_mut().for_each(|t| *t = x);
        cx.notify();
    });
}

#[test]
fn animation_frames_stop_when_settled() {
    let (mut host, m) = boxes(3, MotionClass::Standard, false, MotionConfig::default());
    assert!(!host.app.needs_frame());
    set_all(&mut host, m, 100.0);
    let frames = host.run_until_idle(500);
    assert!(frames > 5 && frames < 120, "{frames} frames");
    assert!(!host.app.needs_frame());
    assert_eq!(host.app.animation_stats().active, 0);
    let b0 = host.app.find("b0").unwrap();
    assert_eq!(host.app.translation(b0), Some(Vec2::new(100.0, 0.0)));
    // Idle afterwards.
    assert_eq!(host.advance(Duration::from_secs(1)), 0);
}

#[test]
fn reduce_motion_jumps_movement_but_fades_appearance() {
    let motion = MotionConfig::parse("reduce_motion = true").unwrap();
    let (mut host, m) = boxes(1, MotionClass::Standard, true, motion);
    set_all(&mut host, m, 100.0);
    host.step();
    let b0 = host.app.find("b0").unwrap();
    assert_eq!(
        host.app.translation(b0),
        Some(Vec2::new(100.0, 0.0)),
        "movement is instant"
    );
    assert!(!host.app.needs_frame());
    // Hover: scale (movement) jumps, color (appearance) animates.
    let p = host.app.bounds(b0).unwrap().center() + Vec2::new(100.0, 0.0);
    host.pointer_move(p.x, p.y);
    assert!(host.app.is_hovered(b0));
    host.step();
    assert_eq!(host.app.scale_of(b0), Some(1.1));
    let bg = host.app.background(b0).unwrap();
    assert!(bg.l > 0.0 && bg.l < 1.0, "fading: {bg:?}");
    assert!(host.run_until_idle(500) > 2);
    assert_eq!(host.app.background(b0), Some(Color::WHITE));
}

#[test]
fn speed_zero_makes_the_driver_instant() {
    let (mut host, m) = boxes(
        2,
        MotionClass::Large,
        false,
        MotionConfig::parse("speed = 0").unwrap(),
    );
    set_all(&mut host, m, 50.0);
    assert!(host.step().is_some());
    assert!(!host.app.needs_frame(), "nothing left to animate");
    let b1 = host.app.find("b1").unwrap();
    assert_eq!(host.app.translation(b1), Some(Vec2::new(50.0, 0.0)));
}

#[test]
fn at_most_two_large_animations() {
    let (mut host, m) = boxes(3, MotionClass::Large, false, MotionConfig::default());
    set_all(&mut host, m, 200.0);
    host.step();
    let stats = host.app.animation_stats();
    assert_eq!(stats.large, 2);
    assert_eq!(stats.large_evicted, 1);
    // The evicted one is already at its target; the others are moving.
    let at_target = (0..3)
        .filter(|i| {
            let id = host.app.find(&format!("b{i}")).unwrap();
            host.app.translation(id) == Some(Vec2::new(200.0, 0.0))
        })
        .count();
    assert_eq!(at_target, 1);
    host.run_until_idle(500);
    assert_eq!(host.app.animation_stats().large, 0);
}

#[test]
fn micro_animations_do_not_grow_storage() {
    let (mut host, _m) = boxes(8, MotionClass::Micro, true, MotionConfig::default());
    let warm = host.app.animation_stats().capacity;
    let nodes = host.last_frame().unwrap().stats.nodes;
    // Sweep the pointer over every box many times, interrupting animations.
    for round in 0..50 {
        for i in 0..8 {
            let y = i as f32 * 20.0 + 10.0;
            host.pointer_move(10.0 + (round % 3) as f32, y);
            host.step();
        }
    }
    let stats = host.app.animation_stats();
    assert_eq!(stats.capacity, warm, "active list never reallocated");
    assert!(host.run_until_idle(500) > 0);
    assert_eq!(host.last_frame().unwrap().stats.nodes, nodes);
    // `Animated` is plain inline data.
    assert!(std::mem::size_of::<Animated<Color>>() <= 128);
    fn is_copy<T: Copy>() {}
    is_copy::<Animated<Color>>();
    is_copy::<Animated<TransformParts>>();
}

#[test]
fn motion_hot_reload_applies_to_next_retarget() {
    let (mut host, m) = boxes(1, MotionClass::Micro, false, MotionConfig::default());
    set_all(&mut host, m, 100.0);
    let normal = host.run_until_idle(500);
    host.app
        .apply_config_event(livspeil_ui_theme::ConfigEvent::Motion(
            MotionConfig::parse("speed = 0.25").unwrap(),
        ))
        .unwrap();
    set_all(&mut host, m, 0.0);
    let slow = host.run_until_idle(2000);
    assert!(slow > normal * 3, "slow {slow} vs normal {normal}");
}
