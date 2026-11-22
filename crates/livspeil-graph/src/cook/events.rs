// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Events for the cooker (milestone M0.10).
//!
//! Values are pulled lazily and cached; events are pushed, once, in a
//! defined phase of every tick: **input -> events -> cook -> publish**.
//! [`Cooker::tick`] runs the first two (the host's events are the input);
//! the host then cooks what it shows ([`Cooker::cook`]) and publishes it.
//!
//! # Model
//!
//! * `Event` ports carry [`Event`]s (a trigger with an optional payload).
//!   Links between `Event` ports are ordinary links of the graph (stored,
//!   type checked, serialized and undone like the others), but they are not
//!   value dependencies: they feed no values and may form cycles.
//! * Event nodes (`livspeil_node_api::event`) handle events delivered to
//!   their `Event` inputs and, if they poll, are called once per tick. They
//!   read their resolved value inputs, emit events and update a runtime
//!   **state**. State is the only way events reach values: a node's value
//!   outputs are a function of its inputs and its state
//!   (`NodeImpl::cook_with_state`). This is the "binding": a counter's
//!   `count`, a toggle's `on` or a latch's `out` can be linked to any input,
//!   so events set parameters through ordinary value links.
//! * The event queue and the states belong to the **cooker**, like the
//!   clock and the value caches: each cooker runs its own timeline, so
//!   several cookers on one graph (a live view and a preview at another
//!   time) do not disturb each other, and `tick` takes the graph by shared
//!   reference. States are runtime only: documents never store them, and a
//!   new cooker (or one used with another graph) starts from each node's
//!   initial state. States survive edits; a removed node's state is
//!   dropped.
//!
//! # Delivery order
//!
//! Deterministic and independent of caching. One FIFO queue per tick,
//! seeded with, in order:
//!
//! 1. the host's events ([`ExternalEvent`]), in the order given: an event
//!    for an `Event` input is delivered to it, one for an `Event` output is
//!    emitted from it;
//! 2. a poll of every polling node, in node id order.
//!
//! Then items are taken from the front until the queue is empty. A poll
//! or delivery calls the node's handler; the events it emits are appended
//! in emission order. An emission is recorded in [`TickReport::fired`] and
//! queues one delivery per linked `Event` input, ordered by target node id
//! and input port. Interface nodes of subnets (`core.subnet.input`,
//! `core.output` with port type `Event`) forward what they receive to their
//! output, so events cross subnet boundaries like values.
//!
//! # Bounds
//!
//! Every queue item (emission, delivery or poll) counts against the
//! cooker's event limit ([`Cooker::set_event_limit`], default
//! [`DEFAULT_EVENT_LIMIT`]). When it is reached, the rest of the queue is
//! dropped and counted in [`TickReport::dropped`]; the next tick starts
//! with an empty queue, so an event loop costs at most the limit per tick
//! and never hangs the host. A handler that fails (an error, a panic, an
//! event emitted from a port that is not an `Event` output) keeps its
//! state, emits nothing and is reported in [`TickReport::errors`].
//!
//! # Caching
//!
//! Before a handler runs, the cooker cooks the node (and so its upstream)
//! as usual, so handlers see current values and nothing is cooked twice.
//! When a handler changes the state (`!=`), the state gets a new version
//! from the cooker's version counter, and the node and its downstream (as
//! far as it is not already stale) get `validated = 0`, which makes them
//! stale for this cooker only (as in `cook/system.rs`). The node then
//! re-cooks because its state version is newer than its stamp, and the
//! usual rules (early cutoff) apply downstream. A tick without external
//! events and without polling nodes costs `O(1)`; routing tables are
//! rebuilt when the graph's structure revision changes.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};

use livspeil_node_api::{CookContext, Event, EventCall, EventTrigger, PortType, Value};
use uuid::Uuid;

use super::{CookError, Cooker, gather_inputs, panic_message};
use crate::graph::{Graph, GraphError, NodeId, PortDirection};
use crate::network;

/// Default for [`Cooker::set_event_limit`]: queue items per tick.
pub const DEFAULT_EVENT_LIMIT: usize = 10_000;

/// An event injected by the host for one tick.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalEvent {
    /// The node receiving (input) or emitting (output) the event.
    pub node: NodeId,
    /// Index of the port in the spec's inputs or outputs.
    pub port: usize,
    /// Whether the event is delivered to an input or emitted from an
    /// output.
    pub direction: PortDirection,
    pub event: Event,
}

