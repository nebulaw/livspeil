// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Decoding and validating a package's `describe` buffer.

use std::collections::HashSet;

use livspeil_node_api::{PortType, Value};

use crate::abi::{self, DecodeError, Reader};

/// Most node types in one package.
pub const MAX_NODES: usize = 256;
/// Most input (or output) ports of one node type.
pub const MAX_PORTS: usize = 64;
/// Longest node id or port name, in bytes.
pub const MAX_NAME: usize = 128;
/// Longest summary, in bytes.
pub const MAX_SUMMARY: usize = 1024;
/// Longest string default, in bytes.
pub const MAX_DEFAULT_STRING: usize = 4096;

/// A port as described by the guest.
#[derive(Debug, Clone, PartialEq)]
pub struct DescribedPort {
    pub name: String,
    pub ty: PortType,
    pub default: Option<Value>,
}

/// A node type as described by the guest.
#[derive(Debug, Clone, PartialEq)]
pub struct DescribedNode {
    pub id: String,
    pub version: u32,
    pub summary: String,
    pub time_dependent: bool,
    pub inputs: Vec<DescribedPort>,
    pub outputs: Vec<DescribedPort>,
}

impl DescribedNode {
    fn qualified_id(&self) -> String {
        format!("{}@{}", self.id, self.version)
    }
}

/// Why a description was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeError {
    /// The buffer does not follow the ABI encoding.
    Decode(String),
    /// A node type breaks a Livspeil rule.
    Invalid { node: String, reason: String },
}

impl From<DecodeError> for DescribeError {
    fn from(err: DecodeError) -> Self {
        DescribeError::Decode(err.0)
    }
}

/// Decode and validate a `describe` buffer.
///
/// # Errors
/// Returns a [`DescribeError`] for a malformed buffer or an invalid node.
pub fn describe(bytes: &[u8]) -> Result<Vec<DescribedNode>, DescribeError> {
    let mut r = Reader::new(bytes);
    let mut magic = [0u8; 4];
    for byte in &mut magic {
        *byte = r.u8("magic")?;
    }
    if magic != abi::MAGIC {
        return Err(DescribeError::Decode(format!(
            "bad magic {magic:?} (expected \"LSND\")"
        )));
    }
    let version = r.u32("ABI version")?;
    if version != abi::ABI_VERSION {
        return Err(DescribeError::Decode(format!(
            "ABI version {version} is not supported (this host speaks {})",
            abi::ABI_VERSION
        )));
    }
    let count = r.count("node list", MAX_NODES)?;
    let mut nodes = Vec::with_capacity(count);
    for _ in 0..count {
        nodes.push(node(&mut r)?);
    }
    r.finish("describe buffer")?;
    if nodes.is_empty() {
        return Err(DescribeError::Decode(
            "the package describes no node types".to_owned(),
        ));
    }
    let mut seen = HashSet::new();
    for node in &nodes {
        let invalid = |reason: String| DescribeError::Invalid {
            node: node.qualified_id(),
            reason,
        };
        validate(node).map_err(invalid)?;
        if !seen.insert(node.qualified_id()) {
            return Err(invalid("described twice".to_owned()));
        }
    }
    Ok(nodes)
}

fn node(r: &mut Reader<'_>) -> Result<DescribedNode, DescribeError> {
    let id = r.string("node id", MAX_NAME)?;
    let version = r.u32("node version")?;
    let summary = r.string("node summary", MAX_SUMMARY)?;
    let time_dependent = r.bool("time_dependent")?;
    let inputs = ports(r, "input")?;
    let outputs = ports(r, "output")?;
    Ok(DescribedNode {
        id,
        version,
        summary,
        time_dependent,
        inputs,
        outputs,
    })
}

fn ports(r: &mut Reader<'_>, kind: &str) -> Result<Vec<DescribedPort>, DescribeError> {
    let count = r.count(&format!("{kind} list"), MAX_PORTS)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let name = r.string(&format!("{kind} name"), MAX_NAME)?;
        let tag = r.u8(&format!("{kind} type"))?;
        let ty = abi::port_type(tag).ok_or_else(|| {
            DescribeError::Decode(format!(
                "{kind} '{name}': unknown port type tag {tag} (0..=7: Float, Int, Bool, Vec2, Vec3, Vec4, Color, String)"
            ))
        })?;
        let default = match r.u8(&format!("{kind} default"))? {
            0 => None,
            1 => Some(r.value(&format!("{kind} default"), MAX_DEFAULT_STRING)?),
            other => {
                return Err(DescribeError::Decode(format!(
                    "{kind} '{name}': invalid option tag {other}"
                )));
            }
        };
        out.push(DescribedPort { name, ty, default });
    }
    Ok(out)
}

