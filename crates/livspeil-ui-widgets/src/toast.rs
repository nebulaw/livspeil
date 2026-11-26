// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Toasts`]: a stack of transient notifications in a corner of the
//! window.

use std::rc::Rc;
use std::time::Duration;

use livspeil_ui_core::{
    App, Element, ElementId, Render, SharedString, View, ViewContext, actions as core, div, text,
};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Color, MotionClass, MotionSpec, PropertyKind, Theme};

use crate::button::Button;
use crate::env::{Env, LEAVE_TIME, LayerBuilder, OverlayKey, OverlayKind};
use crate::icon_button::{IconButton, IconSize};
use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};

/// How long a toast stays by default.
pub const TOAST_DURATION: Duration = Duration::from_secs(4);
/// Width of a toast (less in a narrow window).
pub const TOAST_WIDTH: f32 = 340.0;
/// Distance of the stack from the window's right and bottom edges.
pub const TOAST_MARGIN: f32 = 16.0;
/// Gap between stacked toasts.
pub const TOAST_GAP: f32 = 8.0;
/// Most toasts shown at once; the oldest leaves when another arrives.
pub const MAX_TOASTS: usize = 4;
/// How far a toast slides in from the right (px).
const SLIDE: f32 = 24.0;
/// Inner padding of a toast.
const PAD: f32 = 12.0;
/// Width of the dismiss button.
const CLOSE: f32 = 24.0;
/// Key of the stack's root element in the overlay layer.
const STACK_KEY: &str = "toasts";

/// What a toast is about; sets the color of its dot and how it is
/// announced (danger toasts are alerts, the others polite status
/// messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ToastTone {
    /// Neutral information (accent dot).
    #[default]
    Info,
    /// Something worked.
    Success,
    /// Something needs attention.
    Warning,
    /// Something failed.
    Danger,
}

impl ToastTone {
    fn color(self, theme: &Theme) -> Color {
        let c = theme.colors();
        match self {
            ToastTone::Info => theme.accent(),
            ToastTone::Success => c.success,
            ToastTone::Warning => c.warning,
            ToastTone::Danger => c.danger,
        }
    }
}

/// Identifies a shown toast.
pub type ToastId = u64;

type ActionFn = Rc<dyn Fn(&mut App)>;

/// A notification to show with [`Toasts::show`].
#[derive(Clone)]
pub struct Toast {
    title: Option<SharedString>,
    message: SharedString,
    tone: ToastTone,
    duration: Option<Duration>,
    action: Option<(SharedString, ActionFn)>,
}

impl std::fmt::Debug for Toast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toast")
            .field("title", &self.title)
            .field("message", &self.message)
            .field("tone", &self.tone)
            .field("duration", &self.duration)
            .finish_non_exhaustive()
    }
}

impl Toast {
    /// A toast with `message`, dismissed after [`TOAST_DURATION`].
    #[must_use]
    pub fn new(message: impl Into<SharedString>) -> Self {
        Toast {
            title: None,
            message: message.into(),
            tone: ToastTone::Info,
            duration: Some(TOAST_DURATION),
            action: None,
        }
    }

    /// A bold first line.
    #[must_use]
    pub fn title(mut self, t: impl Into<SharedString>) -> Self {
        self.title = Some(t.into());
        self
    }

    /// The tone.
    #[must_use]
    pub fn tone(mut self, t: ToastTone) -> Self {
        self.tone = t;
        self
    }

    /// How long it stays (the countdown pauses while the pointer is over
    /// it or focus is on its buttons).
    #[must_use]
    pub fn duration(mut self, d: Duration) -> Self {
        self.duration = Some(d);
        self
    }

    /// Stay until dismissed.
    #[must_use]
    pub fn sticky(mut self) -> Self {
        self.duration = None;
        self
    }

