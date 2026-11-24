// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! ARIA-like semantics of a widget: role, label, value, state and the
//! actions it supports. Recorded per widget so the accessibility layer
//! (`livspeil-ui-a11y`, AccessKit) can be wired to it later.

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
        self.actions = if disabled { Vec::new() } else { actions.to_vec() };
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
