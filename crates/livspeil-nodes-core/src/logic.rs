// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Logic nodes (milestone M0.6a): selection, comparison and boolean algebra.
//!
//! - `core.logic.select@1` is `Float`-only; `core.logic.select@2` (M0.17)
//!   is polymorphic, `T: Any`, and supersedes it (`@1` stays registered
//!   so existing documents load unchanged).
//! - `core.logic.compare` takes its operator as a `String` input; it becomes
//!   an `Enum` parameter once parameters land (M0.5).

use livspeil_node_api::{
    CookContext, NodeError, NodeImpl, NodeSpec, PortType, TypeClass, TypeVar, Value,
};

use crate::{float, string};

fn boolean(inputs: &[Value], index: usize, port: &'static str) -> Result<bool, NodeError> {
    match inputs.get(index) {
        Some(Value::Bool(b)) => Ok(*b),
        _ => Err(NodeError::InvalidInput {
            port,
            reason: "expected a Bool".to_owned(),
        }),
    }
}

fn bool_in(spec: NodeSpec, name: &'static str, default: bool) -> NodeSpec {
    spec.input(name, PortType::Bool, Some(Value::Bool(default)))
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

/// `core.logic.select@1`: `out = if cond { a } else { b }`.
///
/// `Float`-only; superseded by the polymorphic [`Select2`]
/// (`core.logic.select@2`).
#[derive(Debug)]
pub struct Select {
    spec: NodeSpec,
}

impl Select {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.logic.select", 1, "Pick a when cond is true, else b");
        Select {
            spec: bool_in(spec, "cond", false)
                .input("a", PortType::Float, Some(Value::Float(1.0)))
                .input("b", PortType::Float, Some(Value::Float(0.0)))
                .output("out", PortType::Float),
        }
    }
}
node_boilerplate!(Select);

/// `core.logic.select@2`: `out = if cond { a } else { b }` for any value
/// type: `<T: Any = Float>(cond: Bool, a: T = 1, b: T = 0) -> out: T`.
///
/// `T` resolves per instance from what is linked or set (see
/// `livspeil_node_api::types`); unresolved it is `Float` and the node
/// behaves exactly like `@1`. Defaults follow the resolved type (`1.0`
/// becomes `[1, 1, 1]` for `Vec3`, the grey `oklch(1 0 0)` for colors,
/// the empty string, `false`, the empty table, the constant curve 0).
#[derive(Debug)]
pub struct Select2 {
    spec: NodeSpec,
    vars: Vec<TypeVar>,
}

impl Select2 {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.logic.select",
            2,
            "Pick a when cond is true, else b (any type)",
        );
        Select2 {
            spec: bool_in(spec, "cond", false)
                .input("a", PortType::Float, Some(Value::Float(1.0)))
                .input("b", PortType::Float, Some(Value::Float(0.0)))
                .output("out", PortType::Float),
            vars: vec![
                TypeVar::new("T", TypeClass::Any, PortType::Float)
                    .input("a")
                    .input("b")
                    .output("out"),
            ],
        }
    }
}
node_boilerplate!(Select2);

impl NodeImpl for Select2 {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn type_vars(&self) -> &[TypeVar] {
        &self.vars
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let cond = boolean(inputs, 0, "cond")?;
        let (port, index) = if cond { ("a", 1) } else { ("b", 2) };
        let value = inputs.get(index).ok_or_else(|| NodeError::InvalidInput {
            port,
            reason: "missing input".to_owned(),
        })?;
        Ok(vec![value.clone()])
    }
}

impl NodeImpl for Select {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let cond = boolean(inputs, 0, "cond")?;
        let a = float(inputs, 1, "a")?;
        let b = float(inputs, 2, "b")?;
        Ok(vec![Value::Float(if cond { a } else { b })])
    }
}

/// A comparison operator accepted by [`Compare`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareOp {
    Lt,
    Le,
    Eq,
    Ne,
    Ge,
    Gt,
}

impl CompareOp {
    /// The accepted spellings, in documentation order.
    const NAMES: [&'static str; 6] = ["<", "<=", "==", "!=", ">=", ">"];

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "<" => CompareOp::Lt,
            "<=" => CompareOp::Le,
            "==" => CompareOp::Eq,
            "!=" => CompareOp::Ne,
            ">=" => CompareOp::Ge,
            ">" => CompareOp::Gt,
            _ => return None,
        })
    }

    fn apply(self, a: f64, b: f64) -> bool {
        match self {
            CompareOp::Lt => a < b,
            CompareOp::Le => a <= b,
            CompareOp::Eq => a == b,
            CompareOp::Ne => a != b,
            CompareOp::Ge => a >= b,
            CompareOp::Gt => a > b,
        }
    }
}

