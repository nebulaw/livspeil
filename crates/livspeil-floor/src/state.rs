// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Compositor state and Wayland protocol handlers.
//!
//! Renderer independent: nothing here touches GL, winit or DRM, so the whole
//! of it runs headless (see `tests/protocol.rs`). Backends feed it input
//! through [`FloorState::pointer_motion`] and friends and read the window
//! list back to draw it.

use std::time::Instant;

use smithay::{
    backend::{
        allocator::dmabuf::Dmabuf,
        input::{ButtonState, KeyState},
        renderer::utils::on_commit_buffer_handler,
    },
    delegate_compositor, delegate_data_device, delegate_dmabuf, delegate_output, delegate_seat,
    delegate_shm, delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{PopupKind, PopupManager, Window, WindowSurfaceType},
    input::{
        Seat, SeatHandler, SeatState,
        keyboard::{FilterResult, Keysym, XkbConfig},
        pointer::{ButtonEvent, CursorImageStatus, MotionEvent},
    },
    output::{Mode as OutputMode, Output, PhysicalProperties, Subpixel},
    reexports::{
        wayland_protocols::xdg::{
            decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
            shell::server::xdg_toplevel::State as ToplevelState,
        },
        wayland_server::{
            Client, DisplayHandle,
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_buffer::WlBuffer, wl_seat::WlSeat, wl_surface::WlSurface},
        },
    },
    utils::{Serial, Transform},
    wayland::{
        buffer::BufferHandler,
        compositor::{CompositorClientState, CompositorHandler, CompositorState, get_parent},
        dmabuf::{DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        output::OutputHandler,
        selection::{
            SelectionHandler,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
            },
        },
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            decoration::{XdgDecorationHandler, XdgDecorationState},
        },
        shm::{ShmHandler, ShmState},
    },
};

use crate::geometry::{FrameLayout, Hit};

/// A toplevel window the Floor manages.
#[derive(Debug)]
pub struct FloorWindow {
    pub window: Window,
    /// Top-left of the outer frame (titlebar included), logical pixels.
    pub origin: (i32, i32),
    /// The client bound `zxdg_toplevel_decoration_v1`, so the Floor draws
    /// the titlebar. Clients that do not (they draw their own) get only a
    /// shadow.
    pub server_side: bool,
}

impl FloorWindow {
    /// Layout of the frame around the client's current geometry.
    pub fn layout(&self) -> FrameLayout {
        let size = self.window.geometry().size;
        if self.server_side {
            FrameLayout::new(self.origin, (size.w, size.h))
        } else {
            FrameLayout::undecorated(self.origin, (size.w, size.h))
        }
    }

    /// Has the client committed a buffer yet?
    pub fn is_mapped(&self) -> bool {
        let g = self.window.geometry();
        g.size.w > 0 && g.size.h > 0
    }

    pub fn surface(&self) -> Option<WlSurface> {
        self.window.toplevel().map(|t| t.wl_surface().clone())
    }
}

/// Per-client data Smithay needs.
#[derive(Debug, Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[derive(Debug, Clone, Copy)]
struct Drag {
    /// Pointer position minus window origin at grab time.
    offset: (f64, f64),
}

/// Callback that imports a client DMA-BUF into the active renderer.
pub type DmabufImporter = Box<dyn FnMut(&Dmabuf) -> bool>;

/// The compositor.
pub struct FloorState {
    pub display_handle: DisplayHandle,
    pub start: Instant,

    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub xdg_decoration_state: XdgDecorationState,
    pub shm_state: ShmState,
    pub data_device_state: DataDeviceState,
    pub dmabuf_state: DmabufState,
    pub dmabuf_global: Option<DmabufGlobal>,
    pub dmabuf_importer: Option<DmabufImporter>,
    pub seat_state: SeatState<FloorState>,
    pub popups: PopupManager,

    pub seat: Seat<FloorState>,
    pub output: Output,

    /// Bottom to top.
    pub windows: Vec<FloorWindow>,
    pub pointer: (f64, f64),
    /// What the pointer currently hovers (for button highlights).
    pub hover: Option<(usize, Hit)>,
    drag: Option<Drag>,
    /// A press was consumed by the Floor; swallow its release.
    swallow_release: bool,
    pub quit: bool,
    pub needs_redraw: bool,
}

impl std::fmt::Debug for FloorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloorState")
            .field("windows", &self.windows.len())
            .field("pointer", &self.pointer)
            .finish_non_exhaustive()
    }
}

