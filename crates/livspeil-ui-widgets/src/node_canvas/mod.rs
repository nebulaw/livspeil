// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`NodeCanvas`]: the node editor prototype of Dream Mode (UI spec §9).
//!
//! An infinite canvas (`livspeil_ui_layout::Canvas`) showing a
//! [`NodeGraphModel`]: node cards with typed ports and parameter rows,
//! links as Bézier wires, a minimap. See [`NodeCanvas`] for interactions.

mod card;
mod geom;
mod minimap;
mod model;
mod wires;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use livspeil_ui_core::{
    Animated, App, CursorShape, DragEvent, DragPhase, Element, EventCx, Key, Model, MouseButton,
    NamedKey, Phase, PointerEvent, PointerFilter, PointerKind, Render, Role as CoreRole,
    ScrollDelta, SharedString, View, ViewContext, Wire, actions as core, div,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto, Position};
use livspeil_ui_layout::{Canvas, Point, Rect, Size, Vec2};
use livspeil_ui_theme::{Color, Material, MotionClass, PropertyKind};

pub use geom::{
    Applied, CardMetrics, GraphCache, NODE_WIDTH, NodeGeom, PORT_DOT, SNAP_RADIUS, WIRE_WIDTH,
    find_snap, graph_bounds, oriented, rect_from, rubber_select,
};
pub use minimap::{MINIMAP, minimap_mapping};
pub use model::{
    ChangeLog, DemoGraph, DemoNode, GraphChange, Link, NodeGraphModel, NodeId, ParamInfo,
    ParamValue, PortInfo, PortRef, PortSide, palette_color,
};

use crate::env::Env;
use crate::semantics::{Role, Semantics};
use card::{CardState, NodeCard, Pending};
use minimap::{MinimapNodes, Shown};
use wires::WireLayer;

/// Motion scope of canvas zoom (`[overrides.canvas]`).
pub const CANVAS_SCOPE: &str = "canvas";
/// Zoom from which cards show labels and parameters.
pub const LOD_FULL: f32 = 0.6;
/// Zoom from which cards show their header and ports (below: blocks).
pub const LOD_COMPACT: f32 = 0.3;
/// The built region reaches past the view on each side by this fraction of
/// the view, and at least by [`MIN_MARGIN`] view pixels.
const MARGIN: f32 = 0.25;
const MIN_MARGIN: f32 = 96.0;
/// The built region is rebuilt smaller once it is this many times wider or
/// taller than a new one would be.
const SHRINK: f32 = 2.0;
/// Key of the zoom spring's frame task.
const ZOOM_TASK: &str = "zoom";
/// Key of the frame task that applies model changes.
const SYNC_TASK: &str = "sync";
/// Zoom factor per wheel line.
const WHEEL_ZOOM: f32 = 1.15;
/// Zoom exponent per pixel of a ctrl/pinch scroll.
const PIXEL_ZOOM: f32 = 0.01;

/// Card detail level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Lod {
    /// Labels, ports and parameters.
    #[default]
    Full,
    /// Header and port dots.
    Compact,
    /// A plain block per node.
    Block,
}

impl Lod {
    /// The level for a zoom.
    #[must_use]
    pub fn for_zoom(zoom: f32) -> Lod {
        if zoom >= LOD_FULL {
            Lod::Full
        } else if zoom >= LOD_COMPACT {
            Lod::Compact
        } else {
            Lod::Block
        }
    }
}

/// What the last render built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CanvasStats {
    /// Nodes in the model.
    pub nodes: usize,
    /// Links in the model.
    pub links: usize,
    /// Nodes built: those intersecting the built region (the view and a
    /// margin around it) when it was last built or a node entered or left it.
    pub built_nodes: usize,
    /// Wires built.
    pub built_wires: usize,
    /// Detail level.
    pub lod: Lod,
}

#[derive(Clone, Debug, Default)]
enum Gesture {
    #[default]
    None,
    Pan,
    Move(Vec<(NodeId, Point)>),
    Rubber {
        start: Point,
        now: Point,
        base: BTreeSet<NodeId>,
    },
    Wire {
        from: PortRef,
        pointer: Point,
        snap: Option<PortRef>,
    },
}

#[derive(Clone, Copy, Debug)]
struct ZoomAnim {
    /// The view point (relative to the canvas) that stays fixed.
    anchor: Point,
    /// The canvas point under it.
    anchor_canvas: Point,
    zoom: Animated<f32>,
}

/// What a render built: the part of the canvas, and in how much detail. It
/// stays good while the view stays inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Built {
    /// Canvas space.
    area: Rect,
    lod: Lod,
    /// The view size it was built for.
    size: Size,
}

impl Built {
    /// What to build for `canvas` seen in a view of `size`.
    fn around(canvas: &Canvas, size: Size) -> Built {
        let view = canvas.visible_rect(size);
        let margin = |len: f32| (len * MARGIN).max(MIN_MARGIN) / canvas.zoom;
        let (x, y) = (margin(size.width), margin(size.height));
        Built {
            area: Rect::new(
                view.x - x,
                view.y - y,
                view.width + 2.0 * x,
                view.height + 2.0 * y,
            ),
            lod: Lod::for_zoom(canvas.zoom),
            size,
        }
    }

