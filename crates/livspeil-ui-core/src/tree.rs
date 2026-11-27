// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The retained element tree and the diff (reconcile) from elements.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::rc::Rc;
use std::time::Duration;

use livspeil_ui_layout::geometry::num;
use livspeil_ui_layout::{LayoutNode, LayoutTree, Measure, Point, Rect, Transform2D, Vec2};
use livspeil_ui_theme::{Color, MotionClass, MotionConfig, MotionSpec, PropertyKind};

use crate::anim::{Animatable, Animated};
use crate::app::App;
use crate::element::{
    Element, ElementId, ElementKind, Handlers, IconSpec, Refinement, SharedString, TextSpec,
    Transition, Visual,
};
use crate::entity::EntityId;
use crate::layer::{Layer, Layers};

/// Animated visual state of one node. Inline, `Copy`: never allocates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NodeAnims {
    pub(crate) bg: Animated<Color>,
    pub(crate) border: Animated<Color>,
    pub(crate) opacity: Animated<f32>,
    pub(crate) translate: Animated<Vec2>,
    pub(crate) scale: Animated<f32>,
}

/// Resolved targets of the animated properties.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Targets {
    bg: Color,
    border: Color,
    opacity: f32,
    translate: Vec2,
    scale: f32,
}

impl Targets {
    pub(crate) fn resolve(
        v: &Visual,
        hovered: bool,
        pressed: bool,
        focus_visible: bool,
    ) -> Targets {
        let refinements = [
            (focus_visible, v.focused.as_ref()),
            (hovered, v.hover.as_ref()),
            (pressed, v.active.as_ref()),
        ];
        // A color that only appears in a refinement fades in from its own
        // transparent version, not from transparent black.
        let any_bg = refinements
            .iter()
            .find_map(|(_, r)| r.and_then(|r| r.background));
        let any_border = refinements
            .iter()
            .find_map(|(_, r)| r.and_then(|r| r.border_color));
        let mut t = Targets {
            bg: v
                .background
                .or_else(|| any_bg.map(|c| c.with_alpha(0.0)))
                .unwrap_or(Color::TRANSPARENT),
            border: v
                .border
                .map(|b| b.1)
                .or_else(|| any_border.map(|c| c.with_alpha(0.0)))
                .unwrap_or(Color::TRANSPARENT),
            opacity: v.opacity,
            translate: v.translate,
            scale: v.scale,
        };
        for (on, r) in refinements {
            if let (true, Some(r)) = (on, r) {
                t.refine(r);
            }
        }
        t
    }

    fn refine(&mut self, r: &Refinement) {
        if let Some(c) = r.background {
            self.bg = c;
        }
        if let Some(c) = r.border_color {
            self.border = c;
        }
        if let Some(o) = r.opacity {
            self.opacity = o;
        }
        if let Some(t) = r.translate {
            self.translate = t;
        }
        if let Some(s) = r.scale {
            self.scale = s;
        }
    }
}

/// What a retarget did.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Started {
    pub(crate) any: bool,
    pub(crate) large: bool,
}

fn drive<T: Animatable>(
    anim: &mut Animated<T>,
    target: T,
    transition: Option<Transition>,
    kind: PropertyKind,
    scope: Option<&str>,
    motion: &MotionConfig,
) -> bool {
    let pending_velocity = !anim.is_animating() && anim.has_velocity();
    if target == anim.target() && !pending_velocity {
        return anim.is_animating();
    }
    match transition {
        None => {
            anim.set(target);
            false
        }
        Some(Transition::Motion(class)) => {
            anim.animate_to(target, motion.resolve(scope, class, kind))
        }
        Some(Transition::Tween(duration, easing)) => {
            // Tweens obey speed and Reduce Motion too (via the micro class).
            match motion.resolve(scope, MotionClass::Micro, kind) {
                MotionSpec::Instant => {
                    anim.set(target);
                    false
                }
                MotionSpec::Spring(_) => {
                    let speed = motion.speed(scope).max(1e-3);
                    anim.tween_to(
                        target,
                        Duration::from_secs_f32(duration.as_secs_f32() / speed),
                        easing,
                    )
                }
            }
        }
    }
}

