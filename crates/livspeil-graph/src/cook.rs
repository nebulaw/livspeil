// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The cook scheduler: lazy, cached, deterministic evaluation with
//! push-based invalidation.
//!
//! # Invalidation
//!
//! Edits push, cooks pull:
//!
//! * Every edit to a graph gets a new *epoch*. An edit that can change what
//!   a node computes (an input value, link, expression, animation channel,
//!   a removed upstream node) marks the node and everything downstream of
//!   it, through links and `ch()`/`out()` readers, with that epoch (the graph's
//!   per-slot `dirty` epochs; see [`Graph`] for how the walk stops early).
//!   Renames do not mark anything.
//! * A [`Cooker`] stores, per node, the graph epoch at which it last
//!   validated the node (`validated`) and the cooker's clock generation at
//!   that time. A node is *stale* for a cooker when it has no cache entry,
//!   when `dirty > validated`, or when it is in the downstream closure of a
//!   time-dependent node (`tainted`: spec flag, `$F`/`$T`/`$DT`
//!   expressions, animation channels) and the clock changed since. Each
//!   cooker keeps its own `validated` epochs, so any number of cookers can
//!   work on one graph, and one cooker can be reused across edits; a
//!   cooker used with a different graph (another `Graph` instance) resets.
//! * Staleness is closed downstream, so `cook(target)` is `O(1)` when the
//!   target is not stale. Otherwise it walks the cached evaluation order of
//!   the target's upstream and visits only the stale nodes.
//! * A stale node is re-cooked only if something it reads really changed:
//!   its own revision, the output version of a linked upstream node or of
//!   a node it reads through `out()`, or the resolved-inputs version of a
//!   node it reads through `ch()` is newer than the node's own `stamp`, or (if time-dependent itself) the clock
//!   differs. A re-cooked node whose outputs are equal keeps its version,
//!   so its dependents are reused (early cutoff). Versions come from one
//!   monotonic counter per cooker, so "newer than the stamp" is exact.
//!
//! # Caches
//!
//! The graph's `topo_rev` changes when the node set, links, `ch()` edges
//! or the set of time-dependent nodes change. Until it does, the cooker
//! keeps the evaluation orders of the last few targets, their upstream
//! sizes and the `tainted` set. Node results are kept per dense slot.
//!
//! # Statistics
//!
//! [`CookStats`] counts as before: every node in the target's upstream
//! (target included) is either `cooked` (its `cook` was called), `reused`
//! (still valid), or neither (its inputs could not be gathered, for
//! example a failing upstream). Nodes that were not stale count as reused
//! without being visited.

use std::collections::HashMap;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use livspeil_node_api::{CookContext, NodeError, SystemContext, Value};
use livspeil_params::expr::{EvalContext, ExprError, RefKind};
use uuid::Uuid;

use crate::graph::{Graph, Node, NodeId};
use crate::sources::Expression;

// System bindings (M0.8): `Cooker::set_system` and the invalidation of
// system-reading nodes; see `cook/system.rs`.
mod system;

/// Why a node has no fresh output.
#[derive(Debug, Clone, PartialEq)]
pub enum CookError {
    /// The requested node does not exist.
    UnknownNode(NodeId),
    /// An unlinked input has neither a value nor a default.
    MissingInput { port: &'static str },
    /// An upstream node failed and has never produced a good output.
    Upstream { node: NodeId },
    /// The node returned an error.
    Node(NodeError),
    /// The node panicked; the panic was contained.
    Panicked(String),
    /// The node returned outputs that do not match its spec.
    BadOutput(String),
    /// The expression driving input `port` failed while evaluating.
    Expression {
        port: &'static str,
        error: ExprError,
    },
}

impl fmt::Display for CookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CookError::UnknownNode(id) => write!(f, "no node with id {id}"),
            CookError::MissingInput { port } => {
                write!(f, "input '{port}' has no value and no default")
            }
            CookError::Upstream { node } => {
                write!(f, "upstream node {node} has no usable output")
            }
            CookError::Node(err) => write!(f, "{err}"),
            CookError::Panicked(msg) => write!(f, "node panicked: {msg}"),
            CookError::BadOutput(msg) => write!(f, "bad node output: {msg}"),
            CookError::Expression { port, error } => {
                write!(f, "expression on input '{port}' failed: {error}")
            }
        }
    }
}

