// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Polymorphic ports: constraint classes and type variables (milestone
//! M0.17; kernel spec, section 4: "polymorphic math nodes declare a
//! constraint class resolved when a link is made; no general generics in
//! v0").
//!
//! # Declaring a polymorphic node
//!
//! A node declares **type variables** with [`NodeImpl::type_vars`]. Each
//! [`TypeVar`] has a name (`T`), a constraint class ([`TypeClass`]), a
//! fallback type and the input and output ports it types. Every port of a
//! variable resolves to the same concrete [`PortType`]; ports not named by
//! a variable keep their declared type. In the [`NodeSpec`] the
//! variable's ports are declared with the **fallback** type and defaults
//! of that type, so the spec as written is the node's unresolved instance
//! and code that ignores polymorphism sees a valid concrete node:
//!
//! ```text
//! core.logic.select@2<T: Any = Float>(cond: Bool, a: T, b: T) -> out: T
//! core.math.lerp@2<T: Interpolable = Float>(a: T, b: T, t: Float) -> out: T
//! ```
//!
//! # Instances
//!
//! The graph resolves the variables of every node instance (see
//! `livspeil-graph`, module `types`, for the rules) and cooks the
//! instance returned by [`instantiate`]: a node whose spec has the
//! resolved port types and [`resolved_default`] defaults. A node can build
//! its own instance by overriding [`NodeImpl::with_types`] (needed when
//! `cook` must know a type it cannot see in its inputs, such as an
//! output-only variable); otherwise [`instantiate`] wraps the node and
//! forwards every call, so `cook` receives inputs of the resolved types
//! and must dispatch on the [`Value`] variants. Either way the cooker
//! checks that the outputs have the resolved types.
//!
//! [`NodeImpl::type_vars`]: crate::NodeImpl::type_vars
//! [`NodeImpl::with_types`]: crate::NodeImpl::with_types

use std::fmt;
use std::sync::Arc;

use crate::{
    CookContext, EventCall, NodeError, NodeImpl, NodeSpec, PortSpec, PortType, SystemContext, Value,
};

/// A constraint class: the set of concrete types a type variable may
/// resolve to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum TypeClass {
    /// `Float`, `Int`, `Vec2`, `Vec3`, `Vec4`.
    Numeric,
    /// `Vec2`, `Vec3`, `Vec4` (the kernel spec's `VectorLike`).
    Vector,
    /// Types with a linear interpolation: `Float`, `Vec2`, `Vec3`, `Vec4`,
    /// `Color` (colors interpolate in OKLab).
    Interpolable,
    /// Every type a [`Value`] can hold except `Event`: `Float`, `Int`,
    /// `Bool`, `Vec2`, `Vec3`, `Vec4`, `Color`, `String`, `Table`, `Curve`.
    Any,
}

impl TypeClass {
    /// Every class known to this version of the API.
    pub const ALL: [TypeClass; 4] = [
        TypeClass::Numeric,
        TypeClass::Vector,
        TypeClass::Interpolable,
        TypeClass::Any,
    ];

    /// Stable name used in node specs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            TypeClass::Numeric => "Numeric",
            TypeClass::Vector => "Vector",
            TypeClass::Interpolable => "Interpolable",
            TypeClass::Any => "Any",
        }
    }

    /// The class called `name` ([`TypeClass::name`]; `VectorLike` is
    /// accepted for `Vector`).
    #[must_use]
    pub fn from_name(name: &str) -> Option<TypeClass> {
        match name {
            "VectorLike" => Some(TypeClass::Vector),
            _ => TypeClass::ALL.into_iter().find(|c| c.name() == name),
        }
    }

    /// Whether `ty` is a member of the class.
    #[must_use]
    pub fn contains(self, ty: PortType) -> bool {
        use PortType as P;
        match self {
            TypeClass::Numeric => matches!(ty, P::Float | P::Int | P::Vec2 | P::Vec3 | P::Vec4),
            TypeClass::Vector => matches!(ty, P::Vec2 | P::Vec3 | P::Vec4),
            TypeClass::Interpolable => {
                matches!(ty, P::Float | P::Vec2 | P::Vec3 | P::Vec4 | P::Color)
            }
            TypeClass::Any => ty != P::Event && Value::zero(ty).is_some(),
        }
    }

    /// The members of the class, in [`PortType::ALL`] order.
    pub fn members(self) -> impl Iterator<Item = PortType> {
        PortType::ALL.into_iter().filter(move |t| self.contains(*t))
    }
}

