// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Error types of the command bus and the journal.

use std::fmt;

use livspeil_graph::GraphError;

/// Why a command, undo or redo did not apply.
///
/// Except for [`CmdError::RollbackFailed`], the graph is unchanged when a
/// command returns an error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CmdError {
    /// The graph rejected the edit.
    Graph(GraphError),
    /// Step `index` of a transaction failed; the earlier steps were rolled
    /// back.
    InTransaction { index: usize, source: Box<CmdError> },
    /// Step `index` of a transaction failed and rolling back the earlier
    /// steps failed too. The graph may be partially edited. This indicates
    /// a bug in an inverse and should never happen.
    RollbackFailed {
        index: usize,
        source: Box<CmdError>,
        rollback: Box<CmdError>,
    },
    /// The undo stack is empty.
    NothingToUndo,
    /// The redo stack is empty.
    NothingToRedo,
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CmdError::Graph(err) => err.fmt(f),
            CmdError::InTransaction { index, source } => {
                write!(f, "transaction step {index} failed: {source}")
            }
            CmdError::RollbackFailed {
                index,
                source,
                rollback,
            } => write!(
                f,
                "transaction step {index} failed ({source}) and rolling back failed: {rollback}"
            ),
            CmdError::NothingToUndo => f.write_str("nothing to undo"),
            CmdError::NothingToRedo => f.write_str("nothing to redo"),
        }
    }
}

impl std::error::Error for CmdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CmdError::Graph(err) => Some(err),
            CmdError::InTransaction { source, .. } | CmdError::RollbackFailed { source, .. } => {
                Some(source.as_ref())
            }
            CmdError::NothingToUndo | CmdError::NothingToRedo => None,
        }
    }
}

impl From<GraphError> for CmdError {
    fn from(err: GraphError) -> Self {
        CmdError::Graph(err)
    }
}

/// Why writing or replaying a journal failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum JournalError {
    /// Reading or writing failed.
    Io(std::io::Error),
    /// An entry could not be encoded.
    Encode(serde_json::Error),
    /// An entry holds a NaN or infinite float, which JSON cannot represent.
    NonFiniteFloat,
    /// Line `line` (1-based) is not a valid entry.
    Parse {
        line: usize,
        source: serde_json::Error,
    },
    /// The entry on line `line` (1-based) did not apply. Entries before it
    /// were applied; the failing entry left the graph unchanged.
    Apply { line: usize, source: CmdError },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::Io(err) => write!(f, "journal i/o failed: {err}"),
            JournalError::Encode(err) => write!(f, "could not encode journal entry: {err}"),
            JournalError::NonFiniteFloat => {
                f.write_str("journal entries cannot hold NaN or infinite floats")
            }
            JournalError::Parse { line, source } => {
                write!(f, "journal line {line} is not a valid entry: {source}")
            }
            JournalError::Apply { line, source } => {
                write!(f, "journal line {line} did not apply: {source}")
            }
        }
    }
}

impl std::error::Error for JournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            JournalError::Io(err) => Some(err),
            JournalError::Encode(err) | JournalError::Parse { source: err, .. } => Some(err),
            JournalError::Apply { source, .. } => Some(source),
            JournalError::NonFiniteFloat => None,
        }
    }
}

impl From<std::io::Error> for JournalError {
    fn from(err: std::io::Error) -> Self {
        JournalError::Io(err)
    }
}
