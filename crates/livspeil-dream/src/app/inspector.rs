// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The selected node: its name, outputs and inputs, the unlinked inputs
//! editable.

use std::collections::HashMap;

use livspeil_graph::NodeId;
use livspeil_node_api::{PortType, Value};
use livspeil_ui_core::{App, Element, Model, Render, View, ViewContext, div, text};
use livspeil_ui_layout::style::LengthPercentageAuto;
use livspeil_ui_theme::{Color, Theme};
use livspeil_ui_widgets::{Env, TextField, Toggle};

use super::INSPECTOR_WIDTH;
use crate::canvas::Editor;
use crate::edit::{self, Entry};
use crate::inspect::{Inspection, Source, written_value};
use crate::shown::Port;

const PADDING: f32 = 14.0;

/// A text field and the text it was last given. Typing is left alone until
/// the graph's value changes.
struct Field {
    view: View<TextField>,
    given: String,
}

pub(super) struct Inspector {
    env: Env,
    editor: Model<Editor>,
    /// The node the fields belong to.
    node: Option<NodeId>,
    name: Option<Field>,
    inputs: HashMap<String, Field>,
}

fn change(editor: Model<Editor>, app: &mut App, f: impl FnOnce(&mut Editor)) {
    editor.update(app, |e, cx| {
        f(e);
        cx.notify();
    });
}

fn enter(editor: Model<Editor>, app: &mut App, port: &Port, entry: Result<Entry, String>) {
    change(editor, app, |e| {
        e.edit(|s| s.enter(port, entry?));
    });
}

/// What the field of an unlinked input shows.
fn field_text(source: &Source) -> String {
    match source {
        Source::Set(value) => edit::text(value),
        Source::Expression(e) => format!("={e}"),
        _ => String::new(),
    }
}

/// What it shows when empty.
fn placeholder(source: &Source) -> String {
    match source {
        Source::Default(value) => format!("{}  (default)", edit::text(value)),
        Source::Animation => "animated".to_owned(),
        _ => "not set".to_owned(),
    }
}

fn give(field: &mut Field, cx: &mut ViewContext<'_, Inspector>, text: String) {
    if field.given != text {
        field.view.update(cx, |f, fcx| {
            f.set_text(&text);
            fcx.notify();
        });
        field.given = text;
    }
}

fn line(content: String, size: f32, color: Color) -> Element {
    text(content).font_size(size).text_color(color)
}

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

/// A port: its name and type, then what is below.
fn row(theme: &Theme, key: String, name: &str, ty: &str, body: Vec<Element>) -> Element {
    let colors = theme.colors();
    let head = div()
        .key("head")
        .flex_row()
        .gap(8.0)
        .child(line(name.to_owned(), 13.0, colors.text_primary).key("name"))
        .child(line(ty.to_owned(), 11.0, colors.text_secondary).key("type"));
    div()
        .key(key.clone())
        .name(key)
        .flex_col()
        .gap(4.0)
        .style(|s| s.margin.top = LengthPercentageAuto::length(10.0))
        .child(head)
        .children(body)
}

fn shown_value(theme: &Theme, value: Option<&Value>, detail: String) -> Element {
    div()
        .key("value")
        .flex_row()
        .items_center()
        .gap(8.0)
        .children(value.and_then(swatch))
        .child(line(detail, 12.0, theme.colors().text_secondary).key("detail"))
}

impl Inspector {
    pub(super) fn new(env: &Env, editor: Model<Editor>) -> Self {
        Inspector {
            env: env.clone(),
            editor,
            node: None,
            name: None,
            inputs: HashMap::new(),
        }
    }

    /// Drop the fields when another node (or none) is selected.
    fn follow(&mut self, cx: &mut ViewContext<'_, Self>, node: Option<NodeId>) {
        if self.node == node {
            return;
        }
        self.node = node;
        for field in self
            .name
            .take()
            .into_iter()
            .chain(self.inputs.drain().map(|f| f.1))
        {
            cx.release(field.view.id());
        }
    }

    fn name_field(&mut self, cx: &mut ViewContext<'_, Self>, node: NodeId, name: &str) -> Element {
        let field = match &mut self.name {
            Some(field) => {
                give(field, cx, name.to_owned());
                field.view
            }
            None => {
                let editor = self.editor;
                let view = cx.new_view(
                    TextField::new(&self.env, name)
                        .width(INSPECTOR_WIDTH - 2.0 * PADDING)
                        .name("field:name")
                        .label("Node name")
                        .on_submit(move |name, app| {
                            let name = name.trim().to_owned();
                            change(editor, app, |e| {
                                e.edit(|s| s.rename(node, &name));
                            });
                        }),
                );
                cx.own(view.id());
                self.name = Some(Field {
                    view,
                    given: name.to_owned(),
                });
                view
            }
        };
        Element::view(&field).key("name-field")
    }