    /// Whether it still serves: same detail and view size, the whole view
    /// inside, and not far larger than what the view needs now (zoomed in a lot).
    fn covers(&self, canvas: &Canvas, size: Size) -> bool {
        let view = canvas.visible_rect(size);
        let wanted = Built::around(canvas, size);
        let a = self.area;
        self.lod == wanted.lod
            && self.size == size
            && a.x <= view.x
            && a.y <= view.y
            && a.right() >= view.right()
            && a.bottom() >= view.bottom()
            && a.width <= SHRINK * wanted.area.width
            && a.height <= SHRINK * wanted.area.height
    }
}

/// A node editor canvas over a [`NodeGraphModel`] (UI spec §9 NodeCanvas).
///
/// - **Pan**: Space+drag, middle drag, two-finger trackpad scroll.
/// - **Zoom** about the pointer: Ctrl/Cmd+scroll (hosts deliver pinch as
///   Ctrl+pixel scroll) and the mouse wheel; animated with the Standard
///   spring (instant under Reduce Motion); the canvas point under the
///   pointer stays fixed throughout.
/// - **Select**: click a node (only it), Shift+click toggles, click empty
///   space clears, drag on empty space draws a rubber band (Shift adds),
///   Ctrl/Cmd+A selects all, Escape clears.
/// - **Move**: drag a node; every selected node moves with it.
/// - **Disconnect**: drag from a linked input: its link comes off and
///   follows the pointer; drop it elsewhere, or nowhere to remove it.
/// - **Connect**: drag from a port: a pending wire follows the pointer and
///   snaps to the nearest compatible port within [`SNAP_RADIUS`] px
///   ([`NodeGraphModel::can_connect`]); incompatible ports dim; releasing
///   on a snapped port connects.
/// - **Parameters**: numbers are `NumberField`s (scrub or type), flags
///   `Toggle`s; edits go to [`NodeGraphModel::set_param`].
/// - **Virtualization**: only nodes and wires intersecting the built
///   region (the viewport and a margin) are built, through a spatial index;
///   cards lose detail when zoomed out ([`Lod`]).
/// - **Layers**: pan and zoom set the transform of one layer holding the
///   wires and cards, and move the minimap's viewport frame. Each card is a
///   view on a layer of its own, the wires and the minimap content are views
///   too.
/// - **Changes**: the canvas keeps a copy of the graph and applies what the
///   model reports ([`NodeGraphModel::changes_since`]) at the start of the
///   next frame. A moved node sets its card's layer and redraws its wires;
///   no card and not the canvas renders. The canvas renders again only when
///   the view leaves the built region, a node enters or leaves it, the level
///   of detail or the selection changes, or the graph's structure does.
pub struct NodeCanvas<M: NodeGraphModel> {
    env: Env,
    model: Model<M>,
    canvas: Canvas,
    zoom: Option<ZoomAnim>,
    zoom_target: f32,
    selection: BTreeSet<NodeId>,
    gesture: Gesture,
    space: bool,
    button: Option<MouseButton>,
    shift: bool,
    press: Option<(NodeId, bool)>,
    /// Layout box of the canvas as of the last render.
    origin: Point,
    size: Size,
    built: Option<Built>,
    cache: Option<Rc<GraphCache>>,
    /// Whether the model is being watched.
    watching: bool,
    /// The built cards.
    cards: HashMap<NodeId, View<NodeCard<M>>>,
    wires: Option<View<WireLayer>>,
    /// The minimap's content and what it shows of the canvas.
    minimap: Option<View<MinimapNodes>>,
    shown: Shown,
    /// The minimap's rects must all be placed again.
    minimap_stale: bool,
    show_minimap: bool,
    stats: CanvasStats,
    name: SharedString,
}

impl<M: NodeGraphModel> fmt::Debug for NodeCanvas<M> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeCanvas")
            .field("canvas", &self.canvas)
            .field("selection", &self.selection)
            .field("gesture", &self.gesture)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

impl<M: NodeGraphModel> NodeCanvas<M> {
    /// A canvas over `model`, filling its parent.
    #[must_use]
    pub fn new(env: &Env, model: Model<M>) -> Self {
        NodeCanvas {
            env: env.clone(),
            model,
            canvas: Canvas::default(),
            zoom: None,
            zoom_target: 1.0,
            selection: BTreeSet::new(),
            gesture: Gesture::None,
            space: false,
            button: None,
            shift: false,
            press: None,
            origin: Point::ZERO,
            size: Size::new(0.0, 0.0),
            built: None,
            cache: None,
            watching: false,
            cards: HashMap::new(),
            wires: None,
            minimap: None,
            shown: Shown::default(),
            minimap_stale: true,
            show_minimap: true,
            stats: CanvasStats::default(),
            name: "node-canvas".into(),
        }
    }

