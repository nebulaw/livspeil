// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Semantic diff of two documents.
//!
//! [`diff`] matches nodes by id across the whole tree (the root network and
//! every nested subnet network), so a rename is one change instead of a
//! removal and an addition, a node moved into another subnet is one
//! [`Subject::NodeMove`], and links and meta positions are matched through
//! the ids of the nodes they name (renaming a node does not show its links
//! as changed). Every value is compared and shown by its canonical KDL text
//! (the text [`crate::format()`] writes), so the diff agrees with what is
//! on disk and new scalar value types need no diff code.
//!
//! Nodes are named by their path from the root without the leading `/`
//! (`lamp`, `desk_lamp/sunrise`), as in `meta`; networks by their path
//! with it (`/`, `/desk_lamp`).
//!
//! # Order
//!
//! The result is deterministic: room header (name, format, min-tier),
//! then nodes by id (for each: added/removed, name, type, network move,
//! `port-type`, Charm metadata, then its ports by name, each port as static
//! value, expression, animation), then links by network path and
//! `(from, to)` text, then meta positions by node id. An added node lists
//! its `port-type`, Charm and input sources as added too; a removed node is
//! one change (the nodes inside a removed subnet are each one change).
//!
//! # Animation and curves
//!
//! A channel that appears or disappears is one [`Subject::Anim`] change
//! with a summary (`3 keys 0.0..2.0 pre="loop"`). A channel that changes
//! is reported as setting changes ([`Subject::AnimSetting`]: `pre`, `post`,
//! `offset`) and key changes ([`Subject::AnimKey`]), keys matched by time.
//! A key's text is its value followed by its non-default properties, as in
//! `room.kdl` (`0.8 interp="step"`). A `Curve` input value that changes is
//! reported the same way ([`Subject::CurveSetting`], [`Subject::CurveKey`],
//! keys matched by position).
//!
//! # Tables
//!
//! A `Table` input value that changes is one [`Subject::Table`] change with
//! a size summary (`2 cols x 3 rows -> 2 cols x 4 rows`) and the number of
//! changed cells, followed, when at most [`TABLE_DETAIL_CELLS`] cells
//! changed, by column changes ([`Subject::TableColumn`], by name: added,
//! removed or retyped) and row changes ([`Subject::TableRow`], by index,
//! the row's cells as `room.kdl` writes them).
//!
//! # Charms
//!
//! A `charm` block that appears or disappears is one [`Subject::Charm`]
//! change with a summary (`"Daylight" version="1.0.0" min-tier="T1"
//! capability="time"`). A changed one is reported as
//! [`Subject::CharmField`] (`name`, `version`, `icon`, `min-tier`,
//! `capabilities` for a pure reordering) and [`Subject::CharmCapability`]
//! (one capability added or removed).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use livspeil_graph::{CharmMeta, NodeId};
use livspeil_node_api::{Table, Value};
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe, Tangent};

use crate::anim::{extrapolation_name, interpolation_name};
use crate::model::{DocLink, DocNode, Document, Network, ROOT_NETWORK};
use crate::value::{format_float, format_value, quote};

/// At most this many changed cells, a table change lists its column and
/// row changes; with more, only the summary.
pub const TABLE_DETAIL_CELLS: usize = 20;

/// Whether something appeared, disappeared or changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Op {
    Added,
    Removed,
    Changed,
}

impl Op {
    /// `+`, `-` or `~`.
    #[must_use]
    pub fn sign(self) -> char {
        match self {
            Op::Added => '+',
            Op::Removed => '-',
            Op::Changed => '~',
        }
    }

    /// `added`, `removed` or `changed`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Op::Added => "added",
            Op::Removed => "removed",
            Op::Changed => "changed",
        }
    }
}

/// A node as the diff names it: its id, its name and its path from the
/// root without the leading `/` (`lamp`, `desk_lamp/sunrise`), both in the
/// newer document (the older one if it was removed). For a node of the
/// root network the path is the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRef {
    pub id: NodeId,
    pub name: String,
    pub path: String,
}

impl NodeRef {
    /// Whether the node is inside a subnet.
    #[must_use]
    pub fn is_nested(&self) -> bool {
        self.path != self.name
    }
}

