// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Event nodes (milestone M0.10): `core.event.*`.
//!
//! See `livspeil_node_api::event` for the model and the graph crate's
//! `Cooker::tick` for delivery. In value cooking every `Event` port holds
//! the placeholder [`Value::Event`]. Nodes with value outputs that depend on
//! events (`counter`, `toggle`, `latch`) keep them in their runtime state,
//! which the cooker stores per cooker and never saves; their plain `cook`
//! returns the initial state's outputs.
//!
//! - `every`: fires each time the clock crosses `offset + k * period`
//!   (polls; stateless, a pure function of `time` and `dt`).
//! - `on_change`: fires with the new value when its `value` input differs
//!   from the previous tick's (polls).
//! - `counter`: counts `trigger` events, `reset` sets it back to 0.
//! - `toggle`: flips `on` on every `trigger`.
//! - `latch`: on `trigger`, holds the event's numeric payload, or else the
//!   `value` input (sample and hold).
//! - `gate`: forwards `trigger` to `out` while `open` is true.
//! - `select`: routes `trigger` to `if_true` or `if_false` by `cond`.

use livspeil_node_api::{
    CookContext, Event, EventCall, EventTrigger, NodeError, NodeImpl, NodeSpec, PortType,
    SystemContext, Value,
};

use crate::float;
use crate::time::finite;

/// The most events `core.event.every` emits in one tick (when `period` is
/// shorter than `dt`); the earliest crossings are kept.
pub const MAX_CROSSINGS_PER_TICK: i64 = 1_024;

fn bool_input(inputs: &[Value], index: usize, port: &'static str) -> Result<bool, NodeError> {
    match inputs.get(index) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(NodeError::InvalidInput {
            port,
            reason: "expected a Bool".to_owned(),
        }),
    }
}

/// The input port an event arrived at, if any.
fn arrived(call: &EventCall<'_>) -> Option<(usize, Event)> {
    match call.trigger() {
        EventTrigger::Input { port, event } => Some((port, event.clone())),
        EventTrigger::Tick => None,
    }
}

macro_rules! default_new {
    ($($ty:ident),*) => {$(
        impl Default for $ty {
            fn default() -> Self {
                Self::new()
            }
        }
    )*};
}

default_new!(Every, OnChange, Counter, Toggle, Latch, Gate, EventSelect);

/// `core.event.every@1`: a timer.
///
/// Inputs: `period` (Float seconds, > 0, default 1), `offset` (Float
/// seconds, default 0). Output: `fired` (Event, payload the Int `k` of the
/// crossed time `offset + k * period`). Polls: in the tick at `time` with
/// step `dt`, fires once for every `k` with
/// `time - dt < offset + k * period <= time`, in increasing `k`, at most
/// [`MAX_CROSSINGS_PER_TICK`] times. Only the clock is used, so the
/// result does not depend on earlier ticks.
#[derive(Debug)]
pub struct Every {
    spec: NodeSpec,
}

impl Every {
    #[must_use]
    pub fn new() -> Self {
        Every {
            spec: NodeSpec::new(
                "core.event.every",
                1,
                "Fire each time the clock crosses offset + k * period",
            )
            .input("period", PortType::Float, Some(Value::Float(1.0)))
            .input("offset", PortType::Float, Some(Value::Float(0.0)))
            .output("fired", PortType::Event),
        }
    }
}

/// `floor(x)` as an `i64`, rejecting values out of range.
fn floor_index(x: f64) -> Result<i64, NodeError> {
    let f = x.floor();
    // i64::MIN and MAX as f64 are -2^63 and 2^63.
    if f.is_finite() && f >= -9.223_372_036_854_775_808e18 && f < 9.223_372_036_854_775_808e18 {
        // In range by the check above.
        #[allow(clippy::cast_possible_truncation)]
        Ok(f as i64)
    } else {
        Err(NodeError::Failed(format!("time index {x} is out of range")))
    }
}

/// The `k` crossed in `(time - dt, time]` for `offset + k * period`.
pub fn crossings(
    time: f64,
    dt: f64,
    period: f64,
    offset: f64,
) -> Result<std::ops::RangeInclusive<i64>, NodeError> {
    let now = floor_index((time - offset) / period)?;
    let before = floor_index((time - dt - offset) / period)?;
    let first = before.saturating_add(1);
    let last = now.min(first.saturating_add(MAX_CROSSINGS_PER_TICK - 1));
    Ok(first..=last)
}

