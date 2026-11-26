// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Select`]: a button showing the chosen option that opens a popup list
//! in the overlay layer.

use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, ElementId, EventCx, Model, Render, SharedString, ValueRequest, View, ViewContext,
    actions as core, div, text,
};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Material, MotionClass};

use crate::env::{Env, OverlayKey, OverlayKind, PopupBuilder};
use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics};

/// Padding inside the popup list.
const LIST_PAD: f32 = 4.0;
/// Gap between trigger and popup.
const LIST_GAP: f32 = 4.0;

/// A select (combo box) bound to a `Model<Option<usize>>`.
///
/// - Click, Space, Enter, Alt+Down, Up or Down open the popup with the
///   chosen option highlighted (key context `Select`).
/// - While open: Up/Down move the highlight over enabled options (no
///   wrap), Home/End go to the ends, Enter/Space choose, Escape closes,
///   Tab closes and moves focus. Hovering highlights; clicking an option
///   chooses it; clicking outside closes. Typing a letter jumps to the next
///   option starting with it (chooses it directly while closed).
/// - The popup is a Glass list in the overlay layer of [`Env::root`],
///   below the trigger (above if there is no room), fading and scaling in
///   with the standard spring.
///
/// The trigger element is named [`Select::element_name`]; options are
/// named `option:<label>` (keys `option-<i>`).
pub struct Select {
    /// The chosen index.
    pub selected: Model<Option<usize>>,
    options: Vec<SharedString>,
    enabled: Vec<bool>,
    placeholder: SharedString,
    label: Option<SharedString>,
    width: f32,
    disabled: bool,
    name: Option<SharedString>,
    env: Env,
    open: bool,
    highlighted: Option<usize>,
    anchor: Rect,
    trigger: Option<ElementId>,
}

impl std::fmt::Debug for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Select")
            .field("options", &self.options)
            .field("open", &self.open)
            .field("highlighted", &self.highlighted)
            .finish_non_exhaustive()
    }
}

impl Select {
    /// A 160 px select over `options`.
    #[must_use]
    pub fn new<S: Into<SharedString>>(
        env: &Env,
        selected: Model<Option<usize>>,
        options: impl IntoIterator<Item = S>,
    ) -> Self {
        let options: Vec<SharedString> = options.into_iter().map(Into::into).collect();
        let enabled = vec![true; options.len()];
        Select {
            selected,
            options,
            enabled,
            placeholder: "Choose…".into(),
            label: None,
            width: 160.0,
            disabled: false,
            name: None,
            env: env.clone(),
            open: false,
            highlighted: None,
            anchor: Rect::ZERO,
            trigger: None,
        }
    }

    /// Text shown when nothing is chosen.
    #[must_use]
    pub fn placeholder(mut self, p: impl Into<SharedString>) -> Self {
        self.placeholder = p.into();
        self
    }

    /// Accessible name.
    #[must_use]
    pub fn label(mut self, l: impl Into<SharedString>) -> Self {
        self.label = Some(l.into());
        self
    }

    /// Width.
    #[must_use]
    pub fn width(mut self, w: f32) -> Self {
        self.width = w;
        self
    }

    /// Disable.
    #[must_use]
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    /// Disable one option.
    #[must_use]
    pub fn disable_option(mut self, i: usize) -> Self {
        if let Some(e) = self.enabled.get_mut(i) {
            *e = false;
        }
        self
    }

    /// Element name (unique; default `select:<entity>`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Whether the popup is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The highlighted option while open.
    #[must_use]
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// The trigger element's name for `view`.
    #[must_use]
    pub fn element_name(view: &View<Select>, app: &App) -> String {
        view.read(app)
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("select", view.id()), str::to_owned)
    }

    /// Semantics: `ComboBox` with the chosen option as value and the
    /// expanded state; options as `ListBoxOption`s.
    #[must_use]
    pub fn semantics(&self, app: &App) -> Semantics {
        let sel = *self.selected.read(app);
        let mut s = Semantics::new(Role::ComboBox)
            .label_opt(self.label.as_deref())
            .expanded(self.open)
            .placeholder(Some(&self.placeholder))
            .interactive(
                self.disabled,
                &[
                    SemanticAction::Click,
                    if self.open {
                        SemanticAction::Collapse
                    } else {
                        SemanticAction::Expand
                    },
                    SemanticAction::SetValue,
                    SemanticAction::Focus,
                ],
            );
        s = s.value(sel.and_then(|i| self.options.get(i)).map_or("", |o| &**o));
        for (i, o) in self.options.iter().enumerate() {
            s = s.child(
                Semantics::new(Role::ListBoxOption)
                    .label(&**o)
                    .selected(sel == Some(i))
                    .interactive(self.disabled || !self.enabled[i], &[SemanticAction::Click]),
            );
        }
        s
    }

    fn option_starting(&self, c: char, after: Option<usize>) -> Option<usize> {
        let n = self.options.len();
        let start = after.map_or(0, |a| a + 1);
        (0..n).map(|k| (start + k) % n).find(|&i| {
            self.enabled[i]
                && self.options[i]
                    .chars()
                    .next()
                    .is_some_and(|f| f.to_lowercase().eq(c.to_lowercase()))
        })
    }
}

