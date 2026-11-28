// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Where rooms live: the folder new ones go to, what is in it, and what a
//! typed path means.

use std::fs;
use std::path::{Path, PathBuf};

use livspeil_doc::{PACKAGE_EXTENSION, ROOM_FILE};

/// The environment variable that moves the rooms folder.
pub const HOME_VARIABLE: &str = "LIVSPEIL_ROOMS";
/// The rooms folder, in the home directory.
const FOLDER: &str = "Livspeil";

/// The user's home directory.
fn user_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The folder rooms are kept in: `$LIVSPEIL_ROOMS`, else `~/Livspeil`, else
/// (without a home directory) the current one.
#[must_use]
pub fn home() -> PathBuf {
    std::env::var_os(HOME_VARIABLE)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| user_home().map(|h| h.join(FOLDER)))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// What a typed path names: `~` is the user's home directory, and a relative
/// path is inside the rooms folder `rooms`. `None` for an empty one, or for
/// `~` without a home directory.
#[must_use]
pub fn resolve(typed: &str, rooms: &Path) -> Option<PathBuf> {
    let typed = typed.trim();
    if typed.is_empty() {
        return None;
    }
    match typed.strip_prefix('~') {
        Some("") => user_home(),
        Some(rest) if rest.starts_with('/') => {
            Some(user_home()?.join(rest.trim_start_matches('/')))
        }
        // `join` keeps an absolute path as it is.
        _ => Some(rooms.join(typed)),
    }
}

/// `path` with the user's home directory written `~`.
#[must_use]
pub fn shorten(path: &Path) -> String {
    match user_home().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// A room that can be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its folder's name without `.room` (a lone file's, without `.kdl`).
    pub name: String,
    /// What to open.
    pub path: PathBuf,
}

fn is_package(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == PACKAGE_EXTENSION) && path.join(ROOM_FILE).is_file()
}

fn found(path: PathBuf) -> Option<Found> {
    // Names that are not text could not be shown, or typed.
    path.to_str()?;
    let name = path.file_stem()?.to_str()?.to_owned();
    Some(Found { name, path })
}

/// The room packages in `dir`, by name. None if it cannot be read.
#[must_use]
pub fn list(dir: &Path) -> Vec<Found> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut rooms: Vec<Found> = entries
        .filter_map(|e| Some(e.ok()?.path()))
        .filter(|p| is_package(p))
        .filter_map(found)
        .collect();
    rooms.sort_by_cached_key(|r| (r.name.to_lowercase(), r.path.clone()));
    rooms
}

/// Whether a query is a path rather than part of a name.
#[must_use]
pub fn is_path(query: &str) -> bool {
    let query = query.trim();
    query.contains('/') || query.starts_with('~') || query.starts_with('.')
}

/// The rooms a typed path leads to: the room it names (a package or a room
/// file), else the rooms of the folder typed so far whose names start with
/// the rest.
#[must_use]
pub fn at_path(typed: &str, rooms: &Path) -> Vec<Found> {
    let Some(path) = resolve(typed, rooms) else {
        return Vec::new();
    };
    let named =
        is_package(&path) || (path.extension().is_some_and(|e| e == "kdl") && path.is_file());
    if named {
        return found(path).into_iter().collect();
    }
    let typed = typed.trim_end();
    let (dir, start) = if typed.ends_with('/') || typed == "~" {
        (path.as_path(), String::new())
    } else {
        let start = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        (path.parent().unwrap_or(&path), start)
    };
    list(dir)
        .into_iter()
        .filter(|r| r.name.to_lowercase().starts_with(&start))
        .collect()
}

/// Where "save as" suggests putting a room: in the rooms folder, under the
/// name of the folder it is in now (`untitled` without one), numbered if
/// that is taken.
#[must_use]
pub fn suggestion(file: Option<&Path>, rooms: &Path) -> PathBuf {
    let stem = file
        .and_then(|file| {
            let package = file
                .parent()
                .filter(|p| file.ends_with(ROOM_FILE) && p.extension().is_some());
            package.unwrap_or(file).file_stem()?.to_str()
        })
        .unwrap_or("untitled");
    let candidate = |suffix: &str| rooms.join(format!("{stem}{suffix}.{PACKAGE_EXTENSION}"));
    let mut path = candidate("");
    let mut n = 1_u32;
    while fs::symlink_metadata(&path).is_ok() {
        n += 1;
        path = candidate(&format!("-{n}"));
    }
    path
}