    /// An action button (for example Undo): runs `f` and dismisses the
    /// toast.
    #[must_use]
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        f: impl Fn(&mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), Rc::new(f)));
        self
    }

    /// Semantics: `Alert` for danger, else `Status`; named by the title or
    /// message, described by the message when there is a title.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        let role = if self.tone == ToastTone::Danger {
            Role::Alert
        } else {
            Role::Status
        };
        Semantics::new(role).label(&*self.title.clone().unwrap_or_else(|| self.message.clone()))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Rendered once in the hidden look.
    Entering,
    Shown,
    /// Animating out (seconds so far).
    Leaving(f32),
}

struct Item {
    id: ToastId,
    toast: Toast,
    /// Seconds left; `None` for sticky toasts.
    remaining: Option<f32>,
    phase: Phase,
}

/// The toast stack: a view in the overlay layer, created once per app with
/// [`Toasts::new`].
///
/// - New toasts slide in from the right edge and fade in at the bottom of
///   the stack (the newest is lowest); the others move up with the
///   standard spring. Leaving toasts fade and slide out. Under Reduce
///   Motion the movement is instant and only the fades remain.
/// - Each timed toast counts down its [`Toast::duration`] with the frame
///   clock; the countdown pauses while the pointer is over the toast or
///   focus is on one of its buttons, and resumes afterwards. Sticky toasts
///   stay until dismissed.
/// - Dismiss with the close button (`Dismiss`), Escape while focus is in a
///   toast, [`Toasts::dismiss`] or [`Toasts::clear`]; the optional action
///   button runs its callback and dismisses.
/// - At most [`MAX_TOASTS`] are shown; the oldest leaves first.
/// - For assistive technology each toast is a live region: `Status`
///   (polite), or `Alert` (assertive) for danger.
///
/// Toasts are named `toast` (key `toast-<id>`), buttons `button:<action>`
/// and `icon-button:Dismiss`. While any timed toast is on screen the
/// stack renders every frame; with none it is idle.
pub struct Toasts {
    env: Env,
    items: Vec<Item>,
    next_id: ToastId,
}

impl std::fmt::Debug for Toasts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toasts")
            .field("count", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl Toasts {
    /// Create the stack and mount it in the overlay layer of `env`.
    pub fn new(env: &Env, app: &mut App) -> View<Toasts> {
        let view = app.new_view(Toasts {
            env: env.clone(),
            items: Vec::new(),
            next_id: 1,
        });
        let build: LayerBuilder = Rc::new(move |_, _| vec![Element::view(&view).key(STACK_KEY)]);
        env.show_overlay(app, OverlayKey::Popup(view.id()), OverlayKind::Layer(build));
        view
    }

    /// Show `toast`; returns its id.
    pub fn show(view: View<Toasts>, app: &mut App, toast: Toast) -> ToastId {
        view.update(app, |t, cx| {
            let id = t.next_id;
            t.next_id += 1;
            t.items.push(Item {
                id,
                remaining: toast.duration.map(|d| d.as_secs_f32()),
                toast,
                phase: Phase::Entering,
            });
            // Too many: the oldest still-visible toasts leave.
            let live: Vec<usize> = t
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| !matches!(i.phase, Phase::Leaving(_)))
                .map(|(k, _)| k)
                .collect();
            let extra = live.len().saturating_sub(MAX_TOASTS);
            for k in live.into_iter().take(extra) {
                t.items[k].phase = Phase::Leaving(0.0);
            }
            cx.notify();
            id
        })
    }

    /// Dismiss toast `id` (it fades out). Unknown ids are ignored.
    pub fn dismiss(view: View<Toasts>, app: &mut App, id: ToastId) {
        view.update(app, |t, cx| {
            if let Some(i) = t.items.iter_mut().find(|i| i.id == id)
                && !matches!(i.phase, Phase::Leaving(_))
            {
                i.phase = Phase::Leaving(0.0);
                cx.notify();
            }
        });
    }

    /// Dismiss every toast.
    pub fn clear(view: View<Toasts>, app: &mut App) {
        view.update(app, |t, cx| {
            for i in &mut t.items {
                if !matches!(i.phase, Phase::Leaving(_)) {
                    i.phase = Phase::Leaving(0.0);
                }
            }
            cx.notify();
        });
    }

    /// Ids of the toasts that have not started leaving, oldest first.
    #[must_use]
    pub fn active(&self) -> Vec<ToastId> {
        self.items
            .iter()
            .filter(|i| !matches!(i.phase, Phase::Leaving(_)))
            .map(|i| i.id)
            .collect()
    }

    /// Seconds left of toast `id` (`None` if unknown or sticky).
    #[must_use]
    pub fn remaining(&self, id: ToastId) -> Option<f32> {
        self.items.iter().find(|i| i.id == id)?.remaining
    }

    /// Size of a toast in `theme` (text measured and wrapped).
    fn measure(&self, theme: &Theme, width: f32, toast: &Toast) -> (f32, f32) {
        let m = metrics(theme);
        let action_w = toast.action.as_ref().map_or(0.0, |(l, _)| {
            style::measure_text(&self.env, l, m.font, 500.0).0 + 2.0 * m.pad_x + TOAST_GAP
        });
        let text_w = width - 2.0 * PAD - 8.0 - 8.0 - action_w - CLOSE - 4.0;
        let mut h = 0.0;
        if let Some(t) = &toast.title {
            h += style::measure_wrapped(&self.env, t, m.font, 600.0, text_w);
        }
        h += style::measure_wrapped(&self.env, &toast.message, m.font, 400.0, text_w);
        (text_w, h.max(m.height) + 2.0 * PAD)
    }
}

