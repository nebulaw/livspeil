// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The byte encoding of the core-module ABI, version 1.
//!
//! The logical types are those of `wit/livspeil-node.wit`; this module is
//! their canonical binary form, used across the guest's linear memory. All
//! integers and floats are little-endian; see `docs/wasm-nodes.md` for the
//! full reference.
//!
//! | Type | Encoding |
//! |---|---|
//! | `bool` | `u8`, 0 or 1 |
//! | `u32`, `u64`, `s64`, `f64` | 4 or 8 bytes, little-endian |
//! | `string` | `u32` byte length, then UTF-8 bytes |
//! | `list<T>` | `u32` count, then the items |
//! | `option<T>` | `u8` 0 (none) or 1 followed by `T` |
//! | `port-type` | `u8` tag ([`TAG_FLOAT`] .. [`TAG_STRING`]) |
//! | `value` | `u8` tag, then the payload (`f64`, `s64`, `bool`, 2/3/4 × `f64`, `string`) |

use livspeil_node_api::{CookContext, NodeError, PortType, Value};

/// Magic bytes at the start of the `describe` buffer.
pub const MAGIC: [u8; 4] = *b"LSND";
/// The ABI version this host speaks.
pub const ABI_VERSION: u32 = 1;

/// Value and port-type tags.
pub const TAG_FLOAT: u8 = 0;
pub const TAG_INT: u8 = 1;
pub const TAG_BOOL: u8 = 2;
pub const TAG_VEC2: u8 = 3;
pub const TAG_VEC3: u8 = 4;
pub const TAG_VEC4: u8 = 5;
pub const TAG_COLOR: u8 = 6;
pub const TAG_STRING: u8 = 7;

/// Result tags of `livspeil_cook`.
pub const RESULT_OK: u8 = 0;
pub const RESULT_ERR: u8 = 1;
/// `node-error` tags.
pub const ERROR_INVALID_INPUT: u8 = 0;
pub const ERROR_FAILED: u8 = 1;

/// The port type of a tag, if it is one this ABI carries.
#[must_use]
pub fn port_type(tag: u8) -> Option<PortType> {
    Some(match tag {
        TAG_FLOAT => PortType::Float,
        TAG_INT => PortType::Int,
        TAG_BOOL => PortType::Bool,
        TAG_VEC2 => PortType::Vec2,
        TAG_VEC3 => PortType::Vec3,
        TAG_VEC4 => PortType::Vec4,
        TAG_COLOR => PortType::Color,
        TAG_STRING => PortType::String,
        _ => return None,
    })
}

/// The tag of a port type, if this ABI carries it.
#[must_use]
pub fn tag(ty: PortType) -> Option<u8> {
    Some(match ty {
        PortType::Float => TAG_FLOAT,
        PortType::Int => TAG_INT,
        PortType::Bool => TAG_BOOL,
        PortType::Vec2 => TAG_VEC2,
        PortType::Vec3 => TAG_VEC3,
        PortType::Vec4 => TAG_VEC4,
        PortType::Color => TAG_COLOR,
        PortType::String => TAG_STRING,
        _ => return None,
    })
}

/// A buffer that does not follow the encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError(pub String);

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