/// What a [`Change`] is about.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Subject {
    /// The room's display name.
    RoomName,
    /// The document format version.
    Format,
    /// The room's minimum tier.
    MinTier,
    /// A whole node; `old`/`new` hold its type id.
    Node(NodeRef),
    /// A node's name (a rename); `node.name` is the new name.
    NodeName(NodeRef),
    /// A node's type id.
    NodeType(NodeRef),
    /// A node moved to another network; `old`/`new` are the quoted network
    /// paths (`"/"`, `"/desk_lamp"`).
    NodeMove(NodeRef),
    /// The declared `port-type` of a boundary node; `old`/`new` are the
    /// quoted type names.
    PortType(NodeRef),
    /// A whole `charm` block of a subnet; `old`/`new` are summaries.
    Charm(NodeRef),
    /// One field of a `charm` block: `name`, `version`, `icon`, `min-tier`
    /// or `capabilities` (only when the same capabilities were reordered).
    CharmField { node: NodeRef, field: &'static str },
    /// One capability of a `charm` block, added or removed; `old`/`new`
    /// is the quoted capability.
    CharmCapability(NodeRef),
    /// The static value of an input.
    Input { node: NodeRef, port: String },
    /// The expression on an input.
    Expr { node: NodeRef, port: String },
    /// A whole animation channel on an input; `old`/`new` are summaries.
    Anim { node: NodeRef, port: String },
    /// `pre`, `post` or `offset` of an animation channel.
    AnimSetting {
        node: NodeRef,
        port: String,
        setting: &'static str,
    },
    /// One key of an animation channel, matched by its time (canonical
    /// float text).
    AnimKey {
        node: NodeRef,
        port: String,
        time: String,
    },
    /// A `Table` input value that changed (both sides tables); `old`/`new`
    /// are sizes (`2 cols x 3 rows`), `cells` the number of changed cells.
    Table {
        node: NodeRef,
        port: String,
        cells: usize,
    },
    /// A column of a changed table, by name; `old`/`new` are the quoted
    /// column type.
    TableColumn {
        node: NodeRef,
        port: String,
        column: String,
    },
    /// A row of a changed table, by index; `old`/`new` are its cells as
    /// `room.kdl` writes them (`1.0 "a"`).
    TableRow {
        node: NodeRef,
        port: String,
        row: usize,
    },
    /// `pre` or `post` of a `Curve` input value.
    CurveSetting {
        node: NodeRef,
        port: String,
        setting: &'static str,
    },
    /// One key of a `Curve` input value, matched by its position
    /// (canonical float text).
    CurveKey {
        node: NodeRef,
        port: String,
        time: String,
    },
    /// A link, as `node.port` texts with current names, in the network at
    /// `network` (`/` for the root).
    Link {
        from: String,
        to: String,
        network: String,
    },
    /// A node's editor position (meta), by its meta key (the node's path);
    /// `old`/`new` are `x y`.
    Position { node: String, id: Option<NodeId> },
}

impl Subject {
    /// A stable kebab-case name, used as `kind` in JSON output.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Subject::RoomName => "room-name",
            Subject::Format => "format",
            Subject::MinTier => "min-tier",
            Subject::Node(_) => "node",
            Subject::NodeName(_) => "node-name",
            Subject::NodeType(_) => "node-type",
            Subject::NodeMove(_) => "node-move",
            Subject::PortType(_) => "port-type",
            Subject::Charm(_) => "charm",
            Subject::CharmField { .. } => "charm-field",
            Subject::CharmCapability(_) => "charm-capability",
            Subject::Input { .. } => "input",
            Subject::Expr { .. } => "expr",
            Subject::Anim { .. } => "anim",
            Subject::AnimSetting { .. } => "anim-setting",
            Subject::AnimKey { .. } => "anim-key",
            Subject::Table { .. } => "table",
            Subject::TableColumn { .. } => "table-column",
            Subject::TableRow { .. } => "table-row",
            Subject::CurveSetting { .. } => "curve-setting",
            Subject::CurveKey { .. } => "curve-key",
            Subject::Link { .. } => "link",
            Subject::Position { .. } => "position",
        }
    }

    /// The node this is about, if any.
    #[must_use]
    pub fn node(&self) -> Option<&NodeRef> {
        match self {
            Subject::Node(node)
            | Subject::NodeName(node)
            | Subject::NodeType(node)
            | Subject::NodeMove(node)
            | Subject::PortType(node)
            | Subject::Charm(node)
            | Subject::CharmField { node, .. }
            | Subject::CharmCapability(node)
            | Subject::Input { node, .. }
            | Subject::Expr { node, .. }
            | Subject::Anim { node, .. }
            | Subject::AnimSetting { node, .. }
            | Subject::AnimKey { node, .. }
            | Subject::Table { node, .. }
            | Subject::TableColumn { node, .. }
            | Subject::TableRow { node, .. }
            | Subject::CurveSetting { node, .. }
            | Subject::CurveKey { node, .. } => Some(node),
            _ => None,
        }
    }

    /// The input port this is about, if any.
    #[must_use]
    pub fn port(&self) -> Option<&str> {
        match self {
            Subject::Input { port, .. }
            | Subject::Expr { port, .. }
            | Subject::Anim { port, .. }
            | Subject::AnimSetting { port, .. }
            | Subject::AnimKey { port, .. }
            | Subject::Table { port, .. }
            | Subject::TableColumn { port, .. }
            | Subject::TableRow { port, .. }
            | Subject::CurveSetting { port, .. }
            | Subject::CurveKey { port, .. } => Some(port),
            _ => None,
        }
    }

    /// Whether this is editor-only data (meta), which tools usually hide.
    #[must_use]
    pub fn is_meta(&self) -> bool {
        matches!(self, Subject::Position { .. })
    }
}

