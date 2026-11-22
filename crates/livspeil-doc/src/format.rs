// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical writer.

use std::fmt::Write as _;

use livspeil_node_api::Value;
use livspeil_params::{AnimChannel, Extrapolation, Interpolation, Tangent};

use crate::anim::{extrapolation_name, interpolation_name};
use crate::error::DocError;
use crate::model::{DocNode, Document, ROOT_NETWORK};
use crate::value::{format_float, format_value, is_finite, quote};

const INDENT: &str = "  ";

/// Write `doc` in canonical form.
///
/// The output is deterministic: two-space indentation, nodes sorted by id,
/// each node's inputs sorted by port name (one `input` per port holding,
/// in this order, its static value, `expr=` and `anim` block; key
/// properties in the order `interp`, `frequency`, `damping`, `tangent`,
/// `in`, `out`, with default settings left out), links sorted by
/// `(from node, from port, to node, to port)`, meta positions sorted by
/// node name, floats in shortest round-trip form, strings always quoted.
/// Comments are not part of the model and are not written.
///
/// # Errors
/// Fails with [`DocError::NonFinite`] if an input or position is `NaN` or
/// infinite, and with [`DocError::Invalid`] for a value type this format
/// cannot store.
pub fn format(doc: &Document) -> Result<String, DocError> {
    let mut network = doc.network.clone();
    network.sort();

    let mut out = String::new();
    let _ = writeln!(
        out,
        "room {} format={} min-tier={} {{",
        quote(&doc.name),
        doc.format,
        quote(&doc.min_tier.to_string())
    );

    let _ = write!(out, "{INDENT}network {}", quote(ROOT_NETWORK));
    if network.nodes.is_empty() && network.links.is_empty() {
        out.push('\n');
    } else {
        out.push_str(" {\n");
        for node in &network.nodes {
            let _ = write!(
                out,
                "{INDENT}{INDENT}node {} type={} id={}",
                quote(&node.name),
                quote(&node.type_id),
                quote(&node.id.to_string())
            );
            let ports: Vec<&str> = node.ports().collect();
            if ports.is_empty() {
                out.push('\n');
                continue;
            }
            out.push_str(" {\n");
            for port in ports {
                write_input(&mut out, node, port)?;
            }
            let _ = writeln!(out, "{INDENT}{INDENT}}}");
        }
        for link in &network.links {
            let _ = writeln!(
                out,
                "{INDENT}{INDENT}link from={} to={}",
                quote(&format!("{}.{}", link.from_node, link.from_port)),
                quote(&format!("{}.{}", link.to_node, link.to_port)),
            );
        }
        let _ = writeln!(out, "{INDENT}}}");
    }

    if !doc.meta.is_empty() {
        let _ = writeln!(out, "{INDENT}meta {{");
        for (name, [x, y]) in &doc.meta.positions {
            if !x.is_finite() || !y.is_finite() {
                return Err(DocError::invalid(
                    None,
                    format!("meta position of node '{name}' is NaN or infinite"),
                ));
            }
            let _ = writeln!(
                out,
                "{INDENT}{INDENT}node {} x={} y={}",
                quote(name),
                format_float(*x),
                format_float(*y)
            );
        }
        let _ = writeln!(out, "{INDENT}}}");
    }

    out.push_str("}\n");
    Ok(out)
}

/// One `input` line (and its `anim` block) for a port with any source.
fn write_input(out: &mut String, node: &DocNode, port: &str) -> Result<(), DocError> {
    let _ = write!(out, "{INDENT}{INDENT}{INDENT}input {}", quote(port));
    if let Some(value) = node.inputs.get(port) {
        let _ = write!(out, " {}", value_text(node, port, value)?);
    }
    if let Some(expr) = node.expressions.get(port) {
        let _ = write!(out, " expr={}", quote(&expr.source));
    }
    let Some(anim) = node.animations.get(port) else {
        out.push('\n');
        return Ok(());
    };
    out.push_str(" {\n");
    write_anim(out, node, port, &anim.channel)?;
    let _ = writeln!(out, "{INDENT}{INDENT}{INDENT}}}");
    Ok(())
}

fn write_anim(
    out: &mut String,
    node: &DocNode,
    port: &str,
    channel: &AnimChannel,
) -> Result<(), DocError> {
    let pad = INDENT.repeat(4);
    out.push_str(&pad);
    out.push_str("anim");
    if channel.pre() != Extrapolation::Hold {
        let _ = write!(out, " pre={}", quote(extrapolation_name(channel.pre())));
    }
    if channel.post() != Extrapolation::Hold {
        let _ = write!(out, " post={}", quote(extrapolation_name(channel.post())));
    }
    // Compare bits so that an offset of -0.0 survives the round trip.
    if channel.offset().to_bits() != 0 {
        let _ = write!(out, " offset={}", float_text(node, port, channel.offset())?);
    }
    out.push_str(" {\n");
    for key in channel.keys() {
        let _ = write!(
            out,
            "{pad}{INDENT}key {} {}",
            float_text(node, port, key.time)?,
            value_text(node, port, &key.value)?
        );
        if key.interpolation != Interpolation::Linear {
            let _ = write!(
                out,
                " interp={}",
                quote(interpolation_name(&key.interpolation))
            );
        }
        if let Interpolation::Spring { frequency, damping } = key.interpolation {
            let _ = write!(
                out,
                " frequency={} damping={}",
                float_text(node, port, frequency)?,
                float_text(node, port, damping)?
            );
        }
        match &key.tangent {
            Tangent::Auto => {}
            Tangent::Flat => out.push_str(" tangent=\"flat\""),
            Tangent::Explicit { incoming, outgoing } => {
                let _ = write!(
                    out,
                    " tangent=\"explicit\" in={} out={}",
                    value_text(node, port, incoming)?,
                    value_text(node, port, outgoing)?
                );
            }
        }
        out.push('\n');
    }
    let _ = writeln!(out, "{pad}}}");
    Ok(())
}

fn float_text(node: &DocNode, port: &str, v: f64) -> Result<String, DocError> {
    value_text(node, port, &Value::Float(v))
}

fn value_text(node: &DocNode, port: &str, value: &Value) -> Result<String, DocError> {
    if !is_finite(value) {
        return Err(DocError::NonFinite {
            node: node.name.clone(),
            port: port.to_owned(),
        });
    }
    format_value(value).ok_or_else(|| {
        DocError::invalid(
            None,
            format!(
                "node '{}' input '{port}': {} values cannot be stored in documents yet",
                node.name,
                value.port_type()
            ),
        )
    })
}
