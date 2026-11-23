// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil cook`: cook a document over a deterministic frame clock and
//! print the outputs of its sinks.
//!
//! # Clock
//!
//! Frame `i` (counting from 0) cooks at `time = start + i / fps` seconds with
//! `dt = 1 / fps`. Every frame is cooked, in order, with one [`Cooker`], so
//! caching behaves as in a live session; only every `K`-th frame (`i % K ==
//! 0`) is printed.
//!
//! # Sinks
//!
//! When the root network has `core.output` nodes, they are the sinks and
//! only they are printed (each has one port, `value`). Otherwise the sinks
//! are the root network's nodes with no outgoing links, subnets excluded (a
//! node that is only read by another node's `ch()` expression still counts
//! as a sink). Nodes inside subnets are never sinks. `--node NAME`
//! (repeatable) prints the named nodes instead, sink or not; a node inside
//! a subnet is named by its path (`lamp/color`, printed that way). Nodes
//! are printed sorted by name, ports in the order of the node type's
//! spec.
//!
//! # Text format
//!
//! One line per printed frame and node:
//!
//! ```text
//! frame=0 time=0.000000 lamp_color.out=oklch(0.628704 0.065007 51.483430 1.000000)
//! frame=0 time=0.000000 lamp_intensity.out=0.792593
//! frame=0 time=0.000000 sun.direction=vec3(0.917409 0.264725 -0.297121) sun.elevation=15.350625 sun.azimuth=72.054432
//! ```
//!
//! Floats use exactly six decimals (`-0.000000` is written `0.000000`), so
//! the last-bit differences between platform math libraries do not show.
//! Values: Float `0.500000`, Int `3`, Bool `true`, `vec2(x y)`,
//! `vec3(x y z)`, `vec4(x y z w)`, Color `oklch(L C H A)`, String as a JSON
//! string literal. A failed node prints `<error: message>` for each port.
//! With `--stats`, each printed frame ends with
//! `frame=N time=T stats cooked=C reused=R`, the scheduler counters for that
//! frame alone ([`livspeil_graph::CookStats`]; a node shared by several
//! printed nodes is counted once per visit).
//!
//! Tables print as `table[<col>:<Type> ...][<cells of row 0>; <cells of
//! row 1>; ...]` (cells space-separated, in the value format above) and
//! curves as `curve(<position>:<value> ...)` (keys only).
//!
//! # Events
//!
//! Every frame is one tick of the cooker: input (the frame clock and the
//! events injected for that frame), events (delivered with
//! `Cooker::tick`), cook (the printed nodes), publish (the printed line).
//! `--event NODE.PORT@FRAME[=VALUE]` (repeatable) injects an event in
//! frame `FRAME` (counting from 0): delivered to the `Event` input `PORT`
//! of `NODE`, or emitted from its `Event` output `PORT` if it has no such
//! input. `VALUE`, if given, is the payload, written in the `room.kdl`
//! value syntax (`1.0`, `3`, `#true`, `"text"`, `(vec3)"1 2 3"`,
//! `(color)"oklch(0.8 0.1 60)"`; see `livspeil_doc::parse_value`); the
//! first `=` starts it. Events of one frame are injected in command-line
//! order.
//! Event node state is runtime only: every run starts from the initial
//! states.
//!
//! An `Event` port of a printed node shows the events it fired in that
//! frame: `none`, or a comma-separated list of `event` (no payload) and
//! `event(<value>)` (with payload, in the value format below), for example
//! `blink.fired=event(4)`. When the frame's events were cut at the event
//! limit, the frame ends with `frame=N time=T events overflow
//! processed=P dropped=D`; an event handler that failed adds
//! `frame=N time=T event-error <node>: <message>` and makes the exit code
//! 3, like a node error.
//!
//! # File system
//!
//! Nodes that read files (`core.fs.list_dir`) see the room package
//! directory (the folder holding `room.kdl`) as their file root, or the
//! directory given with `--fs-root DIR`. They cannot reach outside it
//! (see `livspeil_nodes_core::StdFs`). The listing is a snapshot taken
//! when first needed; it is not refreshed during a run, so every frame
//! sees the same files.
//!
//! # JSONL format
//!
//! One JSON object per printed frame, keys in this order:
//!
//! ```text
//! {"frame":0,"time":0.0,"outputs":{"<node>":{"<port>":{"<type>":<value>}}},"stats":{"cooked":9,"reused":0}}
//! ```
//!
//! `<type>` is `float`, `int`, `bool`, `string` (scalars) or `vec2`, `vec3`,
//! `vec4`, `color` (arrays; color is `[L, C, H, A]` in OKLCH, hue in
//! degrees). A table is `{"table":{"columns":[{"name":..,"type":..}],
//! "rows":[[<value>,..],..]}}` with types and untagged values as above; a
//! curve is `{"curve":[[<position>,<value>],..]}` (keys only).
//! A failed node is `{"error":"<message>"}` instead of its ports.
//! An `Event` port is `{"event":[<payload>,..]}`, one entry per event
//! fired in the frame (`[]` when none): `null` without payload, else the
//! tagged value as above. After `outputs`, `"events":{"processed":P,
//! "dropped":D}` is present only when the events were cut at the limit,
//! and `"event_errors":[{"node":..,"error":..}]` only when a handler
//! failed.
//! Floats are rounded to six decimals, like the text format, and written in
//! the shortest form that reads back as that value (`0.8`, `1.0`);
//! non-finite floats are `null`. `stats` is present only with `--stats`.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use livspeil_doc::parse;
use livspeil_graph::{CookError, Cooker, ExternalEvent, Graph, NodeId, OUTPUT_TYPE, TickReport};
use livspeil_node_api::{CookContext, Event, PortType, Value};
use livspeil_nodes_core::{StdFs, core_registry};