/// One difference. `old` is set unless `op` is [`Op::Added`], `new` unless
/// it is [`Op::Removed`]; both are canonical KDL text (strings quoted),
/// except for the summaries of [`Subject::Anim`], [`Subject::Charm`],
/// [`Subject::Table`], [`Subject::TableRow`], [`Subject::Position`] and of
/// `Table`/`Curve` values of [`Subject::Input`] (`table 2 cols x 3 rows`,
/// `curve 2 keys 0.0..1.0`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub op: Op,
    pub subject: Subject,
    pub old: Option<String>,
    pub new: Option<String>,
}

impl Change {
    fn new(op: Op, subject: Subject, old: Option<String>, new: Option<String>) -> Self {
        Change {
            op,
            subject,
            old,
            new,
        }
    }

    fn pair(subject: Subject, old: String, new: String) -> Option<Self> {
        (old != new).then(|| Change::new(Op::Changed, subject, Some(old), Some(new)))
    }

    fn opt(subject: Subject, old: Option<String>, new: Option<String>) -> Option<Self> {
        let op = match (&old, &new) {
            (None, None) => return None,
            (Some(a), Some(b)) if a == b => return None,
            (None, Some(_)) => Op::Added,
            (Some(_), None) => Op::Removed,
            (Some(_), Some(_)) => Op::Changed,
        };
        Some(Change::new(op, subject, old, new))
    }
}

/// One line, as `livspeil diff` prints it:
///
/// ```text
/// ~ room name: "Old" -> "New"
/// + node "lamp" type="core.math.lerp@1" id="..."
/// ~ node "lamp" renamed: "light" -> "lamp"
/// ~ node "lamp" input "a": 0.2 -> 0.3
/// + node "lamp" input "b" anim key 1.0: 0.8 interp="step"
/// ~ node "desk/x" moved: "/" -> "/desk"
/// ~ node "desk/in" port-type: "Float" -> "Color"
/// ~ node "desk" charm version: "1.0.0" -> "1.1.0"
/// + node "desk" charm capability: "net"
/// ~ node "desk/t" input "rows" table: 2 cols x 3 rows -> 2 cols x 4 rows (2 cells changed)
/// + node "desk/t" input "rows" table row 3: 4.0 "d"
/// ~ node "desk/r" input "curve" curve key 0.5: 1.0 -> 2.0
/// - link "sun.phase" -> "lamp.t"
/// + link "in.value" -> "x.a" in "/desk"
/// ~ meta node "lamp" position: 0.0 0.0 -> 10.0 20.0
/// ```
impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.op.sign())?;
        let at = |node: &NodeRef| quote(&node.path);
        let input = |node: &NodeRef, port: &str| format!("node {} input {}", at(node), quote(port));
        match &self.subject {
            Subject::RoomName => f.write_str("room name")?,
            Subject::Format => f.write_str("room format")?,
            Subject::MinTier => f.write_str("room min-tier")?,
            Subject::Node(node) => {
                // `old`/`new` is the type; print it like the document does.
                let ty = self.new.as_deref().or(self.old.as_deref()).unwrap_or("");
                return write!(
                    f,
                    "node {} type={ty} id={}",
                    at(node),
                    quote(&node.id.to_string())
                );
            }
            Subject::NodeName(node) => write!(f, "node {} renamed", at(node))?,
            Subject::NodeType(node) => write!(f, "node {} type", at(node))?,
            Subject::NodeMove(node) => write!(f, "node {} moved", at(node))?,
            Subject::PortType(node) => write!(f, "node {} port-type", at(node))?,
            Subject::Charm(node) => write!(f, "node {} charm", at(node))?,
            Subject::CharmField { node, field } => {
                write!(f, "node {} charm {field}", at(node))?;
            }
            Subject::CharmCapability(node) => write!(f, "node {} charm capability", at(node))?,
            Subject::Input { node, port } => f.write_str(&input(node, port))?,
            Subject::Expr { node, port } => write!(f, "{} expr", input(node, port))?,
            Subject::Anim { node, port } => write!(f, "{} anim", input(node, port))?,
            Subject::AnimSetting {
                node,
                port,
                setting,
            } => write!(f, "{} anim {setting}", input(node, port))?,
            Subject::AnimKey { node, port, time } => {
                write!(f, "{} anim key {time}", input(node, port))?;
            }
            Subject::Table { node, port, cells } => {
                write!(f, "{} table", input(node, port))?;
                write_values(f, self)?;
                let s = if *cells == 1 { "" } else { "s" };
                return write!(f, " ({cells} cell{s} changed)");
            }
            Subject::TableColumn { node, port, column } => {
                write!(f, "{} table column {}", input(node, port), quote(column))?;
            }
            Subject::TableRow { node, port, row } => {
                write!(f, "{} table row {row}", input(node, port))?;
            }
            Subject::CurveSetting {
                node,
                port,
                setting,
            } => write!(f, "{} curve {setting}", input(node, port))?,
            Subject::CurveKey { node, port, time } => {
                write!(f, "{} curve key {time}", input(node, port))?;
            }
            Subject::Link { from, to, network } => {
                write!(f, "link {} -> {}", quote(from), quote(to))?;
                if network != ROOT_NETWORK {
                    write!(f, " in {}", quote(network))?;
                }
                return Ok(());
            }
            Subject::Position { node, .. } => write!(f, "meta node {} position", quote(node))?,
        }
        write_values(f, self)
    }
}

