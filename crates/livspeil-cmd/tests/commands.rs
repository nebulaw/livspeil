// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every command, its inverse, its changes and its failure modes.

mod common;

use common::{FLAG, NUM, graph, id};
use livspeil_cmd::{Applied, CmdError, Command, DocChange};
use livspeil_graph::{CookStats, Cooker, Graph, GraphError, NamedLink, NodeId};
use livspeil_node_api::{PortType, Value};

fn create(n: u128, type_id: &str, name: &str) -> Command {
    Command::CreateNode {
        id: id(n),
        type_id: type_id.to_owned(),
        name: name.to_owned(),
    }
}

fn set(node: NodeId, port: &str, value: Value) -> Command {
    Command::SetInput {
        node,
        port: port.to_owned(),
        value,
    }
}

fn link(from: NodeId, from_port: &str, to: NodeId, to_port: &str) -> Command {
    Command::Link {
        from,
        from_port: from_port.to_owned(),
        to,
        to_port: to_port.to_owned(),
    }
}

fn named(from: NodeId, from_port: &str, to: NodeId, to_port: &str) -> NamedLink {
    NamedLink {
        from,
        from_port: from_port.to_owned(),
        to,
        to_port: to_port.to_owned(),
    }
}

fn run(g: &mut Graph, cmd: Command) {
    cmd.apply(g).expect("setup command applies");
}

/// Apply `cmd`, then its inverse, then the inverse of that: the snapshots
/// must alternate between "after" and "before".
fn round_trip(g: &mut Graph, cmd: &Command) -> Applied {
    let before = g.snapshot();
    let applied = cmd.apply(g).expect("command applies");
    let after = g.snapshot();
    let redo = applied.inverse.apply(g).expect("inverse applies");
    assert_eq!(g.snapshot(), before, "inverse restores the state");
    redo.inverse.apply(g).expect("redo applies");
    assert_eq!(g.snapshot(), after, "inverse of the inverse redoes");
    applied
}

/// Three nodes: a -> b.a, a -> c.a, b -> c.b; a.a = 1, c.b set too.
fn triangle() -> Graph {
    let mut g = graph();
    run(&mut g, create(1, NUM, "a"));
    run(&mut g, create(2, NUM, "b"));
    run(&mut g, create(3, NUM, "c"));
    run(&mut g, set(id(1), "a", Value::Float(1.0)));
    run(&mut g, set(id(3), "b", Value::Float(5.0)));
    run(&mut g, link(id(1), "out", id(2), "a"));
    run(&mut g, link(id(1), "out", id(3), "a"));
    run(&mut g, link(id(2), "out", id(3), "b"));
    g
}

#[test]
fn create_node_and_its_inverse() {
    let mut g = graph();
    let applied = round_trip(&mut g, &create(1, NUM, "a"));
    assert_eq!(applied.inverse, Command::DeleteNode { id: id(1) });
    assert_eq!(applied.changes, vec![DocChange::NodeAdded { id: id(1) }]);
    assert_eq!(g.node_name(id(1)), Some("a"));
    assert_eq!(g.node_type(id(1)), Some(NUM));
}

#[test]
fn create_node_rejects_bad_input() {
    let mut g = graph();
    run(&mut g, create(1, NUM, "a"));
    let cases = [
        (
            create(2, "t.nope@1", "x"),
            GraphError::UnknownNodeType("t.nope@1".into()),
        ),
        (create(2, NUM, ""), GraphError::EmptyName),
        (create(2, NUM, "a"), GraphError::DuplicateName("a".into())),
        (create(1, NUM, "z"), GraphError::DuplicateId(id(1))),
    ];
    for (cmd, err) in cases {
        let before = g.snapshot();
        assert_eq!(cmd.apply(&mut g).unwrap_err(), CmdError::Graph(err));
        assert_eq!(g.snapshot(), before);
    }
}

#[test]
fn create_node_helper_uses_fresh_ids() {
    let (Command::CreateNode { id: a, .. }, Command::CreateNode { id: b, .. }) = (
        Command::create_node(NUM, "a"),
        Command::create_node(NUM, "a"),
    ) else {
        panic!("create_node builds CreateNode");
    };
    assert_ne!(a, b);
}

