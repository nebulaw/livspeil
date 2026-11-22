// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Documents from every released format must load forever.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::registry;
use livspeil_doc::{CURRENT_FORMAT, Document, Tier, canonicalize, format, parse, room_file};
use livspeil_node_api::Value;

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut rooms = Vec::new();
    for version in fs::read_dir(&root).unwrap() {
        let version = version.unwrap().path();
        if !version.is_dir() {
            continue;
        }
        for room in fs::read_dir(&version).unwrap() {
            let room = room.unwrap().path();
            if room.extension().is_some_and(|e| e == "room") {
                rooms.push(room);
            }
        }
    }
    rooms.sort();
    rooms
}

fn load(path: &Path) -> Document {
    let text = fs::read_to_string(room_file(path)).unwrap();
    parse(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_fixture_loads_and_builds_a_graph() {
    let rooms = fixtures();
    assert!(!rooms.is_empty(), "no fixtures found");
    for path in rooms {
        let doc = load(&path);
        assert_eq!(doc.format, CURRENT_FORMAT);
        let graph = doc
            .to_graph(registry())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        // Loading is stable: canonical text reloads to the same graph.
        let text = canonicalize(&fs::read_to_string(room_file(&path)).unwrap()).unwrap();
        let again = parse(&text).unwrap().to_graph(registry()).unwrap();
        assert_eq!(again.snapshot(), graph.snapshot(), "{}", path.display());
    }
}

#[test]
fn format_1_sun_and_lamp_content() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format-1/sun-and-lamp.room");
    let doc = load(&path);
    assert_eq!(doc.name, "Sun and Lamp");
    assert_eq!(doc.min_tier, Tier(1));
    assert_eq!(doc.meta.positions["sun"], [120.0, 40.5]);
    let graph = doc.to_graph(registry()).unwrap();
    let snap = graph.snapshot();
    assert_eq!(snap.nodes.len(), 2, "the slashdashed node is ignored");
    assert_eq!(snap.links.len(), 1);
    let sun = graph.find("sun").unwrap();
    assert_eq!(
        graph.input_value(sun, "offset").unwrap(),
        Some(&Value::Float(1.0))
    );
    let lamp = graph.find("lamp").unwrap();
    assert_eq!(
        graph.input_value(lamp, "b").unwrap(),
        Some(&Value::Color([0.35, 0.08, 260.0, 1.0]))
    );
    // Re-saving from the graph gives a canonical document that matches.
    let mut saved = Document::from_graph(&graph, doc.name.clone()).unwrap();
    saved.min_tier = doc.min_tier;
    saved.meta = doc.meta.clone();
    assert_eq!(parse(&format(&saved).unwrap()).unwrap(), saved);
}