impl NodeImpl for Every {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Event])
    }

    fn polls_events(&self) -> bool {
        true
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let period = finite(float(call.inputs(), 0, "period")?, "period")?;
        let offset = finite(float(call.inputs(), 1, "offset")?, "offset")?;
        if period <= 0.0 {
            return Err(NodeError::InvalidInput {
                port: "period",
                reason: format!("must be positive, got {period}"),
            });
        }
        let ctx = *call.ctx();
        for k in crossings(ctx.time, ctx.dt, period, offset)? {
            call.emit(0, Event::with(Value::Int(k)));
        }
        Ok(())
    }
}

/// `core.event.on_change@1`: a change detector.
///
/// Input: `value` (Float, default 0). Output: `changed` (Event, payload
/// the new Float value). Polls: fires when `value` differs from its value
/// at the previous tick (NaN counts as equal to NaN); the first tick only
/// records the value. State: the last value (runtime only).
#[derive(Debug)]
pub struct OnChange {
    spec: NodeSpec,
}

impl OnChange {
    #[must_use]
    pub fn new() -> Self {
        OnChange {
            spec: NodeSpec::new(
                "core.event.on_change",
                1,
                "Fire with the new value when the input changes between ticks",
            )
            .input("value", PortType::Float, Some(Value::Float(0.0)))
            .output("changed", PortType::Event),
        }
    }
}

impl NodeImpl for OnChange {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Event])
    }

    fn polls_events(&self) -> bool {
        true
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let value = float(call.inputs(), 0, "value")?;
        let changed = match call.state().and_then(Value::as_float) {
            Some(last) => !(last == value || (last.is_nan() && value.is_nan())),
            None => false,
        };
        if changed {
            call.emit(0, Event::with(Value::Float(value)));
        }
        call.set_state(Some(Value::Float(value)));
        Ok(())
    }
}

/// `core.event.counter@1`: counts events.
///
/// Inputs: `trigger` and `reset` (Event). Output: `count` (Int, starts at
/// 0). `trigger` adds one (wrapping at `i64::MAX`), `reset` sets 0.
/// Payloads are ignored. State: the count (runtime only).
#[derive(Debug)]
pub struct Counter {
    spec: NodeSpec,
}

impl Counter {
    #[must_use]
    pub fn new() -> Self {
        Counter {
            spec: NodeSpec::new("core.event.counter", 1, "Count trigger events; reset to 0")
                .input("trigger", PortType::Event, Some(Value::Event))
                .input("reset", PortType::Event, Some(Value::Event))
                .output("count", PortType::Int),
        }
    }
}

impl NodeImpl for Counter {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Int(0)])
    }

    fn initial_state(&self) -> Option<Value> {
        Some(Value::Int(0))
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let count = match call.state() {
            Some(Value::Int(n)) => *n,
            _ => 0,
        };
        let next = match arrived(call) {
            Some((0, _)) => count.wrapping_add(1),
            Some((1, _)) => 0,
            _ => count,
        };
        call.set_state(Some(Value::Int(next)));
        Ok(())
    }

    fn cook_with_state(
        &self,
        _ctx: &CookContext,
        _system: &SystemContext<'_>,
        state: Option<&Value>,
        _inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        Ok(vec![state.cloned().unwrap_or(Value::Int(0))])
    }
}

/// `core.event.toggle@1`: flips on every event.
///
/// Input: `trigger` (Event). Output: `on` (Bool, starts false). State: the
/// flag (runtime only).
#[derive(Debug)]
pub struct Toggle {
    spec: NodeSpec,
}

impl Toggle {
    #[must_use]
    pub fn new() -> Self {
        Toggle {
            spec: NodeSpec::new("core.event.toggle", 1, "Flip a flag on every trigger event")
                .input("trigger", PortType::Event, Some(Value::Event))
                .output("on", PortType::Bool),
        }
    }
}

impl NodeImpl for Toggle {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Bool(false)])
    }

    fn initial_state(&self) -> Option<Value> {
        Some(Value::Bool(false))
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let on = matches!(call.state(), Some(Value::Bool(true)));
        if arrived(call).is_some() {
            call.set_state(Some(Value::Bool(!on)));
        }
        Ok(())
    }

    fn cook_with_state(
        &self,
        _ctx: &CookContext,
        _system: &SystemContext<'_>,
        state: Option<&Value>,
        _inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        Ok(vec![state.cloned().unwrap_or(Value::Bool(false))])
    }
}