impl NodeAnims {
    pub(crate) fn at(t: Targets) -> Self {
        NodeAnims {
            bg: Animated::new(t.bg),
            border: Animated::new(t.border),
            opacity: Animated::new(t.opacity),
            translate: Animated::new(t.translate),
            scale: Animated::new(t.scale),
        }
    }

    /// Retarget every property; returns whether anything animates.
    pub(crate) fn apply(&mut self, t: Targets, v: &Visual, motion: &MotionConfig) -> Started {
        let scope = v.motion_scope.as_deref();
        let tr = v.transition;
        let a = drive(
            &mut self.bg,
            t.bg,
            tr,
            PropertyKind::Appearance,
            scope,
            motion,
        );
        let b = drive(
            &mut self.border,
            t.border,
            tr,
            PropertyKind::Appearance,
            scope,
            motion,
        );
        let c = drive(
            &mut self.opacity,
            t.opacity,
            tr,
            PropertyKind::Appearance,
            scope,
            motion,
        );
        let d = drive(
            &mut self.translate,
            t.translate,
            tr,
            PropertyKind::Movement,
            scope,
            motion,
        );
        let e = drive(
            &mut self.scale,
            t.scale,
            tr,
            PropertyKind::Movement,
            scope,
            motion,
        );
        let any = a || b || c || d || e;
        Started {
            any,
            large: any && tr == Some(Transition::Motion(MotionClass::Large)),
        }
    }

    /// The animated values that change where things are.
    pub(crate) fn placement(&self) -> (Vec2, f32) {
        (self.translate.value(), self.scale.value())
    }

    pub(crate) fn step(&mut self, dt: f32) -> bool {
        let a = self.bg.step(dt);
        let b = self.border.step(dt);
        let c = self.opacity.step(dt);
        let d = self.translate.step(dt);
        let e = self.scale.step(dt);
        a || b || c || d || e
    }

    pub(crate) fn finish(&mut self) {
        self.bg.finish();
        self.border.finish();
        self.opacity.finish();
        self.translate.finish();
        self.scale.finish();
    }
}

/// A retained node.
#[derive(Debug)]
pub(crate) struct Node {
    pub(crate) parent: Option<ElementId>,
    pub(crate) children: Vec<ElementId>,
    pub(crate) label: SharedString,
    pub(crate) key: Option<SharedString>,
    pub(crate) text: Option<TextSpec>,
    pub(crate) icon: Option<IconSpec>,
    pub(crate) view: Option<EntityId>,
    pub(crate) owner: EntityId,
    pub(crate) layout: LayoutNode,
    pub(crate) visual: Visual,
    pub(crate) handlers: Option<Rc<Handlers>>,
    pub(crate) tab_index: Option<i32>,
    pub(crate) focus_scope: bool,
    pub(crate) key_context: Option<SharedString>,
    /// Compositor properties applied to this element and its subtree.
    pub(crate) layer: Option<Layer>,
    /// Children outside the visible area are neither painted nor hit.
    pub(crate) cull: bool,
    pub(crate) hit_test: bool,
    pub(crate) semantics: Option<Box<crate::semantics::Semantics>>,
    pub(crate) wires: Vec<crate::wire::Wire>,
    /// Absolute untransformed layout bounds.
    pub(crate) bounds: Rect,
    pub(crate) anims: NodeAnims,
    pub(crate) hovered: bool,
    pub(crate) pressed: bool,
}

impl Node {
    /// The layer's transform about the layout origin: where the element
    /// *is* (scrolled, panned), as opposed to how it is drawn (press scale).
    pub(crate) fn layer_transform(&self, layers: &Layers) -> Transform2D {
        let t = layers.props(self.layer).transform;
        if t.is_identity() {
            t
        } else {
            t.about(self.bounds.origin())
        }
    }

