// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Events (M0.10): event links, `Cooker::tick`, delivery order, bounds,
//! state and caching, and a randomized differential test (a long-lived
//! cooker against replays from scratch).

mod common;

use std::sync::Arc;

use livspeil_graph::{
    CookError, CookStats, Cooker, ExternalEvent, Graph, GraphError, NodeId, PortDirection,
    TickReport,
};
use livspeil_node_api::{
    CookContext, Event, EventCall, NodeError, NodeImpl, NodeRegistry, NodeSpec, PortType, Value,
};

const ADD: &str = "core.math.add@1";
const EVERY: &str = "core.event.every@1";
const COUNTER: &str = "core.event.counter@1";
const TOGGLE: &str = "core.event.toggle@1";
const GATE: &str = "core.event.gate@1";
const LATCH: &str = "core.event.latch@1";
const ON_CHANGE: &str = "core.event.on_change@1";
const SELECT: &str = "core.event.select@1";

/// Emits on `out` once per event it gets, then fails or panics on demand:
/// payload `Int(-1)` fails, `Int(-2)` panics, `Int(-3)` emits from a value
/// port; `Int(n > 0)` emits `n` events.
#[derive(Debug)]
struct Burst {
    spec: NodeSpec,
}

impl Burst {
    fn new() -> Self {
        Burst {
            spec: NodeSpec::new("test.burst", 1, "Emits n events")
                .input("trigger", PortType::Event, Some(Value::Event))
                .output("out", PortType::Event)
                .output("seen", PortType::Int),
        }
    }
}

impl NodeImpl for Burst {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _: &CookContext, _: &[Value]) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Event, Value::Int(0)])
    }

    fn initial_state(&self) -> Option<Value> {
        Some(Value::Int(0))
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        let n = match call.trigger() {
            livspeil_node_api::EventTrigger::Input { event, .. } => match event.payload {
                Some(Value::Int(n)) => n,
                _ => 1,
            },
            livspeil_node_api::EventTrigger::Tick => 0,
        };
        let seen = match call.state() {
            Some(Value::Int(s)) => *s,
            _ => 0,
        };
        call.set_state(Some(Value::Int(seen + 1)));
        match n {
            -1 => return Err(NodeError::Failed("burst refused".to_owned())),
            -2 => panic!("burst panicked"),
            -3 => call.emit(1, Event::bang()),
            _ => {
                for _ in 0..n {
                    call.emit(0, Event::with(Value::Int(n)));
                }
            }
        }
        Ok(())
    }

    fn cook_with_state(
        &self,
        _: &CookContext,
        _: &livspeil_node_api::SystemContext<'_>,
        state: Option<&Value>,
        _: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        Ok(vec![Value::Event, state.cloned().unwrap_or(Value::Int(0))])
    }
}

fn registry() -> Arc<NodeRegistry> {
    let mut registry = NodeRegistry::new();
    livspeil_nodes_core::register_all(&mut registry).unwrap();
    registry.register(Arc::new(Burst::new())).unwrap();
    Arc::new(registry)
}

fn graph() -> Graph {
    Graph::new(registry())
}

fn id(n: u128) -> NodeId {
    NodeId::from_uuid(uuid::Uuid::from_u128(n))
}

fn clock(frame: u64) -> CookContext {
    CookContext {
        frame,
        time: frame as f64 * 0.25,
        dt: 0.25,
    }
}

fn ext(g: &Graph, node: NodeId, port: &str) -> ExternalEvent {
    ExternalEvent::new(g, node, port, Event::bang()).unwrap()
}

fn int(cooker: &mut Cooker, g: &Graph, node: NodeId, port: &str) -> i64 {
    match cooker.cook_output(g, node, port).unwrap() {
        Value::Int(i) => i,
        other => panic!("not an Int: {other:?}"),
    }
}

fn float(cooker: &mut Cooker, g: &Graph, node: NodeId, port: &str) -> f64 {
    cooker
        .cook_output(g, node, port)
        .unwrap()
        .as_float()
        .unwrap()
}

