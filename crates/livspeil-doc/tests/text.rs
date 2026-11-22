// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Value round trips, canonical form and error reporting.

mod common;

use common::{id, registry};
use livspeil_doc::{
    DocError, DocLink, DocNode, Document, Location, Tier, canonicalize, format, parse,
};
use livspeil_graph::GraphError;
use livspeil_node_api::{PortType, Value};

fn doc_with(value: Value) -> Document {
    let mut doc = Document::new("values");
    let mut node = DocNode::new(id(1), "n", "test.all@1");
    node.inputs.insert("x".into(), value);
    doc.network.nodes.push(node);
    doc
}

fn round_trip(value: Value) -> String {
    let doc = doc_with(value.clone());
    let text = format(&doc).unwrap();
    let back = parse(&text).unwrap();
    assert_eq!(back, doc, "{text}");
    let got = &back.network.nodes[0].inputs["x"];
    assert_eq!(got.port_type(), value.port_type());
    // Bit-exact, including the sign of zero.
    assert_eq!(format!("{got:?}"), format!("{value:?}"));
    assert_eq!(format(&back).unwrap(), text);
    text.lines()
        .find(|l| l.trim_start().starts_with("input"))
        .unwrap()
        .trim()
        .to_owned()
}

#[test]
fn float_and_int_stay_distinct() {
    assert_eq!(round_trip(Value::Float(1.0)), "input \"x\" 1.0");
    assert_eq!(round_trip(Value::Int(1)), "input \"x\" 1");
    assert_eq!(
        round_trip(Value::Int(i64::MIN)),
        "input \"x\" -9223372036854775808"
    );
    assert_eq!(
        round_trip(Value::Int(i64::MAX)),
        "input \"x\" 9223372036854775807"
    );
}

#[test]
fn floats_round_trip_exactly() {
    for v in [
        0.0,
        -0.0,
        0.1,
        -2.5,
        1e-7,
        1e16,
        1e300,
        f64::MAX,
        f64::MIN_POSITIVE,
        5e-324,
        std::f64::consts::PI,
    ] {
        round_trip(Value::Float(v));
    }
    assert_eq!(round_trip(Value::Float(1e-7)), "input \"x\" 1e-7");
}

#[test]
fn bools_vectors_and_colors_round_trip() {
    assert_eq!(round_trip(Value::Bool(true)), "input \"x\" #true");
    assert_eq!(round_trip(Value::Bool(false)), "input \"x\" #false");
    assert_eq!(
        round_trip(Value::Vec2([1.0, -0.5])),
        "input \"x\" (vec2)\"1.0 -0.5\""
    );
    assert_eq!(
        round_trip(Value::Vec3([0.1, 0.2, 0.30000000000000004])),
        "input \"x\" (vec3)\"0.1 0.2 0.30000000000000004\""
    );
    assert_eq!(
        round_trip(Value::Vec4([1.0, 2.0, 3.0, 1e-9])),
        "input \"x\" (vec4)\"1.0 2.0 3.0 1e-9\""
    );
    assert_eq!(
        round_trip(Value::Color([0.8, 0.12, 60.0, 1.0])),
        "input \"x\" (color)\"oklch(0.8 0.12 60.0 1.0)\""
    );
}

#[test]
fn strings_round_trip_with_escapes() {
    assert_eq!(round_trip(Value::String(String::new())), "input \"x\" \"\"");
    assert_eq!(
        round_trip(Value::String("say \"hi\"\\\n\ttab".into())),
        r#"input "x" "say \"hi\"\\\n\ttab""#
    );
    for s in [
        "héllo wörld ✓ 日本",
        "#true",
        "1.0",
        "null",
        "\u{0}\u{7f}\u{85}\u{2028}\u{feff}\u{202e}",
        "/* not a comment */ // nor this",
        "\r\n\u{8}\u{c}",
        "emoji 🌅",
    ] {
        round_trip(Value::String(s.into()));
    }
}

