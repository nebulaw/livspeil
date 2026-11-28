// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The window: a transport bar, the node canvas and the inspector.

use livspeil_node_api::Value;
use livspeil_ui_core::{App, Element, Model, Render, View, ViewContext, div, text};
use livspeil_ui_layout::style::{Dimension, LengthPercentageAuto};
use livspeil_ui_theme::{Color, Theme};
use livspeil_ui_widgets::{Button, CardMetrics, Env, NodeCanvas, ScrollView};

use crate::canvas::{GraphView, node_index};
use crate::inspect::{Inspection, Source, written_value};
use crate::session::Session;

/// Width of the inspector.
pub const INSPECTOR_WIDTH: f32 = 340.0;
const TOOLBAR_HEIGHT: f32 = 44.0;
const PLAY: &str = "play";
const SELECT: &str = "select";
const FIT: &str = "fit";

/// What [`mount`] built.
#[derive(Debug, Clone, Copy)]
pub struct Mounted {
    /// The open room.
    pub session: Model<Session>,
    /// The canvas's model.
    pub graph: Model<GraphView>,
    /// The node canvas.
    pub canvas: View<NodeCanvas<GraphView>>,
}

/// Start the clock, or stop it.
pub fn set_playing(session: Model<Session>, app: &mut App, playing: bool) {
    session.update(app, |s, cx| {
        s.set_playing(playing);
        cx.notify();
    });
    if playing {
        app.animate(session.id(), PLAY, move |app, dt| {
            session.update(app, |s, cx| {
                if s.advance(f64::from(dt)) > 0 {
                    cx.notify();
                }
                s.playing()
            })
        });
    }
}

fn change(session: Model<Session>, app: &mut App, f: impl FnOnce(&mut Session)) {
    session.update(app, |s, cx| {
        f(s);
        cx.notify();
    });
}

/// The bar of transport buttons and the clock.
struct Transport {
    session: Model<Session>,
}

impl Render for Transport {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let session = self.session;
        let s = session.read(cx);
        let (playing, frame, time) = (s.playing(), s.frame(), s.time());
        let name = s.name().to_owned();
        let play = Button::new(if playing { "Pause" } else { "Play" })
            .primary()
            .on_click(move |_, cx| set_playing(session, cx, !playing))
            .render(&theme)
            .key("play");
        let step = Button::new("Step")
            .disabled(playing)
            .on_click(move |_, cx| change(session, cx, Session::step))
            .render(&theme)
            .key("step");
        let rewind = Button::new("Rewind")
            .on_click(move |_, cx| change(session, cx, Session::rewind))
            .render(&theme)
            .key("rewind");
        div()
            .name("transport")
            .w_full()
            .h(TOOLBAR_HEIGHT)
            .flex_row()
            .items_center()
            .gap(8.0)
            .px(12.0)
            .bg(theme.colors().surface[1])
            .child(play)
            .child(step)
            .child(rewind)
            .child(
                text(format!("frame {frame}   {time:.2} s"))
                    .key("clock")
                    .font_size(13.0)
                    .text_color(theme.colors().text_secondary),
            )
            .child(div().key("space").grow())
            .child(
                text(name)
                    .key("room")
                    .font_size(13.0)
                    .text_color(theme.colors().text_secondary),
            )
    }
}

fn line(content: String, size: f32, color: Color) -> Element {
    text(content).font_size(size).text_color(color)
}

/// A heading above the rows of a part of the inspector.
fn heading(theme: &Theme, title: &str) -> Element {
    line(title.to_owned(), 11.0, theme.colors().text_secondary)
        .key(format!("heading-{title}"))
        .font_weight(600.0)
        .style(|s| s.margin.top = LengthPercentageAuto::length(14.0))
}

fn swatch(value: &Value) -> Option<Element> {
    let &Value::Color([l, c, h, a]) = value else {
        return None;
    };
    // Colors are display values here; single precision is plenty.
    #[allow(clippy::cast_possible_truncation)]
    let color = Color::oklch(l as f32, c as f32, h as f32, a as f32);
    Some(
        div()
            .key("swatch")
            .name("swatch")
            .size(28.0, 16.0)
            .rounded(4.0)
            .bg(color),
    )
}

/// A port: its name and type, then what it holds.
fn row(
    theme: &Theme,
    key: String,
    name: &str,
    ty: &str,
    detail: String,
    value: Option<&Value>,
) -> Element {
    let colors = theme.colors();
    let head = div()
        .key("head")
        .flex_row()
        .gap(8.0)
        .child(line(name.to_owned(), 13.0, colors.text_primary).key("name"))
        .child(line(ty.to_owned(), 11.0, colors.text_secondary).key("type"));
    let body = div()
        .key("body")
        .flex_row()
        .items_center()
        .gap(8.0)
        .children(value.and_then(swatch))
        .child(line(detail, 12.0, colors.text_secondary).key("detail"));
    div()
        .key(key.clone())
        .name(key)
        .flex_col()
        .gap(2.0)
        .style(|s| s.margin.top = LengthPercentageAuto::length(8.0))
        .child(head)
        .child(body)
}