#[test]
fn event_links_are_type_checked_may_cycle_and_round_trip() {
    let mut g = graph();
    let every = g.add_node(EVERY, "every").unwrap();
    let gate = g.add_node(GATE, "gate").unwrap();
    let add = g.add_node(ADD, "add").unwrap();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    let mismatch = |expected, found| GraphError::TypeMismatch { expected, found };
    assert_eq!(
        g.link(every, "fired", add, "a"),
        Err(mismatch(PortType::Float, PortType::Event))
    );
    assert_eq!(
        g.link(add, "out", gate, "trigger"),
        Err(mismatch(PortType::Event, PortType::Float))
    );
    g.link(every, "fired", gate, "trigger").unwrap();
    // An event loop is allowed: events are bounded per tick instead.
    g.unlink(gate, "trigger").unwrap();
    g.link(gate, "out", gate, "trigger").unwrap();
    g.link(gate, "out", counter, "trigger").unwrap();
    g.link(every, "fired", counter, "reset").unwrap();
    // Value links still refuse cycles, and event links do not count.
    g.link(add, "out", add, "b").unwrap_err();
    assert_eq!(g.link_count(), 3);
    let snap = g.snapshot();
    assert_eq!(snap.links.len(), 3);
    assert_eq!(
        g.link_source(counter, "trigger").unwrap(),
        Some((gate, "out"))
    );
    assert!(g.unlink(counter, "reset").unwrap());
    assert!(!g.unlink(counter, "reset").unwrap());
    g.remove_node(gate).unwrap();
    assert_eq!(g.link_count(), 0);
    // Event inputs cannot take a value.
    assert!(g.set_input(counter, "trigger", Value::Float(1.0)).is_err());
}

#[test]
fn external_events_resolve_ports() {
    let mut g = graph();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    let every = g.add_node(EVERY, "every").unwrap();
    let e = ExternalEvent::new(&g, counter, "reset", Event::bang()).unwrap();
    assert_eq!((e.port, e.direction), (1, PortDirection::Input));
    let e = ExternalEvent::new(&g, every, "fired", Event::bang()).unwrap();
    assert_eq!((e.port, e.direction), (0, PortDirection::Output));
    assert!(matches!(
        ExternalEvent::new(&g, counter, "count", Event::bang()),
        Err(GraphError::TypeMismatch { .. })
    ));
    assert!(matches!(
        ExternalEvent::new(&g, counter, "nope", Event::bang()),
        Err(GraphError::UnknownPort { .. })
    ));
    assert!(matches!(
        ExternalEvent::new(&g, NodeId::new(), "reset", Event::bang()),
        Err(GraphError::UnknownNode(_))
    ));
}

/// every (1 s) -> counter -> add (+ 10); toggled by an external event.
#[test]
fn a_timer_drives_a_counter_and_values_stay_cached() {
    let mut g = graph();
    let every = g.add_node(EVERY, "every").unwrap();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    let add = g.add_node(ADD, "add").unwrap();
    g.link(every, "fired", counter, "trigger").unwrap();
    g.link(counter, "count", add, "a").unwrap();
    g.set_input(add, "b", Value::Float(10.0)).unwrap();

    let mut c = Cooker::new();
    let mut counts = Vec::new();
    for frame in 0..12 {
        c.reset_stats();
        let report = c.tick(&g, clock(frame), &[]);
        let fired = report.fired_at(every, 0).count();
        let before = c.stats();
        let value = float(&mut c, &g, add, "out");
        let cook = CookStats {
            cooked: c.stats().cooked - before.cooked,
            reused: c.stats().reused - before.reused,
        };
        counts.push((fired, value));
        // The value cook re-cooks counter and add only after a change.
        if frame > 0 {
            assert_eq!(cook.cooked, if fired > 0 { 2 } else { 0 }, "{frame}");
        }
    }
    // t = 0, 1, 2 at frames 0, 4, 8.
    assert_eq!(
        counts.iter().map(|(f, _)| *f).collect::<Vec<_>>(),
        [1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]
    );
    assert_eq!(counts[11].1, 13.0);
    assert_eq!(c.node_state(&g, counter), Some(&Value::Int(3)));
    // A new cooker starts from the initial state; the graph holds none.
    let mut fresh = Cooker::new();
    assert_eq!(float(&mut fresh, &g, add, "out"), 10.0);
    assert_eq!(fresh.node_state(&g, counter), None);

    // Reset from outside.
    let report = c.tick(&g, clock(12), &[ext(&g, counter, "reset")]);
    assert_eq!(report.fired_at(every, 0).count(), 1);
    assert_eq!(int(&mut c, &g, counter, "count"), 1); // reset, then t = 3 fires
}

