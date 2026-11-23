// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! One script session: spawn, handshake, serve requests, drive ticks and
//! events, shut down.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

use livspeil_graph::{Cooker, TickReport};
use livspeil_node_api::CookContext;
use livspeil_nodes_core::StdFs;
use serde_json::{Value as Json, json};

use crate::capability::{Capabilities, Capability};
use crate::error::ScriptError;
use crate::host::{self, HandlerState, Scene};
use crate::process::{Incoming, Worker, WorkerCommand};
use crate::protocol::{self, Message, PROTOCOL_VERSION, RemoteError, codes};

/// How many stderr lines a [`ScriptError::Crashed`] carries.
const STDERR_TAIL: usize = 20;

/// Time and size limits of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// The script must send `hello` within this time of starting.
    pub handshake_timeout: Duration,
    /// Each host request (`tick`, `event`, `shutdown`) must be answered
    /// within this time.
    pub request_timeout: Duration,
    /// After a timed-out request is cancelled, how long the script has to
    /// answer it before it is killed.
    pub cancel_grace: Duration,
    /// Total wall-clock time the script may live; it is killed after.
    pub run_timeout: Duration,
    /// How long a script may take to exit after stdout closes or after
    /// `shutdown`.
    pub exit_grace: Duration,
    /// Longest line (message) the script may send; longer kills it.
    pub max_message_bytes: usize,
    /// Log text kept per session; later log lines are counted, not kept.
    pub max_log_bytes: usize,
    /// Advisory address-space limit passed to the worker as
    /// `LIVSPEIL_MEMORY_LIMIT` (bytes); the Python SDK applies it with
    /// `resource.setrlimit` where the platform allows. Not enforced by the
    /// host.
    pub memory_bytes: Option<u64>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            handshake_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(5),
            cancel_grace: Duration::from_millis(250),
            run_timeout: Duration::from_secs(60),
            exit_grace: Duration::from_secs(2),
            max_message_bytes: 8 * 1024 * 1024,
            max_log_bytes: 1024 * 1024,
            memory_bytes: Some(1024 * 1024 * 1024),
        }
    }
}

/// Everything needed to start a session.
#[derive(Debug, Clone)]
pub struct ScriptConfig {
    /// Name used in reports (the script's file name, say).
    pub name: String,
    pub command: WorkerCommand,
    pub capabilities: Capabilities,
    pub limits: Limits,
}

impl ScriptConfig {
    /// A session running `command` with `capabilities` and default limits.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        command: WorkerCommand,
        capabilities: Capabilities,
    ) -> Self {
        ScriptConfig {
            name: name.into(),
            command,
            capabilities,
            limits: Limits::default(),
        }
    }

    /// A Python script. The interpreter is `$LIVSPEIL_PYTHON` or
    /// `python3`; the SDK directory ([`python_sdk_dir`]) is put on
    /// `PYTHONPATH`.
    #[must_use]
    pub fn python(script: &Path, capabilities: Capabilities) -> Self {
        let program = std::env::var_os("LIVSPEIL_PYTHON").unwrap_or_else(|| "python3".into());
        let mut command = WorkerCommand::new(program, [OsString::from("-B"), script.into()]);
        if let Some(sdk) = python_sdk_dir() {
            command.env.push(("PYTHONPATH".into(), sdk.into()));
        }
        command
            .env
            .push(("PYTHONIOENCODING".into(), "utf-8".into()));
        command.env.push(("PYTHONUNBUFFERED".into(), "1".into()));
        let name = script.file_name().map_or_else(
            || script.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        ScriptConfig::new(name, command, capabilities)
    }
}

/// The Python SDK directory: `$LIVSPEIL_PYTHON_SDK` if set, else the
/// `sdk/python` directory of the source tree this crate was built from,
/// if it exists.
#[must_use]
pub fn python_sdk_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("LIVSPEIL_PYTHON_SDK") {
        return Some(dir.into());
    }
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../sdk/python");
    dir.is_dir().then(|| dir.canonicalize().unwrap_or(dir))
}

/// Severity of a log line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    /// A line the script wrote to stderr (including `print` output in
    /// the Python SDK, and tracebacks).
    Stderr,
}

