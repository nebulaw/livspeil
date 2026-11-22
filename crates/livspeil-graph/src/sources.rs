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
//! A path names an input of a node in the same network: `"node/port"`, or
//! equivalently `"../node/port"` (Houdini style, relative to the node that
//! holds the expression). The port is the text after the last `/`; the node
//! name is the text before it, after removing one leading `../`. So a node
//! called `a/b` is reachable as `"a/b/port"`, and a node whose own name
//! starts with `../` needs the prefix spelled out (`"../../x/port"`). Paths
//! must name a node, so an expression cannot read an input of its own node
//! (that would be a self-dependency and is rejected as a cycle).
//!
//! `ch()` reads the *resolved* value of the input (after links,
//! expressions and animation), with the input's port type.
//!
//! # Dependencies
//!
//! An expression that reads `ch("m/p")` on node `n` makes `n` depend on node
//! `m`, exactly like a link from `m` to `n`: such an edge may not close a
//! cycle ([`GraphError::WouldCycle`]), the cooker evaluates `m` first, and
//! `n` re-cooks when the resolved inputs of `m` change.
//!
//! # Renames and deletes
//!
//! Expression sources stay the single source of truth, and they never
//! dangle:
//!
//! * Renaming a node rewrites every `ch()` path literal that refers to it,
//!   in every expression of the graph (the literal is re-rendered with
//!   [`livspeil_params::expr::quote`], keeping a `../` prefix if it had
//!   one). Renaming back rewrites them back.
//! * Removing a node that another node's expression reads is refused with
//!   [`GraphError::Referenced`]; clear or change those expressions first.

use std::cell::RefCell;

use livspeil_params::AnimChannel;
use livspeil_params::expr::{self, Channel, Env, Program};

use crate::graph::{Graph, GraphError, NodeId};

/// An input of a node, by port index: the target of a `ch()` reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct InputRef {
    pub(crate) node: NodeId,
    pub(crate) port: usize,
}

/// A compiled expression attached to an input.
#[derive(Debug, Clone)]
pub(crate) struct Expression {
    pub(crate) source: String,
    pub(crate) program: Program,
    /// Slot `i` of the program reads input `targets[i]`.
    pub(crate) targets: Vec<InputRef>,
}

/// Split a `ch()` path into node name and port name.
pub(crate) fn split_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("../").unwrap_or(path);
    let (node, port) = rest.rsplit_once('/')?;
    (!node.is_empty() && !port.is_empty()).then_some((node, port))
}

/// Resolves `ch()` paths against a graph, assigning one slot per distinct
/// input.
struct GraphEnv<'a> {
    graph: &'a Graph,
    targets: RefCell<Vec<InputRef>>,
}

