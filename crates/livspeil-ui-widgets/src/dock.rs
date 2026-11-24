// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Panel`] (a titled Solid or Frost surface) and [`DockView`] (panels
//! around a center on `livspeil_ui_layout::Dock`, with draggable splitters
//! and collapsible sides).

use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use livspeil_ui_core::{
    Animated, App, CursorShape, DOUBLE_PRESS_TIME, DragPhase, Element, PointerKind, Render,
    SharedString, View, ViewContext, div, text,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto, Position};
use livspeil_ui_layout::{Dock, DockEdge, DockSide, SplitterDrag};
use livspeil_ui_theme::{Material, MotionClass, PropertyKind, Theme};

use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};

/// Motion scope of panel animations (`[overrides.panels]`).
pub const PANEL_SCOPE: &str = "panels";

/// A panel's surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PanelMaterial {
    /// Dense tools and inspectors: surface.1/2 with a hairline.
    #[default]
    Solid,
    /// Sidebars and docks: backdrop blur.
    Frost,
}

/// A titled panel: header row (title and trailing actions) over a body
/// that fills the rest and clips.
pub struct Panel {
    title: SharedString,
    material: PanelMaterial,
    actions: Vec<Element>,
    body: Vec<Element>,
    radius: Option<f32>,
}

impl fmt::Debug for Panel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Panel")
            .field("title", &self.title)
            .field("material", &self.material)
            .finish_non_exhaustive()
    }
}

impl Panel {
    /// A Solid panel.
    #[must_use]
    pub fn new(title: impl Into<SharedString>) -> Self {
        Panel {
            title: title.into(),
            material: PanelMaterial::Solid,
            actions: Vec::new(),
            body: Vec::new(),
            radius: None,
        }
    }

    /// Use the Frost material.
    #[must_use]
    pub fn frost(mut self) -> Self {
        self.material = PanelMaterial::Frost;
        self
    }

    /// Set the material.
    #[must_use]
    pub fn material(mut self, m: PanelMaterial) -> Self {
        self.material = m;
        self
    }

    /// Corner radius (default none: docked panels are square).
    #[must_use]
    pub fn rounded(mut self, r: f32) -> Self {
        self.radius = Some(r);
        self
    }

    /// A trailing header item (an icon button, a menu).
    #[must_use]
    pub fn action(mut self, el: Element) -> Self {
        self.actions.push(el);
        self
    }

    /// Body content.
    #[must_use]
    pub fn child(mut self, el: impl Into<Element>) -> Self {
        self.body.push(el.into());
        self
    }

    /// Semantics: a `Panel` (group) named by its title.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Panel).label(&*self.title)
    }

    /// Build the element (fills its parent).
    #[must_use]
    pub fn render(self, theme: &Theme) -> Element {
        let m = metrics(theme);
        let c = theme.colors();
        let sem = self.semantics();
        let header = div()
            .key("header")
            .h(m.height + 4.0)
            .px(m.pad_x)
            .flex_row()
            .items_center()
            .gap(m.gap)
            .child(
                text(self.title.clone())
                    .key("title")
                    .font_size(m.font)
                    .font_weight(600.0)
                    .text_color(c.text_primary)
                    .hit_test(false)
                    .grow(),
            )
            .children(self.actions);
        let rule = div().key("rule").h(1.0).bg(style::hairline(theme));
        let body = div()
            .key("body")
            .grow()
            .flex_col()
            .clip()
            .style(|s| s.min_size.height = LengthPercentageAuto::length(0.0))
            .children(self.body);
        let el = div()
            .name(format!("panel:{}", self.title))
            .w_full()
            .h_full()
            .flex_col()
            .material(match self.material {
                PanelMaterial::Solid => Material::Solid,
                PanelMaterial::Frost => Material::Frost,
            })
            .child(header)
            .child(rule)
            .child(body);
        let el = match self.radius {
            Some(r) => el.rounded(r),
            None => el,
        };
        sem.apply(el)
    }
}

/// Builds a dock slot's content.
pub type SlotFn = Rc<dyn Fn(&App) -> Element>;

fn index(edge: DockEdge) -> usize {
    match edge {
        DockEdge::Left => 0,
        DockEdge::Right => 1,
        DockEdge::Top => 2,
        DockEdge::Bottom => 3,
    }
}