fn dismiss_action(view: View<Toasts>, id: ToastId) -> impl Fn(&mut livspeil_ui_core::EventCx<'_>) {
    move |cx| Toasts::dismiss(view, cx, id)
}

impl Render for Toasts {
    #[allow(clippy::too_many_lines)]
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let vp = cx.viewport();
        let dt = cx.frame_delta();
        let instant = matches!(
            cx.motion()
                .resolve(None, MotionClass::Standard, PropertyKind::Appearance),
            MotionSpec::Instant
        );
        let pointer = cx.pointer_position();
        let width = TOAST_WIDTH.min(vp.width - 2.0 * TOAST_MARGIN).max(200.0);
        let x = vp.width - TOAST_MARGIN - width;
        let sizes: Vec<(f32, f32)> = self
            .items
            .iter()
            .map(|i| self.measure(&theme, width, &i.toast))
            .collect();
        // Where each toast rests (the tree cannot be queried while
        // rendering, so hover is the pointer against these rectangles).
        let mut rects = vec![Rect::ZERO; self.items.len()];
        let mut lift = 0.0;
        for k in (0..self.items.len()).rev() {
            let h = sizes[k].1;
            rects[k] = Rect::new(x, vp.height - TOAST_MARGIN - h - lift, width, h);
            lift += h + TOAST_GAP;
        }
        // The look of this render (before the phases advance below).
        let looks: Vec<Phase> = self.items.iter().map(|i| i.phase).collect();

