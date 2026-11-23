// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `*.node.toml` node specs (kernel spec, section 9; milestone M0.13).
//!
//! Every registered core node has a spec file
//! `specs/<id>.node.toml` (`<id>@<version>.node.toml` for versions other
//! than 1). The file is the node's documentation and test table:
//!
//! - its header (`id`, `version`, `summary`, `time_dependent`,
//!   `reads_system`, `[[type_vars]]`, `[[inputs]]`, `[[outputs]]`) is
//!   generated from the Rust [`NodeSpec`] (and the node's type variables)
//!   and must match it exactly;
//! - `description` and every `[[tests]]` entry are hand-written and kept
//!   when the file is regenerated.
//!
//! This test checks that there is exactly one file per registered node,
//! that each header matches the Rust spec (and is in canonical form), runs
//! every `[[tests]]` case through the node's cook and checks that
//! `docs/nodes.md` is up to date. The file format is described in
//! [`DOCS_PREAMBLE`] (rendered into `docs/nodes.md`).
//!
//! Regenerate the headers (and create files for new nodes) with
//! `cargo xtask node-specs`, which runs this test with
//! `UPDATE_NODE_SPECS=1`; then review the diff.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use livspeil_node_api::{
    CookContext, Curve, CurveExtrapolation, CurveInterpolation, CurveKey, CurveTangent, Event,
    EventCall, EventTrigger, NodeError, NodeImpl, NodeSpec, PortType, SystemContext, Table,
    TableColumn, TypeClass, TypeVar, TypedCurveKey, TypedCurveTangent, Value, instantiate,
};
use livspeil_nodes_core::{MemFs, core_registry};

const UPDATE_ENV: &str = "UPDATE_NODE_SPECS";

const HOW_TO_UPDATE: &str = "run `cargo xtask node-specs` (same as \
     `UPDATE_NODE_SPECS=1 cargo test -p livspeil-nodes-core --test node_specs`) \
     and review the diff";

const FILE_HEADER: &str = "\
# SPDX-FileCopyrightText: 2026 Livspeil contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Node spec (kernel spec, section 9). The keys and ports above the first
# [[tests]] are generated from the Rust NodeSpec by `cargo xtask node-specs`
# and checked by crates/livspeil-nodes-core/tests/node_specs.rs; only
# `description` and the [[tests]] are hand-written. Format: docs/nodes.md.
";

const TOP_KEYS: [&str; 10] = [
    "id",
    "version",
    "summary",
    "description",
    "time_dependent",
    "reads_system",
    "type_vars",
    "inputs",
    "outputs",
    "tests",
];
const PORT_KEYS: [&str; 5] = ["name", "type", "var", "default", "default_debug"];
const VAR_KEYS: [&str; 3] = ["name", "class", "fallback"];
const TEST_KEYS: [&str; 11] = [
    "name",
    "types",
    "inputs",
    "context",
    "fs",
    "events",
    "expect",
    "expect_events",
    "expect_error",
    "tolerance",
    "note",
];
const STEP_KEYS: [&str; 5] = ["input", "tick", "payload", "inputs", "context"];

/// Whether a port's default is the `Event` placeholder, which spec files
/// leave out (an `Event` port carries no value).
fn event_placeholder(port: &livspeil_node_api::PortSpec) -> bool {
    port.ty == PortType::Event && port.default == Some(Value::Event)
}

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn specs_dir() -> PathBuf {
    crate_dir().join("specs")
}

fn docs_path() -> PathBuf {
    crate_dir().join("../../docs/nodes.md")
}

/// File name of a node's spec: `<id>.node.toml`, or `<id>@<v>.node.toml`
/// for versions other than 1.
fn file_name(spec: &NodeSpec) -> String {
    if spec.version == 1 {
        format!("{}.node.toml", spec.id)
    } else {
        format!("{}@{}.node.toml", spec.id, spec.version)
    }
}

// ---------------------------------------------------------------------------
// Value encoding
// ---------------------------------------------------------------------------

fn float_text(v: f64) -> String {
    if v.is_nan() {
        "nan".to_owned()
    } else if v.is_infinite() {
        if v > 0.0 { "inf" } else { "-inf" }.to_owned()
    } else {
        // Shortest round-trip form; always contains '.' or 'e', so it is
        // a TOML float.
        format!("{v:?}")
    }
}

/// A TOML basic string.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn floats(values: &[f64]) -> String {
    let parts: Vec<String> = values.iter().map(|v| float_text(*v)).collect();
    format!("[{}]", parts.join(", "))
}

fn interpolation_name(i: CurveInterpolation) -> String {
    match i {
        CurveInterpolation::Step => quote("step"),
        CurveInterpolation::Linear => quote("linear"),
        CurveInterpolation::Bezier => quote("bezier"),
        CurveInterpolation::Spring { frequency, damping } => format!(
            "{{ spring = {{ frequency = {}, damping = {} }} }}",
            float_text(frequency),
            float_text(damping)
        ),
    }
}

fn extrapolation_name(e: CurveExtrapolation) -> &'static str {
    match e {
        CurveExtrapolation::Hold => "hold",
        CurveExtrapolation::Loop => "loop",
        CurveExtrapolation::PingPong => "ping_pong",
    }
}

fn encode_curve(curve: &Curve) -> String {
    if curve.value_type() != PortType::Float {
        return encode_typed_curve(curve);
    }
    let keys: Vec<String> = curve
        .keys()
        .iter()
        .map(|k| {
            let mut key = format!(
                "{{ position = {}, value = {}",
                float_text(k.position),
                float_text(k.value)
            );
            if k.interpolation != CurveInterpolation::Linear {
                let _ = write!(
                    key,
                    ", interpolation = {}",
                    interpolation_name(k.interpolation)
                );
            }
            match k.tangent {
                CurveTangent::Auto => {}
                CurveTangent::Flat => key.push_str(", tangent = \"flat\""),
                CurveTangent::Explicit { incoming, outgoing } => {
                    let _ = write!(
                        key,
                        ", tangent = {{ incoming = {}, outgoing = {} }}",
                        float_text(incoming),
                        float_text(outgoing)
                    );
                }
            }
            key.push_str(" }");
            key
        })
        .collect();
    let mut out = format!("{{ keys = [{}]", keys.join(", "));
    if curve.pre() != CurveExtrapolation::Hold {
        let _ = write!(out, ", pre = \"{}\"", extrapolation_name(curve.pre()));
    }
    if curve.post() != CurveExtrapolation::Hold {
        let _ = write!(out, ", post = \"{}\"", extrapolation_name(curve.post()));
    }
    out.push_str(" }");
    out
}

/// A curve of vector or color keys: `{ type = "Vec3", keys = [...] }`
/// with array values and tangents.
fn encode_typed_curve(curve: &Curve) -> String {
    let text = |v: &Value| encode(v).unwrap_or_default();
    let keys: Vec<String> = curve
        .typed_keys()
        .iter()
        .map(|k| {
            let mut key = format!(
                "{{ position = {}, value = {}",
                float_text(k.position),
                text(&k.value)
            );
            if k.interpolation != CurveInterpolation::Linear {
                let _ = write!(
                    key,
                    ", interpolation = {}",
                    interpolation_name(k.interpolation)
                );
            }
            match &k.tangent {
                TypedCurveTangent::Auto => {}
                TypedCurveTangent::Flat => key.push_str(", tangent = \"flat\""),
                TypedCurveTangent::Explicit { incoming, outgoing } => {
                    let _ = write!(
                        key,
                        ", tangent = {{ incoming = {}, outgoing = {} }}",
                        text(incoming),
                        text(outgoing)
                    );
                }
            }
            key.push_str(" }");
            key
        })
        .collect();
    let mut out = format!(
        "{{ type = {}, keys = [{}]",
        quote(curve.value_type().name()),
        keys.join(", ")
    );
    if curve.pre() != CurveExtrapolation::Hold {
        let _ = write!(out, ", pre = \"{}\"", extrapolation_name(curve.pre()));
    }
    if curve.post() != CurveExtrapolation::Hold {
        let _ = write!(out, ", post = \"{}\"", extrapolation_name(curve.post()));
    }
    out.push_str(" }");
    out
}

