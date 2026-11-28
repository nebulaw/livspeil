// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Input sources besides links and static values: expressions and
//! animation channels (kernel spec, section 6).
//!
//! # Priority
//!
//! An input's value comes from, in order: its link, its expression, its
//! animation channel, its static value, the spec default (then the type's
//! zero value). Setting a higher-priority source does not remove the lower
//! ones; clearing it makes them visible again.
//!
//! # `ch()` paths
//!
//! A path names an input of a node; the port is the text after the last
//! `/`, the text before it is a node path (M0.7, see the crate docs on
//! networks):
//!
//! * `"node/port"`: a node in the same network as the node that holds the
//!   expression (a sibling); `"sub/inner/port"` descends into subnet `sub`.
//! * `"../x/port"`: node `x` of the *parent* network (each leading `../`
//!   goes up one network; at the root it stays at the root, so in the root
//!   network `"../x/port"` and `"x/port"` are the same, as before M0.7).
//! * `"/a/b/port"`: absolute, from the root network.
//!
//! Compatibility with flat graphs: in the root network a relative path is
//! first read the pre-M0.7 way, removing one leading `../` and taking all
//! the rest before the last `/` as a single node name, so root nodes whose
//! names contain `/` stay reachable (`"a/b/port"` reads the root node
//! called `a/b` when there is one). Only when no such node exists is the
//! path walked through subnets. Paths must name a node, so an expression
//! cannot read an input of its own node (that would be a self-dependency
//! and is rejected as a cycle).
//!
//! `ch()` reads the *resolved* value of the input (after links,
//! expressions and animation), with the input's port type.
//!
//! # `out()` paths (M0.11, decision D48)
//!
//! `out("node/port")` reads the computed value of *output* `port` of a
//! node instead, with the output's port type, so a formula can replace a
//! chain of math nodes: `smoothstep(5, 8, out("time_of_day/hour"))`. Paths
//! use exactly the syntax and resolution rules of `ch()` above; only the
//! port name is looked up among the node's outputs. The value read is the
//! node's last good output, so a node that fails after producing a good
//! output keeps feeding its readers (as through a link), and one that
//! never produced an output gives its readers
//! [`crate::CookError::Upstream`].
//!
//! # Dependencies
//!
//! An expression that reads `ch("m/p")` or `out("m/p")` on node `n` makes
//! `n` depend on node `m`, exactly like a link from `m` to `n`: such an
//! edge may not close a cycle ([`GraphError::WouldCycle`]), the cooker
//! evaluates `m` first, and `n` re-cooks when the resolved inputs of `m`
//! change (`ch()`) or when the outputs of `m` change (`out()`; early cutoff
//! applies: a re-cooked `m` with equal outputs does not re-cook `n`). Both
//! kinds are indexed the same way (one edge per reading input and read
//! node), so invalidation, evaluation order and time dependence need
//! nothing new. [`Graph::expression_refs`] lists every reference, for
//! example to draw them as dashed wires.
//!
//! # Renames and deletes
//!
//! Expression sources stay the single source of truth, and they never
//! dangle:
//!
//! * Renaming a node rewrites every `ch()` and `out()` path literal that
//!   refers to it
//!   or passes through it (renaming a subnet rewrites the paths into it),
//!   in every expression of the graph: only the path segment naming the
//!   node changes, so relative paths stay relative and a `../` prefix is
//!   kept; the literal is re-rendered with
//!   [`livspeil_params::expr::quote`]. Renaming back rewrites them back.
//! * Moving a node to another network ([`Graph::move_node`]) re-renders
//!   the paths that would otherwise read something else, as the shortest
//!   relative path from the reader's network.
//! * Removing a node that another node's expression reads is refused with
//!   [`GraphError::Referenced`]; clear or change those expressions first.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::Range;

use livspeil_params::AnimChannel;
use livspeil_params::expr::{self, Channel, Env, Program, RefKind};

