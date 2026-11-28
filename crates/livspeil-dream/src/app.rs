// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window: a bar of transport, edit and file buttons, the node canvas
//! and the inspector.

mod add;
mod files;
mod inspector;
mod open;

use std::cell::RefCell;
use std::path::PathBuf;

use livspeil_ui_core::{
    App, Element, Keymap, Keystroke, Model, Render, View, ViewContext, div, text,
};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto};
use livspeil_ui_widgets::{Button, CardMetrics, Env, NodeCanvas, Picker, ScrollView};

use crate::canvas::Editor;
use crate::session::Session;
use files::{Files, Then};
use inspector::Inspector;

/// Width of the inspector.
pub const INSPECTOR_WIDTH: f32 = 340.0;
const TOOLBAR_HEIGHT: f32 = 44.0;
const ADD: &str = "dream.add";
const NEW: &str = "dream.new";
const OPEN: &str = "dream.open";
const SAVE: &str = "dream.save";
const SAVE_AS: &str = "dream.save_as";
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
    files: Files,
}

impl Mounted {
    /// Whether the window may close now. With unsaved changes it may not:
    /// a dialog asks, and closes the window itself if so answered.
    pub fn may_close(&self, app: &mut App) -> bool {
        self.files.may_leave(app, Then::Close)
    }
}

/// The shortcuts the app adds to the widget keymap.
#[must_use]
pub fn keymap() -> Keymap {
    let mut km = Keymap::default();
    // The widget keymap binds undo and redo only inside text fields, which
    // keep their own: these reach the window from everywhere else.
    // Shift+A types an A elsewhere: only the canvas takes it. Space, held,
    // pans there.
    if let Ok(k) = Keystroke::parse("shift-a") {
        km.bind(k, ADD, Some("NodeCanvas"));
    }
    for (keys, action) in [
        ("ctrl-n", NEW),
        ("cmd-n", NEW),
        ("ctrl-o", OPEN),
        ("cmd-o", OPEN),
        ("ctrl-s", SAVE),
        ("cmd-s", SAVE),
        ("ctrl-shift-s", SAVE_AS),
        ("cmd-shift-s", SAVE_AS),
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

/// Give the canvas the keyboard, so its shortcuts work.
fn focus_canvas(env: &Env, app: &mut App) {
    if let Some(canvas) = app.find("node-canvas") {
        env.focus(app, canvas);
    }
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
    picker: Picker,
    rooms: Picker,
    files: Files,
}

impl Render for Toolbar {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let (editor, picker, files) = (self.editor, self.picker, self.files);
        let rooms = self.rooms;
        let session = editor.read(cx).session();
        let has_file = session.path().is_some();
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
            .child(
                button("Add")
                    .on_click(move |_, cx| picker.open(cx))
                    .render(&theme)
                    .key("add"),
            )
            .child(div().key("space").grow())
            // The first to give up room in a narrow window.
            .child(
                div()
                    .key("name")
                    .clip()
                    .style(|s| {
                        s.flex_shrink = 1.0;
                        s.min_size.width = LengthPercentageAuto::length(0.0);
                    })
                    .child(muted(name, "room").h(16.0)),
            )
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
                button("New")
                    .on_click(move |_, cx| files.new_room(cx))
                    .render(&theme)
                    .key("new"),
            )
            .child(
                button("Open")
                    .on_click(move |_, cx| rooms.open(cx))
                    .render(&theme)
                    .key("open"),
            )
            .child(
                button("Save")
                    .disabled(!dirty && has_file)
                    .on_click(move |_, cx| files.save(cx, Then::Stay))
                    .render(&theme)
                    .key("save"),
            )
            .child(
                button("Save as")
                    .on_click(move |_, cx| files.save_as(cx))
                    .render(&theme)
                    .key("save-as"),
            )
    }
}

struct Window {
    editor: Model<Editor>,
    picker: Picker,
    rooms: Picker,
    files: Files,
    toolbar: View<Toolbar>,
    canvas: View<NodeCanvas<Editor>>,
    inspector: View<ScrollView>,
}

impl Render for Window {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let colors = cx.theme().colors();
        let (editor, picker, files) = (self.editor, self.picker, self.files);
        let rooms = self.rooms;
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
            .on_action(ADD, move |cx| picker.open(cx))
            .on_action(NEW, move |cx| files.new_room(cx))
            .on_action(OPEN, move |cx| rooms.open(cx))
            .on_action(SAVE, move |cx| files.save(cx, Then::Stay))
            .on_action(SAVE_AS, move |cx| files.save_as(cx))
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

/// The window's title for the room called `name`.
#[must_use]
pub fn title(name: &str) -> String {
    format!("{name} — Livspeil Dream")
}

/// Build the window over `session`, make it the app's root and install the
/// shortcuts. `folder` is where rooms are kept ([`crate::rooms::home`]).
pub fn mount(app: &mut App, env: &Env, session: Session, folder: PathBuf) -> Mounted {
    let mut bound = app.keymap().clone();
    bound.extend(&livspeil_ui_widgets::keymap());
    bound.extend(&keymap());
    app.set_keymap(bound);
    let metrics = CardMetrics::from_theme(app.theme());
    let selected = session.selected();
    let editor = app.new_model(Editor::new(session, metrics));
    let canvas = app.new_view(NodeCanvas::new(env, editor));
    if let Some(key) = selected.and_then(|i| editor.read(app).canvas_id(i)) {
        canvas.update(app, |c, cx| {
            c.set_selection([key]);
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
                .and_then(|key| editor.read(app).index_of(key));
            if editor.read(app).session().selected() != one {
                change(editor, app, |e| e.change(|s| s.select(one)));
            }
            false
        });
    });

    // Paths are made from it and read back: a relative one would be applied
    // twice.
    let folder = std::path::absolute(&folder).unwrap_or(folder);
    let picker = add::picker(app, env, editor, canvas);
    let files = Files::new(app, env, editor, canvas, folder.clone());
    let rooms = open::picker(app, env, files, folder);
    // The window is called after the room that is open.
    let titled = RefCell::new(editor.read(app).session().name().to_owned());
    let _ = app.observe(editor.id(), move |app| {
        let name = editor.read(app).session().name();
        if *titled.borrow() != name {
            name.clone_into(&mut titled.borrow_mut());
            let title = title(name);
            app.set_window_title(title);
        }
    });
    let toolbar = app.new_view(Toolbar {
        editor,
        picker,
        rooms,
        files,
    });
    let inspected = app.new_view(Inspector::new(env, editor));
    let inspector = app.new_view(
        ScrollView::new(env, move |_, _| Element::view(&inspected))
            .name("inspector-scroll")
            .label("Inspector")
            .focusable(false),
    );
    let window = app.new_view(Window {
        editor,
        picker,
        rooms,
        files,
        toolbar,
        canvas,
        inspector,
    });
    let root = env.root(app, window);
    app.set_root(root);
    let focus = env.clone();
    // Once laid out: show the whole graph, and take the keyboard so the
    // shortcuts work before anything is clicked.
    app.animate(canvas.id(), START, move |app, _| {
        if canvas.read(app).frame().1.width <= 0.0 {
            return true;
        }
        NodeCanvas::fit(canvas, app);
        if app.focused().is_none() {
            focus_canvas(&focus, app);
        }
        false
    });
    Mounted {
        editor,
        canvas,
        files,
    }
}
