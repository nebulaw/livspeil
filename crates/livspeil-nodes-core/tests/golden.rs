// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests: cook a table of cases for every math, logic, vector and
//! text node.
//!
//! Each line reads `node(port=value, ...) => out=value, ...` or
//! `=> error: ...`. Inputs not listed in a case take the spec default, and
//! every node's first case uses only defaults. Floats print with `{:?}`
//! (shortest round-trip form, so `-0.0`, `NaN` and `inf` are visible).
//!
//! Refresh with `UPDATE_GOLDEN=1 cargo test -p livspeil-nodes-core --test golden`
//! and review the diff by hand.

use std::fmt::Write as _;
use std::path::Path;

use livspeil_node_api::{CookContext, Value};
use livspeil_nodes_core::core_registry;
use livspeil_testkit::assert_golden;

const NAN: f64 = f64::NAN;
const INF: f64 = f64::INFINITY;

fn f(v: f64) -> Value {
    Value::Float(v)
}

fn b(v: bool) -> Value {
    Value::Bool(v)
}

fn v3(x: f64, y: f64, z: f64) -> Value {
    Value::Vec3([x, y, z])
}

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

fn show(value: &Value) -> String {
    match value {
        Value::Float(v) => format!("{v:?}"),
        Value::Bool(v) => format!("{v}"),
        Value::Int(v) => format!("{v}"),
        Value::Vec3([x, y, z]) => format!("[{x:?}, {y:?}, {z:?}]"),
        Value::String(v) => format!("{v:?}"),
        other => format!("{other:?}"),
    }
}

/// Cook `id` once per case and render one line per case. A case lists only
/// the inputs it overrides; an empty case cooks from the defaults.
fn report(cases: &[(&str, Vec<(&str, Value)>)]) -> String {
    let registry = core_registry();
    let mut out = String::new();
    for (id, overrides) in cases {
        let node = registry
            .get(&format!("{id}@1"))
            .unwrap_or_else(|| panic!("{id} is not registered"));
        let spec = node.spec();
        let mut inputs: Vec<Value> = spec
            .inputs
            .iter()
            .map(|p| p.default.clone().unwrap())
            .collect();
        for (port, value) in overrides {
            let index = spec
                .input_index(port)
                .unwrap_or_else(|| panic!("{id} has no input {port}"));
            inputs[index] = value.clone();
        }
        let args = spec
            .inputs
            .iter()
            .zip(&inputs)
            .map(|(p, v)| format!("{}={}", p.name, show(v)))
            .collect::<Vec<_>>()
            .join(", ");
        let result = match node.cook(&CookContext::default(), &inputs) {
            Ok(values) => spec
                .outputs
                .iter()
                .zip(&values)
                .map(|(p, v)| format!("{}={}", p.name, show(v)))
                .collect::<Vec<_>>()
                .join(", "),
            Err(err) => format!("error: {err}"),
        };
        writeln!(out, "{id}({args}) => {result}").unwrap();
    }
    out
}

