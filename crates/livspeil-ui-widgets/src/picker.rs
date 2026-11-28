// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! [`Picker`]: choose one of many by typing part of its name.

use std::rc::Rc;

use livspeil_ui_core::{
    App, Element, Key, Model, NamedKey, Phase, Render, SharedString, View, ViewContext, div, text,
};
use livspeil_ui_layout::style::LengthPercentage;
use livspeil_ui_layout::{Rect, Vec2};

use crate::dialog::Dialog;
use crate::env::Env;
use crate::scroll::ScrollView;
use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};
use crate::text_field::TextField;

/// Width of the picker's panel.
const WIDTH: f32 = 460.0;
/// Space the panel keeps around its content.
const PAD: f32 = 16.0;
/// Rows shown at once.
const ROWS: usize = 9;
/// Height of a row.
const ROW: f32 = 40.0;
/// Space between the query and the list.
const GAP: f32 = 8.0;

/// One thing a [`Picker`] offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerItem {
    /// What it is called.
    pub label: SharedString,
    /// What it is, shown under the label and searched too.
    pub detail: SharedString,
}

impl PickerItem {
    /// An item called `label`, described by `detail`.
    #[must_use]
    pub fn new(label: impl Into<SharedString>, detail: impl Into<SharedString>) -> Self {
        PickerItem {
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// The indices of the `items` that `query` finds, best first.
///
/// Every word of the query must be in the label or the detail, whatever the
/// case. A label that is the query comes first, then labels starting with
/// it, then labels holding every word, then the rest; each group keeps the
/// order of `items`.
#[must_use]
pub fn search(items: &[PickerItem], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let words: Vec<&str> = query.split_whitespace().collect();
    let mut found: Vec<(u8, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let label = item.label.to_lowercase();
            let detail = item.detail.to_lowercase();
            let all = words
                .iter()
                .all(|w| label.contains(w) || detail.contains(w));
            let rank = if words.is_empty() || label == query {
                0
            } else if label.starts_with(&query) {
                1
            } else if words.iter().all(|w| label.contains(w)) {
                2
            } else {
                3
            };
            all.then_some((rank, i))
        })
        .collect();
    found.sort_unstable();
    found.into_iter().map(|(_, i)| i).collect()
}

type PickFn = Rc<dyn Fn(usize, &mut App)>;

struct State {
    env: Env,
    on_pick: PickFn,
    items: Rc<[PickerItem]>,
    matches: Vec<usize>,
    /// Index into `matches`.
    highlight: usize,
}

impl State {
    fn filter(&mut self, query: &str) {
        self.matches = search(&self.items, query);
        self.highlight = 0;
    }
}

/// A modal list narrowed by typing: a query field over the items it finds.
///
/// - Opens as a [`Dialog`] with the keyboard in the query field.
/// - Up/Down and Page Up/Down move the highlight, Enter or a click chooses
///   it, Escape closes. Choosing closes the picker, returns focus to what
///   had it, then calls the callback with the item's index.
/// - See [`search`] for what a query finds.
///
/// The query field is named `<name>-query` and rows `<name>:<label>`.
#[derive(Debug, Clone, Copy)]
pub struct Picker {
    parts: Parts,
    field: View<TextField>,
    name: &'static str,
}

/// What exists before the query field, whose callbacks use it.
#[derive(Debug, Clone, Copy)]
struct Parts {
    state: Model<State>,
    dialog: View<Dialog>,
    list: View<ScrollView>,
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State")
            .field("matches", &self.matches.len())
            .field("highlight", &self.highlight)
            .finish_non_exhaustive()
    }
}

/// The query field over the list.
struct Body {
    picker: Picker,
}

/// The rows, rendered again when the matches or the highlight change.
struct Rows {
    picker: Picker,
}

impl Picker {
    /// A picker titled `title` over `items`; `on_pick` gets the index of the
    /// chosen one. `name` prefixes its element names.
    pub fn new(
        app: &mut App,
        env: &Env,
        name: &'static str,
        title: impl Into<SharedString>,
        items: Vec<PickerItem>,
        on_pick: impl Fn(usize, &mut App) + 'static,
    ) -> Picker {
        let items: Rc<[PickerItem]> = items.into();
        let matches = search(&items, "");
        let state = app.new_model(State {
            env: env.clone(),
            on_pick: Rc::new(on_pick),
            items,
            matches,
            highlight: 0,
        });
        let list = app.new_view(
            ScrollView::new(env, |_, _| div())
                .name(format!("{name}-list"))
                .label("Results")
                .focusable(false),
        );
        let dialog = app.new_view(Dialog::new(env, title).width(WIDTH).dismiss_on_scrim(true));
        let parts = Parts {
            state,
            dialog,
            list,
        };
        let field = app.new_view(
            TextField::new(env, "")
                .width(WIDTH - 2.0 * PAD)
                .name(format!("{name}-query"))
                .label("Search")
                .placeholder("Type to search")
                .on_change(move |query, app| {
                    state.update(app, |s, cx| {
                        s.filter(query);
                        cx.notify();
                    });
                    parts.reveal(app);
                })
                .on_submit(move |_, app| {
                    let highlight = state.read(app).highlight;
                    parts.choose(app, highlight);
                }),
        );
        let picker = Picker { parts, field, name };
        let rows = app.new_view(Rows { picker });
        list.update(app, |s, _| {
            s.set_content(move |_, _| Element::view(&rows));
        });
        let body = app.new_view(Body { picker });
        #[allow(clippy::cast_precision_loss)]
        let height = metrics(app.theme()).height + GAP + ROWS as f32 * ROW;
        dialog.update(app, |d, _| {
            d.set_content(height, move |_| Element::view(&body).key("body"));
        });
        // Releasing the dialog releases the whole picker.
        app.own(dialog.id(), state.id());
        for view in [field.id(), list.id(), rows.id(), body.id()] {
            app.own(dialog.id(), view);
        }
        picker
    }

