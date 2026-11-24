// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The [`App`]: entities, observation, the retained tree, input routing,
//! focus, animations and the frame pipeline.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::time::Duration;

use livspeil_ui_layout::{Point, Rect, Size, Vec2};
use livspeil_ui_theme::{ConfigError, ConfigEvent, MotionConfig, Theme};

use crate::element::{ClickHandler, Element, ElementId, ElementKind, Phase, PointerFilter, div};
use crate::entity::{AnyView, EntityId, Model, Render, View};
use crate::input::{
    ClickEvent, DragEvent, DragPhase, ImeEvent, KeyEvent, Modifiers, PointerEvent, PointerKind,
    VelocityTracker,
};
use crate::keymap::{Keymap, actions};
use crate::paint::{PaintInput, PaintStats, Scene, paint};
use crate::tree::{Started, Targets, Tree, dump, hit_path, reconcile, update_bounds};

/// Pointer travel (px) before a press becomes a drag.
pub const DRAG_THRESHOLD: f32 = 3.0;

/// A subscription made with [`App::observe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subscription {
    entity: EntityId,
    id: u64,
}

type Callback = Rc<dyn Fn(&mut App)>;

#[derive(Debug, Clone, Copy)]
struct DragState {
    target: ElementId,
    start: Point,
    last: Point,
    active: bool,
}

#[derive(Debug, Default)]
struct InputState {
    pointer: Option<Point>,
    hovered: Vec<ElementId>,
    pressed: Vec<ElementId>,
    capture: Option<Vec<ElementId>>,
    drag: Option<DragState>,
    tracker: VelocityTracker,
    modifiers: Modifiers,
}

/// Animation bookkeeping: the list of animating nodes (capacity reserved
/// up front and reused) and the two large-animation slots.
#[derive(Debug)]
struct AnimationDriver {
    active: Vec<ElementId>,
    large: [Option<(ElementId, u64)>; 2],
    seq: u64,
    evicted: usize,
}

impl Default for AnimationDriver {
    fn default() -> Self {
        AnimationDriver {
            active: Vec::with_capacity(64),
            large: [None; 2],
            seq: 0,
            evicted: 0,
        }
    }
}

/// Animation counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimationStats {
    /// Nodes with a moving property.
    pub active: usize,
    /// Capacity of the active list (stable once warmed up: no allocation).
    pub capacity: usize,
    /// Large animations running (at most two).
    pub large: usize,
    /// Large animations finished early to respect the budget.
    pub large_evicted: usize,
}

/// Counters for one frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameStats {
    /// Views whose `render` ran.
    pub views_rendered: usize,
    /// Whether layout was computed.
    pub layout: bool,
    /// Nodes in the tree.
    pub nodes: usize,
    /// Animating nodes after this frame.
    pub animating: usize,
    /// Paint counters.
    pub paint: PaintStats,
}

/// The output of [`App::draw_frame`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// The display list.
    pub scene: Scene,
    /// Whether another frame is needed (something is still moving).
    pub needs_next_frame: bool,
    /// Counters.
    pub stats: FrameStats,
}

/// The application context.
pub struct App {
    next_id: u64,
    entities: HashMap<EntityId, Option<Box<dyn Any>>>,
    views: HashMap<EntityId, AnyView>,
    deps: HashMap<EntityId, Vec<EntityId>>,
    observers: HashMap<EntityId, BTreeSet<EntityId>>,
    subscriptions: HashMap<EntityId, Vec<(u64, Callback)>>,
    next_sub: u64,
    tracking: RefCell<Option<Vec<EntityId>>>,
    pending: Vec<EntityId>,
    depth: u32,
    flushing: bool,
    pub(crate) dirty_views: BTreeSet<EntityId>,
    pub(crate) view_slots: HashMap<EntityId, ElementId>,
    render_counts: HashMap<EntityId, usize>,
    renders_this_frame: usize,
    root_view: Option<AnyView>,
    root_mounted: bool,
    pub(crate) tree: Tree,
    theme: Theme,
    motion: MotionConfig,
    keymap: Keymap,
    input: InputState,
    focus: Option<ElementId>,
    focus_visible: bool,
    element_state: HashMap<ElementId, HashMap<TypeId, Box<dyn Any>>>,
    driver: AnimationDriver,
    viewport: Size,
    paint_dirty: bool,
    frame_requested: bool,
    frame_dt: f32,
    frame_count: u64,
    pub(crate) text_measurer: crate::text_measure::MeasurerSlot,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("entities", &self.entities.len())
            .field("nodes", &self.tree.nodes.len())
            .field("viewport", &self.viewport)
            .field("frames", &self.frame_count)
            .finish_non_exhaustive()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// An app with the default theme, motion and keymap, and an 800×600 viewport.
    #[must_use]
    pub fn new() -> Self {
        App {
            next_id: 1,
            entities: HashMap::new(),
            views: HashMap::new(),
            deps: HashMap::new(),
            observers: HashMap::new(),
            subscriptions: HashMap::new(),
            next_sub: 1,
            tracking: RefCell::new(None),
            pending: Vec::new(),
            depth: 0,
            flushing: false,
            dirty_views: BTreeSet::new(),
            view_slots: HashMap::new(),
            render_counts: HashMap::new(),
            renders_this_frame: 0,
            root_view: None,
            root_mounted: false,
            tree: Tree::default(),
            theme: Theme::default(),
            motion: MotionConfig::default(),
            keymap: Keymap::defaults(),
            input: InputState::default(),
            focus: None,
            focus_visible: false,
            element_state: HashMap::new(),
            driver: AnimationDriver::default(),
            viewport: Size::new(800.0, 600.0),
            paint_dirty: false,
            frame_requested: false,
            frame_dt: 0.0,
            frame_count: 0,
            text_measurer: crate::text_measure::MeasurerSlot::default(),
        }
    }

