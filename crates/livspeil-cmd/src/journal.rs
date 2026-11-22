// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The append-only JSON Lines journal.

use std::io::{BufRead, Write};

use livspeil_graph::Graph;

use crate::{JournalEntry, JournalError};

/// Appends [`JournalEntry`]s to any writer, one JSON object per line.
#[derive(Debug)]
pub struct JournalWriter<W: Write> {
    out: W,
}

impl<W: Write> JournalWriter<W> {
    /// Write entries to `out`, after whatever it already holds.
    pub fn new(out: W) -> Self {
        JournalWriter { out }
    }

    /// Append one entry and a newline.
    ///
    /// # Errors
    /// [`JournalError::NonFiniteFloat`] for NaN or infinite values (JSON
    /// cannot hold them; nothing is written), or an encoding or I/O error.
    pub fn append(&mut self, entry: &JournalEntry) -> Result<(), JournalError> {
        if entry.command.has_non_finite() {
            return Err(JournalError::NonFiniteFloat);
        }
        // Encode first so a failed encoding never leaves half a line.
        let mut line = serde_json::to_vec(entry).map_err(JournalError::Encode)?;
        line.push(b'\n');
        self.out.write_all(&line)?;
        Ok(())
    }

    /// Flush the underlying writer.
    ///
    /// # Errors
    /// Any I/O error of the writer.
    pub fn flush(&mut self) -> Result<(), JournalError> {
        self.out.flush()?;
        Ok(())
    }

    /// The underlying writer.
    pub fn into_inner(self) -> W {
        self.out
    }
}

/// Read every entry of a journal. Blank lines are skipped.
///
/// # Errors
/// An I/O error, or [`JournalError::Parse`] naming the first bad line.
pub fn read_journal<R: BufRead>(input: R) -> Result<Vec<JournalEntry>, JournalError> {
    let mut entries = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry = serde_json::from_str(&line).map_err(|source| JournalError::Parse {
            line: index + 1,
            source,
        })?;
        entries.push(entry);
    }
    Ok(entries)
}

/// Apply every entry of a journal to `graph`, in order. Returns the number
/// of entries applied. Replaying onto the graph the journal started from
/// reproduces the state it ended in.
///
/// Entries are parsed and applied line by line. On error, the entries
/// before the failing line stay applied and the failing one changes
/// nothing; wrap the call around a fresh graph if you need all or nothing.
///
/// # Errors
/// An I/O error, [`JournalError::Parse`] or [`JournalError::Apply`].
pub fn replay<R: BufRead>(input: R, graph: &mut Graph) -> Result<usize, JournalError> {
    let mut applied = 0;
    for (index, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: JournalEntry =
            serde_json::from_str(&line).map_err(|source| JournalError::Parse {
                line: index + 1,
                source,
            })?;
        entry
            .command
            .apply(graph)
            .map_err(|source| JournalError::Apply {
                line: index + 1,
                source,
            })?;
        applied += 1;
    }
    Ok(applied)
}