/// `core.logic.compare@1`: `out = a <op> b`.
///
/// `op` is one of `<`, `<=`, `==`, `!=`, `>=`, `>` (exact spelling, no
/// whitespace); anything else is an [`NodeError::InvalidInput`] on `op`.
/// Comparisons are IEEE 754: `==` is exact, `-0.0 == 0.0`, and any
/// comparison with `NaN` is false except `!=`, which is true.
///
/// `op` is a `String` input for now and becomes an `Enum` parameter after
/// milestone M0.5.
#[derive(Debug)]
pub struct Compare {
    spec: NodeSpec,
}

impl Compare {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new(
            "core.logic.compare",
            1,
            "Compare a and b with op (<, <=, ==, !=, >=, >)",
        );
        Compare {
            spec: spec
                .input("a", PortType::Float, Some(Value::Float(0.0)))
                .input("b", PortType::Float, Some(Value::Float(0.0)))
                .input("op", PortType::String, Some(Value::String("<".to_owned())))
                .output("out", PortType::Bool),
        }
    }
}
node_boilerplate!(Compare);

impl NodeImpl for Compare {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = float(inputs, 0, "a")?;
        let b = float(inputs, 1, "b")?;
        let text = string(inputs, 2, "op")?;
        let op = CompareOp::parse(text).ok_or_else(|| NodeError::InvalidInput {
            port: "op",
            reason: format!(
                "unknown operator {text:?}; expected one of {}",
                CompareOp::NAMES.join(" ")
            ),
        })?;
        Ok(vec![Value::Bool(op.apply(a, b))])
    }
}

/// `core.logic.and@1`: `out = a && b`. Both inputs default to `true` (the
/// identity), so linking only one passes it through.
#[derive(Debug)]
pub struct And {
    spec: NodeSpec,
}

impl And {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.logic.and", 1, "True when both a and b are true");
        And {
            spec: bool_in(bool_in(spec, "a", true), "b", true).output("out", PortType::Bool),
        }
    }
}
node_boilerplate!(And);

impl NodeImpl for And {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = boolean(inputs, 0, "a")?;
        let b = boolean(inputs, 1, "b")?;
        Ok(vec![Value::Bool(a && b)])
    }
}

/// `core.logic.or@1`: `out = a || b`. Both inputs default to `false` (the
/// identity), so linking only one passes it through.
#[derive(Debug)]
pub struct Or {
    spec: NodeSpec,
}

impl Or {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.logic.or", 1, "True when a or b (or both) is true");
        Or {
            spec: bool_in(bool_in(spec, "a", false), "b", false).output("out", PortType::Bool),
        }
    }
}
node_boilerplate!(Or);

impl NodeImpl for Or {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = boolean(inputs, 0, "a")?;
        let b = boolean(inputs, 1, "b")?;
        Ok(vec![Value::Bool(a || b)])
    }
}

/// `core.logic.not@1`: `out = !a`.
#[derive(Debug)]
pub struct Not {
    spec: NodeSpec,
}

impl Not {
    #[must_use]
    pub fn new() -> Self {
        let spec = NodeSpec::new("core.logic.not", 1, "Negate a");
        Not {
            spec: bool_in(spec, "a", false).output("out", PortType::Bool),
        }
    }
}
node_boilerplate!(Not);

