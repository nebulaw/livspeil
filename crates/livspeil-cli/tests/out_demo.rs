// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `examples/out-demo.room` (M0.11): an expression reading
//! `out("time_of_day/hour")` computes the same lamp level as a chain of
//! four math nodes, frame for frame.

use std::path::Path;
use std::process::Command;

use livspeil_testkit::assert_golden;

fn cook(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_livspeil"))
        .args(args)
        .output()
        .expect("failed to run livspeil binary");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {stderr}");
    assert!(out.stderr.is_empty(), "{stderr}");
    String::from_utf8(out.stdout).expect("utf-8 output")
}

#[test]
fn out_demo_text_golden() {
    let demo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/out-demo.room");
    let demo = demo.to_str().expect("utf-8 path");
    let args = ["cook", demo, "--frames", "600", "--every", "25"];
    let out = cook(&args);
    assert_eq!(out.lines().count(), 24 * 2);
    // The chain and the out() expression agree on every frame.
    let lines: Vec<&str> = out.lines().collect();
    for pair in lines.chunks(2) {
        let [chain, expr] = pair else {
            panic!("odd line count");
        };
        let (chain_head, chain_value) = chain.split_once(" chain_level.value=").unwrap();
        let (expr_head, expr_value) = expr.split_once(" expr_level.value=").unwrap();
        assert_eq!(chain_head, expr_head);
        assert_eq!(chain_value, expr_value, "{chain} / {expr}");
    }
    assert_golden(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
        "out-demo.text",
        &out,
    );
    assert_eq!(cook(&args), out);
}
