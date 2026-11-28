// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Widget actions and their default key bindings (merged into the app
//! keymap by [`crate::Env::new`]; users rebind them in `keymap.toml`).

use livspeil_ui_core::Keymap;

/// Action names handled by widgets, in addition to the core's
/// [`livspeil_ui_core::actions`].
pub mod actions {
    /// Next item (radio group, segmented control, select).
    pub const NEXT: &str = "ui.next";
    /// Previous item.
    pub const PREV: &str = "ui.prev";
    /// First item.
    pub const FIRST: &str = "ui.first";
    /// Last item.
    pub const LAST: &str = "ui.last";
    /// Open a popup (select).
    pub const OPEN: &str = "ui.open";
    /// Open the highlighted submenu, or the next menu bar menu (Right).
    pub const MENU_RIGHT: &str = "menu.right";
    /// Close a submenu, or open the previous menu bar menu (Left).
    pub const MENU_LEFT: &str = "menu.left";
    /// Focus the registered menu bar (F10).
    pub const FOCUS_MENUBAR: &str = "menu.focus_bar";
    /// Open the focused element's context menu (Shift+F10).
    pub const CONTEXT_MENU: &str = "menu.context";
    /// Delete the selected nodes (Delete, Backspace in a node canvas).
    pub const DELETE_NODES: &str = "canvas.delete";
    /// Close the selected tab (Delete, Backspace in a closable tab bar).
    pub const CLOSE_TAB: &str = "tabs.close";
    /// Fine step up (number field, ×0.1).
    pub const INCREMENT_SMALL: &str = "ui.increment_small";
    /// Fine step down (number field, ×0.1).
    pub const DECREMENT_SMALL: &str = "ui.decrement_small";

    /// Caret one grapheme left (visual).
    pub const MOVE_LEFT: &str = "text.move_left";
    /// Caret one grapheme right (visual).
    pub const MOVE_RIGHT: &str = "text.move_right";
    /// Caret one word left.
    pub const MOVE_WORD_LEFT: &str = "text.move_word_left";
    /// Caret one word right.
    pub const MOVE_WORD_RIGHT: &str = "text.move_word_right";
    /// Caret to the start.
    pub const MOVE_HOME: &str = "text.move_home";
    /// Caret to the end.
    pub const MOVE_END: &str = "text.move_end";
    /// Extend the selection one grapheme left.
    pub const SELECT_LEFT: &str = "text.select_left";
    /// Extend the selection one grapheme right.
    pub const SELECT_RIGHT: &str = "text.select_right";
    /// Extend the selection one word left.
    pub const SELECT_WORD_LEFT: &str = "text.select_word_left";
    /// Extend the selection one word right.
    pub const SELECT_WORD_RIGHT: &str = "text.select_word_right";
    /// Extend the selection to the start.
    pub const SELECT_HOME: &str = "text.select_home";
    /// Extend the selection to the end.
    pub const SELECT_END: &str = "text.select_end";
    /// Delete the selection or the previous grapheme.
    pub const DELETE_BACKWARD: &str = "text.delete_backward";
    /// Delete the selection or the next grapheme.
    pub const DELETE_FORWARD: &str = "text.delete_forward";
    /// Delete to the previous word start.
    pub const DELETE_WORD_BACKWARD: &str = "text.delete_word_backward";
    /// Delete to the next word end.
    pub const DELETE_WORD_FORWARD: &str = "text.delete_word_forward";
    /// Submit (Enter in a text field).
    pub const SUBMIT: &str = "text.submit";
    /// Swallow a key that is delivered as text input (Space in text fields,
    /// so it does not also activate).
    pub const TEXT_KEY: &str = "text.key";

    /// Select all.
    pub const SELECT_ALL: &str = "edit.select_all";
    /// Copy the selection.
    pub const COPY: &str = "edit.copy";
    /// Cut the selection.
    pub const CUT: &str = "edit.cut";
    /// Paste.
    pub const PASTE: &str = "edit.paste";
    /// Undo.
    pub const UNDO: &str = "edit.undo";
    /// Redo.
    pub const REDO: &str = "edit.redo";
}

/// Default widget bindings (TOML, same format as `keymap.toml`).
pub const WIDGET_KEYMAP: &str = r#"
[bindings]
"f10" = "menu.focus_bar"
"shift-f10" = "menu.context"

