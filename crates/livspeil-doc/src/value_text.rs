// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A single value written in the document syntax, outside a document
//! (for command-line arguments such as `livspeil cook --event`).

use kdl::KdlDocument;
use livspeil_node_api::Value;

use crate::error::DocError;
use crate::value::decode;

/// Parse one value in the KDL value syntax of documents (see the table in
/// the crate docs): `1.0` (Float), `3` (Int), `#true`, `"text"`,
/// `(vec3)"1 2 3"`, `(color)"oklch(0.8 0.12 60)"`. Raw strings and the
/// other KDL string forms are accepted too. Table and curve blocks are
/// not single values and cannot be given this way.
///
/// # Errors
/// [`DocError::Invalid`] (without a location) when `text` is not exactly
/// one value or the value is not valid (unknown annotation, NaN, ...).
pub fn parse_value(text: &str) -> Result<Value, DocError> {
    let invalid = |reason: &str| DocError::invalid(None, format!("'{text}': {reason}"));
    if text.trim().is_empty() {
        return Err(invalid("no value"));
    }
    let doc = KdlDocument::parse_v2(&format!("v {text}"))
        .map_err(|_| invalid("not a value in the document syntax (for example 1.0, 3, #true, \"text\", (vec3)\"1 2 3\")"))?;
    let [node] = doc.nodes() else {
        return Err(invalid("expected a single value"));
    };
    if node.children().is_some() {
        return Err(invalid("expected a single value"));
    }
    let [entry] = node.entries() else {
        return Err(invalid("expected a single value"));
    };
    if entry.name().is_some() {
        return Err(invalid("expected a value, not a property"));
    }
    decode(entry.ty().map(|t| t.value()), entry.value()).map_err(|reason| invalid(&reason))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values() {
        assert_eq!(parse_value("1.0"), Ok(Value::Float(1.0)));
        assert_eq!(parse_value(" -3 "), Ok(Value::Int(-3)));
        assert_eq!(parse_value("#true"), Ok(Value::Bool(true)));
        assert_eq!(parse_value("\"a b\""), Ok(Value::String("a b".into())));
        assert_eq!(parse_value("plain"), Ok(Value::String("plain".into())));
        assert_eq!(parse_value("\"x;{}\""), Ok(Value::String("x;{}".into())));
        assert_eq!(
            parse_value("(vec3)\"1 2 3\""),
            Ok(Value::Vec3([1.0, 2.0, 3.0]))
        );
        assert_eq!(
            parse_value("(color)\"oklch(0.8 0.1 60)\""),
            Ok(Value::Color([0.8, 0.1, 60.0, 1.0]))
        );
    }

    #[test]
    fn rejects() {
        for text in [
            "",
            " ",
            "1 2",
            "a=1",
            "#null",
            "#nan",
            "(vec3)\"1 2\"",
            "(rgb)\"x\"",
            "1 { }",
            "1; 2",
            "\"open",
        ] {
            assert!(parse_value(text).is_err(), "{text:?}");
        }
    }
}
