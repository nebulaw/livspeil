// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests for the event nodes (M0.10).
//!
//! Each case drives one node through a sequence of steps, keeping its state
//! between steps as a cooker would. A step is a poll (`tick`) or an event
//! arriving at an input (`<port>!` or `<port>!<payload>`), with the clock
//! and any value inputs that differ from the spec defaults. Each line
//! reads `<node> t=<time> dt=<dt> <trigger> [inputs] => <emitted> | <value
//! outputs>`, where emitted events print as `<port>!` or
//! `<port>!<payload>` (or `-` for none). Value outputs are cooked from the
//! state after the step.
//!
//! Refresh with
//! `UPDATE_GOLDEN=1 cargo test -p livspeil-nodes-core --test golden_event`
//! and review the diff by hand.

use std::fmt::Write as _;
use std::path::Path;

use livspeil_node_api::{CookContext, Event, EventCall, EventTrigger, SystemContext, Value};
use livspeil_nodes_core::core_registry;
use livspeil_testkit::assert_golden;

fn show(value: &Value) -> String {
    match value {
        Value::Float(v) => format!("{v:?}"),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        other => format!("{other:?}"),
    }
}

fn show_event(port: &str, event: &Event) -> String {
    match &event.payload {
        Some(p) => format!("{port}!{}", show(p)),
        None => format!("{port}!"),
    }
}

/// One step: time, dt, the trigger (`None`: poll; `Some((port, payload))`:
/// an event at that input) and value input overrides.
type Step<'a> = (
    f64,
    f64,
    Option<(&'a str, Option<Value>)>,
    Vec<(&'a str, Value)>,
);

fn run(out: &mut String, id: &str, steps: &[Step<'_>]) {
    let registry = core_registry();
    let node = registry
        .get(&format!("{id}@1"))
        .unwrap_or_else(|| panic!("{id} is not registered"));
    let spec = node.spec();
    let mut state = node.initial_state();
    for (time, dt, trigger, overrides) in steps {
        let mut inputs: Vec<Value> = spec
            .inputs
            .iter()
            .map(|p| p.default.clone().unwrap())
            .collect();
        for (port, value) in overrides {
            inputs[spec.input_index(port).unwrap()] = value.clone();
        }
        let ctx = CookContext {
            frame: 0,
            time: *time,
            dt: *dt,
        };
        let event;
        let (trigger, label) = match trigger {
            None => (EventTrigger::Tick, "tick".to_owned()),
            Some((port, payload)) => {
                event = Event {
                    payload: payload.clone(),
                };
                let index = spec.input_index(port).unwrap();
                (
                    EventTrigger::Input {
                        port: index,
                        event: &event,
                    },
                    show_event(port, &event),
                )
            }
        };
        let args: Vec<String> = overrides
            .iter()
            .map(|(p, v)| format!("{p}={}", show(v)))
            .collect();
        let mut call = EventCall::new(&ctx, trigger, &inputs, state.clone());
        let line = match node.on_event(&mut call) {
            Ok(()) => {
                let (next, emitted) = call.finish();
                state = next;
                let emitted: Vec<String> = emitted
                    .iter()
                    .map(|(port, ev)| show_event(spec.outputs[*port].name, ev))
                    .collect();
                let values = node
                    .cook_with_state(&ctx, &SystemContext::none(), state.as_ref(), &inputs)
                    .unwrap();
                let values: Vec<String> = spec
                    .outputs
                    .iter()
                    .zip(&values)
                    .filter(|(_, v)| **v != Value::Event)
                    .map(|(p, v)| format!("{}={}", p.name, show(v)))
                    .collect();
                format!(
                    "{} | {}",
                    if emitted.is_empty() {
                        "-".to_owned()
                    } else {
                        emitted.join(" ")
                    },
                    if values.is_empty() {
                        "-".to_owned()
                    } else {
                        values.join(" ")
                    }
                )
            }
            Err(err) => format!("error: {err}"),
        };
        writeln!(
            out,
            "{id} t={time:?} dt={dt:?} {label}{}{} => {line}",
            if args.is_empty() { "" } else { " " },
            args.join(" ")
        )
        .unwrap();
    }
}

fn poll<'a>(time: f64, dt: f64, inputs: Vec<(&'a str, Value)>) -> Step<'a> {
    (time, dt, None, inputs)
}