fn golden_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[test]
fn math() {
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("core.math.sub", vec![]),
        ("core.math.sub", vec![("a", f(5.0)), ("b", f(3.5))]),
        ("core.math.sub", vec![("a", f(0.3)), ("b", f(0.1))]),
        ("core.math.sub", vec![("a", f(INF)), ("b", f(INF))]),
        ("core.math.mul", vec![]),
        ("core.math.mul", vec![("a", f(2.5))]),
        ("core.math.mul", vec![("a", f(2.5)), ("b", f(-4.0))]),
        ("core.math.mul", vec![("a", f(-0.0)), ("b", f(3.0))]),
        ("core.math.mul", vec![("a", f(0.0)), ("b", f(INF))]),
        ("core.math.mul", vec![("a", f(NAN)), ("b", f(2.0))]),
        ("core.math.div", vec![]),
        ("core.math.div", vec![("a", f(7.0)), ("b", f(2.0))]),
        ("core.math.div", vec![("a", f(1.0)), ("b", f(3.0))]),
        ("core.math.div", vec![("a", f(1.0)), ("b", f(0.0))]),
        ("core.math.div", vec![("a", f(1.0)), ("b", f(-0.0))]),
        ("core.math.div", vec![("a", f(0.0)), ("b", f(0.0))]),
        ("core.math.div", vec![("a", f(1.0)), ("b", f(INF))]),
        ("core.math.div", vec![("a", f(1.0)), ("b", f(NAN))]),
        ("core.math.clamp", vec![]),
        ("core.math.clamp", vec![("x", f(0.25))]),
        ("core.math.clamp", vec![("x", f(-2.0))]),
        ("core.math.clamp", vec![("x", f(3.0))]),
        (
            "core.math.clamp",
            vec![("x", f(3.0)), ("min", f(1.0)), ("max", f(-1.0))],
        ),
        (
            "core.math.clamp",
            vec![("x", f(-3.0)), ("min", f(1.0)), ("max", f(-1.0))],
        ),
        (
            "core.math.clamp",
            vec![("x", f(5.0)), ("min", f(2.0)), ("max", f(2.0))],
        ),
        ("core.math.clamp", vec![("x", f(-INF))]),
        ("core.math.clamp", vec![("x", f(NAN))]),
        ("core.math.clamp", vec![("x", f(5.0)), ("min", f(NAN))]),
        ("core.math.clamp", vec![("x", f(-5.0)), ("min", f(NAN))]),
        ("core.math.remap", vec![]),
        ("core.math.remap", vec![("x", f(0.25))]),
        (
            "core.math.remap",
            vec![
                ("x", f(2.5)),
                ("in_max", f(10.0)),
                ("out_min", f(100.0)),
                ("out_max", f(200.0)),
            ],
        ),
        (
            "core.math.remap",
            vec![
                ("x", f(20.0)),
                ("in_max", f(10.0)),
                ("out_min", f(100.0)),
                ("out_max", f(200.0)),
            ],
        ),
        (
            "core.math.remap",
            vec![
                ("x", f(20.0)),
                ("in_max", f(10.0)),
                ("out_min", f(100.0)),
                ("out_max", f(200.0)),
                ("clamp", b(true)),
            ],
        ),
        ("core.math.remap", vec![("x", f(-1.0)), ("clamp", b(true))]),
        (
            "core.math.remap",
            vec![("x", f(0.25)), ("out_min", f(1.0)), ("out_max", f(0.0))],
        ),
        (
            "core.math.remap",
            vec![
                ("x", f(2.0)),
                ("out_min", f(1.0)),
                ("out_max", f(0.0)),
                ("clamp", b(true)),
            ],
        ),
        (
            "core.math.remap",
            vec![("x", f(0.25)), ("in_min", f(1.0)), ("in_max", f(0.0))],
        ),
        (
            "core.math.remap",
            vec![("x", f(1.0)), ("in_min", f(2.0)), ("in_max", f(2.0))],
        ),
        ("core.math.remap", vec![("x", f(NAN)), ("clamp", b(true))]),
        ("core.math.remap", vec![("x", f(INF))]),
        ("core.math.remap", vec![("x", f(INF)), ("clamp", b(true))]),
        ("core.math.min", vec![]),
        ("core.math.min", vec![("a", f(1.0)), ("b", f(-2.0))]),
        ("core.math.min", vec![("a", f(0.0)), ("b", f(-0.0))]),
        ("core.math.min", vec![("a", f(-0.0)), ("b", f(0.0))]),
        ("core.math.min", vec![("a", f(NAN)), ("b", f(3.0))]),
        ("core.math.min", vec![("a", f(3.0)), ("b", f(NAN))]),
        ("core.math.min", vec![("a", f(NAN)), ("b", f(NAN))]),
        ("core.math.min", vec![("a", f(-INF)), ("b", f(0.0))]),
        ("core.math.max", vec![]),
        ("core.math.max", vec![("a", f(1.0)), ("b", f(-2.0))]),
        ("core.math.max", vec![("a", f(0.0)), ("b", f(-0.0))]),
        ("core.math.max", vec![("a", f(-0.0)), ("b", f(0.0))]),
        ("core.math.max", vec![("a", f(NAN)), ("b", f(3.0))]),
        ("core.math.max", vec![("a", f(3.0)), ("b", f(NAN))]),
        ("core.math.max", vec![("a", f(NAN)), ("b", f(NAN))]),
        ("core.math.max", vec![("a", f(INF)), ("b", f(0.0))]),
        ("core.math.smoothstep", vec![]),
        ("core.math.smoothstep", vec![("x", f(-1.0))]),
        ("core.math.smoothstep", vec![("x", f(0.25))]),
        ("core.math.smoothstep", vec![("x", f(0.5))]),
        ("core.math.smoothstep", vec![("x", f(0.75))]),
        ("core.math.smoothstep", vec![("x", f(1.0))]),
        ("core.math.smoothstep", vec![("x", f(2.0))]),
        (
            "core.math.smoothstep",
            vec![("edge0", f(2.0)), ("edge1", f(6.0)), ("x", f(3.0))],
        ),
        (
            "core.math.smoothstep",
            vec![("edge0", f(1.0)), ("edge1", f(0.0)), ("x", f(0.25))],
        ),
        (
            "core.math.smoothstep",
            vec![("edge0", f(0.5)), ("edge1", f(0.5)), ("x", f(0.4))],
        ),
        (
            "core.math.smoothstep",
            vec![("edge0", f(0.5)), ("edge1", f(0.5)), ("x", f(0.5))],
        ),
        (
            "core.math.smoothstep",
            vec![("edge0", f(0.5)), ("edge1", f(0.5)), ("x", f(0.6))],
        ),
        ("core.math.smoothstep", vec![("x", f(NAN))]),
        ("core.math.smoothstep", vec![("x", f(INF))]),
        ("core.math.abs", vec![]),
        ("core.math.abs", vec![("x", f(-2.5))]),
        ("core.math.abs", vec![("x", f(3.0))]),
        ("core.math.abs", vec![("x", f(-0.0))]),
        ("core.math.abs", vec![("x", f(-INF))]),
        ("core.math.abs", vec![("x", f(NAN))]),
        ("core.math.pow", vec![]),
        (
            "core.math.pow",
            vec![("base", f(2.0)), ("exponent", f(10.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(4.0)), ("exponent", f(0.5))],
        ),
        (
            "core.math.pow",
            vec![("base", f(2.0)), ("exponent", f(-2.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(-2.0)), ("exponent", f(3.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(-8.0)), ("exponent", f(0.5))],
        ),
        (
            "core.math.pow",
            vec![("base", f(NAN)), ("exponent", f(0.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(1.0)), ("exponent", f(NAN))],
        ),
        (
            "core.math.pow",
            vec![("base", f(NAN)), ("exponent", f(2.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(0.0)), ("exponent", f(-1.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(-0.0)), ("exponent", f(-1.0))],
        ),
        (
            "core.math.pow",
            vec![("base", f(0.5)), ("exponent", f(INF))],
        ),
        (
            "core.math.pow",
            vec![("base", f(INF)), ("exponent", f(-1.0))],
        ),
        ("core.math.sin", vec![]),
        ("core.math.sin", vec![("x", f(-0.0))]),
        ("core.math.sin", vec![("x", f(std::f64::consts::FRAC_PI_2))]),
        (
            "core.math.sin",
            vec![("x", f(-std::f64::consts::FRAC_PI_2))],
        ),
        ("core.math.sin", vec![("x", f(INF))]),
        ("core.math.sin", vec![("x", f(NAN))]),
        ("core.math.cos", vec![]),
        ("core.math.cos", vec![("x", f(-0.0))]),
        ("core.math.cos", vec![("x", f(std::f64::consts::PI))]),
        ("core.math.cos", vec![("x", f(-INF))]),
        ("core.math.cos", vec![("x", f(NAN))]),
        ("core.math.wrap", vec![]),
        ("core.math.wrap", vec![("x", f(0.25))]),
        ("core.math.wrap", vec![("x", f(1.25))]),
        ("core.math.wrap", vec![("x", f(-0.25))]),
        ("core.math.wrap", vec![("x", f(1.0))]),
        ("core.math.wrap", vec![("x", f(-1.0))]),
        ("core.math.wrap", vec![("x", f(-0.0))]),
        ("core.math.wrap", vec![("x", f(-1e-300))]),
        (
            "core.math.wrap",
            vec![("x", f(370.0)), ("min", f(0.0)), ("max", f(360.0))],
        ),
        (
            "core.math.wrap",
            vec![("x", f(-190.0)), ("min", f(-180.0)), ("max", f(180.0))],
        ),
        (
            "core.math.wrap",
            vec![("x", f(180.0)), ("min", f(-180.0)), ("max", f(180.0))],
        ),
        (
            "core.math.wrap",
            vec![("x", f(1.25)), ("min", f(1.0)), ("max", f(0.0))],
        ),
        (
            "core.math.wrap",
            vec![("x", f(0.5)), ("min", f(2.0)), ("max", f(2.0))],
        ),
        ("core.math.wrap", vec![("min", f(NAN))]),
        ("core.math.wrap", vec![("max", f(INF))]),
        (
            "core.math.wrap",
            vec![("min", f(-f64::MAX)), ("max", f(f64::MAX))],
        ),
        ("core.math.wrap", vec![("x", f(NAN))]),
        ("core.math.wrap", vec![("x", f(INF))]),
        ("core.math.round", vec![]),
        ("core.math.round", vec![("x", f(2.4))]),
        ("core.math.round", vec![("x", f(2.5))]),
        ("core.math.round", vec![("x", f(-2.5))]),
        ("core.math.round", vec![("x", f(-0.4))]),
        ("core.math.round", vec![("x", f(7.5)), ("step", f(5.0))]),
        ("core.math.round", vec![("x", f(7.5)), ("step", f(-5.0))]),
        ("core.math.round", vec![("x", f(10.0)), ("step", f(3.0))]),
        ("core.math.round", vec![("x", f(0.3)), ("step", f(0.1))]),
        ("core.math.round", vec![("x", f(0.25)), ("step", f(0.1))]),
        (
            "core.math.round",
            vec![("x", f(1.234567)), ("step", f(0.01))],
        ),
        ("core.math.round", vec![("x", f(0.375)), ("step", f(0.25))]),
        ("core.math.round", vec![("x", f(1.0)), ("step", f(0.0))]),
        ("core.math.round", vec![("x", f(1.0)), ("step", f(-0.0))]),
        ("core.math.round", vec![("x", f(1.0)), ("step", f(NAN))]),
        ("core.math.round", vec![("x", f(1.0)), ("step", f(INF))]),
        ("core.math.round", vec![("x", f(NAN))]),
        ("core.math.round", vec![("x", f(-INF)), ("step", f(0.1))]),
        ("core.math.round", vec![("x", f(1e308)), ("step", f(0.01))]),
    ];
    assert_golden(&golden_dir(), "math", &report(&cases));
}

#[test]
fn logic() {
    let mut cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("core.logic.select", vec![]),
        ("core.logic.select", vec![("cond", b(true))]),
        (
            "core.logic.select",
            vec![("cond", b(true)), ("a", f(2.0)), ("b", f(-3.0))],
        ),
        (
            "core.logic.select",
            vec![("cond", b(false)), ("a", f(2.0)), ("b", f(-3.0))],
        ),
        ("core.logic.select", vec![("cond", b(true)), ("b", f(NAN))]),
        ("core.logic.compare", vec![]),
    ];
    for op in ["<", "<=", "==", "!=", ">=", ">"] {
        for (a, bb) in [(1.0, 2.0), (2.0, 2.0), (3.0, 2.0), (NAN, 2.0)] {
            cases.push((
                "core.logic.compare",
                vec![("a", f(a)), ("b", f(bb)), ("op", s(op))],
            ));
        }
    }
    cases.extend([
        (
            "core.logic.compare",
            vec![("a", f(-0.0)), ("b", f(0.0)), ("op", s("=="))],
        ),
        (
            "core.logic.compare",
            vec![("a", f(INF)), ("b", f(INF)), ("op", s("=="))],
        ),
        (
            "core.logic.compare",
            vec![("a", f(0.1 + 0.2)), ("b", f(0.3)), ("op", s("=="))],
        ),
        ("core.logic.compare", vec![("op", s(""))]),
        ("core.logic.compare", vec![("op", s("=>"))]),
        ("core.logic.compare", vec![("op", s(" <"))]),
        ("core.logic.and", vec![]),
        ("core.logic.or", vec![]),
        ("core.logic.not", vec![]),
    ]);
    for a in [false, true] {
        for bb in [false, true] {
            cases.push(("core.logic.and", vec![("a", b(a)), ("b", b(bb))]));
        }
    }
    for a in [false, true] {
        for bb in [false, true] {
            cases.push(("core.logic.or", vec![("a", b(a)), ("b", b(bb))]));
        }
    }
    for a in [false, true] {
        cases.push(("core.logic.not", vec![("a", b(a))]));
    }
    assert_golden(&golden_dir(), "logic", &report(&cases));
}

#[test]
fn vector() {
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("core.vector.compose3", vec![]),
        (
            "core.vector.compose3",
            vec![("x", f(1.5)), ("y", f(-0.0)), ("z", f(NAN))],
        ),
        ("core.vector.decompose3", vec![]),
        ("core.vector.decompose3", vec![("v", v3(1.5, -2.0, INF))]),
        ("core.vector.length3", vec![]),
        ("core.vector.length3", vec![("v", v3(3.0, 4.0, 0.0))]),
        ("core.vector.length3", vec![("v", v3(2.0, -3.0, 6.0))]),
        ("core.vector.length3", vec![("v", v3(1.0, 1.0, 1.0))]),
        ("core.vector.length3", vec![("v", v3(-0.0, 0.0, -0.0))]),
        ("core.vector.length3", vec![("v", v3(3e200, 4e200, 0.0))]),
        ("core.vector.length3", vec![("v", v3(3e-200, 4e-200, 0.0))]),
        ("core.vector.length3", vec![("v", v3(f64::MAX, 0.0, 0.0))]),
        ("core.vector.length3", vec![("v", v3(5e-324, 0.0, 0.0))]),
        ("core.vector.length3", vec![("v", v3(-INF, 1.0, 0.0))]),
        ("core.vector.length3", vec![("v", v3(INF, NAN, 0.0))]),
        ("core.vector.normalize3", vec![]),
        ("core.vector.normalize3", vec![("v", v3(-0.0, 0.0, -0.0))]),
        ("core.vector.normalize3", vec![("v", v3(3.0, 0.0, 4.0))]),
        ("core.vector.normalize3", vec![("v", v3(0.0, -7.0, 0.0))]),
        ("core.vector.normalize3", vec![("v", v3(2.0, 3.0, 6.0))]),
        ("core.vector.normalize3", vec![("v", v3(1.0, 1.0, 1.0))]),
        (
            "core.vector.normalize3",
            vec![("v", v3(3e-200, 4e-200, 0.0))],
        ),
        ("core.vector.normalize3", vec![("v", v3(3e200, 4e200, 0.0))]),
        ("core.vector.normalize3", vec![("v", v3(INF, 5.0, 0.0))]),
        ("core.vector.normalize3", vec![("v", v3(INF, -INF, 1.0))]),
        ("core.vector.normalize3", vec![("v", v3(NAN, 1.0, 0.0))]),
        ("core.vector.dot3", vec![]),
        (
            "core.vector.dot3",
            vec![("a", v3(1.0, 2.0, 3.0)), ("b", v3(4.0, -5.0, 6.0))],
        ),
        (
            "core.vector.dot3",
            vec![("a", v3(1.0, 0.0, 0.0)), ("b", v3(0.0, 1.0, 0.0))],
        ),
        (
            "core.vector.dot3",
            vec![("a", v3(0.6, 0.8, 0.0)), ("b", v3(0.6, 0.8, 0.0))],
        ),
        (
            "core.vector.dot3",
            vec![("a", v3(INF, 0.0, 0.0)), ("b", v3(0.0, 1.0, 0.0))],
        ),
        (
            "core.vector.dot3",
            vec![("a", v3(1e200, 0.0, 0.0)), ("b", v3(1e200, 0.0, 0.0))],
        ),
    ];
    assert_golden(&golden_dir(), "vector", &report(&cases));
}

#[test]
fn text() {
    let t = "core.text.format";
    let i = Value::Int;
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        (t, vec![]),
        (t, vec![("a", f(12.3456))]),
        (
            t,
            vec![
                ("template", s("{a} of {b}")),
                ("a", f(3.0)),
                ("b", f(10.0)),
                ("precision", i(0)),
            ],
        ),
        (
            t,
            vec![
                ("template", s("{b}/{a}/{b}")),
                ("a", f(1.0)),
                ("b", f(2.0)),
                ("precision", i(1)),
            ],
        ),
        (
            t,
            vec![
                ("template", s("Temp: {a} °C")),
                ("a", f(21.456)),
                ("precision", i(1)),
            ],
        ),
        (t, vec![("template", s("plain text"))]),
        (t, vec![("template", s(""))]),
        (t, vec![("template", s("{{a}} = {a}")), ("a", f(1.0))]),
        (t, vec![("template", s("{{{a}}}")), ("a", f(1.0))]),
        (t, vec![("template", s("}}{{"))]),
        (t, vec![("a", f(0.125))]),
        (t, vec![("a", f(2.675))]),
        (t, vec![("a", f(-0.0))]),
        (t, vec![("a", f(-0.001))]),
        (t, vec![("a", f(-0.006))]),
        (t, vec![("a", f(2.5)), ("precision", i(0))]),
        (t, vec![("a", f(3.5)), ("precision", i(0))]),
        (t, vec![("a", f(1.0 / 3.0)), ("precision", i(12))]),
        (t, vec![("a", f(1.0 / 3.0)), ("precision", i(99))]),
        (t, vec![("a", f(1.0 / 3.0)), ("precision", i(-1))]),
        (t, vec![("a", f(1e21)), ("precision", i(0))]),
        (
            t,
            vec![("template", s("{a} {b}")), ("a", f(NAN)), ("b", f(-INF))],
        ),
        (t, vec![("template", s("{c}"))]),
        (t, vec![("template", s("x{}"))]),
        (t, vec![("template", s("{ a }"))]),
        (t, vec![("template", s("ok {"))]),
        (t, vec![("template", s("{a}}"))]),
        (t, vec![("template", s("é}"))]),
    ];
    assert_golden(&golden_dir(), "text", &report(&cases));
}
