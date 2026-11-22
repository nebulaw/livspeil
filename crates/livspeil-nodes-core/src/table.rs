// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Table nodes (milestone M0.8): `core.table.count`, `core.table.column`,
//! `core.table.get`, `core.table.filter_text` and `core.table.sort`.
//!
//! Tables are immutable values ([`Table`]); every node returns a new table
//! and never fails on an empty one. Naming a column that does not exist is
//! a [`NodeError::InvalidInput`] on the port that names it, listing the
//! columns that do exist, except on a table without any column (the
//! default of an unconnected input), which every node treats as an empty
//! table of the right shape: no rows, nothing found.
//!
//! **Typed access.** Ports have fixed types while table columns have any
//! scalar type, so `core.table.get` returns a cell three ways at once: as
//! text (`text`, the `str()` form of expressions: strings unchanged,
//! floats in shortest round-trip form, vectors `vec3(1.0, 2.0, 3.0)`), as
//! a number (`number`: Floats, Ints converted, Bools 0 or 1, anything else
//! 0) and whether the cell exists (`found`). Typed per-column outputs can
//! come later with polymorphic ports.

use std::cmp::Ordering;

use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeSpec, PortType, Table, TableColumn, Value,
};
use livspeil_params::expr::format_value;

use crate::string;

fn table<'a>(
    inputs: &'a [Value],
    index: usize,
    port: &'static str,
) -> Result<&'a Table, NodeError> {
    inputs
        .get(index)
        .and_then(Value::as_table)
        .ok_or_else(|| NodeError::InvalidInput {
            port,
            reason: "expected a Table".to_owned(),
        })
}

fn boolean(inputs: &[Value], index: usize, port: &'static str) -> Result<bool, NodeError> {
    match inputs.get(index) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(NodeError::InvalidInput {
            port,
            reason: "expected a Bool".to_owned(),
        }),
    }
}

fn count(n: usize) -> Value {
    Value::Int(i64::try_from(n).unwrap_or(i64::MAX))
}

/// The index of column `name`, or an error naming the columns there are.
/// `Ok(None)` for a table without columns.
fn column_index(table: &Table, name: &str, port: &'static str) -> Result<Option<usize>, NodeError> {
    if table.column_count() == 0 {
        return Ok(None);
    }
    table.column_index(name).map(Some).ok_or_else(|| {
        let names: Vec<String> = table
            .columns()
            .iter()
            .map(|c| format!("\"{}\"", c.name()))
            .collect();
        NodeError::InvalidInput {
            port,
            reason: format!(
                "no column \"{name}\" (columns: {})",
                if names.is_empty() {
                    "none".to_owned()
                } else {
                    names.join(", ")
                }
            ),
        }
    })
}

fn empty_table() -> Option<Value> {
    Some(Value::Table(Table::empty()))
}

fn text(v: &str) -> Option<Value> {
    Some(Value::String(v.to_owned()))
}

