// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests for the table, curve and file-system nodes (M0.8).
//!
//! Each line reads `node(port=value, ...) => out=value, ...` or
//! `=> error: ...`. Inputs not listed in a case take the spec default.
//! Tables print as `table[col:Type, ...]{(cell, ...), ...}`, floats with
//! `{:?}`. File-system cases cook against an in-memory file system
//! ([`MemFs`]), so they are identical on every machine.
//!
//! Refresh with
//! `UPDATE_GOLDEN=1 cargo test -p livspeil-nodes-core --test golden_table_curve_fs`
//! and review the diff by hand.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use livspeil_node_api::{
    CookContext, Curve, CurveExtrapolation, CurveInterpolation, CurveKey, CurveTangent, PortType,
    SystemContext, Table, Value,
};
use livspeil_nodes_core::{MemFs, core_registry};
use livspeil_testkit::assert_golden;

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

fn i(v: i64) -> Value {
    Value::Int(v)
}

fn f(v: f64) -> Value {
    Value::Float(v)
}

fn b(v: bool) -> Value {
    Value::Bool(v)
}

fn show(value: &Value) -> String {
    match value {
        Value::Float(v) => format!("{v:?}"),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::String(v) => format!("{v:?}"),
        Value::Vec2(v) => format!("{v:?}"),
        Value::Table(t) => {
            let cols: Vec<String> = t
                .columns()
                .iter()
                .map(|c| format!("{}:{}", c.name(), c.ty()))
                .collect();
            let rows: Vec<String> = (0..t.row_count())
                .map(|r| {
                    let cells: Vec<String> = t.row(r).map(show).collect();
                    format!("({})", cells.join(", "))
                })
                .collect();
            format!("table[{}]{{{}}}", cols.join(", "), rows.join(", "))
        }
        Value::Curve(c) => {
            let keys: Vec<String> = c
                .keys()
                .iter()
                .map(|k| format!("{:?}:{:?}", k.position, k.value))
                .collect();
            format!("curve({})", keys.join(" "))
        }
        other => format!("{other:?}"),
    }
}

