// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! System bindings (M0.8): `Cooker::set_system` and the caching of nodes
//! that read the system (`core.fs.list_dir`).

use std::sync::Arc;

use livspeil_graph::{CookError, CookStats, Cooker, Graph};
use livspeil_node_api::{CookContext, NodeError, SystemHost, Value};
use livspeil_nodes_core::{MemFs, core_registry};

fn shelf() -> (Graph, livspeil_graph::NodeId, livspeil_graph::NodeId) {
    let mut g = Graph::new(Arc::new(core_registry()));
    let list = g.add_node("core.fs.list_dir@1", "list").unwrap();
    g.set_input(list, "path", Value::String("books".into()))
        .unwrap();
    let count = g.add_node("core.table.count@1", "count").unwrap();
    g.link(list, "entries", count, "table").unwrap();
    // An unrelated, time-independent node.
    let add = g.add_node("core.math.add@1", "add").unwrap();
    g.set_input(add, "a", Value::Float(1.0)).unwrap();
    (g, list, count)
}

fn fs() -> Arc<MemFs> {
    Arc::new(
        MemFs::new()
            .with_file("books/a.pdf", 1, 1)
            .unwrap()
            .with_file("books/b.pdf", 2, 2)
            .unwrap(),
    )
}

fn count_of(cooker: &mut Cooker, g: &Graph, count: livspeil_graph::NodeId) -> i64 {
    match cooker.cook_output(g, count, "count").unwrap() {
        Value::Int(n) => n,
        other => panic!("{other:?}"),
    }
}

fn stats(cooker: &mut Cooker) -> CookStats {
    let s = cooker.stats();
    cooker.reset_stats();
    s
}

#[test]
fn without_a_host_list_dir_fails() {
    let (g, list, count) = shelf();
    let mut cooker = Cooker::new();
    let err = cooker.cook(&g, count).unwrap_err();
    assert_eq!(err, CookError::Upstream { node: list });
    assert!(matches!(
        cooker.error(list),
        Some(CookError::Node(NodeError::Failed(_)))
    ));
    assert!(cooker.system().is_none());
}

#[test]
fn listings_are_cached_per_revision() {
    let (g, list, count) = shelf();
    let host = fs();
    let mut cooker = Cooker::new();
    cooker.set_system(Some(host.clone()));
    assert_eq!(cooker.system().map(|h| h.revision()), Some(host.revision()));
    assert_eq!(count_of(&mut cooker, &g, count), 2);
    assert_eq!(
        stats(&mut cooker),
        CookStats {
            cooked: 2,
            reused: 0
        }
    );

    // Same revision: everything is reused, also across clock changes.
    cooker.set_clock(CookContext {
        frame: 1,
        time: 1.0,
        dt: 1.0,
    });
    assert_eq!(count_of(&mut cooker, &g, count), 2);
    assert_eq!(
        stats(&mut cooker),
        CookStats {
            cooked: 0,
            reused: 2
        }
    );

    // The disk changes, but the host has not refreshed: still the snapshot.
    host.add_file("books/c.pdf", 3, 3).unwrap();
    assert_eq!(count_of(&mut cooker, &g, count), 2);
    assert_eq!(
        stats(&mut cooker),
        CookStats {
            cooked: 0,
            reused: 2
        }
    );

    // Refresh: the listing and its consumer are re-cooked.
    host.bump();
    assert_eq!(count_of(&mut cooker, &g, count), 3);
    assert_eq!(
        stats(&mut cooker),
        CookStats {
            cooked: 2,
            reused: 0
        }
    );

    // A refresh without changes re-lists, and early cutoff keeps `count`.
    host.bump();
    assert_eq!(count_of(&mut cooker, &g, count), 3);
    assert_eq!(
        stats(&mut cooker),
        CookStats {
            cooked: 1,
            reused: 1
        }
    );

    // Other nodes never notice refreshes.
    let add = g.find("add").unwrap();
    cooker.cook(&g, add).unwrap();
    host.bump();
    cooker.reset_stats();
    cooker.cook(&g, add).unwrap();
    assert_eq!(
        stats(&mut cooker),
        CookStats {
            cooked: 0,
            reused: 1
        }
    );
    assert!(cooker.error(list).is_none());
}

#[test]
fn replacing_or_removing_the_host_recooks() {
    let (g, list, count) = shelf();
    let mut cooker = Cooker::new();
    cooker.set_system(Some(fs()));
    assert_eq!(count_of(&mut cooker, &g, count), 2);

    let other = Arc::new(MemFs::new().with_file("books/x.pdf", 1, 1).unwrap());
    cooker.set_system(Some(other));
    assert_eq!(count_of(&mut cooker, &g, count), 1);

    // Without a host the node fails and keeps its last good output.
    cooker.set_system(None);
    assert_eq!(count_of(&mut cooker, &g, count), 1);
    assert!(cooker.error(list).is_some());

    // Input edits re-cook as usual.
    let mut g = g;
    cooker.set_system(Some(fs()));
    g.set_input(list, "pattern", Value::String("a*".into()))
        .unwrap();
    assert_eq!(count_of(&mut cooker, &g, count), 1);
    assert!(cooker.error(list).is_none());
}
