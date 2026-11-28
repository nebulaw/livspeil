// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The open room: its graph, what it currently computes, the frame clock
//! and the selection. Everything the views show comes from here.

use std::fmt;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};

use livspeil_cmd::{Command, GroupId, History, Origin};
use livspeil_doc::{DocError, Document, Tier, package_dir, parse, rewrite, room_file};
use livspeil_graph::{Cooker, Graph, GraphSnapshot, NodeId};
use livspeil_node_api::{CookContext, NodeRegistry, RegistryError, SystemHost, Value};
use livspeil_nodes_core::{StdFs, register_all};

use crate::edit::Entry;
use crate::inspect::{Inspection, inspect, output};
use crate::shown::{Port, Shown, ShownLink, links, root_nodes};

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

/// The file a session was opened from.
#[derive(Debug)]
struct File {
    path: PathBuf,
    /// Its text when last read or written.
    text: String,
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

fn registry() -> Result<Arc<NodeRegistry>, OpenError> {
    let mut registry = NodeRegistry::new();
    register_all(&mut registry).map_err(OpenError::Registry)?;
    Ok(Arc::new(registry))
}

/// Write `text` to `path` without ever leaving a part of it there: write a
/// neighbor, then rename it over.
fn replace(path: &Path, text: &str) -> io::Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".saving");
    let temporary = path.with_file_name(name);
    let written = fs::File::create(&temporary).and_then(|mut f| {
        f.write_all(text.as_bytes())?;
        f.sync_all()
    });
    let result = written
        .and_then(|()| fs::set_permissions(&temporary, fs::metadata(path)?.permissions()))
        .and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        // Nothing to do if even removing it fails.
        let _ = fs::remove_file(&temporary);
    }
    result
}

impl Session {
    /// Open `path`: a `Name.room/` package or a `room.kdl` file.
    ///
    /// # Errors
    /// [`OpenError`] when it cannot be read or is not a valid room.
    pub fn open(path: &Path) -> Result<Session, OpenError> {
        let file = room_file(path);
        let text = fs::read_to_string(&file).map_err(|e| OpenError::Read(file.clone(), e))?;
        let invalid = |e| OpenError::Document(file.clone(), Box::new(e));
        let doc = parse(&text).map_err(invalid)?;
        let graph = doc.to_graph(registry()?).map_err(invalid)?;
        // File nodes read inside the package, as `livspeil cook` has them.
        // Without a directory they report the lack themselves.
        let system = StdFs::new(package_dir(&file))
            .ok()
            .map(|fs| Arc::new(fs) as Arc<dyn SystemHost>);
        let mut session = Session::new(doc.name, graph, system);
        session.min_tier = doc.min_tier;
        session.file = Some(File { path: file, text });
        Ok(session)
    }

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

