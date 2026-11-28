// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The rooms folder: what is listed, what a typed path means and what is
//! suggested.

use std::fs;
use std::path::{Path, PathBuf};

use livspeil_dream::rooms::{at_path, is_path, list, resolve, suggestion};

/// A rooms folder for one test with rooms `Beta` and `alpha`, a folder that
/// is not a room, and `more/gamma`.
fn folder(tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("rooms-{tag}"));
    let _ = fs::remove_dir_all(&dir);
    for room in ["Beta.room", "alpha.room", "more/gamma.room"] {
        fs::create_dir_all(dir.join(room)).unwrap();
        fs::write(dir.join(room).join("room.kdl"), "").unwrap();
    }
    fs::create_dir_all(dir.join("empty.room")).unwrap();
    fs::create_dir_all(dir.join("notes")).unwrap();
    fs::write(dir.join("lone.kdl"), "").unwrap();
    dir
}

fn names(found: &[livspeil_dream::rooms::Found]) -> Vec<&str> {
    found.iter().map(|r| r.name.as_str()).collect()
}

#[test]
fn only_room_packages_are_listed_by_name() {
    let dir = folder("list");
    assert_eq!(names(&list(&dir)), ["alpha", "Beta"]);
    assert!(list(&dir.join("nothing")).is_empty());
}

#[test]
fn a_typed_path_leads_to_rooms() {
    let dir = folder("typed");
    assert!(is_path("more/") && is_path("~") && is_path("./x"));
    assert!(!is_path("alpha"));
    // A folder, and the start of a name in it.
    assert_eq!(names(&at_path("more/", &dir)), ["gamma"]);
    assert_eq!(names(&at_path("more/GA", &dir)), ["gamma"]);
    assert_eq!(names(&at_path("more/x", &dir)), Vec::<&str>::new());
    // A room itself, a lone file, and an absolute path.
    assert_eq!(
        at_path("more/gamma.room", &dir)[0].path,
        dir.join("more/gamma.room")
    );
    assert_eq!(names(&at_path("./lone.kdl", &dir)), ["lone"]);
    let absolute = format!("{}/", dir.display());
    assert_eq!(
        names(&at_path(&absolute, Path::new("/nowhere"))),
        ["alpha", "Beta"]
    );
    assert_eq!(resolve("  ", &dir), None);
    assert_eq!(resolve("x", &dir), Some(dir.join("x")));
}

#[test]
fn save_as_suggests_a_free_name_in_the_rooms_folder() {
    let dir = folder("suggest");
    assert_eq!(suggestion(None, &dir), dir.join("untitled.room"));
    let elsewhere = Path::new("/somewhere/sun.room/room.kdl");
    assert_eq!(suggestion(Some(elsewhere), &dir), dir.join("sun.room"));
    let taken = dir.join("alpha.room/room.kdl");
    assert_eq!(suggestion(Some(&taken), &dir), dir.join("alpha-2.room"));
    let lone = Path::new("/somewhere/scene.kdl");
    assert_eq!(suggestion(Some(lone), &dir), dir.join("scene.room"));
}