impl NodeImpl for Not {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let a = boolean(inputs, 0, "a")?;
        Ok(vec![Value::Bool(!a)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(node: &dyn NodeImpl, inputs: &[Value]) -> Result<Value, NodeError> {
        let mut out = node.cook(&CookContext::default(), inputs)?;
        assert_eq!(out.len(), 1);
        Ok(out.remove(0))
    }

    fn compare(a: f64, b: f64, op: &str) -> Result<Value, NodeError> {
        run(
            &Compare::new(),
            &[
                Value::Float(a),
                Value::Float(b),
                Value::String(op.to_owned()),
            ],
        )
    }

    #[test]
    fn select2_picks_any_type() {
        let s = Select2::new();
        let ctx = CookContext::default();
        for (a, b) in [
            (Value::Float(1.0), Value::Float(2.0)),
            (Value::Vec3([1.0; 3]), Value::Vec3([2.0; 3])),
            (Value::String("a".into()), Value::String("b".into())),
            (Value::Color([0.5, 0.1, 30.0, 1.0]), Value::Color([0.0; 4])),
        ] {
            let pick = |c| {
                s.cook(&ctx, &[Value::Bool(c), a.clone(), b.clone()])
                    .unwrap()
            };
            assert_eq!(pick(true), vec![a.clone()]);
            assert_eq!(pick(false), vec![b.clone()]);
        }
        assert!(s.cook(&ctx, &[Value::Float(1.0)]).is_err());
        assert!(s.cook(&ctx, &[Value::Bool(true)]).is_err());
        assert_eq!(s.type_vars().len(), 1);
    }

    #[test]
    fn select_picks_by_cond() {
        let s = Select::new();
        let pick = |cond| {
            run(
                &s,
                &[Value::Bool(cond), Value::Float(2.0), Value::Float(-3.0)],
            )
        };
        assert_eq!(pick(true).unwrap(), Value::Float(2.0));
        assert_eq!(pick(false).unwrap(), Value::Float(-3.0));
        // The unselected branch may hold anything, NaN included.
        let out = run(
            &s,
            &[Value::Bool(true), Value::Float(1.0), Value::Float(f64::NAN)],
        );
        assert_eq!(out.unwrap(), Value::Float(1.0));
    }

    #[test]
    fn select_rejects_non_bool_cond() {
        let err = run(
            &Select::new(),
            &[Value::Float(1.0), Value::Float(1.0), Value::Float(0.0)],
        )
        .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "cond", .. }));
    }

    #[test]
    fn compare_every_operator() {
        let table = [
            ("<", [true, false, false]),
            ("<=", [true, true, false]),
            ("==", [false, true, false]),
            ("!=", [true, false, true]),
            (">=", [false, true, true]),
            (">", [false, false, true]),
        ];
        for (op, expected) in table {
            for ((a, b), want) in [(1.0, 2.0), (2.0, 2.0), (3.0, 2.0)]
                .into_iter()
                .zip(expected)
            {
                assert_eq!(
                    compare(a, b, op).unwrap(),
                    Value::Bool(want),
                    "{a} {op} {b}"
                );
            }
        }
    }

    #[test]
    fn compare_ieee_edge_cases() {
        assert_eq!(compare(-0.0, 0.0, "==").unwrap(), Value::Bool(true));
        for op in CompareOp::NAMES {
            let want = op == "!=";
            assert_eq!(
                compare(f64::NAN, 1.0, op).unwrap(),
                Value::Bool(want),
                "{op}"
            );
            assert_eq!(
                compare(f64::NAN, f64::NAN, op).unwrap(),
                Value::Bool(want),
                "{op}"
            );
        }
        assert_eq!(
            compare(f64::INFINITY, f64::INFINITY, "==").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(compare(0.1 + 0.2, 0.3, "==").unwrap(), Value::Bool(false));
    }

    #[test]
    fn compare_unknown_op_is_invalid_input() {
        for op in ["", "=", "< ", "lt", "<>", "=>"] {
            let err = compare(1.0, 2.0, op).unwrap_err();
            assert!(
                matches!(err, NodeError::InvalidInput { port: "op", .. }),
                "{op:?}"
            );
        }
        let err = run(
            &Compare::new(),
            &[Value::Float(1.0), Value::Float(2.0), Value::Float(0.0)],
        )
        .unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "op", .. }));
    }

    #[test]
    fn boolean_truth_tables() {
        for a in [false, true] {
            assert_eq!(
                run(&Not::new(), &[Value::Bool(a)]).unwrap(),
                Value::Bool(!a)
            );
            for b in [false, true] {
                let ins = [Value::Bool(a), Value::Bool(b)];
                assert_eq!(run(&And::new(), &ins).unwrap(), Value::Bool(a && b));
                assert_eq!(run(&Or::new(), &ins).unwrap(), Value::Bool(a || b));
            }
        }
    }

    #[test]
    fn boolean_nodes_reject_floats() {
        let err = run(&And::new(), &[Value::Bool(true), Value::Float(1.0)]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "b", .. }));
        let err = run(&Not::new(), &[]).unwrap_err();
        assert!(matches!(err, NodeError::InvalidInput { port: "a", .. }));
    }
}
