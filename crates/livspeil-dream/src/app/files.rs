// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! New, save, save as and close, with the two questions they ask: what to
//! do with unsaved changes, and where to save.

use std::path::{Path, PathBuf};

use livspeil_ui_core::{App, Element, Model, View, WindowRequest, div, text};
use livspeil_ui_layout::Canvas;
use livspeil_ui_widgets::{Dialog, DialogButton, Env, NodeCanvas, TextField};

use super::{change, focus_canvas};
use crate::canvas::Editor;
use crate::session::Session;

/// Width of the "save as" dialog, and of the space inside it.
const ASK_WIDTH: f32 = 480.0;
const ASK_INNER: f32 = ASK_WIDTH - 32.0;
/// Height of its field and of the room for why saving failed.
const ASK_HEIGHT: f32 = 76.0;
/// The name of its field.
const PATH_FIELD: &str = "save-as-path";

/// What was waiting for the room to be saved or given up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Then {
    /// Nothing.
    #[default]
    Stay,
    /// Closing the window.
    Close,
    /// Starting a new room.
    New,
}

#[derive(Debug, Clone, Copy)]
struct Dialogs {
    unsaved: View<Dialog>,
    ask: View<Dialog>,
    path: View<TextField>,
}

#[derive(Debug)]
struct State {
    env: Env,
    editor: Model<Editor>,
    canvas: View<NodeCanvas<Editor>>,
    then: Then,
    /// Set once the dialogs, whose buttons need this state, are made.
    dialogs: Option<Dialogs>,
}

/// The handle to what the window does with its file.
#[derive(Debug, Clone, Copy)]
pub(super) struct Files(Model<State>);

/// Where "save as" suggests to put the room: beside its file, or in the
/// directory the app was started in.
fn suggestion(file: Option<&Path>) -> String {
    let here = || std::env::current_dir().unwrap_or_default();
    let suggested: PathBuf = match file {
        None => here().join("untitled.room"),
        Some(file) => {
            let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
            let package = file.parent().filter(|_| file.ends_with("room.kdl"));
            match package.filter(|p| p.extension().is_some_and(|e| e == "room")) {
                Some(package) => {
                    let mut name = package.file_stem().unwrap_or_default().to_owned();
                    name.push("-copy.room");
                    package.with_file_name(name)
                }
                None => file.with_extension("room"),
            }
        }
    };
    suggested.display().to_string()
}

impl Files {
    pub(super) fn new(
        app: &mut App,
        env: &Env,
        editor: Model<Editor>,
        canvas: View<NodeCanvas<Editor>>,
    ) -> Files {
        let state = app.new_model(State {
            env: env.clone(),
            editor,
            canvas,
            then: Then::Stay,
            dialogs: None,
        });
        let files = Files(state);
        let path = app.new_view(
            TextField::new(env, "")
                .width(ASK_INNER)
                .name(PATH_FIELD)
                .label("Where to save the room")
                .on_submit(move |_, app| files.confirm_path(app)),
        );
        let unsaved = app.new_view(
            Dialog::new(env, "Unsaved changes")
                .message("Save the changes to this room first?")
                .button(
                    DialogButton::new("Cancel")
                        .cancel()
                        .on_press(move |app| files.wait_for(app, Then::Stay)),
                )
                .button(
                    DialogButton::new("Discard")
                        .destructive()
                        .on_press(move |app| files.proceed(app)),
                )
                .button(
                    DialogButton::new("Save")
                        .default_button()
                        .on_press(move |app| {
                            let then = state.read(app).then;
                            files.save(app, then);
                        }),
                ),
        );
        let ask = app.new_view(
            Dialog::new(env, "Save room as")
                .width(ASK_WIDTH)
                .message("A room is a folder; everything it needs goes into it.")
                .content(ASK_HEIGHT, move |app| {
                    let colors = app.theme().colors();
                    let mut body = div()
                        .key("ask")
                        .flex_col()
                        .gap(8.0)
                        .child(Element::view(&path).key("path"));
                    if let Some(why) = editor.read(app).message() {
                        body = body.child(
                            text(why.to_owned())
                                .key("why")
                                .name("save-as-error")
                                .w(ASK_INNER)
                                .font_size(12.0)
                                .text_color(colors.danger),
                        );
                    }
                    body
                })
                .button(
                    DialogButton::new("Cancel")
                        .cancel()
                        .on_press(move |app| files.wait_for(app, Then::Stay)),
                )
                .button(
                    DialogButton::new("Save")
                        .default_button()
                        .on_press(move |app| files.save_typed(app)),
                ),
        );
        state.update(app, |s, _| {
            s.dialogs = Some(Dialogs { unsaved, ask, path });
        });
        files
    }

