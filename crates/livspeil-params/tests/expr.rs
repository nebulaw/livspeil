// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Unit tests of the expression language through its public API.

use std::collections::BTreeMap;

use livspeil_node_api::{CookContext, PortType, Value};
use livspeil_params::expr::{
    Channel, Env, ErrorKind, EvalContext, ExprError, MAX_DEPTH, MAX_SOURCE_LEN, NoChannels,
    Program, Span, compile, format_value,
};

fn env() -> BTreeMap<String, Channel> {
    let mut env = BTreeMap::new();
    for (path, slot, ty) in [
        ("a", 0, PortType::Float),
        ("n", 1, PortType::Int),
        ("v", 2, PortType::Vec3),
        ("on", 3, PortType::Bool),
        ("name", 4, PortType::String),
        ("col", 5, PortType::Color),
        ("img", 6, PortType::Image),
        ("node1/size", 7, PortType::Vec2),
    ] {
        env.insert(path.to_owned(), Channel { slot, ty });
    }
    env
}

fn slots() -> Vec<Value> {
    vec![
        Value::Float(2.5),
        Value::Int(4),
        Value::Vec3([1.0, 2.0, 3.0]),
        Value::Bool(true),
        Value::String("Lamp".to_owned()),
        Value::Color([0.5, 0.1, 90.0, 1.0]),
        Value::Float(0.0),
        Value::Vec2([3.0, 4.0]),
    ]
}