    /// Transform from this node's (and its subtree's) layout space to its
    /// parent's: the layer, then scale and static transform about the
    /// center, then translate.
    pub(crate) fn transform(&self, layers: &Layers) -> Transform2D {
        let s = self.anims.scale.value();
        let t = self.anims.translate.value();
        let local = Transform2D::scale(s, s).then(self.visual.transform);
        self.layer_transform(layers)
            .then(local.about(self.bounds.center()))
            .then(Transform2D::translate(t))
    }

    pub(crate) fn opacity(&self, layers: &Layers) -> f32 {
        self.anims.opacity.value() * layers.props(self.layer).opacity.clamp(0.0, 1.0)
    }

    /// The visible area inside this node (its layout space), given the
    /// visible area in its parent's.
    pub(crate) fn clip_inside(&self, transform: Transform2D, visible: Rect) -> Rect {
        let local = match transform.inverse() {
            Some(inv) if !transform.is_identity() => inv.apply_rect(&visible),
            _ => visible,
        };
        if self.visual.clip {
            local.intersection(&self.bounds).unwrap_or(Rect::ZERO)
        } else {
            local
        }
    }

    /// Whether `child` is left out because it is outside `visible` (this
    /// node's layout space).
    pub(crate) fn culls(&self, layers: &Layers, child: &Node, visible: Rect) -> bool {
        self.cull
            && !child
                .transform(layers)
                .apply_rect(&child.bounds)
                .intersects(&visible)
    }
}

/// The retained tree.
#[derive(Debug, Default)]
pub(crate) struct Tree {
    pub(crate) nodes: HashMap<ElementId, Node>,
    pub(crate) root: Option<ElementId>,
    pub(crate) layout: LayoutTree,
    /// Nodes whose visual targets must be resolved again.
    pub(crate) visual_dirty: Vec<ElementId>,
    /// The elements each layer is attached to.
    layered: HashMap<Layer, Vec<ElementId>>,
    by_layout: HashMap<LayoutNode, ElementId>,
    /// Elements whose layer changed: their bounds must be placed again.
    pub(crate) relayered: Vec<ElementId>,
}

impl Tree {
    /// The elements `layer` is attached to.
    pub(crate) fn layered(&self, layer: Layer) -> &[ElementId] {
        self.layered.get(&layer).map_or(&[], Vec::as_slice)
    }

    /// The element of a layout node.
    pub(crate) fn element_of(&self, node: LayoutNode) -> Option<ElementId> {
        self.by_layout.get(&node).copied()
    }

    fn attach(&mut self, id: ElementId, old: Option<Layer>, new: Option<Layer>) {
        if old == new {
            return;
        }
        if let Some(old) = old
            && let Some(list) = self.layered.get_mut(&old)
        {
            list.retain(|e| *e != id);
            if list.is_empty() {
                self.layered.remove(&old);
            }
        }
        if let Some(new) = new {
            self.layered.entry(new).or_default().push(id);
        }
        self.relayered.push(id);
    }

    /// Recompute the absolute bounds of `id` and its subtree from the
    /// layout and layer frames.
    pub(crate) fn update_bounds_from(&mut self, layers: &Layers, id: ElementId) {
        let origin = self
            .nodes
            .get(&id)
            .and_then(|n| n.parent)
            .and_then(|p| self.nodes.get(&p))
            .map_or(Point::ZERO, |p| p.bounds.origin());
        let mut stack = vec![(id, origin)];
        while let Some((id, origin)) = stack.pop() {
            let Some(node) = self.nodes.get_mut(&id) else {
                continue;
            };
            let r = match layers.props(node.layer).frame {
                Some(frame) if node.children.is_empty() => frame,
                _ => self.layout.rect(node.layout).unwrap_or_default(),
            };
            node.bounds = r.translate(origin);
            let o = node.bounds.origin();
            stack.extend(node.children.iter().map(|c| (*c, o)));
        }
    }
}