    /// Element name (default `node-canvas`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = n.into();
        self
    }

    /// Show the minimap (default true).
    #[must_use]
    pub fn minimap(mut self, on: bool) -> Self {
        self.show_minimap = on;
        self
    }

    /// Start at this pan/zoom.
    #[must_use]
    pub fn with_canvas(mut self, canvas: Canvas) -> Self {
        self.jump(canvas);
        self
    }

    fn jump(&mut self, canvas: Canvas) {
        self.canvas = canvas;
        self.zoom_target = canvas.zoom;
        self.zoom = None;
    }

    /// The graph this canvas shows.
    #[must_use]
    pub fn model(&self) -> Model<M> {
        self.model
    }

    /// The current pan/zoom.
    #[must_use]
    pub fn canvas(&self) -> Canvas {
        self.canvas
    }

    /// The zoom being animated to.
    #[must_use]
    pub fn zoom_target(&self) -> f32 {
        self.zoom_target
    }

    /// Whether a zoom animation is running.
    #[must_use]
    pub fn is_zooming(&self) -> bool {
        self.zoom.is_some()
    }

    /// Selected nodes.
    #[must_use]
    pub fn selection(&self) -> &BTreeSet<NodeId> {
        &self.selection
    }

    /// Replace the selection.
    pub fn set_selection(&mut self, ids: impl IntoIterator<Item = NodeId>) {
        self.selection = ids.into_iter().collect();
    }

    /// The pending wire: its start and the port it snaps to.
    #[must_use]
    pub fn pending_wire(&self) -> Option<(PortRef, Option<PortRef>)> {
        match self.gesture {
            Gesture::Wire { from, snap, .. } => Some((from, snap)),
            _ => None,
        }
    }

    /// The rubber band (canvas space) while selecting.
    #[must_use]
    pub fn rubber_band(&self) -> Option<Rect> {
        match &self.gesture {
            Gesture::Rubber { start, now, .. } => Some(rect_from(*start, *now)),
            _ => None,
        }
    }

    /// What the last render built.
    #[must_use]
    pub fn stats(&self) -> CanvasStats {
        self.stats
    }

    /// The canvas's copy of the graph, as of the last frame or interaction.
    #[must_use]
    pub fn cache(&self) -> Option<&GraphCache> {
        self.cache.as_deref()
    }

    /// The canvas element's layout origin and size as of the last render
    /// (zero before it was laid out).
    #[must_use]
    pub fn frame(&self) -> (Point, Size) {
        (self.origin, self.size)
    }

    /// Jump to a pan/zoom (cancels a zoom animation). Like panning and
    /// zooming, this renders the canvas only if the built region no longer
    /// serves.
    pub fn set_canvas(view: View<Self>, app: &mut App, canvas: Canvas) {
        view.update(app, |s, cx| {
            s.jump(canvas);
            s.camera_changed(cx);
        });
    }

    /// Zoom by `factor` about `anchor` (window coordinates), animated.
    pub fn zoom_about(view: View<Self>, app: &mut App, anchor: Point, factor: f32) {
        let spec = app.motion().resolve(
            Some(CANVAS_SCOPE),
            MotionClass::Standard,
            PropertyKind::Movement,
        );
        let local = view_point(view, app, anchor);
        let animate = view.update(app, |s, cx| {
            let target = (s.zoom_target * factor).clamp(s.canvas.min_zoom, s.canvas.max_zoom);
            s.zoom_target = target;
            let mut za = s.zoom.unwrap_or(ZoomAnim {
                anchor: local,
                anchor_canvas: Point::ZERO,
                zoom: Animated::new(s.canvas.zoom),
            });
            za.anchor = local;
            za.anchor_canvas = s.canvas.to_canvas(local);
            if za.zoom.animate_to(target, spec) {
                s.zoom = Some(za);
                true
            } else {
                s.zoom = None;
                s.canvas.zoom = target;
                s.canvas.pan = local - za.anchor_canvas * target;
                s.camera_changed(cx);
                false
            }
        });
        if animate {
            app.animate(view.id(), ZOOM_TASK, move |app, dt| {
                view.update(app, |s, cx| s.step_zoom(dt, cx))
            });
        }
    }

    /// Advance the zoom spring (the anchor stays put). Returns whether it
    /// is still moving.
    fn step_zoom(&mut self, dt: f32, cx: &mut ViewContext<'_, Self>) -> bool {
        let Some(mut za) = self.zoom else {
            return false;
        };
        let moving = za.zoom.step(dt);
        self.canvas.zoom = za.zoom.value();
        self.canvas.pan = za.anchor - za.anchor_canvas * self.canvas.zoom;
        self.zoom = moving.then_some(za);
        self.camera_changed(cx);
        moving
    }

    /// Pan by a view-space delta.
    pub fn pan_by(view: View<Self>, app: &mut App, delta: Vec2) {
        view.update(app, |s, cx| {
            s.pan(delta);
            s.camera_changed(cx);
        });
    }

    fn pan(&mut self, delta: Vec2) {
        self.canvas.pan_by(delta);
        if let Some(za) = &mut self.zoom {
            za.anchor_canvas = self.canvas.to_canvas(za.anchor);
        }
    }

    /// Zoom and pan so the whole graph fits with a margin.
    pub fn fit(view: View<Self>, app: &mut App) {
        let metrics = CardMetrics::from_theme(app.theme());
        let model = view.read(app).model;
        let bounds = graph_bounds(&GraphCache::build(model.read(app), metrics));
        let vp = app.viewport();
        let s = view.read(app);
        let size = if s.size.width > 0.0 { s.size } else { vp };
        let m = 40.0;
        let z = ((size.width - 2.0 * m) / bounds.width.max(1.0))
            .min((size.height - 2.0 * m) / bounds.height.max(1.0))
            .clamp(s.canvas.min_zoom, s.canvas.max_zoom.min(1.0));
        let canvas = Canvas {
            zoom: z,
            pan: Vec2::new(size.width / 2.0, size.height / 2.0) - bounds.center() * z,
            ..s.canvas
        };
        Self::set_canvas(view, app, canvas);
    }

    /// The view size: the canvas's own once laid out, else the window.
    fn view_size(&self, app: &App) -> Size {
        if self.size.width > 0.0 {
            self.size
        } else {
            app.viewport()
        }
    }

    /// The canvas moved: set the layers, and render only if what is built
    /// no longer serves.
    fn camera_changed(&mut self, cx: &mut ViewContext<'_, Self>) {
        self.place(cx);
        let size = self.view_size(cx);
        // The rubber band is drawn in view coordinates.
        if matches!(self.gesture, Gesture::Rubber { .. })
            || !self.built.is_some_and(|b| b.covers(&self.canvas, size))
        {
            cx.notify();
        }
    }

    /// Put the world and the minimap's viewport frame where the canvas is.
    fn place(&mut self, cx: &mut ViewContext<'_, Self>) {
        let world = cx.layer("world");
        world.set_transform(cx, self.canvas.transform());
        let frame = self.cache.as_ref().map(|_| {
            let (k, off) = self.shown.mapping();
            let seen = self.canvas.visible_rect(self.view_size(cx));
            Rect::new(
                seen.x * k + off.x,
                seen.y * k + off.y,
                seen.width * k,
                seen.height * k,
            )
        });
        let viewport = cx.layer("minimap-viewport");
        viewport.set_frame(cx, frame);
    }

    fn model_ok(&self, app: &App, from: PortRef, to: PortRef) -> bool {
        self.model.read(app).can_connect(from, to)
    }

    /// Bring the copy of the graph, and what shows it, up to the model.
    /// Returns whether the canvas must render.
    fn sync(&mut self, cx: &mut ViewContext<'_, Self>) -> bool {
        let metrics = CardMetrics::from_theme(cx.theme());
        let model = self.model;
        if let Some(cache) = &mut self.cache
            && cache.metrics == metrics
        {
            if cx.untracked(|app| model.read(app).revision()) == cache.revision {
                return false;
            }
            let applied = cx.untracked(|app| {
                let graph = model.read(app);
                let changes = graph.changes_since(cache.revision)?;
                Rc::make_mut(cache).apply(graph, &changes)
            });
            if let Some(applied) = applied {
                return self.follow(&applied, cx);
            }
        }
        let cache = cx.untracked(|app| GraphCache::build(model.read(app), metrics));
        self.shown = Shown::all(&cache);
        self.minimap_stale = true;
        self.cache = Some(Rc::new(cache));
        true
    }

    /// [`NodeCanvas::sync`] before using the copy outside a render.
    fn catch_up(&mut self, cx: &mut ViewContext<'_, Self>) {
        if self.sync(cx) {
            cx.notify();
        }
    }

    /// Show the changes the copy of the graph just took in. Returns whether
    /// the canvas must render for them.
    fn follow(&mut self, applied: &Applied, cx: &mut ViewContext<'_, Self>) -> bool {
        let Some(cache) = self.cache.clone() else {
            return true;
        };
        let area = self.built.map(|b| b.area);
        let mut render = !applied.edited.is_empty() || matches!(self.gesture, Gesture::Wire { .. });
        for &(i, _) in &applied.moved {
            let node = &cache.nodes[i];
            let inside = area.is_some_and(|a| node.rect.intersects(&a));
            match self.cards.get(&node.id) {
                Some(card) => {
                    NodeCard::place(*card, cx, node.rect.origin());
                    render |= !inside;
                }
                None => render |= inside,
            }
        }
        let wires_changed = area.is_some_and(|a| {
            applied
                .links
                .iter()
                .any(|&(i, old)| old.intersects(&a) || cache.links[i].1.intersects(&a))
        });
        if !render && wires_changed {
            self.update_wires(cx, &cache);
        }
        if self.shown.follow(&cache) {
            self.minimap_stale = true;
            self.place(cx);
        }
        if self.minimap_stale {
            self.place_minimap(cx, &cache);
        } else if let Some(mini) = self.minimap {
            for &(i, _) in &applied.moved {
                MinimapNodes::place(mini, cx, i, self.shown.map(cache.nodes[i].rect));
            }
        }
        render
    }

    /// Place every minimap rect (once there is a minimap).
    fn place_minimap(&mut self, cx: &mut ViewContext<'_, Self>, cache: &GraphCache) {
        let Some(mini) = self.minimap else {
            return;
        };
        for (i, n) in cache.nodes.iter().enumerate() {
            MinimapNodes::place(mini, cx, i, self.shown.map(n.rect));
        }
        self.minimap_stale = false;
    }

    /// Give the wire layer the links that reach into the built region.
    fn update_wires(&mut self, cx: &mut ViewContext<'_, Self>, cache: &GraphCache) {
        let (Some(layer), Some(built)) = (self.wires, self.built) else {
            return;
        };
        let wires: Vec<Wire> = cache
            .links
            .iter()
            .filter(|(_, bounds)| bounds.intersects(&built.area))
            .filter_map(|(l, _)| {
                let mut w = cache.wire(*l, cache.port_color(l.from))?;
                if self.selection.contains(&l.from.node) || self.selection.contains(&l.to.node) {
                    w.width = WIRE_WIDTH * 1.5;
                }
                Some(w)
            })
            .collect();
        self.stats.built_wires = wires.len();
        WireLayer::set(layer, cx, built.area, wires);
    }

    /// Create, update, place and release cards so that exactly the nodes at
    /// `shown` have one. Returns their elements in paint order.
    fn update_cards(
        &mut self,
        cx: &mut ViewContext<'_, Self>,
        cache: &GraphCache,
        shown: &[usize],
        lod: Lod,
    ) -> Vec<Element> {
        let pending = match self.gesture {
            Gesture::Wire { from, snap, .. } => Some((from, snap)),
            _ => None,
        };
        let view = cx.view();
        let mut elements = Vec::with_capacity(shown.len());
        let mut kept = HashSet::with_capacity(shown.len());
        for &i in shown {
            let node = &cache.nodes[i];
            let state = CardState {
                metrics: cache.metrics,
                lod,
                selected: self.selection.contains(&node.id),
                pending: pending.map(|(from, snap)| Pending::on(node.id, from, snap)),
            };
            let card = match self.cards.get(&node.id) {
                Some(card) => {
                    NodeCard::set(*card, cx, cache, node, state);
                    *card
                }
                None => {
                    let props = card::CardProps::new(cache, node, state);
                    let card = cx.new_view(NodeCard::new(&self.env, view, self.model, props));
                    cx.own(card.id());
                    self.cards.insert(node.id, card);
                    card
                }
            };
            NodeCard::place(card, cx, node.rect.origin());
            kept.insert(node.id);
            elements.push(Element::view(&card).key(format!("node-{}", node.id)));
        }
        let gone: Vec<NodeId> = self
            .cards
            .keys()
            .copied()
            .filter(|id| !kept.contains(id))
            .collect();
        for id in gone {
            if let Some(card) = self.cards.remove(&id) {
                cx.release(card.id());
            }
        }
        elements
    }

    /// Watch the model: apply its changes at the start of the next frame, so
    /// several edits cost one update.
    fn watch(&mut self, cx: &mut ViewContext<'_, Self>) {
        if self.watching {
            return;
        }
        self.watching = true;
        let view = cx.view();
        cx.watch(self.model.id(), move |app| {
            app.animate(view.id(), SYNC_TASK, move |app, _| {
                view.update(app, |s, vcx| s.catch_up(vcx));
                false
            });
        });
    }
}

