// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`NodeCanvas`]: the node editor prototype of Dream Mode (UI spec §9).
//!
//! An infinite canvas (`livspeil_ui_layout::Canvas`) showing a
//! [`NodeGraphModel`]: node cards with typed ports and parameter rows,
//! links as Bézier wires, a minimap. See [`NodeCanvas`] for interactions.

mod geom;
mod model;

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::rc::Rc;

use livspeil_ui_core::{
    Animated, App, CursorShape, DragEvent, DragPhase, Element, EventCx, Key, Model, MouseButton,
    NamedKey, Phase, PointerEvent, PointerFilter, PointerKind, Render, Role as CoreRole,
    ScrollDelta, SharedString, View, ViewContext, Wire, actions as core, div, text,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto, Position};
use livspeil_ui_layout::{Canvas, Point, Rect, Size, Vec2};
use livspeil_ui_theme::{Color, Elevation, Material, MotionClass, PropertyKind, Theme};

pub use geom::{
    CardMetrics, GraphCache, NODE_WIDTH, NodeGeom, PORT_DOT, SNAP_RADIUS, WIRE_WIDTH, find_snap,
    graph_bounds, oriented, rect_from, rubber_select,
};
pub use model::{
    DemoGraph, DemoNode, Link, NodeGraphModel, NodeId, ParamInfo, ParamValue, PortInfo, PortRef,
    PortSide, palette_color,
};

use crate::env::Env;
use crate::number_field::NumberField;
use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};
use crate::toggle::Toggle;

/// Motion scope of canvas zoom (`[overrides.canvas]`).
pub const CANVAS_SCOPE: &str = "canvas";
/// Zoom from which cards show labels and parameters.
pub const LOD_FULL: f32 = 0.6;
/// Zoom from which cards show their header and ports (below: blocks).
pub const LOD_COMPACT: f32 = 0.3;
/// Minimap size.
pub const MINIMAP: Size = Size {
    width: 200.0,
    height: 130.0,
};
/// The built region reaches past the view on each side by this fraction of
/// the view, and at least by [`MIN_MARGIN`] view pixels.
const MARGIN: f32 = 0.25;
const MIN_MARGIN: f32 = 96.0;
/// The built region is rebuilt smaller once it is this many times wider or
/// taller than a new one would be.
const SHRINK: f32 = 2.0;
/// Key of the zoom spring's frame task.
const ZOOM_TASK: &str = "zoom";
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
    /// margin around it).
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

/// Minimap content: one rect per node, rebuilt per model revision only.
struct MinimapNodes {
    rects: Rc<Vec<(Rect, bool)>>,
    color: Color,
    selected: Color,
}

impl Render for MinimapNodes {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        let mut el = div().name("minimap-nodes").hit_test(false).style(|s| {
            s.position = Position::Absolute;
            s.size.width = Dimension::percent(1.0);
            s.size.height = Dimension::percent(1.0);
        });
        for (i, (r, sel)) in self.rects.iter().enumerate() {
            el = el.child(
                div()
                    .key(format!("{i}"))
                    .absolute(*r)
                    .rounded(1.0)
                    .bg(if *sel { self.selected } else { self.color })
                    .hit_test(false),
            );
        }
        el
    }
}

/// What the minimap shows: the graph with a margin of half its size (so
/// the viewport frame stays readable around it).
fn minimap_bounds(cache: &GraphCache) -> Rect {
    let b = graph_bounds(cache);
    b.inflate(b.width.max(b.height) * 0.15)
}

