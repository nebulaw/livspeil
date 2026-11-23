// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared Charm definitions and their instances (milestone M0.16).
//!
//! # Design
//!
//! A graph holds a library of **definitions** ([`CharmDef`]: id, version,
//! [`CharmMeta`], and the definition's network as node snapshots and
//! links with *definition-local* node ids). A definition is data, not
//! part of the cooked graph.
//!
//! An **instance** is a node of type [`CHARM_TYPE`] (`core.charm@1`) that
//! uses a definition. Storage stays flat: an instance is a container like
//! a subnet, and its contents are *materialized* as ordinary nodes of the
//! one flat graph, so cooking, caching, early cutoff, `ch()`/`out()`
//! paths and the network rules need nothing new. The id of each
//! materialized node is derived from the instance id and the
//! definition-local id ([`instance_node_id`]: FNV-1a 128 plus a mixing
//! step, as a UUIDv8), so it is the same in every session, journal and
//! document, and `ch("lamp/level/x")` from outside keeps reading the same
//! node across edits of the definition.
//!
//! **Per-instance data** is exactly: the instance's id, name and place,
//! its Charm metadata copy (the definition's), and the sources (static
//! value, expression, animation channel) of the `value` input of each of
//! its interface inputs (the definition's promoted parameters), plus the
//! links into its interface inputs and out of its interface outputs.
//! Everything inside an instance is otherwise **read-only**
//! ([`GraphError::InInstance`]): instances cannot override inner
//! parameters except through promoted interface inputs. Edit the
//! definition ([`Graph::update_definition`]) to change every instance, or
//! [`Graph::detach`] one instance into a plain, editable subnet.
//!
//! **Defaults and overrides**: the definition's interface inputs carry
//! default sources. Each kind of source (value, expression, animation) of
//! an instance input either *follows* the definition (equal to its
//! default) or is an *override* (different). Clearing a source of an
//! instance input restores the definition's default of that kind, so an
//! instance input never lacks a source its definition has. When the
//! definition changes, followed sources take the new defaults and
//! overrides are kept. Documents write only the overrides.
//!
//! **Self-contained**: links and `ch()`/`out()` paths inside a definition
//! stay inside it (no absolute paths, no `../` above the definition's
//! network), so materialized contents never depend on where the instance
//! is or what it is called. Definitions cannot contain instances yet.
//!
//! **Updating** a definition re-synchronizes every instance with the
//! smallest edits it can (nodes matched by local id: kept, renamed,
//! re-sourced, relinked, added or removed). Nodes whose inputs or links
//! change are invalidated by the ordinary primitive edits, so long-lived
//! cookers stay correct. Expressions outside an instance that read inside
//! it are re-rendered (like [`Graph::move_node`] does) so they keep reading
//! the same nodes; reading a node the new definition removes is refused
//! ([`GraphError::Referenced`]). Links into interface ports that the new
//! definition removes are dropped and reported. Composite operations are
//! all-or-nothing: a failed update leaves the graph exactly as it was.

use std::collections::{BTreeMap, BTreeSet};

use livspeil_node_api::Value;
use livspeil_params::AnimChannel;
use livspeil_params::expr;
use uuid::Uuid;

use crate::graph::{Graph, GraphError, NodeId};
use crate::network::{CharmMeta, INPUT_TYPE, INTERFACE_PORT, SUBNET_TYPE};
use crate::snapshot::{NamedLink, NodeSnapshot};
use crate::sources::{render_path, resolve_path};

/// Qualified id of the Charm instance node type: a container whose
/// contents come from a [`CharmDef`].
pub const CHARM_TYPE: &str = "core.charm@1";

/// A shared Charm definition (see the [module docs](self)).
///
/// `nodes` are the definition's network, every network inside it
/// included, with definition-local ids; `parent` is `None` for nodes of
/// the definition's own network and the local id of an inner subnet
/// otherwise. `links` use local ids and port names. `meta.version` is
/// not used: the definition's version is `version`, and instances report
/// it in their metadata copy ([`Graph::charm`]).
///
/// [`Graph::add_definition`] stores a canonical form: nodes by id, links
/// ordered, input sources in port order, as [`crate::Graph::snapshot`]
/// writes them.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CharmDef {
    /// Identifier, unique in the graph (letters, digits, `_`, `-`, `.`).
    pub id: String,
    /// Version, free-form (semver recommended); no `@` or whitespace.
    pub version: String,
    /// Display name, icon, capabilities, minimum tier.
    pub meta: CharmMeta,
    pub nodes: Vec<NodeSnapshot>,
    pub links: Vec<NamedLink>,
}

impl CharmDef {
    /// An empty definition.
    #[must_use]
    pub fn new(id: impl Into<String>, version: impl Into<String>, meta: CharmMeta) -> Self {
        CharmDef {
            id: id.into(),
            version: version.into(),
            meta,
            nodes: Vec::new(),
            links: Vec::new(),
        }
    }

    /// The node with definition-local id `id`.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&NodeSnapshot> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// The metadata instances report: [`CharmDef::meta`] with the
    /// definition's version.
    #[must_use]
    pub fn instance_meta(&self) -> CharmMeta {
        let mut meta = self.meta.clone();
        meta.version = Some(self.version.clone());
        meta
    }

    /// The interface inputs (`core.subnet.input@1` nodes of the
    /// definition's own network), in id order.
    pub fn interface_inputs(&self) -> impl Iterator<Item = &NodeSnapshot> {
        self.nodes
            .iter()
            .filter(|n| n.parent.is_none() && n.type_id == INPUT_TYPE)
    }
}

