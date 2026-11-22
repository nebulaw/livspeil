// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Typed, column-oriented tables ([`Value::Table`](crate::Value::Table)).
//!
//! A [`Table`] is a list of named columns, each with one [`PortType`] and
//! one value per row; every column has the same number of rows. A table
//! with a single column doubles as a list. Column names are unique and not
//! empty. Columns hold scalar-like values only: `Float`, `Int`, `Bool`,
//! `Vec2`, `Vec3`, `Vec4`, `Color` and `String` (no nested tables or
//! curves). A table with no columns has no rows.
//!
//! Tables are immutable and cheap to clone (the data sits behind an
//! [`Arc`]), because the cook scheduler clones and compares values on
//! every cook. Equality compares the schema and every cell (IEEE equality,
//! so a `NaN` cell makes two separately built tables unequal), except that
//! two clones of the same table are always equal without looking at the
//! cells.
//!
//! With the `serde` feature a table serializes as
//! `{"columns":[{"name":..,"ty":..,"values":[..]}]}` and deserializing
//! validates like [`Table::new`].

use std::fmt;
use std::sync::Arc;

use crate::{PortType, Value};

/// Whether values of type `ty` may be stored in a table column.
#[must_use]
pub fn is_column_type(ty: PortType) -> bool {
    matches!(
        ty,
        PortType::Float
            | PortType::Int
            | PortType::Bool
            | PortType::Vec2
            | PortType::Vec3
            | PortType::Vec4
            | PortType::Color
            | PortType::String
    )
}

/// One named, typed column of a [`Table`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TableColumn {
    name: String,
    ty: PortType,
    values: Vec<Value>,
}

impl TableColumn {
    /// A column; checked when it becomes part of a [`Table`].
    #[must_use]
    pub fn new(name: impl Into<String>, ty: PortType, values: Vec<Value>) -> Self {
        TableColumn {
            name: name.into(),
            ty,
            values,
        }
    }

    /// The column name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The type of every value in the column.
    #[must_use]
    pub fn ty(&self) -> PortType {
        self.ty
    }

    /// The values, one per row.
    #[must_use]
    pub fn values(&self) -> &[Value] {
        &self.values
    }
}

/// Why a table could not be built. Indices are 0-based.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TableError {
    /// Column names must not be empty.
    EmptyName { column: usize },
    /// Two columns share a name.
    DuplicateName(String),
    /// The column type cannot be stored in a table.
    UnsupportedType { column: String, ty: PortType },
    /// A cell has another type than its column.
    WrongType {
        column: String,
        row: usize,
        expected: PortType,
        found: PortType,
    },
    /// A column has another length than the first one.
    ColumnLength {
        column: String,
        expected: usize,
        found: usize,
    },
    /// A row has the wrong number of cells.
    RaggedRows {
        row: usize,
        expected: usize,
        found: usize,
    },
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableError::EmptyName { column } => write!(f, "column {column} has an empty name"),
            TableError::DuplicateName(name) => write!(f, "column '{name}' appears more than once"),
            TableError::UnsupportedType { column, ty } => {
                write!(
                    f,
                    "column '{column}': {ty} values cannot be stored in a table"
                )
            }
            TableError::WrongType {
                column,
                row,
                expected,
                found,
            } => write!(
                f,
                "column '{column}' row {row}: expected {expected}, found {found}"
            ),
            TableError::ColumnLength {
                column,
                expected,
                found,
            } => write!(
                f,
                "column '{column}' has {found} rows, but the first column has {expected}"
            ),
            TableError::RaggedRows {
                row,
                expected,
                found,
            } => write!(f, "row {row}: expected {expected} cells, found {found}"),
        }
    }
}

impl std::error::Error for TableError {}

#[derive(Debug, PartialEq, Default)]
struct TableData {
    columns: Vec<TableColumn>,
    rows: usize,
}

/// An immutable, typed, column-oriented table. See the [module
/// documentation](self).
#[derive(Clone, Default)]
#[cfg_attr(
    feature = "serde",
    derive(serde::Serialize, serde::Deserialize),
    serde(try_from = "RawTable", into = "RawTable")
)]
pub struct Table {
    data: Arc<TableData>,
}

/// Serialized form of a [`Table`].
#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct RawTable {
    columns: Vec<TableColumn>,
}

