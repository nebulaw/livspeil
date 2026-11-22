// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Subnet commands: create and delete subnets, interface ports, moves,
//! promotion and Charm metadata, with exact inverses and journaling.

mod common;

use std::io::Cursor;

use common::{NUM, graph, id};
use livspeil_cmd::{Command, DocChange, History, JournalWriter, Origin, replay};
use livspeil_graph::{CharmMeta, Graph, INTERFACE_PORT, PortDirection};
use livspeil_node_api::{PortType, Value};

/// Apply, check the inverse restores the state exactly, re-apply.
fn round_trip(g: &mut Graph, cmd: &Command) -> Vec<DocChange> {
    let before = g.snapshot();
    let done = cmd.apply(g).unwrap();
    let after = g.snapshot();
    done.inverse.apply(g).unwrap();
    assert_eq!(g.snapshot(), before, "inverse of {cmd:?}");
    cmd.apply(g).unwrap();
    assert_eq!(g.snapshot(), after);
    done.changes
}

fn node_in(n: u128, parent: Option<u128>, name: &str) -> Command {
    Command::CreateNodeIn {
        id: id(n),
        parent: parent.map(id),
        type_id: NUM.to_owned(),
        name: name.to_owned(),
        port_type: None,
    }
}

/// `/s` with interface input `in` (11) feeding `/s/inner` (2), whose
/// output goes to interface output `out` (12), read by `/outer` (3),
/// whose input `/other` (4) reads through `ch()`.
fn scene(g: &mut Graph) {
    let steps = vec![
        Command::CreateSubnet {
            id: id(1),
            parent: None,
            name: "s".to_owned(),
        },
        node_in(2, Some(1), "inner"),
        node_in(3, None, "outer"),
        node_in(4, None, "other"),
        Command::AddInterfacePort {
            id: id(11),
            subnet: id(1),
            direction: PortDirection::Input,
            name: "in".to_owned(),
            port_type: PortType::Float,
        },
        Command::AddInterfacePort {
            id: id(12),
            subnet: id(1),
            direction: PortDirection::Output,
            name: "out".to_owned(),
            port_type: PortType::Float,
        },
        Command::Link {
            from: id(11),
            from_port: INTERFACE_PORT.to_owned(),
            to: id(2),
            to_port: "a".to_owned(),
        },
        Command::Link {
            from: id(2),
            from_port: "out".to_owned(),
            to: id(12),
            to_port: INTERFACE_PORT.to_owned(),
        },
        Command::Link {
            from: id(12),
            from_port: INTERFACE_PORT.to_owned(),
            to: id(3),
            to_port: "a".to_owned(),
        },
        Command::SetExpression {
            node: id(4),
            port: "b".to_owned(),
            source: r#"ch("outer/a") + 1"#.to_owned(),
        },
        Command::SetExpression {
            node: id(2),
            port: "b".to_owned(),
            source: r#"ch("in/value") * 2"#.to_owned(),
        },
    ];
    Command::Transaction(steps).apply(g).unwrap();
}

#[test]
fn deleting_a_subnet_deletes_its_contents_and_undoes_exactly() {
    let mut g = graph();
    scene(&mut g);
    let mut charm = CharmMeta::new("Doubler");
    charm.capabilities = vec!["net".to_owned()];
    Command::SetCharm {
        subnet: id(1),
        charm: Some(charm),
    }
    .apply(&mut g)
    .unwrap();
    let changes = round_trip(&mut g, &Command::DeleteNode { id: id(1) });
    assert!(changes.contains(&DocChange::NodeRemoved { id: id(1) }));
    assert!(changes.contains(&DocChange::NodeRemoved { id: id(2) }));
    assert_eq!(g.node_count(), 2);
}

#[test]
fn deleting_a_subnet_read_from_outside_is_refused() {
    let mut g = graph();
    scene(&mut g);
    Command::SetExpression {
        node: id(3),
        port: "b".to_owned(),
        source: r#"ch("s/inner/a")"#.to_owned(),
    }
    .apply(&mut g)
    .unwrap();
    let before = g.snapshot();
    assert!(Command::DeleteNode { id: id(1) }.apply(&mut g).is_err());
    assert_eq!(g.snapshot(), before);
}

#[test]
fn interface_ports_add_and_remove() {
    let mut g = graph();
    scene(&mut g);
    round_trip(
        &mut g,
        &Command::RemoveInterfacePort {
            subnet: id(1),
            direction: PortDirection::Output,
            name: "out".to_owned(),
        },
    );
    assert!(
        Command::RemoveInterfacePort {
            subnet: id(1),
            direction: PortDirection::Input,
            name: "out".to_owned(),
        }
        .apply(&mut g)
        .is_err()
    );
    round_trip(
        &mut g,
        &Command::AddInterfacePort {
            id: id(13),
            subnet: id(1),
            direction: PortDirection::Output,
            name: "color".to_owned(),
            port_type: PortType::Color,
        },
    );
    assert_eq!(g.port_type(id(13)), Some(PortType::Color));
    assert!(
        Command::AddInterfacePort {
            id: id(14),
            subnet: id(3),
            direction: PortDirection::Input,
            name: "x".to_owned(),
            port_type: PortType::Float,
        }
        .apply(&mut g)
        .is_err()
    );
}