#[test]
fn edits_and_events_mix_without_stale_values() {
    let mut g = graph();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    let add = g.add_node(ADD, "add").unwrap();
    let sink = g.add_node(ADD, "sink").unwrap();
    g.link(counter, "count", add, "a").unwrap();
    g.link(add, "out", sink, "a").unwrap();
    let mut c = Cooker::new();
    assert_eq!(float(&mut c, &g, sink, "out"), 0.0);
    c.tick(&g, clock(1), &[ext(&g, counter, "trigger")]);
    g.set_input(add, "b", Value::Float(5.0)).unwrap();
    assert_eq!(float(&mut c, &g, sink, "out"), 6.0);
    c.tick(&g, clock(2), &[ext(&g, counter, "trigger")]);
    c.tick(&g, clock(3), &[ext(&g, counter, "trigger")]);
    assert_eq!(float(&mut c, &g, sink, "out"), 8.0);
    // Trigger then reset in one tick: the state ends where it started, but
    // it changed twice, so values are re-validated and stay right.
    c.tick(
        &g,
        clock(4),
        &[ext(&g, counter, "trigger"), ext(&g, counter, "reset")],
    );
    assert_eq!(float(&mut c, &g, sink, "out"), 5.0);
    // Removing the node drops its state; a new node in the slot starts over.
    g.remove_node(counter).unwrap();
    let again = g.add_node(COUNTER, "counter").unwrap();
    g.link(again, "count", add, "a").unwrap();
    assert_eq!(c.node_state(&g, again), None);
    assert_eq!(float(&mut c, &g, sink, "out"), 5.0);
}

#[test]
fn cookers_keep_their_own_state() {
    let mut g = graph();
    let toggle = g.add_node(TOGGLE, "toggle").unwrap();
    let mut a = Cooker::new();
    let mut b = Cooker::new();
    a.tick(&g, clock(0), &[ext(&g, toggle, "trigger")]);
    b.tick(&g, clock(0), &[]);
    assert_eq!(a.cook_output(&g, toggle, "on").unwrap(), Value::Bool(true));
    assert_eq!(b.cook_output(&g, toggle, "on").unwrap(), Value::Bool(false));
    // A cooker used with another graph starts over.
    let other = graph();
    a.tick(&other, clock(0), &[]);
    assert_eq!(a.cook_output(&g, toggle, "on").unwrap(), Value::Bool(false));
}

#[test]
fn delivery_order_is_root_then_emission_then_target_id() {
    let mut g = graph();
    // Ids chosen so that creation order and id order differ.
    let src = g.add_node_with_id(id(50), GATE, "src").unwrap();
    let late = g.add_node_with_id(id(30), GATE, "late").unwrap();
    let early = g.add_node_with_id(id(20), GATE, "early").unwrap();
    let last = g.add_node_with_id(id(10), LATCH, "last").unwrap();
    g.link(src, "out", late, "trigger").unwrap();
    g.link(src, "out", early, "trigger").unwrap();
    g.link(early, "out", last, "trigger").unwrap();
    g.link(late, "out", last, "value").unwrap_err();
    let mut c = Cooker::new();
    let payload = |v: i64| ExternalEvent::new(&g, src, "trigger", Event::with(Value::Int(v)));
    let report = c.tick(&g, clock(0), &[payload(1).unwrap(), payload(2).unwrap()]);
    let order: Vec<(NodeId, Option<Value>)> = report
        .fired
        .iter()
        .map(|f| (f.node, f.event.payload.clone()))
        .collect();
    let p = |v| Some(Value::Int(v));
    assert_eq!(
        order,
        [
            (src, p(1)),
            (early, p(1)),
            (late, p(1)),
            (src, p(2)),
            (early, p(2)),
            (late, p(2)),
        ]
    );
    // The latch saw 1 then 2.
    assert_eq!(float(&mut c, &g, last, "out"), 2.0);
    assert!(report.errors.is_empty());
    assert!(!report.overflowed());
}

