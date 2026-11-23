// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared Charm definitions in documents (M0.16): the `charms` block,
//! instances with `def="id@version"`, round trips, strict errors with
//! line and column, and the diff.

mod common;

use common::registry;
use livspeil_doc::{Document, Subject, canonicalize, diff, format, parse};
use livspeil_graph::{Cooker, INTERFACE_PORT, PortDirection, instance_node_id};
use livspeil_node_api::Value;

/// A definition `twice` (`out = in * k`, `k` defaults to 2) and two
/// instances; `b` overrides `k` with an expression.
const ROOM: &str = r#"room "defs" format=1 min-tier="T0" {
  charms {
    def "twice" version="1.0.0" name="Twice" icon="x2" min-tier="T1" {
      capability "time"
      network {
        node "in" type="core.subnet.input@1" id="01920000-0000-7000-8000-000000000001" port-type="Float"
        node "k" type="core.subnet.input@1" id="01920000-0000-7000-8000-000000000002" port-type="Float" {
          input "value" 2.0
        }
        node "mul" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003"
        node "out" type="core.output@1" id="01920000-0000-7000-8000-000000000004" port-type="Float"
        link from="in.value" to="mul.a"
        link from="k.value" to="mul.b"
        link from="mul.out" to="out.value"
      }
    }
  }
  network "/" {
    node "x" type="core.math.add@1" id="01920000-0000-7000-8000-000000000011" {
      input "a" 3.0
    }
    node "a" type="core.charm@1" id="01920000-0000-7000-8000-000000000012" def="twice@1.0.0"
    node "b" type="core.charm@1" id="01920000-0000-7000-8000-000000000013" def="twice@1.0.0" {
      input "k" 1.0 expr="ch(\"../x/a\") + 1"
    }
    node "y" type="core.math.add@1" id="01920000-0000-7000-8000-000000000014"
    link from="a.out" to="y.a"
    link from="b.out" to="y.b"
    link from="x.out" to="a.in"
    link from="x.out" to="b.in"
  }
}
"#;

fn err(text: &str) -> livspeil_doc::DocError {
    match parse(text) {
        Err(e) => e,
        Ok(doc) => doc.to_graph(registry()).expect_err("an error"),
    }
}

