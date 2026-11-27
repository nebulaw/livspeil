// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Layers: compositor properties (transform, opacity, frame) held beside
//! the element tree instead of baked into it, and the per-frame tasks that
//! drive them (`specs/livspeil-ui-frame-pipeline.md`, 4.1).
//!
//! A view owns a [`Layer`] (see [`ViewContext::layer`](crate::ViewContext::layer))
//! and attaches it to an element with [`Element::layer`](crate::Element::layer).
//! Setting a property schedules a frame and nothing else: no view renders,
//! nothing is diffed and no layout runs. Painting, hit testing, the focus
//! ring, [`App::visual_bounds`], accessibility bounds and IME placement all
//! read the same values, so they cannot disagree with what is drawn. A
//! layer element is also a repaint boundary: its content is described in
//! its own coordinates and does not depend on the properties.
//!
//! Layers are dropped when their view is released; a handle to a dropped
//! layer is inert (setters return `false`).
//!
//! Motion (springs, momentum, gestures) drives layers through the same
//! setters, from a task registered with [`App::animate`] that runs at the
//! start of each frame, before any view renders.

use std::collections::{HashMap, HashSet};
use std::fmt;

use livspeil_ui_layout::{Rect, Transform2D};

use crate::app::App;
use crate::element::{ElementId, SharedString};
use crate::entity::EntityId;

/// A handle to a set of compositor properties (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Layer(u64);

/// The properties of one layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Props {
    pub(crate) transform: Transform2D,
    pub(crate) opacity: f32,
    pub(crate) frame: Option<Rect>,
}

impl Default for Props {
    fn default() -> Self {
        Props {
            transform: Transform2D::IDENTITY,
            opacity: 1.0,
            frame: None,
        }
    }
}

#[derive(Debug)]
struct Owned {
    owner: EntityId,
    key: SharedString,
    props: Props,
}

/// Every live layer. Kept outside the element tree so views can use their
/// layers while the tree is being reconciled.
#[derive(Debug, Default)]
pub(crate) struct Layers {
    next: u64,
    live: HashMap<Layer, Owned>,
    by_key: HashMap<(EntityId, SharedString), Layer>,
    /// Layers whose frame changed since the last frame was drawn.
    pub(crate) reframed: HashSet<Layer>,
}

impl Layers {
    pub(crate) fn get(&mut self, owner: EntityId, key: SharedString) -> Layer {
        if let Some(l) = self.by_key.get(&(owner, key.clone())) {
            return *l;
        }
        self.next += 1;
        let layer = Layer(self.next);
        self.live.insert(
            layer,
            Owned {
                owner,
                key: key.clone(),
                props: Props::default(),
            },
        );
        self.by_key.insert((owner, key), layer);
        layer
    }

    /// The properties of `layer`; the neutral ones for none or a dropped one.
    pub(crate) fn props(&self, layer: Option<Layer>) -> Props {
        layer
            .and_then(|l| self.live.get(&l))
            .map_or_else(Props::default, |o| o.props)
    }

    fn entry(&mut self, layer: Layer) -> Option<&mut Props> {
        self.live.get_mut(&layer).map(|o| &mut o.props)
    }

    fn lookup(&self, layer: Layer) -> Option<Props> {
        self.live.get(&layer).map(|o| o.props)
    }

    /// Drop every layer of `owner`.
    pub(crate) fn release(&mut self, owner: EntityId) {
        let by_key = &mut self.by_key;
        let reframed = &mut self.reframed;
        self.live.retain(|layer, o| {
            let keep = o.owner != owner;
            if !keep {
                by_key.remove(&(owner, o.key.clone()));
                reframed.remove(layer);
            }
            keep
        });
    }
}

impl Layer {
    /// Set the transform of the element's subtree, about the element's
    /// layout origin (so a translation is simply an offset). Returns
    /// `false` if the layer is gone.
    pub fn set_transform(self, app: &mut App, transform: Transform2D) -> bool {
        app.change_layer(self, |p| p.transform = transform)
    }

    /// Set an opacity, multiplied with the element's own. At zero the
    /// subtree is neither painted nor hit. Returns `false` if the layer is
    /// gone.
    pub fn set_opacity(self, app: &mut App, opacity: f32) -> bool {
        app.change_layer(self, |p| p.opacity = opacity)
    }