impl std::error::Error for CookError {}

/// Counters for observing laziness and caching in tests and benchmarks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CookStats {
    /// Nodes whose `cook` function was called.
    pub cooked: u64,
    /// Nodes visited whose cached result was still valid.
    pub reused: u64,
}

/// How many targets' evaluation orders a cooker keeps.
const PLAN_CACHE: usize = 8;

/// Cached result of one node.
#[derive(Debug)]
struct Entry {
    /// The node this entry belongs to (slots are reused after removal).
    id: NodeId,
    /// The node's revision when it was last processed.
    rev: u64,
    /// The cooker's version counter right after the node was last
    /// processed: an upstream version above it is a change.
    stamp: u64,
    /// Frame, time bits and dt bits of the last processing, for nodes that
    /// are time-dependent themselves.
    clock: Option<(u64, u64, u64)>,
    /// Graph epoch at which the node was last validated.
    validated: u64,
    /// Clock generation at which the node was last validated.
    clock_gen: u64,
    /// Last good outputs.
    outputs: Option<Vec<Value>>,
    /// Changes only when the outputs change (0 = never produced).
    version: u64,
    error: Option<CookError>,
    /// Last successfully resolved input values, read by `ch()`.
    inputs: Option<Vec<Value>>,
    /// Changes only when `inputs` change (0 = never resolved).
    inputs_version: u64,
}

impl Entry {
    fn new(id: NodeId) -> Self {
        Entry {
            id,
            rev: 0,
            stamp: 0,
            clock: None,
            validated: 0,
            clock_gen: 0,
            outputs: None,
            version: 0,
            error: None,
            inputs: None,
            inputs_version: 0,
        }
    }
}

/// Evaluates nodes of a [`Graph`] and caches the results.
///
/// A cooker may be reused across edits of the same graph; it learns about
/// edits from the graph's dirty marks (see the module documentation).
/// Several cookers may cook the same graph. Using a cooker with a different
/// graph resets it.
#[derive(Debug, Default)]
pub struct Cooker {
    graph: Option<Uuid>,
    /// The graph's `topo_rev` the structural caches below belong to.
    topo_rev: Option<u64>,
    clock: CookContext,
    /// Changes whenever the clock changes.
    clock_gen: u64,
    /// Cached results by slot.
    entries: Vec<Option<Entry>>,
    /// Slot of every node whose entry carries an error (for
    /// [`Cooker::error`]; empty when nothing fails, so cooks hash no ids).
    failed: HashMap<NodeId, usize>,
    /// Per slot: whether the node itself is time-dependent.
    time_dependent: Vec<bool>,
    /// Per slot: whether the node or something upstream is time-dependent.
    tainted: Vec<bool>,
    /// Per slot: size of the node's upstream, itself included, if known.
    upstream_len: Vec<Option<usize>>,
    /// Evaluation orders of recent targets, least recently used first.
    plans: Vec<(usize, Vec<usize>)>,
    /// DFS scratch: visit marks by slot and the stack.
    seen: Vec<u32>,
    seen_gen: u32,
    stack: Vec<(usize, bool)>,
    /// Reused buffers for gathered inputs and `ch()` slot values.
    inputs_buf: Vec<Value>,
    slots_buf: Vec<Value>,
    version_counter: u64,
    stats: CookStats,
    /// System bindings (M0.8), see `cook/system.rs`.
    system: system::SystemState,
}

