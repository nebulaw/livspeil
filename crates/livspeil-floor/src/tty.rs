// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The bare-metal Floor: DRM/KMS output through Smithay's session, udev,
//! DRM, GBM and libinput backends. Compiles on Linux with the `tty`
//! feature; it has NOT been run (no TTY, GPU or seat manager in the build
//! sandbox). See `docs/floor-spike-checklist.md` for how to test it.
//!
//! Flow: libseat session (logind or seatd) -> primary GPU from udev ->
//! device fd opened through the session -> `DrmDevice` + GBM -> one
//! connected connector with its preferred mode -> `GbmBufferedSurface`
//! whose buffers the same GLES renderer as the nested build draws into ->
//! page-flip on every vblank. Input comes from libinput on the same seat.
//!
//! Way back: Ctrl+Alt+Q quits (the session is released and logind/seatd
//! restore the VT), Ctrl+Alt+F1..F12 switch VT while the Floor keeps
//! running, and `--timeout SECS` exits by itself so a broken build cannot
//! hold the screen.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::drm::DrmNode,
    backend::{
        allocator::{
            Fourcc,
            gbm::{GbmAllocator, GbmBufferFlags},
        },
        drm::{DrmDevice, DrmDeviceFd, DrmEvent, GbmBufferedSurface},
        egl::EGLDevice,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            Bind, ImportDma,
            gles::GlesRenderer,
            multigpu::{GpuManager, gbm::GbmGlesBackend},
        },
        session::{Event as SessionEvent, Session, libseat::LibSeatSession},
        udev::{all_gpus, primary_gpu},
    },
    output::Mode as OutputMode,
    reexports::{
        calloop::EventLoop,
        drm::control::{Device as ControlDevice, ModeTypeFlags, connector, crtc},
        gbm::Device as GbmDevice,
        input::Libinput,
        rustix::fs::OFlags,
        wayland_server::Display,
    },
    utils::{DeviceFd, Transform},
};

use crate::{
    common::{BackendError, Children, Data, ctx, handle_input, insert_wayland_sources},
    geometry::Rect,
    render::{Effects, FloorRenderer, Panel},
    state::FloorState,
};

/// Options of `floor-tty`.
#[derive(Debug, Clone)]
pub struct Options {
    /// Exit after this long (a safety net while testing on real hardware).
    pub timeout: Option<Duration>,
    pub spawn: Vec<String>,
    pub panel: Option<Rect>,
    pub fx: Effects,
    /// Print connectors, modes and GPUs, then exit without taking the screen.
    pub list_only: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            timeout: Some(Duration::from_secs(120)),
            spawn: Vec::new(),
            panel: Some(Rect::new(120, 150, 360, 96)),
            fx: Effects {
                cursor: true,
                ..Effects::default()
            },
            list_only: false,
        }
    }
}

type GbmSurface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, ()>;

/// Everything the vblank and session callbacks touch.
struct Backend {
    drm: DrmDevice,
    surface: GbmSurface,
    gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    node: DrmNode,
    libinput: Libinput,
    /// A frame was queued and its vblank has not arrived yet.
    pending: bool,
    active: bool,
}

/// A connector we can light up.
struct Pick {
    connector: connector::Handle,
    crtc: crtc::Handle,
    mode: smithay::reexports::drm::control::Mode,
    name: String,
}

fn describe(info: &connector::Info) -> String {
    format!("{:?}-{}", info.interface(), info.interface_id())
}

/// Find the first connected connector with a usable CRTC.
fn pick_output(drm: &DrmDevice, list: bool) -> Result<Pick, BackendError> {
    let res = drm
        .resource_handles()
        .map_err(|e| ctx("drm resources", e))?;
    let mut chosen = None;
    for handle in res.connectors() {
        let Ok(info) = drm.get_connector(*handle, true) else {
            continue;
        };
        let name = describe(&info);
        let connected = info.state() == connector::State::Connected;
        if list {
            println!(
                "connector {name}: {:?}, {} modes",
                info.state(),
                info.modes().len()
            );
            for m in info.modes() {
                println!(
                    "    {}x{}@{}{}",
                    m.size().0,
                    m.size().1,
                    m.vrefresh(),
                    if m.mode_type().contains(ModeTypeFlags::PREFERRED) {
                        "  (preferred)"
                    } else {
                        ""
                    }
                );
            }
        }
        if !connected || chosen.is_some() {
            continue;
        }
        let Some(mode) = info
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .or_else(|| info.modes().first())
            .copied()
        else {
            continue;
        };
        // CRTC: the one already driving the connector, else any compatible.
        let current = info
            .current_encoder()
            .and_then(|e| drm.get_encoder(e).ok())
            .and_then(|e| e.crtc());
        let crtc = current.or_else(|| {
            info.encoders()
                .iter()
                .filter_map(|e| drm.get_encoder(*e).ok())
                .find_map(|e| res.filter_crtcs(e.possible_crtcs()).first().copied())
        });
        if let Some(crtc) = crtc {
            chosen = Some(Pick {
                connector: *handle,
                crtc,
                mode,
                name,
            });
        }
    }
    chosen.ok_or_else(|| BackendError("no connected connector with a free CRTC".into()))
}

