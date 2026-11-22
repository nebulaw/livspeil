// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Expressions and animation channels on inputs: round trips, canonical
//! text and errors with locations.

mod common;

use common::{id, registry};
use livspeil_doc::{DocAnim, DocError, DocExpr, DocNode, Document, Location, format, parse};
use livspeil_graph::{Graph, GraphError};
use livspeil_node_api::Value;
use livspeil_params::expr::{ErrorKind, quote as expr_quote};
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe, Tangent};

const HEADER: &str = "room \"r\" format=1 {\n  network \"/\" {\n";

/// `text` inside a room with one `test.all@1` node `n` (the node is on
/// line 3, its children start on line 4).
fn all_node(body: &str) -> String {
    format!(
        "{HEADER}    node \"n\" type=\"test.all@1\" id=\"01920000-0000-7000-8000-000000000001\" {{\n{body}\n    }}\n  }}\n}}\n"
    )
}

fn err(text: &str) -> DocError {
    parse(text).expect_err("expected a parse error")
}

fn loc(line: usize, column: usize) -> Option<Location> {
    Some(Location { line, column })
}

/// doc -> text -> doc is lossless and the text is canonical; the document
/// also survives doc -> graph -> doc.
fn round_trip(doc: &Document) -> String {
    let text = format(doc).unwrap();
    let back = parse(&text).unwrap();
    assert_eq!(&back, doc, "{text}");
    assert_eq!(format(&back).unwrap(), text);
    let graph = back.to_graph(registry()).unwrap();
    let again = Document::from_graph(&graph, doc.name.clone()).unwrap();
    assert_eq!(&again, doc, "{text}");
    assert_eq!(
        again.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
    text
}

fn channel(keys: Vec<Keyframe>) -> AnimChannel {
    AnimChannel::new(keys).unwrap()
}

fn doc_with_anim(port: &str, channel: AnimChannel) -> Document {
    let mut doc = Document::new("anim");
    let mut node = DocNode::new(id(1), "n", "test.all@1");
    node.animations.insert(port.into(), DocAnim::new(channel));
    doc.network.nodes.push(node);
    doc
}

/// The lines of the `anim` block, trimmed.
fn anim_lines(text: &str) -> Vec<String> {
    text.lines()
        .skip_while(|l| !l.trim_start().starts_with("anim"))
        .take_while(|l| l.trim() != "}")
        .map(|l| l.trim().to_owned())
        .collect()
}

#[test]
fn every_interpolation_round_trips() {
    let ch = channel(vec![
        Keyframe::float(0.0, 0.0).with_interpolation(Interpolation::Step),
        Keyframe::float(1.0, 1.0).with_interpolation(Interpolation::Linear),
        Keyframe::float(2.0, 0.5).with_interpolation(Interpolation::Bezier),
        Keyframe::float(3.0, 2.0).with_interpolation(Interpolation::Spring {
            frequency: 2.0,
            damping: 0.25,
        }),
        Keyframe::float(4.0, -1.0),
    ]);
    let text = round_trip(&doc_with_anim("f", ch));
    assert_eq!(
        anim_lines(&text),
        [
            "anim {",
            "key 0.0 0.0 interp=\"step\"",
            "key 1.0 1.0",
            "key 2.0 0.5 interp=\"bezier\"",
            "key 3.0 2.0 interp=\"spring\" frequency=2.0 damping=0.25",
            "key 4.0 -1.0",
        ]
    );
}

#[test]
fn every_tangent_round_trips() {
    let ch = channel(vec![
        Keyframe::new(0.0, Value::Vec3([0.0, 1.0, 2.0]))
            .with_interpolation(Interpolation::Bezier)
            .with_tangent(Tangent::Auto),
        Keyframe::new(1.0, Value::Vec3([1.0, 1.0, 1.0]))
            .with_interpolation(Interpolation::Bezier)
            .with_tangent(Tangent::Flat),
        Keyframe::new(2.0, Value::Vec3([2.0, 0.0, -1.0])).with_tangent(Tangent::Explicit {
            incoming: Value::Vec3([0.5, -0.0, 1e-7]),
            outgoing: Value::Vec3([1.0, 2.0, 3.0]),
        }),
    ]);
    let text = round_trip(&doc_with_anim("v3", ch));
    assert_eq!(
        anim_lines(&text),
        [
            "anim {",
            "key 0.0 (vec3)\"0.0 1.0 2.0\" interp=\"bezier\"",
            "key 1.0 (vec3)\"1.0 1.0 1.0\" interp=\"bezier\" tangent=\"flat\"",
            "key 2.0 (vec3)\"2.0 0.0 -1.0\" tangent=\"explicit\" in=(vec3)\"0.5 -0.0 1e-7\" out=(vec3)\"1.0 2.0 3.0\"",
        ]
    );
}

#[test]
fn every_extrapolation_and_offset_round_trips() {
    let all = [
        Extrapolation::Hold,
        Extrapolation::Loop,
        Extrapolation::PingPong,
    ];
    for pre in all {
        for post in all {
            for offset in [0.0, -0.0, 1.5, -2.0, 1e-9] {
                let ch = channel(vec![Keyframe::float(0.0, 1.0), Keyframe::float(1.0, 2.0)])
                    .with_extrapolation(pre, post)
                    .with_offset(offset)
                    .unwrap();
                let text = round_trip(&doc_with_anim("f", ch.clone()));
                let parsed = parse(&text).unwrap();
                let got = &parsed.network.nodes[0].animations["f"].channel;
                assert_eq!(got.offset().to_bits(), offset.to_bits(), "{text}");
                assert_eq!((got.pre(), got.post()), (pre, post));
            }
        }
    }
    let ch = channel(vec![Keyframe::float(0.0, 1.0)])
        .with_extrapolation(Extrapolation::PingPong, Extrapolation::Loop)
        .with_offset(1.5)
        .unwrap();
    let text = round_trip(&doc_with_anim("f", ch));
    assert_eq!(
        anim_lines(&text)[0],
        "anim pre=\"ping-pong\" post=\"loop\" offset=1.5 {"
    );
}

#[test]
fn every_animatable_type_round_trips() {
    let mut doc = Document::new("types");
    let mut node = DocNode::new(id(1), "n", "test.all@1");
    for (port, a, b) in [
        ("f", Value::Float(0.0), Value::Float(1.0)),
        ("v2", Value::Vec2([0.0, 1.0]), Value::Vec2([2.0, 3.0])),
        (
            "v3",
            Value::Vec3([0.0, 1.0, 2.0]),
            Value::Vec3([3.0, 4.0, 5.0]),
        ),
        (
            "v4",
            Value::Vec4([0.0, 1.0, 2.0, 3.0]),
            Value::Vec4([4.0, 5.0, 6.0, 7.0]),
        ),
        (
            "c",
            Value::Color([0.5, 0.1, 30.0, 1.0]),
            Value::Color([0.8, 0.2, 400.0, 0.5]),
        ),
    ] {
        let ch = channel(vec![Keyframe::new(0.0, a), Keyframe::new(1.0, b)]);
        node.animations.insert(port.into(), DocAnim::new(ch));
    }
    doc.network.nodes.push(node);
    let text = round_trip(&doc);
    assert!(
        text.contains("key 1.0 (color)\"oklch(0.8 0.2 400.0 0.5)\""),
        "{text}"
    );
}

#[test]
fn reader_accepts_integers_and_defaults_in_anim() {
    let text = all_node(
        "      input \"f\" {\n        anim pre=\"hold\" offset=2 {\n          key 0 1 interp=\"linear\" tangent=\"auto\"\n          key 1 (vec2)\"1 2\"\n        }\n      }",
    );
    // Mixed types are caught (Int reads as Float, then Vec2 differs).
    let e = err(&text);
    assert_eq!(e.location(), loc(7, 11), "{e}");
    assert!(e.to_string().contains("key 1: expected Float"), "{e}");

    let text = all_node(
        "      input \"f\" {\n        anim pre=\"hold\" offset=2 {\n          key 0 1 interp=\"linear\" tangent=\"auto\"\n          key 1 2 tangent=\"explicit\" in=0 out=-1\n        }\n      }",
    );
    let doc = parse(&text).unwrap();
    let ch = &doc.network.nodes[0].animations["f"].channel;
    assert_eq!(ch.offset(), 2.0);
    assert_eq!(ch.keys()[0], Keyframe::float(0.0, 1.0));
    assert_eq!(
        ch.keys()[1].tangent,
        Tangent::Explicit {
            incoming: Value::Float(0.0),
            outgoing: Value::Float(-1.0)
        }
    );
    // Defaults are not written back.
    let canonical = format(&doc).unwrap();
    assert_eq!(
        anim_lines(&canonical),
        [
            "anim offset=2.0 {",
            "key 0.0 1.0",
            "key 1.0 2.0 tangent=\"explicit\" in=0.0 out=-1.0",
        ]
    );
}

#[test]
fn expressions_round_trip_with_escapes_and_unicode() {
    for source in [
        "sin($T)",
        "ch(\"sun/offset\") * 0.5",
        "ch(\"a \\\"quoted\\\" \\\\ name/x\")",
        "\"héllo ✓ 日本\" + \"\\u{1F600}\"",
        "tab\there\nnewline\r\u{1}\u{2028}\u{feff}",
        "",
    ] {
        let mut doc = Document::new("exprs");
        let mut node = DocNode::new(id(1), "n", "test.all@1");
        node.expressions.insert("s".into(), DocExpr::new(source));
        doc.network.nodes.push(node);
        let text = format(&doc).unwrap();
        assert_eq!(text.lines().count(), 7, "one line per expression: {text}");
        let back = parse(&text).unwrap();
        assert_eq!(back, doc, "{text}");
        assert_eq!(back.network.nodes[0].expressions["s"].source, source);
        assert_eq!(format(&back).unwrap(), text);
    }
}

/// Nodes whose names need escaping, read by `ch()` from expressions.
#[test]
fn expressions_with_awkward_names_build_graphs() {
    let mut graph = Graph::new(registry());
    let sun = graph
        .add_node_with_id(id(1), "core.time.timeofday@1", "sön \"☀\" \\ a/b")
        .unwrap();
    let lamp = graph
        .add_node_with_id(id(2), "core.math.lerp@1", "lamp")
        .unwrap();
    graph.set_input(sun, "offset", Value::Float(0.25)).unwrap();
    let path = expr_quote("sön \"☀\" \\ a/b/offset");
    graph
        .set_expression(lamp, "t", &format!("ch({path}) * 2.0 + $T"))
        .unwrap();
    graph
        .set_expression(lamp, "a", &format!("ch(\"lamp/t\") + ch({path})"))
        .unwrap_err(); // A node cannot read itself.
    graph
        .set_expression(lamp, "a", &format!("ch({path})"))
        .unwrap();
    let doc = Document::from_graph(&graph, "awkward").unwrap();
    let text = round_trip(&doc);
    assert!(
        text.contains(r#"expr="ch(\"sön \\\"☀\\\" \\\\ a/b/offset\") * 2.0 + $T""#),
        "{text}"
    );
    let back = parse(&text).unwrap().to_graph(registry()).unwrap();
    assert_eq!(back.snapshot(), graph.snapshot());
}

#[test]
fn combined_sources_on_one_port() {
    let text = concat!(
        "room \"r\" format=1 min-tier=\"T0\" {\n  network \"/\" {\n",
        "    node \"sun\" type=\"core.time.timeofday@1\" id=\"01920000-0000-7000-8000-000000000001\" {\n",
        "      input \"offset\" 0.25\n    }\n",
        "    node \"lamp\" type=\"core.math.lerp@1\" id=\"01920000-0000-7000-8000-000000000002\" {\n",
        "      input \"t\" 0.1 expr=\"ch(\\\"sun/offset\\\") * 0.5\" {\n",
        "        anim post=\"loop\" {\n",
        "          key 0.0 0.2 interp=\"bezier\"\n          key 1.0 0.8\n",
        "        }\n      }\n    }\n  }\n}\n",
    );
    let doc = parse(text).unwrap();
    let lamp = &doc.network.nodes[1];
    assert_eq!(lamp.inputs["t"], Value::Float(0.1));
    assert_eq!(lamp.expressions["t"].source, "ch(\"sun/offset\") * 0.5");
    assert_eq!(lamp.animations["t"].channel.keys().len(), 2);
    assert_eq!(lamp.ports().collect::<Vec<_>>(), ["t"]);
    assert_eq!(
        format(&doc).unwrap(),
        text,
        "the input is already canonical"
    );

    let graph = doc.to_graph(registry()).unwrap();
    let id = graph.find("lamp").unwrap();
    assert_eq!(
        graph.input_value(id, "t").unwrap(),
        Some(&Value::Float(0.1))
    );
    assert_eq!(
        graph.expression(id, "t").unwrap(),
        Some("ch(\"sun/offset\") * 0.5")
    );
    assert!(graph.animation(id, "t").unwrap().is_some());
    round_trip(&doc);

    // A link on top of all three is kept too.
    let linked = text.replace(
        "    }\n  }\n}\n",
        "    }\n    link from=\"sun.phase\" to=\"lamp.t\"\n  }\n}\n",
    );
    let doc = parse(&linked).unwrap();
    assert_eq!(doc.network.links.len(), 1);
    round_trip(&doc);
}

#[test]
fn anim_errors_carry_locations() {
    let anim = |keys: &str| {
        all_node(&format!(
            "      input \"f\" {{\n        anim {{\n{keys}\n        }}\n      }}"
        ))
    };
    // Line 4: input, line 5: anim, keys from line 6.
    let cases: &[(&str, &str, Option<Location>)] = &[
        (
            "          key 0.0 1.0 interp=\"cubic\"",
            "unknown interpolation \"cubic\"; expected one of \"step\", \"linear\", \"bezier\", \"spring\"",
            loc(6, 23),
        ),
        (
            "          key 0.0 1.0 tangent=\"smooth\"",
            "unknown tangent \"smooth\"",
            loc(6, 23),
        ),
        (
            "          key 0.0 1.0 interp=\"spring\" damping=1.0",
            "interp=\"spring\" needs `frequency=`",
            loc(6, 11),
        ),
        (
            "          key 0.0 1.0 frequency=1.0",
            "`frequency` is only allowed with interp=\"spring\"",
            loc(6, 23),
        ),
        (
            "          key 0.0 1.0 tangent=\"explicit\" in=1.0",
            "tangent=\"explicit\" needs `out=`",
            loc(6, 11),
        ),
        (
            "          key 0.0 1.0 out=1.0",
            "`out` is only allowed with tangent=\"explicit\"",
            loc(6, 23),
        ),
        (
            "          key 0.0 1.0 interp=\"spring\" frequency=0.0 damping=1.0",
            "key 0: spring frequency must be positive",
            loc(6, 11),
        ),
        (
            "          key 0.0 1.0\n          key 0.0 2.0",
            "key 1: key times must be strictly increasing",
            loc(7, 11),
        ),
        (
            "          key 0.0 1.0\n          key 1.0 #nan",
            "NaN",
            loc(7, 19),
        ),
        ("          key #inf 1.0", "NaN", loc(6, 15)),
        (
            "          key 0.0 #true",
            "key 0: Bool values cannot be animated",
            loc(6, 11),
        ),
        (
            "          key 0.0 \"text\"",
            "key 0: String values cannot be animated",
            loc(6, 11),
        ),
        (
            "          key 0.0",
            "expected `key <time> <value>`",
            loc(6, 11),
        ),
        (
            "          frame 0.0 1.0",
            "unknown node `frame` in anim",
            loc(6, 11),
        ),
        (
            "          key (vec2)\"1 2\" 1.0",
            "key time must be a number",
            loc(6, 15),
        ),
        ("", "at least one key", loc(5, 9)),
    ];
    for (keys, needle, at) in cases {
        let e = err(&anim(keys));
        assert!(e.to_string().contains(needle), "{keys}: {e}");
        assert_eq!(e.location(), *at, "{keys}: {e}");
    }

    let input = |body: &str| err(&all_node(body));
    for (body, needle, at) in [
        (
            "      input \"f\" { anim pre=\"bounce\" { key 0.0 1.0; }; }",
            "unknown pre extrapolation \"bounce\"; expected one of \"hold\", \"loop\", \"ping-pong\"",
            loc(4, 24),
        ),
        (
            "      input \"f\" { anim offset=#inf { key 0.0 1.0; }; }",
            "NaN",
            loc(4, 24),
        ),
        (
            "      input \"f\" { anim speed=2.0 { key 0.0 1.0; }; }",
            "unknown property `speed`",
            loc(4, 24),
        ),
        (
            "      input \"f\" { anim 1.0 { key 0.0 1.0; }; }",
            "`anim` takes no arguments",
            loc(4, 24),
        ),
        (
            "      input \"f\" { anim { key 0.0 1.0; }; anim { key 0.0 1.0; }; }",
            "more than one `anim` block",
            loc(4, 42),
        ),
        (
            "      input \"f\" { curve; }",
            "unknown node `curve` in input 'f'",
            loc(4, 19),
        ),
        (
            "      input \"f\" expr=1.0",
            "expr must be a string",
            loc(4, 17),
        ),
        ("      input \"f\" {}", "input 'f' sets nothing", loc(4, 7)),
    ] {
        let e = input(body);
        assert!(e.to_string().contains(needle), "{body}: {e}");
        assert_eq!(e.location(), at, "{body}: {e}");
    }
}

/// A sun and two lamps whose `t` inputs carry `rest` and `other`.
fn lamps(rest: &str, other: &str) -> String {
    format!(
        "{HEADER}    node \"sun\" type=\"core.time.timeofday@1\" id=\"01920000-0000-7000-8000-000000000001\"\n    node \"a\" type=\"core.math.lerp@1\" id=\"01920000-0000-7000-8000-000000000002\" {{\n      input \"t\" {rest}\n    }}\n    node \"b\" type=\"core.math.lerp@1\" id=\"01920000-0000-7000-8000-000000000003\" {{\n      input \"t\" {other}\n    }}\n  }}\n}}\n"
    )
}

fn graph_err(text: &str) -> DocError {
    parse(text)
        .unwrap()
        .to_graph(registry())
        .expect_err("expected a graph error")
}

#[test]
fn expression_errors_carry_locations() {
    // Forward references work: `a` reads `b`, which comes later.
    let text = lamps(r#"expr="ch(\"b/a\")""#, "0.5");
    parse(&text).unwrap().to_graph(registry()).unwrap();

    let e = graph_err(&lamps(r#"expr="ch(\"moon/offset\")""#, "0.5"));
    assert!(
        matches!(&e, DocError::Graph { source: GraphError::Expression(x), .. }
            if matches!(&x.kind, ErrorKind::UnknownChannel(p) if p == "moon/offset")),
        "{e}"
    );
    assert_eq!(e.location(), loc(5, 17), "{e}");
    assert!(
        e.to_string()
            .starts_with("5:17: node 'a' input 't' expr: expression error: unknown parameter path \"moon/offset\""),
        "{e}"
    );

    // A cycle through two expressions is reported at the second one.
    let e = graph_err(&lamps(r#"expr="ch(\"b/a\")""#, r#"expr="ch(\"a/b\")""#));
    assert!(
        matches!(
            &e,
            DocError::Graph {
                source: GraphError::WouldCycle,
                ..
            }
        ),
        "{e}"
    );
    assert_eq!(e.location(), loc(8, 17), "{e}");
    assert!(e.to_string().contains("node 'b' input 't' expr"), "{e}");

    // A cycle closed by an expression on top of a link.
    let text = lamps("0.5", r#"expr="ch(\"a/a\")""#)
        .replace("  }\n}\n", "    link from=\"b.out\" to=\"a.t\"\n  }\n}\n");
    let e = graph_err(&text);
    assert!(
        matches!(
            &e,
            DocError::Graph {
                source: GraphError::WouldCycle,
                ..
            }
        ),
        "{e}"
    );
    assert_eq!(e.location(), loc(8, 17), "{e}");

    // Syntax and type errors.
    let e = graph_err(&lamps(r#"expr="1 +""#, "0.5"));
    assert_eq!(e.location(), loc(5, 17), "{e}");
    let e = graph_err(&lamps(r#"expr="vec2(1.0, 2.0)""#, "0.5"));
    assert!(e.to_string().contains("type mismatch"), "{e}");

    // An animation of the wrong type, at the anim node.
    let e = graph_err(&lamps(
        "{\n        anim { key 0.0 (vec2)\"1 2\"; }\n      }",
        "0.5",
    ));
    assert!(
        e.to_string()
            .starts_with("6:9: node 'a' input 't' anim: type mismatch"),
        "{e}"
    );
}
