// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Dialog`]: a modal dialog, or a sheet attached to the window's top
//! edge, over a scrim in the overlay layer.

use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, ElementId, Render, SharedString, View, ViewContext, actions as core, div, text,
};
use livspeil_ui_layout::style::{JustifyContent, length};
use livspeil_ui_layout::{Corners, Rect, Vec2};
use livspeil_ui_theme::{Color, Elevation, MotionClass, Scheme, Theme};

use crate::button::Button;
use crate::env::{Env, LayerBuilder, Look, OverlayKey, OverlayKind};
use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};

/// Default width of a dialog.
pub const DIALOG_WIDTH: f32 = 340.0;
/// Default width of a sheet.
pub const SHEET_WIDTH: f32 = 420.0;
/// How far a dialog slides in (px); sheets slide in from above the window.
const SLIDE: f32 = 8.0;
/// Margin kept between a dialog and the window edges.
const MARGIN: f32 = 16.0;

type PressFn = Rc<dyn Fn(&mut App)>;
type ContentFn = Rc<dyn Fn(&App) -> Element>;

/// A button of a [`Dialog`].
#[derive(Clone)]
pub struct DialogButton {
    label: SharedString,
    default: bool,
    destructive: bool,
    cancel: bool,
    disabled: bool,
    on_press: Option<PressFn>,
}

impl std::fmt::Debug for DialogButton {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogButton")
            .field("label", &self.label)
            .field("default", &self.default)
            .field("destructive", &self.destructive)
            .field("cancel", &self.cancel)
            .finish_non_exhaustive()
    }
}

impl DialogButton {
    /// A plain button.
    #[must_use]
    pub fn new(label: impl Into<SharedString>) -> Self {
        DialogButton {
            label: label.into(),
            default: false,
            destructive: false,
            cancel: false,
            disabled: false,
            on_press: None,
        }
    }

    /// The default button: filled with the accent, focused when the dialog
    /// opens, and pressed by Enter while focus is not on another button.
    #[must_use]
    pub fn default_button(mut self) -> Self {
        self.default = true;
        self
    }

