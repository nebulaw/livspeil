// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Strict parser of the SVG subset the icon set uses: `<svg>`, `<g>`,
//! `<path>`, `<rect>` and comments, monochrome (`fill="currentColor"`), on
//! a 24x24 view box, without transforms, strokes or styles. Anything else
//! is an error: it would be drawn wrong.

use std::fmt;

use zeno::{Command, PathBuilder, Point};

use crate::path::parse_path;

/// Edge length of every icon's view box.
pub const VIEW_BOX: f32 = 24.0;

/// Why an SVG was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgError(String);

impl fmt::Display for SvgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported SVG: {}", self.0)
    }
}

impl std::error::Error for SvgError {}

fn err<T>(why: impl Into<String>) -> Result<T, SvgError> {
    Err(SvgError(why.into()))
}

/// A start tag.
struct Tag<'a> {
    name: &'a str,
    attrs: Vec<(&'a str, &'a str)>,
    self_closing: bool,
}

impl<'a> Tag<'a> {
    /// `body` is the text between `<` and `>`.
    fn parse(body: &'a str) -> Result<Tag<'a>, SvgError> {
        let (body, self_closing) = match body.strip_suffix('/') {
            Some(b) => (b, true),
            None => (body, false),
        };
        let name_end = body
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(body.len());
        let (name, mut rest) = body.split_at(name_end);
        let mut attrs: Vec<(&str, &str)> = Vec::new();
        loop {
            rest = rest.trim_start();
            if rest.is_empty() {
                break;
            }
            let Some((key, after)) = rest.split_once("=\"") else {
                return err(format!("bad attribute syntax in <{name}>"));
            };
            let Some((value, after)) = after.split_once('"') else {
                return err(format!("unterminated attribute in <{name}>"));
            };
            if attrs.iter().any(|(k, _)| *k == key) {
                return err(format!("duplicate attribute {key} in <{name}>"));
            }
            attrs.push((key, value));
            rest = after;
        }
        Ok(Tag {
            name,
            attrs,
            self_closing,
        })
    }

    fn get(&self, key: &str) -> Option<&'a str> {
        self.attrs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    fn number(&self, key: &str) -> Result<Option<f32>, SvgError> {
        self.get(key)
            .map(|v| match v.parse::<f32>() {
                Ok(n) if n.is_finite() => Ok(n),
                _ => err(format!("{key}=\"{v}\" is not a number")),
            })
            .transpose()
    }

    /// Rejects attributes outside `allowed` and values that must be fixed.
    fn check(&self, allowed: &[&str]) -> Result<(), SvgError> {
        for (key, value) in &self.attrs {
            if !allowed.contains(key) {
                return err(format!("attribute {key} on <{}>", self.name));
            }
            let fixed = match *key {
                "fill" => Some("currentColor"),
                "xmlns" => Some("http://www.w3.org/2000/svg"),
                "viewBox" => Some("0 0 24 24"),
                "width" | "height" if self.name == "svg" => Some("24"),
                _ => None,
            };
            if fixed.is_some_and(|f| f != *value) {
                return err(format!("{key}=\"{value}\" on <{}>", self.name));
            }
        }
        Ok(())
    }

    /// The outline of a shape element.
    fn shape(&self) -> Result<Vec<Command>, SvgError> {
        if self.name == "path" {
            // `class` has no effect: no stylesheet is accepted.
            self.check(&["d", "fill", "class"])?;
            let Some(d) = self.get("d") else {
                return err("<path> without d");
            };
            return parse_path(d).or_else(err);
        }
        self.check(&["x", "y", "width", "height", "rx", "ry", "fill"])?;
        let x = self.number("x")?.unwrap_or(0.0);
        let y = self.number("y")?.unwrap_or(0.0);
        let (Some(w), Some(h)) = (self.number("width")?, self.number("height")?) else {
            return err("<rect> without width and height");
        };
        let (rx, ry) = (self.number("rx")?, self.number("ry")?);
        // One radius given stands for both.
        let rx_or = rx.or(ry).unwrap_or(0.0);
        let ry_or = ry.or(rx).unwrap_or(0.0);
        if w <= 0.0 || h <= 0.0 || rx_or < 0.0 || ry_or < 0.0 {
            return err("<rect> with a negative or empty size");
        }
        let mut out: Vec<Command> = Vec::new();
        if rx_or > 0.0 && ry_or > 0.0 {
            out.add_round_rect(Point::new(x, y), w, h, rx_or, ry_or);
        } else {
            out.add_rect(Point::new(x, y), w, h);
        }
        Ok(out)
    }
}

/// The filled outlines of an icon in view box coordinates, one per shape
/// element, each filled with the non-zero rule and painted over the ones
/// before it.
///
/// # Errors
/// [`SvgError`] for any element, attribute or path data outside the subset.
pub fn parse_svg(source: &str) -> Result<Vec<Vec<Command>>, SvgError> {
    let mut shapes = Vec::new();
    let mut open: Vec<&str> = Vec::new();
    let mut seen_root = false;
    let mut rest = source;
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if let Some(comment) = rest.strip_prefix("<!--") {
            let Some((_, after)) = comment.split_once("-->") else {
                return err("unterminated comment");
            };
            rest = after;
            continue;
        }
        let Some(tag) = rest.strip_prefix('<') else {
            return err("text content");
        };
        let Some((body, after)) = tag.split_once('>') else {
            return err("unterminated tag");
        };
        rest = after;
        if let Some(name) = body.strip_prefix('/') {
            if open.pop() != Some(name.trim_end()) {
                return err(format!("unexpected </{name}>"));
            }
            continue;
        }
        let tag = Tag::parse(body)?;
        let in_container = matches!(open.last(), Some(&("svg" | "g")));
        match tag.name {
            "svg" if open.is_empty() && !seen_root => {
                tag.check(&["xmlns", "viewBox", "width", "height", "fill"])?;
                if tag.get("viewBox").is_none() {
                    return err("<svg> without viewBox");
                }
                seen_root = true;
            }
            "g" if in_container => tag.check(&["fill"])?,
            "path" | "rect" if in_container => shapes.push(tag.shape()?),
            other => return err(format!("element <{other}>")),
        }
        if !tag.self_closing {
            open.push(tag.name);
        }
    }
    if !seen_root || !open.is_empty() {
        return err("incomplete document");
    }
    if shapes.is_empty() {
        return err("no shapes");
    }
    Ok(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(inner: &str) -> String {
        format!(r#"<svg viewBox="0 0 24 24"><g fill="currentColor"><!--c-->{inner}</g></svg>"#)
    }

    #[test]
    fn accepts_paths_and_rects() {
        let svg = wrap(
            r#"<path d="M0 0h2v2z"></path><rect width="4" height="4" x="1" y="1" rx="1"/>
               <rect width="4" height="4"></rect>"#,
        );
        let shapes = parse_svg(&svg).unwrap();
        assert_eq!(shapes.len(), 3);
        // The rounded rect has arcs, the plain one only lines.
        assert!(shapes[1].iter().any(|c| matches!(c, Command::CurveTo(..))));
        assert!(!shapes[2].iter().any(|c| matches!(c, Command::CurveTo(..))));
    }

    #[test]
    fn rejects_everything_else() {
        for inner in [
            r#"<circle r="2"/>"#,
            r#"<path d="M0 0h2v2z" transform="rotate(45)"/>"#,
            r#"<path d="M0 0h2v2z" stroke="red"/>"#,
            r#"<path d="M0 0h2v2z" fill="none"/>"#,
            r#"<path d="M0 0h2v2z" fill-rule="evenodd"/>"#,
            r#"<path d="M0 0h2v2z" style="opacity:0.5"/>"#,
            r#"<path d="M0 0h2v2z" opacity="0.5"/>"#,
            r#"<style>.b{fill:none}</style>"#,
            r#"<rect width="4" height="4" transform="rotate(45)"/>"#,
            r#"<rect width="-4" height="4"/>"#,
            r#"<rect width="50%" height="4"/>"#,
            r#"<path d="M0 0L1"/>"#,
            r#"<path d="M0 0h2v2z">text</path>"#,
            r#"<path d="M0 0h2v2z"><path d="M0 0h1z"/></path>"#,
            r#""#,
        ] {
            assert!(parse_svg(&wrap(inner)).is_err(), "{inner} was accepted");
        }
        for doc in [
            r#"<svg viewBox="0 0 16 16"><path d="M0 0h2z"/></svg>"#,
            r#"<svg><path d="M0 0h2z"/></svg>"#,
            r#"<svg viewBox="0 0 24 24"><path d="M0 0h2z"/>"#,
            r#"<path d="M0 0h2z"/>"#,
        ] {
            assert!(parse_svg(doc).is_err(), "{doc} was accepted");
        }
    }
}
