// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editable graph: nodes, links and the rules they must obey.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use livspeil_node_api::{NodeImpl, NodeRegistry, NodeSpec, PortType, TypeClass, Value};
use livspeil_params::AnimChannel;
use livspeil_params::expr::{ExprError, RefKind};
use uuid::Uuid;

use crate::charm_defs::{self, CharmDef};
use crate::network::{self, CharmMeta, Subnet};
use crate::snapshot::{GraphSnapshot, NamedLink, NodeSnapshot};
use crate::sources::{Expression, InputRef};
use crate::types::{Poly, SourceKind};

/// Stable identifier of a node (UUIDv7, so ids sort by creation time).
///
/// With the `serde` feature it serializes as its UUID string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(transparent)
)]
pub struct NodeId(Uuid);

impl NodeId {
    /// The smallest id (the nil UUID); handy for range queries.
    pub(crate) const MIN: NodeId = NodeId(Uuid::nil());

    /// A fresh, time-ordered id.
    #[must_use]
    pub fn new() -> Self {
        NodeId(Uuid::now_v7())
    }

    /// Wrap an existing UUID (used when loading documents and in tests).
    #[must_use]
    pub const fn from_uuid(uuid: Uuid) -> Self {
        NodeId(uuid)
    }

    /// The underlying UUID.
    #[must_use]
    pub const fn uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A port on a specific node, by index into the node spec's port list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Endpoint {
    pub node: NodeId,
    pub port: usize,
}

/// A connection from an output port to an input port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Link {
    pub from: Endpoint,
    pub to: Endpoint,
}

/// Which side of a node a port is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(rename_all = "snake_case")
)]
pub enum PortDirection {
    Input,
    Output,
}

impl fmt::Display for PortDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PortDirection::Input => "input",
            PortDirection::Output => "output",
        })
    }
}

/// Why an edit to the graph was rejected. The graph is unchanged on error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// No node type with this qualified id is registered.
    UnknownNodeType(String),
    /// The name is empty.
    EmptyName,
    /// Another node already has this name.
    DuplicateName(String),
    /// Another node already has this id.
    DuplicateId(NodeId),
    /// No node with this id.
    UnknownNode(NodeId),
    /// The node has no port with this name.
    UnknownPort {
        node: NodeId,
        port: String,
        direction: PortDirection,
    },
    /// The value or link has the wrong type for the input port.
    TypeMismatch { expected: PortType, found: PortType },
    /// The input port already has a link; unlink it first.
    InputOccupied { node: NodeId, port: String },
    /// The link or `ch()` reference would make a node depend on itself.
    WouldCycle,
    /// An expression failed to parse, resolve its `ch()` paths or type
    /// check.
    Expression(Box<ExprError>),
    /// The node cannot be removed: input `port` of node `by` reads it
    /// through `ch()` or `out()`.
    Referenced {
        node: NodeId,
        by: NodeId,
        port: String,
    },
    /// The node is not a subnet, so it holds no network (and no
    /// interface or Charm metadata).
    NotASubnet(NodeId),
    /// Interface inputs and promotion only exist inside a subnet; the node
    /// (or the node to be created) is in the root network.
    NotInSubnet(NodeId),
    /// The name cannot be used where paths must address the node: names of
    /// subnets and of nodes inside subnets must not contain `/` and must
    /// not be `.` or `..`; names of interface ports (boundary nodes inside
    /// a subnet) must not contain `.` either.
    InvalidName(String),
    /// The link would connect two networks other than through a subnet's
    /// interface (see [`Graph::link`]).
    CrossNetwork { from: NodeId, to: NodeId },
    /// A port type was given for a node type whose ports are not
    /// polymorphic (only `core.subnet.input@1` and `core.output@1` are).
    NotPolymorphic(String),
    /// The subnet cannot be moved into itself or one of its descendants.
    WouldNest(NodeId),
    // Charm definitions (M0.16, see `charm_defs.rs`).
    /// The node is a Charm instance, or inside one: its contents and
    /// metadata come from the definition. Only the values, expressions and
    /// animation channels of the instance's interface inputs (and links
    /// into them from outside) are per-instance; edit the definition or
    /// detach the instance.
    InInstance(NodeId),
    /// No Charm definition with this id.
    UnknownDefinition(String),
    /// A Charm definition with this id already exists.
    DuplicateDefinition(String),
    /// The definition cannot be removed while `instance` uses it.
    DefinitionInUse { def: String, instance: NodeId },
    /// The definition (or the subnet it is made from) is not valid.
    InvalidDefinition { def: String, reason: String },
    /// The node is not a Charm instance.
    NotAnInstance(NodeId),
    /// The edit would leave type variable `var` of `node` (and the
    /// variables linked to it) without a type: it must be `expected` but
    /// would also need to be `found` (M0.17; see the `types` module).
    TypeConflict {
        node: NodeId,
        var: String,
        expected: PortType,
        found: PortType,
    },
    /// Type variable `var` of `node` would need type `found`, which is not
    /// in its constraint class `class` (or that of a variable linked to
    /// it).
    NotInClass {
        node: NodeId,
        var: String,
        class: TypeClass,
        found: PortType,
    },
    /// The node type declares type variables that do not fit its spec.
    InvalidTypeVars { node_type: String, reason: String },
    /// An expression reads port `port` of `node` through `ch()` or
    /// `out()`, but the port's type variable is not pinned, so its type
    /// could change under the expression (see the `types` module).
    UnpinnedRead { node: NodeId, port: String },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::UnknownNodeType(id) => write!(f, "unknown node type '{id}'"),
            GraphError::EmptyName => write!(f, "node names must not be empty"),
            GraphError::DuplicateName(name) => write!(f, "a node named '{name}' already exists"),
            GraphError::DuplicateId(id) => write!(f, "a node with id {id} already exists"),
            GraphError::UnknownNode(id) => write!(f, "no node with id {id}"),
            GraphError::UnknownPort {
                node,
                port,
                direction,
            } => write!(f, "node {node} has no {direction} port '{port}'"),
            GraphError::TypeMismatch { expected, found } => {
                write!(f, "type mismatch: expected {expected}, found {found}")
            }
            GraphError::InputOccupied { node, port } => {
                write!(f, "input '{port}' of node {node} is already linked")
            }
            GraphError::WouldCycle => write!(f, "the link would create a cycle"),
            GraphError::Expression(err) => write!(f, "expression error: {err}"),
            GraphError::Referenced { node, by, port } => write!(
                f,
                "node {node} is read by ch() or out() in input '{port}' of node {by}"
            ),
            GraphError::NotASubnet(id) => write!(f, "node {id} is not a subnet"),
            GraphError::NotInSubnet(id) => {
                write!(f, "node {id} is in the root network, not inside a subnet")
            }
            GraphError::InvalidName(name) => write!(
                f,
                "'{name}' cannot name a subnet or a node inside one (no '/', not '.' or '..'; no '.' in interface ports)"
            ),
            GraphError::CrossNetwork { from, to } => write!(
                f,
                "nodes {from} and {to} are in different networks; link through the subnet's interface"
            ),
            GraphError::NotPolymorphic(id) => {
                write!(f, "node type '{id}' has no polymorphic ports")
            }
            GraphError::WouldNest(id) => {
                write!(f, "subnet {id} cannot be moved into itself")
            }
            GraphError::InInstance(id) => write!(
                f,
                "node {id} is (inside) a Charm instance; edit the definition or detach the instance"
            ),
            GraphError::UnknownDefinition(def) => write!(f, "no Charm definition '{def}'"),
            GraphError::DuplicateDefinition(def) => {
                write!(f, "a Charm definition '{def}' already exists")
            }
            GraphError::DefinitionInUse { def, instance } => {
                write!(f, "Charm definition '{def}' is used by instance {instance}")
            }
            GraphError::InvalidDefinition { def, reason } => {
                write!(f, "invalid Charm definition '{def}': {reason}")
            }
            GraphError::NotAnInstance(id) => write!(f, "node {id} is not a Charm instance"),
            GraphError::TypeConflict {
                node,
                var,
                expected,
                found,
            } => write!(
                f,
                "type conflict on {var} of node {node}: it is {expected} here, but {found} is needed"
            ),
            GraphError::NotInClass {
                node,
                var,
                class,
                found,
            } => write!(
                f,
                "{var} of node {node} cannot be {found}: it must be {class}"
            ),
            GraphError::InvalidTypeVars { node_type, reason } => {
                write!(
                    f,
                    "node type '{node_type}' has invalid type variables: {reason}"
                )
            }
            GraphError::UnpinnedRead { node, port } => write!(
                f,
                "port '{port}' of node {node} is polymorphic and not pinned; \
                 add the node with a port type to read it from an expression"
            ),
        }
    }
}