    /// Apply a command as part of the gesture under way, or on its own.
    fn run(&mut self, command: Command, in_gesture: bool) -> Result<(), String> {
        let group = if in_gesture {
            let groups = &mut self.groups;
            Some(*self.gesture.get_or_insert_with(|| {
                *groups += 1;
                GroupId(*groups)
            }))
        } else {
            self.end_gesture();
            None
        };
        self.history
            .apply(&mut self.graph, command, Origin::Ui, group)
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// The gesture under way ended: later edits are a new undo step.
    pub fn end_gesture(&mut self) {
        if self.gesture.take().is_some() {
            self.history.seal();
            self.dirty = self.graph.snapshot() != self.saved;
        }
    }

    /// Place a node, as part of a drag. Positions change nothing the nodes
    /// compute, so nothing else is brought up to date.
    ///
    /// # Errors
    /// Why the node cannot be placed there.
    pub fn move_node(&mut self, node: NodeId, position: [f64; 2]) -> Result<(), String> {
        let command = Command::SetPosition {
            node,
            position: Some(position),
        };
        self.run(command, true)?;
        // Settled when the gesture ends.
        self.dirty = true;
        Ok(())
    }

    /// Whether output `from` may feed input `to`, replacing its link.
    #[must_use]
    pub fn can_connect(&self, from: &Port, to: &Port) -> bool {
        self.graph
            .check_link(from.node, &from.port, to.node, &to.port)
            .is_ok()
    }

    fn linked(&self, to: &Port) -> bool {
        matches!(self.graph.link_source(to.node, &to.port), Ok(Some(_)))
    }

    fn unlink(to: &Port) -> Command {
        Command::Unlink {
            to: to.node,
            to_port: to.port.clone(),
        }
    }

    /// Link output `from` to input `to`, replacing what fed it. Part of a
    /// wire drag.
    ///
    /// # Errors
    /// Why the link is not allowed.
    pub fn connect(&mut self, from: &Port, to: &Port) -> Result<(), String> {
        let link = Command::Link {
            from: from.node,
            from_port: from.port.clone(),
            to: to.node,
            to_port: to.port.clone(),
        };
        let command = if self.linked(to) {
            Command::Transaction(vec![Self::unlink(to), link])
        } else {
            link
        };
        self.run(command, true)?;
        self.refresh();
        Ok(())
    }

    /// Remove the link of input `to`. Part of a wire drag.
    ///
    /// # Errors
    /// Why it cannot be removed.
    pub fn disconnect(&mut self, to: &Port) -> Result<(), String> {
        self.run(Self::unlink(to), true)?;
        self.refresh();
        Ok(())
    }

    /// Give input `port` what a field asked for. One undo step.
    ///
    /// # Errors
    /// The kernel's reason for refusing it; nothing changes.
    pub fn enter(&mut self, port: &Port, entry: Entry) -> Result<(), String> {
        let (node, name) = (port.node, port.port.clone());
        let has_expression = matches!(self.graph.expression(node, &name), Ok(Some(_)));
        let has_value = matches!(self.graph.input_value(node, &name), Ok(Some(_)));
        let clear_expression = || Command::ClearExpression {
            node,
            port: name.clone(),
        };
        let clear_value = || Command::ClearInput {
            node,
            port: name.clone(),
        };
        let mut steps = Vec::new();
        match entry {
            Entry::Clear => {
                steps.extend(has_expression.then(clear_expression));
                steps.extend(has_value.then(clear_value));
            }
            Entry::Expression(source) => {
                if self.graph.expression(node, &name).ok().flatten() != Some(source.as_str()) {
                    steps.push(Command::SetExpression {
                        node,
                        port: name.clone(),
                        source,
                    });
                }
            }
            Entry::Value(value) => {
                // An expression would keep winning over the value.
                steps.extend(has_expression.then(clear_expression));
                if self.graph.input_value(node, &name).ok().flatten() != Some(&value) {
                    steps.push(Command::SetInput {
                        node,
                        port: name.clone(),
                        value,
                    });
                }
            }
        }
        if steps.is_empty() {
            return Ok(());
        }
        self.run(Command::Transaction(steps), false)?;
        self.refresh();
        Ok(())
    }

    /// Rename a node. One undo step.
    ///
    /// # Errors
    /// Why the name is refused.
    pub fn rename(&mut self, node: NodeId, name: &str) -> Result<(), String> {
        if self.graph.node_name(node) == Some(name) {
            return Ok(());
        }
        let command = Command::Rename {
            node,
            name: name.to_owned(),
        };
        self.run(command, false)?;
        self.refresh();
        Ok(())
    }

    /// Undo the latest edit.
    ///
    /// # Errors
    /// When there is none.
    pub fn undo(&mut self) -> Result<(), String> {
        self.end_gesture();
        self.history
            .undo(&mut self.graph)
            .map_err(|e| e.to_string())?;
        self.refresh();
        Ok(())
    }

    /// Redo the latest undone edit.
    ///
    /// # Errors
    /// When there is none.
    pub fn redo(&mut self) -> Result<(), String> {
        self.end_gesture();
        self.history
            .redo(&mut self.graph)
            .map_err(|e| e.to_string())?;
        self.refresh();
        Ok(())
    }

    /// Write the room back to its file, keeping its comments.
    ///
    /// # Errors
    /// When there is no file, it changed on disk since it was read, or it
    /// cannot be written. The file is then left as it was.
    pub fn save(&mut self) -> Result<(), String> {
        self.end_gesture();
        let Some(file) = &mut self.file else {
            return Err("this room has no file".to_owned());
        };
        let shown = file.path.display().to_string();
        let on_disk = fs::read_to_string(&file.path).map_err(|e| format!("{shown}: {e}"))?;
        if on_disk != file.text {
            return Err(format!("{shown} changed on disk; not overwriting it"));
        }
        let mut doc =
            Document::from_graph(&self.graph, self.name.clone()).map_err(|e| e.to_string())?;
        doc.min_tier = self.min_tier;
        let text = rewrite(&file.text, &doc).map_err(|e| e.to_string())?;
        replace(&file.path, &text).map_err(|e| format!("{shown}: {e}"))?;
        file.text = text;
        self.saved = self.graph.snapshot();
        self.dirty = false;
        Ok(())
    }

    /// Bring everything derived from the graph up to date after an edit.
    fn refresh(&mut self) {
        self.nodes = root_nodes(&self.graph);
        self.links = links(&self.graph, &self.nodes);
        self.dirty = self.graph.snapshot() != self.saved;
        self.cook();
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