    /// The control that edits an unlinked input.
    fn control(
        &mut self,
        cx: &mut ViewContext<'_, Self>,
        theme: &Theme,
        port: &Port,
        source: &Source,
    ) -> Element {
        let editor = self.editor;
        let value = match source {
            Source::Set(v) | Source::Default(v) => Some(v),
            _ => None,
        };
        if port.ty == PortType::Bool && !matches!(source, Source::Expression(_)) {
            let on = matches!(value, Some(Value::Bool(true)));
            let port = port.clone();
            return Toggle::new(on)
                .label(port.name.clone())
                .on_change(move |v, app| {
                    enter(editor, app, &port, Ok(Entry::Value(Value::Bool(v))));
                })
                .render(theme)
                .key("toggle");
        }
        let text = field_text(source);
        let field = match self.inputs.get_mut(&port.name) {
            Some(field) => {
                give(field, cx, text);
                field.view
            }
            None => {
                let (target, ty) = (port.clone(), port.ty);
                let view = cx.new_view(
                    TextField::new(&self.env, &text)
                        .width(INSPECTOR_WIDTH - 2.0 * PADDING)
                        .name(format!("field:{}", port.name))
                        .label(port.name.clone())
                        .placeholder(placeholder(source))
                        .on_submit(move |typed, app| {
                            enter(editor, app, &target, edit::parse(ty, typed));
                        }),
                );
                cx.own(view.id());
                self.inputs
                    .insert(port.name.clone(), Field { view, given: text });
                view
            }
        };
        div()
            .key("control")
            .flex_col()
            .gap(4.0)
            .children(value.and_then(swatch))
            .child(Element::view(&field).key("field"))
    }

    fn panel(
        &mut self,
        cx: &mut ViewContext<'_, Self>,
        theme: &Theme,
        ports: &[Port],
        node: NodeId,
        i: &Inspection,
    ) -> Element {
        let colors = theme.colors();
        let mut panel = div()
            .key("panel")
            .flex_col()
            .p(PADDING)
            .child(self.name_field(cx, node, &i.name))
            .child(
                line(i.type_id.clone(), 11.0, colors.text_secondary)
                    .key("kind")
                    .style(|s| s.margin.top = LengthPercentageAuto::length(4.0)),
            );
        if let Some(problem) = &i.problem {
            panel = panel.child(
                line(problem.clone(), 12.0, colors.danger)
                    .key("problem")
                    .name("problem"),
            );
        }
        if !i.outputs.is_empty() {
            panel = panel.child(heading(theme, "OUTPUTS"));
            for r in &i.outputs {
                let detail = r
                    .value
                    .as_ref()
                    .map_or_else(|| "no value".to_owned(), written_value);
                let body = vec![shown_value(theme, r.value.as_ref(), detail)];
                let key = format!("output:{}", r.name);
                panel = panel.child(row(theme, key, &r.name, r.ty, body));
            }
        }
        if !i.inputs.is_empty() {
            panel = panel.child(heading(theme, "INPUTS"));
            for (port, r) in ports.iter().zip(&i.inputs) {
                let body = match &r.value {
                    Source::Link { from, value } => {
                        let detail = value.as_ref().map_or_else(
                            || format!("from {from}"),
                            |v| format!("{}   from {from}", written_value(v)),
                        );
                        vec![shown_value(theme, value.as_ref(), detail)]
                    }
                    source => vec![self.control(cx, theme, port, source)],
                };
                let key = format!("input:{}", r.name);
                panel = panel.child(row(theme, key, &r.name, r.ty, body));
            }
        }
        panel
    }
}

impl Render for Inspector {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let editor = self.editor.read(cx);
        let session = editor.session();
        let message = editor.message().map(str::to_owned);
        let empty = session.nodes().is_empty();
        let selected = session.selected().and_then(|i| session.nodes().get(i));
        let selected = selected.cloned().zip(session.inspection().cloned());
        self.follow(cx, selected.as_ref().map(|(node, _)| node.id));
        let mut root = div().key("inspected").flex_col().w_full();
        if let Some(message) = message {
            root = root.child(
                line(message, 12.0, colors.danger)
                    .key("message")
                    .name("message")
                    .style(|s| {
                        s.margin.top = LengthPercentageAuto::length(PADDING);
                        s.margin.left = LengthPercentageAuto::length(PADDING);
                    }),
            );
        }
        match selected {
            Some((node, inspection)) => {
                root.child(self.panel(cx, &theme, &node.inputs, node.id, &inspection))
            }
            None => root.child(div().key("none").p(PADDING).child(line(
                if empty {
                    "Press Shift+A to add a node.".to_owned()
                } else {
                    "Select a node to inspect it.".to_owned()
                },
                13.0,
                colors.text_secondary,
            ))),
        }
    }
}
