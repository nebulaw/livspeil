// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Commands on shared Charm definitions and instances, and rewired moves
//! (M0.16): every one applies, inverts exactly, re-applies and journals.

mod common;

use common::{D_GAIN, D_IN, D_NUM, D_OUT, NUM, definition, graph, id};
use livspeil_cmd::{Applied, Command, DocChange};
use livspeil_graph::{
    Cooker, Graph, GraphError, INPUT_TYPE, INTERFACE_PORT, NodeId, OUTPUT_TYPE, PortDirection,
    instance_node_id,
};
use livspeil_node_api::{PortType, Value};

fn iface(instance: u128, local: u128) -> NodeId {
    instance_node_id(id(instance), id(local))
}

/// Apply `cmd`, check that its inverse restores the previous snapshot,
/// that it survives a JSON round trip, and re-apply it.
fn check(g: &mut Graph, cmd: &Command) -> Applied {
    let json = serde_json::to_string(cmd).unwrap();
    let parsed: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(&parsed, cmd);
    let before = g.snapshot();
    let done = cmd.apply(g).unwrap_or_else(|e| panic!("{cmd:?}: {e}"));
    let after = g.snapshot();
    done.inverse
        .apply(g)
        .unwrap_or_else(|e| panic!("inverse of {cmd:?}: {e}"));
    assert_eq!(g.snapshot(), before, "inverse of {cmd:?}");
    let again = cmd.apply(g).unwrap();
    assert_eq!(g.snapshot(), after);
    again
}

fn out(g: &Graph, node: NodeId) -> f64 {
    Cooker::new()
        .cook_output(g, node, INTERFACE_PORT)
        .unwrap()
        .as_float()
        .unwrap()
}

fn instantiate(n: u128, name: &str) -> Command {
    Command::Instantiate {
        id: id(n),
        parent: None,
        name: name.to_owned(),
        def: "d".to_owned(),
    }
}

/// Definition `d` with instances 10 (`a`) and 20 (`b`), node 1 (`x`,
/// value 3) feeding `a.in`, and `b.gain` overridden to 5.
fn setup() -> Graph {
    let mut g = graph();
    check(
        &mut g,
        &Command::AddDefinition {
            def: definition("d", "1.0.0", 2.0, 0),
        },
    );
    check(&mut g, &instantiate(10, "a"));
    check(&mut g, &instantiate(20, "b"));
    let x = Command::CreateNode {
        id: id(1),
        type_id: NUM.to_owned(),
        name: "x".to_owned(),
    };
    check(&mut g, &x);
    check(
        &mut g,
        &Command::SetInput {
            node: id(1),
            port: "a".to_owned(),
            value: Value::Float(3.0),
        },
    );
    check(
        &mut g,
        &Command::Link {
            from: id(1),
            from_port: "out".to_owned(),
            to: iface(10, D_IN),
            to_port: INTERFACE_PORT.to_owned(),
        },
    );
    check(
        &mut g,
        &Command::SetInput {
            node: iface(20, D_GAIN),
            port: INTERFACE_PORT.to_owned(),
            value: Value::Float(5.0),
        },
    );
    g
}

#[test]
fn definitions_and_instances_invert_exactly() {
    let mut g = setup();
    assert_eq!(out(&g, iface(10, D_OUT)), 5.0);
    assert_eq!(out(&g, iface(20, D_OUT)), 6.0);
    // Clearing an override restores the default; the inverse restores it.
    check(
        &mut g,
        &Command::ClearInput {
            node: iface(20, D_GAIN),
            port: INTERFACE_PORT.to_owned(),
        },
    );
    check(
        &mut g,
        &Command::SetExpression {
            node: iface(20, D_GAIN),
            port: INTERFACE_PORT.to_owned(),
            source: "ch(\"../a/gain/value\") * 3".to_owned(),
        },
    );
    // Deleting an instance restores its overrides and links.
    // `b` reads `a`, so `a` cannot go first.
    assert!(Command::DeleteNode { id: id(10) }.apply(&mut g).is_err());
    let done = check(&mut g, &Command::DeleteNode { id: id(20) });
    assert!(matches!(
        done.changes.last(),
        Some(DocChange::NodeRemoved { .. })
    ));
    check(&mut g, &Command::DeleteNode { id: id(10) });
    check(&mut g, &instantiate(20, "b"));
    // The rest of the commands.
    check(&mut g, &Command::Detach { instance: id(20) });
    check(&mut g, &Command::DeleteNode { id: id(20) });
    check(&mut g, &Command::RemoveDefinition { id: "d".to_owned() });
    let err = Command::RemoveDefinition {
        id: "zz".to_owned(),
    }
    .apply(&mut g)
    .unwrap_err();
    assert!(err.to_string().contains("no Charm definition"), "{err}");
}