impl FloorState {
    pub fn new(dh: &DisplayHandle, output_size: (i32, i32)) -> Self {
        let mut seat_state = SeatState::new();
        let mut seat = seat_state.new_wl_seat(dh, "floor");
        // Defaults come from the XKB_DEFAULT_* environment. A missing XKB
        // keymap is the one failure we cannot continue past with a keyboard.
        if let Err(e) = seat.add_keyboard(XkbConfig::default(), 200, 25) {
            tracing::error!("keyboard unavailable: {e}");
        }
        seat.add_pointer();

        let output = Output::new(
            "floor-0".into(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Livspeil".into(),
                model: "Floor".into(),
            },
        );
        let mode = OutputMode {
            size: output_size.into(),
            refresh: 60_000,
        };
        output.create_global::<FloorState>(dh);
        output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((0, 0).into()),
        );
        output.set_preferred(mode);

        FloorState {
            display_handle: dh.clone(),
            start: Instant::now(),
            compositor_state: CompositorState::new::<FloorState>(dh),
            xdg_shell_state: XdgShellState::new::<FloorState>(dh),
            xdg_decoration_state: XdgDecorationState::new::<FloorState>(dh),
            shm_state: ShmState::new::<FloorState>(dh, vec![]),
            data_device_state: DataDeviceState::new::<FloorState>(dh),
            dmabuf_state: DmabufState::new(),
            dmabuf_global: None,
            dmabuf_importer: None,
            seat_state,
            popups: PopupManager::default(),
            seat,
            output,
            windows: Vec::new(),
            pointer: (0.0, 0.0),
            hover: None,
            drag: None,
            swallow_release: false,
            quit: false,
            needs_redraw: true,
        }
    }

    /// Milliseconds since start, the timestamp base for input and frames.
    pub fn time_ms(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    pub fn output_size(&self) -> (i32, i32) {
        self.output
            .current_mode()
            .map(|m| (m.size.w, m.size.h))
            .unwrap_or((0, 0))
    }

    /// Number of windows whose client has committed a buffer.
    pub fn mapped_count(&self) -> usize {
        self.windows.iter().filter(|w| w.is_mapped()).count()
    }

    fn index_of_surface(&self, surface: &WlSurface) -> Option<usize> {
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }
        self.windows
            .iter()
            .position(|w| w.surface().as_ref() == Some(&root))
    }

    /// Topmost window and hit under a point, with the surface origin of
    /// that window in compositor coordinates.
    pub fn hit_at(&self, x: f64, y: f64) -> Option<(usize, Hit)> {
        self.windows.iter().enumerate().rev().find_map(|(i, w)| {
            if !w.is_mapped() {
                return None;
            }
            let layout = w.layout();
            if w.server_side {
                match layout.hit_test(x, y) {
                    Hit::None => None,
                    hit => Some((i, hit)),
                }
            } else {
                layout
                    .client_rect()
                    .contains(x, y)
                    .then_some((i, Hit::Client))
            }
        })
    }

    /// Where the client's surface origin (0,0) sits in compositor space, so
    /// that the window geometry lands on the frame's client rectangle.
    pub fn surface_origin(&self, index: usize) -> (i32, i32) {
        let w = &self.windows[index];
        let rect = w.layout().client_rect();
        let geo = w.window.geometry();
        (rect.x - geo.loc.x, rect.y - geo.loc.y)
    }

    /// Raise and focus a window.
    pub fn focus_window(&mut self, index: usize) {
        if index >= self.windows.len() {
            return;
        }
        let win = self.windows.remove(index);
        self.windows.push(win);
        let surface = self.windows.last().and_then(|w| w.surface());
        for (i, w) in self.windows.iter().enumerate() {
            let active = i + 1 == self.windows.len();
            if w.window.set_activated(active)
                && let Some(t) = w.window.toplevel()
            {
                t.send_pending_configure();
            }
        }
        if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, surface, SERIAL_COUNTER.next_serial());
        }
        self.needs_redraw = true;
    }

    fn pointer_focus(
        &self,
        x: f64,
        y: f64,
    ) -> Option<(
        WlSurface,
        smithay::utils::Point<f64, smithay::utils::Logical>,
    )> {
        let (i, hit) = self.hit_at(x, y)?;
        if hit != Hit::Client {
            return None;
        }
        let (ox, oy) = self.surface_origin(i);
        let w = &self.windows[i];
        let rel = (x - f64::from(ox), y - f64::from(oy));
        let (surface, loc) = w.window.surface_under(rel, WindowSurfaceType::ALL)?;
        Some((
            surface,
            (f64::from(ox + loc.x), f64::from(oy + loc.y)).into(),
        ))
    }

    /// Absolute pointer motion in logical output coordinates.
    pub fn pointer_motion(&mut self, x: f64, y: f64) {
        let (w, h) = self.output_size();
        self.pointer = (
            x.clamp(0.0, f64::from(w.max(1)) - 1.0),
            y.clamp(0.0, f64::from(h.max(1)) - 1.0),
        );
        let (x, y) = self.pointer;

        if let Some(drag) = self.drag {
            if let Some(win) = self.windows.last_mut() {
                win.origin = (
                    (x - drag.offset.0).round() as i32,
                    (y - drag.offset.1).round() as i32,
                );
            }
            self.needs_redraw = true;
        }

        let hover = self
            .hit_at(x, y)
            .filter(|(_, h)| *h != Hit::Client && *h != Hit::None);
        if hover != self.hover {
            self.hover = hover;
            self.needs_redraw = true;
        }

        let focus = self.pointer_focus(x, y);
        if let Some(pointer) = self.seat.get_pointer() {
            let time = self.time_ms();
            pointer.motion(
                self,
                focus,
                &MotionEvent {
                    location: (x, y).into(),
                    serial: SERIAL_COUNTER.next_serial(),
                    time,
                },
            );
            pointer.frame(self);
        }
    }

    /// Pointer button (evdev code, e.g. 0x110 for left).
    pub fn pointer_button(&mut self, button: u32, pressed: bool) {
        let time = self.time_ms();
        let serial = SERIAL_COUNTER.next_serial();
        let (x, y) = self.pointer;
        if pressed {
            self.swallow_release = false;
            if let Some((i, hit)) = self.hit_at(x, y) {
                self.focus_window(i);
                let top = self.windows.len() - 1;
                match hit {
                    Hit::Titlebar if button == BTN_LEFT => {
                        let o = self.windows[top].origin;
                        self.drag = Some(Drag {
                            offset: (x - f64::from(o.0), y - f64::from(o.1)),
                        });
                        self.swallow_release = true;
                        return;
                    }
                    Hit::Close if button == BTN_LEFT => {
                        if let Some(t) = self.windows[top].window.toplevel() {
                            t.send_close();
                        }
                        self.swallow_release = true;
                        return;
                    }
                    Hit::Maximize | Hit::Minimize | Hit::Titlebar | Hit::Close => {
                        // Not implemented in the spike: swallow.
                        self.swallow_release = true;
                        return;
                    }
                    _ => {}
                }
            }
        } else {
            self.drag = None;
            if std::mem::take(&mut self.swallow_release) {
                return;
            }
        }
        if let Some(pointer) = self.seat.get_pointer() {
            pointer.button(
                self,
                &ButtonEvent {
                    button,
                    state: if pressed {
                        ButtonState::Pressed
                    } else {
                        ButtonState::Released
                    },
                    serial,
                    time,
                },
            );
            pointer.frame(self);
        }
    }

    /// Scroll by `h`/`v` in logical pixels (positive is right/down).
    pub fn pointer_axis(&mut self, h: f64, v: f64) {
        use smithay::{
            backend::input::{Axis, AxisSource},
            input::pointer::AxisFrame,
        };
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let mut frame = AxisFrame::new(self.time_ms()).source(AxisSource::Wheel);
        if h != 0.0 {
            frame = frame.value(Axis::Horizontal, h);
        }
        if v != 0.0 {
            frame = frame.value(Axis::Vertical, v);
        }
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    /// Keyboard key (evdev code + 8 is added by the caller as an XKB code).
    /// Returns `true` when the Floor consumed the key.
    pub fn key(&mut self, keycode: smithay::input::keyboard::Keycode, pressed: bool) {
        let Some(kb) = self.seat.get_keyboard() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        let time = self.time_ms();
        let quit = kb.input::<bool, _>(
            self,
            keycode,
            if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
            serial,
            time,
            |_, mods, handle| {
                // Ctrl+Alt+Q quits the Floor (nested and TTY).
                if pressed && mods.ctrl && mods.alt && handle.modified_sym() == Keysym::q {
                    FilterResult::Intercept(true)
                } else {
                    FilterResult::Forward
                }
            },
        );
        if quit == Some(true) {
            self.quit = true;
        }
    }

    fn place_new_window(&self) -> (i32, i32) {
        let n = self.windows.len() as i32;
        (80 + 36 * n, 80 + 36 * n)
    }
}