#[test]
fn definitions_round_trip_and_cook() {
    assert_eq!(canonicalize(ROOM).unwrap(), ROOM);
    let doc = parse(ROOM).unwrap();
    assert_eq!(format(&doc).unwrap(), ROOM);
    assert_eq!(doc.charms.len(), 1);
    assert_eq!(doc.charms[0].meta.name, "Twice");
    let graph = doc.to_graph(registry()).unwrap();
    let a = graph.find("a").unwrap();
    assert!(graph.is_instance(a));
    assert_eq!(graph.charm(a).unwrap().version.as_deref(), Some("1.0.0"));
    assert_eq!(graph.interface(a, PortDirection::Input).len(), 2);
    // y = 3 * 2 + 3 * (3 + 1)
    let y = graph.find("y").unwrap();
    let out = Cooker::new().cook_output(&graph, y, "out").unwrap();
    assert_eq!(out, Value::Float(18.0));
    let back = Document::from_graph(&graph, doc.name.clone()).unwrap();
    assert_eq!(back, doc);
    assert_eq!(
        back.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
}

#[test]
fn editing_the_definition_changes_every_instance() {
    let doc = parse(ROOM).unwrap();
    let mut graph = doc.to_graph(registry()).unwrap();
    let mut def = graph.definition("twice").unwrap().clone();
    def.version = "1.1.0".to_owned();
    let k = def.nodes.iter_mut().find(|n| n.name == "k").unwrap();
    k.inputs = vec![(INTERFACE_PORT.to_owned(), Value::Float(10.0))];
    graph.update_definition(def).unwrap();
    let y = graph.find("y").unwrap();
    // a follows the new default; b keeps its override.
    let out = Cooker::new().cook_output(&graph, y, "out").unwrap();
    assert_eq!(out, Value::Float(30.0 + 12.0));
    let back = Document::from_graph(&graph, doc.name.clone()).unwrap();
    let text = format(&back).unwrap();
    assert!(text.contains(r#"def="twice@1.1.0""#), "{text}");
    assert!(text.contains(r#"input "value" 10.0"#), "{text}");
    // b's static value 1.0 differs from the new default too.
    assert!(text.contains(r#"input "k" 1.0 expr="#), "{text}");
    let changes = diff(&doc, &back);
    let defs: Vec<_> = changes
        .changes
        .iter()
        .filter(|c| matches!(c.subject, Subject::Definition(_)))
        .collect();
    assert_eq!(defs.len(), 1);
    assert_eq!(
        defs[0].to_string(),
        r#"~ def "twice": version="1.0.0" 4 nodes -> version="1.1.0" 4 nodes"#
    );
    assert!(
        changes
            .changes
            .iter()
            .any(|c| matches!(c.subject, Subject::InstanceDef(_)))
    );
    // Detaching writes a plain subnet with its contents.
    let a = graph.find("a").unwrap();
    graph.detach(a).unwrap();
    let detached = Document::from_graph(&graph, "d").unwrap();
    let node = detached
        .network
        .nodes
        .iter()
        .find(|n| n.name == "a")
        .unwrap();
    assert!(node.is_subnet());
    let inner = node.network.as_ref().unwrap();
    assert_eq!(inner.nodes.len(), 4);
    // The contents keep the instance's derived ids.
    let mul = inner.nodes.iter().find(|n| n.name == "mul").unwrap();
    assert_eq!(mul.id, instance_node_id(a, common::id(3)));
    let reparsed = parse(&format(&detached).unwrap()).unwrap();
    assert_eq!(
        reparsed.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
}

#[test]
fn errors_name_the_problem_and_its_line() {
    let with = |old: &str, new: &str| {
        assert!(ROOM.contains(old), "{old}");
        ROOM.replacen(old, new, 1)
    };
    let cases = [
        (
            with(r#"id="01920000-0000-7000-8000-000000000012" def="twice@1.0.0""#,
                 r#"id="01920000-0000-7000-8000-000000000012" def="nope@1.0.0""#),
            "unknown Charm definition 'nope'",
            22,
            76,
        ),
        (
            with(r#"id="01920000-0000-7000-8000-000000000012" def="twice@1.0.0""#,
                 r#"id="01920000-0000-7000-8000-000000000012" def="twice@2.0.0""#),
            "is at version 1.0.0, not 2.0.0",
            22,
            76,
        ),
        (
            with(r#"def="twice@1.0.0""#, r#"def="twice""#),
            "def must be \"id@version\"",
            22,
            76,
        ),
        (
            with(r#" def="twice@1.0.0""#, ""),
            "needs def=",
            22,
            5,
        ),
        (
            with(r#"input "a" 3.0"#, r#"input "a" 3.0"#)
                .replacen(r#"node "x" type="core.math.add@1" id="01920000-0000-7000-8000-000000000011""#,
                    r#"node "x" type="core.math.add@1" id="01920000-0000-7000-8000-000000000011" def="twice@1.0.0""#, 1),
            "only allowed on Charm instances",
            19,
            79,
        ),
        (
            with(r#"input "k" 1.0"#, r#"input "q" 1.0"#),
            "has no interface input 'q'",
            23,
            5,
        ),
        (
            with(r#"version="1.0.0" name="Twice""#, r#"name="Twice""#),
            "needs a version",
            3,
            5,
        ),
        (
            with(r#"capability "time""#, r#"capability "time"
      capability "time""#),
            "listed twice",
            5,
            7,
        ),
        (
            with("  charms {\n", "  charms {\n    def \"twice\" version=\"1\"\n"),
            "duplicate Charm definition 'twice'",
            4,
            5,
        ),
        (
            with(r#"node "mul" type="core.math.mul@1""#, r#"node "mul" type="core.math.nope@1""#),
            "unknown node type",
            10,
            9,
        ),
        (
            with(r#"node "mul" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003""#,
                 r#"node "mul" type="core.charm@1" id="01920000-0000-7000-8000-000000000003" def="twice@1.0.0""#),
            "not supported yet",
            10,
            9,
        ),
        (
            with(r#"node "mul" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003""#,
                 r#"node "mul" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003" {
          input "b" expr="ch(\"../x/a\")"
        }"#),
            "unknown parameter path \"../x/a\"",
            11,
            21,
        ),
        (
            with(r#"node "mul" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003""#,
                 r#"node "mul" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003" {
          input "b" expr="ch(\"../twice/k/value\")"
        }"#),
            "stay inside",
            3,
            5,
        ),
        (
            with(r#"to="a.in""#, r#"to="a.nope""#),
            "subnet 'a' has no input port 'nope'",
            29,
            5,
        ),
    ];
    for (text, needle, line, column) in cases {
        let e = err(&text);
        assert!(
            e.to_string()
                .to_lowercase()
                .contains(&needle.to_lowercase()),
            "{needle}: {e}"
        );
        let at = e.location().map(|l| (l.line, l.column));
        assert_eq!(at, Some((line, column)), "{needle}: {e}");
    }
}
