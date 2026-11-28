// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Property tests over random sequences of valid and invalid commands:
//!
//! 1. applying a command and then its inverse restores the snapshot;
//! 2. undoing everything and redoing everything round-trips;
//! 3. replaying the journal onto a fresh graph reproduces the state;
//! 4. a failed command leaves the snapshot unchanged.
//!
//! The commands include subnets (ids 9 and 10), interface ports (ids 11
//! to 14), moves between networks, promotion and Charm metadata, and
//! (M0.16) Charm definitions (`d`, `e`), instances (ids 15 and 16), edits
//! on their contents (mostly refused), definition updates, detaching,
//! making definitions from subnets and rewired moves.

mod common;

use std::io::Cursor;

use common::{D_GAIN, D_IN, D_NUM, D_OUT, FLAG, NUM, definition, graph, id};
use livspeil_cmd::{Command, GroupId, History, JournalWriter, Origin, replay};
use livspeil_graph::{
    CharmDef, CharmMeta, GraphSnapshot, INPUT_TYPE, MAX_POSITION, NodeId, PortDirection,
    instance_node_id,
};
use livspeil_node_api::{PortType, Value};
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe};
use proptest::prelude::*;

// Small pools so that random commands often hit existing nodes, names and
// ports, and also often miss them. Ids 1, 2, 5, 6 are meant for `t.num`
// nodes and 3, 4, 7, 8 for `t.flag` nodes; see `seed`.
const NUM_IDS: [u128; 4] = [1, 2, 5, 6];
const FLAG_IDS: [u128; 4] = [3, 4, 7, 8];
const NAMES: [&str; 11] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", ""];
const SUBNET_IDS: [u128; 2] = [9, 10];
const PORT_IDS: [u128; 4] = [11, 12, 13, 14];
const INPUTS: [&str; 6] = ["a", "b", "flag", "count", "out", "value"];
const OUTPUTS: [&str; 5] = ["out", "flag", "n", "a", "value"];
const ORIGINS: [Origin; 5] = [
    Origin::Ui,
    Origin::Script,
    Origin::Ai,
    Origin::Import,
    Origin::Collab,
];

const INSTANCE_IDS: [u128; 2] = [15, 16];
const LOCALS: [u128; 4] = [D_IN, D_GAIN, D_NUM, D_OUT];

fn any_id() -> impl Strategy<Value = NodeId> {
    prop_oneof![
        6 => (1..=16u128).prop_map(id),
        1 => inner_id(),
    ]
}

fn instance_id() -> impl Strategy<Value = NodeId> {
    prop::sample::select(INSTANCE_IDS.to_vec()).prop_map(id)
}

/// A node inside instance 15 or 16 (or inside subnet 9 after it became
/// an instance), by definition-local id.
fn inner_id() -> impl Strategy<Value = NodeId> {
    (
        prop::sample::select(vec![15u128, 16, 9]),
        prop::sample::select(LOCALS.to_vec()),
    )
        .prop_map(|(i, l)| instance_node_id(id(i), id(l)))
}

/// Definitions `d` (four versions) and `e`.
fn def() -> impl Strategy<Value = CharmDef> {
    prop_oneof![
        3 => (0u8..3, 1.0..4.0f64).prop_map(|(variant, gain)| {
            definition("d", &format!("{variant}.0"), gain.round(), variant)
        }),
        1 => Just(definition("e", "1", 2.0, 0)),
    ]
}

