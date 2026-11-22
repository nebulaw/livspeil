// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Format migrations.
//!
//! Promise: a document written by any released format version loads
//! forever. When the format changes incompatibly, bump
//! [`CURRENT_FORMAT`], append a [`Migration`] from the previous version to
//! [`MIGRATIONS`], and add a fixture of the old format under
//! `crates/livspeil-doc/tests/fixtures/format-<n>/`. Fixtures are never
//! edited or deleted.
//!
//! Additive changes do not bump the format: new syntax that older files
//! cannot contain (because the strict reader rejected it before) keeps the
//! version, as long as every existing file still parses to the same
//! document. Expressions and animation channels on inputs were added to
//! format 1 this way.
//!
//! Migrations work on the raw KDL tree of the `room` node (before it is
//! interpreted), so an old format does not need to be representable in the
//! current [`crate::Document`] model. Each migration upgrades by exactly one
//! version; [`migrate`] chains them.

use kdl::{KdlNode, KdlValue};

use crate::error::DocError;
use crate::model::CURRENT_FORMAT;

/// One upgrade step from format `from` to `from + 1`.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub from: u32,
    /// Rewrite the `room` node in place. Returns a reason on failure.
    pub apply: fn(&mut KdlNode) -> Result<(), String>,
}

/// Every registered migration, in order. Empty while only format 1 exists.
pub const MIGRATIONS: &[Migration] = &[];

/// Upgrade the `room` node from format `from` to [`CURRENT_FORMAT`] and set
/// its `format` property accordingly.
///
/// # Errors
/// [`DocError::UnsupportedFormat`] when `from` is newer than this build or
/// no migration path exists, [`DocError::Migration`] when a step fails.
pub fn migrate(room: &mut KdlNode, from: u32) -> Result<(), DocError> {
    migrate_with(room, from, MIGRATIONS, CURRENT_FORMAT)
}

/// [`migrate`] with an explicit migration list and target (for tests).
///
/// # Errors
/// Same as [`migrate`].
pub fn migrate_with(
    room: &mut KdlNode,
    from: u32,
    migrations: &[Migration],
    target: u32,
) -> Result<(), DocError> {
    if from == 0 || from > target {
        return Err(DocError::UnsupportedFormat {
            found: i128::from(from),
            newest: target,
        });
    }
    let mut version = from;
    while version < target {
        let step =
            migrations
                .iter()
                .find(|m| m.from == version)
                .ok_or(DocError::UnsupportedFormat {
                    found: i128::from(from),
                    newest: target,
                })?;
        (step.apply)(room).map_err(|message| DocError::Migration {
            from: version,
            message,
        })?;
        version += 1;
        if let Some(format) = room.entry_mut("format") {
            *format.value_mut() = KdlValue::Integer(i128::from(version));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use kdl::KdlDocument;

    fn rename_room(room: &mut KdlNode) -> Result<(), String> {
        room.entries_mut()
            .iter_mut()
            .find(|e| e.name().is_none())
            .map(|e| *e.value_mut() = KdlValue::String("renamed".into()))
            .ok_or_else(|| "no name".to_owned())
    }

    fn fail(_: &mut KdlNode) -> Result<(), String> {
        Err("boom".into())
    }

    fn room() -> KdlNode {
        KdlDocument::parse_v2("room \"old\" format=1")
            .unwrap()
            .nodes()[0]
            .clone()
    }

    #[test]
    fn chains_migrations_and_bumps_format() {
        let mut node = room();
        let steps = [
            Migration {
                from: 1,
                apply: rename_room,
            },
            Migration {
                from: 2,
                apply: |_| Ok(()),
            },
        ];
        migrate_with(&mut node, 1, &steps, 3).unwrap();
        assert_eq!(node.get("format"), Some(&KdlValue::Integer(3)));
        assert_eq!(node.get(0), Some(&KdlValue::String("renamed".into())));
    }

    #[test]
    fn missing_step_and_failures_are_errors() {
        let mut node = room();
        assert!(matches!(
            migrate_with(&mut node, 1, &[], 2),
            Err(DocError::UnsupportedFormat {
                found: 1,
                newest: 2
            })
        ));
        let steps = [Migration {
            from: 1,
            apply: fail,
        }];
        assert_eq!(
            migrate_with(&mut node, 1, &steps, 2),
            Err(DocError::Migration {
                from: 1,
                message: "boom".into()
            })
        );
    }

    #[test]
    fn current_format_needs_no_migration() {
        let mut node = room();
        migrate(&mut node, CURRENT_FORMAT).unwrap();
        assert!(migrate(&mut node, CURRENT_FORMAT + 1).is_err());
        assert!(migrate(&mut node, 0).is_err());
    }
}
