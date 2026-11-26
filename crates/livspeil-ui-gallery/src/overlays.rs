// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The "Overlays & tabs" page: icon buttons, tabs, a popover, a dialog and
//! a sheet, and toasts.

use livspeil_ui_core::{App, Element, Model, View, div};
use livspeil_ui_theme::Theme;
use livspeil_ui_widgets::{
    Button, Dialog, DialogButton, Env, IconButton, IconSize, Placement, Popover, SegmentedControl,
    TabItem, Tabs, Toast, ToastTone, Toasts, Toggle,
};

use crate::pages::{caption, card, column, columns, row, set};

/// State of the Overlays & tabs page.
pub struct Overlays {
    /// Tabs: the plain bar's selection.
    pub tab: Model<usize>,
    /// Tabs: the closable bar's selection.
    pub closable_tab: Model<usize>,
    /// Tabs: the overflowing bar's selection.
    pub overflow_tab: Model<usize>,
    /// Tabs: the plain, closable and overflowing bars.
    pub tabs: [View<Tabs>; 3],
    /// Tabs: how many tabs were added to the closable bar.
    pub added: Model<u32>,
    /// Icon buttons: the toggle's state.
    pub pinned: Model<bool>,
    /// Popover: one per side.
    pub popovers: [View<Popover>; 4],
    /// Popover: the settings it shows.
    pub snap: Model<bool>,
    /// Popover: the settings it shows.
    pub unit: Model<usize>,
    /// A centered dialog.
    pub dialog: View<Dialog>,
    /// A sheet.
    pub sheet: View<Dialog>,
    /// The toast stack.
    pub toasts: View<Toasts>,
}

impl std::fmt::Debug for Overlays {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlays").finish_non_exhaustive()
    }
}

impl Overlays {
    /// Create the page's state.
    pub fn new(app: &mut App, env: &Env) -> Overlays {
        let tab = app.new_model(0usize);
        let closable_tab = app.new_model(1usize);
        let overflow_tab = app.new_model(2usize);
        let tabs = [
            app.new_view(
                Tabs::new(env, tab, ["Scene", "Charms", "Timeline"])
                    .name("tabs:plain")
                    .label("View"),
            ),
            app.new_view(
                Tabs::new(
                    env,
                    closable_tab,
                    ["Living room", "Kitchen", "Studio", "Garden"],
                )
                .closable(true)
                .name("tabs:closable")
                .label("Rooms"),
            ),
            app.new_view(
                Tabs::new(
                    env,
                    overflow_tab,
                    [
                        "Scene", "Charms", "Timeline", "Assets", "History", "Settings", "Plugins",
                    ],
                )
                .name("tabs:overflow")
                .label("Panels"),
            ),
        ];
        let snap = app.new_model(true);
        let unit = app.new_model(0usize);
        let popover = |app: &mut App, side: Placement, label: &'static str| {
            app.new_view(
                Popover::new(env, move |app| {
                    let theme = app.theme().clone();
                    column()
                        .child(caption(&theme, "Canvas"))
                        .child(
                            Toggle::new(*snap.read(app))
                                .label("Snap to grid")
                                .on_change(move |v, cx| set(snap, cx, v))
                                .render(&theme),
                        )
                        .child(
                            SegmentedControl::new(["px", "pt", "mm"], *unit.read(app))
                                .label("Unit")
                                .segment_width(40.0)
                                .on_change(move |i, cx| set(unit, cx, i))
                                .render(&theme),
                        )
                })
                .trigger(move |theme| {
                    IconButton::new("bx-gear", label)
                        .toggle(false)
                        .size(IconSize::S20)
                        .render(theme)
                })
                .placement(side)
                .size(200.0, 112.0)
                .label("Canvas settings"),
            )
        };
        let popovers = [
            popover(app, Placement::Top, "Settings (above)"),
            popover(app, Placement::Bottom, "Settings (below)"),
            popover(app, Placement::Left, "Settings (left)"),
            popover(app, Placement::Right, "Settings (right)"),
        ];
        let toasts = Toasts::new(env, app);
        let dialog = app.new_view(
            Dialog::new(env, "Delete \u{201c}Lighthouse\u{201d}?")
                .message("The layer and its three Charms are removed from this scene. You can undo this from the toast that follows.")
                .button(DialogButton::new("Cancel").cancel())
                .button(
                    DialogButton::new("Delete")
                        .destructive()
                        .default_button()
                        .on_press(move |app| {
                            Toasts::show(
                                toasts,
                                app,
                                Toast::new("Layer \u{201c}Lighthouse\u{201d} deleted")
                                    .action("Undo", |_| {}),
                            );
                        }),
                ),
        );
        let sheet = app.new_view(
            Dialog::new(env, "Save changes to \u{201c}Living room\u{201d}?")
                .sheet()
                .message("Your changes are lost if you don\u{2019}t save them.")
                .button(DialogButton::new("Don\u{2019}t save").destructive())
                .button(DialogButton::new("Cancel").cancel())
                .button(
                    DialogButton::new("Save")
                        .default_button()
                        .on_press(move |app| {
                            Toasts::show(
                                toasts,
                                app,
                                Toast::new("Scene saved").tone(ToastTone::Success),
                            );
                        }),
                ),
        );
        Overlays {
            tab,
            closable_tab,
            overflow_tab,
            tabs,
            added: app.new_model(0),
            pinned: app.new_model(false),
            popovers,
            snap,
            unit,
            dialog,
            sheet,
            toasts,
        }
    }
}

