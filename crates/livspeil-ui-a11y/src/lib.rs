// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI accessibility: builds [AccessKit](accesskit) trees from the
//! UI core's retained element tree and routes AccessKit action requests
//! back to the core.
//!
//! The host (winit adapter) owns an [`A11yTree`], calls
//! [`A11yTree::update`] after each frame or focus change and forwards the
//! returned [`TreeUpdate`] to the platform adapter (VoiceOver on macOS,
//! AT-SPI on Linux). Action requests from the adapter go to
//! [`handle_action`].
//!
//! - Node ids are the core's [`ElementId`]s (`NodeId(element_id.0)`), so
//!   they are stable across renders and keyed reorders.
//! - Only semantic elements are exposed (declared role, text, focusable,
//!   labelled, and the window root); other elements are transparent and
//!   their semantic descendants are attached to the nearest exposed
//!   ancestor. Roles with presentational children (button, slider…) hide
//!   their descendants. Hidden elements are dropped with their subtree.
//! - Updates are incremental: the tree caches the last AccessKit node of
//!   every id and sends only nodes that changed (the full tree first).

use std::collections::HashMap;
use std::fmt::{self, Write as _};

use accesskit::{Action, ActionData, ActionRequest, Node, NodeId, TreeId, TreeInfo, TreeUpdate};
use livspeil_ui_core::{
    App, ElementId, Role, SemanticAction, SemanticActionError, SemanticNode, SemanticRequest,
    SemanticValue, Toggled, ValueRequest,
};

pub use accesskit;

/// Toolkit name reported to platform adapters.
pub const TOOLKIT_NAME: &str = "Livspeil UI";

/// The AccessKit node id of an element.
#[must_use]
pub fn node_id(id: ElementId) -> NodeId {
    NodeId(id.0)
}

/// The element of an AccessKit node id.
#[must_use]
pub fn element_id(id: NodeId) -> ElementId {
    ElementId(id.0)
}

/// The AccessKit role of a core role.
#[must_use]
pub fn map_role(role: Role) -> accesskit::Role {
    use accesskit::Role as R;
    match role {
        Role::Window => R::Window,
        Role::Dialog => R::Dialog,
        Role::Group => R::Group,
        Role::Label => R::Label,
        Role::Heading => R::Heading,
        Role::Button => R::Button,
        Role::Link => R::Link,
        Role::CheckBox => R::CheckBox,
        Role::Switch => R::Switch,
        Role::Slider => R::Slider,
        Role::ProgressIndicator => R::ProgressIndicator,
        Role::TextInput => R::TextInput,
        Role::Image => R::Image,
        Role::List => R::List,
        Role::ListItem => R::ListItem,
        Role::ScrollView => R::ScrollView,
        Role::Menu => R::Menu,
        Role::MenuItem => R::MenuItem,
        Role::TabList => R::TabList,
        Role::Tab => R::Tab,
        Role::Toolbar => R::Toolbar,
        // `Role` is non-exhaustive; `Generic` and future roles.
        _ => R::GenericContainer,
    }
}

/// The AccessKit action of a semantic action.
#[must_use]
pub fn map_action(action: SemanticAction) -> Action {
    match action {
        SemanticAction::Activate => Action::Click,
        SemanticAction::Focus => Action::Focus,
        SemanticAction::Increment => Action::Increment,
        SemanticAction::Decrement => Action::Decrement,
        SemanticAction::SetValue => Action::SetValue,
        SemanticAction::ScrollIntoView => Action::ScrollIntoView,
    }
}

fn map_toggled(t: Toggled) -> accesskit::Toggled {
    match t {
        Toggled::Off => accesskit::Toggled::False,
        Toggled::On => accesskit::Toggled::True,
        Toggled::Mixed => accesskit::Toggled::Mixed,
    }
}