/// What [`Graph::update_definition`] changed besides the instances'
/// contents.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DefinitionUpdate {
    /// The instances that were re-synchronized, in id order.
    pub instances: Vec<NodeId>,
    /// Expressions (outside the instances, or overrides on their interface
    /// inputs) whose `ch()`/`out()` paths were re-rendered, as `(node,
    /// input port, previous source)`.
    pub rewritten: Vec<(NodeId, String, String)>,
    /// Links into interface ports the new definition no longer has.
    pub unlinked: Vec<NamedLink>,
}

/// The id of the node with definition-local id `local` inside instance
/// `instance`: deterministic, the same in every session.
#[must_use]
pub fn instance_node_id(instance: NodeId, local: NodeId) -> NodeId {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    const MIX: u128 = 0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835;
    let mut h = OFFSET;
    for b in instance
        .uuid()
        .as_bytes()
        .iter()
        .chain(local.uuid().as_bytes())
    {
        h ^= u128::from(*b);
        h = h.wrapping_mul(PRIME);
    }
    h ^= h >> 64;
    h = h.wrapping_mul(MIX);
    h ^= h >> 61;
    let mut bytes = h.to_be_bytes();
    bytes[6] = (bytes[6] & 0x0f) | 0x80; // version 8 (custom)
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // RFC 9562 variant
    NodeId::from_uuid(Uuid::from_bytes(bytes))
}

/// The container of the definition network while validating.
const SCRATCH: NodeId = NodeId::from_uuid(Uuid::max());