/// The page.
pub(crate) fn page(theme: &Theme, st: &Overlays, cx: &App) -> Element {
    let pinned = st.pinned;
    let on = *pinned.read(cx);
    let (tabs, dialog, sheet, toasts) = (st.tabs, st.dialog, st.sheet, st.toasts);
    let added = st.added;
    let icons = card(theme, "Icon buttons")
        .child(
            row()
                .child(caption(theme, "Sizes"))
                .children(IconSize::ALL.map(|s| {
                    IconButton::new("bx-star", format!("Star {} px", s.px()))
                        .size(s)
                        .render(theme)
                })),
        )
        .child(
            row()
                .child(caption(theme, "Styles"))
                .child(IconButton::new("bx-grid-plus", "Add").render(theme))
                .child(
                    IconButton::new("bx-grid-plus", "Add (filled)")
                        .filled()
                        .render(theme),
                )
                .child(
                    IconButton::new("bx-bookmark-alt", "Pin")
                        .toggle(on)
                        .on_click(move |_, cx| set(pinned, cx, !on))
                        .render(theme),
                )
                .child(
                    IconButton::new("bx-unlink", "Unlink")
                        .disabled(true)
                        .render(theme),
                ),
        );
    let tab_card = card(theme, "Tabs")
        .child(row().child(Element::view(&tabs[0])))
        .child(
            row().child(Element::view(&tabs[1])).child(
                Button::new("New tab")
                    .ghost()
                    .on_click(move |_, cx| {
                        let n = *added.read(cx) + 1;
                        set(added, cx, n);
                        Tabs::push(tabs[1], cx, TabItem::new(format!("Room {n}")));
                    })
                    .render(theme),
            ),
        )
        .child(
            div()
                .w(300.0)
                .flex_col()
                .gap(4.0)
                .child(Element::view(&tabs[2]))
                .child(caption(
                    theme,
                    "Too many tabs scroll (wheel, or arrow keys).",
                )),
        )
        .child(caption(
            theme,
            "Arrows, Home and End choose a tab; Delete closes it when it has a close button.",
        ));
    let pop = card(theme, "Popover")
        .child(
            row()
                .child(Element::view(&st.popovers[0]))
                .child(Element::view(&st.popovers[1]))
                .child(Element::view(&st.popovers[2]))
                .child(Element::view(&st.popovers[3])),
        )
        .child(caption(
            theme,
            "Top, bottom, left, right. Fades in without scaling and flips at the window edge; Escape or a click outside closes it.",
        ));
    let modal = card(theme, "Dialog and sheet")
        .child(
            row()
                .child(
                    Button::new("Delete layer\u{2026}")
                        .destructive()
                        .on_click(move |_, cx| Dialog::open(dialog, cx, None))
                        .render(theme),
                )
                .child(
                    Button::new("Close document\u{2026}")
                        .on_click(move |_, cx| Dialog::open(sheet, cx, None))
                        .render(theme),
                ),
        )
        .child(caption(
            theme,
            "Modal with a focus trap: Enter presses the default button, Escape cancels.",
        ));
    let toast = card(theme, "Toasts")
        .child(
            row()
                .child(
                    Button::new("Info")
                        .on_click(move |_, cx| {
                            Toasts::show(toasts, cx, Toast::new("Scene autosaved"));
                        })
                        .render(theme),
                )
                .child(
                    Button::new("With action")
                        .on_click(move |_, cx| {
                            Toasts::show(
                                toasts,
                                cx,
                                Toast::new("Moved 3 Charms to Kitchen")
                                    .title("Moved")
                                    .action("Undo", |_| {}),
                            );
                        })
                        .render(theme),
                )
                .child(
                    Button::new("Error")
                        .on_click(move |_, cx| {
                            Toasts::show(
                                toasts,
                                cx,
                                Toast::new("The disk is full. Free some space and try again.")
                                    .title("Save failed")
                                    .tone(ToastTone::Danger)
                                    .sticky(),
                            );
                        })
                        .render(theme),
                )
                .child(
                    Button::new("Clear")
                        .ghost()
                        .on_click(move |_, cx| Toasts::clear(toasts, cx))
                        .render(theme),
                ),
        )
        .child(caption(
            theme,
            "They stay for four seconds; hovering pauses the countdown.",
        ));
    column()
        .child(columns(icons, pop))
        .child(tab_card)
        .child(columns(modal, toast))
}
