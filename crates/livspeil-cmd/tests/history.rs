// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Undo, redo, coalescing and the journal.

mod common;

use std::io::Cursor;

use common::{NUM, graph, id};
use livspeil_cmd::{
    CmdError, Command, DocChange, EntryKind, GroupId, History, JournalEntry, JournalError,
    JournalWriter, Origin, read_journal, replay,
};
use livspeil_graph::NodeId;
use livspeil_node_api::Value;

fn create(n: u128, name: &str) -> Command {
    Command::CreateNode {
        id: id(n),
        type_id: NUM.to_owned(),
        name: name.to_owned(),
    }
}

fn set(node: NodeId, v: f64) -> Command {
    Command::SetInput {
        node,
        port: "a".to_owned(),
        value: Value::Float(v),
    }
}

#[test]
fn undo_and_redo_walk_the_history() {
    let mut g = graph();
    let mut h = History::new();
    assert!(!h.can_undo() && !h.can_redo());
    assert_eq!(h.undo(&mut g).unwrap_err(), CmdError::NothingToUndo);
    assert_eq!(h.redo(&mut g).unwrap_err(), CmdError::NothingToRedo);

    let s0 = g.snapshot();
    h.apply(&mut g, create(1, "a"), Origin::Ui, None).unwrap();
    let s1 = g.snapshot();
    h.apply(&mut g, set(id(1), 2.0), Origin::Script, None)
        .unwrap();
    let s2 = g.snapshot();
    assert_eq!(h.undo_len(), 2);

    let record = h.undo(&mut g).unwrap();
    assert_eq!(record.entry.kind, EntryKind::Undo);
    assert_eq!(record.entry.origin, Origin::Script);
    assert_eq!(
        record.changes,
        vec![DocChange::InputChanged {
            node: id(1),
            port: "a".into()
        }]
    );
    assert_eq!(g.snapshot(), s1);
    h.undo(&mut g).unwrap();
    assert_eq!(g.snapshot(), s0);
    assert!(!h.can_undo() && h.can_redo());

    let record = h.redo(&mut g).unwrap();
    assert_eq!(record.entry.kind, EntryKind::Redo);
    assert_eq!(g.snapshot(), s1);
    h.redo(&mut g).unwrap();
    assert_eq!(g.snapshot(), s2);
    assert!(!h.can_redo());
}

#[test]
fn a_new_command_after_undo_clears_redo() {
    let mut g = graph();
    let mut h = History::new();
    h.apply(&mut g, create(1, "a"), Origin::Ui, None).unwrap();
    h.apply(&mut g, create(2, "b"), Origin::Ui, None).unwrap();
    h.undo(&mut g).unwrap();
    assert!(h.can_redo());
    h.apply(&mut g, create(3, "c"), Origin::Ui, None).unwrap();
    assert!(!h.can_redo());
    assert_eq!(h.undo_len(), 2);
}

#[test]
fn a_failed_command_leaves_history_and_graph_alone() {
    let mut g = graph();
    let mut h = History::new();
    h.apply(&mut g, create(1, "a"), Origin::Ui, None).unwrap();
    h.undo(&mut g).unwrap();
    let before = g.snapshot();
    assert!(h.apply(&mut g, set(id(1), 1.0), Origin::Ui, None).is_err());
    assert_eq!(g.snapshot(), before);
    assert!(h.can_redo(), "a failed command does not clear redo");
}

#[test]
fn a_group_coalesces_into_one_undo_step() {
    let mut g = graph();
    let mut h = History::new();
    h.apply(&mut g, create(1, "a"), Origin::Ui, None).unwrap();
    let start = g.snapshot();
    let drag = Some(GroupId(7));
    for v in 1..=5 {
        h.apply(&mut g, set(id(1), f64::from(v)), Origin::Ui, drag)
            .unwrap();
    }
    let end = g.snapshot();
    assert_eq!(h.undo_len(), 2);
    h.undo(&mut g).unwrap();
    assert_eq!(g.snapshot(), start);
    h.redo(&mut g).unwrap();
    assert_eq!(g.snapshot(), end);

    // After undo/redo or seal, the same group starts a new step.
    h.apply(&mut g, set(id(1), 9.0), Origin::Ui, drag).unwrap();
    assert_eq!(h.undo_len(), 3);
    h.seal();
    h.apply(&mut g, set(id(1), 10.0), Origin::Ui, drag).unwrap();
    assert_eq!(h.undo_len(), 4);
    // A different group or no group breaks the run.
    h.apply(&mut g, set(id(1), 11.0), Origin::Ui, Some(GroupId(8)))
        .unwrap();
    h.apply(&mut g, set(id(1), 12.0), Origin::Ui, None).unwrap();
    h.apply(&mut g, set(id(1), 13.0), Origin::Ui, None).unwrap();
    assert_eq!(h.undo_len(), 7);
    h.clear();
    assert!(!h.can_undo());
}

