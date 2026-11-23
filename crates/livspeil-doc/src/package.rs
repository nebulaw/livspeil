// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Room packages on disk and whole-file canonicalization.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use crate::assets::{
    ASSETS_DIR, ASSETS_LOCK_FILE, AssetEntry, AssetIssue, AssetsError, AssetsLock, FileFacts,
    check_license, valid_asset_name, valid_source, verify,
};
use crate::error::DocError;
use crate::format::format;
use crate::parse::parse;

/// File name of the document inside a `Name.room/` package.
pub const ROOM_FILE: &str = "room.kdl";

/// The `room.kdl` to read for `path`: `path` itself if it is a file, or
/// `path/room.kdl` if it is a directory (a `Name.room/` package).
#[must_use]
pub fn room_file(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(ROOM_FILE)
    } else {
        path.to_path_buf()
    }
}

/// Split `text` into its prolog (the leading lines that are blank or `//`
/// comments, such as license headers) and the rest.
#[must_use]
pub fn split_prolog(text: &str) -> (&str, &str) {
    let mut end = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            end += line.len();
        } else {
            break;
        }
    }
    text.split_at(end)
}

/// The canonical form of a whole `room.kdl` file: the prolog comment lines
/// (trailing whitespace and surrounding blank lines removed, then one blank
/// line) followed by [`format()`] of the parsed document, with the
/// comments attached to its items (`//` lines directly above a node,
/// input, link, ... and `//` comments at the end of its line) carried over;
/// other comments are dropped (see `comments.rs` for the exact rules).
/// Idempotent: `canonicalize(canonicalize(t)) == canonicalize(t)`.
///
/// # Errors
/// Any error of [`parse`] or [`format()`].
pub fn canonicalize(text: &str) -> Result<String, DocError> {
    let (prolog, _) = split_prolog(text);
    let doc = parse(text)?;
    let mut out = canonical_prolog(prolog);
    out.push_str(&format(&doc)?);
    Ok(crate::comments::carry(text, &out))
}

/// The canonical form of a whole `assets.lock` file: the prolog comment
/// lines (as in [`canonicalize`]) followed by [`AssetsLock::format`] of the
/// parsed lock. This is what [`write_assets_lock`] writes. Idempotent.
///
/// # Errors
/// Any error of [`AssetsLock::parse`].
pub fn canonicalize_assets_lock(text: &str) -> Result<String, DocError> {
    let (prolog, _) = split_prolog(text);
    let lock = AssetsLock::parse(text)?;
    let mut out = canonical_prolog(prolog);
    out.push_str(&lock.format());
    Ok(out)
}

/// The package directory of `path`: `path` itself if it is a directory,
/// else the directory holding the `room.kdl` file.
#[must_use]
pub fn package_dir(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.to_path_buf()
    } else {
        match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }
}

fn io_error(path: &Path, err: &std::io::Error) -> AssetsError {
    AssetsError::Io {
        path: path.to_path_buf(),
        message: err.to_string(),
    }
}