/// The TOML text of a value, or `None` for a value type the format does
/// not cover yet (the spec then records `default_debug`).
fn encode(value: &Value) -> Option<String> {
    Some(match value {
        Value::Float(v) => float_text(*v),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::String(s) => quote(s),
        Value::Vec2(v) => floats(v),
        Value::Vec3(v) => floats(v),
        Value::Vec4(v) | Value::Color(v) => floats(v),
        Value::Table(t) => {
            let columns = t
                .columns()
                .iter()
                .map(|c| {
                    let values = c.values().iter().map(encode).collect::<Option<Vec<_>>>()?;
                    Some(format!(
                        "{{ name = {}, type = {}, values = [{}] }}",
                        quote(c.name()),
                        quote(c.ty().name()),
                        values.join(", ")
                    ))
                })
                .collect::<Option<Vec<_>>>()?;
            format!("{{ columns = [{}] }}", columns.join(", "))
        }
        Value::Curve(c) => encode_curve(c),
        _ => return None,
    })
}

fn port_type(name: &str) -> Result<PortType, String> {
    PortType::ALL
        .into_iter()
        .find(|t| t.name() == name)
        .ok_or_else(|| format!("unknown port type {name:?}"))
}

fn type_name(v: &toml::Value) -> &'static str {
    v.type_str()
}

fn number(v: &toml::Value) -> Result<f64, String> {
    match v {
        toml::Value::Float(f) => Ok(*f),
        // Lossless for the small integers written in specs.
        toml::Value::Integer(i) => Ok(*i as f64),
        other => Err(format!("expected a number, got a {}", type_name(other))),
    }
}

fn fixed<const N: usize>(v: &toml::Value, ty: PortType) -> Result<[f64; N], String> {
    let toml::Value::Array(items) = v else {
        return Err(format!("a {ty} is an array of {N} numbers"));
    };
    let mut out = [0.0; N];
    // A Color may leave out alpha (then 1.0).
    if ty == PortType::Color && items.len() == 3 {
        out[3] = 1.0;
    } else if items.len() != N {
        return Err(format!(
            "a {ty} is an array of {N} numbers, got {}",
            items.len()
        ));
    }
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = number(item)?;
    }
    Ok(out)
}

fn table_of<'a>(v: &'a toml::Value, what: &str) -> Result<&'a toml::Table, String> {
    v.as_table()
        .ok_or_else(|| format!("{what} must be a table, got a {}", type_name(v)))
}

fn check_keys(table: &toml::Table, allowed: &[&str], what: &str) -> Result<(), String> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "unknown key {key:?} in {what} (allowed: {})",
                allowed.join(", ")
            ));
        }
    }
    Ok(())
}

fn str_field<'a>(table: &'a toml::Table, key: &str, what: &str) -> Result<&'a str, String> {
    match table.get(key) {
        Some(toml::Value::String(s)) => Ok(s),
        Some(other) => Err(format!(
            "{what}.{key} must be a string, got a {}",
            type_name(other)
        )),
        None => Err(format!("{what}.{key} is missing")),
    }
}

fn decode_table(v: &toml::Value) -> Result<Table, String> {
    let t = table_of(v, "a Table value")?;
    check_keys(t, &["columns"], "a Table value")?;
    let Some(columns) = t.get("columns") else {
        return Ok(Table::empty());
    };
    let toml::Value::Array(columns) = columns else {
        return Err("Table columns must be an array".to_owned());
    };
    let mut out = Vec::new();
    for column in columns {
        let c = table_of(column, "a Table column")?;
        check_keys(c, &["name", "type", "values"], "a Table column")?;
        let name = str_field(c, "name", "column")?;
        let ty = port_type(str_field(c, "type", "column")?)?;
        let values = match c.get("values") {
            Some(toml::Value::Array(values)) => values
                .iter()
                .map(|v| decode(ty, v))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            Some(_) => return Err(format!("column {name:?}: values must be an array")),
        };
        out.push(TableColumn::new(name, ty, values));
    }
    Table::new(out).map_err(|e| e.to_string())
}

fn decode_interpolation(v: &toml::Value) -> Result<CurveInterpolation, String> {
    match v {
        toml::Value::String(s) => match s.as_str() {
            "step" => Ok(CurveInterpolation::Step),
            "linear" => Ok(CurveInterpolation::Linear),
            "bezier" => Ok(CurveInterpolation::Bezier),
            other => Err(format!(
                "unknown interpolation {other:?}; expected step, linear, bezier or {{ spring = {{ frequency, damping }} }}"
            )),
        },
        toml::Value::Table(t) => {
            check_keys(t, &["spring"], "an interpolation")?;
            let spring = table_of(
                t.get("spring").ok_or("interpolation table needs spring")?,
                "spring",
            )?;
            check_keys(spring, &["frequency", "damping"], "spring")?;
            let get = |k: &str| {
                spring
                    .get(k)
                    .ok_or_else(|| format!("spring.{k} is missing"))
                    .and_then(number)
            };
            Ok(CurveInterpolation::Spring {
                frequency: get("frequency")?,
                damping: get("damping")?,
            })
        }
        other => Err(format!("bad interpolation: {}", type_name(other))),
    }
}

fn decode_tangent(v: &toml::Value) -> Result<CurveTangent, String> {
    match v {
        toml::Value::String(s) if s == "auto" => Ok(CurveTangent::Auto),
        toml::Value::String(s) if s == "flat" => Ok(CurveTangent::Flat),
        toml::Value::Table(t) => {
            check_keys(t, &["incoming", "outgoing"], "a tangent")?;
            let get = |k: &str| {
                t.get(k)
                    .ok_or_else(|| format!("tangent.{k} is missing"))
                    .and_then(number)
            };
            Ok(CurveTangent::Explicit {
                incoming: get("incoming")?,
                outgoing: get("outgoing")?,
            })
        }
        _ => Err("tangent must be \"auto\", \"flat\" or { incoming, outgoing }".to_owned()),
    }
}

fn decode_extrapolation(v: Option<&toml::Value>) -> Result<CurveExtrapolation, String> {
    match v.map(toml::Value::as_str) {
        None => Ok(CurveExtrapolation::Hold),
        Some(Some("hold")) => Ok(CurveExtrapolation::Hold),
        Some(Some("loop")) => Ok(CurveExtrapolation::Loop),
        Some(Some("ping_pong")) => Ok(CurveExtrapolation::PingPong),
        Some(_) => Err("extrapolation must be \"hold\", \"loop\" or \"ping_pong\"".to_owned()),
    }
}

/// A curve key type: `Float` (default), `Vec2`, `Vec3`, `Vec4` or
/// `Color`.
fn curve_type(t: &toml::Table) -> Result<PortType, String> {
    match t.get("type") {
        None => Ok(PortType::Float),
        Some(v) => {
            let ty = port_type(v.as_str().ok_or("a curve type must be a string")?)?;
            if matches!(
                ty,
                PortType::Float
                    | PortType::Vec2
                    | PortType::Vec3
                    | PortType::Vec4
                    | PortType::Color
            ) {
                Ok(ty)
            } else {
                Err(format!("curves cannot hold {ty} keys"))
            }
        }
    }
}