impl fmt::Display for TypeClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A type variable of a polymorphic node. See the [module
/// documentation](self).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TypeVar {
    /// Name, unique per node, for example `T`.
    pub name: &'static str,
    /// The types the variable may resolve to.
    pub class: TypeClass,
    /// The type of an unconstrained variable; a member of `class`, and the
    /// type the variable's ports are declared with in the [`NodeSpec`].
    pub fallback: PortType,
    /// Names of the input ports the variable types.
    pub inputs: Vec<&'static str>,
    /// Names of the output ports the variable types.
    pub outputs: Vec<&'static str>,
}

impl TypeVar {
    /// A variable with no ports yet.
    #[must_use]
    pub fn new(name: &'static str, class: TypeClass, fallback: PortType) -> Self {
        TypeVar {
            name,
            class,
            fallback,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// The same variable also typing input `port`.
    #[must_use]
    pub fn input(mut self, port: &'static str) -> Self {
        self.inputs.push(port);
        self
    }

    /// The same variable also typing output `port`.
    #[must_use]
    pub fn output(mut self, port: &'static str) -> Self {
        self.outputs.push(port);
        self
    }
}

/// Why type variables or their resolution are invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeError {
    /// Two variables share a name.
    DuplicateVar(&'static str),
    /// The fallback is not a member of the class.
    FallbackNotInClass { var: &'static str },
    /// The variable names a port the spec does not have.
    UnknownPort {
        var: &'static str,
        port: &'static str,
    },
    /// The port is named by two variables (or twice by one).
    PortReused { port: &'static str },
    /// The spec declares the port with another type than the fallback.
    NotFallbackType { port: &'static str },
    /// The number of types given is not the number of variables.
    Arity { expected: usize, found: usize },
    /// The type is not a member of the variable's class.
    NotInClass {
        var: &'static str,
        class: TypeClass,
        found: PortType,
    },
    /// A [`NodeImpl::with_types`] instance has another port type than the
    /// resolution asks for.
    BadInstance { port: &'static str },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::DuplicateVar(var) => write!(f, "type variable {var} is declared twice"),
            TypeError::FallbackNotInClass { var } => {
                write!(f, "the fallback of type variable {var} is not in its class")
            }
            TypeError::UnknownPort { var, port } => {
                write!(f, "type variable {var} names unknown port '{port}'")
            }
            TypeError::PortReused { port } => {
                write!(f, "port '{port}' belongs to more than one type variable")
            }
            TypeError::NotFallbackType { port } => write!(
                f,
                "port '{port}' must be declared with its type variable's fallback type"
            ),
            TypeError::Arity { expected, found } => {
                write!(f, "expected {expected} types, found {found}")
            }
            TypeError::NotInClass { var, class, found } => {
                write!(f, "{found} is not {class} (type variable {var})")
            }
            TypeError::BadInstance { port } => write!(
                f,
                "the node's instance gives port '{port}' another type than its resolution"
            ),
        }
    }
}

impl std::error::Error for TypeError {}

/// Which type variable (index into `vars`) types each input and each
/// output port of `spec`: `(inputs, outputs)`.
///
/// # Errors
/// A [`TypeError`] when the variables do not fit the spec (see
/// [`validate_type_vars`]).
#[allow(clippy::type_complexity)]
pub fn port_vars(
    spec: &NodeSpec,
    vars: &[TypeVar],
) -> Result<(Vec<Option<usize>>, Vec<Option<usize>>), TypeError> {
    let mut inputs = vec![None; spec.inputs.len()];
    let mut outputs = vec![None; spec.outputs.len()];
    for (i, var) in vars.iter().enumerate() {
        if vars[..i].iter().any(|v| v.name == var.name) {
            return Err(TypeError::DuplicateVar(var.name));
        }
        if !var.class.contains(var.fallback) {
            return Err(TypeError::FallbackNotInClass { var: var.name });
        }
        for (names, ports, slots) in [
            (&var.inputs, &spec.inputs, &mut inputs),
            (&var.outputs, &spec.outputs, &mut outputs),
        ] {
            for &name in names {
                let index =
                    ports
                        .iter()
                        .position(|p| p.name == name)
                        .ok_or(TypeError::UnknownPort {
                            var: var.name,
                            port: name,
                        })?;
                if slots[index].is_some() {
                    return Err(TypeError::PortReused { port: name });
                }
                if ports[index].ty != var.fallback {
                    return Err(TypeError::NotFallbackType { port: name });
                }
                slots[index] = Some(i);
            }
        }
    }
    Ok((inputs, outputs))
}

/// Check that `vars` fit `spec`: unique names, fallbacks in their
/// classes, known ports declared with the fallback type, each port in at
/// most one variable.
///
/// # Errors
/// The first [`TypeError`] found.
pub fn validate_type_vars(spec: &NodeSpec, vars: &[TypeVar]) -> Result<(), TypeError> {
    port_vars(spec, vars).map(|_| ())
}

/// The default of a port declared with `default` once its type variable
/// resolves to `ty`:
///
/// - a default that converts to `ty` ([`Value::coerce`]) is kept;
/// - a number `x` (`Float` or `Int`) becomes `Int(x)` when `x` is whole,
///   `[x, x, ...]` for vectors and the grey `oklch(x 0 0 1)` for colors;
/// - otherwise, and without a default, the zero value of `ty`.
#[must_use]
pub fn resolved_default(default: Option<&Value>, ty: PortType) -> Option<Value> {
    let Some(default) = default else {
        return Value::zero(ty);
    };
    if let Some(v) = default.clone().coerce(ty) {
        return Some(v);
    }
    let x = match default {
        Value::Float(x) => *x,
        Value::Int(i) => *i as f64,
        _ => return Value::zero(ty),
    };
    Some(match ty {
        PortType::Int if x.fract() == 0.0 && x.abs() < 9.0e15 => Value::Int(x as i64),
        PortType::Vec2 => Value::Vec2([x; 2]),
        PortType::Vec3 => Value::Vec3([x; 3]),
        PortType::Vec4 => Value::Vec4([x; 4]),
        PortType::Color => Value::Color([x, 0.0, 0.0, 1.0]),
        _ => return Value::zero(ty),
    })
}

/// `spec` with the ports of each variable retyped to `types` (one per
/// variable) and their defaults converted by [`resolved_default`].
///
/// # Errors
/// A [`TypeError`] for variables that do not fit the spec, the wrong
/// number of types or a type outside its variable's class.
pub fn resolve_spec(
    spec: &NodeSpec,
    vars: &[TypeVar],
    types: &[PortType],
) -> Result<NodeSpec, TypeError> {
    let (inputs, outputs) = port_vars(spec, vars)?;
    check_types(vars, types)?;
    let mut out = spec.clone();
    for (port, var) in out.inputs.iter_mut().zip(&inputs) {
        if let Some(&ty) = var.and_then(|v| types.get(v)) {
            retype(port, ty);
        }
    }
    for (port, var) in out.outputs.iter_mut().zip(&outputs) {
        if let Some(&ty) = var.and_then(|v| types.get(v)) {
            port.ty = ty;
        }
    }
    Ok(out)
}

fn retype(port: &mut PortSpec, ty: PortType) {
    if port.ty != ty {
        port.default = resolved_default(port.default.as_ref(), ty);
        port.ty = ty;
    }
}

fn check_types(vars: &[TypeVar], types: &[PortType]) -> Result<(), TypeError> {
    if vars.len() != types.len() {
        return Err(TypeError::Arity {
            expected: vars.len(),
            found: types.len(),
        });
    }
    for (var, &ty) in vars.iter().zip(types) {
        if !var.class.contains(ty) {
            return Err(TypeError::NotInClass {
                var: var.name,
                class: var.class,
                found: ty,
            });
        }
    }
    Ok(())
}

/// The instance of `node` whose type variables resolve to `types` (one
/// per [`NodeImpl::type_vars`] entry, in order).
///
/// - When every type is its variable's fallback, `node` itself.
/// - Else the node's own [`NodeImpl::with_types`] instance, checked to
///   have exactly the resolved port types.
/// - Else a wrapper with the [`resolve_spec`] spec that forwards every
///   call to `node`.
///
/// # Errors
/// A [`TypeError`] for invalid variables, the wrong number of types, a
/// type outside its class or a bad [`NodeImpl::with_types`] instance.
pub fn instantiate(
    node: &Arc<dyn NodeImpl>,
    types: &[PortType],
) -> Result<Arc<dyn NodeImpl>, TypeError> {
    let vars = node.type_vars();
    let spec = resolve_spec(node.spec(), vars, types)?;
    if vars.iter().zip(types).all(|(v, &t)| v.fallback == t) {
        return Ok(Arc::clone(node));
    }
    if let Some(own) = node.with_types(types) {
        let got = own.spec();
        let same = |a: &[PortSpec], b: &[PortSpec]| {
            a.iter()
                .zip(b)
                .find(|(x, y)| x.ty != y.ty || x.name != y.name)
                .map(|(x, _)| x.name)
                .or_else(|| (a.len() != b.len()).then_some("<port count>"))
        };
        if let Some(port) =
            same(&got.inputs, &spec.inputs).or_else(|| same(&got.outputs, &spec.outputs))
        {
            return Err(TypeError::BadInstance { port });
        }
        return Ok(own);
    }
    Ok(Arc::new(Resolved {
        inner: Arc::clone(node),
        spec,
    }))
}

/// A polymorphic node with resolved port types, forwarding to the node.
#[derive(Debug)]
struct Resolved {
    inner: Arc<dyn NodeImpl>,
    spec: NodeSpec,
}

impl NodeImpl for Resolved {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        self.inner.cook(ctx, inputs)
    }

    fn reads_system(&self) -> bool {
        self.inner.reads_system()
    }

    fn cook_with_system(
        &self,
        ctx: &CookContext,
        system: &SystemContext<'_>,
        inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        self.inner.cook_with_system(ctx, system, inputs)
    }

    fn polls_events(&self) -> bool {
        self.inner.polls_events()
    }

    fn initial_state(&self) -> Option<Value> {
        self.inner.initial_state()
    }

    fn on_event(&self, call: &mut EventCall<'_>) -> Result<(), NodeError> {
        self.inner.on_event(call)
    }

    fn cook_with_state(
        &self,
        ctx: &CookContext,
        system: &SystemContext<'_>,
        state: Option<&Value>,
        inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        self.inner.cook_with_state(ctx, system, state, inputs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Pick {
        spec: NodeSpec,
        vars: Vec<TypeVar>,
    }

    impl Pick {
        fn new() -> Self {
            Pick {
                spec: NodeSpec::new("t.pick", 1, "pick")
                    .input("cond", PortType::Bool, Some(Value::Bool(false)))
                    .input("a", PortType::Float, Some(Value::Float(1.0)))
                    .input("b", PortType::Float, None)
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

    impl NodeImpl for Pick {
        fn spec(&self) -> &NodeSpec {
            &self.spec
        }
        fn type_vars(&self) -> &[TypeVar] {
            &self.vars
        }
        fn cook(&self, _: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
            let pick = matches!(inputs.first(), Some(Value::Bool(true)));
            Ok(vec![inputs[if pick { 1 } else { 2 }].clone()])
        }
    }

    #[test]
    fn classes() {
        let names: Vec<&str> = TypeClass::ALL.iter().map(|c| c.name()).collect();
        assert_eq!(names, ["Numeric", "Vector", "Interpolable", "Any"]);
        for c in TypeClass::ALL {
            assert_eq!(TypeClass::from_name(c.name()), Some(c));
            assert!(!c.contains(PortType::Event));
            assert!(c.members().all(|t| TypeClass::Any.contains(t)));
        }
        assert_eq!(TypeClass::from_name("VectorLike"), Some(TypeClass::Vector));
        assert_eq!(TypeClass::from_name("Nope"), None);
        assert_eq!(TypeClass::Numeric.members().count(), 5);
        assert_eq!(TypeClass::Vector.members().count(), 3);
        assert!(TypeClass::Interpolable.contains(PortType::Color));
        assert!(!TypeClass::Interpolable.contains(PortType::Int));
        let any: Vec<PortType> = TypeClass::Any.members().collect();
        assert_eq!(any.len(), 10, "{any:?}");
        assert!(!TypeClass::Any.contains(PortType::Text));
    }

    #[test]
    fn validation() {
        let pick = Pick::new();
        assert_eq!(validate_type_vars(&pick.spec, &pick.vars), Ok(()));
        let (i, o) = port_vars(&pick.spec, &pick.vars).unwrap();
        assert_eq!(i, [None, Some(0), Some(0)]);
        assert_eq!(o, [Some(0)]);
        let bad = |vars: Vec<TypeVar>| validate_type_vars(&pick.spec, &vars).unwrap_err();
        let t = || TypeVar::new("T", TypeClass::Any, PortType::Float);
        assert_eq!(bad(vec![t(), t()]), TypeError::DuplicateVar("T"));
        assert_eq!(
            bad(vec![TypeVar::new("T", TypeClass::Vector, PortType::Float)]),
            TypeError::FallbackNotInClass { var: "T" }
        );
        assert_eq!(
            bad(vec![t().input("zz")]),
            TypeError::UnknownPort {
                var: "T",
                port: "zz"
            }
        );
        assert_eq!(
            bad(vec![t().input("a").input("a")]),
            TypeError::PortReused { port: "a" }
        );
        assert_eq!(
            bad(vec![t().input("cond")]),
            TypeError::NotFallbackType { port: "cond" }
        );
        for e in [
            TypeError::Arity {
                expected: 1,
                found: 0,
            },
            TypeError::BadInstance { port: "a" },
        ] {
            assert!(!e.to_string().is_empty());
        }
    }

    #[test]
    fn defaults_follow_the_resolved_type() {
        let one = Value::Float(1.0);
        assert_eq!(
            resolved_default(Some(&one), PortType::Vec3),
            Some(Value::Vec3([1.0; 3]))
        );
        assert_eq!(
            resolved_default(Some(&one), PortType::Color),
            Some(Value::Color([1.0, 0.0, 0.0, 1.0]))
        );
        assert_eq!(
            resolved_default(Some(&one), PortType::Int),
            Some(Value::Int(1))
        );
        assert_eq!(
            resolved_default(Some(&Value::Float(0.5)), PortType::Int),
            Some(Value::Int(0))
        );
        assert_eq!(
            resolved_default(Some(&Value::Int(2)), PortType::Float),
            Some(Value::Float(2.0))
        );
        assert_eq!(
            resolved_default(Some(&one), PortType::String),
            Some(Value::String(String::new()))
        );
        assert_eq!(
            resolved_default(None, PortType::Bool),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn instances() {
        let node: Arc<dyn NodeImpl> = Arc::new(Pick::new());
        let same = instantiate(&node, &[PortType::Float]).unwrap();
        assert!(Arc::ptr_eq(&same, &node));
        let vec = instantiate(&node, &[PortType::Vec2]).unwrap();
        let spec = vec.spec();
        assert_eq!(spec.inputs[0].ty, PortType::Bool);
        assert_eq!(spec.inputs[1].ty, PortType::Vec2);
        assert_eq!(spec.inputs[1].default, Some(Value::Vec2([1.0; 2])));
        assert_eq!(spec.inputs[2].default, Some(Value::Vec2([0.0; 2])));
        assert_eq!(spec.outputs[0].ty, PortType::Vec2);
        assert!(vec.type_vars().is_empty());
        let out = vec
            .cook_with_state(
                &CookContext::default(),
                &SystemContext::none(),
                None,
                &[
                    Value::Bool(true),
                    Value::Vec2([1.0, 2.0]),
                    Value::Vec2([0.0; 2]),
                ],
            )
            .unwrap();
        assert_eq!(out, [Value::Vec2([1.0, 2.0])]);
        assert!(!vec.reads_system() && !vec.polls_events());
        assert_eq!(vec.initial_state(), None);
        assert_eq!(
            instantiate(&node, &[PortType::Event]).err(),
            Some(TypeError::NotInClass {
                var: "T",
                class: TypeClass::Any,
                found: PortType::Event
            })
        );
        assert_eq!(
            instantiate(&node, &[]).err(),
            Some(TypeError::Arity {
                expected: 1,
                found: 0
            })
        );
    }

    #[derive(Debug)]
    struct Liar(Pick);

    impl NodeImpl for Liar {
        fn spec(&self) -> &NodeSpec {
            &self.0.spec
        }
        fn type_vars(&self) -> &[TypeVar] {
            &self.0.vars
        }
        fn with_types(&self, _: &[PortType]) -> Option<Arc<dyn NodeImpl>> {
            Some(Arc::new(Pick::new()))
        }
        fn cook(&self, _: &CookContext, _: &[Value]) -> Result<Vec<Value>, NodeError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn own_instances_are_checked() {
        let node: Arc<dyn NodeImpl> = Arc::new(Liar(Pick::new()));
        assert_eq!(
            instantiate(&node, &[PortType::Vec3]).err(),
            Some(TypeError::BadInstance { port: "a" })
        );
    }
}