fn open(view: View<Select>, cx: &mut EventCx<'_>) {
    let trigger = cx.element_id();
    let anchor = cx.visual_bounds(trigger).unwrap_or_else(|| cx.bounds());
    let env = view.update(cx, |s, vcx| {
        let sel = *s.selected.read(vcx);
        s.open = true;
        s.anchor = anchor;
        s.trigger = Some(trigger);
        s.highlighted = sel
            .filter(|i| s.enabled.get(*i).copied().unwrap_or(false))
            .or_else(|| style::edge_enabled(&s.enabled, 1));
        vcx.notify();
        s.env.clone()
    });
    let build: PopupBuilder = Rc::new(move |app, appearing| popup(view, app, appearing));
    env.show_overlay(cx, OverlayKey::Popup(view.id()), OverlayKind::Popup(build));
}

/// Close; with `refocus`, focus goes back to the trigger.
fn close(view: View<Select>, cx: &mut App, refocus: bool) {
    let (env, trigger) = view.update(cx, |s, vcx| {
        s.open = false;
        s.highlighted = None;
        vcx.notify();
        (s.env.clone(), s.trigger)
    });
    env.hide_overlay(cx, OverlayKey::Popup(view.id()));
    if let (true, Some(t)) = (refocus, trigger) {
        env.focus(cx, t);
    }
}

fn choose(view: View<Select>, cx: &mut App, i: usize) {
    let model = view.read(cx).selected;
    model.update(cx, |s, mcx| {
        if *s != Some(i) {
            *s = Some(i);
            mcx.notify();
        }
    });
    close(view, cx, true);
}

fn highlight(view: View<Select>, cx: &mut App, f: impl FnOnce(&Select) -> Option<usize>) {
    view.update(cx, |s, vcx| {
        if let Some(i) = f(s).filter(|i| Some(*i) != s.highlighted) {
            s.highlighted = Some(i);
            vcx.notify();
        }
    });
}

/// The popup (scrim + list) in window coordinates.
fn popup(view: View<Select>, app: &App, appearing: bool) -> Vec<Element> {
    let s = view.read(app);
    let theme = app.theme();
    let m = metrics(theme);
    let c = theme.colors();
    let vp = app.viewport();
    let sel = *s.selected.read(app);
    #[allow(clippy::cast_precision_loss)]
    let h = s.options.len() as f32 * m.height + 2.0 * LIST_PAD;
    let w = s.anchor.width.max(120.0);
    let mut y = s.anchor.bottom() + LIST_GAP;
    if y + h > vp.height - 4.0 && s.anchor.y - LIST_GAP - h >= 4.0 {
        y = s.anchor.y - LIST_GAP - h;
    }
    let x = s.anchor.x.min(vp.width - 4.0 - w).max(4.0);
    let sem = s.semantics(app);
    let mut list = div()
        .key(format!("listbox-{:?}", view.id()))
        .name("listbox")
        .absolute(Rect::new(x, y, w, h))
        .flex_col()
        .p(LIST_PAD)
        .rounded(theme.tokens.radii.sm)
        .material(Material::Glass)
        .opacity(if appearing { 0.0 } else { 1.0 })
        .translate(Vec2::new(0.0, if appearing { -4.0 } else { 0.0 }))
        .transition(MotionClass::Standard)
        .semantics(|x| x.role(livspeil_ui_core::Role::ListBox));
    for (i, label) in s.options.iter().enumerate() {
        let enabled = s.enabled[i];
        let hl = s.highlighted == Some(i);
        let mut row = div()
            .key(format!("option-{i}"))
            .name(format!("option:{label}"))
            .h(m.height)
            .px(m.pad_x - LIST_PAD)
            .flex_row()
            .items_center()
            .gap(m.gap)
            .rounded(theme.tokens.radii.xs)
            .bg(if hl {
                theme.accent().with_alpha(0.3)
            } else {
                theme.accent().with_alpha(0.0)
            })
            .transition(MotionClass::Micro)
            .child(
                text("✓")
                    .key("check")
                    .font_size(m.font)
                    .text_color(c.text_primary)
                    .opacity(if sel == Some(i) { 1.0 } else { 0.0 })
                    .hit_test(false),
            )
            .child(
                text(label.clone())
                    .key("label")
                    .font_size(m.font)
                    .text_color(if enabled {
                        c.text_primary
                    } else {
                        c.text_tertiary
                    })
                    .hit_test(false),
            );
        if let Some(part) = sem.children.get(i) {
            row = part.apply(row);
        }
        if enabled && !s.disabled {
            row = row
                .on_pointer_move(move |_, cx| highlight(view, cx, |_| Some(i)))
                .on_click(move |_, cx| choose(view, cx, i));
        }
        list = list.child(row);
    }
    let scrim = div()
        .key(format!("scrim-{:?}", view.id()))
        .name("scrim")
        .absolute(Rect::new(0.0, 0.0, vp.width, vp.height))
        .on_pointer_down(move |_, cx| {
            close(view, cx, false);
            cx.stop_propagation();
        });
    vec![scrim, list]
}

