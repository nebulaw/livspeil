// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Elements: the cheap description a view's `render` returns. The App diffs
//! it into the retained tree by [`ElementId`].

use std::fmt;
use std::rc::Rc;
use std::time::Duration;

use livspeil_ui_layout::style::{
    AlignItems, Dimension, FlexDirection, JustifyContent, LengthPercentage, LengthPercentageAuto,
    Position, length,
};
use livspeil_ui_layout::{Canvas, Corners, Rect, Style, Transform2D, Vec2, taffy};
use livspeil_ui_theme::{Color, Elevation, Material, MotionClass};

use crate::anim::Easing;
use crate::app::EventCx;
use crate::entity::{AnyView, Render, View};
use crate::input::{ClickEvent, DragEvent, ImeEvent, KeyEvent, PointerEvent};

/// Cheaply clonable string.
pub type SharedString = Rc<str>;

/// Stable identity of an element: a hash of its parent's id and its key
/// (or its position among unkeyed siblings). Used for diffing, focus,
/// animations, per-element state and (later) accessibility.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId(pub u64);

impl fmt::Debug for ElementId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ElementId({:016x})", self.0)
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

impl ElementId {
    /// The window root.
    pub const ROOT: ElementId = ElementId(FNV_OFFSET);

    /// The id of a child with explicit `key`.
    #[must_use]
    pub fn child(self, key: &str) -> ElementId {
        let h = fnv(fnv(self.0, &self.0.to_le_bytes()), b"k");
        ElementId(fnv(h, key.as_bytes()))
    }

    /// The id of the `index`th unkeyed child.
    #[must_use]
    pub fn child_index(self, index: usize) -> ElementId {
        let h = fnv(fnv(self.0, &self.0.to_le_bytes()), b"i");
        ElementId(fnv(h, &(index as u64).to_le_bytes()))
    }
}

/// Animated visual properties a refinement (hover, active, focus) can change.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Refinement {
    pub(crate) background: Option<Color>,
    pub(crate) border_color: Option<Color>,
    pub(crate) opacity: Option<f32>,
    pub(crate) translate: Option<Vec2>,
    pub(crate) scale: Option<f32>,
}

impl Refinement {
    /// Background color.
    #[must_use]
    pub fn bg(mut self, c: Color) -> Self {
        self.background = Some(c);
        self
    }
    /// Border color.
    #[must_use]
    pub fn border_color(mut self, c: Color) -> Self {
        self.border_color = Some(c);
        self
    }
    /// Opacity.
    #[must_use]
    pub fn opacity(mut self, o: f32) -> Self {
        self.opacity = Some(o);
        self
    }
    /// Translation.
    #[must_use]
    pub fn translate(mut self, v: Vec2) -> Self {
        self.translate = Some(v);
        self
    }
    /// Uniform scale about the center.
    #[must_use]
    pub fn scale(mut self, s: f32) -> Self {
        self.scale = Some(s);
        self
    }
}

/// How visual changes animate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transition {
    /// A spring from the active motion profile (`motion.toml`).
    Motion(MotionClass),
    /// A fixed tween.
    Tween(Duration, Easing),
}

/// Base visual style.
#[derive(Clone, Debug, PartialEq)]
pub struct Visual {
    pub(crate) background: Option<Color>,
    pub(crate) border: Option<(f32, Color)>,
    pub(crate) radii: Corners,
    pub(crate) elevation: Option<Elevation>,
    pub(crate) material: Option<Material>,
    pub(crate) opacity: f32,
    pub(crate) translate: Vec2,
    pub(crate) scale: f32,
    pub(crate) transform: Transform2D,
    pub(crate) clip: bool,
    pub(crate) hover: Option<Refinement>,
    pub(crate) active: Option<Refinement>,
    pub(crate) focused: Option<Refinement>,
    pub(crate) transition: Option<Transition>,
    pub(crate) motion_scope: Option<SharedString>,
}