const BTN_LEFT: u32 = 0x110;

/// Global serial counter.
pub static SERIAL_COUNTER: smithay::utils::SerialCounter = smithay::utils::SerialCounter::new();

// ---------------------------------------------------------------- handlers

impl CompositorHandler for FloorState {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        match client.get_data::<ClientState>() {
            Some(data) => &data.compositor_state,
            // Every client is inserted with ClientState by the backends; a
            // foreign client would be a programming error, not a crash.
            None => {
                static FALLBACK: std::sync::OnceLock<CompositorClientState> =
                    std::sync::OnceLock::new();
                FALLBACK.get_or_init(CompositorClientState::default)
            }
        }
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.popups.commit(surface);

        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface) {
            if !popup.is_initial_configure_sent()
                && let Err(e) = popup.send_configure()
            {
                tracing::warn!("popup configure failed: {e}");
            }
            self.needs_redraw = true;
            return;
        }

        if let Some(i) = self.index_of_surface(surface) {
            let was_mapped = self.windows[i].is_mapped();
            self.windows[i].window.on_commit();
            if let Some(t) = self.windows[i].window.toplevel()
                && !t.is_initial_configure_sent()
            {
                t.send_configure();
            }
            if !was_mapped && self.windows[i].is_mapped() {
                self.output.enter(surface);
                self.focus_window(i);
            }
            self.needs_redraw = true;
        }
    }
}
delegate_compositor!(FloorState);

