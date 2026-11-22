// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `out("path")`: reading another node's output in an expression (D48).

use std::collections::BTreeMap;

use livspeil_node_api::{PortType, Value};
use livspeil_params::expr::{
    Channel, Env, ErrorKind, EvalContext, NoChannels, Program, RefKind, Span, channel_refs, compile,
};

/// Inputs and outputs in one slot space.
struct TwoKinds {
    inputs: BTreeMap<String, Channel>,
    outputs: BTreeMap<String, Channel>,
}

impl Env for TwoKinds {
    fn resolve(&self, path: &str) -> Option<Channel> {
        self.inputs.get(path).copied()
    }

    fn resolve_output(&self, path: &str) -> Option<Channel> {
        self.outputs.get(path).copied()
    }
}

fn env() -> TwoKinds {
    let mut inputs = BTreeMap::new();
    let mut outputs = BTreeMap::new();
    for (path, slot, ty) in [
        ("sun/elevation", 0, PortType::Float),
        ("books/limit", 1, PortType::Int),
    ] {
        inputs.insert(path.to_owned(), Channel { slot, ty });
    }
    for (path, slot, ty) in [
        ("sun/elevation", 2, PortType::Float),
        ("time_of_day/hour", 3, PortType::Float),
        ("books/count", 4, PortType::Int),
        ("sub/x/pos", 5, PortType::Vec2),
        ("../up/name", 6, PortType::String),
        ("/abs/x/tint", 7, PortType::Color),
        ("db/rows", 8, PortType::Table),
        ("ramp/curve", 9, PortType::Curve),
        ("cam/frame", 10, PortType::Image),
    ] {
        outputs.insert(path.to_owned(), Channel { slot, ty });
    }
    TwoKinds { inputs, outputs }
}

fn slots() -> Vec<Value> {
    vec![
        Value::Float(-10.0),
        Value::Int(3),
        Value::Float(45.0),
        Value::Float(6.5),
        Value::Int(25),
        Value::Vec2([1.0, 2.0]),
        Value::String("up".to_owned()),
        Value::Color([0.5, 0.1, 90.0, 1.0]),
    ]
}

