// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Robustness: hostile input never panics the compiler, and well-typed
//! numeric programs never panic the virtual machine.

use std::collections::BTreeMap;

use livspeil_node_api::{PortType, Value};
use livspeil_params::expr::{Channel, EvalContext, compile};
use proptest::prelude::*;

fn env() -> BTreeMap<String, Channel> {
    let mut env = BTreeMap::new();
    for (path, slot, ty) in [
        ("a", 0, PortType::Float),
        ("n", 1, PortType::Int),
        ("v", 2, PortType::Vec3),
        ("s", 3, PortType::String),
    ] {
        env.insert(path.to_owned(), Channel { slot, ty });
    }
    env
}

fn slots() -> Vec<Value> {
    vec![
        Value::Float(-0.75),
        Value::Int(i64::MIN),
        Value::Vec3([1.0, f64::NAN, -3.0]),
        Value::String("ß".to_owned()),
    ]
}

/// Compile `src`; if it compiles, evaluate it under a few contexts. The only
/// requirement is that nothing panics and errors are typed.
fn compile_and_run(src: &str) {
    let env = env();
    let slots = slots();
    if let Ok(p) = compile(src, &env) {
        for (frame, time) in [(0, 0.0), (i64::MAX, f64::MAX), (-1, f64::NAN)] {
            let ctx = EvalContext::new(frame, time, -time, &slots);
            match p.eval(&ctx) {
                Ok(v) => assert_eq!(v.port_type(), p.result_type(), "{src}"),
                Err(e) => assert!(e.kind.is_runtime(), "{src}: {e}"),
            }
        }
        // Too few slots must be an error, not a panic.
        let _ = p.eval(&EvalContext::default());
    }
}

const TOKENS: &[&str] = &[
    "(",
    ")",
    ",",
    ".",
    "?",
    ":",
    "+",
    "-",
    "*",
    "**",
    "/",
    "%",
    "!",
    "&&",
    "||",
    "==",
    "!=",
    "<",
    "<=",
    ">",
    ">=",
    "=",
    "&",
    "|",
    "$",
    "$F",
    "$T",
    "$DT",
    "$Q",
    "0",
    "1",
    "-1",
    "2.5",
    "1e3",
    "1e999",
    "9223372036854775807",
    "99999999999999999999",
    "true",
    "false",
    "pi",
    "x",
    "y",
    "xyzw",
    "lch",
    "\"s\"",
    "\"\\q\"",
    "\"",
    "sin",
    "min",
    "clamp",
    "lerp",
    "vec2",
    "vec3",
    "vec4",
    "oklch",
    "len",
    "upper",
    "concat",
    "str",
    "int",
    "if",
    "ch",
    "ch(\"a\")",
    "ch(\"n\")",
    "ch(\"v\")",
    "ch(\"s\")",
    "ch(\"zz\")",
    "cross",
    "normalize",
    "dot",
    "mod",
    "é",
    "\u{0}",
    " ",
];

fn token_soup() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(TOKENS), 0..48).prop_map(|t| t.join(" "))
}