/// `core.event.latch@1`: sample and hold.
///
/// Inputs: `trigger` (Event), `value` (Float, default 0). Output: `out`
/// (Float, starts at 0). On `trigger`, `out` becomes the event's payload
/// if it is a Float or an Int, else the current `value`. State: the held
/// value (runtime only).
#[derive(Debug)]
pub struct Latch {
    spec: NodeSpec,
}

impl Latch {
    #[must_use]
    pub fn new() -> Self {
        Latch {
            spec: NodeSpec::new(
                "core.event.latch",
                1,
                "Hold the trigger's payload (or the value input) until the next trigger",
            )
            .input("trigger", PortType::Event, Some(Value::Event))
            .input("value", PortType::Float, Some(Value::Float(0.0)))
            .output("out", PortType::Float),
        }
    }
}

impl NodeImpl for Latch {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Float(0.0)])
    }

    fn initial_state(&self) -> Option<Value> {
        Some(Value::Float(0.0))
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let Some((_, event)) = arrived(call) else {
            return Ok(());
        };
        let held = match event.payload.and_then(|p| p.coerce(PortType::Float)) {
            Some(Value::Float(v)) => v,
            _ => float(call.inputs(), 1, "value")?,
        };
        call.set_state(Some(Value::Float(held)));
        Ok(())
    }

    fn cook_with_state(
        &self,
        _ctx: &CookContext,
        _system: &SystemContext<'_>,
        state: Option<&Value>,
        _inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        Ok(vec![state.cloned().unwrap_or(Value::Float(0.0))])
    }
}

/// `core.event.gate@1`: forwards events while open.
///
/// Inputs: `trigger` (Event), `open` (Bool, default true). Output: `out`
/// (Event): each `trigger` event, payload included, while `open` is true.
/// Stateless.
#[derive(Debug)]
pub struct Gate {
    spec: NodeSpec,
}

impl Gate {
    #[must_use]
    pub fn new() -> Self {
        Gate {
            spec: NodeSpec::new("core.event.gate", 1, "Forward trigger events while open")
                .input("trigger", PortType::Event, Some(Value::Event))
                .input("open", PortType::Bool, Some(Value::Bool(true)))
                .output("out", PortType::Event),
        }
    }
}

impl NodeImpl for Gate {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Event])
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let open = bool_input(call.inputs(), 1, "open")?;
        if let Some((_, event)) = arrived(call) {
            if open {
                call.emit(0, event);
            }
        }
        Ok(())
    }
}

/// `core.event.select@1`: routes events by a condition.
///
/// Inputs: `trigger` (Event), `cond` (Bool, default false). Outputs:
/// `if_false`, `if_true` (Event): each `trigger` event, payload included,
/// goes to `if_true` when `cond` is true, else to `if_false`. Stateless.
#[derive(Debug)]
pub struct EventSelect {
    spec: NodeSpec,
}

impl EventSelect {
    #[must_use]
    pub fn new() -> Self {
        EventSelect {
            spec: NodeSpec::new(
                "core.event.select",
                1,
                "Route trigger events to if_true or if_false",
            )
            .input("trigger", PortType::Event, Some(Value::Event))
            .input("cond", PortType::Bool, Some(Value::Bool(false)))
            .output("if_false", PortType::Event)
            .output("if_true", PortType::Event),
        }
    }
}

