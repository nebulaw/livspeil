// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Choosing a room to open: those of the rooms folder, or a typed path.

use std::path::{Path, PathBuf};

use livspeil_ui_core::App;
use livspeil_ui_widgets::{Env, Picker, PickerItem, search_items};

use super::files::Files;
use crate::rooms::{self, Found};

fn item(room: &Found) -> PickerItem {
    PickerItem::new(room.name.clone(), rooms::shorten(&room.path))
}

/// What the list offers for `query`.
fn offered(query: &str, folder: &Path) -> Vec<PickerItem> {
    if rooms::is_path(query) {
        return rooms::at_path(query, folder).iter().map(item).collect();
    }
    let all: Vec<PickerItem> = rooms::list(folder).iter().map(item).collect();
    // By name only: every path holds the folder's.
    let names: Vec<PickerItem> = all
        .iter()
        .map(|i| PickerItem::new(i.label.clone(), ""))
        .collect();
    search_items(&names, query)
        .into_iter()
        .filter_map(|i| all.get(i).cloned())
        .collect()
}

/// The list of rooms; choosing one opens it.
pub(super) fn picker(app: &mut App, env: &Env, files: Files, folder: PathBuf) -> Picker {
    let nothing = format!(
        "No room. Rooms in {} are listed; or type a path.",
        rooms::shorten(&folder)
    );
    let home = folder.clone();
    let picker = Picker::new(
        app,
        env,
        "open",
        "Open a room",
        move |query| offered(query, &home),
        move |chosen, app| {
            // The detail is the path, with `~` for home.
            if let Some(path) = rooms::resolve(&chosen.detail, &folder) {
                files.open(app, path);
            }
        },
    );
    picker.set_hints(app, "Name or path", nothing);
    picker
}