/// Well-typed expressions of type Float or Int (as source text).
fn numeric() -> impl Strategy<Value = String> {
    let leaf = prop_oneof![
        any::<f64>()
            .prop_filter("finite", |x| x.is_finite())
            .prop_map(|x| format!("({x:?})")),
        any::<i64>().prop_map(|i| if i < 0 {
            format!("(0 - {})", i.unsigned_abs().min(i64::MAX as u64))
        } else {
            i.to_string()
        }),
        (-3i64..4).prop_map(|i| format!("({i})")),
        Just("$T".to_owned()),
        Just("$F".to_owned()),
        Just("$DT".to_owned()),
        Just("ch(\"a\")".to_owned()),
        Just("ch(\"n\")".to_owned()),
        Just("ch(\"v\").y".to_owned()),
        Just("len(ch(\"s\"))".to_owned()),
        Just("pi".to_owned()),
    ];
    leaf.prop_recursive(6, 64, 3, |inner| {
        const UNARY: &[&str] = &[
            "-", "sin", "cos", "tan", "asin", "acos", "atan", "sqrt", "exp", "ln", "log10", "abs",
            "floor", "ceil", "round", "fract", "sign", "int", "float",
        ];
        const BINARY: &[&str] = &["+", "-", "*", "/", "%", "**"];
        const FN2: &[&str] = &["atan2", "pow", "step", "min", "max", "mod"];
        const FN3: &[&str] = &["clamp", "lerp", "smoothstep"];
        prop_oneof![
            (prop::sample::select(UNARY), inner.clone()).prop_map(|(f, a)| if f == "-" {
                format!("-({a})")
            } else {
                format!("{f}({a})")
            }),
            (inner.clone(), prop::sample::select(BINARY), inner.clone())
                .prop_map(|(a, op, b)| format!("({a} {op} {b})")),
            (prop::sample::select(FN2), inner.clone(), inner.clone())
                .prop_map(|(f, a, b)| format!("{f}({a}, {b})")),
            (
                prop::sample::select(FN3),
                inner.clone(),
                inner.clone(),
                inner.clone()
            )
                .prop_map(|(f, a, b, c)| format!("{f}({a}, {b}, {c})")),
            (inner.clone(), inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(a, b, x, y)| format!("({a} < {b} || {x} == {y} ? {x} : {y})")),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(a, b, c)| format!(
                "length(normalize(vec3({a}, {b}, {c}) * 2 + ch(\"v\")))"
            )),
            (inner.clone(), inner.clone(), inner.clone()).prop_map(|(a, b, c)| format!(
                "dot(cross(vec3({a}, {b}, {c}), ch(\"v\")), vec3(1, 2, 3)) + vec2({a}, {b}).yx.y"
            )),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| format!("len(concat({a}, str({b}), upper(ch(\"s\"))))")),
            (inner.clone(), inner).prop_map(|(a, b)| format!(
                "oklch({a}, {b}, 1).h + clamp(vec4({a}, {b}, 1, 2), 0, 1).w"
            )),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn arbitrary_strings_never_panic(src in any::<String>()) {
        compile_and_run(&src);
    }

    #[test]
    fn ascii_noise_never_panics(src in "[ -~]{0,80}") {
        compile_and_run(&src);
    }

    #[test]
    fn token_soup_never_panics(src in token_soup()) {
        compile_and_run(&src);
    }

    #[test]
    fn deep_token_soup_never_panics(
        open in prop::collection::vec(prop::sample::select(&["(", "-", "!", "sin(", "vec2(1,", "1 + ", "2 ** ", "true ? 1 : ", "x."][..]), 0..600),
        close in 0usize..600,
    ) {
        let src = format!("{}1{}", open.concat(), ")".repeat(close));
        compile_and_run(&src);
    }

    #[test]
    fn well_typed_numeric_expressions_compile_and_never_panic(src in numeric()) {
        let env = env();
        let p = compile(&src, &env).map_err(|e| TestCaseError::fail(format!("{src}: {e}")))?;
        prop_assert!(matches!(p.result_type(), PortType::Float | PortType::Int), "{}", src);
        let slots = slots();
        for (frame, time) in [(0, 0.0), (7, 0.5), (i64::MAX, 1e300)] {
            let ctx = EvalContext::new(frame, time, 0.25, &slots);
            match p.eval(&ctx) {
                Ok(v) => {
                    prop_assert_eq!(v.port_type(), p.result_type());
                    // Deterministic: a second run gives bit-identical output.
                    let again = p.eval(&ctx).map_err(|e| TestCaseError::fail(e.to_string()))?;
                    prop_assert_eq!(format!("{v:?}"), format!("{again:?}"));
                }
                Err(e) => prop_assert!(e.kind.is_runtime(), "{}: {}", src, e),
            }
        }
    }
}