impl std::error::Error for GraphError {}

#[derive(Clone)]
pub(crate) struct Node {
    pub(crate) id: NodeId,
    pub(crate) name: String,
    pub(crate) type_id: String,
    pub(crate) imp: Arc<dyn NodeImpl>,
    /// The subnet whose network holds the node; `None` for the root
    /// network.
    pub(crate) parent: Option<NodeId>,
    /// The declared port type of a boundary node (`core.subnet.input@1`,
    /// `core.output@1`), or the pinned type of a polymorphic node's type
    /// variable; `None` for other nodes.
    pub(crate) port_type: Option<PortType>,
    /// Type variables of a polymorphic node (`imp` is then the resolved
    /// instance); see the `types` module.
    pub(crate) poly: Option<Box<Poly>>,
    /// Explicitly set input values, by input port index. `None` means "use
    /// the spec default".
    pub(crate) values: Vec<Option<Value>>,
    /// Expressions, by input port index.
    pub(crate) exprs: Vec<Option<Expression>>,
    /// Animation channels, by input port index.
    pub(crate) anims: Vec<Option<AnimChannel>>,
    /// The link feeding each input port as `(source slot, output port)`: a
    /// dense mirror of [`Graph::links`] for the cooker's hot path.
    pub(crate) link_in: Vec<Option<(usize, usize)>>,
    /// Bumped whenever this node's inputs or links change.
    pub(crate) rev: u64,
}

impl Node {
    /// Whether any input source (expression or animation channel) depends
    /// on the frame clock.
    pub(crate) fn sources_time_dependent(&self) -> bool {
        self.anims.iter().any(Option::is_some)
            || self
                .exprs
                .iter()
                .flatten()
                .any(|e| e.program.is_time_dependent())
    }

    /// Whether the node must re-cook when the clock changes: its spec says
    /// so, or one of its sources reads the clock.
    pub(crate) fn time_dependent(&self) -> bool {
        self.imp.spec().time_dependent || self.sources_time_dependent()
    }

    /// Slots of every node this one directly depends on (link sources and
    /// nodes read by `ch()`), possibly with repeats.
    pub(crate) fn dependencies(&self) -> impl Iterator<Item = usize> + '_ {
        let reads = self
            .exprs
            .iter()
            .flatten()
            .flat_map(|e| e.targets.iter().map(|t: &InputRef| t.slot));
        self.link_in
            .iter()
            .flatten()
            .map(|&(slot, _)| slot)
            .chain(reads)
    }
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("type_id", &self.type_id)
            .field("rev", &self.rev)
            .finish_non_exhaustive()
    }
}

/// `dependents` in compressed sparse row form: the dependents of slot `s`
/// are `targets[offsets[s]..offsets[s + 1]]`. Contiguous, so the downstream
/// walk does not chase one heap allocation per node.
#[derive(Debug, Default, Clone)]
struct DependentsIndex {
    /// The `deps_rev` this copy was built for.
    built_for: Option<u64>,
    offsets: Vec<usize>,
    targets: Vec<usize>,
}

impl DependentsIndex {
    fn rebuild(&mut self, dependents: &[Vec<usize>]) {
        self.offsets.clear();
        self.targets.clear();
        self.offsets.push(0);
        for list in dependents {
            self.targets.extend_from_slice(list);
            self.offsets.push(self.targets.len());
        }
    }

    fn dependents(&self, slot: usize) -> &[usize] {
        match (self.offsets.get(slot), self.offsets.get(slot + 1)) {
            (Some(&start), Some(&end)) => self.targets.get(start..end).unwrap_or(&[]),
            _ => &[],
        }
    }
}