/// Reads the encoding from a byte slice.
#[derive(Debug)]
pub struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Reader { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize, what: &str) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.bytes.len());
        match end {
            Some(end) => {
                let out = &self.bytes[self.pos..end];
                self.pos = end;
                Ok(out)
            }
            None => Err(DecodeError(format!(
                "truncated {what} at byte {} (buffer is {} bytes)",
                self.pos,
                self.bytes.len()
            ))),
        }
    }

    fn array<const N: usize>(&mut self, what: &str) -> Result<[u8; N], DecodeError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N, what)?);
        Ok(out)
    }

    pub fn u8(&mut self, what: &str) -> Result<u8, DecodeError> {
        Ok(self.array::<1>(what)?[0])
    }

    pub fn u32(&mut self, what: &str) -> Result<u32, DecodeError> {
        Ok(u32::from_le_bytes(self.array(what)?))
    }

    pub fn u64(&mut self, what: &str) -> Result<u64, DecodeError> {
        Ok(u64::from_le_bytes(self.array(what)?))
    }

    pub fn i64(&mut self, what: &str) -> Result<i64, DecodeError> {
        Ok(i64::from_le_bytes(self.array(what)?))
    }

    pub fn f64(&mut self, what: &str) -> Result<f64, DecodeError> {
        Ok(f64::from_le_bytes(self.array(what)?))
    }

    pub fn bool(&mut self, what: &str) -> Result<bool, DecodeError> {
        match self.u8(what)? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(DecodeError(format!("{what}: invalid bool {other}"))),
        }
    }

    /// A string of at most `max` bytes.
    pub fn string(&mut self, what: &str, max: usize) -> Result<String, DecodeError> {
        let len = self.u32(what)? as usize;
        if len > max {
            return Err(DecodeError(format!(
                "{what}: {len} bytes, more than the limit of {max}"
            )));
        }
        let bytes = self.take(len, what)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| DecodeError(format!("{what}: not valid UTF-8")))
    }

    /// A list count of at most `max` items.
    pub fn count(&mut self, what: &str, max: usize) -> Result<usize, DecodeError> {
        let n = self.u32(what)? as usize;
        if n > max {
            return Err(DecodeError(format!(
                "{what}: {n} items, more than the limit of {max}"
            )));
        }
        Ok(n)
    }

    /// A value; strings are limited to `max_string` bytes.
    pub fn value(&mut self, what: &str, max_string: usize) -> Result<Value, DecodeError> {
        let tag = self.u8(what)?;
        let floats = |n: usize, this: &mut Self| -> Result<[f64; 4], DecodeError> {
            let mut out = [0.0; 4];
            for slot in out.iter_mut().take(n) {
                *slot = this.f64(what)?;
            }
            Ok(out)
        };
        Ok(match tag {
            TAG_FLOAT => Value::Float(self.f64(what)?),
            TAG_INT => Value::Int(self.i64(what)?),
            TAG_BOOL => Value::Bool(self.bool(what)?),
            TAG_VEC2 => {
                let v = floats(2, self)?;
                Value::Vec2([v[0], v[1]])
            }
            TAG_VEC3 => {
                let v = floats(3, self)?;
                Value::Vec3([v[0], v[1], v[2]])
            }
            TAG_VEC4 => Value::Vec4(floats(4, self)?),
            TAG_COLOR => Value::Color(floats(4, self)?),
            TAG_STRING => Value::String(self.string(what, max_string)?),
            other => return Err(DecodeError(format!("{what}: unknown value tag {other}"))),
        })
    }

    /// Fail unless every byte was read.
    pub fn finish(&self, what: &str) -> Result<(), DecodeError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(DecodeError(format!(
                "{what}: {} trailing bytes",
                self.bytes.len() - self.pos
            )))
        }
    }
}

/// Builds an encoded buffer.
#[derive(Debug, Default)]
pub struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }

    pub fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    pub fn f64(&mut self, v: f64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    /// A length or count; `None` when it does not fit in a `u32`.
    fn len(&mut self, n: usize) -> Option<()> {
        self.u32(u32::try_from(n).ok()?);
        Some(())
    }

    /// A string; `None` when longer than `u32::MAX` bytes.
    pub fn string(&mut self, s: &str) -> Option<()> {
        self.len(s.len())?;
        self.bytes.extend_from_slice(s.as_bytes());
        Some(())
    }

    /// A list count; `None` when larger than `u32::MAX`.
    pub fn count(&mut self, n: usize) -> Option<()> {
        self.len(n)
    }

    /// A value; `None` for types this ABI does not carry.
    pub fn value(&mut self, value: &Value) -> Option<()> {
        self.u8(tag(value.port_type())?);
        match value {
            Value::Float(x) => self.f64(*x),
            Value::Int(i) => self.bytes.extend_from_slice(&i.to_le_bytes()),
            Value::Bool(b) => self.u8(u8::from(*b)),
            Value::Vec2(v) => v.iter().for_each(|&x| self.f64(x)),
            Value::Vec3(v) => v.iter().for_each(|&x| self.f64(x)),
            Value::Vec4(v) | Value::Color(v) => v.iter().for_each(|&x| self.f64(x)),
            Value::String(s) => self.string(s)?,
            _ => return None,
        }
        Some(())
    }
}