macro_rules! node_boilerplate {
    ($ty:ident) => {
        impl Default for $ty {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

/// `core.table.count@1`: `count` (Int) is the number of rows of `table`,
/// `columns` (Int) the number of columns.
#[derive(Debug)]
pub struct TableCount {
    spec: NodeSpec,
}

impl TableCount {
    #[must_use]
    pub fn new() -> Self {
        TableCount {
            spec: NodeSpec::new(
                "core.table.count",
                1,
                "Number of rows and columns of a table",
            )
            .input("table", PortType::Table, empty_table())
            .output("count", PortType::Int)
            .output("columns", PortType::Int),
        }
    }
}
node_boilerplate!(TableCount);

impl NodeImpl for TableCount {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let t = table(inputs, 0, "table")?;
        Ok(vec![count(t.row_count()), count(t.column_count())])
    }
}

/// `core.table.column@1`: `out` is the single-column table (a list) of
/// column `name` of `table`.
#[derive(Debug)]
pub struct TableColumnNode {
    spec: NodeSpec,
}

impl TableColumnNode {
    #[must_use]
    pub fn new() -> Self {
        TableColumnNode {
            spec: NodeSpec::new("core.table.column", 1, "One column of a table, as a list")
                .input("table", PortType::Table, empty_table())
                .input("name", PortType::String, text("name"))
                .output("out", PortType::Table),
        }
    }
}
node_boilerplate!(TableColumnNode);

impl NodeImpl for TableColumnNode {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let t = table(inputs, 0, "table")?;
        let name = string(inputs, 1, "name")?;
        let out = column_index(t, name, "name")?
            .and_then(|index| t.select_column(index))
            .unwrap_or_default();
        Ok(vec![Value::Table(out)])
    }
}

/// `core.table.get@1`: one cell. Inputs `table`, `row` (Int, 0-based),
/// `column` (String, default `"name"`). Outputs `text` (String), `number`
/// (Float) and `found` (Bool); see the module documentation. A row out of
/// range (negative or past the end) gives `found = false`, `text = ""`,
/// `number = 0`; an unknown column is an error.
#[derive(Debug)]
pub struct TableGet {
    spec: NodeSpec,
}

impl TableGet {
    #[must_use]
    pub fn new() -> Self {
        TableGet {
            spec: NodeSpec::new(
                "core.table.get",
                1,
                "One cell of a table as text and number",
            )
            .input("table", PortType::Table, empty_table())
            .input("row", PortType::Int, Some(Value::Int(0)))
            .input("column", PortType::String, text("name"))
            .output("text", PortType::String)
            .output("number", PortType::Float)
            .output("found", PortType::Bool),
        }
    }
}
node_boilerplate!(TableGet);

/// The numeric view of a cell.
fn number(value: &Value) -> f64 {
    match value {
        Value::Float(v) => *v,
        // Lossy above 2^53 like every Int to Float conversion.
        Value::Int(i) => *i as f64,
        Value::Bool(b) => f64::from(u8::from(*b)),
        _ => 0.0,
    }
}

impl NodeImpl for TableGet {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let t = table(inputs, 0, "table")?;
        let row = match inputs.get(1) {
            Some(Value::Int(r)) => *r,
            _ => {
                return Err(NodeError::InvalidInput {
                    port: "row",
                    reason: "expected an Int".to_owned(),
                });
            }
        };
        let column = column_index(t, string(inputs, 2, "column")?, "column")?;
        let cell = column.and_then(|c| usize::try_from(row).ok().and_then(|r| t.cell(r, c)));
        Ok(match cell {
            Some(v) => vec![
                Value::String(format_value(v)),
                Value::Float(number(v)),
                Value::Bool(true),
            ],
            None => vec![
                Value::String(String::new()),
                Value::Float(0.0),
                Value::Bool(false),
            ],
        })
    }
}

/// `core.table.filter_text@1`: the rows of `table` whose String column
/// `column` (default `"name"`) contains `text` (default `""`, which keeps
/// every row), in their original order. With `ignore_case` (default
/// false) both sides are lowercased first (Unicode lowercase). Outputs
/// `out` (Table) and `count` (Int, rows kept). A column that is not a
/// String column is an error.
#[derive(Debug)]
pub struct TableFilterText {
    spec: NodeSpec,
}

impl TableFilterText {
    #[must_use]
    pub fn new() -> Self {
        TableFilterText {
            spec: NodeSpec::new(
                "core.table.filter_text",
                1,
                "Keep the rows whose text column contains a substring",
            )
            .input("table", PortType::Table, empty_table())
            .input("column", PortType::String, text("name"))
            .input("text", PortType::String, text(""))
            .input("ignore_case", PortType::Bool, Some(Value::Bool(false)))
            .output("out", PortType::Table)
            .output("count", PortType::Int),
        }
    }
}
node_boilerplate!(TableFilterText);

