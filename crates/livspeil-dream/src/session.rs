// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The open room: its graph, what it currently computes, the frame clock
//! and the selection. Everything the views show comes from here.

mod edits;
mod files;

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use livspeil_cmd::{GroupId, History};
use livspeil_doc::{DocError, Tier};
use livspeil_graph::{Cooker, Graph, GraphSnapshot, NodeId};
use livspeil_node_api::{CookContext, RegistryError, SystemHost, Value};

use self::files::File;
use crate::inspect::{Inspection, inspect, output};
use crate::shown::{Shown, ShownLink, links, root_nodes};

/// Frames of the clock per second, as `livspeil cook` has by default.
pub const FPS: f64 = 60.0;
/// Most frames one update steps when the window fell behind.
const MAX_CATCH_UP: u32 = 6;

/// Why a room could not be opened.
#[derive(Debug)]
pub enum OpenError {
    /// The file could not be read.
    Read(PathBuf, io::Error),
    /// It is not a valid document.
    Document(PathBuf, Box<DocError>),
    /// The built-in node types could not be registered.
    Registry(RegistryError),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Read(path, e) => write!(f, "{}: {e}", path.display()),
            OpenError::Document(path, e) => write!(f, "{}: {e}", path.display()),
            OpenError::Registry(e) => write!(f, "built-in nodes: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

/// What a shown node computed this frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeResult {
    /// One value per shown output; `None` when it has none.
    pub outputs: Vec<Option<Value>>,
    /// Why it failed, if it did.
    pub problem: Option<String>,
}

/// An open room.
#[derive(Debug)]
pub struct Session {
    name: String,
    graph: Graph,
    cooker: Cooker,
    system: Option<Arc<dyn SystemHost>>,
    nodes: Vec<Shown>,
    links: Vec<ShownLink>,
    results: Vec<NodeResult>,
    selected: Option<NodeId>,
    inspection: Option<Inspection>,
    frame: u64,
    playing: bool,
    /// Time not yet turned into frames, in frames.
    carry: f64,
    file: Option<File>,
    min_tier: Tier,
    history: History,
    /// The group of the gesture under way.
    gesture: Option<GroupId>,
    groups: u64,
    /// The graph as saved.
    saved: GraphSnapshot,
    dirty: bool,
}

impl Session {
    /// A session over `graph`, at frame 0.
    #[must_use]
    pub fn new(name: String, graph: Graph, system: Option<Arc<dyn SystemHost>>) -> Session {
        let nodes = root_nodes(&graph);
        let links = links(&graph, &nodes);
        let mut session = Session {
            name,
            graph,
            cooker: Cooker::new(),
            system,
            nodes,
            links,
            results: Vec::new(),
            selected: None,
            inspection: None,
            frame: 0,
            playing: false,
            carry: 0.0,
            file: None,
            min_tier: Tier::default(),
            history: History::new(),
            gesture: None,
            groups: 0,
            saved: GraphSnapshot::default(),
            dirty: false,
        };
        session.saved = session.graph.snapshot();
        session.rewind();
        session
    }

    /// The room's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The graph.
    #[must_use]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The nodes of the root network.
    #[must_use]
    pub fn nodes(&self) -> &[Shown] {
        &self.nodes
    }

    /// The links between them.
    #[must_use]
    pub fn links(&self) -> &[ShownLink] {
        &self.links
    }

    /// What each node computed, in the order of [`Session::nodes`].
    #[must_use]
    pub fn results(&self) -> &[NodeResult] {
        &self.results
    }

    /// The index of the node called `name`.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// The selected node.
    #[must_use]
    pub fn selected(&self) -> Option<usize> {
        let id = self.selected?;
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The selected node, inspected.
    #[must_use]
    pub fn inspection(&self) -> Option<&Inspection> {
        self.inspection.as_ref()
    }

    /// The current frame.
    #[must_use]
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The current time in seconds.
    #[must_use]
    pub fn time(&self) -> f64 {
        // Exact for every frame count a session can reach.
        #[allow(clippy::cast_precision_loss)]
        let frame = self.frame as f64;
        frame / FPS
    }

    /// Whether the clock runs.
    #[must_use]
    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Select a node by index (`None`: nothing).
    pub fn select(&mut self, node: Option<usize>) {
        self.selected = node.and_then(|n| self.nodes.get(n)).map(|n| n.id);
        self.inspect();
    }

    /// Whether the graph differs from the saved one.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// Whether there is an edit to undo.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    /// Whether there is an edit to redo.
    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    /// Start or stop the clock.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
        self.carry = 0.0;
    }

    /// One frame forward.
    pub fn step(&mut self) {
        self.frame += 1;
        self.tick();
        self.cook();
    }

    /// Back to frame 0, with event nodes in their initial state.
    pub fn rewind(&mut self) {
        self.cooker = Cooker::new();
        self.cooker.set_system(self.system.clone());
        self.frame = 0;
        self.carry = 0.0;
        self.tick();
        self.cook();
    }

    /// Let `seconds` pass while playing. Every frame is ticked, so event
    /// nodes see each one. Returns how many frames passed.
    pub fn advance(&mut self, seconds: f64) -> u32 {
        if !self.playing || !seconds.is_finite() || seconds <= 0.0 {
            return 0;
        }
        self.carry += seconds * FPS;
        let mut frames = 0;
        // The slack keeps a step of exactly one frame from rounding away.
        while self.carry >= 1.0 - 1e-6 && frames < MAX_CATCH_UP {
            self.carry -= 1.0;
            self.frame += 1;
            self.tick();
            frames += 1;
        }
        // A stall is not caught up later.
        self.carry = self.carry.clamp(0.0, 1.0);
        if frames > 0 {
            self.cook();
        }
        frames
    }

    fn tick(&mut self) {
        let clock = CookContext {
            frame: self.frame,
            time: self.time(),
            dt: 1.0 / FPS,
        };
        self.cooker.tick(&self.graph, clock, &[]);
    }

    /// Compute what the nodes show. The cooker's cache keeps this to what
    /// changed.
    fn cook(&mut self) {
        let (graph, cooker) = (&self.graph, &mut self.cooker);
        self.results = self
            .nodes
            .iter()
            .map(|node| {
                let outputs = node
                    .outputs
                    .iter()
                    .map(|p| output(graph, cooker, p.node, &p.port))
                    .collect();
                // Sinks have no outputs, and are cooked to see their errors.
                if node.outputs.is_empty() {
                    let _ = cooker.cook(graph, node.id);
                }
                let owners = std::iter::once(node.id).chain(node.outputs.iter().map(|p| p.node));
                let problem = owners
                    .filter_map(|id| cooker.error(id).or_else(|| cooker.event_error(id)))
                    .map(ToString::to_string)
                    .next();
                NodeResult { outputs, problem }
            })
            .collect();
        self.inspect();
    }

    fn inspect(&mut self) {
        let selected = self.selected();
        let (graph, cooker, nodes) = (&self.graph, &mut self.cooker, &self.nodes);
        self.inspection = selected.and_then(|i| {
            let problem = self.results.get(i).and_then(|r| r.problem.clone());
            Some(inspect(graph, cooker, nodes, nodes.get(i)?, problem))
        });
    }
}
