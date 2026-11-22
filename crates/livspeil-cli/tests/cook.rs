// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil cook`, including the Phase 0 acceptance test: the "sun and
//! lamp" example cooks deterministically and matches golden files (the same
//! files on macOS and Linux; see the rounding-boundary unit test in
//! `src/cook.rs` for why six decimals are platform independent).

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

fn example() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sun-and-lamp.room")
        .to_str()
        .unwrap()
        .to_owned()
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Run and require exit code 0 and an empty stderr.
fn ok(args: &[&str]) -> String {
    let out = livspeil(args);
    assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    assert!(out.stderr.is_empty(), "{}", stderr(&out));
    stdout(&out)
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
            "livspeil-cook-{tag}-{}-{nanos}",
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

/// `ratio = 1 / (frame - 1)`: fine at frame 0, division by zero at frame 1,
/// fine again from frame 2. `after = ratio + 0` depends on it.
const FAILING: &str = r#"room "fail" format=1 {
  network "/" {
    node "clock" type="core.time.clock@1" id="01920000-0000-7000-8000-000000000001"
    node "minus_one" type="core.math.sub@1" id="01920000-0000-7000-8000-000000000002" {
      input "b" 1.0
    }
    node "ratio" type="core.math.div@1" id="01920000-0000-7000-8000-000000000003" {
      input "a" 1.0
    }
    node "after" type="core.math.add@1" id="01920000-0000-7000-8000-000000000004"
    link from="clock.frame" to="minus_one.a"
    link from="minus_one.out" to="ratio.b"
    link from="ratio.out" to="after.a"
  }
}
"#;

/// No time dependency: everything is reused after the first frame.
const STATIC: &str = r#"room "static" format=1 {
  network "/" {
    node "a" type="core.math.add@1" id="01920000-0000-7000-8000-000000000001" {
      input "a" 1.5
      input "b" 2
    }
    node "b" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000002" {
      input "b" 2.0
    }
    node "flag" type="core.logic.compare@1" id="01920000-0000-7000-8000-000000000003" {
      input "op" ">"
    }
    node "label" type="core.text.format@1" id="01920000-0000-7000-8000-000000000004"
    link from="a.out" to="b.a"
    link from="b.out" to="flag.a"
  }
}
"#;

#[test]
fn acceptance_sun_and_lamp_text_golden() {
    let example = example();
    let args = ["cook", &example, "--frames", "600", "--every", "50"];
    let first = ok(&args);
    assert_eq!(first.lines().count(), 12 * 3);
    assert_golden(&golden_dir(), "sun-and-lamp.text", &first);
    // Deterministic across runs.
    assert_eq!(ok(&args), first);
}

#[test]
fn acceptance_sun_and_lamp_jsonl_golden() {
    let example = example();
    let out = ok(&[
        "cook", &example, "--frames", "600", "--every", "50", "--format", "jsonl",
    ]);
    assert_golden(&golden_dir(), "sun-and-lamp.jsonl", &out);
    let frames: Vec<serde_json::Value> = out
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames.len(), 12);
    for (i, frame) in frames.iter().enumerate() {
        assert_eq!(frame["frame"], serde_json::json!(i * 50));
        let outputs = frame["outputs"].as_object().unwrap();
        let names: Vec<&str> = outputs.keys().map(String::as_str).collect();
        assert_eq!(names, ["lamp_color", "lamp_intensity", "sun"]);
        assert_eq!(frame["outputs"]["lamp_color"]["out"]["color"][3], 1.0);
        assert!(frame.get("stats").is_none());
    }
    // 06:00 at frame 0; 18:00 (frame 300) is during sunset.
    assert_eq!(
        frames[0]["outputs"]["lamp_intensity"]["out"]["float"],
        0.792593
    );
    assert_eq!(frames[6]["time"], 5.0);
}

