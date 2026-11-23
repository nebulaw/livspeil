// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Test helpers: an independent encoder for `describe` buffers and a WAT
//! template for small guest modules.

#![allow(dead_code)]

use std::path::PathBuf;

pub const FLOAT: u8 = 0;
pub const INT: u8 = 1;
pub const BOOL: u8 = 2;
pub const STRING: u8 = 7;

/// The example package, as committed.
pub fn hello_wasm() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/wasm/hello-node/hello-node.wasm")
}

pub fn string(s: &str) -> Vec<u8> {
    let mut out = (s.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(s.as_bytes());
    out
}

pub fn float_value(x: f64) -> Vec<u8> {
    let mut out = vec![FLOAT];
    out.extend_from_slice(&x.to_le_bytes());
    out
}

/// A port: name, type tag and encoded default value.
pub struct Port {
    pub name: &'static str,
    pub tag: u8,
    pub default: Option<Vec<u8>>,
}

pub fn port(name: &'static str, tag: u8, default: Option<Vec<u8>>) -> Port {
    Port { name, tag, default }
}

pub struct Node {
    pub id: &'static str,
    pub version: u32,
    pub time_dependent: bool,
    pub inputs: Vec<Port>,
    pub outputs: Vec<Port>,
}

/// A node with one Float input `a` (default 0.0) and one Float output.
pub fn simple(id: &'static str) -> Node {
    Node {
        id,
        version: 1,
        time_dependent: false,
        inputs: vec![port("a", FLOAT, Some(float_value(0.0)))],
        outputs: vec![port("out", FLOAT, None)],
    }
}

fn ports(out: &mut Vec<u8>, ports: &[Port]) {
    out.extend_from_slice(&(ports.len() as u32).to_le_bytes());
    for p in ports {
        out.extend(string(p.name));
        out.push(p.tag);
        match &p.default {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                out.extend_from_slice(v);
            }
        }
    }
}

pub fn describe(nodes: &[Node]) -> Vec<u8> {
    let mut out = b"LSND".to_vec();
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(nodes.len() as u32).to_le_bytes());
    for n in nodes {
        out.extend(string(n.id));
        out.extend_from_slice(&n.version.to_le_bytes());
        out.extend(string("test node"));
        out.push(u8::from(n.time_dependent));
        ports(&mut out, &n.inputs);
        ports(&mut out, &n.outputs);
    }
    out
}

fn escape(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("\\{b:02x}")).collect()
}

/// A guest module: `describe` returns `describe` (stored at offset 0),
/// `alloc` bumps from 64 KiB, and `livspeil_cook` runs `cook_body`
/// (params `$node`, `$in`, `$len`; result i64; local `$out` i32). `extra`
/// adds module fields.
pub fn module(describe: &[u8], cook_body: &str, extra: &str) -> String {
    format!(
        r#"(module
  (memory (export "memory") 2 1000)
  (data (i32.const 0) "{data}")
  (global $heap (mut i32) (i32.const 65536))
  (func $alloc (export "livspeil_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $heap))
    (global.set $heap (i32.add (local.get $p) (local.get $len)))
    (local.get $p))
  (func $pack (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
            (i64.extend_i32_u (local.get $len))))
  (func (export "livspeil_describe") (result i64) (i64.const {len}))
  (func (export "livspeil_cook") (param $node i32) (param $in i32) (param $len i32) (result i64)
    (local $out i32)
    {cook_body})
  {extra}
)"#,
        data = escape(describe),
        len = describe.len(),
    )
}

/// A cook body returning the bytes `result` (written at 32 KiB).
pub fn returning(result: &[u8]) -> (String, String) {
    (
        format!(
            "(call $pack (i32.const 32768) (i32.const {}))",
            result.len()
        ),
        format!(r#"(data (i32.const 32768) "{}")"#, escape(result)),
    )
}

/// A cook result `ok([values])`.
pub fn ok(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0];
    out.extend_from_slice(&(values.len() as u32).to_le_bytes());
    for v in values {
        out.extend_from_slice(v);
    }
    out
}
