// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! What the user types into an input's field, and what the field shows.
//!
//! One field per input, for every type: a value (`1.5`, `3`, `true`,
//! `0 1 0`, `0.7 0.1 60`, any text for a string), an expression starting
//! with `=`, or nothing to go back to the default.

use livspeil_node_api::{PortType, Value};

/// What a field's text asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    /// Forget the value and expression: the default applies.
    Clear,
    /// An expression (without the `=`).
    Expression(String),
    /// A value.
    Value(Value),
}

/// The `N` numbers of `text`, separated by spaces or commas.
fn numbers<const N: usize>(text: &str) -> Option<[f64; N]> {
    let list: Vec<f64> = text
        .split([' ', ','])
        .filter(|s| !s.is_empty())
        .map(|s| s.parse().ok().filter(|x: &f64| x.is_finite()))
        .collect::<Option<_>>()?;
    list.try_into().ok()
}

/// Read `text` for an input of type `ty`.
///
/// # Errors
/// A message saying what the type expects.
pub fn parse(ty: PortType, text: &str) -> Result<Entry, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Entry::Clear);
    }
    if let Some(expression) = trimmed.strip_prefix('=') {
        return Ok(Entry::Expression(expression.trim().to_owned()));
    }
    let value = match ty {
        PortType::Float => numbers::<1>(trimmed).map(|[x]| Value::Float(x)),
        PortType::Int => trimmed.parse().ok().map(Value::Int),
        PortType::Bool => trimmed.parse().ok().map(Value::Bool),
        PortType::Vec2 => numbers(trimmed).map(Value::Vec2),
        PortType::Vec3 => numbers(trimmed).map(Value::Vec3),
        PortType::Vec4 => numbers(trimmed).map(Value::Vec4),
        // Alpha may be left out.
        PortType::Color => numbers::<4>(trimmed)
            .or_else(|| numbers::<3>(trimmed).map(|[l, c, h]| [l, c, h, 1.0]))
            .map(Value::Color),
        // Taken as typed, spaces and all.
        PortType::String => Some(Value::String(text.to_owned())),
        _ => None,
    };
    value.map(Entry::Value).ok_or_else(|| {
        let wanted = match ty {
            PortType::Float => "a number",
            PortType::Int => "a whole number",
            PortType::Bool => "true or false",
            PortType::Vec2 => "two numbers",
            PortType::Vec3 => "three numbers",
            PortType::Vec4 => "four numbers",
            PortType::Color => "lightness, chroma, hue and optionally alpha",
            _ => "an expression starting with =, or a link",
        };
        format!("{} takes {wanted}", ty.name())
    })
}

/// The text a field shows for `value`: what [`parse`] reads back exactly.
/// Empty for values that cannot be typed.
#[must_use]
pub fn text(value: &Value) -> String {
    let list = |v: &[f64]| {
        v.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    match value {
        Value::Float(x) => x.to_string(),
        Value::Int(x) => x.to_string(),
        Value::Bool(x) => x.to_string(),
        Value::Vec2(v) => list(v),
        Value::Vec3(v) => list(v),
        Value::Vec4(v) | Value::Color(v) => list(v),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_shown_reads_back() {
        for value in [
            Value::Float(-1.5e-7),
            Value::Float(8640.0),
            Value::Int(i64::MIN),
            Value::Bool(false),
            Value::Vec2([1.0, -2.25]),
            Value::Vec3([0.1, 0.2, 0.3]),
            Value::Vec4([0.0, 1.0, 2.0, 3.0]),
            Value::Color([0.35, 0.08, 260.0, 0.5]),
            Value::String("two  words".to_owned()),
        ] {
            let ty = value.port_type();
            assert_eq!(parse(ty, &text(&value)), Ok(Entry::Value(value)), "{ty}");
        }
    }

    #[test]
    fn blank_clears_and_equals_is_an_expression() {
        assert_eq!(parse(PortType::Float, "  "), Ok(Entry::Clear));
        assert_eq!(
            parse(PortType::Color, " = ch(\"a/b\") "),
            Ok(Entry::Expression("ch(\"a/b\")".to_owned()))
        );
        assert_eq!(
            parse(PortType::Color, "0.5, 0.1, 40"),
            Ok(Entry::Value(Value::Color([0.5, 0.1, 40.0, 1.0])))
        );
    }

    #[test]
    fn what_does_not_fit_the_type_is_refused() {
        for (ty, text) in [
            (PortType::Float, "one"),
            (PortType::Float, "nan"),
            (PortType::Float, "inf"),
            (PortType::Float, "1 2"),
            (PortType::Int, "1.5"),
            (PortType::Bool, "yes"),
            (PortType::Vec3, "1 2"),
            (PortType::Color, "1 2"),
            (PortType::Table, "x"),
        ] {
            let err = parse(ty, text).unwrap_err();
            assert!(err.starts_with(ty.name()), "{err}");
        }
    }
}
