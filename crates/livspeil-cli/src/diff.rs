// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil diff <a> <b> [--format text|jsonl] [--meta]`.
//!
//! The diff itself is [`livspeil_doc::diff()`]; this module loads the two
//! documents and renders the changes.
//!
//! Text: one line per change, `+` added, `-` removed, `~` changed, values
//! in canonical KDL text (see [`livspeil_doc::Change`]'s `Display`). Nodes
//! are named by path (`lamp`, `desk_lamp/sunrise`); links inside a subnet
//! end with `in "/desk_lamp"`.
//!
//! JSONL: one object per change, keys in this order, absent keys left out.
//! Values are strings except `cells` and `row`, which are numbers. New
//! kinds and keys may be added; existing ones keep their meaning.
//!
//! | key       | when                          | value |
//! |-----------|-------------------------------|-------|
//! | `op`      | always                        | `"added"`, `"removed"`, `"changed"` |
//! | `kind`    | always                        | `room-name`, `format`, `min-tier`, `node`, `node-name`, `node-type`, `node-move`, `port-type`, `charm`, `charm-field`, `charm-capability`, `input`, `expr`, `anim`, `anim-setting`, `anim-key`, `table`, `table-column`, `table-row`, `curve-setting`, `curve-key`, `link`, `position` |
//! | `node`    | node kinds, `position`        | node name (new name if renamed); for `position` the meta key, which is the node's path |
//! | `id`      | node kinds, `position`        | node UUID |
//! | `path`    | node kinds, node in a subnet  | node path from the root without the leading `/` (`desk_lamp/sunrise`) |
//! | `port`    | `input`, `expr`, `anim*`, `table*`, `curve*` | input port |
//! | `setting` | `anim-setting`, `curve-setting` | `pre`, `post`, `offset` |
//! | `time`    | `anim-key`, `curve-key`       | key time (curve: position), canonical float text |
//! | `field`   | `charm-field`                 | `name`, `version`, `icon`, `min-tier`, `capabilities` (reordered only) |
//! | `cells`   | `table`                       | number of changed cells; rows and columns are listed only when at most 20 |
//! | `column`  | `table-column`                | column name |
//! | `row`     | `table-row`                   | row index from 0 |
//! | `from`, `to` | `link`                     | `node.port`, names in the link's network |
//! | `network` | `link` in a subnet            | network path (`/desk_lamp`) |
//! | `old`, `new` | unless added / removed     | canonical KDL text of the value (`"0.2"`, `"\"text\""`, `"(color)\"oklch(...)\""`), a summary for `anim` (`"3 keys 0.0..2.0"`), `charm` (`"\"Daylight\" version=\"1.0\" capability=\"time\""`), `table` (`"2 cols x 3 rows"`) and Table/Curve `input` values (`"table 2 cols x 3 rows"`, `"curve 2 keys 0.0..1.0"`), the row's cells for `table-row` (`"\"b\" 2.5"`), the quoted network path for `node-move`, `"x y"` for `position`, the quoted type id for `node` |

use std::path::Path;
use std::process::ExitCode;

use livspeil_doc::{Change, Document, ROOT_NETWORK, Subject, diff, parse, room_file};

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
    let text = |s: &str| json_string(s);
    let mut fields: Vec<(&str, String)> = vec![
        ("op", text(change.op.name())),
        ("kind", text(change.subject.kind())),
    ];
    if let Some(node) = change.subject.node() {
        fields.push(("node", text(&node.name)));
        fields.push(("id", text(&node.id.to_string())));
        if node.is_nested() {
            fields.push(("path", text(&node.path)));
        }
    }
    if let Subject::Position { node, id } = &change.subject {
        fields.push(("node", text(node)));
        if let Some(id) = id {
            fields.push(("id", text(&id.to_string())));
        }
    }
    if let Some(port) = change.subject.port() {
        fields.push(("port", text(port)));
    }
    match &change.subject {
        Subject::AnimSetting { setting, .. } | Subject::CurveSetting { setting, .. } => {
            fields.push(("setting", text(setting)));
        }
        Subject::AnimKey { time, .. } | Subject::CurveKey { time, .. } => {
            fields.push(("time", text(time)));
        }
        Subject::CharmField { field, .. } => fields.push(("field", text(field))),
        Subject::Table { cells, .. } => fields.push(("cells", cells.to_string())),
        Subject::TableColumn { column, .. } => fields.push(("column", text(column))),
        Subject::TableRow { row, .. } => fields.push(("row", row.to_string())),
        Subject::Link { from, to, network } => {
            fields.push(("from", text(from)));
            fields.push(("to", text(to)));
            if network != ROOT_NETWORK {
                fields.push(("network", text(network)));
            }
        }
        _ => {}
    }
    if let Some(old) = &change.old {
        fields.push(("old", text(old)));
    }
    if let Some(new) = &change.new {
        fields.push(("new", text(new)));
    }
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}:{v}", json_string(k)))
        .collect();
    format!("{{{}}}", body.join(","))
}