/// SHA-256 (lowercase hex) and size of a file, read in chunks.
///
/// # Errors
/// [`AssetsError::Io`] if the file cannot be read.
pub fn sha256_file(path: &Path) -> Result<(String, u64), AssetsError> {
    let mut file = fs::File::open(path).map_err(|e| io_error(path, &e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let n = file.read(&mut buf).map_err(|e| io_error(path, &e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let digest = hasher.finalize();
    let hex = digest.iter().map(|b| format!("{b:02x}")).collect();
    Ok((hex, size))
}

/// Every entry of `dir/assets/` by file name: regular files with their
/// hash and size, anything else as [`FileFacts::NotAFile`]. Empty if the
/// directory does not exist.
///
/// # Errors
/// [`AssetsError::Io`] if `assets/` exists but cannot be listed.
pub fn scan_assets(dir: &Path) -> Result<BTreeMap<String, FileFacts>, AssetsError> {
    let assets = dir.join(ASSETS_DIR);
    let mut files = BTreeMap::new();
    let read = match fs::read_dir(&assets) {
        Ok(read) => read,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(files),
        Err(err) => return Err(io_error(&assets, &err)),
    };
    for entry in read {
        let entry = entry.map_err(|e| io_error(&assets, &e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let facts = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => match sha256_file(&path) {
                Ok((sha256, size)) => FileFacts::File { sha256, size },
                Err(err) => FileFacts::Unreadable(err.to_string()),
            },
            Ok(_) => FileFacts::NotAFile,
            Err(err) => FileFacts::Unreadable(err.to_string()),
        };
        files.insert(name, facts);
    }
    Ok(files)
}

/// Read and parse `dir/assets.lock`, with its text; `None` if it does not
/// exist.
///
/// # Errors
/// [`AssetsError::Io`] if it cannot be read, [`AssetsError::Lock`] if it
/// does not parse.
pub fn read_assets_lock(dir: &Path) -> Result<Option<(AssetsLock, String)>, AssetsError> {
    let file = dir.join(ASSETS_LOCK_FILE);
    let text = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_error(&file, &err)),
    };
    Ok(Some((AssetsLock::parse(&text)?, text)))
}

/// Verify the assets of the package in `dir`: `None` if it has neither
/// `assets.lock` nor `assets/`; otherwise the lock (empty if missing) and
/// the issues of [`verify`].
///
/// # Errors
/// See [`read_assets_lock`] and [`scan_assets`].
pub fn check_assets(dir: &Path) -> Result<Option<(AssetsLock, Vec<AssetIssue>)>, AssetsError> {
    let lock = read_assets_lock(dir)?;
    if lock.is_none() && !dir.join(ASSETS_DIR).exists() {
        return Ok(None);
    }
    let lock = lock.map(|(lock, _)| lock).unwrap_or_default();
    let files = scan_assets(dir)?;
    let issues = verify(&lock, &files);
    Ok(Some((lock, issues)))
}

/// Write `lock` canonically to `dir/assets.lock`, keeping the leading
/// comment lines (license headers) of the existing file.
///
/// # Errors
/// [`AssetsError::Io`] if the file cannot be read or written.
pub fn write_assets_lock(dir: &Path, lock: &AssetsLock) -> Result<(), AssetsError> {
    let file = dir.join(ASSETS_LOCK_FILE);
    let old = match fs::read_to_string(&file) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(io_error(&file, &err)),
    };
    let mut out = canonical_prolog(split_prolog(&old).0);
    out.push_str(&lock.format());
    fs::write(&file, out).map_err(|e| io_error(&file, &e))
}

/// What [`add_asset`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct AddedAsset {
    /// The entry as recorded.
    pub entry: AssetEntry,
    /// False if the identical entry was already there (nothing written)
    /// or an existing entry was replaced.
    pub added: bool,
    /// True if an existing entry with other data was replaced (only with
    /// [`AddMode::Replace`]).
    pub replaced: bool,
    /// License ids outside the allow-list (a warning, see
    /// [`check_license`]).
    pub unlisted_licenses: Vec<String>,
}

/// What [`add_asset_with`] does when the asset's path is already listed
/// with other data (another license, source or author, or a stale hash or
/// size).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AddMode {
    /// Refuse with [`AssetsError::Conflict`].
    #[default]
    Add,
    /// Replace the entry (hash, size, license, source, author).
    Replace,
}

/// Copy `file` into `dir/assets/<sha256>.<ext>` and record it in
/// `dir/assets.lock` (created if missing, written canonically). `ext` is
/// the file's extension in lower case, left out if it is not plain ASCII
/// letters and digits. Same as [`add_asset_with`] in [`AddMode::Add`].
///
/// # Errors
/// [`AssetsError::License`] for a license that [`check_license`] rejects
/// or a bad source or author, [`AssetsError::Conflict`] if the path is
/// already listed with other data, [`AssetsError::Lock`] if the existing
/// lock does not parse, [`AssetsError::Io`] for file errors.
pub fn add_asset(
    dir: &Path,
    file: &Path,
    license: &str,
    source: &str,
    author: Option<&str>,
) -> Result<AddedAsset, AssetsError> {
    add_asset_with(dir, file, license, source, author, AddMode::Add)
}