impl BufferHandler for FloorState {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for FloorState {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
delegate_shm!(FloorState);

impl SeatHandler for FloorState {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, _seat: &Seat<Self>, _focused: Option<&WlSurface>) {}

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {
        // The nested window shows the host cursor; the TTY build will draw
        // the client cursor itself.
    }
}
delegate_seat!(FloorState);

impl SelectionHandler for FloorState {
    type SelectionUserData = ();
}
impl DataDeviceHandler for FloorState {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}
impl ClientDndGrabHandler for FloorState {}
impl ServerDndGrabHandler for FloorState {}
delegate_data_device!(FloorState);

impl OutputHandler for FloorState {}
delegate_output!(FloorState);

impl XdgShellHandler for FloorState {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Ask for server-side decorations up front; clients that bind the
        // decoration protocol confirm it in `new_decoration`.
        surface.with_pending_state(|s| {
            s.states.set(ToplevelState::Activated);
        });
        let origin = self.place_new_window();
        self.windows.push(FloorWindow {
            window: Window::new_wayland_window(surface),
            origin,
            server_side: false,
        });
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        if let Err(e) = self.popups.track_popup(PopupKind::Xdg(surface)) {
            tracing::warn!("cannot track popup: {e}");
        }
    }

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {
        // Popup grabs are not implemented in the spike.
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.windows
            .retain(|w| w.window.toplevel().map(|t| t != &surface).unwrap_or(true));
        self.hover = None;
        self.needs_redraw = true;
        let next = self.windows.len().checked_sub(1);
        if let Some(i) = next {
            self.focus_window(i);
        } else if let Some(kb) = self.seat.get_keyboard() {
            kb.set_focus(self, None, SERIAL_COUNTER.next_serial());
        }
    }
}
delegate_xdg_shell!(FloorState);

impl XdgDecorationHandler for FloorState {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.set_server_side(&toplevel);
    }
    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        // The Floor owns window chrome (D58): always server side.
        self.set_server_side(&toplevel);
    }
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.set_server_side(&toplevel);
    }
}
delegate_xdg_decoration!(FloorState);

impl FloorState {
    fn set_server_side(&mut self, toplevel: &ToplevelSurface) {
        toplevel.with_pending_state(|s| s.decoration_mode = Some(DecorationMode::ServerSide));
        if let Some(w) = self
            .windows
            .iter_mut()
            .find(|w| w.window.toplevel() == Some(toplevel))
        {
            w.server_side = true;
        }
        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
        self.needs_redraw = true;
    }
}

impl DmabufHandler for FloorState {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.dmabuf_state
    }

    fn dmabuf_imported(
        &mut self,
        _global: &DmabufGlobal,
        dmabuf: Dmabuf,
        notifier: ImportNotifier,
    ) {
        let ok = self.dmabuf_importer.as_mut().is_some_and(|f| f(&dmabuf));
        if ok {
            if notifier.successful::<FloorState>().is_err() {
                tracing::debug!("dmabuf client went away during import");
            }
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(FloorState);