#[cfg(feature = "serde")]
impl TryFrom<RawTable> for Table {
    type Error = TableError;

    fn try_from(raw: RawTable) -> Result<Self, TableError> {
        Table::new(raw.columns)
    }
}

#[cfg(feature = "serde")]
impl From<Table> for RawTable {
    fn from(table: Table) -> Self {
        RawTable {
            columns: table.data.columns.clone(),
        }
    }
}

impl PartialEq for Table {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) || self.data == other.data
    }
}

impl fmt::Debug for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table")
            .field("rows", &self.data.rows)
            .field("columns", &self.data.columns)
            .finish()
    }
}

impl Table {
    /// The table with no columns and no rows.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// A table from its columns.
    ///
    /// # Errors
    /// A [`TableError`] for empty or duplicate names, unsupported column
    /// types, cells of the wrong type or columns of different lengths.
    pub fn new(columns: Vec<TableColumn>) -> Result<Self, TableError> {
        let rows = columns.first().map_or(0, |c| c.values.len());
        for (index, column) in columns.iter().enumerate() {
            if column.name.is_empty() {
                return Err(TableError::EmptyName { column: index });
            }
            if columns[..index].iter().any(|c| c.name == column.name) {
                return Err(TableError::DuplicateName(column.name.clone()));
            }
            if !is_column_type(column.ty) {
                return Err(TableError::UnsupportedType {
                    column: column.name.clone(),
                    ty: column.ty,
                });
            }
            if column.values.len() != rows {
                return Err(TableError::ColumnLength {
                    column: column.name.clone(),
                    expected: rows,
                    found: column.values.len(),
                });
            }
            if let Some((row, value)) = column
                .values
                .iter()
                .enumerate()
                .find(|(_, v)| v.port_type() != column.ty)
            {
                return Err(TableError::WrongType {
                    column: column.name.clone(),
                    row,
                    expected: column.ty,
                    found: value.port_type(),
                });
            }
        }
        Ok(Table {
            data: Arc::new(TableData { columns, rows }),
        })
    }

    /// A table from a schema and rows of cells in column order.
    ///
    /// # Errors
    /// As [`Table::new`]; a row with the wrong number of cells is
    /// [`TableError::RaggedRows`].
    pub fn from_rows(
        schema: &[(&str, PortType)],
        rows: impl IntoIterator<Item = Vec<Value>>,
    ) -> Result<Self, TableError> {
        let mut columns: Vec<TableColumn> = schema
            .iter()
            .map(|(name, ty)| TableColumn::new(*name, *ty, Vec::new()))
            .collect();
        for (index, row) in rows.into_iter().enumerate() {
            if row.len() != columns.len() {
                return Err(TableError::RaggedRows {
                    row: index,
                    expected: columns.len(),
                    found: row.len(),
                });
            }
            for (column, value) in columns.iter_mut().zip(row) {
                column.values.push(value);
            }
        }
        Table::new(columns)
    }

    /// A single-column table (a list).
    ///
    /// # Errors
    /// As [`Table::new`].
    pub fn list(name: &str, ty: PortType, values: Vec<Value>) -> Result<Self, TableError> {
        Table::new(vec![TableColumn::new(name, ty, values)])
    }