impl Default for Visual {
    fn default() -> Self {
        Visual {
            background: None,
            border: None,
            radii: Corners::default(),
            elevation: None,
            material: None,
            opacity: 1.0,
            translate: Vec2::ZERO,
            scale: 1.0,
            transform: Transform2D::IDENTITY,
            clip: false,
            hover: None,
            active: None,
            focused: None,
            transition: None,
            motion_scope: None,
        }
    }
}

/// Event phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Root to target.
    Capture,
    /// Target to root.
    Bubble,
}

/// Which pointer events a handler wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerFilter {
    /// Button down.
    Down,
    /// Button up.
    Up,
    /// Move.
    Move,
    /// Scroll.
    Scroll,
}

pub(crate) type PointerHandler = Rc<dyn Fn(&PointerEvent, &mut EventCx<'_>)>;
pub(crate) type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut EventCx<'_>)>;
pub(crate) type DragHandler = Rc<dyn Fn(&DragEvent, &mut EventCx<'_>)>;
pub(crate) type KeyHandler = Rc<dyn Fn(&KeyEvent, &mut EventCx<'_>)>;
pub(crate) type TextHandler = Rc<dyn Fn(&str, &mut EventCx<'_>)>;
pub(crate) type ImeHandler = Rc<dyn Fn(&ImeEvent, &mut EventCx<'_>)>;
pub(crate) type ActionHandler = Rc<dyn Fn(&mut EventCx<'_>)>;

/// Event handlers of one element.
#[derive(Default)]
pub(crate) struct Handlers {
    pub(crate) pointer: Vec<(PointerFilter, Phase, PointerHandler)>,
    pub(crate) click: Option<ClickHandler>,
    pub(crate) drag: Option<DragHandler>,
    pub(crate) key: Vec<(Phase, KeyHandler)>,
    pub(crate) text: Option<TextHandler>,
    pub(crate) ime: Option<ImeHandler>,
    pub(crate) actions: Vec<(SharedString, ActionHandler)>,
}

impl fmt::Debug for Handlers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handlers")
            .field("pointer", &self.pointer.len())
            .field("click", &self.click.is_some())
            .field("drag", &self.drag.is_some())
            .field("key", &self.key.len())
            .field(
                "actions",
                &self.actions.iter().map(|a| &*a.0).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// Text placeholder content (until `livspeil-ui-text` lands).
#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub(crate) text: SharedString,
    pub(crate) size: Option<f32>,
    pub(crate) color: Option<Color>,
}

#[derive(Debug)]
pub(crate) enum ElementKind {
    Div,
    Text(TextSpec),
    View(AnyView),
}

/// An element: layout style, visuals, handlers and children.
#[derive(Debug)]
pub struct Element {
    pub(crate) key: Option<SharedString>,
    pub(crate) name: Option<SharedString>,
    pub(crate) kind: ElementKind,
    pub(crate) style: Style,
    pub(crate) visual: Visual,
    pub(crate) children: Vec<Element>,
    pub(crate) handlers: Option<Box<Handlers>>,
    pub(crate) tab_index: Option<i32>,
    pub(crate) focus_scope: bool,
    pub(crate) key_context: Option<SharedString>,
    pub(crate) canvas: Option<Canvas>,
    pub(crate) hit_test: bool,
}

/// A container element.
#[must_use]
pub fn div() -> Element {
    Element::new(ElementKind::Div)
}

/// A text element (placeholder box sized by character count).
#[must_use]
pub fn text(text: impl Into<SharedString>) -> Element {
    Element::new(ElementKind::Text(TextSpec {
        text: text.into(),
        size: None,
        color: None,
    }))
}

impl<V: Render> From<View<V>> for Element {
    fn from(view: View<V>) -> Self {
        Element::new(ElementKind::View(view.into_any()))
    }
}

impl Element {
    fn new(kind: ElementKind) -> Self {
        Element {
            key: None,
            name: None,
            kind,
            style: Style::default(),
            visual: Visual::default(),
            children: Vec::new(),
            handlers: None,
            tab_index: None,
            focus_scope: false,
            key_context: None,
            canvas: None,
            hit_test: true,
        }
    }

    /// Embed a view; it renders separately and only when it is notified.
    #[must_use]
    pub fn view<V: Render>(view: &View<V>) -> Element {
        Element::from(*view)
    }

    // ---- identity ----

    /// Explicit key: the element keeps its id (state, focus, animations)
    /// when siblings are reordered.
    #[must_use]
    pub fn key(mut self, key: impl Into<SharedString>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// Debug name for dumps (defaults to the key or kind).
    #[must_use]
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = Some(name.into());
        self
    }

    // ---- children ----

    /// Add a child.
    #[must_use]
    pub fn child(mut self, child: impl Into<Element>) -> Self {
        self.children.push(child.into());
        self
    }

    /// Add children.
    #[must_use]
    pub fn children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
        self.children.extend(children);
        self
    }

    // ---- layout ----

    /// Edit the Taffy style directly (grid templates and anything else).
    #[must_use]
    pub fn style(mut self, f: impl FnOnce(&mut Style)) -> Self {
        f(&mut self.style);
        self
    }

    /// Fixed size.
    #[must_use]
    pub fn size(self, w: f32, h: f32) -> Self {
        self.w(w).h(h)
    }

    /// Fixed width.
    #[must_use]
    pub fn w(mut self, w: f32) -> Self {
        self.style.size.width = Dimension::length(w);
        self
    }

    /// Fixed height.
    #[must_use]
    pub fn h(mut self, h: f32) -> Self {
        self.style.size.height = Dimension::length(h);
        self
    }

    /// Full width of the parent.
    #[must_use]
    pub fn w_full(mut self) -> Self {
        self.style.size.width = Dimension::percent(1.0);
        self
    }

    /// Full height of the parent.
    #[must_use]
    pub fn h_full(mut self) -> Self {
        self.style.size.height = Dimension::percent(1.0);
        self
    }

    /// Padding on all sides.
    #[must_use]
    pub fn p(mut self, v: f32) -> Self {
        let l = LengthPercentage::length(v);
        self.style.padding = taffy::Rect {
            left: l,
            right: l,
            top: l,
            bottom: l,
        };
        self
    }

    /// Horizontal padding.
    #[must_use]
    pub fn px(mut self, v: f32) -> Self {
        let l = LengthPercentage::length(v);
        self.style.padding.left = l;
        self.style.padding.right = l;
        self
    }

    /// Vertical padding.
    #[must_use]
    pub fn py(mut self, v: f32) -> Self {
        let l = LengthPercentage::length(v);
        self.style.padding.top = l;
        self.style.padding.bottom = l;
        self
    }

    /// Gap between children.
    #[must_use]
    pub fn gap(mut self, v: f32) -> Self {
        self.style.gap = taffy::Size {
            width: length(v),
            height: length(v),
        };
        self
    }

    /// Lay children out in a row (the default).
    #[must_use]
    pub fn flex_row(mut self) -> Self {
        self.style.flex_direction = FlexDirection::Row;
        self
    }

    /// Lay children out in a column.
    #[must_use]
    pub fn flex_col(mut self) -> Self {
        self.style.flex_direction = FlexDirection::Column;
        self
    }

    /// Grow to fill free space.
    #[must_use]
    pub fn grow(mut self) -> Self {
        self.style.flex_grow = 1.0;
        self
    }

    /// Center children on the cross axis.
    #[must_use]
    pub fn items_center(mut self) -> Self {
        self.style.align_items = AlignItems::CENTER;
        self
    }

    /// Center children on the main axis.
    #[must_use]
    pub fn justify_center(mut self) -> Self {
        self.style.justify_content = JustifyContent::CENTER;
        self
    }

    /// Absolutely positioned at `rect` within the parent.
    #[must_use]
    pub fn absolute(mut self, rect: Rect) -> Self {
        self.style.position = Position::Absolute;
        self.style.inset.left = LengthPercentageAuto::length(rect.x);
        self.style.inset.top = LengthPercentageAuto::length(rect.y);
        self.size(rect.width, rect.height)
    }

    // ---- visuals ----

    /// Background color.
    #[must_use]
    pub fn bg(mut self, c: Color) -> Self {
        self.visual.background = Some(c);
        self
    }

    /// Inner border.
    #[must_use]
    pub fn border(mut self, width: f32, c: Color) -> Self {
        self.visual.border = Some((width, c));
        self
    }

    /// Uniform corner radius.
    #[must_use]
    pub fn rounded(mut self, r: f32) -> Self {
        self.visual.radii = Corners::all(r);
        self
    }

    /// Per-corner radii.
    #[must_use]
    pub fn radii(mut self, r: Corners) -> Self {
        self.visual.radii = r;
        self
    }

    /// Shadow.
    #[must_use]
    pub fn elevation(mut self, e: Elevation) -> Self {
        self.visual.elevation = Some(e);
        self
    }

    /// Surface material (recipe resolved from the theme and tier).
    #[must_use]
    pub fn material(mut self, m: Material) -> Self {
        self.visual.material = Some(m);
        self
    }

    /// Opacity of the element and its subtree.
    #[must_use]
    pub fn opacity(mut self, o: f32) -> Self {
        self.visual.opacity = o;
        self
    }

    /// Translation (animatable).
    #[must_use]
    pub fn translate(mut self, v: Vec2) -> Self {
        self.visual.translate = v;
        self
    }

    /// Uniform scale about the center (animatable).
    #[must_use]
    pub fn scale(mut self, s: f32) -> Self {
        self.visual.scale = s;
        self
    }

    /// An extra static transform about the center.
    #[must_use]
    pub fn transform(mut self, t: Transform2D) -> Self {
        self.visual.transform = t;
        self
    }

    /// Clip children to the bounds (and hit testing with them).
    #[must_use]
    pub fn clip(mut self) -> Self {
        self.visual.clip = true;
        self
    }

    /// Visual changes while hovered.
    #[must_use]
    pub fn hover(mut self, f: impl FnOnce(Refinement) -> Refinement) -> Self {
        self.visual.hover = Some(f(Refinement::default()));
        self
    }

    /// Visual changes while pressed.
    #[must_use]
    pub fn active(mut self, f: impl FnOnce(Refinement) -> Refinement) -> Self {
        self.visual.active = Some(f(Refinement::default()));
        self
    }

    /// Visual changes while focused from the keyboard.
    #[must_use]
    pub fn focus_visual(mut self, f: impl FnOnce(Refinement) -> Refinement) -> Self {
        self.visual.focused = Some(f(Refinement::default()));
        self
    }

    /// Animate visual changes with a spring of the motion profile.
    #[must_use]
    pub fn transition(mut self, class: MotionClass) -> Self {
        self.visual.transition = Some(Transition::Motion(class));
        self
    }

    /// Animate visual changes with a tween, or not at all (`None`).
    #[must_use]
    pub fn transition_with(mut self, t: Option<Transition>) -> Self {
        self.visual.transition = t;
        self
    }

    /// Motion scope (`[overrides.<scope>]` in `motion.toml`).
    #[must_use]
    pub fn motion_scope(mut self, scope: impl Into<SharedString>) -> Self {
        self.visual.motion_scope = Some(scope.into());
        self
    }

    /// Text size (text elements).
    #[must_use]
    pub fn font_size(mut self, size: f32) -> Self {
        if let ElementKind::Text(t) = &mut self.kind {
            t.size = Some(size);
        }
        self
    }

    /// Text color (text elements).
    #[must_use]
    pub fn text_color(mut self, c: Color) -> Self {
        if let ElementKind::Text(t) = &mut self.kind {
            t.color = Some(c);
        }
        self
    }

    // ---- behavior ----

    /// Children are laid out in canvas space and drawn through `canvas`'s
    /// pan/zoom, culled to the visible area.
    #[must_use]
    pub fn canvas(mut self, canvas: Canvas) -> Self {
        self.canvas = Some(canvas);
        self.visual.clip = true;
        self
    }

    /// Whether the element can be the target of pointer events (children
    /// still can). Default true.
    #[must_use]
    pub fn hit_test(mut self, on: bool) -> Self {
        self.hit_test = on;
        self
    }

    /// Focusable, in tab order (tab index 0).
    #[must_use]
    pub fn focusable(self) -> Self {
        self.tab_index(0)
    }

    /// Focusable with a tab index: positive indices come first in
    /// ascending order, then 0 in tree order; negative indices are
    /// focusable by pointer or code but skipped by Tab.
    #[must_use]
    pub fn tab_index(mut self, index: i32) -> Self {
        self.tab_index = Some(index);
        self
    }

    /// Tab traversal stays inside this subtree while focus is in it.
    #[must_use]
    pub fn focus_scope(mut self) -> Self {
        self.focus_scope = true;
        self
    }

    /// Key context for keymap bindings (`[contexts.<name>]`).
    #[must_use]
    pub fn key_context(mut self, name: impl Into<SharedString>) -> Self {
        self.key_context = Some(name.into());
        self
    }

    fn handlers_mut(&mut self) -> &mut Handlers {
        self.handlers.get_or_insert_with(Box::default)
    }

    /// Pointer handler for `filter` events in `phase`.
    #[must_use]
    pub fn on_pointer(
        mut self,
        filter: PointerFilter,
        phase: Phase,
        f: impl Fn(&PointerEvent, &mut EventCx<'_>) + 'static,
    ) -> Self {
        self.handlers_mut()
            .pointer
            .push((filter, phase, Rc::new(f)));
        self
    }

    /// Pointer down (bubble).
    #[must_use]
    pub fn on_pointer_down(self, f: impl Fn(&PointerEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.on_pointer(PointerFilter::Down, Phase::Bubble, f)
    }

    /// Pointer up (bubble).
    #[must_use]
    pub fn on_pointer_up(self, f: impl Fn(&PointerEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.on_pointer(PointerFilter::Up, Phase::Bubble, f)
    }

    /// Pointer move (bubble).
    #[must_use]
    pub fn on_pointer_move(self, f: impl Fn(&PointerEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.on_pointer(PointerFilter::Move, Phase::Bubble, f)
    }

    /// Scroll (bubble).
    #[must_use]
    pub fn on_scroll(self, f: impl Fn(&PointerEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.on_pointer(PointerFilter::Scroll, Phase::Bubble, f)
    }

    /// Click (and `ui.activate` when focused).
    #[must_use]
    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.handlers_mut().click = Some(Rc::new(f));
        self
    }

    /// Drag gesture (the deepest element with a drag handler under the
    /// pointer-down position receives it).
    #[must_use]
    pub fn on_drag(mut self, f: impl Fn(&DragEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.handlers_mut().drag = Some(Rc::new(f));
        self
    }

    /// Key handler in `phase` (focused path), before keymap actions.
    #[must_use]
    pub fn on_key(
        mut self,
        phase: Phase,
        f: impl Fn(&KeyEvent, &mut EventCx<'_>) + 'static,
    ) -> Self {
        self.handlers_mut().key.push((phase, Rc::new(f)));
        self
    }

    /// Text input (focused path, bubble).
    #[must_use]
    pub fn on_text_input(mut self, f: impl Fn(&str, &mut EventCx<'_>) + 'static) -> Self {
        self.handlers_mut().text = Some(Rc::new(f));
        self
    }

    /// IME events (focused path, bubble).
    #[must_use]
    pub fn on_ime(mut self, f: impl Fn(&ImeEvent, &mut EventCx<'_>) + 'static) -> Self {
        self.handlers_mut().ime = Some(Rc::new(f));
        self
    }

    /// Handle a named action (dispatched from the focused element upward).
    #[must_use]
    pub fn on_action(mut self, action: &str, f: impl Fn(&mut EventCx<'_>) + 'static) -> Self {
        self.handlers_mut()
            .actions
            .push((action.into(), Rc::new(f)));
        self
    }
}