#[test]
fn reader_accepts_lenient_spellings() {
    let text = r##"
        room "r" format=1 {
            network "/" {
                node "n" type="test.all@1" id="01920000-0000-7000-8000-000000000001" {
                    input "a" 0x10
                    input "b" bare
                    input "c" (color)"oklch(0.5 0.1 30)"
                    input "d" (vec2)"  1   2 "
                    input "e" #"raw "string""#
                    input "f" 1_000.5
                }
            }
        }
    "##;
    let doc = parse(text).unwrap();
    let inputs = &doc.network.nodes[0].inputs;
    assert_eq!(inputs["a"], Value::Int(16));
    assert_eq!(inputs["b"], Value::String("bare".into()));
    assert_eq!(inputs["c"], Value::Color([0.5, 0.1, 30.0, 1.0]));
    assert_eq!(inputs["d"], Value::Vec2([1.0, 2.0]));
    assert_eq!(inputs["e"], Value::String("raw \"string\"".into()));
    assert_eq!(inputs["f"], Value::Float(1000.5));
    assert_eq!(doc.min_tier, Tier(0));
    let canonical = format(&doc).unwrap();
    assert_eq!(format(&parse(&canonical).unwrap()).unwrap(), canonical);
}

const HEADER: &str = "room \"r\" format=1 min-tier=\"T1\" {\n  network \"/\" {\n";

fn err(text: &str) -> DocError {
    parse(text).expect_err("expected a parse error")
}

fn node_with_input(input: &str) -> String {
    format!(
        "{HEADER}    node \"n\" type=\"test.all@1\" id=\"01920000-0000-7000-8000-000000000001\" {{\n      {input}\n    }}\n  }}\n}}\n"
    )
}

#[test]
fn syntax_errors_carry_line_and_column() {
    let e = err("room \"r\" format=1 {\n  network \"/\" {\n    node 1.\n  }\n}\n");
    assert!(matches!(e, DocError::Syntax { .. }), "{e}");
    assert_eq!(e.location().map(|l| l.line), Some(3), "{e}");

    let e = err("room \"r\" format=1 {\n  network \"/\" {\n");
    assert!(matches!(e, DocError::Syntax { .. }), "{e}");
}

#[test]
fn locations_count_characters_after_non_ascii() {
    // "é" is two bytes; the column must count characters.
    let text = node_with_input("input \"é\" (rgb)\"x\"");
    let e = err(&text);
    assert_eq!(
        e.location(),
        Some(Location {
            line: 4,
            column: 17
        }),
        "{e}"
    );
    assert!(e.to_string().starts_with("4:17: "), "{e}");
}

#[test]
fn non_finite_numbers_are_rejected() {
    for input in [
        "input \"x\" #nan",
        "input \"x\" #inf",
        "input \"x\" #-inf",
        "input \"x\" (vec2)\"1.0 inf\"",
        "input \"x\" (vec3)\"NaN 1 2\"",
        "input \"x\" 1e999",
    ] {
        let e = err(&node_with_input(input));
        assert!(e.to_string().contains("NaN"), "{input}: {e}");
        assert_eq!(e.location().map(|l| l.line), Some(4), "{input}");
    }
    let doc = doc_with(Value::Float(f64::NAN));
    assert!(matches!(format(&doc), Err(DocError::NonFinite { .. })));
}

#[test]
fn bad_values_are_explained() {
    for (input, needle) in [
        ("input \"x\" #null", "#null"),
        ("input \"x\" 99999999999999999999", "64 bits"),
        ("input \"x\" (vec3)\"1 2\"", "needs 3 numbers"),
        ("input \"x\" (vec3)1.0", "must be a string"),
        ("input \"x\" (color)\"rgb(1 2 3)\"", "oklch"),
        ("input \"x\" (i32)1", "unknown type annotation"),
        ("input \"x\" (vec2)\"1 two\"", "not a number"),
        ("input \"x\"", "input 'x' sets nothing"),
        ("input \"x\" 1 2", "expected `input \"port\" <value>`"),
        ("input \"x\" 1 unit=\"m\"", "unknown property `unit`"),
    ] {
        let e = err(&node_with_input(input));
        assert!(e.to_string().contains(needle), "{input}: {e}");
    }
}