#[test]
fn event_loops_are_cut_at_the_limit_and_reported() {
    let mut g = graph();
    let gate = g.add_node(GATE, "gate").unwrap();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    g.link(gate, "out", gate, "trigger").unwrap();
    g.link(gate, "out", counter, "trigger").unwrap();
    let mut c = Cooker::new();
    c.set_event_limit(101);
    assert_eq!(c.event_limit(), 101);
    let report = c.tick(&g, clock(0), &[ext(&g, gate, "trigger")]);
    assert!(report.overflowed());
    assert_eq!(report.processed, 101);
    assert!(report.dropped > 0);
    // deliver, emit, deliver x2, emit, ...: 1 + 3k items for k emissions.
    assert_eq!(report.fired.len(), 34);
    assert_eq!(int(&mut c, &g, counter, "count"), 33);
    // The next tick starts empty.
    let quiet = c.tick(&g, clock(1), &[]);
    assert_eq!(quiet, TickReport::default());
    // Closing the gate stops the loop.
    g.set_input(gate, "open", Value::Bool(false)).unwrap();
    let report = c.tick(&g, clock(2), &[ext(&g, gate, "trigger")]);
    assert!(!report.overflowed());
    assert_eq!(report.processed, 1);
    c.set_event_limit(0);
    assert_eq!(c.event_limit(), 1);
}

#[test]
fn fan_out_explosions_are_bounded() {
    // A chain of bursts, each emitting 8 events per event: 8^6 deliveries
    // without a limit.
    let mut g = graph();
    let nodes: Vec<NodeId> = (0..6)
        .map(|i| g.add_node("test.burst@1", &format!("b{i}")).unwrap())
        .collect();
    for pair in nodes.windows(2) {
        g.link(pair[0], "out", pair[1], "trigger").unwrap();
    }
    let mut c = Cooker::new();
    let start = ExternalEvent::new(&g, nodes[0], "trigger", Event::with(Value::Int(8))).unwrap();
    let report = c.tick(&g, clock(0), &[start]);
    assert_eq!(report.processed, livspeil_graph::DEFAULT_EVENT_LIMIT);
    assert!(report.overflowed());
}

#[test]
fn failing_handlers_keep_state_and_emit_nothing() {
    let mut g = graph();
    let burst = g.add_node("test.burst@1", "burst").unwrap();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    g.link(burst, "out", counter, "trigger").unwrap();
    let mut c = Cooker::new();
    let with =
        |v: i64| ExternalEvent::new(&g, burst, "trigger", Event::with(Value::Int(v))).unwrap();
    let report = c.tick(&g, clock(0), &[with(2), with(-1), with(-2), with(-3)]);
    assert_eq!(report.errors.len(), 3);
    assert!(matches!(report.errors[0].error, CookError::Node(_)));
    assert!(matches!(report.errors[1].error, CookError::Panicked(_)));
    assert!(matches!(report.errors[2].error, CookError::BadOutput(_)));
    assert!(report.errors.iter().all(|e| e.node == burst));
    // Only the successful call changed the state and emitted.
    assert_eq!(int(&mut c, &g, burst, "seen"), 1);
    assert_eq!(int(&mut c, &g, counter, "count"), 2);
}

#[test]
fn polls_see_the_values_set_by_earlier_events() {
    let mut g = graph();
    let counter = g.add_node(COUNTER, "counter").unwrap();
    let watch = g.add_node(ON_CHANGE, "watch").unwrap();
    let latch = g.add_node(LATCH, "latch").unwrap();
    g.link(counter, "count", watch, "value").unwrap();
    g.link(watch, "changed", latch, "trigger").unwrap();
    let mut c = Cooker::new();
    assert!(c.tick(&g, clock(0), &[]).fired.is_empty());
    let report = c.tick(&g, clock(1), &[ext(&g, counter, "trigger")]);
    assert_eq!(
        report.fired_at(watch, 0).cloned().collect::<Vec<_>>(),
        [Event::with(Value::Float(1.0))]
    );
    assert_eq!(float(&mut c, &g, latch, "out"), 1.0);
    assert!(c.tick(&g, clock(2), &[]).fired.is_empty());
}

