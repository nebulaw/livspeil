// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Pieces shared by the nested and the TTY backends: the calloop data
//! type, the Wayland socket and display sources, backend-independent input
//! translation and client spawning.

use std::{
    process::{Child, Command},
    sync::Arc,
};

use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, ButtonState, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
        PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
    },
    reexports::{
        calloop::{Interest, LoopHandle, Mode as PollMode, PostAction, generic::Generic},
        wayland_server::Display,
    },
    wayland::socket::ListeningSocketSource,
};

use crate::state::{ClientState, FloorState};

/// Data owned by the event loop.
pub struct Data {
    pub state: FloorState,
    pub display: Display<FloorState>,
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Data").field("state", &self.state).finish_non_exhaustive()
    }
}

/// Fatal error of a backend.
#[derive(Debug)]
pub struct BackendError(pub String);

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for BackendError {}

/// Wrap an error with what was being attempted.
pub fn ctx(what: &str, err: impl std::fmt::Display) -> BackendError {
    BackendError(format!("{what}: {err}"))
}

/// Create the listening socket and the display dispatch source. Returns the
/// socket name (`wayland-N`) for `WAYLAND_DISPLAY`.
pub fn insert_wayland_sources(
    handle: &LoopHandle<'_, Data>,
    display: &mut Display<FloorState>,
) -> Result<String, BackendError> {
    let socket = ListeningSocketSource::new_auto().map_err(|e| ctx("wayland socket", e))?;
    let name = socket.socket_name().to_string_lossy().into_owned();
    handle
        .insert_source(socket, |stream, _, data| {
            if let Err(err) = data
                .display
                .handle()
                .insert_client(stream, Arc::new(ClientState::default()))
            {
                tracing::warn!("client rejected: {err}");
            }
        })
        .map_err(|e| ctx("socket source", e))?;

    // The display's poll fd wakes the loop when clients send requests.
    let poll_fd = display
        .backend()
        .poll_fd()
        .try_clone_to_owned()
        .map_err(|e| ctx("poll fd", e))?;
    handle
        .insert_source(Generic::new(poll_fd, Interest::READ, PollMode::Level), |_, _, data| {
            if let Err(err) = data.display.dispatch_clients(&mut data.state) {
                tracing::warn!("dispatch: {err}");
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| ctx("display source", e))?;
    Ok(name)
}

/// Translate backend input into Floor state changes.
pub fn handle_input<B: InputBackend>(state: &mut FloorState, event: InputEvent<B>) {
    match event {
        InputEvent::Keyboard { event } => {
            state.key(event.key_code(), event.state() == KeyState::Pressed);
        }
        InputEvent::PointerMotion { event } => {
            let (x, y) = state.pointer;
            state.pointer_motion(x + event.delta_x(), y + event.delta_y());
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let (w, h) = state.output_size();
            state.pointer_motion(event.x_transformed(w), event.y_transformed(h));
        }
        InputEvent::PointerButton { event } => {
            state.pointer_button(event.button_code(), event.state() == ButtonState::Pressed);
        }
        InputEvent::PointerAxis { event } => {
            let step = |axis| {
                event
                    .amount(axis)
                    .or_else(|| event.amount_v120(axis).map(|v| v * 15.0 / 120.0))
                    .unwrap_or(0.0)
            };
            state.pointer_axis(step(Axis::Horizontal), step(Axis::Vertical));
        }
        _ => {}
    }
}

/// Spawned clients; killed on drop.
#[derive(Debug, Default)]
pub struct Children(Vec<Child>);

impl Children {
    /// Start `cmd` through `sh -c` with `WAYLAND_DISPLAY` set (and `DISPLAY`
    /// removed so X11 apps do not leak into the host).
    pub fn spawn(&mut self, cmd: &str, socket_name: &str) {
        match Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .env("WAYLAND_DISPLAY", socket_name)
            .env_remove("DISPLAY")
            .spawn()
        {
            Ok(c) => self.0.push(c),
            Err(err) => tracing::warn!("cannot start {cmd:?}: {err}"),
        }
    }
}

impl Drop for Children {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
