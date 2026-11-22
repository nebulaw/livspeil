// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Property tests over random sequences of valid and invalid commands:
//!
//! 1. applying a command and then its inverse restores the snapshot;
//! 2. undoing everything and redoing everything round-trips;
//! 3. replaying the journal onto a fresh graph reproduces the state;
//! 4. a failed command leaves the snapshot unchanged.

mod common;

use std::io::Cursor;

use common::{FLAG, NUM, graph, id};
use livspeil_cmd::{Command, GroupId, History, JournalWriter, Origin, replay};
use livspeil_graph::{GraphSnapshot, NodeId};
use livspeil_node_api::Value;
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe};
use proptest::prelude::*;

// Small pools so that random commands often hit existing nodes, names and
// ports, and also often miss them. Ids 1, 2, 5, 6 are meant for `t.num`
// nodes and 3, 4, 7, 8 for `t.flag` nodes; see `seed`.
const NUM_IDS: [u128; 4] = [1, 2, 5, 6];
const FLAG_IDS: [u128; 4] = [3, 4, 7, 8];
const NAMES: [&str; 11] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", ""];
const INPUTS: [&str; 5] = ["a", "b", "flag", "count", "out"];
const OUTPUTS: [&str; 4] = ["out", "flag", "n", "a"];
const ORIGINS: [Origin; 5] = [
    Origin::Ui,
    Origin::Script,
    Origin::Ai,
    Origin::Import,
    Origin::Collab,
];

fn any_id() -> impl Strategy<Value = NodeId> {
    (1..=8u128).prop_map(id)
}

fn num_id() -> impl Strategy<Value = NodeId> {
    prop::sample::select(NUM_IDS.to_vec()).prop_map(id)
}

fn flag_id() -> impl Strategy<Value = NodeId> {
    prop::sample::select(FLAG_IDS.to_vec()).prop_map(id)
}

