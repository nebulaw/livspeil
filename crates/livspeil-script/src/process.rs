// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The worker process and its pipe threads.
//!
//! Three threads per worker: one reads stdout lines (bounded by the
//! message limit) and decodes JSON, one reads stderr lines, one writes
//! stdin. The session thread never blocks on a pipe: it waits on one
//! channel with a deadline, so every timeout is enforced even when the
//! script stops reading or writing.

use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value as Json;

/// Longest stderr line kept (longer lines are cut).
const MAX_STDERR_LINE: usize = 16 * 1024;

/// How to start a worker process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerCommand {
    /// The program (for Python, the interpreter).
    pub program: OsString,
    /// Its arguments (for Python, the script path and its options).
    pub args: Vec<OsString>,
    /// Environment variables set for the worker.
    pub env: Vec<(OsString, OsString)>,
    /// Names of the host's environment variables passed through. Every
    /// other variable is cleared, so secrets in the host's environment do
    /// not leak to scripts by accident.
    pub inherit_env: Vec<OsString>,
    /// Working directory (default: the host's).
    pub current_dir: Option<PathBuf>,
}

impl WorkerCommand {
    /// Run `program` with `args`, passing through `PATH`, `HOME`,
    /// `TMPDIR` and the locale variables.
    #[must_use]
    pub fn new(
        program: impl Into<OsString>,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Self {
        WorkerCommand {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: Vec::new(),
            inherit_env: [
                "PATH",
                "HOME",
                "TMPDIR",
                "LANG",
                "LC_ALL",
                "LC_CTYPE",
                "SYSTEMROOT",
            ]
            .into_iter()
            .map(OsString::from)
            .collect(),
            current_dir: None,
        }
    }
}

/// What the pipe threads report.
#[derive(Debug)]
pub(crate) enum Incoming {
    /// One decoded line of stdout.
    Message(Json),
    /// A stdout line that is not JSON (the parse error).
    Malformed(String),
    /// A stdout line exceeded the limit; reading stopped.
    TooLong,
    /// One line of stderr.
    Stderr(String),
    /// Stdout reached end of file.
    Closed,
}

/// A running worker.
#[derive(Debug)]
pub(crate) struct Worker {
    child: Child,
    incoming: Receiver<Incoming>,
    outgoing: Option<Sender<Vec<u8>>>,
    status: Option<ExitStatus>,
}

impl Worker {
    /// Start the worker with piped stdio.
    pub(crate) fn spawn(command: &WorkerCommand, max_line: usize) -> std::io::Result<Worker> {
        let mut cmd = Command::new(&command.program);
        cmd.args(&command.args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in &command.inherit_env {
            if let Some(value) = std::env::var_os(name) {
                cmd.env(name, value);
            }
        }
        for (name, value) in &command.env {
            cmd.env(name, value);
        }
        if let Some(dir) = &command.current_dir {
            cmd.current_dir(dir);
        }
        let mut child = cmd.spawn()?;
        let (tx, incoming) = mpsc::channel();
        let pipes = (child.stdin.take(), child.stdout.take(), child.stderr.take());
        let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
            child.kill().ok();
            child.wait().ok();
            return Err(std::io::Error::other("worker pipes are missing"));
        };
        let out_tx = tx.clone();
        thread::Builder::new()
            .name("livspeil-script-stdout".into())
            .spawn(move || read_stdout(stdout, max_line, &out_tx))?;
        thread::Builder::new()
            .name("livspeil-script-stderr".into())
            .spawn(move || read_stderr(stderr, &tx))?;
        let (otx, orx) = mpsc::channel::<Vec<u8>>();
        thread::Builder::new()
            .name("livspeil-script-stdin".into())
            .spawn(move || write_stdin(stdin, &orx))?;
        Ok(Worker {
            child,
            incoming,
            outgoing: Some(otx),
            status: None,
        })
    }

    /// Queue one message (and a newline) for the worker's stdin. Silently
    /// dropped when the worker no longer reads; the caller notices through
    /// [`Incoming::Closed`] or a timeout.
    pub(crate) fn send(&self, message: &Json) {
        if let (Some(tx), Ok(mut line)) = (&self.outgoing, serde_json::to_vec(message)) {
            line.push(b'\n');
            tx.send(line).ok();
        }
    }

    /// Close the worker's stdin (end of file for the script).
    pub(crate) fn close_stdin(&mut self) {
        self.outgoing = None;
    }

    /// The next report, waiting until `deadline` at most.
    pub(crate) fn recv(&self, deadline: Instant) -> Result<Incoming, RecvTimeoutError> {
        let wait = deadline.saturating_duration_since(Instant::now());
        self.incoming.recv_timeout(wait)
    }

    /// Wait up to `grace` for the process to exit; kill it otherwise.
    /// Returns the exit status, or `None` when it had to be killed.
    pub(crate) fn finish(&mut self, grace: Duration) -> Option<ExitStatus> {
        if let Some(status) = self.status {
            return Some(status);
        }
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.status = Some(status);
                    return Some(status);
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                _ => {
                    self.kill();
                    return None;
                }
            }
        }
    }

    /// Kill the process (if still running) and reap it.
    pub(crate) fn kill(&mut self) {
        self.outgoing = None;
        if self.status.is_none() {
            self.child.kill().ok();
            if let Ok(status) = self.child.wait() {
                self.status = Some(status);
            }
        }
    }

    /// Drain stderr lines that arrived after the process ended, for at most
    /// `wait`.
    pub(crate) fn drain_stderr(&self, wait: Duration) -> Vec<String> {
        let deadline = Instant::now() + wait;
        let mut lines = Vec::new();
        while let Ok(item) = self.recv(deadline) {
            if let Incoming::Stderr(line) = item {
                lines.push(line);
            }
        }
        lines
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.kill();
    }
}

fn read_stdout(stdout: impl Read, max_line: usize, tx: &Sender<Incoming>) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    let limit = u64::try_from(max_line)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    loop {
        buf.clear();
        match (&mut reader).take(limit).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => {
                tx.send(Incoming::Closed).ok();
                return;
            }
            Ok(_) => {}
        }
        if buf.last() != Some(&b'\n') && buf.len() > max_line {
            tx.send(Incoming::TooLong).ok();
            return;
        }
        let line = buf.strip_suffix(b"\n").unwrap_or(&buf);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let item = match serde_json::from_slice(line) {
            Ok(json) => Incoming::Message(json),
            Err(err) => Incoming::Malformed(err.to_string()),
        };
        if tx.send(item).is_err() {
            return;
        }
    }
}

fn read_stderr(stderr: impl Read, tx: &Sender<Incoming>) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    let limit = u64::try_from(MAX_STDERR_LINE).unwrap_or(u64::MAX);
    loop {
        buf.clear();
        match (&mut reader).take(limit).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&buf);
        let line = text.trim_end_matches(['\n', '\r']).to_owned();
        if tx.send(Incoming::Stderr(line)).is_err() {
            return;
        }
    }
}

fn write_stdin(mut stdin: ChildStdin, rx: &Receiver<Vec<u8>>) {
    while let Ok(line) = rx.recv() {
        if stdin.write_all(&line).and_then(|()| stdin.flush()).is_err() {
            return;
        }
    }
}
