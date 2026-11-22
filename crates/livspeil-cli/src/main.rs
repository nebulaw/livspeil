// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Headless command line for Livspeil documents.
//!
//! Subcommands are reserved and report which milestone will implement them.

use std::process::ExitCode;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
livspeil: headless tools for Livspeil documents

USAGE:
    livspeil <COMMAND>

COMMANDS (reserved, not implemented yet):
    cook        Cook a document and print sink outputs   (milestone M0.6)
    validate    Check a document for errors              (milestone M0.4)
    fmt         Rewrite a document in canonical form     (milestone M0.4)
    diff        Semantic diff of two documents           (later)

OPTIONS:
    -h, --help       Print this help
    -V, --version    Print version
";

fn main() -> ExitCode {
    run(std::env::args().skip(1))
}

fn run(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        None | Some("-h" | "--help") => {
            print!("{HELP}");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("livspeil {VERSION}");
            ExitCode::SUCCESS
        }
        Some(cmd @ ("cook" | "validate" | "fmt" | "diff")) => {
            eprintln!("livspeil {cmd}: not implemented yet");
            ExitCode::from(2)
        }
        Some(other) => {
            eprintln!("livspeil: unknown command or option '{other}'\n");
            eprint!("{HELP}");
            ExitCode::from(2)
        }
    }
}
