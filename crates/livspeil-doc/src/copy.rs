// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Making a new room package, as "save as" does.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use crate::assets::{ASSETS_DIR, ASSETS_LOCK_FILE};
use crate::package::ROOM_FILE;

/// The extension of a room package directory.
pub const PACKAGE_EXTENSION: &str = "room";

/// Why a package was not created. Nothing is left behind.
#[derive(Debug)]
pub enum PackageError {
    /// Something is already there; it is not touched.
    Exists(PathBuf),
    /// The destination is inside the package being copied.
    Inside(PathBuf),
    /// The package holds something that is neither a file nor a directory,
    /// such as a symbolic link.
    NotAFile(PathBuf),
    /// Reading or writing failed.
    Io(PathBuf, io::Error),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PackageError::Exists(p) => write!(f, "{} already exists", p.display()),
            PackageError::Inside(p) => {
                write!(f, "{} is inside the room being copied", p.display())
            }
            PackageError::NotAFile(p) => write!(
                f,
                "{} is not a plain file or directory; not copying it",
                p.display()
            ),
            PackageError::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for PackageError {}

fn failed(path: &Path) -> impl FnOnce(io::Error) -> PackageError + '_ {
    |e| PackageError::Io(path.to_path_buf(), e)
}

/// Create the package directory `dest` with `room` as its `room.kdl`.
///
/// `source` is the room file this one is a copy of. What the room can read
/// beside it comes along: everything in its package when it is in a
/// `*.room/` directory, otherwise only `assets/` and `assets.lock`, since
/// the directory of a lone room file holds other things too.
///
/// The package is built beside `dest` and renamed into place, so `dest` is
/// complete or absent.
///
/// # Errors
/// [`PackageError`]; nothing is created then.
pub fn create_package(dest: &Path, room: &str, source: Option<&Path>) -> Result<(), PackageError> {
    if fs::symlink_metadata(dest).is_ok() {
        return Err(PackageError::Exists(dest.to_path_buf()));
    }
    let Some(name) = dest.file_name() else {
        return Err(PackageError::Io(
            dest.to_path_buf(),
            io::Error::new(io::ErrorKind::InvalidInput, "not a directory name"),
        ));
    };
    let parent = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if !parent.is_dir() {
        return Err(PackageError::Io(
            parent.to_path_buf(),
            io::Error::new(io::ErrorKind::NotFound, "no such directory"),
        ));
    }
    let copied = source.map(Copied::of);
    if let Some(Copied::All(from)) = &copied {
        let from = fs::canonicalize(from).map_err(failed(from))?;
        let into = fs::canonicalize(parent).map_err(failed(parent))?;
        if into.starts_with(&from) {
            return Err(PackageError::Inside(dest.to_path_buf()));
        }
    }
    let mut building = OsString::from(name);
    building.push(".saving");
    let building = parent.join(building);
    // Fails if it is there: it is not ours to remove.
    fs::create_dir(&building).map_err(failed(&building))?;
    let result = fill(&building, room, copied.as_ref())
        .and_then(|()| fs::rename(&building, dest).map_err(failed(dest)));
    if result.is_err() {
        // Nothing to do if even removing it fails.
        let _ = fs::remove_dir_all(&building);
    }
    result
}

/// What of the source package is copied.
enum Copied {
    All(PathBuf),
    Assets(PathBuf),
}

impl Copied {
    fn of(room_file: &Path) -> Copied {
        let dir = match room_file.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        };
        // `.` has no extension of its own; ask where it really is.
        let real = fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
        let package = real.extension().is_some_and(|e| e == PACKAGE_EXTENSION)
            && room_file.file_name().is_some_and(|n| n == ROOM_FILE);
        if package {
            Copied::All(dir)
        } else {
            Copied::Assets(dir)
        }
    }
}

fn fill(into: &Path, room: &str, copied: Option<&Copied>) -> Result<(), PackageError> {
    match copied {
        Some(Copied::All(from)) => copy_entries(from, into, &|name| name != ROOM_FILE)?,
        Some(Copied::Assets(from)) => copy_entries(from, into, &|name| {
            name == ASSETS_DIR || name == ASSETS_LOCK_FILE
        })?,
        None => {}
    }
    let file = into.join(ROOM_FILE);
    fs::File::create(&file)
        .and_then(|mut f| {
            f.write_all(room.as_bytes())?;
            f.sync_all()
        })
        .map_err(failed(&file))
}

/// Copy the entries of `from` that `wanted` names, with all they hold.
fn copy_entries(
    from: &Path,
    into: &Path,
    wanted: &dyn Fn(&str) -> bool,
) -> Result<(), PackageError> {
    for entry in fs::read_dir(from).map_err(failed(from))? {
        let entry = entry.map_err(failed(from))?;
        let name = entry.file_name();
        if !wanted(&name.to_string_lossy()) {
            continue;
        }
        let (source, target) = (entry.path(), into.join(&name));
        // Of the entry itself: a link is not followed out of the package.
        let kind = entry.file_type().map_err(failed(&source))?;
        if kind.is_dir() {
            fs::create_dir(&target).map_err(failed(&target))?;
            copy_entries(&source, &target, &|_| true)?;
        } else if kind.is_file() {
            fs::copy(&source, &target).map_err(failed(&source))?;
        } else {
            return Err(PackageError::NotAFile(source));
        }
    }
    Ok(())
}
