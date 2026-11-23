// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Keymaps: keystrokes bound to named actions, globally or in a key context.
//!
//! ```toml
//! # ~/.config/livspeil/ui/keymap.toml
//! [bindings]
//! "ctrl-z" = "edit.undo"
//!
//! [contexts.Slider]
//! "shift-right" = "ui.increment_large"
//! ```

use std::collections::BTreeMap;
use std::rc::Rc;

use livspeil_ui_theme::ConfigError;
use serde::Deserialize;
use toml::Spanned;

use crate::input::Keystroke;

/// File name used in error messages.
pub const KEYMAP_FILE: &str = "keymap.toml";

/// Built-in action names.
pub mod actions {
    /// Move focus to the next element in tab order.
    pub const FOCUS_NEXT: &str = "focus.next";
    /// Move focus to the previous element in tab order.
    pub const FOCUS_PREV: &str = "focus.prev";
    /// Activate the focused control (clicks it if nothing else handles it).
    pub const ACTIVATE: &str = "ui.activate";
    /// Cancel / dismiss.
    pub const CANCEL: &str = "ui.cancel";
    /// Step up.
    pub const INCREMENT: &str = "ui.increment";
    /// Step down.
    pub const DECREMENT: &str = "ui.decrement";
    /// Large step up.
    pub const INCREMENT_LARGE: &str = "ui.increment_large";
    /// Large step down.
    pub const DECREMENT_LARGE: &str = "ui.decrement_large";
    /// Go to the minimum.
    pub const MIN: &str = "ui.min";
    /// Go to the maximum.
    pub const MAX: &str = "ui.max";
}

/// One binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The keystroke.
    pub keystroke: Keystroke,
    /// Action name.
    pub action: Rc<str>,
    /// Key context it is limited to (`None`: global).
    pub context: Option<Rc<str>>,
}

/// An ordered list of bindings; later bindings win.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

const DEFAULTS: &str = r#"
[bindings]
"tab" = "focus.next"
"shift-tab" = "focus.prev"
"space" = "ui.activate"
"enter" = "ui.activate"
"escape" = "ui.cancel"

[contexts.Slider]
"right" = "ui.increment"
"up" = "ui.increment"
"left" = "ui.decrement"
"down" = "ui.decrement"
"shift-right" = "ui.increment_large"
"shift-up" = "ui.increment_large"
"shift-left" = "ui.decrement_large"
"shift-down" = "ui.decrement_large"
"pageup" = "ui.increment_large"
"pagedown" = "ui.decrement_large"
"home" = "ui.min"
"end" = "ui.max"
"#;

impl Keymap {
    /// The built-in bindings.
    #[must_use]
    pub fn defaults() -> Keymap {
        Keymap::parse(DEFAULTS).unwrap_or_default()
    }

    /// All bindings in order.
    #[must_use]
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Add a binding (it wins over earlier ones).
    pub fn bind(&mut self, keystroke: Keystroke, action: &str, context: Option<&str>) {
        self.bindings.push(Binding {
            keystroke,
            action: action.into(),
            context: context.map(Into::into),
        });
    }

    /// Append `other`'s bindings (they win over ours).
    pub fn extend(&mut self, other: &Keymap) {
        self.bindings.extend(other.bindings.iter().cloned());
    }

    /// The action bound to `keystroke`, trying `contexts` from the
    /// innermost (first) outwards, then global bindings.
    #[must_use]
    pub fn action_for(&self, keystroke: &Keystroke, contexts: &[Rc<str>]) -> Option<Rc<str>> {
        for ctx in contexts {
            if let Some(b) = self
                .bindings
                .iter()
                .rev()
                .find(|b| b.keystroke == *keystroke && b.context.as_deref() == Some(&**ctx))
            {
                return Some(b.action.clone());
            }
        }
        self.bindings
            .iter()
            .rev()
            .find(|b| b.keystroke == *keystroke && b.context.is_none())
            .map(|b| b.action.clone())
    }

    /// Parse `keymap.toml`. Keys are keystrokes, values action names
    /// (`[a-z0-9_.-]`, non-empty).
    ///
    /// # Errors
    /// A [`ConfigError`] at the offending keystroke or action.
    pub fn parse(source: &str) -> Result<Keymap, ConfigError> {
        let raw: RawKeymap =
            toml::from_str(source).map_err(|e| ConfigError::from_toml(KEYMAP_FILE, source, &e))?;
        let mut keymap = Keymap::default();
        let mut add = |table: Bindings, context: Option<&str>| -> Result<(), ConfigError> {
            for (keys, action) in table {
                let keystroke = Keystroke::parse(keys.get_ref())
                    .map_err(|m| ConfigError::at(KEYMAP_FILE, source, Some(keys.span()), m))?;
                let name = action.get_ref();
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c));
                if !valid {
                    return Err(ConfigError::at(
                        KEYMAP_FILE,
                        source,
                        Some(action.span()),
                        format!(
                            "invalid action name `{name}` (use lowercase letters, digits, `_`, `.`, `-`)"
                        ),
                    ));
                }
                keymap.bind(keystroke, name, context);
            }
            Ok(())
        };
        add(raw.bindings, None)?;
        for (ctx, table) in raw.contexts {
            add(table, Some(&ctx))?;
        }
        Ok(keymap)
    }
}

type Bindings = BTreeMap<Spanned<String>, Spanned<String>>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawKeymap {
    #[serde(default)]
    bindings: Bindings,
    #[serde(default)]
    contexts: BTreeMap<String, Bindings>,
}