/// Commands on Charm definitions and instances (M0.16).
fn charms() -> impl Strategy<Value = Command> {
    let def_id = || {
        prop_oneof![
            4 => Just("d".to_owned()),
            1 => owned(&["e", "nope"]),
        ]
    };
    prop_oneof![
        2 => def().prop_map(|def| Command::AddDefinition { def }),
        1 => def_id().prop_map(|id| Command::RemoveDefinition { id }),
        3 => def().prop_map(|def| Command::UpdateDefinition { def }),
        6 => (instance_id(), network(), owned(&NAMES), def_id()).prop_map(
            |(id, parent, name, def)| Command::Instantiate { id, parent, name, def }
        ),
        2 => instance_id().prop_map(|instance| Command::Detach { instance }),
        2 => (prop_oneof![instance_id(), subnet_id()], def_id())
            .prop_map(|(subnet, def)| Command::Attach { subnet, def }),
        2 => (subnet_id(), owned(&["m", "d"]))
            .prop_map(|(subnet, def)| Command::MakeDefinition {
                subnet,
                def,
                version: "1".to_owned(),
            }),
        3 => (any_id(), network(), (100..104u128).prop_map(id))
            .prop_map(|(node, parent, seed)| Command::MoveNodeRewired { node, parent, seed }),
        2 => (num_id(), network(), (100..104u128).prop_map(id))
            .prop_map(|(node, parent, seed)| Command::MoveNodeRewired { node, parent, seed }),
        3 => (instance_id(), prop::sample::select(vec![D_IN, D_GAIN]), value()).prop_map(
            |(i, l, value)| Command::SetInput {
                node: instance_node_id(i, id(l)),
                port: "value".to_owned(),
                value,
            }
        ),
        1 => (instance_id(), prop::sample::select(vec![D_IN, D_GAIN])).prop_map(|(i, l)| {
            Command::ClearInput {
                node: instance_node_id(i, id(l)),
                port: "value".to_owned(),
            }
        }),
        2 => (instance_id(), prop::sample::select(vec![D_IN, D_GAIN]), source()).prop_map(
            |(i, l, source)| Command::SetExpression {
                node: instance_node_id(i, id(l)),
                port: "value".to_owned(),
                source,
            }
        ),
        3 => (num_id(), instance_id(), prop::sample::select(vec![D_IN, D_GAIN])).prop_map(
            |(from, i, l)| Command::Link {
                from,
                from_port: "out".to_owned(),
                to: instance_node_id(i, id(l)),
                to_port: "value".to_owned(),
            }
        ),
        2 => (instance_id(), num_id(), owned(&["a", "b"])).prop_map(|(i, to, to_port)| {
            Command::Link {
                from: instance_node_id(i, id(D_OUT)),
                from_port: "value".to_owned(),
                to,
                to_port,
            }
        }),
        1 => (inner_id(), owned(&INPUTS), value())
            .prop_map(|(node, port, value)| Command::SetInput { node, port, value }),
    ]
}

fn subnet_id() -> impl Strategy<Value = NodeId> {
    prop::sample::select(SUBNET_IDS.to_vec()).prop_map(id)
}

fn port_id() -> impl Strategy<Value = NodeId> {
    prop::sample::select(PORT_IDS.to_vec()).prop_map(id)
}

/// A network: the root or one of the subnets.
fn network() -> impl Strategy<Value = Option<NodeId>> {
    prop::option::of(subnet_id())
}

fn direction() -> impl Strategy<Value = PortDirection> {
    prop::sample::select(vec![PortDirection::Input, PortDirection::Output])
}