    // ---- entities ----------------------------------------------------

    fn alloc(&mut self, value: Box<dyn Any>) -> EntityId {
        let id = EntityId(self.next_id);
        self.next_id += 1;
        self.entities.insert(id, Some(value));
        id
    }

    /// Create a model.
    pub fn new_model<T: 'static>(&mut self, value: T) -> Model<T> {
        Model::from_id(self.alloc(Box::new(value)))
    }

    /// Create a view.
    pub fn new_view<V: Render>(&mut self, value: V) -> View<V> {
        let id = self.alloc(Box::new(value));
        let view = View::from_id(id);
        self.views.insert(id, view.into_any());
        self.dirty_views.insert(id);
        view
    }

    /// Drop an entity. Handles to it become invalid.
    pub fn release(&mut self, id: EntityId) {
        self.entities.remove(&id);
        self.views.remove(&id);
        self.dirty_views.remove(&id);
        self.subscriptions.remove(&id);
        if let Some(observed) = self.deps.remove(&id) {
            for m in observed {
                if let Some(set) = self.observers.get_mut(&m) {
                    set.remove(&id);
                }
            }
        }
        self.observers.remove(&id);
    }

    pub(crate) fn read_entity<T: 'static>(&self, id: EntityId) -> Option<&T> {
        if let Some(reads) = self.tracking.borrow_mut().as_mut() {
            reads.push(id);
        }
        self.entities.get(&id)?.as_ref()?.downcast_ref::<T>()
    }

    /// Take an entity out, run `f`, put it back. `None` if missing or leased.
    pub(crate) fn lease_entity<T: 'static, R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut T, &mut App) -> R,
    ) -> Option<R> {
        let mut boxed = self.entities.get_mut(&id)?.take()?;
        let result = boxed.downcast_mut::<T>().map(|v| f(v, self));
        if let Some(slot) = self.entities.get_mut(&id) {
            *slot = Some(boxed);
        }
        result
    }

    pub(crate) fn update_entity<T: 'static, R>(
        &mut self,
        id: EntityId,
        f: impl FnOnce(&mut T, &mut App) -> R,
    ) -> R {
        self.depth += 1;
        let result = self.lease_entity(id, f);
        self.depth -= 1;
        if self.depth == 0 {
            self.flush_effects();
        }
        match result {
            Some(r) => r,
            None => panic!("entity {} is being updated or was released", id.0),
        }
    }

    /// Mark `id` changed: views that read it (and the view itself) re-render
    /// on the next frame, and observers run once the outermost update ends.
    pub fn notify(&mut self, id: EntityId) {
        self.pending.push(id);
        if self.depth == 0 {
            self.flush_effects();
        }
    }

    pub(crate) fn flush_effects(&mut self) {
        if self.flushing {
            return;
        }
        self.flushing = true;
        while !self.pending.is_empty() {
            let batch = std::mem::take(&mut self.pending);
            for id in batch {
                if self.views.contains_key(&id) {
                    self.dirty_views.insert(id);
                }
                if let Some(obs) = self.observers.get(&id) {
                    self.dirty_views.extend(obs.iter().copied());
                }
                let callbacks: Vec<Callback> = self
                    .subscriptions
                    .get(&id)
                    .map(|s| s.iter().map(|(_, c)| c.clone()).collect())
                    .unwrap_or_default();
                for cb in callbacks {
                    self.depth += 1;
                    cb(self);
                    self.depth -= 1;
                }
            }
        }
        self.flushing = false;
    }

    /// Run `f` whenever `entity` is notified.
    pub fn observe(&mut self, entity: EntityId, f: impl Fn(&mut App) + 'static) -> Subscription {
        let id = self.next_sub;
        self.next_sub += 1;
        self.subscriptions
            .entry(entity)
            .or_default()
            .push((id, Rc::new(f)));
        Subscription { entity, id }
    }

    /// Cancel a subscription.
    pub fn unsubscribe(&mut self, sub: Subscription) {
        if let Some(list) = self.subscriptions.get_mut(&sub.entity) {
            list.retain(|(id, _)| *id != sub.id);
        }
    }

    /// How many times a view has rendered.
    #[must_use]
    pub fn render_count(&self, view: EntityId) -> usize {
        self.render_counts.get(&view).copied().unwrap_or(0)
    }

    pub(crate) fn render_any(&mut self, view: AnyView) -> Option<Element> {
        let previous = self.tracking.replace(Some(Vec::new()));
        let element = (view.render)(self, view.id);
        let reads = self.tracking.replace(previous).unwrap_or_default();
        if let Some(old) = self.deps.remove(&view.id) {
            for m in old {
                if let Some(set) = self.observers.get_mut(&m) {
                    set.remove(&view.id);
                }
            }
        }
        for m in &reads {
            if *m != view.id {
                self.observers.entry(*m).or_default().insert(view.id);
            }
        }
        self.deps.insert(view.id, reads);
        *self.render_counts.entry(view.id).or_default() += 1;
        self.renders_this_frame += 1;
        element
    }

    // ---- configuration -----------------------------------------------

    /// The root view, filling the viewport.
    pub fn set_root<V: Render>(&mut self, view: View<V>) {
        self.root_view = Some(view.into_any());
        self.root_mounted = false;
    }

    /// The viewport size in logical pixels.
    #[must_use]
    pub fn viewport(&self) -> Size {
        self.viewport
    }

    /// Resize the viewport.
    pub fn set_viewport(&mut self, size: Size) {
        if size != self.viewport {
            self.viewport = size;
            self.root_mounted = false;
        }
    }

    /// The theme.
    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The motion configuration.
    #[must_use]
    pub fn motion(&self) -> &MotionConfig {
        &self.motion
    }

    /// The keymap.
    #[must_use]
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Replace the theme; every view re-renders.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
        self.invalidate_all();
    }

    /// Replace the motion configuration. Running animations keep going;
    /// every retarget from now on uses the new springs and speed.
    pub fn set_motion(&mut self, motion: MotionConfig) {
        self.motion = motion;
        self.paint_dirty = true;
    }

    /// Replace the keymap.
    pub fn set_keymap(&mut self, keymap: Keymap) {
        self.keymap = keymap;
    }

    /// Apply a hot-reload event. A `keymap` raw file replaces the default
    /// keymap extended by the file; removal restores the defaults.
    ///
    /// # Errors
    /// The event's error, or a keymap parse error (nothing is changed).
    pub fn apply_config_event(&mut self, event: ConfigEvent) -> Result<(), ConfigError> {
        match event {
            ConfigEvent::Motion(m) => self.set_motion(m),
            ConfigEvent::Theme(t) => self.set_theme(*t),
            ConfigEvent::Raw {
                tag: "keymap",
                contents,
            } => {
                let mut keymap = Keymap::defaults();
                if let Some(text) = contents {
                    keymap.extend(&Keymap::parse(&text)?);
                }
                self.keymap = keymap;
            }
            ConfigEvent::Raw { .. } => {}
            ConfigEvent::Error { error, .. } => return Err(error),
        }
        Ok(())
    }

    fn invalidate_all(&mut self) {
        let mounted: Vec<EntityId> = self.view_slots.keys().copied().collect();
        self.dirty_views.extend(mounted);
        let ids: Vec<ElementId> = self.tree.nodes.keys().copied().collect();
        self.tree.visual_dirty.extend(ids);
        self.paint_dirty = true;
    }

    // ---- frames --------------------------------------------------------

    /// Ask for a frame even if nothing is dirty (custom animations).
    pub fn request_frame(&mut self) {
        self.frame_requested = true;
    }

    /// Seconds since the previous frame (as given to [`App::draw_frame`]).
    #[must_use]
    pub fn frame_delta(&self) -> f32 {
        self.frame_dt
    }

    /// Frames drawn so far.
    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Whether a frame should be drawn: something is dirty or moving.
    /// When this is false the host draws nothing (idle = zero frames).
    #[must_use]
    pub fn needs_frame(&self) -> bool {
        (self.root_view.is_some() && !self.root_mounted)
            || !self.dirty_views.is_empty()
            || !self.pending.is_empty()
            || !self.tree.visual_dirty.is_empty()
            || !self.driver.active.is_empty()
            || self.paint_dirty
            || self.frame_requested
            || self
                .tree
                .root
                .and_then(|r| self.tree.nodes.get(&r))
                .is_some_and(|n| self.tree.layout.is_dirty(n.layout).unwrap_or(false))
    }

    /// Run one frame: re-render dirty views and diff them into the retained
    /// tree, lay out dirty subtrees, retarget animations, advance them by
    /// `dt` seconds and paint.
    pub fn draw_frame(&mut self, dt: f32) -> Frame {
        self.flush_effects();
        self.frame_dt = dt;
        self.frame_requested = false;
        self.renders_this_frame = 0;
        self.frame_count += 1;

        let mut tree = std::mem::take(&mut self.tree);
        if !self.root_mounted
            && let Some(root) = self.root_view
        {
            let window = div()
                .name("window")
                .size(self.viewport.width, self.viewport.height)
                .child(Element::from_any(root));
            reconcile(&mut tree, self, ElementId::ROOT, None, window, root.id);
            self.root_mounted = true;
        }
        self.render_dirty_views(&mut tree);

        let mut layout = false;
        if let Some(root) = tree.root.and_then(|r| tree.nodes.get(&r)) {
            let node = root.layout;
            if tree.layout.is_dirty(node).unwrap_or(true) {
                let _ = tree.layout.compute(node, self.viewport);
                update_bounds(&mut tree);
                layout = true;
            }
        }
        self.tree = tree;
        if layout {
            self.refresh_hover();
        }

        self.resolve_visuals();
        self.tick_animations(dt);

        let (scene, paint_stats) = paint(&PaintInput {
            tree: &self.tree,
            theme: &self.theme,
            focus: self.focus,
            focus_visible: self.focus_visible,
        });
        self.paint_dirty = false;
        Frame {
            scene,
            needs_next_frame: self.needs_frame(),
            stats: FrameStats {
                views_rendered: self.renders_this_frame,
                layout,
                nodes: self.tree.nodes.len(),
                animating: self.driver.active.len(),
                paint: paint_stats,
            },
        }
    }

    fn render_dirty_views(&mut self, tree: &mut Tree) {
        let depth_of = |tree: &Tree, mut id: ElementId| {
            let mut d = 0usize;
            while let Some(p) = tree.nodes.get(&id).and_then(|n| n.parent) {
                d += 1;
                id = p;
            }
            d
        };
        let mut order: Vec<(usize, EntityId, ElementId)> = Vec::new();
        for vid in std::mem::take(&mut self.dirty_views) {
            match self.view_slots.get(&vid) {
                Some(slot) if tree.nodes.get(slot).is_some_and(|n| n.view == Some(vid)) => {
                    order.push((depth_of(tree, *slot), vid, *slot));
                }
                _ => {}
            }
        }
        order.sort_unstable();
        self.dirty_views = order.iter().map(|(_, v, _)| *v).collect();
        for (_, vid, slot) in order {
            if !self.dirty_views.contains(&vid) {
                continue; // rendered while reconciling an ancestor
            }
            let Some(any) = self.views.get(&vid).copied() else {
                self.dirty_views.remove(&vid);
                continue;
            };
            let (parent, owner) = match tree.nodes.get(&slot) {
                Some(n) => (n.parent, n.owner),
                None => continue,
            };
            reconcile(tree, self, slot, parent, Element::from_any(any), owner);
        }
        // Views rendered during this frame that were notified again stay
        // dirty for the next frame; unmounted views are dropped.
        let mounted: BTreeSet<EntityId> = self
            .dirty_views
            .iter()
            .copied()
            .filter(|v| self.view_slots.contains_key(v))
            .collect();
        self.dirty_views = mounted;
    }

    pub(crate) fn forget_element(&mut self, id: ElementId, view: Option<EntityId>) {
        self.element_state.remove(&id);
        if self.focus == Some(id) {
            self.focus = None;
        }
        self.input.hovered.retain(|e| *e != id);
        self.input.pressed.retain(|e| *e != id);
        if self.input.drag.is_some_and(|d| d.target == id) {
            self.input.drag = None;
        }
        self.driver.active.retain(|e| *e != id);
        for slot in &mut self.driver.large {
            if slot.is_some_and(|(e, _)| e == id) {
                *slot = None;
            }
        }
        if let Some(v) = view
            && self.view_slots.get(&v) == Some(&id)
        {
            self.view_slots.remove(&v);
        }
    }

    fn resolve_visuals(&mut self) {
        let dirty = std::mem::take(&mut self.tree.visual_dirty);
        for id in dirty {
            let focus_visible = self.focus == Some(id) && self.focus_visible;
            let Some(node) = self.tree.nodes.get_mut(&id) else {
                continue;
            };
            let targets = Targets::resolve(&node.visual, node.hovered, node.pressed, focus_visible);
            let started: Started = node.anims.apply(targets, &node.visual, &self.motion);
            if started.any {
                self.activate(id, started.large);
            }
        }
    }

    fn activate(&mut self, id: ElementId, large: bool) {
        if !self.driver.active.contains(&id) {
            self.driver.active.push(id);
        }
        if !large || self.driver.large.iter().flatten().any(|(e, _)| *e == id) {
            return;
        }
        self.driver.seq += 1;
        let seq = self.driver.seq;
        if let Some(free) = self.driver.large.iter_mut().find(|s| s.is_none()) {
            *free = Some((id, seq));
            return;
        }
        // Budget: at most two large animations. Finish the oldest.
        let oldest = usize::from(
            self.driver.large[1].map_or(0, |(_, s)| s) < self.driver.large[0].map_or(0, |(_, s)| s),
        );
        if let Some((old, _)) = self.driver.large[oldest] {
            if let Some(n) = self.tree.nodes.get_mut(&old) {
                n.anims.finish();
            }
            self.driver.evicted += 1;
        }
        self.driver.large[oldest] = Some((id, seq));
    }

    fn tick_animations(&mut self, dt: f32) {
        let nodes = &mut self.tree.nodes;
        self.driver
            .active
            .retain(|id| nodes.get_mut(id).is_some_and(|n| n.anims.step(dt)));
        for slot in &mut self.driver.large {
            if let Some((id, _)) = *slot
                && !self.driver.active.contains(&id)
            {
                *slot = None;
            }
        }
    }

    /// Animation counters.
    #[must_use]
    pub fn animation_stats(&self) -> AnimationStats {
        AnimationStats {
            active: self.driver.active.len(),
            capacity: self.driver.active.capacity(),
            large: self.driver.large.iter().flatten().count(),
            large_evicted: self.driver.evicted,
        }
    }

    // ---- queries -------------------------------------------------------

    /// Absolute layout bounds of an element (before transforms).
    #[must_use]
    pub fn bounds(&self, id: ElementId) -> Option<Rect> {
        self.tree.nodes.get(&id).map(|n| n.bounds)
    }

    /// The first element (depth-first) whose name or key is `name`.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<ElementId> {
        let mut stack: Vec<ElementId> = self.tree.root.into_iter().collect();
        while let Some(id) = stack.pop() {
            let node = self.tree.nodes.get(&id)?;
            if &*node.label == name || node.key.as_deref() == Some(name) {
                return Some(id);
            }
            stack.extend(node.children.iter().rev());
        }
        None
    }

    /// Children of an element.
    #[must_use]
    pub fn children_of(&self, id: ElementId) -> Vec<ElementId> {
        self.tree
            .nodes
            .get(&id)
            .map(|n| n.children.clone())
            .unwrap_or_default()
    }

    /// The debug label (name, key or kind) of an element.
    #[must_use]
    pub fn label(&self, id: ElementId) -> Option<String> {
        self.tree.nodes.get(&id).map(|n| n.label.to_string())
    }

    /// Current (animated) translation of an element.
    #[must_use]
    pub fn translation(&self, id: ElementId) -> Option<Vec2> {
        self.tree.nodes.get(&id).map(|n| n.anims.translate.value())
    }

    /// Current (animated) scale of an element.
    #[must_use]
    pub fn scale_of(&self, id: ElementId) -> Option<f32> {
        self.tree.nodes.get(&id).map(|n| n.anims.scale.value())
    }

    /// Current (animated) background of an element.
    #[must_use]
    pub fn background(&self, id: ElementId) -> Option<livspeil_ui_theme::Color> {
        self.tree.nodes.get(&id).map(|n| n.anims.bg.value())
    }

    /// The topmost element under `point`.
    #[must_use]
    pub fn hit_test(&self, point: Point) -> Option<ElementId> {
        hit_path(&self.tree, point).last().copied()
    }

    /// Whether the pointer is over the element (or a descendant).
    #[must_use]
    pub fn is_hovered(&self, id: ElementId) -> bool {
        self.tree.nodes.get(&id).is_some_and(|n| n.hovered)
    }

    /// Whether the element is pressed.
    #[must_use]
    pub fn is_pressed(&self, id: ElementId) -> bool {
        self.tree.nodes.get(&id).is_some_and(|n| n.pressed)
    }

    /// The focused element.
    #[must_use]
    pub fn focused(&self) -> Option<ElementId> {
        self.focus
    }

    /// Whether the focus ring is shown (focus came from the keyboard).
    #[must_use]
    pub fn focus_visible(&self) -> bool {
        self.focus_visible
    }

    /// Text dump of the retained tree with absolute bounds.
    #[must_use]
    pub fn dump_tree(&self) -> String {
        dump(&self.tree)
    }

    /// Dump a scene with element labels.
    #[must_use]
    pub fn dump_scene(&self, scene: &Scene) -> String {
        scene.dump(|id| self.label(id).unwrap_or_else(|| format!("{id:?}")))
    }

    /// Layout counters.
    #[must_use]
    pub fn layout_stats(&self) -> livspeil_ui_layout::LayoutStats {
        self.tree.layout.stats()
    }

    // ---- element state -------------------------------------------------

    /// State of type `S` stored for element `id` (dropped when the element
    /// leaves the tree).
    #[must_use]
    pub fn element_state<S: 'static>(&self, id: ElementId) -> Option<&S> {
        self.element_state
            .get(&id)?
            .get(&TypeId::of::<S>())?
            .downcast_ref()
    }

    /// State of type `S` for element `id`, created with `Default` if absent.
    pub fn element_state_mut<S: Default + 'static>(&mut self, id: ElementId) -> &mut S {
        let slot = self
            .element_state
            .entry(id)
            .or_default()
            .entry(TypeId::of::<S>())
            .or_insert_with(|| Box::new(S::default()));
        if !slot.is::<S>() {
            *slot = Box::new(S::default());
        }
        match slot.downcast_mut::<S>() {
            Some(s) => s,
            None => unreachable!("slot type checked above"),
        }
    }

    // ---- focus ---------------------------------------------------------

    /// Focus an element (no focus ring, as from a pointer).
    pub fn focus(&mut self, id: ElementId) {
        self.set_focus(Some(id), false);
    }

    /// Clear focus.
    pub fn blur(&mut self) {
        self.set_focus(None, false);
    }

    pub(crate) fn set_focus(&mut self, id: Option<ElementId>, visible: bool) {
        if self.focus == id && self.focus_visible == visible {
            return;
        }
        for e in self.focus.into_iter().chain(id) {
            self.tree.visual_dirty.push(e);
        }
        self.focus = id;
        self.focus_visible = visible && id.is_some();
        self.paint_dirty = true;
    }

    /// Tab order of the active focus scope: positive tab indices ascending,
    /// then index 0 in tree order. If focus is inside a focus scope,
    /// traversal stays in it.
    #[must_use]
    pub fn tab_order(&self) -> Vec<ElementId> {
        let mut scope = self.tree.root;
        let mut cur = self.focus;
        while let Some(id) = cur {
            let Some(n) = self.tree.nodes.get(&id) else {
                break;
            };
            if n.focus_scope {
                scope = Some(id);
                break;
            }
            cur = n.parent;
        }
        let mut order: Vec<(i32, usize, ElementId)> = Vec::new();
        let mut stack: Vec<ElementId> = scope.into_iter().collect();
        let mut seq = 0usize;
        while let Some(id) = stack.pop() {
            let Some(n) = self.tree.nodes.get(&id) else {
                continue;
            };
            if let Some(ti) = n.tab_index.filter(|t| *t >= 0) {
                order.push((if ti == 0 { i32::MAX } else { ti }, seq, id));
                seq += 1;
            }
            stack.extend(n.children.iter().rev());
        }
        order.sort_unstable();
        order.into_iter().map(|(_, _, id)| id).collect()
    }

    /// Move focus forward in tab order (shows the focus ring).
    pub fn focus_next(&mut self) {
        self.traverse(1);
    }

    /// Move focus backward in tab order (shows the focus ring).
    pub fn focus_prev(&mut self) {
        self.traverse(-1);
    }

    fn traverse(&mut self, dir: isize) {
        let order = self.tab_order();
        if order.is_empty() {
            return;
        }
        let len = order.len().cast_signed();
        let next = match self.focus.and_then(|f| order.iter().position(|e| *e == f)) {
            Some(i) => (i.cast_signed() + dir).rem_euclid(len),
            None if dir > 0 => 0,
            None => len - 1,
        };
        self.set_focus(Some(order[next.cast_unsigned()]), true);
    }

    fn focus_path(&self) -> Vec<ElementId> {
        let mut path = Vec::new();
        let mut cur = self.focus.or(self.tree.root);
        while let Some(id) = cur {
            path.push(id);
            cur = self.tree.nodes.get(&id).and_then(|n| n.parent);
        }
        path.reverse();
        path
    }

    // ---- input ---------------------------------------------------------

    fn set_flag(&mut self, ids: &[ElementId], on: bool, hover: bool) {
        for id in ids {
            if let Some(n) = self.tree.nodes.get_mut(id) {
                let refinement = if hover {
                    n.hovered = on;
                    n.visual.hover.is_some()
                } else {
                    n.pressed = on;
                    n.visual.active.is_some()
                };
                if refinement {
                    self.tree.visual_dirty.push(*id);
                    self.paint_dirty = true;
                }
            }
        }
    }

    fn set_hovered(&mut self, path: Vec<ElementId>) {
        let old = std::mem::take(&mut self.input.hovered);
        let gone: Vec<ElementId> = old.iter().copied().filter(|e| !path.contains(e)).collect();
        let new: Vec<ElementId> = path.iter().copied().filter(|e| !old.contains(e)).collect();
        self.set_flag(&gone, false, true);
        self.set_flag(&new, true, true);
        self.input.hovered = path;
    }

    fn refresh_hover(&mut self) {
        if let Some(p) = self.input.pointer {
            let path = hit_path(&self.tree, p);
            self.set_hovered(path);
        }
    }

    /// Route an input event.
    pub fn dispatch(&mut self, event: InputEvent) {
        match event {
            InputEvent::Pointer(e) => self.dispatch_pointer(e),
            InputEvent::PointerLeft => {
                self.input.pointer = None;
                self.set_hovered(Vec::new());
            }
            InputEvent::Key(e) => self.dispatch_key(e),
            InputEvent::Text(t) => self.dispatch_text(&t),
            InputEvent::Ime(e) => self.dispatch_ime(&e),
        }
        self.flush_effects();
    }

    fn dispatch_pointer(&mut self, ev: PointerEvent) {
        let pos = ev.position;
        self.input.pointer = Some(pos);
        self.input.modifiers = ev.modifiers;
        match ev.kind {
            PointerKind::Move => {
                let hit = hit_path(&self.tree, pos);
                self.set_hovered(hit.clone());
                if let Some(mut d) = self.input.drag {
                    self.input.tracker.push(ev.time, pos);
                    let phase = if d.active {
                        Some(DragPhase::Move)
                    } else if (pos - d.start).length() >= DRAG_THRESHOLD {
                        d.active = true;
                        Some(DragPhase::Start)
                    } else {
                        None
                    };
                    let delta = pos - d.last;
                    d.last = pos;
                    self.input.drag = Some(d);
                    if let Some(phase) = phase {
                        self.fire_drag(d, phase, pos, delta);
                    }
                }
                let path = self.input.capture.clone().unwrap_or(hit);
                self.dispatch_pointer_path(&path, PointerFilter::Move, &ev);
            }
            PointerKind::Down(_) => {
                let path = hit_path(&self.tree, pos);
                let focusable = path
                    .iter()
                    .rev()
                    .find(|id| {
                        self.tree
                            .nodes
                            .get(id)
                            .is_some_and(|n| n.tab_index.is_some())
                    })
                    .copied();
                self.set_focus(focusable, false);
                self.set_flag(&path, true, false);
                self.input.pressed.clone_from(&path);
                self.input.capture = Some(path.clone());
                self.input.drag = path
                    .iter()
                    .rev()
                    .find(|id| {
                        self.tree
                            .nodes
                            .get(id)
                            .and_then(|n| n.handlers.as_ref())
                            .is_some_and(|h| h.drag.is_some())
                    })
                    .map(|&target| DragState {
                        target,
                        start: pos,
                        last: pos,
                        active: false,
                    });
                self.input.tracker.reset();
                self.input.tracker.push(ev.time, pos);
                self.dispatch_pointer_path(&path, PointerFilter::Down, &ev);
            }
            PointerKind::Up(_) => {
                let path = self
                    .input
                    .capture
                    .take()
                    .unwrap_or_else(|| hit_path(&self.tree, pos));
                self.dispatch_pointer_path(&path, PointerFilter::Up, &ev);
                let drag = self.input.drag.take();
                if let Some(d) = drag.filter(|d| d.active) {
                    self.input.tracker.push(ev.time, pos);
                    let delta = pos - d.last;
                    self.fire_drag(d, DragPhase::End, pos, delta);
                } else {
                    let now = hit_path(&self.tree, pos);
                    let target = path.iter().rev().find(|id| {
                        now.contains(id)
                            && self
                                .tree
                                .nodes
                                .get(id)
                                .and_then(|n| n.handlers.as_ref())
                                .is_some_and(|h| h.click.is_some())
                    });
                    if let Some(&target) = target {
                        self.fire_click(target, pos, ev.modifiers, false);
                    }
                }
                let pressed = std::mem::take(&mut self.input.pressed);
                self.set_flag(&pressed, false, false);
                let hit = hit_path(&self.tree, pos);
                self.set_hovered(hit);
            }
            PointerKind::Scroll(_) => {
                let path = hit_path(&self.tree, pos);
                self.dispatch_pointer_path(&path, PointerFilter::Scroll, &ev);
            }
        }
    }

    pub(crate) fn node_handlers(
        &self,
        id: ElementId,
    ) -> Option<(Rect, Rc<crate::element::Handlers>)> {
        let n = self.tree.nodes.get(&id)?;
        Some((n.bounds, n.handlers.clone()?))
    }

    fn dispatch_pointer_path(
        &mut self,
        path: &[ElementId],
        filter: PointerFilter,
        ev: &PointerEvent,
    ) {
        let Some(&target) = path.last() else {
            return;
        };
        let mut calls = Vec::new();
        for phase in [Phase::Capture, Phase::Bubble] {
            let ids: Vec<ElementId> = if phase == Phase::Capture {
                path.to_vec()
            } else {
                path.iter().rev().copied().collect()
            };
            for id in ids {
                if let Some((bounds, h)) = self.node_handlers(id) {
                    for (f, p, handler) in &h.pointer {
                        if *f == filter && *p == phase {
                            calls.push((id, bounds, phase, handler.clone()));
                        }
                    }
                }
            }
        }
        for (id, bounds, phase, handler) in calls {
            let mut cx = EventCx::new(self, target, id, bounds, phase);
            handler(ev, &mut cx);
            if cx.stopped {
                break;
            }
        }
    }

    fn fire_drag(&mut self, d: DragState, phase: DragPhase, pos: Point, delta: Vec2) {
        let Some((bounds, h)) = self.node_handlers(d.target) else {
            return;
        };
        let Some(handler) = h.drag.clone() else {
            return;
        };
        let event = DragEvent {
            phase,
            start: d.start,
            position: pos,
            delta,
            velocity: self.input.tracker.velocity(),
        };
        let mut cx = EventCx::new(self, d.target, d.target, bounds, Phase::Bubble);
        handler(&event, &mut cx);
    }

    pub(crate) fn fire_click(
        &mut self,
        id: ElementId,
        pos: Point,
        modifiers: Modifiers,
        keyboard: bool,
    ) -> bool {
        let Some((bounds, h)) = self.node_handlers(id) else {
            return false;
        };
        let Some(handler): Option<ClickHandler> = h.click.clone() else {
            return false;
        };
        let event = ClickEvent {
            position: pos,
            modifiers,
            keyboard,
        };
        let mut cx = EventCx::new(self, id, id, bounds, Phase::Bubble);
        handler(&event, &mut cx);
        true
    }

    fn dispatch_key(&mut self, ev: KeyEvent) {
        let path = self.focus_path();
        let Some(&target) = path.last() else {
            return;
        };
        let mut calls = Vec::new();
        for phase in [Phase::Capture, Phase::Bubble] {
            let ids: Vec<ElementId> = if phase == Phase::Capture {
                path.clone()
            } else {
                path.iter().rev().copied().collect()
            };
            for id in ids {
                if let Some((bounds, h)) = self.node_handlers(id) {
                    for (p, handler) in &h.key {
                        if *p == phase {
                            calls.push((id, bounds, phase, handler.clone()));
                        }
                    }
                }
            }
        }
        for (id, bounds, phase, handler) in calls {
            let mut cx = EventCx::new(self, target, id, bounds, phase);
            handler(&ev, &mut cx);
            if cx.stopped {
                return;
            }
        }
        if !ev.pressed {
            return;
        }
        let contexts: Vec<Rc<str>> = path
            .iter()
            .rev()
            .filter_map(|id| self.tree.nodes.get(id).and_then(|n| n.key_context.clone()))
            .collect();
        if let Some(action) = self.keymap.action_for(&ev.keystroke(), &contexts) {
            self.dispatch_action(&action);
        }
    }

    /// Dispatch a named action from the focused element up to the root.
    /// The first element with a handler for it handles it. Unhandled
    /// built-ins: `focus.next`, `focus.prev`, and `ui.activate` (clicks the
    /// focused element). Returns whether anything handled it.
    pub fn dispatch_action(&mut self, action: &str) -> bool {
        let path = self.focus_path();
        let Some(&target) = path.last() else {
            return false;
        };
        for id in path.iter().rev() {
            let Some((bounds, h)) = self.node_handlers(*id) else {
                continue;
            };
            if let Some((_, handler)) = h.actions.iter().find(|(name, _)| &**name == action) {
                let handler = handler.clone();
                let mut cx = EventCx::new(self, target, *id, bounds, Phase::Bubble);
                handler(&mut cx);
                self.flush_effects();
                return true;
            }
        }
        let handled = match action {
            actions::FOCUS_NEXT => {
                self.focus_next();
                true
            }
            actions::FOCUS_PREV => {
                self.focus_prev();
                true
            }
            actions::ACTIVATE => match self.focus {
                Some(f) => {
                    let center = self.bounds(f).map_or(Point::ZERO, |b| b.center());
                    self.fire_click(f, center, Modifiers::NONE, true)
                }
                None => false,
            },
            _ => false,
        };
        self.flush_effects();
        handled
    }

    fn dispatch_text(&mut self, text: &str) {
        let path = self.focus_path();
        let Some(&target) = path.last() else {
            return;
        };
        for id in path.iter().rev() {
            if let Some((bounds, h)) = self.node_handlers(*id)
                && let Some(handler) = h.text.clone()
            {
                let mut cx = EventCx::new(self, target, *id, bounds, Phase::Bubble);
                handler(text, &mut cx);
                return;
            }
        }
    }

    fn dispatch_ime(&mut self, ev: &ImeEvent) {
        let path = self.focus_path();
        let Some(&target) = path.last() else {
            return;
        };
        for id in path.iter().rev() {
            if let Some((bounds, h)) = self.node_handlers(*id)
                && let Some(handler) = h.ime.clone()
            {
                let mut cx = EventCx::new(self, target, *id, bounds, Phase::Bubble);
                handler(ev, &mut cx);
                return;
            }
        }
    }

    /// Hand a velocity (px/s) to an element's translation spring, for
    /// example a drag's release velocity. It is used by the next retarget.
    pub fn set_translate_velocity(&mut self, id: ElementId, velocity: Vec2) {
        if let Some(n) = self.tree.nodes.get_mut(&id) {
            n.anims.translate.set_velocity(velocity);
            self.tree.visual_dirty.push(id);
        }
    }

    /// Pointer position, if known.
    #[must_use]
    pub fn pointer_position(&self) -> Option<Point> {
        self.input.pointer
    }
}

