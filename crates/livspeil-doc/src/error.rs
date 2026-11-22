// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Errors of the document layer.

use std::fmt;

use livspeil_graph::GraphError;

/// A 1-based line and column (in characters) inside `room.kdl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Why a document could not be parsed, migrated, built from a graph or
/// turned into a graph.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DocError {
    /// The text is not valid KDL v2.
    Syntax { at: Location, message: String },
    /// The text is valid KDL but not a valid Livspeil document (unknown
    /// node, missing property, bad value, duplicate name, ...).
    Invalid {
        at: Option<Location>,
        message: String,
    },
    /// The document was written by a newer (or never released) format.
    UnsupportedFormat { found: i128, newest: u32 },
    /// A registered migration failed.
    Migration { from: u32, message: String },
    /// A graph input holds `NaN` or an infinity, which documents reject.
    NonFinite { node: String, port: String },
    /// Building the graph failed: unknown node type, unknown port, bad
    /// link, type mismatch, cycle, ...
    Graph {
        at: Option<Location>,
        /// What was being built, for example `node 'lamp' input 't'`.
        context: String,
        source: GraphError,
    },
}

impl DocError {
    /// Where in the text the problem is, when known.
    #[must_use]
    pub fn location(&self) -> Option<Location> {
        match self {
            DocError::Syntax { at, .. } => Some(*at),
            DocError::Invalid { at, .. } | DocError::Graph { at, .. } => *at,
            _ => None,
        }
    }

    pub(crate) fn invalid(at: Option<Location>, message: impl Into<String>) -> Self {
        DocError::Invalid {
            at,
            message: message.into(),
        }
    }
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(at) = self.location() {
            write!(f, "{at}: ")?;
        }
        match self {
            DocError::Syntax { message, .. } => write!(f, "syntax error: {message}"),
            DocError::Invalid { message, .. } => f.write_str(message),
            DocError::UnsupportedFormat { found, newest } => write!(
                f,
                "document format {found} is not supported (this build reads formats 1 to {newest})"
            ),
            DocError::Migration { from, message } => {
                write!(f, "migrating from format {from} failed: {message}")
            }
            DocError::NonFinite { node, port } => write!(
                f,
                "node '{node}' input '{port}' is NaN or infinite; documents only store finite numbers"
            ),
            DocError::Graph {
                context, source, ..
            } => write!(f, "{context}: {source}"),
        }
    }
}

impl std::error::Error for DocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DocError::Graph { source, .. } => Some(source),
            _ => None,
        }
    }
}
