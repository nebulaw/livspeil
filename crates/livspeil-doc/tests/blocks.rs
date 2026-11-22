// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Table and curve input values (M0.8): canonical text, round trips,
//! strict reading with locations, and a round-trip property.

mod common;

use std::sync::Arc;

use common::{id, registry};
use livspeil_doc::{DocError, DocNode, Document, Location, canonicalize, format, parse};
use livspeil_node_api::{
    Curve, CurveExtrapolation, CurveInterpolation, CurveKey, CurveTangent, PortType, Table,
    TableColumn, Value,
};
use proptest::prelude::*;

const HEADER: &str = "room \"r\" format=1 {\n  network \"/\" {\n";

/// A room with one node of type `ty` named `n` whose children are `body`
/// (the node is on line 3, its children start on line 4).
fn room(ty: &str, body: &str) -> String {
    format!(
        "{HEADER}    node \"n\" type=\"{ty}\" id=\"01920000-0000-7000-8000-000000000001\" {{\n{body}\n    }}\n  }}\n}}\n"
    )
}

fn err(text: &str) -> DocError {
    parse(text).expect_err("expected a parse error")
}

fn loc(line: usize, column: usize) -> Option<Location> {
    Some(Location { line, column })
}

fn doc_with(ty: &str, port: &str, value: Value) -> Document {
    let mut doc = Document::new("blocks");
    let mut node = DocNode::new(id(1), "n", ty);
    node.inputs.insert(port.into(), value);
    doc.network.nodes.push(node);
    doc
}

/// doc -> text -> doc -> graph -> doc is lossless and the text canonical.
fn round_trip(doc: &Document) -> String {
    let text = format(doc).unwrap();
    let back = parse(&text).unwrap();
    assert_eq!(&back, doc, "{text}");
    assert_eq!(format(&back).unwrap(), text);
    assert_eq!(canonicalize(&text).unwrap(), text);
    let graph = back.to_graph(registry()).unwrap();
    let again = Document::from_graph(&graph, doc.name.clone()).unwrap();
    assert_eq!(&again, doc, "{text}");
    text
}

/// The lines of the `input` of `text`, without the shared indentation.
fn input_lines(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.trim_start().starts_with("input "))
        .take_while(|l| l.starts_with("      "))
        .map(|l| l[6..].to_owned())
        .collect()
}

fn s(v: &str) -> Value {
    Value::String(v.to_owned())
}

#[test]
fn tables_round_trip_canonically() {
    let table = Table::from_rows(
        &[
            ("name", PortType::String),
            ("pages", PortType::Int),
            ("rating", PortType::Float),
            ("read", PortType::Bool),
            ("at", PortType::Vec2),
            ("pos", PortType::Vec3),
            ("q", PortType::Vec4),
            ("tint", PortType::Color),
        ],
        [
            vec![
                s("Dune \"1965\"\n✓"),
                Value::Int(-412),
                Value::Float(-0.0),
                Value::Bool(true),
                Value::Vec2([1.0, 2.5]),
                Value::Vec3([0.0, 1e-7, 3.0]),
                Value::Vec4([1.0, 2.0, 3.0, 4.0]),
                Value::Color([0.8, 0.12, 60.0, 1.0]),
            ],
            vec![
                s(""),
                Value::Int(i64::MAX),
                Value::Float(1e300),
                Value::Bool(false),
                Value::Vec2([0.0; 2]),
                Value::Vec3([0.0; 3]),
                Value::Vec4([0.0; 4]),
                Value::Color([0.0; 4]),
            ],
        ],
    )
    .unwrap();
    let text = round_trip(&doc_with("core.table.sort@1", "table", Value::Table(table)));
    assert_eq!(
        input_lines(&text),
        [
            "input \"table\" {",
            "  table {",
            "    column \"name\" type=\"String\"",
            "    column \"pages\" type=\"Int\"",
            "    column \"rating\" type=\"Float\"",
            "    column \"read\" type=\"Bool\"",
            "    column \"at\" type=\"Vec2\"",
            "    column \"pos\" type=\"Vec3\"",
            "    column \"q\" type=\"Vec4\"",
            "    column \"tint\" type=\"Color\"",
            "    row \"Dune \\\"1965\\\"\\n✓\" -412 -0.0 #true (vec2)\"1.0 2.5\" (vec3)\"0.0 1e-7 3.0\" (vec4)\"1.0 2.0 3.0 4.0\" (color)\"oklch(0.8 0.12 60.0 1.0)\"",
            "    row \"\" 9223372036854775807 1e300 #false (vec2)\"0.0 0.0\" (vec3)\"0.0 0.0 0.0\" (vec4)\"0.0 0.0 0.0 0.0\" (color)\"oklch(0.0 0.0 0.0 0.0)\"",
            "  }",
            "}",
        ]
    );
}