    /// Number of rows.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.data.rows
    }

    /// Number of columns.
    #[must_use]
    pub fn column_count(&self) -> usize {
        self.data.columns.len()
    }

    /// Whether the table has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.rows == 0
    }

    /// The columns, in order.
    #[must_use]
    pub fn columns(&self) -> &[TableColumn] {
        &self.data.columns
    }

    /// Index of the column called `name`.
    #[must_use]
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.data.columns.iter().position(|c| c.name == name)
    }

    /// The column called `name`.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&TableColumn> {
        self.data.columns.iter().find(|c| c.name == name)
    }

    /// The cell at `row` in column number `column`.
    #[must_use]
    pub fn cell(&self, row: usize, column: usize) -> Option<&Value> {
        self.data.columns.get(column)?.values.get(row)
    }

    /// The cells of `row` in column order.
    pub fn row(&self, row: usize) -> impl Iterator<Item = &Value> + '_ {
        let valid = row < self.data.rows;
        self.data
            .columns
            .iter()
            .filter(move |_| valid)
            .filter_map(move |c| c.values.get(row))
    }

    /// A table with the same columns and the rows at `rows`, in that order
    /// (indices may repeat). Out-of-range indices are skipped.
    #[must_use]
    pub fn select_rows(&self, rows: &[usize]) -> Table {
        let rows: Vec<usize> = rows
            .iter()
            .copied()
            .filter(|&r| r < self.data.rows)
            .collect();
        let columns = self
            .data
            .columns
            .iter()
            .map(|c| TableColumn {
                name: c.name.clone(),
                ty: c.ty,
                values: rows.iter().map(|&r| c.values[r].clone()).collect(),
            })
            .collect();
        Table {
            data: Arc::new(TableData {
                columns,
                rows: rows.len(),
            }),
        }
    }

    /// A single-column table holding column number `column`.
    #[must_use]
    pub fn select_column(&self, column: usize) -> Option<Table> {
        let c = self.data.columns.get(column)?;
        Some(Table {
            data: Arc::new(TableData {
                columns: vec![c.clone()],
                rows: self.data.rows,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Value {
        Value::String(v.to_owned())
    }

    fn books() -> Table {
        Table::from_rows(
            &[("name", PortType::String), ("size", PortType::Int)],
            [vec![s("a"), Value::Int(3)], vec![s("b"), Value::Int(1)]],
        )
        .unwrap()
    }

    #[test]
    fn builds_and_reads() {
        let t = books();
        assert_eq!(t.row_count(), 2);
        assert_eq!(t.column_count(), 2);
        assert_eq!(t.column_index("size"), Some(1));
        assert_eq!(t.cell(1, 0), Some(&s("b")));
        assert_eq!(t.cell(2, 0), None);
        assert_eq!(
            t.row(0).cloned().collect::<Vec<_>>(),
            [s("a"), Value::Int(3)]
        );
        assert_eq!(t.row(5).count(), 0);
        assert!(Table::empty().is_empty());
        assert_eq!(Table::empty().column_count(), 0);
    }

    #[test]
    fn rejects_bad_tables() {
        let err = |cols| Table::new(cols).unwrap_err();
        assert_eq!(
            err(vec![TableColumn::new("", PortType::Int, vec![])]),
            TableError::EmptyName { column: 0 }
        );
        assert_eq!(
            err(vec![
                TableColumn::new("a", PortType::Int, vec![]),
                TableColumn::new("a", PortType::Int, vec![]),
            ]),
            TableError::DuplicateName("a".into())
        );
        assert!(matches!(
            err(vec![TableColumn::new("t", PortType::Table, vec![])]),
            TableError::UnsupportedType { .. }
        ));
        assert!(matches!(
            err(vec![TableColumn::new(
                "a",
                PortType::Int,
                vec![Value::Float(1.0)]
            )]),
            TableError::WrongType { row: 0, .. }
        ));
        assert!(matches!(
            err(vec![
                TableColumn::new("a", PortType::Int, vec![Value::Int(1)]),
                TableColumn::new("b", PortType::Int, vec![]),
            ]),
            TableError::ColumnLength { .. }
        ));
        assert!(matches!(
            Table::from_rows(&[("a", PortType::Int)], [vec![]]),
            Err(TableError::RaggedRows {
                row: 0,
                expected: 1,
                found: 0
            })
        ));
    }

    #[test]
    fn selection_and_equality() {
        let t = books();
        let rev = t.select_rows(&[1, 0, 9]);
        assert_eq!(rev.row_count(), 2);
        assert_eq!(rev.cell(0, 0), Some(&s("b")));
        assert_ne!(rev, t);
        assert_eq!(rev.select_rows(&[1, 0]), t);
        let names = t.select_column(0).unwrap();
        assert_eq!(
            names,
            Table::list("name", PortType::String, vec![s("a"), s("b")]).unwrap()
        );
        assert!(t.select_column(2).is_none());
        // Clones are equal even with NaN cells; separate builds are not.
        let nan = Table::list("x", PortType::Float, vec![Value::Float(f64::NAN)]).unwrap();
        assert_eq!(nan, nan.clone());
        assert_ne!(
            nan,
            Table::list("x", PortType::Float, vec![Value::Float(f64::NAN)]).unwrap()
        );
    }
}