/// `: old -> new`, `: value` or nothing.
fn write_values(f: &mut fmt::Formatter<'_>, change: &Change) -> fmt::Result {
    match (&change.old, &change.new) {
        (Some(old), Some(new)) => write!(f, ": {old} -> {new}"),
        (Some(v), None) | (None, Some(v)) => write!(f, ": {v}"),
        (None, None) => Ok(()),
    }
}

/// The differences between two documents, in the order described in the
/// module docs. Empty if they are semantically equal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DocDiff {
    pub changes: Vec<Change>,
}

impl DocDiff {
    /// Whether the documents are equal.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Number of changes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// The changes, without editor-only (meta) ones.
    pub fn without_meta(&self) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(|c| !c.subject.is_meta())
    }
}

/// A node of a document with where it sits in the tree.
struct Entry<'a> {
    node: &'a DocNode,
    /// Path without the leading `/`.
    path: String,
    /// The subnet holding it, `None` in the root network.
    owner: Option<NodeId>,
    /// Path of the network holding it (`/`, `/desk`).
    network: String,
}

impl Entry<'_> {
    fn node_ref(&self) -> NodeRef {
        NodeRef {
            id: self.node.id,
            name: self.node.name.clone(),
            path: self.path.clone(),
        }
    }
}

/// A network of a document: its owning subnet and path.
struct Net<'a> {
    owner: Option<NodeId>,
    path: String,
    network: &'a Network,
}

/// Every node and network of a document.
struct Tree<'a> {
    nodes: BTreeMap<NodeId, Entry<'a>>,
    networks: Vec<Net<'a>>,
    /// Node id by path, for meta keys.
    paths: BTreeMap<String, NodeId>,
}

impl<'a> Tree<'a> {
    fn new(doc: &'a Document) -> Self {
        let mut tree = Tree {
            nodes: BTreeMap::new(),
            networks: Vec::new(),
            paths: BTreeMap::new(),
        };
        // (owner, node path prefix, network path, network)
        let mut stack: Vec<(Option<NodeId>, String, String, &Network)> =
            vec![(None, String::new(), ROOT_NETWORK.to_owned(), &doc.network)];
        while let Some((owner, prefix, net_path, network)) = stack.pop() {
            for node in &network.nodes {
                let path = format!("{prefix}{}", node.name);
                if let Some(inner) = &node.network {
                    stack.push((Some(node.id), format!("{path}/"), format!("/{path}"), inner));
                }
                tree.paths.entry(path.clone()).or_insert(node.id);
                tree.nodes.entry(node.id).or_insert(Entry {
                    node,
                    path,
                    owner,
                    network: net_path.clone(),
                });
            }
            tree.networks.push(Net {
                owner,
                path: net_path,
                network,
            });
        }
        tree
    }
}

