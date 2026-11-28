// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The editor: the session as a node canvas model.
//!
//! It owns the [`Session`], so what the canvas edits is the graph itself;
//! what it keeps besides is only how the graph looks, which
//! [`Editor::sync`] brings up to date after any change.

use std::collections::HashMap;

use livspeil_graph::NodeId as GraphId;
use livspeil_ui_core::SharedString;
use livspeil_ui_layout::Point;
use livspeil_ui_widgets::{
    CardMetrics, ChangeLog, GraphChange, Link, NodeGraphModel, NodeId, ParamValue, PortInfo,
    PortRef, PortSide,
};

use crate::arrange::arrange;
use crate::session::Session;
use crate::shown::Port;

/// One node as the canvas shows it.
#[derive(Debug, Clone, PartialEq)]
struct Card {
    id: GraphId,
    title: SharedString,
    inputs: Vec<PortInfo>,
    outputs: Vec<PortInfo>,
    position: Point,
    problem: Option<SharedString>,
}

/// An open room being edited. Canvas node ids are the node's index in
/// [`Session::nodes`] plus one.
#[derive(Debug)]
pub struct Editor {
    session: Session,
    /// Where the nodes the room does not place were put when it was opened.
    /// They stay there.
    arranged: HashMap<GraphId, Point>,
    cards: Vec<Card>,
    links: Vec<Link>,
    log: ChangeLog,
    /// Why the last edit or save was refused.
    message: Option<String>,
}

fn id(index: usize) -> NodeId {
    index as NodeId + 1
}

/// The node index of canvas node `id`.
#[must_use]
pub fn node_index(id: NodeId) -> Option<usize> {
    usize::try_from(id).ok()?.checked_sub(1)
}

/// The kernel bounds positions well inside `f32`.
#[allow(clippy::cast_possible_truncation)]
fn point([x, y]: [f64; 2]) -> Point {
    Point::new(x as f32, y as f32)
}

fn ports(ports: &[Port]) -> Vec<PortInfo> {
    ports
        .iter()
        .map(|p| PortInfo::new(p.name.clone(), p.ty.name()))
        .collect()
}

impl Editor {
    /// An editor of `session`, with cards sized by `metrics`.
    #[must_use]
    pub fn new(session: Session, metrics: CardMetrics) -> Self {
        let nodes = session.nodes();
        let placed: Vec<Option<[f32; 2]>> = nodes
            .iter()
            .map(|n| {
                let p = session.graph().position(n.id).map(point)?;
                Some([p.x, p.y])
            })
            .collect();
        let heights: Vec<f32> = nodes
            .iter()
            .map(|n| metrics.card_size(n.inputs.len(), n.outputs.len(), 0).height)
            .collect();
        let arranged = arrange(nodes, session.links(), &placed, &heights)
            .into_iter()
            .zip(nodes)
            .zip(&placed)
            .filter(|(_, placed)| placed.is_none())
            .map(|(([x, y], node), _)| (node.id, Point::new(x, y)))
            .collect();
        let mut editor = Editor {
            session,
            arranged,
            cards: Vec::new(),
            links: Vec::new(),
            log: ChangeLog::new(),
            message: None,
        };
        editor.sync();
        editor
    }

    /// The session.
    #[must_use]
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// Why the last edit or save was refused.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }

    /// Change the session, then show the change.
    pub fn change<R>(&mut self, change: impl FnOnce(&mut Session) -> R) -> R {
        let out = change(&mut self.session);
        self.sync();
        out
    }

    /// Edit the session: a refusal is kept as the message, success clears
    /// it. Returns whether it succeeded.
    pub fn edit(&mut self, edit: impl FnOnce(&mut Session) -> Result<(), String>) -> bool {
        let result = self.change(edit);
        self.message = result.err();
        self.message.is_none()
    }

    fn current(&self) -> Vec<Card> {
        let session = &self.session;
        session
            .nodes()
            .iter()
            .zip(session.results())
            .map(|(node, result)| Card {
                id: node.id,
                title: node.name.as_str().into(),
                inputs: ports(&node.inputs),
                outputs: ports(&node.outputs),
                position: session
                    .graph()
                    .position(node.id)
                    .map(point)
                    .or_else(|| self.arranged.get(&node.id).copied())
                    .unwrap_or(Point::ZERO),
                problem: result.problem.as_deref().map(SharedString::from),
            })
            .collect()
    }