/// A network of typed nodes, with nested subnets (kernel spec, section 3;
/// see the crate docs for the network model).
///
/// # Storage and invalidation
///
/// Nodes live in a slab of dense *slots* (`usize` indices, reused after
/// removal); [`NodeId`] stays the public identity and `index` maps ids to
/// slots. The cooker's hot paths work on slots only.
///
/// Every edit gets a new *epoch* (`rev_counter`). An edit that can change
/// what a node computes marks the node and everything downstream of it
/// (links and `ch()` readers, `dependents`) with that epoch in `dirty`.
/// A cooker remembers, per node, the epoch at which it last validated the
/// node; the node is stale for that cooker when `dirty > validated`. See
/// the `cook` module for the cooker side.
///
/// The walk uses a compact copy of the dependents lists that is rebuilt
/// (in `O(nodes + edges)`) by the first edit after the lists changed.
///
/// The downstream walk stops at nodes that are already stale for *every*
/// cooker: `observed` is the largest epoch any cooker has started a cook
/// at, so a node with `dirty > observed` (and, by induction, everything
/// downstream of it) is already newer than any cooker's validation. Right
/// after a cook nothing is above `observed`, so the first edit walks the
/// whole affected subgraph once; further edits before the next cook (a
/// slider drag between frames) stop almost immediately.
///
/// `topo_rev` changes whenever the dependency structure (node set, links,
/// `ch()` edges) or the set of time-dependent nodes changes; cookers cache
/// evaluation orders and the time-dependent closure until it changes.
#[derive(Debug)]
pub struct Graph {
    id: Uuid,
    registry: Arc<NodeRegistry>,
    /// Node storage by slot; `None` marks a free slot.
    slots: Vec<Option<Node>>,
    /// Free slots, reused by the next added node.
    free: Vec<usize>,
    /// Slot of every node, in id order.
    index: BTreeMap<NodeId, usize>,
    /// Links keyed by their target input, which enforces one link per input.
    links: BTreeMap<(NodeId, usize), Endpoint>,
    /// The same links as `(source node, target node, target port)`, for fast
    /// downstream traversal.
    outgoing: BTreeSet<(NodeId, NodeId, usize)>,
    /// Names of the root network's nodes.
    names: HashMap<String, NodeId>,
    /// Every subnet: its child names and Charm metadata.
    pub(crate) subnets: BTreeMap<NodeId, Subnet>,
    /// `ch()` references as `(read node, reading node, reading input)`.
    pub(crate) ch_edges: BTreeSet<(NodeId, NodeId, usize)>,
    rev_counter: u64,
    /// Per slot: the epoch of the last edit that affected the node (its own
    /// or one upstream).
    dirty: Vec<u64>,
    /// Per slot: direct dependents (link targets and `ch()` readers), with
    /// one entry per link or `ch()` edge.
    dependents: Vec<Vec<usize>>,
    /// Changes whenever `dependents` changes.
    deps_rev: u64,
    /// A compact copy of `dependents` for the downstream walk.
    walk: DependentsIndex,
    /// Largest epoch any cooker has started a cook at.
    observed: AtomicU64,
    /// Revision of the dependency structure and time dependence.
    topo_rev: u64,
    /// Scratch stack for the downstream walk.
    mark_stack: Vec<usize>,
    /// Charm definitions by id (M0.16, see `charm_defs.rs`).
    pub(crate) defs: BTreeMap<String, CharmDef>,
    /// Charm instances and the definition each one uses.
    pub(crate) instances: BTreeMap<NodeId, String>,
}

impl Graph {
    /// An empty graph that instantiates nodes from `registry`.
    #[must_use]
    pub fn new(registry: Arc<NodeRegistry>) -> Self {
        Graph {
            id: Uuid::now_v7(),
            registry,
            slots: Vec::new(),
            free: Vec::new(),
            index: BTreeMap::new(),
            links: BTreeMap::new(),
            outgoing: BTreeSet::new(),
            names: HashMap::new(),
            subnets: BTreeMap::new(),
            ch_edges: BTreeSet::new(),
            rev_counter: 0,
            dirty: Vec::new(),
            dependents: Vec::new(),
            deps_rev: 0,
            walk: DependentsIndex::default(),
            observed: AtomicU64::new(0),
            topo_rev: 0,
            mark_stack: Vec::new(),
            defs: BTreeMap::new(),
            instances: BTreeMap::new(),
        }
    }

    /// An exact copy (same instance id, epochs and slots), used to roll
    /// back composite edits (Charm definition updates, rewired moves).
    pub(crate) fn duplicate(&self) -> Graph {
        Graph {
            id: self.id,
            registry: Arc::clone(&self.registry),
            slots: self.slots.clone(),
            free: self.free.clone(),
            index: self.index.clone(),
            links: self.links.clone(),
            outgoing: self.outgoing.clone(),
            names: self.names.clone(),
            subnets: self.subnets.clone(),
            ch_edges: self.ch_edges.clone(),
            rev_counter: self.rev_counter,
            dirty: self.dirty.clone(),
            dependents: self.dependents.clone(),
            deps_rev: self.deps_rev,
            walk: self.walk.clone(),
            observed: AtomicU64::new(self.observed.load(Ordering::Relaxed)),
            topo_rev: self.topo_rev,
            mark_stack: Vec::new(),
            defs: self.defs.clone(),
            instances: self.instances.clone(),
        }
    }

    /// The registry nodes are instantiated from.
    pub(crate) fn registry(&self) -> &Arc<NodeRegistry> {
        &self.registry
    }

    /// Identity of this graph instance (lets a cooker detect a different
    /// graph and drop stale caches).
    pub(crate) fn uid(&self) -> Uuid {
        self.id
    }

    /// Add a node of type `qualified_id` (for example `core.math.add@1`).
    ///
    /// # Errors
    /// Fails for an unknown node type, an empty name or a duplicate name.
    pub fn add_node(&mut self, qualified_id: &str, name: &str) -> Result<NodeId, GraphError> {
        self.add_node_with_id(NodeId::new(), qualified_id, name)
    }

    /// Like [`Graph::add_node`] with a caller-chosen id (document loading,
    /// deterministic tests).
    ///
    /// # Errors
    /// Same as [`Graph::add_node`], plus [`GraphError::DuplicateId`].
    pub fn add_node_with_id(
        &mut self,
        id: NodeId,
        qualified_id: &str,
        name: &str,
    ) -> Result<NodeId, GraphError> {
        self.add_node_in(None, id, qualified_id, name, None)
    }