impl Element {
    pub(crate) fn from_any(view: AnyView) -> Element {
        let mut e = div();
        e.kind = ElementKind::View(view);
        e
    }
}

/// Any input event.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// Pointer.
    Pointer(PointerEvent),
    /// The pointer left the window.
    PointerLeft,
    /// Key.
    Key(KeyEvent),
    /// Committed text input.
    Text(String),
    /// IME composition.
    Ime(ImeEvent),
}

/// Context passed to event handlers; derefs to [`App`].
pub struct EventCx<'a> {
    app: &'a mut App,
    target: ElementId,
    current: ElementId,
    bounds: Rect,
    phase: Phase,
    stopped: bool,
}

impl fmt::Debug for EventCx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventCx")
            .field("target", &self.target)
            .field("current", &self.current)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl<'a> EventCx<'a> {
    pub(crate) fn new(
        app: &'a mut App,
        target: ElementId,
        current: ElementId,
        bounds: Rect,
        phase: Phase,
    ) -> Self {
        EventCx {
            app,
            target,
            current,
            bounds,
            phase,
            stopped: false,
        }
    }

    /// The element the event is for (deepest in the path).
    #[must_use]
    pub fn target(&self) -> ElementId {
        self.target
    }

    /// The element whose handler is running.
    #[must_use]
    pub fn element_id(&self) -> ElementId {
        self.current
    }

    /// Bounds of the current element.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Stop the event from reaching further handlers.
    pub fn stop_propagation(&mut self) {
        self.stopped = true;
    }

    /// Per-element state of the current element.
    pub fn state<S: Default + 'static>(&mut self) -> &mut S {
        let id = self.current;
        self.app.element_state_mut::<S>(id)
    }
}

impl Deref for EventCx<'_> {
    type Target = App;
    fn deref(&self) -> &App {
        self.app
    }
}

impl DerefMut for EventCx<'_> {
    fn deref_mut(&mut self) -> &mut App {
        self.app
    }
}

/// Convenience: a pointer event at time `t`.
#[must_use]
pub fn pointer(kind: PointerKind, x: f32, y: f32, t: Duration) -> PointerEvent {
    PointerEvent {
        kind,
        position: Point::new(x, y),
        modifiers: Modifiers::NONE,
        time: t,
    }
}