#[test]
fn inner_nodes_are_refused() {
    let mut g = setup();
    let before = g.snapshot();
    for cmd in [
        Command::SetInput {
            node: iface(10, D_NUM),
            port: "a".to_owned(),
            value: Value::Float(1.0),
        },
        Command::DeleteNode {
            id: iface(10, D_NUM),
        },
        Command::Rename {
            node: iface(10, D_NUM),
            name: "n".to_owned(),
        },
        Command::AddInterfacePort {
            id: id(99),
            subnet: id(10),
            direction: PortDirection::Input,
            name: "p".to_owned(),
            port_type: PortType::Float,
        },
        Command::RemoveInterfacePort {
            subnet: id(10),
            direction: PortDirection::Input,
            name: "in".to_owned(),
        },
        Command::Promote {
            node: iface(10, D_NUM),
            port: "a".to_owned(),
            interface: id(99),
            name: "p".to_owned(),
        },
        Command::MakeDefinition {
            subnet: id(10),
            def: "e".to_owned(),
            version: "1".to_owned(),
        },
        Command::Attach {
            subnet: id(10),
            def: "d".to_owned(),
        },
    ] {
        assert!(cmd.apply(&mut g).is_err(), "{cmd:?}");
        assert_eq!(g.snapshot(), before);
    }
}

#[test]
fn updating_a_definition_inverts_exactly() {
    let mut g = setup();
    // `b.gain` is overridden to 5, the new default: the inverse must still
    // restore the override, not the old default.
    let done = check(
        &mut g,
        &Command::UpdateDefinition {
            def: definition("d", "2.0.0", 5.0, 1),
        },
    );
    assert!(
        done.changes
            .contains(&DocChange::DefinitionChanged { id: "d".to_owned() })
    );
    assert!(
        done.changes
            .contains(&DocChange::InstanceUpdated { instance: id(10) })
    );
    assert_eq!(out(&g, iface(10, D_OUT)), 3.0 + 5.0 + 6.0);
    assert_eq!(out(&g, iface(20, D_OUT)), 1.0 + 5.0 + 6.0);
    // Removing the `gain` port drops the link into it; the inverse
    // relinks.
    check(
        &mut g,
        &Command::Link {
            from: id(1),
            from_port: "out".to_owned(),
            to: iface(20, D_GAIN),
            to_port: INTERFACE_PORT.to_owned(),
        },
    );
    let done = check(
        &mut g,
        &Command::UpdateDefinition {
            def: definition("d", "3.0.0", 2.0, 2),
        },
    );
    assert!(
        done.changes
            .iter()
            .any(|c| matches!(c, DocChange::Unlinked(_)))
    );
}

