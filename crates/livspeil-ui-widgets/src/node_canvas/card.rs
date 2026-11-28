// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! A node card: a view of its own on a layer of its own. The canvas gives
//! it what it shows and places it; it renders again only when that content
//! changes, never when it moves.

use std::collections::HashMap;

use livspeil_ui_core::{
    App, CursorShape, Element, Model, MouseButton, PointerKind, Render, SharedString, View,
    ViewContext, div, text,
};
use livspeil_ui_layout::{Point, Rect, Transform2D, Vec2};
use livspeil_ui_theme::{Color, Elevation, MotionClass, Theme};

use super::geom::{CardMetrics, GraphCache, NodeGeom, PORT_DOT};
use super::model::{NodeGraphModel, NodeId, ParamInfo, ParamValue, PortRef, PortSide};
use super::{Lod, NodeCanvas, fill, node_click, node_down, node_drag, port_drag};
use crate::env::Env;
use crate::number_field::NumberField;
use crate::semantics::{Role, Semantics};
use crate::style::{self, metrics};
use crate::toggle::Toggle;

/// Key of the layer that places a card.
const CARD_LAYER: &str = "card";

/// How a pending wire marks a card's ports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pending {
    /// The side the wire started from.
    side: PortSide,
    /// It started on this card.
    own: bool,
    /// The port of this card it snaps to.
    hot: Option<(PortSide, usize)>,
}

impl Pending {
    /// How a wire from `from`, snapping to `snap`, marks `node`.
    pub(super) fn on(node: NodeId, from: PortRef, snap: Option<PortRef>) -> Pending {
        Pending {
            side: from.side,
            own: from.node == node,
            hot: snap.filter(|p| p.node == node).map(|p| (p.side, p.index)),
        }
    }
}

/// How the canvas wants a card shown, apart from the node itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CardState {
    pub(super) metrics: CardMetrics,
    pub(super) lod: Lod,
    pub(super) selected: bool,
    pub(super) pending: Option<Pending>,
}

/// Everything a card shows.
#[derive(Debug)]
pub(super) struct CardProps {
    /// Its position is not part of what the card shows.
    node: NodeGeom,
    /// Port colors: inputs, then outputs.
    colors: Vec<Color>,
    metrics: CardMetrics,
    lod: Lod,
    selected: bool,
    pending: Option<Pending>,
}

fn port_colors<'a>(cache: &'a GraphCache, node: &'a NodeGeom) -> impl Iterator<Item = Color> + 'a {
    node.inputs
        .iter()
        .chain(&node.outputs)
        .map(|p| cache.colors.get(&p.ty).copied().unwrap_or(Color::WHITE))
}

impl CardProps {
    pub(super) fn new(cache: &GraphCache, node: &NodeGeom, state: CardState) -> Self {
        CardProps {
            node: node.clone(),
            colors: port_colors(cache, node).collect(),
            metrics: state.metrics,
            lod: state.lod,
            selected: state.selected,
            pending: state.pending,
        }
    }

    /// Whether [`CardProps::new`] with the same arguments would show the
    /// same, without building it.
    pub(super) fn shows(&self, cache: &GraphCache, node: &NodeGeom, state: CardState) -> bool {
        let n = &self.node;
        self.metrics == state.metrics
            && self.lod == state.lod
            && self.selected == state.selected
            && self.pending == state.pending
            && n.id == node.id
            && n.title == node.title
            && n.rect.size() == node.rect.size()
            && n.inputs == node.inputs
            && n.outputs == node.outputs
            && n.params == node.params
            && n.problem == node.problem
            && self.colors.iter().copied().eq(port_colors(cache, node))
    }

    fn color(&self, side: PortSide, index: usize) -> Color {
        let at = match side {
            PortSide::Input => index,
            PortSide::Output => self.node.inputs.len() + index,
        };
        self.colors.get(at).copied().unwrap_or(Color::WHITE)
    }
}

/// The view of one node.
pub(super) struct NodeCard<M: NodeGraphModel> {
    env: Env,
    canvas: View<NodeCanvas<M>>,
    graph: Model<M>,
    props: CardProps,
    /// The number fields of its parameters, by parameter index.
    fields: HashMap<usize, (Model<f64>, View<NumberField>)>,
}

impl<M: NodeGraphModel> NodeCard<M> {
    pub(super) fn new(
        env: &Env,
        canvas: View<NodeCanvas<M>>,
        graph: Model<M>,
        props: CardProps,
    ) -> Self {
        NodeCard {
            env: env.clone(),
            canvas,
            graph,
            props,
            fields: HashMap::new(),
        }
    }

    /// Show `node` in `state`, rendering again only if that differs from
    /// what the card shows.
    pub(super) fn set(
        view: View<Self>,
        app: &mut App,
        cache: &GraphCache,
        node: &NodeGeom,
        state: CardState,
    ) {
        let same = app.untracked(|app| view.read(app).props.shows(cache, node, state));
        if !same {
            let props = CardProps::new(cache, node, state);
            view.update(app, |s, cx| {
                let old = std::mem::replace(&mut s.props, props);
                s.follow_params(&old, cx);
                cx.notify();
            });
        }
    }

