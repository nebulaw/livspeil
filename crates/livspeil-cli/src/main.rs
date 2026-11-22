// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Headless command line for Livspeil documents.
//!
//! Exit codes: 0 success, 1 the document is invalid (or not canonical for
//! `fmt --check`) or could not be read or written, 2 usage error, 3 `cook`
//! printed at least one node error. `diff` follows `git diff --exit-code`
//! instead: 0 equal, 1 different, 2 usage error, 3 load error.
//!
//! See the `cook` module for the cook clock, sinks and output formats, the
//! `diff` module for the JSONL schema and `assets` for `assets.lock`.

mod assets;
mod cook;
mod diff;

use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use livspeil_doc::{DocError, canonicalize, parse, room_file};
use livspeil_nodes_core::core_registry;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
livspeil: headless tools for Livspeil documents

USAGE:
    livspeil <COMMAND> [ARGS]

COMMANDS:
    validate <path>         Check a document against the core nodes, and
                            its assets.lock and assets/ when present
    fmt <path> [--check]    Rewrite a document in canonical form; with
                            --check, write nothing and fail if it is not
    cook <path> [OPTIONS]   Cook a document over a frame clock and print
                            the outputs of its sinks (nodes with no
                            outgoing links)
    diff <a> <b> [OPTIONS]  Semantic diff of two documents (nodes matched
                            by id)
    assets add <room> <file> --license SPDX [--source URL] [--author NAME]
                            Copy a file into <room>/assets/<sha256>.<ext>
                            and record it in assets.lock (source defaults
                            to \"original\")
    assets check <room> [--strict]
                            Verify assets.lock against assets/ (hashes,
                            sizes, unlisted files, licenses); --strict
                            also fails on warnings

<path> is a room package directory (Name.room/) or its room.kdl file.

COOK OPTIONS:
    --frames N              Frames to cook (default 1)
    --fps F                 Frames per second (default 60); frame i cooks
                            at time = S + i/F seconds with dt = 1/F
    --start S               Time of frame 0 in seconds (default 0)
    --every K               Print only frames i with i % K == 0 (default 1);
                            every frame is still cooked
    --node NAME             Print this node instead of the sinks
                            (repeatable)
    --format text|jsonl     One line per frame and node (default), or one
                            JSON object per frame
    --stats                 Also print cooked/reused node counts per frame
    --event NODE.PORT@FRAME Inject an event (no payload) in frame FRAME
                            into the Event input PORT of NODE, or from its
                            Event output PORT (repeatable); Event outputs
                            print the events fired in the frame (none,
                            event, event(<payload>))
    --fs-root DIR           File root of file-system nodes such as
                            core.fs.list_dir (default: the room package
                            directory); paths outside it are refused

    Floats are printed with 6 decimals. Example:
        livspeil cook examples/sun-and-lamp.room --frames 600 --every 50

DIFF OPTIONS:
    --format text|jsonl     One line per change (default) or one JSON
                            object per change (schema in the README)
    --meta                  Also report editor positions (meta), which are
                            hidden by default

    Text lines start with + (added), - (removed) or ~ (changed), e.g.
        ~ node \"lamp\" input \"a\": 0.2 -> 0.3

ASSET LICENSES:
    license= is an SPDX expression. Allowed: CC0-1.0, CC-BY-4.0,
    CC-BY-SA-4.0, OFL-1.1, MIT, Apache-2.0, MPL-2.0, GPL-3.0-or-later,
    LGPL-3.0-or-later. Other valid ids are warnings; a malformed
    expression, NONE or NOASSERTION is an error.

OPTIONS:
    -h, --help       Print this help
    -V, --version    Print version

EXIT CODES:
    0 success
    1 invalid or non-canonical document, asset errors, or an I/O error
    2 usage error (including an unknown --node name)
    3 cook: at least one printed value is a node error

    diff (like git diff --exit-code): 0 no differences, 1 differences,
    2 usage error, 3 a document cannot be read or parsed
";

fn main() -> ExitCode {
    run(std::env::args().skip(1).collect())
}

fn run(args: Vec<String>) -> ExitCode {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None | Some("-h" | "--help") => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("livspeil {VERSION}");
            ExitCode::SUCCESS
        }
        Some("validate") => match args.collect::<Vec<_>>().as_slice() {
            [path] if !path.starts_with('-') => validate(Path::new(path)),
            _ => usage("validate takes exactly one <path>"),
        },
        Some("fmt") => {
            let mut check = false;
            let mut paths = Vec::new();
            for arg in args {
                match arg.as_str() {
                    "--check" => check = true,
                    flag if flag.starts_with('-') => {
                        return usage(&format!("fmt: unknown option '{flag}'"));
                    }
                    _ => paths.push(arg),
                }
            }
            match paths.as_slice() {
                [path] => fmt(Path::new(path), check),
                _ => usage("fmt takes exactly one <path>"),
            }
        }
        Some("cook") => cook::run(args),
        Some("diff") => diff::run(args),
        Some("assets") => assets::run(args),
        Some(other) => usage(&format!("unknown command or option '{other}'")),
    }
}

fn usage(message: &str) -> ExitCode {
    eprintln!("livspeil: {message}\n");
    eprint!("{HELP}");
    ExitCode::from(2)
}

fn read(path: &Path) -> Result<(std::path::PathBuf, String), ExitCode> {
    let file = room_file(path);
    match fs::read_to_string(&file) {
        Ok(text) => Ok((file, text)),
        Err(err) => {
            eprintln!("{}: cannot read: {err}", file.display());
            Err(ExitCode::FAILURE)
        }
    }
}

/// `file:line:col: message` (the compiler convention) or `file: message`.
fn report(file: &Path, err: &DocError) {
    if err.location().is_some() {
        eprintln!("{}:{err}", file.display());
    } else {
        eprintln!("{}: {err}", file.display());
    }
}

fn validate(path: &Path) -> ExitCode {
    let (file, text) = match read(path) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let result = parse(&text).and_then(|doc| {
        let graph = doc.to_graph(Arc::new(core_registry()))?;
        Ok((graph.node_count(), graph.link_count()))
    });
    match result {
        Ok((nodes, links)) => {
            println!("{}: ok ({nodes} nodes, {links} links)", file.display());
            if assets::validate(path) {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(err) => {
            report(&file, &err);
            ExitCode::FAILURE
        }
    }
}

fn fmt(path: &Path, check: bool) -> ExitCode {
    let (file, text) = match read(path) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let canonical = match canonicalize(&text) {
        Ok(c) => c,
        Err(err) => {
            report(&file, &err);
            return ExitCode::FAILURE;
        }
    };
    if canonical == text {
        return ExitCode::SUCCESS;
    }
    if check {
        eprintln!(
            "{}: not in canonical form (run `livspeil fmt` to fix)",
            file.display()
        );
        return ExitCode::FAILURE;
    }
    match fs::write(&file, canonical) {
        Ok(()) => {
            println!("{}: formatted", file.display());
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{}: cannot write: {err}", file.display());
            ExitCode::FAILURE
        }
    }
}
