// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Node positions between documents and graphs (the `meta` block).

mod common;

use common::{id, registry};
use livspeil_doc::{Document, format, parse};
use livspeil_graph::{GraphSnapshot, NodeId};

const ROOM: &str = r#"room "Layout" format=1 min-tier="T0" {
  network "/" {
    node "a" type="core.math.add@1" id="01920000-0000-7000-8000-000000000001"
    node "box" type="core.subnet@1" id="01920000-0000-7000-8000-000000000002" {
      network {
        node "deep" type="core.math.add@1" id="01920000-0000-7000-8000-000000000003"
      }
    }
  }
  meta {
    node "a" x=-12.5 y=3
    node "box" x=100.0 y=0.0
    node "box/deep" x=1.0 y=2.0
  }
}
"#;

fn positions(snapshot: &GraphSnapshot) -> Vec<(NodeId, [f64; 2])> {
    snapshot.positions.iter().map(|(n, p)| (*n, *p)).collect()
}

#[test]
fn positions_load_into_the_graph_and_save_back() {
    let doc = parse(ROOM).unwrap();
    let graph = doc.to_graph(registry()).unwrap();
    assert_eq!(
        positions(&graph.snapshot()),
        [
            (id(1), [-12.5, 3.0]),
            (id(2), [100.0, 0.0]),
            (id(3), [1.0, 2.0])
        ]
    );
    let saved = Document::from_graph(&graph, "Layout").unwrap();
    assert_eq!(saved.meta, doc.meta);
    let again = parse(&format(&saved).unwrap())
        .unwrap()
        .to_graph(registry())
        .unwrap();
    assert_eq!(again.snapshot(), graph.snapshot());
}

#[test]
fn saved_paths_follow_renames_and_deletions() {
    let mut graph = parse(ROOM).unwrap().to_graph(registry()).unwrap();
    graph.rename(id(2), "crate").unwrap();
    graph.remove_node(id(1)).unwrap();
    let saved = Document::from_graph(&graph, "Layout").unwrap();
    let keys: Vec<&str> = saved.meta.positions.keys().map(String::as_str).collect();
    assert_eq!(keys, ["crate", "crate/deep"]);
    parse(&format(&saved).unwrap()).unwrap();
}

#[test]
fn hostile_meta_is_refused() {
    let with = |meta: &str| {
        let start = ROOM.find("  meta {").unwrap();
        format!("{}  meta {{\n    {meta}\n  }}\n}}\n", &ROOM[..start])
    };
    for meta in [
        r#"node "nobody" x=1.0 y=1.0"#,
        r#"node "a/deep" x=1.0 y=1.0"#,
        r#"node "a" x=#nan y=1.0"#,
        r#"node "a" x=#inf y=1.0"#,
        r#"node "a" x=1.0"#,
        r#"node "a" x="1" y=1.0"#,
        "node \"a\" x=1.0 y=1.0\n    node \"a\" x=2.0 y=2.0",
    ] {
        assert!(parse(&with(meta)).is_err(), "{meta}");
    }
    // A document built in code is checked when it becomes a graph.
    for (path, position) in [("nobody", [0.0, 0.0]), ("a", [f64::NAN, 0.0])] {
        let mut doc = parse(ROOM).unwrap();
        doc.meta.positions.insert(path.to_owned(), position);
        assert!(doc.to_graph(registry()).is_err(), "{path}");
    }
}
