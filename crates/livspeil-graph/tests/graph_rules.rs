// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Structural rules enforced at edit time.

mod common;

use common::{float, graph};
use livspeil_graph::{GraphError, NodeId, PortDirection};
use livspeil_node_api::{PortType, Value};
use uuid::Uuid;

const ADD: &str = "core.math.add@1";
const CLOCK: &str = "core.time.clock@1";
const SINK: &str = "test.sink@1";

#[test]
fn rejects_unknown_types_and_bad_names() {
    let mut g = graph();
    assert_eq!(
        g.add_node("core.nope@1", "x").unwrap_err(),
        GraphError::UnknownNodeType("core.nope@1".to_owned())
    );
    assert_eq!(g.add_node(ADD, "").unwrap_err(), GraphError::EmptyName);
    g.add_node(ADD, "a").unwrap();
    assert_eq!(
        g.add_node(ADD, "a").unwrap_err(),
        GraphError::DuplicateName("a".to_owned())
    );
    assert_eq!(g.node_count(), 1);
}

#[test]
fn rejects_duplicate_ids() {
    let mut g = graph();
    let id = NodeId::from_uuid(Uuid::from_u128(1));
    g.add_node_with_id(id, ADD, "a").unwrap();
    assert_eq!(
        g.add_node_with_id(id, ADD, "b").unwrap_err(),
        GraphError::DuplicateId(id)
    );
    assert_eq!(g.node_count(), 1);
    assert!(g.find("b").is_none());
}

#[test]
fn exposes_names_types_and_specs() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    assert_eq!(g.find("a"), Some(a));
    assert_eq!(g.node_name(a), Some("a"));
    assert_eq!(g.node_type(a), Some(ADD));
    assert_eq!(g.node_spec(a).unwrap().id, "core.math.add");
    assert_eq!(g.node_ids().collect::<Vec<_>>(), vec![a]);
}

#[test]
fn unknown_ports_name_their_direction() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    assert_eq!(
        g.link(a, "nope", b, "a").unwrap_err(),
        GraphError::UnknownPort {
            node: a,
            port: "nope".to_owned(),
            direction: PortDirection::Output
        }
    );
    assert_eq!(
        g.link(a, "out", b, "nope").unwrap_err(),
        GraphError::UnknownPort {
            node: b,
            port: "nope".to_owned(),
            direction: PortDirection::Input
        }
    );
    assert_eq!(
        g.set_input(a, "nope", float(1.0)).unwrap_err(),
        GraphError::UnknownPort {
            node: a,
            port: "nope".to_owned(),
            direction: PortDirection::Input
        }
    );
}

#[test]
fn links_are_type_checked() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let clock = g.add_node(CLOCK, "clock").unwrap();
    let sink = g.add_node(SINK, "sink").unwrap();

    // Float -> Bool and Float -> Int are not allowed.
    assert_eq!(
        g.link(a, "out", sink, "flag").unwrap_err(),
        GraphError::TypeMismatch {
            expected: PortType::Bool,
            found: PortType::Float
        }
    );
    assert_eq!(
        g.link(a, "out", sink, "count").unwrap_err(),
        GraphError::TypeMismatch {
            expected: PortType::Int,
            found: PortType::Float
        }
    );
    // Int -> Float is the one implicit conversion.
    g.link(clock, "frame", a, "a").unwrap();
    assert_eq!(g.link_count(), 1);
}

#[test]
fn set_input_checks_and_coerces_types() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    assert_eq!(
        g.set_input(a, "a", Value::Bool(true)).unwrap_err(),
        GraphError::TypeMismatch {
            expected: PortType::Float,
            found: PortType::Bool
        }
    );
    g.set_input(a, "a", Value::Int(3)).unwrap();
    g.clear_input(a, "a").unwrap();
}

#[test]
fn an_input_takes_one_link() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    g.link(a, "out", c, "a").unwrap();
    assert_eq!(
        g.link(b, "out", c, "a").unwrap_err(),
        GraphError::InputOccupied {
            node: c,
            port: "a".to_owned()
        }
    );
    // An output may feed many inputs.
    g.link(a, "out", c, "b").unwrap();
    assert_eq!(g.link_count(), 2);

    assert!(g.unlink(c, "a").unwrap());
    assert!(!g.unlink(c, "a").unwrap());
    g.link(b, "out", c, "a").unwrap();
}

#[test]
fn cycles_are_rejected_and_leave_the_graph_unchanged() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();

    assert_eq!(
        g.link(a, "out", a, "a").unwrap_err(),
        GraphError::WouldCycle
    );
    g.link(a, "out", b, "a").unwrap();
    g.link(b, "out", c, "a").unwrap();
    assert_eq!(
        g.link(c, "out", a, "a").unwrap_err(),
        GraphError::WouldCycle
    );
    assert_eq!(
        g.link(b, "out", a, "b").unwrap_err(),
        GraphError::WouldCycle
    );
    assert_eq!(g.link_count(), 2);

    // Diamonds are fine.
    g.link(a, "out", c, "b").unwrap();
    assert_eq!(g.link_count(), 3);
}

#[test]
fn removing_a_node_removes_its_links() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    let c = g.add_node(ADD, "c").unwrap();
    g.link(a, "out", b, "a").unwrap();
    g.link(b, "out", c, "a").unwrap();

    g.remove_node(b).unwrap();
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.link_count(), 0);
    assert!(g.find("b").is_none());
    assert_eq!(g.remove_node(b).unwrap_err(), GraphError::UnknownNode(b));

    // The name and the freed input can be reused.
    let b2 = g.add_node(ADD, "b").unwrap();
    g.link(b2, "out", c, "a").unwrap();
    assert_eq!(g.link_count(), 1);
}

#[test]
fn links_are_listed_in_target_order() {
    let mut g = graph();
    let a = g.add_node(ADD, "a").unwrap();
    let b = g.add_node(ADD, "b").unwrap();
    g.link(a, "out", b, "b").unwrap();
    let links: Vec<_> = g.links().collect();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].from.node, a);
    assert_eq!(links[0].to.node, b);
    assert_eq!(links[0].to.port, 1);
}