use crate::graph::{Graph, GraphError, NodeId};
use crate::types::SourceKind;

/// A port of a node, by port index: the target of a `ch()` reference (an
/// input, [`RefKind::Input`]) or of an `out()` reference (an output,
/// [`RefKind::Output`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct InputRef {
    pub(crate) node: NodeId,
    /// The node's slot (see [`Graph`]); fixed while the node exists.
    pub(crate) slot: usize,
    /// Index into the node's inputs or outputs, depending on `kind`.
    pub(crate) port: usize,
    pub(crate) kind: RefKind,
}

/// One `ch()` or `out()` reference of an expression: input `reader_port`
/// of node `reader` reads port `target_port` of node `target` (an input
/// for [`RefKind::Input`], an output for [`RefKind::Output`]). See
/// [`Graph::expression_refs`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExprRef {
    /// The node holding the expression.
    pub reader: NodeId,
    /// The input the expression drives.
    pub reader_port: String,
    /// `ch()` reads an input, `out()` an output.
    pub kind: RefKind,
    /// The node read.
    pub target: NodeId,
    /// The port read: an input or output name, depending on `kind`.
    pub target_port: String,
}

/// A compiled expression attached to an input.
#[derive(Debug, Clone)]
pub(crate) struct Expression {
    pub(crate) source: String,
    pub(crate) program: Program,
    /// Slot `i` of the program reads port `targets[i]`.
    pub(crate) targets: Vec<InputRef>,
}

/// Split a `ch()` path the pre-M0.7 way into node name and port name.
pub(crate) fn split_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("../").unwrap_or(path);
    let (node, port) = rest.rsplit_once('/')?;
    (!node.is_empty() && !port.is_empty()).then_some((node, port))
}

/// How a `ch()` path resolved to a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Resolved {
    pub(crate) node: NodeId,
    /// The port name (text after the last `/`).
    pub(crate) port: String,
    /// Byte ranges of the path's name segments and the node each named.
    pub(crate) segments: Vec<(Range<usize>, NodeId)>,
    /// Resolved the pre-M0.7 way (a single literal name in the root).
    pub(crate) legacy: bool,
}

/// Resolve the node part of `path` for a reader in network `network`
/// (see the module docs).
pub(crate) fn resolve_path(graph: &Graph, network: Option<NodeId>, path: &str) -> Option<Resolved> {
    let (node_part, port) = path.rsplit_once('/')?;
    if port.is_empty() {
        return None;
    }
    let absolute = node_part.starts_with('/');
    if network.is_none()
        && !absolute
        && let Some((name, port)) = split_path(path)
        && let Some(node) = graph.find_in(None, name)
    {
        return Some(Resolved {
            node,
            port: port.to_owned(),
            segments: Vec::new(),
            legacy: true,
        });
    }
    let (mut current, body, mut offset) = match node_part.strip_prefix('/') {
        Some(rest) => (None, rest, 1),
        None => (network, node_part, 0),
    };
    let mut found: Option<NodeId> = None;
    let mut segments = Vec::new();
    for segment in body.split('/') {
        let range = offset..offset + segment.len();
        offset += segment.len() + 1;
        if segment == ".." && found.is_none() && !absolute {
            current = current.and_then(|s| graph.parent(s));
            continue;
        }
        if segment.is_empty() {
            return None;
        }
        if let Some(prev) = found {
            if !graph.is_subnet(prev) {
                return None;
            }
            current = Some(prev);
        }
        let node = graph.find_in(current, segment)?;
        segments.push((range, node));
        found = Some(node);
    }
    Some(Resolved {
        node: found?,
        port: port.to_owned(),
        segments,
        legacy: false,
    })
}

