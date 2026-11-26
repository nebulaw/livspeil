// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Tabs`]: a horizontal tab bar with a sliding selection pill.

use std::rc::Rc;

use livspeil_ui_core::{
    Animated, App, Element, EventCx, Model, PointerKind, Render, ScrollDelta, SharedString, View,
    ViewContext, div, text,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto};
use livspeil_ui_layout::{Rect, Vec2};
use livspeil_ui_theme::{Elevation, MotionClass, PropertyKind};

use crate::env::Env;
use crate::keymap::actions;
use crate::semantics::{Role, SemanticAction, Semantics};
use crate::style::{self, metrics};

/// Motion scope of the tab pill and tab scrolling (`[overrides.tabs]`).
pub const TABS_SCOPE: &str = "tabs";
/// Inset of the pill and tabs inside the track.
pub const TAB_INSET: f32 = 2.0;
/// Narrowest a tab gets.
pub const TAB_MIN_WIDTH: f32 = 56.0;
/// Widest a tab gets (longer titles are clipped).
pub const TAB_MAX_WIDTH: f32 = 220.0;
/// Edge length of a close button.
pub const CLOSE_SIZE: f32 = 16.0;
/// Margin kept around the selected tab when it is scrolled into view.
const REVEAL_MARGIN: f32 = 8.0;
/// Pixels per wheel line.
const WHEEL_LINE: f32 = 40.0;

type CloseFn = Rc<dyn Fn(usize, &mut EventCx<'_>)>;

/// One tab of a [`Tabs`] bar.
#[derive(Clone, Debug, PartialEq)]
pub struct TabItem {
    /// Title (also the accessible name).
    pub title: SharedString,
    /// Whether this tab can be closed; `None` follows [`Tabs::closable`].
    pub closable: Option<bool>,
    /// Disabled tabs cannot be chosen.
    pub disabled: bool,
}

impl TabItem {
    /// A tab titled `title`.
    #[must_use]
    pub fn new(title: impl Into<SharedString>) -> Self {
        TabItem {
            title: title.into(),
            closable: None,
            disabled: false,
        }
    }

    /// Force the close button on or off for this tab.
    #[must_use]
    pub fn closable(mut self, on: bool) -> Self {
        self.closable = Some(on);
        self
    }

    /// Disable this tab.
    #[must_use]
    pub fn disabled(mut self, on: bool) -> Self {
        self.disabled = on;
        self
    }
}

impl<S: Into<SharedString>> From<S> for TabItem {
    fn from(s: S) -> Self {
        TabItem::new(s)
    }
}

/// A horizontal tab bar bound to a `Model<usize>` (the selected tab).
///
/// - The selected tab sits on a pill that slides (position and width) to a
///   new selection with the standard spring of the `tabs` motion scope,
///   keeping its velocity when retargeted. The pill is raised like the
///   [`crate::SegmentedControl`]'s: lighter than the track in dark, bright
///   on a slightly darker track in light, with a faint contact shadow.
/// - One tab stop (key context `Tabs`): Left/Right choose the previous/next
///   enabled tab (no wrap), Home/End the first/last; with
///   [`Tabs::closable`], Delete/Backspace close the selected tab.
/// - Tabs are as wide as their title (at least 56, at most 220 px). When
///   they do not fit, the bar scrolls horizontally (wheel, trackpad), and
///   choosing a tab scrolls it into view with the same spring.
/// - Close buttons (`close:<title>`) belong to a tab; clicking one removes
///   the tab from the bar, moves the selection to the tab that took its
///   place and calls [`Tabs::on_close`] with the removed index.
///
/// Place it in a column or a sized box: it is as wide as its tabs but never
/// wider than its container. Tabs are children keyed `tab-<i>`, named
/// `tab:<title>`.
pub struct Tabs {
    /// The selected tab.
    pub selected: Model<usize>,
    env: Env,
    items: Vec<TabItem>,
    closable: bool,
    label: Option<SharedString>,
    name: Option<SharedString>,
    disabled: bool,
    on_close: Option<CloseFn>,
    /// Pill (x, width) in strip coordinates.
    pill: Animated<Vec2>,
    pill_placed: bool,
    /// Scroll offset of the strip.
    offset: Animated<f32>,
    /// What the offset last revealed: (selected, viewport width, total).
    revealed: Option<(usize, f32, f32)>,
}

impl std::fmt::Debug for Tabs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tabs")
            .field("items", &self.items)
            .field("closable", &self.closable)
            .finish_non_exhaustive()
    }
}

