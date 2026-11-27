// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Accessibility semantics: what an element *is* (role), what it is
//! called (label), its value and state, and which semantic actions it
//! supports. Toolkit-agnostic; `livspeil-ui-a11y` maps it to AccessKit.
//!
//! ```
//! use livspeil_ui_core::{Role, SemanticValue, div};
//!
//! let volume = div()
//!     .role(Role::Slider)
//!     .semantics(|s| s.label("Volume").value(SemanticValue::number(0.5, 0.0, 1.0, 0.1)));
//! # let _ = volume;
//! ```
//!
//! Elements without a role are transparent: their semantic children are
//! attached to the nearest semantic ancestor. Text elements are labels,
//! focusable elements are always exposed (so focus can be reported).

use std::fmt;
use std::rc::Rc;

use livspeil_ui_layout::Rect;

use crate::app::{App, EventCx};
use crate::element::{Element, ElementId, Phase, SharedString};
use crate::input::Modifiers;
use crate::keymap::actions;

/// Built-in action dispatched (bubbling from the target) when assistive
/// technology asks to scroll an element into view. Scroll containers
/// handle it with [`Element::on_action`].
pub const SCROLL_INTO_VIEW: &str = "ui.scroll_into_view";

/// What an element is, for assistive technology.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// A generic container with no particular meaning.
    Generic,
    /// A top-level window.
    Window,
    /// A dialog.
    Dialog,
    /// A group of related controls.
    Group,
    /// Static text.
    Label,
    /// A heading.
    Heading,
    /// A push button.
    Button,
    /// A link.
    Link,
    /// A check box (`checked` holds its state).
    CheckBox,
    /// An on/off switch (`checked` holds its state).
    Switch,
    /// A radio button (`checked` holds whether it is the chosen one).
    RadioButton,
    /// A group of [`Role::RadioButton`]s (one choice).
    RadioGroup,
    /// A slider (numeric value with min/max/step).
    Slider,
    /// A progress indicator (numeric value).
    ProgressIndicator,
    /// A numeric field stepped with the keyboard (ARIA `spinbutton`;
    /// numeric value with min/max/step).
    SpinButton,
    /// Single-line text input.
    TextInput,
    /// A collapsed select that opens a [`Role::ListBox`] (ARIA
    /// `combobox`; `expanded` holds whether the popup is open, the value
    /// is the chosen option's text).
    ComboBox,
    /// A list of options to choose from (a select's popup).
    ListBox,
    /// An option of a [`Role::ListBox`] (`selected` holds its state).
    ListBoxOption,
    /// A tooltip (its label is the tip text).
    Tooltip,
    /// An image (`label` is its alternative text).
    Image,
    /// A list.
    List,
    /// An item of a [`Role::List`].
    ListItem,
    /// A scrollable area.
    ScrollView,
    /// A menu.
    Menu,
    /// An item of a [`Role::Menu`].
    MenuItem,
    /// A tab strip.
    TabList,
    /// A tab of a [`Role::TabList`] (`selected` holds its state).
    Tab,
    /// A toolbar.
    Toolbar,
    /// A horizontal bar of [`Role::MenuItem`]s that open menus.
    MenuBar,
    /// A menu item with a check mark (`checked` holds its state).
    MenuItemCheckBox,
    /// A separator between groups (menu separators; focusable separators
    /// are splitters).
    Separator,
    /// A live region whose changes are announced politely (toasts; ARIA
    /// `status`).
    Status,
    /// A live region whose changes are announced at once (urgent toasts;
    /// ARIA `alert`).
    Alert,
}

impl Role {
    /// Whether the element's descendants are presentational: assistive
    /// technology sees only the element itself (its label, value and
    /// state), like ARIA's "children presentational" roles.
    #[must_use]
    pub fn children_presentational(self) -> bool {
        matches!(
            self,
            Role::Button
                | Role::Link
                | Role::CheckBox
                | Role::Switch
                | Role::RadioButton
                | Role::Slider
                | Role::SpinButton
                | Role::ProgressIndicator
                | Role::TextInput
                | Role::ComboBox
                | Role::ListBoxOption
                | Role::Tooltip
                | Role::Image
                | Role::Label
                | Role::Heading
                | Role::Tab
                | Role::MenuItem
                | Role::MenuItemCheckBox
                | Role::Separator
        )
    }
}

