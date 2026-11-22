// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Nested networks, interface ports and Charm metadata in `room.kdl`.

mod common;

use std::path::Path;

use common::registry;
use livspeil_doc::{DocError, Document, Location, canonicalize, format, parse, split_prolog};
use livspeil_graph::{GraphError, PortDirection};
use livspeil_node_api::PortType;

fn charm_demo() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/charm-demo.room/room.kdl");
    std::fs::read_to_string(path).unwrap()
}

const ID: &str = "0193a6c0-1d2e-7b40-8000-0000000000";

fn id(n: u8) -> String {
    format!("id=\"{ID}{n:02x}\"")
}

/// `/x -> /s.in`, `/s.out -> /y`, with a Charm block.
fn small() -> String {
    format!(
        r#"room "small" format=1 min-tier="T0" {{
  network "/" {{
    node "x" type="core.math.add@1" {x} {{
      input "a" 1.5
    }}
    node "s" type="core.subnet@1" {s} {{
      charm "Doubler" version="0.1.0" icon="x2" min-tier="T2" {{
        capability "net"
        capability "time"
      }}
      network {{
        node "in" type="core.subnet.input@1" {i} port-type="Float"
        node "double" type="core.math.mul@1" {d} {{
          input "a" expr="ch(\"in/value\") + ch(\"../x/a\")"
          input "b" 2.0
        }}
        node "out" type="core.output@1" {o} port-type="Float"
        node "inner" type="core.subnet@1" {n} {{
          network
        }}
        link from="double.out" to="out.value"
      }}
    }}
    node "y" type="core.math.add@1" {y}
    link from="s.out" to="y.a"
    link from="x.out" to="s.in"
  }}
  meta {{
    node "s/double" x=1.0 y=2.0
    node "x" x=0.0 y=0.0
  }}
}}
"#,
        x = id(1),
        s = id(2),
        i = id(3),
        d = id(4),
        o = id(5),
        n = id(6),
        y = id(7),
    )
}