use crate::{read, report, usage};

/// Exit code when at least one printed value is a node error.
const EXIT_NODE_ERROR: u8 = 3;

/// Output format of `cook`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Jsonl,
}

/// Parsed `cook` arguments.
#[derive(Debug, Clone, PartialEq)]
struct Options {
    path: String,
    frames: u64,
    fps: f64,
    start: f64,
    every: u64,
    nodes: Vec<String>,
    format: Format,
    stats: bool,
    /// `--fs-root`: file root of file-system nodes (default: the package).
    fs_root: Option<String>,
    /// `--event NODE.PORT@FRAME`, in command-line order.
    events: Vec<EventArg>,
}

/// One `--event NODE.PORT@FRAME[=VALUE]`.
#[derive(Debug, Clone, PartialEq)]
struct EventArg {
    node: String,
    port: String,
    frame: u64,
    /// The payload, in the document value syntax.
    payload: Option<Value>,
}

impl EventArg {
    /// Parse the argument; the error says what is wrong.
    fn parse(text: &str) -> Result<Self, String> {
        const SHAPE: &str = "expected NODE.PORT@FRAME or NODE.PORT@FRAME=VALUE";
        let bad = || format!("invalid value '{text}' for --event ({SHAPE})");
        // Node names cannot contain '=', so the first one starts the value.
        let (head, payload) = match text.split_once('=') {
            Some((head, value)) => {
                let value = livspeil_doc::parse_value(value)
                    .map_err(|err| format!("--event {head}: invalid payload {err}"))?;
                (head, Some(value))
            }
            None => (text, None),
        };
        let (target, frame) = head.rsplit_once('@').ok_or_else(bad)?;
        let (node, port) = target.rsplit_once('.').ok_or_else(bad)?;
        if node.is_empty() || port.is_empty() {
            return Err(bad());
        }
        Ok(EventArg {
            node: node.to_owned(),
            port: port.to_owned(),
            frame: frame.parse().map_err(|_| bad())?,
            payload,
        })
    }
}

/// What a `cook` argument list asked for.
#[derive(Debug)]
enum Parsed {
    Help,
    Run(Options),
}

