// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The in-memory document model.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use livspeil_graph::{CharmMeta, NodeId, SUBNET_TYPE};
use livspeil_node_api::{PortType, Value};
use livspeil_params::AnimChannel;

use crate::error::Location;

/// The newest document format this build reads and the one it writes.
pub const CURRENT_FORMAT: u32 = 1;

/// Path of the root network. Subnets hold nested, unnamed `network`
/// blocks inside their `node`.
pub const ROOT_NETWORK: &str = "/";

/// Minimum hardware tier a room needs, written `T0`, `T1`, ...
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Tier(pub u8);

impl fmt::Display for Tier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "T{}", self.0)
    }
}

/// The text is not of the form `T<number>` (number 0 to 255).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTierError(pub String);

impl fmt::Display for ParseTierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' is not a tier (expected T0, T1, ...)", self.0)
    }
}

impl std::error::Error for ParseTierError {}

impl FromStr for Tier {
    type Err = ParseTierError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.strip_prefix('T')
            .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|digits| digits.parse().ok())
            .map(Tier)
            .ok_or_else(|| ParseTierError(s.to_owned()))
    }
}

/// Where an item was parsed from. Purely diagnostic: two `SourceLoc`s are
/// always equal, so documents compare by content only.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceLoc(pub Option<Location>);

impl PartialEq for SourceLoc {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A Livspeil document (the contents of `Name.room/room.kdl`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Document {
    /// Display name of the room.
    pub name: String,
    /// Format version the document is in. Parsing migrates older formats,
    /// so a parsed document always has [`CURRENT_FORMAT`].
    pub format: u32,
    /// Minimum hardware tier.
    pub min_tier: Tier,
    /// The root network (`network "/"`).
    pub network: Network,
    /// Editor-only data; not needed to cook.
    pub meta: Meta,
}

impl Document {
    /// An empty document in the current format.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Document {
            name: name.into(),
            format: CURRENT_FORMAT,
            min_tier: Tier::default(),
            network: Network::default(),
            meta: Meta::default(),
        }
    }
}

/// One network: nodes and the links between them.
///
/// Canonical order: nodes by id (ids are UUIDv7, so that is creation order
/// and a rename never moves a node), links by `(from, to)` endpoint text.
/// [`crate::format`] sorts; this struct does not enforce it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Network {
    pub nodes: Vec<DocNode>,
    pub links: Vec<DocLink>,
}

impl Network {
    /// Sort nodes and links into canonical order, in nested subnet
    /// networks too.
    pub fn sort(&mut self) {
        self.nodes.sort_by_key(|n| n.id);
        self.links.sort();
        for node in &mut self.nodes {
            if let Some(network) = &mut node.network {
                network.sort();
            }
        }
    }

    /// Every node of this network and of the nested ones, with its path
    /// relative to this network (`name`, `sub/name`, ...), parents first.
    #[must_use]
    pub fn walk(&self) -> Vec<(String, &DocNode)> {
        let mut out = Vec::new();
        let mut stack: Vec<(String, &Network)> = vec![(String::new(), self)];
        while let Some((prefix, network)) = stack.pop() {
            for node in &network.nodes {
                let path = format!("{prefix}{}", node.name);
                if let Some(inner) = &node.network {
                    stack.push((format!("{path}/"), inner));
                }
                out.push((path, node));
            }
        }
        out
    }
}

/// A node of a network.
#[derive(Debug, Clone, PartialEq)]
pub struct DocNode {
    pub id: NodeId,
    /// Unique within the network; links and meta refer to nodes by name.
    pub name: String,
    /// Qualified type id, for example `core.math.add@1`.
    pub type_id: String,
    /// Explicitly set (static) input values by port name. Inputs not
    /// listed use the node type's default.
    pub inputs: BTreeMap<String, Value>,
    /// Expressions driving inputs, by port name.
    pub expressions: BTreeMap<String, DocExpr>,
    /// Animation channels driving inputs, by port name.
    pub animations: BTreeMap<String, DocAnim>,
    /// Declared port type of a boundary node (`core.subnet.input@1`,
    /// `core.output@1`), written `port-type="Color"`.
    pub port_type: Option<PortType>,
    /// Charm metadata of a subnet (`charm` block).
    pub charm: Option<CharmMeta>,
    /// The network inside a subnet (nested `network` block). Always `Some`
    /// for subnets built by [`Document::from_graph`].
    pub network: Option<Network>,
    pub at: SourceLoc,
}

