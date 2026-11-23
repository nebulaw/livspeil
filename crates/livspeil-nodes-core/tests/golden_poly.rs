// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Golden tests for the polymorphic nodes and typed curves (M0.17):
//! `core.logic.select@2`, `core.math.lerp@2`, `core.curve.ramp@2` and
//! `core.table.get_value@1`, each instantiated for several types.
//!
//! Each line reads `node<T>(port=value, ...) => out=value, ...` or
//! `=> error: ...`. Inputs not listed take the resolved spec default.
//!
//! Refresh with
//! `UPDATE_GOLDEN=1 cargo test -p livspeil-nodes-core --test golden_poly`
//! and review the diff by hand.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use livspeil_node_api::{
    CookContext, Curve, CurveExtrapolation, CurveInterpolation, PortType, Table, TypedCurveKey,
    Value, instantiate,
};
use livspeil_nodes_core::core_registry;
use livspeil_testkit::assert_golden;

fn show(value: &Value) -> String {
    match value {
        Value::Float(v) => format!("{v:?}"),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => v.to_string(),
        Value::String(v) => format!("{v:?}"),
        Value::Vec2(v) => format!("vec2{v:?}"),
        Value::Vec3(v) => format!("vec3{v:?}"),
        Value::Vec4(v) => format!("vec4{v:?}"),
        Value::Color(v) => format!("oklch{v:?}"),
        Value::Table(t) => format!("table[{}x{}]", t.column_count(), t.row_count()),
        Value::Curve(c) => format!("curve<{}>[{} keys]", c.value_type(), c.keys().len()),
        other => format!("{other:?}"),
    }
}

fn case(out: &mut String, id: &str, ty: PortType, inputs: &[(&str, Value)]) {
    let registry = core_registry();
    let base = registry.get(id).expect("registered");
    let node = instantiate(base, &[ty]).expect("in class");
    let spec = node.spec();
    let mut values: Vec<Value> = spec
        .inputs
        .iter()
        .map(|p| p.default.clone().expect("default"))
        .collect();
    let mut shown = Vec::new();
    for (port, value) in inputs {
        let index = spec.input_index(port).expect("port");
        values[index] = value.clone();
        shown.push(format!("{port}={}", show(value)));
    }
    let _ = write!(out, "{}<{ty}>({}) => ", spec.id, shown.join(", "));
    match node.cook(&CookContext::default(), &values) {
        Ok(outputs) => {
            let parts: Vec<String> = spec
                .outputs
                .iter()
                .zip(&outputs)
                .map(|(p, v)| format!("{}={}", p.name, show(v)))
                .collect();
            let _ = writeln!(out, "{}", parts.join(", "));
        }
        Err(e) => {
            let _ = writeln!(out, "error: {e}");
        }
    }
}

fn curve(keys: Vec<(f64, Value)>) -> Value {
    Value::Curve(Arc::new(
        Curve::typed(
            keys.into_iter()
                .map(|(p, v)| TypedCurveKey::new(p, v))
                .collect(),
        )
        .expect("valid curve"),
    ))
}

