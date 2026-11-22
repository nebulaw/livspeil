// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil assets add|check` and asset checks in `validate`.

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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// SHA-256 of "hello world\n".
const HELLO_SHA: &str = "a948904f2f0f479b8f8197694b30184b0d2ed1c1cd2a1ec0fb85d299a192a447";

const ROOM: &str = "room \"r\" format=1 min-tier=\"T0\" {\n  network \"/\"\n}\n";

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

    fn room(&self) -> PathBuf {
        let pkg = self.0.join("R.room");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("room.kdl"), ROOM).unwrap();
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

#[test]
fn add_then_check_then_validate() {
    let tmp = TempDir::new("assets-add");
    let room = tmp.room();
    let file = tmp.0.join("hello.txt");
    fs::write(&file, "hello world\n").unwrap();

    let out = livspeil(&[
        "assets",
        "add",
        s(&room),
        s(&file),
        "--license",
        "CC-BY-4.0",
        "--source",
        "https://example.org/hello.txt",
        "--author",
        "Jane Doe",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let asset = format!("assets/{HELLO_SHA}.txt");
    assert!(stdout(&out).contains(&format!("added {asset}")));
    assert_eq!(
        fs::read_to_string(room.join("assets.lock")).unwrap(),
        format!(
            "assets-lock format=1 {{\n  asset \"{asset}\" sha256=\"{HELLO_SHA}\" size=12 license=\"CC-BY-4.0\" source=\"https://example.org/hello.txt\" author=\"Jane Doe\"\n}}\n"
        )
    );
    assert_eq!(
        fs::read_to_string(room.join(&asset)).unwrap(),
        "hello world\n"
    );

    // Same again: unchanged. The room.kdl path form works too.
    let out = livspeil(&[
        "assets",
        "add",
        s(&room.join("room.kdl")),
        s(&file),
        "--license",
        "CC-BY-4.0",
        "--source",
        "https://example.org/hello.txt",
        "--author",
        "Jane Doe",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("unchanged"));

    let out = livspeil(&["assets", "check", s(&room)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("ok (1 asset, 0 warnings)"));

    let out = livspeil(&["validate", s(&room)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("assets.lock: ok (1 asset"));
}

#[test]
fn verification_failures_fail_validate_with_locations() {
    let tmp = TempDir::new("assets-bad");
    let room = tmp.room();
    let file = tmp.0.join("hello.txt");
    fs::write(&file, "hello world\n").unwrap();
    let out = livspeil(&["assets", "add", s(&room), s(&file), "--license", "CC0-1.0"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let asset = room.join(format!("assets/{HELLO_SHA}.txt"));

    // Hash and size mismatch.
    fs::write(&asset, "hello there\n!").unwrap();
    let out = livspeil(&["validate", s(&room)]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("assets.lock:2:3: error: assets/a948"), "{err}");
    assert!(err.contains("sha256 mismatch"), "{err}");
    assert!(err.contains("size mismatch"), "{err}");

    // Missing file and an unlisted one.
    fs::remove_file(&asset).unwrap();
    fs::write(room.join("assets/stray.png"), "x").unwrap();
    let out = livspeil(&["assets", "check", s(&room)]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(err.contains("assets.lock:2:3: error:"), "{err}");
    assert!(err.contains("file is missing"), "{err}");
    assert!(
        err.contains("assets/stray.png: error: not listed in assets.lock"),
        "{err}"
    );

    // Bad license (error) and an unlisted one (warning; --strict fails).
    fs::remove_file(room.join("assets/stray.png")).unwrap();
    fs::write(&asset, "hello world\n").unwrap();
    let lock = room.join("assets.lock");
    let text = fs::read_to_string(&lock).unwrap();
    fs::write(&lock, text.replace("\"CC0-1.0\"", "\"MIT and CC0-1.0\"")).unwrap();
    let out = livspeil(&["validate", s(&room)]);
    assert_eq!(out.status.code(), Some(1));
    let col = text.find("license=").unwrap() - text.find("\n  asset").unwrap();
    let err = stderr(&out);
    assert!(
        err.contains(&format!("assets.lock:2:{col}: error:")),
        "{err} (column {col})"
    );
    assert!(err.contains("upper case"), "{err}");

    fs::write(&lock, text.replace("\"CC0-1.0\"", "\"CC-PDDC\"")).unwrap();
    let out = livspeil(&["validate", s(&room)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stderr(&out).contains("warning:"), "{}", stderr(&out));
    let out = livspeil(&["assets", "check", s(&room), "--strict"]);
    assert_eq!(out.status.code(), Some(1));

    // A syntax error in the lock.
    fs::write(&lock, "assets-lock format=1 {\n  asset 1\n}\n").unwrap();
    let out = livspeil(&["validate", s(&room)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("assets.lock:2:9:"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn add_rejects_bad_input() {
    let tmp = TempDir::new("assets-usage");
    let room = tmp.room();
    let file = tmp.0.join("a.txt");
    fs::write(&file, "a").unwrap();
    let r = s(&room);
    let f = s(&file);
    // Usage errors.
    for args in [
        vec!["assets"],
        vec!["assets", "nope"],
        vec!["assets", "add", r, f],
        vec!["assets", "add", r, "--license", "MIT"],
        vec!["assets", "add", r, f, "--license"],
        vec!["assets", "add", r, f, "--license", "MIT", "--bogus"],
        vec!["assets", "check"],
    ] {
        assert_eq!(livspeil(&args).status.code(), Some(2), "{args:?}");
    }
    // Rejected licenses, sources and missing files are errors (1).
    for args in [
        vec!["assets", "add", r, f, "--license", "NONE"],
        vec![
            "assets",
            "add",
            r,
            f,
            "--license",
            "MIT",
            "--source",
            "somewhere",
        ],
        vec!["assets", "add", r, "/nonexistent/file", "--license", "MIT"],
        vec!["assets", "add", s(&tmp.0), f, "--license", "MIT"],
    ] {
        let out = livspeil(&args);
        assert_eq!(out.status.code(), Some(1), "{args:?}: {}", stderr(&out));
    }
    assert!(!room.join("assets.lock").exists());
    let out = livspeil(&["assets", "check", r]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("no assets"));
}

#[test]
fn example_room_assets_check() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/sun-and-lamp.room");
    let out = livspeil(&["assets", "check", s(&example), "--strict"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("ok (1 asset, 0 warnings)"));
}

#[test]
fn add_replace_updates_a_listed_entry() {
    let tmp = TempDir::new("assets-replace");
    let room = tmp.room();
    let file = tmp.0.join("hello.txt");
    fs::write(&file, "hello world\n").unwrap();
    let add = |extra: &[&str]| {
        let mut args = vec!["assets", "add", s(&room), s(&file)];
        args.extend_from_slice(extra);
        livspeil(&args)
    };
    assert!(add(&["--license", "CC0-1.0"]).status.success());

    // Other data without --replace: refused, lock untouched.
    let lock = room.join("assets.lock");
    let before = fs::read_to_string(&lock).unwrap();
    let out = add(&["--license", "CC-BY-4.0", "--author", "Jane Doe"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("--replace"), "{}", stderr(&out));
    assert_eq!(fs::read_to_string(&lock).unwrap(), before);

    // With --replace: the entry is rewritten canonically, comments kept.
    fs::write(&lock, format!("// license header\n{before}")).unwrap();
    let out = add(&[
        "--replace",
        "--license",
        "CC-BY-4.0",
        "--author",
        "Jane Doe",
        "--source",
        "https://example.org/h",
    ]);
    assert!(out.status.success(), "{}", stderr(&out));
    let asset = format!("assets/{HELLO_SHA}.txt");
    assert!(stdout(&out).contains(&format!("replaced {asset}")));
    assert_eq!(
        fs::read_to_string(&lock).unwrap(),
        format!(
            "// license header\n\nassets-lock format=1 {{\n  asset \"{asset}\" sha256=\"{HELLO_SHA}\" size=12 license=\"CC-BY-4.0\" source=\"https://example.org/h\" author=\"Jane Doe\"\n}}\n"
        )
    );
    // A stale hash or size is repaired too.
    let text = fs::read_to_string(&lock)
        .unwrap()
        .replace("size=12", "size=13");
    fs::write(&lock, text).unwrap();
    assert_eq!(
        livspeil(&["assets", "check", s(&room)]).status.code(),
        Some(1)
    );
    let out = add(&[
        "--license",
        "CC-BY-4.0",
        "--author",
        "Jane Doe",
        "--source",
        "https://example.org/h",
        "--replace",
    ]);
    assert!(stdout(&out).contains("replaced"), "{}", stderr(&out));
    assert!(livspeil(&["assets", "check", s(&room)]).status.success());
    // Identical data with --replace: unchanged.
    let out = add(&[
        "--replace",
        "--license",
        "CC-BY-4.0",
        "--author",
        "Jane Doe",
        "--source",
        "https://example.org/h",
    ]);
    assert!(stdout(&out).contains("unchanged"));
}

#[test]
fn fmt_checks_and_rewrites_assets_lock() {
    let tmp = TempDir::new("assets-fmt");
    let room = tmp.room();
    let lock = room.join("assets.lock");
    let canonical = format!(
        "assets-lock format=1 {{\n  asset \"assets/a.txt\" sha256=\"{HELLO_SHA}\" size=12 license=\"MIT\" source=\"original\"\n}}\n"
    );
    fs::write(&lock, &canonical).unwrap();
    let out = livspeil(&["fmt", s(&room), "--check"]);
    assert!(out.status.success(), "{}", stderr(&out));

    // Not canonical (spacing, property order): --check fails and names it.
    let messy = format!(
        "\n// header   \nassets-lock format=1 {{\n    asset \"assets/a.txt\" size=12 sha256=\"{HELLO_SHA}\" source=\"original\" license=\"MIT\"\n}}"
    );
    fs::write(&lock, &messy).unwrap();
    let out = livspeil(&["fmt", s(&room.join("room.kdl")), "--check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("assets.lock: not in canonical form"));
    assert_eq!(fs::read_to_string(&lock).unwrap(), messy);

    // fmt rewrites it, keeping the header comment.
    let out = livspeil(&["fmt", s(&room)]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert!(stdout(&out).contains("assets.lock: formatted"));
    assert_eq!(
        fs::read_to_string(&lock).unwrap(),
        format!("// header\n\n{canonical}")
    );
    assert!(livspeil(&["fmt", s(&room), "--check"]).status.success());

    // A lock that does not parse is an error with its location.
    fs::write(&lock, "assets-lock format=1 {\n  bogus\n}\n").unwrap();
    let out = livspeil(&["fmt", s(&room), "--check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("assets.lock:2:"), "{}", stderr(&out));
}
