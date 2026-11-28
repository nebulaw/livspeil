// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The session over the example rooms: what it shows, what it computes and
//! how its clock moves.

use std::path::PathBuf;

use livspeil_dream::arrange::arrange;
use livspeil_dream::inspect::{Source, written_value};
use livspeil_dream::session::{FPS, OpenError, Session};
use livspeil_graph::{Cooker, Graph};
use livspeil_node_api::{CookContext, Value};

fn example(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
}

fn open(name: &str) -> Session {
    Session::open(&example(name)).unwrap_or_else(|e| panic!("{e}"))
}

fn output(s: &Session, node: &str, port: &str) -> Value {
    let i = s.find(node).unwrap_or_else(|| panic!("no node {node}"));
    let at = s.nodes()[i]
        .outputs
        .iter()
        .position(|p| p.name == port)
        .unwrap_or_else(|| panic!("no output {port}"));
    s.results()[i].outputs[at].clone().expect("a value")
}

/// The value a fresh cooker gives at `frame`, ticking every frame as
/// `livspeil cook` does.
fn cooked(graph: &Graph, node: &str, frame: u64) -> Value {
    let id = graph.find(node).unwrap();
    let mut cooker = Cooker::new();
    for f in 0..=frame {
        cooker.tick(
            graph,
            CookContext {
                frame: f,
                time: f as f64 / FPS,
                dt: 1.0 / FPS,
            },
            &[],
        );
    }
    cooker.cook(graph, id).unwrap().remove(0)
}

#[test]
fn the_lamp_changes_through_a_simulated_day() {
    let mut s = open("sun-and-lamp.room");
    assert_eq!(s.name(), "Sun and Lamp");
    assert_eq!((s.nodes().len(), s.links().len()), (9, 9));
    let night = output(&s, "lamp_color", "out");
    // 10 seconds are a day: 600 frames; noon is a quarter after 06:00.
    for _ in 0..150 {
        s.step();
    }
    assert_eq!(s.frame(), 150);
    let noon = output(&s, "lamp_color", "out");
    assert_ne!(night, noon);
    assert_eq!(noon, cooked(s.graph(), "lamp_color", 150));
    assert_ne!(
        output(&s, "lamp_intensity", "out"),
        cooked(s.graph(), "lamp_intensity", 0)
    );
    s.rewind();
    assert_eq!((s.frame(), output(&s, "lamp_color", "out")), (0, night));
    assert!(s.results().iter().all(|r| r.problem.is_none()));
}

#[test]
fn playing_steps_whole_frames_and_does_not_chase_a_stall() {
    let mut s = open("sun-and-lamp.room");
    assert_eq!(s.advance(1.0), 0, "stopped");
    s.set_playing(true);
    let mut frames = 0;
    // One second in 144 Hz window frames is 60 clock frames.
    for _ in 0..144 {
        frames += s.advance(1.0 / 144.0);
    }
    assert_eq!((frames, s.frame()), (60, 60));
    assert_eq!(s.advance(1.0 / FPS), 1);
    assert_eq!(s.advance(30.0), 6, "a stall steps a few frames, not 1800");
    assert_eq!(s.advance(0.0), 0);
    assert_eq!(s.advance(f64::NAN), 0);
    assert!(s.advance(1.0 / FPS) <= 2);
}

#[test]
fn the_inspector_says_where_each_input_comes_from() {
    let mut s = open("sun-and-lamp.room");
    s.select(s.find("time_of_day"));
    let i = s.inspection().unwrap();
    assert_eq!(i.type_id, "core.time.timeofday@1");
    let source = |name: &str| {
        i.inputs
            .iter()
            .find(|r| r.name == name)
            .map(|r| r.value.clone())
            .unwrap()
    };
    assert!(matches!(
        source("seconds"),
        Source::Link { from, value: Some(Value::Float(_)) } if from == "day_seconds.out"
    ));
    assert_eq!(source("offset"), Source::Set(Value::Float(6.0)));
    assert!(
        i.outputs.iter().any(
            |r| r.name == "hour" && r.value.as_ref().map(written_value).as_deref() == Some("6")
        )
    );
    // It follows the clock.
    for _ in 0..60 {
        s.step();
    }
    let hour = s.inspection().unwrap().outputs[0].value.clone();
    assert_ne!(hour, Some(Value::Float(6.0)));
    s.select(Some(999));
    assert!(s.inspection().is_none());
}

