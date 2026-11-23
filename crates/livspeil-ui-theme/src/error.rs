// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Config errors with source locations.

use std::fmt;
use std::ops::Range;

/// An error in a config file, located at a 1-based line and column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// File name used in messages (for example `motion.toml`).
    pub file: String,
    /// 1-based line of the offending value; 0 when unknown.
    pub line: usize,
    /// 1-based column (in characters); 0 when unknown.
    pub column: usize,
    /// Human-readable description.
    pub message: String,
}

impl ConfigError {
    /// An error at byte `span` of `source`.
    #[must_use]
    pub fn at(
        file: &str,
        source: &str,
        span: Option<Range<usize>>,
        message: impl Into<String>,
    ) -> Self {
        let (line, column) = span.map_or((0, 0), |s| line_col(source, s.start));
        ConfigError {
            file: file.to_owned(),
            line,
            column,
            message: message.into(),
        }
    }

    /// Wrap a TOML syntax or schema error.
    #[must_use]
    pub fn from_toml(file: &str, source: &str, err: &toml::de::Error) -> Self {
        Self::at(file, source, err.span(), err.message().trim().to_owned())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(
                f,
                "{}:{}:{}: {}",
                self.file, self.line, self.column, self.message
            )
        }
    }
}

impl std::error::Error for ConfigError {}

/// 1-based line and column (in characters) of byte `offset` in `source`.
#[must_use]
pub fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = source.get(..offset).unwrap_or(source);
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before.get(line_start..).map_or(0, |s| s.chars().count()) + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_is_one_based() {
        let src = "a = 1\nbb = 2\n";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 6), (2, 1));
        assert_eq!(line_col(src, 11), (2, 6));
    }
}
