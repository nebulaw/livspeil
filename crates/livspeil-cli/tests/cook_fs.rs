// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil cook` with file-system nodes (M0.8): the room package is the
//! file root by default, `--fs-root` overrides it, and nothing outside the
//! root can be listed.

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

/// `dir/R.room/room.kdl` with `R.room/books/{a,b}.pdf` and `notes.txt`,
/// plus `dir/other/books/c.pdf`; removed on drop.
struct Shelf(PathBuf);

impl Shelf {
    fn new(tag: &str, path: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "livspeil-cook-fs-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let pkg = dir.join("R.room");
        fs::create_dir_all(pkg.join("books")).unwrap();
        fs::create_dir_all(dir.join("other/books")).unwrap();
        fs::write(pkg.join("books/b.pdf"), b"bb").unwrap();
        fs::write(pkg.join("books/a.pdf"), b"a").unwrap();
        fs::write(pkg.join("books/notes.txt"), b"").unwrap();
        fs::write(dir.join("other/books/c.pdf"), b"ccc").unwrap();
        fs::write(pkg.join("room.kdl"), room(path)).unwrap();
        Shelf(dir)
    }

    fn room(&self) -> String {
        self.0.join("R.room").to_str().unwrap().to_owned()
    }

    fn other(&self) -> String {
        self.0.join("other").to_str().unwrap().to_owned()
    }
}

impl Drop for Shelf {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn room(path: &str) -> String {
    format!(
        r#"room "shelf" format=1 {{
  network "/" {{
    node "list" type="core.fs.list_dir@1" id="01920000-0000-7000-8000-000000000001" {{
      input "path" "{path}"
      input "pattern" "*.pdf"
    }}
    node "names" type="core.table.column@1" id="01920000-0000-7000-8000-000000000002"
    node "count" type="core.table.count@1" id="01920000-0000-7000-8000-000000000003"
    node "last" type="core.table.get@1" id="01920000-0000-7000-8000-000000000004" {{
      input "row" 1
      input "column" "path"
    }}
    link from="list.entries" to="names.table"
    link from="list.entries" to="count.table"
    link from="list.entries" to="last.table"
  }}
}}
"#
    )
}

#[test]
fn lists_the_room_package_by_default() {
    let shelf = Shelf::new("default", "books");
    let out = livspeil(&["cook", &shelf.room(), "--frames", "2", "--stats"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 count.count=2 count.columns=5\n\
         frame=0 time=0.000000 last.text=\"books/b.pdf\" last.number=0.000000 last.found=true\n\
         frame=0 time=0.000000 names.out=table[name:String][\"a.pdf\"; \"b.pdf\"]\n\
         frame=0 time=0.000000 stats cooked=4 reused=2\n\
         frame=1 time=0.016667 count.count=2 count.columns=5\n\
         frame=1 time=0.016667 last.text=\"books/b.pdf\" last.number=0.000000 last.found=true\n\
         frame=1 time=0.016667 names.out=table[name:String][\"a.pdf\"; \"b.pdf\"]\n\
         frame=1 time=0.016667 stats cooked=0 reused=6\n"
    );
    let out = livspeil(&[
        "cook",
        &shelf.room(),
        "--format",
        "jsonl",
        "--node",
        "names",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "{\"frame\":0,\"time\":0.0,\"outputs\":{\"names\":{\"out\":{\"table\":{\"columns\":[{\"name\":\"name\",\"type\":\"string\"}],\"rows\":[[\"a.pdf\"],[\"b.pdf\"]]}}}}}\n"
    );
}

#[test]
fn fs_root_overrides_the_package() {
    let shelf = Shelf::new("override", "books");
    let out = livspeil(&[
        "cook",
        &shelf.room(),
        "--fs-root",
        &shelf.other(),
        "--node",
        "names",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "frame=0 time=0.000000 names.out=table[name:String][\"c.pdf\"]\n"
    );
    let out = livspeil(&["cook", &shelf.room(), "--fs-root", "/definitely/not/here"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("file root /definitely/not/here"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn paths_outside_the_root_are_node_errors() {
    for (path, reason) in [
        ("../other/books", "leads outside the allowed file root"),
        ("/etc", "absolute paths are not allowed"),
        ("missing", "'missing' does not exist"),
    ] {
        let shelf = Shelf::new("outside", path);
        let out = livspeil(&["cook", &shelf.room(), "--node", "count", "--node", "list"]);
        assert_eq!(out.status.code(), Some(3), "{path}: {}", stderr(&out));
        let text = stdout(&out);
        assert!(
            text.contains("count.count=<error: upstream node 'list'"),
            "{text}"
        );
        assert!(
            text.contains("list.entries=<error: invalid input 'path'"),
            "{text}"
        );
        assert!(text.contains(reason), "{path}: {text}");
    }
}
