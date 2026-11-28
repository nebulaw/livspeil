// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window: a bar of transport and file buttons, the node canvas and the
//! inspector.

mod inspector;

use livspeil_ui_core::{
    App, Element, Keymap, Keystroke, Model, Render, View, ViewContext, WindowRequest, div, text,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto};
use livspeil_ui_widgets::{Button, CardMetrics, Dialog, DialogButton, Env, NodeCanvas, ScrollView};

use crate::canvas::{Editor, node_index};
use crate::session::Session;
use inspector::Inspector;

/// Width of the inspector.
pub const INSPECTOR_WIDTH: f32 = 340.0;
const TOOLBAR_HEIGHT: f32 = 44.0;
const SAVE: &str = "dream.save";
const UNDO: &str = "edit.undo";
const REDO: &str = "edit.redo";
const PLAY: &str = "play";
const SELECT: &str = "select";
const START: &str = "start";

/// What [`mount`] built.
#[derive(Debug, Clone, Copy)]
pub struct Mounted {
    /// The open room.
    pub editor: Model<Editor>,
    /// The node canvas.
    pub canvas: View<NodeCanvas<Editor>>,
    /// Asks what to do with unsaved changes.
    close: View<Dialog>,
}

impl Mounted {
    /// Whether the window may close now. With unsaved changes it may not:
    /// a dialog asks, and closes the window itself if so answered.
    pub fn may_close(&self, app: &mut App) -> bool {
        if !self.editor.read(app).session().dirty() {
            return true;
        }
        Dialog::open(self.close, app, None);
        false
    }
}

/// The shortcuts the app adds to the widget keymap.
#[must_use]
pub fn keymap() -> Keymap {
    let mut km = Keymap::default();
    // The widget keymap binds undo and redo only inside text fields, which
    // keep their own: these reach the window from everywhere else.
    for (keys, action) in [
        ("ctrl-s", SAVE),
        ("cmd-s", SAVE),
        ("ctrl-z", UNDO),
        ("cmd-z", UNDO),
        ("ctrl-shift-z", REDO),
        ("cmd-shift-z", REDO),
        ("ctrl-y", REDO),
    ] {
        if let Ok(k) = Keystroke::parse(keys) {
            km.bind(k, action, None);
        }
    }
    km
}

fn change(editor: Model<Editor>, app: &mut App, f: impl FnOnce(&mut Editor)) {
    editor.update(app, |e, cx| {
        f(e);
        cx.notify();
    });
}

fn undo(editor: Model<Editor>, app: &mut App) {
    change(editor, app, |e| {
        if e.session().can_undo() {
            e.edit(Session::undo);
        }
    });
}

fn redo(editor: Model<Editor>, app: &mut App) {
    change(editor, app, |e| {
        if e.session().can_redo() {
            e.edit(Session::redo);
        }
    });
}

fn save(editor: Model<Editor>, app: &mut App) -> bool {
    editor.update(app, |e, cx| {
        let saved = e.edit(Session::save);
        cx.notify();
        saved
    })
}

/// Start the clock, or stop it.
pub fn set_playing(editor: Model<Editor>, app: &mut App, playing: bool) {
    change(editor, app, |e| e.change(|s| s.set_playing(playing)));
    if playing {
        app.animate(editor.id(), PLAY, move |app, dt| {
            editor.update(app, |e, cx| {
                if e.change(|s| s.advance(f64::from(dt))) > 0 {
                    cx.notify();
                }
                e.session().playing()
            })
        });
    }
}

/// The bar of buttons and the clock.
struct Toolbar {
    editor: Model<Editor>,
}

impl Render for Toolbar {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let editor = self.editor;
        let session = editor.read(cx).session();
        let (playing, frame, time) = (session.playing(), session.frame(), session.time());
        let (dirty, can_undo, can_redo) = (session.dirty(), session.can_undo(), session.can_redo());
        let name = if dirty {
            format!("{} — unsaved", session.name())
        } else {
            session.name().to_owned()
        };
        let button = |label: &str| Button::new(label.to_owned());
        let muted = |content: String, key: &str| {
            text(content)
                .key(key.to_owned())
                .font_size(13.0)
                .text_color(theme.colors().text_secondary)
        };
        div()
            .name("toolbar")
            .w_full()
            .h(TOOLBAR_HEIGHT)
            .flex_row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .bg(theme.colors().surface[1])
            .child(
                button(if playing { "Pause" } else { "Play" })
                    .primary()
                    .on_click(move |_, cx| set_playing(editor, cx, !playing))
                    .render(&theme)
                    .key("play"),
            )
            .child(
                button("Step")
                    .disabled(playing)
                    .on_click(move |_, cx| change(editor, cx, |e| e.change(Session::step)))
                    .render(&theme)
                    .key("step"),
            )
            .child(
                button("Rewind")
                    .on_click(move |_, cx| change(editor, cx, |e| e.change(Session::rewind)))
                    .render(&theme)
                    .key("rewind"),
            )
            .child(muted(format!("frame {frame}   {time:.2} s"), "clock"))
            .child(div().key("space").grow())
            .child(muted(name, "room"))
            .child(
                button("Undo")
                    .disabled(!can_undo)
                    .on_click(move |_, cx| undo(editor, cx))
                    .render(&theme)
                    .key("undo"),
            )
            .child(
                button("Redo")
                    .disabled(!can_redo)
                    .on_click(move |_, cx| redo(editor, cx))
                    .render(&theme)
                    .key("redo"),
            )
            .child(
                button("Save")
                    .disabled(!dirty)
                    .on_click(move |_, cx| {
                        save(editor, cx);
                    })
                    .render(&theme)
                    .key("save"),
            )
    }
}

