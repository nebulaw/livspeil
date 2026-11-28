// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Editing through the session: every edit with undo and redo, saving, and
//! that a saved room reopens as it was.

use std::fs;
use std::path::PathBuf;

use livspeil_doc::{diff, parse};
use livspeil_dream::edit::Entry;
use livspeil_dream::session::Session;
use livspeil_dream::shown::Port;
use livspeil_node_api::Value;

/// A copy of an example room that is removed afterwards.
struct Copy(PathBuf);

impl Copy {
    fn of(room: &str, tag: &str) -> Copy {
        let from = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(room)
            .join("room.kdl");
        let dir = std::env::temp_dir().join(format!("dream-{}-{tag}.room", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::copy(from, dir.join("room.kdl")).unwrap();
        Copy(dir)
    }

    fn text(&self) -> String {
        fs::read_to_string(self.0.join("room.kdl")).unwrap()
    }

    fn open(&self) -> Session {
        Session::open(&self.0).unwrap_or_else(|e| panic!("{e}"))
    }
}

impl Drop for Copy {
    fn drop(&mut self) {
        // A leftover in the temporary directory is harmless.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn port(s: &Session, node: &str, name: &str) -> Port {
    let n = &s.nodes()[s.find(node).unwrap()];
    n.inputs
        .iter()
        .chain(&n.outputs)
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("{node} has no port {name}"))
        .clone()
}

/// Apply `edit`; undo gives the graph before it, redo the one after.
fn undoable(s: &mut Session, edit: impl FnOnce(&mut Session)) {
    let before = s.graph().snapshot();
    edit(s);
    s.end_gesture();
    let after = s.graph().snapshot();
    assert_ne!(before, after, "the edit did something");
    assert!(s.dirty());
    s.undo().unwrap();
    assert_eq!(s.graph().snapshot(), before);
    s.redo().unwrap();
    assert_eq!(s.graph().snapshot(), after);
}

#[test]
fn every_edit_can_be_undone() {
    let room = Copy::of("sun-and-lamp.room", "undo");
    let mut s = room.open();
    assert!(!s.dirty() && !s.can_undo());

    let node = s.nodes()[s.find("sun").unwrap()].id;
    undoable(&mut s, |s| {
        for x in 0..50 {
            s.move_node(node, [f64::from(x), 0.0]).unwrap();
        }
    });
    let offset = port(&s, "time_of_day", "offset");
    undoable(&mut s, |s| {
        s.enter(&offset, Entry::Value(Value::Float(12.0))).unwrap();
    });
    undoable(&mut s, |s| {
        s.enter(&offset, Entry::Expression("1 + 2".into())).unwrap();
    });
    // A value takes over from the expression.
    undoable(&mut s, |s| {
        s.enter(&offset, Entry::Value(Value::Float(3.0))).unwrap();
        assert_eq!(s.graph().expression(offset.node, &offset.port), Ok(None));
    });
    undoable(&mut s, |s| s.enter(&offset, Entry::Clear).unwrap());
    // `daylight.a` is linked from `sunrise`: relink it, then unlink it.
    let a = port(&s, "daylight", "a");
    let from = port(&s, "sunset", "out");
    assert!(s.can_connect(&from, &a));
    undoable(&mut s, |s| s.connect(&from, &a).unwrap());
    undoable(&mut s, |s| s.disconnect(&a).unwrap());
    let sunrise = s.nodes()[s.find("sunrise").unwrap()].id;
    s.select(s.find("sunrise"));
    undoable(&mut s, |s| s.rename(sunrise, "dawn").unwrap());
    assert_eq!(s.inspection().unwrap().name, "dawn", "still selected");
}

#[test]
fn refused_edits_change_nothing() {
    let room = Copy::of("sun-and-lamp.room", "refused");
    let mut s = room.open();
    let before = s.graph().snapshot();
    let offset = port(&s, "time_of_day", "offset");
    assert!(s.enter(&offset, Entry::Expression("1 +".into())).is_err());
    assert!(s.enter(&offset, Entry::Value(Value::Bool(true))).is_err());
    // A mismatched type, and a node feeding itself.
    let seconds = port(&s, "day_seconds", "out");
    let clamp = port(&s, "lamp_intensity", "clamp");
    assert!(!s.can_connect(&seconds, &clamp));
    assert!(s.connect(&seconds, &clamp).is_err());
    let own = port(&s, "day_seconds", "out");
    let input = port(&s, "day_seconds", "a");
    assert!(!s.can_connect(&own, &input));
    let sun = s.nodes()[s.find("sun").unwrap()].id;
    assert!(s.rename(sun, "sunset").is_err());
    assert!(s.rename(sun, "").is_err());
    assert!(s.move_node(sun, [f64::NAN, 0.0]).is_err());
    s.end_gesture();
    assert_eq!(s.graph().snapshot(), before);
    assert!(!s.can_undo());
}

#[test]
fn a_saved_room_has_exactly_the_edits_and_its_comments() {
    let room = Copy::of("sun-and-lamp.room", "save");
    let original = room.text();
    let mut s = room.open();
    let offset = port(&s, "time_of_day", "offset");
    s.enter(&offset, Entry::Value(Value::Float(12.0))).unwrap();
    let sun = s.nodes()[s.find("sun").unwrap()].id;
    s.move_node(sun, [10.0, 20.0]).unwrap();
    s.end_gesture();
    s.save().unwrap();
    assert!(!s.dirty());
    let text = room.text();
    assert!(text.starts_with("// SPDX-FileCopyrightText"), "{text}");
    assert!(text.contains("// The Phase 0 acceptance scene"), "{text}");
    let changes: Vec<String> = diff(&parse(&original).unwrap(), &parse(&text).unwrap())
        .changes
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(
        changes
            .iter()
            .any(|c| c.contains("offset") && c.contains("12.0")),
        "{changes:?}"
    );
    assert!(
        changes.iter().any(|c| c.contains("position")),
        "{changes:?}"
    );
    assert_eq!(room.open().graph().snapshot(), s.graph().snapshot());
    // No temporary file stays.
    assert_eq!(fs::read_dir(&room.0).unwrap().count(), 1);

    // Back to the saved state by undo is not dirty.
    s.enter(&offset, Entry::Value(Value::Float(1.0))).unwrap();
    assert!(s.dirty());
    s.undo().unwrap();
    assert!(!s.dirty());
}

#[test]
fn a_file_changed_elsewhere_is_not_overwritten() {
    let room = Copy::of("sun-and-lamp.room", "changed");
    let mut s = room.open();
    let offset = port(&s, "time_of_day", "offset");
    s.enter(&offset, Entry::Value(Value::Float(12.0))).unwrap();
    let theirs = room.text().replace("Sun and Lamp", "Theirs");
    fs::write(room.0.join("room.kdl"), &theirs).unwrap();
    let err = s.save().unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert_eq!(room.text(), theirs);
    assert!(s.dirty());
}

#[test]
fn random_edits_then_save_and_reopen_give_the_same_graph() {
    let room = Copy::of("charm-demo.room", "random");
    let mut s = room.open();
    let mut state = 7_u64;
    let mut below = |n: usize| {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        usize::try_from((state >> 33) % n as u64).unwrap()
    };
    for step in 0..600 {
        let n = below(s.nodes().len());
        let node = s.nodes()[n].clone();
        let other = s.nodes()[below(s.nodes().len())].clone();
        // Refusals are expected: they must only change nothing.
        let _ = match below(8) {
            0 => s.move_node(node.id, [below(900) as f64, below(900) as f64]),
            1 => s.rename(node.id, &format!("n{step}")),
            2 if !node.inputs.is_empty() => {
                let p = &node.inputs[below(node.inputs.len())];
                s.enter(p, Entry::Value(Value::Float(below(10) as f64)))
            }
            3 if !node.inputs.is_empty() && !other.outputs.is_empty() => {
                let to = &node.inputs[below(node.inputs.len())];
                let from = &other.outputs[below(other.outputs.len())];
                let allowed = s.can_connect(from, to);
                let done = s.connect(from, to);
                assert_eq!(allowed, done.is_ok(), "{from:?} {to:?} {done:?}");
                done
            }
            4 if !node.inputs.is_empty() => s.disconnect(&node.inputs[below(node.inputs.len())]),
            5 => s.undo(),
            6 => s.redo(),
            _ => {
                s.end_gesture();
                Ok(())
            }
        };
        if step % 50 == 49 {
            s.save().unwrap_or_else(|e| panic!("step {step}: {e}"));
            assert_eq!(
                room.open().graph().snapshot(),
                s.graph().snapshot(),
                "step {step}"
            );
        }
    }
}