/// Check / toggle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Toggled {
    /// Unchecked / off.
    Off,
    /// Checked / on.
    On,
    /// Indeterminate.
    Mixed,
}

impl From<bool> for Toggled {
    fn from(on: bool) -> Self {
        if on { Toggled::On } else { Toggled::Off }
    }
}

/// The value of an element.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticValue {
    /// A text value (text inputs, combo boxes).
    Text(SharedString),
    /// A numeric value (sliders, progress).
    Number {
        /// Current value.
        value: f64,
        /// Minimum, if bounded.
        min: Option<f64>,
        /// Maximum, if bounded.
        max: Option<f64>,
        /// Step, if stepped.
        step: Option<f64>,
    },
}

impl SemanticValue {
    /// A bounded, stepped numeric value.
    #[must_use]
    pub fn number(value: f64, min: f64, max: f64, step: f64) -> Self {
        SemanticValue::Number {
            value,
            min: Some(min),
            max: Some(max),
            step: Some(step),
        }
    }

    /// A text value.
    #[must_use]
    pub fn text(text: impl Into<SharedString>) -> Self {
        SemanticValue::Text(text.into())
    }
}

/// A semantic action an element supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticAction {
    /// Click / press / activate.
    Activate,
    /// Take keyboard focus.
    Focus,
    /// Step the value up.
    Increment,
    /// Step the value down.
    Decrement,
    /// Set the value (see [`Element::on_set_value`]).
    SetValue,
    /// Scroll the element into view.
    ScrollIntoView,
}

impl SemanticAction {
    /// Every action, in a fixed order.
    pub const ALL: [SemanticAction; 6] = [
        SemanticAction::Activate,
        SemanticAction::Focus,
        SemanticAction::Increment,
        SemanticAction::Decrement,
        SemanticAction::SetValue,
        SemanticAction::ScrollIntoView,
    ];

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// A set of [`SemanticAction`]s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct ActionSet(u8);

impl ActionSet {
    /// The empty set.
    pub const EMPTY: ActionSet = ActionSet(0);

    /// Whether `action` is in the set.
    #[must_use]
    pub fn contains(self, action: SemanticAction) -> bool {
        self.0 & action.bit() != 0
    }

    /// Add `action`.
    pub fn insert(&mut self, action: SemanticAction) {
        self.0 |= action.bit();
    }

    /// Whether the set is empty.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The actions in the set, in [`SemanticAction::ALL`] order.
    pub fn iter(self) -> impl Iterator<Item = SemanticAction> {
        SemanticAction::ALL
            .into_iter()
            .filter(move |a| self.contains(*a))
    }
}

impl FromIterator<SemanticAction> for ActionSet {
    fn from_iter<I: IntoIterator<Item = SemanticAction>>(iter: I) -> Self {
        let mut s = ActionSet::EMPTY;
        for a in iter {
            s.insert(a);
        }
        s
    }
}

/// A value requested by assistive technology.
#[derive(Clone, Debug, PartialEq)]
pub enum ValueRequest {
    /// Set a text value.
    Text(String),
    /// Set a numeric value.
    Number(f64),
}

pub(crate) type SetValueHandler = Rc<dyn Fn(&ValueRequest, &mut EventCx<'_>)>;

/// Semantics of one element. Build with the chained setters, usually via
/// [`Element::semantics`].
#[derive(Clone, Default)]
pub struct Semantics {
    role: Option<Role>,
    label: Option<SharedString>,
    description: Option<SharedString>,
    value: Option<SemanticValue>,
    checked: Option<Toggled>,
    expanded: Option<bool>,
    selected: Option<bool>,
    disabled: bool,
    hidden: bool,
    actions: ActionSet,
    set_value: Option<SetValueHandler>,
}

impl fmt::Debug for Semantics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Semantics")
            .field("role", &self.role)
            .field("label", &self.label)
            .field("description", &self.description)
            .field("value", &self.value)
            .field("checked", &self.checked)
            .field("expanded", &self.expanded)
            .field("selected", &self.selected)
            .field("disabled", &self.disabled)
            .field("hidden", &self.hidden)
            .field("actions", &self.actions)
            .field("set_value", &self.set_value.is_some())
            .finish()
    }
}