    /// A destructive action (filled with the danger color). Combine with
    /// [`DialogButton::default_button`] only when Enter should do it.
    #[must_use]
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// The cancel button: pressed by Escape.
    #[must_use]
    pub fn cancel(mut self) -> Self {
        self.cancel = true;
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    /// Run `f` after the dialog closed because this button was pressed.
    #[must_use]
    pub fn on_press(mut self, f: impl Fn(&mut App) + 'static) -> Self {
        self.on_press = Some(Rc::new(f));
        self
    }
}

/// A modal dialog or a sheet.
///
/// - Opens over a scrim that takes every pointer press (the content below
///   is inert); the panel has a title, an optional wrapped message,
///   optional custom content ([`Dialog::content`], given a height) and a
///   right-aligned row of [`DialogButton`]s.
/// - **Focus trap:** keyboard focus moves to the default button (or the
///   panel) when it opens and Tab/Shift+Tab cycle inside it; closing
///   returns focus to what had it before.
/// - **Keys:** Enter presses the default button (when focus is not on a
///   button: then Enter/Space press that one), Escape presses the cancel
///   button, or just closes when there is none. Any button closes the
///   dialog, then runs its [`DialogButton::on_press`].
/// - [`Dialog::sheet`]: attached to the window's top edge (slides down
///   from above, bottom corners rounded); otherwise centered, fading in
///   and sliding 8 px. Neither scales; both fade out when closing. Under
///   Reduce Motion the slides are dropped and the fade stays.
///
/// Open and close with [`Dialog::open`] / [`Dialog::close`]. The panel is
/// named `dialog` (or `sheet`), the scrim `dialog-scrim`, buttons
/// `button:<label>`. Assistive technology sees a `Dialog` named by the
/// title and described by the message.
pub struct Dialog {
    env: Env,
    title: SharedString,
    message: Option<SharedString>,
    content: Option<(f32, ContentFn)>,
    buttons: Vec<DialogButton>,
    sheet: bool,
    width: Option<f32>,
    dismiss_on_scrim: bool,
    open: bool,
    restore: Option<ElementId>,
}

impl std::fmt::Debug for Dialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dialog")
            .field("title", &self.title)
            .field("sheet", &self.sheet)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl Dialog {
    /// A centered dialog titled `title`.
    #[must_use]
    pub fn new(env: &Env, title: impl Into<SharedString>) -> Self {
        Dialog {
            env: env.clone(),
            title: title.into(),
            message: None,
            content: None,
            buttons: Vec::new(),
            sheet: false,
            width: None,
            dismiss_on_scrim: false,
            open: false,
            restore: None,
        }
    }

    /// Attach to the window's top edge instead of floating in the center.
    #[must_use]
    pub fn sheet(mut self) -> Self {
        self.sheet = true;
        self
    }

    /// Explanatory text under the title (wraps).
    #[must_use]
    pub fn message(mut self, m: impl Into<SharedString>) -> Self {
        self.message = Some(m.into());
        self
    }

    /// Custom content of `height` px between the message and the buttons.
    #[must_use]
    pub fn content(mut self, height: f32, f: impl Fn(&App) -> Element + 'static) -> Self {
        self.content = Some((height, Rc::new(f)));
        self
    }

    /// Add a button (shown left to right, so the default goes last).
    #[must_use]
    pub fn button(mut self, b: DialogButton) -> Self {
        self.buttons.push(b);
        self
    }

    /// Panel width (default 340 for dialogs, 420 for sheets).
    #[must_use]
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    /// Whether a press on the scrim acts like Escape (default off: dialogs
    /// are modal).
    #[must_use]
    pub fn dismiss_on_scrim(mut self, on: bool) -> Self {
        self.dismiss_on_scrim = on;
        self
    }

    /// Whether the dialog is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The buttons.
    #[must_use]
    pub fn buttons(&self) -> &[DialogButton] {
        &self.buttons
    }

    /// Semantics of the panel: a `Dialog` named by the title.
    #[must_use]
    pub fn semantics(&self) -> Semantics {
        Semantics::new(Role::Dialog).label(&*self.title)
    }

    fn panel_width(&self, vp_w: f32) -> f32 {
        self.width
            .unwrap_or(if self.sheet {
                SHEET_WIDTH
            } else {
                DIALOG_WIDTH
            })
            .min(vp_w - 2.0 * MARGIN)
            .max(160.0)
    }

    /// Panel height for the theme (measured text plus fixed parts).
    fn panel_height(&self, theme: &Theme, width: f32) -> f32 {
        let m = metrics(theme);
        let pad = theme.tokens.spacing.step(4);
        let inner = width - 2.0 * pad;
        let (tsize, _) = title_size(theme);
        let mut h = 2.0 * pad + style::measure_wrapped(&self.env, &self.title, tsize, 600.0, inner);
        if let Some(msg) = &self.message {
            h += 6.0 + style::measure_wrapped(&self.env, msg, m.font, 400.0, inner);
        }
        if let Some((ch, _)) = &self.content {
            h += 12.0 + ch;
        }
        if !self.buttons.is_empty() {
            h += pad + m.height;
        }
        h
    }

    /// Open (or re-open while it is fading out). Focus moves into the
    /// panel; `restore` (default: what has focus now) gets it back.
    pub fn open(view: View<Dialog>, app: &mut App, restore: Option<ElementId>) {
        let focused = restore.or_else(|| app.focused());
        let (env, default) = view.update(app, |d, cx| {
            if !d.open {
                d.restore = focused;
            }
            d.open = true;
            cx.notify();
            (
                d.env.clone(),
                d.buttons.iter().position(|b| b.default && !b.disabled),
            )
        });
        let build: LayerBuilder = Rc::new(move |app, look| layer(view, app, look));
        env.show_overlay(app, OverlayKey::Popup(view.id()), OverlayKind::Layer(build));
        if let Some(wrap) = env.overlay_child(app, &layer_key(view)) {
            let panel = wrap.child("panel");
            let target = match default {
                Some(i) => panel.child("buttons").child(&button_key(i)),
                None => panel,
            };
            env.focus(app, target);
        }
    }

    /// Close without pressing any button (fades out, returns focus).
    pub fn close(view: View<Dialog>, app: &mut App) {
        let (env, restore, was_open) = view.update(app, |d, cx| {
            let was = std::mem::replace(&mut d.open, false);
            cx.notify();
            (d.env.clone(), d.restore.take(), was)
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
    }

    /// Press button `i` as a click does: close, then run its callback.
    pub fn press(view: View<Dialog>, app: &mut App, i: usize) {
        let Some(b) = view.read(app).buttons.get(i).cloned() else {
            return;
        };
        if b.disabled || !view.read(app).open {
            return;
        }
        Dialog::close(view, app);
        if let Some(f) = b.on_press {
            f(app);
        }
    }

    /// What Escape (and a dismissing scrim press) does.
    fn cancel(view: View<Dialog>, app: &mut App) {
        let cancel = view.read(app).buttons.iter().position(|b| b.cancel);
        match cancel {
            Some(i) => Dialog::press(view, app, i),
            None => Dialog::close(view, app),
        }
    }
}

fn title_size(theme: &Theme) -> (f32, f32) {
    let s = theme.tokens.type_scale.sizes[3];
    (s, (s * 1.3).ceil())
}

fn layer_key(view: View<Dialog>) -> String {
    format!("dialog-{:?}", view.id())
}

fn button_key(i: usize) -> String {
    format!("button-{i}")
}

/// The scrim color: the neutral dark ramp's black, stronger in dark.
fn scrim(theme: &Theme) -> Color {
    match theme.scheme {
        Scheme::Dark => Color::BLACK.with_alpha(0.45),
        Scheme::Light => Color::BLACK.with_alpha(0.25),
    }
}

#[allow(clippy::too_many_lines)]
fn layer(view: View<Dialog>, app: &App, look: Look) -> Vec<Element> {
    let d = view.read(app);
    let theme = app.theme();
    let m = metrics(theme);
    let c = theme.colors();
    let vp = app.viewport();
    let hidden = look != Look::Shown;
    let leaving = look == Look::Leaving;
    let pad = theme.tokens.spacing.step(4);
    let w = d.panel_width(vp.width);
    let h = d
        .panel_height(theme, w)
        .min(vp.height - 2.0 * MARGIN)
        .max(40.0);
    let x = ((vp.width - w) / 2.0).max(0.0);
    let y = if d.sheet {
        0.0
    } else {
        ((vp.height - h) / 2.0).max(MARGIN)
    };
    let inner = w - 2.0 * pad;
    let (tsize, _) = title_size(theme);
    let title_h = style::measure_wrapped(&d.env, &d.title, tsize, 600.0, inner);

    let mut body = div().key("body").flex_col().gap(6.0).child(
        text(d.title.clone())
            .key("title")
            .name("dialog-title")
            .w(inner)
            .h(title_h)
            .font_size(tsize)
            .font_weight(600.0)
            .text_color(c.text_primary)
            .hit_test(false),
    );
    if let Some(msg) = &d.message {
        let mh = style::measure_wrapped(&d.env, msg, m.font, 400.0, inner);
        body = body.child(
            text(msg.clone())
                .key("message")
                .name("dialog-message")
                .w(inner)
                .h(mh)
                .font_size(m.font)
                .text_color(c.text_secondary)
                .hit_test(false),
        );
    }
    let mut panel = div()
        .key("panel")
        .name(if d.sheet { "sheet" } else { "dialog" })
        .absolute(Rect::new(x, y, w, h))
        .flex_col()
        .p(pad)
        .bg(if c.surface[3].l > c.surface[1].l {
            c.surface[2]
        } else {
            c.surface[0]
        })
        .border(1.0, style::hairline(theme))
        .elevation(Elevation::E4)
        .opacity(if hidden { 0.0 } else { 1.0 })
        .translate(if !hidden {
            Vec2::ZERO
        } else if d.sheet {
            Vec2::new(0.0, -h)
        } else {
            Vec2::new(0.0, SLIDE)
        })
        .transition(if d.sheet {
            MotionClass::Large
        } else {
            MotionClass::Standard
        })
        .key_context("Dialog")
        .child(body);
    panel = if d.sheet {
        panel.radii(Corners {
            top_left: 0.0,
            top_right: 0.0,
            bottom_right: theme.tokens.radii.lg,
            bottom_left: theme.tokens.radii.lg,
        })
    } else {
        panel.rounded(theme.tokens.radii.lg)
    };
    if let Some((ch, f)) = &d.content {
        panel = panel.child(
            div()
                .key("content")
                .h(*ch)
                .style(|s| s.margin.top = length(12.0))
                .child(f(app)),
        );
    }
    if !d.buttons.is_empty() {
        let mut row = div()
            .key("buttons")
            .flex_row()
            .gap(theme.tokens.spacing.step(2))
            .h(m.height)
            .style(move |s| {
                s.justify_content = JustifyContent::FLEX_END;
                s.margin.top = length(pad);
            });
        for (i, b) in d.buttons.iter().enumerate() {
            let mut btn = Button::new(b.label.clone()).disabled(b.disabled || leaving);
            if b.destructive {
                btn = btn.destructive();
            } else if b.default {
                btn = btn.primary();
            }
            let mut el = btn
                .on_click(move |_, cx| Dialog::press(view, cx, i))
                .render(theme)
                .key(button_key(i));
            if !b.disabled && !leaving {
                // Enter on a focused button presses that button, not the
                // default one handled by the panel.
                el = el.on_action(core::ACTIVATE, move |cx| Dialog::press(view, cx, i));
            }
            row = row.child(el);
        }
        panel = panel.child(row);
    }
    let sem = d.semantics();
    panel = sem.apply(panel);
    if let Some(msg) = d.message.clone() {
        panel = panel.semantics(move |s| s.description(msg.clone()));
    }
    panel = if leaving {
        panel.hit_test(false)
    } else {
        panel
            .tab_index(-1)
            .on_pointer_down(|_, cx| cx.stop_propagation())
    };
    let mut scrim_el = div()
        .key("scrim")
        .name("dialog-scrim")
        .absolute(Rect::new(0.0, 0.0, vp.width, vp.height))
        .bg(if hidden {
            scrim(theme).with_alpha(0.0)
        } else {
            scrim(theme)
        })
        .transition(MotionClass::Standard);
    scrim_el = if leaving {
        scrim_el.hit_test(false)
    } else {
        let dismiss = d.dismiss_on_scrim;
        // Focusable by pointer so a press on the scrim keeps focus inside
        // the trap.
        scrim_el.tab_index(-1).on_pointer_down(move |_, cx| {
            if dismiss {
                Dialog::cancel(view, cx);
            }
            cx.stop_propagation();
        })
    };
    // One wrapper holds the scrim and the panel: it is the focus scope
    // (Tab never leaves it, wherever inside it focus is) and handles Enter
    // and Escape for both.
    let mut wrap = div()
        .key(layer_key(view))
        .name("dialog-layer")
        .absolute(Rect::new(0.0, 0.0, vp.width, vp.height))
        .hit_test(false)
        .child(scrim_el)
        .child(panel);
    if !leaving {
        let default = d.buttons.iter().position(|b| b.default && !b.disabled);
        wrap = wrap
            .focus_scope()
            .on_action(core::CANCEL, move |cx| Dialog::cancel(view, cx))
            .on_action(core::ACTIVATE, move |cx| {
                if let Some(i) = default {
                    Dialog::press(view, cx, i);
                }
            });
    }
    vec![wrap]
}

impl Render for Dialog {
    /// A dialog has no element of its own in the tree; mount the view
    /// anywhere (or just keep the handle) and open it with
    /// [`Dialog::open`].
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        div().name("dialog-anchor").hit_test(false)
    }
}