/// Compare `old` with `new`. See the module docs for what is reported and
/// in which order.
#[must_use]
pub fn diff(old: &Document, new: &Document) -> DocDiff {
    let mut out = Vec::new();

    out.extend(Change::pair(
        Subject::RoomName,
        quote(&old.name),
        quote(&new.name),
    ));
    out.extend(Change::pair(
        Subject::Format,
        old.format.to_string(),
        new.format.to_string(),
    ));
    out.extend(Change::pair(
        Subject::MinTier,
        quote(&old.min_tier.to_string()),
        quote(&new.min_tier.to_string()),
    ));

    let old_tree = Tree::new(old);
    let new_tree = Tree::new(new);
    let ids: BTreeSet<NodeId> = old_tree
        .nodes
        .keys()
        .chain(new_tree.nodes.keys())
        .copied()
        .collect();
    for id in ids {
        match (old_tree.nodes.get(&id), new_tree.nodes.get(&id)) {
            (Some(a), None) => out.push(Change::new(
                Op::Removed,
                Subject::Node(a.node_ref()),
                Some(quote(&a.node.type_id)),
                None,
            )),
            (None, Some(b)) => {
                let r = b.node_ref();
                out.push(Change::new(
                    Op::Added,
                    Subject::Node(r.clone()),
                    None,
                    Some(quote(&b.node.type_id)),
                ));
                let empty = DocNode::new(b.node.id, b.node.name.clone(), "");
                node_details(&mut out, &r, &empty, b.node);
            }
            (Some(a), Some(b)) => {
                let r = b.node_ref();
                out.extend(Change::pair(
                    Subject::NodeName(r.clone()),
                    quote(&a.node.name),
                    quote(&b.node.name),
                ));
                out.extend(Change::pair(
                    Subject::NodeType(r.clone()),
                    quote(&a.node.type_id),
                    quote(&b.node.type_id),
                ));
                if a.owner != b.owner {
                    out.push(Change::new(
                        Op::Changed,
                        Subject::NodeMove(r.clone()),
                        Some(quote(&a.network)),
                        Some(quote(&b.network)),
                    ));
                }
                node_details(&mut out, &r, a.node, b.node);
            }
            (None, None) => {}
        }
    }

    links(&mut out, &old_tree, &new_tree);
    positions(&mut out, old, new, &old_tree, &new_tree);
    DocDiff { changes: out }
}

/// Canonical text of a value; `Table` and `Curve` values (written as
/// blocks) as summaries. Values the format cannot store yet (which a
/// parsed document never holds) fall back to their debug form.
fn value_text(value: &Value) -> String {
    match value {
        Value::Table(table) => format!("table {}", table_size(table)),
        Value::Curve(curve) => match AnimChannel::from_curve(curve) {
            Ok(channel) => format!("curve {}", anim_summary(&channel)),
            Err(_) => format!("{value:?}"),
        },
        _ => format_value(value).unwrap_or_else(|| format!("{value:?}")),
    }
}

/// `port-type`, Charm and input sources of a node present in both (or
/// added: `a` is then an empty node).
fn node_details(out: &mut Vec<Change>, node: &NodeRef, a: &DocNode, b: &DocNode) {
    out.extend(Change::opt(
        Subject::PortType(node.clone()),
        a.port_type.map(|t| quote(t.name())),
        b.port_type.map(|t| quote(t.name())),
    ));
    match (&a.charm, &b.charm) {
        (Some(x), Some(y)) => charm_changes(out, node, x, y),
        (x, y) => out.extend(Change::opt(
            Subject::Charm(node.clone()),
            x.as_ref().map(charm_summary),
            y.as_ref().map(charm_summary),
        )),
    }
    node_sources(out, node, a, b);
}

fn tier_text(tier: u8) -> String {
    quote(&format!("T{tier}"))
}

/// `"Daylight" version="1.0.0" icon="sun" min-tier="T1" capability="time"`,
/// non-default fields only.
fn charm_summary(charm: &CharmMeta) -> String {
    let mut s = quote(&charm.name);
    if let Some(version) = &charm.version {
        s.push_str(&format!(" version={}", quote(version)));
    }
    if let Some(icon) = &charm.icon {
        s.push_str(&format!(" icon={}", quote(icon)));
    }
    if charm.min_tier != 0 {
        s.push_str(&format!(" min-tier={}", tier_text(charm.min_tier)));
    }
    for capability in &charm.capabilities {
        s.push_str(&format!(" capability={}", quote(capability)));
    }
    s
}

