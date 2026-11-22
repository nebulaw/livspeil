// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden test: cook a hand-written "sun and lamp" graph over several frames.
//!
//! Refresh with `UPDATE_GOLDEN=1 cargo test -p livspeil-graph --test golden`
//! and review the diff by hand.

mod common;

use std::fmt::Write as _;
use std::path::Path;

use common::{float, graph};
use livspeil_graph::Cooker;
use livspeil_node_api::CookContext;
use livspeil_testkit::assert_golden;

#[test]
fn sun_and_lamp() {
    // clock.time -> lamp = lerp(0.2, 1.0, t) -> total = lamp + 0.05
    let mut g = graph();
    let clock = g.add_node("core.time.clock@1", "clock").unwrap();
    let lamp = g.add_node("core.math.lerp@1", "lamp").unwrap();
    let total = g.add_node("core.math.add@1", "total").unwrap();
    g.set_input(lamp, "a", float(0.2)).unwrap();
    g.set_input(lamp, "b", float(1.0)).unwrap();
    g.set_input(total, "b", float(0.05)).unwrap();
    g.link(clock, "time", lamp, "t").unwrap();
    g.link(lamp, "out", total, "a").unwrap();

    let mut cooker = Cooker::new();
    let mut report = String::new();
    for frame in 0..6_u32 {
        cooker.set_clock(CookContext {
            frame: u64::from(frame),
            time: f64::from(frame) * 0.25,
            dt: 0.25,
        });
        cooker.reset_stats();
        let value = cooker
            .cook_output(&g, total, "out")
            .unwrap()
            .as_float()
            .unwrap();
        let stats = cooker.stats();
        writeln!(
            report,
            "frame={frame} time={:.2} total={value:.4} cooked={} reused={}",
            f64::from(frame) * 0.25,
            stats.cooked,
            stats.reused
        )
        .unwrap();
    }

    // Cooking the same frame again is free.
    cooker.reset_stats();
    cooker.cook(&g, total).unwrap();
    let stats = cooker.stats();
    writeln!(
        report,
        "again cooked={} reused={}",
        stats.cooked, stats.reused
    )
    .unwrap();

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    assert_golden(&dir, "sun_and_lamp", &report);
}
