// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Entities: models (state) and views (state that renders).

use std::fmt;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use livspeil_ui_theme::{MotionConfig, Theme};

use crate::app::App;
use crate::element::Element;
use crate::layer::Layer;

/// Identity of a model or view.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId(pub(crate) u64);

/// A handle to state of type `T` owned by the [`App`].
///
/// Read with [`Model::read`]; change with [`Model::update`] and call
/// [`ModelContext::notify`] so views that read it re-render.
pub struct Model<T> {
    pub(crate) id: EntityId,
    _ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Model<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Model<T> {}
impl<T> PartialEq for Model<T> {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}
impl<T> fmt::Debug for Model<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Model<{}>({})", std::any::type_name::<T>(), self.id.0)
    }
}

impl<T: 'static> Model<T> {
    pub(crate) fn from_id(id: EntityId) -> Self {
        Model {
            id,
            _ty: PhantomData,
        }
    }

    /// Entity id.
    #[must_use]
    pub fn id(&self) -> EntityId {
        self.id
    }

    /// Read the state. During a view's `render` this also records that the
    /// view observes this model.
    ///
    /// # Panics
    /// If the model is being updated (re-entrant access from inside its own
    /// `update`) or was released: both are programming errors.
    #[must_use]
    pub fn read<'a>(&self, cx: &'a App) -> &'a T {
        match cx.read_entity::<T>(self.id) {
            Some(v) => v,
            None => panic!("{self:?} is being updated or was released"),
        }
    }

    /// Update the state.
    ///
    /// # Panics
    /// If the model is already being updated or was released.
    pub fn update<R>(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut T, &mut ModelContext<'_, T>) -> R,
    ) -> R {
        cx.update_entity(self.id, |value: &mut T, app| {
            let mut mcx = ModelContext {
                app,
                id: self.id,
                _ty: PhantomData,
            };
            f(value, &mut mcx)
        })
    }
}

/// Context passed to [`Model::update`]; derefs to [`App`].
pub struct ModelContext<'a, T> {
    app: &'a mut App,
    id: EntityId,
    _ty: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for ModelContext<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ModelContext({})", self.id.0)
    }
}

impl<T: 'static> ModelContext<'_, T> {
    /// Re-render every view that observed this model.
    pub fn notify(&mut self) {
        self.app.notify(self.id);
    }

    /// The model being updated.
    #[must_use]
    pub fn model(&self) -> Model<T> {
        Model::from_id(self.id)
    }
}

impl<T> Deref for ModelContext<'_, T> {
    type Target = App;
    fn deref(&self) -> &App {
        self.app
    }
}

impl<T> DerefMut for ModelContext<'_, T> {
    fn deref_mut(&mut self) -> &mut App {
        self.app
    }
}

/// State that renders an element tree.
pub trait Render: Sized + 'static {
    /// Build the element tree. Called only when this view was notified or a
    /// model it read in its last render changed.
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element;
}

/// A handle to a view.
pub struct View<V> {
    pub(crate) id: EntityId,
    _ty: PhantomData<fn() -> V>,
}

impl<V> Clone for View<V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<V> Copy for View<V> {}
impl<V> PartialEq for View<V> {
    fn eq(&self, o: &Self) -> bool {
        self.id == o.id
    }
}
impl<V> fmt::Debug for View<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "View<{}>({})", std::any::type_name::<V>(), self.id.0)
    }
}

impl<V: Render> View<V> {
    pub(crate) fn from_id(id: EntityId) -> Self {
        View {
            id,
            _ty: PhantomData,
        }
    }

    /// Entity id.
    #[must_use]
    pub fn id(&self) -> EntityId {
        self.id
    }

