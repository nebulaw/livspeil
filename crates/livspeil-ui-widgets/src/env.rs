// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Env`]: what stateful widgets share (fonts, clipboard, focus tracking,
//! the overlay layer) and [`WidgetRoot`], the root view that hosts popups
//! and tooltips above the content.

use std::cell::{RefCell, RefMut};
use std::fmt;
use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, ElementId, EntityId, EventCx, Model, Phase, PointerFilter, Render, View,
    ViewContext, actions, div,
};
use livspeil_ui_layout::{Style, taffy};
use livspeil_ui_text::FontSystem;

use crate::keymap;
use crate::tooltip;

/// The system clipboard as seen by text widgets. Hosts implement it over
/// the platform clipboard; [`MemoryClipboard`] is the headless default.
pub trait Clipboard {
    /// The clipboard text, if any.
    fn read_text(&self) -> Option<String>;
    /// Replace the clipboard text.
    fn write_text(&mut self, text: &str);
}

/// An in-process clipboard.
#[derive(Clone, Debug, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl Clipboard for MemoryClipboard {
    fn read_text(&self) -> Option<String> {
        self.text.clone()
    }
    fn write_text(&mut self, text: &str) {
        self.text = Some(text.to_owned());
    }
}

/// Key of an overlay entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OverlayKey {
    /// A popup owned by a view.
    Popup(EntityId),
    /// A tooltip anchored at an element.
    Tooltip(ElementId),
}

/// Where an overlay entry is in its appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OverlayPhase {
    /// Waiting for a delay (seconds elapsed so far).
    Pending(f32),
    /// Rendered once in its initial (hidden) look so the next render
    /// animates it in.
    Appearing,
    /// Fully shown.
    Shown,
}

/// Builds a popup's elements in window coordinates (direct, absolutely
/// positioned children of the overlay layer); `appearing` is true for the
/// first render (initial look).
///
/// They are not wrapped in a common absolute container: taffy 0.16 keeps
/// stale hoisted keys when an absolute subtree containing further absolute
/// boxes is removed.
pub(crate) type PopupBuilder = Rc<dyn Fn(&App, bool) -> Vec<Element>>;

#[derive(Clone)]
pub(crate) enum OverlayKind {
    Popup(PopupBuilder),
    Tooltip(tooltip::TooltipEntry),
}

#[derive(Clone)]
pub(crate) struct OverlayEntry {
    pub(crate) key: OverlayKey,
    pub(crate) kind: OverlayKind,
    pub(crate) phase: OverlayPhase,
}

/// Popups and tooltips currently in the overlay layer.
#[derive(Clone, Default)]
pub(crate) struct OverlayState {
    pub(crate) entries: Vec<OverlayEntry>,
}

impl fmt::Debug for OverlayState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|e| (e.key, e.phase)))
            .finish()
    }
}

struct EnvInner {
    fonts: RefCell<FontSystem>,
    clipboard: RefCell<Box<dyn Clipboard>>,
    overlay: Model<OverlayState>,
    /// The focused element and its name, as of the last sync.
    focus: Model<Option<(ElementId, String)>>,
}

/// Shared widget environment. Cheap to clone (a reference-counted handle).
///
/// Create one per app with [`Env::new`] (or [`Env::headless`] for tests),
/// which also installs the widget keymap, and mount the content through
/// [`Env::root`] so popups, tooltips and focus tracking work.
#[derive(Clone)]
pub struct Env {
    inner: Rc<EnvInner>,
}

impl fmt::Debug for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Env")
            .field("overlay", &self.inner.overlay)
            .field("focus", &self.inner.focus)
            .finish_non_exhaustive()
    }
}

impl Env {
    /// An environment using `fonts`; installs the widget keymap
    /// ([`crate::keymap()`]) into `app`.
    pub fn new(app: &mut App, fonts: FontSystem) -> Env {
        let mut km = app.keymap().clone();
        km.extend(&keymap::keymap());
        app.set_keymap(km);
        let overlay = app.new_model(OverlayState::default());
        let focus = app.new_model(None);
        Env {
            inner: Rc::new(EnvInner {
                fonts: RefCell::new(fonts),
                clipboard: RefCell::new(Box::new(MemoryClipboard::default())),
                overlay,
                focus,
            }),
        }
    }

    /// Deterministic environment for tests: bundled fonts only and an
    /// in-memory clipboard.
    pub fn headless(app: &mut App) -> Env {
        Env::new(app, FontSystem::test())
    }

