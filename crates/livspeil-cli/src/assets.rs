// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil assets add|check` and the asset part of `validate`.
//!
//! The rules (lock syntax, license policy) live in
//! [`livspeil_doc::assets`]; this module only runs them and prints.

use std::path::Path;
use std::process::ExitCode;

use livspeil_doc::assets::{ASSETS_LOCK_FILE, AssetIssue, Severity};
use livspeil_doc::{AssetsError, ROOM_FILE, add_asset, check_assets, package_dir};

use crate::usage;

/// Entry point for `livspeil assets ARGS`.
pub fn run(mut args: impl Iterator<Item = String>) -> ExitCode {
    match args.next().as_deref() {
        Some("add") => add(args),
        Some("check") => check(args),
        Some("-h" | "--help") => {
            print!("{}", crate::HELP);
            ExitCode::SUCCESS
        }
        Some(other) => usage(&format!(
            "assets: unknown subcommand '{other}' (add or check)"
        )),
        None => usage("assets needs a subcommand: add or check"),
    }
}

fn add(args: impl Iterator<Item = String>) -> ExitCode {
    let mut positional = Vec::new();
    let mut license = None;
    let mut source = None;
    let mut author = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let slot = match arg.as_str() {
            "--license" => &mut license,
            "--source" => &mut source,
            "--author" => &mut author,
            flag if flag.starts_with('-') => {
                return usage(&format!("assets add: unknown option '{flag}'"));
            }
            _ => {
                positional.push(arg);
                continue;
            }
        };
        match args.next() {
            Some(value) => *slot = Some(value),
            None => return usage(&format!("assets add: {arg} needs a value")),
        }
    }
    let [room, file] = positional.as_slice() else {
        return usage("assets add takes <room> <file>");
    };
    let Some(license) = license else {
        return usage("assets add: --license SPDX is required (for example --license CC0-1.0)");
    };
    let dir = package_dir(Path::new(room));
    if !dir.join(ROOM_FILE).is_file() {
        eprintln!("{}: not a room package (no {ROOM_FILE})", dir.display());
        return ExitCode::FAILURE;
    }
    match add_asset(
        &dir,
        Path::new(file),
        &license,
        source.as_deref().unwrap_or("original"),
        author.as_deref(),
    ) {
        Ok(added) => {
            if !added.unlisted_licenses.is_empty() {
                eprintln!(
                    "warning: license {} not in the allow-list; the official store will not accept it",
                    added.unlisted_licenses.join(", ")
                );
            }
            let verb = if added.added { "added" } else { "unchanged" };
            println!("{}: {verb} {}", dir.display(), added.entry.path);
            ExitCode::SUCCESS
        }
        Err(err) => {
            report_error(&dir, &err);
            ExitCode::FAILURE
        }
    }
}

fn check(args: impl Iterator<Item = String>) -> ExitCode {
    let mut strict = false;
    let mut rooms = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--strict" => strict = true,
            flag if flag.starts_with('-') => {
                return usage(&format!("assets check: unknown option '{flag}'"));
            }
            _ => rooms.push(arg),
        }
    }
    let [room] = rooms.as_slice() else {
        return usage("assets check takes exactly one <room>");
    };
    let dir = package_dir(Path::new(room));
    match verify_package(&dir, strict) {
        Some(true) => ExitCode::SUCCESS,
        Some(false) => ExitCode::FAILURE,
        None => {
            println!("{}: no assets", dir.display());
            ExitCode::SUCCESS
        }
    }
}

/// Used by `validate`: verify the package's assets if it has any. Prints
/// issues on stderr and a summary on stdout; false if there are errors.
pub fn validate(path: &Path) -> bool {
    verify_package(&package_dir(path), false).unwrap_or(true)
}

/// `None` if the package has no assets; else whether it passed (no errors,
/// and with `strict` no warnings either).
fn verify_package(dir: &Path, strict: bool) -> Option<bool> {
    let (lock, issues) = match check_assets(dir) {
        Ok(Some(result)) => result,
        Ok(None) => return None,
        Err(err) => {
            report_error(dir, &err);
            return Some(false);
        }
    };
    for issue in &issues {
        print_issue(dir, issue);
    }
    let errors = issues
        .iter()
        .filter(|i| i.severity() == Severity::Error)
        .count();
    let warnings = issues.len() - errors;
    let lock_file = dir.join(ASSETS_LOCK_FILE);
    let n = lock.assets.len();
    if errors == 0 {
        println!(
            "{}: ok ({n} asset{}, {warnings} warning{})",
            lock_file.display(),
            if n == 1 { "" } else { "s" },
            if warnings == 1 { "" } else { "s" },
        );
    }
    Some(errors == 0 && (!strict || warnings == 0))
}

/// `assets.lock:line:col: severity: path: message` for listed assets,
/// `room/assets/name: severity: message` for files only on disk.
fn print_issue(dir: &Path, issue: &AssetIssue) {
    let lock_file = dir.join(ASSETS_LOCK_FILE);
    match issue.at {
        Some(at) => eprintln!(
            "{}:{at}: {}: {}: {}",
            lock_file.display(),
            issue.severity(),
            issue.path,
            issue.kind
        ),
        None => eprintln!(
            "{}: {}: {}",
            dir.join(&issue.path).display(),
            issue.severity(),
            issue.kind
        ),
    }
}

fn report_error(dir: &Path, err: &AssetsError) {
    match err {
        AssetsError::Lock(doc) if doc.location().is_some() => {
            eprintln!("{}:{doc}", dir.join(ASSETS_LOCK_FILE).display());
        }
        AssetsError::Lock(doc) => eprintln!("{}: {doc}", dir.join(ASSETS_LOCK_FILE).display()),
        other => eprintln!("error: {other}"),
    }
}