fn charm_changes(out: &mut Vec<Change>, node: &NodeRef, a: &CharmMeta, b: &CharmMeta) {
    let field = |field: &'static str| Subject::CharmField {
        node: node.clone(),
        field,
    };
    out.extend(Change::pair(field("name"), quote(&a.name), quote(&b.name)));
    out.extend(Change::opt(
        field("version"),
        a.version.as_deref().map(quote),
        b.version.as_deref().map(quote),
    ));
    out.extend(Change::opt(
        field("icon"),
        a.icon.as_deref().map(quote),
        b.icon.as_deref().map(quote),
    ));
    out.extend(Change::pair(
        field("min-tier"),
        tier_text(a.min_tier),
        tier_text(b.min_tier),
    ));
    let old: BTreeSet<&String> = a.capabilities.iter().collect();
    let new: BTreeSet<&String> = b.capabilities.iter().collect();
    for capability in old.union(&new) {
        out.extend(Change::opt(
            Subject::CharmCapability(node.clone()),
            old.contains(capability).then(|| quote(capability)),
            new.contains(capability).then(|| quote(capability)),
        ));
    }
    if old == new && a.capabilities != b.capabilities {
        let list = |c: &[String]| c.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
        out.extend(Change::pair(
            field("capabilities"),
            list(&a.capabilities),
            list(&b.capabilities),
        ));
    }
}