struct Window {
    editor: Model<Editor>,
    toolbar: View<Toolbar>,
    canvas: View<NodeCanvas<Editor>>,
    inspector: View<ScrollView>,
}

impl Render for Window {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let colors = cx.theme().colors();
        let editor = self.editor;
        div()
            .name("dream")
            .w_full()
            .h_full()
            .flex_col()
            .bg(colors.surface[0])
            // Shortcuts reach here from whatever is focused inside.
            .focusable()
            .on_action(UNDO, move |cx| undo(editor, cx))
            .on_action(REDO, move |cx| redo(editor, cx))
            .on_action(SAVE, move |cx| {
                save(editor, cx);
            })
            .child(Element::view(&self.toolbar).key("toolbar"))
            .child(
                div()
                    .key("content")
                    .w_full()
                    .grow()
                    .flex_row()
                    .style(|s| s.min_size.height = LengthPercentageAuto::length(0.0))
                    .child(
                        div()
                            .key("canvas")
                            .grow()
                            .h_full()
                            .child(Element::view(&self.canvas)),
                    )
                    .child(
                        div()
                            .key("inspector")
                            .name("inspector")
                            .h_full()
                            .style(|s| {
                                s.size.width = Dimension::length(INSPECTOR_WIDTH);
                                s.flex_shrink = 0.0;
                            })
                            .bg(colors.surface[1])
                            .child(Element::view(&self.inspector)),
                    ),
            )
    }
}

fn close_dialog(env: &Env, editor: Model<Editor>) -> Dialog {
    Dialog::new(env, "Unsaved changes")
        .message("Save the changes to this room before closing?")
        .button(DialogButton::new("Cancel").cancel())
        .button(
            DialogButton::new("Discard")
                .destructive()
                .on_press(|app| app.request_window(WindowRequest::Close)),
        )
        .button(
            DialogButton::new("Save")
                .default_button()
                .on_press(move |app| {
                    // A failed save keeps the window open and says why.
                    if save(editor, app) {
                        app.request_window(WindowRequest::Close);
                    }
                }),
        )
}

/// Build the window over `session`, make it the app's root and install the
/// shortcuts.
pub fn mount(app: &mut App, env: &Env, session: Session) -> Mounted {
    let mut bound = app.keymap().clone();
    bound.extend(&livspeil_ui_widgets::keymap());
    bound.extend(&keymap());
    app.set_keymap(bound);
    let metrics = CardMetrics::from_theme(app.theme());
    let selected = session.selected();
    let editor = app.new_model(Editor::new(session, metrics));
    let canvas = app.new_view(NodeCanvas::new(env, editor));
    if let Some(i) = selected {
        canvas.update(app, |c, cx| {
            c.set_selection([i as u64 + 1]);
            cx.notify();
        });
    }
    // The inspector follows the canvas's selection, a frame later: the
    // canvas may be in the middle of changing.
    let _ = app.observe(canvas.id(), move |app| {
        app.animate(editor.id(), SELECT, move |app, _| {
            let selection = canvas.read(app).selection();
            let one = (selection.len() == 1)
                .then(|| selection.iter().next().copied())
                .flatten()
                .and_then(node_index);
            if editor.read(app).session().selected() != one {
                change(editor, app, |e| e.change(|s| s.select(one)));
            }
            false
        });
    });

    let toolbar = app.new_view(Toolbar { editor });
    let inspected = app.new_view(Inspector::new(env, editor));
    let inspector = app.new_view(
        ScrollView::new(env, move |_, _| Element::view(&inspected))
            .name("inspector-scroll")
            .label("Inspector")
            .focusable(false),
    );
    let window = app.new_view(Window {
        editor,
        toolbar,
        canvas,
        inspector,
    });
    let root = env.root(app, window);
    app.set_root(root);
    let close = app.new_view(close_dialog(env, editor));
    // Once laid out: show the whole graph, and take the keyboard so the
    // shortcuts work before anything is clicked.
    app.animate(canvas.id(), START, move |app, _| {
        if canvas.read(app).frame().1.width <= 0.0 {
            return true;
        }
        NodeCanvas::fit(canvas, app);
        if app.focused().is_none()
            && let Some(window) = app.find("dream")
        {
            app.focus(window);
        }
        false
    });
    Mounted {
        editor,
        canvas,
        close,
    }
}