impl LogLevel {
    fn parse(text: &str) -> LogLevel {
        match text {
            "debug" => LogLevel::Debug,
            "warn" | "warning" => LogLevel::Warn,
            "error" => LogLevel::Error,
            _ => LogLevel::Info,
        }
    }

    /// Lower-case name.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Warn => "warn",
            LogLevel::Error => "error",
            LogLevel::Stderr => "stderr",
        }
    }
}

/// One log line of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogEntry {
    pub level: LogLevel,
    pub message: String,
}

/// Where a session is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Spawned, no `hello` yet.
    Starting,
    /// Handshake done; the script's main body runs.
    Running,
    /// The script sent `listen` and waits for `tick`/`event`.
    Listening,
    /// The script exited normally.
    Exited,
    /// The session failed (see the error returned then); the worker is
    /// gone.
    Failed,
}

/// How [`ScriptSession::run`] ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The script finished its main body and exited with status 0.
    Exited,
    /// The script listens for ticks and events; drive it with
    /// [`ScriptSession::tick`] and [`ScriptSession::send_event`], then
    /// [`ScriptSession::shutdown`].
    Listening,
}

#[derive(Debug, Clone, Copy, Default)]
struct Topics {
    tick: bool,
    event: bool,
}

/// What the serve loop waits for.
#[derive(Debug, Clone, Copy)]
enum Wait<'a> {
    Hello,
    Main,
    Response { id: u64, method: &'a str },
}

enum Got {
    Hello,
    Listen,
    Exited,
    Response(Result<Json, RemoteError>),
}

/// A running script and the host side of its protocol.
///
/// Synchronous: the calling thread serves the script's requests inside
/// [`run`](Self::run), [`tick`](Self::tick), [`send_event`](Self::send_event)
/// and [`shutdown`](Self::shutdown), with every wait bounded by the
/// [`Limits`]. The worker is killed when the session is dropped.
#[derive(Debug)]
pub struct ScriptSession {
    config: ScriptConfig,
    worker: Worker,
    state: SessionState,
    handler: HandlerState,
    topics: Topics,
    started: Instant,
    deadline: Instant,
    next_id: u64,
    client: Option<String>,
    logs: Vec<LogEntry>,
    log_bytes: usize,
    dropped_logs: usize,
    stderr_tail: VecDeque<String>,
    exit_code: Option<i32>,
}

impl ScriptSession {
    /// Start the worker process. Nothing is exchanged until
    /// [`run`](Self::run).
    ///
    /// # Errors
    /// [`ScriptError::Spawn`] if the process cannot be started or the
    /// `fs.read` root is not a directory.
    pub fn spawn(config: ScriptConfig) -> Result<Self, ScriptError> {
        let mut cooker = Cooker::new();
        if let Some(root) = config.capabilities.fs_root() {
            let fs = StdFs::new(root).map_err(|e| {
                ScriptError::Spawn(std::io::Error::new(
                    e.kind(),
                    format!("fs.read root {}: {e}", root.display()),
                ))
            })?;
            cooker.set_system(Some(Arc::new(fs)));
        }
        let mut command = config.command.clone();
        command.env.push((
            "LIVSPEIL_PROTOCOL".into(),
            PROTOCOL_VERSION.to_string().into(),
        ));
        if let Some(bytes) = config.limits.memory_bytes {
            command
                .env
                .push(("LIVSPEIL_MEMORY_LIMIT".into(), bytes.to_string().into()));
        }
        let worker =
            Worker::spawn(&command, config.limits.max_message_bytes).map_err(ScriptError::Spawn)?;
        let started = Instant::now();
        Ok(ScriptSession {
            deadline: started + config.limits.run_timeout,
            handler: HandlerState {
                capabilities: config.capabilities.clone(),
                groups: HashMap::new(),
                cooker,
            },
            config,
            worker,
            state: SessionState::Starting,
            topics: Topics::default(),
            started,
            next_id: 0,
            client: None,
            logs: Vec::new(),
            log_bytes: 0,
            dropped_logs: 0,
            stderr_tail: VecDeque::new(),
            exit_code: None,
        })
    }

