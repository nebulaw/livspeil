// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The room as files: opening, saving and saving as.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};

use livspeil_doc::{
    Document, PACKAGE_EXTENSION, ROOM_FILE, create_package, package_dir, parse, rewrite, room_file,
};
use livspeil_graph::Graph;
use livspeil_node_api::SystemHost;
use livspeil_nodes_core::StdFs;

use super::{OpenError, Session};
use crate::catalog;

/// The file a session was opened from or saved to.
#[derive(Debug)]
pub(super) struct File {
    path: PathBuf,
    /// Its text when last read or written.
    text: String,
}

/// Write `text` to `path` without ever leaving a part of it there: write a
/// neighbor, then rename it over.
fn replace(path: &Path, text: &str) -> io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".saving");
    let temporary = path.with_file_name(name);
    let written = fs::File::create(&temporary).and_then(|mut f| {
        f.write_all(text.as_bytes())?;
        f.sync_all()
    });
    let result = written
        .and_then(|()| fs::set_permissions(&temporary, fs::metadata(path)?.permissions()))
        .and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        // Nothing to do if even removing it fails.
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// `path` as the package directory it names: `.room` is added if it is not
/// there.
fn package(path: &Path) -> Result<PathBuf, String> {
    let Some(name) = path.file_name() else {
        return Err(format!("{} does not name a room", path.display()));
    };
    if path.extension().is_some_and(|e| e == PACKAGE_EXTENSION) {
        return Ok(path.to_path_buf());
    }
    let mut name = name.to_owned();
    name.push(".");
    name.push(PACKAGE_EXTENSION);
    Ok(path.with_file_name(name))
}

/// Whether `file` is the room file of the package `dest`.
fn is_room_of(file: &Path, dest: &Path) -> bool {
    match (
        fs::canonicalize(file),
        fs::canonicalize(dest.join(ROOM_FILE)),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

impl Session {
    /// A new, empty room with no file yet.
    ///
    /// # Errors
    /// [`OpenError::Registry`] without the built-in node types.
    pub fn untitled() -> Result<Session, OpenError> {
        let graph = Graph::new(catalog::registry()?);
        Ok(Session::new("Untitled".to_owned(), graph, None))
    }

    /// The room file, if the room has one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.file.as_ref().map(|f| f.path.as_path())
    }

    /// Open `path`: a `Name.room/` package or a `room.kdl` file.
    ///
    /// # Errors
    /// [`OpenError`] when it cannot be read or is not a valid room.
    pub fn open(path: &Path) -> Result<Session, OpenError> {
        let file = room_file(path);
        let text = fs::read_to_string(&file).map_err(|e| OpenError::Read(file.clone(), e))?;
        let invalid = |e| OpenError::Document(file.clone(), Box::new(e));
        let doc = parse(&text).map_err(invalid)?;
        let graph = doc.to_graph(catalog::registry()?).map_err(invalid)?;
        // File nodes read inside the package, as `livspeil cook` has them.
        // Without a directory they report the lack themselves.
        let system = StdFs::new(package_dir(&file))
            .ok()
            .map(|fs| Arc::new(fs) as Arc<dyn SystemHost>);
        let mut session = Session::new(doc.name, graph, system);
        session.min_tier = doc.min_tier;
        session.file = Some(File { path: file, text });
        Ok(session)
    }

    /// Write the room back to its file, keeping its comments.
    ///
    /// # Errors
    /// When there is no file, it changed on disk since it was read, or it
    /// cannot be written. The file is then left as it was.
    pub fn save(&mut self) -> Result<(), String> {
        self.end_gesture();
        let Some(file) = &mut self.file else {
            return Err("this room has no file".to_owned());
        };
        let shown = file.path.display().to_string();
        let on_disk = fs::read_to_string(&file.path).map_err(|e| format!("{shown}: {e}"))?;
        if on_disk != file.text {
            return Err(format!("{shown} changed on disk; not overwriting it"));
        }
        let mut doc =
            Document::from_graph(&self.graph, self.name.clone()).map_err(|e| e.to_string())?;
        doc.min_tier = self.min_tier;
        let text = rewrite(&file.text, &doc).map_err(|e| e.to_string())?;
        replace(&file.path, &text).map_err(|e| format!("{shown}: {e}"))?;
        file.text = text;
        self.saved = self.graph.snapshot();
        self.dirty = false;
        Ok(())
    }

    /// Save the room as a new package at `path` (`.room` is added to it if
    /// missing), which the room is saved to from then on. What the room reads beside it comes along. A
    /// room that never had a file takes the package's name.
    ///
    /// # Errors
    /// When something else is already there, or it cannot be written. The
    /// room then keeps its file.
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        self.end_gesture();
        let dest = package(path)?;
        if self.path().is_some_and(|file| is_room_of(file, &dest)) {
            return self.save();
        }
        let name = match (&self.file, dest.file_stem()) {
            (None, Some(stem)) => stem.to_string_lossy().into_owned(),
            _ => self.name.clone(),
        };
        let mut doc = Document::from_graph(&self.graph, name.clone()).map_err(|e| e.to_string())?;
        doc.min_tier = self.min_tier;
        let previous = self.file.as_ref().map_or("", |f| f.text.as_str());
        let text = rewrite(previous, &doc).map_err(|e| e.to_string())?;
        create_package(&dest, &text, self.path()).map_err(|e| e.to_string())?;
        // File nodes read inside the new package from now on.
        self.system = StdFs::new(dest.clone())
            .ok()
            .map(|fs| Arc::new(fs) as Arc<dyn SystemHost>);
        self.cooker.set_system(self.system.clone());
        self.file = Some(File {
            path: dest.join(ROOM_FILE),
            text,
        });
        self.name = name;
        self.saved = self.graph.snapshot();
        self.dirty = false;
        self.cook();
        Ok(())
    }
}