impl Semantics {
    /// Semantics with a role.
    #[must_use]
    pub fn new(role: Role) -> Self {
        Semantics {
            role: Some(role),
            ..Semantics::default()
        }
    }

    /// Role.
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }
    /// Accessible name.
    #[must_use]
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }
    /// Longer description (tooltip-like).
    #[must_use]
    pub fn description(mut self, d: impl Into<SharedString>) -> Self {
        self.description = Some(d.into());
        self
    }
    /// Value.
    #[must_use]
    pub fn value(mut self, v: SemanticValue) -> Self {
        self.value = Some(v);
        self
    }
    /// Checked / toggled state (check boxes, switches, toggle buttons).
    #[must_use]
    pub fn checked(mut self, t: impl Into<Toggled>) -> Self {
        self.checked = Some(t.into());
        self
    }
    /// Expanded (`true`) or collapsed (`false`).
    #[must_use]
    pub fn expanded(mut self, on: bool) -> Self {
        self.expanded = Some(on);
        self
    }
    /// Selected state (list items, tabs).
    #[must_use]
    pub fn selected(mut self, on: bool) -> Self {
        self.selected = Some(on);
        self
    }
    /// Disabled: exposed but not operable; semantic actions are refused.
    #[must_use]
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
    /// Hidden from assistive technology, with its subtree.
    #[must_use]
    pub fn hidden(mut self, on: bool) -> Self {
        self.hidden = on;
        self
    }
    /// Declare a supported action (most are inferred from handlers).
    #[must_use]
    pub fn action(mut self, a: SemanticAction) -> Self {
        self.actions.insert(a);
        self
    }

    /// Role, if any.
    #[must_use]
    pub fn get_role(&self) -> Option<Role> {
        self.role
    }
    /// Label, if any.
    #[must_use]
    pub fn get_label(&self) -> Option<&str> {
        self.label.as_deref()
    }
    /// Description, if any.
    #[must_use]
    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    /// Value, if any.
    #[must_use]
    pub fn get_value(&self) -> Option<&SemanticValue> {
        self.value.as_ref()
    }
    /// Checked state, if applicable.
    #[must_use]
    pub fn get_checked(&self) -> Option<Toggled> {
        self.checked
    }
    /// Expanded state, if applicable.
    #[must_use]
    pub fn get_expanded(&self) -> Option<bool> {
        self.expanded
    }
    /// Selected state, if applicable.
    #[must_use]
    pub fn get_selected(&self) -> Option<bool> {
        self.selected
    }
    /// Whether disabled.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
    /// Whether hidden.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.hidden
    }
    /// Explicitly declared actions.
    #[must_use]
    pub fn declared_actions(&self) -> ActionSet {
        self.actions
    }
}

impl Element {
    fn semantics_mut(&mut self) -> &mut Semantics {
        self.semantics.get_or_insert_with(Box::default)
    }

    /// Accessibility role.
    #[must_use]
    pub fn role(mut self, role: Role) -> Self {
        self.semantics_mut().role = Some(role);
        self
    }

    /// Edit the element's accessibility semantics (label, value, state…).
    #[must_use]
    pub fn semantics(mut self, f: impl FnOnce(Semantics) -> Semantics) -> Self {
        let s = std::mem::take(self.semantics_mut());
        *self.semantics_mut() = f(s);
        self
    }

    /// Handle a value set by assistive technology (`SetValue`).
    #[must_use]
    pub fn on_set_value(mut self, f: impl Fn(&ValueRequest, &mut EventCx<'_>) + 'static) -> Self {
        self.semantics_mut().set_value = Some(Rc::new(f));
        self
    }
}

/// A read-only view of one retained element for accessibility.
#[derive(Clone, Copy, Debug)]
pub struct SemanticNode<'a> {
    /// The element.
    pub id: ElementId,
    /// Parent element.
    pub parent: Option<ElementId>,
    /// Retained children in order.
    pub children: &'a [ElementId],
    /// Declared semantics, if any.
    pub semantics: Option<&'a Semantics>,
    /// Text content (text elements).
    pub text: Option<&'a str>,
    /// Where the element is on screen, in logical window pixels
    /// ([`App::visual_bounds`]).
    pub bounds: Rect,
    /// Whether the element can take focus.
    pub focusable: bool,
    /// Supported actions: declared plus inferred from handlers; empty
    /// when disabled.
    pub actions: ActionSet,
    /// Debug label (name, key or kind).
    pub debug_label: &'a str,
}

