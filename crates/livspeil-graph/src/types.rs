// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Polymorphic ports: resolving type variables per node instance (M0.17).
//!
//! A node type may declare type variables (`livspeil_node_api::TypeVar`,
//! see `livspeil_node_api::types`): `core.logic.select@2` has
//! `T: Any` on `a`, `b` and `out`. Every *instance* in a graph resolves
//! each variable to one concrete [`PortType`]; the instance then behaves
//! exactly like a concrete node with those port types:
//! [`Graph::node_spec`] returns the resolved spec (so documents, the CLI,
//! `ch()`/`out()` and the cooker see concrete types), and
//! [`Graph::resolved_type`] / [`Graph::type_vars`] expose the variables.
//!
//! # Constraints
//!
//! Resolution is a pure function of the graph's current state (it never
//! depends on the order of edits), computed per **component**: variables
//! of different nodes joined by links between their ports must resolve
//! to the same type, so a link from `select.out` (`T`) into `lerp.a`
//! (`U`) puts `T` and `U` in one component. Each port of a component's
//! variables contributes:
//!
//! - **lower bounds** (the type must accept them): the output type of a
//!   concrete port linked into one of its inputs, and the type of an
//!   input's static value, expression result and animation channel;
//! - **upper bounds** (they must accept the type): the input type of a
//!   concrete port its outputs are linked into;
//! - an **exact** type: the type declared when the node was added (the
//!   *pin*, [`Graph::add_node_in`]'s `port_type`, stored in documents as
//!   `port-type=`; only for nodes with one type variable).
//!
//! Expressions are type-checked once, against the types of what they
//! read, so a port whose type could change under them may not be read:
//! `ch()` and `out()` may name a type variable's port only on a pinned
//! node ([`GraphError::UnpinnedRead`] otherwise). Other ports of a
//! polymorphic node (such as `select`'s `cond`) are read as usual. This
//! keeps resolution independent of readers, hence of edit order.
//!
//! # Rules
//!
//! 1. With a pin: that is the type.
//! 2. Else with lower bounds: their join, where `Int` and `Float` join to
//!    `Float` and any other two different types conflict. An `Int` join
//!    becomes `Float` in a class without `Int` (`Interpolable`).
//! 3. Else with upper bounds: the type they all accept, preferring
//!    `Float` over `Int` (an output feeding `Float` and `Int` inputs is
//!    `Int`).
//! 4. Else the fallback of the component's first variable (by node id,
//!    then variable order).
//!
//! The type must be in the class of every variable of the component,
//! accept every lower bound and be accepted by every upper bound.
//!
//! An edit that adds a constraint (a link, value, expression or animation
//! on a polymorphic port) is checked first and rejected with
//! [`GraphError::TypeConflict`] or [`GraphError::NotInClass`] when the
//! component would have no type; the graph is then unchanged. Removing a
//! constraint (unlink, clear, removing a node) never fails: every subset
//! of a consistent set of constraints is consistent, so the component may
//! only *unresolve* towards a type that accepts less, an upper bound or
//! its fallback. For the same reason a graph can be rebuilt by adding its
//! nodes, values, links and expressions in any order (documents and undo
//! do), and documents store nothing but the pin.
//!
//! # Effects
//!
//! A change of resolution replaces the instance (`livspeil_node_api::
//! instantiate`), converts the stored values of the variable's inputs
//! (`Int` to `Float`), and invalidates the node and its downstream like
//! any input edit (it changes what the node computes) and the cookers'
//! cached orders. Undo needs nothing extra: undoing the edit restores the
//! constraints, and with them the resolution.

use std::collections::BTreeSet;
use std::sync::Arc;

use livspeil_node_api::{NodeImpl, PortType, TypeClass, TypeVar, Value, instantiate};
use livspeil_params::expr::RefKind;

use crate::graph::{Endpoint, Graph, GraphError, NodeId};
use crate::sources::InputRef;

/// The type variables of a polymorphic node instance.
#[derive(Clone)]
pub(crate) struct Poly {
    /// The registered (unresolved) node type.
    pub(crate) base: Arc<dyn NodeImpl>,
    /// The variable typing each input, by input index.
    pub(crate) input_var: Vec<Option<usize>>,
    /// The variable typing each output, by output index.
    pub(crate) output_var: Vec<Option<usize>>,
    /// The resolved type of each variable.
    pub(crate) types: Vec<PortType>,
}

