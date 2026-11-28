// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Node positions as commands: undo through every command that removes or
//! rebuilds nodes, refusals, no effect on cooking, compact drag history and
//! the JSON form.

mod common;

use std::io::Cursor;

use common::{D_NUM, NUM, definition, graph, id};
use livspeil_cmd::{Command, DocChange, GroupId, History, JournalWriter, Origin, replay};
use livspeil_graph::{Cooker, Graph, GraphError, MAX_POSITION, NodeId, instance_node_id};
use livspeil_node_api::Value;

fn place(node: NodeId, x: f64, y: f64) -> Command {
    Command::SetPosition {
        node,
        position: Some([x, y]),
    }
}

fn num(n: u128, name: &str) -> Command {
    Command::CreateNode {
        id: id(n),
        type_id: NUM.to_owned(),
        name: name.to_owned(),
    }
}

fn subnet(n: u128, parent: Option<u128>, name: &str) -> Command {
    Command::CreateSubnet {
        id: id(n),
        parent: parent.map(id),
        name: name.to_owned(),
    }
}

fn inside(n: u128, parent: u128, name: &str) -> Command {
    Command::CreateNodeIn {
        id: id(n),
        parent: Some(id(parent)),
        type_id: NUM.to_owned(),
        name: name.to_owned(),
        port_type: None,
    }
}

fn run(g: &mut Graph, commands: impl IntoIterator<Item = Command>) {
    for c in commands {
        c.apply(g).unwrap_or_else(|e| panic!("{c:?} failed: {e}"));
    }
}

/// Apply `command`, then its inverse, then the command again, checking the
/// snapshot each time. Returns the changes of the first application.
fn round_trip(g: &mut Graph, command: Command) -> Vec<DocChange> {
    let before = g.snapshot();
    let done = command.apply(g).expect("applies");
    let after = g.snapshot();
    done.inverse.apply(g).expect("inverts");
    assert_eq!(g.snapshot(), before, "inverse of {command:?}");
    command.apply(g).expect("applies again");
    assert_eq!(g.snapshot(), after);
    done.changes
}

/// A root node, and subnet 10 holding node 11 and subnet 12 with node 13,
/// all placed.
fn nested() -> Graph {
    let mut g = graph();
    run(
        &mut g,
        [
            num(1, "a"),
            subnet(10, None, "outer"),
            inside(11, 10, "b"),
            subnet(12, Some(10), "inner"),
            inside(13, 12, "c"),
        ],
    );
    for (i, n) in [1, 10, 11, 12, 13].into_iter().enumerate() {
        run(&mut g, [place(id(n), i as f64 * 10.0, -5.5)]);
    }
    g
}

#[test]
fn deleting_restores_every_position_inside() {
    let mut g = nested();
    round_trip(&mut g, Command::DeleteNode { id: id(10) });
    assert_eq!(g.positions().count(), 1);

    let mut g = nested();
    round_trip(&mut g, Command::DeleteNode { id: id(13) });
}

#[test]
fn a_position_follows_its_node_through_renames_and_moves() {
    let mut g = nested();
    let at = g.position(id(11));
    round_trip(
        &mut g,
        Command::Rename {
            node: id(11),
            name: "renamed".to_owned(),
        },
    );
    round_trip(
        &mut g,
        Command::MoveNode {
            node: id(11),
            parent: None,
        },
    );
    assert_eq!(g.position(id(11)), at);
}

#[test]
fn instances_hold_no_layout_and_attaching_can_be_undone() {
    let mut g = graph();
    let def = definition("d", "1", 2.0, 0);
    run(
        &mut g,
        [
            Command::AddDefinition { def },
            Command::Instantiate {
                id: id(15),
                parent: None,
                name: "charm".to_owned(),
                def: "d".to_owned(),
            },
            place(id(15), 3.0, 4.0),
        ],
    );
    let child = instance_node_id(id(15), id(D_NUM));
    let refused = place(child, 1.0, 1.0).apply(&mut g).unwrap_err();
    assert!(
        refused
            .to_string()
            .contains(&GraphError::InInstance(child).to_string()),
        "{refused}"
    );
    // Deleting and restoring the instance keeps its own position.
    round_trip(&mut g, Command::DeleteNode { id: id(15) });
    run(
        &mut g,
        [Command::Instantiate {
            id: id(15),
            parent: None,
            name: "charm".to_owned(),
            def: "d".to_owned(),
        }],
    );
    // Detached, the contents can be placed; attaching drops that layout and
    // its inverse brings it back.
    run(
        &mut g,
        [Command::Detach { instance: id(15) }, place(child, 7.0, 8.0)],
    );
    let changes = round_trip(
        &mut g,
        Command::Attach {
            subnet: id(15),
            def: "d".to_owned(),
        },
    );
    assert!(changes.contains(&DocChange::PositionChanged { node: child }));
    assert_eq!(g.position(child), None);
}