fn invalid(def: &str, reason: impl Into<String>) -> GraphError {
    GraphError::InvalidDefinition {
        def: def.to_owned(),
        reason: reason.into(),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn valid_version(version: &str) -> bool {
    !version.is_empty() && !version.contains('@') && !version.chars().any(char::is_whitespace)
}

/// Whether every `ch()`/`out()` path of `source` stays inside a
/// definition, for a node `depth` subnets deep in it.
fn paths_stay_inside(source: &str, depth: usize) -> bool {
    let Ok(refs) = expr::channel_refs(source) else {
        return true; // Reported by the compiler.
    };
    refs.iter().all(|r| {
        if r.path.starts_with('/') {
            return false;
        }
        let ups = r.path.split('/').take_while(|s| *s == "..").count();
        ups <= depth
    })
}

/// The sources of an interface input's `value` port.
#[derive(Debug, Clone, PartialEq)]
struct Sources {
    value: Option<Value>,
    expr: Option<String>,
    anim: Option<AnimChannel>,
}

fn template_sources(node: &NodeSnapshot) -> Sources {
    let pick = |port: &str| port == INTERFACE_PORT;
    Sources {
        value: node
            .inputs
            .iter()
            .find(|(p, _)| pick(p))
            .map(|(_, v)| v.clone()),
        expr: node
            .expressions
            .iter()
            .find(|(p, _)| pick(p))
            .map(|(_, s)| s.clone()),
        anim: node
            .animations
            .iter()
            .find(|(p, _)| pick(p))
            .map(|(_, a)| a.clone()),
    }
}

/// An expression to restore after a sync, with what each path read.
struct Reader {
    node: NodeId,
    port: String,
    source: String,
    reads: Vec<(expr::ChannelRef, Option<NodeId>)>,
}

impl Graph {
    /// Every Charm definition, by id.
    pub fn definitions(&self) -> impl Iterator<Item = &CharmDef> {
        self.defs.values()
    }

    /// The Charm definition `id`.
    #[must_use]
    pub fn definition(&self, id: &str) -> Option<&CharmDef> {
        self.defs.get(id)
    }

    /// Whether `id` is a Charm instance.
    #[must_use]
    pub fn is_instance(&self, id: NodeId) -> bool {
        self.instances.contains_key(&id)
    }

    /// The definition Charm instance `id` uses.
    #[must_use]
    pub fn instance_def(&self, id: NodeId) -> Option<&str> {
        self.instances.get(&id).map(String::as_str)
    }

    /// The instances of definition `def`, in id order.
    #[must_use]
    pub fn instances_of(&self, def: &str) -> Vec<NodeId> {
        self.instances
            .iter()
            .filter(|(_, d)| d.as_str() == def)
            .map(|(&id, _)| id)
            .collect()
    }

    /// The Charm instance whose contents hold `node` (`None` for a node
    /// that is not inside an instance, the instance node itself included).
    #[must_use]
    pub fn instance_holding(&self, node: NodeId) -> Option<NodeId> {
        if self.instances.is_empty() {
            return None;
        }
        let mut current = self.parent(node);
        while let Some(s) = current {
            if self.instances.contains_key(&s) {
                return Some(s);
            }
            current = self.parent(s);
        }
        None
    }

    /// Fails unless `node` may change structurally (be removed, renamed,
    /// moved, promoted, get Charm metadata): not inside an instance.
    pub(crate) fn check_structure_editable(&self, node: NodeId) -> Result<(), GraphError> {
        match self.instance_holding(node) {
            Some(_) => Err(GraphError::InInstance(node)),
            None => Ok(()),
        }
    }

    /// Fails unless nodes may be added to (or moved into) network `net`.
    pub(crate) fn check_network_editable(&self, net: Option<NodeId>) -> Result<(), GraphError> {
        match net {
            Some(s) if self.is_instance(s) || self.instance_holding(s).is_some() => {
                Err(GraphError::InInstance(s))
            }
            _ => Ok(()),
        }
    }

    /// Fails unless the input sources and incoming links of `node` may
    /// change: not inside an instance, or an interface input of one.
    pub(crate) fn check_sources_editable(&self, node: NodeId) -> Result<(), GraphError> {
        match self.instance_holding(node) {
            None => Ok(()),
            Some(inst)
                if self.parent(node) == Some(inst) && self.node_type(node) == Some(INPUT_TYPE) =>
            {
                Ok(())
            }
            Some(_) => Err(GraphError::InInstance(node)),
        }
    }

    /// The definition's template of `node` when it is an interface input
    /// of an instance.
    fn instance_template(&self, node: NodeId) -> Option<&NodeSnapshot> {
        let inst = self.instance_holding(node)?;
        if self.parent(node) != Some(inst) || self.node_type(node) != Some(INPUT_TYPE) {
            return None;
        }
        let def = self.defs.get(self.instances.get(&inst)?)?;
        def.interface_inputs()
            .find(|n| instance_node_id(inst, n.id) == node)
    }

    /// The definition's static value of input `port` of `node`, when it is
    /// an interface input of an instance.
    pub(crate) fn instance_default_value(&self, node: NodeId, port: &str) -> Option<Value> {
        let t = self.instance_template(node)?;
        t.inputs
            .iter()
            .find(|(p, _)| p == port)
            .map(|(_, v)| v.clone())
    }

    /// Like [`Graph::instance_default_value`] for the expression.
    pub(crate) fn instance_default_expression(&self, node: NodeId, port: &str) -> Option<String> {
        let t = self.instance_template(node)?;
        t.expressions
            .iter()
            .find(|(p, _)| p == port)
            .map(|(_, s)| s.clone())
    }

    /// Like [`Graph::instance_default_value`] for the animation channel.
    pub(crate) fn instance_default_animation(
        &self,
        node: NodeId,
        port: &str,
    ) -> Option<AnimChannel> {
        let t = self.instance_template(node)?;
        t.animations
            .iter()
            .find(|(p, _)| p == port)
            .map(|(_, a)| a.clone())
    }

    /// Add a Charm definition to the graph's library. It is validated by
    /// building it on its own (node types, names, ports, links, cycles,
    /// expressions) and stored in canonical form.
    ///
    /// # Errors
    /// [`GraphError::DuplicateDefinition`], or
    /// [`GraphError::InvalidDefinition`] (bad id or version, an instance
    /// inside, paths leaving the definition, or any edit error building
    /// it).
    pub fn add_definition(&mut self, def: CharmDef) -> Result<(), GraphError> {
        if self.defs.contains_key(&def.id) {
            return Err(GraphError::DuplicateDefinition(def.id));
        }
        let def = self.validate_definition(&def)?;
        self.defs.insert(def.id.clone(), def);
        Ok(())
    }

    /// Remove an unused definition and return it.
    ///
    /// # Errors
    /// [`GraphError::UnknownDefinition`], or
    /// [`GraphError::DefinitionInUse`] while an instance uses it.
    pub fn remove_definition(&mut self, id: &str) -> Result<CharmDef, GraphError> {
        if !self.defs.contains_key(id) {
            return Err(GraphError::UnknownDefinition(id.to_owned()));
        }
        if let Some(&instance) = self.instances_of(id).first() {
            return Err(GraphError::DefinitionInUse {
                def: id.to_owned(),
                instance,
            });
        }
        self.defs
            .remove(id)
            .ok_or_else(|| GraphError::UnknownDefinition(id.to_owned()))
    }

    /// Check `def` and return its canonical form.
    fn validate_definition(&self, def: &CharmDef) -> Result<CharmDef, GraphError> {
        let id = def.id.as_str();
        if !valid_id(id) {
            return Err(invalid(
                id,
                "ids use letters, digits, '_', '-' and '.' (not '.' or '..')",
            ));
        }
        if !valid_version(&def.version) {
            return Err(invalid(
                id,
                format!(
                    "version '{}' must be non-empty, without '@' or spaces",
                    def.version
                ),
            ));
        }
        let by_id: BTreeMap<NodeId, &NodeSnapshot> = def.nodes.iter().map(|n| (n.id, n)).collect();
        if by_id.len() != def.nodes.len() {
            return Err(invalid(id, "duplicate node id"));
        }
        if by_id.contains_key(&SCRATCH) {
            return Err(invalid(id, "the maximal UUID is reserved"));
        }
        for n in &def.nodes {
            if n.type_id == CHARM_TYPE {
                return Err(invalid(
                    id,
                    format!(
                        "node '{}' is a Charm instance; definitions cannot contain instances yet",
                        n.name
                    ),
                ));
            }
            let depth = self::depth(&by_id, n).ok_or_else(|| {
                invalid(
                    id,
                    format!(
                        "node '{}' has a parent that is not a subnet of the definition",
                        n.name
                    ),
                )
            })?;
            for (port, source) in &n.expressions {
                if !paths_stay_inside(source, depth) {
                    return Err(invalid(
                        id,
                        format!(
                            "node '{}' input '{port}': ch()/out() paths must stay inside the definition (no absolute paths, no '../' above it)",
                            n.name
                        ),
                    ));
                }
            }
        }
        for l in &def.links {
            if !by_id.contains_key(&l.from) || !by_id.contains_key(&l.to) {
                return Err(invalid(
                    id,
                    "a link names a node the definition does not have",
                ));
            }
        }
        let mut scratch = Graph::new(std::sync::Arc::clone(self.registry()));
        scratch
            .insert_node(None, SCRATCH, SUBNET_TYPE, "charm", None)
            .map_err(|e| invalid(id, e.to_string()))?;
        scratch
            .sync(SCRATCH, def, None, &|l| l)
            .map_err(|e| invalid(id, e.to_string()))?;
        let mut canonical = scratch.capture(SCRATCH, id, &def.version, false)?;
        canonical.meta = def.meta.clone();
        canonical.meta.version = None;
        Ok(canonical)
    }

    /// Capture subnet `subnet`'s network as a definition `id` at `version`
    /// (local ids are the current node ids; metadata from the subnet's
    /// Charm metadata, or its name). Does not change the graph.
    ///
    /// # Errors
    /// [`GraphError::UnknownNode`], [`GraphError::NotASubnet`] (also for a
    /// Charm instance), or [`GraphError::InvalidDefinition`] when the
    /// network holds an instance or a `ch()`/`out()` path leaves it.
    pub fn definition_from_subnet(
        &self,
        subnet: NodeId,
        id: &str,
        version: &str,
    ) -> Result<CharmDef, GraphError> {
        self.capture(subnet, id, version, false)
    }

    /// [`Graph::definition_from_subnet`]; with `strip`, expressions of the
    /// subnet's own interface inputs that leave it are left out (they are
    /// per-instance data) instead of failing.
    fn capture(
        &self,
        subnet: NodeId,
        id: &str,
        version: &str,
        strip: bool,
    ) -> Result<CharmDef, GraphError> {
        if !self.contains(subnet) {
            return Err(GraphError::UnknownNode(subnet));
        }
        if self.node_type(subnet) != Some(SUBNET_TYPE) {
            return Err(GraphError::NotASubnet(subnet));
        }
        let members: BTreeSet<NodeId> = self.subtree(subnet).into_iter().collect();
        let mut nodes = Vec::new();
        for &n in &members {
            if n == subnet {
                continue;
            }
            let (Some(name), Some(type_id)) = (self.node_name(n), self.node_type(n)) else {
                continue;
            };
            if type_id == CHARM_TYPE {
                return Err(invalid(
                    id,
                    format!(
                        "node '{name}' is a Charm instance; definitions cannot contain instances yet"
                    ),
                ));
            }
            let parent = self.parent(n).filter(|&p| p != subnet);
            let interface = parent.is_none() && type_id == INPUT_TYPE;
            let depth = self.ancestors(self.parent(n)).len()
                - self.ancestors(self.parent(subnet)).len()
                - 1;
            let mut expressions = Vec::new();
            for (port, source) in self.expressions(n) {
                let inside = paths_stay_inside(source, depth)
                    && self
                        .expression_targets(n, port)
                        .iter()
                        .all(|t| members.contains(t));
                if inside {
                    expressions.push((port.to_owned(), source.to_owned()));
                } else if !(strip && interface) {
                    return Err(invalid(
                        id,
                        format!(
                            "node '{name}' input '{port}': ch()/out() paths must stay inside the definition"
                        ),
                    ));
                }
            }
            nodes.push(NodeSnapshot {
                id: n,
                name: name.to_owned(),
                type_id: type_id.to_owned(),
                inputs: self
                    .input_values(n)
                    .map(|(p, v)| (p.to_owned(), v.clone()))
                    .collect(),
                expressions,
                animations: self
                    .animations(n)
                    .map(|(p, a)| (p.to_owned(), a.clone()))
                    .collect(),
                parent,
                port_type: self.port_type(n),
                charm: self.charm(n).cloned(),
                def: None,
            });
        }
        let links = self
            .links()
            .filter(|l| members.contains(&l.from.node) && members.contains(&l.to.node))
            .filter_map(|l| self.named(l))
            .collect();
        let mut meta = self
            .charm(subnet)
            .cloned()
            .unwrap_or_else(|| CharmMeta::new(self.node_name(subnet).unwrap_or(id)));
        meta.version = None;
        Ok(CharmDef {
            id: id.to_owned(),
            version: version.to_owned(),
            meta,
            nodes,
            links,
        })
    }

    /// The nodes the expression on input `port` of `node` reads.
    fn expression_targets(&self, node: NodeId, port: &str) -> Vec<NodeId> {
        let Ok((index, _)) = self.input_port(node, port) else {
            return Vec::new();
        };
        self.node(node)
            .and_then(|n| n.exprs.get(index))
            .and_then(Option::as_ref)
            .map(|e| e.targets.iter().map(|t| t.node).collect())
            .unwrap_or_default()
    }

    /// Add a Charm instance `id` called `name` of definition `def` to the
    /// network of `parent` (`None`: the root). Its contents are the
    /// definition's, with ids from [`instance_node_id`]; its interface
    /// inputs start with the definition's defaults.
    ///
    /// # Errors
    /// [`GraphError::UnknownDefinition`], the errors of
    /// [`Graph::add_node_in`] for the instance node (the node type
    /// `core.charm@1` must be registered), and
    /// [`GraphError::DuplicateId`] when a derived id is taken.
    pub fn instantiate(
        &mut self,
        parent: Option<NodeId>,
        id: NodeId,
        name: &str,
        def: &str,
    ) -> Result<NodeId, GraphError> {
        let template = self
            .defs
            .get(def)
            .cloned()
            .ok_or_else(|| GraphError::UnknownDefinition(def.to_owned()))?;
        self.check_network_editable(parent)?;
        if self.contains(id) {
            return Err(GraphError::DuplicateId(id));
        }
        if let Some(taken) = template
            .nodes
            .iter()
            .map(|n| instance_node_id(id, n.id))
            .find(|&d| self.contains(d))
        {
            return Err(GraphError::DuplicateId(taken));
        }
        self.insert_node(parent, id, CHARM_TYPE, name, None)?;
        if let Err(err) = self.sync(id, &template, None, &|l| instance_node_id(id, l)) {
            let _ = self.remove_subtree(id);
            return Err(err);
        }
        self.instances.insert(id, def.to_owned());
        if let Some(slot) = self.charm_slot(id) {
            *slot = Some(template.instance_meta());
        }
        Ok(id)
    }

    /// Turn Charm instance `instance` into a plain subnet (`core.subnet@1`)
    /// with the same contents, ids and metadata, which can then be edited
    /// freely. Returns the definition it used. Cooked results stay valid.
    ///
    /// # Errors
    /// [`GraphError::NotAnInstance`].
    pub fn detach(&mut self, instance: NodeId) -> Result<String, GraphError> {
        let def = self
            .instances
            .get(&instance)
            .cloned()
            .ok_or(GraphError::NotAnInstance(instance))?;
        self.set_container_type(instance, SUBNET_TYPE)?;
        self.instances.remove(&instance);
        Ok(def)
    }

    /// The inverse of [`Graph::detach`]: turn plain subnet `subnet`, whose
    /// contents and metadata are exactly those of an instance of `def`
    /// with id `subnet` (as `detach` leaves them; interface input sources
    /// may differ), back into an instance.
    ///
    /// # Errors
    /// [`GraphError::UnknownDefinition`], [`GraphError::NotASubnet`],
    /// [`GraphError::InInstance`], or [`GraphError::InvalidDefinition`]
    /// when the contents differ from the definition.
    pub fn attach(&mut self, subnet: NodeId, def: &str) -> Result<(), GraphError> {
        let template = self
            .defs
            .get(def)
            .ok_or_else(|| GraphError::UnknownDefinition(def.to_owned()))?;
        if !self.contains(subnet) {
            return Err(GraphError::UnknownNode(subnet));
        }
        if self.node_type(subnet) != Some(SUBNET_TYPE) {
            return Err(GraphError::NotASubnet(subnet));
        }
        self.check_structure_editable(subnet)?;
        if !self.matches_definition(subnet, template) {
            return Err(invalid(
                def,
                format!("the contents of subnet {subnet} differ from the definition"),
            ));
        }
        self.set_container_type(subnet, CHARM_TYPE)?;
        self.instances.insert(subnet, def.to_owned());
        Ok(())
    }

    /// Change a port-less container node between `core.subnet@1` and
    /// `core.charm@1` in place.
    fn set_container_type(&mut self, id: NodeId, type_id: &str) -> Result<(), GraphError> {
        let imp = self
            .registry()
            .get(type_id)
            .cloned()
            .ok_or_else(|| GraphError::UnknownNodeType(type_id.to_owned()))?;
        let node = self.node_mut(id).ok_or(GraphError::UnknownNode(id))?;
        node.type_id = type_id.to_owned();
        node.imp = imp;
        Ok(())
    }

    /// Whether the contents of `root` are those of an instance `root` of
    /// `def` (interface input sources aside).
    fn matches_definition(&self, root: NodeId, def: &CharmDef) -> bool {
        if self.charm(root) != Some(&def.instance_meta()) {
            return false;
        }
        let members: BTreeSet<NodeId> = self.subtree(root).into_iter().collect();
        if members.len() != def.nodes.len() + 1 {
            return false;
        }
        let map = |l: NodeId| instance_node_id(root, l);
        for t in &def.nodes {
            let id = map(t.id);
            let parent = Some(t.parent.map_or(root, map));
            if !members.contains(&id)
                || self.node_name(id) != Some(t.name.as_str())
                || self.node_type(id) != Some(t.type_id.as_str())
                || self.parent(id) != parent
                || self.port_type(id) != t.port_type
                || self.charm(id) != t.charm.as_ref()
            {
                return false;
            }
            if parent == Some(root) && t.type_id == INPUT_TYPE {
                continue;
            }
            let inputs: Vec<(String, Value)> = self
                .input_values(id)
                .map(|(p, v)| (p.to_owned(), v.clone()))
                .collect();
            let exprs: Vec<(String, String)> = self
                .expressions(id)
                .map(|(p, s)| (p.to_owned(), s.to_owned()))
                .collect();
            let anims: Vec<(String, AnimChannel)> = self
                .animations(id)
                .map(|(p, a)| (p.to_owned(), a.clone()))
                .collect();
            if inputs != t.inputs || exprs != t.expressions || anims != t.animations {
                return false;
            }
        }
        let internal: BTreeSet<(NodeId, String, NodeId, String)> = self
            .links()
            .filter(|l| members.contains(&l.from.node) && members.contains(&l.to.node))
            .filter_map(|l| self.named(l))
            .map(|l| (l.from, l.from_port, l.to, l.to_port))
            .collect();
        let wanted: BTreeSet<(NodeId, String, NodeId, String)> = def
            .links
            .iter()
            .map(|l| {
                (
                    map(l.from),
                    l.from_port.clone(),
                    map(l.to),
                    l.to_port.clone(),
                )
            })
            .collect();
        internal == wanted
    }

    /// Replace definition `def.id` with `def` (typically a new version)
    /// and re-synchronize every instance (see the [module docs](self)):
    /// followed interface sources take the new defaults, overrides stay,
    /// outside readers keep reading the same nodes.
    ///
    /// # Errors
    /// [`GraphError::UnknownDefinition`], [`GraphError::InvalidDefinition`],
    /// [`GraphError::Referenced`] when an expression outside an instance
    /// reads a node the new definition removes, or any edit error while
    /// re-synchronizing (for example [`GraphError::WouldCycle`] with the
    /// links around an instance). The graph is unchanged on error.
    pub fn update_definition(&mut self, def: CharmDef) -> Result<DefinitionUpdate, GraphError> {
        let old = self
            .defs
            .get(&def.id)
            .cloned()
            .ok_or_else(|| GraphError::UnknownDefinition(def.id.clone()))?;
        let new = self.validate_definition(&def)?;
        let backup = self.duplicate();
        match self.update_instances(&old, new) {
            Ok(report) => Ok(report),
            Err(err) => {
                *self = backup;
                Err(err)
            }
        }
    }

    fn update_instances(
        &mut self,
        old: &CharmDef,
        new: CharmDef,
    ) -> Result<DefinitionUpdate, GraphError> {
        let instances = self.instances_of(&new.id);
        let mut report = DefinitionUpdate {
            instances: instances.clone(),
            ..DefinitionUpdate::default()
        };
        for &inst in &instances {
            self.instances.remove(&inst);
            let mut unlinked = Vec::new();
            let rewritten = self.sync_into(
                inst,
                &new,
                Some(old),
                &|l| instance_node_id(inst, l),
                &mut unlinked,
            )?;
            report.rewritten.extend(rewritten);
            report.unlinked.extend(unlinked);
            self.instances.insert(inst, new.id.clone());
            if let Some(slot) = self.charm_slot(inst) {
                *slot = Some(new.instance_meta());
            }
        }
        self.defs.insert(new.id.clone(), new);
        Ok(report)
    }

    /// Convert plain subnet `subnet` into definition `id` at `version`
    /// and an instance of it (same id, name and place). The subnet's
    /// contents get derived ids; links into its interface and expressions
    /// reading inside it are carried over, and expressions on its
    /// interface inputs that read outside it become per-instance
    /// overrides. Data flow is unchanged.
    ///
    /// # Errors
    /// The errors of [`Graph::definition_from_subnet`],
    /// [`Graph::add_definition`] and [`Graph::instantiate`]. The graph is
    /// unchanged on error.
    pub fn make_definition(
        &mut self,
        subnet: NodeId,
        id: &str,
        version: &str,
    ) -> Result<(), GraphError> {
        self.check_structure_editable(subnet)?;
        let def = self.capture(subnet, id, version, true)?;
        let backup = self.duplicate();
        if let Err(err) = self.convert_subnet(subnet, def) {
            *self = backup;
            return Err(err);
        }
        Ok(())
    }

    fn convert_subnet(&mut self, subnet: NodeId, def: CharmDef) -> Result<(), GraphError> {
        let plan = self.conversion_plan(subnet, &def)?;
        for (node, port, _) in &plan.readers {
            self.clear_expression(*node, port)?;
        }
        self.remove_node(subnet)?;
        let def_id = def.id.clone();
        self.add_definition(def)?;
        self.instantiate(plan.parent, subnet, &plan.name, &def_id)?;
        for (node, source) in &plan.overrides {
            self.set_expression(*node, INTERFACE_PORT, source)?;
        }
        for l in &plan.links {
            self.link(l.from, &l.from_port, l.to, &l.to_port)?;
        }
        for (node, port, source) in &plan.readers {
            self.set_expression(*node, port, source)?;
        }
        Ok(())
    }

    /// What converting `subnet` into an instance of `def` (captured from
    /// it) must carry over, with ids already mapped to the instance's.
    /// Used by [`Graph::make_definition`] and by the command bus.
    ///
    /// # Errors
    /// [`GraphError::UnknownNode`].
    pub fn conversion_plan(
        &self,
        subnet: NodeId,
        def: &CharmDef,
    ) -> Result<ConversionPlan, GraphError> {
        let name = self
            .node_name(subnet)
            .ok_or(GraphError::UnknownNode(subnet))?
            .to_owned();
        let members: BTreeSet<NodeId> = self.subtree(subnet).into_iter().collect();
        let map = |n: NodeId| {
            if n != subnet && members.contains(&n) {
                instance_node_id(subnet, n)
            } else {
                n
            }
        };
        let mut links = Vec::new();
        for l in self.links() {
            let (a, b) = (members.contains(&l.from.node), members.contains(&l.to.node));
            if a != b {
                if let Some(mut named) = self.named(l) {
                    named.from = map(named.from);
                    named.to = map(named.to);
                    links.push(named);
                }
            }
        }
        let mut readers = Vec::new();
        for &n in &members {
            for (reader, port) in self.referenced_by(n) {
                if members.contains(&reader) {
                    continue;
                }
                if let Ok(Some(source)) = self.expression(reader, &port) {
                    let entry = (reader, port.clone(), source.to_owned());
                    if !readers.contains(&entry) {
                        readers.push(entry);
                    }
                }
            }
        }
        let mut overrides = Vec::new();
        for t in def.interface_inputs() {
            let kept = t.expressions.iter().any(|(p, _)| p == INTERFACE_PORT);
            if let (false, Ok(Some(source))) = (kept, self.expression(t.id, INTERFACE_PORT)) {
                overrides.push((map(t.id), source.to_owned()));
            }
        }
        Ok(ConversionPlan {
            parent: self.parent(subnet),
            name,
            links,
            readers,
            overrides,
        })
    }

    /// Make the contents of container `root` those of `new` (see the
    /// module docs), with node ids `map(local id)`. `prev` is the
    /// definition the contents were made from, if any (for interface
    /// overrides). Returns the re-rendered outside expressions.
    pub(crate) fn sync(
        &mut self,
        root: NodeId,
        new: &CharmDef,
        prev: Option<&CharmDef>,
        map: &dyn Fn(NodeId) -> NodeId,
    ) -> Result<Vec<(NodeId, String, String)>, GraphError> {
        let mut unlinked = Vec::new();
        self.sync_into(root, new, prev, map, &mut unlinked)
    }

    #[allow(clippy::too_many_lines)]
    fn sync_into(
        &mut self,
        root: NodeId,
        new: &CharmDef,
        prev: Option<&CharmDef>,
        map: &dyn Fn(NodeId) -> NodeId,
        unlinked: &mut Vec<NamedLink>,
    ) -> Result<Vec<(NodeId, String, String)>, GraphError> {
        // The wanted nodes, parents first.
        let by_local: BTreeMap<NodeId, &NodeSnapshot> =
            new.nodes.iter().map(|n| (n.id, n)).collect();
        let mut order: Vec<(usize, &NodeSnapshot)> = new
            .nodes
            .iter()
            .map(|n| (depth(&by_local, n).unwrap_or(0), n))
            .collect();
        order.sort_by_key(|(d, n)| (*d, n.id));
        let wanted: BTreeMap<NodeId, (NodeId, &NodeSnapshot)> = order
            .iter()
            .map(|(_, n)| (map(n.id), (n.parent.map_or(root, map), *n)))
            .collect();
        let interface = |t: &NodeSnapshot| t.parent.is_none() && t.type_id == INPUT_TYPE;

        let current: Vec<NodeId> = self
            .subtree(root)
            .into_iter()
            .filter(|&n| n != root)
            .collect();
        let members: BTreeSet<NodeId> = current.iter().copied().chain([root]).collect();
        let fits = |g: &Graph, id: NodeId| {
            wanted.get(&id).is_some_and(|(parent, t)| {
                g.node_type(id) == Some(t.type_id.as_str())
                    && g.port_type(id) == t.port_type
                    && g.parent(id) == Some(*parent)
            })
        };
        let removed: BTreeSet<NodeId> = current
            .iter()
            .copied()
            .filter(|&id| !fits(self, id))
            .collect();
        // A kept node whose ancestor is removed goes too.
        let gone = |g: &Graph, id: NodeId| {
            let mut n = Some(id);
            while let Some(x) = n {
                if x == root {
                    return false;
                }
                if removed.contains(&x) {
                    return true;
                }
                n = g.parent(x);
            }
            false
        };

        // Interface inputs that stay: their current sources, and which
        // kinds are overrides.
        let mut kept_sources: BTreeMap<NodeId, (Sources, [bool; 3])> = BTreeMap::new();
        for (&id, (_, t)) in &wanted {
            if !interface(t) || !members.contains(&id) || gone(self, id) {
                continue;
            }
            let current = Sources {
                value: self.input_value(id, INTERFACE_PORT)?.cloned(),
                expr: self.expression(id, INTERFACE_PORT)?.map(str::to_owned),
                anim: self.animation(id, INTERFACE_PORT)?.cloned(),
            };
            let old = prev.and_then(|p| p.node(t.id)).map(template_sources);
            let keep = match &old {
                Some(old) => [
                    current.value != old.value,
                    current.expr != old.expr,
                    current.anim != old.anim,
                ],
                None => [false; 3],
            };
            kept_sources.insert(id, (current, keep));
        }

        // Expressions to restore afterwards: outside readers of the
        // contents, and kept expression overrides.
        let mut readers: Vec<Reader> = Vec::new();
        let mut seen = BTreeSet::new();
        for &n in &current {
            for (reader, port) in self.referenced_by(n) {
                if !members.contains(&reader) && seen.insert((reader, port.clone())) {
                    readers.push(self.reader(reader, &port)?);
                }
            }
        }
        for (&id, (_, keep)) in &kept_sources {
            if keep[1] && self.expression(id, INTERFACE_PORT)?.is_some() {
                readers.push(self.reader(id, INTERFACE_PORT)?);
            }
        }
        for r in &readers {
            for (_, target) in &r.reads {
                if let Some(t) = target {
                    if members.contains(t) && gone(self, *t) {
                        return Err(GraphError::Referenced {
                            node: *t,
                            by: r.node,
                            port: r.port.clone(),
                        });
                    }
                }
            }
        }

        // 1. Clear every expression of the contents and of the readers.
        for &n in &current {
            let ports: Vec<String> = self.expressions(n).map(|(p, _)| p.to_owned()).collect();
            for port in ports {
                self.drop_expression(n, &port)?;
            }
        }
        for r in &readers {
            self.drop_expression(r.node, &r.port)?;
        }
        // 2. Unlink internal links that are not wanted.
        let wanted_links: BTreeSet<(NodeId, String, NodeId, String)> = new
            .links
            .iter()
            .map(|l| {
                (
                    map(l.from),
                    l.from_port.clone(),
                    map(l.to),
                    l.to_port.clone(),
                )
            })
            .collect();
        let internal: Vec<NamedLink> = self
            .links()
            .filter(|l| members.contains(&l.from.node) && members.contains(&l.to.node))
            .filter_map(|l| self.named(l))
            .collect();
        for l in &internal {
            let key = (l.from, l.from_port.clone(), l.to, l.to_port.clone());
            if !wanted_links.contains(&key) {
                self.unlink(l.to, &l.to_port)?;
            }
        }
        // 3. Remove what does not fit, reporting outside links that go.
        for &id in &current {
            if !removed.contains(&id) || !self.contains(id) {
                continue;
            }
            let sub: BTreeSet<NodeId> = self.subtree(id).into_iter().collect();
            let mut links = Vec::new();
            for &n in &sub {
                links.extend(self.incoming_links(n));
                links.extend(self.outgoing_links(n));
            }
            for l in links {
                if !members.contains(&l.from) || !members.contains(&l.to) {
                    unlinked.push(l);
                }
            }
            self.remove_subtree(id)?;
        }
        // 4. Rename what stays.
        let renames: Vec<(NodeId, Option<NodeId>, String, String)> = wanted
            .iter()
            .filter(|(id, _)| self.contains(**id))
            .filter_map(|(&id, (_, t))| {
                let name = self.node_name(id)?;
                (name != t.name).then(|| (id, self.parent(id), name.to_owned(), t.name.clone()))
            })
            .collect();
        for (_, parent, old, _) in &renames {
            self.remove_name(*parent, old);
        }
        for (id, parent, _, name) in &renames {
            self.insert_name(*parent, name, *id);
            if let Some(n) = self.node_mut(*id) {
                n.name.clone_from(name);
            }
        }
        // 5. Add what is missing, parents first.
        for (_, t) in &order {
            let id = map(t.id);
            if !self.contains(id) {
                let parent = t.parent.map_or(root, map);
                self.insert_node(Some(parent), id, &t.type_id, &t.name, t.port_type)?;
            }
            if t.type_id == SUBNET_TYPE && self.charm(id) != t.charm.as_ref() {
                if let Some(slot) = self.charm_slot(id) {
                    slot.clone_from(&t.charm);
                }
            }
        }
        // 6. Static values and animation channels.
        for (_, t) in &order {
            let id = map(t.id);
            let ports: Vec<&str> = self
                .node_spec(id)
                .map(|s| s.inputs.iter().map(|p| p.name).collect())
                .unwrap_or_default();
            let kept = kept_sources.get(&id).filter(|_| interface(t));
            for port in ports {
                let mut value = t.inputs.iter().find(|(p, _)| p == port).map(|(_, v)| v);
                let mut anim = t.animations.iter().find(|(p, _)| p == port).map(|(_, a)| a);
                if let Some((sources, keep)) = kept {
                    if keep[0] {
                        value = sources.value.as_ref();
                    }
                    if keep[2] {
                        anim = sources.anim.as_ref();
                    }
                }
                if self.input_value(id, port)? != value {
                    match value {
                        Some(v) => self.set_input(id, port, v.clone())?,
                        None => self.clear_input(id, port)?,
                    }
                }
                if self.animation(id, port)? != anim {
                    match anim {
                        Some(a) => self.set_animation(id, port, a.clone())?,
                        None => self.clear_animation(id, port)?,
                    }
                }
            }
        }
        // 7. Links.
        for (from, from_port, to, to_port) in &wanted_links {
            if self
                .link_source(*to, to_port)?
                .map(|(n, p)| (n, p.to_owned()))
                != Some((*from, from_port.clone()))
            {
                self.link(*from, from_port, *to, to_port)?;
            }
        }
        // 8. Expressions of the definition.
        for (_, t) in &order {
            let id = map(t.id);
            let kept = kept_sources.get(&id).is_some_and(|(_, keep)| keep[1]);
            for (port, source) in &t.expressions {
                if kept && port == INTERFACE_PORT {
                    continue;
                }
                self.set_expression(id, port, source)?;
            }
        }
        // 9. Readers, re-rendered where a path would read something else.
        let mut rewritten = Vec::new();
        for r in readers {
            if !self.contains(r.node) {
                continue;
            }
            let net = self.parent(r.node);
            let mut out = r.source.clone();
            for (cref, target) in r.reads.iter().rev() {
                let Some(target) = target else {
                    continue;
                };
                let now = resolve_path(self, net, &cref.path).map(|x| x.node);
                if now != Some(*target) {
                    let port = cref.path.rsplit_once('/').map_or("", |(_, p)| p);
                    let path = render_path(self, net, *target, port);
                    out.replace_range(cref.span.start..cref.span.end, &expr::quote(&path));
                }
            }
            self.set_expression(r.node, &r.port, &out)?;
            if out != r.source {
                rewritten.push((r.node, r.port, r.source));
            }
        }
        Ok(rewritten)
    }

    /// An expression with what each of its paths reads now.
    fn reader(&self, node: NodeId, port: &str) -> Result<Reader, GraphError> {
        let source = self
            .expression(node, port)?
            .map(str::to_owned)
            .unwrap_or_default();
        let net = self.parent(node);
        let reads = expr::channel_refs(&source)
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                let target = resolve_path(self, net, &r.path).map(|x| x.node);
                (r, target)
            })
            .collect();
        Ok(Reader {
            node,
            port: port.to_owned(),
            source,
            reads,
        })
    }

    /// Remove an expression without the instance rules (no restoring of
    /// defaults).
    fn drop_expression(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        if self.node(node).is_some_and(|n| n.exprs[index].is_some()) {
            self.unindex_expression(node, index);
            if let Some(n) = self.node_mut(node) {
                n.exprs[index] = None;
            }
            self.structure_changed();
            self.bump(node);
        }
        Ok(())
    }
}