#[test]
fn empty_tables_and_lists_round_trip() {
    let text = round_trip(&doc_with(
        "core.table.sort@1",
        "table",
        Value::Table(Table::empty()),
    ));
    assert_eq!(input_lines(&text), ["input \"table\" {", "  table", "}"]);

    let no_rows = Table::new(vec![TableColumn::new("tag", PortType::String, vec![])]).unwrap();
    let text = round_trip(&doc_with(
        "core.table.sort@1",
        "table",
        Value::Table(no_rows),
    ));
    assert_eq!(
        input_lines(&text),
        [
            "input \"table\" {",
            "  table {",
            "    column \"tag\" type=\"String\"",
            "  }",
            "}"
        ]
    );

    let list = Table::list("tag", PortType::String, vec![s("a"), s("b")]).unwrap();
    round_trip(&doc_with("core.table.count@1", "table", Value::Table(list)));
}

#[test]
fn curves_round_trip_canonically() {
    let curve = Curve::new(vec![
        CurveKey::new(0.0, 0.0),
        CurveKey::new(0.25, 1.0).with_interpolation(CurveInterpolation::Step),
        CurveKey::new(0.5, 0.5)
            .with_interpolation(CurveInterpolation::Bezier)
            .with_tangent(CurveTangent::Explicit {
                incoming: 1.0,
                outgoing: -2.0,
            }),
        CurveKey::new(0.75, 0.25)
            .with_interpolation(CurveInterpolation::Spring {
                frequency: 2.0,
                damping: 0.5,
            })
            .with_tangent(CurveTangent::Flat),
        CurveKey::new(1.0, 1.0),
    ])
    .unwrap()
    .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::PingPong);
    let text = round_trip(&doc_with(
        "core.curve.ramp@1",
        "curve",
        Value::Curve(Arc::new(curve)),
    ));
    assert_eq!(
        input_lines(&text),
        [
            "input \"curve\" {",
            "  curve pre=\"loop\" post=\"ping-pong\" {",
            "    key 0.0 0.0",
            "    key 0.25 1.0 interp=\"step\"",
            "    key 0.5 0.5 interp=\"bezier\" tangent=\"explicit\" in=1.0 out=-2.0",
            "    key 0.75 0.25 interp=\"spring\" frequency=2.0 damping=0.5 tangent=\"flat\"",
            "    key 1.0 1.0",
            "  }",
            "}",
        ]
    );
    let text = round_trip(&doc_with(
        "core.curve.ramp@1",
        "curve",
        Value::Curve(Arc::new(Curve::identity())),
    ));
    assert_eq!(
        input_lines(&text),
        [
            "input \"curve\" {",
            "  curve {",
            "    key 0.0 0.0",
            "    key 1.0 1.0",
            "  }",
            "}",
        ]
    );
}