    /// Replace the layout box of a **leaf** element with `frame`, relative
    /// to its parent and snapped to whole logical pixels as layout does
    /// (`None`: back to the layout box). For indicators that move and
    /// resize without anything else depending on them: scrollbar thumbs, a
    /// minimap's viewport frame. Elements with children ignore it. Returns
    /// `false` if the layer is gone.
    pub fn set_frame(self, app: &mut App, frame: Option<Rect>) -> bool {
        let snap = |v: f32| (v + 0.5).floor();
        let frame = frame.map(|r| Rect::new(snap(r.x), snap(r.y), snap(r.width), snap(r.height)));
        app.change_layer(self, |p| p.frame = frame)
    }

    /// The transform, if the layer exists.
    #[must_use]
    pub fn transform(self, app: &App) -> Option<Transform2D> {
        app.layer_props(self).map(|p| p.transform)
    }

    /// The opacity, if the layer exists.
    #[must_use]
    pub fn opacity(self, app: &App) -> Option<f32> {
        app.layer_props(self).map(|p| p.opacity)
    }

    /// The frame override, if the layer exists and has one.
    #[must_use]
    pub fn frame(self, app: &App) -> Option<Rect> {
        app.layer_props(self).and_then(|p| p.frame)
    }

    /// Whether the layer exists (its view was not released).
    #[must_use]
    pub fn is_alive(self, app: &App) -> bool {
        app.layer_props(self).is_some()
    }
}

type Step = Box<dyn FnMut(&mut App, f32) -> bool>;

/// A task run at the start of each frame (see [`App::animate`]).
pub(crate) struct Task {
    pub(crate) owner: EntityId,
    key: &'static str,
    step: Step,
}

impl fmt::Debug for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Task({}, {})", self.owner.0, self.key)
    }
}

impl App {
    /// The layer `key` of the entity `owner`, created on first use. Views
    /// use [`ViewContext::layer`](crate::ViewContext::layer).
    pub fn layer(&mut self, owner: EntityId, key: impl Into<SharedString>) -> Layer {
        self.layers.get(owner, key.into())
    }

    pub(crate) fn layer_props(&self, layer: Layer) -> Option<Props> {
        self.layers.lookup(layer)
    }

    pub(crate) fn change_layer(&mut self, layer: Layer, change: impl FnOnce(&mut Props)) -> bool {
        let Some(props) = self.layers.entry(layer) else {
            return false;
        };
        let before = *props;
        change(props);
        let after = *props;
        if before == after {
            return true;
        }
        if before.frame != after.frame {
            self.layers.reframed.insert(layer);
        }
        self.positions_changed = true;
        // Not attached (not shown, or the tree is being reconciled, which
        // draws a frame anyway): nothing to schedule.
        let elements: Vec<ElementId> = self.tree.layered(layer).to_vec();
        for id in elements {
            if before.frame != after.frame {
                self.tree.update_bounds_from(&self.layers, id);
            }
            self.note_moved(id);
            self.paint_dirty = true;
        }
        true
    }

    /// Run `step` at the start of every following frame, with the frame's
    /// `dt` in seconds, until it returns `false`. Frames are drawn as long
    /// as a task runs. A task replaces an earlier one of the same `owner`
    /// and `key`, and is dropped when `owner` is released.
    ///
    /// This is how motion drives layers: step a spring or a physics model,
    /// set the properties, and notify a view only if its content must
    /// change (it then renders in the same frame).
    pub fn animate(
        &mut self,
        owner: EntityId,
        key: &'static str,
        step: impl FnMut(&mut App, f32) -> bool + 'static,
    ) {
        self.stop_animation(owner, key);
        self.tasks.push(Task {
            owner,
            key,
            step: Box::new(step),
        });
    }

    /// Whether a task of `owner` and `key` is scheduled.
    #[must_use]
    pub fn is_animating(&self, owner: EntityId, key: &str) -> bool {
        self.tasks.iter().any(|t| t.owner == owner && t.key == key)
    }

    /// Drop the task of `owner` and `key`.
    pub fn stop_animation(&mut self, owner: EntityId, key: &str) {
        self.tasks.retain(|t| t.owner != owner || t.key != key);
    }

    pub(crate) fn run_tasks(&mut self, dt: f32) {
        let mut running = std::mem::take(&mut self.tasks);
        running.retain_mut(|t| self.exists(t.owner) && (t.step)(self, dt));
        // A task registered while running wins over the one that ran.
        running.retain(|t| self.exists(t.owner) && !self.is_animating(t.owner, t.key));
        self.tasks.append(&mut running);
        self.flush_effects();
    }
}