fn decode_typed_tangent(v: &toml::Value, ty: PortType) -> Result<TypedCurveTangent, String> {
    match v {
        toml::Value::String(s) if s == "auto" => Ok(TypedCurveTangent::Auto),
        toml::Value::String(s) if s == "flat" => Ok(TypedCurveTangent::Flat),
        toml::Value::Table(t) => {
            check_keys(t, &["incoming", "outgoing"], "a tangent")?;
            let get = |k: &str| {
                t.get(k)
                    .ok_or_else(|| format!("tangent.{k} is missing"))
                    .and_then(|v| decode(ty, v))
            };
            Ok(TypedCurveTangent::Explicit {
                incoming: get("incoming")?,
                outgoing: get("outgoing")?,
            })
        }
        _ => Err("tangent must be \"auto\", \"flat\" or { incoming, outgoing }".to_owned()),
    }
}

fn decode_curve(v: &toml::Value) -> Result<Curve, String> {
    let t = table_of(v, "a Curve value")?;
    check_keys(t, &["type", "keys", "pre", "post"], "a Curve value")?;
    let ty = curve_type(t)?;
    if ty != PortType::Float {
        let Some(toml::Value::Array(keys)) = t.get("keys") else {
            return Err("a Curve value needs keys = [...]".to_owned());
        };
        let mut out = Vec::new();
        for key in keys {
            let k = table_of(key, "a curve key")?;
            check_keys(
                k,
                &["position", "value", "interpolation", "tangent"],
                "a curve key",
            )?;
            let position = number(k.get("position").ok_or("curve key position is missing")?)?;
            let value = decode(ty, k.get("value").ok_or("curve key value is missing")?)?;
            let mut key = TypedCurveKey::new(position, value);
            if let Some(i) = k.get("interpolation") {
                key = key.with_interpolation(decode_interpolation(i)?);
            }
            if let Some(tangent) = k.get("tangent") {
                key = key.with_tangent(decode_typed_tangent(tangent, ty)?);
            }
            out.push(key);
        }
        return Ok(Curve::typed(out)
            .map_err(|e| e.to_string())?
            .with_extrapolation(
                decode_extrapolation(t.get("pre"))?,
                decode_extrapolation(t.get("post"))?,
            ));
    }
    let Some(toml::Value::Array(keys)) = t.get("keys") else {
        return Err("a Curve value needs keys = [...]".to_owned());
    };
    let mut out = Vec::new();
    for key in keys {
        let k = table_of(key, "a curve key")?;
        check_keys(
            k,
            &["position", "value", "interpolation", "tangent"],
            "a curve key",
        )?;
        let get = |name: &str| {
            k.get(name)
                .ok_or_else(|| format!("curve key {name} is missing"))
                .and_then(number)
        };
        let mut key = CurveKey::new(get("position")?, get("value")?);
        if let Some(i) = k.get("interpolation") {
            key = key.with_interpolation(decode_interpolation(i)?);
        }
        if let Some(tangent) = k.get("tangent") {
            key = key.with_tangent(decode_tangent(tangent)?);
        }
        out.push(key);
    }
    Ok(Curve::new(out)
        .map_err(|e| e.to_string())?
        .with_extrapolation(
            decode_extrapolation(t.get("pre"))?,
            decode_extrapolation(t.get("post"))?,
        ))
}

/// Decode a TOML value as a value of port type `ty`.
fn decode(ty: PortType, v: &toml::Value) -> Result<Value, String> {
    let wrong = || format!("expected a {ty}, got a {}", type_name(v));
    Ok(match ty {
        PortType::Float => Value::Float(number(v).map_err(|_| wrong())?),
        PortType::Int => Value::Int(v.as_integer().ok_or_else(wrong)?),
        PortType::Bool => Value::Bool(v.as_bool().ok_or_else(wrong)?),
        PortType::String => Value::String(v.as_str().ok_or_else(wrong)?.to_owned()),
        PortType::Vec2 => Value::Vec2(fixed(v, ty)?),
        PortType::Vec3 => Value::Vec3(fixed(v, ty)?),
        PortType::Vec4 => Value::Vec4(fixed(v, ty)?),
        PortType::Color => Value::Color(fixed(v, ty)?),
        PortType::Table => Value::Table(decode_table(v)?),
        PortType::Curve => Value::Curve(Arc::new(decode_curve(v)?)),
        other => {
            return Err(format!(
                "{other} values cannot be written in node specs yet"
            ));
        }
    })
}

/// Whether two floats match: both NaN, or equal within `tolerance`. A zero
/// tolerance compares bits, so `-0.0` and `0.0` differ.
fn float_close(a: f64, b: f64, tolerance: f64) -> bool {
    if a.is_nan() || b.is_nan() {
        return a.is_nan() && b.is_nan();
    }
    if tolerance == 0.0 {
        a.to_bits() == b.to_bits()
    } else {
        a == b || (a - b).abs() <= tolerance
    }
}

fn floats_close(a: &[f64], b: &[f64], tolerance: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| float_close(*x, *y, tolerance))
}

fn values_close(a: &Value, b: &Value, tolerance: f64) -> bool {
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => float_close(*x, *y, tolerance),
        (Value::Vec2(x), Value::Vec2(y)) => floats_close(x, y, tolerance),
        (Value::Vec3(x), Value::Vec3(y)) => floats_close(x, y, tolerance),
        (Value::Vec4(x), Value::Vec4(y)) | (Value::Color(x), Value::Color(y)) => {
            floats_close(x, y, tolerance)
        }
        (Value::Table(x), Value::Table(y)) => {
            x.column_count() == y.column_count()
                && x.columns().iter().zip(y.columns()).all(|(c, d)| {
                    c.name() == d.name()
                        && c.ty() == d.ty()
                        && c.values().len() == d.values().len()
                        && c.values()
                            .iter()
                            .zip(d.values())
                            .all(|(v, w)| values_close(v, w, tolerance))
                })
        }
        _ => a == b,
    }
}

fn show(value: &Value) -> String {
    encode(value).unwrap_or_else(|| format!("{value:?}"))
}

// ---------------------------------------------------------------------------
// Rendering (generator)
// ---------------------------------------------------------------------------

/// The hand-written parts of an existing spec file: `description` and the
/// text from the first `[[tests]]` line on.
#[derive(Debug, Default)]
struct HandWritten {
    description: Option<String>,
    tests: String,
}

fn hand_written(text: &str) -> Result<HandWritten, String> {
    let table: toml::Table = text.parse().map_err(|e| format!("invalid TOML: {e}"))?;
    let description = match table.get("description") {
        None => None,
        Some(toml::Value::String(s)) => Some(s.trim().to_owned()).filter(|s| !s.is_empty()),
        Some(_) => return Err("description must be a string".to_owned()),
    };
    let mut tests = String::new();
    let mut in_tests = false;
    for line in text.lines() {
        if !in_tests && line.trim_start().starts_with("[[tests]]") {
            in_tests = true;
        }
        if in_tests {
            tests.push_str(line);
            tests.push('\n');
        }
    }
    Ok(HandWritten {
        description,
        tests: tests.trim_end().to_owned(),
    })
}

fn multiline(text: &str) -> String {
    if text.contains("'''") || text.chars().any(|c| c.is_control() && c != '\n') {
        quote(text)
    } else {
        format!("'''\n{text}\n'''")
    }
}

/// The type variable typing a port, if any.
fn port_var<'a>(node: &'a dyn NodeImpl, port: &str, input: bool) -> Option<&'a TypeVar> {
    node.type_vars().iter().find(|v| {
        let ports = if input { &v.inputs } else { &v.outputs };
        ports.contains(&port)
    })
}

/// `type = "<Type>"`, or `type = "<Class>"` and `var = "<Var>"` for a
/// polymorphic port.
fn write_port_type(out: &mut String, node: &dyn NodeImpl, port: &str, input: bool, ty: PortType) {
    match port_var(node, port, input) {
        Some(var) => {
            let _ = writeln!(out, "type = {}", quote(var.class.name()));
            let _ = writeln!(out, "var = {}", quote(var.name));
        }
        None => {
            let _ = writeln!(out, "type = {}", quote(ty.name()));
        }
    }
}

