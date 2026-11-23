// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil fmt` keeps the comments attached to items (M0.18).

use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const MESSY: &str = "// Header\n\nroom \"r\" format=1 { network \"/\" {\n  // second node\n  node b type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\" {\n    input a 1 // one\n  }\n  node a type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\" // first\n  link from=\"a.out\" to=\"b.b\" /* gone */ } }\n";

const CANONICAL: &str = "// Header\n\nroom \"r\" format=1 min-tier=\"T0\" {\n  network \"/\" {\n    node \"a\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000001\" // first\n    // second node\n    node \"b\" type=\"core.math.add@1\" id=\"01920000-0000-7000-8000-000000000002\" {\n      input \"a\" 1 // one\n    }\n    link from=\"a.out\" to=\"b.b\"\n  }\n}\n";

#[test]
fn fmt_keeps_attached_comments() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let dir = std::env::temp_dir().join(format!(
        "livspeil-fmt-comments-{}-{nanos}",
        std::process::id()
    ));
    let pkg = dir.join("R.room");
    fs::create_dir_all(&pkg).unwrap();
    let file = pkg.join("room.kdl");
    fs::write(&file, MESSY).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_livspeil"))
            .args(args)
            .output()
            .unwrap()
    };
    let pkg_arg = pkg.to_str().unwrap();
    assert_eq!(run(&["fmt", "--check", pkg_arg]).status.code(), Some(1));
    let out = run(&["fmt", pkg_arg]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), CANONICAL);
    assert!(run(&["fmt", "--check", pkg_arg]).status.success());
    fs::remove_dir_all(&dir).ok();
}