/// Reconcile `el` into the node `id`. Returns the node's layout node, or
/// `None` if nothing was mounted (a view that failed to render).
pub(crate) fn reconcile(
    tree: &mut Tree,
    app: &mut App,
    id: ElementId,
    parent: Option<ElementId>,
    el: Element,
    owner: EntityId,
) -> Option<LayoutNode> {
    app.reconciled += 1;
    let Element {
        key,
        name,
        kind,
        style,
        visual,
        children,
        handlers,
        tab_index,
        focus_scope,
        key_context,
        layer,
        cull,
        hit_test,
        semantics,
        wires,
    } = el;
    let mut icon = None;
    let text = match kind {
        ElementKind::View(any) => {
            let vid = any.id;
            if let Some(node) = tree.nodes.get_mut(&id)
                && node.view == Some(vid)
                && !app.dirty_views.contains(&vid)
            {
                node.parent = parent;
                return Some(node.layout);
            }
            app.dirty_views.remove(&vid);
            let rendered = app.render_any(any)?;
            app.view_slots.insert(vid, id);
            let layout = reconcile(tree, app, id, parent, rendered, vid)?;
            if let Some(node) = tree.nodes.get_mut(&id) {
                node.view = Some(vid);
            }
            return Some(layout);
        }
        ElementKind::Text(t) => Some(t),
        ElementKind::Icon(i) => {
            icon = Some(i);
            None
        }
        ElementKind::Div => None,
    };
    let label: SharedString = name.or_else(|| key.clone()).unwrap_or_else(|| {
        if text.is_some() {
            "text".into()
        } else if icon.is_some() {
            "icon".into()
        } else {
            "div".into()
        }
    });
    let measure = text
        .as_ref()
        .map_or(Measure::None, |t| crate::text_measure::text_measure(app, t));
    let handlers = handlers.map(|h| Rc::new(*h));

    let layout = if let Some(node) = tree.nodes.get_mut(&id) {
        let _ = tree.layout.set_style(node.layout, &style);
        let _ = tree.layout.set_measure(node.layout, measure);
        if node.visual != visual {
            tree.visual_dirty.push(id);
        }
        node.parent = parent;
        node.label = label;
        node.key = key;
        node.text = text;
        node.icon = icon;
        node.view = None;
        node.owner = owner;
        node.visual = visual;
        node.handlers = handlers;
        if node.tab_index.is_some() != tab_index.is_some() {
            tree.visual_dirty.push(id);
        }
        node.tab_index = tab_index;
        node.focus_scope = focus_scope;
        node.key_context = key_context;
        let old_layer = std::mem::replace(&mut node.layer, layer);
        node.cull = cull;
        node.hit_test = hit_test;
        node.semantics = semantics;
        node.wires = wires;
        let layout = node.layout;
        tree.attach(id, old_layer, layer);
        layout
    } else {
        let layout = tree.layout.create(style, measure).ok()?;
        let anims = NodeAnims::at(Targets::resolve(&visual, false, false, false));
        tree.nodes.insert(
            id,
            Node {
                parent,
                children: Vec::new(),
                label,
                key,
                text,
                icon,
                view: None,
                owner,
                layout,
                visual,
                handlers,
                tab_index,
                focus_scope,
                key_context,
                layer,
                cull,
                hit_test,
                semantics,
                wires,
                bounds: Rect::ZERO,
                anims,
                hovered: false,
                pressed: false,
            },
        );
        if parent.is_none() {
            tree.root = Some(id);
        }
        tree.by_layout.insert(layout, id);
        tree.attach(id, None, layer);
        layout
    };

    // Children: explicit keys, else position among unkeyed siblings.
    let mut ids = Vec::with_capacity(children.len());
    let mut layouts = Vec::with_capacity(children.len());
    let mut unkeyed = 0;
    let mut seen_keys: HashSet<SharedString> = HashSet::new();
    for child in children {
        let cid = match &child.key {
            Some(k) => {
                let mut key = k.clone();
                let mut n = 1;
                while !seen_keys.insert(key.clone()) {
                    n += 1;
                    key = format!("{k}#{n}").into();
                }
                id.child(&key)
            }
            None => {
                unkeyed += 1;
                id.child_index(unkeyed - 1)
            }
        };
        if let Some(l) = reconcile(tree, app, cid, Some(id), child, owner) {
            ids.push(cid);
            layouts.push(l);
        }
    }
    let old = tree
        .nodes
        .get_mut(&id)
        .map(|n| std::mem::replace(&mut n.children, ids.clone()))
        .unwrap_or_default();
    if !old.is_empty() {
        let keep: HashSet<ElementId> = ids.iter().copied().collect();
        for o in old {
            if !keep.contains(&o) {
                remove_subtree(tree, app, o);
            }
        }
    }
    let _ = tree.layout.set_children(layout, &layouts);
    Some(layout)
}

