// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The data side of the node canvas: [`NodeGraphModel`] (what the canvas
//! reads and the edits it requests) and [`DemoGraph`], an in-memory model.
//!
//! The canvas is data-agnostic: Dream Mode adapts its kernel graph to this
//! trait in the app (the UI crates do not depend on kernel crates).

use std::collections::VecDeque;

use livspeil_ui_core::SharedString;
use livspeil_ui_layout::Point;
use livspeil_ui_theme::Color;

/// A node's id (stable across edits).
pub type NodeId = u64;

/// Which side of a node a port is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PortSide {
    /// Inputs, on the left edge.
    Input,
    /// Outputs, on the right edge.
    Output,
}

/// A port: node, side and index among that side's ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PortRef {
    /// The node.
    pub node: NodeId,
    /// Input or output.
    pub side: PortSide,
    /// Index on that side.
    pub index: usize,
}

impl PortRef {
    /// An input port.
    #[must_use]
    pub fn input(node: NodeId, index: usize) -> Self {
        PortRef {
            node,
            side: PortSide::Input,
            index,
        }
    }

    /// An output port.
    #[must_use]
    pub fn output(node: NodeId, index: usize) -> Self {
        PortRef {
            node,
            side: PortSide::Output,
            index,
        }
    }
}

/// A port's name and type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortInfo {
    /// Label.
    pub name: SharedString,
    /// Type name (colors the port; compatibility is up to the model).
    pub ty: SharedString,
}

impl PortInfo {
    /// A port.
    #[must_use]
    pub fn new(name: impl Into<SharedString>, ty: impl Into<SharedString>) -> Self {
        PortInfo {
            name: name.into(),
            ty: ty.into(),
        }
    }
}

/// A parameter's value (edited in the card).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    /// A number in a range (a `NumberField`).
    Number {
        /// Value.
        value: f64,
        /// Minimum.
        min: f64,
        /// Maximum.
        max: f64,
        /// Step.
        step: f64,
    },
    /// On/off (a `Toggle`).
    Toggle(bool),
}

/// A parameter row.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    /// Label.
    pub name: SharedString,
    /// Value.
    pub value: ParamValue,
}

/// A link from an output to an input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Link {
    /// The output.
    pub from: PortRef,
    /// The input.
    pub to: PortRef,
}

/// One change a model reports through [`NodeGraphModel::changes_since`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GraphChange {
    /// Only the node's position changed.
    Moved(NodeId),
    /// The node's title, ports or parameters changed. The set of nodes, their
    /// paint order and the links did not.
    Edited(NodeId),
}

/// How many changes a [`ChangeLog`] remembers.
const LOG_CAPACITY: usize = 4096;

/// A revision counter that remembers recent changes, for models to
/// implement [`NodeGraphModel::revision`] and
/// [`NodeGraphModel::changes_since`] with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChangeLog {
    revision: u64,
    /// Changes after revisions older than this are not all known.
    known_from: u64,
    entries: VecDeque<(u64, GraphChange)>,
}

impl ChangeLog {
    /// An empty log at revision 0.
    #[must_use]
    pub fn new() -> Self {
        ChangeLog::default()
    }

    /// The current revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Record one change as a new revision.
    pub fn record(&mut self, change: GraphChange) {
        self.revision += 1;
        if self.entries.len() == LOG_CAPACITY
            && let Some((dropped, _)) = self.entries.pop_front()
        {
            self.known_from = dropped;
        }
        self.entries.push_back((self.revision, change));
    }

    /// Record a change that [`GraphChange`] cannot describe (nodes or links
    /// added, removed or reordered) as a new revision.
    pub fn restructure(&mut self) {
        self.revision += 1;
        self.known_from = self.revision;
        self.entries.clear();
    }

    /// What changed after revision `since`, each change once, or `None` if
    /// the log cannot tell.
    #[must_use]
    pub fn since(&self, since: u64) -> Option<Vec<GraphChange>> {
        if since < self.known_from || since > self.revision {
            return None;
        }
        let first = self.entries.partition_point(|(r, _)| *r <= since);
        let mut changes: Vec<GraphChange> = self.entries.range(first..).map(|(_, c)| *c).collect();
        changes.sort_unstable();
        changes.dedup();
        Some(changes)
    }
}

