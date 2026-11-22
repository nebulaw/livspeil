// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Semantic diff: one test per kind of change, plus a golden file for the
//! fixture pair `tests/diff-fixtures/{before,after}.room`.

use std::fs;
use std::path::{Path, PathBuf};

use livspeil_doc::{Change, DocDiff, Document, Op, Subject, diff, parse, room_file};
use livspeil_testkit::assert_golden;

const BASE: &str = r#"room "R" format=1 min-tier="T0" {
  network "/" {
    node "a" type="core.math.add@1" id="01920000-0000-7000-8000-000000000001" {
      input "a" 1.0
      input "b" 2.0 expr="ch(\"b/a\")" {
        anim pre="loop" {
          key 0.0 0.0
          key 1.0 1.0 interp="step"
        }
      }
    }
    node "b" type="core.math.add@1" id="01920000-0000-7000-8000-000000000002" {
      input "a" "text"
    }
    link from="a.out" to="b.b"
  }
  meta {
    node "a" x=0.0 y=0.0
    node "b" x=100.0 y=0.0
  }
}
"#;

fn doc(text: &str) -> Document {
    parse(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn run(new: &str) -> DocDiff {
    diff(&doc(BASE), &doc(new))
}

fn lines(d: &DocDiff) -> Vec<String> {
    d.changes.iter().map(ToString::to_string).collect()
}

/// `BASE` with `from` replaced by `to` (which must occur).
fn edit(from: &str, to: &str) -> String {
    assert!(BASE.contains(from), "{from}");
    BASE.replace(from, to)
}

#[test]
fn equal_documents_and_formatting_do_not_matter() {
    assert!(run(BASE).is_empty());
    // Reordered, re-spaced text with the same content: no changes.
    let canonical = livspeil_doc::format(&doc(BASE)).unwrap();
    assert!(run(&canonical).is_empty());
}

#[test]
fn header_changes() {
    let new = edit(
        r#"room "R" format=1 min-tier="T0""#,
        r#"room "S" format=1 min-tier="T2""#,
    );
    assert_eq!(
        lines(&run(&new)),
        [
            r#"~ room name: "R" -> "S""#,
            r#"~ room min-tier: "T0" -> "T2""#
        ]
    );
}

#[test]
fn input_value_changed_added_removed() {
    let d = run(&edit("input \"a\" 1.0", "input \"a\" 0.5"));
    assert_eq!(lines(&d), [r#"~ node "a" input "a": 1.0 -> 0.5"#]);
    assert_eq!(d.changes[0].op, Op::Changed);
    assert_eq!(d.changes[0].old.as_deref(), Some("1.0"));
    assert_eq!(d.changes[0].new.as_deref(), Some("0.5"));

    let d = run(&edit(
        "input \"a\" \"text\"\n",
        "input \"a\" \"text\"\n      input \"b\" (color)\"oklch(0.5 0.1 30.0 1.0)\"\n",
    ));
    assert_eq!(
        lines(&d),
        [r#"+ node "b" input "b": (color)"oklch(0.5 0.1 30.0 1.0)""#]
    );
    let d = run(&edit("      input \"a\" 1.0\n", ""));
    assert_eq!(lines(&d), [r#"- node "a" input "a": 1.0"#]);
}

#[test]
fn value_type_change_is_shown_by_canonical_text() {
    // 1 (Int) and 1.0 (Float) are different values.
    let d = run(&edit("input \"a\" 1.0", "input \"a\" 1"));
    assert_eq!(lines(&d), [r#"~ node "a" input "a": 1.0 -> 1"#]);
    let d = run(&edit("input \"a\" \"text\"", "input \"a\" \"te\\\"xt\""));
    assert_eq!(lines(&d), [r#"~ node "b" input "a": "text" -> "te\"xt""#]);
}

#[test]
fn expressions() {
    let d = run(&edit(
        r#"expr="ch(\"b/a\")""#,
        r#"expr="ch(\"b/a\") + 1.0""#,
    ));
    assert_eq!(
        lines(&d),
        [r#"~ node "a" input "b" expr: "ch(\"b/a\")" -> "ch(\"b/a\") + 1.0""#]
    );
    assert_eq!(d.changes[0].subject.kind(), "expr");
    let d = run(&edit(r#" expr="ch(\"b/a\")""#, ""));
    assert_eq!(lines(&d), [r#"- node "a" input "b" expr: "ch(\"b/a\")""#]);
}

#[test]
fn animation_settings_and_keys() {
    let new = edit(
        "anim pre=\"loop\" {",
        "anim post=\"ping-pong\" offset=2.0 {",
    )
    .replace(
        "key 1.0 1.0 interp=\"step\"",
        "key 1.0 1.0\n          key 4.0 2.0 interp=\"spring\" frequency=2.0 damping=0.5",
    );
    let new = new.replace(
        "key 0.0 0.0",
        "key 0.0 0.5 interp=\"bezier\" tangent=\"explicit\" in=0.0 out=1.0",
    );
    assert_eq!(
        lines(&run(&new)),
        [
            r#"~ node "a" input "b" anim pre: "loop" -> "hold""#,
            r#"~ node "a" input "b" anim post: "hold" -> "ping-pong""#,
            r#"~ node "a" input "b" anim offset: 0.0 -> 2.0"#,
            r#"~ node "a" input "b" anim key 0.0: 0.0 -> 0.5 interp="bezier" tangent="explicit" in=0.0 out=1.0"#,
            r#"~ node "a" input "b" anim key 1.0: 1.0 interp="step" -> 1.0"#,
            r#"+ node "a" input "b" anim key 4.0: 2.0 interp="spring" frequency=2.0 damping=0.5"#,
        ]
    );
}

#[test]
fn whole_animation_added_or_removed_is_summarized() {
    let removed = edit(
        " {\n        anim pre=\"loop\" {\n          key 0.0 0.0\n          key 1.0 1.0 interp=\"step\"\n        }\n      }",
        "",
    );
    let d = run(&removed);
    assert_eq!(
        lines(&d),
        [r#"- node "a" input "b" anim: 2 keys 0.0..1.0 pre="loop""#]
    );
    let d = diff(&doc(&removed), &doc(BASE));
    assert_eq!(d.changes[0].op, Op::Added);
    assert_eq!(d.changes[0].subject.kind(), "anim");
}

#[test]
fn nodes_added_removed_renamed_retyped() {
    let new = edit(
        "    link from",
        "    node \"c\" type=\"core.math.mul@1\" id=\"01920000-0000-7000-8000-000000000003\" {\n      input \"b\" 3.0\n    }\n    link from",
    );
    assert_eq!(
        lines(&run(&new)),
        [
            r#"+ node "c" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003""#,
            r#"+ node "c" input "b": 3.0"#,
        ]
    );

    let d = diff(&doc(&new), &doc(BASE));
    assert_eq!(
        lines(&d),
        [r#"- node "c" type="core.math.mul@1" id="01920000-0000-7000-8000-000000000003""#]
    );

    let d = run(&edit(
        "node \"b\" type=\"core.math.add@1\"",
        "node \"b\" type=\"core.math.sub@1\"",
    ));
    assert_eq!(
        lines(&d),
        [r#"~ node "b" type: "core.math.add@1" -> "core.math.sub@1""#]
    );
    assert!(matches!(d.changes[0].subject, Subject::NodeType(_)));
}

#[test]
fn rename_is_one_change_even_with_links_meta_and_expressions() {
    let new = BASE
        .replace("node \"b\"", "node \"bee\"")
        .replace("b.b", "bee.b")
        .replace("ch(\\\"b/a\\\")", "ch(\\\"bee/a\\\")");
    let d = run(&new);
    assert_eq!(
        lines(&d),
        [
            r#"~ node "a" input "b" expr: "ch(\"b/a\")" -> "ch(\"bee/a\")""#,
            r#"~ node "bee" renamed: "b" -> "bee""#,
        ]
    );
}

#[test]
fn same_id_matches_even_if_everything_else_changed() {
    // A node with a new id but the old name is a removal plus an addition.
    let new = edit(
        "id=\"01920000-0000-7000-8000-000000000002\"",
        "id=\"01920000-0000-7000-8000-000000000009\"",
    );
    let l = lines(&run(&new));
    assert_eq!(
        l[0],
        r#"- node "b" type="core.math.add@1" id="01920000-0000-7000-8000-000000000002""#
    );
    assert_eq!(
        l[1],
        r#"+ node "b" type="core.math.add@1" id="01920000-0000-7000-8000-000000000009""#
    );
    assert_eq!(l[2], r#"+ node "b" input "a": "text""#);
    // The link names "b", which is now another node: removed and re-added.
    assert_eq!(l[3], r#"- link "a.out" -> "b.b""#);
    assert_eq!(l[4], r#"+ link "a.out" -> "b.b""#);
    // Meta is keyed by node too.
    assert!(l[5].starts_with("- meta node \"b\""));
    assert!(l[6].starts_with("+ meta node \"b\""));
}

#[test]
fn links_added_removed() {
    let d = run(&edit(
        "link from=\"a.out\" to=\"b.b\"",
        "link from=\"b.out\" to=\"a.a\"",
    ));
    assert_eq!(
        lines(&d),
        [r#"- link "a.out" -> "b.b""#, r#"+ link "b.out" -> "a.a""#]
    );
    assert_eq!(d.changes[0].subject.kind(), "link");
}

#[test]
fn meta_positions_are_reported_and_marked() {
    let d = run(&edit(
        "node \"b\" x=100.0 y=0.0",
        "node \"b\" x=100.0 y=50.0",
    ));
    assert_eq!(
        lines(&d),
        [r#"~ meta node "b" position: 100.0 0.0 -> 100.0 50.0"#]
    );
    assert!(d.changes[0].subject.is_meta());
    assert_eq!(d.without_meta().count(), 0);
    let d = run(&edit("    node \"b\" x=100.0 y=0.0\n", ""));
    assert_eq!(lines(&d), [r#"- meta node "b" position: 100.0 0.0"#]);
}

#[test]
fn reverse_diff_mirrors_links_and_values() {
    let before = load("before");
    let after = load("after");
    let forward = diff(&before, &after);
    assert_eq!(forward, diff(&before, &after), "deterministic");
    let backward = diff(&after, &before);
    let links = |d: &DocDiff| -> Vec<(Op, String)> {
        d.changes
            .iter()
            .filter(|c| c.subject.kind() == "link")
            // Each side names links with its own node names.
            .map(|c| (c.op, c.to_string()[2..].replace("light", "lamp")))
            .collect()
    };
    let flipped: Vec<(Op, String)> = links(&forward)
        .into_iter()
        .map(|(op, text)| {
            let op = match op {
                Op::Added => Op::Removed,
                Op::Removed => Op::Added,
                Op::Changed => Op::Changed,
            };
            (op, text)
        })
        .collect();
    let mut back = links(&backward);
    back.sort_by(|x, y| x.1.cmp(&y.1));
    let mut flipped = flipped;
    flipped.sort_by(|x, y| x.1.cmp(&y.1));
    assert_eq!(back, flipped);

    let value = |d: &DocDiff| -> Option<Change> {
        d.changes
            .iter()
            .find(|c| c.subject.kind() == "input" && c.subject.port() == Some("a"))
            .cloned()
    };
    let (f, b) = (value(&forward).unwrap(), value(&backward).unwrap());
    assert_eq!((f.old, f.new), (b.new, b.old));
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("tests/diff-fixtures/{name}.room"))
}

fn load(name: &str) -> Document {
    doc(&fs::read_to_string(room_file(&fixture(name))).unwrap())
}

#[test]
fn golden_fixture_diff() {
    let d = diff(&load("before"), &load("after"));
    let mut text = String::new();
    for change in &d.changes {
        text.push_str(&change.to_string());
        text.push('\n');
    }
    assert_golden(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden"),
        "diff_fixtures",
        &text,
    );
}
