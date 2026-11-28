// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! `livspeil run <room> <script> [OPTIONS]`: run a script against a room
//! through the script worker protocol (`livspeil-script`).
//!
//! The room is loaded into a scene; the script's commands go through the
//! command bus with origin `script`. Afterwards the command prints the
//! script's log lines and a summary of the applied commands:
//!
//! ```text
//! log info: dimming 1 lamp by 20%
//! applied 1 command (1 undo step):
//!   transaction (2 steps)
//!     set_input lamp_intensity.out_max = 0.16
//!     set_input lamp_intensity.out_min = 0.8
//! script dim_lamps.py exited with status 0
//! ```
//!
//! With `--save` the edited room is written back canonically (prolog
//! comments and editor positions kept; positions follow renames). With
//! `--journal FILE` the applied commands are appended to FILE as journal
//! lines. A script that calls `listen()` is driven for `--ticks N` frames
//! (`--fps`, default 60) and then shut down.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use livspeil_cmd::{Command, JournalWriter};
use livspeil_doc::{Document, canonicalize, format, package_dir, parse, split_prolog};
use livspeil_graph::{Graph, NodeId};
use livspeil_node_api::{CookContext, Value};
use livspeil_nodes_core::{StdFs, core_registry};
use livspeil_script::{
    Capabilities, LogLevel, Outcome, Scene, ScriptConfig, ScriptError, ScriptSession,
};

use crate::{read, report, usage};

/// Exit code when the script fails (crash, timeout, protocol error).
const SCRIPT_FAILED: u8 = 4;

#[derive(Debug, Default)]
struct Options {
    room: Option<String>,
    script: Option<String>,
    grants: Vec<String>,
    save: bool,
    ticks: u64,
    fps: f64,
    python: Option<String>,
    timeout: Option<f64>,
    request_timeout: Option<f64>,
    journal: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options {
        fps: 60.0,
        ..Options::default()
    };
    let mut args = args.peekable();
    let seconds = |flag: &str, v: Option<String>| -> Result<f64, String> {
        v.as_deref()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|x| x.is_finite() && *x > 0.0)
            .ok_or_else(|| format!("run: {flag} takes a positive number"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--grant" => {
                let value = args.next().ok_or("run: --grant takes a capability")?;
                opts.grants.extend(
                    value
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned),
                );
            }
            "--save" => opts.save = true,
            "--ticks" => {
                opts.ticks = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("run: --ticks takes a whole number")?;
            }
            "--fps" => opts.fps = seconds("--fps", args.next())?,
            "--python" => opts.python = Some(args.next().ok_or("run: --python takes a program")?),
            "--timeout" => opts.timeout = Some(seconds("--timeout", args.next())?),
            "--request-timeout" => {
                opts.request_timeout = Some(seconds("--request-timeout", args.next())?);
            }
            "--journal" => {
                opts.journal = Some(args.next().ok_or("run: --journal takes a file")?.into());
            }
            flag if flag.starts_with('-') => return Err(format!("run: unknown option '{flag}'")),
            _ if opts.room.is_none() => opts.room = Some(arg),
            _ if opts.script.is_none() => opts.script = Some(arg),
            _ => return Err(format!("run: unexpected argument '{arg}'")),
        }
    }
    Ok(opts)
}

