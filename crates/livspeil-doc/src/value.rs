// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Encoding of [`Value`]s as KDL values.
//!
//! | Value     | KDL                                   |
//! |-----------|---------------------------------------|
//! | `Float`   | `1.0`, `-0.5`, `1e-7` (always has `.` or `e`) |
//! | `Int`     | `1`, `-42` (no `.` and no exponent)   |
//! | `Bool`    | `#true`, `#false`                     |
//! | `String`  | `"text"` with KDL escapes             |
//! | `Vec2`    | `(vec2)"1.0 2.0"`                     |
//! | `Vec3`    | `(vec3)"1.0 2.0 3.0"`                 |
//! | `Vec4`    | `(vec4)"1.0 2.0 3.0 4.0"`             |
//! | `Color`   | `(color)"oklch(0.8 0.12 60.0 1.0)"` (L C H alpha) |
//!
//! `Table` and `Curve` values are not single KDL values; they are written
//! as `table { ... }` and `curve { ... }` blocks inside the `input` (see
//! the crate documentation and `format/blocks.rs`, `parse/blocks.rs`).
//!
//! Floats are written in Rust's shortest round-trip form, so parsing gives
//! back the bit-identical `f64`. `NaN` and infinities are rejected in both
//! directions. Components inside vector and color strings are separated by
//! whitespace; when reading, integers such as `60` are accepted there and the
//! alpha of a color may be omitted (it then defaults to `1.0`).

use std::fmt::Write as _;

use kdl::KdlValue;
use livspeil_node_api::Value;

/// Shortest representation of a finite `f64` that parses back exactly and
/// always reads as a KDL float (contains `.` or `e`).
pub(crate) fn format_float(v: f64) -> String {
    // Rust's `Debug` for f64 is the shortest round-trip form and always
    // contains a '.' or an exponent ("1.0", "1e-7", "1.5e300").
    format!("{v:?}")
}

/// KDL v2 quoted string with escapes. Every character that the KDL v2 spec
/// disallows in source, that is a newline or that is a control character
/// is escaped, so the output is always a single-line, valid string.
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if needs_unicode_escape(c) => {
                let _ = write!(out, "\\u{{{:x}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn needs_unicode_escape(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}' | '\u{FEFF}'
        )
}

fn components(values: &[f64]) -> String {
    values
        .iter()
        .map(|v| format_float(*v))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether every number in the value is finite.
pub(crate) fn is_finite(value: &Value) -> bool {
    match value {
        Value::Float(v) => v.is_finite(),
        Value::Vec2(v) => v.iter().all(|c| c.is_finite()),
        Value::Vec3(v) => v.iter().all(|c| c.is_finite()),
        Value::Vec4(v) | Value::Color(v) => v.iter().all(|c| c.is_finite()),
        Value::Table(t) => t.columns().iter().all(|c| c.values().iter().all(is_finite)),
        // Curves are validated, so their numbers are finite.
        _ => true,
    }
}

/// The canonical KDL text of a value. `None` for a value type this format
/// version cannot store (only possible once `Value` gains variants).
pub(crate) fn format_value(value: &Value) -> Option<String> {
    Some(match value {
        Value::Float(v) => format_float(*v),
        Value::Int(v) => v.to_string(),
        Value::Bool(v) => format!("#{v}"),
        Value::String(s) => quote(s),
        Value::Vec2(v) => format!("(vec2){}", quote(&components(v))),
        Value::Vec3(v) => format!("(vec3){}", quote(&components(v))),
        Value::Vec4(v) => format!("(vec4){}", quote(&components(v))),
        Value::Color(v) => format!("(color){}", quote(&format!("oklch({})", components(v)))),
        _ => return None,
    })
}

/// Decode a KDL value with an optional type annotation.
///
/// # Errors
/// Returns a human-readable reason.
pub(crate) fn decode(ty: Option<&str>, value: &KdlValue) -> Result<Value, String> {
    match (ty, value) {
        (None, KdlValue::Float(v)) => finite(*v).map(Value::Float),
        (None, KdlValue::Integer(v)) => i64::try_from(*v)
            .map(Value::Int)
            .map_err(|_| format!("integer {v} does not fit in 64 bits")),
        (None, KdlValue::Bool(v)) => Ok(Value::Bool(*v)),
        (None, KdlValue::String(s)) => Ok(Value::String(s.clone())),
        (None, KdlValue::Null) => Err("#null is not a value; leave the input out instead".into()),
        (Some("vec2"), KdlValue::String(s)) => fixed::<2>(s, "vec2").map(Value::Vec2),
        (Some("vec3"), KdlValue::String(s)) => fixed::<3>(s, "vec3").map(Value::Vec3),
        (Some("vec4"), KdlValue::String(s)) => fixed::<4>(s, "vec4").map(Value::Vec4),
        (Some("color"), KdlValue::String(s)) => color(s).map(Value::Color),
        (Some(t @ ("vec2" | "vec3" | "vec4" | "color")), _) => Err(format!(
            "a ({t}) value must be a string, for example {}",
            example(t)
        )),
        (Some(other), _) => Err(format!(
            "unknown type annotation ({other}); expected (vec2), (vec3), (vec4) or (color)"
        )),
    }
}

fn example(ty: &str) -> &'static str {
    match ty {
        "vec2" => "(vec2)\"1.0 2.0\"",
        "vec3" => "(vec3)\"1.0 2.0 3.0\"",
        "vec4" => "(vec4)\"1.0 2.0 3.0 4.0\"",
        _ => "(color)\"oklch(0.8 0.12 60.0 1.0)\"",
    }
}