/// A window position relative to the canvas's top left (view space), under
/// whatever transforms the canvas itself.
fn view_point<M: NodeGraphModel>(view: View<NodeCanvas<M>>, app: &App, window: Point) -> Point {
    let root = app.view_element(view.id());
    match root.and_then(|r| Some((app.to_layout(r, window)?, app.bounds(r)?))) {
        Some((p, b)) => p - b.origin(),
        None => window,
    }
}

fn on_scroll<M: NodeGraphModel>(
    view: View<NodeCanvas<M>>,
    ev: &PointerEvent,
    cx: &mut EventCx<'_>,
) {
    let PointerKind::Scroll(delta) = ev.kind else {
        return;
    };
    let zoom_mod = ev.modifiers.ctrl || ev.modifiers.cmd;
    match delta {
        ScrollDelta::Pixels(d) if !zoom_mod => NodeCanvas::pan_by(view, cx, d),
        ScrollDelta::Pixels(d) => {
            NodeCanvas::zoom_about(view, cx, ev.position, (d.y * PIXEL_ZOOM).exp());
        }
        ScrollDelta::Lines(l) => {
            NodeCanvas::zoom_about(view, cx, ev.position, WHEEL_ZOOM.powf(l.y));
        }
    }
}

/// Whether the press that started a drag pans the canvas.
fn panning<M: NodeGraphModel>(s: &NodeCanvas<M>) -> bool {
    s.space || s.button == Some(MouseButton::Middle)
}