#[test]
fn every_prints_the_same_values_as_printing_all_frames() {
    let example = example();
    let all = ok(&["cook", &example, "--frames", "600"]);
    assert_eq!(all.lines().count(), 600 * 3);
    let sampled = ok(&["cook", &example, "--frames", "600", "--every", "50"]);
    let filtered: Vec<&str> = all
        .lines()
        .filter(|line| {
            let frame: u64 = line
                .strip_prefix("frame=")
                .and_then(|rest| rest.split(' ').next())
                .unwrap()
                .parse()
                .unwrap();
            frame % 50 == 0
        })
        .collect();
    assert_eq!(sampled.lines().collect::<Vec<_>>(), filtered);
    // The file path form gives the same output.
    let file = format!("{example}/room.kdl");
    assert_eq!(
        ok(&["cook", &file, "--frames", "600", "--every", "50"]),
        sampled
    );
}

#[test]
fn node_selects_any_nodes_sorted_by_name() {
    let example = example();
    let out = ok(&[
        "cook",
        &example,
        "--node",
        "time_of_day",
        "--node=clock",
        "--node",
        "time_of_day",
        "--frames",
        "2",
        "--fps",
        "4",
        "--start",
        "10",
    ]);
    // time 10 s * 8640 = one full day, so 06:00 again; 0.25 s later is 06:36.
    assert_eq!(
        out,
        "frame=0 time=10.000000 clock.frame=0 clock.time=10.000000 clock.dt=0.250000\n\
         frame=0 time=10.000000 time_of_day.phase=0.250000 time_of_day.hour=6.000000\n\
         frame=1 time=10.250000 clock.frame=1 clock.time=10.250000 clock.dt=0.250000\n\
         frame=1 time=10.250000 time_of_day.phase=0.275000 time_of_day.hour=6.600000\n"
    );
}

#[test]
fn default_cook_is_one_frame_at_time_zero() {
    let room = TempRoom::new("static", STATIC);
    let out = ok(&["cook", &room.path()]);
    assert_eq!(
        out,
        "frame=0 time=0.000000 flag.out=true\n\
         frame=0 time=0.000000 label.out=\"0.00\"\n"
    );
}

#[test]
fn stats_show_caching() {
    let room = TempRoom::new("stats", STATIC);
    let out = ok(&["cook", &room.path(), "--frames", "3", "--stats"]);
    let stats: Vec<&str> = out.lines().filter(|l| l.contains(" stats ")).collect();
    assert_eq!(
        stats,
        [
            "frame=0 time=0.000000 stats cooked=4 reused=0",
            "frame=1 time=0.016667 stats cooked=0 reused=4",
            "frame=2 time=0.033333 stats cooked=0 reused=4",
        ]
    );

    // In the acceptance scene the clock chain re-cooks every frame, but the
    // smoothsteps cut off propagation while their output stays flat.
    let example = example();
    let out = ok(&[
        "cook", &example, "--frames", "200", "--every", "100", "--stats", "--format", "jsonl",
    ]);
    let frames: Vec<serde_json::Value> = out
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        frames[0]["stats"],
        serde_json::json!({"cooked": 9, "reused": 9})
    );
    assert_eq!(
        frames[1]["stats"],
        serde_json::json!({"cooked": 6, "reused": 12})
    );
}

#[test]
fn node_errors_are_printed_and_exit_with_code_3() {
    let room = TempRoom::new("fail", FAILING);
    let path = room.path();
    let out = livspeil(&[
        "cook", &path, "--frames", "3", "--node", "ratio", "--node", "after",
    ]);
    assert_eq!(out.status.code(), Some(3), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 after.out=-1.000000\n\
         frame=0 time=0.000000 ratio.out=-1.000000\n\
         frame=1 time=0.016667 after.out=<error: upstream node 'ratio' failed: invalid input 'b': division by zero>\n\
         frame=1 time=0.016667 ratio.out=<error: invalid input 'b': division by zero>\n\
         frame=2 time=0.033333 after.out=1.000000\n\
         frame=2 time=0.033333 ratio.out=1.000000\n"
    );

    let out = livspeil(&["cook", &path, "--frames", "2", "--format", "jsonl"]);
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        stdout(&out),
        "{\"frame\":0,\"time\":0.0,\"outputs\":{\"after\":{\"out\":{\"float\":-1.0}}}}\n\
         {\"frame\":1,\"time\":0.016667,\"outputs\":{\"after\":{\"error\":\"upstream node 'ratio' failed: invalid input 'b': division by zero\"}}}\n"
    );

    // Errors only count on printed frames.
    let out = livspeil(&["cook", &path, "--frames", "3", "--every", "2"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 after.out=-1.000000\n\
         frame=2 time=0.033333 after.out=1.000000\n"
    );
}

