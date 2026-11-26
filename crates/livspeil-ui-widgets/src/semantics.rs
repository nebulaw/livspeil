// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! ARIA-like semantics of a widget: role, label, value, state and the
//! actions it supports. Each widget declares them on its element
//! ([`Semantics::apply`]), which is what `livspeil-ui-a11y` exposes to
//! AccessKit.

use std::fmt;

/// What a widget is (maps to AccessKit / ARIA roles).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Push button (also icon buttons).
    Button,
    /// On/off switch.
    Switch,
    /// Check box (may be mixed).
    CheckBox,
    /// One radio button.
    RadioButton,
    /// A group of radio buttons.
    RadioGroup,
    /// Segmented control: a group of exclusive segments (ARIA `tablist`-like
    /// radio group).
    SegmentedControl,
    /// One segment of a segmented control.
    Segment,
    /// Slider.
    Slider,
    /// Numeric field (ARIA `spinbutton`).
    SpinButton,
    /// Single-line text input.
    TextInput,
    /// Collapsed select (ARIA `combobox`).
    ComboBox,
    /// The popup list of a select.
    ListBox,
    /// One option in a list box.
    ListBoxOption,
    /// Tooltip.
    Tooltip,
    /// Static text label.
    Label,
    /// A scrollable area.
    ScrollView,
    /// A panel (a titled group).
    Panel,
    /// A draggable splitter between panels (ARIA focusable `separator`).
    Splitter,
    /// A menu (popup list of items).
    Menu,
    /// A menu bar.
    MenuBar,
    /// A menu item (also a menu bar title).
    MenuItem,
    /// A menu item with a check mark.
    MenuItemCheckBox,
    /// A separator between menu items.
    Separator,
    /// A window titlebar (title and toolbar items).
    Titlebar,
    /// A node editor canvas.
    NodeCanvas,
    /// One node card on a node canvas.
    Node,
    /// A tab strip ([`crate::Tabs`]).
    TabList,
    /// One tab of a tab strip (selected state).
    Tab,
    /// A modal dialog or sheet.
    Dialog,
    /// A popover (an anchored, non-modal group).
    Popover,
    /// A live region announcing status messages politely (toasts).
    Status,
    /// A live region announcing an important message at once.
    Alert,
}

/// Toggle state of switches and check boxes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Toggled {
    /// Off / unchecked.
    False,
    /// On / checked.
    True,
    /// Mixed (indeterminate).
    Mixed,
}

/// Actions assistive technology may perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticAction {
    /// Click / activate / toggle.
    Click,
    /// Move focus here.
    Focus,
    /// Step up.
    Increment,
    /// Step down.
    Decrement,
    /// Set the value (numeric or text).
    SetValue,
    /// Open a popup.
    Expand,
    /// Close a popup.
    Collapse,
    /// Set the text selection.
    SetTextSelection,
    /// Copy / cut / paste.
    Clipboard,
}

/// Numeric value with range (sliders, number fields).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NumericValue {
    /// Current value.
    pub value: f64,
    /// Minimum.
    pub min: f64,
    /// Maximum.
    pub max: f64,
    /// Step.
    pub step: f64,
}

/// A semantic description of one widget (and its parts).
#[derive(Clone, Debug, PartialEq)]
pub struct Semantics {
    /// Role.
    pub role: Role,
    /// Accessible name.
    pub label: Option<String>,
    /// Human-readable value (text content, formatted number, chosen option).
    pub value: Option<String>,
    /// Numeric value and range.
    pub numeric: Option<NumericValue>,
    /// Toggle state.
    pub toggled: Option<Toggled>,
    /// Selected (radio buttons, segments, options).
    pub selected: Option<bool>,
    /// Popup expanded (select).
    pub expanded: Option<bool>,
    /// Placeholder text.
    pub placeholder: Option<String>,
    /// Text selection as byte offsets (anchor, focus).
    pub text_selection: Option<(usize, usize)>,
    /// Disabled: not focusable, ignores input.
    pub disabled: bool,
    /// In the tab order (or focusable by pointer).
    pub focusable: bool,
    /// Supported actions.
    pub actions: Vec<SemanticAction>,
    /// Parts (radio buttons of a group, segments, options).
    pub children: Vec<Semantics>,
}

