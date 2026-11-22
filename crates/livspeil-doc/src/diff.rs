// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Semantic diff of two documents.
//!
//! [`diff`] matches nodes by id, so a rename is one change instead of a
//! removal and an addition, and links and meta positions are matched
//! through the ids of the nodes they name (renaming a node does not show
//! its links as changed). Every value is compared and shown by its
//! canonical KDL text (the text [`crate::format()`] writes), so the diff
//! agrees with what is on disk and new value types need no diff code.
//!
//! # Order
//!
//! The result is deterministic: room header (name, format, min-tier),
//! then nodes by id (for each: added/removed, name, type, then its ports by
//! name, each port as static value, expression, animation), then links by
//! `(from, to)` text, then meta positions by node id. An added node lists
//! its input sources as added too; a removed node is one change.
//!
//! # Animation
//!
//! A channel that appears or disappears is one [`Subject::Anim`] change
//! with a summary (`3 keys 0.0..2.0 pre="loop"`). A channel that changes
//! is reported as setting changes ([`Subject::AnimSetting`]: `pre`, `post`,
//! `offset`) and key changes ([`Subject::AnimKey`]), keys matched by time.
//! A key's text is its value followed by its non-default properties, as in
//! `room.kdl` (`0.8 interp="step"`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use livspeil_graph::NodeId;
use livspeil_node_api::Value;
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Keyframe, Tangent};

use crate::anim::{extrapolation_name, interpolation_name};
use crate::model::{DocLink, DocNode, Document};
use crate::value::{format_float, format_value, quote};

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

/// A node as the diff names it: its id and its name in the newer document
/// (the older one if it was removed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRef {
    pub id: NodeId,
    pub name: String,
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
    /// A link, as `node.port` texts with current names.
    Link { from: String, to: String },
    /// A node's editor position (meta); `old`/`new` are `x y`.
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
            Subject::Input { .. } => "input",
            Subject::Expr { .. } => "expr",
            Subject::Anim { .. } => "anim",
            Subject::AnimSetting { .. } => "anim-setting",
            Subject::AnimKey { .. } => "anim-key",
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
            | Subject::Input { node, .. }
            | Subject::Expr { node, .. }
            | Subject::Anim { node, .. }
            | Subject::AnimSetting { node, .. }
            | Subject::AnimKey { node, .. } => Some(node),
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
            | Subject::AnimKey { port, .. } => Some(port),
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
/// except for the summaries of [`Subject::Anim`] and [`Subject::Position`].
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
/// - link "sun.phase" -> "lamp.t"
/// ~ meta node "lamp" position: 0.0 0.0 -> 10.0 20.0
/// ```
impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ", self.op.sign())?;
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
                    quote(&node.name),
                    quote(&node.id.to_string())
                );
            }
            Subject::NodeName(node) => write!(f, "node {} renamed", quote(&node.name))?,
            Subject::NodeType(node) => write!(f, "node {} type", quote(&node.name))?,
            Subject::Input { node, port } => {
                write!(f, "node {} input {}", quote(&node.name), quote(port))?;
            }
            Subject::Expr { node, port } => {
                write!(f, "node {} input {} expr", quote(&node.name), quote(port))?;
            }
            Subject::Anim { node, port } => {
                write!(f, "node {} input {} anim", quote(&node.name), quote(port))?;
            }
            Subject::AnimSetting {
                node,
                port,
                setting,
            } => write!(
                f,
                "node {} input {} anim {setting}",
                quote(&node.name),
                quote(port)
            )?,
            Subject::AnimKey { node, port, time } => write!(
                f,
                "node {} input {} anim key {time}",
                quote(&node.name),
                quote(port)
            )?,
            Subject::Link { from, to } => {
                return write!(f, "link {} -> {}", quote(from), quote(to));
            }
            Subject::Position { node, .. } => write!(f, "meta node {} position", quote(node))?,
        }
        match (&self.old, &self.new) {
            (Some(old), Some(new)) => write!(f, ": {old} -> {new}"),
            (Some(v), None) | (None, Some(v)) => write!(f, ": {v}"),
            (None, None) => Ok(()),
        }
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

    let old_nodes = by_id(&old.network.nodes);
    let new_nodes = by_id(&new.network.nodes);
    let ids: BTreeSet<NodeId> = old_nodes.keys().chain(new_nodes.keys()).copied().collect();
    for id in ids {
        match (old_nodes.get(&id), new_nodes.get(&id)) {
            (Some(a), None) => out.push(Change::new(
                Op::Removed,
                Subject::Node(node_ref(a)),
                Some(quote(&a.type_id)),
                None,
            )),
            (None, Some(b)) => {
                out.push(Change::new(
                    Op::Added,
                    Subject::Node(node_ref(b)),
                    None,
                    Some(quote(&b.type_id)),
                ));
                let empty = DocNode::new(b.id, b.name.clone(), b.type_id.clone());
                node_sources(&mut out, &empty, b);
            }
            (Some(a), Some(b)) => {
                out.extend(Change::pair(
                    Subject::NodeName(node_ref(b)),
                    quote(&a.name),
                    quote(&b.name),
                ));
                out.extend(Change::pair(
                    Subject::NodeType(node_ref(b)),
                    quote(&a.type_id),
                    quote(&b.type_id),
                ));
                node_sources(&mut out, a, b);
            }
            (None, None) => {}
        }
    }

    links(&mut out, old, new);
    positions(&mut out, old, new);
    DocDiff { changes: out }
}