impl Poly {
    /// The polymorphism of `base`, `None` for a concrete node type.
    ///
    /// # Errors
    /// [`GraphError::InvalidTypeVars`] when the variables do not fit the
    /// spec.
    pub(crate) fn of(base: &Arc<dyn NodeImpl>) -> Result<Option<Poly>, GraphError> {
        let vars = base.type_vars();
        if vars.is_empty() {
            return Ok(None);
        }
        let (input_var, output_var) = livspeil_node_api::types::port_vars(base.spec(), vars)
            .map_err(|e| GraphError::InvalidTypeVars {
                node_type: base.spec().qualified_id(),
                reason: e.to_string(),
            })?;
        Ok(Some(Poly {
            base: Arc::clone(base),
            input_var,
            output_var,
            types: vars.iter().map(|v| v.fallback).collect(),
        }))
    }

    fn var(&self, kind: RefKind, port: usize) -> Option<usize> {
        match kind {
            RefKind::Input => self.input_var.get(port).copied().flatten(),
            RefKind::Output => self.output_var.get(port).copied().flatten(),
        }
    }
}

/// Which source of an input a pending edit sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceKind {
    Value,
    Expr,
    Anim,
}

/// An edit about to be made, checked before the graph changes.
#[derive(Clone, Copy)]
pub(crate) enum Pending {
    None,
    /// A new link.
    Link {
        from: Endpoint,
        to: Endpoint,
    },
    /// A new (or replaced) source of type `ty` on input `port` of `node`.
    Source {
        node: NodeId,
        port: usize,
        kind: SourceKind,
        ty: PortType,
    },
}

/// The constraints of one component.
#[derive(Default)]
struct Component {
    /// `(node, variable)`, sorted.
    members: Vec<(NodeId, usize)>,
    classes: Vec<TypeClass>,
    lower: Vec<PortType>,
    upper: Vec<PortType>,
    exact: Vec<PortType>,
}

/// Why a component has no type.
enum Conflict {
    Types { expected: PortType, found: PortType },
    Class { class: TypeClass, found: PortType },
}

/// `Int` and `Float` join to `Float`; other different types do not join.
fn join(a: PortType, b: PortType) -> Option<PortType> {
    if a.accepts(b) {
        Some(a)
    } else if b.accepts(a) {
        Some(b)
    } else {
        None
    }
}

impl Component {
    fn allowed(&self, ty: PortType) -> Result<(), Conflict> {
        match self.classes.iter().find(|c| !c.contains(ty)) {
            Some(&class) => Err(Conflict::Class { class, found: ty }),
            None => Ok(()),
        }
    }

    /// The component's type (see the module documentation), given the
    /// fallback of its first variable.
    fn solve(&self, fallback: PortType) -> Result<PortType, Conflict> {
        let ty = if let Some((&first, rest)) = self.exact.split_first() {
            if let Some(&other) = rest.iter().find(|&&t| t != first) {
                return Err(Conflict::Types {
                    expected: first,
                    found: other,
                });
            }
            self.allowed(first)?;
            if let Some(&low) = self.lower.iter().find(|&&l| !first.accepts(l)) {
                return Err(Conflict::Types {
                    expected: first,
                    found: low,
                });
            }
            first
        } else if let Some((&first, rest)) = self.lower.split_first() {
            let mut acc = first;
            for &low in rest {
                acc = join(acc, low).ok_or(Conflict::Types {
                    expected: acc,
                    found: low,
                })?;
            }
            match self.allowed(acc) {
                Ok(()) => acc,
                Err(_) if acc == PortType::Int && self.allowed(PortType::Float).is_ok() => {
                    PortType::Float
                }
                Err(e) => return Err(e),
            }
        } else if !self.upper.is_empty() {
            // Candidates are the upper bounds themselves (and Int, which
            // Float accepts); prefer anything over Int.
            let fits =
                |t: PortType| self.allowed(t).is_ok() && self.upper.iter().all(|u| u.accepts(t));
            let mut candidates: Vec<PortType> = self
                .upper
                .iter()
                .copied()
                .chain([PortType::Int])
                .filter(|&t| fits(t))
                .collect();
            candidates.sort_by_key(|&t| (t == PortType::Int, t));
            match candidates.first() {
                Some(&t) => return Ok(t),
                None => {
                    let (&first, rest) = self.upper.split_first().unwrap_or((&fallback, &[]));
                    let found = rest.iter().copied().find(|&u| u != first).unwrap_or(first);
                    self.allowed(found)?;
                    return Err(Conflict::Types {
                        expected: first,
                        found,
                    });
                }
            }
        } else {
            return match self.allowed(fallback) {
                Ok(()) => Ok(fallback),
                Err(e) => PortType::ALL
                    .into_iter()
                    .find(|&t| self.allowed(t).is_ok())
                    .ok_or(e),
            };
        };
        if let Some(&up) = self.upper.iter().find(|u| !u.accepts(ty)) {
            return Err(Conflict::Types {
                expected: up,
                found: ty,
            });
        }
        Ok(ty)
    }
}

