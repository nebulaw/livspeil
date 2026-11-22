// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The canonical writer.

use std::fmt::Write as _;

use crate::error::DocError;
use crate::model::{Document, ROOT_NETWORK};
use crate::value::{format_float, format_value, is_finite, quote};

const INDENT: &str = "  ";

/// Write `doc` in canonical form.
///
/// The output is deterministic: two-space indentation, nodes sorted by id,
/// each node's inputs sorted by port name, links sorted by
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
            if node.inputs.is_empty() {
                out.push('\n');
                continue;
            }
            out.push_str(" {\n");
            for (port, value) in &node.inputs {
                if !is_finite(value) {
                    return Err(DocError::NonFinite {
                        node: node.name.clone(),
                        port: port.clone(),
                    });
                }
                let text = format_value(value).ok_or_else(|| {
                    DocError::invalid(
                        None,
                        format!(
                            "node '{}' input '{port}': {} values cannot be stored in documents yet",
                            node.name,
                            value.port_type()
                        ),
                    )
                })?;
                let _ = writeln!(out, "{INDENT}{INDENT}{INDENT}input {} {text}", quote(port));
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
