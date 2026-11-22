// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Room packages on disk and whole-file canonicalization.

use std::path::{Path, PathBuf};

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
/// line) followed by [`format()`] of the parsed document. Comments anywhere
/// else are dropped. Idempotent: `canonicalize(canonicalize(t)) ==
/// canonicalize(t)`.
///
/// # Errors
/// Any error of [`parse`] or [`format()`].
pub fn canonicalize(text: &str) -> Result<String, DocError> {
    let (prolog, _) = split_prolog(text);
    let doc = parse(text)?;
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
    out.push_str(&format(&doc)?);
    Ok(out)
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