#[test]
fn make_definition_inverts_to_the_original_ids() {
    let mut g = graph();
    let s = id(9);
    let steps = vec![
        Command::CreateNode {
            id: id(1),
            type_id: NUM.to_owned(),
            name: "x".to_owned(),
        },
        Command::CreateSubnet {
            id: s,
            parent: None,
            name: "s".to_owned(),
        },
        Command::AddInterfacePort {
            id: id(11),
            subnet: s,
            direction: PortDirection::Input,
            name: "in".to_owned(),
            port_type: PortType::Float,
        },
        Command::AddInterfacePort {
            id: id(12),
            subnet: s,
            direction: PortDirection::Output,
            name: "out".to_owned(),
            port_type: PortType::Float,
        },
        Command::CreateNodeIn {
            id: id(13),
            parent: Some(s),
            type_id: NUM.to_owned(),
            name: "n".to_owned(),
            port_type: None,
        },
        Command::Link {
            from: id(1),
            from_port: "out".to_owned(),
            to: id(11),
            to_port: INTERFACE_PORT.to_owned(),
        },
        Command::Link {
            from: id(11),
            from_port: INTERFACE_PORT.to_owned(),
            to: id(13),
            to_port: "a".to_owned(),
        },
        Command::Link {
            from: id(13),
            from_port: "out".to_owned(),
            to: id(12),
            to_port: INTERFACE_PORT.to_owned(),
        },
        Command::SetExpression {
            node: id(11),
            port: INTERFACE_PORT.to_owned(),
            source: "ch(\"../x/a\") + 1".to_owned(),
        },
        Command::CreateNode {
            id: id(2),
            type_id: NUM.to_owned(),
            name: "y".to_owned(),
        },
        Command::SetExpression {
            node: id(2),
            port: "b".to_owned(),
            source: "ch(\"s/n/b\") + 2".to_owned(),
        },
    ];
    Command::Transaction(steps).apply(&mut g).unwrap();
    let done = check(
        &mut g,
        &Command::MakeDefinition {
            subnet: s,
            def: "m".to_owned(),
            version: "1".to_owned(),
        },
    );
    assert!(!done.changes.is_empty());
    assert!(g.is_instance(s));
    assert_eq!(g.node_type(instance_node_id(s, id(11))), Some(INPUT_TYPE));
    assert_eq!(g.node_type(instance_node_id(s, id(12))), Some(OUTPUT_TYPE));
    assert_eq!(g.expression(id(2), "b").unwrap(), Some("ch(\"s/n/b\") + 2"));
}

#[test]
fn rewired_moves_invert_exactly() {
    let mut g = graph();
    Command::Transaction(vec![
        Command::CreateNode {
            id: id(1),
            type_id: NUM.to_owned(),
            name: "x".to_owned(),
        },
        Command::CreateNode {
            id: id(2),
            type_id: NUM.to_owned(),
            name: "y".to_owned(),
        },
        Command::CreateSubnet {
            id: id(9),
            parent: None,
            name: "s".to_owned(),
        },
        Command::Link {
            from: id(1),
            from_port: "out".to_owned(),
            to: id(2),
            to_port: "a".to_owned(),
        },
        Command::SetInput {
            node: id(1),
            port: "a".to_owned(),
            value: Value::Float(4.0),
        },
    ])
    .apply(&mut g)
    .unwrap();
    let plain = Command::MoveNode {
        node: id(2),
        parent: Some(id(9)),
    };
    assert!(plain.apply(&mut g).is_err());
    let done = check(
        &mut g,
        &Command::MoveNodeRewired {
            node: id(2),
            parent: Some(id(9)),
            seed: id(500),
        },
    );
    assert!(
        done.changes
            .iter()
            .any(|c| matches!(c, DocChange::NodeMoved { .. }))
    );
    assert_eq!(g.interface(id(9), PortDirection::Input).len(), 1);
    let y = Cooker::new().cook_output(&g, id(2), "out").unwrap();
    assert_eq!(y, Value::Float(4.0));
    // Moving with nothing to rewire is a plain move.
    check(
        &mut g,
        &Command::MoveNodeRewired {
            node: id(1),
            parent: Some(id(9)),
            seed: id(501),
        },
    );
}

#[test]
fn deleting_a_subnet_with_an_instance_inside_inverts_exactly() {
    let mut g = setup();
    check(
        &mut g,
        &Command::CreateSubnet {
            id: id(9),
            parent: None,
            name: "s".to_owned(),
        },
    );
    check(
        &mut g,
        &Command::MoveNode {
            node: id(20),
            parent: Some(id(9)),
        },
    );
    check(
        &mut g,
        &Command::SetExpression {
            node: iface(20, D_IN),
            port: INTERFACE_PORT.to_owned(),
            source: "ch(\"../../x/a\") + 1".to_owned(),
        },
    );
    check(&mut g, &Command::DeleteNode { id: id(9) });
    let err = Command::Instantiate {
        id: id(30),
        parent: Some(id(10)),
        name: "c".to_owned(),
        def: "d".to_owned(),
    }
    .apply(&mut g)
    .unwrap_err();
    assert!(matches!(
        err,
        livspeil_cmd::CmdError::Graph(GraphError::InInstance(_))
    ));
}