/// Build the AccessKit node for an exposed element (without children).
fn build_node(n: &SemanticNode<'_>, role: Role, scale: f64) -> Node {
    let mut node = Node::new(map_role(role));
    if let Some(label) = n.label() {
        // AccessKit convention: a label's text is its value.
        if role == Role::Label {
            node.set_value(label);
        } else {
            node.set_label(label);
        }
    }
    let b = n.bounds;
    node.set_bounds(accesskit::Rect {
        x0: f64::from(b.x) * scale,
        y0: f64::from(b.y) * scale,
        x1: f64::from(b.x + b.width) * scale,
        y1: f64::from(b.y + b.height) * scale,
    });
    if let Some(s) = n.semantics {
        if let Some(d) = s.get_description() {
            node.set_description(d);
        }
        match s.get_value() {
            Some(SemanticValue::Text(t)) => node.set_value(&**t),
            Some(SemanticValue::Number {
                value,
                min,
                max,
                step,
            }) => {
                node.set_numeric_value(*value);
                if let Some(v) = min {
                    node.set_min_numeric_value(*v);
                }
                if let Some(v) = max {
                    node.set_max_numeric_value(*v);
                }
                if let Some(v) = step {
                    node.set_numeric_value_step(*v);
                }
            }
            None => {}
        }
        if let Some(t) = s.get_checked() {
            node.set_toggled(map_toggled(t));
        }
        if let Some(e) = s.get_expanded() {
            node.set_expanded(e);
        }
        if let Some(sel) = s.get_selected() {
            node.set_selected(sel);
        }
        if s.is_disabled() {
            node.set_disabled();
        }
    }
    for a in n.actions.iter() {
        node.add_action(map_action(a));
    }
    node
}

/// Collect the exposed ids for `id` (itself if exposed, else its exposed
/// descendants), building their nodes into `out`.
fn collect(app: &App, id: ElementId, scale: f64, out: &mut HashMap<NodeId, Node>) -> Vec<NodeId> {
    let Some(n) = app.semantic_node(id) else {
        return Vec::new();
    };
    if n.is_hidden() {
        return Vec::new();
    }
    let role = n.role();
    let presentational = role.is_some_and(Role::children_presentational);
    let mut kids = Vec::new();
    if !presentational {
        for c in n.children {
            kids.extend(collect(app, *c, scale, out));
        }
    }
    match role {
        Some(role) => {
            let mut node = build_node(&n, role, scale);
            node.set_children(kids);
            let nid = node_id(id);
            out.insert(nid, node);
            vec![nid]
        }
        None => kids,
    }
}

/// Every exposed node of the app's tree, and the root id.
fn build_all(app: &App, scale: f64) -> Option<(NodeId, HashMap<NodeId, Node>)> {
    let root = app.semantic_root()?;
    let mut out = HashMap::new();
    let ids = collect(app, root, scale, &mut out);
    let root = node_id(root);
    // The root is always exposed (window role), unless hidden.
    (ids == [root]).then_some((root, out))
}

/// The exposed node that reports the app's focus: the focused element or
/// its nearest exposed ancestor; the root when nothing is focused.
fn focus_node(app: &App, nodes: &HashMap<NodeId, Node>, root: NodeId) -> NodeId {
    let mut cur = app.focused();
    while let Some(id) = cur {
        if nodes.contains_key(&node_id(id)) {
            return node_id(id);
        }
        cur = app.semantic_node(id).and_then(|n| n.parent);
    }
    root
}

/// Counters of the last update.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UpdateStats {
    /// Exposed nodes in the tree.
    pub nodes: usize,
    /// Nodes sent (new or changed).
    pub sent: usize,
    /// Nodes removed since the previous update.
    pub removed: usize,
}

/// The accessibility tree mirrored to the platform adapter: caches the
/// last sent node of every id to produce incremental updates.
#[derive(Debug, Default)]
pub struct A11yTree {
    nodes: HashMap<NodeId, Node>,
    root: Option<NodeId>,
    focus: Option<NodeId>,
    stats: UpdateStats,
}

