// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Sandboxing and validation: traps, fuel, memory and stack limits, ABI
//! violations and invalid descriptions become errors, never host panics.

mod common;

use common::{
    FLOAT, INT, Node, STRING, describe, float_value, module, ok, port, returning, simple,
};
use livspeil_node_api::{CookContext, NodeError, NodeImpl, Value};
use livspeil_wasm_host::{LoadError, WasmHost, WasmLimits};

fn host() -> WasmHost {
    WasmHost::with_limits(WasmLimits {
        fuel: 1_000_000,
        memory_bytes: 4 << 20,
        max_stack_bytes: 256 << 10,
    })
    .unwrap()
}

/// Load a one-node package (`demo.t@1`, Float `a` -> Float `out`).
fn load(cook_body: &str, extra: &str) -> std::sync::Arc<dyn NodeImpl> {
    let wat = module(&describe(&[simple("demo.t")]), cook_body, extra);
    host().load_bytes(wat.as_bytes()).unwrap().nodes().remove(0)
}

fn cook(node: &dyn NodeImpl) -> Result<Vec<Value>, NodeError> {
    node.cook(&CookContext::default(), &[Value::Float(2.0)])
}

fn failed(result: Result<Vec<Value>, NodeError>) -> String {
    match result {
        Err(NodeError::Failed(msg)) => msg,
        other => panic!("expected a failure, got {other:?}"),
    }
}

fn load_err(wat: &str) -> String {
    host().load_bytes(wat.as_bytes()).unwrap_err().to_string()
}

#[test]
fn cook_success_and_int_to_float() {
    let (body, data) = returning(&ok(&[float_value(4.5)]));
    assert_eq!(cook(&*load(&body, &data)).unwrap(), [Value::Float(4.5)]);
    let mut int = vec![INT];
    int.extend_from_slice(&3i64.to_le_bytes());
    let (body, data) = returning(&ok(&[int]));
    assert_eq!(cook(&*load(&body, &data)).unwrap(), [Value::Float(3.0)]);
}

#[test]
fn echo_round_trips_every_value_type() {
    let all = [
        ("f", FLOAT, Value::Float(-0.25)),
        ("i", INT, Value::Int(i64::MIN)),
        ("b", common::BOOL, Value::Bool(true)),
        ("v2", 3, Value::Vec2([1.0, -2.0])),
        ("v3", 4, Value::Vec3([1.0, 2.0, 3.0])),
        ("v4", 5, Value::Vec4([4.0, 3.0, 2.0, 1.0])),
        ("c", 6, Value::Color([0.7, 0.1, 200.0, 0.5])),
        ("s", STRING, Value::String("ünïcode ✓".to_owned())),
    ];
    let node = Node {
        id: "demo.echo",
        version: 1,
        time_dependent: false,
        inputs: all.iter().map(|(n, t, _)| port(n, *t, None)).collect(),
        outputs: all.iter().map(|(n, t, _)| port(n, *t, None)).collect(),
    };
    // The output list encodes like the input list: overwrite the last
    // byte of `dt` with the ok tag and return from there.
    let body = "(i32.store8 offset=23 (local.get $in) (i32.const 0))
                (call $pack (i32.add (local.get $in) (i32.const 23))
                            (i32.sub (local.get $len) (i32.const 23)))";
    let wat = module(&describe(&[node]), body, "");
    let echo = host().load_bytes(wat.as_bytes()).unwrap().nodes().remove(0);
    let values: Vec<Value> = all.iter().map(|(_, _, v)| v.clone()).collect();
    assert_eq!(echo.cook(&CookContext::default(), &values).unwrap(), values);
}

#[test]
fn guest_errors_are_typed() {
    let mut failed_result = vec![1, 1];
    failed_result.extend(common::string("no luck"));
    let (body, data) = returning(&failed_result);
    assert_eq!(
        cook(&*load(&body, &data)),
        Err(NodeError::Failed("no luck".to_owned()))
    );
    let mut invalid = vec![1, 0];
    invalid.extend(common::string("a"));
    invalid.extend(common::string("too big"));
    let (body, data) = returning(&invalid);
    assert_eq!(
        cook(&*load(&body, &data)),
        Err(NodeError::InvalidInput {
            port: "a",
            reason: "too big".to_owned()
        })
    );
    // A port the node does not have is a plain failure.
    let mut unknown = vec![1, 0];
    unknown.extend(common::string("zz"));
    unknown.extend(common::string("bad"));
    let (body, data) = returning(&unknown);
    assert_eq!(
        cook(&*load(&body, &data)),
        Err(NodeError::Failed("invalid input 'zz': bad".to_owned()))
    );
}