impl Cooker {
    /// A cooker with an empty cache and the clock at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the frame clock used by time-dependent nodes.
    pub fn set_clock(&mut self, clock: CookContext) {
        if clock_key(&clock) != clock_key(&self.clock) {
            self.clock_gen += 1;
        }
        self.clock = clock;
    }

    /// The current frame clock.
    #[must_use]
    pub fn clock(&self) -> CookContext {
        self.clock
    }

    /// Counters since creation or the last [`Cooker::reset_stats`].
    #[must_use]
    pub fn stats(&self) -> CookStats {
        self.stats
    }

    /// Zero the counters.
    pub fn reset_stats(&mut self) {
        self.stats = CookStats::default();
    }

    /// The error state of a node after the last cook that visited it.
    #[must_use]
    pub fn error(&self, id: NodeId) -> Option<&CookError> {
        let slot = *self.failed.get(&id)?;
        self.entries
            .get(slot)?
            .as_ref()
            .filter(|e| e.id == id)?
            .error
            .as_ref()
    }

    /// Cook `id` and everything it depends on, and return its outputs in
    /// port order.
    ///
    /// If the node failed but produced a good output earlier, that last good
    /// output is returned and the failure is available through
    /// [`Cooker::error`].
    ///
    /// # Errors
    /// Returns the cook error when the node has never produced an output.
    pub fn cook(&mut self, graph: &Graph, id: NodeId) -> Result<Vec<Value>, CookError> {
        let Some(target) = graph.slot_of(id) else {
            return Err(CookError::UnknownNode(id));
        };
        self.sync(graph);
        self.sync_system(graph);
        let epoch = graph.epoch();
        graph.observe(epoch);

        if self.is_fresh(graph, target) {
            let len = match self.upstream_len.get(target).copied().flatten() {
                Some(len) => len,
                None => {
                    let plan = self.take_plan(graph, target);
                    let len = plan.len();
                    self.put_plan(target, plan);
                    len
                }
            };
            self.stats.reused += len as u64;
        } else {
            let plan = self.take_plan(graph, target);
            let mut visited = 0_u64;
            for &slot in &plan {
                if !self.is_fresh(graph, slot) && self.cook_one(graph, slot, epoch) {
                    visited += 1;
                }
            }
            self.stats.reused += plan.len() as u64 - visited;
            self.put_plan(target, plan);
        }

        let entry = self.entries.get(target).and_then(Option::as_ref);
        match entry.and_then(|e| e.outputs.clone()) {
            Some(outputs) => Ok(outputs),
            None => Err(entry
                .and_then(|e| e.error.clone())
                .unwrap_or(CookError::UnknownNode(id))),
        }
    }

    /// Cook `id` and return the single output called `port`.
    ///
    /// # Errors
    /// Returns the cook error when the node has no output, or
    /// [`CookError::BadOutput`] for an unknown output port.
    pub fn cook_output(
        &mut self,
        graph: &Graph,
        id: NodeId,
        port: &str,
    ) -> Result<Value, CookError> {
        let outputs = self.cook(graph, id)?;
        let index = graph
            .node_spec(id)
            .and_then(|spec| spec.output_index(port))
            .ok_or_else(|| CookError::BadOutput(format!("no output port '{port}'")))?;
        outputs
            .into_iter()
            .nth(index)
            .ok_or_else(|| CookError::BadOutput(format!("missing output '{port}'")))
    }

    /// Bring the structural caches up to date with `graph`.
    fn sync(&mut self, graph: &Graph) {
        if self.graph != Some(graph.uid()) {
            self.graph = Some(graph.uid());
            self.topo_rev = None;
            self.entries.clear();
            self.failed.clear();
        }
        if self.topo_rev == Some(graph.topo_rev()) {
            return;
        }
        self.topo_rev = Some(graph.topo_rev());
        let slots = graph.slot_count();
        self.entries.resize_with(slots, || None);
        // Drop the results of removed nodes (their slot may hold a new one).
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            let stale = entry
                .as_ref()
                .is_some_and(|e| graph.node_at(slot).is_none_or(|n| n.id != e.id));
            if stale {
                if let Some(e) = entry.take() {
                    self.failed.remove(&e.id);
                }
            }
        }
        self.upstream_len.clear();
        self.upstream_len.resize(slots, None);
        self.plans.clear();

