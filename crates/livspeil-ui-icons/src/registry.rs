// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The embedded icon set, by Boxicons name.

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        /// `(name, SVG source)`, sorted by name.
        const ICONS: &[(&str, &str)] = &[$((
            $name,
            include_str!(concat!("../../../assets/icons/boxicons/", $name, ".svg")),
        )),*];
    };
}

icons![
    "bx-bar-chart-big",
    "bx-bell",
    "bx-blob",
    "bx-blocks",
    "bx-bolt-alt",
    "bx-book-content",
    "bx-bookmark-alt",
    "bx-bookmarks",
    "bx-brick",
    "bx-brush-sparkles",
    "bx-bullseye",
    "bx-circuit-board",
    "bx-clover",
    "bx-cognition",
    "bx-columns-4",
    "bx-copy",
    "bx-core",
    "bx-dock-bottom-arrow",
    "bx-dock-left-arrow",
    "bx-dock-right-arrow",
    "bx-enter",
    "bx-envelope",
    "bx-fire-alt",
    "bx-flag",
    "bx-flower",
    "bx-folder",
    "bx-gear",
    "bx-grid-plus",
    "bx-groceries",
    "bx-group-alt",
    "bx-heart",
    "bx-home-alt-2",
    "bx-inbox",
    "bx-layers-down-left",
    "bx-medical-flask",
    "bx-message-circle-dots-2",
    "bx-message-circle-notification",
    "bx-meteor",
    "bx-path",
    "bx-pencil-sparkles",
    "bx-route",
    "bx-scribble",
    "bx-sidebar-right",
    "bx-sparkles-alt",
    "bx-star",
    "bx-task",
    "bx-ticket",
    "bx-ticket-star",
    "bx-unlink",
];

/// An icon of the embedded set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IconId(u16);

impl IconId {
    /// The icon with this Boxicons name (`bx-copy`), or `None` if the set
    /// has no such icon.
    #[must_use]
    pub fn from_name(name: &str) -> Option<IconId> {
        let at = ICONS.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
        u16::try_from(at).ok().map(IconId)
    }

    /// Every icon, sorted by name.
    pub fn all() -> impl ExactSizeIterator<Item = IconId> {
        (0..ICONS.len() as u16).map(IconId)
    }

    /// The Boxicons name.
    #[must_use]
    pub fn name(self) -> &'static str {
        ICONS[usize::from(self.0)].0
    }

    /// The SVG source.
    #[must_use]
    pub fn svg(self) -> &'static str {
        ICONS[usize::from(self.0)].1
    }

    /// Position in [`IconId::all`]; a compact cache key.
    #[must_use]
    pub fn index(self) -> u16 {
        self.0
    }
}
