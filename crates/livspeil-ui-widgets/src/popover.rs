// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Popover`]: arbitrary content in a Glass panel anchored to an element,
//! in the overlay layer.

use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, ElementId, EventCx, Render, SharedString, View, ViewContext, actions as core, div,
};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Material, MotionClass, Theme};

use crate::env::{Env, Look, OverlayKey, OverlayKind};
use crate::semantics::{Role, Semantics};

/// Gap between the anchor and the popover.
pub const POPOVER_GAP: f32 = 6.0;
/// Margin kept from the viewport edges.
pub const POPOVER_MARGIN: f32 = 8.0;
/// How far the popover slides in from its anchor (px).
const SLIDE: f32 = 4.0;

/// Which side of the anchor a popover opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Placement {
    /// Above the anchor.
    Top,
    /// Below the anchor (default).
    #[default]
    Bottom,
    /// Left of the anchor.
    Left,
    /// Right of the anchor.
    Right,
}

impl Placement {
    /// The opposite side.
    #[must_use]
    pub fn flipped(self) -> Placement {
        match self {
            Placement::Top => Placement::Bottom,
            Placement::Bottom => Placement::Top,
            Placement::Left => Placement::Right,
            Placement::Right => Placement::Left,
        }
    }

    /// The direction the popover sits in, as seen from the anchor.
    fn direction(self) -> Vec2 {
        match self {
            Placement::Top => Vec2::new(0.0, -1.0),
            Placement::Bottom => Vec2::new(0.0, 1.0),
            Placement::Left => Vec2::new(-1.0, 0.0),
            Placement::Right => Vec2::new(1.0, 0.0),
        }
    }
}

/// Room for a `size` panel on `side` of `anchor` in `viewport`: the panel
/// fits when the result is not negative.
fn room(anchor: Rect, size: Vec2, viewport: Vec2, side: Placement) -> f32 {
    match side {
        Placement::Top => anchor.y - POPOVER_GAP - POPOVER_MARGIN - size.y,
        Placement::Bottom => viewport.y - POPOVER_MARGIN - (anchor.bottom() + POPOVER_GAP) - size.y,
        Placement::Left => anchor.x - POPOVER_GAP - POPOVER_MARGIN - size.x,
        Placement::Right => viewport.x - POPOVER_MARGIN - (anchor.right() + POPOVER_GAP) - size.x,
    }
}

/// Where a `size` popover goes for `anchor` in `viewport`, and on which
/// side. The preferred side is used if the popover fits there; otherwise
/// it flips to the opposite side if that fits, otherwise to whichever of
/// the two has more room. Across the axis it is centered on the anchor and
/// clamped to keep an 8 px margin from the window edges.
#[must_use]
pub fn place_popover(
    anchor: Rect,
    size: Vec2,
    viewport: Vec2,
    preferred: Placement,
) -> (Rect, Placement) {
    let first = room(anchor, size, viewport, preferred);
    let second = room(anchor, size, viewport, preferred.flipped());
    let side = if first >= 0.0 || second < first {
        preferred
    } else {
        preferred.flipped()
    };
    let clamp = |v: f32, len: f32, vp: f32| v.min(vp - POPOVER_MARGIN - len).max(POPOVER_MARGIN);
    let (x, y) = match side {
        Placement::Top => (
            anchor.center().x - size.x / 2.0,
            anchor.y - POPOVER_GAP - size.y,
        ),
        Placement::Bottom => (
            anchor.center().x - size.x / 2.0,
            anchor.bottom() + POPOVER_GAP,
        ),
        Placement::Left => (
            anchor.x - POPOVER_GAP - size.x,
            anchor.center().y - size.y / 2.0,
        ),
        Placement::Right => (
            anchor.right() + POPOVER_GAP,
            anchor.center().y - size.y / 2.0,
        ),
    };
    // Across the axis the panel is clamped; along it only when neither
    // side fits (it then overlaps the anchor rather than leave the window).
    let (x, y) = (clamp(x, size.x, viewport.x), clamp(y, size.y, viewport.y));
    (Rect::new(x, y, size.x, size.y), side)
}

type ContentFn = Rc<dyn Fn(&App) -> Element>;
type TriggerFn = Rc<dyn Fn(&Theme) -> Element>;
type DismissFn = Rc<dyn Fn(&mut App)>;

/// An anchored overlay panel with arbitrary content.
///
/// - Arrow-less Glass panel with a soft shadow, [`Placement`] top, bottom,
///   left or right of its anchor (flipping to the other side when it would
///   leave the window, see [`place_popover`]), centered on the anchor and
///   clamped to the window.
/// - It fades in and slides 4 px away from the anchor (no scaling), and
///   fades out again when it closes; under Reduce Motion the slide is
///   dropped and the fade stays.
/// - Opened by clicking the trigger (the element [`Popover::trigger`]
///   builds; its own click handler is replaced) or from code
///   ([`Popover::open_at`]). Clicking outside, Escape and
///   [`Popover::close`] dismiss it; focus returns to where it was before.
/// - While open, keyboard focus is inside the panel (key context
///   `Popover`, Tab stays in it); the panel itself is the first stop.
///
/// The panel size is given (`size`, default 260×160): content is laid out
/// inside it with 12 px padding. The panel is named `popover` (key
/// `popover-<entity>`), the click-outside scrim `popover-scrim`.
pub struct Popover {
    env: Env,
    content: ContentFn,
    trigger: Option<TriggerFn>,
    placement: Placement,
    size: Vec2,
    label: Option<SharedString>,
    on_dismiss: Option<DismissFn>,
    open: bool,
    anchor: Rect,
    restore: Option<ElementId>,
}

