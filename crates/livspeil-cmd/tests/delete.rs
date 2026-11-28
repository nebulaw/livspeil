// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `delete_nodes`: several nodes in one step, whatever reads what among
//! them, and exactly undone.

mod common;

use common::{NUM, graph, id};
use livspeil_cmd::{Command, delete_nodes};
use livspeil_graph::{Graph, NodeId};
use proptest::prelude::*;

fn num(n: u128, name: &str) -> Command {
    Command::CreateNode {
        id: id(n),
        type_id: NUM.to_owned(),
        name: name.to_owned(),
    }
}

fn reads(n: u128, port: &str, path: &str) -> Command {
    Command::SetExpression {
        node: id(n),
        port: port.to_owned(),
        source: format!("ch(\"{path}\")"),
    }
}

fn run(g: &mut Graph, commands: impl IntoIterator<Item = Command>) {
    for c in commands {
        c.apply(g).unwrap_or_else(|e| panic!("{c:?} failed: {e}"));
    }
}

/// `delete_nodes(ids)` either fails leaving `g` alone, or deletes exactly
/// `ids` (and what is inside them), and undoes and redoes exactly.
fn check(g: &mut Graph, ids: &[NodeId]) -> bool {
    let before = g.snapshot();
    let command = delete_nodes(g, ids);
    match command.apply(g) {
        Ok(done) => {
            assert!(ids.iter().all(|&id| !g.contains(id)));
            let after = g.snapshot();
            done.inverse.apply(g).expect("inverts");
            assert_eq!(g.snapshot(), before);
            command.apply(g).expect("applies again");
            assert_eq!(g.snapshot(), after);
            true
        }
        Err(_) => {
            assert_eq!(g.snapshot(), before);
            false
        }
    }
}

#[test]
fn nodes_that_read_each_other_go_together() {
    let mut g = graph();
    run(
        &mut g,
        [
            num(1, "a"),
            num(2, "b"),
            num(3, "c"),
            reads(2, "a", "a/a"),
            reads(3, "a", "b/a"),
        ],
    );
    // `c` stays and reads `b`.
    assert!(!check(&mut g, &[id(1), id(2)]));
    assert_eq!(g.node_count(), 3);
    assert!(check(&mut g, &[id(2), id(1), id(3), id(2)]));
    assert_eq!(g.node_count(), 0);
}

#[test]
fn a_subnet_goes_with_what_reads_into_and_out_of_it() {
    let mut g = graph();
    run(
        &mut g,
        [
            num(1, "outer"),
            Command::CreateSubnet {
                id: id(2),
                parent: None,
                name: "s".into(),
            },
            Command::CreateNodeIn {
                id: id(3),
                parent: Some(id(2)),
                type_id: NUM.to_owned(),
                name: "inner".into(),
                port_type: None,
            },
            num(4, "last"),
            reads(3, "a", "../outer/a"),
            reads(4, "a", "s/inner/b"),
        ],
    );
    assert!(!check(&mut g, &[id(2)]), "`last` stays and reads inside");
    assert!(!check(&mut g, &[id(1)]), "`inner` stays and reads outer");
    // Listing the inner node along with its subnet changes nothing.
    assert!(check(&mut g, &[id(3), id(1), id(2), id(4)]));
    assert_eq!(g.node_count(), 0);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn any_set_is_deleted_whole_or_not_at_all(
        reading in proptest::collection::vec((0..7u128, 0..2usize, 0..7u128, 0..2usize), 0..16),
        links in proptest::collection::vec((0..7u128, 0..7u128, 0..2usize), 0..8),
        chosen in proptest::collection::vec(0..7u128, 1..8),
    ) {
        let mut g = graph();
        let name = |n: u128| format!("n{n}");
        for n in 0..7 {
            run(&mut g, [num(n + 1, &name(n))]);
        }
        let ports = ["a", "b"];
        // Cycles are refused; the others stand.
        for (reader, port, read, from) in reading {
            let _ = reads(reader + 1, ports[port], &format!("{}/{}", name(read), ports[from]))
                .apply(&mut g);
        }
        for (from, to, port) in links {
            let _ = Command::Link {
                from: id(from + 1),
                from_port: "out".into(),
                to: id(to + 1),
                to_port: ports[port].into(),
            }
            .apply(&mut g);
        }
        let ids: Vec<NodeId> = chosen.iter().map(|&n| id(n + 1)).collect();
        // It must fail exactly when a node that stays reads one that goes.
        let held = ids.iter().any(|&gone| {
            g.expression_readers(gone).iter().any(|(by, _)| !ids.contains(by))
        });
        prop_assert_eq!(check(&mut g, &ids), !held);
    }
}
