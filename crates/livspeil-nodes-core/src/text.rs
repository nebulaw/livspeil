// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Text nodes (milestone M0.6c).

use livspeil_node_api::{CookContext, NodeError, NodeImpl, NodeSpec, PortType, Value};

use crate::{float, string};

/// Largest number of decimals `core.text.format` prints.
pub const MAX_PRECISION: i64 = 12;

/// `x` with exactly `precision` decimals.
///
/// Uses Rust's own float formatting (not the C library), which rounds the
/// exact binary value correctly and is identical on every platform; so
/// `0.125` with two decimals prints `0.12` (ties go to even) while `2.675`
/// prints `2.67` because the stored value is slightly below 2.675. A result
/// that shows only zeros never carries a minus sign (`-0.0` and `-0.001`
/// print as `0.00`). `NaN` prints as `NaN`, infinities as `inf` and `-inf`.
#[must_use]
pub fn format_number(x: f64, precision: usize) -> String {
    let s = format!("{x:.precision$}");
    match s.strip_prefix('-') {
        Some(rest) if rest.bytes().all(|b| b == b'0' || b == b'.') => rest.to_owned(),
        _ => s,
    }
}

/// Why a template could not be expanded.
fn template_error(reason: String) -> NodeError {
    NodeError::InvalidInput {
        port: "template",
        reason,
    }
}

/// Expand `template`: `{a}` and `{b}` are replaced by `a` and `b`, `{{` and
/// `}}` stand for literal braces. Any other `{...}`, an unclosed `{` or a
/// lone `}` is an error naming its byte offset.
fn expand(template: &str, a: &str, b: &str) -> Result<String, NodeError> {
    let bytes = template.as_bytes();
    let mut out = String::with_capacity(template.len() + a.len() + b.len());
    // Start of the literal text not yet copied. Every index below points at
    // an ASCII brace, so slicing there is always on a char boundary.
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                out.push_str(&template[start..i]);
                if bytes.get(i + 1) == Some(&b'{') {
                    out.push('{');
                    i += 2;
                } else {
                    let rest = &template[i + 1..];
                    let Some(end) = rest.find('}') else {
                        return Err(template_error(format!(
                            "unclosed '{{' at byte {i} (write '{{{{' for a literal brace)"
                        )));
                    };
                    match &rest[..end] {
                        "a" => out.push_str(a),
                        "b" => out.push_str(b),
                        name => {
                            return Err(template_error(format!(
                                "unknown placeholder {{{name}}} at byte {i} (only {{a}} and {{b}} exist)"
                            )));
                        }
                    }
                    i += end + 2;
                }
                start = i;
            }
            b'}' => {
                out.push_str(&template[start..i]);
                if bytes.get(i + 1) == Some(&b'}') {
                    out.push('}');
                    i += 2;
                    start = i;
                } else {
                    return Err(template_error(format!(
                        "unmatched '}}' at byte {i} (write '}}}}' for a literal brace)"
                    )));
                }
            }
            _ => i += 1,
        }
    }
    out.push_str(&template[start..]);
    Ok(out)
}

/// `core.text.format@1`: put two numbers into a text template.
///
/// Inputs: `template` (String, default `"{a}"`), `a` and `b` (Float,
/// default 0), `precision` (Int, decimals, default 2, clamped to
/// `0..=12`). Output: `out` (String).
///
/// In the template, `{a}` and `{b}` are replaced by the numbers printed with
/// `precision` decimals (see [`format_number`]: platform-independent,
/// no `-0.00`, `NaN`/`inf`/`-inf` for the specials), and `{{` and `}}` are
/// literal braces. Placeholders may repeat or be absent. Anything else in
/// braces (`{c}`, `{}`, `{ a }`), an unclosed `{` or a lone `}` is a
/// [`NodeError::InvalidInput`] on `template`. Other text, including
/// non-ASCII, is copied unchanged.
#[derive(Debug)]
pub struct Format {
    spec: NodeSpec,
}

impl Format {
    #[must_use]
    pub fn new() -> Self {
        Format {
            spec: NodeSpec::new(
                "core.text.format",
                1,
                "Fill {a} and {b} in a template with numbers ({{ and }} for braces)",
            )
            .input(
                "template",
                PortType::String,
                Some(Value::String("{a}".to_owned())),
            )
            .input("a", PortType::Float, Some(Value::Float(0.0)))
            .input("b", PortType::Float, Some(Value::Float(0.0)))
            .input("precision", PortType::Int, Some(Value::Int(2)))
            .output("out", PortType::String),
        }
    }
}

impl Default for Format {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeImpl for Format {
    fn spec(&self) -> &NodeSpec {
        &self.spec
    }

