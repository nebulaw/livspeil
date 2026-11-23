// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Headless command line for Livspeil documents.
//!
//! Exit codes: 0 success, 1 the document is invalid (or not canonical for
//! `fmt --check`) or could not be read or written, 2 usage error, 3 `cook`
//! printed at least one node error, 4 `run`: the script failed. `diff`
//! follows `git diff --exit-code` instead: 0 equal, 1 different, 2 usage
//! error, 3 load error.
//!
//! See the `cook` module for the cook clock, sinks and output formats, the
//! `diff` module for the JSONL schema, `assets` for `assets.lock` and `run`
//! for scripts.

mod assets;
mod cook;
mod diff;
mod run;

use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use livspeil_doc::assets::ASSETS_LOCK_FILE;
use livspeil_doc::{
    DocError, canonicalize, canonicalize_assets_lock, package_dir, parse, room_file,
};
use livspeil_nodes_core::core_registry;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
livspeil: headless tools for Livspeil documents

USAGE:
    livspeil <COMMAND> [ARGS]

COMMANDS:
    validate <path>         Check a document against the core nodes, and
                            its assets.lock and assets/ when present
    fmt <path> [--check]    Rewrite a document (and the package's
                            assets.lock, if any) in canonical form; with
                            --check, write nothing and fail if either is not
    cook <path> [OPTIONS]   Cook a document over a frame clock and print
                            the outputs of its sinks (nodes with no
                            outgoing links)
    diff <a> <b> [OPTIONS]  Semantic diff of two documents (nodes matched
                            by id, also inside subnets)
    assets add <room> <file> --license SPDX [--source URL] [--author NAME]
               [--replace]  Copy a file into <room>/assets/<sha256>.<ext>
                            and record it in assets.lock (source defaults
                            to \"original\"); an entry already listed with
                            other data is refused unless --replace, which
                            updates it
    assets check <room> [--strict]
                            Verify assets.lock against assets/ (hashes,
                            sizes, unlisted files, licenses); --strict
                            also fails on warnings
    run <room> <script.py> [OPTIONS]
                            Run a Python script against a room through the
                            script worker protocol and print its logs and
                            the commands it applied (journaled with origin
                            \"script\"); nothing is written without --save

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

    Text lines start with + (added), - (removed) or ~ (changed). Nodes
    inside subnets are named by path; tables are summarized (with rows
    and columns when at most 20 cells changed); curves are diffed per key:
        ~ node \"lamp\" input \"a\": 0.2 -> 0.3
        ~ node \"desk/x\" moved: \"/\" -> \"/desk\"
        + node \"desk\" charm capability: \"net\"
        ~ node \"t\" input \"rows\" table: 2 cols x 3 rows -> 2 cols x 4 rows (2 cells changed)
        + link \"in.value\" -> \"x.a\" in \"/desk\"

RUN OPTIONS:
    --grant CAP             Grant a capability (repeatable, or comma
                            separated): graph.read, graph.write,
                            graph.cook, fs.read:<dir>. Default: none
    --save                  Write the edited room back in canonical form
                            (not done when the script fails)
    --journal FILE          Append the applied commands to FILE as
                            journal lines
    --ticks N               If the script listens (on_tick/on_event), drive
                            it for N frames (default 0), then shut it down
    --fps F                 Frame rate of those ticks (default 60)
    --python CMD            Interpreter (default $LIVSPEIL_PYTHON or
                            python3); the SDK is put on PYTHONPATH
    --timeout SECS          Total run time limit (default 60)
    --request-timeout SECS  Limit per tick/event handler (default 5)

    The script runs in the room package directory, out of process; it can
    change the room only through commands, so each is validated and
    undoable. Example:
        livspeil run examples/sun-and-lamp.room examples/scripts/dim_lamps.py \\
            --grant graph.read,graph.write

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
    4 run: the script failed (crash, timeout, protocol error)

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
        Some("run") => run::run(args),
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
    let room = fmt_file(&file, &text, check, canonicalize);
    // The package's assets.lock, when present, is checked and written too.
    let lock_file = package_dir(&file).join(ASSETS_LOCK_FILE);
    let lock = match fs::read_to_string(&lock_file) {
        Ok(text) => fmt_file(&lock_file, &text, check, canonicalize_assets_lock),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => true,
        Err(err) => {
            eprintln!("{}: cannot read: {err}", lock_file.display());
            false
        }
    };
    if room && lock {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// Check or rewrite one file in canonical form; false on failure.
fn fmt_file(
    file: &Path,
    text: &str,
    check: bool,
    canonical: fn(&str) -> Result<String, DocError>,
) -> bool {
    let canonical = match canonical(text) {
        Ok(c) => c,
        Err(err) => {
            report(file, &err);
            return false;
        }
    };
    if canonical == text {
        return true;
    }
    if check {
        eprintln!(
            "{}: not in canonical form (run `livspeil fmt` to fix)",
            file.display()
        );
        return false;
    }
    match fs::write(file, canonical) {
        Ok(()) => {
            println!("{}: formatted", file.display());
            true
        }
        Err(err) => {
            eprintln!("{}: cannot write: {err}", file.display());
            false
        }
    }
}