#[test]
fn structural_errors_are_explained() {
    let id = "id=\"01920000-0000-7000-8000-000000000001\"";
    let cases = [
        ("", "empty document"),
        ("network \"/\"", "expected the `room` header"),
        (
            "room \"r\" format=1 { network \"/\" }\nroom \"s\" format=1",
            "only one top-level",
        ),
        ("room \"r\" { network \"/\" }", "needs a format"),
        (
            "room \"r\" format=\"1\" { network \"/\" }",
            "format must be an integer",
        ),
        (
            "room \"r\" format=2 { network \"/\" }",
            "format 2 is not supported",
        ),
        (
            "room \"r\" format=0 { network \"/\" }",
            "format 0 is not supported",
        ),
        ("room \"r\" format=1", "no network"),
        (
            "room \"r\" format=1 min-tier=\"high\" { network \"/\" }",
            "not a tier",
        ),
        (
            "room \"r\" format=1 colour=1 { network \"/\" }",
            "unknown property `colour`",
        ),
        ("room \"r\" format=1 { network \"/sub\" }", "subnets"),
        (
            "room \"r\" format=1 { network \"/\"; network \"/\" }",
            "only one network",
        ),
        (
            "room \"r\" format=1 { network \"/\"; widgets }",
            "unknown node `widgets`",
        ),
        (
            &format!("room \"r\" format=1 {{ network \"/\" {{ node \"n\" {id} }} }}"),
            "needs a type",
        ),
        (
            "room \"r\" format=1 { network \"/\" { node \"n\" type=\"t@1\" } }",
            "needs an id",
        ),
        (
            "room \"r\" format=1 { network \"/\" { node \"n\" type=\"t@1\" id=\"nope\" } }",
            "invalid node id",
        ),
        (
            &format!("room \"r\" format=1 {{ network \"/\" {{ node \"\" type=\"t@1\" {id} }} }}"),
            "must not be empty",
        ),
        (
            &format!(
                "room \"r\" format=1 {{ network \"/\" {{ node \"n\" type=\"t@1\" {id}; node \"n\" type=\"t@1\" id=\"01920000-0000-7000-8000-000000000002\" }} }}"
            ),
            "duplicate node name",
        ),
        (
            &format!(
                "room \"r\" format=1 {{ network \"/\" {{ node \"a\" type=\"t@1\" {id}; node \"b\" type=\"t@1\" {id} }} }}"
            ),
            "duplicate node id",
        ),
        (
            &format!(
                "room \"r\" format=1 {{ network \"/\" {{ node \"n\" type=\"t@1\" {id} type=\"u@1\" }} }}"
            ),
            "more than once",
        ),
        (
            &format!(
                "room \"r\" format=1 {{ network \"/\" {{ node \"n\" type=\"t@1\" {id} {{ input \"x\" 1; input \"x\" 2 }} }} }}"
            ),
            "set more than once",
        ),
        (
            "room \"r\" format=1 { network \"/\" { link from=\"a\" to=\"b.x\" } }",
            "must be \"node.port\"",
        ),
        (
            "room \"r\" format=1 { network \"/\" { link to=\"b.x\" } }",
            "link needs `from=",
        ),
        (
            "room \"r\" format=1 { network \"/\"; meta { node \"ghost\" x=1 y=2 } }",
            "unknown node 'ghost'",
        ),
        (
            "room \"r\" format=1 { network \"/\"; meta { node \"g\" x=1 } }",
            "needs `y=`",
        ),
        (
            "(t)room \"r\" format=1 { network \"/\" }",
            "type annotation",
        ),
    ];
    for (text, needle) in cases {
        let e = err(text);
        assert!(e.to_string().contains(needle), "{text:?}: {e}");
    }
}