impl DocNode {
    /// A node with no explicitly set inputs, expressions or animations.
    #[must_use]
    pub fn new(id: NodeId, name: impl Into<String>, type_id: impl Into<String>) -> Self {
        DocNode {
            id,
            name: name.into(),
            type_id: type_id.into(),
            inputs: BTreeMap::new(),
            expressions: BTreeMap::new(),
            animations: BTreeMap::new(),
            port_type: None,
            charm: None,
            network: None,
            at: SourceLoc::default(),
        }
    }

    /// Whether the node is a subnet (`core.subnet@1`).
    #[must_use]
    pub fn is_subnet(&self) -> bool {
        self.type_id == SUBNET_TYPE
    }

    /// Every port that has at least one source (static value, expression
    /// or animation), sorted and without duplicates.
    pub fn ports(&self) -> impl Iterator<Item = &str> {
        let mut ports: Vec<&str> = self
            .inputs
            .keys()
            .chain(self.expressions.keys())
            .chain(self.animations.keys())
            .map(String::as_str)
            .collect();
        ports.sort_unstable();
        ports.dedup();
        ports.into_iter()
    }
}

/// An expression on an input, written `input "port" expr="..."`. The
/// source is stored verbatim; it is compiled by [`Document::to_graph`],
/// once every node exists, because `ch()` paths name other nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct DocExpr {
    pub source: String,
    /// Location of the `expr=` property.
    pub at: SourceLoc,
}

impl DocExpr {
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        DocExpr {
            source: source.into(),
            at: SourceLoc::default(),
        }
    }
}

/// An animation channel on an input, written as an `anim { key ... }`
/// child of the `input` node.
#[derive(Debug, Clone, PartialEq)]
pub struct DocAnim {
    pub channel: AnimChannel,
    /// Location of the `anim` node.
    pub at: SourceLoc,
}

impl DocAnim {
    #[must_use]
    pub fn new(channel: AnimChannel) -> Self {
        DocAnim {
            channel,
            at: SourceLoc::default(),
        }
    }
}

/// A link from `from_node.from_port` (an output) to `to_node.to_port` (an
/// input), by name. Written `link from="sun.phase" to="lamp.t"`; node names
/// may contain dots because the port is everything after the last dot.
///
/// Both nodes are in the network holding the link. When a node is a
/// subnet, the port names one of its interface ports (`lamp.hour` is the
/// subnet's input `hour`, that is the `value` input of its
/// `core.subnet.input@1` node `hour`).
#[derive(Debug, Clone, PartialEq)]
pub struct DocLink {
    pub from_node: String,
    pub from_port: String,
    pub to_node: String,
    pub to_port: String,
    pub at: SourceLoc,
}

impl DocLink {
    #[must_use]
    pub fn new(
        from_node: impl Into<String>,
        from_port: impl Into<String>,
        to_node: impl Into<String>,
        to_port: impl Into<String>,
    ) -> Self {
        DocLink {
            from_node: from_node.into(),
            from_port: from_port.into(),
            to_node: to_node.into(),
            to_port: to_port.into(),
            at: SourceLoc::default(),
        }
    }

    fn key(&self) -> (&str, &str, &str, &str) {
        (
            &self.from_node,
            &self.from_port,
            &self.to_node,
            &self.to_port,
        )
    }
}

impl Eq for DocLink {}

impl PartialOrd for DocLink {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DocLink {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key().cmp(&other.key())
    }
}

/// Editor-only data, written last so logic diffs stay clean.
///
/// Positions are keyed by node *name*, like links and `ch()` paths; nodes
/// inside subnets by their path from the root without the leading `/`
/// (`lamp/hour`). A tool
/// that renames a node must rename its meta entry too (the graph rewrites
/// its own `ch()` paths on rename, but knows nothing about meta). Keying by
/// id would survive renames but make hand-edited files much harder to
/// read; this may change in a later format.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Meta {
    /// Node editor positions `[x, y]` by node name.
    pub positions: BTreeMap<String, [f64; 2]>,
}

impl Meta {
    /// Whether there is nothing to write.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}