fn report(system: &SystemContext<'_>, cases: &[(&str, Vec<(&str, Value)>)]) -> String {
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
        let result = match node.cook_with_system(&CookContext::default(), system, &inputs) {
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

fn books() -> Value {
    Value::Table(
        Table::from_rows(
            &[
                ("name", PortType::String),
                ("pages", PortType::Int),
                ("rating", PortType::Float),
                ("read", PortType::Bool),
                ("size", PortType::Vec2),
            ],
            [
                vec![s("Dune"), i(412), f(4.5), b(true), Value::Vec2([1.0, 2.0])],
                vec![
                    s("Emma"),
                    i(474),
                    f(3.75),
                    b(false),
                    Value::Vec2([1.0, 1.0]),
                ],
                vec![
                    s("dracula"),
                    i(418),
                    f(-0.0),
                    b(true),
                    Value::Vec2([0.5, 2.0]),
                ],
                vec![
                    s("Ubik"),
                    i(202),
                    f(f64::NAN),
                    b(false),
                    Value::Vec2([2.0, 0.0]),
                ],
                vec![
                    s("Émile"),
                    i(412),
                    f(0.0),
                    b(false),
                    Value::Vec2([1.0, 2.0]),
                ],
            ],
        )
        .unwrap(),
    )
}

fn list() -> Value {
    Value::Table(
        Table::list(
            "tag",
            PortType::String,
            vec![s("sci-fi"), s("classic"), s("Sci-Fi"), s("")],
        )
        .unwrap(),
    )
}

#[test]
fn table() {
    let none = SystemContext::none();
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("core.table.count", vec![]),
        ("core.table.count", vec![("table", books())]),
        ("core.table.count", vec![("table", list())]),
        ("core.table.column", vec![]),
        ("core.table.column", vec![("table", books())]),
        (
            "core.table.column",
            vec![("table", books()), ("name", s("rating"))],
        ),
        (
            "core.table.column",
            vec![("table", books()), ("name", s("Name"))],
        ),
        (
            "core.table.column",
            vec![("table", list()), ("name", s("tag"))],
        ),
        ("core.table.get", vec![]),
        ("core.table.get", vec![("table", books())]),
        ("core.table.get", vec![("table", books()), ("row", i(4))]),
        ("core.table.get", vec![("table", books()), ("row", i(5))]),
        ("core.table.get", vec![("table", books()), ("row", i(-1))]),
        (
            "core.table.get",
            vec![("table", books()), ("column", s("pages"))],
        ),
        (
            "core.table.get",
            vec![("table", books()), ("column", s("rating"))],
        ),
        (
            "core.table.get",
            vec![("table", books()), ("row", i(2)), ("column", s("rating"))],
        ),
        (
            "core.table.get",
            vec![("table", books()), ("row", i(3)), ("column", s("rating"))],
        ),
        (
            "core.table.get",
            vec![("table", books()), ("column", s("read"))],
        ),
        (
            "core.table.get",
            vec![("table", books()), ("column", s("size"))],
        ),
        (
            "core.table.get",
            vec![("table", books()), ("column", s("nope"))],
        ),
        ("core.table.filter_text", vec![]),
        ("core.table.filter_text", vec![("table", books())]),
        (
            "core.table.filter_text",
            vec![("table", books()), ("text", s("u"))],
        ),
        (
            "core.table.filter_text",
            vec![("table", books()), ("text", s("D"))],
        ),
        (
            "core.table.filter_text",
            vec![
                ("table", books()),
                ("text", s("D")),
                ("ignore_case", b(true)),
            ],
        ),
        (
            "core.table.filter_text",
            vec![
                ("table", books()),
                ("text", s("É")),
                ("ignore_case", b(true)),
            ],
        ),
        (
            "core.table.filter_text",
            vec![("table", books()), ("text", s("zzz"))],
        ),
        (
            "core.table.filter_text",
            vec![("table", list()), ("column", s("tag")), ("text", s("sci"))],
        ),
        (
            "core.table.filter_text",
            vec![
                ("table", list()),
                ("column", s("tag")),
                ("text", s("sci")),
                ("ignore_case", b(true)),
            ],
        ),
        (
            "core.table.filter_text",
            vec![("table", books()), ("column", s("pages"))],
        ),
        (
            "core.table.filter_text",
            vec![("table", books()), ("column", s("title"))],
        ),
        ("core.table.sort", vec![]),
        ("core.table.sort", vec![("table", books())]),
        (
            "core.table.sort",
            vec![("table", books()), ("ascending", b(false))],
        ),
        (
            "core.table.sort",
            vec![("table", books()), ("column", s("pages"))],
        ),
        (
            "core.table.sort",
            vec![
                ("table", books()),
                ("column", s("pages")),
                ("ascending", b(false)),
            ],
        ),
        (
            "core.table.sort",
            vec![("table", books()), ("column", s("rating"))],
        ),
        (
            "core.table.sort",
            vec![("table", books()), ("column", s("read"))],
        ),
        (
            "core.table.sort",
            vec![("table", books()), ("column", s("size"))],
        ),
        (
            "core.table.sort",
            vec![("table", list()), ("column", s("tag"))],
        ),
        (
            "core.table.sort",
            vec![("table", books()), ("column", s("x"))],
        ),
    ];
    assert_golden(&golden_dir(), "table", &report(&none, &cases));
}

fn curve(keys: Vec<CurveKey>) -> Value {
    Value::Curve(Arc::new(Curve::new(keys).unwrap()))
}

#[test]
fn curve_ramp() {
    let none = SystemContext::none();
    let three = || {
        vec![
            CurveKey::new(0.0, 0.0),
            CurveKey::new(0.5, 1.0),
            CurveKey::new(1.0, 0.25),
        ]
    };
    let smooth = curve(
        three()
            .into_iter()
            .map(|k| k.with_interpolation(CurveInterpolation::Bezier))
            .collect(),
    );
    let step = curve(
        three()
            .into_iter()
            .map(|k| k.with_interpolation(CurveInterpolation::Step))
            .collect(),
    );
    let explicit = curve(vec![
        CurveKey::new(0.0, 0.0)
            .with_interpolation(CurveInterpolation::Bezier)
            .with_tangent(CurveTangent::Explicit {
                incoming: 0.0,
                outgoing: 4.0,
            }),
        CurveKey::new(1.0, 1.0),
    ]);
    let spring = curve(vec![
        CurveKey::new(0.0, 0.0).with_interpolation(CurveInterpolation::Spring {
            frequency: 2.0,
            damping: 0.3,
        }),
        CurveKey::new(1.0, 1.0),
    ]);
    let looped = Value::Curve(Arc::new(
        Curve::new(three())
            .unwrap()
            .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::PingPong),
    ));
    let mut cases: Vec<(&str, Vec<(&str, Value)>)> = vec![("core.curve.ramp", vec![])];
    for x in [-0.5, 0.25, 0.5, 1.0, 1.5, f64::NAN, f64::INFINITY] {
        cases.push(("core.curve.ramp", vec![("position", f(x))]));
    }
    for c in [&smooth, &step, &explicit, &spring] {
        for x in [0.1, 0.25, 0.75] {
            cases.push((
                "core.curve.ramp",
                vec![("curve", c.clone()), ("position", f(x))],
            ));
        }
    }
    for x in [-0.25, 1.25, 1.75, 2.25] {
        cases.push((
            "core.curve.ramp",
            vec![("curve", looped.clone()), ("position", f(x))],
        ));
    }
    cases.push((
        "core.curve.ramp",
        vec![
            ("curve", Value::Curve(Arc::new(Curve::constant(3.5)))),
            ("position", f(-100.0)),
        ],
    ));
    assert_golden(&golden_dir(), "curve", &report(&none, &cases));
}

