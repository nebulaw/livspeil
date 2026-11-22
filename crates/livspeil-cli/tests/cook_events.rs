// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil cook` with events (M0.10): the blink example (goldens),
//! `--event` injection, event loops cut at the limit, failing handlers and
//! usage errors.

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

fn ok(args: &[&str]) -> String {
    let out = livspeil(args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    assert!(out.stderr.is_empty(), "{}", stderr(&out));
    stdout(&out)
}

fn blink() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/blink.room")
        .to_str()
        .unwrap()
        .to_owned()
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// A temp `Name.room` package, removed on drop.
struct TempRoom(PathBuf);

impl TempRoom {
    fn new(tag: &str, text: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "livspeil-cook-events-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let pkg = dir.join("R.room");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("room.kdl"), text).unwrap();
        TempRoom(dir)
    }

    fn path(&self) -> String {
        self.0.join("R.room").to_str().unwrap().to_owned()
    }
}

impl Drop for TempRoom {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

/// Four seconds at 20 fps: the lamp toggles every 0.5 s, the count is reset
/// at frame 30 and an extra blink is injected at frame 32.
const BLINK_ARGS: [&str; 10] = [
    "--frames",
    "80",
    "--fps",
    "20",
    "--every",
    "2",
    "--event",
    "blinks.reset@30",
    "--event",
    "blink.fired@32",
];

#[test]
fn blink_text_golden() {
    let blink = blink();
    let mut args = vec!["cook", blink.as_str()];
    args.extend(BLINK_ARGS);
    let out = ok(&args);
    assert_eq!(out.lines().count(), 40 * 4);
    assert!(out.contains("frame=0 time=0.000000 ticks.value=event(0)"));
    assert!(out.contains("frame=10 time=0.500000 changed.value=event(0.200000)"));
    assert!(out.contains("frame=32 time=1.600000 ticks.value=event\n"));
    assert_golden(&golden_dir(), "blink.text", &out);
    assert_eq!(ok(&args), out);
}

#[test]
fn blink_jsonl_golden() {
    let blink = blink();
    let mut args = vec!["cook", blink.as_str()];
    args.extend(BLINK_ARGS);
    args.extend(["--format", "jsonl", "--stats"]);
    let out = ok(&args);
    for line in out.lines() {
        let _: serde_json::Value = serde_json::from_str(line).unwrap();
    }
    assert!(out.contains(r#""ticks":{"value":{"event":[null]}}"#));
    assert_golden(&golden_dir(), "blink.jsonl", &out);
}

/// Without --event the run is the plain timer; frame 30 keeps counting.
#[test]
fn events_are_injected_only_where_asked() {
    let blink = blink();
    let plain = ok(&[
        "cook", &blink, "--frames", "31", "--fps", "20", "--every", "30",
    ]);
    assert!(
        plain.contains("frame=30 time=1.500000 count.value=4"),
        "{plain}"
    );
    let reset = ok(&[
        "cook",
        &blink,
        "--frames",
        "31",
        "--fps",
        "20",
        "--every",
        "30",
        "--event",
        "blinks.reset@30",
    ]);
    assert!(
        reset.contains("frame=30 time=1.500000 count.value=1"),
        "{reset}"
    );
}

/// A gate feeding itself: the loop is cut at the event limit every time.
#[test]
fn event_loops_report_overflow() {
    const LOOP: &str = r#"room "loop" format=1 {
  network "/" {
    node "gate" type="core.event.gate@1" id="01920000-0000-7000-8000-000000000001"
    node "count" type="core.event.counter@1" id="01920000-0000-7000-8000-000000000002"
    link from="gate.out" to="count.trigger"
    link from="gate.out" to="gate.trigger"
  }
}
"#;
    let room = TempRoom::new("loop", LOOP);
    let path = room.path();
    let out = ok(&["cook", &path, "--frames", "2", "--event", "gate.trigger@0"]);
    assert_eq!(
        out,
        "frame=0 time=0.000000 count.count=3333\n\
         frame=0 time=0.000000 events overflow processed=10000 dropped=1\n\
         frame=1 time=0.016667 count.count=3333\n"
    );
    let json = ok(&[
        "cook",
        &path,
        "--event",
        "gate.trigger@0",
        "--format",
        "jsonl",
    ]);
    assert_eq!(
        json,
        "{\"frame\":0,\"time\":0.0,\"outputs\":{\"count\":{\"count\":{\"int\":3333}}},\
         \"events\":{\"processed\":10000,\"dropped\":1}}\n"
    );
}

/// A handler error (a timer with period 0) is printed and exits with 3.
#[test]
fn failing_handlers_exit_3() {
    const BAD: &str = r#"room "bad" format=1 {
  network "/" {
    node "timer" type="core.event.every@1" id="01920000-0000-7000-8000-000000000001" {
      input "period" 0.0
    }
    node "count" type="core.event.counter@1" id="01920000-0000-7000-8000-000000000002"
    link from="timer.fired" to="count.trigger"
  }
}
"#;
    let room = TempRoom::new("bad", BAD);
    let path = room.path();
    let out = livspeil(&["cook", &path]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 count.count=0\n\
         frame=0 time=0.000000 event-error timer: invalid input 'period': must be positive, got 0\n"
    );
    let out = livspeil(&["cook", &path, "--format", "jsonl"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(stdout(&out).contains(
        r#""event_errors":[{"node":"timer","error":"invalid input 'period': must be positive, got 0"}]"#
    ));
}

#[test]
fn bad_event_arguments_are_usage_errors() {
    let blink = blink();
    for (event, message) in [
        ("nope.trigger@0", "no node named 'nope'"),
        ("blinks.count@0", "no Event port 'count'"),
        ("blinks.nope@0", "no Event port 'nope'"),
        ("blinks.reset", "NODE.PORT@FRAME"),
    ] {
        let out = livspeil(&["cook", &blink, "--event", event]);
        assert_eq!(out.status.code(), Some(2), "{event}");
        assert!(stderr(&out).contains(message), "{event}: {}", stderr(&out));
    }
}

#[test]
fn help_documents_events() {
    let out = ok(&["cook", "--help"]);
    assert!(out.contains("--event NODE.PORT@FRAME"));
}
