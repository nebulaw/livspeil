// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

fn livspeil(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_livspeil"))
        .args(args)
        .output()
        .expect("failed to run livspeil binary")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh temp directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("livspeil-cli-{tag}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    /// Write `Name.room/room.kdl` and return the package path.
    fn room(&self, name: &str, text: &str) -> PathBuf {
        let pkg = self.0.join(format!("{name}.room"));
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("room.kdl"), text).unwrap();
        pkg
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

const MESSY: &str = "// A license header comment\n\nroom \"r\" format=1 { network \"/\" {\n node b type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\" { input a 1 }\n node a type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\"\n link from=\"a.out\" to=\"b.b\" } }\n";

const CANONICAL: &str = "// A license header comment\n\nroom \"r\" format=1 min-tier=\"T0\" {\n  network \"/\" {\n    node \"a\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\"\n    node \"b\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\" {\n      input \"a\" 1\n    }\n    link from=\"a.out\" to=\"b.b\"\n  }\n}\n";

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
fn help_lists_commands() {
    let out = livspeil(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    for cmd in ["cook", "validate", "fmt", "diff", "--check"] {
        assert!(stdout.contains(cmd), "help is missing '{cmd}'");
    }
}

#[test]
fn reserved_commands_exit_with_usage_code() {
    assert_eq!(livspeil(&["diff"]).status.code(), Some(2));
}

#[test]
fn usage_errors_exit_with_code_2() {
    for args in [
        &["frobnicate"][..],
        &["validate"],
        &["validate", "a", "b"],
        &["fmt"],
        &["fmt", "--frob", "x"],
    ] {
        assert_eq!(livspeil(args).status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn example_room_validates_and_is_canonical() {
    let example = repo_root().join("examples/sun-and-lamp.room");
    let out = livspeil(&["validate", s(&example)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("9 nodes, 9 links"));
    let out = livspeil(&["fmt", "--check", s(&example)]);
    assert!(out.status.success(), "{}", stderr(&out));
    // The file path form works too.
    let out = livspeil(&["validate", s(&example.join("room.kdl"))]);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn every_example_room_validates_and_is_canonical() {
    let examples = repo_root().join("examples");
    let mut count = 0;
    for entry in fs::read_dir(examples).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "room") {
            count += 1;
            let out = livspeil(&["validate", s(&path)]);
            assert!(out.status.success(), "{}", stderr(&out));
            let out = livspeil(&["fmt", "--check", s(&path)]);
            assert!(out.status.success(), "{}", stderr(&out));
        }
    }
    assert!(count > 0);
}

#[test]
fn fmt_check_reports_without_writing_then_fmt_rewrites() {
    let tmp = TempDir::new("fmt");
    let pkg = tmp.room("Messy", MESSY);
    let file = pkg.join("room.kdl");

    let out = livspeil(&["fmt", "--check", s(&pkg)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("not in canonical form"));
    assert_eq!(fs::read_to_string(&file).unwrap(), MESSY);

    let out = livspeil(&["fmt", s(&pkg)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(fs::read_to_string(&file).unwrap(), CANONICAL);

    let out = livspeil(&["fmt", s(&file), "--check"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let out = livspeil(&["validate", s(&pkg)]);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn validate_reports_errors_with_locations() {
    let tmp = TempDir::new("validate");
    let bad_type = tmp.room(
        "BadType",
        "room \"r\" format=1 {\n  network \"/\" {\n    node \"n\" type=\"core.nope@1\" id=\"01920000-0000-7000-8000-000000000001\"\n  }\n}\n",
    );
    let out = livspeil(&["validate", s(&bad_type)]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.contains("room.kdl:3:5: node 'n': unknown node type 'core.nope@1'"),
        "{err}"
    );

    let syntax = tmp.room("Syntax", "room \"r\" format=1 {\n  network \"/\" {\n");
    let out = livspeil(&["validate", s(&syntax)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("syntax error"), "{}", stderr(&out));

    // fmt also refuses (and leaves) unparseable files.
    let out = livspeil(&["fmt", s(&syntax)]);
    assert_eq!(out.status.code(), Some(1));

    let newer = tmp.room("Newer", "room \"r\" format=99 { network \"/\" }\n");
    let out = livspeil(&["validate", s(&newer)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("format 99 is not supported"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn missing_files_are_reported() {
    let tmp = TempDir::new("missing");
    let ghost = tmp.0.join("Ghost.room");
    for args in [&["validate", s(&ghost)][..], &["fmt", "--check", s(&ghost)]] {
        let out = livspeil(args);
        assert_eq!(out.status.code(), Some(1));
        assert!(stderr(&out).contains("cannot read"), "{}", stderr(&out));
    }
}
