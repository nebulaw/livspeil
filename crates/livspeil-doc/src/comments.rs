// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Comments carried through [`canonicalize`](crate::canonicalize) (and so
//! `livspeil fmt`).
//!
//! Comments are not part of the document model; [`format()`](crate::format)
//! never writes them. `canonicalize` keeps the prolog (the leading comment
//! block) and, with this pass, the comments attached to items:
//!
//! - **leading** comments: consecutive `//` lines directly above an item
//!   (no blank line in between) whose line starts with the item;
//! - a **trailing** comment: a `//` comment after the item on its first
//!   line (after its last property or argument, and after the `{` that
//!   opens its block or a `;`, if any).
//!
//! Items are the `network` blocks, `node`s, their `input`s, `charm` blocks
//! and `link`s, the `meta` block and its `node` entries. Each is identified
//! by a path that does not depend on its position (a node by its id, an
//! input by node id and port, a link by its network and ends, a meta entry
//! by node name), so comments move with their items when the canonical
//! order moves them. They are written back re-indented to the item:
//! leading lines above it, a trailing comment as ` // ...` at the end of
//! its first line. The text after `//` is kept as is (trailing whitespace
//! removed). Running `canonicalize` on its own output changes nothing.
//!
//! Still dropped: `/* */` block comments; comments that are not directly
//! above or behind an item (separated by a blank line, before a closing
//! `}`, after the last item of a block); comments of items not listed
//! above (`anim` and `key`, `table`, `column` and `row`, `curve`,
//! `capability`) and inside value literals and multi-line strings;
//! slashdashed (`/-`) items and everything around them on their line
//! (also a slashdashed argument or property before a trailing comment);
//! comments behind an item that shares its line with another item (such
//! as `node "a" { input "x" 1 } // c`); and comments of items that are gone after
//! parsing (such as the room header, whose comments are the prolog).
//!
//! The pass works on the text and the KDL tree only (the document parser
//! is not involved): it reads the comments of the source with their item
//! paths, then finds the same paths in the canonical output.

use std::collections::HashMap;

use kdl::{KdlDocument, KdlNode};

/// The comments of one item.
#[derive(Debug, Default)]
struct Attached {
    /// `//` lines above the item, trimmed, top first.
    leading: Vec<String>,
    /// The `//` comment behind the item's first line, trimmed.
    trailing: Option<String>,
}

/// `canonical` (the canonical text of `source`) with the comments attached
/// to items in `source` written back (see the module docs). Returns
/// `canonical` unchanged when `source` has none or cannot be read.
pub(crate) fn carry(source: &str, canonical: &str) -> String {
    let Ok(tree) = KdlDocument::parse_v2(source) else {
        return canonical.to_owned();
    };
    let mut attached: HashMap<String, Attached> = HashMap::new();
    walk(&tree, &mut |key, node| {
        let found = comments_of(source, node);
        if found.leading.is_empty() && found.trailing.is_none() {
            return;
        }
        let entry = attached.entry(key).or_default();
        entry.leading.extend(found.leading);
        if found.trailing.is_some() {
            entry.trailing = found.trailing;
        }
    });
    if attached.is_empty() {
        return canonical.to_owned();
    }
    let Ok(out_tree) = KdlDocument::parse_v2(canonical) else {
        return canonical.to_owned();
    };
    // (offset, text) insertions into `canonical`.
    let mut inserts: Vec<(usize, String)> = Vec::new();
    walk(&out_tree, &mut |key, node| {
        let Some(comments) = attached.remove(&key) else {
            return;
        };
        let start = node.span().offset();
        let line_start = canonical[..start].rfind('\n').map_or(0, |i| i + 1);
        let indent = &canonical[line_start..start];
        if !indent.trim().is_empty() {
            return;
        }
        if !comments.leading.is_empty() {
            let mut text = String::new();
            for line in &comments.leading {
                text.push_str(indent);
                text.push_str(line);
                text.push('\n');
            }
            inserts.push((line_start, text));
        }
        if let Some(trailing) = comments.trailing {
            let line_end = canonical[start..]
                .find('\n')
                .map_or(canonical.len(), |i| start + i);
            inserts.push((line_end, format!(" {trailing}")));
        }
    });
    // Stable: leading text before a trailing comment at the same offset
    // cannot happen (a line start is never a line end of another item).
    inserts.sort_by_key(|(at, _)| *at);
    let mut out = String::with_capacity(canonical.len() + 64 * inserts.len());
    let mut done = 0;
    for (at, text) in inserts {
        out.push_str(&canonical[done..at]);
        out.push_str(&text);
        done = at;
    }
    out.push_str(&canonical[done..]);
    out
}