/// The canonical text of a node's spec file.
fn render(node: &dyn NodeImpl, hand: &HandWritten) -> String {
    let spec = node.spec();
    let mut out = String::from(FILE_HEADER);
    let _ = writeln!(out);
    let _ = writeln!(out, "id = {}", quote(spec.id));
    let _ = writeln!(out, "version = {}", spec.version);
    let _ = writeln!(out, "summary = {}", quote(spec.summary));
    let _ = writeln!(out, "time_dependent = {}", spec.time_dependent);
    let _ = writeln!(out, "reads_system = {}", node.reads_system());
    if let Some(description) = &hand.description {
        let _ = writeln!(out, "description = {}", multiline(description));
    }
    if spec.inputs.is_empty() {
        let _ = writeln!(out, "inputs = []");
    }
    if spec.outputs.is_empty() {
        let _ = writeln!(out, "outputs = []");
    }
    for var in node.type_vars() {
        let _ = writeln!(out, "\n[[type_vars]]");
        let _ = writeln!(out, "name = {}", quote(var.name));
        let _ = writeln!(out, "class = {}", quote(var.class.name()));
        let _ = writeln!(out, "fallback = {}", quote(var.fallback.name()));
    }
    for port in &spec.inputs {
        let _ = writeln!(out, "\n[[inputs]]");
        let _ = writeln!(out, "name = {}", quote(port.name));
        write_port_type(&mut out, node, port.name, true, port.ty);
        if let Some(default) = port.default.as_ref().filter(|_| !event_placeholder(port)) {
            match encode(default) {
                Some(text) => {
                    let _ = writeln!(out, "default = {text}");
                }
                None => {
                    let _ = writeln!(out, "default_debug = {}", quote(&format!("{default:?}")));
                }
            }
        }
    }
    for port in &spec.outputs {
        let _ = writeln!(out, "\n[[outputs]]");
        let _ = writeln!(out, "name = {}", quote(port.name));
        write_port_type(&mut out, node, port.name, false, port.ty);
    }
    if !hand.tests.is_empty() {
        let _ = writeln!(out, "\n{}", hand.tests);
    }
    out
}

// ---------------------------------------------------------------------------
// Checking
// ---------------------------------------------------------------------------

/// Compare a parsed spec file with the Rust spec. Every mismatch is pushed
/// to `problems`.
fn check_header(node: &dyn NodeImpl, file: &toml::Table, problems: &mut Vec<String>) {
    let spec = node.spec();
    let mut problem = |p: String| problems.push(p);
    if let Err(e) = check_keys(file, &TOP_KEYS, "the spec") {
        problem(e);
    }
    let expect_str = |key: &str, want: &str, problem: &mut dyn FnMut(String)| match file
        .get(key)
        .and_then(toml::Value::as_str)
    {
        Some(got) if got == want => {}
        got => problem(format!("{key} is {got:?}, the Rust spec says {want:?}")),
    };
    expect_str("id", spec.id, &mut problem);
    expect_str("summary", spec.summary, &mut problem);
    match file.get("version").and_then(toml::Value::as_integer) {
        Some(v) if i64::from(spec.version) == v => {}
        got => problem(format!(
            "version is {got:?}, the Rust spec says {}",
            spec.version
        )),
    }
    let flags = [
        ("time_dependent", spec.time_dependent),
        ("reads_system", node.reads_system()),
    ];
    for (key, want) in flags {
        match file.get(key).and_then(toml::Value::as_bool) {
            Some(v) if v == want => {}
            got => problem(format!("{key} is {got:?}, the Rust spec says {want}")),
        }
    }
    if let Some(d) = file.get("description")
        && !d.is_str()
    {
        problem("description must be a string".to_owned());
    }
    check_type_vars(node, file, &mut problem);
    for (key, ports, is_input) in [
        ("inputs", &spec.inputs, true),
        ("outputs", &spec.outputs, false),
    ] {
        let empty = Vec::new();
        let listed = match file.get(key) {
            None => &empty,
            Some(toml::Value::Array(a)) => a,
            Some(_) => {
                problem(format!("{key} must be an array of tables"));
                continue;
            }
        };
        if listed.len() != ports.len() {
            problem(format!(
                "{key}: the file lists {} ports, the Rust spec {}",
                listed.len(),
                ports.len()
            ));
        }
        for (i, (entry, port)) in listed.iter().zip(ports.iter()).enumerate() {
            let what = format!("{key}[{i}]");
            let Some(entry) = entry.as_table() else {
                problem(format!("{what} must be a table"));
                continue;
            };
            if let Err(e) = check_keys(entry, &PORT_KEYS, &what) {
                problem(e);
            }
            match str_field(entry, "name", &what) {
                Ok(name) if name == port.name => {}
                Ok(name) => problem(format!(
                    "{what}.name is {name:?}, the Rust spec says {:?}",
                    port.name
                )),
                Err(e) => problem(e),
            }
            let var = port_var(node, port.name, is_input);
            let listed_var = entry
                .get("var")
                .map(|v| v.as_str().unwrap_or("<not a string>"));
            match (var, listed_var) {
                (None, None) => match str_field(entry, "type", &what).and_then(port_type) {
                    Ok(ty) if ty == port.ty => {}
                    Ok(ty) => problem(format!(
                        "{what} ({}) type is {ty}, the Rust spec says {}",
                        port.name, port.ty
                    )),
                    Err(e) => problem(e),
                },
                (Some(var), Some(listed)) if listed == var.name => {
                    match str_field(entry, "type", &what) {
                        Ok(class) if class == var.class.name() => {}
                        Ok(class) => problem(format!(
                            "{what} ({}) type is {class:?}, the Rust spec says {:?} (class of {})",
                            port.name,
                            var.class.name(),
                            var.name
                        )),
                        Err(e) => problem(e),
                    }
                }
                (var, listed) => problem(format!(
                    "{what} ({}) var is {listed:?}, the Rust spec says {:?}",
                    port.name,
                    var.map(|v| v.name)
                )),
            }
            let default = entry.get("default");
            let debug = entry.get("default_debug");
            // The `Event` placeholder default is not written (see the docs).
            let want_default = port.default.as_ref().filter(|_| !event_placeholder(port));
            match (want_default, is_input) {
                (_, false) | (None, true) => {
                    if default.is_some() || debug.is_some() {
                        problem(format!(
                            "{what} ({}) has a default, the Rust spec has none",
                            port.name
                        ));
                    }
                }
                (Some(want), true) => match (encode(want), default, debug) {
                    (Some(_), Some(text), None) => match decode(port.ty, text) {
                        Ok(got) if values_close(&got, want, 0.0) => {}
                        Ok(got) => problem(format!(
                            "{what} ({}) default is {}, the Rust spec says {}",
                            port.name,
                            show(&got),
                            show(want)
                        )),
                        Err(e) => problem(format!("{what} ({}) default: {e}", port.name)),
                    },
                    (None, None, Some(toml::Value::String(d))) if *d == format!("{want:?}") => {}
                    _ => problem(format!(
                        "{what} ({}) default does not match the Rust spec ({})",
                        port.name,
                        show(want)
                    )),
                },
            }
        }
    }
}

/// Compare the file's `[[type_vars]]` with the node's type variables.
fn check_type_vars(node: &dyn NodeImpl, file: &toml::Table, problem: &mut dyn FnMut(String)) {
    let empty = Vec::new();
    let listed = match file.get("type_vars") {
        None => &empty,
        Some(toml::Value::Array(a)) => a,
        Some(_) => return problem("type_vars must be an array of tables".to_owned()),
    };
    let vars = node.type_vars();
    if listed.len() != vars.len() {
        problem(format!(
            "type_vars: the file lists {} type variables, the Rust spec {}",
            listed.len(),
            vars.len()
        ));
    }
    for (i, (entry, var)) in listed.iter().zip(vars).enumerate() {
        let what = format!("type_vars[{i}]");
        let Some(entry) = entry.as_table() else {
            problem(format!("{what} must be a table"));
            continue;
        };
        if let Err(e) = check_keys(entry, &VAR_KEYS, &what) {
            problem(e);
        }
        let want = [
            ("name", var.name),
            ("class", var.class.name()),
            ("fallback", var.fallback.name()),
        ];
        for (key, want) in want {
            match str_field(entry, key, &what) {
                Ok(got) if got == want => {}
                Ok(got) => problem(format!(
                    "{what}.{key} is {got:?}, the Rust spec says {want:?}"
                )),
                Err(e) => problem(e),
            }
        }
        if TypeClass::from_name(var.class.name()) != Some(var.class) {
            problem(format!("{what}: unknown class {}", var.class));
        }
    }
}