#[test]
fn charm_demo_is_canonical_and_round_trips() {
    let text = charm_demo();
    assert_eq!(canonicalize(&text).unwrap(), text);
    let doc = parse(&text).unwrap();
    let (_, body) = split_prolog(&text);
    assert_eq!(format(&doc).unwrap(), body);
    let graph = doc.to_graph(registry()).unwrap();
    // 4 root nodes + 4 outputs + 2 Charms of 11 nodes each.
    assert_eq!(graph.node_count(), 29);
    let desk = graph.find("desk_lamp").unwrap();
    assert_eq!(graph.charm(desk).unwrap().name, "Daylight");
    assert_eq!(graph.interface(desk, PortDirection::Input).len(), 3);
    let back = Document::from_graph(&graph, doc.name.clone()).unwrap();
    let mut expected = doc.clone();
    expected.min_tier = back.min_tier;
    assert_eq!(back, expected);
    assert_eq!(
        back.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
}

#[test]
fn small_subnet_round_trips_exactly() {
    let text = small();
    assert_eq!(canonicalize(&text).unwrap(), text);
    let doc = parse(&text).unwrap();
    let s = &doc.network.nodes[1];
    let charm = s.charm.as_ref().unwrap();
    assert_eq!(charm.min_tier, 2);
    assert_eq!(charm.capabilities, ["net", "time"]);
    let inner = s.network.as_ref().unwrap();
    assert_eq!(inner.nodes[0].port_type, Some(PortType::Float));
    assert_eq!(inner.nodes[3].network.as_ref().unwrap().nodes.len(), 0);
    let graph = doc.to_graph(registry()).unwrap();
    assert_eq!(
        graph.path(graph.find_path("/s/double").unwrap()).as_deref(),
        Some("/s/double")
    );
    let mut back = Document::from_graph(&graph, "small").unwrap();
    back.meta = doc.meta.clone();
    assert_eq!(back, doc);
    assert_eq!(format(&back).unwrap(), text);
}

fn err(text: &str) -> DocError {
    match parse(text) {
        Err(e) => e,
        Ok(doc) => doc.to_graph(registry()).unwrap_err(),
    }
}

/// Replace the first occurrence of `from` in the small document.
fn small_with(from: &str, to: &str) -> String {
    let text = small();
    assert!(text.contains(from), "{from}");
    text.replacen(from, to, 1)
}

#[test]
fn errors_name_the_problem_and_its_line() {
    let cases: Vec<(String, &str, usize)> = vec![
        (
            small_with(
                r#"node "y" type="core.math.add@1""#,
                r#"node "y" type="core.math.add@1" port-type="Colour""#,
            ),
            "unknown port type \"Colour\"",
            24,
        ),
        (
            small_with("      network {", "      network \"/s\" {"),
            "takes no path",
            11,
        ),
        (
            small_with(
                r#"node "y" type="core.math.add@1" id="0193a6c0-1d2e-7b40-8000-000000000007""#,
                r#"node "y" type="core.math.add@1" id="0193a6c0-1d2e-7b40-8000-000000000007" { network; }"#,
            ),
            "only allowed in a subnet",
            24,
        ),
        (
            small_with(r#"capability "time""#, r#"capability "net""#),
            "listed twice",
            9,
        ),
        (
            small_with(r#"min-tier="T2""#, r#"min-tier="2""#),
            "not a tier",
            7,
        ),
        (
            small_with(r#"to="s.in""#, r#"to="s.nope""#),
            "subnet 's' has no input port 'nope'",
            26,
        ),
        (
            small_with(r#"to="y.a""#, r#"to="s.out""#),
            "subnet 's' has no input port 'out'",
            25,
        ),
        (
            small_with(r#"node "inner""#, r#"node "in/ner""#),
            "cannot name a subnet",
            18,
        ),
        (
            small_with(r#"node "x" x=0.0"#, r#"node "double" x=0.0"#),
            "unknown node 'double'",
            28,
        ),
        (
            small_with(
                r#"from="double.out" to="out.value""#,
                r#"from="x.out" to="out.value""#,
            ),
            "no node named 'x'",
            21,
        ),
        (
            small_with(r#"ch(\"../x/a\")"#, r#"ch(\"x/a\")"#),
            "unknown parameter path \"x/a\"",
            14,
        ),
    ];
    for (text, needle, line) in cases {
        let e = err(&text);
        assert!(
            e.to_string()
                .to_lowercase()
                .contains(&needle.to_lowercase()),
            "{needle}: {e}"
        );
        assert_eq!(e.location().map(|l| l.line), Some(line), "{needle}: {e}");
    }
}

#[test]
fn graph_rules_apply_inside_subnets() {
    // A link from outside straight to an inner node crosses networks.
    let text = small_with(
        r#"link from="x.out" to="s.in""#,
        r#"link from="x.out" to="s.in"
    link from="y.out" to="s.in""#,
    );
    let e = err(&text);
    assert!(
        matches!(
            &e,
            DocError::Graph {
                source: GraphError::InputOccupied { .. },
                ..
            }
        ),
        "{e}"
    );
    // An interface input in the root network.
    let text = small_with(
        r#"node "y" type="core.math.add@1""#,
        r#"node "y" type="core.subnet.input@1""#,
    );
    let e = err(&text);
    assert!(
        matches!(
            &e,
            DocError::Graph {
                source: GraphError::NotInSubnet(_),
                at: Some(Location { line: 24, .. }),
                ..
            }
        ),
        "{e}"
    );
    // A port type on a node type without polymorphic ports.
    let text = small_with(
        &format!(r#"node "y" type="core.math.add@1" {}"#, id(7)),
        &format!(
            r#"node "y" type="core.math.add@1" {} port-type="Float""#,
            id(7)
        ),
    );
    assert!(matches!(
        err(&text),
        DocError::Graph {
            source: GraphError::NotPolymorphic(_),
            ..
        }
    ));
    // Names are unique per network only.
    let text = small_with(
        r#"node "out" type="core.output@1""#,
        r#"node "x" type="core.output@1""#,
    )
    .replace(r#"to="out.value""#, r#"to="x.value""#)
    .replace(r#"from="s.out""#, r#"from="s.x""#);
    parse(&text).unwrap().to_graph(registry()).unwrap();
}