/// The shortest relative path from network `from` to input `port` of
/// `target`: `../` per level up, then the names down.
pub(crate) fn render_path(
    graph: &Graph,
    from: Option<NodeId>,
    target: NodeId,
    port: &str,
) -> String {
    let from_chain = graph.ancestors(from);
    let to_chain = graph.ancestors(graph.parent(target));
    let common = from_chain
        .iter()
        .zip(&to_chain)
        .take_while(|(a, b)| a == b)
        .count();
    let mut path = "../".repeat(from_chain.len() - common);
    for &n in to_chain.iter().skip(common).chain(std::iter::once(&target)) {
        path.push_str(graph.node_name(n).unwrap_or_default());
        path.push('/');
    }
    path.push_str(port);
    path
}

/// Resolves `ch()` and `out()` paths against a graph, assigning one slot
/// per distinct port.
struct GraphEnv<'a> {
    graph: &'a Graph,
    network: Option<NodeId>,
    targets: RefCell<Vec<InputRef>>,
}

impl GraphEnv<'_> {
    fn resolve_kind(&self, path: &str, kind: RefKind) -> Option<Channel> {
        let resolved = resolve_path(self.graph, self.network, path)?;
        let (node, port_name) = (resolved.node, resolved.port.as_str());
        let spec = self.graph.node_spec(node)?;
        let (port, ty) = match kind {
            RefKind::Input => {
                let port = spec.input_index(port_name)?;
                (port, spec.inputs.get(port)?.ty)
            }
            RefKind::Output => {
                let port = spec.output_index(port_name)?;
                (port, spec.outputs.get(port)?.ty)
            }
        };
        let slot = self.graph.slot_of(node)?;
        let target = InputRef {
            node,
            slot,
            port,
            kind,
        };
        let mut targets = self.targets.borrow_mut();
        let slot = match targets.iter().position(|t| *t == target) {
            Some(slot) => slot,
            None => {
                targets.push(target);
                targets.len() - 1
            }
        };
        Some(Channel { slot, ty })
    }
}

impl Env for GraphEnv<'_> {
    fn resolve(&self, path: &str) -> Option<Channel> {
        self.resolve_kind(path, RefKind::Input)
    }

    fn resolve_output(&self, path: &str) -> Option<Channel> {
        self.resolve_kind(path, RefKind::Output)
    }
}

/// Compile `source` against the current names of `graph`, for a reader in
/// network `network`.
pub(crate) fn compile(
    graph: &Graph,
    network: Option<NodeId>,
    source: &str,
) -> Result<Expression, GraphError> {
    let env = GraphEnv {
        graph,
        network,
        targets: RefCell::new(Vec::new()),
    };
    let program = expr::compile(source, &env).map_err(|e| GraphError::Expression(Box::new(e)))?;
    let targets = env.targets.borrow();
    for target in targets.iter() {
        graph.check_poly_read(target)?;
    }
    drop(targets);
    Ok(Expression {
        source: source.to_owned(),
        program,
        targets: env.targets.into_inner(),
    })
}

/// `source` (of an expression on a node in network `network`) with every
/// `ch()` and `out()` path that names node `renamed` changed to name it `new`, judged
/// against the current names. Returns `None` when nothing changes.
pub(crate) fn rewrite(
    graph: &Graph,
    network: Option<NodeId>,
    source: &str,
    renamed: NodeId,
    new: &str,
) -> Option<String> {
    let refs = expr::channel_refs(source).ok()?;
    let mut out = source.to_owned();
    let mut changed = false;
    // Back to front, so earlier spans stay valid.
    for r in refs.iter().rev() {
        let Some(resolved) = resolve_path(graph, network, &r.path) else {
            continue;
        };
        let path = if resolved.legacy {
            if resolved.node != renamed {
                continue;
            }
            let prefix = if r.path.starts_with("../") || new.starts_with("../") {
                "../"
            } else {
                ""
            };
            format!("{prefix}{new}/{}", resolved.port)
        } else {
            let mut path = r.path.clone();
            let mut hit = false;
            for (range, node) in resolved.segments.iter().rev() {
                if *node == renamed {
                    path.replace_range(range.clone(), new);
                    hit = true;
                }
            }
            if !hit {
                continue;
            }
            path
        };
        out.replace_range(r.span.start..r.span.end, &expr::quote(&path));
        changed = true;
    }
    changed.then_some(out)
}

