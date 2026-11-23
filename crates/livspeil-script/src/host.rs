// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The scene a script works on and the handlers of its requests.

use std::collections::HashMap;

use livspeil_cmd::{CmdError, Command, DocChange, GroupId, History, JournalEntry, Origin};
use livspeil_graph::{Cooker, Graph, NodeId};
use livspeil_node_api::{CookContext, Value};
use serde_json::{Map, Value as Json, json};

use crate::capability::{Capabilities, Capability};
use crate::protocol::{RemoteError, codes};

/// Most ids one `graph.new_id` request may allocate.
const MAX_NEW_IDS: u64 = 1024;

/// First coalescing group handed to scripts; script groups are mapped to
/// fresh host groups so they never merge with a UI gesture.
const SCRIPT_GROUP_BASE: u64 = 1 << 48;

/// What scripts edit: a graph with its undo history, the scene's cooker
/// (used for ticks) and the journal of every command applied by scripts.
#[derive(Debug)]
pub struct Scene {
    /// Room name, reported in the handshake.
    pub name: String,
    pub graph: Graph,
    pub history: History,
    /// The scene's cooker: ticks run on it, and `graph.cook` without an
    /// explicit time reads it (so a script sees the scene's event state).
    pub cooker: Cooker,
    /// Every command applied through scripts, in order, with
    /// [`Origin::Script`]; append them to the room's journal.
    pub journal: Vec<JournalEntry>,
    next_group: u64,
}

impl Scene {
    /// A scene around `graph` with an empty history.
    #[must_use]
    pub fn new(name: impl Into<String>, graph: Graph) -> Self {
        Scene {
            name: name.into(),
            graph,
            history: History::new(),
            cooker: Cooker::new(),
            journal: Vec::new(),
            next_group: SCRIPT_GROUP_BASE,
        }
    }

    fn fresh_group(&mut self) -> GroupId {
        self.next_group += 1;
        GroupId(self.next_group)
    }
}

/// Per-session state the handlers need besides the scene.
#[derive(Debug)]
pub(crate) struct HandlerState {
    pub(crate) capabilities: Capabilities,
    /// Script group id -> host group id.
    pub(crate) groups: HashMap<u64, GroupId>,
    /// Cooker for `graph.cook` at an explicit time, so scripts never move
    /// the scene's clock.
    pub(crate) cooker: Cooker,
}

/// The capability a method needs, if any.
fn required(method: &str) -> Option<Capability> {
    match method {
        "graph.nodes" | "graph.node" | "graph.snapshot" => Some(Capability::GraphRead),
        "graph.apply" | "graph.new_id" => Some(Capability::GraphWrite),
        "graph.cook" => Some(Capability::GraphCook),
        _ => None,
    }
}

fn invalid(message: impl Into<String>) -> RemoteError {
    RemoteError::new(codes::INVALID_PARAMS, message)
}

/// Serve one script request (other than `hello` and `listen`, which the
/// session handles).
pub(crate) fn dispatch(
    scene: &mut Scene,
    state: &mut HandlerState,
    method: &str,
    params: &Json,
) -> Result<Json, RemoteError> {
    if let Some(cap) = required(method)
        && !state.capabilities.has(&cap)
    {
        return Err(RemoteError::new(
            codes::CAPABILITY_DENIED,
            format!("'{method}' needs the '{cap}' capability"),
        )
        .with_data(json!({ "capability": cap.token(), "method": method })));
    }
    match method {
        "graph.nodes" => nodes(&scene.graph, params),
        "graph.node" => {
            let id = node_param(&scene.graph, params, "node")?;
            Ok(node_info(&scene.graph, id))
        }
        "graph.snapshot" => snapshot(&scene.graph),
        "graph.apply" => apply(scene, state, params),
        "graph.new_id" => new_ids(params),
        "graph.cook" => cook(scene, state, params),
        _ => Err(RemoteError::new(
            codes::UNKNOWN_METHOD,
            format!("unknown method '{method}'"),
        )),
    }
}

/// Resolve a node reference: a UUID string naming an existing node, or a
/// path (`/lamp/bulb` or `lamp/bulb`).
pub(crate) fn resolve(graph: &Graph, reference: &str) -> Option<NodeId> {
    if let Ok(id) = serde_json::from_value::<NodeId>(Json::String(reference.to_owned()))
        && graph.contains(id)
    {
        return Some(id);
    }
    graph.find_path(reference)
}