impl Graph {
    /// The polymorphism of a node instance, if it has type variables.
    pub(crate) fn poly(&self, id: NodeId) -> Option<&Poly> {
        self.node(id)?.poly.as_deref()
    }

    /// Whether port `port` (`kind` side) of `node` belongs to a type
    /// variable.
    pub(crate) fn var_port(&self, node: NodeId, kind: RefKind, port: usize) -> Option<usize> {
        self.poly(node)?.var(kind, port)
    }

    /// The type variables of a node's type (empty for concrete nodes and
    /// unknown ids). Their resolved types: [`Graph::resolved_type`].
    #[must_use]
    pub fn type_vars(&self, id: NodeId) -> &[TypeVar] {
        self.poly(id).map_or(&[], |p| p.base.type_vars())
    }

    /// The type variable `var` of node `id` currently resolves to (see the
    /// `types` module docs for the rules); `None` for an unknown node or
    /// variable.
    #[must_use]
    pub fn resolved_type(&self, id: NodeId, var: &str) -> Option<PortType> {
        let poly = self.poly(id)?;
        let index = poly.base.type_vars().iter().position(|v| v.name == var)?;
        poly.types.get(index).copied()
    }

    /// The resolved type of every type variable of node `id`, in
    /// declaration order (empty for concrete nodes).
    #[must_use]
    pub fn resolved_types(&self, id: NodeId) -> &[PortType] {
        self.poly(id).map_or(&[], |p| p.types.as_slice())
    }

    /// The constraints of the component holding variable `start`, with
    /// `pending` applied.
    fn component(&self, start: (NodeId, usize), pending: Pending) -> Component {
        let mut c = Component::default();
        let mut seen = BTreeSet::new();
        let mut stack = vec![start];
        while let Some((id, var)) = stack.pop() {
            if !seen.insert((id, var)) {
                continue;
            }
            let Some(node) = self.node(id) else { continue };
            let Some(poly) = node.poly.as_deref() else {
                continue;
            };
            if let Some(decl) = poly.base.type_vars().get(var) {
                c.classes.push(decl.class);
            }
            if let (Some(pin), 0) = (node.port_type, var) {
                c.exact.push(pin);
            }
            // Inputs: link sources and sources of the input itself.
            for (port, _) in poly
                .input_var
                .iter()
                .enumerate()
                .filter(|(_, v)| **v == Some(var))
            {
                let source = match pending {
                    Pending::Link { from, to } if to.node == id && to.port == port => Some(from),
                    _ => self.link_source_endpoint(id, port),
                };
                if let Some(src) = source {
                    match self.var_port(src.node, RefKind::Output, src.port) {
                        Some(w) => stack.push((src.node, w)),
                        None => c.lower.extend(self.output_type(src.node, src.port)),
                    }
                }
                let over = |kind: SourceKind| match pending {
                    Pending::Source {
                        node,
                        port: p,
                        kind: k,
                        ty,
                    } if node == id && p == port && k == kind => Some(Some(ty)),
                    _ => None,
                };
                let value = over(SourceKind::Value)
                    .unwrap_or_else(|| node.values[port].as_ref().map(Value::port_type));
                let expr = over(SourceKind::Expr)
                    .unwrap_or_else(|| node.exprs[port].as_ref().map(|e| e.program.result_type()));
                let anim = over(SourceKind::Anim)
                    .unwrap_or_else(|| node.anims[port].as_ref().map(|a| a.value_type()));
                c.lower.extend([value, expr, anim].into_iter().flatten());
            }
            // Outputs: link targets.
            let outputs: Vec<usize> = poly
                .output_var
                .iter()
                .enumerate()
                .filter(|(_, v)| **v == Some(var))
                .map(|(port, _)| port)
                .collect();
            if !outputs.is_empty() {
                let mut targets = self.link_targets(id, &outputs);
                if let Pending::Link { from, to } = pending {
                    if from.node == id && outputs.contains(&from.port) {
                        targets.push(to);
                    }
                }
                for to in targets {
                    match self.var_port(to.node, RefKind::Input, to.port) {
                        Some(u) => stack.push((to.node, u)),
                        None => c.upper.extend(self.input_type(to.node, to.port)),
                    }
                }
            }
        }
        c.members = seen.into_iter().collect();
        c
    }