impl Semantics {
    /// Semantics with `role` and nothing else.
    #[must_use]
    pub fn new(role: Role) -> Self {
        Semantics {
            role,
            label: None,
            value: None,
            numeric: None,
            toggled: None,
            selected: None,
            expanded: None,
            placeholder: None,
            text_selection: None,
            disabled: false,
            focusable: false,
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Set the label.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Set the label if any.
    #[must_use]
    pub fn label_opt(mut self, label: Option<&str>) -> Self {
        self.label = label.map(str::to_owned);
        self
    }

    /// Set the value text.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// Set the numeric value.
    #[must_use]
    pub fn numeric(mut self, value: f64, min: f64, max: f64, step: f64) -> Self {
        self.numeric = Some(NumericValue {
            value,
            min,
            max,
            step,
        });
        self
    }

    /// Set the toggle state.
    #[must_use]
    pub fn toggled(mut self, t: Toggled) -> Self {
        self.toggled = Some(t);
        self
    }

    /// Set the selected state.
    #[must_use]
    pub fn selected(mut self, s: bool) -> Self {
        self.selected = Some(s);
        self
    }

    /// Set the expanded state.
    #[must_use]
    pub fn expanded(mut self, e: bool) -> Self {
        self.expanded = Some(e);
        self
    }

    /// Set the placeholder.
    #[must_use]
    pub fn placeholder(mut self, p: Option<&str>) -> Self {
        self.placeholder = p.map(str::to_owned);
        self
    }

    /// Mark disabled (clears `focusable` and the actions), or enabled and
    /// focusable with `actions`.
    #[must_use]
    pub fn interactive(mut self, disabled: bool, actions: &[SemanticAction]) -> Self {
        self.disabled = disabled;
        self.focusable = !disabled;
        self.actions = if disabled {
            Vec::new()
        } else {
            actions.to_vec()
        };
        self
    }

    /// Add a part.
    #[must_use]
    pub fn child(mut self, c: Semantics) -> Self {
        self.children.push(c);
        self
    }

    fn write(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        write!(f, "{}{:?}", "  ".repeat(depth), self.role)?;
        if let Some(l) = &self.label {
            write!(f, " label={l:?}")?;
        }
        if let Some(v) = &self.value {
            write!(f, " value={v:?}")?;
        }
        if let Some(n) = &self.numeric {
            write!(f, " range={}..={} step={}", n.min, n.max, n.step)?;
        }
        if let Some(t) = self.toggled {
            write!(f, " toggled={t:?}")?;
        }
        if let Some(s) = self.selected {
            write!(f, " selected={s}")?;
        }
        if let Some(e) = self.expanded {
            write!(f, " expanded={e}")?;
        }
        if let Some(p) = &self.placeholder {
            write!(f, " placeholder={p:?}")?;
        }
        if let Some((a, b)) = self.text_selection {
            write!(f, " selection={a}..{b}")?;
        }
        if self.disabled {
            f.write_str(" disabled")?;
        }
        if self.focusable {
            f.write_str(" focusable")?;
        }
        if !self.actions.is_empty() {
            write!(f, " actions={:?}", self.actions)?;
        }
        writeln!(f)?;
        for c in &self.children {
            c.write(f, depth + 1)?;
        }
        Ok(())
    }
}

/// One line per node, children indented.
impl fmt::Display for Semantics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f, 0)
    }
}

