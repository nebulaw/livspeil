// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Public node and Charm API surface.
//!
//! This crate is the part of Livspeil that node and Charm authors compile
//! against. It is licensed under the MPL-2.0 (weak, file-level copyleft;
//! decision D33); the rest of the kernel is GPL. See
//! `specs/livspeil-kernel-spec-v0.md`, sections 2, 4 and 9.
//!
//! Milestone M0.2 scope: concrete port types, [`Value`], [`NodeSpec`],
//! [`NodeImpl`] and [`NodeRegistry`]. Polymorphic constraint classes
//! (`Numeric`, `VectorLike`) and `*.node.toml` generation arrive later.
//!
//! Milestone M0.8 adds [`Value::Table`] ([`Table`], typed column-oriented
//! tables; also lists), [`PortType::Curve`] and [`Value::Curve`]
//! ([`Curve`], float ramps sampled by `livspeil-params`), and system
//! bindings ([`SystemHost`], [`SystemContext`], [`FileSystem`]) passed to
//! nodes through [`NodeImpl::cook_with_system`]; see the [`system`] module
//! for the design and caching semantics.
//!
//! Cargo feature `serde` (off by default) derives `Serialize` and
//! `Deserialize` for [`PortType`] and [`Value`]. It adds derives only, so
//! node authors who do not need it pay nothing. Values use serde's default
//! externally tagged form, for example `{"Float":1.5}`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

pub mod curve;
pub mod system;
pub mod table;

pub use curve::{
    Curve, CurveError, CurveExtrapolation, CurveInterpolation, CurveKey, CurveTangent,
};
pub use system::{DirEntry, FileSystem, FsError, FsPath, SystemContext, SystemHost};
pub use table::{Table, TableColumn, TableError};

/// Value types that can flow through node ports (kernel spec, section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PortType {
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    /// OKLCH plus alpha.
    Color,
    String,
    Enum,
    Event,
    Table,
    Text,
    /// Vector path.
    Path,
    Image,
    Geometry,
    Material,
    Scene,
    Window,
    /// A float curve (ramp); see [`Curve`].
    Curve,
}

impl PortType {
    /// Every port type known to this version of the API.
    pub const ALL: [PortType; 19] = [
        PortType::Float,
        PortType::Int,
        PortType::Bool,
        PortType::Vec2,
        PortType::Vec3,
        PortType::Vec4,
        PortType::Color,
        PortType::String,
        PortType::Enum,
        PortType::Event,
        PortType::Table,
        PortType::Text,
        PortType::Path,
        PortType::Image,
        PortType::Geometry,
        PortType::Material,
        PortType::Scene,
        PortType::Window,
        PortType::Curve,
    ];

    /// Stable name used in documents and node specs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            PortType::Float => "Float",
            PortType::Int => "Int",
            PortType::Bool => "Bool",
            PortType::Vec2 => "Vec2",
            PortType::Vec3 => "Vec3",
            PortType::Vec4 => "Vec4",
            PortType::Color => "Color",
            PortType::String => "String",
            PortType::Enum => "Enum",
            PortType::Event => "Event",
            PortType::Table => "Table",
            PortType::Text => "Text",
            PortType::Path => "Path",
            PortType::Image => "Image",
            PortType::Geometry => "Geometry",
            PortType::Material => "Material",
            PortType::Scene => "Scene",
            PortType::Window => "Window",
            PortType::Curve => "Curve",
        }
    }

    /// Whether a value of type `from` may be connected to a port of type
    /// `self`. Equal types always connect; the only implicit conversion is
    /// the lossless `Int -> Float` (kernel spec, section 4).
    #[must_use]
    pub fn accepts(self, from: PortType) -> bool {
        self == from || (self == PortType::Float && from == PortType::Int)
    }
}

impl fmt::Display for PortType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// A concrete value carried by a port.
///
/// Only the types needed so far exist; more arrive with the packs that
/// introduce them. Equality is IEEE equality, so a `NaN` never equals itself
/// (the scheduler then simply treats it as "changed", which is safe).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Value {
    Float(f64),
    Int(i64),
    Bool(bool),
    Vec2([f64; 2]),
    Vec3([f64; 3]),
    Vec4([f64; 4]),
    /// OKLCH plus alpha: `[lightness, chroma, hue_degrees, alpha]`.
    Color([f64; 4]),
    String(String),
    /// A typed, column-oriented table; cheap to clone.
    Table(Table),
    /// A float curve; cheap to clone.
    Curve(Arc<Curve>),
}