/// The instance of `node` a test case runs: `types = { T = "Vec3" }`
/// resolves its type variables (others keep their fallback).
fn test_instance(
    node: &Arc<dyn NodeImpl>,
    types: Option<&toml::Value>,
) -> Result<Arc<dyn NodeImpl>, String> {
    let Some(types) = types else {
        return Ok(Arc::clone(node));
    };
    let given = table_of(types, "types")?;
    let vars = node.type_vars();
    for name in given.keys() {
        if !vars.iter().any(|v| v.name == name) {
            return Err(format!("types: the node has no type variable {name:?}"));
        }
    }
    let resolved = vars
        .iter()
        .map(|v| match given.get(v.name) {
            Some(t) => port_type(t.as_str().ok_or("types values must be type names")?),
            None => Ok(v.fallback),
        })
        .collect::<Result<Vec<_>, String>>()?;
    instantiate(node, &resolved).map_err(|e| format!("types: {e}"))
}

fn host_fs(fs: &toml::Value) -> Result<MemFs, String> {
    let t = table_of(fs, "fs")?;
    check_keys(t, &["files", "dirs"], "fs")?;
    let mut host = MemFs::new();
    let entries = |key: &str| -> Result<Vec<toml::Table>, String> {
        match t.get(key) {
            None => Ok(Vec::new()),
            Some(toml::Value::Array(a)) => a
                .iter()
                .map(|e| table_of(e, key).cloned())
                .collect::<Result<Vec<_>, _>>(),
            Some(_) => Err(format!("fs.{key} must be an array of tables")),
        }
    };
    for dir in entries("dirs")? {
        check_keys(&dir, &["path", "modified"], "fs.dirs[]")?;
        let path = str_field(&dir, "path", "fs.dirs[]")?;
        let modified = dir.get("modified").and_then(toml::Value::as_integer);
        host = host
            .with_dir(path, modified.unwrap_or(0))
            .map_err(|e| e.to_string())?;
    }
    for file in entries("files")? {
        check_keys(&file, &["path", "size", "modified"], "fs.files[]")?;
        let path = str_field(&file, "path", "fs.files[]")?;
        let size = file
            .get("size")
            .and_then(toml::Value::as_integer)
            .unwrap_or(0);
        let size = u64::try_from(size).map_err(|_| format!("{path}: negative size"))?;
        let modified = file.get("modified").and_then(toml::Value::as_integer);
        host = host
            .with_file(path, size, modified.unwrap_or(0))
            .map_err(|e| e.to_string())?;
    }
    Ok(host)
}

fn context(v: Option<&toml::Value>) -> Result<CookContext, String> {
    let mut ctx = CookContext::default();
    let Some(v) = v else { return Ok(ctx) };
    let t = table_of(v, "context")?;
    check_keys(t, &["frame", "time", "dt"], "context")?;
    if let Some(frame) = t.get("frame") {
        let frame = frame
            .as_integer()
            .ok_or("context.frame must be an integer")?;
        ctx.frame = u64::try_from(frame).map_err(|_| "context.frame must not be negative")?;
    }
    if let Some(time) = t.get("time") {
        ctx.time = number(time)?;
    }
    if let Some(dt) = t.get("dt") {
        ctx.dt = number(dt)?;
    }
    Ok(ctx)
}

/// Run one `[[tests]]` case. Returns the reason it failed.
fn run_case(node: &Arc<dyn NodeImpl>, case: &toml::Value) -> Result<(), String> {
    let case = table_of(case, "a [[tests]] entry")?;
    check_keys(case, &TEST_KEYS, "[[tests]]")?;
    let instance = test_instance(node, case.get("types"))?;
    let node = instance.as_ref();
    let spec = node.spec();
    if let Some(name) = case.get("name") {
        name.as_str().ok_or("name must be a string")?;
    }
    let mut inputs = Vec::with_capacity(spec.inputs.len());
    for port in &spec.inputs {
        inputs.push(port.default.clone().or_else(|| Value::zero(port.ty)));
    }
    if let Some(given) = case.get("inputs") {
        for (name, v) in table_of(given, "inputs")? {
            let index = spec
                .input_index(name)
                .ok_or_else(|| format!("no input port {name:?}"))?;
            let ty = spec.inputs[index].ty;
            inputs[index] = Some(decode(ty, v).map_err(|e| format!("input {name}: {e}"))?);
        }
    }
    let inputs = spec
        .inputs
        .iter()
        .zip(inputs)
        .map(|(port, v)| v.ok_or_else(|| format!("input {} has no default; set it", port.name)))
        .collect::<Result<Vec<_>, _>>()?;
    let ctx = context(case.get("context"))?;
    let host = case.get("fs").map(host_fs).transpose()?;
    let system = match &host {
        Some(host) => SystemContext::new(host),
        None => SystemContext::none(),
    };
    let tolerance = case
        .get("tolerance")
        .map(number)
        .transpose()?
        .unwrap_or(0.0);
    // Event steps (optional), then the value cook from the reached state.
    let (result, emitted) = match run_events(node, case.get("events"), &inputs, &ctx) {
        Ok((state, emitted)) => (
            node.cook_with_state(&ctx, &system, state.as_ref(), &inputs),
            emitted,
        ),
        Err(StepFailure::Node(err)) => (Err(err), Vec::new()),
        Err(StepFailure::Spec(e)) => return Err(e),
    };
    if let Some(want) = case.get("expect_events") {
        check_events(want, &emitted, tolerance)?;
    }
    match (case.get("expect"), case.get("expect_error"), result) {
        (Some(_), Some(_), _) => Err("give expect or expect_error, not both".to_owned()),
        (None, None, _) if case.contains_key("expect_events") => Ok(()),
        (None, None, _) => Err("a test needs expect, expect_events or expect_error".to_owned()),
        (None, Some(want), Ok(outputs)) => {
            let shown: Vec<String> = outputs.iter().map(show).collect();
            let _ = want;
            Err(format!(
                "expected an error, got outputs [{}]",
                shown.join(", ")
            ))
        }
        (None, Some(want), Err(err)) => {
            let want = table_of(want, "expect_error")?;
            check_keys(want, &["port", "message"], "expect_error")?;
            if let Some(port) = want.get("port") {
                let port = port.as_str().ok_or("expect_error.port must be a string")?;
                match &err {
                    NodeError::InvalidInput { port: got, .. } if *got == port => {}
                    _ => return Err(format!("expected an error on input {port:?}, got: {err}")),
                }
            }
            if let Some(message) = want.get("message") {
                let message = message
                    .as_str()
                    .ok_or("expect_error.message must be a string")?;
                if !err.to_string().contains(message) {
                    return Err(format!(
                        "expected an error containing {message:?}, got: {err}"
                    ));
                }
            }
            Ok(())
        }
        (Some(_), None, Err(err)) => Err(format!("cook failed: {err}")),
        (Some(want), None, Ok(outputs)) => {
            if outputs.len() != spec.outputs.len() {
                return Err(format!(
                    "cook returned {} outputs, the spec declares {}",
                    outputs.len(),
                    spec.outputs.len()
                ));
            }
            for (name, v) in table_of(want, "expect")? {
                let index = spec
                    .output_index(name)
                    .ok_or_else(|| format!("no output port {name:?}"))?;
                let expected =
                    decode(spec.outputs[index].ty, v).map_err(|e| format!("expect.{name}: {e}"))?;
                if !values_close(&outputs[index], &expected, tolerance) {
                    return Err(format!(
                        "output {name}: expected {}, got {}",
                        show(&expected),
                        show(&outputs[index])
                    ));
                }
            }
            Ok(())
        }
    }
}

