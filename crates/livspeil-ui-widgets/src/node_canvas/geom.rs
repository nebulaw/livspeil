// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Node card geometry (canvas space), the per-revision cache with its
//! spatial index, wire snapping and rubber-band selection.

use std::collections::HashMap;

use livspeil_ui_core::{SharedString, Wire};
use livspeil_ui_layout::{CanvasIndex, Point, Rect, Size};
use livspeil_ui_theme::{Color, Theme};

use super::model::{
    GraphChange, Link, NodeGraphModel, NodeId, ParamInfo, PortInfo, PortRef, PortSide,
};
use crate::style::metrics;

/// Card width in canvas units.
pub const NODE_WIDTH: f32 = 200.0;
/// Port dot diameter.
pub const PORT_DOT: f32 = 10.0;
/// Wire width in canvas units.
pub const WIRE_WIDTH: f32 = 2.0;
/// Snap radius of a pending wire, in view pixels.
pub const SNAP_RADIUS: f32 = 24.0;

/// Card row sizes (follow the theme density).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardMetrics {
    /// Header height.
    pub header: f32,
    /// Port and parameter row height.
    pub row: f32,
    /// Padding below the last row.
    pub pad: f32,
}

impl CardMetrics {
    /// From the theme (rows as tall as a control).
    #[must_use]
    pub fn from_theme(theme: &Theme) -> Self {
        let m = metrics(theme);
        CardMetrics {
            header: m.height + 4.0,
            row: m.height,
            pad: 6.0,
        }
    }

    /// Size of a card.
    #[must_use]
    pub fn card_size(&self, inputs: usize, outputs: usize, params: usize) -> Size {
        #[allow(clippy::cast_precision_loss)]
        let rows = (inputs.max(outputs) + params) as f32;
        Size::new(NODE_WIDTH, self.header + rows * self.row + self.pad)
    }

    /// Center of a port, for a card at `origin`.
    #[must_use]
    pub fn port_position(&self, origin: Point, side: PortSide, index: usize) -> Point {
        #[allow(clippy::cast_precision_loss)]
        let y = origin.y + self.header + (index as f32 + 0.5) * self.row;
        match side {
            PortSide::Input => Point::new(origin.x, y),
            PortSide::Output => Point::new(origin.x + NODE_WIDTH, y),
        }
    }
}

/// One node's cached data.
#[derive(Clone, Debug)]
pub struct NodeGeom {
    /// Id.
    pub id: NodeId,
    /// Title.
    pub title: SharedString,
    /// Card rect in canvas space.
    pub rect: Rect,
    /// Inputs.
    pub inputs: Vec<PortInfo>,
    /// Outputs.
    pub outputs: Vec<PortInfo>,
    /// Parameters.
    pub params: Vec<ParamInfo>,
    /// What is wrong with it, if something is.
    pub problem: Option<SharedString>,
}

impl NodeGeom {
    /// The ports of a side.
    #[must_use]
    pub fn ports(&self, side: PortSide) -> &[PortInfo] {
        match side {
            PortSide::Input => &self.inputs,
            PortSide::Output => &self.outputs,
        }
    }
}

/// What [`GraphCache::apply`] changed. The rects are those from before.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// Nodes whose card rect changed, by index into [`GraphCache::nodes`].
    pub moved: Vec<(usize, Rect)>,
    /// Nodes whose content was read again, by index.
    pub edited: Vec<usize>,
    /// Links of those nodes, by index into [`GraphCache::links`].
    pub links: Vec<(usize, Rect)>,
}

/// Everything the canvas needs from the model at one revision.
#[derive(Clone, Debug)]
pub struct GraphCache {
    /// The model revision.
    pub revision: u64,
    /// Card metrics used.
    pub metrics: CardMetrics,
    /// Nodes in paint order.
    pub nodes: Vec<NodeGeom>,
    /// Node index by id.
    pub by_id: HashMap<NodeId, usize>,
    /// Spatial index over node rects (indices into `nodes`).
    pub index: CanvasIndex,
    /// Links with their wire bounds.
    pub links: Vec<(Link, Rect)>,
    /// Port type colors.
    pub colors: HashMap<SharedString, Color>,
    /// The links of each node, by node index.
    attached: Vec<Vec<usize>>,
}