impl std::fmt::Debug for Popover {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Popover")
            .field("placement", &self.placement)
            .field("size", &self.size)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl Popover {
    /// A popover whose panel shows `content` (built with the app, so it can
    /// read models; rebuilt on every render of the overlay layer).
    #[must_use]
    pub fn new(env: &Env, content: impl Fn(&App) -> Element + 'static) -> Self {
        Popover {
            env: env.clone(),
            content: Rc::new(content),
            trigger: None,
            placement: Placement::Bottom,
            size: Vec2::new(260.0, 160.0),
            label: None,
            on_dismiss: None,
            open: false,
            anchor: Rect::ZERO,
            restore: None,
        }
    }

    /// The element that opens the popover when clicked (the view renders
    /// it). Without one, open the popover with [`Popover::open_at`].
    #[must_use]
    pub fn trigger(mut self, f: impl Fn(&Theme) -> Element + 'static) -> Self {
        self.trigger = Some(Rc::new(f));
        self
    }

    /// Preferred side (default bottom).
    #[must_use]
    pub fn placement(mut self, p: Placement) -> Self {
        self.placement = p;
        self
    }

    /// Panel size.
    #[must_use]
    pub fn size(mut self, w: f32, h: f32) -> Self {
        self.size = Vec2::new(w, h);
        self
    }

    /// Accessible name of the panel.
    #[must_use]
    pub fn label(mut self, l: impl Into<SharedString>) -> Self {
        self.label = Some(l.into());
        self
    }

    /// Called after the popover closed (by any means).
    #[must_use]
    pub fn on_dismiss(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(f));
        self
    }

    /// Whether the popover is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Where the open panel is and on which side (for the current window
    /// size).
    #[must_use]
    pub fn placed(&self, app: &App) -> (Rect, Placement) {
        let vp = app.viewport();
        place_popover(
            self.anchor,
            self.size,
            Vec2::new(vp.width, vp.height),
            self.placement,
        )
    }

    /// Semantics of the panel: a `Popover` group with the label.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Popover).label_opt(self.label.as_deref())
    }

    /// Open below/beside `anchor` (window coordinates). Focus moves into
    /// the panel and returns to `restore` (default: what has focus now)
    /// when it closes.
    pub fn open_at(view: View<Popover>, app: &mut App, anchor: Rect, restore: Option<ElementId>) {
        let focused = restore.or_else(|| app.focused());
        let env = view.update(app, |p, cx| {
            p.open = true;
            p.anchor = anchor;
            p.restore = focused;
            cx.notify();
            p.env.clone()
        });
        let build: crate::env::LayerBuilder = Rc::new(move |app, look| layer(view, app, look));
        env.show_overlay(app, OverlayKey::Popup(view.id()), OverlayKind::Layer(build));
        if let Some(id) = env.overlay_child(app, &panel_key(view)) {
            env.focus(app, id);
        }
    }

    /// Close (fading out) and return focus.
    pub fn close(view: View<Popover>, app: &mut App) {
        let (env, restore, dismiss, was_open) = view.update(app, |p, cx| {
            let was = std::mem::replace(&mut p.open, false);
            cx.notify();
            (p.env.clone(), p.restore.take(), p.on_dismiss.clone(), was)
        });
        if !was_open {
            return;
        }
        env.leave_overlay(app, OverlayKey::Popup(view.id()));
        match restore {
            Some(r) => env.focus(app, r),
            None => {
                app.blur();
                env.sync_focus(app);
            }
        }
        if let Some(f) = dismiss {
            f(app);
        }
    }
}

fn panel_key(view: View<Popover>) -> String {
    format!("popover-{:?}", view.id())
}

fn layer(view: View<Popover>, app: &App, look: Look) -> Vec<Element> {
    let p = view.read(app);
    let theme = app.theme();
    let (rect, side) = p.placed(app);
    let vp = app.viewport();
    let hidden = look != Look::Shown;
    let away = side.direction() * -SLIDE;
    let sem = p.semantics();
    let panel = div()
        .key(panel_key(view))
        .name("popover")
        .absolute(rect)
        .p(theme.tokens.spacing.step(3))
        .rounded(theme.tokens.radii.md + 2.0)
        .material(Material::Glass)
        .opacity(if hidden { 0.0 } else { 1.0 })
        .translate(if hidden { away } else { Vec2::ZERO })
        .transition(MotionClass::Standard)
        .key_context("Popover")
        .child((p.content)(app));
    let panel = sem.apply(panel);
    let panel = if look == Look::Leaving {
        panel.hit_test(false)
    } else {
        panel
            .tab_index(-1)
            .focus_scope()
            .on_pointer_down(|_, cx| cx.stop_propagation())
            .on_action(core::CANCEL, move |cx| Popover::close(view, cx))
    };
    let mut scrim = div()
        .key(format!("popover-scrim-{:?}", view.id()))
        .name("popover-scrim")
        .absolute(Rect::new(0.0, 0.0, vp.width, vp.height));
    scrim = if look == Look::Leaving {
        scrim.hit_test(false)
    } else {
        scrim.on_pointer_down(move |_, cx| {
            Popover::close(view, cx);
            cx.stop_propagation();
        })
    };
    vec![scrim, panel]
}

impl Render for Popover {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let Some(trigger) = self.trigger.clone() else {
            return div().name("popover-anchor").hit_test(false);
        };
        let open = self.open;
        trigger(&theme)
            .on_click(move |_, cx| toggle(view, cx))
            .semantics(move |s| s.expanded(open))
    }
}

fn toggle(view: View<Popover>, cx: &mut EventCx<'_>) {
    if view.read(cx).open {
        Popover::close(view, cx);
    } else {
        let anchor = cx.bounds();
        let trigger = cx.element_id();
        Popover::open_at(view, cx, anchor, Some(trigger));
    }
}
