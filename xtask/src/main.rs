// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Project automation. Run with `cargo xtask <task>`.

use std::process::{Command, ExitCode, Stdio};

const HELP: &str = "\
cargo xtask <task>

TASKS:
    check    fmt --check, clippy -D warnings, tests, then license checks when
             the tools are installed (cargo-deny, reuse)
    fmt      Format the whole workspace
    test     Run all tests
    deny     Run cargo-deny (license and advisory checks)
    reuse    Run reuse lint (SPDX compliance)
";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let ok = match task.as_deref() {
        Some("check") => check(),
        Some("fmt") => run("cargo", &["fmt", "--all"]),
        Some("test") => run("cargo", &["test", "--workspace"]),
        Some("deny") => run_required("cargo", &["deny", "check"], "cargo-deny"),
        Some("reuse") => run_required("reuse", &["lint"], "reuse"),
        None | Some("help" | "-h" | "--help") => {
            print!("{HELP}");
            true
        }
        Some(other) => {
            eprintln!("unknown task '{other}'\n");
            eprint!("{HELP}");
            false
        }
    };
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn check() -> bool {
    run("cargo", &["fmt", "--all", "--", "--check"])
        && run(
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        )
        && run("cargo", &["test", "--workspace"])
        && run_optional("cargo", &["deny", "check"], "cargo-deny")
        && run_optional("reuse", &["lint"], "reuse")
}

/// Run a command, inheriting stdio. Returns whether it succeeded.
fn run(program: &str, args: &[&str]) -> bool {
    eprintln!("$ {program} {}", args.join(" "));
    match Command::new(program).args(args).status() {
        Ok(status) => status.success(),
        Err(err) => {
            eprintln!("failed to start {program}: {err}");
            false
        }
    }
}

/// Like [`run`], but a missing tool is a failure with an install hint.
fn run_required(program: &str, args: &[&str], tool: &str) -> bool {
    if !tool_available(program, args.first().copied()) {
        eprintln!("{tool} is not installed");
        return false;
    }
    run(program, args)
}

/// Like [`run`], but a missing tool is skipped (CI runs these as separate jobs).
fn run_optional(program: &str, args: &[&str], tool: &str) -> bool {
    if !tool_available(program, args.first().copied()) {
        eprintln!("skipping {tool}: not installed");
        return true;
    }
    run(program, args)
}

/// For `cargo <sub>` tools, probe `cargo <sub> --version`; otherwise
/// `<program> --version`.
fn tool_available(program: &str, subcommand: Option<&str>) -> bool {
    let mut cmd = Command::new(program);
    if program == "cargo" {
        if let Some(sub) = subcommand {
            cmd.arg(sub);
        }
    }
    cmd.arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.status().is_ok_and(|status| status.success())
}