#[test]
fn events_cross_subnet_interfaces() {
    let mut g = graph();
    let every = g.add_node(EVERY, "every").unwrap();
    let sub = g.add_node("core.subnet@1", "sub").unwrap();
    let input = g
        .add_node_in(
            Some(sub),
            NodeId::new(),
            "core.subnet.input@1",
            "tick",
            Some(PortType::Event),
        )
        .unwrap();
    let counter = g
        .add_node_in(Some(sub), NodeId::new(), COUNTER, "counter", None)
        .unwrap();
    let output = g
        .add_node_in(
            Some(sub),
            NodeId::new(),
            "core.output@1",
            "relay",
            Some(PortType::Event),
        )
        .unwrap();
    let top = g
        .add_node_in(
            None,
            NodeId::new(),
            "core.output@1",
            "seen",
            Some(PortType::Event),
        )
        .unwrap();
    g.link(every, "fired", input, "value").unwrap();
    g.link(input, "value", counter, "trigger").unwrap();
    g.link(input, "value", output, "value").unwrap();
    g.link(output, "value", top, "value").unwrap();
    let mut c = Cooker::new();
    let report = c.tick(&g, clock(0), &[]);
    assert_eq!(report.fired_at(top, 0).count(), 1);
    assert_eq!(report.fired_at(input, 0).count(), 1);
    assert_eq!(int(&mut c, &g, counter, "count"), 1);
    // Cooking an event-typed interface node yields the placeholder.
    assert_eq!(c.cook_output(&g, top, "value").unwrap(), Value::Event);
}

#[test]
fn ticks_without_event_nodes_do_nothing() {
    let mut g = graph();
    let add = g.add_node(ADD, "add").unwrap();
    let mut c = Cooker::new();
    c.cook(&g, add).unwrap();
    c.reset_stats();
    assert_eq!(c.tick(&g, clock(3), &[]), TickReport::default());
    assert_eq!(c.stats(), CookStats::default());
    assert_eq!(c.clock(), clock(3));
}

/// The same scenario twice, in two graphs built the same way, gives the
/// same reports and values.
#[test]
fn ticks_are_deterministic() {
    let run = || {
        let (g, script) = random_scenario(0xDEC0_DE00_0000_0001);
        let mut c = Cooker::new();
        let mut log = Vec::new();
        for (frame, step) in script.iter().enumerate() {
            let report = c.tick(&g, clock(frame as u64), &step.events(&g));
            log.push(format!("{report:?}"));
            for &n in &g.node_ids().collect::<Vec<_>>() {
                log.push(format!("{:?}", c.cook(&g, n)));
            }
        }
        log
    };
    assert_eq!(run(), run());
}