fn by_id(nodes: &[DocNode]) -> BTreeMap<NodeId, &DocNode> {
    nodes.iter().map(|n| (n.id, n)).collect()
}

fn node_ref(node: &DocNode) -> NodeRef {
    NodeRef {
        id: node.id,
        name: node.name.clone(),
    }
}

/// Canonical text of a value; values the format cannot store yet (which a
/// parsed document never holds) fall back to their debug form.
fn value_text(value: &Value) -> String {
    format_value(value).unwrap_or_else(|| format!("{value:?}"))
}

/// Inputs, expressions and animations of a node present in both (or added).
fn node_sources(out: &mut Vec<Change>, a: &DocNode, b: &DocNode) {
    let ports: BTreeSet<&str> = a.ports().chain(b.ports()).collect();
    for port in ports {
        let at = |port: &str| (node_ref(b), port.to_owned());
        let (node, port_name) = at(port);
        out.extend(Change::opt(
            Subject::Input {
                node,
                port: port_name,
            },
            a.inputs.get(port).map(value_text),
            b.inputs.get(port).map(value_text),
        ));
        let (node, port_name) = at(port);
        out.extend(Change::opt(
            Subject::Expr {
                node,
                port: port_name,
            },
            a.expressions.get(port).map(|e| quote(&e.source)),
            b.expressions.get(port).map(|e| quote(&e.source)),
        ));
        let old = a.animations.get(port).map(|x| &x.channel);
        let new = b.animations.get(port).map(|x| &x.channel);
        match (old, new) {
            (Some(x), Some(y)) => anim_changes(out, &node_ref(b), port, x, y),
            (x, y) => {
                let (node, port_name) = at(port);
                out.extend(Change::opt(
                    Subject::Anim {
                        node,
                        port: port_name,
                    },
                    x.map(anim_summary),
                    y.map(anim_summary),
                ));
            }
        }
    }
}

fn extrapolation_text(e: Extrapolation) -> String {
    quote(extrapolation_name(e))
}

/// `3 keys 0.0..2.0` plus non-default settings, for a whole channel.
fn anim_summary(channel: &AnimChannel) -> String {
    let keys = channel.keys();
    let mut s = match (keys.first(), keys.last()) {
        (Some(first), Some(last)) => format!(
            "{} key{} {}..{}",
            keys.len(),
            if keys.len() == 1 { "" } else { "s" },
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

fn anim_changes(
    out: &mut Vec<Change>,
    node: &NodeRef,
    port: &str,
    a: &AnimChannel,
    b: &AnimChannel,
) {
    let setting = |setting: &'static str| Subject::AnimSetting {
        node: node.clone(),
        port: port.to_owned(),
        setting,
    };
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
        out.extend(Change::opt(
            Subject::AnimKey {
                node: node.clone(),
                port: port.to_owned(),
                time: format_float(t),
            },
            find(a, t),
            find(b, t),
        ));
    }
}

/// A node named by a link or meta entry: its id if the name exists, else
/// the bare name (only in documents that would not build a graph).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NodeKey {
    Id(NodeId),
    Name(String),
}

fn node_key(doc: &Document, name: &str) -> NodeKey {
    doc.network
        .nodes
        .iter()
        .find(|n| n.name == name)
        .map_or_else(|| NodeKey::Name(name.to_owned()), |n| NodeKey::Id(n.id))
}

type LinkKey = (NodeKey, String, NodeKey, String);

fn link_key(doc: &Document, link: &DocLink) -> LinkKey {
    (
        node_key(doc, &link.from_node),
        link.from_port.clone(),
        node_key(doc, &link.to_node),
        link.to_port.clone(),
    )
}

fn links(out: &mut Vec<Change>, old: &Document, new: &Document) {
    let old_links: BTreeMap<LinkKey, &DocLink> = old
        .network
        .links
        .iter()
        .map(|l| (link_key(old, l), l))
        .collect();
    let new_links: BTreeMap<LinkKey, &DocLink> = new
        .network
        .links
        .iter()
        .map(|l| (link_key(new, l), l))
        .collect();
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
    let mut changes: Vec<(String, String, Op)> = changes
        .into_iter()
        .map(|(op, l)| {
            (
                format!("{}.{}", l.from_node, l.from_port),
                format!("{}.{}", l.to_node, l.to_port),
                op,
            )
        })
        .collect();
    // Removals before additions of the same text (a node was replaced).
    changes.sort_by(|x, y| (&x.0, &x.1, x.2 == Op::Added).cmp(&(&y.0, &y.1, y.2 == Op::Added)));
    for (from, to, op) in changes {
        out.push(Change::new(op, Subject::Link { from, to }, None, None));
    }
}

fn positions(out: &mut Vec<Change>, old: &Document, new: &Document) {
    let text = |[x, y]: [f64; 2]| format!("{} {}", format_float(x), format_float(y));
    let collect = |doc: &Document| -> BTreeMap<NodeKey, (String, [f64; 2])> {
        doc.meta
            .positions
            .iter()
            .map(|(name, pos)| (node_key(doc, name), (name.clone(), *pos)))
            .collect()
    };
    let a = collect(old);
    let b = collect(new);
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