#[test]
fn traps_are_contained() {
    let node = load("unreachable", "");
    let msg = failed(cook(&*node));
    assert!(msg.contains("demo.t@1") && msg.contains("trapped"), "{msg}");
    // The host survives and the node can be cooked again (same error).
    assert_eq!(failed(cook(&*node)), msg);
    let div = load(
        "(drop (i32.div_u (i32.const 1) (i32.sub (local.get $node) (local.get $node)))) (i64.const 0)",
        "",
    );
    assert!(failed(cook(&*div)).contains("divide by zero"));
    let oob = load("(drop (i32.load (i32.const -4))) (i64.const 0)", "");
    assert!(failed(cook(&*oob)).contains("out of bounds"));
}

#[test]
fn fuel_stops_infinite_loops() {
    let node = load("(loop $l (br $l)) (i64.const 0)", "");
    let msg = failed(cook(&*node));
    assert!(msg.contains("ran out of fuel (limit 1000000)"), "{msg}");
    // Fuel is per cook: a cheap cook after an exhausted one still works.
    let (body, data) = returning(&ok(&[float_value(1.0)]));
    let fine = load(&body, &data);
    for _ in 0..3 {
        assert!(cook(&*fine).is_ok());
    }
}

#[test]
fn memory_limit() {
    // 4 MiB limit: growing by 100 pages (6.25 MiB) more traps.
    let node = load("(drop (memory.grow (i32.const 100))) (i64.const 0)", "");
    let msg = failed(cook(&*node));
    assert!(msg.contains("memory limit of 4194304 bytes"), "{msg}");
    // Within the limit is fine.
    let (body, data) = returning(&ok(&[float_value(1.0)]));
    let body = format!("(drop (memory.grow (i32.const 10))) {body}");
    assert!(cook(&*load(&body, &data)).is_ok());
    // An initial memory above the limit fails at load time.
    let wat = module(&describe(&[simple("demo.t")]), "(i64.const 0)", "").replace(
        "(memory (export \"memory\") 2 1000)",
        "(memory (export \"memory\") 100)",
    );
    assert!(
        load_err(&wat).contains("memory limit"),
        "{}",
        load_err(&wat)
    );
}

#[test]
fn stack_overflow_is_contained() {
    let node = load(
        "(call $deep (i32.const 0)) (i64.const 0)",
        "(func $deep (param i32) (call $deep (i32.add (local.get 0) (i32.const 1))))",
    );
    let msg = failed(cook(&*node));
    assert!(msg.contains("overflowed the wasm stack"), "{msg}");
}

#[test]
fn abi_violations_at_cook() {
    // Result outside memory.
    let node = load("(i64.const 0x7fffffff00000010)", "");
    assert!(failed(cook(&*node)).contains("outside linear memory"));
    // Wrong output count.
    let (body, data) = returning(&ok(&[]));
    assert!(failed(cook(&*load(&body, &data))).contains("0 values for 1 outputs"));
    // Wrong output type.
    let (body, data) = returning(&ok(&[vec![common::BOOL, 1]]));
    let msg = failed(cook(&*load(&body, &data)));
    assert!(
        msg.contains("returned Bool for output 'out' of type Float"),
        "{msg}"
    );
    // Malformed buffer: invalid UTF-8 string.
    let (body, data) = returning(&ok(&[vec![STRING, 1, 0, 0, 0, 0xff]]));
    assert!(failed(cook(&*load(&body, &data))).contains("broke the ABI"));
    // Trailing bytes.
    let mut extra = ok(&[float_value(1.0)]);
    extra.push(0);
    let (body, data) = returning(&extra);
    assert!(failed(cook(&*load(&body, &data))).contains("trailing"));
}

#[test]
fn time_is_hidden_from_time_independent_nodes() {
    // Return `time` from the context as the output.
    let body = "(local.set $out (call $alloc (i32.const 14)))
        (i32.store8 (local.get $out) (i32.const 0))
        (i32.store offset=1 (local.get $out) (i32.const 1))
        (i32.store8 offset=5 (local.get $out) (i32.const 0))
        (f64.store offset=6 (local.get $out) (f64.load offset=8 (local.get $in)))
        (call $pack (local.get $out) (i32.const 14))";
    let ctx = CookContext {
        frame: 3,
        time: 2.5,
        dt: 0.5,
    };
    let mut timed = simple("demo.timed");
    timed.time_dependent = true;
    let wat = module(&describe(&[simple("demo.pure"), timed]), body, "");
    let nodes = host().load_bytes(wat.as_bytes()).unwrap().nodes();
    assert_eq!(
        nodes[0].cook(&ctx, &[Value::Float(0.0)]).unwrap(),
        [Value::Float(0.0)]
    );
    assert_eq!(
        nodes[1].cook(&ctx, &[Value::Float(0.0)]).unwrap(),
        [Value::Float(2.5)]
    );
}