fn background_drag<M: NodeGraphModel>(view: View<NodeCanvas<M>>, ev: &DragEvent, cx: &mut App) {
    let start_at = view_point(view, cx, ev.start);
    let now_at = view_point(view, cx, ev.position);
    view.update(cx, |s, vcx| {
        if ev.phase == DragPhase::Start {
            s.catch_up(vcx);
            s.gesture = if panning(s) {
                Gesture::Pan
            } else {
                let start = s.canvas.to_canvas(start_at);
                let base = if s.shift {
                    s.selection.clone()
                } else {
                    BTreeSet::new()
                };
                Gesture::Rubber {
                    start,
                    now: start,
                    base,
                }
            };
        }
        let now_c = s.canvas.to_canvas(now_at);
        let mut changed = false;
        let mut panned = false;
        match &mut s.gesture {
            Gesture::Pan => panned = true,
            Gesture::Rubber { start, now, base } => {
                *now = now_c;
                if let Some(cache) = &s.cache {
                    let mut sel = base.clone();
                    sel.extend(rubber_select(cache, *start, *now));
                    s.selection = sel;
                }
                changed = true;
            }
            _ => {}
        }
        if panned {
            s.pan(ev.delta);
            s.camera_changed(vcx);
        }
        if ev.phase == DragPhase::End {
            s.gesture = Gesture::None;
        }
        if changed {
            vcx.notify();
        }
    });
}