impl ExternalEvent {
    /// The event `event` for port `port` of `node`: its `Event` input of
    /// that name if it has one, else its `Event` output of that name.
    ///
    /// # Errors
    /// [`GraphError::UnknownNode`], [`GraphError::UnknownPort`] when the
    /// node has no port of that name, [`GraphError::TypeMismatch`] when the
    /// port is not an `Event` port.
    pub fn new(graph: &Graph, node: NodeId, port: &str, event: Event) -> Result<Self, GraphError> {
        let spec = graph.node_spec(node).ok_or(GraphError::UnknownNode(node))?;
        let (index, direction, ty) = if let Some(i) = spec.input_index(port) {
            (i, PortDirection::Input, spec.inputs[i].ty)
        } else if let Some(i) = spec.output_index(port) {
            (i, PortDirection::Output, spec.outputs[i].ty)
        } else {
            return Err(GraphError::UnknownPort {
                node,
                port: port.to_owned(),
                direction: PortDirection::Input,
            });
        };
        if ty != PortType::Event {
            return Err(GraphError::TypeMismatch {
                expected: PortType::Event,
                found: ty,
            });
        }
        Ok(ExternalEvent {
            node,
            port: index,
            direction,
            event,
        })
    }
}

/// An event emitted from an output port during a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct FiredEvent {
    pub node: NodeId,
    /// Index of the output port in the node's spec.
    pub port: usize,
    pub event: Event,
}

/// An event handler that failed during a tick.
#[derive(Debug, Clone, PartialEq)]
pub struct EventError {
    pub node: NodeId,
    pub error: CookError,
}

/// What happened in the events phase of one tick.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TickReport {
    /// Every emission, in queue order.
    pub fired: Vec<FiredEvent>,
    /// Queue items processed (emissions, deliveries and polls).
    pub processed: usize,
    /// Queue items dropped because the event limit was reached (0 when
    /// the queue drained).
    pub dropped: usize,
    /// Handlers that failed, in queue order.
    pub errors: Vec<EventError>,
}

impl TickReport {
    /// Whether the event limit cut the tick short.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.dropped > 0
    }

    /// The events fired from output `port` of `node`, in order.
    pub fn fired_at(&self, node: NodeId, port: usize) -> impl Iterator<Item = &Event> {
        self.fired
            .iter()
            .filter(move |f| f.node == node && f.port == port)
            .map(|f| &f.event)
    }
}

/// The runtime state of one node.
#[derive(Debug)]
struct NodeState {
    id: NodeId,
    value: Option<Value>,
    /// From the cooker's version counter; 0 = initial state.
    version: u64,
}

/// One queue item.
#[derive(Debug)]
enum Item {
    Emit {
        slot: usize,
        port: usize,
        event: Event,
    },
    Deliver {
        slot: usize,
        port: usize,
        event: Event,
    },
    Poll {
        slot: usize,
    },
}

/// Event routing, queue limit and node states of a cooker.
#[derive(Debug)]
pub(super) struct EventState {
    /// `(graph, topo_rev)` the routing tables belong to.
    built_for: Option<(Uuid, u64)>,
    /// Polling nodes, in id order.
    pollers: Vec<usize>,
    /// `(slot, output port)` to the linked `(slot, input port)`s, ordered
    /// by target id and port.
    routes: HashMap<(usize, usize), Vec<(usize, usize)>>,
    /// State by slot (checked against the node id: slots are reused).
    states: Vec<Option<NodeState>>,
    limit: usize,
    /// Reused input buffers for handlers.
    inputs: Vec<Value>,
    slots: Vec<Value>,
}

impl Default for EventState {
    fn default() -> Self {
        EventState {
            built_for: None,
            pollers: Vec::new(),
            routes: HashMap::new(),
            states: Vec::new(),
            limit: DEFAULT_EVENT_LIMIT,
            inputs: Vec::new(),
            slots: Vec::new(),
        }
    }
}

impl EventState {
    /// Forget states and routing (the cooker moved to another graph).
    pub(super) fn reset(&mut self) {
        self.built_for = None;
        self.states.clear();
    }

    fn get(&self, slot: usize, id: NodeId) -> Option<&NodeState> {
        self.states
            .get(slot)
            .and_then(Option::as_ref)
            .filter(|s| s.id == id)
    }