/// [`add_asset`], with `mode` choosing what happens when the path is
/// already listed with other data.
///
/// # Errors
/// As [`add_asset`]; [`AssetsError::Conflict`] for a listed path only in
/// [`AddMode::Add`].
pub fn add_asset_with(
    dir: &Path,
    file: &Path,
    license: &str,
    source: &str,
    author: Option<&str>,
    mode: AddMode,
) -> Result<AddedAsset, AssetsError> {
    let unlisted_licenses = check_license(license).map_err(|message| AssetsError::License {
        license: license.to_owned(),
        message,
    })?;
    if !valid_source(source) {
        return Err(AssetsError::Conflict {
            path: file.display().to_string(),
            message: format!(
                "source '{source}' must be \"original\" or a URL such as \"https://...\""
            ),
        });
    }
    if author.is_some_and(|a| a.trim().is_empty()) {
        return Err(AssetsError::Conflict {
            path: file.display().to_string(),
            message: "author must not be empty".into(),
        });
    }
    let (sha256, size) = sha256_file(file)?;
    let ext = file
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.bytes().all(|b| b.is_ascii_alphanumeric()));
    let name = match ext {
        Some(ext) => format!("{sha256}.{ext}"),
        None => sha256.clone(),
    };
    debug_assert!(valid_asset_name(&name));
    let path = format!("{ASSETS_DIR}/{name}");
    let entry = AssetEntry::new(
        path.clone(),
        sha256.clone(),
        size,
        license,
        source,
        author.map(str::to_owned),
    );

    let mut lock = read_assets_lock(dir)?
        .map(|(lock, _)| lock)
        .unwrap_or_default();
    let replaced = match lock.get(&path) {
        Some(existing) if *existing != entry => {
            if mode == AddMode::Add {
                return Err(AssetsError::Conflict {
                    path,
                    message: "already listed in assets.lock with other data; use --replace \
                              (AddMode::Replace) to update it"
                        .into(),
                });
            }
            true
        }
        _ => false,
    };
    let added = lock.get(&path).is_none();

    let assets = dir.join(ASSETS_DIR);
    fs::create_dir_all(&assets).map_err(|e| io_error(&assets, &e))?;
    let dest = assets.join(&name);
    let present = dest.is_file() && sha256_file(&dest).is_ok_and(|(h, _)| h == sha256);
    if !present {
        fs::copy(file, &dest).map_err(|e| io_error(&dest, &e))?;
    }
    if added {
        lock.assets.push(entry.clone());
    } else if replaced {
        for slot in lock.assets.iter_mut().filter(|a| a.path == path) {
            *slot = entry.clone();
        }
    }
    if added || replaced {
        write_assets_lock(dir, &lock)?;
    }
    Ok(AddedAsset {
        entry,
        added,
        replaced,
        unlisted_licenses,
    })
}

/// Prolog comment lines without trailing whitespace or surrounding blank
/// lines, followed by one blank line; empty if there are none.
fn canonical_prolog(prolog: &str) -> String {
    let lines: Vec<&str> = prolog.lines().map(str::trim_end).collect();
    let start = lines.iter().position(|l| !l.is_empty());
    let end = lines.iter().rposition(|l| !l.is_empty());
    let mut out = String::new();
    if let (Some(start), Some(end)) = (start, end) {
        for line in &lines[start..=end] {
            out.push_str(line);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prolog_is_leading_comments_only() {
        let text = "// a\n\n// b\nroom \"x\" format=1 // c\n";
        let (prolog, rest) = split_prolog(text);
        assert_eq!(prolog, "// a\n\n// b\n");
        assert!(rest.starts_with("room"));
    }

    #[test]
    fn canonicalize_normalizes_prolog_and_is_idempotent() {
        let text = "\n// SPDX   \n\n\nroom \"x\" format=1 { network \"/\" }";
        let once = canonicalize(text).unwrap();
        assert_eq!(
            once,
            "// SPDX\n\nroom \"x\" format=1 min-tier=\"T0\" {\n  network \"/\"\n}\n"
        );
        assert_eq!(canonicalize(&once).unwrap(), once);
    }
}