impl NodeImpl for TableFilterText {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let t = table(inputs, 0, "table")?;
        let index = column_index(t, string(inputs, 1, "column")?, "column")?;
        let needle = string(inputs, 2, "text")?;
        let ignore_case = boolean(inputs, 3, "ignore_case")?;
        let Some(column) = index.map(|i| &t.columns()[i]) else {
            return Ok(vec![Value::Table(t.clone()), count(0)]);
        };
        let column: &TableColumn = column;
        if column.ty() != PortType::String {
            return Err(NodeError::InvalidInput {
                port: "column",
                reason: format!(
                    "column \"{}\" holds {} values; filter_text needs a String column",
                    column.name(),
                    column.ty()
                ),
            });
        }
        let needle = if ignore_case {
            needle.to_lowercase()
        } else {
            needle.to_owned()
        };
        let keep: Vec<usize> = column
            .values()
            .iter()
            .enumerate()
            .filter(|(_, v)| match v {
                Value::String(s) if ignore_case => s.to_lowercase().contains(&needle),
                Value::String(s) => s.contains(&needle),
                _ => false,
            })
            .map(|(i, _)| i)
            .collect();
        let out = t.select_rows(&keep);
        let n = out.row_count();
        Ok(vec![Value::Table(out), count(n)])
    }
}

/// Total order of two cells of one column: numbers by IEEE total order
/// (`-0.0` before `0.0`, NaN after infinity), `false` before `true`,
/// strings by Unicode scalar values, vectors and colors component by
/// component.
fn compare(a: &Value, b: &Value) -> Ordering {
    fn lex(a: &[f64], b: &[f64]) -> Ordering {
        a.iter()
            .zip(b)
            .map(|(x, y)| x.total_cmp(y))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
    }
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.total_cmp(y),
        (Value::Int(x), Value::Int(y)) => x.cmp(y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        (Value::Vec2(x), Value::Vec2(y)) => lex(x, y),
        (Value::Vec3(x), Value::Vec3(y)) => lex(x, y),
        (Value::Vec4(x), Value::Vec4(y)) | (Value::Color(x), Value::Color(y)) => lex(x, y),
        _ => Ordering::Equal,
    }
}

/// `core.table.sort@1`: `out` is `table` with its rows sorted by column
/// `column` (default `"name"`), ascending by default (`ascending`). The
/// sort is stable in both directions: rows with equal keys keep their
/// order. See [`compare`] for the order of each type.
#[derive(Debug)]
pub struct TableSort {
    spec: NodeSpec,
}

impl TableSort {
    #[must_use]
    pub fn new() -> Self {
        TableSort {
            spec: NodeSpec::new("core.table.sort", 1, "Sort the rows of a table by a column")
                .input("table", PortType::Table, empty_table())
                .input("column", PortType::String, text("name"))
                .input("ascending", PortType::Bool, Some(Value::Bool(true)))
                .output("out", PortType::Table),
        }
    }
}
node_boilerplate!(TableSort);