/// Whether `s` is a lowercase identifier: `[a-z][a-z0-9_]*`.
fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// The Livspeil rules for a third-party node type.
fn validate(node: &DescribedNode) -> Result<(), String> {
    let segments: Vec<&str> = node.id.split('.').collect();
    if segments.len() < 2 || !segments.iter().all(|s| is_ident(s)) {
        return Err(format!(
            "id '{}' must be namespaced: two or more dot-separated lowercase segments, e.g. 'acme.wave.sine'",
            node.id
        ));
    }
    if segments[0] == "core" {
        return Err("the 'core' namespace is reserved for built-in nodes".to_owned());
    }
    if node.version == 0 {
        return Err("version must be at least 1".to_owned());
    }
    if node.summary.trim().is_empty() {
        return Err("summary is empty".to_owned());
    }
    if node.outputs.is_empty() {
        return Err("a node needs at least one output".to_owned());
    }
    for (kind, ports) in [("input", &node.inputs), ("output", &node.outputs)] {
        let mut names = HashSet::new();
        for port in ports {
            if !is_ident(&port.name) {
                return Err(format!(
                    "{kind} name '{}' must match [a-z][a-z0-9_]*",
                    port.name
                ));
            }
            if !names.insert(port.name.as_str()) {
                return Err(format!("{kind} '{}' is declared twice", port.name));
            }
            let Some(default) = &port.default else {
                continue;
            };
            if kind == "output" {
                return Err(format!("output '{}' has a default", port.name));
            }
            if default.port_type() != port.ty {
                return Err(format!(
                    "input '{}' is {} but its default is {}",
                    port.name,
                    port.ty,
                    default.port_type()
                ));
            }
            if !floats(default).iter().all(|x| x.is_finite()) {
                return Err(format!("input '{}' has a non-finite default", port.name));
            }
        }
    }
    Ok(())
}

fn floats(value: &Value) -> Vec<f64> {
    match value {
        Value::Float(x) => vec![*x],
        Value::Vec2(v) => v.to_vec(),
        Value::Vec3(v) => v.to_vec(),
        Value::Vec4(v) | Value::Color(v) => v.to_vec(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        assert!(is_ident("a"));
        assert!(is_ident("wave_2"));
        assert!(!is_ident(""));
        assert!(!is_ident("2a"));
        assert!(!is_ident("Wave"));
        assert!(!is_ident("a-b"));
    }

    fn node(id: &str) -> DescribedNode {
        DescribedNode {
            id: id.to_owned(),
            version: 1,
            summary: "s".to_owned(),
            time_dependent: false,
            inputs: vec![DescribedPort {
                name: "a".to_owned(),
                ty: PortType::Float,
                default: Some(Value::Float(1.0)),
            }],
            outputs: vec![DescribedPort {
                name: "out".to_owned(),
                ty: PortType::Float,
                default: None,
            }],
        }
    }

    #[test]
    fn rules() {
        assert_eq!(validate(&node("demo.a")), Ok(()));
        assert!(validate(&node("demo")).is_err());
        assert!(validate(&node("core.math.x")).is_err());
        assert!(validate(&node("demo..x")).is_err());
        let mut n = node("demo.a");
        n.version = 0;
        assert!(validate(&n).is_err());
        let mut n = node("demo.a");
        n.inputs[0].default = Some(Value::Int(1));
        assert!(validate(&n).unwrap_err().contains("default is Int"));
        let mut n = node("demo.a");
        n.inputs[0].default = Some(Value::Float(f64::NAN));
        assert!(validate(&n).is_err());
        let mut n = node("demo.a");
        n.outputs[0].default = Some(Value::Float(0.0));
        assert!(validate(&n).is_err());
        let mut n = node("demo.a");
        n.inputs.push(n.inputs[0].clone());
        assert!(validate(&n).unwrap_err().contains("twice"));
        let mut n = node("demo.a");
        n.outputs.clear();
        assert!(validate(&n).is_err());
    }
}
