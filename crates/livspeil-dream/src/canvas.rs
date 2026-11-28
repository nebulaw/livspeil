// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The session as a node canvas model. This slice only shows: the canvas's
//! edit requests are not taken.

use livspeil_ui_core::SharedString;
use livspeil_ui_layout::Point;
use livspeil_ui_widgets::{
    CardMetrics, ChangeLog, GraphChange, Link, NodeGraphModel, NodeId, ParamValue, PortInfo,
    PortRef,
};

use crate::arrange::arrange;
use crate::session::Session;
use crate::shown::{Port, Shown};

/// What the canvas shows of a session. Canvas node ids are the node's index
/// plus one.
#[derive(Debug)]
pub struct GraphView {
    nodes: Vec<Shown>,
    positions: Vec<Point>,
    links: Vec<Link>,
    problems: Vec<Option<SharedString>>,
    log: ChangeLog,
}

fn id(index: usize) -> NodeId {
    index as NodeId + 1
}

fn index(id: NodeId) -> Option<usize> {
    usize::try_from(id).ok()?.checked_sub(1)
}

/// The node index of canvas node `id`.
#[must_use]
pub fn node_index(id: NodeId) -> Option<usize> {
    index(id)
}

fn problems(session: &Session) -> impl Iterator<Item = Option<SharedString>> + '_ {
    session
        .results()
        .iter()
        .map(|r| r.problem.as_deref().map(SharedString::from))
}

impl GraphView {
    /// The view of `session`'s graph, with cards sized by `metrics`.
    #[must_use]
    pub fn new(session: &Session, metrics: CardMetrics) -> Self {
        let nodes = session.nodes().to_vec();
        // The kernel bounds positions well inside `f32`.
        #[allow(clippy::cast_possible_truncation)]
        let placed: Vec<Option<[f32; 2]>> = nodes
            .iter()
            .map(|n| {
                session
                    .graph()
                    .position(n.id)
                    .map(|[x, y]| [x as f32, y as f32])
            })
            .collect();
        let heights: Vec<f32> = nodes
            .iter()
            .map(|n| metrics.card_size(n.inputs.len(), n.outputs.len(), 0).height)
            .collect();
        let positions = arrange(&nodes, session.links(), &placed, &heights)
            .into_iter()
            .map(|[x, y]| Point::new(x, y))
            .collect();
        let links = session
            .links()
            .iter()
            .map(|l| Link {
                from: PortRef::output(id(l.from.0), l.from.1),
                to: PortRef::input(id(l.to.0), l.to.1),
            })
            .collect();
        GraphView {
            nodes,
            positions,
            links,
            problems: problems(session).collect(),
            log: ChangeLog::new(),
        }
    }

    /// Take over which nodes now have problems. Returns whether any changed.
    pub fn follow(&mut self, session: &Session) -> bool {
        let mut changed = false;
        for (i, (old, new)) in self.problems.iter_mut().zip(problems(session)).enumerate() {
            if *old != new {
                *old = new;
                self.log.record(GraphChange::Edited(id(i)));
                changed = true;
            }
        }
        changed
    }

    fn node(&self, id: NodeId) -> Option<&Shown> {
        self.nodes.get(index(id)?)
    }
}

fn ports(ports: &[Port]) -> Vec<PortInfo> {
    ports
        .iter()
        .map(|p| PortInfo::new(p.name.clone(), p.ty.name()))
        .collect()
}

impl NodeGraphModel for GraphView {
    fn revision(&self) -> u64 {
        self.log.revision()
    }
    fn changes_since(&self, since: u64) -> Option<Vec<GraphChange>> {
        self.log.since(since)
    }
    fn node_ids(&self) -> Vec<NodeId> {
        (0..self.nodes.len()).map(id).collect()
    }
    fn title(&self, node: NodeId) -> SharedString {
        self.node(node)
            .map_or_else(|| "?".into(), |n| n.name.as_str().into())
    }
    fn position(&self, node: NodeId) -> Point {
        index(node)
            .and_then(|i| self.positions.get(i))
            .copied()
            .unwrap_or(Point::ZERO)
    }
    fn inputs(&self, node: NodeId) -> Vec<PortInfo> {
        self.node(node)
            .map(|n| ports(&n.inputs))
            .unwrap_or_default()
    }
    fn outputs(&self, node: NodeId) -> Vec<PortInfo> {
        self.node(node)
            .map(|n| ports(&n.outputs))
            .unwrap_or_default()
    }
    fn links(&self) -> Vec<Link> {
        self.links.clone()
    }
    fn problem(&self, node: NodeId) -> Option<SharedString> {
        self.problems.get(index(node)?)?.clone()
    }
    fn can_connect(&self, _from: PortRef, _to: PortRef) -> bool {
        false
    }
    fn set_position(&mut self, _node: NodeId, _position: Point) {}
    fn connect(&mut self, _link: Link) {}
    fn set_param(&mut self, _node: NodeId, _index: usize, _value: ParamValue) {}
}