impl GraphCache {
    /// Read `model`.
    pub fn build<M: NodeGraphModel + ?Sized>(model: &M, metrics: CardMetrics) -> Self {
        let ids = model.node_ids();
        let mut nodes = Vec::with_capacity(ids.len());
        let mut by_id = HashMap::with_capacity(ids.len());
        let mut index = CanvasIndex::new(512.0);
        let mut colors = HashMap::new();
        for id in ids {
            let inputs = model.inputs(id);
            let outputs = model.outputs(id);
            let params = model.params(id);
            for p in inputs.iter().chain(&outputs) {
                if !colors.contains_key(&p.ty) {
                    colors.insert(p.ty.clone(), model.type_color(&p.ty));
                }
            }
            let size = metrics.card_size(inputs.len(), outputs.len(), params.len());
            let pos = model.position(id);
            let rect = Rect::new(pos.x, pos.y, size.width, size.height);
            index.insert(rect);
            by_id.insert(id, nodes.len());
            nodes.push(NodeGeom {
                id,
                title: model.title(id),
                rect,
                inputs,
                outputs,
                params,
                problem: model.problem(id),
            });
        }
        let mut cache = GraphCache {
            revision: model.revision(),
            metrics,
            nodes,
            by_id,
            index,
            links: Vec::new(),
            colors,
            attached: Vec::new(),
        };
        cache.links = model
            .links()
            .into_iter()
            .filter_map(|l| Some((l, cache.wire(l, Color::WHITE)?.bounds())))
            .collect();
        cache.attached = vec![Vec::new(); cache.nodes.len()];
        for (i, (l, _)) in cache.links.iter().enumerate() {
            for node in [l.from.node, l.to.node] {
                if let Some(&n) = cache.by_id.get(&node) {
                    cache.attached[n].push(i);
                }
            }
        }
        cache
    }

    /// Bring the cache to the model's revision from `changes`, the model's
    /// report since [`GraphCache::revision`]. `None` if they cannot be
    /// applied one by one (an unknown node, or ports added or removed); the
    /// cache is then partly updated and must be built again.
    pub fn apply<M: NodeGraphModel + ?Sized>(
        &mut self,
        model: &M,
        changes: &[GraphChange],
    ) -> Option<Applied> {
        let mut applied = Applied::default();
        let mut touched = Vec::new();
        for change in changes {
            let (id, edited) = match *change {
                GraphChange::Moved(id) => (id, false),
                GraphChange::Edited(id) => (id, true),
            };
            let i = *self.by_id.get(&id)?;
            let metrics = self.metrics;
            let node = &mut self.nodes[i];
            let old = node.rect;
            let pos = model.position(id);
            (node.rect.x, node.rect.y) = (pos.x, pos.y);
            if edited {
                let (inputs, outputs) = (model.inputs(id), model.outputs(id));
                if inputs.len() != node.inputs.len() || outputs.len() != node.outputs.len() {
                    return None;
                }
                for p in inputs.iter().chain(&outputs) {
                    if !self.colors.contains_key(&p.ty) {
                        self.colors.insert(p.ty.clone(), model.type_color(&p.ty));
                    }
                }
                node.title = model.title(id);
                node.params = model.params(id);
                node.problem = model.problem(id);
                let size = metrics.card_size(inputs.len(), outputs.len(), node.params.len());
                (node.rect.width, node.rect.height) = (size.width, size.height);
                (node.inputs, node.outputs) = (inputs, outputs);
                applied.edited.push(i);
            }
            let rect = node.rect;
            if rect != old {
                self.index.update(u32::try_from(i).ok()?, rect);
                applied.moved.push((i, old));
            }
            if rect != old || edited {
                touched.extend_from_slice(&self.attached[i]);
            }
        }
        touched.sort_unstable();
        touched.dedup();
        for i in touched {
            let (link, old) = self.links[i];
            self.links[i].1 = self.wire(link, Color::WHITE)?.bounds();
            applied.links.push((i, old));
        }
        self.revision = model.revision();
        Some(applied)
    }

    /// A node by id.
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&NodeGeom> {
        self.nodes.get(*self.by_id.get(&id)?)
    }

    /// A port's center in canvas space.
    #[must_use]
    pub fn port_position(&self, p: PortRef) -> Option<Point> {
        let n = self.node(p.node)?;
        (p.index < n.ports(p.side).len())
            .then(|| self.metrics.port_position(n.rect.origin(), p.side, p.index))
    }