    /// A field edits a model of its own: give it the values the graph
    /// changed. Only changes are passed on, so a value being scrubbed is not
    /// overwritten by the one the card still shows. A parameter whose name or
    /// range changed gets a new field.
    fn follow_params(&mut self, old: &CardProps, cx: &mut ViewContext<'_, Self>) {
        let mut replaced = Vec::new();
        for (&i, &(model, _)) in &self.fields {
            let (old, new) = (old.node.params.get(i), self.props.node.params.get(i));
            if old == new {
                continue;
            }
            match (old, new) {
                (Some(old), Some(new)) if same_field(old, new) => {
                    if let ParamValue::Number { value, .. } = new.value
                        && cx.untracked(|app| *model.read(app)).to_bits() != value.to_bits()
                    {
                        model.update(cx, |v, mcx| {
                            *v = value;
                            mcx.notify();
                        });
                    }
                }
                _ => replaced.push(i),
            }
        }
        for i in replaced {
            if let Some(field) = self.fields.remove(&i) {
                release_field(cx, field);
            }
        }
    }

    /// Put the card's top left at `at` (canvas space), on whole units as
    /// layout would. Renders nothing.
    pub(super) fn place(view: View<Self>, app: &mut App, at: Point) {
        let snap = |v: f32| (v + 0.5).floor();
        let to = Vec2::new(snap(at.x), snap(at.y));
        app.layer(view.id(), CARD_LAYER)
            .set_transform(app, Transform2D::translate(to));
    }

    /// The field editing number parameter `index`, created on first use.
    fn field(
        &mut self,
        cx: &mut ViewContext<'_, Self>,
        index: usize,
        name: &SharedString,
        value: f64,
        (min, max, step): (f64, f64, f64),
    ) -> View<NumberField> {
        if let Some((_, field)) = self.fields.get(&index) {
            return *field;
        }
        let node = self.props.node.id;
        let model = cx.new_model(value);
        let field = cx.new_view(
            NumberField::new(&self.env, model, min, max, step)
                .width(84.0)
                .accessible_label(name.clone())
                .name(format!("param:{node}:{index}")),
        );
        cx.own(model.id());
        cx.own(field.id());
        let graph = self.graph;
        // Cancelled when the card is released.
        cx.watch(model.id(), move |app| {
            let value = *model.read(app);
            graph.update(app, |g, mcx| {
                let value = ParamValue::Number {
                    value,
                    min,
                    max,
                    step,
                };
                g.set_param(node, index, value);
                mcx.notify();
            });
        });
        self.fields.insert(index, (model, field));
        field
    }
}

/// Whether the field made for number parameter `a` also serves `b`.
fn same_field(a: &ParamInfo, b: &ParamInfo) -> bool {
    match (a.value, b.value) {
        (
            ParamValue::Number { min, max, step, .. },
            ParamValue::Number {
                min: min_b,
                max: max_b,
                step: step_b,
                ..
            },
        ) => a.name == b.name && (min, max, step) == (min_b, max_b, step_b),
        _ => false,
    }
}

fn release_field(app: &mut App, (model, field): (Model<f64>, View<NumberField>)) {
    app.release(field.id());
    app.release(model.id());
}