#[test]
fn fs_list_dir() {
    let fs = MemFs::new()
        .with_file("books/Dune.epub", 2_048, 1_700_000_300)
        .unwrap()
        .with_file("books/emma.pdf", 4_096, 1_700_000_100)
        .unwrap()
        .with_file("books/Ubik.pdf", 1_024, 1_700_000_200)
        .unwrap()
        .with_file("books/.hidden.pdf", 1, 1)
        .unwrap()
        .with_file("books/notes.txt", 10, -5)
        .unwrap()
        .with_dir("books/series", 1_600_000_000)
        .unwrap()
        .with_file("books/series/Foundation.pdf", 99, 2)
        .unwrap()
        .with_file("readme.md", 5, 3)
        .unwrap();
    let system = SystemContext::new(&fs);
    let cases: Vec<(&str, Vec<(&str, Value)>)> = vec![
        ("core.fs.list_dir", vec![]),
        ("core.fs.list_dir", vec![("path", s("books"))]),
        ("core.fs.list_dir", vec![("path", s("./books/"))]),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("include_hidden", b(true))],
        ),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("pattern", s("*.pdf"))],
        ),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("pattern", s("*.pdf;*.epub"))],
        ),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("pattern", s("?????.*"))],
        ),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("sort", s("size"))],
        ),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("sort", s("modified"))],
        ),
        ("core.fs.list_dir", vec![("path", s("books/series"))]),
        ("core.fs.list_dir", vec![("path", s("books/series/.."))]),
        (
            "core.fs.list_dir",
            vec![("path", s("books")), ("sort", s("date"))],
        ),
        ("core.fs.list_dir", vec![("path", s("/etc"))]),
        ("core.fs.list_dir", vec![("path", s("../secret"))]),
        ("core.fs.list_dir", vec![("path", s("books/../.."))]),
        ("core.fs.list_dir", vec![("path", s("missing"))]),
        ("core.fs.list_dir", vec![("path", s("readme.md"))]),
    ];
    let mut text = report(&system, &cases);
    text.push_str(&report(
        &SystemContext::none(),
        &[("core.fs.list_dir", vec![("path", s("books"))])],
    ));
    assert_golden(&golden_dir(), "fs", &text);
}