fn node_down<M: NodeGraphModel>(view: View<NodeCanvas<M>>, id: NodeId, cx: &mut App) {
    view.update(cx, |s, vcx| {
        if panning(s) {
            return;
        }
        let was = s.selection.contains(&id);
        s.press = Some((id, was));
        if s.shift {
            if was {
                s.selection.remove(&id);
            } else {
                s.selection.insert(id);
            }
        } else if !was {
            s.selection.clear();
            s.selection.insert(id);
        }
        vcx.notify();
    });
}

fn node_click<M: NodeGraphModel>(view: View<NodeCanvas<M>>, id: NodeId, cx: &mut App) {
    view.update(cx, |s, vcx| {
        // A plain click on a node of a multi-selection selects only it.
        if !s.shift && s.press == Some((id, true)) && s.selection.len() > 1 {
            s.selection.clear();
            s.selection.insert(id);
            vcx.notify();
        }
        s.press = None;
    });
}

fn node_drag<M: NodeGraphModel>(view: View<NodeCanvas<M>>, ev: &DragEvent, cx: &mut App) {
    let model = view.read(cx).model;
    let moves = view.update(cx, |s, vcx| {
        if ev.phase == DragPhase::Start {
            s.catch_up(vcx);
            if panning(s) {
                s.gesture = Gesture::Pan;
            } else {
                let starts = s
                    .selection
                    .iter()
                    .filter_map(|id| Some((*id, s.cache.as_ref()?.node(*id)?.rect.origin())))
                    .collect();
                s.gesture = Gesture::Move(starts);
            }
            s.press = None;
        }
        let out = match &s.gesture {
            Gesture::Move(starts) => {
                let d = ev.total() * (1.0 / s.canvas.zoom);
                starts.iter().map(|(id, p)| (*id, *p + d)).collect()
            }
            _ => Vec::new(),
        };
        if matches!(s.gesture, Gesture::Pan) {
            s.pan(ev.delta);
            s.camera_changed(vcx);
        }
        if ev.phase == DragPhase::End {
            if matches!(s.gesture, Gesture::Move(_)) {
                vcx.notify();
            }
            s.gesture = Gesture::None;
        }
        out
    });
    let ended = ev.phase == DragPhase::End;
    if !moves.is_empty() {
        model.update(cx, |g, mcx| {
            for (id, p) in moves {
                g.set_position(id, p);
            }
            if ended {
                g.end_gesture();
            }
            mcx.notify();
        });
    }
}