impl<M: NodeGraphModel> Render for NodeCard<M> {
    fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
        let theme = cx.theme().clone();
        let n = self.props.node.clone();
        let col = theme.colors();
        let m = metrics(&theme);
        let cm = self.props.metrics;
        let (id, lod, selected) = (n.id, self.props.lod, self.props.selected);
        let (view, graph) = (self.canvas, self.graph);
        let radius = theme.tokens.radii.sm + 2.0;
        let mut card = div()
            .name(format!("node:{}", n.title))
            .absolute(Rect::new(0.0, 0.0, n.rect.width, n.rect.height))
            .layer(cx.layer(CARD_LAYER))
            .flex_col()
            .rounded(radius)
            .bg(col.surface[1].with_alpha(0.97))
            .on_pointer_down(move |ev, cx| {
                if matches!(ev.kind, PointerKind::Down(MouseButton::Left)) {
                    node_down(view, id, cx);
                }
            })
            .on_click(move |_, cx| node_click(view, id, cx))
            .on_drag(move |ev, cx| node_drag(view, ev, cx));
        let mut sem = Semantics::new(Role::Node)
            .label(&*n.title)
            .selected(selected);
        if let Some(problem) = &n.problem {
            sem.value = Some(problem.to_string());
        }
        if lod == Lod::Block {
            let tint = match (n.outputs.first(), n.inputs.first()) {
                (Some(_), _) => self.props.color(PortSide::Output, 0),
                (None, Some(_)) => self.props.color(PortSide::Input, 0),
                (None, None) => col.surface[3],
            };
            return sem.apply(card.bg(tint.with_alpha(0.55)).border(
                if selected { 4.0 } else { 1.0 },
                if selected { theme.accent() } else { tint },
            ));
        }
        card = card
            .border(1.0, style::hairline(&theme).with_alpha(0.12))
            .elevation(if selected {
                Elevation::E2
            } else {
                Elevation::E1
            });
        let header = div()
            .key("header")
            .h(cm.header)
            .px(10.0)
            .flex_row()
            .items_center()
            .radii(livspeil_ui_layout::Corners {
                top_left: theme.tokens.radii.sm + 2.0,
                top_right: theme.tokens.radii.sm + 2.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            })
            .bg(col.surface[2])
            .hit_test(false)
            .child(
                text(n.title.clone())
                    .font_size(m.font)
                    .font_weight(600.0)
                    .text_color(col.text_primary)
                    .hit_test(false),
            );
        card = card.child(header);
        if lod == Lod::Full {
            let rows = n.inputs.len().max(n.outputs.len());
            let label = |s: &SharedString| {
                text(s.clone())
                    .font_size(m.font - 1.0)
                    .text_color(col.text_secondary)
                    .hit_test(false)
            };
            for r in 0..rows {
                let mut row = div()
                    .key(format!("ports-{r}"))
                    .h(cm.row)
                    .px(12.0)
                    .flex_row()
                    .items_center()
                    .hit_test(false);
                row = match n.inputs.get(r) {
                    Some(p) => row.child(label(&p.name)),
                    None => row,
                };
                row = row.child(div().grow());
                row = match n.outputs.get(r) {
                    Some(p) => row.child(label(&p.name)),
                    None => row,
                };
                card = card.child(row);
            }
            for (i, p) in n.params.iter().enumerate() {
                let control = match p.value {
                    ParamValue::Number {
                        value,
                        min,
                        max,
                        step,
                    } => Some(Element::view(&self.field(
                        cx,
                        i,
                        &p.name,
                        value,
                        (min, max, step),
                    ))),
                    ParamValue::Toggle(on) => Some(
                        Toggle::new(on)
                            .label(p.name.clone())
                            .on_change(move |v, cx| {
                                graph.update(cx, |g, mcx| {
                                    g.set_param(id, i, ParamValue::Toggle(v));
                                    mcx.notify();
                                });
                            })
                            .render(&theme)
                            .key("toggle"),
                    ),
                };
                let mut row = div()
                    .key(format!("param-{i}"))
                    .h(cm.row)
                    .px(12.0)
                    .flex_row()
                    .items_center()
                    .gap(8.0);
                if matches!(p.value, ParamValue::Number { .. }) {
                    row = row.child(label(&p.name).grow());
                }
                card = card.child(row.children(control));
            }
        }
        if n.problem.is_some() {
            // Inside the selection ring, so both show.
            card = card.child(
                div()
                    .key("problem-ring")
                    .name(format!("problem:{}", n.title))
                    .style(fill)
                    .rounded(radius)
                    .border(2.0, col.danger)
                    .hit_test(false),
            );
        }
        if selected {
            // The selection ring paints above the header and rows (which
            // would cover a border drawn with the card's own quad), below
            // the port dots.
            card = card.child(
                div()
                    .key("selection-ring")
                    .style(fill)
                    .rounded(radius)
                    .border(2.0, theme.accent())
                    .hit_test(false),
            );
        }
        for side in [PortSide::Input, PortSide::Output] {
            for index in 0..n.ports(side).len() {
                let port = PortRef {
                    node: id,
                    side,
                    index,
                };
                let at = cm.port_position(Point::ZERO, side, index);
                card = card.child(self.port_dot(&theme, port, at));
            }
        }
        sem.apply(card)
    }
}

impl<M: NodeGraphModel> NodeCard<M> {
    fn port_dot(&self, theme: &Theme, port: PortRef, local: Point) -> Element {
        let color = self.props.color(port.side, port.index);
        let surface = theme.colors().surface[1];
        let view = self.canvas;
        let (dim, hot) = match self.props.pending {
            Some(p) => (
                p.own || p.side == port.side,
                p.hot == Some((port.side, port.index)),
            ),
            None => (false, false),
        };
        let r = PORT_DOT / 2.0;
        let mut dot = div()
            .key(format!(
                "port-{}-{}",
                if port.side == PortSide::Input {
                    "in"
                } else {
                    "out"
                },
                port.index
            ))
            .name(format!(
                "port:{}:{}:{}",
                port.node,
                if port.side == PortSide::Input {
                    "in"
                } else {
                    "out"
                },
                port.index
            ))
            .absolute(Rect::new(local.x - r, local.y - r, PORT_DOT, PORT_DOT))
            .rounded(r)
            .bg(color)
            .border(2.0, surface)
            .opacity(if dim { 0.3 } else { 1.0 })
            .scale(if hot { 1.6 } else { 1.0 })
            .hover(|x| x.scale(1.4))
            .transition(MotionClass::Micro)
            .cursor(CursorShape::Crosshair);
        if self.props.lod != Lod::Block {
            dot = dot
                .on_pointer_down(|_, cx| cx.stop_propagation())
                .on_drag(move |ev, cx| port_drag(view, port, ev, cx));
        }
        dot
    }
}