    /// The resolved type of output `port` of `node`.
    fn output_type(&self, node: NodeId, port: usize) -> Option<PortType> {
        Some(self.node(node)?.imp.spec().outputs.get(port)?.ty)
    }

    /// The resolved type of input `port` of `node`.
    fn input_type(&self, node: NodeId, port: usize) -> Option<PortType> {
        Some(self.node(node)?.imp.spec().inputs.get(port)?.ty)
    }

    /// Solve a component; `Err` names the variable at `start`.
    fn solve(&self, start: (NodeId, usize), pending: Pending) -> Result<PortType, GraphError> {
        let c = self.component(start, pending);
        let fallback = c
            .members
            .iter()
            .find_map(|&(id, var)| Some(self.poly(id)?.base.type_vars().get(var)?.fallback))
            .unwrap_or(PortType::Float);
        let var = self
            .poly(start.0)
            .and_then(|p| p.base.type_vars().get(start.1))
            .map_or_else(String::new, |v| v.name.to_owned());
        c.solve(fallback).map_err(|conflict| match conflict {
            Conflict::Types { expected, found } => GraphError::TypeConflict {
                node: start.0,
                var,
                expected,
                found,
            },
            Conflict::Class { class, found } => GraphError::NotInClass {
                node: start.0,
                var,
                class,
                found,
            },
        })
    }

    /// Check a new link touching a variable's port before making it.
    /// `Ok(false)` when neither end is polymorphic (the caller checks the
    /// concrete types).
    pub(crate) fn check_poly_link(&self, from: Endpoint, to: Endpoint) -> Result<bool, GraphError> {
        let start = match (
            self.var_port(to.node, RefKind::Input, to.port),
            self.var_port(from.node, RefKind::Output, from.port),
        ) {
            (Some(v), _) => (to.node, v),
            (None, Some(w)) => (from.node, w),
            (None, None) => return Ok(false),
        };
        self.solve(start, Pending::Link { from, to })?;
        Ok(true)
    }

    /// Check a new source of type `ty` on input `port` of `node`. Returns
    /// the type the input will have, or `None` when the input is not
    /// polymorphic (the caller checks the concrete type).
    pub(crate) fn check_poly_source(
        &self,
        node: NodeId,
        port: usize,
        kind: SourceKind,
        ty: PortType,
    ) -> Result<Option<PortType>, GraphError> {
        let Some(var) = self.var_port(node, RefKind::Input, port) else {
            return Ok(None);
        };
        let pending = Pending::Source {
            node,
            port,
            kind,
            ty,
        };
        self.solve((node, var), pending).map(Some)
    }

    /// Re-resolve every component holding a variable of one of `nodes`
    /// after an edit, applying changes. Nodes that are gone or concrete
    /// are skipped.
    pub(crate) fn refresh_types(&mut self, nodes: impl IntoIterator<Item = NodeId>) {
        let mut done = BTreeSet::new();
        let nodes: BTreeSet<NodeId> = nodes.into_iter().collect();
        for id in nodes {
            let vars = self.poly(id).map_or(0, |p| p.types.len());
            for var in 0..vars {
                if done.contains(&(id, var)) {
                    continue;
                }
                let c = self.component((id, var), Pending::None);
                done.extend(c.members.iter().copied());
                // A consistent graph stays consistent when constraints go
                // away, so this does not fail; keep the types if it does.
                let Ok(ty) = self.solve((id, var), Pending::None) else {
                    continue;
                };
                for (member, v) in c.members {
                    self.set_resolved(member, v, ty);
                }
            }
        }
    }