fn port_drag<M: NodeGraphModel>(
    view: View<NodeCanvas<M>>,
    port: PortRef,
    ev: &DragEvent,
    cx: &mut App,
) {
    if ev.phase == DragPhase::Start && panning(view.read(cx)) {
        view.update(cx, |s, _| s.gesture = Gesture::Pan);
    }
    if matches!(view.read(cx).gesture, Gesture::Pan) {
        view.update(cx, |s, vcx| {
            s.pan(ev.delta);
            s.camera_changed(vcx);
            if ev.phase == DragPhase::End {
                s.gesture = Gesture::None;
            }
        });
        return;
    }
    view.update(cx, |s, vcx| s.catch_up(vcx));
    // Dragging from a linked input takes its link off and carries it: the
    // wire then starts at the output it came from.
    if ev.phase == DragPhase::Start && port.side == PortSide::Input {
        let link = view
            .read(cx)
            .cache
            .as_ref()
            .and_then(|c| c.links.iter().map(|(l, _)| *l).find(|l| l.to == port));
        if let Some(link) = link {
            view.update(cx, |s, _| {
                s.gesture = Gesture::Wire {
                    from: link.from,
                    pointer: Point::ZERO,
                    snap: None,
                };
            });
            let model = view.read(cx).model;
            model.update(cx, |g, mcx| {
                g.disconnect(link);
                mcx.notify();
            });
            view.update(cx, |s, vcx| s.catch_up(vcx));
        }
    }
    // Where the wire being dragged starts.
    let port = match view.read(cx).gesture {
        Gesture::Wire { from, .. } => from,
        _ => port,
    };
    let at = view_point(view, cx, ev.position);
    let (pointer, snap) = {
        let s = view.read(cx);
        let pointer = s.canvas.to_canvas(at);
        let snap = s.cache.as_ref().and_then(|cache| {
            find_snap(cache, port, pointer, SNAP_RADIUS / s.canvas.zoom, |o, i| {
                s.model_ok(cx, o, i)
            })
        });
        (pointer, snap)
    };
    if ev.phase == DragPhase::End {
        view.update(cx, |s, vcx| {
            s.gesture = Gesture::None;
            vcx.notify();
        });
        let model = view.read(cx).model;
        model.update(cx, |g, mcx| {
            if let Some(target) = snap {
                let (from, to) = oriented(port, target);
                g.connect(Link { from, to });
            }
            g.end_gesture();
            mcx.notify();
        });
        return;
    }
    view.update(cx, |s, vcx| {
        s.gesture = Gesture::Wire {
            from: port,
            pointer,
            snap,
        };
        vcx.notify();
    });
}

/// Centre the view on the pressed point of the minimap.
fn minimap_jump<M: NodeGraphModel>(view: View<NodeCanvas<M>>, cx: &mut EventCx<'_>, window: Point) {
    let local = cx.layout_position(window) - cx.bounds().origin();
    view.update(cx, |s, vcx| {
        s.catch_up(vcx);
        if s.cache.is_none() {
            return;
        }
        let c = s.shown.to_canvas(local);
        let size = s.view_size(vcx);
        let mut canvas = s.canvas;
        canvas.pan = Vec2::new(size.width / 2.0, size.height / 2.0) - c * canvas.zoom;
        s.jump(canvas);
        s.camera_changed(vcx);
    });
}

/// Fill an absolutely positioned element over its whole parent.
fn fill(s: &mut livspeil_ui_layout::Style) {
    s.position = Position::Absolute;
    let z = LengthPercentageAuto::length(0.0);
    s.inset.left = z;
    s.inset.top = z;
    s.size.width = Dimension::percent(1.0);
    s.size.height = Dimension::percent(1.0);
}

impl<M: NodeGraphModel> Render for NodeCanvas<M> {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let col = theme.colors();

        let _ = self.sync(cx);
        self.watch(cx);
        let Some(cache) = self.cache.clone() else {
            return div();
        };

        // The canvas's own size from the last layout (re-rendered when it
        // changes), so culling and the minimap are right before any
        // pointer event.
        if let Some(b) = cx.layout_bounds().filter(|b| b.width > 0.0) {
            self.origin = b.origin();
            self.size = b.size();
        }
        let size = self.view_size(cx);
        let zoom = self.canvas.zoom;
        let built = match self.built {
            Some(b) if b.covers(&self.canvas, size) => b,
            _ => Built::around(&self.canvas, size),
        };
        self.built = Some(built);
        let (lod, area) = (built.lod, built.area);
        self.place(cx);

        // Wires below the cards, in canvas coordinates.
        let shown = cache.nodes_in(&area);
        let wires = match self.wires {
            Some(w) => w,
            None => {
                let w = cx.new_view(WireLayer::default());
                cx.own(w.id());
                self.wires = Some(w);
                w
            }
        };
        self.update_wires(cx, &cache);
        // The world: one layer panned and zoomed over the canvas.
        let mut canvas = div()
            .key("canvas")
            .hit_test(false)
            .style(fill)
            .layer(cx.layer("world"))
            .cull_offscreen()
            .child(Element::view(&wires).key("wires"))
            .children(self.update_cards(cx, &cache, &shown, lod));
        // The pending wire above the cards.
        if let Gesture::Wire {
            from,
            pointer,
            snap,
        } = self.gesture
            && let Some(a) = cache.port_position(from)
        {
            let b = snap.and_then(|p| cache.port_position(p)).unwrap_or(pointer);
            let (p0, p1) = if from.side == PortSide::Output {
                (a, b)
            } else {
                (b, a)
            };
            let w = Wire::link(p0, p1, WIRE_WIDTH, cache.port_color(from));
            let area = w.bounds().inflate(4.0);
            canvas = canvas.child(
                div()
                    .key("pending")
                    .name("pending-wire")
                    .absolute(area)
                    .hit_test(false)
                    .wires(vec![Wire {
                        from: w.from - area.origin(),
                        c1: w.c1 - area.origin(),
                        c2: w.c2 - area.origin(),
                        to: w.to - area.origin(),
                        ..w
                    }]),
            );
        }
        self.stats = CanvasStats {
            nodes: cache.nodes.len(),
            links: cache.links.len(),
            built_nodes: shown.len(),
            built_wires: self.stats.built_wires,
            lod,
        };