/// A graph the [`NodeCanvas`](crate::NodeCanvas) shows and edits.
///
/// The canvas keeps a copy of what it reads and brings it up to date when
/// [`NodeGraphModel::revision`] changes, so implementations bump the
/// revision on every change and notify. A model that also reports *what*
/// changed ([`NodeGraphModel::changes_since`], see [`ChangeLog`]) makes the
/// cost of a change proportional to it instead of to the graph. Edits are
/// requests: the model may refuse them.
pub trait NodeGraphModel: 'static {
    /// Changes whenever anything the canvas reads changes.
    fn revision(&self) -> u64;
    /// Everything that changed after revision `since`, or `None` if the
    /// model cannot tell (the canvas then reads the whole graph). The
    /// default is `None`. Must be `None` if nodes or links were added,
    /// removed or reordered.
    fn changes_since(&self, _since: u64) -> Option<Vec<GraphChange>> {
        None
    }
    /// Node ids in paint order (later nodes are drawn on top).
    fn node_ids(&self) -> Vec<NodeId>;
    /// A node's title.
    fn title(&self, node: NodeId) -> SharedString;
    /// A node's top-left in canvas space.
    fn position(&self, node: NodeId) -> Point;
    /// Input ports.
    fn inputs(&self, node: NodeId) -> Vec<PortInfo>;
    /// Output ports.
    fn outputs(&self, node: NodeId) -> Vec<PortInfo>;
    /// Parameter rows (default none).
    fn params(&self, _node: NodeId) -> Vec<ParamInfo> {
        Vec::new()
    }
    /// Links.
    fn links(&self) -> Vec<Link>;
    /// What is wrong with a node, if something is: its card is marked and
    /// the text becomes its accessible description. Report a change of it
    /// as [`GraphChange::Edited`].
    fn problem(&self, _node: NodeId) -> Option<SharedString> {
        None
    }

    /// Whether a link from output `from` to input `to` is allowed (the
    /// type check). Default: different nodes and equal type names.
    fn can_connect(&self, from: PortRef, to: PortRef) -> bool {
        if from.node == to.node || from.side != PortSide::Output || to.side != PortSide::Input {
            return false;
        }
        let ty = |p: PortRef, ports: Vec<PortInfo>| ports.get(p.index).map(|i| i.ty.clone());
        let a = ty(from, self.outputs(from.node));
        let b = ty(to, self.inputs(to.node));
        a.is_some() && a == b
    }

    /// The color of a port type. Default: a palette entry picked by a hash
    /// of the name.
    fn type_color(&self, ty: &str) -> Color {
        palette_color(ty)
    }

    /// Move a node.
    fn set_position(&mut self, node: NodeId, position: Point);
    /// Add a link (an input takes one link: replace any existing one).
    fn connect(&mut self, link: Link);
    /// Remove a link.
    fn disconnect(&mut self, _link: Link) {}
    /// A drag that moved nodes, or made or removed a link, ended: the edits
    /// since it started belong together (one undo step).
    fn end_gesture(&mut self) {}
    /// Set a parameter.
    fn set_param(&mut self, _node: NodeId, _index: usize, _value: ParamValue) {}
}

/// A stable color for a type name.
#[must_use]
pub fn palette_color(ty: &str) -> Color {
    const HUES: [f32; 8] = [145.0, 25.0, 250.0, 65.0, 310.0, 195.0, 95.0, 345.0];
    let mut h: u32 = 0x811c_9dc5;
    for b in ty.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    let hue = HUES[(h % 8) as usize];
    Color::oklch(0.74, 0.14, hue, 1.0)
}

/// One node of a [`DemoGraph`].
#[derive(Clone, Debug, PartialEq)]
pub struct DemoNode {
    /// Id.
    pub id: NodeId,
    /// Title.
    pub title: SharedString,
    /// Top-left.
    pub position: Point,
    /// Inputs.
    pub inputs: Vec<PortInfo>,
    /// Outputs.
    pub outputs: Vec<PortInfo>,
    /// Parameters.
    pub params: Vec<ParamInfo>,
}

/// An in-memory graph for demos, tests and benchmarks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DemoGraph {
    /// Nodes in paint order.
    pub nodes: Vec<DemoNode>,
    /// Links.
    pub links: Vec<Link>,
    log: ChangeLog,
}