#[test]
fn reader_is_lenient_where_unambiguous() {
    // Ints in Float columns and as curve numbers; any child order.
    let doc = parse(&room(
        "core.table.sort@1",
        "      input \"table\" {\n        table {\n          column \"x\" type=\"Float\"\n          row 1\n        }\n      }",
    ))
    .unwrap();
    assert_eq!(
        doc.network.nodes[0].inputs["table"],
        Value::Table(Table::list("x", PortType::Float, vec![Value::Float(1.0)]).unwrap())
    );
    let doc = parse(&room(
        "core.curve.ramp@1",
        "      input \"curve\" {\n        curve {\n          key 0 1\n          key 2 3 tangent=\"explicit\" in=1 out=0\n        }\n      }",
    ))
    .unwrap();
    let Value::Curve(c) = &doc.network.nodes[0].inputs["curve"] else {
        panic!()
    };
    assert_eq!(c.keys()[1].position, 2.0);
    assert_eq!(
        c.keys()[1].tangent,
        CurveTangent::Explicit {
            incoming: 1.0,
            outgoing: 0.0
        }
    );
}

#[test]
fn table_errors_carry_locations() {
    let table = |body: &str| {
        room(
            "core.table.sort@1",
            &format!("      input \"table\" {{\n        table {{\n{body}\n        }}\n      }}"),
        )
    };
    let col = "          column \"x\" type=\"Int\"";
    for (body, at, message) in [
        (
            "          row 1".to_owned(),
            loc(6, 11),
            "input 'table' table: declare columns before rows",
        ),
        (
            format!("{col}\n          row 1\n          column \"y\" type=\"Int\""),
            loc(8, 11),
            "input 'table' table: declare every column before the rows",
        ),
        (
            format!("{col}\n          row 1.5"),
            loc(7, 15),
            "input 'table' table row 0 column 'x': expected Int, found Float",
        ),
        (
            format!("{col}\n          row 1 2"),
            loc(7, 11),
            "input 'table' table row 0: expected 1 values (one per column), found 2",
        ),
        (
            format!("{col}\n          row (vec2)\"1\""),
            loc(7, 15),
            "input 'table' table row 0 column 'x': a (vec2) value needs 2 numbers, found 1; for example (vec2)\"1.0 2.0\"",
        ),
        (
            "          column \"x\" type=\"Table\"".to_owned(),
            loc(6, 22),
            "unknown column type \"Table\"; expected one of \"Float\", \"Int\", \"Bool\", \"Vec2\", \"Vec3\", \"Vec4\", \"Color\", \"String\"",
        ),
        (
            "          column \"x\"".to_owned(),
            loc(6, 11),
            "column 'x' needs a type, e.g. type=\"String\"",
        ),
        (
            format!("{col}\n{col}"),
            loc(7, 11),
            "input 'table' table: duplicate column 'x'",
        ),
        (
            "          column \"\" type=\"Int\"".to_owned(),
            loc(6, 11),
            "input 'table' table: column names must not be empty",
        ),
        (
            format!("{col}\n          row 1 x=1"),
            loc(7, 17),
            "unknown property `x`; this node takes no properties",
        ),
        (
            format!("{col}\n          cell 1"),
            loc(7, 11),
            "unknown node `cell` in table; expected `column \"name\" type=\"Type\"` or `row <values>`",
        ),
        (
            format!("{col}\n          row 1 {{ }}"),
            loc(7, 11),
            "`row` takes no children block",
        ),
    ] {
        let e = err(&table(&body));
        assert_eq!(e.location(), at, "{body}: {e}");
        assert_eq!(
            e.to_string(),
            format!(
                "{}: {message}",
                at.map(|l| l.to_string()).unwrap_or_default()
            ),
            "{body}"
        );
    }
    let e = err(&room(
        "core.table.sort@1",
        "      input \"table\" {\n        table 1\n      }",
    ));
    assert_eq!(e.location(), loc(5, 15));
}