fn node_param(graph: &Graph, params: &Json, key: &str) -> Result<NodeId, RemoteError> {
    let reference = params
        .get(key)
        .and_then(Json::as_str)
        .ok_or_else(|| invalid(format!("'{key}' must be a node path or id")))?;
    resolve(graph, reference).ok_or_else(|| {
        RemoteError::new(codes::NOT_FOUND, format!("no node '{reference}'"))
            .with_data(json!({ "node": reference }))
    })
}

fn id_json(id: NodeId) -> Json {
    Json::String(id.to_string())
}

fn path_json(graph: &Graph, id: NodeId) -> Json {
    graph.path(id).map_or(Json::Null, Json::String)
}

fn value_json(value: &Value) -> Json {
    serde_json::to_value(value).unwrap_or(Json::Null)
}

fn nodes(graph: &Graph, params: &Json) -> Result<Json, RemoteError> {
    let recursive = params
        .get("recursive")
        .and_then(Json::as_bool)
        .unwrap_or(true);
    let network = match params.get("network").and_then(Json::as_str) {
        None | Some("/" | "") => None,
        Some(path) => {
            let id = resolve(graph, path).ok_or_else(|| {
                RemoteError::new(codes::NOT_FOUND, format!("no network '{path}'"))
            })?;
            if !graph.is_subnet(id) {
                return Err(invalid(format!("'{path}' is not a subnet")));
            }
            Some(id)
        }
    };
    let mut out = Vec::new();
    let mut stack = vec![network];
    while let Some(net) = stack.pop() {
        let mut children = graph.children(net);
        children.sort();
        for id in children {
            if recursive && graph.is_subnet(id) {
                stack.push(Some(id));
            }
            out.push((graph.path(id).unwrap_or_default(), id));
        }
    }
    out.sort();
    let list: Vec<Json> = out
        .into_iter()
        .map(|(path, id)| {
            json!({
                "id": id_json(id),
                "name": graph.node_name(id),
                "path": path,
                "type": graph.node_type(id),
                "parent": graph.parent(id).map_or(Json::Null, |p| path_json(graph, p)),
                "subnet": graph.is_subnet(id),
            })
        })
        .collect();
    Ok(json!({ "nodes": list }))
}

fn endpoint(graph: &Graph, node: NodeId, port: &str) -> Json {
    json!({ "node": path_json(graph, node), "id": id_json(node), "port": port })
}

fn node_info(graph: &Graph, id: NodeId) -> Json {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    if let Some(spec) = graph.node_spec(id) {
        let outgoing = graph.outgoing_links(id);
        for port in &spec.inputs {
            let value = graph.input_value(id, port.name).ok().flatten();
            let link = graph.link_source(id, port.name).ok().flatten();
            let expression = graph.expression(id, port.name).ok().flatten();
            let animated = graph.animation(id, port.name).ok().flatten().is_some();
            inputs.push(json!({
                "name": port.name,
                "type": port.ty,
                "value": value.map_or(Json::Null, value_json),
                "default": port.default.as_ref().map_or(Json::Null, value_json),
                "link": link.map_or(Json::Null, |(n, p)| endpoint(graph, n, p)),
                "expression": expression,
                "animated": animated,
            }));
        }
        for port in &spec.outputs {
            let links: Vec<Json> = outgoing
                .iter()
                .filter(|l| l.from_port == port.name)
                .map(|l| endpoint(graph, l.to, &l.to_port))
                .collect();
            outputs.push(json!({ "name": port.name, "type": port.ty, "links": links }));
        }
    }
    let charm = graph
        .charm(id)
        .and_then(|c| serde_json::to_value(c).ok())
        .unwrap_or(Json::Null);
    json!({
        "id": id_json(id),
        "name": graph.node_name(id),
        "path": path_json(graph, id),
        "type": graph.node_type(id),
        "summary": graph.node_spec(id).map(|s| s.summary),
        "parent": graph.parent(id).map_or(Json::Null, |p| path_json(graph, p)),
        "subnet": graph.is_subnet(id),
        "port_type": graph.port_type(id),
        "charm": charm,
        "inputs": inputs,
        "outputs": outputs,
    })
}