#[test]
fn polymorphic_nodes() {
    let mut out = String::new();
    let select = "core.logic.select@2";
    for ty in [
        PortType::Float,
        PortType::Int,
        PortType::Bool,
        PortType::Vec2,
        PortType::Vec3,
        PortType::Vec4,
        PortType::Color,
        PortType::String,
        PortType::Table,
        PortType::Curve,
    ] {
        case(&mut out, select, ty, &[]);
        case(&mut out, select, ty, &[("cond", Value::Bool(true))]);
    }
    case(
        &mut out,
        select,
        PortType::String,
        &[
            ("cond", Value::Bool(true)),
            ("a", Value::String("day".into())),
        ],
    );

    let lerp = "core.math.lerp@2";
    case(&mut out, lerp, PortType::Float, &[("t", Value::Float(0.25))]);
    case(&mut out, lerp, PortType::Vec2, &[("t", Value::Float(-1.0))]);
    case(
        &mut out,
        lerp,
        PortType::Vec3,
        &[
            ("a", Value::Vec3([1.0, 2.0, 3.0])),
            ("b", Value::Vec3([3.0, 2.0, 1.0])),
        ],
    );
    case(&mut out, lerp, PortType::Vec4, &[]);
    for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
        case(
            &mut out,
            lerp,
            PortType::Color,
            &[
                ("a", Value::Color([0.6, 0.15, 20.0, 1.0])),
                ("b", Value::Color([0.8, 0.12, 340.0, 0.5])),
                ("t", Value::Float(t)),
            ],
        );
    }
    case(
        &mut out,
        lerp,
        PortType::Color,
        &[
            ("a", Value::Color([0.9, 0.0, 123.0, 1.0])),
            ("b", Value::Color([0.5, 0.2, 250.0, 1.0])),
            ("t", Value::Float(0.5)),
        ],
    );

    let ramp = "core.curve.ramp@2";
    for ty in [
        PortType::Float,
        PortType::Vec2,
        PortType::Vec3,
        PortType::Vec4,
        PortType::Color,
    ] {
        case(&mut out, ramp, ty, &[("position", Value::Float(0.75))]);
    }
    let sunset = curve(vec![
        (0.0, Value::Color([0.95, 0.05, 90.0, 1.0])),
        (0.5, Value::Color([0.7, 0.18, 40.0, 1.0])),
        (1.0, Value::Color([0.3, 0.1, 290.0, 1.0])),
    ]);
    for x in [-1.0, 0.0, 0.25, 0.5, 0.75, 1.0, 2.0] {
        case(
            &mut out,
            ramp,
            PortType::Color,
            &[("curve", sunset.clone()), ("position", Value::Float(x))],
        );
    }
    let path = Value::Curve(Arc::new(
        Curve::typed(vec![
            TypedCurveKey::new(0.0, Value::Vec3([0.0, 0.0, 0.0]))
                .with_interpolation(CurveInterpolation::Bezier),
            TypedCurveKey::new(1.0, Value::Vec3([1.0, 2.0, 0.0]))
                .with_interpolation(CurveInterpolation::Bezier),
            TypedCurveKey::new(2.0, Value::Vec3([2.0, 0.0, 1.0])),
        ])
        .expect("valid curve")
        .with_extrapolation(CurveExtrapolation::Hold, CurveExtrapolation::Loop),
    ));
    for x in [0.5, 1.5, 2.5] {
        case(
            &mut out,
            ramp,
            PortType::Vec3,
            &[("curve", path.clone()), ("position", Value::Float(x))],
        );
    }
    case(
        &mut out,
        ramp,
        PortType::Vec2,
        &[("curve", path.clone()), ("position", Value::Float(0.5))],
    );

    let get = "core.table.get_value@1";
    let table = Value::Table(
        Table::from_rows(
            &[
                ("name", PortType::String),
                ("pages", PortType::Int),
                ("tint", PortType::Color),
            ],
            [
                vec![
                    Value::String("Dune".into()),
                    Value::Int(412),
                    Value::Color([0.7, 0.1, 60.0, 1.0]),
                ],
                vec![
                    Value::String("Emma".into()),
                    Value::Int(474),
                    Value::Color([0.5, 0.2, 300.0, 1.0]),
                ],
            ],
        )
        .expect("valid table"),
    );
    for (ty, column, row) in [
        (PortType::String, "name", 1),
        (PortType::Int, "pages", 0),
        (PortType::Float, "pages", 1),
        (PortType::Color, "tint", 1),
        (PortType::Color, "tint", 2),
        (PortType::Vec3, "tint", 0),
        (PortType::String, "nope", 0),
    ] {
        case(
            &mut out,
            get,
            ty,
            &[
                ("table", table.clone()),
                ("column", Value::String(column.into())),
                ("row", Value::Int(row)),
            ],
        );
    }
    case(&mut out, get, PortType::Vec2, &[]);

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    assert_golden(&dir, "poly", &out);
}