#[test]
fn curve_errors_carry_locations() {
    let curve = |head: &str, body: &str| {
        room(
            "core.curve.ramp@1",
            &format!(
                "      input \"curve\" {{\n        curve{head} {{\n{body}\n        }}\n      }}"
            ),
        )
    };
    for (head, body, at, message) in [
        (
            "",
            "          key 0.0 (vec2)\"1 2\"",
            loc(6, 11),
            "input 'curve' curve: key values must be numbers, found Vec2",
        ),
        (
            "",
            "          key 1.0 0.0\n          key 1.0 1.0",
            loc(7, 11),
            "input 'curve' curve: key 1: key positions must be strictly increasing",
        ),
        (
            "",
            "",
            loc(5, 9),
            "input 'curve' curve: a curve needs at least one key",
        ),
        (
            " offset=1.0",
            "          key 0.0 0.0",
            loc(5, 15),
            "unknown property `offset`; expected one of: pre, post",
        ),
        (
            " pre=\"bounce\"",
            "          key 0.0 0.0",
            loc(5, 15),
            "unknown pre extrapolation \"bounce\"; expected one of \"hold\", \"loop\", \"ping-pong\"",
        ),
        (
            "",
            "          key 0.0 0.0 interp=\"spring\" frequency=0.0 damping=1.0",
            loc(6, 11),
            "input 'curve' curve: key 0: spring frequency must be positive and damping not negative",
        ),
        (
            "",
            "          key 0.0 0.0 tangent=\"explicit\" in=(vec2)\"1 2\" out=0.0",
            loc(6, 11),
            "input 'curve' curve: key 0: expected Float, found Vec2",
        ),
        (
            "",
            "          point 0.0 0.0",
            loc(6, 11),
            "unknown node `point` in curve; expected `key <position> <value>`",
        ),
    ] {
        let e = err(&curve(head, body));
        assert_eq!(e.location(), at, "{head} {body}: {e}");
        assert_eq!(
            e.to_string(),
            format!(
                "{}: {message}",
                at.map(|l| l.to_string()).unwrap_or_default()
            ),
            "{head} {body}"
        );
    }
}

#[test]
fn inputs_hold_at_most_one_static_value() {
    let e = err(&room(
        "core.table.sort@1",
        "      input \"table\" {\n        table\n        table\n      }",
    ));
    assert_eq!(e.location(), loc(6, 9));
    assert!(
        e.to_string()
            .ends_with("input 'table' has more than one static value")
    );
    let e = err(&room(
        "core.curve.ramp@1",
        "      input \"position\" 1.0 {\n        curve {\n          key 0.0 0.0\n        }\n      }",
    ));
    assert_eq!(e.location(), loc(5, 9));
    let e = err(&room(
        "core.curve.ramp@1",
        "      input \"position\" {\n        list\n      }",
    ));
    assert!(
        e.to_string().ends_with(
            "unknown node `list` in input 'position'; expected `table`, `curve` or `anim`"
        ),
        "{e}"
    );
}

#[test]
fn graph_checks_block_value_types() {
    // A table on a Float input is a type error at the node.
    let doc = parse(&room(
        "core.curve.ramp@1",
        "      input \"position\" {\n        table\n      }",
    ))
    .unwrap();
    let e = doc.to_graph(registry()).unwrap_err();
    assert!(matches!(e, DocError::Graph { .. }), "{e}");
    assert_eq!(e.location(), loc(3, 5));
}

#[test]
fn non_finite_table_cells_are_rejected_when_writing() {
    let table = Table::list("x", PortType::Float, vec![Value::Float(f64::NAN)]).unwrap();
    let doc = doc_with("core.table.sort@1", "table", Value::Table(table));
    assert!(matches!(format(&doc), Err(DocError::NonFinite { .. })));
}

fn finite() -> impl Strategy<Value = f64> {
    prop_oneof![
        any::<f64>().prop_filter("finite", |v| v.is_finite()),
        -1e3..1e3f64,
        Just(0.0),
        Just(-0.0),
    ]
}

fn cell(ty: PortType) -> BoxedStrategy<Value> {
    match ty {
        PortType::Float => finite().prop_map(Value::Float).boxed(),
        PortType::Int => any::<i64>().prop_map(Value::Int).boxed(),
        PortType::Bool => any::<bool>().prop_map(Value::Bool).boxed(),
        PortType::Vec2 => [finite(), finite()].prop_map(Value::Vec2).boxed(),
        PortType::Vec3 => [finite(), finite(), finite()].prop_map(Value::Vec3).boxed(),
        PortType::Vec4 => [finite(), finite(), finite(), finite()]
            .prop_map(Value::Vec4)
            .boxed(),
        PortType::Color => [finite(), finite(), finite(), finite()]
            .prop_map(Value::Color)
            .boxed(),
        _ => any::<String>().prop_map(Value::String).boxed(),
    }
}

