// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Renaming, read access to set inputs and links, and snapshots.

mod common;

use common::{float, graph};
use livspeil_graph::{CookStats, Cooker, GraphError, NamedLink, NodeId, PortDirection};
use livspeil_node_api::Value;
use uuid::Uuid;

const ADD: &str = "core.math.add@1";

fn id(n: u128) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(n))
}

fn named(from: NodeId, from_port: &str, to: NodeId, to_port: &str) -> NamedLink {
    NamedLink {
        from,
        from_port: from_port.to_owned(),
        to,
        to_port: to_port.to_owned(),
    }
}

#[test]
fn rename_updates_lookup_and_checks_names() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    g.add_node(ADD, "b").unwrap();
    assert_eq!(g.rename(a, "c").unwrap(), "a");
    assert_eq!(g.find("c"), Some(a));
    assert_eq!(g.find("a"), None);
    assert_eq!(g.node_name(a), Some("c"));
    // Same name is a no-op.
    assert_eq!(g.rename(a, "c").unwrap(), "c");
    assert_eq!(g.rename(a, "b"), Err(GraphError::DuplicateName("b".into())));
    assert_eq!(g.rename(a, ""), Err(GraphError::EmptyName));
    assert_eq!(g.rename(id(99), "z"), Err(GraphError::UnknownNode(id(99))));
    assert_eq!(g.node_name(a), Some("c"));
    // The old name is free again.
    g.add_node(ADD, "a").unwrap();
}

#[test]
fn rename_keeps_cooked_results() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    g.link(a, "out", b, "a").unwrap();
    let mut cooker = Cooker::new();
    cooker.cook(&g, b).unwrap();
    cooker.reset_stats();
    g.rename(a, "renamed").unwrap();
    g.rename(b, "also renamed").unwrap();
    cooker.cook(&g, b).unwrap();
    assert_eq!(
        cooker.stats(),
        CookStats {
            cooked: 0,
            reused: 2
        }
    );
}

#[test]
fn input_values_report_only_explicit_values() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    assert_eq!(g.input_value(a, "b").unwrap(), None);
    g.set_input(a, "b", Value::Int(2)).unwrap();
    assert_eq!(g.input_value(a, "b").unwrap(), Some(&float(2.0)));
    assert_eq!(
        g.input_values(a).collect::<Vec<_>>(),
        vec![("b", &float(2.0))]
    );
    assert!(matches!(
        g.input_value(a, "nope"),
        Err(GraphError::UnknownPort {
            direction: PortDirection::Input,
            ..
        })
    ));
    assert_eq!(g.input_values(id(99)).count(), 0);
}

#[test]
fn links_are_reported_with_port_names() {
    let mut g = graph();
    let a = g.add_node_with_id(id(1), ADD, "a").unwrap();
    let b = g.add_node_with_id(id(2), ADD, "b").unwrap();
    let c = g.add_node_with_id(id(3), ADD, "c").unwrap();
    g.link(a, "out", b, "b").unwrap();
    g.link(a, "out", c, "a").unwrap();
    g.link(b, "out", c, "b").unwrap();

    assert_eq!(g.link_source(c, "b").unwrap(), Some((b, "out")));
    assert_eq!(g.link_source(b, "a").unwrap(), None);
    assert_eq!(
        g.outgoing_links(a),
        vec![named(a, "out", b, "b"), named(a, "out", c, "a")]
    );
    assert_eq!(
        g.incoming_links(c),
        vec![named(a, "out", c, "a"), named(b, "out", c, "b")]
    );
    assert!(g.incoming_links(a).is_empty());
}

#[test]
fn snapshots_compare_editable_state_only() {
    let build = |names: [&str; 2]| {
        let mut g = graph();
        let a = g.add_node_with_id(id(1), ADD, names[0]).unwrap();
        let b = g.add_node_with_id(id(2), ADD, names[1]).unwrap();
        g.set_input(a, "a", float(1.0)).unwrap();
        g.link(a, "out", b, "a").unwrap();
        g
    };
    let mut g1 = build(["a", "b"]);
    let g2 = build(["a", "b"]);
    assert_eq!(g1.snapshot(), g2.snapshot());

    let snap = g1.snapshot();
    assert_eq!(snap.nodes.len(), 2);
    assert_eq!(snap.nodes[0].inputs, vec![("a".to_owned(), float(1.0))]);
    assert_eq!(snap.links, vec![named(id(1), "out", id(2), "a")]);

    // Edits that end where they started give the same snapshot.
    g1.set_input(id(2), "b", float(3.0)).unwrap();
    g1.clear_input(id(2), "b").unwrap();
    assert_eq!(g1.snapshot(), g2.snapshot());
    assert_ne!(build(["a", "x"]).snapshot(), g2.snapshot());
}