impl Render for Select {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let name = self
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("select", view.id()), str::to_owned);
        let sel = *self.selected.read(cx);
        let (shown, color) = match sel.and_then(|i| self.options.get(i)) {
            Some(o) => (o.clone(), c.text_primary),
            None => (self.placeholder.clone(), c.text_tertiary),
        };
        let mut el = div()
            .name(name)
            .w(self.width)
            .h(m.height)
            .px(m.pad_x)
            .flex_row()
            .items_center()
            .gap(m.gap)
            .rounded(m.radius)
            .bg(c.surface[2])
            .border(1.0, style::hairline(&theme))
            .transition(MotionClass::Micro)
            .child(
                text(shown)
                    .key("value")
                    .font_size(m.font)
                    .text_color(color)
                    .hit_test(false)
                    .grow(),
            )
            .child(
                text("▾")
                    .key("chevron")
                    .font_size(m.font)
                    .text_color(c.text_secondary)
                    .hit_test(false),
            );
        el = self.semantics(cx).apply(el);
        if self.disabled {
            return style::disabled(el, true);
        }
        el = el
            .key_context("Select")
            .focusable()
            .hover(|r| r.bg(c.surface[3]))
            .active(|r| r.scale(0.98))
            .focus_visual(|r| r.border_color(c.focus_ring))
            .on_click(move |_, cx| {
                if view.read(cx).open {
                    close(view, cx, true);
                } else {
                    open(view, cx);
                }
            })
            .on_action(core::ACTIVATE, move |cx| {
                let (is_open, hl) = {
                    let s = view.read(cx);
                    (s.open, s.highlighted)
                };
                match (is_open, hl) {
                    (true, Some(i)) => choose(view, cx, i),
                    (true, None) => close(view, cx, true),
                    (false, _) => open(view, cx),
                }
            })
            .on_action(actions::OPEN, move |cx| {
                if !view.read(cx).open {
                    open(view, cx);
                }
            })
            .on_set_value(move |req, cx| {
                if let ValueRequest::Text(t) = req {
                    let i = view.read(cx).options.iter().position(|o| &**o == t);
                    if let Some(i) = i {
                        choose(view, cx, i);
                    }
                }
            })
            .on_text_input(move |t, cx| {
                let Some(ch) = t.chars().next() else {
                    return;
                };
                let (is_open, from) = {
                    let s = view.read(cx);
                    (
                        s.open,
                        if s.open {
                            s.highlighted
                        } else {
                            *s.selected.read(cx)
                        },
                    )
                };
                let target = view.read(cx).option_starting(ch, from);
                if let Some(i) = target {
                    if is_open {
                        highlight(view, cx, |_| Some(i));
                    } else {
                        let model = view.read(cx).selected;
                        model.update(cx, |s, mcx| {
                            *s = Some(i);
                            mcx.notify();
                        });
                    }
                }
            });
        let nav = |dir: isize, edge: bool| {
            move |cx: &mut EventCx<'_>| {
                if !view.read(cx).open {
                    if edge {
                        let i = style::edge_enabled(&view.read(cx).enabled, dir);
                        if let Some(i) = i {
                            let model = view.read(cx).selected;
                            model.update(cx, |s, mcx| {
                                *s = Some(i);
                                mcx.notify();
                            });
                        }
                    } else {
                        open(view, cx);
                    }
                    return;
                }
                highlight(view, cx, |s| {
                    if edge {
                        style::edge_enabled(&s.enabled, dir)
                    } else {
                        style::step_enabled(&s.enabled, s.highlighted, dir, false)
                    }
                });
            }
        };
        el = el
            .on_action(actions::NEXT, nav(1, false))
            .on_action(actions::PREV, nav(-1, false))
            .on_action(actions::FIRST, nav(1, true))
            .on_action(actions::LAST, nav(-1, true));
        if self.open {
            let env = self.env.clone();
            let env2 = env.clone();
            el = el
                .on_action(core::CANCEL, move |cx| close(view, cx, true))
                .on_action(core::FOCUS_NEXT, move |cx| {
                    close(view, cx, false);
                    cx.focus_next();
                    env.sync_focus(cx);
                })
                .on_action(core::FOCUS_PREV, move |cx| {
                    close(view, cx, false);
                    cx.focus_prev();
                    env2.sync_focus(cx);
                });
        }
        el
    }
}