impl Env for GraphEnv<'_> {
    fn resolve(&self, path: &str) -> Option<Channel> {
        let (node_name, port_name) = split_path(path)?;
        let node = self.graph.find(node_name)?;
        let spec = self.graph.node_spec(node)?;
        let port = spec.input_index(port_name)?;
        let ty = spec.inputs.get(port)?.ty;
        let target = InputRef { node, port };
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

/// Compile `source` against the current names of `graph`.
pub(crate) fn compile(graph: &Graph, source: &str) -> Result<Expression, GraphError> {
    let env = GraphEnv {
        graph,
        targets: RefCell::new(Vec::new()),
    };
    let program = expr::compile(source, &env).map_err(|e| GraphError::Expression(Box::new(e)))?;
    Ok(Expression {
        source: source.to_owned(),
        program,
        targets: env.targets.into_inner(),
    })
}

/// `source` with every `ch()` path that names node `old` changed to name
/// `new`. Returns `None` when nothing changes.
pub(crate) fn rewrite(source: &str, old: &str, new: &str) -> Option<String> {
    let refs = expr::channel_refs(source).ok()?;
    let mut out = source.to_owned();
    let mut changed = false;
    // Back to front, so earlier spans stay valid.
    for r in refs.iter().rev() {
        let Some((node, port)) = split_path(&r.path) else {
            continue;
        };
        if node != old {
            continue;
        }
        let prefix = if r.path.starts_with("../") || new.starts_with("../") {
            "../"
        } else {
            ""
        };
        let literal = expr::quote(&format!("{prefix}{new}/{port}"));
        out.replace_range(r.span.start..r.span.end, &literal);
        changed = true;
    }
    changed.then_some(out)
}

impl Graph {
    /// Drive input `port` of `node` with an expression (see the
    /// [`livspeil_params::expr`] language). A link, if present, still wins.
    /// The result type must fit the input like a link would (`Int` results
    /// are accepted for `Float` inputs). Replaces a previous expression.
    ///
    /// Priority of an input's sources: link, expression, animation channel,
    /// static value, spec default.
    ///
    /// `ch()` paths: `"node/port"` or, equivalently, `"../node/port"` names
    /// input `port` of the sibling node `node` in this (flat) network. The
    /// port is the text after the last `/`; the node name is the text before
    /// it after removing one leading `../`. `ch()` reads the input's
    /// *resolved* value (after links, expressions and animation). A node
    /// cannot read its own inputs.
    ///
    /// Each `ch()` reference makes `node` depend on the node it reads, like
    /// a link: it must not close a cycle, and `node` re-cooks when the read
    /// node's resolved inputs change. Expressions reading `$F`, `$T` or
    /// `$DT` (and animation channels) make the node re-cook every frame.
    /// Evaluation errors become [`crate::CookError::Expression`] on the
    /// node, which keeps its last good output.
    ///
    /// Sources never dangle: [`Graph::rename`] rewrites every path literal
    /// naming the renamed node (keeping a `../` prefix), and
    /// [`Graph::remove_node`] refuses to remove a node that is read
    /// ([`GraphError::Referenced`]).
    ///
    /// # Errors
    /// Fails for an unknown node or port, a syntax, name or type error
    /// ([`GraphError::Expression`], including unknown `ch()` paths), a
    /// result type the input does not accept ([`GraphError::TypeMismatch`])
    /// or a `ch()` reference that would create a cycle
    /// ([`GraphError::WouldCycle`]).
    pub fn set_expression(
        &mut self,
        node: NodeId,
        port: &str,
        source: &str,
    ) -> Result<(), GraphError> {
        let (index, ty) = self.input_port(node, port)?;
        let compiled = compile(self, source)?;
        let found = compiled.program.result_type();
        if !ty.accepts(found) {
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
        self.unindex_expression(node, index);
        for t in &compiled.targets {
            self.ch_edges.insert((t.node, node, index));
        }
        if let Some(n) = self.nodes.get_mut(&node) {
            n.exprs[index] = Some(compiled);
        }
        self.bump(node);
        Ok(())
    }

    /// Remove the expression of an input, if any.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn clear_expression(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        let had = self
            .nodes
            .get(&node)
            .is_some_and(|n| n.exprs[index].is_some());
        if had {
            self.unindex_expression(node, index);
            if let Some(n) = self.nodes.get_mut(&node) {
                n.exprs[index] = None;
            }
            self.bump(node);
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
            .nodes
            .get(&node)
            .and_then(|n| n.exprs.get(index))
            .and_then(Option::as_ref)
            .map(|e| e.source.as_str()))
    }

    /// Every expression of a node as `(port name, source)`, in port order.
    /// Empty for an unknown node.
    pub fn expressions(&self, node: NodeId) -> impl Iterator<Item = (&str, &str)> + '_ {
        self.nodes.get(&node).into_iter().flat_map(|n| {
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
        let found = channel.value_type();
        if !ty.accepts(found) {
            return Err(GraphError::TypeMismatch {
                expected: ty,
                found,
            });
        }
        if let Some(n) = self.nodes.get_mut(&node) {
            n.anims[index] = Some(channel);
        }
        self.bump(node);
        Ok(())
    }

    /// Remove the animation channel of an input, if any.
    ///
    /// # Errors
    /// Fails for an unknown node or port.
    pub fn clear_animation(&mut self, node: NodeId, port: &str) -> Result<(), GraphError> {
        let (index, _) = self.input_port(node, port)?;
        let had = self
            .nodes
            .get_mut(&node)
            .and_then(|n| n.anims[index].take())
            .is_some();
        if had {
            self.bump(node);
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
            .nodes
            .get(&node)
            .and_then(|n| n.anims.get(index))
            .and_then(Option::as_ref))
    }

    /// Every animation channel of a node as `(port name, channel)`, in port
    /// order. Empty for an unknown node.
    pub fn animations(&self, node: NodeId) -> impl Iterator<Item = (&str, &AnimChannel)> + '_ {
        self.nodes.get(&node).into_iter().flat_map(|n| {
            let spec = n.imp.spec();
            n.anims
                .iter()
                .zip(&spec.inputs)
                .filter_map(|(a, port)| a.as_ref().map(|a| (port.name, a)))
        })
    }

    /// The inputs of other nodes whose expressions read an input of `node`
    /// through `ch()`, as `(node, port name)`, ordered by node id and port.
    #[must_use]
    pub fn referenced_by(&self, node: NodeId) -> Vec<(NodeId, String)> {
        self.ch_dependents(node)
            .filter_map(|(dep, port)| {
                let spec = self.nodes.get(&dep)?.imp.spec();
                Some((dep, spec.inputs.get(port)?.name.to_owned()))
            })
            .collect()
    }

    /// `(dependent node, dependent input index)` of every expression that
    /// reads an input of `node`.
    pub(crate) fn ch_dependents(&self, node: NodeId) -> impl Iterator<Item = (NodeId, usize)> + '_ {
        self.ch_edges
            .range((node, NodeId::MIN, 0)..)
            .take_while(move |(t, _, _)| *t == node)
            .map(|&(_, dep, port)| (dep, port))
    }

    /// Forget the dependency edges of the expression on `node`'s input
    /// `index`, if any.
    pub(crate) fn unindex_expression(&mut self, node: NodeId, index: usize) {
        let Some(e) = self.nodes.get(&node).and_then(|n| n.exprs[index].as_ref()) else {
            return;
        };
        let targets: Vec<NodeId> = e.targets.iter().map(|t| t.node).collect();
        for t in targets {
            self.ch_edges.remove(&(t, node, index));
        }
    }

    /// After renaming `node` from `old` to `new` (names already updated),
    /// rewrite the paths of every expression that reads it. On error
    /// nothing was changed.
    pub(crate) fn rewrite_references(
        &mut self,
        node: NodeId,
        old: &str,
        new: &str,
    ) -> Result<(), GraphError> {
        let mut deps: Vec<(NodeId, usize)> = self.ch_dependents(node).collect();
        deps.dedup();
        let mut updates = Vec::with_capacity(deps.len());
        for (dep, port) in deps {
            let Some(e) = self.nodes.get(&dep).and_then(|n| n.exprs[port].as_ref()) else {
                continue;
            };
            let Some(source) = rewrite(&e.source, old, new) else {
                continue;
            };
            let compiled = compile(self, &source)?;
            if compiled.targets != e.targets {
                return Err(GraphError::Expression(Box::new(expr::ExprError::new(
                    expr::ErrorKind::Internal("rename changed what an expression reads"),
                    expr::Span::default(),
                ))));
            }
            updates.push((dep, port, compiled));
        }
        // Same program and targets, new source text: no revision bump,
        // cooked results stay valid.
        for (dep, port, compiled) in updates {
            if let Some(n) = self.nodes.get_mut(&dep) {
                n.exprs[port] = Some(compiled);
            }
        }
        Ok(())
    }

    /// Whether any input source of `node` depends on the frame clock.
    pub(crate) fn sources_time_dependent(&self, node: NodeId) -> bool {
        self.nodes.get(&node).is_some_and(|n| {
            n.anims.iter().any(Option::is_some)
                || n.exprs
                    .iter()
                    .flatten()
                    .any(|e| e.program.is_time_dependent())
        })
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

    #[test]
    fn rewrite_keeps_prefixes_and_other_paths() {
        let src = r#"ch("sun/phase") + ch("../sun/x") * ch("moon/phase")"#;
        assert_eq!(
            rewrite(src, "sun", "star").as_deref(),
            Some(r#"ch("star/phase") + ch("../star/x") * ch("moon/phase")"#)
        );
        assert_eq!(rewrite(src, "nope", "x"), None);
        assert_eq!(
            rewrite(r#"ch("a/p")"#, "a", "q\"r").as_deref(),
            Some(r#"ch("q\"r/p")"#)
        );
    }
}