    /// Do the handshake and serve the script's main body until it exits
    /// or sends `listen`.
    ///
    /// # Errors
    /// Handshake, timeout, crash and protocol errors (the worker is gone
    /// afterwards).
    pub fn run(&mut self, scene: &mut Scene) -> Result<Outcome, ScriptError> {
        if self.state != SessionState::Starting {
            return Err(ScriptError::InvalidState("run may only be called once"));
        }
        match self.serve(scene, Wait::Hello)? {
            Got::Hello => {}
            Got::Exited => return Ok(Outcome::Exited),
            Got::Listen | Got::Response(_) => {
                return self.fail(ScriptError::Handshake("unexpected message".into()));
            }
        }
        match self.serve(scene, Wait::Main)? {
            Got::Listen => Ok(Outcome::Listening),
            _ => Ok(Outcome::Exited),
        }
    }

    /// Run the scene cooker's tick at `clock` and deliver it: an `event`
    /// request per fired event (if the script listens for events), then a
    /// `tick` request (if it listens for ticks). Returns the tick report.
    ///
    /// With several sessions on one scene, tick the cooker once yourself
    /// and call [`deliver_tick`](Self::deliver_tick) on each.
    ///
    /// # Errors
    /// [`ScriptError::InvalidState`] unless listening; otherwise as
    /// [`deliver_tick`](Self::deliver_tick).
    pub fn tick(
        &mut self,
        scene: &mut Scene,
        clock: CookContext,
    ) -> Result<TickReport, ScriptError> {
        self.ensure_listening()?;
        let report = scene.cooker.tick(&scene.graph, clock, &[]);
        self.deliver_tick(scene, clock, &report)?;
        Ok(report)
    }

    /// Deliver a tick that already ran on the scene's cooker.
    ///
    /// # Errors
    /// [`ScriptError::InvalidState`] unless listening, the script's
    /// handler error, a timeout or a crash.
    pub fn deliver_tick(
        &mut self,
        scene: &mut Scene,
        clock: CookContext,
        report: &TickReport,
    ) -> Result<(), ScriptError> {
        self.ensure_listening()?;
        if self.topics.event {
            for params in host::scene_events(&scene.graph, report) {
                self.host_request(scene, "event", params)?;
            }
        }
        if self.topics.tick {
            let params = json!({
                "frame": clock.frame,
                "time": clock.time,
                "dt": clock.dt,
                "fired": report.fired.len(),
            });
            self.host_request(scene, "tick", params)?;
        }
        Ok(())
    }

    /// Send an `event` request with `params` (for example
    /// `{"kind":"doc_changed","origin":"ui","changes":[...]}` after a UI
    /// edit). A no-op when the script does not listen for events.
    ///
    /// # Errors
    /// As [`deliver_tick`](Self::deliver_tick).
    pub fn send_event(&mut self, scene: &mut Scene, params: Json) -> Result<(), ScriptError> {
        self.ensure_listening()?;
        if self.topics.event {
            self.host_request(scene, "event", params)?;
        }
        Ok(())
    }

    /// Ask the script to exit (`shutdown`), wait for it, and return its
    /// exit code (`None` if it had to be killed). Idempotent.
    ///
    /// # Errors
    /// The script's error answer to `shutdown`, or a crash.
    pub fn shutdown(&mut self, scene: &mut Scene) -> Result<Option<i32>, ScriptError> {
        match self.state {
            SessionState::Exited | SessionState::Failed => return Ok(self.exit_code),
            SessionState::Listening | SessionState::Running => {
                let result = self.host_request(scene, "shutdown", json!({ "reason": "host" }));
                if self.state == SessionState::Exited || self.state == SessionState::Failed {
                    return result.map(|_| self.exit_code);
                }
                result?;
            }
            SessionState::Starting => {}
        }
        self.worker.close_stdin();
        self.drain_until_closed();
        let status = self.worker.finish(self.config.limits.exit_grace);
        self.collect_late_stderr();
        self.exit_code = status.and_then(|s| s.code());
        self.state = SessionState::Exited;
        Ok(self.exit_code)
    }

    /// The session's state.
    #[must_use]
    pub fn state(&self) -> SessionState {
        self.state
    }

    /// The configuration it was started with.
    #[must_use]
    pub fn config(&self) -> &ScriptConfig {
        &self.config
    }

    /// The `client` string the script sent in `hello`.
    #[must_use]
    pub fn client(&self) -> Option<&str> {
        self.client.as_deref()
    }

    /// Log lines (`log` notifications and stderr), in arrival order.
    #[must_use]
    pub fn logs(&self) -> &[LogEntry] {
        &self.logs
    }