fn prog(src: &str) -> Program {
    compile(src, &env()).unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn eval(src: &str) -> Value {
    let slots = slots();
    let ctx = EvalContext::new(30, 1.25, 0.5, &slots);
    prog(src)
        .eval(&ctx)
        .unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn f(src: &str) -> f64 {
    match eval(src) {
        Value::Float(x) => x,
        other => panic!("{src}: expected Float, got {other:?}"),
    }
}

fn i(src: &str) -> i64 {
    match eval(src) {
        Value::Int(x) => x,
        other => panic!("{src}: expected Int, got {other:?}"),
    }
}

fn b(src: &str) -> bool {
    match eval(src) {
        Value::Bool(x) => x,
        other => panic!("{src}: expected Bool, got {other:?}"),
    }
}

fn s(src: &str) -> String {
    match eval(src) {
        Value::String(x) => x,
        other => panic!("{src}: expected String, got {other:?}"),
    }
}

fn cerr(src: &str) -> ExprError {
    match compile(src, &env()) {
        Ok(p) => panic!("{src}: expected a compile error, got {p:?}"),
        Err(e) => e,
    }
}

fn rerr(src: &str) -> ExprError {
    let slots = slots();
    let ctx = EvalContext::new(30, 1.25, 0.5, &slots);
    match prog(src).eval(&ctx) {
        Ok(v) => panic!("{src}: expected a runtime error, got {v:?}"),
        Err(e) => e,
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

// ---- precedence and associativity -------------------------------------------

#[test]
fn arithmetic_precedence() {
    assert_eq!(i("1 + 2 * 3"), 7);
    assert_eq!(i("(1 + 2) * 3"), 9);
    assert_eq!(i("2 * 3 + 4 * 5"), 26);
    assert_eq!(i("10 - 2 - 3"), 5); // left-assoc
    assert_eq!(i("100 / 10 / 5"), 2); // left-assoc
    assert_eq!(i("17 % 5 * 2"), 4);
    assert_eq!(i("-3 * 2"), -6);
    assert_eq!(i("--3"), 3);
    assert_eq!(i("2 - -3"), 5);
}

#[test]
fn power_is_right_associative_and_binds_tighter_than_unary_minus() {
    assert_eq!(f("2 ** 3 ** 2"), 512.0);
    assert_eq!(f("-2 ** 2"), -4.0);
    assert_eq!(f("(-2) ** 2"), 4.0);
    assert_eq!(f("2 ** -2"), 0.25);
    assert_eq!(f("2 * 3 ** 2"), 18.0);
}

#[test]
fn logic_and_comparison_precedence() {
    assert!(b("1 + 1 == 2"));
    assert!(b("1 < 2 == 2 < 3"));
    assert!(b("true || false && false")); // && binds tighter
    assert!(!b("(true || false) && false"));
    assert!(b("!false && !(1 > 2)"));
    assert!(b("1 != 2"));
    assert!(b("3 >= 3 && 3 <= 3 && !(3 > 3) && !(3 < 3)"));
}

#[test]
fn conditional_is_right_associative_and_lowest() {
    assert_eq!(i("true ? 1 : 2 + 10"), 1);
    assert_eq!(i("false ? 1 : 2 + 10"), 12);
    assert_eq!(i("false ? 1 : true ? 2 : 3"), 2);
    assert_eq!(i("1 > 2 || 2 > 1 ? 5 : 6"), 5);
    assert_eq!(i("if(1 > 2, 1, if(true, 2, 3))"), 2);
    // Mixed Int/Float branches widen to Float, from either side.
    assert_eq!(f("true ? 1 : 2.5"), 1.0);
    assert_eq!(f("false ? 1.5 : 2"), 2.0);
    assert_eq!(f("if(false, 1, 2.5)"), 2.5);
}

#[test]
fn only_the_chosen_branch_runs() {
    assert_eq!(i("true ? 1 : 1 / 0"), 1);
    assert_eq!(i("if(false, 1 / 0, 2)"), 2);
    assert!(!b("false && 1 / 0 == 1"));
    assert!(b("true || 1 / 0 == 1"));
    assert_eq!(rerr("true && 1 / 0 == 1").kind, ErrorKind::DivisionByZero);
}

// ---- numbers -----------------------------------------------------------------

#[test]
fn literals() {
    assert_eq!(eval("1"), Value::Int(1));
    assert_eq!(eval("1.5"), Value::Float(1.5));
    assert_eq!(eval("1e3"), Value::Float(1000.0));
    assert_eq!(eval("2.5E-1"), Value::Float(0.25));
    assert_eq!(eval("true"), Value::Bool(true));
    assert_eq!(eval("\"text\""), Value::String("text".to_owned()));
    assert_eq!(
        eval("\"a\\\"b\\\\c\\nd\\te\""),
        Value::String("a\"b\\c\nd\te".to_owned())
    );
    assert!(close(f("pi"), std::f64::consts::PI));
    assert!(close(f("tau"), std::f64::consts::TAU));
}

#[test]
fn int_widens_to_float_but_never_back() {
    assert_eq!(eval("1 + 0.5"), Value::Float(1.5));
    assert_eq!(eval("0.5 + 1"), Value::Float(1.5));
    assert_eq!(eval("float(3)"), Value::Float(3.0));
    assert_eq!(eval("int(3.9)"), Value::Int(3));
    assert_eq!(eval("int(-3.9)"), Value::Int(-3));
    assert_eq!(eval("int(sqrt(-1))"), Value::Int(0));
    assert_eq!(eval("int(7)"), Value::Int(7));
    assert_eq!(prog("sqrt(4)").result_type(), PortType::Float);
}

#[test]
fn integer_semantics_are_checked_and_floored() {
    assert_eq!(i("7 / 2"), 3);
    assert_eq!(i("-7 / 2"), -4);
    assert_eq!(i("7 / -2"), -4);
    assert_eq!(i("-7 % 2"), 1);
    assert_eq!(i("7 % -2"), -1);
    assert_eq!(i("mod(-7, 2)"), 1);
    for (a, d) in [(7, 2), (-7, 2), (7, -2), (-7, -2), (6, 3), (-6, 3)] {
        assert_eq!(i(&format!("({a} / {d}) * {d} + {a} % {d}")), a);
    }
    assert_eq!(rerr("1 / 0").kind, ErrorKind::DivisionByZero);
    assert_eq!(rerr("1 % 0").kind, ErrorKind::DivisionByZero);
    assert_eq!(rerr("mod(1, 0)").kind, ErrorKind::DivisionByZero);
    assert_eq!(
        rerr("9223372036854775807 + 1").kind,
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        rerr("-9223372036854775807 - 2").kind,
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        rerr("4611686018427387904 * 2").kind,
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        rerr("(-9223372036854775807 - 1) / -1").kind,
        ErrorKind::IntegerOverflow
    );
    assert_eq!(
        rerr("-(-9223372036854775807 - 1)").kind,
        ErrorKind::IntegerOverflow
    );
}

#[test]
fn runtime_errors_point_at_the_operation() {
    let e = rerr("2 + 10 / (3 - 3)");
    assert_eq!(e.kind, ErrorKind::DivisionByZero);
    assert_eq!(e.span, Span::new(4, 16));
    assert!(e.kind.is_runtime());
}

#[test]
fn float_math_follows_ieee() {
    assert_eq!(f("1.0 / 0"), f64::INFINITY);
    assert_eq!(f("-1 / 0.0"), f64::NEG_INFINITY);
    assert!(f("0.0 / 0").is_nan());
    assert!(f("sqrt(-1)").is_nan());
    assert!(!b("sqrt(-1) == sqrt(-1)"));
    assert_eq!(f("-7.5 % 2"), 0.5);
    assert_eq!(f("7.5 % -2"), -0.5);
}

// ---- every function ------------------------------------------------------------

#[test]
fn trigonometry() {
    assert_eq!(f("sin(0)"), 0.0);
    assert!(close(f("sin(pi / 2)"), 1.0));
    assert!(close(f("cos(pi)"), -1.0));
    assert!(close(f("tan(pi / 4)"), 1.0));
    assert!(close(f("asin(1)"), std::f64::consts::FRAC_PI_2));
    assert!(close(f("acos(1)"), 0.0));
    assert!(close(f("atan(1)"), std::f64::consts::FRAC_PI_4));
    assert!(close(f("atan2(1, -1)"), 3.0 * std::f64::consts::FRAC_PI_4));
    assert!(f("asin(2)").is_nan());
}

#[test]
fn exponentials() {
    assert_eq!(f("sqrt(16)"), 4.0);
    assert_eq!(f("pow(2, 10)"), 1024.0);
    assert_eq!(f("pow(4, 0.5)"), 2.0);
    assert_eq!(f("exp(0)"), 1.0);
    assert!(close(f("ln(exp(2))"), 2.0));
    assert_eq!(f("log10(1000)"), 3.0);
    assert_eq!(f("ln(0)"), f64::NEG_INFINITY);
}

#[test]
fn rounding_and_sign() {
    assert_eq!(eval("abs(-3)"), Value::Int(3));
    assert_eq!(eval("abs(-2.5)"), Value::Float(2.5));
    assert_eq!(
        rerr("abs(-9223372036854775807 - 1)").kind,
        ErrorKind::IntegerOverflow
    );
    assert_eq!(f("floor(-1.5)"), -2.0);
    assert_eq!(f("ceil(-1.5)"), -1.0);
    assert_eq!(f("floor(3)"), 3.0);
    assert_eq!(f("round(2.5)"), 3.0);
    assert_eq!(f("round(-2.5)"), -3.0);
    assert_eq!(f("round(2.4)"), 2.0);
    assert_eq!(f("fract(2.75)"), 0.75);
    assert_eq!(f("fract(-0.25)"), 0.75);
    assert_eq!(eval("sign(-5)"), Value::Int(-1));
    assert_eq!(eval("sign(0)"), Value::Int(0));
    assert_eq!(f("sign(2.5)"), 1.0);
    assert_eq!(f("sign(-0.1)"), -1.0);
    assert_eq!(f("sign(0.0)"), 0.0);
}

#[test]
fn min_max_clamp_mod() {
    assert_eq!(eval("min(3, 2)"), Value::Int(2));
    assert_eq!(eval("max(3, 2)"), Value::Int(3));
    assert_eq!(eval("min(3, 2.5)"), Value::Float(2.5));
    assert_eq!(eval("max(1, 2.5)"), Value::Float(2.5));
    assert_eq!(eval("clamp(15, 0, 10)"), Value::Int(10));
    assert_eq!(eval("clamp(-1, 0, 10)"), Value::Int(0));
    assert_eq!(eval("clamp(0.5, 0, 1)"), Value::Float(0.5));
    assert_eq!(eval("clamp(1.5, 0, 1)"), Value::Float(1.0));
    // Inverted bounds never panic: min(max(x, lo), hi).
    assert_eq!(eval("clamp(5, 10, 0)"), Value::Int(0));
    assert_eq!(eval("clamp(5.0, 10, 0)"), Value::Float(0.0));
    assert_eq!(eval("mod(7, 3)"), Value::Int(1));
    assert_eq!(eval("mod(-1, 3)"), Value::Int(2));
    assert_eq!(eval("mod(5.5, 2)"), Value::Float(1.5));
    assert_eq!(eval("mod(-0.5, 2)"), Value::Float(1.5));
    assert!(f("mod(1.0, 0)").is_nan());
}

#[test]
fn interpolation() {
    assert_eq!(f("lerp(10, 20, 0.5)"), 15.0);
    assert_eq!(f("lerp(10, 20, 2)"), 30.0);
    assert_eq!(f("smoothstep(0, 1, -1)"), 0.0);
    assert_eq!(f("smoothstep(0, 1, 2)"), 1.0);
    assert_eq!(f("smoothstep(0, 1, 0.25)"), 0.15625);
    assert_eq!(f("smoothstep(1, 1, 0.5)"), 0.0);
    assert_eq!(f("smoothstep(1, 1, 1.5)"), 1.0);
    assert_eq!(f("step(0.5, 0.4)"), 0.0);
    assert_eq!(f("step(0.5, 0.5)"), 1.0);
}

#[test]
fn vector_functions() {
    assert_eq!(f("length(vec3(2, 3, 6))"), 7.0);
    assert_eq!(f("dot(vec2(1, 2), vec2(3, 4))"), 11.0);
    assert_eq!(
        eval("cross(vec3(0, 1, 0), vec3(0, 0, 1))"),
        Value::Vec3([1.0, 0.0, 0.0])
    );
    assert_eq!(eval("normalize(vec2(3, 4))"), Value::Vec2([0.6, 0.8]));
    assert_eq!(eval("normalize(vec4(0, 0, 0, 0))"), Value::Vec4([0.0; 4]));
    assert_eq!(eval("abs(vec3(-1, 2, -3))"), Value::Vec3([1.0, 2.0, 3.0]));
    assert_eq!(eval("floor(vec2(1.5, -1.5))"), Value::Vec2([1.0, -2.0]));
    assert_eq!(eval("min(vec2(1, 5), vec2(3, 2))"), Value::Vec2([1.0, 2.0]));
    assert_eq!(eval("max(vec2(1, 5), vec2(3, 2))"), Value::Vec2([3.0, 5.0]));
    assert_eq!(eval("clamp(vec2(-1, 5), 0, 1)"), Value::Vec2([0.0, 1.0]));
    assert_eq!(
        eval("lerp(vec2(0, 10), vec2(10, 20), 0.5)"),
        Value::Vec2([5.0, 15.0])
    );
    assert_eq!(eval("sin(vec2(0, 0))"), Value::Vec2([0.0, 0.0]));
}

#[test]
fn string_functions() {
    assert_eq!(s("\"ab\" + \"cd\""), "abcd");
    assert_eq!(s("concat(\"x\")"), "x");
    assert_eq!(
        s("concat(\"n=\", 3, \", f=\", 1.5, \", b=\", false)"),
        "n=3, f=1.5, b=false"
    );
    assert_eq!(i("len(\"\")"), 0);
    assert_eq!(i("len(\"日本\")"), 2);
    assert_eq!(s("upper(\"abc\")"), "ABC");
    assert_eq!(s("lower(\"AbC\")"), "abc");
    assert_eq!(s("str(1)"), "1");
    assert_eq!(s("str(1.0)"), "1.0");
    assert_eq!(s("str(0.1)"), "0.1");
    assert_eq!(s("str(1e100)"), "1e100");
    assert_eq!(s("str(-0.0)"), "-0.0");
    assert_eq!(s("str(1.0 / 0)"), "inf");
    assert_eq!(s("str(sqrt(-1))"), "NaN");
    assert_eq!(s("str(true)"), "true");
    assert_eq!(s("str(\"x\")"), "x");
    assert_eq!(s("str(vec3(1, 2.5, -3))"), "vec3(1.0, 2.5, -3.0)");
    assert_eq!(s("str(oklch(0.5, 0.1, 90))"), "oklch(0.5, 0.1, 90.0, 1.0)");
    assert!(b("\"a\" == \"a\" && \"a\" != \"b\""));
    assert_eq!(s("true ? \"yes\" : \"no\""), "yes");
}

#[test]
fn format_value_is_deterministic() {
    assert_eq!(format_value(&Value::Float(2.0)), "2.0");
    assert_eq!(format_value(&Value::Int(-2)), "-2");
    assert_eq!(format_value(&Value::Vec2([0.5, 1e-7])), "vec2(0.5, 1e-7)");
}

// ---- vectors and colors ------------------------------------------------------------

#[test]
fn vector_arithmetic() {
    assert_eq!(eval("vec2(1, 2) + vec2(3, 4)"), Value::Vec2([4.0, 6.0]));
    assert_eq!(
        eval("vec3(1, 2, 3) - vec3(1, 1, 1)"),
        Value::Vec3([0.0, 1.0, 2.0])
    );
    assert_eq!(eval("vec2(2, 3) * vec2(4, 5)"), Value::Vec2([8.0, 15.0]));
    assert_eq!(eval("vec2(8, 9) / vec2(2, 3)"), Value::Vec2([4.0, 3.0]));
    assert_eq!(
        eval("vec4(1, 2, 3, 4) * 2"),
        Value::Vec4([2.0, 4.0, 6.0, 8.0])
    );
    assert_eq!(eval("2 * vec2(1, 2)"), Value::Vec2([2.0, 4.0]));
    assert_eq!(eval("vec2(2, 4) / 2"), Value::Vec2([1.0, 2.0]));
    assert_eq!(eval("-vec2(1, -2)"), Value::Vec2([-1.0, 2.0]));
    assert!(b("vec2(1, 2) == vec2(1, 2) && vec2(1, 2) != vec2(2, 1)"));
}

#[test]
fn components_and_swizzles() {
    assert_eq!(f("vec4(1, 2, 3, 4).x"), 1.0);
    assert_eq!(f("vec4(1, 2, 3, 4).w"), 4.0);
    assert_eq!(eval("vec3(1, 2, 3).zyx"), Value::Vec3([3.0, 2.0, 1.0]));
    assert_eq!(eval("vec2(1, 2).yyyy"), Value::Vec4([2.0; 4]));
    assert_eq!(eval("vec3(1, 2, 3).xy * 2"), Value::Vec2([2.0, 4.0]));
    assert_eq!(f("(vec2(1, 2) + vec2(3, 4)).y"), 6.0);
    assert_eq!(f("oklch(0.7, 0.2, 120, 0.5).a"), 0.5);
    assert!(close(
        f("oklch(0.7, 0.2, 120).l + oklch(0.7, 0.2, 120).c"),
        0.9
    ));
}

#[test]
fn colors() {
    assert_eq!(
        eval("oklch(0.7, 0.1, 200)"),
        Value::Color([0.7, 0.1, 200.0, 1.0])
    );
    assert_eq!(
        eval("oklch(1, 0, 0, 0.25)"),
        Value::Color([1.0, 0.0, 0.0, 0.25])
    );
    assert!(b("oklch(1, 0, 0) == oklch(1.0, 0, 0, 1)"));
    assert_eq!(prog("oklch(1, 0, 0)").result_type(), PortType::Color);
}

// ---- time variables ---------------------------------------------------------------

#[test]
fn time_variables() {
    assert_eq!(eval("$F"), Value::Int(30));
    assert_eq!(eval("$T"), Value::Float(1.25));
    assert_eq!(eval("$DT"), Value::Float(0.5));
    assert_eq!(eval("$F / 2"), Value::Int(15));
    assert_eq!(eval("$F * $DT"), Value::Float(15.0));
    for src in ["$F", "$T + 1", "if(true, 1.0, $DT)", "str($F)"] {
        assert!(prog(src).is_time_dependent(), "{src}");
    }
    for src in ["1", "ch(\"a\")", "sin(pi)"] {
        assert!(!prog(src).is_time_dependent(), "{src}");
    }
}

#[test]
fn from_cook_context() {
    let cook = CookContext {
        frame: u64::MAX,
        time: 2.0,
        dt: 0.25,
    };
    let ctx = EvalContext::from_cook(&cook, &[]);
    assert_eq!(ctx.frame, i64::MAX);
    let p = compile("$T + $DT", &NoChannels).unwrap();
    assert_eq!(p.eval(&ctx).unwrap(), Value::Float(2.25));
}

// ---- ch() -------------------------------------------------------------------------

#[test]
fn ch_reads_slots_with_static_types() {
    assert_eq!(eval("ch(\"a\") * 2"), Value::Float(5.0));
    assert_eq!(eval("ch(\"n\") + 1"), Value::Int(5));
    assert_eq!(eval("ch(\"v\").y"), Value::Float(2.0));
    assert!(b("ch(\"on\")"));
    assert_eq!(s("upper(ch(\"name\"))"), "LAMP");
    assert_eq!(f("ch(\"col\").h"), 90.0);
    assert_eq!(f("length(ch(\"node1/size\"))"), 5.0);
    assert_eq!(prog("ch(\"v\")").result_type(), PortType::Vec3);
}

#[test]
fn dependencies_are_sorted_unique_and_conservative() {
    let p = prog("ch(\"n\") + ch(\"a\") + ch(\"n\")");
    assert_eq!(p.dependencies(), &[0, 1]);
    // Both branches count, even if only one runs.
    let p = prog("ch(\"on\") ? ch(\"v\").x : ch(\"a\")");
    assert_eq!(p.dependencies(), &[0, 2, 3]);
    assert!(prog("1 + 2").dependencies().is_empty());
}

#[test]
fn ch_errors() {
    let e = cerr("ch(\"missing\") + 1");
    assert_eq!(e.kind, ErrorKind::UnknownChannel("missing".to_owned()));
    assert_eq!(e.span, Span::new(3, 12));
    let e = cerr("ch(\"a\" + \"b\")");
    assert_eq!(e.kind, ErrorKind::ChannelPathNotLiteral);
    assert_eq!(e.span, Span::new(3, 12));
    assert!(matches!(
        cerr("ch()").kind,
        ErrorKind::Arity { found: 0, .. }
    ));
    assert!(matches!(
        cerr("ch(\"img\")").kind,
        ErrorKind::UnsupportedChannelType {
            ty: PortType::Image,
            ..
        }
    ));
    assert!(matches!(
        compile("ch(\"a\")", &NoChannels).unwrap_err().kind,
        ErrorKind::UnknownChannel(_)
    ));
}

#[test]
fn ch_runtime_errors() {
    let p = prog("ch(\"n\") + 1");
    let e = p.eval(&EvalContext::default()).unwrap_err();
    assert_eq!(e.kind, ErrorKind::MissingSlot { slot: 1 });
    let bad = [Value::Float(0.0), Value::String("x".to_owned())];
    let e = p.eval(&EvalContext::new(0, 0.0, 0.0, &bad)).unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::SlotTypeMismatch {
            slot: 1,
            expected: PortType::Int,
            found: PortType::String
        }
    );
    // An Int value in a Float slot widens.
    let p = prog("ch(\"a\")");
    let ints = [Value::Int(3)];
    assert_eq!(
        p.eval(&EvalContext::new(0, 0.0, 0.0, &ints)).unwrap(),
        Value::Float(3.0)
    );
}

#[test]
fn hash_map_env_works() {
    let mut env = std::collections::HashMap::new();
    env.insert(
        "x".to_owned(),
        Channel {
            slot: 0,
            ty: PortType::Float,
        },
    );
    let p = compile("ch(\"x\") + 1", &env).unwrap();
    let slots = [Value::Float(1.0)];
    assert_eq!(
        p.eval(&EvalContext::new(0, 0.0, 0.0, &slots)).unwrap(),
        Value::Float(2.0)
    );
    let _: &dyn Env = &env;
}

// ---- compile errors with spans -------------------------------------------------------

fn assert_mismatch(src: &str, expected: &str, found: PortType, span: (usize, usize)) {
    let e = cerr(src);
    match &e.kind {
        ErrorKind::TypeMismatch {
            expected: exp,
            found: fnd,
            ..
        } => {
            assert_eq!(exp, expected, "{src}");
            assert_eq!(*fnd, found, "{src}");
        }
        other => panic!("{src}: expected a type mismatch, got {other:?}"),
    }
    assert_eq!(e.span, Span::new(span.0, span.1), "{src}");
}

#[test]
fn type_errors_have_spans() {
    assert_mismatch("1 + true", "Int or Float", PortType::Bool, (4, 8));
    assert_mismatch("1 * true", "Int, Float or vector", PortType::Bool, (4, 8));
    assert_mismatch(
        "true + 1",
        "Int, Float, vector or String",
        PortType::Bool,
        (0, 4),
    );
    assert_mismatch(
        "\"a\" + 1",
        "String (use str() to convert)",
        PortType::Int,
        (6, 7),
    );
    assert_mismatch(
        "vec2(1, 2) + vec3(1, 2, 3)",
        "Vec2",
        PortType::Vec3,
        (13, 26),
    );
    assert_mismatch("vec2(1, 2) + 1", "Vec2", PortType::Int, (13, 14));
    assert_mismatch("1 / vec2(1, 2)", "Int or Float", PortType::Vec2, (4, 14));
    assert_mismatch(
        "sin(\"x\")",
        "Int, Float or vector",
        PortType::String,
        (4, 7),
    );
    assert_mismatch("pow(2, true)", "Int or Float", PortType::Bool, (7, 11));
    assert_mismatch("1 < \"a\"", "Int or Float", PortType::String, (4, 7));
    assert_mismatch("1 == true", "Int or Float", PortType::Bool, (5, 9));
    assert_mismatch("1 && true", "Bool", PortType::Int, (0, 1));
    assert_mismatch("!1", "Bool", PortType::Int, (1, 2));
    assert_mismatch("-\"a\"", "Int, Float or vector", PortType::String, (1, 4));
    assert_mismatch("if(1, 2, 3)", "Bool", PortType::Int, (3, 4));
    assert_mismatch(
        "true ? 1 : \"a\"",
        "Int or Float",
        PortType::String,
        (11, 14),
    );
    assert_mismatch(
        "cross(vec2(1, 2), vec3(1, 2, 3))",
        "Vec3",
        PortType::Vec2,
        (6, 16),
    );
    assert_mismatch(
        "dot(vec2(1, 2), vec3(1, 2, 3))",
        "Vec2",
        PortType::Vec3,
        (16, 29),
    );
    assert_mismatch("length(1.0)", "a vector", PortType::Float, (7, 10));
    assert_mismatch("len(3)", "String", PortType::Int, (4, 5));
    assert_mismatch("int(true)", "Int or Float", PortType::Bool, (4, 8));
    assert_mismatch("vec2(1, \"y\")", "Int or Float", PortType::String, (8, 11));
    assert_mismatch(
        "oklch(1, 2, vec2(1, 2))",
        "Int or Float",
        PortType::Vec2,
        (12, 22),
    );
    assert_mismatch("lerp(vec2(1, 2), 1, 0.5)", "Vec2", PortType::Int, (17, 18));
    assert_mismatch(
        "oklch(1, 0, 0) + oklch(1, 0, 0)",
        "Int, Float, vector or String",
        PortType::Color,
        (0, 14),
    );
}

#[test]
fn name_and_arity_errors() {
    let e = cerr("1 + frobnicate(2)");
    assert_eq!(e.kind, ErrorKind::UnknownFunction("frobnicate".to_owned()));
    assert_eq!(e.span, Span::new(4, 14));
    let e = cerr("clamp(1, 2)");
    assert_eq!(
        e.kind,
        ErrorKind::Arity {
            function: "clamp".to_owned(),
            expected: "3",
            found: 2
        }
    );
    assert_eq!(e.span, Span::new(0, 11));
    assert_eq!(e.to_string(), "'clamp' takes 3 arguments, found 2 at 0..11");
    assert!(matches!(
        cerr("oklch(1, 2)").kind,
        ErrorKind::Arity {
            expected: "3 or 4",
            ..
        }
    ));
    assert!(matches!(
        cerr("concat()").kind,
        ErrorKind::Arity {
            expected: "at least 1",
            ..
        }
    ));
    assert_eq!(
        cerr("x + 1").kind,
        ErrorKind::UnknownIdentifier("x".to_owned())
    );
    let e = cerr("$F + $FRAME");
    assert_eq!(e.kind, ErrorKind::UnknownVariable("FRAME".to_owned()));
    assert_eq!(e.span, Span::new(5, 11));
    let e = cerr("vec3(1, 2, 3).q");
    assert_eq!(
        e.kind,
        ErrorKind::UnknownComponent {
            component: "q".to_owned(),
            ty: PortType::Vec3
        }
    );
    assert_eq!(e.span, Span::new(14, 15));
    assert!(matches!(
        cerr("vec2(1, 2).xyzxy").kind,
        ErrorKind::UnknownComponent { .. }
    ));
    assert!(matches!(
        cerr("oklch(1, 0, 0).x").kind,
        ErrorKind::UnknownComponent { .. }
    ));
    assert!(matches!(
        cerr("oklch(1, 0, 0).lc").kind,
        ErrorKind::UnknownComponent { .. }
    ));
    assert!(matches!(
        cerr("(1).x").kind,
        ErrorKind::UnknownComponent { .. }
    ));
}

#[test]
fn parse_errors_have_spans() {
    let cases: &[(&str, (usize, usize))] = &[
        ("", (0, 0)),
        ("1 +", (3, 3)),
        ("(1 + 2", (6, 6)),
        ("1 2", (2, 3)),
        ("sin(1,", (6, 6)),
        ("sin(1 2)", (6, 7)),
        ("true ? 1", (8, 8)),
        ("vec2(1, 2).", (11, 11)),
        ("vec2(1, 2).1", (11, 12)),
        (")", (0, 1)),
    ];
    for (src, (start, end)) in cases {
        let e = cerr(src);
        assert!(matches!(e.kind, ErrorKind::Expected { .. }), "{src}: {e}");
        assert_eq!(e.span, Span::new(*start, *end), "{src}: {e}");
    }
    assert_eq!(cerr("1 # 2").kind, ErrorKind::UnexpectedChar('#'));
    assert_eq!(cerr("1 # 2").span, Span::new(2, 3));
    assert_eq!(cerr("1 + é").span, Span::new(4, 6));
    assert_eq!(cerr("1 & 2").kind, ErrorKind::UnexpectedChar('&'));
    assert_eq!(cerr("$").kind, ErrorKind::UnexpectedChar('$'));
    assert_eq!(cerr("\"abc").kind, ErrorKind::UnterminatedString);
    assert_eq!(cerr("\"a\\qb\"").kind, ErrorKind::InvalidEscape('q'));
    assert_eq!(cerr("\"a\\qb\"").span, Span::new(2, 4));
    assert_eq!(
        cerr("99999999999999999999").kind,
        ErrorKind::InvalidNumber("integer literal out of range")
    );
    assert_eq!(
        cerr("1e999").kind,
        ErrorKind::InvalidNumber("float literal out of range")
    );
    let e = cerr("1 = 2");
    assert_eq!(e.span, Span::new(2, 3));
}

#[test]
fn nesting_is_bounded() {
    let ok = format!(
        "{}1{}",
        "(".repeat(MAX_DEPTH - 1),
        ")".repeat(MAX_DEPTH - 1)
    );
    assert_eq!(eval(&ok), Value::Int(1));
    for deep in [
        format!(
            "{}1{}",
            "(".repeat(MAX_DEPTH + 1),
            ")".repeat(MAX_DEPTH + 1)
        ),
        format!("{}1", "-".repeat(10_000)),
        format!("{}1", "!".repeat(10_000)),
        format!("{}1{}", "sin(".repeat(10_000), ")".repeat(10_000)),
        "1 + ".repeat(10_000) + "1",
        "2 ** ".repeat(10_000) + "1",
        "true ? 1 : ".repeat(5_000) + "1",
        format!("{}{}", "vec4(1, 2, 3, 4)", ".x".repeat(10_000)),
    ] {
        let e = compile(&deep, &NoChannels).unwrap_err();
        assert_eq!(
            e.kind,
            ErrorKind::TooDeep { limit: MAX_DEPTH },
            "{}",
            &deep[..20]
        );
    }
    let long = "1".repeat(MAX_SOURCE_LEN + 1);
    assert_eq!(
        compile(&long, &NoChannels).unwrap_err().kind,
        ErrorKind::SourceTooLong {
            limit: MAX_SOURCE_LEN
        }
    );
}

#[test]
fn wide_and_deep_programs_evaluate() {
    // Deep right-nested sums need more stack cells than the inline buffer.
    // Each level costs two parser levels (the parenthesis and the operand).
    let src = format!("{}1{}", "1 + (".repeat(120), ")".repeat(120));
    assert_eq!(eval(&src), Value::Int(121));
    let src = format!("concat({})", vec!["\"ab\""; 1000].join(", "));
    assert_eq!(s(&src).len(), 2000);
}

#[test]
fn errors_display_and_implement_error() {
    let e = cerr("sin(true)");
    let msg = e.to_string();
    assert_eq!(
        msg,
        "type mismatch in argument 1 of 'sin': expected Int, Float or vector, found Bool at 4..8"
    );
    let _: &dyn std::error::Error = &e;
    assert!(!e.kind.is_runtime());
}

#[test]
fn programs_are_reusable_and_compact() {
    let p = prog("sin($T) * ch(\"a\")");
    let slots = slots();
    for frame in 0..10 {
        let t = f64::from(frame) * 0.1;
        let ctx = EvalContext::new(i64::from(frame), t, 0.1, &slots);
        let Value::Float(got) = p.eval(&ctx).unwrap() else {
            panic!("not a float");
        };
        assert!(close(got, t.sin() * 2.5), "{got}");
    }
    assert!(p.len() <= 5, "{}", p.len());
    assert!(!p.is_empty());
}