/// Entry point for `livspeil cook ARGS`.
pub fn run(args: impl Iterator<Item = String>) -> ExitCode {
    match parse_args(args) {
        Ok(Parsed::Help) => {
            print!("{}", crate::HELP);
            ExitCode::SUCCESS
        }
        Ok(Parsed::Run(options)) => cook(&options),
        Err(message) => usage(&format!("cook: {message}")),
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Parsed, String> {
    let mut path = None;
    let mut options = Options {
        path: String::new(),
        frames: 1,
        fps: 60.0,
        start: 0.0,
        every: 1,
        nodes: Vec::new(),
        format: Format::Text,
        stats: false,
        fs_root: None,
        events: Vec::new(),
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        if !arg.starts_with('-') {
            if path.replace(arg).is_some() {
                return Err("takes exactly one <path>".to_owned());
            }
            continue;
        }
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag.to_owned(), Some(value.to_owned())),
            None => (arg, None),
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Parsed::Help),
            "--stats" => {
                if inline.is_some() {
                    return Err("--stats takes no value".to_owned());
                }
                options.stats = true;
                continue;
            }
            "--frames" | "--fps" | "--start" | "--every" | "--node" | "--format" | "--fs-root"
            | "--event" => {}
            _ => return Err(format!("unknown option '{flag}'")),
        }
        let value = match inline.or_else(|| args.next()) {
            Some(value) => value,
            None => return Err(format!("{flag} needs a value")),
        };
        let bad = || format!("invalid value '{value}' for {flag}");
        match flag.as_str() {
            "--frames" => options.frames = value.parse().map_err(|_| bad())?,
            "--every" => {
                options.every = value.parse().map_err(|_| bad())?;
                if options.every == 0 {
                    return Err("--every must be at least 1".to_owned());
                }
            }
            "--fps" => {
                options.fps = value.parse().map_err(|_| bad())?;
                if !(options.fps.is_finite() && options.fps > 0.0) {
                    return Err("--fps must be a positive number".to_owned());
                }
            }
            "--start" => {
                options.start = value.parse().map_err(|_| bad())?;
                if !options.start.is_finite() {
                    return Err("--start must be a finite number".to_owned());
                }
            }
            "--node" => {
                if value.is_empty() {
                    return Err(bad());
                }
                options.nodes.push(value);
            }
            "--event" => options.events.push(EventArg::parse(&value)?),
            "--fs-root" => {
                if value.is_empty() {
                    return Err(bad());
                }
                options.fs_root = Some(value);
            }
            "--format" => {
                options.format = match value.as_str() {
                    "text" => Format::Text,
                    "jsonl" => Format::Jsonl,
                    _ => return Err(format!("unknown format '{value}' (text or jsonl)")),
                };
            }
            _ => return Err(format!("unknown option '{flag}'")),
        }
    }
    match path {
        Some(path) => {
            options.path = path;
            Ok(Parsed::Run(options))
        }
        None => Err("takes exactly one <path>".to_owned()),
    }
}

/// A node to print: name, id, output port names in spec order, and every
/// node it depends on (through links or `ch()` reads), sorted by name.
struct Target {
    name: String,
    id: NodeId,
    ports: Vec<&'static str>,
    /// Per port: whether it is an `Event` port.
    events: Vec<bool>,
    upstream: Vec<(String, NodeId)>,
}

/// The cook result of one target in one frame.
type Outcome = Result<Vec<Value>, String>;

fn cook(options: &Options) -> ExitCode {
    let (file, text) = match read(Path::new(&options.path)) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let graph = match parse(&text).and_then(|doc| doc.to_graph(Arc::new(core_registry()))) {
        Ok(graph) => graph,
        Err(err) => {
            report(&file, &err);
            return ExitCode::FAILURE;
        }
    };
    let targets = match targets(&graph, &options.nodes) {
        Ok(targets) => targets,
        Err(message) => return usage(&format!("cook: {message}")),
    };
    let events = match external_events(&graph, &options.events) {
        Ok(events) => events,
        Err(message) => return usage(&format!("cook: {message}")),
    };

    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());
    let mut cooker = Cooker::new();
    // File root of `core.fs.*` nodes: --fs-root or the package directory.
    let root = options.fs_root.as_ref().map_or_else(
        || {
            file.parent()
                .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf)
        },
        |dir| Path::new(dir).to_path_buf(),
    );
    let root = if root.as_os_str().is_empty() {
        Path::new(".").to_path_buf()
    } else {
        root
    };
    match StdFs::new(&root) {
        Ok(fs) => cooker.set_system(Some(Arc::new(fs))),
        Err(err) => {
            eprintln!("livspeil cook: file root {}: {err}", root.display());
            return ExitCode::FAILURE;
        }
    }
    let dt = 1.0 / options.fps;
    let mut node_error = false;
    for frame in 0..options.frames {
        // Exact for every frame count a run can reach (below 2^53).
        #[allow(clippy::cast_precision_loss)]
        let time = options.start + frame as f64 / options.fps;
        cooker.reset_stats();
        let injected: Vec<ExternalEvent> = events
            .iter()
            .filter(|(f, _)| *f == frame)
            .map(|(_, e)| e.clone())
            .collect();
        let report = cooker.tick(&graph, CookContext { frame, time, dt }, &injected);
        let outcomes: Vec<Outcome> = targets
            .iter()
            .map(|t| cook_target(&mut cooker, &graph, t))
            .collect();
        if frame % options.every != 0 {
            continue;
        }
        node_error |= outcomes.iter().any(Result::is_err) || !report.errors.is_empty();
        let stats = options.stats.then(|| cooker.stats());
        let frame_out = FrameOut {
            frame,
            time,
            targets: &targets,
            outcomes: &outcomes,
            report: &report,
            graph: &graph,
            stats,
        };
        let written = match options.format {
            Format::Text => write_text(&mut out, &frame_out),
            Format::Jsonl => write_jsonl(&mut out, &frame_out),
        };
        if let Err(err) = written {
            return output_failed(&err);
        }
    }
    if let Err(err) = out.flush() {
        return output_failed(&err);
    }
    if node_error {
        ExitCode::from(EXIT_NODE_ERROR)
    } else {
        ExitCode::SUCCESS
    }
}