impl A11yTree {
    /// An empty tree; the first update sends everything.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a full tree has been sent.
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.root.is_some()
    }

    /// Exposed nodes as of the last update.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Counters of the last update.
    #[must_use]
    pub fn stats(&self) -> UpdateStats {
        self.stats
    }

    /// The node as of the last update.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    /// The root node id, once initialized.
    #[must_use]
    pub fn root(&self) -> Option<NodeId> {
        self.root
    }

    /// The focused node as of the last update.
    #[must_use]
    pub fn focus(&self) -> Option<NodeId> {
        self.focus
    }

    /// Forget what was sent; the next update is a full tree (for example
    /// when the platform adapter is (re)activated).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// A full tree update (for `ActivationHandler::request_initial_tree`).
    /// `scale` converts logical to physical pixels. `None` before the app
    /// has mounted its root.
    pub fn full_update(&mut self, app: &App, scale: f64) -> Option<TreeUpdate> {
        self.reset();
        self.update(app, scale)
    }

    /// The changes since the last update: every new or changed node, the
    /// tree info on the first update, and the focus. `None` when nothing
    /// changed (or the app has no root yet). `scale` converts logical to
    /// physical pixels.
    pub fn update(&mut self, app: &App, scale: f64) -> Option<TreeUpdate> {
        let (root, nodes) = build_all(app, scale)?;
        let focus = focus_node(app, &nodes, root);
        let first = self.root != Some(root);
        if first {
            self.nodes.clear();
        }
        let mut changed: Vec<(NodeId, Node)> = nodes
            .iter()
            .filter(|(id, n)| self.nodes.get(id) != Some(*n))
            .map(|(id, n)| (*id, n.clone()))
            .collect();
        changed.sort_unstable_by_key(|(id, _)| id.0);
        let removed = self
            .nodes
            .keys()
            .filter(|id| !nodes.contains_key(id))
            .count();
        self.stats = UpdateStats {
            nodes: nodes.len(),
            sent: changed.len(),
            removed,
        };
        let focus_changed = self.focus != Some(focus);
        self.nodes = nodes;
        self.root = Some(root);
        self.focus = Some(focus);
        if !first && changed.is_empty() && !focus_changed {
            return None;
        }
        Some(TreeUpdate {
            nodes: changed,
            tree: first.then(|| TreeInfo {
                root,
                toolkit_name: Some(TOOLKIT_NAME.to_owned()),
                toolkit_version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            tree_id: TreeId::ROOT,
            focus,
        })
    }

    /// Text dump of the tree as of the last update: one line per node
    /// with id, role, name, value, states, actions and bounds; `*` marks
    /// focus.
    #[must_use]
    pub fn dump(&self) -> String {
        let mut out = String::new();
        if let Some(root) = self.root {
            self.dump_node(root, 0, &mut out);
        }
        out
    }

    fn dump_node(&self, id: NodeId, depth: usize, out: &mut String) {
        let Some(n) = self.nodes.get(&id) else {
            let _ = writeln!(out, "{}<missing {id:?}>", "  ".repeat(depth));
            return;
        };
        let focus = if self.focus == Some(id) { "*" } else { "" };
        let _ = write!(out, "{}{focus}{}", "  ".repeat(depth), NodeDump(n));
        out.push('\n');
        for c in n.children() {
            self.dump_node(*c, depth + 1, out);
        }
    }
}

/// One-line description of an AccessKit node (for dumps and tests).
#[derive(Debug)]
pub struct NodeDump<'a>(pub &'a Node);

fn num(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    if r == r.trunc() {
        format!("{r:.0}")
    } else {
        format!("{r}")
    }
}