fn edge_name(edge: DockEdge) -> &'static str {
    match edge {
        DockEdge::Left => "left",
        DockEdge::Right => "right",
        DockEdge::Top => "top",
        DockEdge::Bottom => "bottom",
    }
}

/// Panels around a center (UI spec §9 Panel/Dock/Splitter).
///
/// - Layout: left and right panels span the full height, top and bottom
///   the space between them (as `livspeil_ui_layout::Dock`).
/// - Splitters sit on each visible panel's inner edge (a 1 px hairline
///   with a `hit_slop` grab area on both sides), show the ew/ns-resize
///   cursor, follow the pointer while dragged (sizes clamped to the
///   panel's minimum and to what leaves the center its minimum) and reset
///   to the initial size on a double press.
/// - [`DockView::set_collapsed`] / [`DockView::toggle`] collapse a side to
///   zero width with the Large spring (instant under Reduce Motion); the
///   panel keeps its full size inside and slides out of view.
pub struct DockView {
    dock: Dock,
    initial: [f32; 4],
    collapsed: [bool; 4],
    shown: [Animated<f32>; 4],
    center: SlotFn,
    slots: [Option<SlotFn>; 4],
    drag: Option<SplitterDrag>,
    pressed: Option<DockEdge>,
    last_press: Option<(DockEdge, Duration)>,
    name: Option<SharedString>,
}

impl fmt::Debug for DockView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DockView")
            .field("dock", &self.dock)
            .field("collapsed", &self.collapsed)
            .finish_non_exhaustive()
    }
}

impl DockView {
    /// A dock with only a center.
    #[must_use]
    pub fn new(center: impl Fn(&App) -> Element + 'static) -> Self {
        DockView {
            dock: Dock {
                hit_slop: 4.0,
                ..Dock::default()
            },
            initial: [0.0; 4],
            collapsed: [false; 4],
            shown: [Animated::new(0.0); 4],
            center: Rc::new(center),
            slots: [None, None, None, None],
            drag: None,
            pressed: None,
            last_press: None,
            name: None,
        }
    }

    /// Add a panel at `edge` with an initial `size` and a minimum.
    #[must_use]
    pub fn panel(
        mut self,
        edge: DockEdge,
        size: f32,
        min: f32,
        content: impl Fn(&App) -> Element + 'static,
    ) -> Self {
        let side = DockSide::new(size, min);
        match edge {
            DockEdge::Left => self.dock.left = Some(side),
            DockEdge::Right => self.dock.right = Some(side),
            DockEdge::Top => self.dock.top = Some(side),
            DockEdge::Bottom => self.dock.bottom = Some(side),
        }
        let i = index(edge);
        self.initial[i] = size;
        self.shown[i] = Animated::new(size);
        self.slots[i] = Some(Rc::new(content));
        self
    }

    /// Minimum size of the center.
    #[must_use]
    pub fn center_min(mut self, w: f32, h: f32) -> Self {
        self.dock.center_min = livspeil_ui_layout::Size::new(w, h);
        self
    }

    /// Element name (default `dock`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// The dock state (sizes, minimums).
    #[must_use]
    pub fn dock(&self) -> &Dock {
        &self.dock
    }

    /// The stored size of a side.
    #[must_use]
    pub fn size(&self, edge: DockEdge) -> Option<f32> {
        self.dock.side(edge).map(|s| s.size)
    }

    /// The size currently shown (animating while collapsing).
    #[must_use]
    pub fn shown_size(&self, edge: DockEdge) -> f32 {
        self.shown[index(edge)].value()
    }

    /// Whether a side is collapsed.
    #[must_use]
    pub fn is_collapsed(&self, edge: DockEdge) -> bool {
        self.collapsed[index(edge)]
    }

    /// Collapse or expand a side (animated).
    pub fn set_collapsed(view: View<DockView>, app: &mut App, edge: DockEdge, collapsed: bool) {
        view.update(app, |d, cx| {
            let i = index(edge);
            if d.collapsed[i] != collapsed {
                d.collapsed[i] = collapsed;
                if let Some(s) = d.dock.side_mut(edge) {
                    s.visible = !collapsed;
                }
                cx.notify();
            }
        });
    }

    /// Toggle a side.
    pub fn toggle(view: View<DockView>, app: &mut App, edge: DockEdge) {
        let now = view.read(app).is_collapsed(edge);
        DockView::set_collapsed(view, app, edge, !now);
    }

