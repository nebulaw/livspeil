// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the inspector shows of one node: where each input comes from and
//! what each output is now.

use livspeil_graph::{Cooker, Graph, NodeId};
use livspeil_node_api::Value;

use crate::shown::{Port, Shown};

/// Where an input's value comes from, in the priority of kernel spec 5.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A link from `from` (`node.port`), carrying the value if known.
    Link {
        /// The output linked from.
        from: String,
        /// Its current value.
        value: Option<Value>,
    },
    /// An expression (its source text).
    Expression(String),
    /// An animation channel.
    Animation,
    /// A value set on the input.
    Set(Value),
    /// The node type's default.
    Default(Value),
    /// Nothing, and the type has no default.
    Unset,
}

/// One input or output.
#[derive(Debug, Clone, PartialEq)]
pub struct Row<T> {
    /// Port name.
    pub name: String,
    /// Type name.
    pub ty: &'static str,
    /// The input's source, or the output's value.
    pub value: T,
}

/// The inspected node.
#[derive(Debug, Clone, PartialEq)]
pub struct Inspection {
    /// Node name.
    pub name: String,
    /// Qualified type id.
    pub type_id: String,
    /// Inputs.
    pub inputs: Vec<Row<Source>>,
    /// Outputs and their current values.
    pub outputs: Vec<Row<Option<Value>>>,
    /// Why it failed, if it did.
    pub problem: Option<String>,
}

/// The current value of output `port` of graph node `node`.
pub(crate) fn output(
    graph: &Graph,
    cooker: &mut Cooker,
    node: NodeId,
    port: &str,
) -> Option<Value> {
    let at = graph.node_spec(node)?.output_index(port)?;
    cooker.cook(graph, node).ok()?.into_iter().nth(at)
}

/// How `(node, port)` is written for the reader: by the shown node that has
/// that output.
fn written(graph: &Graph, nodes: &[Shown], node: NodeId, port: &str) -> String {
    nodes
        .iter()
        .find_map(|n| {
            n.outputs
                .iter()
                .find(|p| p.node == node && p.port == port)
                .map(|p| format!("{}.{}", n.name, p.name))
        })
        .unwrap_or_else(|| format!("{}.{port}", graph.path(node).unwrap_or_default()))
}

fn source(graph: &Graph, cooker: &mut Cooker, nodes: &[Shown], port: &Port) -> Source {
    let (node, name) = (port.node, port.port.as_str());
    if let Ok(Some((from, from_port))) = graph.link_source(node, name) {
        return Source::Link {
            from: written(graph, nodes, from, from_port),
            value: output(graph, cooker, from, from_port),
        };
    }
    if let Ok(Some(expression)) = graph.expression(node, name) {
        return Source::Expression(expression.to_owned());
    }
    if matches!(graph.animation(node, name), Ok(Some(_))) {
        return Source::Animation;
    }
    if let Ok(Some(value)) = graph.input_value(node, name) {
        return Source::Set(value.clone());
    }
    graph
        .node_spec(node)
        .and_then(|s| s.inputs.iter().find(|p| p.name == name))
        .and_then(|p| p.default.clone())
        .map_or(Source::Unset, Source::Default)
}

/// Inspect `node`, one of the `nodes` shown.
#[must_use]
pub(crate) fn inspect(
    graph: &Graph,
    cooker: &mut Cooker,
    nodes: &[Shown],
    node: &Shown,
    problem: Option<String>,
) -> Inspection {
    Inspection {
        name: node.name.clone(),
        type_id: graph.node_type(node.id).unwrap_or_default().to_owned(),
        inputs: node
            .inputs
            .iter()
            .map(|p| Row {
                name: p.name.clone(),
                ty: p.ty.name(),
                value: source(graph, cooker, nodes, p),
            })
            .collect(),
        outputs: node
            .outputs
            .iter()
            .map(|p| Row {
                name: p.name.clone(),
                ty: p.ty.name(),
                value: output(graph, cooker, p.node, &p.port),
            })
            .collect(),
        problem,
    }
}

/// A number with at most four decimals and no trailing zeros.
fn number(x: f64) -> String {
    let s = format!("{x:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

fn numbers(v: &[f64]) -> String {
    v.iter().map(|&x| number(x)).collect::<Vec<_>>().join("  ")
}

/// A value as the inspector writes it.
#[must_use]
pub fn written_value(value: &Value) -> String {
    match value {
        Value::Float(x) => number(*x),
        Value::Int(x) => x.to_string(),
        Value::Bool(x) => x.to_string(),
        Value::Vec2(v) => numbers(v),
        Value::Vec3(v) => numbers(v),
        Value::Vec4(v) => numbers(v),
        Value::Color([l, c, h, a]) => format!("oklch {}", numbers(&[*l, *c, *h, *a])),
        Value::String(s) => format!("{s:?}"),
        Value::Table(_) => "table".to_owned(),
        Value::Curve(_) => "curve".to_owned(),
        Value::Event => "event".to_owned(),
        _ => value.port_type().name().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(written_value(&Value::Float(0.5)), "0.5");
        assert_eq!(written_value(&Value::Float(8640.0)), "8640");
        assert_eq!(written_value(&Value::Float(-0.000_001)), "0");
        assert_eq!(written_value(&Value::Vec2([1.0, 2.25])), "1  2.25");
    }
}