    /// Open with an empty query and the keyboard in it.
    pub fn open(self, app: &mut App) {
        self.field.update(app, |f, cx| {
            f.set_text("");
            cx.notify();
        });
        let Parts {
            state,
            dialog,
            list,
        } = self.parts;
        state.update(app, |s, cx| {
            s.filter("");
            cx.notify();
        });
        ScrollView::jump_to(list, app, Vec2::ZERO);
        Dialog::open(dialog, app, None);
        // The field takes focus once the dialog is on screen.
        let env = state.read(app).env.clone();
        let name = TextField::element_name(&self.field, app);
        app.animate(dialog.id(), "picker-focus", move |app, _| {
            if !self.is_open(app) {
                return false;
            }
            match app.find(&name) {
                Some(field) => {
                    env.focus(app, field);
                    false
                }
                None => true,
            }
        });
    }

    /// Close without choosing.
    pub fn close(self, app: &mut App) {
        Dialog::close(self.parts.dialog, app);
    }

    /// Whether it is open.
    #[must_use]
    pub fn is_open(self, app: &App) -> bool {
        self.parts.dialog.read(app).is_open()
    }
}

impl Parts {
    /// Choose the match at `at`.
    fn choose(self, app: &mut App, at: usize) {
        if !self.dialog.read(app).is_open() {
            return;
        }
        let state = self.state.read(app);
        let Some(&item) = state.matches.get(at) else {
            return;
        };
        let on_pick = state.on_pick.clone();
        Dialog::close(self.dialog, app);
        on_pick(item, app);
    }

    fn step(self, app: &mut App, by: isize) {
        self.state.update(app, |s, cx| {
            let last = s.matches.len().saturating_sub(1);
            let to = s.highlight.saturating_add_signed(by).min(last);
            if to != s.highlight {
                s.highlight = to;
                cx.notify();
            }
        });
        self.reveal(app);
    }