    fn cook(&self, _ctx: &CookContext, inputs: &[Value]) -> Result<Vec<Value>, NodeError> {
        let template = string(inputs, 0, "template")?;
        let a = float(inputs, 1, "a")?;
        let b = float(inputs, 2, "b")?;
        let precision = match inputs.get(3) {
            Some(Value::Int(p)) => *p,
            _ => {
                return Err(NodeError::InvalidInput {
                    port: "precision",
                    reason: "expected an Int".to_owned(),
                });
            }
        };
        // In 0..=12 after the clamp, so the conversion cannot fail.
        let precision = usize::try_from(precision.clamp(0, MAX_PRECISION)).unwrap_or(0);
        let out = expand(
            template,
            &format_number(a, precision),
            &format_number(b, precision),
        )?;
        Ok(vec![Value::String(out)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(template: &str, a: f64, b: f64, precision: i64) -> Result<String, NodeError> {
        let out = Format::new().cook(
            &CookContext::default(),
            &[
                Value::String(template.to_owned()),
                Value::Float(a),
                Value::Float(b),
                Value::Int(precision),
            ],
        )?;
        match &out[..] {
            [Value::String(s)] => Ok(s.clone()),
            other => panic!("unexpected output {other:?}"),
        }
    }

    fn template_err(template: &str) -> String {
        match fmt(template, 1.0, 2.0, 2).unwrap_err() {
            NodeError::InvalidInput {
                port: "template",
                reason,
            } => reason,
            other => panic!("{template:?}: {other}"),
        }
    }

    #[test]
    fn replaces_placeholders() {
        assert_eq!(fmt("{a}", 0.0, 0.0, 2).unwrap(), "0.00");
        assert_eq!(
            fmt("{a} of {b}", 3.0, 10.0, 0).unwrap(),
            "3 of 10".to_owned()
        );
        assert_eq!(fmt("{b}{a}{b}", 1.0, 2.0, 1).unwrap(), "2.01.02.0");
        assert_eq!(fmt("no numbers", 1.0, 2.0, 2).unwrap(), "no numbers");
        assert_eq!(fmt("", 1.0, 2.0, 2).unwrap(), "");
        assert_eq!(
            fmt("Temp: {a} °C, ☀ {b}%", 21.456, 80.0, 1).unwrap(),
            "Temp: 21.5 °C, ☀ 80.0%"
        );
    }

    #[test]
    fn brace_escapes() {
        assert_eq!(fmt("{{a}}", 1.0, 2.0, 0).unwrap(), "{a}");
        assert_eq!(fmt("{{{a}}}", 1.0, 2.0, 0).unwrap(), "{1}");
        assert_eq!(fmt("}}{{", 1.0, 2.0, 0).unwrap(), "}{");
        assert_eq!(fmt("{{}}", 1.0, 2.0, 0).unwrap(), "{}");
    }

    #[test]
    fn bad_templates_are_rejected() {
        assert!(template_err("{c}").contains("unknown placeholder {c} at byte 0"));
        assert!(template_err("x{}").contains("unknown placeholder {} at byte 1"));
        assert!(template_err("{ a }").contains("unknown placeholder { a }"));
        assert!(template_err("{A}").contains("unknown placeholder {A}"));
        assert!(template_err("{ab}").contains("unknown placeholder {ab}"));
        assert!(template_err("{a").contains("unclosed '{' at byte 0"));
        assert!(template_err("ok {").contains("unclosed '{' at byte 3"));
        assert!(template_err("}").contains("unmatched '}' at byte 0"));
        assert!(template_err("{a}}").contains("unmatched '}' at byte 3"));
        assert!(template_err("é}").contains("unmatched '}' at byte 2"));
    }

    #[test]
    fn precision_is_clamped() {
        assert_eq!(fmt("{a}", 2.5, 0.0, -3).unwrap(), "2");
        assert_eq!(fmt("{a}", 3.5, 0.0, 0).unwrap(), "4");
        assert_eq!(fmt("{a}", 1.0 / 3.0, 0.0, 12).unwrap(), "0.333333333333");
        assert_eq!(
            fmt("{a}", 1.0 / 3.0, 0.0, i64::MAX).unwrap(),
            "0.333333333333"
        );
        assert_eq!(fmt("{a}", 1.0, 0.0, i64::MIN).unwrap(), "1");
    }

    #[test]
    fn number_formatting_is_exact_and_sign_safe() {
        // Ties on the exact binary value go to even; 2.675 is stored below.
        assert_eq!(format_number(0.125, 2), "0.12");
        assert_eq!(format_number(0.375, 2), "0.38");
        assert_eq!(format_number(2.675, 2), "2.67");
        assert_eq!(format_number(-1.5, 0), "-2");
        assert_eq!(format_number(-0.0, 2), "0.00");
        assert_eq!(format_number(-0.001, 2), "0.00");
        assert_eq!(format_number(-0.4, 0), "0");
        assert_eq!(format_number(-0.006, 2), "-0.01");
        assert_eq!(format_number(1e21, 0), "1000000000000000000000");
        assert_eq!(format_number(f64::NAN, 2), "NaN");
        assert_eq!(format_number(f64::INFINITY, 2), "inf");
        assert_eq!(format_number(f64::NEG_INFINITY, 2), "-inf");
    }

    #[test]
    fn rejects_wrong_types() {
        let node = Format::new();
        let ctx = CookContext::default();
        let err = node
            .cook(
                &ctx,
                &[
                    Value::Float(1.0),
                    Value::Float(0.0),
                    Value::Float(0.0),
                    Value::Int(2),
                ],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NodeError::InvalidInput {
                port: "template",
                ..
            }
        ));
        let err = node
            .cook(
                &ctx,
                &[
                    Value::String("{a}".to_owned()),
                    Value::Float(0.0),
                    Value::Float(0.0),
                    Value::Float(2.0),
                ],
            )
            .unwrap_err();
        assert!(matches!(
            err,
            NodeError::InvalidInput {
                port: "precision",
                ..
            }
        ));
    }
}
