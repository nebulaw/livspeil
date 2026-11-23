// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Errors of the script host.

use std::fmt;
use std::time::Duration;

use crate::protocol::RemoteError;

/// Why a script session failed. After any of these except
/// [`ScriptError::InvalidState`] the worker process is gone (it exited or
/// was killed); edits it applied before stay applied and undoable.
#[derive(Debug)]
#[non_exhaustive]
pub enum ScriptError {
    /// The worker process could not be started.
    Spawn(std::io::Error),
    /// The handshake failed: the first request was not a valid `hello`.
    Handshake(String),
    /// The script asked for a protocol version this host does not speak.
    UnsupportedProtocol { requested: Json, supported: u32 },
    /// The script did not answer `method` (or say hello) within `after`;
    /// it was cancelled and then killed.
    Timeout { method: String, after: Duration },
    /// The script ran longer than its total run limit and was killed.
    RunTimeout(Duration),
    /// The script sent a line longer than the limit and was killed.
    MessageTooLarge { limit: usize },
    /// The script exited with a failure status (or was killed by a
    /// signal). `stderr` holds the last lines it wrote there.
    Crashed {
        code: Option<i32>,
        stderr: Vec<String>,
    },
    /// The script exited (successfully) while the host was waiting for
    /// its answer to `method`.
    Exited { method: String },
    /// The script answered a host request (`tick`, `event`) with an error.
    HandlerFailed { method: String, error: RemoteError },
    /// The session is not in a state that allows the call (for example
    /// `tick` before the script listens).
    InvalidState(&'static str),
}

type Json = serde_json::Value;

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::Spawn(err) => write!(f, "cannot start the script worker: {err}"),
            ScriptError::Handshake(msg) => write!(f, "handshake failed: {msg}"),
            ScriptError::UnsupportedProtocol {
                requested,
                supported,
            } => write!(
                f,
                "script requested protocol {requested}, host supports {supported}"
            ),
            ScriptError::Timeout { method, after } => {
                write!(
                    f,
                    "script did not answer '{method}' within {after:?}; killed"
                )
            }
            ScriptError::RunTimeout(limit) => {
                write!(f, "script exceeded its run time limit of {limit:?}; killed")
            }
            ScriptError::MessageTooLarge { limit } => {
                write!(f, "script sent a message over {limit} bytes; killed")
            }
            ScriptError::Crashed { code, stderr } => {
                match code {
                    Some(code) => write!(f, "script exited with status {code}")?,
                    None => f.write_str("script was terminated by a signal")?,
                }
                if let Some(last) = stderr.last() {
                    write!(f, ": {last}")?;
                }
                Ok(())
            }
            ScriptError::Exited { method } => {
                write!(f, "script exited before answering '{method}'")
            }
            ScriptError::HandlerFailed { method, error } => {
                write!(f, "script's '{method}' handler failed: {error}")
            }
            ScriptError::InvalidState(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ScriptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ScriptError::Spawn(err) => Some(err),
            ScriptError::HandlerFailed { error, .. } => Some(error),
            _ => None,
        }
    }
}