#[test]
fn delete_node_restores_name_type_values_and_every_link() {
    let mut g = triangle();
    let before = g.snapshot();
    // b has an incoming link from a and an outgoing link to c.
    let applied = round_trip(&mut g, &Command::DeleteNode { id: id(2) });
    assert_eq!(
        applied.changes,
        vec![
            DocChange::Unlinked(named(id(1), "out", id(2), "a")),
            DocChange::Unlinked(named(id(2), "out", id(3), "b")),
            DocChange::NodeRemoved { id: id(2) },
        ]
    );
    // a has explicit values and two outgoing links.
    let applied = round_trip(&mut g, &Command::DeleteNode { id: id(1) });
    let Command::Transaction(steps) = &applied.inverse else {
        panic!("delete restores through a transaction");
    };
    assert_eq!(steps[0], create(1, NUM, "a"));
    assert!(steps.contains(&set(id(1), "a", Value::Float(1.0))));
    // Undo both deletions in reverse order and we are back at the start.
    g = triangle();
    let del_b = (Command::DeleteNode { id: id(2) }).apply(&mut g).unwrap();
    let del_a = (Command::DeleteNode { id: id(1) }).apply(&mut g).unwrap();
    assert_eq!(g.node_count(), 1);
    assert_eq!(g.link_count(), 0);
    del_a.inverse.apply(&mut g).unwrap();
    del_b.inverse.apply(&mut g).unwrap();
    assert_eq!(g.snapshot(), before);
}

#[test]
fn delete_unknown_node_fails() {
    let mut g = triangle();
    let before = g.snapshot();
    assert_eq!(
        (Command::DeleteNode { id: id(9) })
            .apply(&mut g)
            .unwrap_err(),
        CmdError::Graph(GraphError::UnknownNode(id(9)))
    );
    assert_eq!(g.snapshot(), before);
}

#[test]
fn set_input_and_its_inverse() {
    let mut g = triangle();
    // Overwriting an explicit value restores the old value.
    let applied = round_trip(&mut g, &set(id(1), "a", Value::Float(2.0)));
    assert_eq!(applied.inverse, set(id(1), "a", Value::Float(1.0)));
    assert_eq!(
        applied.changes,
        vec![DocChange::InputChanged {
            node: id(1),
            port: "a".into()
        }]
    );
    // Setting a previously unset input is undone by clearing it.
    let applied = round_trip(&mut g, &set(id(1), "b", Value::Int(4)));
    assert_eq!(
        applied.inverse,
        Command::ClearInput {
            node: id(1),
            port: "b".into()
        }
    );
    // Int was coerced to Float on a Float port.
    assert_eq!(g.input_value(id(1), "b").unwrap(), Some(&Value::Float(4.0)));
}

#[test]
fn set_input_rejects_bad_ports_and_types() {
    let mut g = triangle();
    let before = g.snapshot();
    let err = set(id(1), "a", Value::Bool(true))
        .apply(&mut g)
        .unwrap_err();
    assert_eq!(
        err,
        CmdError::Graph(GraphError::TypeMismatch {
            expected: PortType::Float,
            found: PortType::Bool
        })
    );
    assert!(set(id(1), "out", Value::Float(1.0)).apply(&mut g).is_err());
    assert!(set(id(9), "a", Value::Float(1.0)).apply(&mut g).is_err());
    assert_eq!(g.snapshot(), before);
}

#[test]
fn clear_input_and_its_inverse() {
    let mut g = triangle();
    let clear = Command::ClearInput {
        node: id(1),
        port: "a".into(),
    };
    let applied = round_trip(&mut g, &clear);
    assert_eq!(applied.inverse, set(id(1), "a", Value::Float(1.0)));
    assert_eq!(applied.changes.len(), 1);
    // Clearing an unset input changes nothing and reports nothing.
    let unset = Command::ClearInput {
        node: id(2),
        port: "b".into(),
    };
    let applied = round_trip(&mut g, &unset);
    assert!(applied.changes.is_empty());
    assert_eq!(applied.inverse, unset);
    assert!(
        (Command::ClearInput {
            node: id(2),
            port: "zzz".into()
        })
        .apply(&mut g)
        .is_err()
    );
}

#[test]
fn link_and_its_inverse() {
    let mut g = graph();
    run(&mut g, create(1, NUM, "a"));
    run(&mut g, create(2, NUM, "b"));
    let applied = round_trip(&mut g, &link(id(1), "out", id(2), "b"));
    assert_eq!(
        applied.inverse,
        Command::Unlink {
            to: id(2),
            to_port: "b".into()
        }
    );
    assert_eq!(
        applied.changes,
        vec![DocChange::Linked(named(id(1), "out", id(2), "b"))]
    );
}

#[test]
fn link_rejects_bad_links() {
    let mut g = triangle();
    run(&mut g, create(4, FLAG, "f"));
    let before = g.snapshot();
    let bad = [
        link(id(3), "out", id(1), "b"),  // cycle
        link(id(1), "out", id(1), "b"),  // self link
        link(id(1), "out", id(2), "a"),  // occupied
        link(id(4), "flag", id(1), "b"), // Bool into Float
        link(id(1), "nope", id(4), "count"),
        link(id(9), "out", id(1), "b"),
    ];
    for cmd in bad {
        assert!(cmd.apply(&mut g).is_err(), "{cmd:?} must fail");
        assert_eq!(g.snapshot(), before);
    }
    // Int -> Float is allowed.
    round_trip(&mut g, &link(id(4), "n", id(1), "b"));
}

