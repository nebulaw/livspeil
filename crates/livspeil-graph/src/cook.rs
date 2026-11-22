// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The cook scheduler: lazy, cached, deterministic evaluation.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use livspeil_node_api::{CookContext, NodeError, Value};
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
    /// The frame, for time-dependent nodes only.
    frame: Option<u64>,
}

#[derive(Debug)]
struct Entry {
    signature: Signature,
    /// Last good outputs.
    outputs: Option<Vec<Value>>,
    /// Changes only when the outputs change (0 = never produced).
    version: u64,
    error: Option<CookError>,
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
            for (_, src) in graph.upstream(node) {
                if !seen.contains(&src.node) {
                    stack.push((src.node, false));
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

        let signature = Signature {
            rev: node.rev,
            upstream: upstream_versions,
            frame: spec.time_dependent.then_some(self.clock.frame),
        };
        if self
            .cache
            .get(&id)
            .is_some_and(|entry| entry.signature == signature)
        {
            self.stats.reused += 1;
            return;
        }

        let result = self.gather_inputs(graph, id, &linked).and_then(|inputs| {
            self.stats.cooked += 1;
            run_node(node.imp.as_ref(), &self.clock, &inputs)
        });

        match result {
            Ok(outputs) => {
                let previous = self.cache.get(&id);
                let unchanged = previous.is_some_and(|e| e.outputs.as_ref() == Some(&outputs));
                let version = if unchanged {
                    previous.map_or(0, |e| e.version)
                } else {
                    self.next_version()
                };
                self.cache.insert(
                    id,
                    Entry {
                        signature,
                        outputs: Some(outputs),
                        version,
                        error: None,
                    },
                );
            }
            Err(error) => {
                // Keep the last good output and version.
                let entry = self.cache.entry(id).or_insert_with(|| Entry {
                    signature: signature.clone(),
                    outputs: None,
                    version: 0,
                    error: None,
                });
                entry.signature = signature;
                entry.error = Some(error);
            }
        }
    }

    /// Resolve every input of a node to a value: the upstream output if
    /// linked, else the explicit value, else the spec default.
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