#[test]
fn state_does_not_survive_between_cooks() {
    // A counter in a global: every cook sees a fresh instance.
    let body = "(global.set $count (f64.add (global.get $count) (f64.const 1)))
        (local.set $out (call $alloc (i32.const 14)))
        (i32.store8 (local.get $out) (i32.const 0))
        (i32.store offset=1 (local.get $out) (i32.const 1))
        (i32.store8 offset=5 (local.get $out) (i32.const 0))
        (f64.store offset=6 (local.get $out) (global.get $count))
        (call $pack (local.get $out) (i32.const 14))";
    let node = load(body, "(global $count (mut f64) (f64.const 0))");
    for _ in 0..3 {
        assert_eq!(cook(&*node).unwrap(), [Value::Float(1.0)]);
    }
}

#[test]
fn imports_and_bad_exports_are_refused() {
    let base = module(&describe(&[simple("demo.t")]), "(i64.const 0)", "");
    let wasi = base.replacen(
        "(module",
        r#"(module (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
        1,
    );
    let msg = load_err(&wasi);
    assert!(
        msg.contains("imports are not allowed") && msg.contains("fd_write"),
        "{msg}"
    );
    let no_cook = base.replace("(export \"livspeil_cook\")", "");
    assert!(load_err(&no_cook).contains("export 'livspeil_cook': missing"));
    let bad_sig = base.replace(
        "(export \"livspeil_describe\") (result i64) (i64.const",
        "(export \"livspeil_describe\") (result i32) (i32.const",
    );
    assert!(load_err(&bad_sig).contains("export 'livspeil_describe': wrong signature"));
    let wrong = base.replace(
        "(func $alloc (export \"livspeil_alloc\") (param $len i32) (result i32)",
        "(func $alloc (export \"livspeil_alloc\") (param $len i32) (result i32) (unreachable)) (func $alloc2 (param $len i32) (result i32)",
    );
    // `alloc` traps: loading works (describe does not allocate), cooking fails.
    let node = host()
        .load_bytes(wrong.as_bytes())
        .unwrap()
        .nodes()
        .remove(0);
    assert!(failed(cook(&*node)).contains("trapped"));
    assert!(matches!(
        host().load_bytes(b"\0asm garbage"),
        Err(LoadError::Compile(_))
    ));
    let no_memory = base.replace("(export \"memory\")", "");
    assert!(load_err(&no_memory).contains("export 'memory': missing"));
}

#[test]
fn describe_failures() {
    let trap = module(&describe(&[simple("demo.t")]), "(i64.const 0)", "").replace(
        "(result i64) (i64.const",
        "(result i64) unreachable (i64.const",
    );
    assert!(
        load_err(&trap).contains("describe failed: trapped"),
        "{}",
        load_err(&trap)
    );
    let looping = module(&describe(&[simple("demo.t")]), "(i64.const 0)", "").replace(
        "(result i64) (i64.const",
        "(result i64) (loop $l (br $l)) (i64.const",
    );
    assert!(load_err(&looping).contains("ran out of fuel"));
}

fn invalid(bytes: &[u8]) -> String {
    load_err(&module(bytes, "(i64.const 0)", ""))
}

#[test]
fn descriptions_are_validated() {
    assert!(
        invalid(&describe(&[simple("core.math.add")])).contains("'core' namespace is reserved")
    );
    assert!(invalid(&describe(&[simple("noname")])).contains("must be namespaced"));
    assert!(invalid(&describe(&[simple("Demo.x")])).contains("must be namespaced"));
    assert!(invalid(&describe(&[simple("demo.a"), simple("demo.a")])).contains("described twice"));
    let mut bad_default = simple("demo.a");
    bad_default.inputs[0].default = Some(vec![STRING, 0, 0, 0, 0]);
    assert!(invalid(&describe(&[bad_default])).contains("is Float but its default is String"));
    let mut bad_type = simple("demo.a");
    bad_type.inputs[0].tag = 9;
    assert!(invalid(&describe(&[bad_type])).contains("unknown port type tag 9"));
    let mut no_outputs = simple("demo.a");
    no_outputs.outputs.clear();
    assert!(invalid(&describe(&[no_outputs])).contains("at least one output"));
    let mut version0 = simple("demo.a");
    version0.version = 0;
    assert!(invalid(&describe(&[version0])).contains("version"));
    assert!(invalid(&describe(&[])).contains("no node types"));
    let mut wrong_magic = describe(&[simple("demo.a")]);
    wrong_magic[0] = b'X';
    assert!(invalid(&wrong_magic).contains("bad magic"));
    let mut v2 = describe(&[simple("demo.a")]);
    v2[4] = 2;
    assert!(invalid(&v2).contains("ABI version 2 is not supported"));
    let full = describe(&[simple("demo.a")]);
    assert!(invalid(&full[..full.len() - 1]).contains("truncated"));
    let mut trailing = full.clone();
    trailing.push(0);
    assert!(invalid(&trailing).contains("trailing"));
}
