// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil cook` details of M0.18: event payloads on the command line,
//! event problems in frames that are not printed, `--precision` and
//! `--watch-fs`.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

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
    stdout(&out)
}

/// A temp `R.room` package, removed on drop.
struct TempRoom(PathBuf);

impl TempRoom {
    fn new(tag: &str, text: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "livspeil-cook-polish-{tag}-{}-{nanos}",
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

/// A latch (holds a Float payload) and an Event output (prints payloads).
const PAYLOADS: &str = r#"room "payloads" format=1 {
  network "/" {
    node "hold" type="core.event.latch@1" id="01920000-0000-7000-8000-000000000001"
    node "held" type="core.output@1" id="01920000-0000-7000-8000-000000000002" port-type="Float"
    node "ev" type="core.output@1" id="01920000-0000-7000-8000-000000000003" port-type="Event"
    link from="hold.out" to="held.value"
  }
}
"#;

#[test]
fn event_payloads() {
    let room = TempRoom::new("payloads", PAYLOADS);
    let path = room.path();
    let out = ok(&[
        "cook",
        &path,
        "--frames",
        "3",
        "--event",
        "hold.trigger@1=2.5",
        "--event",
        "ev.value@1=(vec3)\"1 2 3\"",
        "--event=ev.value@1=#true",
        "--event",
        "ev.value@2=\"a=b\"",
        "--event",
        "ev.value@2",
        "--event",
        "hold.trigger@2=4",
    ]);
    assert_eq!(
        out,
        "frame=0 time=0.000000 ev.value=none\n\
         frame=0 time=0.000000 held.value=0.000000\n\
         frame=1 time=0.016667 ev.value=event(vec3(1.000000 2.000000 3.000000)),event(true)\n\
         frame=1 time=0.016667 held.value=2.500000\n\
         frame=2 time=0.033333 ev.value=event(\"a=b\"),event\n\
         frame=2 time=0.033333 held.value=4.000000\n"
    );
    let json = ok(&[
        "cook",
        &path,
        "--format",
        "jsonl",
        "--event",
        "ev.value@0=(color)\"oklch(0.5 0.1 30)\"",
    ]);
    assert!(
        json.contains(r#""ev":{"value":{"event":[{"color":[0.5,0.1,30.0,1.0]}]}}"#),
        "{json}"
    );
}

#[test]
fn bad_payloads_are_usage_errors() {
    let room = TempRoom::new("bad-payloads", PAYLOADS);
    let path = room.path();
    for (event, message) in [
        ("hold.trigger@0=1 2", "invalid payload"),
        ("hold.trigger@0=(vec3)\"1\"", "needs 3 numbers"),
        ("hold.trigger@0=", "no value"),
    ] {
        let out = livspeil(&["cook", &path, "--event", event]);
        assert_eq!(out.status.code(), Some(2), "{event}");
        assert!(stderr(&out).contains(message), "{event}: {}", stderr(&out));
    }
}

fn sun_and_lamp() -> String {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sun-and-lamp.room")
        .to_str()
        .unwrap()
        .to_owned()
}

#[test]
fn precision() {
    let room = sun_and_lamp();
    let default = ok(&["cook", &room, "--frames", "2", "--node", "lamp_intensity"]);
    let six = ok(&[
        "cook",
        &room,
        "--frames",
        "2",
        "--node",
        "lamp_intensity",
        "--precision",
        "6",
    ]);
    assert_eq!(default, six);
    let two = ok(&[
        "cook",
        &room,
        "--frames",
        "2",
        "--node",
        "lamp_intensity",
        "--precision",
        "2",
    ]);
    assert_eq!(
        two,
        "frame=0 time=0.00 lamp_intensity.out=0.79\nframe=1 time=0.02 lamp_intensity.out=0.78\n"
    );
    let full = ok(&[
        "cook",
        &room,
        "--frames",
        "2",
        "--node",
        "lamp_intensity",
        "--precision",
        "full",
    ]);
    assert!(
        full.lines()
            .nth(1)
            .unwrap()
            .starts_with("frame=1 time=0.016666666666666666 "),
        "{full}"
    );
    let line = full.lines().next().unwrap();
    let value: f64 = line.rsplit_once('=').unwrap().1.parse().unwrap();
    assert!((value - 0.792593).abs() < 1e-6, "{line}");
    assert!(line.len() > "frame=0 time=0.0 lamp_intensity.out=0.792593".len());
    let json = ok(&[
        "cook",
        &room,
        "--frames",
        "2",
        "--node",
        "lamp_intensity",
        "--format",
        "jsonl",
        "--precision",
        "full",
    ]);
    let frame: serde_json::Value = serde_json::from_str(json.lines().nth(1).unwrap()).unwrap();
    assert_eq!(frame["time"].as_f64(), Some(1.0 / 60.0));
    let out = livspeil(&["cook", &room, "--precision", "99"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("--precision"), "{}", stderr(&out));
}

/// A timer whose period is 0 (a handler error) in frames 1 and 3 only.
const FLAKY: &str = r#"room "flaky" format=1 {
  network "/" {
    node "timer" type="core.event.every@1" id="01920000-0000-7000-8000-000000000001" {
      input "period" expr="if($F == 1 || $F == 3, 0.0, 1.0)"
    }
    node "count" type="core.event.counter@1" id="01920000-0000-7000-8000-000000000002"
    link from="timer.fired" to="count.trigger"
  }
}
"#;

/// A gate feeding itself: every tick with an event overflows.
const LOOP: &str = r#"room "loop" format=1 {
  network "/" {
    node "gate" type="core.event.gate@1" id="01920000-0000-7000-8000-000000000001"
    node "count" type="core.event.counter@1" id="01920000-0000-7000-8000-000000000002"
    link from="gate.out" to="count.trigger"
    link from="gate.out" to="gate.trigger"
  }
}
"#;

#[test]
fn skipped_frames_report_event_errors() {
    let room = TempRoom::new("flaky", FLAKY);
    let path = room.path();
    // Printed: 0 and 4; the errors are in the skipped frames 1 and 3.
    let out = livspeil(&["cook", &path, "--frames", "5", "--every", "4"]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 count.count=1\n\
         frames 1..3: 2 event errors (last: timer: invalid input 'period': must be positive, got 0)\n\
         frame=4 time=0.066667 count.count=1\n"
    );
    // Trailing skipped frames are summarized at the end; a single frame.
    let out = livspeil(&["cook", &path, "--frames", "2", "--every", "4"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 count.count=1\n\
         frame 1: 1 event error (last: timer: invalid input 'period': must be positive, got 0)\n"
    );
    let out = livspeil(&[
        "cook", &path, "--frames", "5", "--every", "4", "--format", "jsonl",
    ]);
    assert_eq!(out.status.code(), Some(3));
    let lines: Vec<serde_json::Value> = stdout(&out)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[1],
        serde_json::json!({"skipped": {"from": 1, "to": 3, "event_errors": 2,
            "last": {"node": "timer", "error": "invalid input 'period': must be positive, got 0"}}})
    );
    // Without errors in the run, nothing extra and exit 0.
    let out = ok(&["cook", &path, "--frames", "1"]);
    assert_eq!(out, "frame=0 time=0.000000 count.count=1\n");
}

#[test]
fn skipped_frames_report_overflows() {
    let room = TempRoom::new("loop", LOOP);
    let path = room.path();
    let out = ok(&[
        "cook",
        &path,
        "--frames",
        "11",
        "--every",
        "10",
        "--event",
        "gate.trigger@3",
        "--event",
        "gate.trigger@5",
    ]);
    assert_eq!(
        out,
        "frame=0 time=0.000000 count.count=0\n\
         frames 3..5: 2 event overflows (last: processed=10000 dropped=1)\n\
         frame=10 time=0.166667 count.count=6666\n"
    );
    let json = ok(&[
        "cook",
        &path,
        "--frames",
        "2",
        "--every",
        "10",
        "--event",
        "gate.trigger@1",
        "--format",
        "jsonl",
    ]);
    assert_eq!(
        json.lines().nth(1),
        Some(
            r#"{"skipped":{"from":1,"to":1,"overflows":1,"last":{"processed":10000,"dropped":1}}}"#
        )
    );
}