        // Downstream closure of the time-dependent nodes.
        self.time_dependent.clear();
        self.time_dependent
            .extend((0..slots).map(|slot| graph.node_at(slot).is_some_and(Node::time_dependent)));
        self.tainted.clear();
        self.tainted.resize(slots, false);
        let mut stack: Vec<usize> = (0..slots).filter(|&s| self.time_dependent[s]).collect();
        while let Some(slot) = stack.pop() {
            if std::mem::replace(&mut self.tainted[slot], true) {
                continue;
            }
            stack.extend(
                graph
                    .dependents_at(slot)
                    .iter()
                    .filter(|&&next| !self.tainted[next]),
            );
        }
    }

    /// Whether the cached result of `slot` is known to be valid.
    fn is_fresh(&self, graph: &Graph, slot: usize) -> bool {
        self.entries
            .get(slot)
            .and_then(Option::as_ref)
            .is_some_and(|e| {
                e.validated >= graph.dirty_at(slot)
                    && (e.clock_gen == self.clock_gen
                        || !self.tainted.get(slot).copied().unwrap_or(true))
            })
    }

    /// The evaluation order of `target`'s upstream, from the cache or
    /// freshly computed. Hand it back with [`Cooker::put_plan`].
    fn take_plan(&mut self, graph: &Graph, target: usize) -> Vec<usize> {
        if let Some(pos) = self.plans.iter().position(|(t, _)| *t == target) {
            return self.plans.remove(pos).1;
        }
        let plan = self.eval_order(graph, target);
        if let Some(len) = self.upstream_len.get_mut(target) {
            *len = Some(plan.len());
        }
        plan
    }

    fn put_plan(&mut self, target: usize, plan: Vec<usize>) {
        if self.plans.len() >= PLAN_CACHE {
            self.plans.remove(0);
        }
        self.plans.push((target, plan));
    }

    /// Nodes `root` depends on, then `root`, upstream first. Iterative so
    /// that very deep graphs cannot overflow the stack.
    fn eval_order(&mut self, graph: &Graph, root: usize) -> Vec<usize> {
        self.seen_gen = self.seen_gen.wrapping_add(1);
        if self.seen_gen == 0 {
            self.seen.clear();
            self.seen_gen = 1;
        }
        self.seen.resize(graph.slot_count(), 0);
        let mark = self.seen_gen;
        let mut order = Vec::new();
        self.stack.clear();
        self.stack.push((root, false));
        while let Some((slot, expanded)) = self.stack.pop() {
            if expanded {
                order.push(slot);
                continue;
            }
            if self.seen[slot] == mark {
                continue;
            }
            self.seen[slot] = mark;
            self.stack.push((slot, true));
            if let Some(node) = graph.node_at(slot) {
                for dep in node.dependencies() {
                    if self.seen[dep] != mark {
                        self.stack.push((dep, false));
                    }
                }
            }
        }
        order
    }

    fn next_version(&mut self) -> u64 {
        self.version_counter += 1;
        self.version_counter
    }

    /// Bring one stale node's cache entry up to date. All its upstream nodes
    /// are already valid. Returns whether the node had to be processed
    /// (re-cooked or failed) rather than reused.
    fn cook_one(&mut self, graph: &Graph, slot: usize, epoch: u64) -> bool {
        let Some(node) = graph.node_at(slot) else {
            return false;
        };
        let time_dependent = self.time_dependent.get(slot).copied().unwrap_or(true);
        debug_assert_eq!(
            time_dependent,
            node.time_dependent(),
            "stale time dependence"
        );
        let clock = time_dependent.then(|| clock_key(&self.clock));
        let clock_gen = self.clock_gen;

        let changed = match self.entries[slot].as_ref() {
            None => true,
            Some(e) => {
                let newer = |dep: usize, f: fn(&Entry) -> u64| {
                    self.entries
                        .get(dep)
                        .and_then(Option::as_ref)
                        .is_none_or(|d| f(d) > e.stamp)
                };
                e.rev != node.rev
                    || e.clock != clock
                    || node
                        .link_in
                        .iter()
                        .flatten()
                        .any(|&(src, _)| newer(src, |d| d.version))
                    || node
                        .exprs
                        .iter()
                        .flatten()
                        .flat_map(|x| x.targets.iter())
                        .any(|t| match t.kind {
                            RefKind::Input => newer(t.slot, |d| d.inputs_version),
                            RefKind::Output => newer(t.slot, |d| d.version),
                        })
            }
        };
        if !changed {
            if let Some(e) = self.entries[slot].as_mut() {
                e.validated = epoch;
                e.clock_gen = clock_gen;
            }
            return false;
        }

        let gathered = gather_inputs(
            &self.entries,
            node,
            &self.clock,
            &mut self.inputs_buf,
            &mut self.slots_buf,
        );
        let result = match gathered {
            Ok(()) => {
                let inputs_changed = self.entries[slot]
                    .as_ref()
                    .is_none_or(|e| e.inputs.as_deref() != Some(self.inputs_buf.as_slice()));
                let inputs_version = inputs_changed.then(|| self.next_version());
                self.stats.cooked += 1;
                let result = run_node(
                    node.imp.as_ref(),
                    &self.clock,
                    &self.system.context(),
                    &self.inputs_buf,
                );
                if let Some(v) = inputs_version {
                    let entry = self.entry_mut(slot, node.id);
                    entry.inputs_version = v;
                    let fresh = std::mem::take(&mut self.inputs_buf);
                    if let Some(old) = self.entry_mut(slot, node.id).inputs.replace(fresh) {
                        self.inputs_buf = old;
                    }
                }
                result
            }
            Err(error) => Err(error),
        };

        let version = match &result {
            Ok(outputs) => {
                let unchanged = self.entries[slot]
                    .as_ref()
                    .is_some_and(|e| e.outputs.as_ref() == Some(outputs));
                (!unchanged).then(|| self.next_version())
            }
            Err(_) => None,
        };
        let stamp = self.version_counter;
        let entry = self.entries[slot].get_or_insert_with(|| Entry::new(node.id));
        match result {
            Ok(outputs) => {
                if let Some(v) = version {
                    entry.version = v;
                    entry.outputs = Some(outputs);
                }
                if entry.error.take().is_some() {
                    self.failed.remove(&node.id);
                }
            }
            // Keep the last good output, version and inputs.
            Err(error) => {
                entry.error = Some(error);
                self.failed.insert(node.id, slot);
            }
        }
        entry.rev = node.rev;
        entry.clock = clock;
        entry.stamp = stamp;
        entry.validated = epoch;
        entry.clock_gen = clock_gen;
        true
    }

    /// The entry of `slot`, created for node `id` if missing.
    fn entry_mut(&mut self, slot: usize, id: NodeId) -> &mut Entry {
        self.entries[slot].get_or_insert_with(|| Entry::new(id))
    }
}