pub(crate) fn run(args: impl Iterator<Item = String>) -> ExitCode {
    let opts = match parse_args(args) {
        Ok(o) => o,
        Err(msg) => return usage(&msg),
    };
    let (Some(room), Some(script)) = (opts.room.as_deref(), opts.script.as_deref()) else {
        return usage("run takes <room> <script>");
    };
    let capabilities = match Capabilities::parse(opts.grants.iter().map(String::as_str)) {
        Ok(c) => c,
        Err(err) => return usage(&format!("run: {err}")),
    };
    let room_path = Path::new(room);
    let (file, text) = match read(room_path) {
        Ok(v) => v,
        Err(code) => return code,
    };
    let doc = match parse(&text) {
        Ok(d) => d,
        Err(err) => {
            report(&file, &err);
            return ExitCode::FAILURE;
        }
    };
    let graph = match doc.to_graph(Arc::new(core_registry())) {
        Ok(g) => g,
        Err(err) => {
            report(&file, &err);
            return ExitCode::FAILURE;
        }
    };
    let script_path = match Path::new(script).canonicalize() {
        Ok(p) if p.is_file() => p,
        _ => {
            eprintln!("{script}: no such script");
            return ExitCode::FAILURE;
        }
    };

    let mut scene = Scene::new(doc.name.clone(), graph);
    if let Some(root) = capabilities.fs_root() {
        match StdFs::new(root) {
            Ok(fs) => scene.cooker.set_system(Some(Arc::new(fs))),
            Err(err) => {
                eprintln!("livspeil run: fs.read root {}: {err}", root.display());
                return ExitCode::FAILURE;
            }
        }
    }
    let mut config = ScriptConfig::python(&script_path, capabilities.clone());
    if let Some(python) = &opts.python {
        config.command.program = python.into();
    }
    // Scripts read relative paths from the room package.
    config.command.current_dir = Some(package_dir(&file));
    if let Some(t) = opts.timeout {
        config.limits.run_timeout = Duration::from_secs_f64(t);
    }
    if let Some(t) = opts.request_timeout {
        config.limits.request_timeout = Duration::from_secs_f64(t);
    }
    let name = config.name.clone();
    let tokens = capabilities.tokens();
    println!(
        "run {name} on \"{}\" (granted: {})",
        scene.name,
        if tokens.is_empty() {
            "none".to_owned()
        } else {
            tokens.join(", ")
        }
    );

    let (session, result) = drive(config, &mut scene, &opts);
    if let Some(session) = &session {
        for entry in session.logs() {
            match entry.level {
                LogLevel::Stderr => println!("stderr: {}", entry.message),
                level => println!("log {}: {}", level.name(), entry.message),
            }
        }
        if session.dropped_logs() > 0 {
            println!("({} log lines dropped)", session.dropped_logs());
        }
    }
    print_summary(&scene);

    let mut ok = true;
    match &result {
        Ok(code) => println!(
            "script {name} exited with status {}",
            code.map_or_else(|| "killed".to_owned(), |c| c.to_string())
        ),
        Err(err) => {
            eprintln!("livspeil run: {name}: {err}");
            ok = false;
        }
    }
    if let Some(path) = &opts.journal
        && let Err(err) = append_journal(path, &scene)
    {
        eprintln!("{}: cannot write journal: {err}", path.display());
        return ExitCode::FAILURE;
    }
    if !ok {
        if opts.save {
            eprintln!("{}: not saved because the script failed", file.display());
        }
        return ExitCode::from(SCRIPT_FAILED);
    }
    if opts.save
        && !scene.journal.is_empty()
        && let Err(code) = save(&file, &text, &doc, &scene.graph)
    {
        return code;
    }
    ExitCode::SUCCESS
}

/// Run the script to completion: main body, then ticks if it listens.
fn drive(
    config: ScriptConfig,
    scene: &mut Scene,
    opts: &Options,
) -> (Option<ScriptSession>, Result<Option<i32>, ScriptError>) {
    let mut session = match ScriptSession::spawn(config) {
        Ok(s) => s,
        Err(err) => return (None, Err(err)),
    };
    let outcome = match session.run(scene) {
        Ok(o) => o,
        Err(err) => return (Some(session), Err(err)),
    };
    if outcome == Outcome::Listening {
        let dt = 1.0 / opts.fps;
        for frame in 0..opts.ticks {
            let clock = CookContext {
                frame,
                time: frame as f64 * dt,
                dt,
            };
            if let Err(err) = session.tick(scene, clock) {
                return (Some(session), Err(err));
            }
        }
    }
    let result = session.shutdown(scene);
    (Some(session), result)
}

fn print_summary(scene: &Scene) {
    let n = scene.journal.len();
    let steps = scene.history.undo_len();
    println!(
        "applied {n} command{} ({steps} undo step{}){}",
        if n == 1 { "" } else { "s" },
        if steps == 1 { "" } else { "s" },
        if n == 0 { "" } else { ":" }
    );
    for entry in &scene.journal {
        print_command(&scene.graph, &entry.command, 1);
    }
}

fn print_command(graph: &Graph, command: &Command, depth: usize) {
    let indent = "  ".repeat(depth);
    if let Command::Transaction(steps) = command {
        println!("{indent}transaction ({} steps)", steps.len());
        for step in steps {
            print_command(graph, step, depth + 1);
        }
        return;
    }
    println!("{indent}{}", describe(graph, command));
}

/// A node's path (without the leading `/`) or, once deleted, its id.
fn name(graph: &Graph, id: NodeId) -> String {
    graph
        .path(id)
        .map_or_else(|| id.to_string(), |p| p.trim_start_matches('/').to_owned())
}