        // Advance: timers (paused under the pointer or focus), exits.
        let mut again = false;
        for (k, item) in self.items.iter_mut().enumerate() {
            match item.phase {
                Phase::Entering => {
                    item.phase = Phase::Shown;
                    again = true;
                }
                Phase::Shown => {
                    if let Some(rem) = &mut item.remaining {
                        let paused = pointer.is_some_and(|p| rects[k].contains(p))
                            || self
                                .env
                                .is_focused(cx, &format!("toast-action:{}", item.id))
                            || self.env.is_focused(cx, &format!("toast-close:{}", item.id));
                        if !paused {
                            *rem -= dt;
                            if *rem <= 0.0 {
                                item.phase = Phase::Leaving(0.0);
                            }
                        }
                        again = true;
                    }
                }
                Phase::Leaving(t) => {
                    item.phase = Phase::Leaving(t + dt);
                    again = true;
                }
            }
        }
        let keep: Vec<bool> = self
            .items
            .iter()
            .map(|i| match i.phase {
                Phase::Leaving(t) => !(instant || t >= LEAVE_TIME),
                _ => true,
            })
            .collect();
        let mut k = 0;
        self.items.retain(|_| {
            k += 1;
            keep[k - 1]
        });
        if again {
            cx.notify();
        }
        // Items that were removed are not drawn (their looks are dropped
        // with them).
        let looks: Vec<Phase> = looks
            .into_iter()
            .zip(&keep)
            .filter_map(|(l, k)| k.then_some(l))
            .collect();
        let sizes: Vec<(f32, f32)> = sizes
            .into_iter()
            .zip(&keep)
            .filter_map(|(r, k)| k.then_some(r))
            .collect();
        let mut root = div()
            .name("toast-stack")
            .absolute(Rect::new(0.0, 0.0, vp.width, vp.height))
            .hit_test(false)
            .semantics(|s| s.role(livspeil_ui_core::Role::Group).label("Notifications"));
        // The newest toast is lowest; older ones are offset upwards.
        let mut lift = 0.0;
        for (k, item) in self.items.iter().enumerate().rev() {
            let (text_w, h) = sizes[k];
            let hidden = !matches!(looks[k], Phase::Shown);
            let leaving = matches!(looks[k], Phase::Leaving(_));
            let id = item.id;
            let t = &item.toast;
            let sem = t.semantics();
            let mut col = div()
                .key("text")
                .flex_col()
                .w(text_w)
                .style(|s| s.flex_shrink = 0.0);
            if let Some(title) = &t.title {
                col = col.child(
                    text(title.clone())
                        .key("title")
                        .font_size(m.font)
                        .font_weight(600.0)
                        .text_color(c.text_primary)
                        .hit_test(false),
                );
            }
            col = col.child(
                text(t.message.clone())
                    .key("message")
                    .font_size(m.font)
                    .text_color(if t.title.is_some() {
                        c.text_secondary
                    } else {
                        c.text_primary
                    })
                    .hit_test(false),
            );
            let mut el = div()
                .key(format!("toast-{id}"))
                .name("toast")
                .absolute(Rect::new(x, vp.height - TOAST_MARGIN - h, width, h))
                .flex_row()
                .items_center()
                .gap(8.0)
                .px(PAD)
                .rounded(theme.tokens.radii.md + 2.0)
                .material(livspeil_ui_theme::Material::Glass)
                .opacity(if hidden { 0.0 } else { 1.0 })
                .translate(Vec2::new(if hidden { SLIDE } else { 0.0 }, -lift))
                .transition(MotionClass::Standard)
                .child(
                    div()
                        .key("dot")
                        .size(8.0, 8.0)
                        .style(|s| s.flex_shrink = 0.0)
                        .rounded(4.0)
                        .bg(t.tone.color(&theme))
                        .hit_test(false),
                )
                .child(col);
            el = el.child(div().grow().hit_test(false));
            if let Some((label, f)) = &t.action {
                let f = f.clone();
                el = el.child(
                    Button::new(label.clone())
                        .ghost()
                        .disabled(leaving)
                        .on_click(move |_, cx| {
                            f(cx);
                            Toasts::dismiss(view, cx, id);
                        })
                        .render(&theme)
                        .name(format!("toast-action:{id}"))
                        .key("action"),
                );
            }
            el = el.child(
                IconButton::new("bx-x", "Dismiss")
                    .size(IconSize::S14)
                    .disabled(leaving)
                    .on_click(move |_, cx| Toasts::dismiss(view, cx, id))
                    .render(&theme)
                    .name(format!("toast-close:{id}"))
                    .key("close"),
            );
            el = sem.apply(el);
            if let Some(msg) = t.title.as_ref().map(|_| t.message.clone()) {
                el = el.semantics(move |s| s.description(msg.clone()));
            }
            el = if leaving {
                el.hit_test(false)
            } else {
                el.on_action(core::CANCEL, dismiss_action(view, id))
            };
            root = root.child(el);
            lift += h + TOAST_GAP;
        }
        root
    }
}

/// The element id a toast has (for tests and hosts), if the stack is
/// mounted.
#[must_use]
pub fn toast_element(env: &Env, app: &App, id: ToastId) -> Option<ElementId> {
    env.overlay_child(app, STACK_KEY)
        .map(|s| s.child(&format!("toast-{id}")))
}