fn table() -> impl Strategy<Value = Table> {
    let ty = prop::sample::select(vec![
        PortType::Float,
        PortType::Int,
        PortType::Bool,
        PortType::Vec2,
        PortType::Vec3,
        PortType::Vec4,
        PortType::Color,
        PortType::String,
    ]);
    (prop::collection::vec(ty, 0..4), 0..5usize).prop_flat_map(|(types, rows)| {
        let names = prop::collection::btree_set(any::<String>(), types.len())
            .prop_map(|set| set.into_iter().collect::<Vec<_>>());
        let columns: Vec<_> = types
            .iter()
            .map(|ty| prop::collection::vec(cell(*ty), rows))
            .collect();
        (Just(types), names, columns).prop_map(|(types, names, columns)| {
            Table::new(
                types
                    .into_iter()
                    .zip(names)
                    .zip(columns)
                    .map(|((ty, name), values)| {
                        // Empty names are invalid; make them non-empty.
                        TableColumn::new(format!("c{name}"), ty, values)
                    })
                    .collect(),
            )
            .unwrap()
        })
    })
}

fn curve_extrapolation() -> impl Strategy<Value = CurveExtrapolation> {
    prop_oneof![
        Just(CurveExtrapolation::Hold),
        Just(CurveExtrapolation::Loop),
        Just(CurveExtrapolation::PingPong),
    ]
}

fn curve() -> impl Strategy<Value = Curve> {
    let interp = prop_oneof![
        Just(CurveInterpolation::Step),
        Just(CurveInterpolation::Linear),
        Just(CurveInterpolation::Bezier),
        (1e-3..100.0f64, 0.0..3.0f64)
            .prop_map(|(frequency, damping)| CurveInterpolation::Spring { frequency, damping }),
    ];
    let tangent = prop_oneof![
        Just(CurveTangent::Auto),
        Just(CurveTangent::Flat),
        (finite(), finite())
            .prop_map(|(incoming, outgoing)| CurveTangent::Explicit { incoming, outgoing }),
    ];
    (
        prop::collection::vec((0.001..10.0f64, finite(), interp, tangent), 1..6),
        -1e3..1e3f64,
        curve_extrapolation(),
        curve_extrapolation(),
    )
        .prop_map(|(keys, start, pre, post)| {
            let mut position = start;
            let keys = keys
                .into_iter()
                .map(|(step, value, interpolation, tangent)| {
                    position += step;
                    CurveKey {
                        position,
                        value,
                        interpolation,
                        tangent,
                    }
                })
                .collect();
            Curve::new(keys)
                .map(|c| c.with_extrapolation(pre, post))
                .unwrap_or_else(|_| Curve::identity())
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn table_and_curve_inputs_round_trip(table in table(), curve in curve(), x in finite()) {
        let mut doc = Document::new("blocks");
        let mut sort = DocNode::new(id(1), "sort", "core.table.sort@1");
        sort.inputs.insert("table".into(), Value::Table(table));
        sort.inputs.insert("column".into(), Value::String("c".into()));
        let mut ramp = DocNode::new(id(2), "ramp", "core.curve.ramp@1");
        ramp.inputs.insert("curve".into(), Value::Curve(Arc::new(curve)));
        ramp.inputs.insert("position".into(), Value::Float(x));
        doc.network.nodes.push(sort);
        doc.network.nodes.push(ramp);

        let text = format(&doc).unwrap();
        let parsed = parse(&text).unwrap();
        prop_assert_eq!(&parsed, &doc);
        prop_assert_eq!(format(&parsed).unwrap(), text.clone());
        prop_assert_eq!(canonicalize(&text).unwrap(), text);
        let graph = parsed.to_graph(registry()).unwrap();
        let again = Document::from_graph(&graph, "blocks").unwrap();
        prop_assert_eq!(&again, &doc);
    }
}