impl Value {
    /// The port type of this value.
    #[must_use]
    pub fn port_type(&self) -> PortType {
        match self {
            Value::Float(_) => PortType::Float,
            Value::Int(_) => PortType::Int,
            Value::Bool(_) => PortType::Bool,
            Value::Vec2(_) => PortType::Vec2,
            Value::Vec3(_) => PortType::Vec3,
            Value::Vec4(_) => PortType::Vec4,
            Value::Color(_) => PortType::Color,
            Value::String(_) => PortType::String,
            Value::Table(_) => PortType::Table,
            Value::Curve(_) => PortType::Curve,
        }
    }

    /// The zero value of a port type, if this version of the API can
    /// represent it. A `Table`'s zero is the empty table (no columns), a
    /// `Curve`'s the constant curve 0.
    #[must_use]
    pub fn zero(ty: PortType) -> Option<Value> {
        Some(match ty {
            PortType::Float => Value::Float(0.0),
            PortType::Int => Value::Int(0),
            PortType::Bool => Value::Bool(false),
            PortType::Vec2 => Value::Vec2([0.0; 2]),
            PortType::Vec3 => Value::Vec3([0.0; 3]),
            PortType::Vec4 => Value::Vec4([0.0; 4]),
            PortType::Color => Value::Color([0.0; 4]),
            PortType::String => Value::String(String::new()),
            PortType::Table => Value::Table(Table::empty()),
            PortType::Curve => Value::Curve(Arc::new(Curve::constant(0.0))),
            _ => return None,
        })
    }

    /// Convert to `ty` using only the implicit conversions allowed by
    /// [`PortType::accepts`]. Returns `None` when the conversion is not
    /// allowed.
    #[must_use]
    pub fn coerce(self, ty: PortType) -> Option<Value> {
        if self.port_type() == ty {
            return Some(self);
        }
        match (self, ty) {
            // Lossy above 2^53 by nature of f64; accepted by the spec.
            (Value::Int(i), PortType::Float) => Some(Value::Float(i as f64)),
            _ => None,
        }
    }

    /// The contained float, if this is a `Float`.
    #[must_use]
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// The contained table, if this is a `Table`.
    #[must_use]
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    /// The contained curve, if this is a `Curve`.
    #[must_use]
    pub fn as_curve(&self) -> Option<&Curve> {
        match self {
            Value::Curve(c) => Some(c),
            _ => None,
        }
    }
}

/// One input or output port of a node type.
#[derive(Debug, Clone, PartialEq)]
pub struct PortSpec {
    pub name: &'static str,
    pub ty: PortType,
    /// Value used when an input is neither linked nor set. Outputs ignore it.
    pub default: Option<Value>,
}

/// Static description of a node type (kernel spec, section 9).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeSpec {
    /// Namespaced id, for example `core.math.lerp`.
    pub id: &'static str,
    /// Spec version; bumped on breaking changes to ports or behavior.
    pub version: u32,
    /// One-line description.
    pub summary: &'static str,
    /// Whether the node must be re-cooked on every frame tick.
    pub time_dependent: bool,
    pub inputs: Vec<PortSpec>,
    pub outputs: Vec<PortSpec>,
}

impl NodeSpec {
    /// Start a spec with no ports.
    #[must_use]
    pub fn new(id: &'static str, version: u32, summary: &'static str) -> Self {
        NodeSpec {
            id,
            version,
            summary,
            time_dependent: false,
            inputs: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// Mark the node as depending on the frame clock.
    #[must_use]
    pub fn time_dependent(mut self) -> Self {
        self.time_dependent = true;
        self
    }

    /// Add an input port with an optional default value.
    #[must_use]
    pub fn input(mut self, name: &'static str, ty: PortType, default: Option<Value>) -> Self {
        self.inputs.push(PortSpec { name, ty, default });
        self
    }

    /// Add an output port.
    #[must_use]
    pub fn output(mut self, name: &'static str, ty: PortType) -> Self {
        self.outputs.push(PortSpec {
            name,
            ty,
            default: None,
        });
        self
    }

    /// Id with version, for example `core.math.lerp@1`.
    #[must_use]
    pub fn qualified_id(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }

    /// Index of the input port called `name`.
    #[must_use]
    pub fn input_index(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|p| p.name == name)
    }

    /// Index of the output port called `name`.
    #[must_use]
    pub fn output_index(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|p| p.name == name)
    }
}

