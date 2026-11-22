// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden test: sample animation channels with every interpolation and
//! extrapolation mode.
//!
//! Refresh with `UPDATE_GOLDEN=1 cargo test -p livspeil-params --test anim_golden`
//! and review every line of the diff by hand.

use std::fmt::Write as _;
use std::path::Path;

use livspeil_node_api::Value;
use livspeil_params::expr::format_value;
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe, Tangent};
use livspeil_testkit::assert_golden;

fn floats(interp: Interpolation) -> AnimChannel {
    AnimChannel::new(vec![
        Keyframe::float(0.0, 0.0).with_interpolation(interp),
        Keyframe::float(1.0, 1.0).with_interpolation(interp),
        Keyframe::float(1.5, 0.25).with_interpolation(interp),
        Keyframe::float(3.0, 2.0),
    ])
    .expect("valid channel")
}

fn cases() -> Vec<(&'static str, AnimChannel)> {
    let spring = |damping| Interpolation::Spring {
        frequency: 1.0,
        damping,
    };
    vec![
        ("step", floats(Interpolation::Step)),
        ("linear", floats(Interpolation::Linear)),
        ("bezier auto", floats(Interpolation::Bezier)),
        (
            "bezier flat",
            AnimChannel::new(vec![
                Keyframe::float(0.0, 0.0)
                    .with_interpolation(Interpolation::Bezier)
                    .with_tangent(Tangent::Flat),
                Keyframe::float(2.0, 1.0).with_tangent(Tangent::Flat),
            ])
            .expect("valid"),
        ),
        (
            "bezier explicit",
            AnimChannel::new(vec![
                Keyframe::float(0.0, 0.0)
                    .with_interpolation(Interpolation::Bezier)
                    .with_tangent(Tangent::Explicit {
                        incoming: Value::Float(0.0),
                        outgoing: Value::Float(4.0),
                    }),
                Keyframe::float(2.0, 1.0).with_tangent(Tangent::Explicit {
                    incoming: Value::Float(-1.0),
                    outgoing: Value::Float(0.0),
                }),
            ])
            .expect("valid"),
        ),
        ("spring 0.2", floats(spring(0.2))),
        ("spring 1.0", floats(spring(1.0))),
        ("spring 3.0", floats(spring(3.0))),
        (
            "loop",
            floats(Interpolation::Linear)
                .with_extrapolation(Extrapolation::Loop, Extrapolation::Loop),
        ),
        (
            "ping-pong offset 0.5",
            floats(Interpolation::Linear)
                .with_extrapolation(Extrapolation::PingPong, Extrapolation::PingPong)
                .with_offset(0.5)
                .expect("finite"),
        ),
        (
            "vec3 bezier",
            AnimChannel::new(vec![
                Keyframe::new(0.0, Value::Vec3([0.0, 1.0, -1.0]))
                    .with_interpolation(Interpolation::Bezier),
                Keyframe::new(1.0, Value::Vec3([1.0, 1.0, 0.0]))
                    .with_interpolation(Interpolation::Bezier),
                Keyframe::new(2.0, Value::Vec3([0.0, 1.0, 2.0])),
            ])
            .expect("valid"),
        ),
        (
            "color linear",
            AnimChannel::new(vec![
                Keyframe::new(0.0, Value::Color([0.2, 0.05, 30.0, 1.0])),
                Keyframe::new(3.0, Value::Color([0.9, 0.15, 90.0, 0.5])),
            ])
            .expect("valid"),
        ),
    ]
}

#[test]
fn anim_golden() {
    let mut report = String::new();
    for (name, channel) in cases() {
        writeln!(report, "# {name}").unwrap();
        for i in -4..=16 {
            let t = f64::from(i) * 0.25;
            writeln!(report, "{t:>5.2} => {}", format_value(&channel.sample(t))).unwrap();
        }
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    assert_golden(&dir, "anim", &report);
}

#[test]
fn channel_refs_report_literals_in_order() {
    use livspeil_params::expr::{Span, channel_refs, quote};
    let src = r#"ch("b/x") + (true ? ch("../a/y") : lerp(0, ch("b/x"), 1))"#;
    let refs = channel_refs(src).unwrap();
    let paths: Vec<_> = refs.iter().map(|r| r.path.as_str()).collect();
    assert_eq!(paths, ["b/x", "../a/y", "b/x"]);
    assert_eq!(refs[0].span, Span::new(3, 8));
    assert_eq!(&src[refs[1].span.start..refs[1].span.end], "\"../a/y\"");
    assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    assert!(channel_refs("1 +").is_err());
}