fn output_failed(err: &io::Error) -> ExitCode {
    eprintln!("livspeil cook: cannot write output: {err}");
    ExitCode::FAILURE
}

/// Resolve `--event` arguments to `(frame, event)`, in order.
fn external_events(graph: &Graph, args: &[EventArg]) -> Result<Vec<(u64, ExternalEvent)>, String> {
    args.iter()
        .map(|arg| {
            let id = graph
                .find(&arg.node)
                .or_else(|| graph.find_path(&arg.node))
                .ok_or_else(|| format!("--event: no node named '{}'", arg.node))?;
            let event = match &arg.payload {
                Some(value) => Event::with(value.clone()),
                None => Event::bang(),
            };
            let event = ExternalEvent::new(graph, id, &arg.port, event).map_err(|_| {
                format!(
                    "--event: node '{}' has no Event port '{}'",
                    arg.node, arg.port
                )
            })?;
            Ok((arg.frame, event))
        })
        .collect()
}

/// How a node is named in the output: its name in the root network, its
/// path (without the leading `/`) inside subnets.
fn display_name(graph: &Graph, id: NodeId) -> Option<String> {
    match graph.parent(id) {
        None => graph.node_name(id).map(str::to_owned),
        Some(_) => graph.path(id).map(|p| p.trim_start_matches('/').to_owned()),
    }
}

