// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil run`: scripts against rooms. Needs `python3` (>= 3.10);
//! skipped with a message otherwise.

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

fn python_available() -> bool {
    let program = std::env::var("LIVSPEIL_PYTHON").unwrap_or_else(|_| "python3".into());
    let ok = Command::new(&program)
        .args([
            "-c",
            "import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)",
        ])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("SKIPPED: {program} (Python >= 3.10) is not available");
    }
    ok
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// A temp copy of an example room, removed on drop.
struct TempRoom {
    dir: PathBuf,
    room: PathBuf,
}

impl TempRoom {
    fn copy(example: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("livspeil-cli-run-{}-{nanos}", std::process::id()));
        let room = dir.join(example);
        fs::create_dir_all(&room).unwrap();
        let src = repo().join("examples").join(example).join("room.kdl");
        fs::copy(src, room.join("room.kdl")).unwrap();
        TempRoom { dir, room }
    }

    fn script(&self, source: &str) -> PathBuf {
        let path = self.dir.join("script.py");
        fs::write(&path, source).unwrap();
        path
    }
}

impl Drop for TempRoom {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.dir).ok();
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn dim_lamps_saves_canonically_and_journals() {
    if !python_available() {
        return;
    }
    let tmp = TempRoom::copy("sun-and-lamp.room");
    let script = repo().join("examples/scripts/dim_lamps.py");
    let journal = tmp.dir.join("journal.jsonl");
    let before = fs::read_to_string(tmp.room.join("room.kdl")).unwrap();
    let out = livspeil(&[
        "run",
        s(&tmp.room),
        s(&script),
        "--grant",
        "graph.read,graph.write",
        "--save",
        "--journal",
        s(&journal),
    ]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", text(&out.stderr));
    assert!(
        stdout.contains("log info: dimming 2 lamp nodes by 20%"),
        "{stdout}"
    );
    assert!(
        stdout.contains("applied 1 command (1 undo step):"),
        "{stdout}"
    );
    assert!(
        stdout.contains("set_input lamp_intensity.out_max = 0.16"),
        "{stdout}"
    );
    assert!(
        stdout.contains("set_input lamp_color.c0 = oklch(0.28 0.08 260 1)"),
        "{stdout}"
    );
    assert!(stdout.contains("exited with status 0"), "{stdout}");

    let after = fs::read_to_string(tmp.room.join("room.kdl")).unwrap();
    assert_ne!(before, after);
    assert!(
        after.starts_with("// SPDX-FileCopyrightText"),
        "prolog kept"
    );
    assert!(after.contains("input \"out_max\" 0.16"), "{after}");
    assert!(after.contains("node \"sun\" x=600.0 y=0.0"), "meta kept");
    let check = livspeil(&["fmt", "--check", s(&tmp.room)]);
    assert!(check.status.success(), "{}", text(&check.stderr));

    let lines = fs::read_to_string(&journal).unwrap();
    assert_eq!(lines.lines().count(), 1);
    assert!(lines.contains(r#""origin":"script""#), "{lines}");
    assert!(lines.contains(r#""transaction""#), "{lines}");
}

#[test]
fn without_save_nothing_is_written() {
    if !python_available() {
        return;
    }
    let tmp = TempRoom::copy("sun-and-lamp.room");
    let script = repo().join("examples/scripts/dim_lamps.py");
    let before = fs::read_to_string(tmp.room.join("room.kdl")).unwrap();
    let out = livspeil(&[
        "run",
        s(&tmp.room),
        s(&script),
        "--grant",
        "graph.read,graph.write",
    ]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(
        fs::read_to_string(tmp.room.join("room.kdl")).unwrap(),
        before
    );
}

#[test]
fn sun_elevation_cooks_without_editing() {
    if !python_available() {
        return;
    }
    let script = repo().join("examples/scripts/sun_elevation.py");
    let room = repo().join("examples/sun-and-lamp.room");
    let out = livspeil(&["run", s(&room), s(&script), "--grant", "graph.cook"]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", text(&out.stderr));
    assert!(
        stdout.contains("log info: 06.00h  elevation   15.4 deg"),
        "{stdout}"
    );
    assert!(
        stdout.contains("log info: 12.00h  elevation   71.7 deg"),
        "{stdout}"
    );
    assert!(
        stdout.contains("applied 0 commands (0 undo steps)"),
        "{stdout}"
    );
}

#[test]
fn blink_watcher_reacts_to_events_and_ticks() {
    if !python_available() {
        return;
    }
    let script = repo().join("examples/scripts/blink_watcher.py");
    let room = repo().join("examples/blink.room");
    let out = livspeil(&[
        "run",
        s(&room),
        s(&script),
        "--grant",
        "graph.read",
        "--grant",
        "graph.write",
        "--ticks",
        "40",
        "--fps",
        "20",
    ]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", text(&out.stderr));
    assert!(stdout.contains("log info: softened the lamp"), "{stdout}");
    assert!(stdout.contains("log debug: frame 20 at 1.00 s"), "{stdout}");
    assert!(stdout.contains("log info: saw 4 blinks"), "{stdout}");
    assert!(stdout.contains("set_input brightness.a = 0.6"), "{stdout}");
}

#[test]
fn denied_capability_fails_the_script_and_saves_nothing() {
    if !python_available() {
        return;
    }
    let tmp = TempRoom::copy("sun-and-lamp.room");
    let script = repo().join("examples/scripts/dim_lamps.py");
    let before = fs::read_to_string(tmp.room.join("room.kdl")).unwrap();
    let out = livspeil(&[
        "run",
        s(&tmp.room),
        s(&script),
        "--grant",
        "graph.read",
        "--save",
    ]);
    assert_eq!(out.status.code(), Some(4));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("CapabilityDenied"), "{stdout}");
    assert!(text(&out.stderr).contains("not saved"));
    assert_eq!(
        fs::read_to_string(tmp.room.join("room.kdl")).unwrap(),
        before
    );
}

#[test]
fn script_timeout_exits_4() {
    if !python_available() {
        return;
    }
    let tmp = TempRoom::copy("sun-and-lamp.room");
    let script = tmp.script("import time, livspeil\nlivspeil.connect()\ntime.sleep(30)\n");
    let out = livspeil(&["run", s(&tmp.room), s(&script), "--timeout", "1"]);
    assert_eq!(out.status.code(), Some(4));
    assert!(
        text(&out.stderr).contains("run time limit"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn usage_errors() {
    let room = repo().join("examples/sun-and-lamp.room");
    for args in [
        vec!["run"],
        vec!["run", s(&room)],
        vec!["run", s(&room), "x.py", "--grant", "graph.everything"],
        vec!["run", s(&room), "x.py", "--ticks", "many"],
    ] {
        let out = livspeil(&args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
    let out = livspeil(&["run", s(&room), "/no/such/script.py"]);
    assert_eq!(out.status.code(), Some(1));
    let help = text(&livspeil(&["--help"]).stdout);
    assert!(help.contains("run <room> <script.py>"));
    assert!(help.contains("--grant CAP"));
}