    /// Read the view's state.
    ///
    /// # Panics
    /// If the view is being updated or rendered.
    #[must_use]
    pub fn read<'a>(&self, cx: &'a App) -> &'a V {
        match cx.read_entity::<V>(self.id) {
            Some(v) => v,
            None => panic!("{self:?} is being updated or was released"),
        }
    }

    /// Update the view's state; call `cx.notify()` to re-render it.
    ///
    /// # Panics
    /// If the view is already being updated or rendered.
    pub fn update<R>(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut V, &mut ViewContext<'_, V>) -> R,
    ) -> R {
        cx.update_entity(self.id, |value: &mut V, app| {
            let mut vcx = ViewContext {
                app,
                id: self.id,
                _ty: PhantomData,
            };
            f(value, &mut vcx)
        })
    }

    /// Type-erased handle.
    #[must_use]
    pub fn into_any(self) -> AnyView {
        AnyView {
            id: self.id,
            render: render_view::<V>,
        }
    }
}

fn render_view<V: Render>(app: &mut App, id: EntityId) -> Option<Element> {
    app.lease_entity(id, |view: &mut V, app| {
        let mut vcx = ViewContext {
            app,
            id,
            _ty: PhantomData,
        };
        view.render(&mut vcx)
    })
}

/// A type-erased view handle.
#[derive(Clone, Copy)]
pub struct AnyView {
    pub(crate) id: EntityId,
    pub(crate) render: fn(&mut App, EntityId) -> Option<Element>,
}

impl fmt::Debug for AnyView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AnyView({})", self.id.0)
    }
}

impl AnyView {
    /// Entity id.
    #[must_use]
    pub fn id(&self) -> EntityId {
        self.id
    }
}

/// Context passed to [`Render::render`] and [`View::update`]; derefs to [`App`].
pub struct ViewContext<'a, V> {
    app: &'a mut App,
    id: EntityId,
    _ty: PhantomData<fn() -> V>,
}

impl<V> fmt::Debug for ViewContext<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ViewContext({})", self.id.0)
    }
}

impl<V: Render> ViewContext<'_, V> {
    /// Re-render this view.
    pub fn notify(&mut self) {
        self.app.notify(self.id);
    }

    /// This view's handle (for event handlers to capture).
    #[must_use]
    pub fn view(&self) -> View<V> {
        View::from_id(self.id)
    }

    /// Run `f` whenever `entity` is notified, until this view is released.
    ///
    /// `f` may run while this view renders, if the render itself notified
    /// `entity`, so it must not update the view directly in that case;
    /// scheduling work with [`App::animate`](crate::App::animate) is
    /// always safe.
    pub fn watch(&mut self, entity: EntityId, f: impl Fn(&mut App) + 'static) {
        self.app.observe_for(self.id, entity, f);
    }

    /// This view's layer `key`: the same handle in every render, until the
    /// view is released. Attach it with
    /// [`Element::layer`](crate::Element::layer) and set its properties at
    /// any time without rendering the view again (see [`Layer`]).
    pub fn layer(&mut self, key: &str) -> Layer {
        self.app.layer(self.id, key)
    }

    /// The theme.
    #[must_use]
    pub fn theme(&self) -> &Theme {
        self.app.theme()
    }

    /// This view's root element bounds (window logical pixels) as of the
    /// last layout; `None` before the view was first laid out. Calling it
    /// subscribes the view to its layout: when the bounds change it
    /// renders again in the next frame (once per change), so views that
    /// size their content to the space they get (canvases, virtual lists)
    /// do not need a pointer event to learn it.
    pub fn layout_bounds(&mut self) -> Option<livspeil_ui_layout::Rect> {
        self.app.watch_layout(self.id)
    }

    /// The motion configuration.
    #[must_use]
    pub fn motion(&self) -> &MotionConfig {
        self.app.motion()
    }
}

impl<V> Deref for ViewContext<'_, V> {
    type Target = App;
    fn deref(&self) -> &App {
        self.app
    }
}

impl<V> DerefMut for ViewContext<'_, V> {
    fn deref_mut(&mut self) -> &mut App {
        self.app
    }
}