/// The nodes to print, sorted by name: the requested ones, or the sinks.
fn targets(graph: &Graph, requested: &[String]) -> Result<Vec<Target>, String> {
    let mut ids = Vec::new();
    if requested.is_empty() {
        let root = graph.children(None);
        ids.extend(
            root.iter()
                .copied()
                .filter(|&id| graph.node_type(id) == Some(OUTPUT_TYPE)),
        );
        if ids.is_empty() {
            ids.extend(
                root.into_iter()
                    .filter(|&id| !graph.is_subnet(id) && graph.outgoing_links(id).is_empty()),
            );
        }
    } else {
        for name in requested {
            let id = graph
                .find(name)
                .or_else(|| graph.find_path(name))
                .ok_or_else(|| format!("--node: no node named '{name}'"))?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    let readers = ch_readers(graph);
    let mut targets: Vec<Target> = ids
        .into_iter()
        .filter_map(|id| {
            let name = display_name(graph, id)?;
            Some(Target {
                name,
                id,
                ports: graph
                    .node_spec(id)?
                    .outputs
                    .iter()
                    .map(|p| p.name)
                    .collect(),
                events: graph
                    .node_spec(id)?
                    .outputs
                    .iter()
                    .map(|p| p.ty == PortType::Event)
                    .collect(),
                upstream: upstream(graph, &readers, id),
            })
        })
        .collect();
    targets.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(targets)
}

/// For every node, the nodes whose inputs its expressions read via `ch()`.
fn ch_readers(graph: &Graph) -> HashMap<NodeId, Vec<NodeId>> {
    let mut reads: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for source in graph.node_ids() {
        for (reader, _) in graph.referenced_by(source) {
            reads.entry(reader).or_default().push(source);
        }
    }
    reads
}

/// Every node `id` depends on, transitively, sorted by name.
fn upstream(
    graph: &Graph,
    reads: &HashMap<NodeId, Vec<NodeId>>,
    id: NodeId,
) -> Vec<(String, NodeId)> {
    let mut seen = HashSet::new();
    let mut stack = vec![id];
    while let Some(node) = stack.pop() {
        let linked = graph.incoming_links(node).into_iter().map(|l| l.from);
        let read = reads.get(&node).into_iter().flatten().copied();
        for dep in linked.chain(read) {
            if dep != id && seen.insert(dep) {
                stack.push(dep);
            }
        }
    }
    let mut deps: Vec<(String, NodeId)> = seen
        .into_iter()
        .filter_map(|dep| Some((graph.node_name(dep)?.to_owned(), dep)))
        .collect();
    deps.sort();
    deps
}

/// Cook one node. It is an error when it failed this frame, even if the
/// cooker still holds an earlier good output for it, and when any node it
/// depends on failed this frame (its value would be stale).
fn cook_target(cooker: &mut Cooker, graph: &Graph, target: &Target) -> Outcome {
    let result = cooker.cook(graph, target.id);
    // Value errors only: handler errors are reported per tick.
    if let Some(err) = cooker.value_error(target.id) {
        return Err(describe(graph, err));
    }
    let values = result.map_err(|err| describe(graph, &err))?;
    for (name, dep) in &target.upstream {
        if let Some(err) = cooker.value_error(*dep) {
            return Err(format!(
                "upstream node '{name}' failed: {}",
                describe(graph, err)
            ));
        }
    }
    Ok(values)
}

/// Error text with node names instead of ids where the error names a node.
fn describe(graph: &Graph, err: &CookError) -> String {
    match err {
        CookError::Upstream { node } => match graph.node_name(*node) {
            Some(name) => format!("upstream node '{name}' has no usable output"),
            None => err.to_string(),
        },
        _ => err.to_string(),
    }
}

/// A float with six decimals; negative zero (after rounding) is unsigned.
fn fixed(x: f64) -> String {
    let s = format!("{x:.6}");
    if s == "-0.000000" {
        "0.000000".to_owned()
    } else {
        s
    }
}

fn fixed_list(xs: &[f64]) -> String {
    xs.iter().map(|&x| fixed(x)).collect::<Vec<_>>().join(" ")
}

fn json_string(s: &str) -> String {
    // Serializing a string cannot fail; the fallback keeps output valid JSON.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

/// A float rounded to six decimals, as JSON (`null` if not finite).
fn json_float(x: f64) -> String {
    if !x.is_finite() {
        return "null".to_owned();
    }
    let rounded: f64 = fixed(x).parse().unwrap_or(x);
    serde_json::to_string(&rounded).unwrap_or_else(|_| "null".to_owned())
}

fn json_list(xs: &[f64]) -> String {
    let items: Vec<String> = xs.iter().map(|&x| json_float(x)).collect();
    format!("[{}]", items.join(","))
}

fn text_value(value: &Value) -> String {
    match value {
        Value::Float(x) => fixed(*x),
        Value::Int(i) => i.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Vec2(v) => format!("vec2({})", fixed_list(v)),
        Value::Vec3(v) => format!("vec3({})", fixed_list(v)),
        Value::Vec4(v) => format!("vec4({})", fixed_list(v)),
        Value::Color(c) => format!("oklch({})", fixed_list(c)),
        Value::String(s) => json_string(s),
        Value::Table(t) => {
            let columns: Vec<String> = t
                .columns()
                .iter()
                .map(|c| format!("{}:{}", c.name(), c.ty()))
                .collect();
            let rows: Vec<String> = (0..t.row_count())
                .map(|r| t.row(r).map(text_value).collect::<Vec<_>>().join(" "))
                .collect();
            format!("table[{}][{}]", columns.join(" "), rows.join("; "))
        }
        Value::Curve(c) => {
            let keys: Vec<String> = c
                .keys()
                .iter()
                .map(|k| format!("{}:{}", fixed(k.position), fixed(k.value)))
                .collect();
            format!("curve({})", keys.join(" "))
        }
        other => format!("<unsupported {}>", other.port_type()),
    }
}

fn json_value(value: &Value) -> String {
    let (tag, body) = json_parts(value);
    format!("{{\"{tag}\":{body}}}")
}

/// The type tag and untagged JSON of a value.
fn json_parts(value: &Value) -> (&'static str, String) {
    match value {
        Value::Float(x) => ("float", json_float(*x)),
        Value::Int(i) => ("int", i.to_string()),
        Value::Bool(b) => ("bool", b.to_string()),
        Value::Vec2(v) => ("vec2", json_list(v)),
        Value::Vec3(v) => ("vec3", json_list(v)),
        Value::Vec4(v) => ("vec4", json_list(v)),
        Value::Color(c) => ("color", json_list(c)),
        Value::String(s) => ("string", json_string(s)),
        Value::Table(t) => {
            let columns: Vec<String> = t
                .columns()
                .iter()
                .map(|c| {
                    let tag = Value::zero(c.ty()).map_or("unsupported", |z| json_parts(&z).0);
                    format!("{{\"name\":{},\"type\":\"{tag}\"}}", json_string(c.name()))
                })
                .collect();
            let rows: Vec<String> = (0..t.row_count())
                .map(|r| {
                    let cells: Vec<String> = t.row(r).map(|v| json_parts(v).1).collect();
                    format!("[{}]", cells.join(","))
                })
                .collect();
            (
                "table",
                format!(
                    "{{\"columns\":[{}],\"rows\":[{}]}}",
                    columns.join(","),
                    rows.join(",")
                ),
            )
        }
        Value::Curve(c) => {
            let keys: Vec<String> = c
                .keys()
                .iter()
                .map(|k| json_list(&[k.position, k.value]))
                .collect();
            ("curve", format!("[{}]", keys.join(",")))
        }
        other => ("unsupported", json_string(other.port_type().name())),
    }
}

/// Everything printed for one frame.
struct FrameOut<'a> {
    frame: u64,
    time: f64,
    targets: &'a [Target],
    outcomes: &'a [Outcome],
    report: &'a TickReport,
    graph: &'a Graph,
    stats: Option<livspeil_graph::CookStats>,
}

impl FrameOut<'_> {
    /// The events fired from output `port` of `target` this frame.
    fn fired(&self, target: &Target, port: usize) -> Vec<&Event> {
        self.report.fired_at(target.id, port).collect()
    }

    /// Failed handlers as `(node name, message)`.
    fn event_errors(&self) -> Vec<(String, String)> {
        self.report
            .errors
            .iter()
            .map(|e| {
                let name = display_name(self.graph, e.node).unwrap_or_else(|| e.node.to_string());
                (name, describe(self.graph, &e.error))
            })
            .collect()
    }
}

fn text_events(events: &[&Event]) -> String {
    if events.is_empty() {
        return "none".to_owned();
    }
    events
        .iter()
        .map(|e| match &e.payload {
            Some(p) => format!("event({})", text_value(p)),
            None => "event".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn json_events(events: &[&Event]) -> String {
    let items: Vec<String> = events
        .iter()
        .map(|e| {
            e.payload
                .as_ref()
                .map_or_else(|| "null".to_owned(), json_value)
        })
        .collect();
    format!("{{\"event\":[{}]}}", items.join(","))
}

fn write_text(out: &mut impl Write, f: &FrameOut<'_>) -> io::Result<()> {
    let prefix = format!("frame={} time={}", f.frame, fixed(f.time));
    for (target, outcome) in f.targets.iter().zip(f.outcomes) {
        write!(out, "{prefix}")?;
        for (index, port) in target.ports.iter().enumerate() {
            let value = match outcome {
                Ok(_) if target.events.get(index) == Some(&true) => {
                    text_events(&f.fired(target, index))
                }
                Ok(values) => values
                    .get(index)
                    .map_or_else(|| "<missing>".to_owned(), text_value),
                Err(message) => format!("<error: {message}>"),
            };
            write!(out, " {}.{port}={value}", target.name)?;
        }
        writeln!(out)?;
    }
    if f.report.overflowed() {
        writeln!(
            out,
            "{prefix} events overflow processed={} dropped={}",
            f.report.processed, f.report.dropped
        )?;
    }
    for (node, message) in f.event_errors() {
        writeln!(out, "{prefix} event-error {node}: {message}")?;
    }
    if let Some(stats) = f.stats {
        writeln!(
            out,
            "{prefix} stats cooked={} reused={}",
            stats.cooked, stats.reused
        )?;
    }
    Ok(())
}

fn write_jsonl(out: &mut impl Write, f: &FrameOut<'_>) -> io::Result<()> {
    let nodes: Vec<String> = f
        .targets
        .iter()
        .zip(f.outcomes)
        .map(|(target, outcome)| {
            let body = match outcome {
                Ok(values) => {
                    let ports: Vec<String> = target
                        .ports
                        .iter()
                        .zip(values)
                        .enumerate()
                        .map(|(index, (port, value))| {
                            let value = if target.events.get(index) == Some(&true) {
                                json_events(&f.fired(target, index))
                            } else {
                                json_value(value)
                            };
                            format!("{}:{value}", json_string(port))
                        })
                        .collect();
                    format!("{{{}}}", ports.join(","))
                }
                Err(message) => format!("{{\"error\":{}}}", json_string(message)),
            };
            format!("{}:{body}", json_string(&target.name))
        })
        .collect();
    write!(
        out,
        "{{\"frame\":{},\"time\":{},\"outputs\":{{{}}}",
        f.frame,
        json_float(f.time),
        nodes.join(",")
    )?;
    if f.report.overflowed() {
        write!(
            out,
            ",\"events\":{{\"processed\":{},\"dropped\":{}}}",
            f.report.processed, f.report.dropped
        )?;
    }
    let errors = f.event_errors();
    if !errors.is_empty() {
        let items: Vec<String> = errors
            .iter()
            .map(|(node, message)| {
                format!(
                    "{{\"node\":{},\"error\":{}}}",
                    json_string(node),
                    json_string(message)
                )
            })
            .collect();
        write!(out, ",\"event_errors\":[{}]", items.join(","))?;
    }
    if let Some(stats) = f.stats {
        write!(
            out,
            ",\"stats\":{{\"cooked\":{},\"reused\":{}}}",
            stats.cooked, stats.reused
        )?;
    }
    writeln!(out, "}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(args: &[&str]) -> Result<Options, String> {
        match parse_args(args.iter().map(|s| (*s).to_owned()))? {
            Parsed::Run(o) => Ok(o),
            Parsed::Help => Err("help".to_owned()),
        }
    }

    #[test]
    fn defaults() {
        let o = opts(&["room.kdl"]).unwrap();
        assert_eq!(
            (o.frames, o.fps, o.start, o.every, o.format, o.stats),
            (1, 60.0, 0.0, 1, Format::Text, false)
        );
    }

    #[test]
    fn values_inline_and_separate() {
        let o = opts(&[
            "--frames=600",
            "r",
            "--fps",
            "30",
            "--every",
            "5",
            "--node",
            "a",
            "--node=b",
            "--format",
            "jsonl",
            "--start=-1.5",
            "--stats",
        ])
        .unwrap();
        assert_eq!(o.path, "r");
        assert_eq!((o.frames, o.fps, o.every, o.start), (600, 30.0, 5, -1.5));
        assert_eq!(o.nodes, ["a", "b"]);
        assert_eq!(o.format, Format::Jsonl);
        assert!(o.stats);
    }

    #[test]
    fn events() {
        let o = opts(&["r", "--event", "blink.fired@3", "--event=sub/c.trigger@0"]).unwrap();
        assert_eq!(
            o.events,
            [
                EventArg {
                    node: "blink".into(),
                    port: "fired".into(),
                    frame: 3,
                    payload: None,
                },
                EventArg {
                    node: "sub/c".into(),
                    port: "trigger".into(),
                    frame: 0,
                    payload: None,
                }
            ]
        );
        let payload = |arg: &str| opts(&["r", "--event", arg]).map(|o| o.events[0].payload.clone());
        assert_eq!(payload("a.b@1=1.5"), Ok(Some(Value::Float(1.5))));
        assert_eq!(payload("a.b@1=3"), Ok(Some(Value::Int(3))));
        assert_eq!(payload("a.b@1=#true"), Ok(Some(Value::Bool(true))));
        assert_eq!(
            payload("a.b@1=\"x=y@z\""),
            Ok(Some(Value::String("x=y@z".into())))
        );
        assert_eq!(
            payload("a.b@1=(vec3)\"1 2 3\""),
            Ok(Some(Value::Vec3([1.0, 2.0, 3.0])))
        );
        assert_eq!(
            payload("a.b@1=(color)\"oklch(0.5 0.1 30)\""),
            Ok(Some(Value::Color([0.5, 0.1, 30.0, 1.0])))
        );
        let o = opts(&["r", "--event=a.b@2=-1"]).unwrap();
        assert_eq!(o.events[0].payload, Some(Value::Int(-1)));
        assert_eq!(o.events[0].frame, 2);
        let o = opts(&["r", "--event", "a.b.c@2"]).unwrap();
        assert_eq!(
            (o.events[0].node.as_str(), o.events[0].port.as_str()),
            ("a.b", "c")
        );
    }

    #[test]
    fn bad_arguments() {
        for args in [
            &[][..],
            &["a", "b"],
            &["r", "--frames"],
            &["r", "--frames", "-1"],
            &["r", "--every", "0"],
            &["r", "--fps", "0"],
            &["r", "--fps", "inf"],
            &["r", "--start", "nan"],
            &["r", "--format", "xml"],
            &["r", "--frob"],
            &["r", "--stats=1"],
            &["r", "--node="],
            &["r", "--event", "a.b"],
            &["r", "--event", "ab@1"],
            &["r", "--event", ".b@1"],
            &["r", "--event", "a.@1"],
            &["r", "--event", "a.b@-1"],
            &["r", "--event"],
            &["r", "--event", "a.b@1="],
            &["r", "--event", "a.b@1=1 2"],
            &["r", "--event", "a.b@1=(vec3)\"1\""],
            &["r", "--event", "a.b@1=#nan"],
            &["r", "--event", "a.b=1@1"],
        ] {
            assert!(opts(args).is_err(), "{args:?}");
        }
    }

    /// Every float of the acceptance scene, over all 600 frames, prints the
    /// same with six decimals after a relative perturbation of 1e-12 (or an
    /// absolute one of 1e-15 near zero). Platform math libraries (`sin`,
    /// `cos`, `atan2`, `cbrt`, ... in the sky and color nodes) differ by at
    /// most a few ulps, far below that, so the golden files are the same on
    /// macOS and Linux.
    /// Cook `room` (relative to the repository root) for 600 frames and
    /// check that every float prints the same with six decimals after a
    /// relative perturbation of 1e-12 (or an absolute one of 1e-15 near
    /// zero). Returns the number of values checked.
    fn far_from_rounding_boundaries(room: &str) -> usize {
        let room = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(room);
        let text = std::fs::read_to_string(room).unwrap();
        let graph = parse(&text)
            .unwrap()
            .to_graph(Arc::new(core_registry()))
            .unwrap();
        let targets = targets(&graph, &[]).unwrap();
        let mut cooker = Cooker::new();
        let stable = |x: f64| {
            let s = fixed(x);
            for y in [x * (1.0 + 1e-12), x * (1.0 - 1e-12), x + 1e-15, x - 1e-15] {
                assert_eq!(fixed(y), s, "{x:e} is too close to a rounding boundary");
            }
        };
        let mut checked = 0;
        for frame in 0..600_u64 {
            let time = frame as f64 / 60.0;
            stable(time);
            cooker.set_clock(CookContext {
                frame,
                time,
                dt: 1.0 / 60.0,
            });
            for target in &targets {
                for value in cook_target(&mut cooker, &graph, target).unwrap() {
                    let floats: Vec<f64> = match value {
                        Value::Float(x) => vec![x],
                        Value::Vec3(v) => v.to_vec(),
                        Value::Color(c) => c.to_vec(),
                        other => panic!("unexpected value {other:?}"),
                    };
                    floats.into_iter().for_each(stable);
                    checked += 1;
                }
            }
        }
        checked
    }

    /// Every float of the acceptance scene, over all 600 frames, is far
    /// from a six-decimal rounding boundary. Platform math libraries
    /// (`sin`, `cos`, `atan2`, `cbrt`, ... in the sky and color nodes)
    /// differ by at most a few ulps, far below that, so the golden files
    /// are the same on macOS and Linux.
    #[test]
    fn acceptance_scene_is_far_from_rounding_boundaries() {
        assert_eq!(
            far_from_rounding_boundaries("examples/sun-and-lamp.room/room.kdl"),
            600 * 5
        );
    }

    /// The same for the Charm demo (four root `core.output` nodes).
    #[test]
    fn charm_demo_is_far_from_rounding_boundaries() {
        assert_eq!(
            far_from_rounding_boundaries("examples/charm-demo.room/room.kdl"),
            600 * 4
        );
    }

    #[test]
    fn float_formatting() {
        assert_eq!(fixed(0.5), "0.500000");
        assert_eq!(fixed(-0.0), "0.000000");
        assert_eq!(fixed(-1e-9), "0.000000");
        assert_eq!(fixed(-0.25), "-0.250000");
        assert_eq!(json_float(0.8000000001), "0.8");
        assert_eq!(json_float(1.0), "1.0");
        assert_eq!(json_float(-1e-9), "0.0");
        assert_eq!(json_float(f64::NAN), "null");
    }

    #[test]
    fn value_formatting() {
        assert_eq!(
            text_value(&Value::Color([0.8, 0.12, 60.0, 1.0])),
            "oklch(0.800000 0.120000 60.000000 1.000000)"
        );
        assert_eq!(text_value(&Value::String("a\"b".into())), "\"a\\\"b\"");
        assert_eq!(
            json_value(&Value::Vec2([1.0, -0.5])),
            "{\"vec2\":[1.0,-0.5]}"
        );
        assert_eq!(json_value(&Value::Int(-3)), "{\"int\":-3}");
        assert_eq!(json_value(&Value::Bool(true)), "{\"bool\":true}");
        let table = livspeil_node_api::Table::from_rows(
            &[
                ("name", livspeil_node_api::PortType::String),
                ("size", livspeil_node_api::PortType::Int),
            ],
            [
                vec![Value::String("a".into()), Value::Int(1)],
                vec![Value::String("b c".into()), Value::Int(2)],
            ],
        )
        .unwrap();
        let table = Value::Table(table);
        assert_eq!(
            text_value(&table),
            "table[name:String size:Int][\"a\" 1; \"b c\" 2]"
        );
        assert_eq!(
            json_value(&table),
            "{\"table\":{\"columns\":[{\"name\":\"name\",\"type\":\"string\"},{\"name\":\"size\",\"type\":\"int\"}],\"rows\":[[\"a\",1],[\"b c\",2]]}}"
        );
        let empty = Value::Table(livspeil_node_api::Table::empty());
        assert_eq!(text_value(&empty), "table[][]");
        let curve = Value::Curve(Arc::new(livspeil_node_api::Curve::identity()));
        assert_eq!(
            text_value(&curve),
            "curve(0.000000:0.000000 1.000000:1.000000)"
        );
        assert_eq!(json_value(&curve), "{\"curve\":[[0.0,0.0],[1.0,1.0]]}");
    }

    #[test]
    fn fs_root_option() {
        assert_eq!(opts(&["r"]).unwrap().fs_root, None);
        assert_eq!(
            opts(&["r", "--fs-root", "/tmp/x"])
                .unwrap()
                .fs_root
                .as_deref(),
            Some("/tmp/x")
        );
        assert!(opts(&["r", "--fs-root="]).is_err());
        assert!(opts(&["r", "--fs-root"]).is_err());
    }
}