    /// Scroll the highlighted row into view.
    fn reveal(self, app: &mut App) {
        #[allow(clippy::cast_precision_loss)]
        let top = self.state.read(app).highlight as f32 * ROW;
        let row = Rect::new(0.0, top, 1.0, ROW);
        ScrollView::jump_into_view(self.list, app, row);
    }
}

impl Render for Body {
    fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
        let picker = self.picker;
        #[allow(clippy::cast_precision_loss)]
        let height = ROWS as f32 * ROW;
        div()
            .key("picker")
            .flex_col()
            .gap(GAP)
            // Before the field sees them: it would move its caret.
            .on_key(Phase::Capture, move |ev, cx| {
                if !ev.pressed {
                    return;
                }
                #[allow(clippy::cast_possible_wrap)]
                let page = ROWS as isize;
                let by = match ev.key {
                    Key::Named(NamedKey::Up) => -1,
                    Key::Named(NamedKey::Down) => 1,
                    Key::Named(NamedKey::PageUp) => -page,
                    Key::Named(NamedKey::PageDown) => page,
                    _ => return,
                };
                picker.parts.step(cx, by);
                cx.stop_propagation();
            })
            .child(Element::view(&picker.field).key("query"))
            .child(
                div()
                    .key("results")
                    .w_full()
                    .h(height)
                    .child(Element::view(&picker.parts.list)),
            )
    }
}

impl Render for Rows {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let font = metrics(&theme).font;
        let picker = self.picker;
        let state = picker.parts.state.read(cx);
        let mut rows = div().key("rows").flex_col().w_full();
        if state.matches.is_empty() {
            return rows.child(
                text("Nothing matches")
                    .key("none")
                    .name(format!("{}-none", picker.name))
                    .font_size(font)
                    .text_color(colors.text_secondary),
            );
        }
        for (at, &i) in state.matches.iter().enumerate() {
            let Some(item) = state.items.get(i) else {
                continue;
            };
            let lit = at == state.highlight;
            let line = |content: SharedString, key: &str, size: f32, color| {
                text(content)
                    .key(key.to_owned())
                    .font_size(size)
                    .text_color(color)
                    .hit_test(false)
            };
            let mut row = div()
                .key(format!("item-{i}"))
                .name(format!("{}:{}", picker.name, item.label))
                .w_full()
                .h(ROW)
                .flex_col()
                .px(8.0)
                .style(|s| s.padding.top = LengthPercentage::length(3.0))
                .rounded(theme.tokens.radii.sm)
                .child(line(item.label.clone(), "label", font, colors.text_primary))
                .child(line(
                    item.detail.clone(),
                    "detail",
                    font - 2.0,
                    colors.text_secondary,
                ))
                .hover(|r| r.bg(colors.surface[1]))
                .on_click(move |_, cx| picker.parts.choose(cx, at));
            if lit {
                row = row.bg(style::emphasize(&theme, colors.surface[2], 0.06));
            }
            let semantics = Semantics::new(Role::ListBoxOption)
                .label(&*item.label)
                .selected(lit);
            rows = rows.child(semantics.apply(row));
        }
        Semantics::new(Role::ListBox).label("Results").apply(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<PickerItem> {
        [
            ("add", "core.math.add — Sum of two numbers"),
            ("compose3", "core.vector.compose3 — A vector from parts"),
            ("gradient", "core.color.gradient — Mix colors along stops"),
            ("mix", "core.color.mix — Mix two colors"),
        ]
        .into_iter()
        .map(|(l, d)| PickerItem::new(l, d))
        .collect()
    }

    #[test]
    fn labels_rank_before_details_and_every_word_must_match() {
        let items = items();
        assert_eq!(search(&items, ""), [0, 1, 2, 3]);
        assert_eq!(search(&items, "  "), [0, 1, 2, 3]);
        // The label "mix" before the detail that mentions mixing.
        assert_eq!(search(&items, "MIX"), [3, 2]);
        assert_eq!(search(&items, "color mix"), [2, 3]);
        assert_eq!(search(&items, "math sum"), [0]);
        assert_eq!(search(&items, "math colors"), Vec::<usize>::new());
        assert_eq!(search(&items, "comp"), [1]);
    }
}
