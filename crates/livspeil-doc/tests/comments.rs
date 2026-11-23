// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Comments carried through `canonicalize` (M0.18): comments attached to
//! nodes, inputs, links and the other items move with them; the rest is
//! dropped; the output is otherwise the canonical form.

use std::fs;
use std::path::{Path, PathBuf};

use livspeil_doc::{canonicalize, format, parse, room_file};
use livspeil_testkit::assert_golden;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// Out of canonical order, oddly indented, with every comment form.
const COMMENTED: &str = r#"// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: CC0-1.0

// The prolog stays as it is.
room "Commented" format=1 min-tier="T1" {
    // About the meta block (moves after the network).
    meta {
        node "sun" x=120.0 y=40.0 // sun position
        // lamp position
        node "lamp" x=360.0 y=40.0
    }
    // The root network.
    network "/" {
        // Links are sorted; their comments follow them.
        link from="sun.phase" to="lamp.t" // phase drives the mix

        // Separated by a blank line: dropped.

        // The lamp mixes two colors.
        // (second line)
        node "lamp" type="core.color.mix@1" id="0192F5A0-7C3E-7A10-8000-000000000002" {   // trailing on a block
            //b comes after a
            input "b" (color)"oklch(0.35 0.08 260)"
            input "a" (color)"oklch(0.8 0.12 60 1)" // alpha written explicitly
            /* block comments are dropped */
            // after the last item: dropped
        }
        /-node "disabled" type="core.math.add@1" id="0192f5a0-7c3e-7a10-8000-0000000000ff" // dropped with it
        node "sun" type="core.time.timeofday@1" id="0192f5a0-7c3e-7a10-8000-000000000001" {
            input "offset" 6.0 expr="$T / 3600.0" { // anim block next
                anim post="loop" {
                    // keys: dropped
                    key 0.0 6.0 // dropped
                    key 1.0 7.0
                }
            }
        }
        node "desk" type="core.subnet@1" id="0192f5a0-7c3e-7a10-8000-000000000010" {
            // The Charm header.
            charm "Desk" version="1.0.0" {
                capability "time" // dropped
            }
            // Inside the subnet.
            network {
                node "out" type="core.output@1" id="0192f5a0-7c3e-7a10-8000-000000000012" port-type="Float" // an output
                // The input.
                node "in" type="core.subnet.input@1" id="0192f5a0-7c3e-7a10-8000-000000000011" port-type="Float" {
                    input "value" 1.0 // default
                }
                link from="in.value" to="out.value" // inner link
            }
        }
    }
}
"#;

/// Remove every line that is only a comment and every ` //` comment at
/// the end of a line (enough for the files in these tests).
fn strip_comments(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .map(|l| l.split_once(" //").map_or(l, |(code, _)| code))
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn commented_room_golden() {
    let out = canonicalize(COMMENTED).unwrap();
    assert_golden(&golden_dir(), "commented_room", &out);
    // Idempotent.
    assert_eq!(canonicalize(&out).unwrap(), out);
    // Same document, and canonical apart from the comments.
    let doc = parse(COMMENTED).unwrap();
    assert_eq!(
        format(&parse(&out).unwrap()).unwrap(),
        format(&doc).unwrap()
    );
    let body = strip_comments(&out);
    assert_eq!(body.trim_start(), format(&doc).unwrap());
    for kept in [
        "// The lamp mixes two colors.",
        "// phase drives the mix",
        "// trailing on a block",
        "// alpha written explicitly",
        "// The Charm header.",
        "// an output",
        "// inner link",
        "// sun position",
    ] {
        assert!(out.contains(kept), "{kept}");
    }
    for dropped in [
        "blank line",
        "block comments",
        "after the last item",
        "dropped",
        "keys:",
    ] {
        assert!(!out.contains(dropped), "{dropped}\n{out}");
    }
}

#[test]
fn rooms_without_attached_comments_are_unchanged() {
    for room in ["sun-and-lamp", "charm-demo", "blink", "out-demo"] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples")
            .join(format!("{room}.room"));
        let text = fs::read_to_string(room_file(&path)).unwrap();
        assert_eq!(canonicalize(&text).unwrap(), text, "{room}");
    }
}

/// The format 1 fixture (4-space indent, bare names, unsorted): its two
/// trailing comments survive.
#[test]
fn format_1_fixture_keeps_its_trailing_comments() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format-1/sun-and-lamp.room");
    let out = canonicalize(&fs::read_to_string(room_file(&path)).unwrap()).unwrap();
    assert!(
        out.contains(
            "      input \"a\" (color)\"oklch(0.8 0.12 60.0 1.0)\" // alpha written explicitly\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("      input \"offset\" 1 // an Int on a Float port is coerced on load\n"),
        "{out}"
    );
    assert_eq!(canonicalize(&out).unwrap(), out);
}

/// Comments move with their nodes when canonical order (by id) moves
/// them.
#[test]
fn comments_follow_items_when_the_order_changes() {
    let text = "room \"r\" format=1 {\n  network \"/\" {\n    // second\n    node \"b\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\"\n    // first\n    node \"a\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\" // a\n  }\n}\n";
    assert_eq!(
        canonicalize(text).unwrap(),
        "room \"r\" format=1 min-tier=\"T0\" {\n  network \"/\" {\n    // first\n    node \"a\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\" // a\n    // second\n    node \"b\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\"\n  }\n}\n"
    );
}
