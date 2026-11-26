// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg(target_os = "linux")]

//! Protocol-level tests: the real compositor state served over a socket
//! pair to a real Wayland client. No display, GPU or compositor needed.

use std::{
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use livspeil_floor::{
    geometry::Hit,
    state::{ClientState, FloorState},
    testclient::{ClientConfig, TestClient},
};
use smithay::reexports::wayland_server::Display;
use smithay_client_toolkit::shell::xdg::window::DecorationMode;
use wayland_client::{Connection, EventQueue};

type Job = Box<dyn FnOnce(&mut FloorState) + Send>;

/// A compositor running on its own thread.
struct Server {
    jobs: mpsc::Sender<Job>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    /// Start a compositor and connect one client stream to it.
    fn start() -> (Server, UnixStream) {
        let (server_end, client_end) = UnixStream::pair().expect("socketpair");
        let (jobs, rx) = mpsc::channel::<Job>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let thread = thread::spawn(move || {
            let mut display: Display<FloorState> = Display::new().expect("display");
            let mut state = FloorState::new(&display.handle(), (1280, 800));
            display
                .handle()
                .insert_client(server_end, Arc::new(ClientState::default()))
                .expect("insert client");
            while !stop2.load(Ordering::Relaxed) {
                while let Ok(job) = rx.try_recv() {
                    job(&mut state);
                }
                display.dispatch_clients(&mut state).expect("dispatch");
                display.flush_clients().expect("flush");
                thread::sleep(Duration::from_millis(2));
            }
        });
        (
            Server {
                jobs,
                stop,
                thread: Some(thread),
            },
            client_end,
        )
    }

    /// Run a closure on the compositor thread and return its result.
    fn with<R: Send + 'static>(&self, f: impl FnOnce(&mut FloorState) -> R + Send + 'static) -> R {
        let (tx, rx) = mpsc::channel();
        let job: Job = Box::new(move |s| {
            let _ = tx.send(f(s));
        });
        self.jobs.send(job).expect("server alive");
        rx.recv_timeout(Duration::from_secs(5))
            .expect("server answered")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Roundtrip until `done` holds or five seconds pass.
fn pump(
    queue: &mut EventQueue<TestClient>,
    client: &mut TestClient,
    mut done: impl FnMut(&TestClient) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(client) && Instant::now() < deadline {
        queue.roundtrip(client).expect("roundtrip");
    }
}

fn connect(cfg: ClientConfig) -> (Server, TestClient, EventQueue<TestClient>) {
    let (server, stream) = Server::start();
    let conn = Connection::from_socket(stream).expect("connection");
    let (client, queue) = TestClient::new(&conn, cfg).expect("client");
    (server, client, queue)
}

#[test]
fn toplevel_is_mapped_with_server_side_decoration() {
    let (server, mut client, mut queue) = connect(ClientConfig::default());
    pump(&mut queue, &mut client, |c| {
        c.configured && c.frames_drawn > 0
    });
    assert!(client.configured);
    assert_eq!(client.decoration_mode, Some(DecorationMode::Server));

    // Wait for the commit to land.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut info = None;
    while Instant::now() < deadline {
        queue.roundtrip(&mut client).expect("roundtrip");
        let got = server.with(|s| {
            s.windows
                .first()
                .map(|w| (s.mapped_count(), w.server_side, w.window.geometry().size))
        });
        if matches!(got, Some((1, _, _))) {
            info = got;
            break;
        }
    }
    let (mapped, server_side, size) = info.expect("window mapped");
    assert_eq!(mapped, 1);
    assert!(server_side, "Floor draws the titlebar");
    assert_eq!((size.w, size.h), (480, 300));
}

#[test]
fn client_without_decoration_protocol_use_gets_no_titlebar() {
    let (server, mut client, mut queue) = connect(ClientConfig {
        decorations: false,
        ..ClientConfig::default()
    });
    pump(&mut queue, &mut client, |c| {
        c.configured && c.frames_drawn > 0
    });
    queue.roundtrip(&mut client).expect("roundtrip");
    let (mapped, server_side) = server.with(|s| (s.mapped_count(), s.windows[0].server_side));
    assert_eq!(mapped, 1);
    assert!(
        !server_side,
        "client-side decorated window has no Floor titlebar"
    );
}

#[test]
fn titlebar_drag_moves_and_close_button_closes() {
    let (server, mut client, mut queue) = connect(ClientConfig::default());
    pump(&mut queue, &mut client, |c| {
        c.configured && c.frames_drawn > 0
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.with(|s| s.mapped_count()) == 0 && Instant::now() < deadline {
        queue.roundtrip(&mut client).expect("roundtrip");
    }

    // Drag the titlebar by (100, 50).
    let moved = server.with(|s| {
        let layout = s.windows[0].layout();
        let t = layout.titlebar();
        let (x, y) = (f64::from(t.x + 20), f64::from(t.y + 10));
        let start = s.windows[0].origin;
        assert_eq!(s.hit_at(x, y), Some((0, Hit::Titlebar)));
        s.pointer_motion(x, y);
        s.pointer_button(0x110, true);
        s.pointer_motion(x + 100.0, y + 50.0);
        s.pointer_button(0x110, false);
        let end = s.windows[0].origin;
        (end.0 - start.0, end.1 - start.1)
    });
    assert_eq!(moved, (100, 50));

    // A click in the client area reaches the client as pointer focus.
    let focus = server.with(|s| {
        let c = s.windows[0].layout().client_rect();
        s.pointer_motion(f64::from(c.x + 10), f64::from(c.y + 10));
        s.seat.get_pointer().map(|p| p.current_focus().is_some())
    });
    assert_eq!(focus, Some(true));

    // The close button asks the client to close.
    server.with(|s| {
        let c = s.windows[0].layout().close_button();
        s.pointer_motion(f64::from(c.x + 3), f64::from(c.y + 3));
        s.pointer_button(0x110, true);
        s.pointer_button(0x110, false);
    });
    pump(&mut queue, &mut client, |c| c.closed);
    assert!(client.closed, "xdg_toplevel.close reached the client");
}

#[test]
fn window_is_removed_when_client_disconnects() {
    let (server, mut client, mut queue) = connect(ClientConfig::default());
    pump(&mut queue, &mut client, |c| {
        c.configured && c.frames_drawn > 0
    });
    assert_eq!(server.with(|s| s.windows.len()), 1);
    drop(queue);
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.with(|s| s.windows.len()) != 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.with(|s| s.windows.len()), 0);
}