    /// Log lines dropped because of [`Limits::max_log_bytes`].
    #[must_use]
    pub fn dropped_logs(&self) -> usize {
        self.dropped_logs
    }

    /// The exit code, once the script has exited.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    /// Time since the worker started.
    #[must_use]
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn ensure_listening(&self) -> Result<(), ScriptError> {
        if self.state == SessionState::Listening {
            Ok(())
        } else {
            Err(ScriptError::InvalidState(
                "the script is not listening for ticks and events",
            ))
        }
    }

    fn host_request(
        &mut self,
        scene: &mut Scene,
        method: &str,
        params: Json,
    ) -> Result<Json, ScriptError> {
        self.next_id += 1;
        let id = self.next_id;
        self.worker.send(&protocol::request(id, method, params));
        match self.serve(scene, Wait::Response { id, method })? {
            Got::Response(Ok(result)) => Ok(result),
            Got::Response(Err(error)) => Err(ScriptError::HandlerFailed {
                method: method.to_owned(),
                error,
            }),
            Got::Exited => Ok(Json::Null),
            Got::Hello | Got::Listen => Err(ScriptError::InvalidState("unexpected message")),
        }
    }

    fn log(&mut self, level: LogLevel, message: String) {
        if level == LogLevel::Stderr {
            if self.stderr_tail.len() == STDERR_TAIL {
                self.stderr_tail.pop_front();
            }
            self.stderr_tail.push_back(message.clone());
        }
        if self.log_bytes + message.len() > self.config.limits.max_log_bytes {
            self.dropped_logs += 1;
            return;
        }
        self.log_bytes += message.len();
        self.logs.push(LogEntry { level, message });
    }

    /// After `shutdown`: keep the script's last log lines until it closes
    /// stdout (at most [`Limits::exit_grace`]). Requests are refused.
    fn drain_until_closed(&mut self) {
        let deadline = Instant::now() + self.config.limits.exit_grace;
        while let Ok(item) = self.worker.recv(deadline) {
            match item {
                Incoming::Stderr(line) => self.log(LogLevel::Stderr, line),
                Incoming::Message(json) => match Message::decode(json) {
                    Ok(Message::Notification { method, params }) if method == "log" => {
                        self.log_notification(&params);
                    }
                    Ok(Message::Request { id, .. }) => self.reply(
                        id,
                        Err(RemoteError::new(
                            codes::INVALID_REQUEST,
                            "the session is shutting down",
                        )),
                    ),
                    _ => {}
                },
                Incoming::Closed | Incoming::TooLong => return,
                Incoming::Malformed(_) => {}
            }
        }
    }

    fn log_notification(&mut self, params: &Json) {
        let level = params
            .get("level")
            .and_then(Json::as_str)
            .map_or(LogLevel::Info, LogLevel::parse);
        let text = params
            .get("message")
            .and_then(Json::as_str)
            .unwrap_or_default()
            .to_owned();
        self.log(level, text);
    }

    fn collect_late_stderr(&mut self) {
        for line in self.worker.drain_stderr(Duration::from_millis(200)) {
            self.log(LogLevel::Stderr, line);
        }
    }

    /// Kill the worker, mark the session failed and return `err`.
    fn fail<T>(&mut self, err: ScriptError) -> Result<T, ScriptError> {
        self.worker.kill();
        self.collect_late_stderr();
        self.state = SessionState::Failed;
        Err(err)
    }

    fn reply(&self, id: Json, outcome: Result<Json, RemoteError>) {
        self.worker.send(&protocol::response(id, outcome));
    }