/// The `livspeil_cook` input buffer: context, then `list<value>`.
///
/// # Errors
/// Returns a [`NodeError::Failed`] for a value this ABI cannot carry.
pub fn encode_cook_input(ctx: &CookContext, inputs: &[Value]) -> Result<Vec<u8>, NodeError> {
    let mut w = Writer::new();
    w.u64(ctx.frame);
    w.f64(ctx.time);
    w.f64(ctx.dt);
    w.count(inputs.len())
        .ok_or_else(|| NodeError::Failed("too many inputs".to_owned()))?;
    for (index, value) in inputs.iter().enumerate() {
        w.value(value).ok_or_else(|| {
            NodeError::Failed(format!(
                "input {index}: a {} value cannot be passed to a wasm node",
                value.port_type()
            ))
        })?;
    }
    Ok(w.into_bytes())
}

/// A decoded `livspeil_cook` result.
#[derive(Debug, Clone, PartialEq)]
pub enum CookResult {
    Ok(Vec<Value>),
    InvalidInput { port: String, reason: String },
    Failed(String),
}

/// Decode a `livspeil_cook` result buffer.
///
/// # Errors
/// Returns a [`DecodeError`] when the buffer does not follow the encoding.
pub fn decode_cook_result(
    bytes: &[u8],
    max_values: usize,
    max_string: usize,
) -> Result<CookResult, DecodeError> {
    let mut r = Reader::new(bytes);
    let result = match r.u8("result tag")? {
        RESULT_OK => {
            let n = r.count("output list", max_values)?;
            let mut values = Vec::with_capacity(n);
            for _ in 0..n {
                values.push(r.value("output value", max_string)?);
            }
            CookResult::Ok(values)
        }
        RESULT_ERR => match r.u8("node-error tag")? {
            ERROR_INVALID_INPUT => CookResult::InvalidInput {
                port: r.string("invalid-input port", max_string)?,
                reason: r.string("invalid-input reason", max_string)?,
            },
            ERROR_FAILED => CookResult::Failed(r.string("failed reason", max_string)?),
            other => return Err(DecodeError(format!("unknown node-error tag {other}"))),
        },
        other => return Err(DecodeError(format!("unknown result tag {other}"))),
    };
    r.finish("cook result")?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_round_trip() {
        for t in 0..=255u8 {
            if let Some(ty) = port_type(t) {
                assert_eq!(tag(ty), Some(t));
            }
        }
        assert_eq!(tag(PortType::Table), None);
        assert_eq!(port_type(8), None);
    }

    #[test]
    fn values_round_trip() {
        let values = [
            Value::Float(-1.5),
            Value::Int(-7),
            Value::Bool(true),
            Value::Vec2([1.0, 2.0]),
            Value::Vec3([1.0, 2.0, 3.0]),
            Value::Vec4([1.0, 2.0, 3.0, 4.0]),
            Value::Color([0.5, 0.1, 120.0, 1.0]),
            Value::String("héllo".to_owned()),
        ];
        let mut w = Writer::new();
        for v in &values {
            w.value(v).unwrap();
        }
        let bytes = w.into_bytes();
        let mut r = Reader::new(&bytes);
        for v in &values {
            assert_eq!(&r.value("v", 64).unwrap(), v);
        }
        r.finish("all").unwrap();
    }

    #[test]
    fn decode_rejects_bad_buffers() {
        assert!(decode_cook_result(&[], 4, 64).is_err());
        assert!(decode_cook_result(&[9], 4, 64).is_err());
        assert!(decode_cook_result(&[0, 1, 0, 0, 0], 4, 64).is_err());
        assert!(decode_cook_result(&[0, 5, 0, 0, 0], 4, 64).is_err());
        assert!(decode_cook_result(&[0, 0, 0, 0, 0, 7], 4, 64).is_err());
        assert!(decode_cook_result(&[1, 1, 2, 0, 0, 0, 0xff, 0xfe], 4, 64).is_err());
        assert_eq!(
            decode_cook_result(&[1, 1, 2, 0, 0, 0, b'n', b'o'], 4, 64),
            Ok(CookResult::Failed("no".to_owned()))
        );
        assert_eq!(
            decode_cook_result(&[0, 1, 0, 0, 0, 2, 1], 4, 64),
            Ok(CookResult::Ok(vec![Value::Bool(true)]))
        );
        assert!(decode_cook_result(&[0, 1, 0, 0, 0, 2, 3], 4, 64).is_err());
    }

    #[test]
    fn non_abi_values_are_refused() {
        let err = encode_cook_input(&CookContext::default(), &[Value::Event]).unwrap_err();
        assert!(err.to_string().contains("Event"), "{err}");
    }
}