    /// Add a node to the network of subnet `parent` (`None`: the root
    /// network). `port_type` sets the type of a boundary node's ports
    /// (`core.subnet.input@1`, `core.output@1`); `None` keeps the
    /// registered type (`Float`). For a polymorphic node type with one
    /// type variable, `port_type` *pins* the variable to that type (it
    /// must be in the variable's class); unpinned variables resolve from
    /// links and sources (see the `types` module). [`Graph::port_type`]
    /// reports the pin.
    ///
    /// Adding a `core.subnet@1` node creates an empty network inside it.
    ///
    /// # Errors
    /// Same as [`Graph::add_node_with_id`] (names are unique per network),
    /// plus [`GraphError::UnknownNode`] / [`GraphError::NotASubnet`] for a
    /// bad parent, [`GraphError::InvalidName`] (see the error),
    /// [`GraphError::NotInSubnet`] for a `core.subnet.input@1` in the root
    /// network, [`GraphError::NotPolymorphic`] for a port type on any
    /// other node type (except polymorphic ones with one type variable),
    /// [`GraphError::NotInClass`] for a pin outside the variable's class
    /// and [`GraphError::InvalidTypeVars`] for a node type whose type
    /// variables do not fit its spec.
    ///
    /// Charm instances (`core.charm@1`) are created with
    /// [`Graph::instantiate`], not here, and nothing can be added inside
    /// one ([`GraphError::InInstance`]).
    pub fn add_node_in(
        &mut self,
        parent: Option<NodeId>,
        id: NodeId,
        qualified_id: &str,
        name: &str,
        port_type: Option<PortType>,
    ) -> Result<NodeId, GraphError> {
        if qualified_id == charm_defs::CHARM_TYPE {
            return Err(GraphError::NotAnInstance(id));
        }
        self.check_network_editable(parent)?;
        self.insert_node(parent, id, qualified_id, name, port_type)
    }

    /// [`Graph::add_node_in`] without the Charm instance checks.
    pub(crate) fn insert_node(
        &mut self,
        parent: Option<NodeId>,
        id: NodeId,
        qualified_id: &str,
        name: &str,
        port_type: Option<PortType>,
    ) -> Result<NodeId, GraphError> {
        if let Some(p) = parent {
            if !self.index.contains_key(&p) {
                return Err(GraphError::UnknownNode(p));
            }
            if !self.subnets.contains_key(&p) {
                return Err(GraphError::NotASubnet(p));
            }
        }
        let is_subnet = network::is_container_type(qualified_id);
        let boundary = network::is_boundary_type(qualified_id);
        network::check_name(
            name,
            parent.is_some() || is_subnet,
            parent.is_some() && boundary,
        )?;
        if self.find_in(parent, name).is_some() {
            return Err(GraphError::DuplicateName(name.to_owned()));
        }
        if self.index.contains_key(&id) {
            return Err(GraphError::DuplicateId(id));
        }
        let mut imp = self
            .registry
            .get(qualified_id)
            .ok_or_else(|| GraphError::UnknownNodeType(qualified_id.to_owned()))?
            .clone();
        let mut poly = Poly::of(&imp)?.map(Box::new);
        let pinnable = poly.as_ref().is_some_and(|p| p.types.len() == 1);
        if port_type.is_some() && !boundary && !pinnable {
            return Err(GraphError::NotPolymorphic(qualified_id.to_owned()));
        }
        if qualified_id == network::INPUT_TYPE && parent.is_none() {
            return Err(GraphError::NotInSubnet(id));
        }
        if let Some(p) = poly.as_deref_mut() {
            if let Some(ty) = port_type {
                p.types = vec![ty];
            }
            imp = livspeil_node_api::instantiate(&p.base, &p.types).map_err(|e| match e {
                livspeil_node_api::TypeError::NotInClass { var, class, found } => {
                    GraphError::NotInClass {
                        node: id,
                        var: var.to_owned(),
                        class,
                        found,
                    }
                }
                e => GraphError::InvalidTypeVars {
                    node_type: qualified_id.to_owned(),
                    reason: e.to_string(),
                },
            })?;
        } else if let Some(ty) = port_type {
            imp = network::retype(imp, ty);
        }
        let port_type = if poly.is_some() {
            port_type
        } else {
            boundary
                .then(|| imp.spec().inputs.first().map(|p| p.ty))
                .flatten()
        };
        let inputs = imp.spec().inputs.len();
        self.rev_counter += 1;
        self.topo_rev += 1;
        let node = Node {
            id,
            name: name.to_owned(),
            type_id: qualified_id.to_owned(),
            imp,
            parent,
            port_type,
            poly,
            values: vec![None; inputs],
            exprs: (0..inputs).map(|_| None).collect(),
            anims: vec![None; inputs],
            link_in: vec![None; inputs],
            rev: self.rev_counter,
        };
        let slot = match self.free.pop() {
            Some(slot) => {
                self.slots[slot] = Some(node);
                self.dependents[slot].clear();
                self.deps_rev += 1;
                self.dirty[slot] = self.rev_counter;
                slot
            }
            None => {
                self.slots.push(Some(node));
                self.dependents.push(Vec::new());
                self.deps_rev += 1;
                self.dirty.push(self.rev_counter);
                self.slots.len() - 1
            }
        };
        self.index.insert(id, slot);
        self.insert_name(parent, name, id);
        if is_subnet {
            self.subnets.insert(id, Subnet::default());
        }
        Ok(id)
    }

    /// Remove a node and every link attached to it. Inputs that were fed by
    /// the node fall back to their own sources, values or defaults.
    ///
    /// Removing a subnet removes everything inside it (its whole subtree).
    ///
    /// # Errors
    /// Fails if the node does not exist, or with [`GraphError::Referenced`]
    /// if an expression outside the removed subtree reads one of its
    /// inputs.
    pub fn remove_node(&mut self, id: NodeId) -> Result<(), GraphError> {
        if !self.index.contains_key(&id) {
            return Err(GraphError::UnknownNode(id));
        }
        self.check_structure_editable(id)?;
        self.remove_subtree(id)
    }