impl SemanticNode<'_> {
    /// Whether the element is hidden from assistive technology.
    #[must_use]
    pub fn is_hidden(&self) -> bool {
        self.semantics.is_some_and(Semantics::is_hidden)
    }

    /// Whether the element is disabled.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.semantics.is_some_and(Semantics::is_disabled)
    }

    /// Effective role: declared, else [`Role::Label`] for text,
    /// [`Role::Window`] for the root, [`Role::Generic`] for other focusable
    /// or labelled elements; `None` for transparent elements.
    #[must_use]
    pub fn role(&self) -> Option<Role> {
        if let Some(r) = self.semantics.and_then(Semantics::get_role) {
            return Some(r);
        }
        if self.parent.is_none() {
            Some(Role::Window)
        } else if self.text.is_some() {
            Some(Role::Label)
        } else if self.focusable || self.semantics.is_some_and(|s| s.label.is_some()) {
            Some(Role::Generic)
        } else {
            None
        }
    }

    /// Accessible name: declared label, else the text content.
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.semantics.and_then(Semantics::get_label).or(self.text)
    }
}

/// Why a semantic action was not performed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticActionError {
    /// No such element.
    UnknownElement(ElementId),
    /// The element is disabled.
    Disabled(ElementId),
    /// The element (or its subtree) is hidden.
    Hidden(ElementId),
    /// The element does not support the action.
    Unsupported(ElementId, SemanticAction),
}

impl fmt::Display for SemanticActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownElement(id) => write!(f, "no element {id:?}"),
            Self::Disabled(id) => write!(f, "element {id:?} is disabled"),
            Self::Hidden(id) => write!(f, "element {id:?} is hidden"),
            Self::Unsupported(id, a) => write!(f, "element {id:?} does not support {a:?}"),
        }
    }
}

impl std::error::Error for SemanticActionError {}

/// A semantic action request for one element.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticRequest {
    /// Click / activate.
    Activate,
    /// Focus (with focus ring, as from the keyboard).
    Focus,
    /// Clear focus if this element has it.
    Blur,
    /// Step up.
    Increment,
    /// Step down.
    Decrement,
    /// Set the value.
    SetValue(ValueRequest),
    /// Scroll into view (dispatches [`SCROLL_INTO_VIEW`] from the element).
    ScrollIntoView,
}

impl App {
    /// The root element of the retained tree (the window), once mounted.
    #[must_use]
    pub fn semantic_root(&self) -> Option<ElementId> {
        self.tree.root
    }