/// Remove a node and its descendants, releasing layout nodes, element
/// state, focus, hover and animations.
pub(crate) fn remove_subtree(tree: &mut Tree, app: &mut App, id: ElementId) {
    let Some(node) = tree.nodes.remove(&id) else {
        return;
    };
    for c in node.children {
        remove_subtree(tree, app, c);
    }
    tree.attach(id, node.layer, None);
    tree.by_layout.remove(&node.layout);
    let _ = tree.layout.remove(node.layout);
    app.forget_element(id, node.view);
    if tree.root == Some(id) {
        tree.root = None;
    }
}

/// Hit test: the path from the root to the topmost element under `point`.
pub(crate) fn hit_path(tree: &Tree, layers: &Layers, viewport: Rect, point: Point) -> Vec<ElementId> {
    let mut path = Vec::new();
    if let Some(root) = tree.root {
        hit(tree, layers, root, point, viewport, &mut path);
    }
    path
}

/// `visible`: the visible area in the parent's space, as painting sees it.
fn hit(
    tree: &Tree,
    layers: &Layers,
    id: ElementId,
    p_parent: Point,
    visible: Rect,
    path: &mut Vec<ElementId>,
) -> bool {
    let Some(node) = tree.nodes.get(&id) else {
        return false;
    };
    if node.opacity(layers) <= 0.001 {
        return false;
    }
    let transform = node.transform(layers);
    let Some(inv) = transform.inverse() else {
        return false;
    };
    let p = inv.apply(p_parent);
    let inside = node.bounds.contains(p);
    if node.visual.clip && !inside {
        return false;
    }
    let visible = node.clip_inside(transform, visible);
    path.push(id);
    for c in node.children.iter().rev() {
        if node.cull
            && let Some(child) = tree.nodes.get(c)
            && node.culls(layers, child, visible)
        {
            continue;
        }
        if hit(tree, layers, *c, p, visible, path) {
            return true;
        }
    }
    if inside && node.hit_test {
        return true;
    }
    path.pop();
    false
}

/// Text dump: one line per node with label and absolute bounds.
pub(crate) fn dump(tree: &Tree, layers: &Layers) -> String {
    let mut out = String::new();
    if let Some(root) = tree.root {
        dump_node(tree, layers, root, 0, &mut out);
    }
    out
}

fn dump_node(tree: &Tree, layers: &Layers, id: ElementId, depth: usize, out: &mut String) {
    let Some(n) = tree.nodes.get(&id) else {
        return;
    };
    let _ = write!(out, "{}{} {}", "  ".repeat(depth), n.label, n.bounds);
    if let Some(t) = &n.text {
        let _ = write!(out, " {:?}", &*t.text);
    }
    let t = n.anims.translate.value();
    if t != Vec2::ZERO {
        let _ = write!(out, " translate={},{}", num(t.x), num(t.y));
    }
    let layer = layers.props(n.layer).transform;
    if !layer.is_identity() {
        let _ = write!(out, " layer={layer}");
    }
    out.push('\n');
    for c in &n.children {
        dump_node(tree, layers, *c, depth + 1, out);
    }
}