    /// Serve messages until what `wait` asks for arrives.
    fn serve(&mut self, scene: &mut Scene, wait: Wait<'_>) -> Result<Got, ScriptError> {
        let limits = self.config.limits;
        let mut wait_deadline = match wait {
            Wait::Hello => Some(self.started + limits.handshake_timeout),
            Wait::Main => None,
            Wait::Response { .. } => Some(Instant::now() + limits.request_timeout),
        };
        let mut cancelled = false;
        loop {
            if Instant::now() >= self.deadline {
                return self.fail(ScriptError::RunTimeout(limits.run_timeout));
            }
            let deadline = wait_deadline.map_or(self.deadline, |d| d.min(self.deadline));
            let item = match self.worker.recv(deadline) {
                Ok(item) => item,
                Err(RecvTimeoutError::Disconnected) => Incoming::Closed,
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= self.deadline {
                        return self.fail(ScriptError::RunTimeout(limits.run_timeout));
                    }
                    match wait {
                        Wait::Response { id, method } if !cancelled => {
                            // Cancel, then give the script a moment to answer.
                            self.worker
                                .send(&protocol::notification("cancel", json!({ "id": id })));
                            cancelled = true;
                            wait_deadline = Some(Instant::now() + limits.cancel_grace);
                            self.log(
                                LogLevel::Warn,
                                format!(
                                    "host cancelled '{method}' after {:?}",
                                    limits.request_timeout
                                ),
                            );
                            continue;
                        }
                        Wait::Response { method, .. } => {
                            return self.fail(ScriptError::Timeout {
                                method: method.to_owned(),
                                after: limits.request_timeout,
                            });
                        }
                        Wait::Hello => {
                            return self.fail(ScriptError::Timeout {
                                method: "hello".into(),
                                after: limits.handshake_timeout,
                            });
                        }
                        Wait::Main => continue,
                    }
                }
            };
            match item {
                Incoming::Stderr(line) => self.log(LogLevel::Stderr, line),
                Incoming::Malformed(err) => self.reply(
                    Json::Null,
                    Err(RemoteError::new(
                        codes::PARSE_ERROR,
                        format!("not a JSON line: {err}"),
                    )),
                ),
                Incoming::TooLong => {
                    return self.fail(ScriptError::MessageTooLarge {
                        limit: limits.max_message_bytes,
                    });
                }
                Incoming::Closed => return self.on_closed(wait),
                Incoming::Message(json) => {
                    if let Some(got) = self.on_message(scene, wait, json)? {
                        if cancelled
                            && let Got::Response(_) = got
                            && let Wait::Response { method, .. } = wait
                        {
                            return Err(ScriptError::Timeout {
                                method: method.to_owned(),
                                after: limits.request_timeout,
                            });
                        }
                        return Ok(got);
                    }
                }
            }
        }
    }

    /// The script closed stdout: wait for its exit and classify it.
    fn on_closed(&mut self, wait: Wait<'_>) -> Result<Got, ScriptError> {
        let status = self.worker.finish(self.config.limits.exit_grace);
        self.collect_late_stderr();
        self.exit_code = status.and_then(|s| s.code());
        let success = status.is_some_and(|s| s.success());
        if !success {
            let stderr = self.stderr_tail.iter().cloned().collect();
            let code = self.exit_code;
            return self.fail(ScriptError::Crashed { code, stderr });
        }
        self.state = SessionState::Exited;
        match wait {
            Wait::Main => Ok(Got::Exited),
            Wait::Hello => {
                self.state = SessionState::Failed;
                Err(ScriptError::Handshake(
                    "the script exited without saying hello".into(),
                ))
            }
            Wait::Response {
                method: "shutdown", ..
            } => Ok(Got::Exited),
            Wait::Response { method, .. } => Err(ScriptError::Exited {
                method: method.to_owned(),
            }),
        }
    }

    /// Handle one decoded line; `Some` when it is what `wait` waits for.
    fn on_message(
        &mut self,
        scene: &mut Scene,
        wait: Wait<'_>,
        json: Json,
    ) -> Result<Option<Got>, ScriptError> {
        let message = match Message::decode(json) {
            Ok(m) => m,
            Err((reason, id)) => {
                self.reply(id, Err(RemoteError::new(codes::INVALID_REQUEST, reason)));
                return Ok(None);
            }
        };
        match message {
            Message::Notification { method, params } => {
                if method == "log" {
                    self.log_notification(&params);
                }
                // `cancel` from the script: requests are served
                // synchronously, so there is never one to cancel. Unknown
                // notifications are ignored.
                Ok(None)
            }
            Message::Response { id, outcome } => match wait {
                Wait::Response { id: want, .. } if id.as_u64() == Some(want) => {
                    Ok(Some(Got::Response(outcome)))
                }
                _ => Ok(None), // Late answer to a cancelled request.
            },
            Message::Request { id, method, params } => {
                self.on_request(scene, wait, id, &method, &params)
            }
        }
    }

    fn on_request(
        &mut self,
        scene: &mut Scene,
        wait: Wait<'_>,
        id: Json,
        method: &str,
        params: &Json,
    ) -> Result<Option<Got>, ScriptError> {
        if method == "hello" {
            if self.state != SessionState::Starting {
                self.reply(
                    id,
                    Err(RemoteError::new(
                        codes::INVALID_REQUEST,
                        "already said hello",
                    )),
                );
                return Ok(None);
            }
            return self.handshake(scene, id, params).map(|()| Some(Got::Hello));
        }
        if self.state == SessionState::Starting {
            self.reply(
                id,
                Err(RemoteError::new(
                    codes::HANDSHAKE_REQUIRED,
                    "send 'hello' first",
                )),
            );
            return Ok(None);
        }
        if method == "listen" {
            if !matches!(wait, Wait::Main) || self.state != SessionState::Running {
                self.reply(
                    id,
                    Err(RemoteError::new(
                        codes::INVALID_REQUEST,
                        "'listen' is only valid once, at the end of the main body",
                    )),
                );
                return Ok(None);
            }
            return match self.listen(params) {
                Ok(result) => {
                    self.reply(id, Ok(result));
                    self.state = SessionState::Listening;
                    Ok(Some(Got::Listen))
                }
                Err(err) => {
                    self.reply(id, Err(err));
                    Ok(None)
                }
            };
        }
        let outcome = host::dispatch(scene, &mut self.handler, method, params);
        self.reply(id, outcome);
        Ok(None)
    }

    fn handshake(&mut self, scene: &Scene, id: Json, params: &Json) -> Result<(), ScriptError> {
        let requested = params.get("protocol").cloned().unwrap_or(Json::Null);
        if requested.as_u64() != Some(u64::from(PROTOCOL_VERSION)) {
            self.reply(
                id,
                Err(RemoteError::new(
                    codes::UNSUPPORTED_PROTOCOL,
                    format!("this host speaks protocol {PROTOCOL_VERSION}"),
                )
                .with_data(json!({ "supported": [PROTOCOL_VERSION] }))),
            );
            // Let the answer reach the script before it is killed.
            std::thread::sleep(Duration::from_millis(50));
            return self.fail(ScriptError::UnsupportedProtocol {
                requested,
                supported: PROTOCOL_VERSION,
            });
        }
        self.client = params
            .get("client")
            .and_then(Json::as_str)
            .map(str::to_owned);
        let limits = self.config.limits;
        let millis = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        let result = json!({
            "protocol": PROTOCOL_VERSION,
            "host": concat!("livspeil-script ", env!("CARGO_PKG_VERSION")),
            "session": self.config.name,
            "capabilities": self.config.capabilities.tokens(),
            "room": { "name": scene.name },
            "limits": {
                "request_timeout_ms": millis(limits.request_timeout),
                "run_timeout_ms": millis(limits.run_timeout),
                "max_message_bytes": limits.max_message_bytes,
            },
        });
        self.reply(id, Ok(result));
        self.state = SessionState::Running;
        Ok(())
    }

    fn listen(&mut self, params: &Json) -> Result<Json, RemoteError> {
        let topics = match params.get("topics") {
            None | Some(Json::Null) => vec!["tick".to_owned()],
            Some(Json::Array(items)) => items
                .iter()
                .map(|t| t.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    RemoteError::new(codes::INVALID_PARAMS, "'topics' must be a list of strings")
                })?,
            Some(_) => {
                return Err(RemoteError::new(
                    codes::INVALID_PARAMS,
                    "'topics' must be a list of strings",
                ));
            }
        };
        let mut chosen = Topics::default();
        for topic in &topics {
            match topic.as_str() {
                "tick" => chosen.tick = true,
                "event" => {
                    if !self.handler.capabilities.has(&Capability::GraphRead) {
                        return Err(RemoteError::new(
                            codes::CAPABILITY_DENIED,
                            "listening for 'event' needs the 'graph.read' capability",
                        )
                        .with_data(json!({ "capability": "graph.read", "method": "listen" })));
                    }
                    chosen.event = true;
                }
                other => {
                    return Err(RemoteError::new(
                        codes::INVALID_PARAMS,
                        format!("unknown topic '{other}' (known: tick, event)"),
                    ));
                }
            }
        }
        self.topics = chosen;
        Ok(json!({ "topics": topics }))
    }
}