fn finite(v: f64) -> Result<f64, String> {
    if v.is_finite() {
        Ok(v)
    } else {
        Err("NaN and infinities are not allowed in documents".into())
    }
}

fn number(text: &str) -> Result<f64, String> {
    // `f64::from_str` also accepts "inf" and "NaN"; reject them explicitly.
    let looks_numeric = text
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'));
    if !looks_numeric {
        let lower = text.to_ascii_lowercase();
        if lower.contains("nan") || lower.contains("inf") {
            return finite(f64::NAN);
        }
        return Err(format!("'{text}' is not a number"));
    }
    text.parse::<f64>()
        .map_err(|_| format!("'{text}' is not a number"))
        .and_then(finite)
}

fn parse_list(text: &str) -> Result<Vec<f64>, String> {
    text.split_whitespace().map(number).collect()
}

fn fixed<const N: usize>(text: &str, ty: &str) -> Result<[f64; N], String> {
    let list = parse_list(text)?;
    <[f64; N]>::try_from(list.as_slice()).map_err(|_| {
        format!(
            "a ({ty}) value needs {N} numbers, found {}; for example {}",
            list.len(),
            example(ty)
        )
    })
}

fn color(text: &str) -> Result<[f64; 4], String> {
    let inner = text
        .trim()
        .strip_prefix("oklch(")
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| {
            format!(
                "colors are written oklch(L C H alpha), for example {}",
                example("color")
            )
        })?;
    match parse_list(inner)?.as_slice() {
        [l, c, h] => Ok([*l, *c, *h, 1.0]),
        [l, c, h, a] => Ok([*l, *c, *h, *a]),
        other => Err(format!(
            "oklch() needs 3 or 4 numbers (L C H [alpha]), found {}",
            other.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_always_look_like_floats() {
        for v in [
            0.0,
            -0.0,
            1.0,
            0.1,
            1e16,
            1e-7,
            123_456.789,
            f64::MAX,
            5e-324,
        ] {
            let s = format_float(v);
            assert!(s.contains('.') || s.contains('e'), "{s}");
            assert_eq!(s.parse::<f64>().unwrap().to_bits(), v.to_bits());
        }
    }

    #[test]
    fn quote_escapes_specials() {
        assert_eq!(quote("a\"b\\c\nd"), r#""a\"b\\c\nd""#);
        assert_eq!(quote("\u{1}\u{2028}\u{feff}"), r#""\u{1}\u{2028}\u{feff}""#);
        assert_eq!(quote("héllo ✓"), "\"héllo ✓\"");
    }

    #[test]
    fn numbers_inside_strings_reject_non_finite() {
        assert!(number("inf").is_err());
        assert!(number("NaN").is_err());
        assert!(number("1e999").is_err());
        assert_eq!(number("60"), Ok(60.0));
    }

    #[test]
    fn color_alpha_defaults_to_one() {
        assert_eq!(color("oklch(0.5 0.1 30)"), Ok([0.5, 0.1, 30.0, 1.0]));
        assert!(color("rgb(1 2 3)").is_err());
        assert!(color("oklch(1 2)").is_err());
    }
}