    /// [`Graph::remove_node`] without the Charm instance checks.
    pub(crate) fn remove_subtree(&mut self, id: NodeId) -> Result<(), GraphError> {
        let subtree = self.subtree(id);
        let mut retype = self.poly_neighbors(&subtree);
        retype.retain(|n| !subtree.contains(n));
        for &n in &subtree {
            for (by, port) in self.ch_dependents(n) {
                if !subtree.contains(&by) {
                    let port = self
                        .node(by)
                        .and_then(|b| b.imp.spec().inputs.get(port))
                        .map_or_else(String::new, |p| p.name.to_owned());
                    return Err(GraphError::Referenced { node: n, by, port });
                }
            }
        }
        // Expressions inside the subtree may read each other: drop them all
        // before removing any node.
        if subtree.len() > 1 {
            for &n in &subtree {
                let inputs = self.node(n).map_or(0, |n| n.exprs.len());
                for index in 0..inputs {
                    self.unindex_expression(n, index);
                    if let Some(node) = self.node_mut(n) {
                        node.exprs[index] = None;
                    }
                }
            }
        }
        for n in subtree {
            self.remove_one(n)?;
        }
        self.refresh_types(retype);
        Ok(())
    }

    /// Remove one node that no remaining expression reads.
    fn remove_one(&mut self, id: NodeId) -> Result<(), GraphError> {
        let Some(&slot) = self.index.get(&id) else {
            return Err(GraphError::UnknownNode(id));
        };
        self.topo_rev += 1;
        let inputs = self.node(id).map_or(0, |n| n.exprs.len());
        for index in 0..inputs {
            self.unindex_expression(id, index);
        }

        // Links into the removed node.
        let incoming: Vec<(NodeId, usize)> = self
            .links
            .range((id, 0)..=(id, usize::MAX))
            .map(|(key, _)| *key)
            .collect();
        for key in incoming {
            if let Some(src) = self.links.remove(&key) {
                self.outgoing.remove(&(src.node, key.0, key.1));
                if self.input_port_type(id, key.1) == Some(PortType::Event) {
                    continue;
                }
                if let Some(&from) = self.index.get(&src.node) {
                    self.remove_dependent(from, slot);
                }
            }
        }

        // Links out of the removed node: their targets change.
        let outgoing: Vec<(NodeId, NodeId, usize)> = self
            .outgoing
            .range((id, NodeId(Uuid::nil()), 0)..)
            .take_while(|(from, _, _)| *from == id)
            .copied()
            .collect();
        for (from, to, port) in outgoing {
            self.outgoing.remove(&(from, to, port));
            self.links.remove(&(to, port));
            if self.input_port_type(to, port) == Some(PortType::Event) {
                continue;
            }
            if let Some(n) = self.node_mut(to) {
                n.link_in[port] = None;
            }
            self.bump(to);
        }

        self.index.remove(&id);
        let node = self.slots[slot].take().ok_or(GraphError::UnknownNode(id))?;
        self.remove_name(node.parent, &node.name);
        self.subnets.remove(&id);
        self.instances.remove(&id);
        self.dependents[slot].clear();
        self.deps_rev += 1;
        self.free.push(slot);
        self.rev_counter += 1;
        Ok(())
    }

    /// Set the value of an unlinked input. A link, if present, still wins.
    /// `Int` values are accepted for `Float` ports.
    ///
    /// # Errors
    /// Fails for an unknown node or port, or a value of the wrong type.
    pub fn set_input(&mut self, node: NodeId, port: &str, value: Value) -> Result<(), GraphError> {
        let (index, ty) = self.input_port(node, port)?;
        self.check_sources_editable(node)?;
        let found = value.port_type();
        let poly = self.check_poly_source(node, index, SourceKind::Value, found)?;
        let ty = poly.unwrap_or(ty);
        let value = value.coerce(ty).ok_or(GraphError::TypeMismatch {
            expected: ty,
            found,
        })?;
        if let Some(n) = self.node_mut(node) {
            n.values[index] = Some(value);
        }
        self.bump(node);
        if poly.is_some() {
            self.refresh_types([node]);
        }
        Ok(())
    }

    /// Forget an explicitly set input value so the spec default applies.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    ///
    /// On an interface input of a Charm instance this restores the
    /// definition's value instead (see [`Graph::instantiate`]).
    pub fn clear_input(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        self.check_sources_editable(node)?;
        let reset = self.instance_default_value(node, port);
        if let Some(n) = self.node_mut(node) {
            n.values[index] = reset;
        }
        self.bump(node);
        if self.var_port(node, RefKind::Input, index).is_some() {
            self.refresh_types([node]);
        }
        Ok(())
    }

    /// Connect `from.from_port` (an output) to `to.to_port` (an input).
    ///
    /// Both nodes must be in the same network, or the link must cross one
    /// subnet boundary through the interface: from the network around a
    /// subnet into the `value` input of one of its `core.subnet.input@1`
    /// nodes, or from the `value` output of one of its `core.output@1`
    /// nodes to the network around it. An interface input's `value` input
    /// cannot be fed from inside its subnet.
    ///
    /// Links between `Event` ports (M0.10) are stored and serialized like
    /// any other link, but they are not value dependencies: they do not
    /// feed values, do not invalidate anything, and may form cycles (event
    /// loops are bounded per tick instead; see [`Cooker::tick`]).
    ///
    /// [`Cooker::tick`]: crate::Cooker::tick
    ///
    /// # Errors
    /// Fails for unknown nodes or ports, incompatible types,
    /// [`GraphError::CrossNetwork`], an input that already has a link, or a
    /// value link that would create a cycle.
    pub fn link(
        &mut self,
        from: NodeId,
        from_port: &str,
        to: NodeId,
        to_port: &str,
    ) -> Result<(), GraphError> {
        let (out_index, out_ty) = self.output_port(from, from_port)?;
        let (in_index, in_ty) = self.input_port(to, to_port)?;
        self.check_sources_editable(to)?;
        let from_end = Endpoint {
            node: from,
            port: out_index,
        };
        let to_end = Endpoint {
            node: to,
            port: in_index,
        };
        let poly = self.check_poly_link(from_end, to_end)?;
        if !poly && !in_ty.accepts(out_ty) {
            return Err(GraphError::TypeMismatch {
                expected: in_ty,
                found: out_ty,
            });
        }
        let (Some(&from_slot), Some(&to_slot)) = (self.index.get(&from), self.index.get(&to))
        else {
            return Err(GraphError::UnknownNode(from));
        };
        // Siblings (the common case) without further lookups.
        let siblings = match (self.node_at(from_slot), self.node_at(to_slot)) {
            (Some(f), Some(t)) => f.parent == t.parent && t.type_id != network::INPUT_TYPE,
            _ => false,
        };
        if !siblings && !self.link_visible(from, to, None) {
            return Err(GraphError::CrossNetwork { from, to });
        }
        if self.links.contains_key(&(to, in_index)) {
            return Err(GraphError::InputOccupied {
                node: to,
                port: to_port.to_owned(),
            });
        }
        // Event links are not value dependencies (see `events.rs`): they
        // may form cycles and are routed by the cooker's events phase.
        let event = out_ty == PortType::Event;
        if !event && self.depends_on(from, to) {
            return Err(GraphError::WouldCycle);
        }
        self.links.insert(
            (to, in_index),
            Endpoint {
                node: from,
                port: out_index,
            },
        );
        self.outgoing.insert((from, to, in_index));
        if event {
            self.rev_counter += 1;
            self.topo_rev += 1;
            return Ok(());
        }
        if let Some(n) = self.node_mut(to) {
            n.link_in[in_index] = Some((from_slot, out_index));
        }
        self.add_dependent(from_slot, to_slot);
        self.topo_rev += 1;
        self.bump(to);
        if poly {
            self.refresh_types([from, to]);
        }
        Ok(())
    }

