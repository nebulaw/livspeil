// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::process::Command;

fn livspeil(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_livspeil"))
        .args(args)
        .output()
        .expect("failed to run livspeil binary")
}

#[test]
fn version_prints_and_succeeds() {
    let out = livspeil(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("livspeil "),
        "unexpected output: {stdout}"
    );
}

#[test]
fn help_lists_reserved_commands() {
    let out = livspeil(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cmd in ["cook", "validate", "fmt", "diff"] {
        assert!(stdout.contains(cmd), "help is missing '{cmd}'");
    }
}

#[test]
fn reserved_command_exits_with_usage_code() {
    let out = livspeil(&["cook"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn unknown_command_exits_with_usage_code() {
    let out = livspeil(&["frobnicate"]);
    assert_eq!(out.status.code(), Some(2));
}