// --- Randomized differential test ------------------------------------------

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// One frame of a scenario: value edits, external events (by node name and
/// port), and which nodes the long-lived cooker cooks.
#[derive(Clone, Debug, Default)]
struct Step {
    edits: Vec<(String, &'static str, Value)>,
    events: Vec<(String, &'static str)>,
    cook: Vec<String>,
}

impl Step {
    fn events(&self, g: &Graph) -> Vec<ExternalEvent> {
        self.events
            .iter()
            .map(|(n, p)| ext(g, g.find(n).unwrap(), p))
            .collect()
    }
}

/// A random event-driven graph (built deterministically from `seed`, with
/// fixed ids) and a 40-frame script.
fn random_scenario(seed: u64) -> (Graph, Vec<Step>) {
    let mut rng = SplitMix64(seed);
    let mut g = graph();
    let kinds = [
        EVERY, COUNTER, TOGGLE, GATE, LATCH, ON_CHANGE, SELECT, ADD, ADD,
    ];
    let n = 8 + rng.below(10);
    let mut nodes = Vec::new();
    for i in 0..n {
        let kind = kinds[rng.below(kinds.len())];
        let name = format!("n{i}");
        let node = g
            .add_node_with_id(
                id(1000 + (rng.next() % 1_000_000) as u128 * 32 + i as u128),
                kind,
                &name,
            )
            .unwrap();
        if kind == EVERY {
            let period = [0.25, 0.5, 0.75, 1.0][rng.below(4)];
            g.set_input(node, "period", Value::Float(period)).unwrap();
        }
        nodes.push((node, kind, name));
    }
    // Random links: event links anywhere (loops allowed), value links only
    // forward (so most succeed).
    for _ in 0..n * 2 {
        let from = nodes[rng.below(n)].0;
        let to = nodes[rng.below(n)].0;
        let outs = g.node_spec(from).unwrap().outputs.clone();
        let ins = g.node_spec(to).unwrap().inputs.clone();
        let out = &outs[rng.below(outs.len())];
        if ins.is_empty() {
            continue;
        }
        let input = &ins[rng.below(ins.len())];
        let _ = g.link(from, out.name, to, input.name);
    }
    let names: Vec<String> = nodes.iter().map(|(_, _, n)| n.clone()).collect();
    let mut script = Vec::new();
    for _ in 0..40 {
        let mut step = Step::default();
        for _ in 0..rng.below(3) {
            let (node, _, name) = &nodes[rng.below(n)];
            let spec = g.node_spec(*node).unwrap();
            let events: Vec<&'static str> = spec
                .inputs
                .iter()
                .filter(|p| p.ty == PortType::Event)
                .map(|p| p.name)
                .collect();
            if !events.is_empty() {
                step.events
                    .push((name.clone(), events[rng.below(events.len())]));
            }
        }
        if rng.below(3) == 0 {
            let (node, _, name) = &nodes[rng.below(n)];
            let spec = g.node_spec(*node).unwrap();
            for p in &spec.inputs {
                let value = match p.ty {
                    PortType::Float => Value::Float(rng.below(4) as f64 * 0.5),
                    PortType::Bool => Value::Bool(rng.below(2) == 0),
                    _ => continue,
                };
                if p.name != "period" && g.link_source(*node, p.name).unwrap().is_none() {
                    step.edits.push((name.clone(), p.name, value));
                    break;
                }
            }
        }
        for name in &names {
            if rng.below(4) == 0 {
                step.cook.push(name.clone());
            }
        }
        script.push(step);
    }
    (g, script)
}

fn apply_edits(g: &mut Graph, step: &Step) {
    for (name, port, value) in &step.edits {
        g.set_input(g.find(name).unwrap(), port, value.clone())
            .unwrap();
    }
}

/// Long-lived cooker (lazy, partial cooks, edits between ticks) against a
/// replay from scratch per checked frame (fresh graph and cooker whose
/// value caches are cleared before every tick, so nothing relies on
/// invalidation): the fired events and every cooked value agree.
#[test]
fn long_lived_cooker_matches_replay_from_scratch() {
    let (mut fired, mut overflows, mut event_links) = (0, 0, 0);
    for seed in 0..12_u64 {
        let seed = 0x5EED_0000_0000_0000 ^ seed.wrapping_mul(0x9E37_79B9);
        let (mut g, script) = random_scenario(seed);
        let mut live = Cooker::new();
        live.set_event_limit(500);
        for (frame, step) in script.iter().enumerate() {
            apply_edits(&mut g, step);
            let report = live.tick(&g, clock(frame as u64), &step.events(&g));
            fired += report.fired.len();
            overflows += usize::from(report.overflowed());
            if frame == 0 {
                event_links += g
                    .links()
                    .filter(|l| {
                        g.node_spec(l.to.node).unwrap().inputs[l.to.port].ty == PortType::Event
                    })
                    .count();
            }
            let cooked: Vec<(String, Result<Vec<Value>, CookError>)> = step
                .cook
                .iter()
                .map(|n| (n.clone(), live.cook(&g, g.find(n).unwrap())))
                .collect();
            // Replay frames 0..=frame from scratch, cooking everything.
            if frame % 7 != 6 && frame != script.len() - 1 {
                continue;
            }
            let (mut fresh_g, _) = random_scenario(seed);
            let mut fresh = Cooker::new();
            fresh.set_event_limit(500);
            let mut fresh_report = None;
            for (f, s) in script.iter().enumerate().take(frame + 1) {
                apply_edits(&mut fresh_g, s);
                // No cached value survives into a tick: handlers and polls
                // see values computed from scratch.
                fresh.clear_values();
                fresh_report = Some(fresh.tick(&fresh_g, clock(f as u64), &s.events(&fresh_g)));
                let ids: Vec<NodeId> = fresh_g.node_ids().collect();
                for n in ids {
                    let _ = fresh.cook(&fresh_g, n);
                }
            }
            assert_eq!(
                Some(&report),
                fresh_report.as_ref(),
                "seed {seed:x} frame {frame}"
            );
            for (name, value) in cooked {
                let expected = fresh.cook(&fresh_g, fresh_g.find(&name).unwrap());
                assert_eq!(value, expected, "seed {seed:x} frame {frame} node {name}");
            }
            for name in g
                .node_ids()
                .map(|n| g.node_name(n).unwrap().to_owned())
                .collect::<Vec<_>>()
            {
                let a = live.cook(&g, g.find(&name).unwrap());
                let b = fresh.cook(&fresh_g, fresh_g.find(&name).unwrap());
                assert_eq!(a, b, "seed {seed:x} frame {frame} node {name}");
            }
        }
    }
    // The scenarios exercise what they claim to.
    eprintln!("fired {fired}, overflows {overflows}, event links {event_links}");
    assert!(fired > 100 && event_links > 12, "{fired} {event_links}");
}