    /// Remove the link feeding `to.to_port`, if any. Returns whether a link
    /// was removed.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn unlink(&mut self, to: NodeId, to_port: &str) -> Result<bool, GraphError> {
        let (index, ty) = self.input_port(to, to_port)?;
        self.check_sources_editable(to)?;
        let Some(src) = self.links.remove(&(to, index)) else {
            return Ok(false);
        };
        self.outgoing.remove(&(src.node, to, index));
        if ty == PortType::Event {
            self.rev_counter += 1;
            self.topo_rev += 1;
            return Ok(true);
        }
        if let Some(n) = self.node_mut(to) {
            n.link_in[index] = None;
        }
        if let (Some(&from_slot), Some(&to_slot)) = (self.index.get(&src.node), self.index.get(&to))
        {
            self.remove_dependent(from_slot, to_slot);
        }
        self.topo_rev += 1;
        self.bump(to);
        if self.var_port(to, RefKind::Input, index).is_some()
            || self.var_port(src.node, RefKind::Output, src.port).is_some()
        {
            self.refresh_types([src.node, to]);
        }
        Ok(true)
    }

    /// Rename a node. Returns the previous name. Renaming does not change
    /// what a node computes, so it does not invalidate cooked results.
    /// Renaming a node to its current name is a no-op.
    ///
    /// Every `ch()` path that reads the node, or passes through it (a
    /// subnet), is rewritten to the new name (see
    /// [`Graph::set_expression`]). Names are unique per network.
    ///
    /// # Errors
    /// Fails for an unknown node, an empty name or a name used by another
    /// node.
    pub fn rename(&mut self, id: NodeId, name: &str) -> Result<String, GraphError> {
        if name.is_empty() {
            return Err(GraphError::EmptyName);
        }
        let current = self.node(id).ok_or(GraphError::UnknownNode(id))?;
        if current.name == name {
            return Ok(name.to_owned());
        }
        self.check_structure_editable(id)?;
        let current = self.node(id).ok_or(GraphError::UnknownNode(id))?;
        let parent = current.parent;
        let port = parent.is_some() && network::is_boundary_type(&current.type_id);
        network::check_name(name, parent.is_some() || self.is_subnet(id), port)?;
        if self.find_in(parent, name).is_some() {
            return Err(GraphError::DuplicateName(name.to_owned()));
        }
        // Rewrite the paths against the current names, then rename and
        // check that every rewritten expression still reads the same.
        let rewrites = self.rename_rewrites(id, name);
        let node = self.node_mut(id).ok_or(GraphError::UnknownNode(id))?;
        let old = std::mem::replace(&mut node.name, name.to_owned());
        self.remove_name(parent, &old);
        self.insert_name(parent, name, id);
        if let Err(err) = self.apply_rewrites(rewrites) {
            self.remove_name(parent, name);
            self.insert_name(parent, &old, id);
            if let Some(node) = self.node_mut(id) {
                node.name = old;
            }
            return Err(err);
        }
        Ok(old)
    }

    /// Whether a node with this id exists.
    #[must_use]
    pub fn contains(&self, id: NodeId) -> bool {
        self.index.contains_key(&id)
    }

    /// The explicitly set value of an input, or `None` when the input uses
    /// its spec default. Links are not considered.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn input_value(&self, node: NodeId, port: &str) -> Result<Option<&Value>, GraphError> {
        let (index, _) = self.input_port(node, port)?;
        Ok(self
            .node(node)
            .and_then(|n| n.values.get(index))
            .and_then(Option::as_ref))
    }

    /// Every explicitly set input value of a node as `(port name, value)`,
    /// in port order. Empty for an unknown node.
    pub fn input_values(&self, node: NodeId) -> impl Iterator<Item = (&str, &Value)> + '_ {
        self.node(node).into_iter().flat_map(|n| {
            let spec = n.imp.spec();
            n.values
                .iter()
                .zip(&spec.inputs)
                .filter_map(|(value, port)| value.as_ref().map(|v| (port.name, v)))
        })
    }

    /// The output feeding input `to_port` of node `to`, as
    /// `(source node, output port name)`, or `None` when it is unlinked.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn link_source(
        &self,
        to: NodeId,
        to_port: &str,
    ) -> Result<Option<(NodeId, &str)>, GraphError> {
        let (index, _) = self.input_port(to, to_port)?;
        Ok(self.links.get(&(to, index)).and_then(|src| {
            let spec = self.node(src.node)?.imp.spec();
            Some((src.node, spec.outputs.get(src.port)?.name))
        }))
    }

    /// The links feeding a node, by input port order, with port names.
    #[must_use]
    pub fn incoming_links(&self, id: NodeId) -> Vec<NamedLink> {
        self.links
            .range((id, 0)..=(id, usize::MAX))
            .filter_map(|(&(node, port), &from)| {
                self.named(Link {
                    from,
                    to: Endpoint { node, port },
                })
            })
            .collect()
    }

    /// The links leaving a node, ordered by target node and input port,
    /// with port names.
    #[must_use]
    pub fn outgoing_links(&self, id: NodeId) -> Vec<NamedLink> {
        self.outgoing
            .range((id, NodeId(Uuid::nil()), 0)..)
            .take_while(|(from, _, _)| *from == id)
            .filter_map(|&(_, to, port)| {
                let from = *self.links.get(&(to, port))?;
                self.named(Link {
                    from,
                    to: Endpoint { node: to, port },
                })
            })
            .collect()
    }

    /// A link with port names instead of indices.
    #[must_use]
    pub fn named(&self, link: Link) -> Option<NamedLink> {
        let from_spec = self.node(link.from.node)?.imp.spec();
        let to_spec = self.node(link.to.node)?.imp.spec();
        Some(NamedLink {
            from: link.from.node,
            from_port: from_spec.outputs.get(link.from.port)?.name.to_owned(),
            to: link.to.node,
            to_port: to_spec.inputs.get(link.to.port)?.name.to_owned(),
        })
    }

    /// A canonical, comparable copy of the editable state: node ids, names,
    /// types, explicitly set inputs, expressions, animation channels and
    /// links. Cook revisions and the graph
    /// instance id are not part of it, so two graphs built by the same
    /// edits have equal snapshots.
    #[must_use]
    pub fn snapshot(&self) -> GraphSnapshot {
        let nodes = self
            .index
            .keys()
            .filter_map(|&id| Some((id, self.node(id)?)))
            .map(|(id, n)| NodeSnapshot {
                id,
                name: n.name.clone(),
                type_id: n.type_id.clone(),
                inputs: self
                    .input_values(id)
                    .map(|(port, value)| (port.to_owned(), value.clone()))
                    .collect(),
                expressions: self
                    .expressions(id)
                    .map(|(port, source)| (port.to_owned(), source.to_owned()))
                    .collect(),
                animations: self
                    .animations(id)
                    .map(|(port, channel)| (port.to_owned(), channel.clone()))
                    .collect(),
                parent: n.parent,
                port_type: n.port_type,
                charm: self.subnets.get(&id).and_then(|s| s.charm.clone()),
                def: self.instances.get(&id).cloned(),
            })
            .collect();
        let links = self.links().filter_map(|l| self.named(l)).collect();
        GraphSnapshot {
            nodes,
            links,
            definitions: self.defs.values().cloned().collect(),
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.index.len()
    }

    /// Number of links.
    #[must_use]
    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    /// All node ids in id order.
    pub fn node_ids(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.index.keys().copied()
    }

    /// The name of a node.
    #[must_use]
    pub fn node_name(&self, id: NodeId) -> Option<&str> {
        self.node(id).map(|n| n.name.as_str())
    }

    /// The qualified type id of a node.
    #[must_use]
    pub fn node_type(&self, id: NodeId) -> Option<&str> {
        self.node(id).map(|n| n.type_id.as_str())
    }

    /// The spec of a node's type.
    #[must_use]
    pub fn node_spec(&self, id: NodeId) -> Option<&NodeSpec> {
        self.node(id).map(|n| n.imp.spec())
    }

    /// Find a node of the root network by name (see [`Graph::find_in`]
    /// and [`Graph::find_path`] for nested networks).
    #[must_use]
    pub fn find(&self, name: &str) -> Option<NodeId> {
        self.names.get(name).copied()
    }

    /// Find a node by name in the network of subnet `network` (`None`:
    /// the root network).
    #[must_use]
    pub fn find_in(&self, network: Option<NodeId>, name: &str) -> Option<NodeId> {
        match network {
            None => self.names.get(name).copied(),
            Some(s) => self.subnets.get(&s)?.names.get(name).copied(),
        }
    }

    pub(crate) fn insert_name(&mut self, network: Option<NodeId>, name: &str, id: NodeId) {
        match network {
            None => {
                self.names.insert(name.to_owned(), id);
            }
            Some(s) => {
                if let Some(sub) = self.subnets.get_mut(&s) {
                    sub.names.insert(name.to_owned(), id);
                }
            }
        }
    }

    pub(crate) fn remove_name(&mut self, network: Option<NodeId>, name: &str) {
        match network {
            None => {
                self.names.remove(name);
            }
            Some(s) => {
                if let Some(sub) = self.subnets.get_mut(&s) {
                    sub.names.remove(name);
                }
            }
        }
    }

    /// Move node `id` (with its subtree) from its network to the network
    /// of `parent`, updating the name tables. No checks.
    pub(crate) fn set_parent(&mut self, id: NodeId, parent: Option<NodeId>) {
        let Some(node) = self.node(id) else {
            return;
        };
        let (old, name) = (node.parent, node.name.clone());
        self.remove_name(old, &name);
        self.insert_name(parent, &name, id);
        if let Some(node) = self.node_mut(id) {
            node.parent = parent;
        }
    }

    /// Set the Charm metadata slot of a subnet (no checks).
    pub(crate) fn charm_slot(&mut self, subnet: NodeId) -> Option<&mut Option<CharmMeta>> {
        self.subnets.get_mut(&subnet).map(|s| &mut s.charm)
    }

    /// The source of the link into input `port` of `node`.
    pub(crate) fn link_source_endpoint(&self, node: NodeId, port: usize) -> Option<Endpoint> {
        self.links.get(&(node, port)).copied()
    }

    /// The inputs fed by outputs `ports` of `node`.
    pub(crate) fn link_targets(&self, node: NodeId, ports: &[usize]) -> Vec<Endpoint> {
        self.outgoing
            .range((node, NodeId::MIN, 0)..)
            .take_while(|(from, _, _)| *from == node)
            .filter(|&&(_, to, port)| {
                self.links
                    .get(&(to, port))
                    .is_some_and(|src| ports.contains(&src.port))
            })
            .map(|&(_, to, port)| Endpoint { node: to, port })
            .collect()
    }

    /// The nodes linked into `node`.
    pub(crate) fn incoming_endpoints(&self, node: NodeId) -> Vec<NodeId> {
        self.links
            .range((node, 0)..=(node, usize::MAX))
            .map(|(_, src)| src.node)
            .collect()
    }

    /// The nodes `node` is linked into.
    pub(crate) fn outgoing_endpoints(&self, node: NodeId) -> Vec<NodeId> {
        self.outgoing
            .range((node, NodeId::MIN, 0)..)
            .take_while(|(from, _, _)| *from == node)
            .map(|&(_, to, _)| to)
            .collect()
    }

    /// All links, ordered by target node and input port.
    pub fn links(&self) -> impl Iterator<Item = Link> + '_ {
        self.links.iter().map(|(&(node, port), &from)| Link {
            from,
            to: Endpoint { node, port },
        })
    }

    /// The node with this id.
    pub(crate) fn node(&self, id: NodeId) -> Option<&Node> {
        self.index.get(&id).and_then(|&slot| self.node_at(slot))
    }

    pub(crate) fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let slot = *self.index.get(&id)?;
        self.slots.get_mut(slot)?.as_mut()
    }

    /// The node in a slot, if the slot is in use.
    pub(crate) fn node_at(&self, slot: usize) -> Option<&Node> {
        self.slots.get(slot)?.as_ref()
    }

    /// The slot of a node.
    pub(crate) fn slot_of(&self, id: NodeId) -> Option<usize> {
        self.index.get(&id).copied()
    }

    /// Number of slots (used and free); every slot is below this.
    pub(crate) fn slot_count(&self) -> usize {
        self.slots.len()
    }

    /// The epoch of the last edit that affected the node in `slot` (see
    /// [`Graph`]). 0 for unknown slots.
    pub(crate) fn dirty_at(&self, slot: usize) -> u64 {
        self.dirty.get(slot).copied().unwrap_or(0)
    }

    /// Direct dependents of the node in `slot` (with repeats).
    pub(crate) fn dependents_at(&self, slot: usize) -> &[usize] {
        self.dependents.get(slot).map_or(&[], Vec::as_slice)
    }

    /// The current edit epoch: changes with every edit.
    pub(crate) fn epoch(&self) -> u64 {
        self.rev_counter
    }

    /// Record that a cooker is validating nodes against epoch `epoch`.
    pub(crate) fn observe(&self, epoch: u64) {
        self.observed.fetch_max(epoch, Ordering::Relaxed);
    }

    /// Revision of the dependency structure and of time dependence.
    pub(crate) fn topo_rev(&self) -> u64 {
        self.topo_rev
    }

    /// Note a change of the dependency structure or time dependence.
    pub(crate) fn structure_changed(&mut self) {
        self.topo_rev += 1;
    }

    /// Record a `ch()` edge `read -> reader` in the dense dependents.
    pub(crate) fn add_dependent(&mut self, from: usize, to: usize) {
        if let Some(list) = self.dependents.get_mut(from) {
            list.push(to);
            self.deps_rev += 1;
        }
    }

    /// Forget one `from -> to` edge from the dense dependents.
    pub(crate) fn remove_dependent(&mut self, from: usize, to: usize) {
        if let Some(list) = self.dependents.get_mut(from) {
            if let Some(pos) = list.iter().position(|&s| s == to) {
                list.swap_remove(pos);
                self.deps_rev += 1;
            }
        }
    }

    /// A new edit epoch for node `id`: bump its revision and mark it and
    /// everything downstream dirty.
    pub(crate) fn bump(&mut self, id: NodeId) {
        self.rev_counter += 1;
        let epoch = self.rev_counter;
        let Some(slot) = self.slot_of(id) else {
            return;
        };
        if let Some(node) = self.slots.get_mut(slot).and_then(Option::as_mut) {
            node.rev = epoch;
        }
        self.mark_dirty(slot, epoch);
    }

    /// Mark `start` and its downstream with `epoch`, stopping at nodes that
    /// are already stale for every cooker (see [`Graph`]).
    fn mark_dirty(&mut self, start: usize, epoch: u64) {
        let observed = *self.observed.get_mut();
        match self.dirty.get_mut(start) {
            Some(dirty) if *dirty <= observed => *dirty = epoch,
            _ => return,
        }
        if self.walk.built_for != Some(self.deps_rev) {
            self.walk.rebuild(&self.dependents);
            self.walk.built_for = Some(self.deps_rev);
        }
        // Nodes are marked when pushed, so each is pushed at most once.
        let mut stack = std::mem::take(&mut self.mark_stack);
        stack.clear();
        stack.push(start);
        while let Some(slot) = stack.pop() {
            for &dep in self.walk.dependents(slot) {
                if let Some(dirty) = self.dirty.get_mut(dep) {
                    if *dirty <= observed {
                        *dirty = epoch;
                        stack.push(dep);
                    }
                }
            }
        }
        self.mark_stack = stack;
    }

    pub(crate) fn input_port(
        &self,
        node: NodeId,
        port: &str,
    ) -> Result<(usize, PortType), GraphError> {
        let n = self.node(node).ok_or(GraphError::UnknownNode(node))?;
        let spec = n.imp.spec();
        spec.input_index(port)
            .map(|i| (i, spec.inputs[i].ty))
            .ok_or_else(|| GraphError::UnknownPort {
                node,
                port: port.to_owned(),
                direction: PortDirection::Input,
            })
    }

    /// The type of input `index` of `node`.
    fn input_port_type(&self, node: NodeId, index: usize) -> Option<PortType> {
        Some(self.node(node)?.imp.spec().inputs.get(index)?.ty)
    }

    fn output_port(&self, node: NodeId, port: &str) -> Result<(usize, PortType), GraphError> {
        let n = self.node(node).ok_or(GraphError::UnknownNode(node))?;
        let spec = n.imp.spec();
        spec.output_index(port)
            .map(|i| (i, spec.outputs[i].ty))
            .ok_or_else(|| GraphError::UnknownPort {
                node,
                port: port.to_owned(),
                direction: PortDirection::Output,
            })
    }

    /// Whether `from` is `to` or already depends on `to`, i.e. whether
    /// adding the link (or `ch()` reference) `from -> to` would close a
    /// cycle. Walks downstream from `to` through links and `ch()`
    /// references, which is cheap when `to` is new.
    pub(crate) fn depends_on(&self, from: NodeId, to: NodeId) -> bool {
        if from == to {
            return true;
        }
        let (Some(from), Some(to)) = (self.slot_of(from), self.slot_of(to)) else {
            return false;
        };
        let mut seen = HashSet::new();
        let mut stack = vec![to];
        while let Some(current) = stack.pop() {
            if !seen.insert(current) {
                continue;
            }
            for &next in self.dependents_at(current) {
                if next == from {
                    return true;
                }
                stack.push(next);
            }
        }
        false
    }
}