/// Inputs, expressions and animations of a node present in both (or added).
fn node_sources(out: &mut Vec<Change>, node: &NodeRef, a: &DocNode, b: &DocNode) {
    let ports: BTreeSet<&str> = a.ports().chain(b.ports()).collect();
    for port in ports {
        let p = port.to_owned();
        match (a.inputs.get(port), b.inputs.get(port)) {
            (Some(Value::Table(x)), Some(Value::Table(y))) if x != y => {
                table_changes(out, node, port, x, y);
            }
            (Some(Value::Curve(x)), Some(Value::Curve(y))) if x != y => {
                match (AnimChannel::from_curve(x), AnimChannel::from_curve(y)) {
                    (Ok(x), Ok(y)) => channel_changes(
                        out,
                        &x,
                        &y,
                        |setting| Subject::CurveSetting {
                            node: node.clone(),
                            port: p.clone(),
                            setting,
                        },
                        |time| Subject::CurveKey {
                            node: node.clone(),
                            port: p.clone(),
                            time,
                        },
                    ),
                    _ => out.push(Change::new(
                        Op::Changed,
                        Subject::Input {
                            node: node.clone(),
                            port: p.clone(),
                        },
                        Some(format!("{x:?}")),
                        Some(format!("{y:?}")),
                    )),
                }
            }
            (x, y) => out.extend(Change::opt(
                Subject::Input {
                    node: node.clone(),
                    port: p.clone(),
                },
                x.map(value_text),
                y.map(value_text),
            )),
        }
        out.extend(Change::opt(
            Subject::Expr {
                node: node.clone(),
                port: p.clone(),
            },
            a.expressions.get(port).map(|e| quote(&e.source)),
            b.expressions.get(port).map(|e| quote(&e.source)),
        ));
        let old = a.animations.get(port).map(|x| &x.channel);
        let new = b.animations.get(port).map(|x| &x.channel);
        match (old, new) {
            (Some(x), Some(y)) => channel_changes(
                out,
                x,
                y,
                |setting| Subject::AnimSetting {
                    node: node.clone(),
                    port: p.clone(),
                    setting,
                },
                |time| Subject::AnimKey {
                    node: node.clone(),
                    port: p.clone(),
                    time,
                },
            ),
            (x, y) => out.extend(Change::opt(
                Subject::Anim {
                    node: node.clone(),
                    port: p.clone(),
                },
                x.map(anim_summary),
                y.map(anim_summary),
            )),
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `2 cols x 3 rows`.
fn table_size(table: &Table) -> String {
    format!(
        "{} x {}",
        plural(table.column_count(), "col", "cols"),
        plural(table.row_count(), "row", "rows")
    )
}

/// A row's cells as `room.kdl` writes them (`1.0 "a"`).
fn row_text(table: &Table, row: usize) -> Option<String> {
    (row < table.row_count()).then(|| table.row(row).map(value_text).collect::<Vec<_>>().join(" "))
}

fn table_changes(out: &mut Vec<Change>, node: &NodeRef, port: &str, a: &Table, b: &Table) {
    let names: BTreeSet<&str> = a
        .columns()
        .iter()
        .chain(b.columns())
        .map(|c| c.name())
        .collect();
    let rows = a.row_count().max(b.row_count());
    let mut row_changes = Vec::new();
    let mut cells = 0;
    for row in 0..rows {
        let (old, new) = (row_text(a, row), row_text(b, row));
        if old == new {
            continue;
        }
        // Cells compared by column name; a row that differs only by column
        // order counts all its cells.
        let cell = |t: &Table, name: &str| {
            t.column(name)
                .and_then(|c| c.values().get(row))
                .map(value_text)
        };
        let changed = names
            .iter()
            .filter(|name| cell(a, name) != cell(b, name))
            .count();
        cells += if changed == 0 { names.len() } else { changed };
        row_changes.push(Change::opt(
            Subject::TableRow {
                node: node.clone(),
                port: port.to_owned(),
                row,
            },
            old,
            new,
        ));
    }
    out.push(Change::new(
        Op::Changed,
        Subject::Table {
            node: node.clone(),
            port: port.to_owned(),
            cells,
        },
        Some(table_size(a)),
        Some(table_size(b)),
    ));
    if cells > TABLE_DETAIL_CELLS {
        return;
    }
    for name in names {
        let ty = |t: &Table| t.column(name).map(|c| quote(c.ty().name()));
        out.extend(Change::opt(
            Subject::TableColumn {
                node: node.clone(),
                port: port.to_owned(),
                column: name.to_owned(),
            },
            ty(a),
            ty(b),
        ));
    }
    out.extend(row_changes.into_iter().flatten());
}

fn extrapolation_text(e: Extrapolation) -> String {
    quote(extrapolation_name(e))
}

/// `3 keys 0.0..2.0` plus non-default settings, for a whole channel.
fn anim_summary(channel: &AnimChannel) -> String {
    let keys = channel.keys();
    let mut s = match (keys.first(), keys.last()) {
        (Some(first), Some(last)) => format!(
            "{} {}..{}",
            plural(keys.len(), "key", "keys"),
            format_float(first.time),
            format_float(last.time)
        ),
        _ => "0 keys".to_owned(),
    };
    if channel.pre() != Extrapolation::Hold {
        s.push_str(&format!(" pre={}", extrapolation_text(channel.pre())));
    }
    if channel.post() != Extrapolation::Hold {
        s.push_str(&format!(" post={}", extrapolation_text(channel.post())));
    }
    if channel.offset().to_bits() != 0 {
        s.push_str(&format!(" offset={}", format_float(channel.offset())));
    }
    s
}

/// A key as `room.kdl` writes it, without the leading time.
fn key_text(key: &Keyframe) -> String {
    let mut s = value_text(&key.value);
    if key.interpolation != Interpolation::Linear {
        s.push_str(&format!(
            " interp={}",
            quote(interpolation_name(&key.interpolation))
        ));
    }
    if let Interpolation::Spring { frequency, damping } = key.interpolation {
        s.push_str(&format!(
            " frequency={} damping={}",
            format_float(frequency),
            format_float(damping)
        ));
    }
    match &key.tangent {
        Tangent::Auto => {}
        Tangent::Flat => s.push_str(" tangent=\"flat\""),
        Tangent::Explicit { incoming, outgoing } => s.push_str(&format!(
            " tangent=\"explicit\" in={} out={}",
            value_text(incoming),
            value_text(outgoing)
        )),
    }
    s
}

/// Settings and keys of a channel (an animation, or a curve converted to
/// one) present on both sides.
fn channel_changes(
    out: &mut Vec<Change>,
    a: &AnimChannel,
    b: &AnimChannel,
    setting: impl Fn(&'static str) -> Subject,
    key: impl Fn(String) -> Subject,
) {
    out.extend(Change::pair(
        setting("pre"),
        extrapolation_text(a.pre()),
        extrapolation_text(b.pre()),
    ));
    out.extend(Change::pair(
        setting("post"),
        extrapolation_text(a.post()),
        extrapolation_text(b.post()),
    ));
    out.extend(Change::pair(
        setting("offset"),
        format_float(a.offset()),
        format_float(b.offset()),
    ));

    // Keys by time; times are finite, so `total_cmp` is a plain order.
    let mut times: Vec<f64> = a.keys().iter().chain(b.keys()).map(|k| k.time).collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|x, y| x.to_bits() == y.to_bits());
    let find = |c: &AnimChannel, t: f64| {
        c.keys()
            .iter()
            .find(|k| k.time.to_bits() == t.to_bits())
            .map(key_text)
    };
    for t in times {
        out.extend(Change::opt(key(format_float(t)), find(a, t), find(b, t)));
    }
}

/// A node named by a link or meta entry: its id if the name exists, else
/// the bare name (only in documents that would not build a graph).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKey {
    Id(NodeId),
    Name(String),
}

fn node_key(network: &Network, name: &str) -> NodeKey {
    network
        .nodes
        .iter()
        .find(|n| n.name == name)
        .map_or_else(|| NodeKey::Name(name.to_owned()), |n| NodeKey::Id(n.id))
}

/// The owning subnet (`None` for the root) and the endpoints.
type LinkKey = (Option<NodeId>, NodeKey, String, NodeKey, String);

fn collect_links<'a>(tree: &'a Tree<'_>) -> BTreeMap<LinkKey, (&'a str, &'a DocLink)> {
    let mut map = BTreeMap::new();
    for net in &tree.networks {
        for link in &net.network.links {
            let key = (
                net.owner,
                node_key(net.network, &link.from_node),
                link.from_port.clone(),
                node_key(net.network, &link.to_node),
                link.to_port.clone(),
            );
            map.insert(key, (net.path.as_str(), link));
        }
    }
    map
}

fn links(out: &mut Vec<Change>, old: &Tree<'_>, new: &Tree<'_>) {
    let old_links = collect_links(old);
    let new_links = collect_links(new);
    let mut changes = Vec::new();
    for (key, link) in &old_links {
        if !new_links.contains_key(key) {
            changes.push((Op::Removed, *link));
        }
    }
    for (key, link) in &new_links {
        if !old_links.contains_key(key) {
            changes.push((Op::Added, *link));
        }
    }
    let mut changes: Vec<(String, String, String, Op)> = changes
        .into_iter()
        .map(|(op, (network, l))| {
            (
                network.to_owned(),
                format!("{}.{}", l.from_node, l.from_port),
                format!("{}.{}", l.to_node, l.to_port),
                op,
            )
        })
        .collect();
    // Removals before additions of the same text (a node was replaced).
    changes.sort_by(|x, y| {
        (&x.0, &x.1, &x.2, x.3 == Op::Added).cmp(&(&y.0, &y.1, &y.2, y.3 == Op::Added))
    });
    for (network, from, to, op) in changes {
        out.push(Change::new(
            op,
            Subject::Link { from, to, network },
            None,
            None,
        ));
    }
}

fn positions(
    out: &mut Vec<Change>,
    old: &Document,
    new: &Document,
    old_tree: &Tree<'_>,
    new_tree: &Tree<'_>,
) {
    let text = |[x, y]: [f64; 2]| format!("{} {}", format_float(x), format_float(y));
    let collect = |doc: &Document, tree: &Tree<'_>| -> BTreeMap<NodeKey, (String, [f64; 2])> {
        doc.meta
            .positions
            .iter()
            .map(|(path, pos)| {
                let key = tree
                    .paths
                    .get(path)
                    .map_or_else(|| NodeKey::Name(path.clone()), |id| NodeKey::Id(*id));
                (key, (path.clone(), *pos))
            })
            .collect()
    };
    let a = collect(old, old_tree);
    let b = collect(new, new_tree);
    let keys: BTreeSet<&NodeKey> = a.keys().chain(b.keys()).collect();
    for key in keys {
        let (name, _) = b
            .get(key)
            .or_else(|| a.get(key))
            .cloned()
            .unwrap_or_default();
        let id = match key {
            NodeKey::Id(id) => Some(*id),
            NodeKey::Name(_) => None,
        };
        out.extend(Change::opt(
            Subject::Position { node: name, id },
            a.get(key).map(|(_, p)| text(*p)),
            b.get(key).map(|(_, p)| text(*p)),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    const BASE: &str = r#"room "R" format=1 {
  network "/" {
    node "a" type="t.x@1" id="01920000-0000-7000-8000-000000000001" {
      input "p" 1.0
    }
    node "b" type="t.x@1" id="01920000-0000-7000-8000-000000000002"
    link from="a.out" to="b.p"
  }
  meta {
    node "a" x=0.0 y=0.0
  }
}
"#;

    fn lines(old: &str, new: &str) -> Vec<String> {
        let d = diff(&parse(old).unwrap(), &parse(new).unwrap());
        d.changes.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn identical_documents_have_no_changes() {
        let d = diff(&parse(BASE).unwrap(), &parse(BASE).unwrap());
        assert!(d.is_empty());
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn rename_keeps_links_and_meta_matched() {
        let new = BASE.replace("\"a\"", "\"z\"").replace("a.out", "z.out");
        assert_eq!(lines(BASE, &new), [r#"~ node "z" renamed: "a" -> "z""#]);
    }

    #[test]
    fn subject_accessors() {
        let new = BASE.replace("input \"p\" 1.0", "input \"p\" 2.0");
        let d = diff(&parse(BASE).unwrap(), &parse(&new).unwrap());
        let c = &d.changes[0];
        assert_eq!(c.subject.kind(), "input");
        assert_eq!(c.subject.port(), Some("p"));
        assert_eq!(c.subject.node().map(|n| n.name.as_str()), Some("a"));
        assert!(!c.subject.is_meta());
    }
}