impl NodeImpl for EventSelect {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, _inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Event, Value::Event])
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let cond = bool_input(call.inputs(), 1, "cond")?;
        if let Some((_, event)) = arrived(call) {
            call.emit(usize::from(cond), event);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(time: f64, dt: f64) -> CookContext {
        CookContext { frame: 0, time, dt }
    }

    /// Run a handler; returns the new state and emitted events.
    fn handle(
        node: &dyn NodeImpl,
        clock: CookContext,
        trigger: EventTrigger<'_>,
        inputs: &[Value],
        state: Option<Value>,
    ) -> (Option<Value>, Vec<(usize, Event)>) {
        let mut call = EventCall::new(&clock, trigger, inputs, state);
        node.on_event(&mut call).unwrap();
        call.finish()
    }

    fn on(port: usize, event: &Event) -> EventTrigger<'_> {
        EventTrigger::Input { port, event }
    }

    #[test]
    fn every_fires_on_crossings() {
        assert_eq!(crossings(0.0, 1.0 / 60.0, 1.0, 0.0).unwrap(), 0..=0);
        assert!(crossings(0.5, 1.0 / 60.0, 1.0, 0.0).unwrap().is_empty());
        assert_eq!(crossings(1.0, 1.0 / 60.0, 1.0, 0.0).unwrap(), 1..=1);
        assert_eq!(crossings(1.0, 0.35, 0.1, 0.0).unwrap(), 7..=10);
        assert!(crossings(1.0, 0.0, 1.0, 0.0).unwrap().is_empty());
        assert_eq!(
            crossings(1e6, 1e6, 1e-3, 0.0).unwrap().count(),
            usize::try_from(MAX_CROSSINGS_PER_TICK).unwrap()
        );
        let node = Every::new();
        let inputs = [Value::Float(0.5), Value::Float(0.25)];
        let (state, out) = handle(&node, ctx(0.75, 0.25), EventTrigger::Tick, &inputs, None);
        assert_eq!(state, None);
        assert_eq!(out, vec![(0, Event::with(Value::Int(1)))]);
        let zero = CookContext::default();
        let mut call = EventCall::new(
            &zero,
            EventTrigger::Tick,
            &[Value::Float(0.0), Value::Float(0.0)],
            None,
        );
        assert!(node.on_event(&mut call).is_err());
        assert!(node.polls_events());
    }

    #[test]
    fn on_change_fires_after_the_first_tick() {
        let node = OnChange::new();
        let c = ctx(0.0, 0.0);
        let (s, out) = handle(&node, c, EventTrigger::Tick, &[Value::Float(1.0)], None);
        assert!(out.is_empty());
        let (s, out) = handle(&node, c, EventTrigger::Tick, &[Value::Float(1.0)], s);
        assert!(out.is_empty());
        let (s, out) = handle(&node, c, EventTrigger::Tick, &[Value::Float(2.0)], s);
        assert_eq!(out, vec![(0, Event::with(Value::Float(2.0)))]);
        let (s, _) = handle(&node, c, EventTrigger::Tick, &[Value::Float(f64::NAN)], s);
        let (_, out) = handle(&node, c, EventTrigger::Tick, &[Value::Float(f64::NAN)], s);
        assert!(out.is_empty());
    }

    #[test]
    fn counter_toggle_latch_keep_state() {
        let c = ctx(0.0, 0.0);
        let bang = Event::bang();
        let counter = Counter::new();
        let ins = [Value::Event, Value::Event];
        let (s, _) = handle(&counter, c, on(0, &bang), &ins, counter.initial_state());
        let (s, _) = handle(&counter, c, on(0, &bang), &ins, s);
        assert_eq!(s, Some(Value::Int(2)));
        let none = SystemContext::none();
        assert_eq!(
            counter
                .cook_with_state(&c, &none, s.as_ref(), &ins)
                .unwrap(),
            vec![Value::Int(2)]
        );
        let (s, _) = handle(&counter, c, on(1, &bang), &ins, s);
        assert_eq!(s, Some(Value::Int(0)));

        let toggle = Toggle::new();
        let (s, _) = handle(&toggle, c, on(0, &bang), &[Value::Event], None);
        assert_eq!(s, Some(Value::Bool(true)));
        let (s, _) = handle(&toggle, c, on(0, &bang), &[Value::Event], s);
        assert_eq!(s, Some(Value::Bool(false)));

        let latch = Latch::new();
        let ins = [Value::Event, Value::Float(4.0)];
        let (s, _) = handle(&latch, c, on(0, &bang), &ins, None);
        assert_eq!(s, Some(Value::Float(4.0)));
        let five = Event::with(Value::Int(5));
        let (s, _) = handle(&latch, c, on(0, &five), &ins, s);
        assert_eq!(
            latch.cook_with_state(&c, &none, s.as_ref(), &ins).unwrap(),
            vec![Value::Float(5.0)]
        );
    }

    #[test]
    fn gate_and_select_route() {
        let c = ctx(0.0, 0.0);
        let ev = Event::with(Value::Int(7));
        let gate = Gate::new();
        let (_, out) = handle(
            &gate,
            c,
            on(0, &ev),
            &[Value::Event, Value::Bool(true)],
            None,
        );
        assert_eq!(out, vec![(0, ev.clone())]);
        let (_, out) = handle(
            &gate,
            c,
            on(0, &ev),
            &[Value::Event, Value::Bool(false)],
            None,
        );
        assert!(out.is_empty());
        let select = EventSelect::new();
        let (_, out) = handle(
            &select,
            c,
            on(0, &ev),
            &[Value::Event, Value::Bool(true)],
            None,
        );
        assert_eq!(out, vec![(1, ev.clone())]);
        let (_, out) = handle(
            &select,
            c,
            on(0, &ev),
            &[Value::Event, Value::Bool(false)],
            None,
        );
        assert_eq!(out, vec![(0, ev)]);
    }
}
