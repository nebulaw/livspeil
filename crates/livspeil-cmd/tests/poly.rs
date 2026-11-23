// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Polymorphic nodes through the command bus (M0.17): undo and redo
//! restore resolutions without commands of their own, because resolution
//! is a function of the graph state (links, sources and pins).

use std::sync::Arc;

use livspeil_cmd::{Command, History, Origin};
use livspeil_graph::{Graph, NodeId};
use livspeil_node_api::{PortType, Value};
use uuid::Uuid;

fn id(n: u128) -> NodeId {
    NodeId::from_uuid(Uuid::from_u128(n))
}

fn graph() -> Graph {
    Graph::new(Arc::new(livspeil_nodes_core::core_registry()))
}

fn types(g: &Graph) -> Vec<Vec<PortType>> {
    g.node_ids().map(|n| g.resolved_types(n).to_vec()).collect()
}

#[test]
fn undo_and_redo_restore_resolutions() {
    let mut g = graph();
    let mut history = History::new();
    let steps = [
        Command::CreateNode {
            id: id(1),
            type_id: "core.logic.select@2".into(),
            name: "pick".into(),
        },
        Command::CreateNode {
            id: id(2),
            type_id: "core.math.lerp@2".into(),
            name: "mix".into(),
        },
        Command::CreateNodeIn {
            id: id(3),
            parent: None,
            type_id: "core.curve.ramp@2".into(),
            name: "ramp".into(),
            port_type: Some(PortType::Color),
        },
        Command::Link {
            from: id(3),
            from_port: "out".into(),
            to: id(2),
            to_port: "a".into(),
        },
        Command::Link {
            from: id(2),
            from_port: "out".into(),
            to: id(1),
            to_port: "b".into(),
        },
        Command::SetInput {
            node: id(2),
            port: "t".into(),
            value: Value::Float(0.25),
        },
    ];
    let mut states = vec![(g.snapshot(), types(&g))];
    for step in steps {
        history
            .apply(&mut g, step, Origin::Ui, None)
            .expect("applies");
        states.push((g.snapshot(), types(&g)));
    }
    assert_eq!(g.resolved_type(id(1), "T"), Some(PortType::Color));
    assert_eq!(g.resolved_type(id(2), "T"), Some(PortType::Color));
    // A conflicting edit fails and leaves history and graph alone.
    let bad = Command::SetInput {
        node: id(1),
        port: "a".into(),
        value: Value::Float(1.0),
    };
    assert!(history.apply(&mut g, bad, Origin::Ui, None).is_err());
    assert_eq!(states.last().map(|s| &s.0), Some(&g.snapshot()));
    // Deleting the pinned ramp unresolves the chain; undo restores it.
    history
        .apply(&mut g, Command::DeleteNode { id: id(3) }, Origin::Ui, None)
        .expect("deletes");
    assert_eq!(g.resolved_type(id(1), "T"), Some(PortType::Float));
    history.undo(&mut g).expect("undo delete");
    assert_eq!(g.resolved_type(id(1), "T"), Some(PortType::Color));
    for state in states.iter().rev().skip(1) {
        history.undo(&mut g).expect("undo");
        assert_eq!(&(g.snapshot(), types(&g)), state);
    }
    for state in states.iter().skip(1) {
        history.redo(&mut g).expect("redo");
        assert_eq!(&(g.snapshot(), types(&g)), state);
    }
}