impl fmt::Display for NodeDump<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.0;
        write!(f, "{:?}", n.role())?;
        if let Some(l) = n.label() {
            write!(f, " {l:?}")?;
        }
        if let Some(v) = n.value() {
            write!(f, " value={v:?}")?;
        }
        if let Some(v) = n.numeric_value() {
            write!(f, " value={}", num(v))?;
            let range = (n.min_numeric_value(), n.max_numeric_value());
            if let (Some(a), Some(b)) = range {
                write!(f, " range={}..{}", num(a), num(b))?;
            }
            if let Some(s) = n.numeric_value_step() {
                write!(f, " step={}", num(s))?;
            }
        }
        if let Some(d) = n.description() {
            write!(f, " desc={d:?}")?;
        }
        if let Some(t) = n.toggled() {
            write!(f, " toggled={t:?}")?;
        }
        if let Some(e) = n.is_expanded() {
            write!(f, " expanded={e}")?;
        }
        if let Some(s) = n.is_selected() {
            write!(f, " selected={s}")?;
        }
        if n.is_disabled() {
            write!(f, " disabled")?;
        }
        let actions: Vec<String> = [
            Action::Click,
            Action::Focus,
            Action::Increment,
            Action::Decrement,
            Action::SetValue,
            Action::ScrollIntoView,
        ]
        .into_iter()
        .filter(|a| n.supports_action(*a))
        .map(|a| format!("{a:?}"))
        .collect();
        if !actions.is_empty() {
            write!(f, " [{}]", actions.join(","))?;
        }
        if let Some(b) = n.bounds() {
            write!(
                f,
                " @{},{} {}x{}",
                num(b.x0),
                num(b.y0),
                num(b.x1 - b.x0),
                num(b.y1 - b.y0)
            )?;
        }
        Ok(())
    }
}

/// Why an action request was not performed.
#[derive(Clone, Debug, PartialEq)]
pub enum A11yError {
    /// The request targets a tree this app does not own.
    UnknownTree(TreeId),
    /// The action has no core equivalent.
    UnsupportedAction(Action),
    /// The action needs data that was missing or of the wrong kind.
    BadData(Action),
    /// The core refused the action.
    Semantic(SemanticActionError),
}

impl fmt::Display for A11yError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTree(t) => write!(f, "unknown accessibility tree {t:?}"),
            Self::UnsupportedAction(a) => write!(f, "unsupported accessibility action {a:?}"),
            Self::BadData(a) => write!(f, "missing or invalid data for {a:?}"),
            Self::Semantic(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for A11yError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Semantic(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SemanticActionError> for A11yError {
    fn from(e: SemanticActionError) -> Self {
        A11yError::Semantic(e)
    }
}

/// The core request for an AccessKit action request.
///
/// # Errors
/// The action has no core equivalent or lacks its data.
pub fn map_request(request: &ActionRequest) -> Result<SemanticRequest, A11yError> {
    if request.target_tree != TreeId::ROOT {
        return Err(A11yError::UnknownTree(request.target_tree));
    }
    Ok(match request.action {
        Action::Click => SemanticRequest::Activate,
        Action::Focus => SemanticRequest::Focus,
        Action::Blur => SemanticRequest::Blur,
        Action::Increment => SemanticRequest::Increment,
        Action::Decrement => SemanticRequest::Decrement,
        Action::ScrollIntoView => SemanticRequest::ScrollIntoView,
        Action::SetValue => match &request.data {
            Some(ActionData::Value(v)) => {
                SemanticRequest::SetValue(ValueRequest::Text(v.to_string()))
            }
            Some(ActionData::NumericValue(v)) => {
                SemanticRequest::SetValue(ValueRequest::Number(*v))
            }
            _ => return Err(A11yError::BadData(Action::SetValue)),
        },
        other => return Err(A11yError::UnsupportedAction(other)),
    })
}

/// Perform an AccessKit action request on the app: click → activate (the
/// click handler, or `ui.activate`), focus, blur, increment/decrement
/// (`ui.increment`/`ui.decrement` from the target), set value, scroll
/// into view (`ui.scroll_into_view` bubbling from the target). Returns
/// whether something handled it.
///
/// # Errors
/// See [`A11yError`].
pub fn handle_action(app: &mut App, request: ActionRequest) -> Result<bool, A11yError> {
    let req = map_request(&request)?;
    Ok(app.perform_semantic_action(element_id(request.target_node), req)?)
}
