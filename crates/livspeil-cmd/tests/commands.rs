// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Every command, its inverse, its changes and its failure modes.

mod common;

use common::{FLAG, NUM, graph, id};
use livspeil_cmd::{Applied, CmdError, Command, DocChange};
use livspeil_graph::{CookStats, Cooker, Graph, GraphError, NamedLink, NodeId};
use livspeil_node_api::{PortType, Value};
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe, Tangent};

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

fn set_expr(node: NodeId, port: &str, source: &str) -> Command {
    Command::SetExpression {
        node,
        port: port.to_owned(),
        source: source.to_owned(),
    }
}

fn channel() -> AnimChannel {
    AnimChannel::new(vec![
        Keyframe::float(0.0, 1.0).with_interpolation(Interpolation::Spring {
            frequency: 2.0,
            damping: 0.4,
        }),
        Keyframe::float(1.0, 2.0)
            .with_interpolation(Interpolation::Bezier)
            .with_tangent(Tangent::Explicit {
                incoming: Value::Float(0.5),
                outgoing: Value::Float(-0.25),
            }),
        Keyframe::float(2.5, 0.1),
    ])
    .unwrap()
    .with_extrapolation(Extrapolation::Loop, Extrapolation::PingPong)
    .with_offset(0.125)
    .unwrap()
}

