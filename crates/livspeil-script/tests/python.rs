// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! End-to-end tests with real Python workers and the Python SDK. Skipped
//! (with a message) when `python3` is not installed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use livspeil_cmd::{EntryKind, Origin};
use livspeil_graph::Graph;
use livspeil_node_api::{CookContext, Value};
use livspeil_nodes_core::core_registry;
use livspeil_script::{
    Capabilities, Capability, Limits, LogLevel, Outcome, Scene, ScriptConfig, ScriptError,
    ScriptSession, SessionState,
};

fn python() -> Option<String> {
    let program = std::env::var("LIVSPEIL_PYTHON").unwrap_or_else(|_| "python3".into());
    let ok = Process::new(&program)
        .arg("-c")
        .arg("import sys; sys.exit(0 if sys.version_info >= (3, 10) else 1)")
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        Some(program)
    } else {
        eprintln!("SKIPPED: {program} (Python >= 3.10) is not available");
        None
    }
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(room: &str) -> Scene {
    let text = fs::read_to_string(repo().join("examples").join(room).join("room.kdl")).unwrap();
    let doc = livspeil_doc::parse(&text).unwrap();
    let graph = doc.to_graph(Arc::new(core_registry())).unwrap();
    Scene::new(doc.name, graph)
}

struct TempScript(PathBuf);

impl TempScript {
    fn new(source: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!(
            "livspeil-script-test-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("script.py");
        fs::write(&path, source).unwrap();
        TempScript(path)
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        if let Some(dir) = self.0.parent() {
            fs::remove_dir_all(dir).ok();
        }
    }
}

fn caps(tokens: &[&str]) -> Capabilities {
    Capabilities::parse(tokens.iter().copied()).unwrap()
}

fn config(script: &TempScript, caps: Capabilities, program: &str) -> ScriptConfig {
    let mut config = ScriptConfig::python(&script.0, caps);
    config.command.program = program.into();
    config.limits.run_timeout = Duration::from_secs(30);
    config
}

/// Run a script's main body against `scene`; returns the session and the
/// result of `run`.
fn run(
    scene: &mut Scene,
    source: &str,
    tokens: &[&str],
    tweak: impl FnOnce(&mut Limits),
) -> Option<(ScriptSession, Result<Outcome, ScriptError>)> {
    let program = python()?;
    let script = TempScript::new(source);
    let mut config = config(&script, caps(tokens), &program);
    tweak(&mut config.limits);
    let mut session = ScriptSession::spawn(config).unwrap();
    let result = session.run(scene);
    Some((session, result))
}

fn messages(session: &ScriptSession, level: LogLevel) -> Vec<String> {
    session
        .logs()
        .iter()
        .filter(|l| l.level == level)
        .map(|l| l.message.clone())
        .collect()
}

fn input(graph: &Graph, node: &str, port: &str) -> Option<Value> {
    let id = graph.find_path(node).unwrap();
    graph.input_value(id, port).unwrap().cloned()
}

#[test]
fn handshake_reports_capabilities_room_and_limits() {
    let mut scene = load("sun-and-lamp.room");
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
s = livspeil.connect(client="test-client/1")
s.log(",".join(sorted(s.capabilities)))
s.log(s.room)
s.log(str(s.limits["request_timeout_ms"] > 0))
"#,
        &["graph.read", "graph.cook"],
        |_| {},
    ) else {
        return;
    };
    assert_eq!(result.unwrap(), Outcome::Exited);
    assert_eq!(session.state(), SessionState::Exited);
    assert_eq!(session.client(), Some("test-client/1"));
    assert_eq!(
        messages(&session, LogLevel::Info),
        ["graph.cook,graph.read", "Sun and Lamp", "True"]
    );
    assert_eq!(session.exit_code(), Some(0));
}

#[test]
fn reads_nodes_inputs_and_snapshot() {
    let mut scene = load("sun-and-lamp.room");
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
s = livspeil.connect()
g = s.graph
nodes = g.nodes()
s.log(str(len(nodes)))
s.log(nodes[0].path)
lamp = g.node("lamp_intensity")
s.log(repr(lamp.input("out_min")))
s.log(repr(lamp.input("in_min")))
s.log(lamp.inputs["x"].link)
s.log(lamp.outputs["out"].type)
remaps = g.nodes(type="core.math.remap")
s.log(",".join(n.name for n in remaps))
snap = g.snapshot()
s.log(str(len(snap["nodes"])) + " " + str(len(snap["links"])))
s.log(snap["paths"][lamp.id])
try:
    g.node("nope")
except livspeil.NotFound as e:
    s.log("not found: " + e.message)
"#,
        &["graph.read"],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    assert_eq!(
        messages(&session, LogLevel::Info),
        [
            "9",
            "/clock",
            "1.0",
            "0.0",
            "/daylight.out",
            "Float",
            "lamp_intensity",
            "9 9",
            "/lamp_intensity",
            "not found: no node 'nope'",
        ]
    );
}