/// Maps graph bounds into the minimap: `(scale, offset)` with
/// `minimap = canvas × scale + offset`.
#[must_use]
pub fn minimap_mapping(bounds: Rect, size: Size, pad: f32) -> (f32, Vec2) {
    let w = (size.width - 2.0 * pad).max(1.0);
    let h = (size.height - 2.0 * pad).max(1.0);
    let s = (w / bounds.width.max(1.0)).min(h / bounds.height.max(1.0));
    let off = Vec2::new(
        pad + (w - bounds.width * s) / 2.0 - bounds.x * s,
        pad + (h - bounds.height * s) / 2.0 - bounds.y * s,
    );
    (s, off)
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
/// - **Connect**: drag from a port: a pending wire follows the pointer and
///   snaps to the nearest compatible port within [`SNAP_RADIUS`] px
///   ([`NodeGraphModel::can_connect`]); incompatible ports dim; releasing
///   on a snapped port connects.
/// - **Parameters**: numbers are `NumberField`s (scrub or type), flags
///   `Toggle`s; edits go to [`NodeGraphModel::set_param`].
/// - **Virtualization**: only nodes and wires intersecting the built
///   region (the viewport and a margin) are built, through a spatial index
///   cached per model revision; cards lose detail when zoomed out
///   ([`Lod`]). The minimap (bottom right, Glass) is a separate view
///   rebuilt per revision.
/// - **Layers**: pan and zoom set the transform of one layer holding the
///   wires and cards, and move the minimap's viewport frame. The canvas
///   renders again only when the view leaves the built region, the level of
///   detail changes, or the content changes.
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
    fields: HashMap<(NodeId, usize), View<NumberField>>,
    /// The minimap's view and the revision and selection it shows.
    minimap: Option<(View<MinimapNodes>, u64, BTreeSet<NodeId>)>,
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
            fields: HashMap::new(),
            minimap: None,
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

    /// The geometry cache of the last render.
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
        let frame = self.cache.as_ref().map(|cache| {
            let (k, off) = minimap_mapping(minimap_bounds(cache), MINIMAP, 8.0);
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
    if !moves.is_empty() {
        model.update(cx, |g, mcx| {
            for (id, p) in moves {
                g.set_position(id, p);
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
        if let Some(target) = snap {
            let (from, to) = oriented(port, target);
            let model = view.read(cx).model;
            model.update(cx, |g, mcx| {
                g.connect(Link { from, to });
                mcx.notify();
            });
        }
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
        let Some(cache) = &s.cache else { return };
        let (k, off) = minimap_mapping(minimap_bounds(cache), MINIMAP, 8.0);
        let c = (local - off) * (1.0 / k);
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

struct CardCx<'a> {
    theme: &'a Theme,
    cache: &'a GraphCache,
    lod: Lod,
    selected: bool,
    pending: Option<(PortRef, Option<PortRef>)>,
}

impl<M: NodeGraphModel> NodeCanvas<M> {
    fn param_field(
        &mut self,
        cx: &mut ViewContext<'_, Self>,
        node: NodeId,
        index: usize,
        name: &SharedString,
        value: ParamValue,
    ) -> Option<Element> {
        let ParamValue::Number {
            value: v,
            min,
            max,
            step,
        } = value
        else {
            return None;
        };
        let view = match self.fields.get(&(node, index)) {
            Some(f) => *f,
            None => {
                let m = cx.new_model(v);
                let f = cx.new_view(
                    NumberField::new(&self.env, m, min, max, step)
                        .width(84.0)
                        .accessible_label(name.clone())
                        .name(format!("param:{node}:{index}")),
                );
                let graph = self.model;
                cx.observe(m.id(), move |app| {
                    let v = *m.read(app);
                    graph.update(app, |g, mcx| {
                        g.set_param(
                            node,
                            index,
                            ParamValue::Number {
                                value: v,
                                min,
                                max,
                                step,
                            },
                        );
                        mcx.notify();
                    });
                });
                self.fields.insert((node, index), f);
                f
            }
        };
        Some(Element::view(&view))
    }

    fn port_dot(&self, view: View<Self>, c: &CardCx<'_>, port: PortRef, local: Point) -> Element {
        let color = c.cache.port_color(port);
        let surface = c.theme.colors().surface[1];
        let (dim, hot) = match c.pending {
            Some((from, snap)) => {
                let same_side = from.side == port.side || from.node == port.node;
                (same_side, snap == Some(port))
            }
            None => (false, false),
        };
        let r = PORT_DOT / 2.0;
        let mut dot = div()
            .key(format!(
                "port-{}-{}",
                if port.side == PortSide::Input {
                    "in"
                } else {
                    "out"
                },
                port.index
            ))
            .name(format!(
                "port:{}:{}:{}",
                port.node,
                if port.side == PortSide::Input {
                    "in"
                } else {
                    "out"
                },
                port.index
            ))
            .absolute(Rect::new(local.x - r, local.y - r, PORT_DOT, PORT_DOT))
            .rounded(r)
            .bg(color)
            .border(2.0, surface)
            .opacity(if dim { 0.3 } else { 1.0 })
            .scale(if hot { 1.6 } else { 1.0 })
            .hover(|x| x.scale(1.4))
            .transition(MotionClass::Micro)
            .cursor(CursorShape::Crosshair);
        if c.lod != Lod::Block {
            dot = dot
                .on_pointer_down(|_, cx| cx.stop_propagation())
                .on_drag(move |ev, cx| port_drag(view, port, ev, cx));
        }
        dot
    }

    fn card(
        &mut self,
        cx: &mut ViewContext<'_, Self>,
        view: View<Self>,
        n: &NodeGeom,
        c: &CardCx<'_>,
    ) -> Element {
        let theme = c.theme;
        let col = theme.colors();
        let m = metrics(theme);
        let cm = c.cache.metrics;
        let id = n.id;
        let radius = theme.tokens.radii.sm + 2.0;
        let mut card = div()
            .key(format!("node-{id}"))
            .name(format!("node:{}", n.title))
            .absolute(n.rect)
            .flex_col()
            .rounded(radius)
            .bg(col.surface[1].with_alpha(0.97))
            .on_pointer_down(move |ev, cx| {
                if matches!(ev.kind, PointerKind::Down(MouseButton::Left)) {
                    node_down(view, id, cx);
                }
            })
            .on_click(move |_, cx| node_click(view, id, cx))
            .on_drag(move |ev, cx| node_drag(view, ev, cx));
        let sem = Semantics::new(Role::Node)
            .label(&*n.title)
            .selected(c.selected);
        if c.lod == Lod::Block {
            let tint = n
                .outputs
                .first()
                .or(n.inputs.first())
                .and_then(|p| c.cache.colors.get(&p.ty))
                .copied()
                .unwrap_or(col.surface[3]);
            return sem.apply(card.bg(tint.with_alpha(0.55)).border(
                if c.selected { 4.0 } else { 1.0 },
                if c.selected { theme.accent() } else { tint },
            ));
        }
        card = card
            .border(1.0, style::hairline(theme).with_alpha(0.12))
            .elevation(if c.selected {
                Elevation::E2
            } else {
                Elevation::E1
            });
        let header = div()
            .key("header")
            .h(cm.header)
            .px(10.0)
            .flex_row()
            .items_center()
            .radii(livspeil_ui_layout::Corners {
                top_left: theme.tokens.radii.sm + 2.0,
                top_right: theme.tokens.radii.sm + 2.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            })
            .bg(col.surface[2])
            .hit_test(false)
            .child(
                text(n.title.clone())
                    .font_size(m.font)
                    .font_weight(600.0)
                    .text_color(col.text_primary)
                    .hit_test(false),
            );
        card = card.child(header);
        if c.lod == Lod::Full {
            let rows = n.inputs.len().max(n.outputs.len());
            let label = |s: &SharedString| {
                text(s.clone())
                    .font_size(m.font - 1.0)
                    .text_color(col.text_secondary)
                    .hit_test(false)
            };
            for r in 0..rows {
                let mut row = div()
                    .key(format!("ports-{r}"))
                    .h(cm.row)
                    .px(12.0)
                    .flex_row()
                    .items_center()
                    .hit_test(false);
                row = match n.inputs.get(r) {
                    Some(p) => row.child(label(&p.name)),
                    None => row,
                };
                row = row.child(div().grow());
                row = match n.outputs.get(r) {
                    Some(p) => row.child(label(&p.name)),
                    None => row,
                };
                card = card.child(row);
            }
            for (i, p) in n.params.iter().enumerate() {
                let control = match p.value {
                    ParamValue::Number { .. } => self.param_field(cx, id, i, &p.name, p.value),
                    ParamValue::Toggle(on) => {
                        let graph = self.model;
                        Some(
                            Toggle::new(on)
                                .label(p.name.clone())
                                .on_change(move |v, cx| {
                                    graph.update(cx, |g, mcx| {
                                        g.set_param(id, i, ParamValue::Toggle(v));
                                        mcx.notify();
                                    });
                                })
                                .render(theme)
                                .key("toggle"),
                        )
                    }
                };
                let mut row = div()
                    .key(format!("param-{i}"))
                    .h(cm.row)
                    .px(12.0)
                    .flex_row()
                    .items_center()
                    .gap(8.0);
                if matches!(p.value, ParamValue::Number { .. }) {
                    row = row.child(label(&p.name).grow());
                }
                card = card.child(row.children(control));
            }
        }
        if c.selected {
            // The selection ring paints above the header and rows (which
            // would cover a border drawn with the card's own quad), below
            // the port dots.
            card = card.child(
                div()
                    .key("selection-ring")
                    .style(fill)
                    .rounded(radius)
                    .border(2.0, theme.accent())
                    .hit_test(false),
            );
        }
        for side in [PortSide::Input, PortSide::Output] {
            for index in 0..n.ports(side).len() {
                let port = PortRef {
                    node: id,
                    side,
                    index,
                };
                let at = cm.port_position(Point::ZERO, side, index);
                card = card.child(self.port_dot(view, c, port, at));
            }
        }
        sem.apply(card)
    }
}

impl<M: NodeGraphModel> Render for NodeCanvas<M> {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let col = theme.colors();

        // Model cache.
        let metrics = CardMetrics::from_theme(&theme);
        let revision = self.model.read(cx).revision();
        let stale = self
            .cache
            .as_ref()
            .is_none_or(|c| c.revision != revision || c.metrics != metrics);
        if stale {
            self.cache = Some(Rc::new(GraphCache::build(self.model.read(cx), metrics)));
        }
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
        let pending = self.pending_wire();
        self.place(cx);

        // Wires below the cards, in canvas coordinates.
        let shown = cache.nodes_in(&area);
        let mut wires = Vec::with_capacity(shown.len() * 2);
        for (l, b) in &cache.links {
            if !b.intersects(&area) {
                continue;
            }
            let hot = self.selection.contains(&l.from.node) || self.selection.contains(&l.to.node);
            let color = cache.port_color(l.from);
            if let Some(mut w) = cache.wire(*l, color) {
                if hot {
                    w.width = WIRE_WIDTH * 1.5;
                }
                wires.push(w);
            }
        }
        let n_wires = wires.len();
        // The world: one layer panned and zoomed over the canvas.
        let mut canvas = div()
            .key("canvas")
            .hit_test(false)
            .style(fill)
            .layer(cx.layer("world"))
            .cull_offscreen()
            .wires(wires);
        // The cards are read straight from the cache (no copies).
        for i in &shown {
            let n = &cache.nodes[*i];
            let c = CardCx {
                theme: &theme,
                cache: &cache,
                lod,
                selected: self.selection.contains(&n.id),
                pending,
            };
            let card = self.card(cx, view, n, &c);
            canvas = canvas.child(card);
        }
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
            built_wires: n_wires,
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
            let bounds = minimap_bounds(&cache);
            let (k, off) = minimap_mapping(bounds, MINIMAP, 8.0);
            // One rect per node: rebuilt only when the model or the
            // selection changed, not on every pan or zoom frame.
            let content = || {
                Rc::new(
                    cache
                        .nodes
                        .iter()
                        .map(|n| {
                            let r = n.rect;
                            (
                                Rect::new(
                                    r.x * k + off.x,
                                    r.y * k + off.y,
                                    (r.width * k).max(1.0),
                                    (r.height * k).max(1.0),
                                ),
                                self.selection.contains(&n.id),
                            )
                        })
                        .collect::<Vec<(Rect, bool)>>(),
                )
            };
            let mini_view = match &self.minimap {
                Some((v, rev, sel)) if *rev == revision && *sel == self.selection => *v,
                Some((v, _, _)) => {
                    let (v, rects) = (*v, content());
                    v.update(cx, |mm, mcx| {
                        mm.rects = rects;
                        mcx.notify();
                    });
                    v
                }
                None => cx.new_view(MinimapNodes {
                    rects: content(),
                    color: col.text_secondary.with_alpha(0.55),
                    selected: theme.accent(),
                }),
            };
            self.minimap = Some((mini_view, revision, self.selection.clone()));
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
                .child(Element::view(&mini_view))
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