#[test]
fn every_example_room_opens_and_shows_its_root_network() {
    for name in [
        "sun-and-lamp.room",
        "charm-demo.room",
        "out-demo.room",
        "blink.room",
    ] {
        let mut s = open(name);
        assert!(!s.nodes().is_empty(), "{name}");
        let linked = s
            .graph()
            .links()
            .filter(|l| s.graph().parent(l.from.node).is_none())
            .count();
        assert!(s.links().len() >= linked, "{name}");
        for _ in 0..120 {
            s.step();
        }
        for i in 0..s.nodes().len() {
            s.select(Some(i));
            let shown = &s.nodes()[i];
            let inspection = s.inspection().unwrap();
            assert_eq!(inspection.inputs.len(), shown.inputs.len(), "{name}");
            assert_eq!(inspection.outputs.len(), shown.outputs.len(), "{name}");
            // A node that did not fail has a value for every output.
            if inspection.problem.is_none() {
                assert!(
                    inspection.outputs.iter().all(|r| r.value.is_some()),
                    "{name} {}: {inspection:?}",
                    shown.name
                );
            }
        }
    }
}

#[test]
fn a_subnet_is_one_node_with_its_interface_as_ports() {
    let s = open("charm-demo.room");
    let subnet = s
        .nodes()
        .iter()
        .find(|n| s.graph().is_subnet(n.id) || s.graph().is_instance(n.id))
        .expect("the demo has a Charm");
    assert!(!subnet.inputs.is_empty() || !subnet.outputs.is_empty());
    assert!(
        subnet
            .outputs
            .iter()
            .chain(&subnet.inputs)
            .all(|p| p.node != subnet.id)
    );
    let at = s.nodes().iter().position(|n| n == subnet).unwrap();
    assert!(
        s.links().iter().any(|l| l.from.0 == at || l.to.0 == at),
        "links reach its ports"
    );
}

#[test]
fn nodes_without_a_position_are_laid_out_below_the_placed_ones() {
    let s = open("sun-and-lamp.room");
    let n = s.nodes().len();
    let heights = vec![100.0; n];
    let mut placed = vec![None; n];
    let free = arrange(s.nodes(), s.links(), &placed, &heights);
    for link in s.links() {
        assert!(free[link.from.0][0] < free[link.to.0][0], "left to right");
    }
    for (i, a) in free.iter().enumerate() {
        for b in &free[i + 1..] {
            assert!(
                (a[0] - b[0]).abs() >= 200.0 || (a[1] - b[1]).abs() >= 100.0,
                "{a:?} {b:?} overlap"
            );
        }
    }
    placed[0] = Some([40.0, 500.0]);
    let mixed = arrange(s.nodes(), s.links(), &placed, &heights);
    assert_eq!(mixed[0], [40.0, 500.0]);
    assert!(mixed[1..].iter().all(|p| p[1] > 600.0 && p[0] >= 40.0));
}

#[test]
fn a_bad_path_or_file_is_a_message() {
    let missing = Session::open(&example("nothing.room")).unwrap_err();
    assert!(matches!(missing, OpenError::Read(..)), "{missing}");
    let bad = Session::open(&example("scripts/dim_lamps.py")).unwrap_err();
    assert!(matches!(bad, OpenError::Document(..)), "{bad}");
    assert!(bad.to_string().contains("dim_lamps.py"));
}