#[test]
fn apply_is_undoable_and_journaled_as_script() {
    let mut scene = load("sun-and-lamp.room");
    let before = scene.graph.snapshot();
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
from livspeil import SetInput, SetExpression
s = livspeil.connect()
g = s.graph
r = g.apply(SetInput("lamp_intensity", "out_min", 0.8))
s.log(str(r.undo_len) + " " + r.changes[0]["kind"] + " " + r.changes[0]["port"])
with g.transaction():
    g.apply(SetInput("sunrise", "edge0", 4.0))
    g.apply(SetInput("sunset", "edge1", 21.0))
    g.apply(SetExpression("sun", "latitude", "40 + 1.5"))
"#,
        &["graph.read", "graph.write"],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    assert_eq!(
        messages(&session, LogLevel::Info),
        ["1 input_changed out_min"]
    );
    let g = &scene.graph;
    assert_eq!(
        input(g, "lamp_intensity", "out_min"),
        Some(Value::Float(0.8))
    );
    assert_eq!(input(g, "sunrise", "edge0"), Some(Value::Float(4.0)));

    // Two applies, two undo steps (the transaction is one), two journal
    // entries tagged script.
    assert_eq!(scene.history.undo_len(), 2);
    assert_eq!(scene.journal.len(), 2);
    for entry in &scene.journal {
        assert_eq!(entry.origin, Origin::Script);
        assert_eq!(entry.kind, EntryKind::Do);
        let line = serde_json::to_string(entry).unwrap();
        assert!(line.contains(r#""origin":"script""#), "{line}");
    }

    scene.history.undo(&mut scene.graph).unwrap();
    assert_eq!(
        input(&scene.graph, "sunrise", "edge0"),
        Some(Value::Float(5.0))
    );
    let undo = scene.history.undo(&mut scene.graph).unwrap();
    assert_eq!(undo.entry.origin, Origin::Script);
    assert_eq!(scene.graph.snapshot(), before);
}

#[test]
fn rejected_commands_change_nothing() {
    let mut scene = load("sun-and-lamp.room");
    let before = scene.graph.snapshot();
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
from livspeil import SetInput
s = livspeil.connect()
g = s.graph
try:
    with g.transaction():
        g.apply(SetInput("sunrise", "edge0", 4.0))
        g.apply(SetInput("sunrise", "no_such_port", 1.0))
except livspeil.CommandFailed as e:
    s.log("failed " + str(e.data["step"]))
try:
    g.apply(SetInput("sunrise", "edge0", "text"))
except livspeil.CommandFailed as e:
    s.log("type error")
try:
    s.request("graph.apply", {"command": {"explode": {}}})
except livspeil.RemoteError as e:
    s.log(e.code)
"#,
        &["graph.read", "graph.write"],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    assert_eq!(
        messages(&session, LogLevel::Info),
        ["failed [1]", "type error", "invalid_params"]
    );
    assert_eq!(scene.graph.snapshot(), before);
    assert!(scene.journal.is_empty());
}

#[test]
fn create_and_link_nodes_with_host_ids() {
    let mut scene = load("sun-and-lamp.room");
    let Some((_session, result)) = run(
        &mut scene,
        r#"
import livspeil
from livspeil import CreateNode, Link, SetInput
s = livspeil.connect()
g = s.graph
with g.transaction():
    half = CreateNode("core.math.mul@1", "half")
    g.apply(half)
    g.apply(SetInput(half.id, "b", 0.5))
    g.apply(Link("daylight", "out", half.id, "a"))
"#,
        &["graph.read", "graph.write"],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    let half = scene.graph.find("half").unwrap();
    assert_eq!(scene.graph.node_type(half), Some("core.math.mul@1"));
    assert_eq!(scene.graph.incoming_links(half).len(), 1);
    assert_eq!(scene.history.undo_len(), 1);
}

#[test]
fn missing_capabilities_are_denied_per_request() {
    let mut scene = load("sun-and-lamp.room");
    let before = scene.graph.snapshot();
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
from livspeil import SetInput
s = livspeil.connect()
for name, call in [
    ("write", lambda: s.graph.apply(SetInput("0192f5a0-7c3e-7a10-8000-000000000005", "edge0", 1.0))),
    ("cook", lambda: s.graph.cook(["sun"])),
    ("new_id", lambda: s.graph.new_ids()),
]:
    try:
        call()
        s.log(name + " allowed")
    except livspeil.CapabilityDenied as e:
        s.log(name + " denied " + e.data["capability"])
s.log(str(len(s.graph.nodes())))
"#,
        &["graph.read"],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    assert_eq!(
        messages(&session, LogLevel::Info),
        [
            "write denied graph.write",
            "cook denied graph.cook",
            "new_id denied graph.write",
            "9",
        ]
    );
    assert_eq!(scene.graph.snapshot(), before);
}

#[test]
fn no_grants_means_no_reads_and_uncaught_denial_crashes() {
    let mut scene = load("sun-and-lamp.room");
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
s = livspeil.connect()
try:
    s.request("listen", {"topics": ["event"]})
except livspeil.CapabilityDenied as e:
    s.log("event denied")
s.graph.nodes()
"#,
        &[],
        |_| {},
    ) else {
        return;
    };
    assert_eq!(messages(&session, LogLevel::Info), ["event denied"]);
    match result {
        Err(ScriptError::Crashed { code, stderr }) => {
            assert_eq!(code, Some(1));
            assert!(
                stderr.iter().any(|l| l.contains("CapabilityDenied")),
                "{stderr:?}"
            );
        }
        other => panic!("expected a crash, got {other:?}"),
    }
    assert_eq!(session.state(), SessionState::Failed);
}

#[test]
fn cook_returns_outputs_at_a_given_time() {
    let mut scene = load("sun-and-lamp.room");
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
s = livspeil.connect()
r = s.graph.cook(["sun", "lamp_color"], time=5.0, frame=300)
s.log("%.6f" % r["/sun"]["elevation"])
c = r["/lamp_color"]["out"]
s.log("%s %.6f" % (type(c).__name__, c.l))
r = s.graph.cook("sun", time=0.0)
s.log("%.6f" % r["/sun"]["elevation"])
"#,
        &["graph.cook"],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    // Same values as `livspeil cook` (README: frames 0 and 300).
    assert_eq!(
        messages(&session, LogLevel::Info),
        ["15.350625", "Color 0.876543", "15.350625"]
    );
    // The scene's clock is untouched by cooks at an explicit time.
    assert_eq!(scene.cooker.clock().time, 0.0);
}

#[test]
fn ticks_and_events_reach_hooks() {
    let Some(program) = python() else {
        return;
    };
    let mut scene = load("blink.room");
    let script = TempScript::new(
        r#"
import livspeil
from livspeil import SetInput
s = livspeil.connect()

@s.on_tick
def tick(t):
    if t.fired:
        s.log("tick %d %.2f fired=%d" % (t.frame, t.time, t.fired))
    if t.frame == 10:
        s.graph.apply(SetInput("brightness", "a", 0.9))

@s.on_event
def event(e):
    if e.kind == "scene":
        s.log("event %s.%s %r" % (e.node, e.port, e.payload))
    else:
        s.log("event %s %s" % (e.kind, e.data.get("name")))

s.listen()
s.log("bye")
"#,
    );
    let config = config(&script, caps(&["graph.read", "graph.write"]), &program);
    let mut session = ScriptSession::spawn(config).unwrap();
    assert_eq!(session.run(&mut scene).unwrap(), Outcome::Listening);
    assert_eq!(session.state(), SessionState::Listening);
    for frame in 0..12_u64 {
        let clock = CookContext {
            frame,
            time: frame as f64 * 0.05,
            dt: 0.05,
        };
        session.tick(&mut scene, clock).unwrap();
    }
    session
        .send_event(
            &mut scene,
            serde_json::json!({"kind": "custom", "name": "hello"}),
        )
        .unwrap();
    assert_eq!(session.shutdown(&mut scene).unwrap(), Some(0));
    assert_eq!(session.state(), SessionState::Exited);
    let logs = messages(&session, LogLevel::Info);
    assert_eq!(
        logs,
        [
            "event /blink.fired 0",
            "event /ticks.value 0",
            "tick 0 0.00 fired=2",
            "event /blink.fired 1",
            "event /ticks.value 1",
            "event /flash.changed 0.2",
            "event /changed.value 0.2",
            "tick 10 0.50 fired=4",
            "event custom hello",
            "bye",
        ],
        "{logs:#?}"
    );
    assert_eq!(
        input(&scene.graph, "brightness", "a"),
        Some(Value::Float(0.9))
    );
    assert_eq!(scene.journal.len(), 1);
}

#[test]
fn run_timeout_kills_a_sleeping_script() {
    let mut scene = load("sun-and-lamp.room");
    let Some((mut session, result)) = run(
        &mut scene,
        r#"
import time, livspeil
s = livspeil.connect()
s.log("sleeping")
time.sleep(30)
"#,
        &[],
        |limits| limits.run_timeout = Duration::from_millis(1500),
    ) else {
        return;
    };
    assert!(
        matches!(result, Err(ScriptError::RunTimeout(_))),
        "{result:?}"
    );
    assert_eq!(messages(&session, LogLevel::Info), ["sleeping"]);
    assert!(session.elapsed() < Duration::from_secs(10));
    assert_eq!(session.state(), SessionState::Failed);
    assert_eq!(session.shutdown(&mut scene).unwrap(), None);
}

#[test]
fn slow_tick_handler_is_cancelled_then_killed() {
    let Some(program) = python() else {
        return;
    };
    let mut scene = load("sun-and-lamp.room");
    let script = TempScript::new(
        r#"
import time, livspeil
s = livspeil.connect()

@s.on_tick
def tick(t):
    time.sleep(30)

s.listen()
"#,
    );
    let mut config = config(&script, Capabilities::none(), &program);
    config.limits.request_timeout = Duration::from_millis(300);
    config.limits.cancel_grace = Duration::from_millis(100);
    let mut session = ScriptSession::spawn(config).unwrap();
    assert_eq!(session.run(&mut scene).unwrap(), Outcome::Listening);
    let err = session
        .tick(&mut scene, CookContext::default())
        .unwrap_err();
    assert!(
        matches!(&err, ScriptError::Timeout { method, .. } if method == "tick"),
        "{err:?}"
    );
    assert_eq!(session.state(), SessionState::Failed);
    assert!(
        messages(&session, LogLevel::Warn)
            .iter()
            .any(|m| m.contains("cancelled 'tick'"))
    );
}

#[test]
fn cooperative_handler_answers_the_cancel() {
    let Some(program) = python() else {
        return;
    };
    let mut scene = load("sun-and-lamp.room");
    let script = TempScript::new(
        r#"
import time, livspeil
s = livspeil.connect()

@s.on_tick
def tick(t):
    while not s.cancelled():
        time.sleep(0.01)

s.listen()
"#,
    );
    let mut config = config(&script, Capabilities::none(), &program);
    config.limits.request_timeout = Duration::from_millis(300);
    config.limits.cancel_grace = Duration::from_secs(5);
    let mut session = ScriptSession::spawn(config).unwrap();
    session.run(&mut scene).unwrap();
    let err = session
        .tick(&mut scene, CookContext::default())
        .unwrap_err();
    assert!(matches!(err, ScriptError::Timeout { .. }), "{err:?}");
    // The script answered within the grace period: it is still usable.
    assert_eq!(session.state(), SessionState::Listening);
    assert_eq!(session.shutdown(&mut scene).unwrap(), Some(0));
}

#[test]
fn crashing_script_reports_its_traceback_and_keeps_earlier_edits() {
    let mut scene = load("sun-and-lamp.room");
    let Some((session, result)) = run(
        &mut scene,
        r#"
import livspeil
from livspeil import SetInput
s = livspeil.connect()
s.graph.apply(SetInput("sunrise", "edge0", 4.5))
print("about to fail")
raise RuntimeError("boom")
"#,
        &["graph.read", "graph.write"],
        |_| {},
    ) else {
        return;
    };
    match result {
        Err(ScriptError::Crashed {
            code: Some(1),
            stderr,
        }) => {
            assert_eq!(
                stderr.last().map(String::as_str),
                Some("RuntimeError: boom")
            );
        }
        other => panic!("expected a crash, got {other:?}"),
    }
    // print() goes to stderr, never into the protocol stream.
    assert!(messages(&session, LogLevel::Stderr).contains(&"about to fail".to_owned()));
    assert_eq!(
        input(&scene.graph, "sunrise", "edge0"),
        Some(Value::Float(4.5))
    );
    assert_eq!(scene.journal.len(), 1);
    scene.history.undo(&mut scene.graph).unwrap();
    assert_eq!(
        input(&scene.graph, "sunrise", "edge0"),
        Some(Value::Float(5.0))
    );
}

#[test]
fn tick_handler_errors_are_reported_and_the_session_survives() {
    let Some(program) = python() else {
        return;
    };
    let mut scene = load("sun-and-lamp.room");
    let script = TempScript::new(
        r#"
import livspeil
s = livspeil.connect()

@s.on_tick
def tick(t):
    if t.frame == 1:
        raise ValueError("bad frame")

s.listen()
"#,
    );
    let config = config(&script, Capabilities::none(), &program);
    let mut session = ScriptSession::spawn(config).unwrap();
    session.run(&mut scene).unwrap();
    session.tick(&mut scene, CookContext::default()).unwrap();
    let clock = CookContext {
        frame: 1,
        time: 0.1,
        dt: 0.1,
    };
    match session.tick(&mut scene, clock) {
        Err(ScriptError::HandlerFailed { method, error }) => {
            assert_eq!(method, "tick");
            assert_eq!(error.code, "handler_failed");
            assert_eq!(error.message, "ValueError: bad frame");
        }
        other => panic!("expected a handler error, got {other:?}"),
    }
    session.tick(&mut scene, CookContext::default()).unwrap();
    assert_eq!(session.shutdown(&mut scene).unwrap(), Some(0));
}

#[test]
fn raw_protocol_errors() {
    let mut scene = load("sun-and-lamp.room");
    // Requests before hello, malformed lines, unknown methods.
    let Some((session, result)) = run(
        &mut scene,
        r#"
import json, sys
def send(m):
    sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()
def recv():
    return json.loads(sys.stdin.readline())
send({"id": 1, "method": "graph.nodes"})
first = recv()
sys.stdout.write("this is not json\n"); sys.stdout.flush()
second = recv()
send({"id": 2, "method": "hello", "params": {"protocol": 1}})
recv()
send({"id": 3, "method": "graph.teleport"})
third = recv()
send({"id": 4, "method": "hello", "params": {"protocol": 1}})
fourth = recv()
codes = [first["error"]["code"], second["error"]["code"], third["error"]["code"], fourth["error"]["code"]]
send({"method": "log", "params": {"message": " ".join(codes)}})
"#,
        &[],
        |_| {},
    ) else {
        return;
    };
    result.unwrap();
    assert_eq!(
        messages(&session, LogLevel::Info),
        ["handshake_required parse_error unknown_method invalid_request"]
    );
}

#[test]
fn unsupported_protocol_fails_the_handshake() {
    let mut scene = load("sun-and-lamp.room");
    let Some((_session, result)) = run(
        &mut scene,
        r#"
import json, sys, time
sys.stdout.write(json.dumps({"id": 1, "method": "hello", "params": {"protocol": 99}}) + "\n")
sys.stdout.flush()
time.sleep(5)
"#,
        &[],
        |_| {},
    ) else {
        return;
    };
    assert!(
        matches!(
            result,
            Err(ScriptError::UnsupportedProtocol { supported: 1, .. })
        ),
        "{result:?}"
    );
}

#[test]
fn silent_script_times_out_in_the_handshake() {
    let mut scene = load("sun-and-lamp.room");
    let Some((_session, result)) =
        run(&mut scene, "import time\ntime.sleep(30)\n", &[], |limits| {
            limits.handshake_timeout = Duration::from_millis(500)
        })
    else {
        return;
    };
    assert!(
        matches!(&result, Err(ScriptError::Timeout { method, .. }) if method == "hello"),
        "{result:?}"
    );
}

#[test]
fn oversized_messages_kill_the_script() {
    let mut scene = load("sun-and-lamp.room");
    let Some((_session, result)) = run(
        &mut scene,
        r#"
import sys, time
sys.stdout.write('{"method":"log","params":{"message":"' + "x" * 200000 + '"}}\n')
sys.stdout.flush()
time.sleep(5)
"#,
        &[],
        |limits| limits.max_message_bytes = 64 * 1024,
    ) else {
        return;
    };
    assert!(
        matches!(result, Err(ScriptError::MessageTooLarge { limit: 65536 })),
        "{result:?}"
    );
}

#[test]
fn fs_read_root_must_exist() {
    let caps = Capabilities::none().with(Capability::FsRead("/definitely/not/here".into()));
    let config = ScriptConfig::python(Path::new("x.py"), caps);
    assert!(matches!(
        ScriptSession::spawn(config),
        Err(ScriptError::Spawn(_))
    ));
}

#[test]
fn sdk_unit_tests_pass() {
    let Some(program) = python() else {
        return;
    };
    let sdk = repo().join("sdk/python");
    let out = Process::new(program)
        .args(["-B", "-m", "unittest", "discover", "-s", "tests"])
        .current_dir(&sdk)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "SDK unit tests failed:\n{stderr}");
    assert!(stderr.contains("OK"), "{stderr}");
}