    /// Fonts for text layout.
    ///
    /// # Panics
    /// If the font system is already borrowed (re-entrant layout).
    #[must_use]
    pub fn fonts(&self) -> RefMut<'_, FontSystem> {
        self.inner.fonts.borrow_mut()
    }

    /// Replace the clipboard (hosts install the platform clipboard).
    pub fn set_clipboard(&self, clipboard: Box<dyn Clipboard>) {
        *self.inner.clipboard.borrow_mut() = clipboard;
    }

    /// Clipboard text.
    #[must_use]
    pub fn clipboard_text(&self) -> Option<String> {
        self.inner.clipboard.borrow().read_text()
    }

    /// Write clipboard text.
    pub fn set_clipboard_text(&self, text: &str) {
        self.inner.clipboard.borrow_mut().write_text(text);
    }

    /// Wrap `content` in a [`WidgetRoot`]; make the result the app root.
    pub fn root<V: Render>(&self, app: &mut App, content: View<V>) -> View<WidgetRoot> {
        app.new_view(WidgetRoot {
            env: self.clone(),
            content: Rc::new(move || Element::view(&content)),
        })
    }

    /// Focus `id` and let focus-aware widgets know.
    pub fn focus(&self, app: &mut App, id: ElementId) {
        app.focus(id);
        self.sync_focus(app);
    }

    /// Tell focus-aware widgets (text fields show a caret, number fields
    /// commit on blur) that focus may have changed. [`WidgetRoot`] calls
    /// this after pointer presses and Tab traversal.
    pub fn sync_focus(&self, app: &mut App) {
        let now = app.focused().map(|f| (f, app.label(f).unwrap_or_default()));
        let model = self.inner.focus;
        if *model.read(app) != now {
            model.update(app, |f, cx| {
                *f = now;
                cx.notify();
            });
        }
    }

    /// Whether the element named `name` is focused (as of the last
    /// [`Env::sync_focus`]; the tree cannot be queried during `render`).
    /// Reading this in `render` re-renders the view when focus changes.
    pub(crate) fn is_focused(&self, app: &App, name: &str) -> bool {
        match self.inner.focus.read(app) {
            Some((id, label)) => app.focused() == Some(*id) && label == name,
            None => false,
        }
    }

    pub(crate) fn overlay(&self) -> Model<OverlayState> {
        self.inner.overlay
    }

    /// Add or replace an overlay entry.
    pub(crate) fn show_overlay(&self, app: &mut App, key: OverlayKey, kind: OverlayKind) {
        let delay = match &kind {
            OverlayKind::Tooltip(t) => Some(t.delay),
            OverlayKind::Popup(_) => None,
        };
        self.inner.overlay.update(app, |s, cx| {
            let phase = match delay {
                Some(d) if d > 0.0 => OverlayPhase::Pending(0.0),
                _ => OverlayPhase::Appearing,
            };
            let entry = OverlayEntry { key, kind, phase };
            match s.entries.iter_mut().find(|e| e.key == key) {
                Some(e) => {
                    e.kind = entry.kind;
                }
                None => s.entries.push(entry),
            }
            cx.notify();
        });
    }

    /// Remove an overlay entry.
    pub(crate) fn hide_overlay(&self, app: &mut App, key: OverlayKey) {
        self.inner.overlay.update(app, |s, cx| {
            let before = s.entries.len();
            s.entries.retain(|e| e.key != key);
            if s.entries.len() != before {
                cx.notify();
            }
        });
    }

    /// Whether an overlay entry exists (shown or pending).
    #[must_use]
    pub(crate) fn has_overlay(&self, app: &App, key: OverlayKey) -> bool {
        self.inner
            .overlay
            .read(app)
            .entries
            .iter()
            .any(|e| e.key == key)
    }

    /// Whether a tooltip is shown (not just pending) for `anchor`.
    #[must_use]
    pub fn tooltip_shown(&self, app: &App, anchor: ElementId) -> bool {
        self.inner.overlay.read(app).entries.iter().any(|e| {
            e.key == OverlayKey::Tooltip(anchor) && !matches!(e.phase, OverlayPhase::Pending(_))
        })
    }

    fn hide_tooltips(&self, cx: &mut EventCx<'_>, keep: impl Fn(&App, ElementId) -> bool) {
        let gone: Vec<OverlayKey> = self
            .inner
            .overlay
            .read(cx)
            .entries
            .iter()
            .filter_map(|e| match e.key {
                OverlayKey::Tooltip(a) if !keep(cx, a) => Some(e.key),
                _ => None,
            })
            .collect();
        for k in gone {
            self.hide_overlay(cx, k);
        }
    }
}