/// The clock as comparable bits.
fn clock_key(clock: &CookContext) -> (u64, u64, u64) {
    (clock.frame, clock.time.to_bits(), clock.dt.to_bits())
}

/// Resolve every input of a node into `inputs`: the upstream output if
/// linked, else its expression, else its animation channel, else the
/// explicit value, else the spec default.
fn gather_inputs(
    entries: &[Option<Entry>],
    node: &Node,
    clock: &CookContext,
    inputs: &mut Vec<Value>,
    slots: &mut Vec<Value>,
) -> Result<(), CookError> {
    let spec = node.imp.spec();
    inputs.clear();
    for (index, port) in spec.inputs.iter().enumerate() {
        let value = match node.link_in[index] {
            Some((src, src_port)) => {
                let src_id = entries
                    .get(src)
                    .and_then(Option::as_ref)
                    .map_or(node.id, |e| e.id);
                let outputs = entries
                    .get(src)
                    .and_then(Option::as_ref)
                    .and_then(|e| e.outputs.as_ref())
                    .ok_or(CookError::Upstream { node: src_id })?;
                outputs
                    .get(src_port)
                    .and_then(|v| v.clone().coerce(port.ty))
                    .ok_or_else(|| {
                        CookError::BadOutput(format!(
                            "upstream output {src_port} of node {src_id} does not fit input '{}'",
                            port.name
                        ))
                    })?
            }
            None => match &node.exprs[index] {
                Some(expr) => eval_expression(entries, expr, clock, slots, port.name, port.ty)?,
                None => match &node.anims[index] {
                    Some(anim) => anim.sample(clock.time).coerce(port.ty).ok_or_else(|| {
                        CookError::BadOutput(format!(
                            "animation does not fit input '{}'",
                            port.name
                        ))
                    })?,
                    None => node.values[index]
                        .clone()
                        .or_else(|| port.default.clone())
                        .or_else(|| Value::zero(port.ty))
                        .ok_or(CookError::MissingInput { port: port.name })?,
                },
            },
        };
        inputs.push(value);
    }
    Ok(())
}