impl Tabs {
    /// A tab bar over `tabs` with `selected` the chosen index.
    #[must_use]
    pub fn new<T: Into<TabItem>>(
        env: &Env,
        selected: Model<usize>,
        tabs: impl IntoIterator<Item = T>,
    ) -> Self {
        Tabs {
            selected,
            env: env.clone(),
            items: tabs.into_iter().map(Into::into).collect(),
            closable: false,
            label: None,
            name: None,
            disabled: false,
            on_close: None,
            pill: Animated::new(Vec2::ZERO),
            pill_placed: false,
            offset: Animated::new(0.0),
            revealed: None,
        }
    }

    /// Give every tab a close button (unless it says otherwise).
    #[must_use]
    pub fn closable(mut self, on: bool) -> Self {
        self.closable = on;
        self
    }

    /// Accessible name of the tab list.
    #[must_use]
    pub fn label(mut self, l: impl Into<SharedString>) -> Self {
        self.label = Some(l.into());
        self
    }

    /// Element name (unique; default `tabs:<entity>`).
    #[must_use]
    pub fn name(mut self, n: impl Into<SharedString>) -> Self {
        self.name = Some(n.into());
        self
    }

    /// Disable the whole bar.
    #[must_use]
    pub fn disabled(mut self, d: bool) -> Self {
        self.disabled = d;
        self
    }

    /// Called with the index of a tab after it was closed (and removed).
    #[must_use]
    pub fn on_close(mut self, f: impl Fn(usize, &mut EventCx<'_>) + 'static) -> Self {
        self.on_close = Some(Rc::new(f));
        self
    }

    /// The tabs.
    #[must_use]
    pub fn items(&self) -> &[TabItem] {
        &self.items
    }

    /// Append a tab and choose it.
    pub fn push(view: View<Tabs>, app: &mut App, tab: impl Into<TabItem>) {
        let (n, sel) = view.update(app, |t, cx| {
            t.items.push(tab.into());
            cx.notify();
            (t.items.len() - 1, t.selected)
        });
        sel.update(app, |s, cx| {
            *s = n;
            cx.notify();
        });
    }

    /// Remove tab `index` as the close button does.
    pub fn close(view: View<Tabs>, app: &mut App, index: usize) -> bool {
        close_tab(view, app, index)
    }

    /// The element name of the bar for `view`.
    #[must_use]
    pub fn element_name(view: &View<Tabs>, app: &App) -> String {
        view.read(app)
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("tabs", view.id()), str::to_owned)
    }

    fn has_close(&self, i: usize) -> bool {
        self.items
            .get(i)
            .is_some_and(|t| t.closable.unwrap_or(self.closable))
    }

    /// Semantics: `TabList` with the selected title as value and one `Tab`
    /// per item.
    #[must_use]
    pub fn semantics(&self, app: &App) -> Semantics {
        let sel = (*self.selected.read(app)).min(self.items.len().saturating_sub(1));
        let mut s = Semantics::new(Role::TabList)
            .label_opt(self.label.as_deref())
            .interactive(self.disabled, &[SemanticAction::Focus]);
        if let Some(t) = self.items.get(sel) {
            s = s.value(&*t.title);
        }
        for (i, t) in self.items.iter().enumerate() {
            s = s.child(
                Semantics::new(Role::Tab)
                    .label(&*t.title)
                    .selected(i == sel)
                    .interactive(self.disabled || t.disabled, &[SemanticAction::Click]),
            );
        }
        s
    }

    /// Tab widths for the theme's text size.
    fn widths(&self, theme: &livspeil_ui_theme::Theme) -> Vec<f32> {
        let m = metrics(theme);
        (0..self.items.len())
            .map(|i| {
                let (w, _) = style::measure_text(&self.env, &self.items[i].title, m.font, 600.0);
                let close = if self.has_close(i) {
                    CLOSE_SIZE + 4.0
                } else {
                    0.0
                };
                (w + 2.0 * m.pad_x + close).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH)
            })
            .collect()
    }
}

fn set_selected(view: View<Tabs>, cx: &mut App, i: usize) {
    let (model, ok) = {
        let t = view.read(cx);
        (
            t.selected,
            t.items.get(i).is_some_and(|x| !x.disabled) && !t.disabled,
        )
    };
    if ok && *model.read(cx) != i {
        model.update(cx, |s, mcx| {
            *s = i;
            mcx.notify();
        });
    }
}

fn close_tab(view: View<Tabs>, cx: &mut App, i: usize) -> bool {
    let (model, removed) = view.update(cx, |t, vcx| {
        if i >= t.items.len() {
            return (t.selected, false);
        }
        t.items.remove(i);
        vcx.notify();
        (t.selected, true)
    });
    if !removed {
        return false;
    }
    // The tab that took the closed one's place is chosen.
    let len = view.read(cx).items.len();
    model.update(cx, |s, mcx| {
        *s = if *s > i {
            *s - 1
        } else {
            (*s).min(len.saturating_sub(1))
        };
        mcx.notify();
    });
    true
}