fn snapshot(graph: &Graph) -> Result<Json, RemoteError> {
    let snap = graph.snapshot();
    let paths: Map<String, Json> = snap
        .nodes
        .iter()
        .map(|n| (n.id.to_string(), path_json(graph, n.id)))
        .collect();
    let mut value = serde_json::to_value(&snap)
        .map_err(|e| RemoteError::new(codes::INTERNAL, format!("cannot encode snapshot: {e}")))?;
    if let Json::Object(obj) = &mut value {
        obj.insert("paths".into(), Json::Object(paths));
    }
    Ok(value)
}

fn apply(scene: &mut Scene, state: &mut HandlerState, params: &Json) -> Result<Json, RemoteError> {
    let raw = params
        .get("command")
        .ok_or_else(|| invalid("'command' is required"))?;
    let command: Command =
        serde_json::from_value(raw.clone()).map_err(|e| invalid(format!("not a command: {e}")))?;
    let group = match params.get("group") {
        None | Some(Json::Null) => None,
        Some(g) => {
            let g = g
                .as_u64()
                .ok_or_else(|| invalid("'group' must be a non-negative integer"))?;
            Some(match state.groups.get(&g) {
                Some(host) => *host,
                None => {
                    let host = scene.fresh_group();
                    state.groups.insert(g, host);
                    host
                }
            })
        }
    };
    let record = scene
        .history
        .apply(&mut scene.graph, command, Origin::Script, group)
        .map_err(|err| command_error(&err))?;
    let changes: Vec<Json> = record
        .changes
        .iter()
        .map(|c| change_json(&scene.graph, c))
        .collect();
    scene.journal.push(record.entry);
    Ok(json!({ "changes": changes, "undo_len": scene.history.undo_len() }))
}

fn command_error(err: &CmdError) -> RemoteError {
    let mut data = Map::new();
    let mut current = err;
    let mut path = Vec::new();
    while let CmdError::InTransaction { index, source } = current {
        path.push(*index);
        current = source;
    }
    if !path.is_empty() {
        data.insert("step".into(), json!(path));
    }
    RemoteError::new(codes::COMMAND_FAILED, err.to_string()).with_data(Json::Object(data))
}

/// A [`DocChange`] as JSON: `{"kind":"input_changed","node":"<id>","path":"/lamp","port":"a"}`.
pub(crate) fn change_json(graph: &Graph, change: &DocChange) -> Json {
    let node = |id: NodeId| (id_json(id), path_json(graph, id));
    match change {
        DocChange::NodeAdded { id } => {
            let (id, path) = node(*id);
            json!({ "kind": "node_added", "node": id, "path": path })
        }
        DocChange::NodeRemoved { id } => json!({ "kind": "node_removed", "node": id_json(*id) }),
        DocChange::InputChanged { node: n, port } => {
            let (id, path) = node(*n);
            json!({ "kind": "input_changed", "node": id, "path": path, "port": port })
        }
        DocChange::ExpressionChanged { node: n, port } => {
            let (id, path) = node(*n);
            json!({ "kind": "expression_changed", "node": id, "path": path, "port": port })
        }
        DocChange::AnimationChanged { node: n, port } => {
            let (id, path) = node(*n);
            json!({ "kind": "animation_changed", "node": id, "path": path, "port": port })
        }
        DocChange::Linked(link) | DocChange::Unlinked(link) => json!({
            "kind": if matches!(change, DocChange::Linked(_)) { "linked" } else { "unlinked" },
            "from": endpoint(graph, link.from, &link.from_port),
            "to": endpoint(graph, link.to, &link.to_port),
        }),
        DocChange::Renamed { node: n, old, new } => {
            let (id, path) = node(*n);
            json!({ "kind": "renamed", "node": id, "path": path, "old": old, "new": new })
        }
        DocChange::NodeMoved { node: n, from, to } => {
            let (id, path) = node(*n);
            json!({
                "kind": "node_moved", "node": id, "path": path,
                "from": from.map_or(Json::Null, |p| path_json(graph, p)),
                "to": to.map_or(Json::Null, |p| path_json(graph, p)),
            })
        }
        DocChange::CharmChanged { subnet } => {
            let (id, path) = node(*subnet);
            json!({ "kind": "charm_changed", "node": id, "path": path })
        }
        _ => json!({ "kind": "other" }),
    }
}