#[test]
fn cook_usage_errors_exit_with_code_2() {
    let example = example();
    let e = example.as_str();
    for args in [
        &["cook"][..],
        &["cook", e, e],
        &["cook", e, "--frames"],
        &["cook", e, "--frames", "ten"],
        &["cook", e, "--every", "0"],
        &["cook", e, "--fps", "-60"],
        &["cook", e, "--start", "inf"],
        &["cook", e, "--format", "csv"],
        &["cook", e, "--frob"],
        &["cook", e, "--node", "nope"],
    ] {
        let out = livspeil(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert!(stderr(&out).contains("livspeil: cook: "), "{args:?}");
    }
    let out = livspeil(&["cook", e, "--node", "nope"]);
    assert!(stderr(&out).contains("no node named 'nope'"));
}

#[test]
fn cook_load_errors_exit_with_code_1() {
    let bad = TempRoom::new("bad", "room \"r\" format=1 {\n  network \"/\" {\n");
    let out = livspeil(&["cook", &bad.path()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("syntax error"), "{}", stderr(&out));

    let ghost = format!("{}/Ghost.room", bad.0.display());
    let out = livspeil(&["cook", &ghost]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("cannot read"), "{}", stderr(&out));
}

#[test]
fn cook_help_documents_options_and_exit_codes() {
    let out = ok(&["cook", "--help"]);
    for needle in [
        "--frames", "--fps", "--start", "--every", "--node", "--format", "--stats", "3 cook",
    ] {
        assert!(out.contains(needle), "help is missing '{needle}'");
    }
}

fn charm_demo() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/charm-demo.room")
        .to_str()
        .unwrap()
        .to_owned()
}

/// The Charm demo: one "Daylight" Charm instantiated twice with different
/// promoted parameters; the root `core.output` nodes are printed.
#[test]
fn charm_demo_text_golden() {
    let demo = charm_demo();
    let args = ["cook", &demo, "--frames", "600", "--every", "50"];
    let out = ok(&args);
    assert_eq!(out.lines().count(), 12 * 4);
    assert!(
        out.lines()
            .all(|l| l.contains("_color.value=") || l.contains("_intensity.value="))
    );
    assert_golden(&golden_dir(), "charm-demo.text", &out);
    assert_eq!(ok(&args), out);
}

/// Without `core.output` nodes the sinks are the root nodes without
/// outgoing links (never subnets or nodes inside them); `--node` takes
/// paths into subnets.
#[test]
fn sinks_and_paths_with_subnets() {
    const ROOM: &str = r#"room "subnets" format=1 {
  network "/" {
    node "x" type="core.math.add@1" id="01920000-0000-7000-8000-000000000001" {
      input "a" 1.5
    }
    node "s" type="core.subnet@1" id="01920000-0000-7000-8000-000000000002" {
      network {
        node "in" type="core.subnet.input@1" id="01920000-0000-7000-8000-000000000003" port-type="Float"
        node "double" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000004" {
          input "b" 2.0
        }
        node "out" type="core.output@1" id="01920000-0000-7000-8000-000000000005" port-type="Float"
        link from="double.out" to="out.value"
        link from="in.value" to="double.a"
      }
    }
    node "y" type="core.math.add@1" id="01920000-0000-7000-8000-000000000006"
    link from="s.out" to="y.a"
    link from="x.out" to="s.in"
  }
}
"#;
    let room = TempRoom::new("subnets", ROOM);
    let path = room.path();
    assert_eq!(
        ok(&["cook", &path]),
        "frame=0 time=0.000000 y.out=3.000000\n"
    );
    assert_eq!(
        ok(&["cook", &path, "--node", "s/double", "--node", "/s/in"]),
        "frame=0 time=0.000000 s/double.out=3.000000\n\
         frame=0 time=0.000000 s/in.value=1.500000\n"
    );
}