/// What [`Graph::conversion_plan`] found: what to carry over when a subnet
/// becomes an instance of a definition captured from it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConversionPlan {
    /// The subnet's network.
    pub parent: Option<NodeId>,
    /// The subnet's name.
    pub name: String,
    /// Links between the subnet's contents and the outside, with the
    /// contents' ids mapped to the instance's.
    pub links: Vec<NamedLink>,
    /// Expressions outside the subnet that read inside it, as `(node,
    /// port, source)`.
    pub readers: Vec<(NodeId, String, String)>,
    /// Expressions of interface inputs that read outside the subnet (not
    /// part of the definition), as `(instance node, source)`.
    pub overrides: Vec<(NodeId, String)>,
}

/// Depth of `node` in its definition (0 for the definition's network);
/// `None` when a parent is missing, not a subnet, or the parents cycle.
fn depth(by_id: &BTreeMap<NodeId, &NodeSnapshot>, node: &NodeSnapshot) -> Option<usize> {
    let mut depth = 0;
    let mut current = node.parent;
    while let Some(p) = current {
        let parent = by_id.get(&p)?;
        if parent.type_id != SUBNET_TYPE || depth > by_id.len() {
            return None;
        }
        depth += 1;
        current = parent.parent;
    }
    Some(depth)
}