/// A pending change of an expression's source: `(node, input index, new
/// source)`.
pub(crate) type Rewrite = (NodeId, usize, String);

impl Graph {
    /// Drive input `port` of `node` with an expression (see the
    /// [`livspeil_params::expr`] language). A link, if present, still wins.
    /// The result type must fit the input like a link would (`Int` results
    /// are accepted for `Float` inputs). Replaces a previous expression.
    ///
    /// Priority of an input's sources: link, expression, animation channel,
    /// static value, spec default.
    ///
    /// `ch("path")` reads an input of another node and `out("path")` an
    /// output (see the [module docs](self) of `sources`). Paths are
    /// relative to the network holding `node`:
    /// `"x/port"` names input `port` of sibling `x` (`"sub/x/port"` looks
    /// inside subnet `sub`), `"../x/port"` names node `x` of the parent
    /// network (in the root network, the root itself, as before subnets
    /// existed), and `"/a/x/port"` is absolute. In the root network a path
    /// is first read the pre-M0.7 way (one leading `../` removed, the rest
    /// before the last `/` as a single name). `ch()` reads the input's
    /// *resolved* value (after links, expressions and animation), in any
    /// network; `out()` reads the output's last good value. A node cannot
    /// read its own inputs or outputs.
    ///
    /// Each `ch()` or `out()` reference makes `node` depend on the node it
    /// reads, like a link: it must not close a cycle, and `node` re-cooks
    /// when the read node's resolved inputs (`ch()`) or outputs (`out()`)
    /// change. If an `out()` target has never produced an output, `node`
    /// fails with [`crate::CookError::Upstream`]. Expressions reading `$F`, `$T` or
    /// `$DT` (and animation channels) make the node re-cook every frame.
    /// Evaluation errors become [`crate::CookError::Expression`] on the
    /// node, which keeps its last good output.
    ///
    /// Sources never dangle: [`Graph::rename`] rewrites every path literal
    /// naming the renamed node or passing through it (keeping a `../`
    /// prefix), [`Graph::move_node`] re-renders paths a move would break,
    /// and
    /// [`Graph::remove_node`] refuses to remove a node that is read
    /// ([`GraphError::Referenced`]).
    ///
    /// # Errors
    /// Fails for an unknown node or port, a syntax, name or type error
    /// ([`GraphError::Expression`], including unknown `ch()`/`out()` paths
    /// and ports of types expressions cannot use), a result type the input
    /// does not accept ([`GraphError::TypeMismatch`]) or a `ch()`/`out()`
    /// reference that would create a cycle
    /// ([`GraphError::WouldCycle`]).
    pub fn set_expression(
        &mut self,
        node: NodeId,
        port: &str,
        source: &str,
    ) -> Result<(), GraphError> {
        let (index, ty) = self.input_port(node, port)?;
        self.check_sources_editable(node)?;
        let compiled = compile(self, self.parent(node), source)?;
        let found = compiled.program.result_type();
        let poly = self.check_poly_source(node, index, SourceKind::Expr, found)?;
        if poly.is_none() && !ty.accepts(found) {
            return Err(GraphError::TypeMismatch {
                expected: ty,
                found,
            });
        }
        if compiled
            .targets
            .iter()
            .any(|t| self.depends_on(t.node, node))
        {
            return Err(GraphError::WouldCycle);
        }
        let before = self.dependency_shape(node, index);
        self.unindex_expression(node, index);
        let reader = self.slot_of(node);
        for t in &compiled.targets {
            if self.ch_edges.insert((t.node, node, index))
                && let Some(reader) = reader
            {
                self.add_dependent(t.slot, reader);
            }
        }
        if let Some(n) = self.node_mut(node) {
            n.exprs[index] = Some(compiled);
        }
        if self.dependency_shape(node, index) != before {
            self.structure_changed();
        }
        self.bump(node);
        if poly.is_some() {
            self.refresh_types([node]);
        }
        Ok(())
    }