/// Place a child in the single cell of the root grid.
fn cell(s: &mut Style) {
    let one = || taffy::Line {
        start: taffy::style_helpers::line(1),
        end: taffy::GridPlacement::Auto,
    };
    s.grid_row = one();
    s.grid_column = one();
}

/// The root view: content plus the overlay layer (popups and tooltips are
/// painted and hit-tested above everything else), and focus tracking for
/// focus-aware widgets.
pub struct WidgetRoot {
    env: Env,
    content: Rc<dyn Fn() -> Element>,
}

impl fmt::Debug for WidgetRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WidgetRoot").finish_non_exhaustive()
    }
}

impl Render for WidgetRoot {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let env = self.env.clone();
        let model = env.overlay();
        let entries = model.read(cx).entries.clone();
        let viewport = cx.viewport();
        let dt = cx.frame_delta();

        let mut layer = Vec::with_capacity(entries.len());
        for e in &entries {
            let appearing = e.phase == OverlayPhase::Appearing;
            match (&e.kind, e.phase) {
                (_, OverlayPhase::Pending(_)) => {}
                (OverlayKind::Popup(build), _) => layer.extend(build(cx, appearing)),
                (OverlayKind::Tooltip(t), _) => {
                    layer.push(tooltip::bubble(&env, cx, e.key, t, appearing));
                }
            }
        }

        // Advance phases; keep rendering while anything is pending or
        // appearing (a pending tooltip polls once per frame until its delay
        // has passed, then the layer is idle again).
        let mut again = false;
        model.update(cx, |s, _| {
            for e in &mut s.entries {
                e.phase = match (e.phase, &e.kind) {
                    (OverlayPhase::Pending(t), OverlayKind::Tooltip(tip)) => {
                        let t = t + dt;
                        if t >= tip.delay {
                            OverlayPhase::Appearing
                        } else {
                            OverlayPhase::Pending(t)
                        }
                    }
                    (OverlayPhase::Pending(_) | OverlayPhase::Appearing, _) => OverlayPhase::Shown,
                    (OverlayPhase::Shown, _) => OverlayPhase::Shown,
                };
                again |= e.phase != OverlayPhase::Shown;
            }
            // Entries that were appearing this render need one more.
            again |= entries.iter().any(|e| e.phase != OverlayPhase::Shown);
        });
        if again {
            cx.notify();
        }

        let (e1, e2, e3, e4, e5) = (env.clone(), env.clone(), env.clone(), env.clone(), env);
        div()
            .name("widget-root")
            .w_full()
            .h_full()
            .style(|s| {
                // A 1×1 grid: content and the overlay layer share the cell,
                // so the overlay covers the content without being absolutely
                // positioned (taffy 0.16 keeps stale hoisted keys when an
                // absolute child of an absolute box is removed).
                s.display = taffy::Display::Grid;
                s.grid_template_rows = vec![taffy::GridTemplateComponent::Single(
                    taffy::style_helpers::fr(1.0),
                )];
                s.grid_template_columns = vec![taffy::GridTemplateComponent::Single(
                    taffy::style_helpers::fr(1.0),
                )];
            })
            .tab_index(-1)
            .on_pointer(PointerFilter::Down, Phase::Capture, move |_, cx| {
                e1.sync_focus(cx);
                e1.hide_tooltips(cx, |_, _| false);
            })
            .on_pointer(PointerFilter::Up, Phase::Capture, move |_, cx| {
                e2.sync_focus(cx);
            })
            .on_pointer(PointerFilter::Move, Phase::Capture, move |_, cx| {
                e3.hide_tooltips(cx, |app, anchor| app.is_hovered(anchor));
            })
            .on_key(Phase::Bubble, move |_, cx| e4.sync_focus(cx))
            .on_action(actions::FOCUS_NEXT, {
                let env = e5.clone();
                move |cx| {
                    cx.focus_next();
                    env.sync_focus(cx);
                }
            })
            .on_action(actions::FOCUS_PREV, move |cx| {
                cx.focus_prev();
                e5.sync_focus(cx);
            })
            .child(
                div()
                    .key("content")
                    .style(cell)
                    .w_full()
                    .h_full()
                    .hit_test(false)
                    .child((self.content)()),
            )
            .child(
                div()
                    .key("overlay")
                    .style(cell)
                    .w(viewport.width)
                    .h(viewport.height)
                    .hit_test(false)
                    .children(layer),
            )
    }
}