#[test]
fn graph_errors_name_the_culprit_and_location() {
    let text = "room \"r\" format=1 {\n  network \"/\" {\n    node \"sun\" type=\"core.time.timeofday@1\" id=\"01920000-0000-7000-8000-000000000001\"\n    node \"lamp\" type=\"core.color.mixx@1\" id=\"01920000-0000-7000-8000-000000000002\"\n  }\n}\n";
    let e = parse(text).unwrap().to_graph(registry()).unwrap_err();
    assert!(
        matches!(&e, DocError::Graph { source: GraphError::UnknownNodeType(t), .. } if t == "core.color.mixx@1"),
        "{e}"
    );
    assert_eq!(
        e.to_string(),
        "4:5: node 'lamp': unknown node type 'core.color.mixx@1'"
    );

    let base = "room \"r\" format=1 {\n  network \"/\" {\n    node \"sun\" type=\"core.time.timeofday@1\" id=\"01920000-0000-7000-8000-000000000001\"\n    node \"lamp\" type=\"core.color.mix@1\" id=\"01920000-0000-7000-8000-000000000002\" {\n      input \"t\" ";
    let load = |rest: &str| {
        parse(&format!("{base}{rest}\n  }}\n}}\n"))
            .unwrap()
            .to_graph(registry())
            .unwrap_err()
    };

    let e = load("\"half\"\n    }");
    assert!(
        matches!(
            &e,
            DocError::Graph {
                source: GraphError::TypeMismatch {
                    expected: PortType::Float,
                    found: PortType::String
                },
                ..
            }
        ),
        "{e}"
    );
    assert!(e.to_string().contains("node 'lamp' input 't'"), "{e}");

    let e = load("0.5\n    }\n    link from=\"sun.phase\" to=\"lamp.a\"");
    assert!(
        e.to_string()
            .starts_with("7:5: link sun.phase -> lamp.a: type mismatch"),
        "{e}"
    );

    let e = load("0.5\n    }\n    link from=\"sun.nope\" to=\"lamp.t\"");
    assert!(e.to_string().contains("no output port 'nope'"), "{e}");

    let e = load("0.5\n    }\n    link from=\"moon.phase\" to=\"lamp.t\"");
    assert!(e.to_string().contains("no node named 'moon'"), "{e}");

    let e = load(
        "0.5\n    }\n    link from=\"sun.phase\" to=\"lamp.t\"\n    link from=\"sun.hour\" to=\"lamp.t\"",
    );
    assert!(e.to_string().contains("already linked"), "{e}");

    let e = load(
        "0.5 }\n    node \"x\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000003\" { input \"nope\" 1.0 }",
    );
    assert!(e.to_string().contains("no input port 'nope'"), "{e}");
}

#[test]
fn ints_coerce_to_float_inputs_when_loading() {
    let mut doc = Document::new("r");
    let mut node = DocNode::new(id(1), "sun", "core.time.timeofday@1");
    node.inputs.insert("offset".into(), Value::Int(2));
    doc.network.nodes.push(node);
    let graph = doc.to_graph(registry()).unwrap();
    let back = Document::from_graph(&graph, "r").unwrap();
    assert_eq!(back.network.nodes[0].inputs["offset"], Value::Float(2.0));
}

#[test]
fn canonical_form_sorts_everything() {
    let mut doc = Document::new("sorted");
    doc.min_tier = Tier(2);
    for (n, name) in [(3, "c"), (1, "b"), (2, "a")] {
        let mut node = DocNode::new(id(n), name, "core.math.add@1");
        node.inputs.insert("b".into(), Value::Float(2.0));
        node.inputs.insert("a".into(), Value::Float(1.0));
        doc.network.nodes.push(node);
    }
    doc.network.links.push(DocLink::new("b", "out", "c", "b"));
    doc.network.links.push(DocLink::new("a", "out", "c", "a"));
    doc.meta.positions.insert("c".into(), [3.0, 0.0]);
    doc.meta.positions.insert("a".into(), [1.0, -1.5]);
    let text = format(&doc).unwrap();
    let order: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("node") || l.starts_with("link") || l.starts_with("input"))
        .map(|l| l.split_whitespace().nth(1).unwrap_or(""))
        .collect();
    assert_eq!(
        order,
        [
            "\"b\"",
            "\"a\"",
            "\"b\"", // node b (id 1) inputs a, b
            "\"a\"",
            "\"a\"",
            "\"b\"", // node a (id 2)
            "\"c\"",
            "\"a\"",
            "\"b\"", // node c (id 3)
            "from=\"a.out\"",
            "from=\"b.out\"",
            "\"a\"",
            "\"c\"", // meta by name
        ]
    );
    let back = parse(&text).unwrap();
    assert_eq!(format(&back).unwrap(), text);
    let mut sorted = doc.clone();
    sorted.network.sort();
    assert_eq!(back, sorted);
    assert_eq!(canonicalize(&text).unwrap(), text);
}

#[test]
fn node_names_with_dots_and_quotes_link_correctly() {
    let mut doc = Document::new("dots");
    doc.network
        .nodes
        .push(DocNode::new(id(1), "a.b \"c\"", "core.time.timeofday@1"));
    doc.network
        .nodes
        .push(DocNode::new(id(2), "x.y", "core.color.mix@1"));
    doc.network
        .links
        .push(DocLink::new("a.b \"c\"", "phase", "x.y", "t"));
    let text = format(&doc).unwrap();
    let back = parse(&text).unwrap();
    assert_eq!(back, doc);
    let graph = back.to_graph(registry()).unwrap();
    assert_eq!(graph.link_count(), 1);
}