/// Run the TTY compositor.
pub fn run(opts: Options) -> Result<(), BackendError> {
    let (session, session_notifier) = LibSeatSession::new().map_err(|e| {
        ctx(
            "libseat session (run from a TTY with seatd or logind, not from inside another compositor)",
            e,
        )
    })?;
    let seat = session.seat();
    let gpu_path = match primary_gpu(&seat).map_err(|e| ctx("udev", e))? {
        Some(p) => p,
        None => all_gpus(&seat)
            .map_err(|e| ctx("udev", e))?
            .into_iter()
            .next()
            .ok_or_else(|| BackendError(format!("no GPU on seat {seat}")))?,
    };
    println!("seat {seat}, GPU {}", gpu_path.display());
    let node = DrmNode::from_path(&gpu_path).map_err(|e| ctx("drm node", e))?;

    let mut session_for_open = session.clone();
    let fd = session_for_open
        .open(
            &gpu_path,
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
        )
        .map_err(|e| ctx("open GPU through the session", e))?;
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, drm_notifier) =
        DrmDevice::new(drm_fd.clone(), true).map_err(|e| ctx("drm device", e))?;
    let gbm = GbmDevice::new(drm_fd.clone()).map_err(|e| ctx("gbm device", e))?;

    let pick = pick_output(&drm, opts.list_only)?;
    println!(
        "output {}: {}x{}@{}",
        pick.name,
        pick.mode.size().0,
        pick.mode.size().1,
        pick.mode.vrefresh()
    );
    if opts.list_only {
        return Ok(());
    }

    // Renderer: Smithay's multi-GPU API creates the EGL display and GLES
    // context for the GBM device without us writing `unsafe`.
    let mut gpus = GpuManager::new(GbmGlesBackend::<GlesRenderer, DrmDeviceFd>::default())
        .map_err(|e| ctx("gpu manager", e))?;
    gpus.as_mut()
        .add_node(node, gbm.clone())
        .map_err(|e| ctx("EGL on the GBM device", e))?;
    let (render_formats, mut floor_renderer) = {
        let mut r = gpus
            .single_renderer(&node)
            .map_err(|e| ctx("renderer", e))?;
        let gles: &mut GlesRenderer = r.as_mut();
        let formats = gles.dmabuf_formats();
        (
            formats,
            FloorRenderer::new(gles).map_err(|e| ctx("shader compile", e))?,
        )
    };
    if let Ok(dev) = EGLDevice::device_for_display(
        gpus.single_renderer(&node)
            .map_err(|e| ctx("renderer", e))?
            .as_mut()
            .egl_context()
            .display(),
    ) {
        println!("EGL device: {:?}", dev.try_get_render_node());
    }

    let crtc_surface = {
        let mut drm = drm;
        let surface = drm
            .create_surface(pick.crtc, pick.mode, &[pick.connector])
            .map_err(|e| ctx("drm surface", e))?;
        let allocator = GbmAllocator::new(
            gbm.clone(),
            GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT,
        );
        let gbm_surface = GbmBufferedSurface::new(
            surface,
            allocator,
            &[Fourcc::Xrgb8888, Fourcc::Argb8888],
            render_formats.iter().copied(),
        )
        .map_err(|e| ctx("scanout swapchain", e))?;
        (drm, gbm_surface)
    };
    let (drm, surface) = crtc_surface;

    // libinput on the same seat, with device access through the session.
    let mut libinput =
        Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(session.clone().into());
    libinput
        .udev_assign_seat(&seat)
        .map_err(|()| BackendError(format!("libinput could not assign seat {seat}")))?;
    let input_backend = LibinputInputBackend::new(libinput.clone());

    let mut display: Display<FloorState> = Display::new().map_err(|e| ctx("wayland display", e))?;
    let size = pick.mode.size();
    let mut state = FloorState::new(&display.handle(), (i32::from(size.0), i32::from(size.1)));
    state.output.change_current_state(
        Some(OutputMode {
            size: (i32::from(size.0), i32::from(size.1)).into(),
            refresh: pick.mode.vrefresh() as i32 * 1000,
        }),
        Some(Transform::Normal),
        None,
        None,
    );
    state.pointer = (f64::from(size.0) / 2.0, f64::from(size.1) / 2.0);

    let backend = Rc::new(RefCell::new(Backend {
        drm,
        surface,
        gpus,
        node,
        libinput,
        pending: false,
        active: true,
    }));

    let mut event_loop: EventLoop<Data> = EventLoop::try_new().map_err(|e| ctx("event loop", e))?;
    let handle = event_loop.handle();
    let socket_name = insert_wayland_sources(&handle, &mut display)?;

    handle
        .insert_source(input_backend, |event, _, data| {
            handle_input(&mut data.state, event)
        })
        .map_err(|e| ctx("libinput source", e))?;

    // VT switch away (pause) and back (activate).
    {
        let backend = backend.clone();
        handle
            .insert_source(session_notifier, move |event, _, data| {
                let mut b = backend.borrow_mut();
                match event {
                    SessionEvent::PauseSession => {
                        tracing::info!("session paused (VT switched away)");
                        b.libinput.suspend();
                        b.drm.pause();
                        b.active = false;
                    }
                    SessionEvent::ActivateSession => {
                        tracing::info!("session activated");
                        if b.libinput.resume().is_err() {
                            tracing::warn!("libinput resume failed");
                        }
                        if let Err(e) = b.drm.activate(true) {
                            tracing::warn!("drm activate failed: {e}");
                        }
                        b.surface.reset_buffers();
                        b.pending = false;
                        b.active = true;
                        data.state.needs_redraw = true;
                    }
                }
            })
            .map_err(|e| ctx("session source", e))?;
    }

    // Page-flip completion.
    {
        let backend = backend.clone();
        handle
            .insert_source(drm_notifier, move |event, _, data| match event {
                DrmEvent::VBlank(_) => {
                    let mut b = backend.borrow_mut();
                    if let Err(e) = b.surface.frame_submitted() {
                        tracing::warn!("frame_submitted: {e}");
                    }
                    b.pending = false;
                    // Keep animating only while something asks for it.
                    let _ = data;
                }
                DrmEvent::Error(e) => tracing::error!("drm: {e}"),
            })
            .map_err(|e| ctx("drm source", e))?;
    }

    println!("WAYLAND_DISPLAY={socket_name}");
    println!("Ctrl+Alt+Q quits, Ctrl+Alt+F1..F12 switch VT");
    let mut children = Children::default();
    for cmd in &opts.spawn {
        children.spawn(cmd, &socket_name);
    }

    let mut data = Data { state, display };
    let started = Instant::now();
    let panel = opts.panel.map(|rect| Panel { rect });

    while !data.state.quit {
        if let Some(limit) = opts.timeout
            && started.elapsed() > limit
        {
            println!("timeout reached, exiting");
            break;
        }
        let timeout = if data.state.needs_redraw {
            Duration::from_millis(1)
        } else {
            Duration::from_millis(100)
        };
        event_loop
            .dispatch(timeout, &mut data)
            .map_err(|e| ctx("dispatch", e))?;

        if let Some(vt) = data.state.vt_switch.take() {
            let mut s = session.clone();
            if let Err(e) = s.change_vt(vt) {
                tracing::warn!("VT switch to {vt} failed: {e}");
            }
        }

        let mut b = backend.borrow_mut();
        if b.active && !b.pending && data.state.needs_redraw {
            data.state.needs_redraw = false;
            let t0 = Instant::now();
            let node = b.node;
            let (mut dmabuf, _age) = b.surface.next_buffer().map_err(|e| ctx("next buffer", e))?;
            let sync = {
                let mut r = b
                    .gpus
                    .single_renderer(&node)
                    .map_err(|e| ctx("renderer", e))?;
                let gles: &mut GlesRenderer = r.as_mut();
                let mut target = gles
                    .bind(&mut dmabuf)
                    .map_err(|e| ctx("bind scanout buffer", e))?;
                floor_renderer
                    .render(
                        gles,
                        &mut target,
                        (i32::from(size.0), i32::from(size.1)),
                        Transform::Normal,
                        &data.state,
                        panel,
                        opts.fx,
                    )
                    .map_err(|e| ctx("render", e))?
            };
            b.surface
                .queue_buffer(Some(sync), None, ())
                .map_err(|e| ctx("queue buffer", e))?;
            b.pending = true;
            tracing::debug!("frame {:?}", t0.elapsed());

            let time = data.state.time_ms();
            let output = data.state.output.clone();
            for w in &data.state.windows {
                w.window.send_frame(
                    &output,
                    Duration::from_millis(u64::from(time)),
                    Some(Duration::ZERO),
                    |_, _| Some(output.clone()),
                );
            }
            data.state.popups.cleanup();
            if let Err(e) = data.display.flush_clients() {
                tracing::warn!("flush: {e}");
            }
        }
    }
    Ok(())
}