fn new_ids(params: &Json) -> Result<Json, RemoteError> {
    let count = match params.get("count") {
        None | Some(Json::Null) => 1,
        Some(c) => c
            .as_u64()
            .filter(|c| (1..=MAX_NEW_IDS).contains(c))
            .ok_or_else(|| invalid(format!("'count' must be 1..={MAX_NEW_IDS}")))?,
    };
    let ids: Vec<Json> = (0..count).map(|_| id_json(NodeId::new())).collect();
    Ok(json!({ "ids": ids }))
}

fn cook(scene: &mut Scene, state: &mut HandlerState, params: &Json) -> Result<Json, RemoteError> {
    let refs = params
        .get("nodes")
        .and_then(Json::as_array)
        .ok_or_else(|| invalid("'nodes' must be a list of node paths or ids"))?;
    let mut ids = Vec::with_capacity(refs.len());
    for r in refs {
        let r = r
            .as_str()
            .ok_or_else(|| invalid("'nodes' must be a list of node paths or ids"))?;
        let id = resolve(&scene.graph, r).ok_or_else(|| {
            RemoteError::new(codes::NOT_FOUND, format!("no node '{r}'"))
                .with_data(json!({ "node": r }))
        })?;
        ids.push(id);
    }
    let number = |key: &str| -> Result<Option<f64>, RemoteError> {
        match params.get(key) {
            None | Some(Json::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|x| x.is_finite())
                .map(Some)
                .ok_or_else(|| invalid(format!("'{key}' must be a finite number"))),
        }
    };
    let time = number("time")?;
    let dt = number("dt")?;
    let frame = match params.get("frame") {
        None | Some(Json::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or_else(|| invalid("'frame' must be a non-negative integer"))?,
        ),
    };
    let cooker = if time.is_some() || frame.is_some() || dt.is_some() {
        let base = scene.cooker.clock();
        let clock = CookContext {
            frame: frame.unwrap_or(base.frame),
            time: time.unwrap_or(base.time),
            dt: dt.unwrap_or(base.dt),
        };
        state.cooker.set_clock(clock);
        &mut state.cooker
    } else {
        &mut scene.cooker
    };
    let clock = cooker.clock();
    let graph = &scene.graph;
    let results: Vec<Json> = ids
        .into_iter()
        .map(|id| {
            let mut entry = Map::new();
            entry.insert("node".into(), path_json(graph, id));
            entry.insert("id".into(), id_json(id));
            match cooker.cook(graph, id) {
                Ok(values) => {
                    let outputs: Map<String, Json> = graph
                        .node_spec(id)
                        .map(|spec| {
                            spec.outputs
                                .iter()
                                .zip(&values)
                                .map(|(port, v)| (port.name.to_owned(), value_json(v)))
                                .collect()
                        })
                        .unwrap_or_default();
                    entry.insert("outputs".into(), Json::Object(outputs));
                    if let Some(err) = cooker.error(id) {
                        entry.insert("error".into(), Json::String(err.to_string()));
                    }
                }
                Err(err) => {
                    entry.insert("error".into(), Json::String(err.to_string()));
                }
            }
            Json::Object(entry)
        })
        .collect();
    Ok(json!({
        "clock": { "frame": clock.frame, "time": clock.time, "dt": clock.dt },
        "results": results,
    }))
}

/// The fired events of a tick as `event` params (`kind: "scene"`).
pub(crate) fn scene_events(graph: &Graph, report: &livspeil_graph::TickReport) -> Vec<Json> {
    report
        .fired
        .iter()
        .map(|fired| {
            let port = graph
                .node_spec(fired.node)
                .and_then(|s| s.outputs.get(fired.port))
                .map(|p| p.name);
            json!({
                "kind": "scene",
                "node": path_json(graph, fired.node),
                "id": id_json(fired.node),
                "port": port,
                "payload": fired.event.payload.as_ref().map_or(Json::Null, value_json),
            })
        })
        .collect()
}