#[test]
fn moves_restore_expression_text_on_undo() {
    let mut g = graph();
    scene(&mut g);
    Command::Unlink {
        to: id(3),
        to_port: "a".to_owned(),
    }
    .apply(&mut g)
    .unwrap();
    // `other` reads `outer` by a sibling path; move `outer` inside.
    let changes = round_trip(
        &mut g,
        &Command::MoveNode {
            node: id(3),
            parent: Some(id(1)),
        },
    );
    assert!(changes.contains(&DocChange::NodeMoved {
        node: id(3),
        from: None,
        to: Some(id(1)),
    }));
    assert_eq!(g.path(id(3)).as_deref(), Some("/s/outer"));
    assert_eq!(
        g.expression(id(4), "b").unwrap(),
        Some(r#"ch("s/outer/a") + 1"#)
    );
    // A node read with a `../` path from the root moves and back, and the
    // undo restores the original text exactly.
    let mut g = graph();
    Command::Transaction(vec![
        Command::CreateSubnet {
            id: id(1),
            parent: None,
            name: "s".to_owned(),
        },
        node_in(2, None, "a"),
        node_in(3, None, "b"),
        Command::SetExpression {
            node: id(3),
            port: "a".to_owned(),
            source: r#"ch("../a/a")"#.to_owned(),
        },
    ])
    .apply(&mut g)
    .unwrap();
    let mut history = History::new();
    history
        .apply(
            &mut g,
            Command::MoveNode {
                node: id(3),
                parent: Some(id(1)),
            },
            Origin::Ui,
            None,
        )
        .unwrap();
    history
        .apply(
            &mut g,
            Command::MoveNode {
                node: id(3),
                parent: None,
            },
            Origin::Ui,
            None,
        )
        .unwrap();
    assert_eq!(g.expression(id(3), "a").unwrap(), Some(r#"ch("../a/a")"#));
    history.undo(&mut g).unwrap();
    history.undo(&mut g).unwrap();
    assert_eq!(g.expression(id(3), "a").unwrap(), Some(r#"ch("../a/a")"#));
    assert_eq!(g.parent(id(3)), None);
}

#[test]
fn promote_and_unpromote_round_trip_and_journal() {
    let mut g = graph();
    scene(&mut g);
    Command::SetInput {
        node: id(2),
        port: "b".to_owned(),
        value: Value::Float(4.0),
    }
    .apply(&mut g)
    .unwrap();
    Command::ClearExpression {
        node: id(2),
        port: "b".to_owned(),
    }
    .apply(&mut g)
    .unwrap();
    let initial = g.snapshot();
    let mut history = History::new();
    let mut journal = JournalWriter::new(Vec::new());
    for cmd in [
        Command::Promote {
            node: id(2),
            port: "b".to_owned(),
            interface: id(20),
            name: "gain".to_owned(),
        },
        Command::SetInput {
            node: id(20),
            port: INTERFACE_PORT.to_owned(),
            value: Value::Float(5.0),
        },
        Command::Unpromote {
            node: id(2),
            port: "b".to_owned(),
        },
    ] {
        let before = g.snapshot();
        let record = history.apply(&mut g, cmd, Origin::Ui, None).unwrap();
        journal.append(&record.entry).unwrap();
        let after = g.snapshot();
        history.undo(&mut g).unwrap();
        assert_eq!(g.snapshot(), before);
        history.redo(&mut g).unwrap();
        assert_eq!(g.snapshot(), after);
    }
    assert!(!g.contains(id(20)));
    assert_eq!(g.input_value(id(2), "b").unwrap(), Some(&Value::Float(5.0)));
    // Unpromoting a port not fed by an interface input is a no-op.
    let done = Command::Unpromote {
        node: id(3),
        port: "a".to_owned(),
    }
    .apply(&mut g)
    .unwrap();
    assert!(done.changes.is_empty());
    let final_state = g.snapshot();
    let mut fresh = graph();
    scene(&mut fresh);
    Command::SetInput {
        node: id(2),
        port: "b".to_owned(),
        value: Value::Float(4.0),
    }
    .apply(&mut fresh)
    .unwrap();
    Command::ClearExpression {
        node: id(2),
        port: "b".to_owned(),
    }
    .apply(&mut fresh)
    .unwrap();
    replay(Cursor::new(journal.into_inner()), &mut fresh).unwrap();
    assert_eq!(fresh.snapshot(), final_state);
    while history.can_undo() {
        history.undo(&mut g).unwrap();
    }
    assert_eq!(g.snapshot(), initial);
}