fn owned(names: &[&'static str]) -> impl Strategy<Value = String> + use<> {
    prop::sample::select(names.to_vec()).prop_map(str::to_owned)
}

fn value() -> impl Strategy<Value = Value> {
    prop_oneof![
        (-1.0e6..1.0e6f64).prop_map(Value::Float),
        (-100i64..100).prop_map(Value::Int),
        any::<bool>().prop_map(Value::Bool),
    ]
}

fn create() -> impl Strategy<Value = Command> {
    let make = |(id, type_id, name): (NodeId, &str, String)| Command::CreateNode {
        id,
        type_id: type_id.to_owned(),
        name,
    };
    prop_oneof![
        4 => (num_id(), Just(NUM), owned(&NAMES)).prop_map(make),
        4 => (flag_id(), Just(FLAG), owned(&NAMES)).prop_map(make),
        1 => (any_id(), prop::sample::select(vec![NUM, FLAG, "t.missing@1"]), owned(&NAMES))
            .prop_map(make),
    ]
}

fn set_input() -> impl Strategy<Value = Command> {
    let make =
        |(node, port, value): (NodeId, String, Value)| Command::SetInput { node, port, value };
    let number = prop_oneof![
        (-1.0e6..1.0e6f64).prop_map(Value::Float),
        (-100i64..100).prop_map(Value::Int),
    ];
    prop_oneof![
        3 => (num_id(), owned(&["a", "b"]), number).prop_map(make),
        1 => (flag_id(), Just("flag".to_owned()), any::<bool>().prop_map(Value::Bool))
            .prop_map(make),
        1 => (flag_id(), Just("count".to_owned()), (-100i64..100).prop_map(Value::Int))
            .prop_map(make),
        1 => (any_id(), owned(&INPUTS), value()).prop_map(make),
    ]
}

fn link() -> impl Strategy<Value = Command> {
    let make = |(from, from_port, to, to_port): (NodeId, String, NodeId, String)| Command::Link {
        from,
        from_port,
        to,
        to_port,
    };
    prop_oneof![
        3 => (num_id(), Just("out".to_owned()), num_id(), owned(&["a", "b"])).prop_map(make),
        1 => (flag_id(), Just("n".to_owned()), num_id(), owned(&["a", "b"])).prop_map(make),
        1 => (flag_id(), Just("flag".to_owned()), flag_id(), Just("flag".to_owned()))
            .prop_map(make),
        1 => (flag_id(), Just("n".to_owned()), flag_id(), Just("count".to_owned()))
            .prop_map(make),
        1 => (any_id(), owned(&OUTPUTS), any_id(), owned(&INPUTS)).prop_map(make),
    ]
}

/// Expression sources: valid ones reading `$T` or other nodes through
/// `ch()` (which may be unknown, mistyped or cyclic), plus broken ones.
fn source() -> impl Strategy<Value = String> {
    let template = prop::sample::select(vec![
        "$T * 2",
        "1 + 2",
        "ch(\"{n}/{p}\") + 1",
        "lerp(0.2, 1.0, ch(\"../{n}/{p}\"))",
        "ch(\"{n}/count\") * 2",
        "ch(\"{n}/flag\") ? 1 : 0",
        "ch(\"{n}/a\") + ch(\"{m}/b\")",
        "true",
        "1 +",
    ]);
    (
        template,
        owned(&NAMES[..10]),
        owned(&NAMES[..10]),
        owned(&INPUTS),
    )
        .prop_map(|(t, n, m, p)| t.replace("{n}", &n).replace("{m}", &m).replace("{p}", &p))
}

fn channel() -> impl Strategy<Value = AnimChannel> {
    let interp = prop::sample::select(vec![
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::Bezier,
        Interpolation::Spring {
            frequency: 1.5,
            damping: 0.5,
        },
    ]);
    let extra = prop::sample::select(vec![
        Extrapolation::Hold,
        Extrapolation::Loop,
        Extrapolation::PingPong,
    ]);
    prop_oneof![
        3 => (
            prop::collection::vec((-1.0e3..1.0e3f64, interp), 1..4),
            extra.clone(),
            extra,
            -10.0..10.0f64,
        )
            .prop_map(|(values, pre, post, offset)| {
                let keys = values
                    .into_iter()
                    .enumerate()
                    .map(|(i, (v, interp))| {
                        Keyframe::float(i as f64 * 0.75, v).with_interpolation(interp)
                    })
                    .collect();
                AnimChannel::new(keys)
                    .expect("valid keys")
                    .with_extrapolation(pre, post)
                    .with_offset(offset)
                    .expect("finite offset")
            }),
        1 => Just(
            AnimChannel::new(vec![Keyframe::new(0.0, Value::Vec2([1.0, 2.0]))])
                .expect("valid keys")
        ),
    ]
}

fn simple_command() -> impl Strategy<Value = Command> {
    prop_oneof![
        3 => create(),
        1 => any_id().prop_map(|id| Command::DeleteNode { id }),
        3 => set_input(),
        1 => (any_id(), owned(&INPUTS))
            .prop_map(|(node, port)| Command::ClearInput { node, port }),
        4 => link(),
        1 => (any_id(), owned(&INPUTS)).prop_map(|(to, to_port)| Command::Unlink { to, to_port }),
        1 => (any_id(), owned(&NAMES)).prop_map(|(node, name)| Command::Rename { node, name }),
        3 => (any_id(), owned(&INPUTS), source())
            .prop_map(|(node, port, source)| Command::SetExpression { node, port, source }),
        1 => (any_id(), owned(&INPUTS))
            .prop_map(|(node, port)| Command::ClearExpression { node, port }),
        2 => (any_id(), owned(&INPUTS), channel())
            .prop_map(|(node, port, channel)| Command::SetAnimation { node, port, channel }),
        1 => (any_id(), owned(&INPUTS))
            .prop_map(|(node, port)| Command::ClearAnimation { node, port }),
    ]
}

fn command() -> impl Strategy<Value = Command> {
    prop_oneof![
        4 => simple_command(),
        1 => prop::collection::vec(simple_command(), 0..5).prop_map(Command::Transaction),
        1 => prop::collection::vec(
            prop_oneof![
                3 => simple_command(),
                1 => prop::collection::vec(simple_command(), 0..3)
                    .prop_map(Command::Transaction),
            ],
            0..4,
        )
        .prop_map(Command::Transaction),
    ]
}

/// A command with its metadata. Groups come from a tiny pool so runs of
/// equal groups (coalescing) are common.
fn op() -> impl Strategy<Value = (Command, Origin, Option<GroupId>)> {
    (
        command(),
        prop::sample::select(ORIGINS.to_vec()),
        prop::option::of((0u64..2).prop_map(GroupId)),
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn commands_invert_undo_redo_and_replay(ops in prop::collection::vec(op(), 1..=30)) {
        let mut g = graph();
        let mut history = History::new();
        let mut journal = JournalWriter::new(Vec::new());
        let initial = g.snapshot();
        // Seed nodes so that random commands mostly address something real.
        let seed = history.apply(&mut g, seed(), Origin::Import, None)
            .map_err(|e| TestCaseError::fail(format!("seed failed: {e}")))?;
        journal.append(&seed.entry).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut applied = 1;

        for (cmd, origin, group) in ops {
            let before = g.snapshot();
            match cmd.apply(&mut g) {
                Err(_) => {
                    // (4) A failed command changes nothing, and fails the
                    // same way through the history.
                    prop_assert_eq!(&g.snapshot(), &before);
                    prop_assert!(history.apply(&mut g, cmd, origin, group).is_err());
                    prop_assert_eq!(&g.snapshot(), &before);
                }
                Ok(done) => {
                    // (1) The inverse restores the previous state.
                    let after = g.snapshot();
                    done.inverse.apply(&mut g).map_err(|e| {
                        TestCaseError::fail(format!("inverse failed: {e}"))
                    })?;
                    prop_assert_eq!(&g.snapshot(), &before);
                    // Re-apply through the history and journal it.
                    let record = history.apply(&mut g, cmd, origin, group).map_err(|e| {
                        TestCaseError::fail(format!("re-apply failed: {e}"))
                    })?;
                    prop_assert_eq!(&g.snapshot(), &after);
                    journal.append(&record.entry).map_err(|e| {
                        TestCaseError::fail(format!("journal append failed: {e}"))
                    })?;
                    applied += 1;
                }
            }
        }

        // (3) The journal replays onto a fresh graph.
        let final_state = g.snapshot();
        check_replay(journal.into_inner().as_slice(), &final_state, applied)?;

        // (2) Undo everything, then redo everything, journaling it all.
        let mut journal = JournalWriter::new(Vec::new());
        while history.can_undo() {
            let record = history.undo(&mut g).map_err(|e| {
                TestCaseError::fail(format!("undo failed: {e}"))
            })?;
            journal.append(&record.entry).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        prop_assert_eq!(&g.snapshot(), &initial);
        while history.can_redo() {
            let record = history.redo(&mut g).map_err(|e| {
                TestCaseError::fail(format!("redo failed: {e}"))
            })?;
            journal.append(&record.entry).map_err(|e| TestCaseError::fail(e.to_string()))?;
        }
        prop_assert_eq!(&g.snapshot(), &final_state);

        // The undo/redo entries start from the final state: replay them
        // after a transaction that rebuilds it, and they end there again.
        let mut replayed = graph();
        let mut prefix = JournalWriter::new(Vec::new());
        prefix.append(&livspeil_cmd::JournalEntry {
            kind: livspeil_cmd::EntryKind::Do,
            origin: Origin::Import,
            group: None,
            command: rebuild(&final_state),
        }).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut bytes = prefix.into_inner();
        bytes.extend(journal.into_inner());
        replay(Cursor::new(bytes), &mut replayed).map_err(|e| {
            TestCaseError::fail(format!("undo/redo replay failed: {e}"))
        })?;
        prop_assert_eq!(&replayed.snapshot(), &final_state);
    }
}

/// Nodes 1, 2, 5 (`t.num`) and 3, 4, 7 (`t.flag`) with one link; ids 6
/// and 8 stay free.
fn seed() -> Command {
    let create = |n: u128, type_id: &str, name: &str| Command::CreateNode {
        id: id(n),
        type_id: type_id.to_owned(),
        name: name.to_owned(),
    };
    Command::Transaction(vec![
        create(1, NUM, "a"),
        create(2, NUM, "b"),
        create(5, NUM, "e"),
        create(3, FLAG, "c"),
        create(4, FLAG, "d"),
        create(7, FLAG, "f"),
        Command::Link {
            from: id(1),
            from_port: "out".to_owned(),
            to: id(2),
            to_port: "a".to_owned(),
        },
    ])
}

fn check_replay(bytes: &[u8], expected: &GraphSnapshot, count: usize) -> Result<(), TestCaseError> {
    let mut fresh = graph();
    let replayed = replay(Cursor::new(bytes), &mut fresh)
        .map_err(|e| TestCaseError::fail(format!("replay failed: {e}")))?;
    prop_assert_eq!(replayed, count);
    prop_assert_eq!(&fresh.snapshot(), expected);
    Ok(())
}

/// A transaction that builds `snapshot` from an empty graph.
fn rebuild(snapshot: &GraphSnapshot) -> Command {
    let mut steps = Vec::new();
    for node in &snapshot.nodes {
        steps.push(Command::CreateNode {
            id: node.id,
            type_id: node.type_id.clone(),
            name: node.name.clone(),
        });
        for (port, value) in &node.inputs {
            steps.push(Command::SetInput {
                node: node.id,
                port: port.clone(),
                value: value.clone(),
            });
        }
    }
    for link in &snapshot.links {
        steps.push(Command::Link {
            from: link.from,
            from_port: link.from_port.clone(),
            to: link.to,
            to_port: link.to_port.clone(),
        });
    }
    // Sources last: ch() may read any node.
    for node in &snapshot.nodes {
        for (port, source) in &node.expressions {
            steps.push(Command::SetExpression {
                node: node.id,
                port: port.clone(),
                source: source.clone(),
            });
        }
        for (port, channel) in &node.animations {
            steps.push(Command::SetAnimation {
                node: node.id,
                port: port.clone(),
                channel: channel.clone(),
            });
        }
    }
    Command::Transaction(steps)
}