#[test]
fn making_a_definition_can_be_undone_with_its_layout() {
    let mut g = nested();
    run(&mut g, [Command::DeleteNode { id: id(12) }]);
    round_trip(
        &mut g,
        Command::MakeDefinition {
            subnet: id(10),
            def: "made".to_owned(),
            version: "1".to_owned(),
        },
    );
    assert!(g.position(id(10)).is_some(), "the instance keeps its place");
}

#[test]
fn bad_positions_are_refused_and_change_nothing() {
    let mut g = nested();
    let before = g.snapshot();
    for bad in [
        place(id(99), 0.0, 0.0),
        place(id(1), f64::NAN, 0.0),
        place(id(1), 0.0, f64::INFINITY),
        Command::Transaction(vec![
            place(id(1), 100.0, 100.0),
            place(id(1), f64::NEG_INFINITY, 0.0),
        ]),
    ] {
        assert!(bad.apply(&mut g).is_err(), "{bad:?}");
        assert_eq!(g.snapshot(), before);
        assert!(
            History::new()
                .apply(&mut g, bad, Origin::Script, None)
                .is_err()
        );
        assert_eq!(g.snapshot(), before);
    }
    // Placing a node where it is changes nothing.
    let same = place(id(1), 0.0, -5.5).apply(&mut g).expect("applies");
    assert!(same.changes.is_empty());
}

#[test]
fn layout_never_recooks() {
    let mut g = graph();
    run(
        &mut g,
        [
            num(1, "a"),
            num(2, "b"),
            Command::Link {
                from: id(1),
                from_port: "out".to_owned(),
                to: id(2),
                to_port: "a".to_owned(),
            },
        ],
    );
    let mut cooker = Cooker::new();
    cooker.cook(&g, id(2)).expect("cooks");
    let cooked = cooker.stats().cooked;
    run(&mut g, [place(id(1), 1.0, 2.0), place(id(2), 3.0, 4.0)]);
    assert_eq!(cooker.cook(&g, id(2)).expect("cooks"), [Value::Float(0.0)]);
    assert_eq!(cooker.stats().cooked, cooked);
}

#[test]
fn a_drag_is_one_small_undo_step_that_replays() {
    let mut g = graph();
    let mut history = History::new();
    let mut journal = JournalWriter::new(Vec::new());
    let mut apply = |g: &mut Graph, history: &mut History, c: Command, group| {
        let record = history.apply(g, c, Origin::Ui, group).expect("applies");
        journal.append(&record.entry).expect("journals");
    };
    for c in [num(1, "a"), num(2, "b"), num(3, "c")] {
        apply(&mut g, &mut history, c, None);
    }
    apply(&mut g, &mut history, place(id(1), -1.0, -1.0), None);
    let before = g.snapshot();
    let steps = history.undo_len();

    // 200 frames of dragging three nodes: nodes 1 and 2 in transactions,
    // node 3 on its own, in the same gesture.
    let drag = Some(GroupId(7));
    for frame in 0..200 {
        let x = f64::from(frame);
        let both = Command::Transaction(vec![place(id(1), x, 0.0), place(id(2), x, 1.0)]);
        apply(&mut g, &mut history, both, drag);
        apply(&mut g, &mut history, place(id(3), x, 2.0), drag);
    }
    history.seal();
    let dragged = g.snapshot();
    assert_eq!(history.undo_len(), steps + 1);

    let undo = history.undo(&mut g).expect("undoes");
    assert_eq!(g.snapshot(), before);
    let Command::Transaction(commands) = &undo.entry.command else {
        panic!("{:?}", undo.entry.command);
    };
    assert_eq!(commands.len(), 3, "one per node, not per frame");
    journal.append(&undo.entry).expect("journals");
    let redo = history.redo(&mut g).expect("redoes");
    assert_eq!(g.snapshot(), dragged);
    journal.append(&redo.entry).expect("journals");
    history.undo(&mut g).expect("undoes again");
    assert_eq!(g.snapshot(), before);
    history.redo(&mut g).expect("redoes again");

    let mut replayed = graph();
    replay(Cursor::new(journal.into_inner()), &mut replayed).expect("replays");
    assert_eq!(replayed.snapshot(), dragged);
}