        let mut root = div()
            .name(self.name.clone())
            .w_full()
            .h_full()
            .clip()
            .bg(col.surface[0])
            .focusable()
            .key_context("NodeCanvas")
            .child(canvas);

        // Rubber band, in view coordinates.
        if let Some(r) = self.rubber_band() {
            let a = self.canvas.to_view(r.origin());
            root = root.child(
                div()
                    .key("rubber")
                    .name("rubber-band")
                    .absolute(Rect::new(a.x, a.y, r.width * zoom, r.height * zoom))
                    .bg(theme.accent().with_alpha(0.12))
                    .border(1.0, theme.accent().with_alpha(0.8))
                    .hit_test(false),
            );
        }

        // Minimap.
        if self.show_minimap && !cache.nodes.is_empty() {
            let mini = match self.minimap {
                Some(m) => m,
                None => {
                    let m = cx.new_view(MinimapNodes::new(Color::WHITE, Color::WHITE));
                    cx.own(m.id());
                    self.minimap = Some(m);
                    self.minimap_stale = true;
                    m
                }
            };
            // One rect per node: rendered only when nodes are added or
            // removed or the selection changes.
            let selected = cache
                .nodes
                .iter()
                .map(|n| self.selection.contains(&n.id))
                .collect();
            let (color, accent) = (col.text_secondary.with_alpha(0.55), theme.accent());
            MinimapNodes::set(mini, cx, selected, color, accent);
            if self.minimap_stale {
                self.place_minimap(cx, &cache);
            }
            let minimap = div()
                .key("minimap")
                .name("minimap")
                .material(Material::Glass)
                .rounded(theme.tokens.radii.md)
                .clip()
                .style(|s| {
                    s.position = Position::Absolute;
                    s.inset.right = LengthPercentageAuto::length(12.0);
                    s.inset.bottom = LengthPercentageAuto::length(12.0);
                    s.size.width = Dimension::length(MINIMAP.width);
                    s.size.height = Dimension::length(MINIMAP.height);
                })
                .child(Element::view(&mini).key("nodes"))
                .child(
                    // Placed by its layer's frame (`NodeCanvas::place`).
                    div()
                        .key("viewport")
                        .name("minimap-viewport")
                        .absolute(Rect::ZERO)
                        .layer(cx.layer("minimap-viewport"))
                        .rounded(2.0)
                        .bg(Color::WHITE.with_alpha(0.06))
                        .border(1.0, Color::WHITE.with_alpha(0.7))
                        .hit_test(false),
                )
                .on_pointer_down(move |ev, cx| {
                    minimap_jump(view, cx, ev.position);
                    cx.stop_propagation();
                })
                .on_drag(move |ev, cx| minimap_jump(view, cx, ev.position));
            root = root.child(minimap);
        }

        let sem = Semantics::new(Role::NodeCanvas).label("Node graph");
        let root = sem.apply(root);
        root.on_pointer(PointerFilter::Down, Phase::Capture, move |ev, cx| {
            let button = match ev.kind {
                PointerKind::Down(b) => Some(b),
                _ => None,
            };
            let shift = ev.modifiers.shift;
            view.update(cx, |s, _| {
                s.button = button;
                s.shift = shift;
            });
        })
        .on_scroll(move |ev, cx| {
            on_scroll(view, ev, cx);
            cx.stop_propagation();
        })
        .on_drag(move |ev, cx| background_drag(view, ev, cx))
        .on_click(move |ev, cx| {
            if ev.keyboard {
                return;
            }
            view.update(cx, |s, vcx| {
                if !ev.modifiers.shift && !s.selection.is_empty() {
                    s.selection.clear();
                    vcx.notify();
                }
            });
        })
        .on_key(Phase::Capture, move |ev, cx| {
            if ev.key == Key::Named(NamedKey::Space) && cx.focused() == Some(cx.element_id()) {
                let pressed = ev.pressed;
                view.update(cx, |s, _| s.space = pressed);
                if pressed {
                    cx.request_cursor(CursorShape::Grab);
                }
                cx.stop_propagation();
            }
        })
        .on_action(crate::keymap::actions::SELECT_ALL, move |cx| {
            view.update(cx, |s, vcx| {
                s.catch_up(vcx);
                if let Some(c) = &s.cache {
                    s.selection = c.nodes.iter().map(|n| n.id).collect();
                    vcx.notify();
                }
            });
        })
        .on_action(core::CANCEL, move |cx| {
            view.update(cx, |s, vcx| {
                s.gesture = Gesture::None;
                s.selection.clear();
                vcx.notify();
            });
        })
        .semantics(|s| s.role(CoreRole::Group))
    }
}