#[test]
fn expressions_set_clear_and_invert() {
    let mut g = graph();
    run(&mut g, create(1, NUM, "a"));
    run(&mut g, create(2, NUM, "b"));
    let applied = round_trip(&mut g, &set_expr(id(2), "a", r#"ch("a/b") * 2"#));
    assert_eq!(
        applied.changes,
        [DocChange::ExpressionChanged {
            node: id(2),
            port: "a".into()
        }]
    );
    assert_eq!(
        applied.inverse,
        Command::ClearExpression {
            node: id(2),
            port: "a".into()
        }
    );
    // Replacing restores the previous source.
    let applied = round_trip(&mut g, &set_expr(id(2), "a", "$T"));
    assert_eq!(applied.inverse, set_expr(id(2), "a", r#"ch("a/b") * 2"#));
    let clear = Command::ClearExpression {
        node: id(2),
        port: "a".into(),
    };
    let applied = round_trip(&mut g, &clear);
    assert_eq!(applied.inverse, set_expr(id(2), "a", "$T"));
    // Clearing nothing is a no-op without changes.
    run(&mut g, clear.clone());
    let applied = clear.apply(&mut g).unwrap();
    assert!(applied.changes.is_empty());
    // Errors leave the graph unchanged.
    let before = g.snapshot();
    assert!(matches!(
        set_expr(id(2), "a", "1 +").apply(&mut g),
        Err(CmdError::Graph(GraphError::Expression(_)))
    ));
    assert_eq!(
        set_expr(id(1), "a", r#"ch("a/b")"#).apply(&mut g),
        Err(CmdError::Graph(GraphError::WouldCycle))
    );
    assert_eq!(g.snapshot(), before);
}

#[test]
fn animations_set_clear_and_invert() {
    let mut g = graph();
    run(&mut g, create(1, NUM, "a"));
    let set = Command::SetAnimation {
        node: id(1),
        port: "b".into(),
        channel: channel(),
    };
    let applied = round_trip(&mut g, &set);
    assert_eq!(
        applied.changes,
        [DocChange::AnimationChanged {
            node: id(1),
            port: "b".into()
        }]
    );
    run(&mut g, set);
    let replace = Command::SetAnimation {
        node: id(1),
        port: "b".into(),
        channel: AnimChannel::constant(3.0).unwrap(),
    };
    let applied = round_trip(&mut g, &replace);
    assert_eq!(
        applied.inverse,
        Command::SetAnimation {
            node: id(1),
            port: "b".into(),
            channel: channel()
        }
    );
    round_trip(
        &mut g,
        &Command::ClearAnimation {
            node: id(1),
            port: "b".into(),
        },
    );
    run(&mut g, create(3, FLAG, "f"));
    assert_eq!(
        Command::SetAnimation {
            node: id(3),
            port: "count".into(),
            channel: channel()
        }
        .apply(&mut g),
        Err(CmdError::Graph(GraphError::TypeMismatch {
            expected: PortType::Int,
            found: PortType::Float
        }))
    );
}

#[test]
fn delete_restores_sources_and_refuses_referenced_nodes() {
    let mut g = graph();
    run(&mut g, create(1, NUM, "sun"));
    run(&mut g, create(2, NUM, "lamp"));
    run(&mut g, set_expr(id(1), "a", "$T / 4"));
    run(
        &mut g,
        Command::SetAnimation {
            node: id(1),
            port: "b".into(),
            channel: channel(),
        },
    );
    run(
        &mut g,
        set_expr(id(2), "a", r#"lerp(0.2, 1.0, ch("sun/a"))"#),
    );
    assert_eq!(
        Command::DeleteNode { id: id(1) }.apply(&mut g),
        Err(CmdError::Graph(GraphError::Referenced {
            node: id(1),
            by: id(2),
            port: "a".into()
        }))
    );
    run(
        &mut g,
        Command::ClearExpression {
            node: id(2),
            port: "a".into(),
        },
    );
    round_trip(&mut g, &Command::DeleteNode { id: id(1) });
}

#[test]
fn rename_rewrites_readers_and_inverts_exactly() {
    let mut g = graph();
    run(&mut g, create(1, NUM, "sun"));
    run(&mut g, create(2, NUM, "lamp"));
    run(
        &mut g,
        set_expr(id(2), "b", r#"ch("sun/a") + ch("../sun/b")"#),
    );
    let applied = round_trip(
        &mut g,
        &Command::Rename {
            node: id(1),
            name: "star".into(),
        },
    );
    assert_eq!(
        applied.changes,
        [
            DocChange::Renamed {
                node: id(1),
                old: "sun".into(),
                new: "star".into()
            },
            DocChange::ExpressionChanged {
                node: id(2),
                port: "b".into()
            }
        ]
    );
    assert_eq!(
        g.expression(id(2), "b").unwrap(),
        Some(r#"ch("star/a") + ch("../star/b")"#)
    );
}

#[test]
fn source_commands_round_trip_through_json() {
    let commands = [
        set_expr(id(1), "a", r#"lerp(0.2, 1.0, ch("sun/phase")) + $T"#),
        Command::ClearExpression {
            node: id(1),
            port: "a".into(),
        },
        Command::SetAnimation {
            node: id(1),
            port: "b".into(),
            channel: channel(),
        },
        Command::ClearAnimation {
            node: id(1),
            port: "b".into(),
        },
    ];
    for cmd in commands {
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(back, cmd, "{json}");
    }
    let json = serde_json::to_string(&set_expr(id(1), "a", "1")).unwrap();
    assert!(json.starts_with(r#"{"set_expression":{"node":"#), "{json}");

    // Deserializing validates the channel.
    let bad = format!(
        r#"{{"set_animation":{{"node":"{}","port":"a","channel":{{"keys":[{{"time":1.0,"value":{{"Float":0.0}}}},{{"time":0.5,"value":{{"Float":1.0}}}}]}}}}}}"#,
        id(1)
    );
    let err = serde_json::from_str::<Command>(&bad).unwrap_err();
    assert!(err.to_string().contains("strictly increasing"), "{err}");
    // Optional fields default.
    let ok = bad.replace("0.5", "2.0");
    let Command::SetAnimation { channel, .. } = serde_json::from_str(&ok).unwrap() else {
        panic!("expected SetAnimation");
    };
    assert_eq!(channel.pre(), Extrapolation::Hold);
    assert_eq!(channel.keys()[0].interpolation, Interpolation::Linear);
}