    /// Accessibility view of a retained element.
    #[must_use]
    pub fn semantic_node(&self, id: ElementId) -> Option<SemanticNode<'_>> {
        let n = self.tree.nodes.get(&id)?;
        let semantics = n.semantics.as_deref();
        let disabled = semantics.is_some_and(Semantics::is_disabled);
        let mut actions = semantics.map_or(ActionSet::EMPTY, Semantics::declared_actions);
        if !disabled {
            if n.tab_index.is_some() {
                actions.insert(SemanticAction::Focus);
            }
            if semantics.is_some_and(|s| s.set_value.is_some()) {
                actions.insert(SemanticAction::SetValue);
            }
            if let Some(h) = &n.handlers {
                if h.click.is_some() || h.actions.iter().any(|a| &*a.0 == actions::ACTIVATE) {
                    actions.insert(SemanticAction::Activate);
                }
                if h.actions.iter().any(|a| &*a.0 == actions::INCREMENT) {
                    actions.insert(SemanticAction::Increment);
                }
                if h.actions.iter().any(|a| &*a.0 == actions::DECREMENT) {
                    actions.insert(SemanticAction::Decrement);
                }
            }
        } else {
            actions = ActionSet::EMPTY;
        }
        Some(SemanticNode {
            id,
            parent: n.parent,
            children: &n.children,
            semantics,
            text: n.text.as_ref().map(|t| &*t.text),
            bounds: self.visual_bounds(id).unwrap_or(n.bounds),
            focusable: n.tab_index.is_some(),
            actions,
            debug_label: &n.label,
        })
    }

    /// Perform a semantic action on `id` (from assistive technology).
    /// Returns whether something handled it.
    ///
    /// # Errors
    /// The element is unknown, disabled or hidden (itself or an ancestor),
    /// or does not support the action.
    pub fn perform_semantic_action(
        &mut self,
        id: ElementId,
        request: SemanticRequest,
    ) -> Result<bool, SemanticActionError> {
        let node = self
            .semantic_node(id)
            .ok_or(SemanticActionError::UnknownElement(id))?;
        let (actions, window) = (node.actions, node.bounds);
        let bounds = self.bounds(id).unwrap_or(window);
        let mut cur = Some(node);
        while let Some(n) = cur {
            if n.is_hidden() {
                return Err(SemanticActionError::Hidden(id));
            }
            if n.is_disabled() {
                return Err(SemanticActionError::Disabled(id));
            }
            cur = n.parent.and_then(|p| self.semantic_node(p));
        }
        let require = |a: SemanticAction| {
            if actions.contains(a) {
                Ok(())
            } else {
                Err(SemanticActionError::Unsupported(id, a))
            }
        };
        let handled = match request {
            SemanticRequest::Activate => {
                require(SemanticAction::Activate)?;
                self.fire_click(id, window.center(), Modifiers::NONE, true)
                    || self.dispatch_action_at(id, actions::ACTIVATE)
            }
            SemanticRequest::Focus => {
                require(SemanticAction::Focus)?;
                self.set_focus(Some(id), true);
                true
            }
            SemanticRequest::Blur => {
                if self.focused() == Some(id) {
                    self.blur();
                    true
                } else {
                    false
                }
            }
            SemanticRequest::Increment => {
                require(SemanticAction::Increment)?;
                self.dispatch_action_at(id, actions::INCREMENT)
            }
            SemanticRequest::Decrement => {
                require(SemanticAction::Decrement)?;
                self.dispatch_action_at(id, actions::DECREMENT)
            }
            SemanticRequest::SetValue(v) => {
                require(SemanticAction::SetValue)?;
                let handler = self
                    .tree
                    .nodes
                    .get(&id)
                    .and_then(|n| n.semantics.as_ref())
                    .and_then(|s| s.set_value.clone());
                match handler {
                    Some(h) => {
                        let mut cx = EventCx::new(self, id, id, bounds, Phase::Bubble);
                        h(&v, &mut cx);
                        true
                    }
                    None => false,
                }
            }
            SemanticRequest::ScrollIntoView => self.dispatch_action_at(id, SCROLL_INTO_VIEW),
        };
        self.flush_effects();
        Ok(handled)
    }

    /// Dispatch a named action starting at `target` (instead of the focused
    /// element) and bubbling to the root. Returns whether it was handled.
    pub fn dispatch_action_at(&mut self, target: ElementId, action: &str) -> bool {
        let mut cur = Some(target);
        while let Some(id) = cur {
            cur = self.tree.nodes.get(&id).and_then(|n| n.parent);
            let Some((bounds, h)) = self.node_handlers(id) else {
                continue;
            };
            if let Some((_, handler)) = h.actions.iter().find(|(name, _)| &**name == action) {
                let handler = handler.clone();
                let mut cx = EventCx::new(self, target, id, bounds, Phase::Bubble);
                handler(&mut cx);
                self.flush_effects();
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::element::div;

    #[test]
    fn builders_compose() {
        let e = div()
            .role(Role::CheckBox)
            .semantics(|s| s.label("A").checked(true).disabled(true))
            .semantics(|s| s.description("d"));
        let s = e.semantics.as_deref().cloned().unwrap_or_default();
        assert_eq!(s.get_role(), Some(Role::CheckBox));
        assert_eq!(s.get_label(), Some("A"));
        assert_eq!(s.get_description(), Some("d"));
        assert_eq!(s.get_checked(), Some(Toggled::On));
        assert!(s.is_disabled());
    }

    #[test]
    fn action_set() {
        let s: ActionSet = [SemanticAction::Focus, SemanticAction::SetValue]
            .into_iter()
            .collect();
        assert!(s.contains(SemanticAction::Focus));
        assert!(!s.contains(SemanticAction::Activate));
        assert_eq!(s.iter().count(), 2);
    }
}