#[test]
fn unlink_and_its_inverse() {
    let mut g = triangle();
    let unlink = Command::Unlink {
        to: id(3),
        to_port: "b".into(),
    };
    let applied = round_trip(&mut g, &unlink);
    assert_eq!(applied.inverse, link(id(2), "out", id(3), "b"));
    assert_eq!(
        applied.changes,
        vec![DocChange::Unlinked(named(id(2), "out", id(3), "b"))]
    );
    // Unlinking an unlinked input is a no-op whose inverse is a no-op.
    let free = Command::Unlink {
        to: id(1),
        to_port: "a".into(),
    };
    let applied = round_trip(&mut g, &free);
    assert_eq!(applied.inverse, Command::noop());
    assert!(applied.changes.is_empty());
    assert!(
        (Command::Unlink {
            to: id(1),
            to_port: "out".into()
        })
        .apply(&mut g)
        .is_err()
    );
}

#[test]
fn rename_and_its_inverse() {
    let mut g = triangle();
    let rename = Command::Rename {
        node: id(1),
        name: "first".into(),
    };
    let applied = round_trip(&mut g, &rename);
    assert_eq!(
        applied.inverse,
        Command::Rename {
            node: id(1),
            name: "a".into()
        }
    );
    assert_eq!(
        applied.changes,
        vec![DocChange::Renamed {
            node: id(1),
            old: "a".into(),
            new: "first".into()
        }]
    );
    for name in ["", "b"] {
        let bad = Command::Rename {
            node: id(1),
            name: name.into(),
        };
        let before = g.snapshot();
        assert!(bad.apply(&mut g).is_err());
        assert_eq!(g.snapshot(), before);
    }
}

#[test]
fn rename_through_a_command_keeps_the_cook_cache() {
    let mut g = triangle();
    let mut cooker = Cooker::new();
    cooker.cook(&g, id(3)).unwrap();
    cooker.reset_stats();
    let applied = (Command::Rename {
        node: id(1),
        name: "renamed".into(),
    })
    .apply(&mut g)
    .unwrap();
    cooker.cook(&g, id(3)).unwrap();
    assert_eq!(
        cooker.stats(),
        CookStats {
            cooked: 0,
            reused: 3
        }
    );
    applied.inverse.apply(&mut g).unwrap();
    cooker.cook(&g, id(3)).unwrap();
    assert_eq!(
        cooker.stats(),
        CookStats {
            cooked: 0,
            reused: 6
        }
    );
}

#[test]
fn transaction_applies_in_order_and_inverts_in_reverse() {
    let mut g = graph();
    let tx = Command::Transaction(vec![
        create(1, NUM, "a"),
        create(2, NUM, "b"),
        link(id(1), "out", id(2), "a"),
        set(id(1), "b", Value::Float(3.0)),
    ]);
    let applied = round_trip(&mut g, &tx);
    assert_eq!(applied.changes.len(), 4);
    let Command::Transaction(steps) = applied.inverse else {
        panic!("inverse of a transaction is a transaction");
    };
    assert_eq!(
        steps,
        vec![
            Command::ClearInput {
                node: id(1),
                port: "b".into()
            },
            Command::Unlink {
                to: id(2),
                to_port: "a".into()
            },
            Command::DeleteNode { id: id(2) },
            Command::DeleteNode { id: id(1) },
        ]
    );
}

#[test]
fn failed_transaction_rolls_back_every_step() {
    let mut g = triangle();
    let before = g.snapshot();
    let tx = Command::Transaction(vec![
        Command::DeleteNode { id: id(2) },
        Command::Rename {
            node: id(1),
            name: "b".into(), // free now that b is gone
        },
        set(id(3), "a", Value::Float(7.0)),
        Command::Transaction(vec![create(5, NUM, "e"), create(6, NUM, "e")]),
    ]);
    let err = tx.apply(&mut g).unwrap_err();
    assert_eq!(
        err,
        CmdError::InTransaction {
            index: 3,
            source: Box::new(CmdError::InTransaction {
                index: 1,
                source: Box::new(CmdError::Graph(GraphError::DuplicateName("e".into())))
            })
        }
    );
    assert_eq!(g.snapshot(), before);
    assert!(err.to_string().contains("step 3"));
}

#[test]
fn empty_transaction_is_a_noop() {
    let mut g = triangle();
    let applied = round_trip(&mut g, &Command::noop());
    assert_eq!(applied.inverse, Command::noop());
    assert!(applied.changes.is_empty());
}
