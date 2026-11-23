// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil cook --wasm` (M0.15): sandboxed wasm node packages are
//! registered next to the core nodes before the room is loaded.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use livspeil_testkit::assert_golden;

fn livspeil(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_livspeil"))
        .args(args)
        .output()
        .expect("failed to run livspeil binary")
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/wasm/hello-node")
}

fn s(path: &Path) -> &str {
    path.to_str().expect("utf-8 path")
}

#[test]
fn hello_room_text_golden() {
    let room = example().join("hello.room");
    let wasm = example().join("hello-node.wasm");
    let out = livspeil(&[
        "cook",
        s(&room),
        "--wasm",
        s(&wasm),
        "--frames",
        "240",
        "--every",
        "30",
        "--stats",
    ]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(out.stderr.is_empty(), "{stderr}");
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    assert!(stdout.contains("frame=60 time=1.000000 banner.value=\"LEVEL 2.00!\""));
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    assert_golden(&golden, "wasm-hello.text", &stdout);
}

#[test]
fn wat_text_packages_load_too() {
    let room = example().join("hello.room");
    let wat = example().join("hello-node.wat");
    let out = livspeil(&["cook", s(&room), "--wasm", s(&wat), "--format", "jsonl"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).expect("utf-8 output");
    assert!(
        stdout.contains(r#""banner":{"value":{"string":"LEVEL 0.00!"}}"#),
        "{stdout}"
    );
}

#[test]
fn without_the_package_the_nodes_are_unknown() {
    let out = livspeil(&["cook", s(&example().join("hello.room"))]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown node type 'demo.wave.sine@1'"));
}

#[test]
fn bad_packages_fail_cleanly() {
    let room = example().join("hello.room");
    let out = livspeil(&["cook", s(&room), "--wasm", "/nonexistent/p.wasm"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("wasm package /nonexistent/p.wasm: cannot read"),
        "{stderr}"
    );
    // The same package twice: duplicate node types.
    let wasm = example().join("hello-node.wasm");
    let out = livspeil(&["cook", s(&room), "--wasm", s(&wasm), "--wasm", s(&wasm)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("already registered"));
    let out = livspeil(&["cook", s(&room), "--wasm"]);
    assert_eq!(out.status.code(), Some(2));
}