    fn splitter(&self, view: View<DockView>, edge: DockEdge, theme: &Theme) -> Element {
        let i = index(edge);
        let at = self.shown[i].value();
        let slop = self.dock.hit_slop;
        let t = self.dock.splitter;
        let size = self.dock.side(edge).map_or(0.0, |s| s.size);
        let min = self.dock.side(edge).map_or(0.0, |s| s.min);
        let line_color = style::hairline(theme);
        let active = self.drag.is_some_and(|d| d.edge == edge);
        let horizontal = edge.is_horizontal();
        let (inset_l, inset_r) = (self.shown[0].value(), self.shown[1].value());
        let line = div()
            .key("line")
            .bg(if active { theme.accent() } else { line_color })
            .hit_test(false)
            .style(move |s| {
                s.position = Position::Absolute;
                if horizontal {
                    s.inset.left = LengthPercentageAuto::length(slop);
                    s.inset.top = LengthPercentageAuto::length(0.0);
                    s.size.width = Dimension::length(t);
                    s.size.height = Dimension::percent(1.0);
                } else {
                    s.inset.top = LengthPercentageAuto::length(slop);
                    s.inset.left = LengthPercentageAuto::length(0.0);
                    s.size.height = Dimension::length(t);
                    s.size.width = Dimension::percent(1.0);
                }
            });
        let el = div()
            .key(format!("splitter-{}", edge_name(edge)))
            .name(format!("splitter:{}", edge_name(edge)))
            .style(move |s| {
                s.position = Position::Absolute;
                let len = LengthPercentageAuto::length;
                let thick = Dimension::length(t + 2.0 * slop);
                match edge {
                    DockEdge::Left => {
                        s.inset.left = len(at - t - slop);
                        s.inset.top = len(0.0);
                        s.inset.bottom = len(0.0);
                        s.size.width = thick;
                    }
                    DockEdge::Right => {
                        s.inset.right = len(at - t - slop);
                        s.inset.top = len(0.0);
                        s.inset.bottom = len(0.0);
                        s.size.width = thick;
                    }
                    DockEdge::Top => {
                        s.inset.top = len(at - t - slop);
                        s.inset.left = len(inset_l);
                        s.inset.right = len(inset_r);
                        s.size.height = thick;
                    }
                    DockEdge::Bottom => {
                        s.inset.bottom = len(at - t - slop);
                        s.inset.left = len(inset_l);
                        s.inset.right = len(inset_r);
                        s.size.height = thick;
                    }
                }
            })
            .child(line)
            .cursor(if horizontal {
                CursorShape::ResizeEw
            } else {
                CursorShape::ResizeNs
            })
            .on_pointer_down(move |ev, cx| {
                if !matches!(
                    ev.kind,
                    PointerKind::Down(livspeil_ui_core::MouseButton::Left)
                ) {
                    return;
                }
                let time = ev.time;
                view.update(cx, |d, vcx| {
                    let double = d.last_press.is_some_and(|(e, t)| {
                        e == edge && time.saturating_sub(t) <= DOUBLE_PRESS_TIME
                    });
                    if double {
                        let reset = d.initial[index(edge)];
                        if let Some(s) = d.dock.side_mut(edge) {
                            s.size = reset;
                        }
                        d.last_press = None;
                        d.pressed = None;
                    } else {
                        d.last_press = Some((edge, time));
                        d.pressed = Some(edge);
                    }
                    vcx.notify();
                });
            });
        let sem = Semantics::new(Role::Splitter)
            .label(format!("{} panel size", edge_name(edge)))
            .numeric(
                f64::from(size),
                f64::from(min),
                f64::from(size.max(min)),
                1.0,
            );
        sem.apply(el)
    }
}

impl Render for DockView {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let spec = cx.motion().resolve(
            Some(PANEL_SCOPE),
            MotionClass::Large,
            PropertyKind::Movement,
        );
        let dt = cx.frame_delta();
        let mut moving = false;
        for edge in DockEdge::ALL {
            let i = index(edge);
            let Some(side) = self.dock.side(edge) else {
                continue;
            };
            let target = if self.collapsed[i] { 0.0 } else { side.size };
            let a = &mut self.shown[i];
            if a.target() != target {
                if self.drag.is_some_and(|d| d.edge == edge) {
                    a.set(target);
                } else {
                    a.animate_to(target, spec);
                }
            } else if a.is_animating() {
                a.step(dt);
            }
            moving |= a.is_animating();
        }
        if moving {
            cx.notify();
        }