impl NodeImpl for TableSort {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let t = table(inputs, 0, "table")?;
        let index = column_index(t, string(inputs, 1, "column")?, "column")?;
        let ascending = boolean(inputs, 2, "ascending")?;
        let Some(index) = index else {
            return Ok(vec![Value::Table(t.clone())]);
        };
        let values = t.columns()[index].values();
        let mut order: Vec<usize> = (0..t.row_count()).collect();
        order.sort_by(|&a, &b| {
            let o = compare(&values[a], &values[b]);
            if ascending { o } else { o.reverse() }
        });
        Ok(vec![Value::Table(t.select_rows(&order))])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::String(v.to_owned())
    }

    fn books() -> Value {
        Value::Table(
            Table::from_rows(
                &[("name", PortType::String), ("size", PortType::Int)],
                [
                    vec![s("Dune"), Value::Int(3)],
                    vec![s("Emma"), Value::Int(1)],
                    vec![s("dracula"), Value::Int(3)],
                ],
            )
            .unwrap(),
        )
    }

    fn cook(node: &dyn NodeImpl, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        node.cook(&CookContext::default(), inputs)
    }

    fn names(v: &Value) -> Vec<String> {
        let t = v.as_table().unwrap();
        let i = t.column_index("name").unwrap();
        t.columns()[i]
            .values()
            .iter()
            .map(|v| match v {
                Value::String(s) => s.clone(),
                _ => panic!(),
            })
            .collect()
    }

    #[test]
    fn count_and_column() {
        let out = cook(&TableCount::new(), &[books()]).unwrap();
        assert_eq!(out, [Value::Int(3), Value::Int(2)]);
        let out = cook(&TableColumnNode::new(), &[books(), s("size")]).unwrap();
        let t = out[0].as_table().unwrap();
        assert_eq!((t.column_count(), t.row_count()), (1, 3));
        let err = cook(&TableColumnNode::new(), &[books(), s("nope")]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid input 'name': no column \"nope\" (columns: \"name\", \"size\")"
        );
    }

    #[test]
    fn get_cells() {
        let get = |row, col: &str| cook(&TableGet::new(), &[books(), Value::Int(row), s(col)]);
        assert_eq!(
            get(1, "name").unwrap(),
            [s("Emma"), Value::Float(0.0), Value::Bool(true)]
        );
        assert_eq!(
            get(0, "size").unwrap(),
            [s("3"), Value::Float(3.0), Value::Bool(true)]
        );
        for row in [-1, 3, i64::MAX] {
            assert_eq!(
                get(row, "name").unwrap(),
                [s(""), Value::Float(0.0), Value::Bool(false)]
            );
        }
        assert!(get(0, "x").is_err());
        // A table without columns is empty for every node, whatever the name.
        let none = Value::Table(Table::empty());
        assert_eq!(
            cook(&TableGet::new(), &[none.clone(), Value::Int(0), s("x")]).unwrap()[2],
            Value::Bool(false)
        );
        assert_eq!(
            cook(&TableColumnNode::new(), &[none.clone(), s("x")]).unwrap()[0],
            none
        );
    }

    #[test]
    fn filter_and_sort() {
        let filter = |text: &str, ignore| {
            cook(
                &TableFilterText::new(),
                &[books(), s("name"), s(text), Value::Bool(ignore)],
            )
            .unwrap()
        };
        assert_eq!(names(&filter("D", false)[0]), ["Dune"]);
        let out = filter("d", true);
        assert_eq!(names(&out[0]), ["Dune", "dracula"]);
        assert_eq!(out[1], Value::Int(2));
        assert_eq!(names(&filter("", false)[0]).len(), 3);
        assert!(
            cook(
                &TableFilterText::new(),
                &[books(), s("size"), s(""), Value::Bool(false)]
            )
            .is_err()
        );

        let sort =
            |col: &str, asc| cook(&TableSort::new(), &[books(), s(col), Value::Bool(asc)]).unwrap();
        assert_eq!(names(&sort("name", true)[0]), ["Dune", "Emma", "dracula"]);
        assert_eq!(names(&sort("name", false)[0]), ["dracula", "Emma", "Dune"]);
        // Stable: Dune and dracula tie on size and keep their order.
        assert_eq!(names(&sort("size", true)[0]), ["Emma", "Dune", "dracula"]);
        assert_eq!(names(&sort("size", false)[0]), ["Dune", "dracula", "Emma"]);
    }

    #[test]
    fn compare_orders_every_type() {
        assert_eq!(
            compare(&Value::Float(-0.0), &Value::Float(0.0)),
            Ordering::Less
        );
        assert_eq!(
            compare(&Value::Float(f64::NAN), &Value::Float(f64::INFINITY)),
            Ordering::Greater
        );
        assert_eq!(
            compare(&Value::Bool(false), &Value::Bool(true)),
            Ordering::Less
        );
        assert_eq!(
            compare(&Value::Vec2([1.0, 2.0]), &Value::Vec2([1.0, 1.0])),
            Ordering::Greater
        );
        assert_eq!(compare(&s("B"), &s("a")), Ordering::Less);
    }
}