    /// Remove the expression of an input, if any. On an interface input of
    /// a Charm instance this restores the definition's expression, if it
    /// has one.
    ///
    /// # Errors
    /// Fails for an unknown node or port (and when restoring the
    /// definition's expression fails).
    pub fn clear_expression(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        self.check_sources_editable(node)?;
        if let Some(source) = self.instance_default_expression(node, port) {
            return self.set_expression(node, port, &source);
        }
        let had = self.node(node).is_some_and(|n| n.exprs[index].is_some());
        if had {
            let before = self.dependency_shape(node, index);
            self.unindex_expression(node, index);
            if let Some(n) = self.node_mut(node) {
                n.exprs[index] = None;
            }
            if self.dependency_shape(node, index) != before {
                self.structure_changed();
            }
            self.bump(node);
            if self.poly(node).is_some() {
                self.refresh_types([node]);
            }
        }
        Ok(())
    }

    /// The expression source of an input, or `None`.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn expression(&self, node: NodeId, port: &str) -> Result<Option<&str>, GraphError> {
        let (index, _) = self.input_port(node, port)?;
        Ok(self
            .node(node)
            .and_then(|n| n.exprs.get(index))
            .and_then(Option::as_ref)
            .map(|e| e.source.as_str()))
    }

    /// Every expression of a node as `(port name, source)`, in port order.
    /// Empty for an unknown node.
    pub fn expressions(&self, node: NodeId) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.node(node).into_iter().flat_map(|n| {
            let spec = n.imp.spec();
            n.exprs
                .iter()
                .zip(&spec.inputs)
                .filter_map(|(e, port)| e.as_ref().map(|e| (port.name, e.source.as_str())))
        })
    }

    /// Drive input `port` of `node` with an animation channel. A link or an
    /// expression, if present, still wins. Replaces a previous channel.
    ///
    /// # Errors
    /// Fails for an unknown node or port, or a channel whose value type the
    /// input does not accept.
    pub fn set_animation(
        &mut self,
        node: NodeId,
        port: &str,
        channel: AnimChannel,
    ) -> Result<(), GraphError> {
        let (index, ty) = self.input_port(node, port)?;
        self.check_sources_editable(node)?;
        let found = channel.value_type();
        let poly = self.check_poly_source(node, index, SourceKind::Anim, found)?;
        if poly.is_none() && !ty.accepts(found) {
            return Err(GraphError::TypeMismatch {
                expected: ty,
                found,
            });
        }
        let before = self.time_dependent(node);
        if let Some(n) = self.node_mut(node) {
            n.anims[index] = Some(channel);
        }
        if self.time_dependent(node) != before {
            self.structure_changed();
        }
        self.bump(node);
        if poly.is_some() {
            self.refresh_types([node]);
        }
        Ok(())
    }

    /// Remove the animation channel of an input, if any. On an interface
    /// input of a Charm instance this restores the definition's channel,
    /// if it has one.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn clear_animation(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        self.check_sources_editable(node)?;
        if let Some(channel) = self.instance_default_animation(node, port) {
            return self.set_animation(node, port, channel);
        }
        let before = self.time_dependent(node);
        let had = self
            .node_mut(node)
            .and_then(|n| n.anims[index].take())
            .is_some();
        if had {
            if self.time_dependent(node) != before {
                self.structure_changed();
            }
            self.bump(node);
            if self.poly(node).is_some() {
                self.refresh_types([node]);
            }
        }
        Ok(())
    }

    /// The animation channel of an input, or `None`.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn animation(&self, node: NodeId, port: &str) -> Result<Option<&AnimChannel>, GraphError> {
        let (index, _) = self.input_port(node, port)?;
        Ok(self
            .node(node)
            .and_then(|n| n.anims.get(index))
            .and_then(Option::as_ref))
    }

    /// Every animation channel of a node as `(port name, channel)`, in port
    /// order. Empty for an unknown node.
    pub fn animations(&self, node: NodeId) -> impl Iterator<Item = (&str, &AnimChannel)> + '_ {
        self.node(node).into_iter().flat_map(|n| {
            let spec = n.imp.spec();
            n.anims
                .iter()
                .zip(&spec.inputs)
                .filter_map(|(a, port)| a.as_ref().map(|a| (port.name, a)))
        })
    }

    /// The inputs of other nodes whose expressions read a port of `node`
    /// (an input through `ch()` or an output through `out()`), as
    /// `(node, port name)`, ordered by node id and port, each once. See
    /// [`Graph::expression_refs`] for what each one reads.
    #[must_use]
    pub fn referenced_by(&self, node: NodeId) -> Vec<(NodeId, String)> {
        self.ch_dependents(node)
            .filter_map(|(dep, port)| {
                let spec = self.node(dep)?.imp.spec();
                Some((dep, spec.inputs.get(port)?.name.to_owned()))
            })
            .collect()
    }

    /// Every `ch()` and `out()` reference of every expression in the
    /// graph (all networks), for example for a UI that draws them as
    /// dashed wires. Sorted by reader, reader port index, kind, target and
    /// target port index; a port read several times by one expression is
    /// listed once.
    #[must_use]
    pub fn expression_refs(&self) -> Vec<ExprRef> {
        // Sort key: (reader, reader input, kind, target, target port).
        type Key = (NodeId, usize, RefKind, NodeId, usize);
        let mut keyed: Vec<(Key, ExprRef)> = Vec::new();
        for reader in self.node_ids() {
            let Some(n) = self.node(reader) else {
                continue;
            };
            let spec = n.imp.spec();
            for (index, e) in n.exprs.iter().enumerate() {
                let (Some(e), Some(port)) = (e, spec.inputs.get(index)) else {
                    continue;
                };
                for t in &e.targets {
                    let Some(target_spec) = self.node_spec(t.node) else {
                        continue;
                    };
                    let target_port = match t.kind {
                        RefKind::Input => target_spec.inputs.get(t.port),
                        RefKind::Output => target_spec.outputs.get(t.port),
                    };
                    let Some(target_port) = target_port else {
                        continue;
                    };
                    keyed.push((
                        (reader, index, t.kind, t.node, t.port),
                        ExprRef {
                            reader,
                            reader_port: port.name.to_owned(),
                            kind: t.kind,
                            target: t.node,
                            target_port: target_port.name.to_owned(),
                        },
                    ));
                }
            }
        }
        keyed.sort_by_key(|(key, _)| *key);
        keyed.into_iter().map(|(_, r)| r).collect()
    }

    /// `(dependent node, dependent input index)` of every expression that
    /// reads a port of `node` (through `ch()` or `out()`).
    pub(crate) fn ch_dependents(&self, node: NodeId) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.ch_edges
            .range((node, NodeId::MIN, 0)..)
            .take_while(move |(t, _, _)| *t == node)
            .map(|&(_, dep, port)| (dep, port))
    }

    /// Forget the dependency edges of the expression on `node`'s input
    /// `index`, if any.
    pub(crate) fn unindex_expression(&mut self, node: NodeId, index: usize) {
        let Some(e) = self.node(node).and_then(|n| n.exprs[index].as_ref()) else {
            return;
        };
        let targets: Vec<(NodeId, usize)> = e.targets.iter().map(|t| (t.node, t.slot)).collect();
        let reader = self.slot_of(node);
        for (t, slot) in targets {
            if self.ch_edges.remove(&(t, node, index))
                && let Some(reader) = reader
            {
                self.remove_dependent(slot, reader);
            }
        }
    }

    /// The inputs whose expression reads `node`, or a node inside it, through
    /// `ch()` or `out()`. Such a node cannot be removed while a reader
    /// outside it is left. Sorted; the reader may be inside `node` too.
    #[must_use]
    pub fn expression_readers(&self, node: NodeId) -> Vec<(NodeId, &str)> {
        self.subtree_readers(node)
            .into_iter()
            .filter_map(|(reader, index)| {
                let port = self.node(reader)?.imp.spec().inputs.get(index)?;
                Some((reader, port.name))
            })
            .collect()
    }

    /// Every `(reader, input index)` whose expression reads a node of
    /// `node`'s subtree.
    pub(crate) fn subtree_readers(&self, node: NodeId) -> BTreeSet<(NodeId, usize)> {
        self.subtree(node)
            .into_iter()
            .flat_map(|n| self.ch_dependents(n).collect::<Vec<_>>())
            .collect()
    }

    /// The expression rewrites renaming `node` to `new` needs, computed
    /// against the current (old) names.
    pub(crate) fn rename_rewrites(&self, node: NodeId, new: &str) -> Vec<Rewrite> {
        self.subtree_readers(node)
            .into_iter()
            .filter_map(|(dep, port)| {
                let n = self.node(dep)?;
                let e = n.exprs.get(port)?.as_ref()?;
                let source = rewrite(self, n.parent, &e.source, node, new)?;
                Some((dep, port, source))
            })
            .collect()
    }

    /// Replace expression sources with rewritten ones that must read
    /// exactly the same inputs. All or nothing; no revision bump, since the
    /// programs read the same inputs and cooked results stay valid.
    pub(crate) fn apply_rewrites(&mut self, rewrites: Vec<Rewrite>) -> Result<(), GraphError> {
        let mut updates = Vec::with_capacity(rewrites.len());
        for (dep, port, source) in rewrites {
            let Some(e) = self.node(dep).and_then(|n| n.exprs[port].as_ref()) else {
                continue;
            };
            let compiled = compile(self, self.parent(dep), &source)?;
            if compiled.targets != e.targets {
                return Err(GraphError::Expression(Box::new(expr::ExprError::new(
                    expr::ErrorKind::Internal("rewriting a path changed what an expression reads"),
                    expr::Span::default(),
                ))));
            }
            updates.push((dep, port, compiled));
        }
        for (dep, port, compiled) in updates {
            if let Some(n) = self.node_mut(dep) {
                n.exprs[port] = Some(compiled);
            }
        }
        Ok(())
    }

    /// Whether `node` re-cooks when the clock changes.
    fn time_dependent(&self, node: NodeId) -> bool {
        self.node(node)
            .is_some_and(super::graph::Node::time_dependent)
    }

    /// What the expression on input `index` of `node` contributes to the
    /// cooker's cached structure: the nodes it reads and whether the node
    /// is time-dependent.
    fn dependency_shape(&self, node: NodeId, index: usize) -> (Vec<usize>, bool) {
        let mut reads: Vec<usize> = self
            .node(node)
            .and_then(|n| n.exprs.get(index))
            .and_then(Option::as_ref)
            .map(|e| e.targets.iter().map(|t| t.slot).collect())
            .unwrap_or_default();
        reads.sort_unstable();
        reads.dedup();
        (reads, self.time_dependent(node))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_at_the_last_slash() {
        assert_eq!(split_path("sun/phase"), Some(("sun", "phase")));
        assert_eq!(split_path("../sun/phase"), Some(("sun", "phase")));
        assert_eq!(split_path("a/b/c"), Some(("a/b", "c")));
        assert_eq!(split_path("../../x/p"), Some(("../x", "p")));
        assert_eq!(split_path("phase"), None);
        assert_eq!(split_path("sun/"), None);
        assert_eq!(split_path("../phase"), None);
    }
}
