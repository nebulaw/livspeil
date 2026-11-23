// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The example package `examples/wasm/hello-node`: the committed `.wasm`
//! matches its WAT source, describes two valid node types, cooks them
//! deterministically and works inside a graph.

mod common;

use std::path::Path;
use std::sync::Arc;

use livspeil_doc::parse;
use livspeil_graph::Cooker;
use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeRegistry, PortType, Value};
use livspeil_nodes_core::core_registry;
use livspeil_wasm_host::{WasmHost, WasmPackage};

fn hello(host: &WasmHost) -> WasmPackage {
    host.load_file(&common::hello_wasm()).unwrap()
}

fn node(pkg: &WasmPackage, id: &str) -> Arc<dyn NodeImpl> {
    pkg.nodes().into_iter().find(|n| n.spec().id == id).unwrap()
}

/// `cargo xtask wasm-example` (`UPDATE_GOLDEN=1`) rewrites the fixture.
#[test]
fn fixture_is_up_to_date() {
    let wasm = common::hello_wasm();
    let wat = wasm.with_extension("wat");
    let built = wat::parse_file(&wat).unwrap();
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&wasm, &built).unwrap();
    }
    let committed = std::fs::read(&wasm).unwrap();
    assert!(
        committed == built,
        "{} is stale: run `cargo xtask wasm-example`",
        wasm.display()
    );
    // Binary and text load to the same package.
    let host = WasmHost::new().unwrap();
    let text = host.load_file(&wat).unwrap();
    assert_eq!(text.specs(), hello(&host).specs());
}

#[test]
fn describes_two_nodes() {
    let host = WasmHost::new().unwrap();
    let pkg = hello(&host);
    let ids: Vec<String> = pkg.specs().iter().map(|s| s.qualified_id()).collect();
    assert_eq!(ids, ["demo.wave.sine@1", "demo.text.shout@1"]);
    let sine = &pkg.specs()[0];
    assert!(sine.time_dependent);
    let inputs: Vec<(&str, PortType, Option<Value>)> = sine
        .inputs
        .iter()
        .map(|p| (p.name, p.ty, p.default.clone()))
        .collect();
    assert_eq!(
        inputs,
        [
            ("frequency", PortType::Float, Some(Value::Float(1.0))),
            ("amplitude", PortType::Float, Some(Value::Float(1.0))),
            ("phase", PortType::Float, Some(Value::Float(0.0))),
        ]
    );
    let shout = &pkg.specs()[1];
    assert!(!shout.time_dependent);
    assert_eq!(
        shout.inputs[0].default,
        Some(Value::String("hello".to_owned()))
    );
    assert_eq!(shout.outputs[0].ty, PortType::String);
    assert_eq!(pkg.sha256_hex().len(), 64);
}

#[test]
fn sine_follows_the_clock() {
    let host = WasmHost::new().unwrap();
    let sine = node(&hello(&host), "demo.wave.sine");
    for (time, freq, amp, phase) in [
        (0.0, 1.0, 1.0, 0.0),
        (0.125, 1.0, 2.0, 0.0),
        (0.3, 0.5, 1.0, 0.25),
        (-1.7, 3.0, 0.5, 0.1),
        (12345.678, 0.25, 1.0, 0.0),
    ] {
        let ctx = CookContext {
            frame: 0,
            time,
            dt: 0.0,
        };
        let out = sine
            .cook(
                &ctx,
                &[Value::Float(freq), Value::Float(amp), Value::Float(phase)],
            )
            .unwrap();
        let expected = amp * (std::f64::consts::TAU * (freq * time + phase)).sin();
        let Value::Float(got) = out[0] else {
            panic!("{out:?}")
        };
        assert!(
            (got - expected).abs() < 1e-8,
            "t={time}: {got} vs {expected}"
        );
    }
}

#[test]
fn shout_and_its_typed_error() {
    let host = WasmHost::new().unwrap();
    let shout = node(&hello(&host), "demo.text.shout");
    let ctx = CookContext::default();
    let out = shout
        .cook(&ctx, &[Value::String("héllo, wasm".to_owned())])
        .unwrap();
    assert_eq!(out, [Value::String("HéLLO, WASM!".to_owned())]);
    let err = shout
        .cook(&ctx, &[Value::String(String::new())])
        .unwrap_err();
    assert_eq!(
        err,
        NodeError::InvalidInput {
            port: "text",
            reason: "text is empty".to_owned()
        }
    );
}

#[test]
fn deterministic_across_cooks_and_hosts() {
    let a = WasmHost::new().unwrap();
    let b = WasmHost::new().unwrap();
    let ctx = CookContext {
        frame: 7,
        time: 0.7,
        dt: 0.1,
    };
    let inputs = [Value::Float(1.3), Value::Float(0.9), Value::Float(0.05)];
    let first = node(&hello(&a), "demo.wave.sine")
        .cook(&ctx, &inputs)
        .unwrap();
    for host in [&a, &b, &a] {
        let again = node(&hello(host), "demo.wave.sine")
            .cook(&ctx, &inputs)
            .unwrap();
        let (Value::Float(x), Value::Float(y)) = (&first[0], &again[0]) else {
            panic!()
        };
        assert_eq!(x.to_bits(), y.to_bits());
    }
}

#[test]
fn modules_are_compiled_once() {
    let host = WasmHost::new().unwrap();
    let a = hello(&host);
    let b = host
        .load_bytes(&std::fs::read(common::hello_wasm()).unwrap())
        .unwrap();
    assert!(a.same_module(&b));
    assert_eq!(host.cached(), 1);
}

#[test]
fn registers_and_cooks_in_a_graph() {
    let host = WasmHost::new().unwrap();
    let pkg = hello(&host);
    let mut registry = core_registry();
    let before = registry.len();
    assert_eq!(pkg.register(&mut registry), Ok(2));
    assert_eq!(registry.len(), before + 2);
    assert!(registry.get("demo.text.shout@1").is_some());
    // Registering the same package twice is a duplicate.
    assert!(pkg.register(&mut NodeRegistry::clone(&registry)).is_err());

    let text = r#"
room "Wasm" format=1 min-tier="T0" {
  network "/" {
    node "wave" type="demo.wave.sine@1" id="0194b1a0-0000-7000-8000-0000000000a1" {
      input "frequency" 0.25
    }
    node "label" type="core.text.format@1" id="0194b1a0-0000-7000-8000-0000000000a2" {
      input "template" "level {a}"
    }
    node "loud" type="demo.text.shout@1" id="0194b1a0-0000-7000-8000-0000000000a3"
    link from="wave.out" to="label.a"
    link from="label.out" to="loud.text"
  }
}
"#;
    let graph = parse(text)
        .and_then(|doc| doc.to_graph(Arc::new(registry)))
        .unwrap();
    let loud = graph.find("loud").unwrap();
    let mut cooker = Cooker::new();
    cooker.set_clock(CookContext {
        frame: 60,
        time: 1.0,
        dt: 1.0 / 60.0,
    });
    assert_eq!(
        cooker.cook(&graph, loud).unwrap(),
        [Value::String("LEVEL 1.00!".to_owned())]
    );
}

#[test]
fn reads_missing_files_as_errors() {
    let host = WasmHost::new().unwrap();
    let err = host
        .load_file(Path::new("/nonexistent/x.wasm"))
        .unwrap_err();
    assert!(err.to_string().contains("cannot read"), "{err}");
}
