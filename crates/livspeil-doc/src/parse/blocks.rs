// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Reading block values (M0.8): `table { ... }` and `curve { ... }` inside
//! an `input`. See the crate documentation for the syntax.

use std::collections::BTreeSet;
use std::sync::Arc;

use kdl::KdlNode;
use livspeil_node_api::table::is_column_type;
use livspeil_node_api::{PortType, Table, TableColumn, Value};
use livspeil_params::{AnimChannel, AnimError, Extrapolation};

use super::{Reader, children};
use crate::anim;
use crate::error::DocError;
use crate::value::decode;

/// Names of the block values an `input` may hold.
pub(super) const BLOCKS: [&str; 2] = ["table", "curve"];

/// The column types a document may declare, in `PortType::ALL` order.
fn column_types() -> impl Iterator<Item = PortType> {
    PortType::ALL.into_iter().filter(|t| is_column_type(*t))
}

impl Reader<'_> {
    /// A `table` or `curve` child of input `port`.
    pub(super) fn block_value(&self, node: &KdlNode, port: &str) -> Result<Value, DocError> {
        match node.name().value() {
            "table" => self.table(node, port).map(Value::Table),
            _ => self.curve(node, port),
        }
    }

    /// `table { column "<name>" type="<Type>" ... row <value>... ... }`.
    fn table(&self, node: &KdlNode, port: &str) -> Result<Table, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &[])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(
                arg,
                "`table` takes no arguments; declare `column` and `row` children",
            ));
        }
        let mut columns: Vec<TableColumn> = Vec::new();
        let mut cells: Vec<Vec<Value>> = Vec::new();
        let mut names = BTreeSet::new();
        for child in children(node) {
            self.no_annotation(child)?;
            if child.children().is_some() {
                return Err(self.err_node(
                    child,
                    format!("`{}` takes no children block", child.name().value()),
                ));
            }
            match child.name().value() {
                "column" => {
                    if !cells.is_empty() {
                        return Err(self.err_node(
                            child,
                            format!("input '{port}' table: declare every column before the rows"),
                        ));
                    }
                    let entries = self.entries(child)?;
                    self.allow_props(&entries, &["type"])?;
                    let name = self.single_string_arg(child, &entries, "column name")?;
                    if name.is_empty() {
                        return Err(self.err_node(
                            child,
                            format!("input '{port}' table: column names must not be empty"),
                        ));
                    }
                    if !names.insert(name.clone()) {
                        return Err(self.err_node(
                            child,
                            format!("input '{port}' table: duplicate column '{name}'"),
                        ));
                    }
                    let type_entry = entries.props.get("type").ok_or_else(|| {
                        self.err_node(
                            child,
                            format!("column '{name}' needs a type, e.g. type=\"String\""),
                        )
                    })?;
                    let type_name = self.plain_string(type_entry, "type")?;
                    let ty = column_types()
                        .find(|t| t.name() == type_name)
                        .ok_or_else(|| {
                            let names: Vec<&str> = column_types().map(PortType::name).collect();
                            self.err_entry(
                                type_entry,
                                format!(
                                    "unknown column type \"{type_name}\"; expected one of {}",
                                    anim::list(names)
                                ),
                            )
                        })?;
                    columns.push(TableColumn::new(name, ty, Vec::new()));
                }
                "row" => {
                    let entries = self.entries(child)?;
                    self.allow_props(&entries, &[])?;
                    if columns.is_empty() {
                        return Err(self.err_node(
                            child,
                            format!("input '{port}' table: declare columns before rows"),
                        ));
                    }
                    if entries.args.len() != columns.len() {
                        return Err(self.err_node(
                            child,
                            format!(
                                "input '{port}' table row {}: expected {} values (one per column), found {}",
                                cells.len(),
                                columns.len(),
                                entries.args.len()
                            ),
                        ));
                    }
                    let mut row = Vec::with_capacity(columns.len());
                    for (entry, column) in entries.args.iter().zip(&columns) {
                        let what = || {
                            format!(
                                "input '{port}' table row {} column '{}'",
                                cells.len(),
                                column.name()
                            )
                        };
                        let value = decode(entry.ty().map(|t| t.value()), entry.value())
                            .map_err(|msg| self.err_entry(entry, format!("{}: {msg}", what())))?;
                        let found = value.port_type();
                        let value = value.coerce(column.ty()).ok_or_else(|| {
                            self.err_entry(
                                entry,
                                format!("{}: expected {}, found {found}", what(), column.ty()),
                            )
                        })?;
                        row.push(value);
                    }
                    cells.push(row);
                }
                other => {
                    return Err(self.err_node(
                        child,
                        format!(
                            "unknown node `{other}` in table; expected `column \"name\" type=\"Type\"` or `row <values>`"
                        ),
                    ));
                }
            }
        }
        let schema: Vec<(String, PortType)> = columns
            .iter()
            .map(|c| (c.name().to_owned(), c.ty()))
            .collect();
        let schema: Vec<(&str, PortType)> = schema.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        Table::from_rows(&schema, cells)
            .map_err(|e| self.err_node(node, format!("input '{port}' table: {e}")))
    }

    /// `curve [pre="<x>"] [post="<x>"] { key <position> <value> ... }`, with
    /// the keys of an `anim` block: numbers, or all `(vec2)`, `(vec3)`,
    /// `(vec4)` or `(color)` values (typed curves, M0.17).
    fn curve(&self, node: &KdlNode, port: &str) -> Result<Value, DocError> {
        let entries = self.entries(node)?;
        self.allow_props(&entries, &["pre", "post"])?;
        if let Some(arg) = entries.args.first() {
            return Err(self.err_entry(
                arg,
                "`curve` takes no arguments; settings are `pre=` and `post=`",
            ));
        }
        let extrapolation = |key: &str| -> Result<Extrapolation, DocError> {
            let Some(entry) = entries.props.get(key) else {
                return Ok(Extrapolation::Hold);
            };
            let name = self.plain_string(entry, key)?;
            anim::extrapolation(&name).ok_or_else(|| {
                self.err_entry(
                    entry,
                    format!(
                        "unknown {key} extrapolation \"{name}\"; expected one of {}",
                        anim::list(anim::EXTRAPOLATIONS.map(|(n, _)| n))
                    ),
                )
            })
        };
        let pre = extrapolation("pre")?;
        let post = extrapolation("post")?;

        let mut keys = Vec::new();
        let mut key_nodes = Vec::new();
        for child in children(node) {
            self.no_annotation(child)?;
            if child.name().value() != "key" {
                return Err(self.err_node(
                    child,
                    format!(
                        "unknown node `{}` in curve; expected `key <position> <value>`",
                        child.name().value()
                    ),
                ));
            }
            let key = self.key(child)?;
            keys.push(key);
            key_nodes.push(child);
        }
        let channel = AnimChannel::new(keys)
            .map(|c| c.with_extrapolation(pre, post))
            .map_err(|e| {
                let at = match &e {
                    AnimError::UnsupportedType { index, .. }
                    | AnimError::MixedTypes { index, .. }
                    | AnimError::NotIncreasing { index }
                    | AnimError::NonFinite { index }
                    | AnimError::InvalidSpring { index } => key_nodes.get(*index).copied(),
                    _ => None,
                };
                let text = match e {
                    AnimError::NoKeys => "a curve needs at least one key".to_owned(),
                    e => e.to_string().replace("key times", "key positions"),
                };
                DocError::invalid(
                    self.at_node(at.unwrap_or(node)),
                    format!("input '{port}' curve: {text}"),
                )
            })?;
        let curve = channel
            .to_curve()
            .map_err(|e| self.err_node(node, format!("input '{port}' curve: {e}")))?;
        Ok(Value::Curve(Arc::new(curve)))
    }
}