/// Why event steps stopped: the node failed (checked against
/// `expect_error`), or the spec file is malformed.
/// Emitted events as `(output name, payload)`.
type Emitted = Vec<(String, Option<Value>)>;

enum StepFailure {
    Node(NodeError),
    Spec(String),
}

/// An event payload written in a spec: its TOML type picks the value type.
fn untyped(v: &toml::Value) -> Result<Value, String> {
    Ok(match v {
        toml::Value::Integer(i) => Value::Int(*i),
        toml::Value::Float(f) => Value::Float(*f),
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::String(s) => Value::String(s.clone()),
        other => {
            return Err(format!(
                "a payload must be a scalar, got a {}",
                type_name(other)
            ));
        }
    })
}

/// Run the `events` steps from the node's initial state. Returns the final
/// state and the emitted events as `(output name, payload)`.
fn run_events(
    node: &dyn NodeImpl,
    steps: Option<&toml::Value>,
    inputs: &[Value],
    ctx: &CookContext,
) -> Result<(Option<Value>, Emitted), StepFailure> {
    let spec = node.spec();
    let mut state = node.initial_state();
    let mut emitted = Vec::new();
    let Some(steps) = steps else {
        return Ok((state, emitted));
    };
    let steps = steps
        .as_array()
        .ok_or_else(|| StepFailure::Spec("events must be an array".to_owned()))?;
    for (i, step) in steps.iter().enumerate() {
        let what = format!("events[{i}]");
        let step = table_of(step, &what).map_err(StepFailure::Spec)?;
        check_keys(step, &STEP_KEYS, &what).map_err(StepFailure::Spec)?;
        let mut step_inputs = inputs.to_vec();
        if let Some(given) = step.get("inputs") {
            for (name, v) in table_of(given, "inputs").map_err(StepFailure::Spec)? {
                let index = spec
                    .input_index(name)
                    .ok_or_else(|| StepFailure::Spec(format!("{what}: no input port {name:?}")))?;
                step_inputs[index] = decode(spec.inputs[index].ty, v)
                    .map_err(|e| StepFailure::Spec(format!("{what}.inputs.{name}: {e}")))?;
            }
        }
        let step_ctx = match step.get("context") {
            Some(c) => context(Some(c)).map_err(StepFailure::Spec)?,
            None => *ctx,
        };
        let event = Event {
            payload: step
                .get("payload")
                .map(untyped)
                .transpose()
                .map_err(StepFailure::Spec)?,
        };
        let trigger = match (step.get("input"), step.get("tick")) {
            (Some(port), None) => {
                let name = port
                    .as_str()
                    .ok_or_else(|| StepFailure::Spec(format!("{what}.input must be a string")))?;
                let port = spec
                    .input_index(name)
                    .filter(|&i| spec.inputs[i].ty == PortType::Event)
                    .ok_or_else(|| StepFailure::Spec(format!("{what}: no Event input {name:?}")))?;
                EventTrigger::Input {
                    port,
                    event: &event,
                }
            }
            (None, Some(toml::Value::Boolean(true))) => EventTrigger::Tick,
            _ => {
                return Err(StepFailure::Spec(format!(
                    "{what} needs input = \"<Event input>\" or tick = true"
                )));
            }
        };
        let mut call = EventCall::new(&step_ctx, trigger, &step_inputs, state.clone());
        node.on_event(&mut call).map_err(StepFailure::Node)?;
        let (next, out) = call.finish();
        state = next;
        for (port, event) in out {
            let name = spec
                .outputs
                .get(port)
                .filter(|p| p.ty == PortType::Event)
                .ok_or_else(|| {
                    StepFailure::Spec(format!("{what}: emitted on non-Event output {port}"))
                })?
                .name;
            emitted.push((name.to_owned(), event.payload));
        }
    }
    Ok((state, emitted))
}