    /// Bring what the canvas shows up to date and record what changed.
    fn sync(&mut self) {
        let cards = self.current();
        let links: Vec<Link> = self
            .session
            .links()
            .iter()
            .map(|l| Link {
                from: PortRef::output(id(l.from.0), l.from.1),
                to: PortRef::input(id(l.to.0), l.to.1),
            })
            .collect();
        let same_shape = links == self.links
            && cards.len() == self.cards.len()
            && cards
                .iter()
                .zip(&self.cards)
                .all(|(a, b)| a.id == b.id && a.inputs == b.inputs && a.outputs == b.outputs);
        if same_shape {
            for (i, (new, old)) in cards.iter().zip(&self.cards).enumerate() {
                if new.title != old.title || new.problem != old.problem {
                    self.log.record(GraphChange::Edited(id(i)));
                }
                if new.position != old.position {
                    self.log.record(GraphChange::Moved(id(i)));
                }
            }
        } else {
            self.log.restructure();
        }
        self.cards = cards;
        self.links = links;
    }

    fn card(&self, node: NodeId) -> Option<&Card> {
        self.cards.get(node_index(node)?)
    }

    fn port(&self, port: PortRef) -> Option<&Port> {
        let node = self.session.nodes().get(node_index(port.node)?)?;
        match port.side {
            PortSide::Input => &node.inputs,
            PortSide::Output => &node.outputs,
        }
        .get(port.index)
    }

    fn ends(&self, link: Link) -> Option<(Port, Port)> {
        Some((self.port(link.from)?.clone(), self.port(link.to)?.clone()))
    }
}

impl NodeGraphModel for Editor {
    fn revision(&self) -> u64 {
        self.log.revision()
    }
    fn changes_since(&self, since: u64) -> Option<Vec<GraphChange>> {
        self.log.since(since)
    }
    fn node_ids(&self) -> Vec<NodeId> {
        (0..self.cards.len()).map(id).collect()
    }
    fn title(&self, node: NodeId) -> SharedString {
        self.card(node)
            .map_or_else(|| "?".into(), |c| c.title.clone())
    }
    fn position(&self, node: NodeId) -> Point {
        self.card(node).map_or(Point::ZERO, |c| c.position)
    }
    fn inputs(&self, node: NodeId) -> Vec<PortInfo> {
        self.card(node)
            .map(|c| c.inputs.clone())
            .unwrap_or_default()
    }
    fn outputs(&self, node: NodeId) -> Vec<PortInfo> {
        self.card(node)
            .map(|c| c.outputs.clone())
            .unwrap_or_default()
    }
    fn links(&self) -> Vec<Link> {
        self.links.clone()
    }
    fn problem(&self, node: NodeId) -> Option<SharedString> {
        self.card(node)?.problem.clone()
    }
    fn can_connect(&self, from: PortRef, to: PortRef) -> bool {
        self.ends(Link { from, to })
            .is_some_and(|(from, to)| self.session.can_connect(&from, &to))
    }
    fn set_position(&mut self, node: NodeId, position: Point) {
        let Some(i) = node_index(node) else {
            return;
        };
        let Some(card) = self.cards.get_mut(i) else {
            return;
        };
        // Cards are drawn on whole units; a room should not carry the
        // pointer's fractions.
        let position = Point::new(position.x.round(), position.y.round());
        if card.position == position {
            return;
        }
        // A move changes nothing but the card: no full sync per frame.
        let to = [f64::from(position.x), f64::from(position.y)];
        match self.session.move_node(card.id, to) {
            Ok(()) => {
                card.position = position;
                self.log.record(GraphChange::Moved(node));
            }
            Err(e) => self.message = Some(e),
        }
    }
    fn connect(&mut self, link: Link) {
        if let Some((from, to)) = self.ends(link) {
            self.edit(|s| s.connect(&from, &to));
        }
    }
    fn disconnect(&mut self, link: Link) {
        if let Some((_, to)) = self.ends(link) {
            self.edit(|s| s.disconnect(&to));
        }
    }
    fn end_gesture(&mut self) {
        self.session.end_gesture();
    }
    fn set_param(&mut self, _node: NodeId, _index: usize, _value: ParamValue) {}
}