    /// Resolve variable `var` of `id` to `ty`: new instance, converted
    /// values, invalidation.
    fn set_resolved(&mut self, id: NodeId, var: usize, ty: PortType) {
        let Some(node) = self.node_mut(id) else {
            return;
        };
        let Some(poly) = node.poly.as_deref_mut() else {
            return;
        };
        if poly.types.get(var) == Some(&ty) {
            return;
        }
        let mut types = poly.types.clone();
        if let Some(slot) = types.get_mut(var) {
            *slot = ty;
        }
        let Ok(imp) = instantiate(&poly.base, &types) else {
            return;
        };
        poly.types = types;
        for (value, v) in node.values.iter_mut().zip(&poly.input_var) {
            if *v == Some(var) {
                if let Some(old) = value.take() {
                    *value = Some(old.clone().coerce(ty).unwrap_or(old));
                }
            }
        }
        node.imp = imp;
        self.structure_changed();
        self.bump(id);
    }

    /// Every polymorphic node linked to a node of `nodes`.
    pub(crate) fn poly_neighbors(&self, nodes: &[NodeId]) -> Vec<NodeId> {
        let mut out = Vec::new();
        for &id in nodes {
            out.extend(self.incoming_endpoints(id));
            out.extend(self.outgoing_endpoints(id));
        }
        out.retain(|&n| self.poly(n).is_some());
        out
    }

    /// Check that an expression may read `target`: a port of a type
    /// variable can only be read when the variable is pinned (see the
    /// module documentation).
    pub(crate) fn check_poly_read(&self, target: &InputRef) -> Result<(), GraphError> {
        let Some(node) = self.node(target.node) else {
            return Ok(());
        };
        let Some(poly) = node.poly.as_deref() else {
            return Ok(());
        };
        if poly.var(target.kind, target.port).is_none() || node.port_type.is_some() {
            return Ok(());
        }
        let spec = node.imp.spec();
        let port = match target.kind {
            RefKind::Input => spec.inputs.get(target.port),
            RefKind::Output => spec.outputs.get(target.port),
        };
        Err(GraphError::UnpinnedRead {
            node: target.node,
            port: port.map_or_else(String::new, |p| p.name.to_owned()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(lower: &[PortType], upper: &[PortType], exact: &[PortType]) -> Component {
        Component {
            members: Vec::new(),
            classes: vec![TypeClass::Numeric],
            lower: lower.to_vec(),
            upper: upper.to_vec(),
            exact: exact.to_vec(),
        }
    }

    fn solve(c: &Component) -> Option<PortType> {
        c.solve(PortType::Float).ok()
    }

    #[test]
    fn rules() {
        use PortType::{Bool, Float, Int, Vec2, Vec3};
        assert_eq!(solve(&comp(&[], &[], &[])), Some(Float));
        assert_eq!(solve(&comp(&[Int], &[], &[])), Some(Int));
        assert_eq!(solve(&comp(&[Int, Float], &[], &[])), Some(Float));
        assert_eq!(solve(&comp(&[Float, Int], &[], &[])), Some(Float));
        assert_eq!(solve(&comp(&[Vec3, Float], &[], &[])), None);
        assert_eq!(solve(&comp(&[Bool], &[], &[])), None);
        assert_eq!(solve(&comp(&[], &[Float], &[])), Some(Float));
        assert_eq!(solve(&comp(&[], &[Float, Int], &[])), Some(Int));
        assert_eq!(solve(&comp(&[], &[Vec2, Vec3], &[])), None);
        assert_eq!(solve(&comp(&[Int], &[Float], &[])), Some(Int));
        assert_eq!(solve(&comp(&[Float], &[Int], &[])), None);
        assert_eq!(solve(&comp(&[Int], &[], &[Float])), Some(Float));
        assert_eq!(solve(&comp(&[Float], &[], &[Int])), None);
        assert_eq!(solve(&comp(&[], &[], &[Vec2, Vec3])), None);
        let mut lerp = comp(&[Int], &[], &[]);
        lerp.classes = vec![TypeClass::Interpolable];
        assert_eq!(solve(&lerp), Some(Float));
        lerp.classes.push(TypeClass::Vector);
        assert_eq!(solve(&lerp), None);
        let none = comp(&[], &[], &[]);
        assert_eq!(none.solve(PortType::Vec2).ok(), Some(Vec2));
        let mut vec = comp(&[], &[], &[]);
        vec.classes = vec![TypeClass::Vector];
        // Fallback outside the classes: the first member.
        assert_eq!(vec.solve(PortType::Float).ok(), Some(Vec2));
    }
}