        let slot = |d: &DockView, edge: DockEdge, app: &App| -> Option<Element> {
            let i = index(edge);
            let f = d.slots[i].as_ref()?;
            let side = d.dock.side(edge)?;
            let shown = d.shown[i].value();
            if shown <= 0.01 && d.collapsed[i] {
                return None;
            }
            let full = side.size.max(shown);
            let inner = f(app);
            let horizontal = edge.is_horizontal();
            // The content keeps its full size and slides out at the
            // panel's outer edge while collapsing.
            let inner = div()
                .key("inner")
                .flex_col()
                .style(move |s| {
                    s.position = Position::Absolute;
                    s.flex_shrink = 0.0;
                    let len = LengthPercentageAuto::length;
                    match edge {
                        DockEdge::Left => s.inset.right = len(0.0),
                        DockEdge::Right => s.inset.left = len(0.0),
                        DockEdge::Top => s.inset.bottom = len(0.0),
                        DockEdge::Bottom => s.inset.top = len(0.0),
                    }
                    if horizontal {
                        s.size.width = Dimension::length(full);
                        s.size.height = Dimension::percent(1.0);
                    } else {
                        s.size.height = Dimension::length(full);
                        s.size.width = Dimension::percent(1.0);
                    }
                })
                .child(inner);
            let wrap = div()
                .key(format!("panel-{}", edge_name(edge)))
                .clip()
                .style(move |s| {
                    s.flex_shrink = 0.0;
                    if horizontal {
                        s.size.width = Dimension::length(shown);
                        s.size.height = Dimension::percent(1.0);
                    } else {
                        s.size.height = Dimension::length(shown);
                        s.size.width = Dimension::percent(1.0);
                    }
                })
                .child(inner);
            Some(wrap)
        };

        let center_min = self.dock.center_min;
        let center = div()
            .key("center")
            .grow()
            .clip()
            .flex_col()
            .style(move |s| {
                s.min_size.height = LengthPercentageAuto::length(center_min.height);
                s.min_size.width = LengthPercentageAuto::length(0.0);
            })
            .child((self.center)(cx));
        let mut middle = div()
            .key("middle")
            .grow()
            .flex_col()
            .style(move |s| s.min_size.width = LengthPercentageAuto::length(center_min.width));
        if let Some(t) = slot(self, DockEdge::Top, cx) {
            middle = middle.child(t);
        }
        middle = middle.child(center);
        if let Some(b) = slot(self, DockEdge::Bottom, cx) {
            middle = middle.child(b);
        }
        let mut root = div()
            .name(self.name.clone().unwrap_or_else(|| "dock".into()))
            .w_full()
            .h_full()
            .flex_row();
        if let Some(l) = slot(self, DockEdge::Left, cx) {
            root = root.child(l);
        }
        root = root.child(middle);
        if let Some(r) = slot(self, DockEdge::Right, cx) {
            root = root.child(r);
        }
        for edge in DockEdge::ALL {
            if self.dock.side(edge).is_some_and(|s| s.visible) && !self.collapsed[index(edge)] {
                root = root.child(self.splitter(view, edge, &theme));
            }
        }
        root.on_drag(move |ev, cx| {
            let bounds = cx.bounds();
            view.update(cx, |d, vcx| {
                match ev.phase {
                    DragPhase::Start => {
                        d.drag = d.pressed.and_then(|e| d.dock.begin_drag(e));
                        // A drag is not the first press of a double press.
                        d.last_press = None;
                    }
                    DragPhase::Move | DragPhase::End => {}
                }
                if let Some(drag) = d.drag {
                    let total = ev.total();
                    let travel = if drag.edge.is_horizontal() {
                        total.x
                    } else {
                        total.y
                    };
                    d.dock.drag(drag, travel, bounds.size());
                    vcx.notify();
                }
                if ev.phase == DragPhase::End {
                    d.drag = None;
                    d.pressed = None;
                    vcx.notify();
                }
            });
        })
        .on_pointer_up(move |_, cx| {
            view.update(cx, |d, _| d.pressed = None);
        })
    }
}