impl Render for Tabs {
    #[allow(clippy::too_many_lines)]
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let view = cx.view();
        let theme = cx.theme().clone();
        let m = metrics(&theme);
        let c = theme.colors();
        let n = self.items.len();
        let sel = (*self.selected.read(cx)).min(n.saturating_sub(1));
        let name = self
            .name
            .as_deref()
            .map_or_else(|| style::entity_name("tabs", view.id()), str::to_owned);
        let widths = self.widths(&theme);
        let total: f32 = widths.iter().sum::<f32>() + 2.0 * TAB_INSET;
        let avail = cx
            .layout_bounds()
            .map_or(total, |b| b.width)
            .clamp(0.0, total.max(1.0));
        let max_off = (total - avail).max(0.0);
        let h = m.height;
        let inner_h = h - 2.0 * TAB_INSET;
        let x_of = |i: usize| TAB_INSET + widths[..i].iter().sum::<f32>();
        let target = if n == 0 {
            Vec2::ZERO
        } else {
            Vec2::new(x_of(sel), widths[sel])
        };

        // Pill and scroll offset: spring to their targets.
        let dt = cx.frame_delta();
        let spec = cx.motion().resolve(
            Some(TABS_SCOPE),
            MotionClass::Standard,
            PropertyKind::Movement,
        );
        if !self.pill_placed {
            self.pill.set(target);
            self.pill_placed = true;
        }
        if self.pill.target() != target {
            self.pill.animate_to(target, spec);
        } else if self.pill.is_animating() {
            self.pill.step(dt);
        }
        if n > 0 && self.revealed != Some((sel, avail, total)) {
            let (x, w) = (x_of(sel), widths[sel]);
            let cur = self.offset.target();
            let mut want = cur;
            if x - REVEAL_MARGIN < want {
                want = x - REVEAL_MARGIN;
            }
            if x + w + REVEAL_MARGIN > want + avail {
                want = x + w + REVEAL_MARGIN - avail;
            }
            want = want.clamp(0.0, max_off);
            if self.revealed.is_none() {
                self.offset.set(want);
            } else if (want - cur).abs() > f32::EPSILON {
                self.offset.animate_to(want, spec);
            }
            self.revealed = Some((sel, avail, total));
        } else if self.offset.target() > max_off {
            self.offset.animate_to(max_off, spec);
        } else if self.offset.is_animating() {
            self.offset.step(dt);
        }
        if self.pill.is_animating() || self.offset.is_animating() {
            cx.notify();
        }
        let pill_v = self.pill.value();
        let off = self
            .offset
            .value()
            .clamp(0.0, max_off.max(self.offset.target()));

        // Dark: a lighter pill on a dark track. Light: a bright pill on a
        // slightly darker track (as in the segmented control).
        let (track, pill_fill) = if c.surface[3].l > c.surface[1].l {
            (c.surface[1], c.surface[3])
        } else {
            (c.surface[2], c.surface[0])
        };
        let pill_radius = (m.radius - TAB_INSET).max(2.0);
        let mut strip = div()
            .key("strip")
            .size(total, h)
            .style(|s| s.flex_shrink = 0.0)
            .flex_row()
            .p(TAB_INSET)
            .translate(Vec2::new(-off, 0.0))
            .child(
                div()
                    .key("pill")
                    .name("tab-pill")
                    .absolute(Rect::new(pill_v.x, TAB_INSET, pill_v.y.max(0.0), inner_h))
                    .rounded(pill_radius)
                    .bg(pill_fill)
                    .elevation(Elevation::E1)
                    .opacity(if n == 0 { 0.0 } else { 1.0 })
                    .hit_test(false),
            );
        let sem = self.semantics(cx);
        for (i, item) in self.items.iter().enumerate() {
            let selected = i == sel;
            let disabled = self.disabled || item.disabled;
            let color = if selected {
                c.text_primary
            } else {
                c.text_secondary
            };
            let mut tab = div()
                .key(format!("tab-{i}"))
                .name(format!("tab:{}", item.title))
                .size(widths[i], inner_h)
                .style(|s| s.flex_shrink = 0.0)
                .flex_row()
                .items_center()
                .justify_center()
                .gap(4.0)
                .px(m.pad_x - 2.0)
                .rounded(pill_radius)
                .clip()
                .transition(MotionClass::Micro)
                .child(
                    text(item.title.clone())
                        .key("label")
                        .font_size(m.font)
                        .font_weight(if selected { 600.0 } else { 500.0 })
                        .text_color(if disabled { c.text_tertiary } else { color })
                        .hit_test(false),
                );
            if self.has_close(i) {
                let mut close = div()
                    .key("close")
                    .name(format!("close:{}", item.title))
                    .size(CLOSE_SIZE, CLOSE_SIZE)
                    .style(|s| s.flex_shrink = 0.0)
                    .items_center()
                    .justify_center()
                    .rounded(CLOSE_SIZE / 2.0)
                    .transition(MotionClass::Micro)
                    .child(
                        text("×")
                            .key("x")
                            .font_size(m.font + 1.0)
                            .text_color(c.text_tertiary)
                            .hit_test(false),
                    );
                if !disabled {
                    let on_close = self.on_close.clone();
                    close = close
                        .hover(|r| r.bg(c.text_primary.with_alpha(0.12)))
                        .on_click(move |_, cx| {
                            if close_tab(view, cx, i)
                                && let Some(f) = &on_close
                            {
                                f(i, cx);
                            }
                            cx.stop_propagation();
                        });
                }
                tab = tab.child(close);
            }
            if !disabled {
                if !selected {
                    tab = tab.hover(|r| r.bg(c.surface[2].with_alpha(0.6)));
                }
                tab = tab.on_click(move |_, cx| set_selected(view, cx, i));
            }
            if let Some(part) = sem.children.get(i) {
                tab = part.apply(tab);
            }
            strip = strip.child(tab);
        }