/// Commands on subnets, interfaces, promotion and moves.
fn structure() -> impl Strategy<Value = Command> {
    let port_type = prop::sample::select(vec![PortType::Float, PortType::Bool, PortType::Int]);
    prop_oneof![
        3 => (subnet_id(), network(), owned(&NAMES))
            .prop_map(|(id, parent, name)| Command::CreateSubnet { id, parent, name }),
        3 => (num_id(), network(), owned(&NAMES)).prop_map(|(id, parent, name)| {
            Command::CreateNodeIn { id, parent, type_id: NUM.to_owned(), name, port_type: None }
        }),
        1 => (flag_id(), network(), owned(&NAMES)).prop_map(|(id, parent, name)| {
            Command::CreateNodeIn { id, parent, type_id: FLAG.to_owned(), name, port_type: None }
        }),
        3 => (port_id(), subnet_id(), direction(), owned(&NAMES), port_type).prop_map(
            |(id, subnet, direction, name, port_type)| Command::AddInterfacePort {
                id, subnet, direction, name, port_type,
            }
        ),
        1 => (subnet_id(), direction(), owned(&NAMES)).prop_map(|(subnet, direction, name)| {
            Command::RemoveInterfacePort { subnet, direction, name }
        }),
        3 => (any_id(), network()).prop_map(|(node, parent)| Command::MoveNode { node, parent }),
        2 => (any_id(), owned(&INPUTS), port_id(), owned(&NAMES)).prop_map(
            |(node, port, interface, name)| Command::Promote { node, port, interface, name }
        ),
        3 => (num_id(), owned(&["a", "b"]), port_id(), owned(&NAMES)).prop_map(
            |(node, port, interface, name)| Command::Promote { node, port, interface, name }
        ),
        1 => (any_id(), owned(&INPUTS)).prop_map(|(node, port)| Command::Unpromote { node, port }),
        2 => (num_id(), owned(&["a", "b"]))
            .prop_map(|(node, port)| Command::Unpromote { node, port }),
        1 => (any_id(), prop::option::of((owned(&NAMES), 0u8..3))).prop_map(|(subnet, meta)| {
            Command::SetCharm {
                subnet,
                charm: meta.map(|(name, tier)| {
                    let mut charm = CharmMeta::new(name);
                    charm.min_tier = tier;
                    charm.capabilities = vec!["net".to_owned()];
                    charm
                }),
            }
        }),
        3 => (any_id(), owned(&OUTPUTS), any_id(), owned(&INPUTS)).prop_map(
            |(from, from_port, to, to_port)| Command::Link { from, from_port, to, to_port }
        ),
    ]
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
/// `ch()` and `out()` (which may be unknown, mistyped or cyclic), plus
/// broken ones.
fn source() -> impl Strategy<Value = String> {
    let template = prop::sample::select(vec![
        "$T * 2",
        "1 + 2",
        "ch(\"{n}/{p}\") + 1",
        "lerp(0.2, 1.0, ch(\"../{n}/{p}\"))",
        "ch(\"{n}/count\") * 2",
        "ch(\"{n}/flag\") ? 1 : 0",
        "ch(\"{n}/a\") + ch(\"{m}/b\")",
        "ch(\"/{m}/{n}/{p}\") + 1",
        "ch(\"{m}/{n}/a\") + ch(\"../../{n}/b\")",
        // out() (M0.11): outputs, possibly unknown, mistyped or cyclic.
        "out(\"{n}/{o}\") + 1",
        "out(\"{n}/out\") * 0.5 + ch(\"{m}/{p}\")",
        "out(\"{n}/flag\") ? out(\"{m}/n\") : 0",
        "out(\"/{m}/{n}/{o}\") + out(\"../{n}/out\")",
        "true",
        "1 +",
    ]);
    (
        template,
        owned(&NAMES[..10]),
        owned(&NAMES[..10]),
        owned(&INPUTS),
        owned(&OUTPUTS),
    )
        .prop_map(|(t, n, m, p, o)| {
            t.replace("{n}", &n)
                .replace("{m}", &m)
                .replace("{p}", &p)
                .replace("{o}", &o)
        })
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

/// Any coordinate, including ones that must be refused.
fn coordinate() -> impl Strategy<Value = f64> {
    prop_oneof![
        8 => -1.0e4..1.0e4f64,
        1 => prop::sample::select(vec![0.0, -0.0, MAX_POSITION, -MAX_POSITION, f64::MIN_POSITIVE]),
        1 => prop::sample::select(vec![f64::MAX, f64::MIN, 1.0e300]),
        1 => prop::sample::select(vec![f64::NAN, f64::INFINITY, f64::NEG_INFINITY]),
    ]
}

/// Placing and unplacing nodes: any node, mostly ones that exist.
fn layout() -> impl Strategy<Value = Command> {
    let position =
        || prop::option::weighted(0.85, (coordinate(), coordinate()).prop_map(|(x, y)| [x, y]));
    prop_oneof![
        3 => (num_id(), position())
            .prop_map(|(node, position)| Command::SetPosition { node, position }),
        1 => (any_id(), position())
            .prop_map(|(node, position)| Command::SetPosition { node, position }),
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
        5 => structure(),
        5 => charms(),
        6 => layout(),
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
/// and 8 stay free. Subnet 9 (`s`) holds subnet 10 (`t`), interface input
/// 11 (`in`), interface output 12 (`out`), and node 5 (`e`).
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
        Command::CreateSubnet {
            id: id(9),
            parent: None,
            name: "s".to_owned(),
        },
        Command::CreateSubnet {
            id: id(10),
            parent: Some(id(9)),
            name: "t".to_owned(),
        },
        Command::AddInterfacePort {
            id: id(11),
            subnet: id(9),
            direction: PortDirection::Input,
            name: "in".to_owned(),
            port_type: PortType::Float,
        },
        Command::AddInterfacePort {
            id: id(12),
            subnet: id(9),
            direction: PortDirection::Output,
            name: "out".to_owned(),
            port_type: PortType::Float,
        },
        Command::MoveNode {
            node: id(5),
            parent: Some(id(9)),
        },
        Command::Link {
            from: id(1),
            from_port: "out".to_owned(),
            to: id(2),
            to_port: "a".to_owned(),
        },
        Command::AddDefinition {
            def: definition("d", "0.0", 2.0, 0),
        },
        Command::Instantiate {
            id: id(15),
            parent: None,
            name: "inst".to_owned(),
            def: "d".to_owned(),
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

/// A transaction that builds `snapshot` from an empty graph. Instances
/// are instantiated; of their contents only the interface inputs' sources
/// and the links around them are set.
fn rebuild(snapshot: &GraphSnapshot) -> Command {
    let mut steps: Vec<Command> = snapshot
        .definitions
        .iter()
        .map(|def| Command::AddDefinition { def: def.clone() })
        .collect();
    let by_id: std::collections::BTreeMap<NodeId, &livspeil_graph::NodeSnapshot> =
        snapshot.nodes.iter().map(|n| (n.id, n)).collect();
    // The instance whose contents hold a node.
    let holder = |n: NodeId| {
        let mut current = by_id.get(&n).and_then(|n| n.parent);
        while let Some(p) = current {
            let node = by_id.get(&p)?;
            if node.def.is_some() {
                return Some(p);
            }
            current = node.parent;
        }
        None
    };
    let inner = |n: NodeId| holder(n).is_some();
    let editable = |n: &livspeil_graph::NodeSnapshot| match holder(n.id) {
        None => true,
        Some(i) => n.parent == Some(i) && n.type_id == INPUT_TYPE,
    };
    let layout: Vec<Command> = snapshot
        .positions
        .iter()
        .map(|(&node, &position)| Command::SetPosition {
            node,
            position: Some(position),
        })
        .collect();
    // Parents before children.
    let mut pending: Vec<_> = snapshot.nodes.iter().collect();
    let mut created = std::collections::BTreeSet::new();
    while !pending.is_empty() {
        let before = pending.len();
        pending.retain(|node| {
            if node.parent.is_some_and(|p| !created.contains(&p)) {
                return true;
            }
            created.insert(node.id);
            if let Some(def) = &node.def {
                steps.push(Command::Instantiate {
                    id: node.id,
                    parent: node.parent,
                    name: node.name.clone(),
                    def: def.clone(),
                });
                return false;
            }
            if inner(node.id) {
                if editable(node) {
                    for (port, value) in &node.inputs {
                        steps.push(Command::SetInput {
                            node: node.id,
                            port: port.clone(),
                            value: value.clone(),
                        });
                    }
                }
                return false;
            }
            steps.push(Command::CreateNodeIn {
                id: node.id,
                parent: node.parent,
                type_id: node.type_id.clone(),
                name: node.name.clone(),
                port_type: node.port_type,
            });
            if node.charm.is_some() {
                steps.push(Command::SetCharm {
                    subnet: node.id,
                    charm: node.charm.clone(),
                });
            }
            for (port, value) in &node.inputs {
                steps.push(Command::SetInput {
                    node: node.id,
                    port: port.clone(),
                    value: value.clone(),
                });
            }
            false
        });
        assert!(pending.len() < before, "snapshot parents form a tree");
    }
    for link in &snapshot.links {
        if inner(link.to) && !by_id.get(&link.to).is_some_and(|n| editable(n)) {
            continue;
        }
        steps.push(Command::Link {
            from: link.from,
            from_port: link.from_port.clone(),
            to: link.to,
            to_port: link.to_port.clone(),
        });
    }
    // Sources last: ch() may read any node.
    for node in snapshot.nodes.iter().filter(|n| editable(n)) {
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
    steps.extend(layout);
    Command::Transaction(steps)
}