fn on<'a>(port: &'a str, payload: Option<Value>, inputs: Vec<(&'a str, Value)>) -> Step<'a> {
    (0.0, 0.0, Some((port, payload)), inputs)
}

#[test]
fn event_nodes_golden() {
    let mut out = String::new();
    let f = Value::Float;
    let b = Value::Bool;
    let dt = 0.25;

    // A 1 s timer over 2.5 s in quarter-second steps, from t = 0.
    let ticks: Vec<Step<'_>> = (0..=10)
        .map(|i| poll(f64::from(i) * dt, dt, vec![]))
        .collect();
    run(&mut out, "core.event.every", &ticks);
    // Offset 0.5, period 0.75; a step longer than the period fires twice.
    run(
        &mut out,
        "core.event.every",
        &[
            poll(0.5, 0.25, vec![("period", f(0.75)), ("offset", f(0.5))]),
            poll(1.0, 0.5, vec![("period", f(0.75)), ("offset", f(0.5))]),
            poll(2.75, 1.75, vec![("period", f(0.75)), ("offset", f(0.5))]),
            poll(1.0, 0.0, vec![("period", f(0.75)), ("offset", f(0.5))]),
            poll(1.0, 0.25, vec![("period", f(0.0))]),
            poll(1.0, 0.25, vec![("offset", f(f64::NAN))]),
        ],
    );

    run(
        &mut out,
        "core.event.on_change",
        &[
            poll(0.0, dt, vec![("value", f(1.0))]),
            poll(0.25, dt, vec![("value", f(1.0))]),
            poll(0.5, dt, vec![("value", f(1.5))]),
            poll(0.75, dt, vec![("value", f(1.5))]),
            poll(1.0, dt, vec![("value", f(-2.0))]),
            poll(1.25, dt, vec![("value", f(f64::NAN))]),
            poll(1.5, dt, vec![("value", f(f64::NAN))]),
            poll(1.75, dt, vec![("value", f(0.0))]),
        ],
    );

    run(
        &mut out,
        "core.event.counter",
        &[
            on("trigger", None, vec![]),
            on("trigger", Some(f(9.0)), vec![]),
            on("trigger", None, vec![]),
            on("reset", None, vec![]),
            on("trigger", None, vec![]),
        ],
    );

    run(
        &mut out,
        "core.event.toggle",
        &[
            on("trigger", None, vec![]),
            on("trigger", None, vec![]),
            on("trigger", Some(b(false)), vec![]),
        ],
    );

    run(
        &mut out,
        "core.event.latch",
        &[
            on("trigger", None, vec![("value", f(0.25))]),
            on("trigger", Some(f(3.5)), vec![("value", f(0.25))]),
            on("trigger", Some(Value::Int(7)), vec![]),
            on("trigger", Some(b(true)), vec![("value", f(-1.0))]),
        ],
    );

    run(
        &mut out,
        "core.event.gate",
        &[
            on("trigger", None, vec![]),
            on("trigger", Some(f(2.0)), vec![("open", b(true))]),
            on("trigger", Some(f(2.0)), vec![("open", b(false))]),
            poll(0.0, 0.0, vec![]),
        ],
    );

    run(
        &mut out,
        "core.event.select",
        &[
            on("trigger", None, vec![]),
            on("trigger", Some(Value::Int(4)), vec![("cond", b(true))]),
            on("trigger", Some(Value::Int(5)), vec![("cond", b(false))]),
        ],
    );

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    assert_golden(&dir, "event", &out);
}