/// Evaluate an expression against the resolved inputs (`ch()`) and last
/// good outputs (`out()`) it reads.
fn eval_expression(
    entries: &[Option<Entry>],
    expr: &Expression,
    clock: &CookContext,
    slots: &mut Vec<Value>,
    port: &'static str,
    ty: livspeil_node_api::PortType,
) -> Result<Value, CookError> {
    slots.clear();
    for target in &expr.targets {
        let value = entries
            .get(target.slot)
            .and_then(Option::as_ref)
            .and_then(|e| match target.kind {
                RefKind::Input => e.inputs.as_ref(),
                RefKind::Output => e.outputs.as_ref(),
            })
            .and_then(|values| values.get(target.port))
            .ok_or(CookError::Upstream { node: target.node })?;
        slots.push(value.clone());
    }
    let ctx = EvalContext::from_cook(clock, slots);
    let value = expr
        .program
        .eval(&ctx)
        .map_err(|error| CookError::Expression { port, error })?;
    value
        .coerce(ty)
        .ok_or_else(|| CookError::BadOutput(format!("expression does not fit input '{port}'")))
}

/// Call a node, containing panics and validating its outputs.
fn run_node(
    imp: &dyn livspeil_node_api::NodeImpl,
    ctx: &CookContext,
    system: &SystemContext<'_>,
    inputs: &[Value],
) -> Result<Vec<Value>, CookError> {
    let outputs = match catch_unwind(AssertUnwindSafe(|| {
        imp.cook_with_system(ctx, system, inputs)
    })) {
        Ok(Ok(outputs)) => outputs,
        Ok(Err(err)) => return Err(CookError::Node(err)),
        Err(payload) => return Err(CookError::Panicked(panic_message(payload.as_ref()))),
    };

    let spec = imp.spec();
    if outputs.len() != spec.outputs.len() {
        return Err(CookError::BadOutput(format!(
            "expected {} outputs, got {}",
            spec.outputs.len(),
            outputs.len()
        )));
    }
    for (value, port) in outputs.iter().zip(&spec.outputs) {
        if value.port_type() != port.ty {
            return Err(CookError::BadOutput(format!(
                "output '{}' should be {}, got {}",
                port.name,
                port.ty,
                value.port_type()
            )));
        }
    }
    Ok(outputs)
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}