fn described(source: &Source) -> (String, Option<&Value>) {
    match source {
        Source::Link { from, value } => match value {
            Some(v) => (format!("{}   from {from}", written_value(v)), Some(v)),
            None => (format!("from {from}"), None),
        },
        Source::Expression(e) => (format!("= {e}"), None),
        Source::Animation => ("animated".to_owned(), None),
        Source::Set(v) => (written_value(v), Some(v)),
        Source::Default(v) => (format!("{}   default", written_value(v)), Some(v)),
        Source::Unset => ("not set".to_owned(), None),
    }
}

fn inspector(theme: &Theme, inspection: Option<&Inspection>) -> Element {
    let colors = theme.colors();
    let mut panel = div().key("panel").flex_col().p(14.0);
    let Some(i) = inspection else {
        return panel.child(line(
            "Select a node to inspect it.".to_owned(),
            13.0,
            colors.text_secondary,
        ));
    };
    panel = panel
        .child(
            line(i.name.clone(), 17.0, colors.text_primary)
                .key("title")
                .font_weight(600.0),
        )
        .child(line(i.type_id.clone(), 11.0, colors.text_secondary).key("kind"));
    if let Some(problem) = &i.problem {
        panel = panel.child(
            line(problem.clone(), 12.0, colors.danger)
                .key("problem")
                .name("problem")
                .style(|s| s.margin.top = LengthPercentageAuto::length(8.0)),
        );
    }
    if !i.outputs.is_empty() {
        panel = panel.child(heading(theme, "OUTPUTS"));
        for r in &i.outputs {
            let detail = r
                .value
                .as_ref()
                .map_or_else(|| "no value".to_owned(), written_value);
            panel = panel.child(row(
                theme,
                format!("output:{}", r.name),
                &r.name,
                r.ty,
                detail,
                r.value.as_ref(),
            ));
        }
    }
    if !i.inputs.is_empty() {
        panel = panel.child(heading(theme, "INPUTS"));
        for r in &i.inputs {
            let (detail, value) = described(&r.value);
            panel = panel.child(row(
                theme,
                format!("input:{}", r.name),
                &r.name,
                r.ty,
                detail,
                value,
            ));
        }
    }
    panel
}

struct Window {
    transport: View<Transport>,
    canvas: View<NodeCanvas<GraphView>>,
    inspector: View<ScrollView>,
}

impl Render for Window {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let colors = cx.theme().colors();
        div()
            .name("dream")
            .w_full()
            .h_full()
            .flex_col()
            .bg(colors.surface[0])
            .child(Element::view(&self.transport).key("transport"))
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

/// Build the window over `session` and make it the app's root.
pub fn mount(app: &mut App, env: &Env, session: Session) -> Mounted {
    let metrics = CardMetrics::from_theme(app.theme());
    let graph = app.new_model(GraphView::new(&session, metrics));
    let selected = session.selected();
    let session = app.new_model(session);
    let canvas = app.new_view(NodeCanvas::new(env, graph));
    if let Some(i) = selected {
        canvas.update(app, |c, cx| {
            c.set_selection([i as u64 + 1]);
            cx.notify();
        });
    }

    // Nodes that fail or recover show it on the canvas.
    let _ = app.observe(session.id(), move |app| {
        graph.update(app, |g, cx| {
            if g.follow(session.read(cx)) {
                cx.notify();
            }
        });
    });
    // The inspector follows the canvas's selection, a frame later: the
    // canvas may be in the middle of changing.
    let _ = app.observe(canvas.id(), move |app| {
        app.animate(session.id(), SELECT, move |app, _| {
            let selection = canvas.read(app).selection();
            let one = (selection.len() == 1)
                .then(|| selection.iter().next().copied())
                .flatten()
                .and_then(node_index);
            if session.read(app).selected() != one {
                change(session, app, |s| s.select(one));
            }
            false
        });
    });

    let transport = app.new_view(Transport { session });
    let inspector_view = app.new_view(
        ScrollView::new(env, move |app, _| {
            inspector(app.theme(), session.read(app).inspection())
        })
        .name("inspector-scroll")
        .label("Inspector")
        .focusable(false),
    );
    let window = app.new_view(Window {
        transport,
        canvas,
        inspector: inspector_view,
    });
    let root = env.root(app, window);
    app.set_root(root);
    // Show the whole graph, once the canvas knows its size.
    app.animate(canvas.id(), FIT, move |app, _| {
        if canvas.read(app).frame().1.width <= 0.0 {
            return true;
        }
        NodeCanvas::fit(canvas, app);
        false
    });
    Mounted {
        session,
        graph,
        canvas,
    }
}