/// Time information handed to a node while cooking.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CookContext {
    /// Frame counter of the frame clock.
    pub frame: u64,
    /// Seconds since the clock started.
    pub time: f64,
    /// Seconds since the previous frame.
    pub dt: f64,
}

/// Why a node failed to cook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeError {
    /// The inputs were valid types but unusable values.
    InvalidInput { port: &'static str, reason: String },
    /// Any other failure.
    Failed(String),
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeError::InvalidInput { port, reason } => {
                write!(f, "invalid input '{port}': {reason}")
            }
            NodeError::Failed(reason) => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for NodeError {}

/// The behavior of a node type.
///
/// Implementations must be **pure**: the outputs depend only on the inputs,
/// the context (when [`NodeSpec::time_dependent`]) and seeded randomness, so
/// that cooking is deterministic and cacheable.
pub trait NodeImpl: Send + Sync + fmt::Debug {
    /// The static description of this node type.
    fn spec(&self) -> &NodeSpec;

    /// Compute the outputs. `inputs` has one value per input port, already
    /// coerced to the port type; the result must have one value per output
    /// port, of the declared type.
    ///
    /// # Errors
    /// Returns a [`NodeError`] when the node cannot produce outputs.
    fn cook(&self, ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError>;

    /// Whether the node reads the system through
    /// [`NodeImpl::cook_with_system`]. Cookers re-cook such nodes when the
    /// host's [`SystemHost::revision`] changes. Default: `false`.
    fn reads_system(&self) -> bool {
        false
    }

    /// Compute the outputs with access to the host's system bindings (see
    /// the [`system`] module). Cookers always call this method; the default
    /// ignores `system` and calls [`NodeImpl::cook`], so only nodes that
    /// read the system override it (and [`NodeImpl::reads_system`]). Such
    /// a node's `cook` should behave as with [`SystemContext::none`].
    ///
    /// # Errors
    /// Returns a [`NodeError`] when the node cannot produce outputs.
    fn cook_with_system(
        &self,
        ctx: &CookContext,
        system: &SystemContext<'_>,
        inputs: &[Value],
    ) -> Result<Vec<Value>, NodeError> {
        let _ = system;
        self.cook(ctx, inputs)
    }
}

/// Why registering a node type failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A node type with the same qualified id is already registered.
    Duplicate(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Duplicate(id) => write!(f, "node type '{id}' is already registered"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// Lookup table from qualified id (`core.math.lerp@1`) to implementation.
#[derive(Debug, Clone, Default)]
pub struct NodeRegistry {
    nodes: BTreeMap<String, Arc<dyn NodeImpl>>,
}

impl NodeRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a node type.
    ///
    /// # Errors
    /// Returns [`RegistryError::Duplicate`] if its qualified id exists.
    pub fn register(&mut self, node: Arc<dyn NodeImpl>) -> Result<(), RegistryError> {
        let id = node.spec().qualified_id();
        if self.nodes.contains_key(&id) {
            return Err(RegistryError::Duplicate(id));
        }
        self.nodes.insert(id, node);
        Ok(())
    }

    /// Look up a node type by qualified id.
    #[must_use]
    pub fn get(&self, qualified_id: &str) -> Option<&Arc<dyn NodeImpl>> {
        self.nodes.get(qualified_id)
    }

    /// All registered qualified ids in sorted order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.nodes.keys().map(String::as_str)
    }

    /// Number of registered node types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether no node type is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn port_type_names_are_unique_and_non_empty() {
        let names: HashSet<&str> = PortType::ALL.iter().map(|t| t.name()).collect();
        assert_eq!(names.len(), PortType::ALL.len());
        assert!(names.iter().all(|n| !n.is_empty()));
    }

    #[test]
    fn all_lists_every_variant_once() {
        let unique: HashSet<PortType> = PortType::ALL.iter().copied().collect();
        assert_eq!(unique.len(), PortType::ALL.len());
    }

    #[test]
    fn accepts_allows_equal_types_and_int_to_float_only() {
        for a in PortType::ALL {
            for b in PortType::ALL {
                let expected = a == b || (a == PortType::Float && b == PortType::Int);
                assert_eq!(a.accepts(b), expected, "{a} <- {b}");
            }
        }
    }

    #[test]
    fn coerce_follows_accepts() {
        assert_eq!(
            Value::Int(3).coerce(PortType::Float),
            Some(Value::Float(3.0))
        );
        assert_eq!(Value::Float(3.0).coerce(PortType::Int), None);
        assert_eq!(Value::Bool(true).coerce(PortType::Float), None);
        assert_eq!(
            Value::Vec3([1.0, 2.0, 3.0]).coerce(PortType::Vec3),
            Some(Value::Vec3([1.0, 2.0, 3.0]))
        );
    }

    #[test]
    fn zero_values_have_their_own_type() {
        for ty in PortType::ALL {
            if let Some(v) = Value::zero(ty) {
                assert_eq!(v.port_type(), ty);
            }
        }
    }

    #[test]
    fn table_and_curve_values() {
        let t = Value::zero(PortType::Table).unwrap();
        assert_eq!(t.as_table().map(Table::column_count), Some(0));
        let c = Value::zero(PortType::Curve).unwrap();
        assert_eq!(c.as_curve().map(|c| c.keys().len()), Some(1));
        assert_eq!(c.clone(), c);
        assert_eq!(t.clone().coerce(PortType::Table), Some(t.clone()));
        assert_eq!(t.coerce(PortType::Curve), None);
        assert!(Value::Int(1).as_table().is_none());
        assert!(Value::Int(1).as_curve().is_none());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn table_and_curve_serde_round_trip_and_validate() {
        let table = Table::from_rows(
            &[("name", PortType::String), ("size", PortType::Int)],
            [vec![Value::String("a".into()), Value::Int(3)]],
        )
        .unwrap();
        let curve = Curve::new(vec![
            CurveKey::new(0.0, 1.0).with_interpolation(CurveInterpolation::Bezier),
            CurveKey::new(1.0, 2.0).with_tangent(CurveTangent::Flat),
        ])
        .unwrap()
        .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::Hold);
        for value in [Value::Table(table), Value::Curve(Arc::new(curve))] {
            let json = serde_json::to_string(&value).unwrap();
            let back: Value = serde_json::from_str(&json).unwrap();
            assert_eq!(back, value, "{json}");
        }
        let bad_table =
            r#"{"Table":{"columns":[{"name":"a","ty":"Int","values":[{"Float":1.0}]}]}}"#;
        assert!(serde_json::from_str::<Value>(bad_table).is_err());
        let bad_curve = r#"{"Curve":{"keys":[]}}"#;
        assert!(serde_json::from_str::<Value>(bad_curve).is_err());
    }

    #[test]
    fn qualified_id_includes_version() {
        let spec = NodeSpec::new("core.math.lerp", 1, "Linear interpolation");
        assert_eq!(spec.qualified_id(), "core.math.lerp@1");
    }

    #[test]
    fn spec_builder_indexes_ports() {
        let spec = NodeSpec::new("test.node", 1, "test")
            .input("a", PortType::Float, Some(Value::Float(0.0)))
            .input("b", PortType::Float, None)
            .output("out", PortType::Float);
        assert_eq!(spec.input_index("b"), Some(1));
        assert_eq!(spec.input_index("nope"), None);
        assert_eq!(spec.output_index("out"), Some(0));
        assert!(!spec.time_dependent);
        assert!(spec.clone().time_dependent().time_dependent);
    }

    #[derive(Debug)]
    struct Dummy(NodeSpec);

    impl NodeImpl for Dummy {
        fn spec(&self) -> &NodeSpec {
            &self.0
        }
        fn cook(&self, _: &CookContext, _: &[Value]) -> Result<Vec<Value>, NodeError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn registry_rejects_duplicates_and_finds_nodes() {
        let mut reg = NodeRegistry::new();
        assert!(reg.is_empty());
        reg.register(Arc::new(Dummy(NodeSpec::new("t.a", 1, "a"))))
            .unwrap();
        reg.register(Arc::new(Dummy(NodeSpec::new("t.a", 2, "a2"))))
            .unwrap();
        let err = reg
            .register(Arc::new(Dummy(NodeSpec::new("t.a", 1, "again"))))
            .unwrap_err();
        assert_eq!(err, RegistryError::Duplicate("t.a@1".to_owned()));
        assert_eq!(reg.len(), 2);
        assert!(reg.get("t.a@2").is_some());
        assert!(reg.get("t.a@3").is_none());
        assert_eq!(reg.ids().collect::<Vec<_>>(), ["t.a@1", "t.a@2"]);
    }
}
