// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden test: compile and evaluate a list of expressions.
//!
//! Refresh with `UPDATE_GOLDEN=1 cargo test -p livspeil-params --test expr_golden`
//! and review every line of the diff by hand.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use livspeil_node_api::{PortType, Value};
use livspeil_params::expr::{Channel, Env, EvalContext, compile, format_value};
use livspeil_testkit::assert_golden;

const CASES: &[&str] = &[
    // literals and precedence
    "1 + 2 * 3",
    "(1 + 2) * 3",
    "10 - 4 - 3",
    "2 ** 3 ** 2",
    "-2 ** 2",
    "2 ** -1",
    "1.5e3 + 1",
    "7 / 2",
    "-7 / 2",
    "-7 % 2",
    "7 / 2.0",
    "-7.5 % 2",
    "1 / 0",
    "1.0 / 0",
    "9223372036854775807 + 1",
    // comparison, logic, conditionals
    "1 < 2 && 2.5 >= 3 || !false",
    "1 == 1.0",
    "true ? 1 : 2.5",
    "if($F > 10, \"late\", \"early\")",
    "false && 1 / 0 == 0",
    "1 < 2 ? 3 < 4 ? \"a\" : \"b\" : \"c\"",
    // math
    "sin(pi / 2)",
    "atan2(1, 1) * 4",
    "sqrt(-1)",
    "round(2.5) + round(-2.5)",
    "fract(-1.25)",
    "clamp(15, 0, 10)",
    "lerp(0, 10, 0.25)",
    "smoothstep(0, 1, 0.5)",
    "mod(-1, 3) + mod(5.5, -2)",
    "min(3, 2) + max(1.5, 2)",
    "int(-3.9) + int(1e300)",
    // vectors and colors
    "vec3(1, 2, 3) + vec3(1, 1, 1) * 2",
    "2 * vec2(1.5, -1) / 4",
    "length(vec2(3, 4))",
    "normalize(vec3(0, 0, 0))",
    "cross(vec3(1, 0, 0), vec3(0, 1, 0))",
    "dot(vec4(1, 2, 3, 4), vec4(1, 1, 1, 1))",
    "vec4(1, 2, 3, 4).wzy",
    "oklch(0.7, 0.1, 200).h",
    "oklch(0.5, 0.2, 30, 0.5)",
    "abs(vec2(-1, 2))",
    // strings
    "\"frame \" + str($F)",
    "concat(\"t=\", $T, \" v=\", vec2(1, 2), \" \", true)",
    "upper(\"straße\") + lower(\"ÅB\")",
    "len(\"héllo\")",
    "str(0.1 + 0.2)",
    // time and ch()
    "$T * 2 + $DT",
    "ch(\"radius\") * 2",
    "ch(\"center\").x + ch(\"count\")",
    "ch(\"tint\").l",
    "ch(\"label\") == \"sun\"",
    // errors
    "1 +",
    "foo(1)",
    "sin(1, 2)",
    "\"a\" + 1",
    "vec2(1, 2) + vec3(1, 2, 3)",
    "ch(\"nope\")",
    "$X",
    "1 ? 2 : 3",
    "vec2(1, 2).z",
    "\"open",
    "1 = 2",
    "abs(-9223372036854775807 - 1)",
    // out(): outputs of other nodes (D48)
    "smoothstep(5, 8, out(\"time_of_day/hour\"))",
    "max(0, out(\"sun/elevation\") / 90)",
    "ceil(out(\"books/count\") / 12.0)",
    "out(\"books/count\") / 12",
    "ch(\"radius\") * out(\"sun/elevation\")",
    "out(\"nope/x\")",
    "out(\"radius\")",
    "out(\"db/rows\")",
    "out(1)",
];

/// Parameters for `ch()` and outputs for `out()`, in one slot space.
struct Golden {
    inputs: BTreeMap<String, Channel>,
    outputs: BTreeMap<String, Channel>,
}

impl Env for Golden {
    fn resolve(&self, path: &str) -> Option<Channel> {
        self.inputs.get(path).copied()
    }

    fn resolve_output(&self, path: &str) -> Option<Channel> {
        self.outputs.get(path).copied()
    }
}

fn env() -> Golden {
    let mut inputs = BTreeMap::new();
    let mut add = |path: &str, slot, ty| {
        inputs.insert(path.to_owned(), Channel { slot, ty });
    };
    add("radius", 0, PortType::Float);
    add("center", 1, PortType::Vec2);
    add("count", 2, PortType::Int);
    add("tint", 3, PortType::Color);
    add("label", 4, PortType::String);
    let mut outputs = BTreeMap::new();
    let mut add = |path: &str, slot, ty| {
        outputs.insert(path.to_owned(), Channel { slot, ty });
    };
    add("time_of_day/hour", 5, PortType::Float);
    add("sun/elevation", 6, PortType::Float);
    add("books/count", 7, PortType::Int);
    add("db/rows", 8, PortType::Table);
    Golden { inputs, outputs }
}

fn render(value: &Value) -> String {
    match value {
        Value::String(s) => format!("{s:?}"),
        v => format_value(v),
    }
}

#[test]
fn expressions() {
    let env = env();
    let slots = [
        Value::Float(1.25),
        Value::Vec2([0.5, -2.0]),
        Value::Int(3),
        Value::Color([0.62, 0.15, 250.0, 1.0]),
        Value::String("sun".to_owned()),
        Value::Float(6.5),
        Value::Float(45.0),
        Value::Int(25),
    ];
    let ctx = EvalContext::new(12, 0.5, 1.0 / 60.0, &slots);
    let mut out = String::new();
    for src in CASES {
        let line = match compile(src, &env) {
            Err(e) => format!("compile error: {e}"),
            Ok(p) => match p.eval(&ctx) {
                Ok(v) => {
                    assert_eq!(v.port_type(), p.result_type(), "{src}");
                    let mut info = String::new();
                    if p.is_time_dependent() {
                        info.push_str(" [time]");
                    }
                    if !p.dependencies().is_empty() {
                        let _ = write!(info, " [deps {:?}]", p.dependencies());
                    }
                    if !p.output_dependencies().is_empty() {
                        let _ = write!(info, " [out deps {:?}]", p.output_dependencies());
                    }
                    format!("{} : {}{info}", render(&v), p.result_type())
                }
                Err(e) => format!("runtime error: {e}"),
            },
        };
        let _ = writeln!(out, "{src} => {line}");
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    assert_golden(&dir, "expressions", &out);
}