fn prog(src: &str) -> Program {
    compile(src, &env()).unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn eval(src: &str) -> Value {
    let slots = slots();
    prog(src)
        .eval(&EvalContext::new(0, 0.0, 0.0, &slots))
        .unwrap_or_else(|e| panic!("{src}: {e}"))
}

#[test]
fn out_reads_outputs_with_their_types() {
    assert_eq!(
        eval(r#"smoothstep(5, 8, out("time_of_day/hour"))"#),
        Value::Float(0.5)
    );
    assert_eq!(
        eval(r#"max(0, out("sun/elevation") / 90)"#),
        Value::Float(0.5)
    );
    // Int output / Int literal stays Int (floored); ceil needs a Float.
    assert_eq!(eval(r#"out("books/count") / 12"#), Value::Int(2));
    assert_eq!(
        eval(r#"ceil(out("books/count") / 12.0)"#),
        Value::Float(3.0)
    );
    assert_eq!(eval(r#"out("sub/x/pos").y"#), Value::Float(2.0));
    assert_eq!(
        eval(r#"upper(out("../up/name"))"#),
        Value::String("UP".to_owned())
    );
    assert_eq!(eval(r#"out("/abs/x/tint").h"#), Value::Float(90.0));
    assert_eq!(prog(r#"out("books/count")"#).result_type(), PortType::Int);
}

#[test]
fn ch_and_out_of_the_same_path_are_different_ports() {
    // ch() reads the input, out() the output of the same name.
    assert_eq!(
        eval(r#"out("sun/elevation") - ch("sun/elevation")"#),
        Value::Float(55.0)
    );
    let p = prog(
        r#"ch("sun/elevation") + out("sun/elevation") + out("books/count") * ch("books/limit") + out("sun/elevation")"#,
    );
    assert_eq!(p.dependencies(), [0, 1]);
    assert_eq!(p.output_dependencies(), [2, 4]);
    let p = prog(r#"true ? 1 : out("books/count")"#);
    assert!(p.dependencies().is_empty());
    assert_eq!(p.output_dependencies(), [4]);
    assert!(!p.is_time_dependent());
    let p = prog(r#"ch("sun/elevation")"#);
    assert!(p.output_dependencies().is_empty());
}

#[test]
fn out_compile_errors_have_spans() {
    let e = compile(r#"1 + out("nope/x")"#, &env()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownOutput("nope/x".to_owned()));
    assert_eq!(e.span, Span::new(8, 16));
    assert_eq!(e.kind.to_string(), "unknown output path \"nope/x\"");

    // An input path is not an output path, and vice versa.
    let e = compile(r#"out("books/limit")"#, &env()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownOutput("books/limit".to_owned()));
    let e = compile(r#"ch("books/count")"#, &env()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownChannel("books/count".to_owned()));

    let e = compile(r#"out("a" + "b")"#, &env()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::OutputPathNotLiteral);
    assert_eq!(e.span, Span::new(4, 13));
    assert_eq!(
        e.kind.to_string(),
        "out() takes exactly one string literal path"
    );
    for src in ["out()", r#"out("a/b", "c/d")"#] {
        assert!(matches!(
            compile(src, &env()).unwrap_err().kind,
            ErrorKind::Arity { .. }
        ));
    }

    // Tables, curves and images are not expression types.
    for (path, ty) in [
        ("db/rows", PortType::Table),
        ("ramp/curve", PortType::Curve),
        ("cam/frame", PortType::Image),
    ] {
        let src = format!("out(\"{path}\")");
        let e = compile(&src, &env()).unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::UnsupportedOutputType {
                path: path.to_owned(),
                ty
            }
        );
        assert_eq!(e.span, Span::new(4, 6 + path.len()));
        assert_eq!(
            e.kind.to_string(),
            format!("output \"{path}\" has type {ty}, which expressions cannot use")
        );
    }

    // Type errors involving out() point at the call.
    let e = compile(r#"out("../up/name") + 1"#, &env()).unwrap_err();
    assert!(matches!(e.kind, ErrorKind::TypeMismatch { .. }));
    assert_eq!(e.span, Span::new(20, 21));
}

#[test]
fn environments_without_outputs_reject_out() {
    let e = compile(r#"out("a/b")"#, &NoChannels).unwrap_err();
    assert_eq!(e.kind, ErrorKind::UnknownOutput("a/b".to_owned()));
    // Map environments only know inputs.
    let mut map = BTreeMap::new();
    map.insert(
        "a/b".to_owned(),
        Channel {
            slot: 0,
            ty: PortType::Float,
        },
    );
    assert!(compile(r#"ch("a/b")"#, &map).is_ok());
    assert_eq!(
        compile(r#"out("a/b")"#, &map).unwrap_err().kind,
        ErrorKind::UnknownOutput("a/b".to_owned())
    );
}

#[test]
fn out_runtime_errors_are_slot_errors() {
    let p = prog(r#"out("books/count") + 1"#);
    let e = p.eval(&EvalContext::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::MissingSlot { slot: 4 });
}

#[test]
fn refs_cover_ch_and_out_in_source_order() {
    let src = r#"out("b/x") + (true ? ch("../a/y") : lerp(0, out("/c/z"), 1)) + ch("b/x")"#;
    let refs = channel_refs(src).unwrap();
    let got: Vec<_> = refs.iter().map(|r| (r.kind, r.path.as_str())).collect();
    assert_eq!(
        got,
        [
            (RefKind::Output, "b/x"),
            (RefKind::Input, "../a/y"),
            (RefKind::Output, "/c/z"),
            (RefKind::Input, "b/x"),
        ]
    );
    assert_eq!(&src[refs[0].span.start..refs[0].span.end], "\"b/x\"");
    assert_eq!(&src[refs[2].span.start..refs[2].span.end], "\"/c/z\"");
    // Non-literal arguments are not references.
    assert!(channel_refs(r#"out("a" + "b")"#).unwrap().is_empty());
    assert_eq!(RefKind::Input.function(), "ch");
    assert_eq!(RefKind::Output.function(), "out");
}