/// Map to the core's accessibility semantics (`livspeil_ui_core::Semantics`).
///
/// Roles map one to one, except `SegmentedControl` → `TabList` and
/// `Segment` → `Tab` (selected). A radio button's selection is also its
/// checked state. `Expand`/`Collapse`/`Click` become `Activate`;
/// text-selection and clipboard actions have no core equivalent yet.
impl Semantics {
    /// The core role for this widget role.
    #[must_use]
    pub fn core_role(&self) -> livspeil_ui_core::Role {
        use livspeil_ui_core::Role as R;
        match self.role {
            Role::Button => R::Button,
            Role::ComboBox => R::ComboBox,
            Role::Switch => R::Switch,
            Role::CheckBox => R::CheckBox,
            Role::RadioButton => R::RadioButton,
            Role::RadioGroup => R::RadioGroup,
            Role::SegmentedControl => R::TabList,
            Role::Segment => R::Tab,
            Role::Slider => R::Slider,
            Role::SpinButton => R::SpinButton,
            Role::TextInput => R::TextInput,
            Role::ListBox => R::ListBox,
            Role::ListBoxOption => R::ListBoxOption,
            Role::Tooltip => R::Tooltip,
            Role::Label => R::Label,
            Role::ScrollView => R::ScrollView,
            Role::Panel | Role::NodeCanvas | Role::Node => R::Group,
            Role::Splitter | Role::Separator => R::Separator,
            Role::Menu => R::Menu,
            Role::MenuBar => R::MenuBar,
            Role::MenuItem => R::MenuItem,
            Role::MenuItemCheckBox => R::MenuItemCheckBox,
            Role::Titlebar => R::Toolbar,
            Role::TabList => R::TabList,
            Role::Tab => R::Tab,
            Role::Dialog => R::Dialog,
            Role::Popover => R::Group,
            Role::Status => R::Group,
            Role::Alert => R::Group,
        }
    }

    /// The core semantics (without children: parts declare their own).
    #[must_use]
    pub fn to_core(&self) -> livspeil_ui_core::Semantics {
        use livspeil_ui_core::{SemanticAction as A, SemanticValue};
        let mut s = livspeil_ui_core::Semantics::new(self.core_role()).disabled(self.disabled);
        if let Some(l) = &self.label {
            s = s.label(l.as_str());
        }
        if let Some(n) = self.numeric {
            s = s.value(SemanticValue::number(n.value, n.min, n.max, n.step));
        } else if let Some(v) = &self.value {
            s = s.value(SemanticValue::text(v.as_str()));
        }
        let toggled = match (self.role, self.toggled, self.selected) {
            (Role::RadioButton, _, Some(sel)) => Some(livspeil_ui_core::Toggled::from(sel)),
            (Role::MenuItemCheckBox, None, _) => Some(livspeil_ui_core::Toggled::Off),
            (_, Some(Toggled::True), _) => Some(livspeil_ui_core::Toggled::On),
            (_, Some(Toggled::False), _) => Some(livspeil_ui_core::Toggled::Off),
            (_, Some(Toggled::Mixed), _) => Some(livspeil_ui_core::Toggled::Mixed),
            _ => None,
        };
        if let Some(t) = toggled {
            s = s.checked(t);
        }
        if let Some(sel) = self.selected {
            s = s.selected(sel);
        }
        if let Some(e) = self.expanded {
            s = s.expanded(e);
        }
        if let Some(p) = self
            .placeholder
            .as_deref()
            .filter(|_| self.value.as_deref() == Some(""))
        {
            s = s.description(p);
        }
        for a in &self.actions {
            let core = match a {
                SemanticAction::Click | SemanticAction::Expand | SemanticAction::Collapse => {
                    Some(A::Activate)
                }
                SemanticAction::Focus => Some(A::Focus),
                SemanticAction::Increment => Some(A::Increment),
                SemanticAction::Decrement => Some(A::Decrement),
                SemanticAction::SetValue => Some(A::SetValue),
                SemanticAction::SetTextSelection | SemanticAction::Clipboard => None,
            };
            if let Some(c) = core {
                s = s.action(c);
            }
        }
        s
    }

    /// Declare these semantics on `el` (role, label, value, state,
    /// actions), keeping anything already declared that this does not set
    /// (for example a tooltip's description).
    #[must_use]
    pub fn apply(&self, el: livspeil_ui_core::Element) -> livspeil_ui_core::Element {
        let core = self.to_core();
        el.semantics(
            move |prev| match prev.get_description().map(str::to_owned) {
                Some(d) if core.get_description().is_none() => core.description(d),
                _ => core,
            },
        )
    }
}
