// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil diff`: golden output for the fixture pair and exit codes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use livspeil_testkit::assert_golden;

fn livspeil(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_livspeil"))
        .args(args)
        .output()
        .expect("failed to run livspeil binary")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).unwrap()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../livspeil-doc/tests/diff-fixtures/{name}.room"))
        .to_str()
        .unwrap()
        .to_owned()
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[test]
fn text_output_matches_golden_and_hides_meta() {
    let out = livspeil(&["diff", &fixture("before"), &fixture("after")]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(!text.contains("meta"));
    assert_golden(&golden_dir(), "diff-fixtures.text", &text);
}

#[test]
fn jsonl_output_matches_golden_and_parses() {
    let out = livspeil(&[
        "diff",
        &fixture("before"),
        &fixture("after"),
        "--format",
        "jsonl",
        "--meta",
    ]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let text = stdout(&out);
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(v["op"].is_string() && v["kind"].is_string(), "{line}");
    }
    assert!(text.contains(r#""kind":"position""#));
    assert_golden(&golden_dir(), "diff-fixtures.jsonl", &text);
}

#[test]
fn equal_documents_exit_zero_with_no_output() {
    // A package and its room.kdl are the same document.
    let before = fixture("before");
    let file = format!("{before}/room.kdl");
    let out = livspeil(&["diff", &before, &file]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).is_empty());
}

#[test]
fn meta_only_changes_need_the_flag() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("livspeil-diff-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let text = fs::read_to_string(format!("{}/room.kdl", fixture("before"))).unwrap();
    let moved = dir.join("moved.kdl");
    fs::write(&moved, text.replace("x=200.0", "x=210.0")).unwrap();
    let moved = moved.to_str().unwrap();

    let out = livspeil(&["diff", &fixture("before"), moved]);
    assert_eq!(out.status.code(), Some(0));
    let out = livspeil(&["diff", &fixture("before"), moved, "--meta"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stdout(&out),
        "~ meta node \"light\" position: 200.0 0.0 -> 210.0 0.0\n"
    );

    // Load errors are exit 3, with the location.
    let broken = dir.join("broken.kdl");
    fs::write(&broken, "room \"x\" format=1 {\n  bogus\n}\n").unwrap();
    let out = livspeil(&["diff", &fixture("before"), broken.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stderr(&out).contains("broken.kdl:2:3:"), "{}", stderr(&out));
    let out = livspeil(&["diff", &fixture("before"), "/nonexistent/x.room"]);
    assert_eq!(out.status.code(), Some(3));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn usage_errors_exit_two() {
    let a = fixture("before");
    for args in [
        vec!["diff"],
        vec!["diff", a.as_str()],
        vec!["diff", a.as_str(), a.as_str(), a.as_str()],
        vec!["diff", a.as_str(), a.as_str(), "--format"],
        vec!["diff", a.as_str(), a.as_str(), "--format", "xml"],
        vec!["diff", a.as_str(), a.as_str(), "--color"],
    ] {
        assert_eq!(livspeil(&args).status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn help_documents_diff_and_assets() {
    let out = livspeil(&["--help"]);
    let text = stdout(&out);
    for needle in [
        "diff <a> <b>",
        "--format text|jsonl",
        "--meta",
        "assets add",
        "assets check",
        "0 no differences, 1 differences",
        "CC-BY-SA-4.0",
    ] {
        assert!(text.contains(needle), "help lacks {needle}");
    }
}