    /// The stored state of node `id` in `slot`, or `None` when it still has
    /// its initial state.
    pub(super) fn state(&self, slot: usize, id: NodeId) -> Option<Option<&Value>> {
        self.get(slot, id).map(|s| s.value.as_ref())
    }

    /// Version of the node's state (0 for the initial state).
    pub(super) fn state_version(&self, slot: usize, id: NodeId) -> u64 {
        self.get(slot, id).map_or(0, |s| s.version)
    }

    /// Rebuild the polling list and routes if the structure changed.
    fn sync(&mut self, graph: &Graph) {
        let key = (graph.uid(), graph.topo_rev());
        if self.built_for == Some(key) {
            return;
        }
        self.built_for = Some(key);
        self.pollers.clear();
        self.pollers.extend(graph.node_ids().filter_map(|id| {
            let slot = graph.slot_of(id)?;
            graph
                .node_at(slot)
                .is_some_and(|n| n.imp.polls_events())
                .then_some(slot)
        }));
        self.routes.clear();
        // Links come ordered by target node and input port.
        for link in graph.links() {
            let (Some(from), Some(to)) = (graph.node(link.from.node), graph.node(link.to.node))
            else {
                continue;
            };
            let event = from
                .imp
                .spec()
                .outputs
                .get(link.from.port)
                .is_some_and(|p| p.ty == PortType::Event);
            let (Some(from), Some(to)) = (graph.slot_of(from.id), graph.slot_of(to.id)) else {
                continue;
            };
            if event {
                self.routes
                    .entry((from, link.from.port))
                    .or_default()
                    .push((to, link.to.port));
            }
        }
        if self.states.len() < graph.slot_count() {
            self.states.resize_with(graph.slot_count(), || None);
        }
    }
}

impl Cooker {
    /// The most queue items (emissions, deliveries, polls) one tick may
    /// process; the rest is dropped and reported (see
    /// [`TickReport::dropped`]). At least 1.
    pub fn set_event_limit(&mut self, limit: usize) {
        self.events.limit = limit.max(1);
    }

    /// The current event limit.
    #[must_use]
    pub fn event_limit(&self) -> usize {
        self.events.limit
    }

    /// Drop every cached value (as a new cooker would have none) but keep
    /// the clock and the event nodes' runtime states. Cooks after it
    /// compute everything again; useful to free memory, and as a
    /// cache-free reference in tests.
    pub fn clear_values(&mut self) {
        self.entries.clear();
        self.failed.clear();
        self.topo_rev = None;
    }

    /// The runtime state of an event node, if an event changed it (`None`
    /// while it has its initial state).
    #[must_use]
    pub fn node_state(&self, graph: &Graph, id: NodeId) -> Option<&Value> {
        let slot = graph.slot_of(id)?;
        self.events.state(slot, id).flatten()
    }

