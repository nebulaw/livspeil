// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil diff <a> <b> [--format text|jsonl] [--meta]`.
//!
//! The diff itself is [`livspeil_doc::diff()`]; this module loads the two
//! documents and renders the changes.
//!
//! Text: one line per change, `+` added, `-` removed, `~` changed, values
//! in canonical KDL text (see [`livspeil_doc::Change`]'s `Display`).
//!
//! JSONL: one object per change, keys in this order, absent keys left out:
//!
//! | key       | when                          | value |
//! |-----------|-------------------------------|-------|
//! | `op`      | always                        | `"added"`, `"removed"`, `"changed"` |
//! | `kind`    | always                        | `room-name`, `format`, `min-tier`, `node`, `node-name`, `node-type`, `input`, `expr`, `anim`, `anim-setting`, `anim-key`, `link`, `position` |
//! | `node`    | node kinds, `position`        | node name (new name if renamed) |
//! | `id`      | node kinds, `position`        | node UUID |
//! | `port`    | `input`, `expr`, `anim*`      | input port |
//! | `setting` | `anim-setting`                | `pre`, `post`, `offset` |
//! | `time`    | `anim-key`                    | key time, canonical float text |
//! | `from`, `to` | `link`                     | `node.port` |
//! | `old`, `new` | unless added / removed     | canonical KDL text of the value (`"0.2"`, `"\"text\""`, `"(color)\"oklch(...)\""`), a summary for `anim` (`"3 keys 0.0..2.0"`), `"x y"` for `position`, the quoted type id for `node` |

use std::path::Path;
use std::process::ExitCode;

use livspeil_doc::{Change, Document, Subject, diff, parse, room_file};

use crate::usage;

/// Exit code when the documents differ.
const EXIT_DIFFERENT: u8 = 1;
/// Exit code when a document cannot be read or parsed.
const EXIT_LOAD: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Text,
    Jsonl,
}

/// Entry point for `livspeil diff ARGS`.
pub fn run(args: impl Iterator<Item = String>) -> ExitCode {
    let mut paths = Vec::new();
    let mut format = Format::Text;
    let mut meta = false;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{}", crate::HELP);
                return ExitCode::SUCCESS;
            }
            "--meta" => meta = true,
            "--format" => match args.next().as_deref() {
                Some("text") => format = Format::Text,
                Some("jsonl") => format = Format::Jsonl,
                Some(other) => {
                    return usage(&format!(
                        "diff: --format must be text or jsonl, not '{other}'"
                    ));
                }
                None => return usage("diff: --format needs a value (text or jsonl)"),
            },
            flag if flag.starts_with('-') => {
                return usage(&format!("diff: unknown option '{flag}'"));
            }
            _ => paths.push(arg),
        }
    }
    let [a, b] = paths.as_slice() else {
        return usage("diff takes exactly two paths: <a> <b>");
    };
    let (old, new) = match (load(Path::new(a)), load(Path::new(b))) {
        (Ok(old), Ok(new)) => (old, new),
        _ => return ExitCode::from(EXIT_LOAD),
    };
    let result = diff(&old, &new);
    let changes: Vec<&Change> = result
        .changes
        .iter()
        .filter(|c| meta || !c.subject.is_meta())
        .collect();
    let mut out = String::new();
    for change in &changes {
        match format {
            Format::Text => out.push_str(&change.to_string()),
            Format::Jsonl => out.push_str(&json(change)),
        }
        out.push('\n');
    }
    print!("{out}");
    if changes.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(EXIT_DIFFERENT)
    }
}

/// Read and parse a document, reporting errors on stderr.
fn load(path: &Path) -> Result<Document, ()> {
    let file = room_file(path);
    let text = std::fs::read_to_string(&file).map_err(|err| {
        eprintln!("{}: cannot read: {err}", file.display());
    })?;
    parse(&text).map_err(|err| crate::report(&file, &err))
}

fn json_string(s: &str) -> String {
    // Serializing a string cannot fail; the fallback keeps output valid JSON.
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_owned())
}

/// One JSON object, keys in the documented order.
fn json(change: &Change) -> String {
    let mut fields: Vec<(&str, String)> = vec![
        ("op", change.op.name().to_owned()),
        ("kind", change.subject.kind().to_owned()),
    ];
    if let Some(node) = change.subject.node() {
        fields.push(("node", node.name.clone()));
        fields.push(("id", node.id.to_string()));
    }
    if let Subject::Position { node, id } = &change.subject {
        fields.push(("node", node.clone()));
        if let Some(id) = id {
            fields.push(("id", id.to_string()));
        }
    }
    if let Some(port) = change.subject.port() {
        fields.push(("port", port.to_owned()));
    }
    match &change.subject {
        Subject::AnimSetting { setting, .. } => fields.push(("setting", (*setting).to_owned())),
        Subject::AnimKey { time, .. } => fields.push(("time", time.clone())),
        Subject::Link { from, to } => {
            fields.push(("from", from.clone()));
            fields.push(("to", to.clone()));
        }
        _ => {}
    }
    if let Some(old) = &change.old {
        fields.push(("old", old.clone()));
    }
    if let Some(new) = &change.new {
        fields.push(("new", new.clone()));
    }
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}:{}", json_string(k), json_string(v)))
        .collect();
    format!("{{{}}}", body.join(","))
}
