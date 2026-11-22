// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests: cook a table of cases for every math and logic node.
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

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

fn show(value: &Value) -> String {
    match value {
        Value::Float(v) => format!("{v:?}"),
        Value::Bool(v) => format!("{v}"),
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