#[test]
fn undo_after_an_outside_edit_fails_cleanly() {
    let mut g = graph();
    let mut h = History::new();
    h.apply(&mut g, create(1, "a"), Origin::Ui, None).unwrap();
    // Deleting the node behind the history's back breaks its undo
    // (DeleteNode of a missing node).
    (Command::DeleteNode { id: id(1) }).apply(&mut g).unwrap();
    let before = g.snapshot();
    assert!(h.undo(&mut g).is_err());
    assert_eq!(g.snapshot(), before);
    assert!(h.can_undo());
}

#[test]
fn journal_lines_are_json_with_metadata() {
    let mut g = graph();
    let mut h = History::new();
    let mut journal = JournalWriter::new(Vec::new());
    let record = h
        .apply(&mut g, create(1, "a"), Origin::Ai, Some(GroupId(3)))
        .unwrap();
    journal.append(&record.entry).unwrap();
    let text = String::from_utf8(journal.into_inner()).unwrap();
    assert_eq!(
        text,
        "{\"kind\":\"do\",\"origin\":\"ai\",\"group\":3,\"command\":{\"create_node\":\
         {\"id\":\"00000000-0000-0000-0000-000000000001\",\"type_id\":\"t.num@1\",\"name\":\"a\"}}}\n"
    );
    let entries = read_journal(Cursor::new(text)).unwrap();
    assert_eq!(entries, vec![record.entry]);
}

#[test]
fn journal_replay_reproduces_the_graph_including_undo() {
    let mut g = graph();
    let mut h = History::new();
    let mut journal = JournalWriter::new(Vec::new());
    let commands = [
        create(1, "a"),
        create(2, "b"),
        Command::Link {
            from: id(1),
            from_port: "out".into(),
            to: id(2),
            to_port: "a".into(),
        },
        set(id(1), 0.1),
        Command::Rename {
            node: id(2),
            name: "bee".into(),
        },
    ];
    for cmd in commands {
        let record = h.apply(&mut g, cmd, Origin::Ui, None).unwrap();
        journal.append(&record.entry).unwrap();
    }
    journal.append(&h.undo(&mut g).unwrap().entry).unwrap();
    journal.append(&h.undo(&mut g).unwrap().entry).unwrap();
    journal.append(&h.redo(&mut g).unwrap().entry).unwrap();
    journal.flush().unwrap();

    let bytes = journal.into_inner();
    let mut fresh = graph();
    assert_eq!(replay(Cursor::new(&bytes), &mut fresh).unwrap(), 8);
    assert_eq!(fresh.snapshot(), g.snapshot());
}

#[test]
fn journal_rejects_non_finite_values_and_bad_lines() {
    let mut journal = JournalWriter::new(Vec::new());
    let entry = JournalEntry {
        kind: EntryKind::Do,
        origin: Origin::Script,
        group: None,
        command: Command::Transaction(vec![set(id(1), f64::NAN)]),
    };
    assert!(matches!(
        journal.append(&entry),
        Err(JournalError::NonFiniteFloat)
    ));
    assert!(journal.into_inner().is_empty());

    // Missing kind and group default; blank lines are skipped.
    let text = "\n{\"origin\":\"import\",\"command\":{\"transaction\":[]}}\nnot json\n";
    let err = read_journal(Cursor::new(text)).unwrap_err();
    assert!(matches!(err, JournalError::Parse { line: 3, .. }), "{err}");

    let mut g = graph();
    let text = "{\"origin\":\"collab\",\"command\":{\"delete_node\":{\"id\":\"00000000-0000-0000-0000-000000000009\"}}}\n";
    let err = replay(Cursor::new(text), &mut g).unwrap_err();
    assert!(matches!(err, JournalError::Apply { line: 1, .. }), "{err}");
}