#[test]
fn a_step_that_does_more_than_place_still_undoes_in_order() {
    let mut g = graph();
    let mut history = History::new();
    let group = Some(GroupId(1));
    let start = g.snapshot();
    // Create, place, delete, recreate, place again: merging the two
    // placements would be wrong, so nothing may be merged.
    for c in [
        num(1, "a"),
        place(id(1), 1.0, 1.0),
        Command::DeleteNode { id: id(1) },
        num(1, "a"),
        place(id(1), 2.0, 2.0),
    ] {
        history
            .apply(&mut g, c, Origin::Ui, group)
            .expect("applies");
    }
    let end = g.snapshot();
    history.undo(&mut g).expect("undoes");
    assert_eq!(g.snapshot(), start);
    history.redo(&mut g).expect("redoes");
    assert_eq!(g.snapshot(), end);
}

#[test]
fn the_json_form_requires_the_position_field() {
    let node = id(1);
    let text = serde_json::to_string(&place(node, 1.5, -2.0)).unwrap();
    assert_eq!(
        text,
        format!(r#"{{"set_position":{{"node":"{node}","position":[1.5,-2.0]}}}}"#)
    );
    assert_eq!(
        serde_json::from_str::<Command>(&text).unwrap(),
        place(node, 1.5, -2.0)
    );
    let unplace = format!(r#"{{"set_position":{{"node":"{node}","position":null}}}}"#);
    assert_eq!(
        serde_json::from_str::<Command>(&unplace).unwrap(),
        Command::SetPosition {
            node,
            position: None
        }
    );
    for bad in [
        format!(r#"{{"set_position":{{"node":"{node}"}}}}"#),
        format!(r#"{{"set_position":{{"node":"{node}","positon":[1,2]}}}}"#),
        format!(r#"{{"set_position":{{"node":"{node}","position":[1]}}}}"#),
        format!(r#"{{"set_position":{{"node":"{node}","position":[1,2,3]}}}}"#),
        format!(r#"{{"set_position":{{"node":"{node}","position":["1","2"]}}}}"#),
        format!(r#"{{"set_position":{{"node":"{node}","position":[1e999,0]}}}}"#),
    ] {
        assert!(serde_json::from_str::<Command>(&bad).is_err(), "{bad}");
    }
}

#[test]
fn the_graph_method_keeps_the_instances_place_too() {
    let mut g = nested();
    run(&mut g, [Command::DeleteNode { id: id(12) }]);
    let at = g.position(id(10));
    g.make_definition(id(10), "made", "1").expect("converts");
    assert_eq!(g.position(id(10)), at);
}

#[test]
fn dragging_a_new_node_remembers_one_placement() {
    let mut g = graph();
    let mut history = History::new();
    let group = Some(GroupId(3));
    history
        .apply(&mut g, num(1, "a"), Origin::Ui, group)
        .expect("creates");
    for frame in 0..100 {
        history
            .apply(
                &mut g,
                place(id(1), f64::from(frame), 0.0),
                Origin::Ui,
                group,
            )
            .expect("places");
    }
    let undo = history.undo(&mut g).expect("undoes");
    let Command::Transaction(commands) = &undo.entry.command else {
        panic!("{:?}", undo.entry.command);
    };
    assert_eq!(commands.len(), 2, "the placement and the creation");
    assert_eq!(g.node_count(), 0);
    history.redo(&mut g).expect("redoes");
    assert_eq!(g.position(id(1)), Some([99.0, 0.0]));
}

#[test]
fn a_placement_that_changes_nothing_keeps_redo() {
    let mut g = graph();
    let mut history = History::new();
    for c in [num(1, "a"), place(id(1), 1.0, 1.0), place(id(1), 2.0, 2.0)] {
        history.apply(&mut g, c, Origin::Ui, None).expect("applies");
    }
    history.undo(&mut g).expect("undoes");
    let steps = history.undo_len();
    // A click that did not move the node.
    let record = history
        .apply(&mut g, place(id(1), 1.0, 1.0), Origin::Ui, None)
        .expect("applies");
    assert!(record.changes.is_empty());
    assert_eq!(history.undo_len(), steps);
    assert!(history.can_redo());
}

#[test]
fn positions_far_outside_any_canvas_are_refused() {
    let mut g = nested();
    let before = g.snapshot();
    for x in [MAX_POSITION * 1.01, -f64::MAX, 1e300] {
        assert!(place(id(1), x, 0.0).apply(&mut g).is_err(), "{x}");
        assert!(place(id(1), 0.0, x).apply(&mut g).is_err(), "{x}");
    }
    assert_eq!(g.snapshot(), before);
    run(&mut g, [place(id(1), MAX_POSITION, -MAX_POSITION)]);
}
