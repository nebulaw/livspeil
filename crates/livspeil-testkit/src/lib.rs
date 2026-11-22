// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden-file harness.
//!
//! Golden files live next to the tests that use them, one `<name>.golden`
//! file per case. Run tests with `UPDATE_GOLDEN=1` to (re)write them, then
//! review the diff by hand: golden changes must never be committed unreviewed.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Why a golden comparison failed.
#[derive(Debug)]
pub enum GoldenError {
    /// The golden file does not exist yet.
    Missing { path: PathBuf },
    /// Contents differ; reports the first differing line (1-based).
    Mismatch {
        path: PathBuf,
        line: usize,
        expected: String,
        actual: String,
    },
    /// Reading or writing the golden file failed.
    Io { path: PathBuf, source: io::Error },
}

impl fmt::Display for GoldenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GoldenError::Missing { path } => write!(
                f,
                "golden file {} is missing; run with UPDATE_GOLDEN=1 to create it",
                path.display()
            ),
            GoldenError::Mismatch {
                path,
                line,
                expected,
                actual,
            } => write!(
                f,
                "golden mismatch in {} at line {line}\n  expected: {expected:?}\n    actual: {actual:?}\n\
                 run with UPDATE_GOLDEN=1 to accept, then review the diff",
                path.display()
            ),
            GoldenError::Io { path, source } => {
                write!(f, "I/O error on {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for GoldenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            GoldenError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Path of the golden file for `name` inside `dir`.
#[must_use]
pub fn golden_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.golden"))
}

/// Normalize line endings and guarantee a single trailing newline.
fn normalize(text: &str) -> String {
    let mut out = text.replace("\r\n", "\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Compare `actual` with the golden file, or rewrite it when `update` is set.
///
/// # Errors
/// Returns a [`GoldenError`] when the file is missing, differs, or cannot be
/// read or written.
pub fn check_golden(dir: &Path, name: &str, actual: &str, update: bool) -> Result<(), GoldenError> {
    let path = golden_path(dir, name);
    let actual = normalize(actual);

    if update {
        fs::create_dir_all(dir).map_err(|source| GoldenError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        return fs::write(&path, actual).map_err(|source| GoldenError::Io { path, source });
    }

    let expected = match fs::read_to_string(&path) {
        Ok(text) => normalize(&text),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(GoldenError::Missing { path });
        }
        Err(source) => return Err(GoldenError::Io { path, source }),
    };

    if expected == actual {
        return Ok(());
    }

    let mut expected_lines = expected.lines();
    let mut actual_lines = actual.lines();
    let mut line = 0;
    loop {
        line += 1;
        let (e, a) = (expected_lines.next(), actual_lines.next());
        if e != a {
            return Err(GoldenError::Mismatch {
                path,
                line,
                expected: e.unwrap_or("<end of file>").to_owned(),
                actual: a.unwrap_or("<end of file>").to_owned(),
            });
        }
    }
}

/// Panicking wrapper used by tests. Honors the `UPDATE_GOLDEN` environment
/// variable.
///
/// # Panics
/// Panics with a descriptive message when the comparison fails.
pub fn assert_golden(dir: &Path, name: &str, actual: &str) {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    if let Err(err) = check_golden(dir, name, actual, update) {
        panic!("{err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "livspeil-testkit-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn missing_golden_is_reported() {
        let dir = temp_dir("missing");
        let err = check_golden(&dir, "case", "hello", false).unwrap_err();
        assert!(matches!(err, GoldenError::Missing { .. }));
    }

    #[test]
    fn update_then_compare_succeeds() {
        let dir = temp_dir("update");
        check_golden(&dir, "case", "line one\nline two", true).unwrap();
        check_golden(&dir, "case", "line one\nline two\n", false).unwrap();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn mismatch_reports_first_differing_line() {
        let dir = temp_dir("mismatch");
        check_golden(&dir, "case", "a\nb\nc", true).unwrap();
        let err = check_golden(&dir, "case", "a\nX\nc", false).unwrap_err();
        match err {
            GoldenError::Mismatch {
                line,
                expected,
                actual,
                ..
            } => {
                assert_eq!(line, 2);
                assert_eq!(expected, "b");
                assert_eq!(actual, "X");
            }
            other => panic!("unexpected error: {other}"),
        }
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn line_endings_are_normalized() {
        let dir = temp_dir("crlf");
        check_golden(&dir, "case", "a\r\nb\r\n", true).unwrap();
        check_golden(&dir, "case", "a\nb\n", false).unwrap();
        fs::remove_dir_all(&dir).ok();
    }
}