    /// Run the *input* and *events* phases of one tick: set the clock to
    /// `clock`, queue `external` (in order) and a poll of every polling
    /// node (in id order), and deliver until the queue is empty or the
    /// event limit is reached. Cook the targets afterwards. Call it once
    /// per frame: polling nodes such as `core.event.every` fire again when
    /// a tick is repeated. See the `cook/events.rs` module docs for the
    /// order and caching rules.
    ///
    /// External events for nodes that no longer exist are ignored.
    pub fn tick(
        &mut self,
        graph: &Graph,
        clock: CookContext,
        external: &[ExternalEvent],
    ) -> TickReport {
        self.set_clock(clock);
        // Value caches first: a different graph resets the states too.
        self.sync(graph);
        self.events.sync(graph);
        let mut report = TickReport::default();
        if external.is_empty() && self.events.pollers.is_empty() {
            return report;
        }
        let mut queue = VecDeque::new();
        for ext in external {
            let Some(slot) = graph.slot_of(ext.node) else {
                continue;
            };
            let (port, event) = (ext.port, ext.event.clone());
            queue.push_back(match ext.direction {
                PortDirection::Input => Item::Deliver { slot, port, event },
                PortDirection::Output => Item::Emit { slot, port, event },
            });
        }
        queue.extend(self.events.pollers.iter().map(|&slot| Item::Poll { slot }));

        while let Some(item) = queue.pop_front() {
            if report.processed >= self.events.limit {
                report.dropped = queue.len() + 1;
                break;
            }
            report.processed += 1;
            let (slot, emitted) = match item {
                Item::Emit { slot, port, event } => {
                    let Some(node) = graph.node_at(slot) else {
                        continue;
                    };
                    if let Some(targets) = self.events.routes.get(&(slot, port)) {
                        queue.extend(targets.iter().map(|&(to, to_port)| Item::Deliver {
                            slot: to,
                            port: to_port,
                            event: event.clone(),
                        }));
                    }
                    report.fired.push(FiredEvent {
                        node: node.id,
                        port,
                        event,
                    });
                    continue;
                }
                Item::Deliver { slot, port, event } => {
                    let Some(node) = graph.node_at(slot) else {
                        continue;
                    };
                    if network::is_boundary_type(&node.type_id) {
                        // Interface nodes pass events through.
                        queue.push_back(Item::Emit {
                            slot,
                            port: 0,
                            event,
                        });
                        continue;
                    }
                    let trigger = EventTrigger::Input {
                        port,
                        event: &event,
                    };
                    (slot, self.handle(graph, slot, trigger))
                }
                Item::Poll { slot } => (slot, self.handle(graph, slot, EventTrigger::Tick)),
            };
            match emitted {
                Ok(emitted) => queue.extend(emitted.into_iter().map(|(port, event)| Item::Emit {
                    slot,
                    port,
                    event,
                })),
                Err(error) => {
                    if let Some(node) = graph.node_at(slot) {
                        report.errors.push(EventError {
                            node: node.id,
                            error,
                        });
                    }
                }
            }
        }
        report
    }

    /// Call the handler of the node in `slot`: bring its inputs up to
    /// date, run it, store a changed state (invalidating values) and return
    /// the emitted events.
    fn handle(
        &mut self,
        graph: &Graph,
        slot: usize,
        trigger: EventTrigger<'_>,
    ) -> Result<Vec<(usize, Event)>, CookError> {
        let Some(node) = graph.node_at(slot) else {
            return Ok(Vec::new());
        };
        // Cook the node, and with it its upstream, as a cook would; its
        // own value error (if any) does not stop event handling.
        let _ = self.cook(graph, node.id);
        let mut inputs = std::mem::take(&mut self.events.inputs);
        let mut slots = std::mem::take(&mut self.events.slots);
        let gathered = gather_inputs(&self.entries, node, &self.clock, &mut inputs, &mut slots);
        self.events.slots = slots;
        if let Err(error) = gathered {
            self.events.inputs = inputs;
            return Err(error);
        }
        let old = match self.events.state(slot, node.id) {
            Some(state) => state.cloned(),
            None => node.imp.initial_state(),
        };
        let clock = self.clock;
        let mut call = EventCall::new(&clock, trigger, &inputs, old.clone());
        let result = catch_unwind(AssertUnwindSafe(|| node.imp.on_event(&mut call)));
        let (state, emitted) = call.finish();
        self.events.inputs = inputs;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => return Err(CookError::Node(err)),
            Err(payload) => return Err(CookError::Panicked(panic_message(payload.as_ref()))),
        }
        let outputs = &node.imp.spec().outputs;
        if let Some((port, _)) = emitted
            .iter()
            .find(|(port, _)| outputs.get(*port).is_none_or(|p| p.ty != PortType::Event))
        {
            return Err(CookError::BadOutput(format!(
                "event emitted from output {port}, which is not an Event port"
            )));
        }
        if state != old {
            let version = self.next_version();
            if self.events.states.len() <= slot {
                self.events.states.resize_with(slot + 1, || None);
            }
            self.events.states[slot] = Some(NodeState {
                id: node.id,
                value: state,
                version,
            });
            self.invalidate_downstream(graph, slot);
        }
        Ok(emitted)
    }

    /// Make `slot` and its downstream stale for this cooker, stopping at
    /// nodes that already are (staleness is closed downstream).
    fn invalidate_downstream(&mut self, graph: &Graph, start: usize) {
        let mut stack = vec![start];
        while let Some(slot) = stack.pop() {
            if !self.is_fresh(graph, slot) {
                continue;
            }
            if let Some(Some(entry)) = self.entries.get_mut(slot) {
                entry.validated = 0;
            }
            stack.extend(graph.dependents_at(slot).iter().copied());
        }
    }
}
