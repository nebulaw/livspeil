// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests for the time, sky and color nodes (M0.6b).
//!
//! Each test cooks a table of cases and prints one line per case:
//! `node(inputs) => outputs`, with six decimals so platform float noise
//! cannot break the files. Refresh with
//! `UPDATE_GOLDEN=1 cargo test -p livspeil-nodes-core --test golden_time_sky_color`
//! and review the diff by hand.

use std::fmt::Write as _;
use std::path::Path;

use livspeil_node_api::{CookContext, Value};
use livspeil_nodes_core::core_registry;
use livspeil_testkit::assert_golden;

/// Six decimals, with negative zero printed as zero so the sign of a
/// rounding-noise value cannot flip a golden line.
fn num(x: f64) -> String {
    let s = format!("{x:.6}");
    if s.trim_start_matches('-')
        .chars()
        .all(|c| c == '0' || c == '.')
    {
        s.trim_start_matches('-').to_owned()
    } else {
        s
    }
}

fn fmt_value(v: &Value) -> String {
    let list = |xs: &[f64]| xs.iter().map(|x| num(*x)).collect::<Vec<_>>().join(", ");
    match v {
        Value::Float(x) => num(*x),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Vec2(xs) => format!("[{}]", list(xs)),
        Value::Vec3(xs) => format!("[{}]", list(xs)),
        Value::Vec4(xs) => format!("[{}]", list(xs)),
        Value::Color(xs) => format!("oklch[{}]", list(xs)),
        Value::String(s) => format!("{s:?}"),
        other => format!("{other:?}"),
    }
}

fn report(id: &str, cases: &[Vec<Value>]) -> String {
    let reg = core_registry();
    let node = reg.get(id).unwrap();
    let spec = node.spec();
    let mut out = String::new();
    for inputs in cases {
        let args = spec
            .inputs
            .iter()
            .zip(inputs)
            .map(|(p, v)| format!("{}={}", p.name, fmt_value(v)))
            .collect::<Vec<_>>()
            .join(", ");
        let result = match node.cook(&CookContext::default(), inputs) {
            Ok(values) => spec
                .outputs
                .iter()
                .zip(&values)
                .map(|(p, v)| format!("{}={}", p.name, fmt_value(v)))
                .collect::<Vec<_>>()
                .join(", "),
            Err(err) => format!("error: {err}"),
        };
        writeln!(out, "{}({args}) => {result}", spec.id).unwrap();
    }
    out
}

fn dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn floats(xs: &[f64]) -> Vec<Value> {
    xs.iter().map(|x| Value::Float(*x)).collect()
}

#[test]
fn time() {
    let cases: Vec<Vec<Value>> = [
        [0.0, 0.0],
        [21_600.0, 0.0],
        [43_200.0, 0.0],
        [64_800.0, 0.0],
        [86_399.0, 0.0],
        [86_400.0, 0.0],
        [-3_600.0, 0.0],
        [-90_000.0, 0.0],
        [300_000.0, 0.0],
        [45_296.0, 0.0],
        [43_200.0, 2.0],
        [3_600.0, -2.5],
        [43_200.0, 36.0],
    ]
    .iter()
    .map(|c| floats(c))
    .collect();
    let mut text = report("core.time.timeofday@1", &cases);
    text.push_str(&report(
        "core.time.timeofday@1",
        &[vec![Value::Float(f64::NAN), Value::Float(0.0)]],
    ));
    assert_golden(&dir(), "time", &text);
}

#[test]
fn sky() {
    let mut cases = Vec::new();
    // A summer and a winter day at the default latitude, every three hours.
    for day in [172.0, 355.0] {
        for hour in [0.0, 6.0, 9.0, 12.0, 15.0, 18.0] {
            cases.push(floats(&[day, hour, 41.7]));
        }
    }
    // Equinox at the equator, southern hemisphere, the poles, an odd time.
    for c in [
        [80.0, 12.0, 0.0],
        [80.0, 6.0, 0.0],
        [172.0, 12.0, -33.9],
        [355.0, 12.0, -33.9],
        [172.0, 12.0, 90.0],
        [172.0, 0.0, 90.0],
        [355.0, 12.0, -90.0],
        [266.0, 7.5, 59.3],
        [1.0, 16.25, 51.5],
        [172.0, 12.0, 91.0],
        [172.0, 12.0, -90.5],
        // day_of_year bounds: 1 and 366 are valid, outside is an error.
        [366.0, 12.0, 41.7],
        [0.0, 12.0, 41.7],
        [0.5, 12.0, 41.7],
        [366.5, 12.0, 41.7],
        [-10.0, 12.0, 41.7],
        [f64::NAN, 12.0, 41.7],
        [f64::INFINITY, 12.0, 41.7],
    ] {
        cases.push(floats(&c));
    }
    assert_golden(&dir(), "sky", &report("core.sky.sun_direction@1", &cases));
}

#[test]
fn color() {
    let a = [0.8, 0.12, 60.0, 1.0];
    let b = [0.9, 0.08, 80.0, 1.0];
    let mut cases = Vec::new();
    for t in [-1.0, 0.0, 0.25, 0.5, 0.75, 1.0, 2.0] {
        cases.push(vec![Value::Color(a), Value::Color(b), Value::Float(t)]);
    }
    for (a, b, t) in [
        // Across the 0/360 seam: the short way.
        ([0.6, 0.1, 350.0, 1.0], [0.6, 0.1, 10.0, 1.0], 0.5),
        ([0.6, 0.1, 350.0, 1.0], [0.6, 0.1, 10.0, 1.0], 0.25),
        // Complementary: passes through grey, hue 0.
        ([0.5, 0.1, 30.0, 1.0], [0.5, 0.1, 210.0, 0.0], 0.5),
        ([0.5, 0.1, 30.0, 1.0], [0.5, 0.1, 210.0, 0.0], 0.3),
        // Grey to a color: hue of the color, chroma ramps.
        ([0.2, 0.0, 0.0, 1.0], [0.7, 0.15, 250.0, 1.0], 0.5),
        // Warm lamp to cool daylight, with alpha.
        ([0.75, 0.13, 55.0, 0.8], [0.95, 0.03, 240.0, 0.2], 0.4),
        // Hues outside [0, 360) on input.
        ([0.5, 0.1, -30.0, 1.0], [0.5, 0.1, 400.0, 1.0], 0.0),
        ([0.5, 0.1, -30.0, 1.0], [0.5, 0.1, 400.0, 1.0], 1.0),
    ] {
        cases.push(vec![Value::Color(a), Value::Color(b), Value::Float(t)]);
    }
    assert_golden(&dir(), "color", &report("core.color.mix@1", &cases));
}
