// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! A tiny Wayland client (smithay-client-toolkit) used as a fixture: the
//! `floor-test-client` binary shows something recognisable in the nested
//! Floor when no terminal is installed, and `tests/protocol.rs` drives it
//! over a socket pair with no display.

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_output, delegate_registry, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{DecorationMode, Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, EventQueue, QueueHandle,
    globals::registry_queue_init,
    protocol::{wl_output, wl_shm, wl_surface},
};

/// What the fixture window looks like and how it is titled.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub title: String,
    pub size: (u32, u32),
    /// Base grey level (0..=255); the content is neutral on purpose (D59).
    pub grey: u8,
    pub decorations: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig {
            title: "floor-test-client".into(),
            size: (480, 300),
            grey: 40,
            decorations: true,
        }
    }
}

/// Fixture client state; inspect it from tests.
pub struct TestClient {
    registry_state: RegistryState,
    output_state: OutputState,
    shm: Shm,
    pool: SlotPool,
    window: Window,
    cfg: ClientConfig,
    pub size: (u32, u32),
    pub configured: bool,
    pub closed: bool,
    pub decoration_mode: Option<DecorationMode>,
    pub frames_drawn: u32,
    redraw: bool,
}

impl std::fmt::Debug for TestClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestClient")
            .field("size", &self.size)
            .field("configured", &self.configured)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

/// Failure to set the fixture client up.
#[derive(Debug)]
pub struct ClientError(pub String);

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ClientError {}

fn err(e: impl std::fmt::Display) -> ClientError {
    ClientError(e.to_string())
}

impl TestClient {
    /// Connect-side setup: bind globals and create the (unmapped) window.
    pub fn new(
        conn: &Connection,
        cfg: ClientConfig,
    ) -> Result<(TestClient, EventQueue<TestClient>), ClientError> {
        let (globals, event_queue) = registry_queue_init(conn).map_err(err)?;
        let qh = event_queue.handle();
        let compositor = CompositorState::bind(&globals, &qh).map_err(err)?;
        let xdg_shell = XdgShell::bind(&globals, &qh).map_err(err)?;
        let shm = Shm::bind(&globals, &qh).map_err(err)?;
        let surface = compositor.create_surface(&qh);
        let decorations = if cfg.decorations {
            WindowDecorations::RequestServer
        } else {
            WindowDecorations::ClientOnly
        };
        let window = xdg_shell.create_window(surface, decorations, &qh);
        window.set_title(cfg.title.clone());
        window.set_app_id("org.livspeil.floor-test-client");
        window.commit();
        let pool = SlotPool::new((cfg.size.0 * cfg.size.1 * 4) as usize, &shm).map_err(err)?;
        let client = TestClient {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            shm,
            pool,
            window,
            size: cfg.size,
            cfg,
            configured: false,
            closed: false,
            decoration_mode: None,
            frames_drawn: 0,
            redraw: false,
        };
        Ok((client, event_queue))
    }

    fn draw(&mut self, qh: &QueueHandle<TestClient>) {
        let (w, h) = self.size;
        let stride = w as i32 * 4;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888)
        else {
            return;
        };
        let base = i32::from(self.cfg.grey);
        for (i, px) in canvas.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x = (i as u32 % w) as i32;
            let y = (i as u32 / w) as i32;
            // Soft vertical gradient plus a lighter block and a grid so
            // clipping at the rounded corners and scaling are visible.
            let mut v = base + y * 24 / h as i32;
            if (x / 40 + y / 40) % 2 == 0 {
                v += 6;
            }
            if x > 24 && x < 24 + 160 && y > 24 && y < 24 + 56 {
                v = 200;
            }
            let v = v.clamp(0, 255) as u8;
            *px = [v, v, v, 255];
        }
        let surface = self.window.wl_surface();
        surface.damage_buffer(0, 0, w as i32, h as i32);
        surface.frame(qh, surface.clone());
        if buffer.attach_to(surface).is_ok() {
            self.window.commit();
            self.frames_drawn += 1;
        }
    }
}

impl CompositorHandler for TestClient {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        // Redraw only on request so an idle fixture does not spin.
        if self.redraw {
            self.redraw = false;
            self.draw(qh);
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for TestClient {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for TestClient {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl WindowHandler for TestClient {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.closed = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _serial: u32,
    ) {
        self.decoration_mode = Some(configure.decoration_mode);
        self.size = (
            configure.new_size.0.map_or(self.cfg.size.0, |v| v.get()),
            configure.new_size.1.map_or(self.cfg.size.1, |v| v.get()),
        );
        if !self.configured {
            self.configured = true;
            self.draw(qh);
        } else {
            self.redraw = true;
        }
    }
}

delegate_compositor!(TestClient);
delegate_output!(TestClient);
delegate_shm!(TestClient);
delegate_xdg_shell!(TestClient);
delegate_xdg_window!(TestClient);
delegate_registry!(TestClient);

impl ProvidesRegistryState for TestClient {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}