    /// A port's type.
    #[must_use]
    pub fn port_type(&self, p: PortRef) -> Option<&SharedString> {
        self.node(p.node)?.ports(p.side).get(p.index).map(|i| &i.ty)
    }

    /// The color of a port.
    #[must_use]
    pub fn port_color(&self, p: PortRef) -> Color {
        self.port_type(p)
            .and_then(|t| self.colors.get(t))
            .copied()
            .unwrap_or(Color::WHITE)
    }

    /// The wire of a link.
    #[must_use]
    pub fn wire(&self, l: Link, color: Color) -> Option<Wire> {
        Some(Wire::link(
            self.port_position(l.from)?,
            self.port_position(l.to)?,
            WIRE_WIDTH,
            color,
        ))
    }

    /// Indices of nodes intersecting `area`, in paint order.
    #[must_use]
    pub fn nodes_in(&self, area: &Rect) -> Vec<usize> {
        self.index
            .query(area)
            .into_iter()
            .map(|i| i as usize)
            .collect()
    }

    /// The topmost node whose card contains `p`.
    #[must_use]
    pub fn node_at(&self, p: Point) -> Option<NodeId> {
        self.nodes_in(&Rect::new(p.x, p.y, 0.0, 0.0))
            .into_iter()
            .rev()
            .map(|i| &self.nodes[i])
            .find(|n| n.rect.contains(p))
            .map(|n| n.id)
    }
}

/// The port a pending wire from `from` snaps to near `pointer` (canvas
/// space): the closest port on the opposite side within `radius` (canvas
/// units) that `ok(output, input)` accepts.
#[must_use]
pub fn find_snap(
    cache: &GraphCache,
    from: PortRef,
    pointer: Point,
    radius: f32,
    ok: impl Fn(PortRef, PortRef) -> bool,
) -> Option<PortRef> {
    let want = match from.side {
        PortSide::Output => PortSide::Input,
        PortSide::Input => PortSide::Output,
    };
    let area = Rect::new(pointer.x, pointer.y, 0.0, 0.0).inflate(radius + NODE_WIDTH);
    let mut best: Option<(f32, PortRef)> = None;
    for i in cache.nodes_in(&area) {
        let n = &cache.nodes[i];
        if n.id == from.node {
            continue;
        }
        for index in 0..n.ports(want).len() {
            let p = PortRef {
                node: n.id,
                side: want,
                index,
            };
            let Some(at) = cache.port_position(p) else {
                continue;
            };
            let d = (at - pointer).length();
            if d > radius || best.is_some_and(|(b, _)| b <= d) {
                continue;
            }
            let (o, i) = oriented(from, p);
            if ok(o, i) {
                best = Some((d, p));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// `(output, input)` of two ports on opposite sides.
#[must_use]
pub fn oriented(a: PortRef, b: PortRef) -> (PortRef, PortRef) {
    if a.side == PortSide::Output {
        (a, b)
    } else {
        (b, a)
    }
}

/// Nodes selected by a rubber band from `a` to `b` (canvas space): those
/// whose cards intersect it.
#[must_use]
pub fn rubber_select(cache: &GraphCache, a: Point, b: Point) -> Vec<NodeId> {
    let r = rect_from(a, b);
    cache
        .nodes_in(&r)
        .into_iter()
        .map(|i| cache.nodes[i].id)
        .collect()
}

/// The rect spanned by two corners.
#[must_use]
pub fn rect_from(a: Point, b: Point) -> Rect {
    Rect::new(
        a.x.min(b.x),
        a.y.min(b.y),
        (a.x - b.x).abs(),
        (a.y - b.y).abs(),
    )
}

/// Bounds of all nodes (zero rect if none).
#[must_use]
pub fn graph_bounds(cache: &GraphCache) -> Rect {
    let mut it = cache.nodes.iter().map(|n| n.rect);
    let Some(first) = it.next() else {
        return Rect::ZERO;
    };
    it.fold(first, |a, r| {
        let x0 = a.x.min(r.x);
        let y0 = a.y.min(r.y);
        Rect::new(
            x0,
            y0,
            a.right().max(r.right()) - x0,
            a.bottom().max(r.bottom()) - y0,
        )
    })
}