fn num(value: f64, min: f64, max: f64, step: f64) -> ParamValue {
    ParamValue::Number {
        value,
        min,
        max,
        step,
    }
}

impl DemoGraph {
    /// An empty graph.
    #[must_use]
    pub fn new() -> Self {
        DemoGraph::default()
    }

    /// Add a node; returns its id.
    pub fn add(
        &mut self,
        title: &str,
        position: Point,
        inputs: &[(&str, &str)],
        outputs: &[(&str, &str)],
        params: Vec<ParamInfo>,
    ) -> NodeId {
        let id = self.nodes.len() as NodeId + 1;
        let ports = |p: &[(&str, &str)]| p.iter().map(|(n, t)| PortInfo::new(*n, *t)).collect();
        self.nodes.push(DemoNode {
            id,
            title: title.into(),
            position,
            inputs: ports(inputs),
            outputs: ports(outputs),
            params,
        });
        self.log.restructure();
        id
    }

    fn node(&self, id: NodeId) -> Option<&DemoNode> {
        // Ids are 1-based indices unless nodes were reordered.
        let i = usize::try_from(id).ok()?.checked_sub(1)?;
        match self.nodes.get(i) {
            Some(n) if n.id == id => Some(n),
            _ => self.nodes.iter().find(|n| n.id == id),
        }
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut DemoNode> {
        let i = usize::try_from(id).ok()?.checked_sub(1)?;
        if self.nodes.get(i).is_some_and(|n| n.id == id) {
            return self.nodes.get_mut(i);
        }
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// A small shader-like graph: noise and time into a mix, a color ramp,
    /// a blur, an output.
    #[must_use]
    pub fn sample() -> Self {
        let mut g = DemoGraph::new();
        let p = Point::new;
        let time = g.add("Time", p(40.0, 40.0), &[], &[("Seconds", "float")], vec![]);
        let noise = g.add(
            "Noise",
            p(40.0, 150.0),
            &[("Seed", "float")],
            &[("Field", "float")],
            vec![ParamInfo {
                name: "Scale".into(),
                value: num(4.0, 0.0, 100.0, 0.5),
            }],
        );
        let mix = g.add(
            "Mix",
            p(280.0, 70.0),
            &[("A", "float"), ("B", "float")],
            &[("Out", "float")],
            vec![ParamInfo {
                name: "Factor".into(),
                value: num(0.5, 0.0, 1.0, 0.01),
            }],
        );
        let ramp = g.add(
            "Color Ramp",
            p(500.0, 60.0),
            &[("Fac", "float")],
            &[("Color", "color")],
            vec![ParamInfo {
                name: "Clamp".into(),
                value: ParamValue::Toggle(true),
            }],
        );
        let blur = g.add(
            "Blur",
            p(500.0, 230.0),
            &[("Image", "image"), ("Radius", "float")],
            &[("Image", "image")],
            vec![],
        );
        let out = g.add(
            "Output",
            p(740.0, 120.0),
            &[("Color", "color"), ("Image", "image")],
            &[],
            vec![],
        );
        g.links = vec![
            Link {
                from: PortRef::output(time, 0),
                to: PortRef::input(mix, 0),
            },
            Link {
                from: PortRef::output(noise, 0),
                to: PortRef::input(mix, 1),
            },
            Link {
                from: PortRef::output(mix, 0),
                to: PortRef::input(ramp, 0),
            },
            Link {
                from: PortRef::output(ramp, 0),
                to: PortRef::input(out, 0),
            },
            Link {
                from: PortRef::output(blur, 0),
                to: PortRef::input(out, 1),
            },
        ];
        g.log.restructure();
        g
    }

    /// A large deterministic graph: `nodes` nodes on a grid (columns of
    /// 20) and `links` links between nearby columns.
    #[must_use]
    pub fn grid(nodes: usize, links: usize) -> Self {
        let mut g = DemoGraph::new();
        let types = ["float", "color", "vec2"];
        for i in 0..nodes {
            let (col, row) = (i / 20, i % 20);
            #[allow(clippy::cast_precision_loss)]
            let pos = Point::new(col as f32 * 260.0, row as f32 * 170.0);
            let t = types[i % 3];
            g.add(
                &format!("Node {i}"),
                pos,
                &[("A", t), ("B", "float")],
                &[("Out", t), ("Mask", "float")],
                vec![ParamInfo {
                    name: "Amount".into(),
                    value: num(0.5, 0.0, 1.0, 0.01),
                }],
            );
        }
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let n = nodes as u64;
        while g.links.len() < links && n > 20 {
            let a = next() % n;
            let col = a / 20;
            let b_col = col + 1 + next() % 2;
            let b = b_col * 20 + next() % 20;
            if b >= n {
                continue;
            }
            g.links.push(Link {
                from: PortRef::output(a + 1, 1),
                to: PortRef::input(b + 1, (next() % 2) as usize),
            });
        }
        g.log.restructure();
        g
    }
}

impl NodeGraphModel for DemoGraph {
    fn revision(&self) -> u64 {
        self.log.revision()
    }
    fn changes_since(&self, since: u64) -> Option<Vec<GraphChange>> {
        self.log.since(since)
    }
    fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.iter().map(|n| n.id).collect()
    }
    fn title(&self, node: NodeId) -> SharedString {
        self.node(node)
            .map_or_else(|| "?".into(), |n| n.title.clone())
    }
    fn position(&self, node: NodeId) -> Point {
        self.node(node).map_or(Point::ZERO, |n| n.position)
    }
    fn inputs(&self, node: NodeId) -> Vec<PortInfo> {
        self.node(node)
            .map(|n| n.inputs.clone())
            .unwrap_or_default()
    }
    fn outputs(&self, node: NodeId) -> Vec<PortInfo> {
        self.node(node)
            .map(|n| n.outputs.clone())
            .unwrap_or_default()
    }
    fn params(&self, node: NodeId) -> Vec<ParamInfo> {
        self.node(node)
            .map(|n| n.params.clone())
            .unwrap_or_default()
    }
    fn links(&self) -> Vec<Link> {
        self.links.clone()
    }
    fn type_color(&self, ty: &str) -> Color {
        match ty {
            "float" => Color::oklch(0.72, 0.03, 260.0, 1.0),
            "color" => Color::oklch(0.78, 0.13, 85.0, 1.0),
            "image" => Color::oklch(0.70, 0.15, 300.0, 1.0),
            "vec2" => Color::oklch(0.70, 0.13, 230.0, 1.0),
            other => palette_color(other),
        }
    }
    fn set_position(&mut self, node: NodeId, position: Point) {
        if let Some(n) = self.node_mut(node)
            && n.position != position
        {
            n.position = position;
            self.log.record(GraphChange::Moved(node));
        }
    }
    fn connect(&mut self, link: Link) {
        self.links.retain(|l| l.to != link.to);
        self.links.push(link);
        self.log.restructure();
    }
    fn disconnect(&mut self, link: Link) {
        let before = self.links.len();
        self.links.retain(|l| *l != link);
        if self.links.len() != before {
            self.log.restructure();
        }
    }
    fn set_param(&mut self, node: NodeId, index: usize, value: ParamValue) {
        if let Some(p) = self.node_mut(node).and_then(|n| n.params.get_mut(index))
            && p.value != value
        {
            p.value = value;
            self.log.record(GraphChange::Edited(node));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_reports_each_change_once_and_admits_what_it_forgot() {
        let mut log = ChangeLog::new();
        assert_eq!(log.since(0), Some(vec![]));
        log.record(GraphChange::Edited(2));
        log.record(GraphChange::Moved(1));
        log.record(GraphChange::Moved(1));
        assert_eq!(log.revision(), 3);
        assert_eq!(
            log.since(0),
            Some(vec![GraphChange::Moved(1), GraphChange::Edited(2)])
        );
        assert_eq!(log.since(1), Some(vec![GraphChange::Moved(1)]));
        assert_eq!(log.since(3), Some(vec![]));
        // A revision from the future belongs to another log.
        assert_eq!(log.since(4), None);

        log.restructure();
        assert_eq!(log.since(3), None);
        assert_eq!(log.since(4), Some(vec![]));

        for i in 0..=LOG_CAPACITY as u64 {
            log.record(GraphChange::Moved(i));
        }
        assert_eq!(log.since(4), None, "the oldest change was dropped");
        assert_eq!(log.since(5).map(|c| c.len()), Some(LOG_CAPACITY));
    }
}