fn describe(graph: &Graph, command: &Command) -> String {
    let n = |id: &NodeId| name(graph, *id);
    match command {
        Command::SetInput { node, port, value } => {
            format!("set_input {}.{port} = {}", n(node), show(value))
        }
        Command::ClearInput { node, port } => format!("clear_input {}.{port}", n(node)),
        Command::Link {
            from,
            from_port,
            to,
            to_port,
        } => format!("link {}.{from_port} -> {}.{to_port}", n(from), n(to)),
        Command::Unlink { to, to_port } => format!("unlink {}.{to_port}", n(to)),
        Command::Rename { node, name } => format!("rename {} (now \"{name}\")", n(node)),
        Command::SetPosition { node, position } => match position {
            Some([x, y]) => format!("set_position {} = {x} {y}", n(node)),
            None => format!("set_position {} (unplaced)", n(node)),
        },
        Command::SetExpression { node, port, source } => {
            format!("set_expression {}.{port} = {source}", n(node))
        }
        Command::ClearExpression { node, port } => format!("clear_expression {}.{port}", n(node)),
        Command::ClearAnimation { node, port } => format!("clear_animation {}.{port}", n(node)),
        Command::SetAnimation { node, port, .. } => format!("set_animation {}.{port}", n(node)),
        Command::CreateNode { id, type_id, .. } | Command::CreateNodeIn { id, type_id, .. } => {
            format!("create_node {} ({type_id})", n(id))
        }
        Command::DeleteNode { id } => format!("delete_node {}", n(id)),
        other => {
            // Rare structural commands: show the variant name from the JSON form.
            let json = serde_json::to_value(other).unwrap_or_default();
            json.as_object()
                .and_then(|o| o.keys().next().cloned())
                .unwrap_or_else(|| "command".to_owned())
        }
    }
}

fn show(value: &Value) -> String {
    let list = |v: &[f64]| {
        v.iter()
            .map(|x| format!("{x}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    match value {
        Value::Float(x) => format!("{x:?}"),
        Value::Int(x) => x.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Vec2(v) => format!("vec2({})", list(v)),
        Value::Vec3(v) => format!("vec3({})", list(v)),
        Value::Vec4(v) => format!("vec4({})", list(v)),
        Value::Color(v) => format!("oklch({})", list(v)),
        other => format!("<{:?}>", other.port_type()),
    }
}

fn append_journal(path: &Path, scene: &Scene) -> Result<(), String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut writer = JournalWriter::new(file);
    for entry in &scene.journal {
        writer.append(entry).map_err(|e| e.to_string())?;
    }
    writer.flush().map_err(|e| e.to_string())
}

/// Write the edited graph back to `file` in canonical form, keeping the
/// original prolog comments and tier. Editor positions come from the graph
/// (renamed nodes keep theirs; deleted nodes lose theirs).
fn save(file: &Path, original: &str, doc: &Document, graph: &Graph) -> Result<(), ExitCode> {
    let mut out = match Document::from_graph(graph, doc.name.clone()) {
        Ok(d) => d,
        Err(err) => {
            report(file, &err);
            return Err(ExitCode::FAILURE);
        }
    };
    out.min_tier = doc.min_tier;
    let (prolog, _) = split_prolog(original);
    let text = match format(&out).and_then(|body| canonicalize(&format!("{prolog}{body}"))) {
        Ok(t) => t,
        Err(err) => {
            report(file, &err);
            return Err(ExitCode::FAILURE);
        }
    };
    match fs::write(file, text) {
        Ok(()) => {
            println!("{}: saved", file.display());
            Ok(())
        }
        Err(err) => {
            eprintln!("{}: cannot write: {err}", file.display());
            Err(ExitCode::FAILURE)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn parses_options() {
        let o = parse_args(args(&[
            "r.room",
            "s.py",
            "--grant",
            "graph.read,graph.write",
            "--grant",
            "graph.cook",
            "--save",
            "--ticks",
            "3",
            "--fps",
            "30",
            "--timeout",
            "2.5",
        ]))
        .unwrap();
        assert_eq!(o.room.as_deref(), Some("r.room"));
        assert_eq!(o.script.as_deref(), Some("s.py"));
        assert_eq!(o.grants, ["graph.read", "graph.write", "graph.cook"]);
        assert!(o.save);
        assert_eq!(o.ticks, 3);
        assert_eq!(o.fps, 30.0);
        assert_eq!(o.timeout, Some(2.5));
        assert!(parse_args(args(&["--fps", "0"])).is_err());
        assert!(parse_args(args(&["a", "b", "c"])).is_err());
        assert!(parse_args(args(&["--bogus"])).is_err());
    }
}