        let mut el = div()
            .name(name)
            .key_context("Tabs")
            .h(h)
            .style(move |s| {
                s.size.width = Dimension::length(total);
                s.max_size.width = LengthPercentageAuto::percent(1.0);
                s.min_size.width = LengthPercentageAuto::length(0.0);
                s.flex_shrink = 1.0;
            })
            .rounded(m.radius)
            .bg(track)
            .border(1.0, style::hairline(&theme))
            .clip()
            .child(strip);
        el = sem.apply(el);
        if self.disabled {
            return style::disabled(el, true);
        }
        el = el
            .focusable()
            .focus_visual(|r| r.border_color(c.focus_ring))
            .on_scroll(move |ev, cx| {
                let PointerKind::Scroll(delta) = ev.kind else {
                    return;
                };
                if ev.modifiers.ctrl || ev.modifiers.cmd {
                    return;
                }
                let d = delta.pixels(WHEEL_LINE);
                // A horizontal bar follows either axis of the wheel.
                let dx = if d.x.abs() >= d.y.abs() { d.x } else { d.y };
                scroll_by(view, cx, dx, matches!(delta, ScrollDelta::Lines(_)));
                cx.stop_propagation();
            });
        let nav = |dir: isize, edge: bool| {
            move |cx: &mut EventCx<'_>| {
                let (enabled, cur) = {
                    let t = view.read(cx);
                    (
                        t.items.iter().map(|i| !i.disabled).collect::<Vec<_>>(),
                        (*t.selected.read(cx)).min(t.items.len().saturating_sub(1)),
                    )
                };
                let next = if edge {
                    style::edge_enabled(&enabled, dir)
                } else {
                    style::step_enabled(&enabled, Some(cur), dir, false)
                };
                if let Some(i) = next.filter(|i| *i != cur) {
                    set_selected(view, cx, i);
                }
            }
        };
        let on_close = self.on_close.clone();
        el.on_action(actions::NEXT, nav(1, false))
            .on_action(actions::PREV, nav(-1, false))
            .on_action(actions::FIRST, nav(1, true))
            .on_action(actions::LAST, nav(-1, true))
            .on_action(actions::CLOSE_TAB, move |cx| {
                let (i, ok) = {
                    let t = view.read(cx);
                    let i = *t.selected.read(cx);
                    (i, t.has_close(i))
                };
                if ok
                    && close_tab(view, cx, i)
                    && let Some(f) = &on_close
                {
                    f(i, cx);
                }
            })
    }
}

/// Scroll the strip by `dx` px (wheel and trackpad): trackpad deltas move
/// at once, wheel steps spring.
fn scroll_by(view: View<Tabs>, cx: &mut EventCx<'_>, dx: f32, springy: bool) {
    let spec = cx.motion().resolve(
        Some(TABS_SCOPE),
        MotionClass::Standard,
        PropertyKind::Movement,
    );
    view.update(cx, |t, vcx| {
        let (total, avail) = match t.revealed {
            Some((_, avail, total)) => (total, avail),
            None => return,
        };
        let max = (total - avail).max(0.0);
        let want = (t.offset.target() + dx).clamp(0.0, max);
        if springy {
            t.offset.animate_to(want, spec);
        } else {
            t.offset.set(want);
        }
        vcx.notify();
    });
}
