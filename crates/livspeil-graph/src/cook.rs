// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The cook scheduler: lazy, cached, deterministic evaluation.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use livspeil_node_api::{CookContext, NodeError, Value};
use livspeil_params::expr::{EvalContext, ExprError};
use uuid::Uuid;

use crate::graph::{Endpoint, Graph, NodeId};

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

/// What a cached node result was computed from.
#[derive(Debug, Clone, PartialEq)]
struct Signature {
    /// The node's own revision (inputs and links).
    rev: u64,
    /// Output version of each linked upstream node, by input port.
    upstream: Vec<Option<u64>>,
    /// Resolved-inputs version of each node read through `ch()`, in the
    /// order of the node's expressions and their slots.
    reads: Vec<Option<u64>>,
    /// Frame, time bits and dt bits, for nodes that are time-dependent
    /// themselves or through an expression or animation channel.
    clock: Option<(u64, u64, u64)>,
}

#[derive(Debug)]
struct Entry {
    signature: Signature,
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
    fn new(signature: Signature) -> Self {
        Entry {
            signature,
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
/// A cooker may be reused across edits of the same graph; it notices edits
/// through per-node revisions. Using it with a different graph resets it.
#[derive(Debug, Default)]
pub struct Cooker {
    graph: Option<Uuid>,
    clock: CookContext,
    cache: HashMap<NodeId, Entry>,
    version_counter: u64,
    stats: CookStats,
}

impl Cooker {
    /// A cooker with an empty cache and the clock at zero.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the frame clock used by time-dependent nodes.
    pub fn set_clock(&mut self, clock: CookContext) {
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
        self.cache.get(&id).and_then(|e| e.error.as_ref())
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
        if !graph.nodes.contains_key(&id) {
            return Err(CookError::UnknownNode(id));
        }
        if self.graph != Some(graph.uid()) {
            self.cache.clear();
            self.graph = Some(graph.uid());
        }
        if self.cache.len() > graph.nodes.len() {
            self.cache.retain(|node, _| graph.nodes.contains_key(node));
        }

        for node in Self::eval_order(graph, id) {
            self.cook_one(graph, node);
        }

        let entry = self.cache.get(&id);
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

    /// Nodes `root` depends on, then `root`, upstream first. Iterative so
    /// that very deep graphs cannot overflow the stack.
    fn eval_order(graph: &Graph, root: NodeId) -> Vec<NodeId> {
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        let mut stack = vec![(root, false)];
        while let Some((node, expanded)) = stack.pop() {
            if expanded {
                order.push(node);
                continue;
            }
            if !seen.insert(node) {
                continue;
            }
            stack.push((node, true));
            for dep in graph.dependencies(node) {
                if !seen.contains(&dep) {
                    stack.push((dep, false));
                }
            }
        }
        order
    }

    fn next_version(&mut self) -> u64 {
        self.version_counter += 1;
        self.version_counter
    }

    /// Bring one node's cache entry up to date. All its upstream nodes have
    /// already been visited.
    fn cook_one(&mut self, graph: &Graph, id: NodeId) {
        let Some(node) = graph.nodes.get(&id) else {
            return;
        };
        let spec = node.imp.spec();

        let mut linked: Vec<Option<Endpoint>> = vec![None; spec.inputs.len()];
        let mut upstream_versions: Vec<Option<u64>> = vec![None; spec.inputs.len()];
        for (port, src) in graph.upstream(id) {
            linked[port] = Some(src);
            upstream_versions[port] = self.cache.get(&src.node).map(|e| e.version);
        }
        let reads = node
            .exprs
            .iter()
            .flatten()
            .flat_map(|e| e.targets.iter())
            .map(|t| self.cache.get(&t.node).map(|e| e.inputs_version))
            .collect();
        let time_dependent = spec.time_dependent || graph.sources_time_dependent(id);
        let signature = Signature {
            rev: node.rev,
            upstream: upstream_versions,
            reads,
            clock: time_dependent.then_some((
                self.clock.frame,
                self.clock.time.to_bits(),
                self.clock.dt.to_bits(),
            )),
        };
        if self
            .cache
            .get(&id)
            .is_some_and(|entry| entry.signature == signature)
        {
            self.stats.reused += 1;
            return;
        }

        let result = match self.gather_inputs(graph, id, &linked) {
            Ok(inputs) => {
                let changed = self
                    .cache
                    .get(&id)
                    .is_none_or(|e| e.inputs.as_ref() != Some(&inputs));
                let inputs_version = if changed {
                    Some(self.next_version())
                } else {
                    None
                };
                self.stats.cooked += 1;
                let result = run_node(node.imp.as_ref(), &self.clock, &inputs);
                let entry = self
                    .cache
                    .entry(id)
                    .or_insert_with(|| Entry::new(signature.clone()));
                if let Some(v) = inputs_version {
                    entry.inputs_version = v;
                    entry.inputs = Some(inputs);
                }
                result
            }
            Err(error) => Err(error),
        };

        let version = match &result {
            Ok(outputs) => {
                let unchanged = self
                    .cache
                    .get(&id)
                    .is_some_and(|e| e.outputs.as_ref() == Some(outputs));
                (!unchanged).then(|| self.next_version())
            }
            Err(_) => None,
        };
        let entry = self
            .cache
            .entry(id)
            .or_insert_with(|| Entry::new(signature.clone()));
        entry.signature = signature;
        match result {
            Ok(outputs) => {
                if let Some(v) = version {
                    entry.version = v;
                    entry.outputs = Some(outputs);
                }
                entry.error = None;
            }
            // Keep the last good output, version and inputs.
            Err(error) => entry.error = Some(error),
        }
    }

    /// Resolve every input of a node to a value: the upstream output if
    /// linked, else its expression, else its animation channel, else the
    /// explicit value, else the spec default.
    fn gather_inputs(
        &self,
        graph: &Graph,
        id: NodeId,
        linked: &[Option<Endpoint>],
    ) -> Result<Vec<Value>, CookError> {
        let Some(node) = graph.nodes.get(&id) else {
            return Err(CookError::UnknownNode(id));
        };
        let spec = node.imp.spec();
        let mut inputs = Vec::with_capacity(spec.inputs.len());
        for (index, port) in spec.inputs.iter().enumerate() {
            let value = match linked[index] {
                Some(src) => {
                    let outputs = self
                        .cache
                        .get(&src.node)
                        .and_then(|e| e.outputs.as_ref())
                        .ok_or(CookError::Upstream { node: src.node })?;
                    outputs
                        .get(src.port)
                        .cloned()
                        .and_then(|v| v.coerce(port.ty))
                        .ok_or_else(|| {
                            CookError::BadOutput(format!(
                                "upstream output {} of node {} does not fit input '{}'",
                                src.port, src.node, port.name
                            ))
                        })?
                }
                None if node.exprs[index].is_some() => {
                    self.eval_expression(graph, id, index, port.name, port.ty)?
                }
                None if node.anims[index].is_some() => node.anims[index]
                    .as_ref()
                    .and_then(|a| a.sample(self.clock.time).coerce(port.ty))
                    .ok_or_else(|| {
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
            };
            inputs.push(value);
        }
        Ok(inputs)
    }

    /// Evaluate the expression on input `index` of node `id`.
    fn eval_expression(
        &self,
        graph: &Graph,
        id: NodeId,
        index: usize,
        port: &'static str,
        ty: livspeil_node_api::PortType,
    ) -> Result<Value, CookError> {
        let Some(expr) = graph
            .nodes
            .get(&id)
            .and_then(|n| n.exprs.get(index))
            .and_then(Option::as_ref)
        else {
            return Err(CookError::UnknownNode(id));
        };
        let mut slots = Vec::with_capacity(expr.targets.len());
        for target in &expr.targets {
            let value = self
                .cache
                .get(&target.node)
                .and_then(|e| e.inputs.as_ref())
                .and_then(|inputs| inputs.get(target.port))
                .ok_or(CookError::Upstream { node: target.node })?;
            slots.push(value.clone());
        }
        let ctx = EvalContext::from_cook(&self.clock, &slots);
        let value = expr
            .program
            .eval(&ctx)
            .map_err(|error| CookError::Expression { port, error })?;
        value
            .coerce(ty)
            .ok_or_else(|| CookError::BadOutput(format!("expression does not fit input '{port}'")))
    }
}

/// Call a node, containing panics and validating its outputs.
fn run_node(
    imp: &dyn livspeil_node_api::NodeImpl,
    ctx: &CookContext,
    inputs: &[Value],
) -> Result<Vec<Value>, CookError> {
    let outputs = match catch_unwind(AssertUnwindSafe(|| imp.cook(ctx, inputs))) {
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