    fn wait_for(self, app: &mut App, then: Then) {
        self.0.update(app, |s, _| s.then = then);
    }

    /// Do what was waiting.
    fn proceed(self, app: &mut App) {
        let then = self.0.read(app).then;
        self.wait_for(app, Then::Stay);
        match then {
            Then::Stay => {}
            Then::Close => app.request_window(WindowRequest::Close),
            Then::New => self.start_new(app),
        }
    }

    /// Whether the room may be left for `then` now. With unsaved changes
    /// it may not: a dialog asks, and does `then` itself if so answered.
    pub(super) fn may_leave(self, app: &mut App, then: Then) -> bool {
        let state = self.0.read(app);
        let (editor, dialogs) = (state.editor, state.dialogs);
        if !editor.read(app).session().dirty() {
            return true;
        }
        if let Some(d) = dialogs {
            self.wait_for(app, then);
            Dialog::open(d.unsaved, app, None);
        }
        false
    }

    /// Start a new, empty room, asking about unsaved changes first.
    pub(super) fn new_room(self, app: &mut App) {
        if self.may_leave(app, Then::New) {
            self.start_new(app);
        }
    }

    fn start_new(self, app: &mut App) {
        let state = self.0.read(app);
        let (env, editor, canvas) = (state.env.clone(), state.editor, state.canvas);
        match Session::untitled() {
            Ok(session) => change(editor, app, |e| e.open(session)),
            Err(e) => change(editor, app, |editor| editor.report(Some(e.to_string()))),
        }
        canvas.update(app, |c, cx| {
            c.set_selection([]);
            cx.notify();
        });
        NodeCanvas::set_canvas(canvas, app, Canvas::default());
        focus_canvas(&env, app);
    }

    /// Save to the room's file, then do `then`. A room without one asks
    /// where first; a failed save does nothing more and says why.
    pub(super) fn save(self, app: &mut App, then: Then) {
        self.wait_for(app, then);
        let editor = self.0.read(app).editor;
        if editor.read(app).session().path().is_none() {
            self.ask_path(app);
            return;
        }
        let mut saved = false;
        change(editor, app, |e| saved = e.edit(Session::save));
        if saved {
            self.proceed(app);
        } else {
            self.wait_for(app, Then::Stay);
        }
    }

    /// Ask where to save the room.
    pub(super) fn save_as(self, app: &mut App) {
        self.wait_for(app, Then::Stay);
        self.ask_path(app);
    }

    fn ask_path(self, app: &mut App) {
        let state = self.0.read(app);
        let (editor, dialogs) = (state.editor, state.dialogs);
        let Some(d) = dialogs else {
            return;
        };
        let suggested = suggestion(editor.read(app).session().path());
        d.path.update(app, |f, cx| {
            f.set_text(&suggested);
            cx.notify();
        });
        change(editor, app, |e| e.report(None));
        Dialog::open(d.ask, app, None);
        Dialog::focus_content(d.ask, app, PATH_FIELD);
    }

    /// Enter in the field: what the dialog's Save does.
    fn confirm_path(self, app: &mut App) {
        if let Some(d) = self.0.read(app).dialogs {
            Dialog::close(d.ask, app);
            self.save_typed(app);
        }
    }

    fn save_typed(self, app: &mut App) {
        let state = self.0.read(app);
        let (editor, dialogs) = (state.editor, state.dialogs);
        let Some(d) = dialogs else {
            return;
        };
        let typed = d.path.read(app).text().to_owned();
        let mut saved = false;
        change(editor, app, |e| saved = e.edit(|s| s.save_as(&typed)));
        if saved {
            self.proceed(app);
        } else {
            // Ask again, with what was typed and why it failed.
            Dialog::open(d.ask, app, None);
            Dialog::focus_content(d.ask, app, PATH_FIELD);
        }
    }
}
