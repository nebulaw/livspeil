// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Events (milestone M0.10): discrete triggers pushed through `Event` ports.
//!
//! Values and events are deliberately separate paradigms. Values are pulled
//! lazily and cached ([`NodeImpl::cook`]); events are pushed, once, through
//! an event queue that the host drains in the *events* phase of every tick
//! (`input -> events -> cook -> publish`).
//!
//! # Ports
//!
//! An [`Event`] carries an optional payload [`Value`]. In value cooking an
//! `Event` port carries no information: its value is the placeholder
//! [`Value::Event`] (the input default and [`Value::zero`] of
//! [`PortType::Event`]), and a node returns [`Value::Event`] for each of
//! its `Event` outputs. Keeping the placeholder keeps port indices the same
//! in both paradigms.
//!
//! # Event nodes
//!
//! A node takes part in the events phase when it has `Event` inputs (it is
//! called once per event delivered to one of them) or when
//! [`NodeImpl::polls_events`] is true (it is called once per tick, after
//! the events injected by the host were queued). Both calls go to
//! [`NodeImpl::on_event`] with an [`EventCall`]: the clock, the trigger,
//! the node's resolved value inputs at that moment, and the node's
//! **state**.
//!
//! The state is an `Option<Value>` kept by the host per node and per
//! cooker, starting at [`NodeImpl::initial_state`]. It is runtime only:
//! documents do not save it, and a new cooker starts from the initial
//! state. A node whose value outputs depend on its state (a counter, a
//! toggle) computes them in [`NodeImpl::cook_with_state`]; when an event
//! changes the state, the host invalidates the node's cached outputs and
//! everything downstream, so events reach values only through state
//! changes, and caching stays exact.
//!
//! Event handling must be deterministic like cooking: the emitted events
//! and the new state depend only on the call's contents.

use crate::{CookContext, Value};

/// One event: a trigger with an optional payload.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Event {
    /// The value carried by the event, if any.
    pub payload: Option<Value>,
}

impl Event {
    /// An event without payload.
    #[must_use]
    pub const fn bang() -> Self {
        Event { payload: None }
    }

    /// An event carrying `payload`.
    #[must_use]
    pub const fn with(payload: Value) -> Self {
        Event {
            payload: Some(payload),
        }
    }
}

/// Why [`NodeImpl::on_event`] is called.
///
/// [`NodeImpl::on_event`]: crate::NodeImpl::on_event
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EventTrigger<'a> {
    /// The once-per-tick call of a node that polls
    /// ([`NodeImpl::polls_events`](crate::NodeImpl::polls_events)).
    Tick,
    /// `event` arrived at input port `port` (an index into the spec's
    /// inputs, always an `Event` port).
    Input { port: usize, event: &'a Event },
}

/// What an event handler sees and what it produces: emitted events and a
/// new state.
#[derive(Debug)]
pub struct EventCall<'a> {
    ctx: &'a CookContext,
    trigger: EventTrigger<'a>,
    inputs: &'a [Value],
    state: Option<Value>,
    emitted: Vec<(usize, Event)>,
}

impl<'a> EventCall<'a> {
    /// A call with the node's current `state`. Hosts (and tests) create
    /// it; nodes only read and fill it.
    #[must_use]
    pub fn new(
        ctx: &'a CookContext,
        trigger: EventTrigger<'a>,
        inputs: &'a [Value],
        state: Option<Value>,
    ) -> Self {
        EventCall {
            ctx,
            trigger,
            inputs,
            state,
            emitted: Vec::new(),
        }
    }

    /// The frame clock of the current tick.
    #[must_use]
    pub fn ctx(&self) -> &CookContext {
        self.ctx
    }

    /// Why the node is called.
    #[must_use]
    pub fn trigger(&self) -> EventTrigger<'a> {
        self.trigger
    }

    /// The node's resolved value inputs, one per input port, as in
    /// [`NodeImpl::cook`](crate::NodeImpl::cook) (`Event` ports hold
    /// [`Value::Event`]).
    #[must_use]
    pub fn inputs(&self) -> &[Value] {
        self.inputs
    }

    /// The node's state.
    #[must_use]
    pub fn state(&self) -> Option<&Value> {
        self.state.as_ref()
    }

    /// Replace the node's state. Setting an equal state is not a change.
    pub fn set_state(&mut self, state: Option<Value>) {
        self.state = state;
    }

    /// Emit `event` from output port `port` (an index into the spec's
    /// outputs; it must be an `Event` port). Emitted events are queued in
    /// call order.
    pub fn emit(&mut self, port: usize, event: Event) {
        self.emitted.push((port, event));
    }

    /// The events emitted so far, as `(output port, event)`.
    #[must_use]
    pub fn emitted(&self) -> &[(usize, Event)] {
        &self.emitted
    }

    /// Split the finished call into its new state and emitted events.
    #[must_use]
    pub fn finish(self) -> (Option<Value>, Vec<(usize, Event)>) {
        (self.state, self.emitted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_collect_state_and_events() {
        let ctx = CookContext::default();
        let event = Event::with(Value::Int(3));
        let inputs = [Value::Event];
        let mut call = EventCall::new(
            &ctx,
            EventTrigger::Input {
                port: 0,
                event: &event,
            },
            &inputs,
            None,
        );
        assert_eq!(call.state(), None);
        assert_eq!(call.inputs(), &inputs);
        assert_eq!(call.ctx(), &ctx);
        assert!(matches!(
            call.trigger(),
            EventTrigger::Input { port: 0, .. }
        ));
        call.set_state(Some(Value::Int(1)));
        call.emit(0, Event::bang());
        call.emit(1, event.clone());
        assert_eq!(call.emitted().len(), 2);
        let (state, emitted) = call.finish();
        assert_eq!(state, Some(Value::Int(1)));
        assert_eq!(emitted, vec![(0, Event::bang()), (1, event)]);
        assert_eq!(Event::default(), Event::bang());
    }
}
