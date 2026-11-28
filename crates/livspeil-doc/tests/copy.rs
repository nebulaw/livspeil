// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Creating a room package: what comes along, and what is never touched.

use std::fs;
use std::path::{Path, PathBuf};

use livspeil_doc::{PackageError, create_package};

/// An empty directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("copy-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Everything under `dir`, as sorted relative paths.
fn listing(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            out.push(path.strip_prefix(root).unwrap().display().to_string());
            if path.is_dir() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn a_new_package_holds_only_the_room() {
    let dir = scratch("new");
    create_package(&dir.join("a.room"), "text", None).unwrap();
    assert_eq!(listing(&dir), ["a.room", "a.room/room.kdl"]);
    assert_eq!(read(&dir.join("a.room/room.kdl")), "text");
}

#[test]
fn a_package_comes_along_whole() {
    let dir = scratch("whole");
    let from = dir.join("from.room");
    for file in ["room.kdl", "assets.lock", "assets/a.png", "data/deep/x"] {
        write(&from.join(file), file);
    }
    let to = dir.join("to.room");
    create_package(&to, "changed", Some(&from.join("room.kdl"))).unwrap();
    assert_eq!(
        listing(&to),
        [
            "assets",
            "assets.lock",
            "assets/a.png",
            "data",
            "data/deep",
            "data/deep/x",
            "room.kdl"
        ]
    );
    assert_eq!(read(&to.join("room.kdl")), "changed");
    assert_eq!(read(&to.join("data/deep/x")), "data/deep/x");
    assert_eq!(read(&from.join("room.kdl")), "room.kdl");
}

#[test]
fn a_lone_room_file_brings_only_its_assets() {
    let dir = scratch("lone");
    for file in ["scene.kdl", "assets.lock", "assets/a.png", "private/notes"] {
        write(&dir.join("home").join(file), file);
    }
    let to = dir.join("to.room");
    create_package(&to, "text", Some(&dir.join("home/scene.kdl"))).unwrap();
    assert_eq!(
        listing(&to),
        ["assets", "assets.lock", "assets/a.png", "room.kdl"]
    );
}

#[test]
fn the_directory_it_goes_into_must_exist() {
    let dir = scratch("no-parent");
    let refused = create_package(&dir.join("missing/a.room"), "r", None).unwrap_err();
    let said = refused.to_string();
    assert!(said.ends_with("missing: no such directory"), "{said}");
    assert!(listing(&dir).is_empty());
}

#[test]
fn what_is_there_is_never_overwritten() {
    let dir = scratch("exists");
    write(&dir.join("a.room/room.kdl"), "mine");
    write(&dir.join("b.room"), "a file");
    for name in ["a.room", "b.room"] {
        let refused = create_package(&dir.join(name), "other", None);
        assert!(matches!(refused, Err(PackageError::Exists(_))), "{name}");
    }
    // Nor a leftover of the name it builds under.
    write(&dir.join("c.room.saving/room.kdl"), "left over");
    assert!(create_package(&dir.join("c.room"), "other", None).is_err());
    assert_eq!(read(&dir.join("a.room/room.kdl")), "mine");
    assert_eq!(read(&dir.join("b.room")), "a file");
    assert_eq!(read(&dir.join("c.room.saving/room.kdl")), "left over");
    assert!(!dir.join("c.room").exists());
}

#[test]
fn a_package_is_not_copied_into_itself() {
    let dir = scratch("inside");
    let from = dir.join("from.room");
    write(&from.join("room.kdl"), "r");
    let refused = create_package(&from.join("again.room"), "r", Some(&from.join("room.kdl")));
    assert!(matches!(refused, Err(PackageError::Inside(_))));
    assert_eq!(listing(&from), ["room.kdl"]);
}

#[cfg(unix)]
#[test]
fn a_link_in_the_package_stops_the_copy_and_leaves_nothing() {
    let dir = scratch("link");
    let from = dir.join("from.room");
    write(&from.join("room.kdl"), "r");
    write(&dir.join("secret"), "s");
    std::os::unix::fs::symlink(dir.join("secret"), from.join("link")).unwrap();
    let refused = create_package(&dir.join("to.room"), "r", Some(&from.join("room.kdl")));
    assert!(matches!(refused, Err(PackageError::NotAFile(_))));
    assert_eq!(
        listing(&dir),
        [
            "from.room",
            "from.room/link",
            "from.room/room.kdl",
            "secret"
        ]
    );
}