[contexts.Menu]
"down" = "ui.next"
"up" = "ui.prev"
"home" = "ui.first"
"end" = "ui.last"
"right" = "menu.right"
"left" = "menu.left"

[contexts.NodeCanvas]
"delete" = "canvas.delete"
"backspace" = "canvas.delete"
"ctrl-a" = "edit.select_all"
"cmd-a" = "edit.select_all"

[contexts.MenuBar]
"right" = "ui.next"
"left" = "ui.prev"
"down" = "ui.open"
[contexts.TextField]
"space" = "text.key"
"enter" = "text.submit"
"left" = "text.move_left"
"right" = "text.move_right"
"shift-left" = "text.select_left"
"shift-right" = "text.select_right"
"ctrl-left" = "text.move_word_left"
"ctrl-right" = "text.move_word_right"
"alt-left" = "text.move_word_left"
"alt-right" = "text.move_word_right"
"ctrl-shift-left" = "text.select_word_left"
"ctrl-shift-right" = "text.select_word_right"
"alt-shift-left" = "text.select_word_left"
"alt-shift-right" = "text.select_word_right"
"home" = "text.move_home"
"end" = "text.move_end"
"cmd-left" = "text.move_home"
"cmd-right" = "text.move_end"
"up" = "text.move_home"
"down" = "text.move_end"
"shift-home" = "text.select_home"
"shift-end" = "text.select_end"
"cmd-shift-left" = "text.select_home"
"cmd-shift-right" = "text.select_end"
"backspace" = "text.delete_backward"
"shift-backspace" = "text.delete_backward"
"delete" = "text.delete_forward"
"ctrl-backspace" = "text.delete_word_backward"
"alt-backspace" = "text.delete_word_backward"
"ctrl-delete" = "text.delete_word_forward"
"alt-delete" = "text.delete_word_forward"
"ctrl-a" = "edit.select_all"
"cmd-a" = "edit.select_all"
"ctrl-c" = "edit.copy"
"cmd-c" = "edit.copy"
"ctrl-x" = "edit.cut"
"cmd-x" = "edit.cut"
"ctrl-v" = "edit.paste"
"cmd-v" = "edit.paste"
"ctrl-z" = "edit.undo"
"cmd-z" = "edit.undo"
"ctrl-shift-z" = "edit.redo"
"cmd-shift-z" = "edit.redo"
"ctrl-y" = "edit.redo"

[contexts.NumberField]
"up" = "ui.increment"
"right" = "ui.increment"
"down" = "ui.decrement"
"left" = "ui.decrement"
"shift-up" = "ui.increment_large"
"shift-right" = "ui.increment_large"
"shift-down" = "ui.decrement_large"
"shift-left" = "ui.decrement_large"
"alt-up" = "ui.increment_small"
"alt-right" = "ui.increment_small"
"alt-down" = "ui.decrement_small"
"alt-left" = "ui.decrement_small"
"pageup" = "ui.increment_large"
"pagedown" = "ui.decrement_large"
"home" = "ui.min"
"end" = "ui.max"

[contexts.RadioGroup]
"down" = "ui.next"
"right" = "ui.next"
"up" = "ui.prev"
"left" = "ui.prev"
"home" = "ui.first"
"end" = "ui.last"

[contexts.SegmentedControl]
"right" = "ui.next"
"down" = "ui.next"
"left" = "ui.prev"
"up" = "ui.prev"
"home" = "ui.first"
"end" = "ui.last"

[contexts.Tabs]
"right" = "ui.next"
"left" = "ui.prev"
"home" = "ui.first"
"end" = "ui.last"
"delete" = "tabs.close"
"backspace" = "tabs.close"

[contexts.Select]
"down" = "ui.next"
"up" = "ui.prev"
"home" = "ui.first"
"end" = "ui.last"
"alt-down" = "ui.open"

[contexts.ScrollView]
"up" = "scroll.line_up"
"down" = "scroll.line_down"
"left" = "scroll.line_left"
"right" = "scroll.line_right"
"pageup" = "scroll.page_up"
"pagedown" = "scroll.page_down"
"home" = "scroll.top"
"end" = "scroll.bottom"
"#;

/// The default widget bindings.
#[must_use]
pub fn keymap() -> Keymap {
    Keymap::parse(WIDGET_KEYMAP).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_keymap_parses() {
        let k = Keymap::parse(WIDGET_KEYMAP).expect("widget keymap");
        assert!(k.bindings().len() > 60);
    }
}
