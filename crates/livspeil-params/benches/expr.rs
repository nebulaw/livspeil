// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Expression language benchmarks: compiling and evaluating representative
//! parameter expressions (kernel spec, section 6).

use std::collections::BTreeMap;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use livspeil_node_api::{PortType, Value};
use livspeil_params::expr::{Channel, EvalContext, NoChannels, compile};

/// `(name, source)`. Names are used as benchmark ids.
const CASES: [(&str, &str); 7] = [
    ("constant", "1.0 + 2.0 * 3.0"),
    ("numeric", "sin($T * tau * 0.25) * 0.5 + 0.5"),
    (
        "numeric_long",
        "clamp(smoothstep(0.0, 1.0, fract($T * 0.1)) * 2.0 - 1.0, -1.0, 1.0) \
         + lerp(0.2, 0.8, abs(cos($T))) * ($F % 60 < 30 ? 1.0 : -1.0)",
    ),
    (
        "vector",
        "normalize(vec3(sin($T), cos($T), 1.0) * 2.0 + vec3(0.1, 0.2, 0.3)).xz",
    ),
    (
        "channels",
        "ch(\"sun/elevation\") * 0.5 + ch(\"sky/haze\") * ch(\"sky/gain\")",
    ),
    (
        "vector_channels",
        "length(ch(\"cam/pos\") - vec3(0.0, 1.0, 0.0)) / ch(\"sky/gain\")",
    ),
    (
        "string",
        "concat(\"frame \", $F, \": \", upper(\"clock\"), \" \", str(round($T)))",
    ),
];

fn env() -> BTreeMap<String, Channel> {
    let mut env = BTreeMap::new();
    for (path, slot, ty) in [
        ("sun/elevation", 0, PortType::Float),
        ("sky/haze", 1, PortType::Float),
        ("sky/gain", 2, PortType::Float),
        ("cam/pos", 3, PortType::Vec3),
    ] {
        env.insert(path.to_owned(), Channel { slot, ty });
    }
    env
}

fn bench_expr(c: &mut Criterion) {
    let env = env();
    let slots = [
        Value::Float(0.7),
        Value::Float(0.25),
        Value::Float(1.5),
        Value::Vec3([1.0, 2.0, 3.0]),
    ];
    let ctx = EvalContext::new(1234, 20.566, 1.0 / 60.0, &slots);

    // `ch()` needs an environment; the others must compile without one too.
    for (_, source) in CASES {
        if !source.contains("ch(") {
            compile(source, &NoChannels).expect("compiles without channels");
        }
    }

    let mut group = c.benchmark_group("expr/compile");
    for (name, source) in CASES {
        group.bench_with_input(BenchmarkId::from_parameter(name), source, |b, src| {
            b.iter(|| black_box(compile(black_box(src), &env).expect("compiles")));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("expr/eval");
    for (name, source) in CASES {
        let program = compile(source, &env).expect("compiles");
        program.eval(&ctx).expect("evaluates");
        group.bench_function(BenchmarkId::from_parameter(name), |b| {
            b.iter(|| black_box(program.eval(black_box(&ctx)).expect("evaluates")));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_expr);
criterion_main!(benches);
