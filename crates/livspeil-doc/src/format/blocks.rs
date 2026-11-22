// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Writing block values (M0.8): `table { ... }` and `curve { ... }` inside
//! an `input`. See the crate documentation for the syntax.

use std::fmt::Write as _;

use livspeil_node_api::{Curve, CurveExtrapolation, Table, Value};
use livspeil_params::AnimChannel;

use super::{INDENT, value_text, write_keys};
use crate::anim::extrapolation_name;
use crate::error::DocError;
use crate::model::DocNode;
use crate::value::quote;

/// Whether `value` is written as a block rather than inline.
pub(super) fn is_block(value: &Value) -> bool {
    matches!(value, Value::Table(_) | Value::Curve(_))
}

/// The block of a `Table` or `Curve` value, at the depth of an `anim`.
pub(super) fn write_block(
    out: &mut String,
    node: &DocNode,
    port: &str,
    value: &Value,
) -> Result<(), DocError> {
    let pad = INDENT.repeat(4);
    match value {
        Value::Table(table) => write_table(out, node, port, table, &pad),
        Value::Curve(curve) => write_curve(out, node, port, curve, &pad),
        _ => Ok(()),
    }
}

fn write_table(
    out: &mut String,
    node: &DocNode,
    port: &str,
    table: &Table,
    pad: &str,
) -> Result<(), DocError> {
    out.push_str(pad);
    out.push_str("table");
    if table.column_count() == 0 {
        out.push('\n');
        return Ok(());
    }
    out.push_str(" {\n");
    for column in table.columns() {
        let _ = writeln!(
            out,
            "{pad}{INDENT}column {} type={}",
            quote(column.name()),
            quote(column.ty().name())
        );
    }
    for row in 0..table.row_count() {
        let _ = write!(out, "{pad}{INDENT}row");
        for cell in table.row(row) {
            let _ = write!(out, " {}", value_text(node, port, cell)?);
        }
        out.push('\n');
    }
    let _ = writeln!(out, "{pad}}}");
    Ok(())
}

fn write_curve(
    out: &mut String,
    node: &DocNode,
    port: &str,
    curve: &Curve,
    pad: &str,
) -> Result<(), DocError> {
    let channel = AnimChannel::from_curve(curve).map_err(|e| {
        DocError::invalid(
            None,
            format!("node '{}' input '{port}': invalid curve: {e}", node.name),
        )
    })?;
    out.push_str(pad);
    out.push_str("curve");
    if curve.pre() != CurveExtrapolation::Hold {
        let _ = write!(out, " pre={}", quote(extrapolation_name(channel.pre())));
    }
    if curve.post() != CurveExtrapolation::Hold {
        let _ = write!(out, " post={}", quote(extrapolation_name(channel.post())));
    }
    out.push_str(" {\n");
    write_keys(out, node, port, channel.keys(), pad)?;
    let _ = writeln!(out, "{pad}}}");
    Ok(())
}
