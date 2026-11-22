<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Format fixtures

One directory per released document format: `format-<n>/<Name>.room/room.kdl`.
`tests/fixtures.rs` loads every package here with the current reader and the
core node registry, so documents from any released format keep loading
forever. Never edit or delete a fixture; when the format changes, add a
migration (`livspeil_doc::migrate`) and new fixtures for the new format.

Fixtures are deliberately written by hand in non-canonical style (comments,
odd spacing, integers where floats are expected) so the reader's leniency is
pinned too.
