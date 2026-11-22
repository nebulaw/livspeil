// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Event links (M0.10) in `room.kdl`: written like other links, type
//! checked when loaded, and round tripped exactly. Event node state is
//! runtime only, so nothing else is stored.

mod common;

use std::path::Path;

use common::registry;
use livspeil_doc::{Document, canonicalize, format, parse, split_prolog};
use livspeil_node_api::PortType;

fn blink() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/blink.room/room.kdl");
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn blink_round_trips_exactly() {
    let text = blink();
    assert_eq!(canonicalize(&text).unwrap(), text);
    let doc = parse(&text).unwrap();
    let (_, body) = split_prolog(&text);
    assert_eq!(format(&doc).unwrap(), body);
    let graph = doc.to_graph(registry()).unwrap();
    assert_eq!(graph.link_count(), 8);
    let lamp = graph.find("lamp").unwrap();
    assert_eq!(
        graph.link_source(lamp, "trigger").unwrap(),
        Some((graph.find("blink").unwrap(), "fired"))
    );
    let ticks = graph.find("ticks").unwrap();
    assert_eq!(graph.port_type(ticks), Some(PortType::Event));
    let back = Document::from_graph(&graph, doc.name.clone()).unwrap();
    let mut expected = doc.clone();
    expected.min_tier = back.min_tier;
    expected.meta = back.meta.clone();
    assert_eq!(back, expected);
    assert_eq!(
        back.to_graph(registry()).unwrap().snapshot(),
        graph.snapshot()
    );
}

#[test]
fn event_links_are_type_checked_on_load() {
    let text = blink().replace(
        r#"link from="lamp.on" to="brightness.cond""#,
        r#"link from="blink.fired" to="brightness.cond""#,
    );
    let doc = parse(&text).unwrap();
    let err = doc.to_graph(registry()).unwrap_err();
    assert!(err.to_string().contains("type mismatch"), "{err}");
}