/// Compare emitted events with `expect_events` (all of them, in order).
fn check_events(
    want: &toml::Value,
    got: &[(String, Option<Value>)],
    tolerance: f64,
) -> Result<(), String> {
    let shown = || {
        got.iter()
            .map(|(port, p)| match p {
                Some(p) => format!("{port}({})", show(p)),
                None => port.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let want = want.as_array().ok_or("expect_events must be an array")?;
    if want.len() != got.len() {
        return Err(format!(
            "expected {} events, got {}: [{}]",
            want.len(),
            got.len(),
            shown()
        ));
    }
    for (i, (w, (port, payload))) in want.iter().zip(got).enumerate() {
        let what = format!("expect_events[{i}]");
        let w = table_of(w, &what)?;
        check_keys(w, &["output", "payload"], &what)?;
        let output = str_field(w, "output", &what)?;
        let expected = w.get("payload").map(untyped).transpose()?;
        let same = match (&expected, payload) {
            (None, None) => true,
            (Some(a), Some(b)) => values_close(a, b, tolerance),
            _ => false,
        };
        if output != port || !same {
            return Err(format!("{what} does not match; got [{}]", shown()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// docs/nodes.md
// ---------------------------------------------------------------------------

const DOCS_PREAMBLE: &str = r#"<!-- Generated from crates/livspeil-nodes-core/specs/*.node.toml by
     `cargo xtask node-specs` (crates/livspeil-nodes-core/tests/node_specs.rs).
     Do not edit by hand. -->

# Core nodes

Every built-in node is described by a spec file
`crates/livspeil-nodes-core/specs/<id>.node.toml` (kernel spec, section 9;
`<id>@<version>.node.toml` for versions other than 1). The spec file is
the node's documentation and its table of test cases; the Rust `cook` body
is the only other hand-written part. How to add a node:
[CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-node).

## Spec file format

```toml
id = "core.math.lerp"           # generated from the Rust NodeSpec
version = 1                     # generated
summary = "Linear interpolation between a and b"   # generated
time_dependent = false          # generated: re-cooked every frame
reads_system = false            # generated: reads the host system (files)
description = '''
Optional, hand-written, multi-line documentation (Markdown).
'''

[[inputs]]                      # generated, one per input port, in order
name = "a"
type = "Float"
default = 0.0

[[outputs]]                     # generated, one per output port, in order
name = "out"
type = "Float"

[[tests]]                       # hand-written, any number
name = "spec example"           # optional
inputs = { b = 10.0, t = 0.25 } # inputs left out take their default
expect = { out = 2.5 }          # outputs left out are not checked

[[tests]]
inputs = { t = 0.1 }
expect = { out = 0.1 }
tolerance = 1e-12               # optional absolute tolerance for floats

[[tests]]                       # (as in core.math.div, which can fail)
inputs = { b = 0.0 }
expect_error = { port = "b", message = "division by zero" }
```

Everything above the first `[[tests]]` except `description` is generated
from the Rust `NodeSpec` by `cargo xtask node-specs` and must match it
exactly; the generator keeps `description` and the `[[tests]]` (copied
verbatim from the first `[[tests]]` line to the end of the file). Other
text above `[[tests]]`, such as comments, is not kept.

`[[tests]]` keys:

| Key | Meaning |
|---|---|
| `name` | Optional label shown when the case fails. |
| `note` | Optional free text. |
| `inputs` | Port name to value; other inputs take their defaults. |
| `context` | Optional `{ frame = 12, time = 0.2, dt = 0.016 }` cook context (default all zero). |
| `fs` | Optional in-memory file system for nodes that read the system: `{ dirs = [{ path = "a", modified = 0 }], files = [{ path = "a/b.txt", size = 10, modified = 3 }] }`. Without it the node gets no system access. |
| `expect` | Output port name to expected value. Floats compare exactly (`-0.0` differs from `0.0`; `nan` matches `nan`) unless `tolerance` is set. |
| `tolerance` | Optional absolute tolerance for every float compared (also inside vectors, colors and tables). |
| `events` | Optional event steps for event nodes, run in order from the node's initial state, the state carried from step to step: `{ input = "trigger", payload = 2.0 }` delivers an event (payload optional) to an `Event` input, `{ tick = true }` is the once-per-tick poll. A step may override `inputs` and `context` for itself. `expect` then checks the outputs cooked from the final state (`cook_with_state`); without `events` that is the initial state. |
| `expect_events` | The events emitted by all steps, in order: `[{ output = "fired", payload = 1 }, { output = "out" }]` (payload left out: none). A payload's TOML type picks its value type (integer `Int`, float `Float`, boolean `Bool`, string `String`). May replace `expect`. |
| `expect_error` | The cook (or an event step) must fail: `port` (an invalid-input error on that port) and/or `message` (a substring of the error text); `{}` accepts any error. |
| `types` | Polymorphic nodes only: the resolved type of each type variable for this case, `{ T = "Vec3" }` (variables left out take their fallback). Inputs, outputs and defaults then have the resolved types. |

### Polymorphic ports

A polymorphic node (M0.17) declares type variables, each with a
constraint class and a fallback, as generated `[[type_vars]]` entries;
its ports of a variable list the class as `type` and the variable as
`var`, and their `default` is the fallback's default:

```toml
[[type_vars]]
name = "T"
class = "Interpolable"   # Numeric, Vector, Interpolable or Any
fallback = "Float"       # the type of an unresolved instance

[[inputs]]
name = "a"
type = "Interpolable"
var = "T"
default = 0.0
```

Classes: `Numeric` = `Float`, `Int`, `Vec2`, `Vec3`, `Vec4`; `Vector` =
`Vec2`, `Vec3`, `Vec4`; `Interpolable` = `Float`, `Vec2`, `Vec3`,
`Vec4`, `Color`; `Any` = every value type except `Event`. A graph
resolves the variables of each instance from its links, values,
expressions and animation channels or a pinned type (documents:
`port-type=`); see `livspeil_graph`'s `types` module. When a variable
resolves to another type, defaults follow it: a number `x` becomes `Int`
when whole, `[x, x, ...]` for vectors and the grey `oklch(x 0 0 1)` for
colors; other defaults become the type's zero.

### Values

Values are written by port type:

| Type | TOML | Example |
|---|---|---|
| `Float` | float (an integer is accepted); `nan`, `inf`, `-inf` | `0.5`, `-0.0`, `1e-7` |
| `Int` | integer | `-42` |
| `Bool` | boolean | `true` |
| `String` | string | `"text"` |
| `Vec2`, `Vec3`, `Vec4` | array of 2, 3 or 4 numbers | `[1.0, 2.0, 3.0]` |
| `Color` | OKLCH plus alpha, `[l, c, h, alpha]` (alpha may be left out: 1.0) | `[0.7, 0.1, 250.0, 1.0]` |
| `Table` | `{ columns = [{ name, type, values = [...] }, ...] }` | `{ columns = [{ name = "name", type = "String", values = ["a", "b"] }] }` |
| `Curve` | `{ type?, keys = [{ position, value, interpolation?, tangent? }, ...], pre?, post? }` | `{ keys = [{ position = 0.0, value = 0.0 }, { position = 1.0, value = 1.0 }] }` |

Curve keys: `interpolation` is `"linear"` (default), `"step"`, `"bezier"`
or `{ spring = { frequency = 2.0, damping = 0.3 } }`; `tangent` is
`"auto"` (default), `"flat"` or `{ incoming = 0.0, outgoing = 4.0 }`;
`pre` and `post` are `"hold"` (default), `"loop"` or `"ping_pong"`.
Typed curves (M0.17) add `type = "Vec2"` (or `"Vec3"`, `"Vec4"`,
`"Color"`; default `"Float"`), and their key values and explicit tangent
slopes are written as values of that type, for example
`{ type = "Color", keys = [{ position = 0.0, value = [0.5, 0.1, 30.0] }] }`
(color tangents are OKLab-plus-alpha slopes).

Large values (tables) read better as TOML sub-tables, for example
`[[tests.inputs.table.columns]]` with `name`, `type` and `values` keys.
A default of a type the format cannot write yet is recorded as
`default_debug` (its Rust `Debug` text) instead of `default`. `Event`
inputs list no default: in value cooking they carry the placeholder
`Value::Event`, and events are tested with `events` and `expect_events`.
"#;

fn md_cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

fn port_list(node: &dyn NodeImpl, inputs: bool) -> String {
    let spec = node.spec();
    let ports = if inputs { &spec.inputs } else { &spec.outputs };
    if ports.is_empty() {
        return "none".to_owned();
    }
    ports
        .iter()
        .map(|p| {
            let default = match (&p.default, inputs) {
                (Some(d), true) if !event_placeholder(p) => format!(" = {}", show(d)),
                _ => String::new(),
            };
            let ty = port_var(node, p.name, inputs)
                .map_or_else(|| p.ty.to_string(), |v| v.name.to_owned());
            md_cell(&format!("`{}: {}{}`", p.name, ty, default))
        })
        .collect::<Vec<_>>()
        .join("<br>")
}

/// `<T: Any = Float>` for a polymorphic node, empty otherwise.
fn type_params(node: &dyn NodeImpl) -> String {
    let vars: Vec<String> = node
        .type_vars()
        .iter()
        .map(|v| format!("{}: {} = {}", v.name, v.class, v.fallback))
        .collect();
    if vars.is_empty() {
        String::new()
    } else {
        format!("<{}>", vars.join(", "))
    }
}

fn render_docs(nodes: &[(Arc<dyn NodeImpl>, Option<String>, usize)]) -> String {
    let mut out = String::from(DOCS_PREAMBLE);
    let _ = writeln!(out, "\n## Node index\n");
    let _ = writeln!(
        out,
        "{} nodes. *time* marks nodes re-cooked every frame, *system* nodes that read the host system.\n",
        nodes.len()
    );
    let _ = writeln!(out, "| Node | Summary | Inputs | Outputs | Tests |");
    let _ = writeln!(out, "|---|---|---|---|---|");
    for (node, _, tests) in nodes {
        let spec = node.spec();
        let mut summary = md_cell(spec.summary);
        let params = type_params(node.as_ref());
        if !params.is_empty() {
            summary.push_str(&format!(" `{params}`"));
        }
        if spec.time_dependent {
            summary.push_str(" *(time)*");
        }
        if node.reads_system() {
            summary.push_str(" *(system)*");
        }
        let _ = writeln!(
            out,
            "| [`{}@{}`](#{}) | {} | {} | {} | {} |",
            spec.id,
            spec.version,
            anchor(spec),
            summary,
            port_list(node.as_ref(), true),
            port_list(node.as_ref(), false),
            tests
        );
    }
    let _ = writeln!(out, "\n## Nodes");
    for (node, description, _) in nodes {
        let spec = node.spec();
        let _ = writeln!(out, "\n### {}@{}\n", spec.id, spec.version);
        let _ = writeln!(out, "{}", spec.summary);
        let params = type_params(node.as_ref());
        if !params.is_empty() {
            let _ = writeln!(out, "\nType variables: `{params}`.");
        }
        if let Some(d) = description {
            let _ = writeln!(out, "\n{d}");
        }
        let _ = writeln!(
            out,
            "\nSpec: [`{}`](../crates/livspeil-nodes-core/specs/{})",
            file_name(spec),
            file_name(spec)
        );
    }
    out
}

fn anchor(spec: &NodeSpec) -> String {
    format!("{}{}", spec.id, spec.version).replace(['.', '@'], "")
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

fn spec_files() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    if let Ok(entries) = std::fs::read_dir(specs_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".node.toml") {
                out.insert(name);
            }
        }
    }
    out
}

fn write_if_changed(path: &Path, text: &str) {
    if std::fs::read_to_string(path).ok().as_deref() != Some(text) {
        std::fs::write(path, text).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        eprintln!("wrote {}", path.display());
    }
}

#[test]
fn node_specs_match_the_registry_and_their_tests_pass() {
    let update = std::env::var_os(UPDATE_ENV).is_some();
    let registry = core_registry();
    let nodes: BTreeMap<String, Arc<dyn NodeImpl>> = registry
        .ids()
        .filter_map(|id| {
            registry
                .get(id)
                .map(|n| (file_name(n.spec()), Arc::clone(n)))
        })
        .collect();
    let dir = specs_dir();
    let mut problems: Vec<String> = Vec::new();

    if update {
        std::fs::create_dir_all(&dir).unwrap();
        for (file, node) in &nodes {
            let path = dir.join(file);
            let hand = match std::fs::read_to_string(&path) {
                Ok(text) => match hand_written(&text) {
                    Ok(hand) => hand,
                    Err(e) => {
                        problems.push(format!("{file}: {e}; not regenerated"));
                        continue;
                    }
                },
                Err(_) => HandWritten::default(),
            };
            write_if_changed(&path, &render(node.as_ref(), &hand));
        }
    }

    let files = spec_files();
    let missing: Vec<&String> = nodes.keys().filter(|f| !files.contains(*f)).collect();
    if !missing.is_empty() {
        problems.push(format!(
            "registered nodes without a spec file: {}\n  create them: {HOW_TO_UPDATE}, then add description and [[tests]]",
            missing
                .iter()
                .map(|f| format!("specs/{f}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for stale in files.iter().filter(|f| !nodes.contains_key(*f)) {
        problems.push(format!(
            "specs/{stale} has no registered node (renamed or removed?); delete it or register the node"
        ));
    }

    let mut docs_nodes = Vec::new();
    let mut case_count = 0;
    for (file, node) in &nodes {
        let Ok(text) = std::fs::read_to_string(dir.join(file)) else {
            continue;
        };
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                problems.push(format!("specs/{file}: invalid TOML: {e}"));
                continue;
            }
        };
        let mut file_problems = Vec::new();
        check_header(node.as_ref(), &table, &mut file_problems);
        if file_problems.is_empty() {
            if let Ok(hand) = hand_written(&text)
                && render(node.as_ref(), &hand) != text
            {
                file_problems.push(format!("not in canonical form; {HOW_TO_UPDATE}"));
            }
        } else {
            file_problems.push(format!(
                "out of sync with the Rust NodeSpec; {HOW_TO_UPDATE}"
            ));
        }
        let cases = match table.get("tests") {
            None => Vec::new(),
            Some(toml::Value::Array(a)) => a.clone(),
            Some(_) => {
                file_problems.push("tests must be an array of tables ([[tests]])".to_owned());
                Vec::new()
            }
        };
        for (i, case) in cases.iter().enumerate() {
            let label = case
                .get("name")
                .and_then(toml::Value::as_str)
                .map(|n| format!("tests[{i}] ({n})"))
                .unwrap_or_else(|| format!("tests[{i}]"));
            if let Err(e) = run_case(node, case) {
                file_problems.push(format!("{label}: {e}"));
            }
        }
        case_count += cases.len();
        problems.extend(
            file_problems
                .into_iter()
                .map(|p| format!("specs/{file}: {p}")),
        );
        let description = table
            .get("description")
            .and_then(toml::Value::as_str)
            .map(|d| d.trim().to_owned())
            .filter(|d| !d.is_empty());
        docs_nodes.push((Arc::clone(node), description, cases.len()));
    }

    if missing.is_empty() {
        let docs = render_docs(&docs_nodes);
        if update {
            write_if_changed(&docs_path(), &docs);
        } else if std::fs::read_to_string(docs_path()).ok().as_deref() != Some(docs.as_str()) {
            problems.push(format!("docs/nodes.md is out of date; {HOW_TO_UPDATE}"));
        }
    }

    assert!(
        problems.is_empty(),
        "{} node spec problem(s) ({case_count} [[tests]] cases run):\n- {}",
        problems.len(),
        problems.join("\n- ")
    );
}

#[test]
fn every_value_type_round_trips_through_toml() {
    let table = Table::from_rows(
        &[
            ("name", PortType::String),
            ("size", PortType::Int),
            ("pos", PortType::Vec2),
        ],
        [
            vec![
                Value::String("a \"b\"\n\u{1}".into()),
                Value::Int(-3),
                Value::Vec2([0.5, -0.0]),
            ],
            vec![
                Value::String(String::new()),
                Value::Int(7),
                Value::Vec2([f64::NAN, f64::INFINITY]),
            ],
        ],
    )
    .unwrap();
    let curve = Curve::new(vec![
        CurveKey::new(0.0, 1.0).with_interpolation(CurveInterpolation::Bezier),
        CurveKey::new(0.5, 2.0)
            .with_interpolation(CurveInterpolation::Spring {
                frequency: 2.0,
                damping: 0.3,
            })
            .with_tangent(CurveTangent::Explicit {
                incoming: 1.0,
                outgoing: -1e-7,
            }),
        CurveKey::new(1.0, 2.0)
            .with_interpolation(CurveInterpolation::Step)
            .with_tangent(CurveTangent::Flat),
    ])
    .unwrap()
    .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::PingPong);
    let values = [
        Value::Float(-0.0),
        Value::Float(1e300),
        Value::Float(f64::NEG_INFINITY),
        Value::Int(i64::MIN),
        Value::Bool(true),
        Value::Vec3([1.0, 2.0, 3.0]),
        Value::Vec4([1.0, 2.0, 3.0, 4.0]),
        Value::Color([0.7, 0.1, 250.0, 1.0]),
        Value::String("tab\tquote\" é".into()),
        Value::Table(table),
        Value::Table(Table::empty()),
        Value::Curve(Arc::new(curve)),
        Value::Curve(Arc::new(Curve::identity())),
        Value::Curve(Arc::new(
            Curve::typed(vec![
                TypedCurveKey::new(0.0, Value::Color([0.5, 0.1, 30.0, 1.0]))
                    .with_interpolation(CurveInterpolation::Bezier)
                    .with_tangent(TypedCurveTangent::Explicit {
                        incoming: Value::Color([0.0, 0.1, -0.1, 0.0]),
                        outgoing: Value::Color([1.0, 0.0, 0.0, -1e-7]),
                    }),
                TypedCurveKey::new(1.0, Value::Color([0.9, 0.0, 0.0, 0.5]))
                    .with_tangent(TypedCurveTangent::Flat),
            ])
            .unwrap()
            .with_extrapolation(CurveExtrapolation::Loop, CurveExtrapolation::Hold),
        )),
        Value::Curve(Arc::new(
            Curve::typed(vec![TypedCurveKey::new(2.0, Value::Vec2([1.0, -2.0]))]).unwrap(),
        )),
    ];
    for value in values {
        let text = encode(&value).unwrap();
        let doc: toml::Table = format!("v = {text}").parse().unwrap();
        let back = decode(value.port_type(), &doc["v"]).unwrap();
        assert!(values_close(&back, &value, 0.0), "{text}");
    }
    // Integers are accepted for floats, and alpha may be left out.
    let doc: toml::Table = "f = 2\nc = [0.5, 0.1, 30]".parse().unwrap();
    assert_eq!(decode(PortType::Float, &doc["f"]), Ok(Value::Float(2.0)));
    assert_eq!(
        decode(PortType::Color, &doc["c"]),
        Ok(Value::Color([0.5, 0.1, 30.0, 1.0]))
    );
    assert!(decode(PortType::Int, &doc["c"]).is_err());
}
