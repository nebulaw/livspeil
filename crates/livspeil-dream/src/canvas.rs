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
    /// Its id on the canvas.
    key: NodeId,
    id: GraphId,
    title: SharedString,
    inputs: Vec<PortInfo>,
    outputs: Vec<PortInfo>,
    position: Point,
    problem: Option<SharedString>,
}

/// An open room being edited.
///
/// A node keeps its canvas id for as long as the editor lives, also through
/// deletion and undo, so a selection never comes to mean another node.
#[derive(Debug)]
pub struct Editor {
    session: Session,
    metrics: CardMetrics,
    /// The canvas id of every node seen.
    keys: HashMap<GraphId, NodeId>,
    /// The last canvas id given out.
    last_key: NodeId,
    /// Where in [`Session::nodes`] each canvas id shown is.
    at: HashMap<NodeId, usize>,
    /// Where the nodes the room does not place were put when it was opened.
    /// They stay there.
    arranged: HashMap<GraphId, Point>,
    cards: Vec<Card>,
    links: Vec<Link>,
    log: ChangeLog,
    /// Why the last edit or save was refused.
    message: Option<String>,
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

/// Places for the nodes the room does not place.
fn arranged(session: &Session, metrics: CardMetrics) -> HashMap<GraphId, Point> {
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
    arrange(nodes, session.links(), &placed, &heights)
        .into_iter()
        .zip(nodes)
        .zip(&placed)
        .filter(|(_, placed)| placed.is_none())
        .map(|(([x, y], node), _)| (node.id, Point::new(x, y)))
        .collect()
}

impl Editor {
    /// An editor of `session`, with cards sized by `metrics`.
    #[must_use]
    pub fn new(session: Session, metrics: CardMetrics) -> Self {
        let arranged = arranged(&session, metrics);
        let mut editor = Editor {
            session,
            metrics,
            keys: HashMap::new(),
            last_key: 0,
            at: HashMap::new(),
            arranged,
            cards: Vec::new(),
            links: Vec::new(),
            log: ChangeLog::new(),
            message: None,
        };
        editor.sync();
        editor
    }

    /// Edit `session` instead.
    pub fn open(&mut self, session: Session) {
        self.arranged = arranged(&session, self.metrics);
        self.session = session;
        // Canvas ids are never given twice, even to another room's nodes.
        self.keys.clear();
        self.message = None;
        self.sync();
        // Whatever it looks like, it is another graph.
        self.log.restructure();
    }

    /// The canvas id of the node at `index` of [`Session::nodes`].
    #[must_use]
    pub fn canvas_id(&self, index: usize) -> Option<NodeId> {
        self.cards.get(index).map(|c| c.key)
    }

    /// Where in [`Session::nodes`] canvas node `id` is.
    #[must_use]
    pub fn index_of(&self, id: NodeId) -> Option<usize> {
        self.at.get(&id).copied()
    }

    /// Say why something the person asked for did not happen.
    pub fn report(&mut self, message: Option<String>) {
        self.message = message;
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

    fn current(&mut self) -> Vec<Card> {
        for node in self.session.nodes() {
            let last = &mut self.last_key;
            self.keys.entry(node.id).or_insert_with(|| {
                *last += 1;
                *last
            });
        }
        let session = &self.session;
        session
            .nodes()
            .iter()
            .zip(session.results())
            .map(|(node, result)| Card {
                // Given just above.
                key: self.keys.get(&node.id).copied().unwrap_or_default(),
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
        let key = |index: usize| cards.get(index).map_or(0, |c| c.key);
        let links: Vec<Link> = self
            .session
            .links()
            .iter()
            .map(|l| Link {
                from: PortRef::output(key(l.from.0), l.from.1),
                to: PortRef::input(key(l.to.0), l.to.1),
            })
            .collect();
        let same_shape = links == self.links
            && cards.len() == self.cards.len()
            && cards
                .iter()
                .zip(&self.cards)
                .all(|(a, b)| a.id == b.id && a.inputs == b.inputs && a.outputs == b.outputs);
        if same_shape {
            for (new, old) in cards.iter().zip(&self.cards) {
                if new.title != old.title || new.problem != old.problem {
                    self.log.record(GraphChange::Edited(new.key));
                }
                if new.position != old.position {
                    self.log.record(GraphChange::Moved(new.key));
                }
            }
        } else {
            self.log.restructure();
        }
        self.at = cards.iter().enumerate().map(|(i, c)| (c.key, i)).collect();
        self.cards = cards;
        self.links = links;
    }

    fn card(&self, node: NodeId) -> Option<&Card> {
        self.cards.get(self.index_of(node)?)
    }

    fn port(&self, port: PortRef) -> Option<&Port> {
        let node = self.session.nodes().get(self.index_of(port.node)?)?;
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
        self.cards.iter().map(|c| c.key).collect()
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
        let Some(i) = self.index_of(node) else {
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
    fn remove(&mut self, nodes: &[NodeId]) {
        let ids: Vec<GraphId> = nodes
            .iter()
            .filter_map(|&n| self.card(n).map(|c| c.id))
            .collect();
        self.edit(|s| s.delete(&ids));
    }
}