/// Call `f` with the path and node of every item (see the module docs).
fn walk(tree: &KdlDocument, f: &mut impl FnMut(String, &KdlNode)) {
    for room in tree.nodes().iter().filter(|n| n.name().value() == "room") {
        for child in children(room) {
            match child.name().value() {
                "network" => {
                    f("network:".to_owned(), child);
                    network(child, "", f);
                }
                "meta" => {
                    f("meta".to_owned(), child);
                    for entry in children(child).filter(|n| n.name().value() == "node") {
                        if let Some(name) = arg(entry) {
                            f(format!("meta:{name}"), entry);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// The items of a `network` block owned by the node with id `owner`
/// (empty for the root network).
fn network(block: &KdlNode, owner: &str, f: &mut impl FnMut(String, &KdlNode)) {
    for item in children(block) {
        match item.name().value() {
            "node" => {
                let Some(id) = prop(item, "id") else {
                    continue;
                };
                let id = id.to_ascii_lowercase();
                f(format!("node:{id}"), item);
                for child in children(item) {
                    match child.name().value() {
                        "input" => {
                            if let Some(port) = arg(child) {
                                f(format!("input:{id}:{port}"), child);
                            }
                        }
                        "charm" => f(format!("charm:{id}"), child),
                        "network" => {
                            f(format!("network:{id}"), child);
                            network(child, &id, f);
                        }
                        _ => {}
                    }
                }
            }
            "link" => {
                if let (Some(from), Some(to)) = (prop(item, "from"), prop(item, "to")) {
                    // Ends are separated by a character that names (and
                    // so these strings) cannot hold unescaped: a newline.
                    f(format!("link:{owner}\n{from}\n{to}"), item);
                }
            }
            _ => {}
        }
    }
}

fn children(node: &KdlNode) -> impl Iterator<Item = &KdlNode> {
    node.children().into_iter().flat_map(|d| d.nodes().iter())
}

/// The first argument, if it is a string.
fn arg(node: &KdlNode) -> Option<&str> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string())
}

/// The string property `key`.
fn prop<'a>(node: &'a KdlNode, key: &str) -> Option<&'a str> {
    node.entries()
        .iter()
        .find(|e| e.name().is_some_and(|n| n.value() == key))
        .and_then(|e| e.value().as_string())
}

/// The leading and trailing comments of `node` in `source`.
fn comments_of(source: &str, node: &KdlNode) -> Attached {
    let start = node.span().offset();
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    if !source[line_start..start].trim().is_empty() {
        // Not the first thing on its line: nothing is attached.
        return Attached::default();
    }
    let mut leading = Vec::new();
    let mut end = line_start;
    while end > 0 {
        let above = &source[..end - 1];
        let from = above.rfind('\n').map_or(0, |i| i + 1);
        let line = above[from..].trim();
        if !line.starts_with("//") {
            break;
        }
        leading.push(line.to_owned());
        end = from;
    }
    leading.reverse();

    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |i| start + i);
    let name = node.name().span();
    let header_end = node
        .entries()
        .iter()
        .map(|e| e.span().offset() + e.span().len())
        .chain([name.offset() + name.len()])
        .filter(|&end| end <= line_end)
        .max()
        .unwrap_or(line_end);
    let rest = source[header_end..line_end].trim_start();
    let rest = rest.strip_prefix(['{', ';']).map_or(rest, str::trim_start);
    let trailing = rest.starts_with("//").then(|| rest.trim_end().to_owned());
    Attached { leading, trailing }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_and_trailing() {
        let src = "room \"r\" {\n  // a\n  //b\n  node \"x\" id=\"1\" { // t\n    input \"p\" 1 // q\n  }\n}\n";
        let tree = KdlDocument::parse_v2(src).unwrap();
        let mut seen = Vec::new();
        walk(&tree, &mut |key, node| {
            let c = comments_of(src, node);
            seen.push((key, c.leading, c.trailing));
        });
        assert!(seen.is_empty(), "no network: nothing walked");
        let src = "room \"r\" {\n  network \"/\" {\n    // a\n    //b\n    node \"x\" id=\"AB\" { // t\n      input \"p\" 1 // q\n    }\n    link from=\"x.o\" to=\"y.i\"; // kept\n  }\n}\n";
        let tree = KdlDocument::parse_v2(src).unwrap();
        walk(&tree, &mut |key, node| {
            let c = comments_of(src, node);
            seen.push((key, c.leading, c.trailing));
        });
        assert_eq!(
            seen,
            [
                ("network:".to_owned(), vec![], None),
                (
                    "node:ab".to_owned(),
                    vec!["// a".to_owned(), "//b".to_owned()],
                    Some("// t".to_owned())
                ),
                ("input:ab:p".to_owned(), vec![], Some("// q".to_owned())),
                (
                    "link:\nx.o\ny.i".to_owned(),
                    vec![],
                    Some("// kept".to_owned())
                ),
            ]
        );
    }
}
