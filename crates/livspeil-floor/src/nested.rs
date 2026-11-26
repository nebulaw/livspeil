// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: GPL-3.0-or-later

//! The nested Floor: a compositor inside a winit window (Smithay's winit
//! backend), for development on a running desktop.

use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{ExportMem, ImportDma, gles::GlesRenderer},
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    output::Mode,
    reexports::{
        calloop::EventLoop,
        wayland_server::Display,
        winit::{dpi::LogicalSize, window::Window},
    },
    utils::{Rectangle, Transform},
};

use crate::{
    common::{BackendError, Children, Data, ctx, handle_input, insert_wayland_sources},
    geometry::Rect,
    render::{Effects, FloorRenderer, Panel},
    state::FloorState,
    stats::FrameStats,
};

/// Command-line options of `floor-nested`.
#[derive(Debug, Clone)]
pub struct Options {
    pub size: (i32, i32),
    pub panel: Option<Rect>,
    pub fx: Effects,
    /// Shell commands to start as clients once the socket exists.
    pub spawn: Vec<String>,
    /// Render this many frames continuously, animating the top window, print
    /// frame statistics and exit.
    pub bench: Option<usize>,
    /// Write a PNG of the first frame in which every spawned client is
    /// mapped, then exit.
    pub screenshot: Option<PathBuf>,
    /// Number of clients to wait for before the screenshot.
    pub expect_windows: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            size: (1280, 800),
            panel: Some(Rect::new(120, 150, 360, 96)),
            fx: Effects::default(),
            spawn: Vec::new(),
            bench: None,
            screenshot: None,
            expect_windows: 0,
        }
    }
}

/// Fatal error of the nested compositor.
pub type NestedError = BackendError;

fn e(what: &str, err: impl std::fmt::Display) -> NestedError {
    ctx(what, err)
}

type Backend = Rc<RefCell<WinitGraphicsBackend<GlesRenderer>>>;

fn handle_winit(data: &mut Data, event: WinitEvent) {
    let state = &mut data.state;
    match event {
        WinitEvent::Resized { size, .. } => {
            let mode = Mode {
                size,
                refresh: 60_000,
            };
            state
                .output
                .change_current_state(Some(mode), Some(Transform::Normal), None, None);
            state.needs_redraw = true;
        }
        WinitEvent::Redraw => state.needs_redraw = true,
        WinitEvent::CloseRequested => state.quit = true,
        WinitEvent::Focus(_) => {}
        WinitEvent::Input(input) => handle_input(state, input),
    }
}

/// Run the nested compositor until the window is closed (or the benchmark
/// or screenshot finishes).
pub fn run(opts: Options) -> Result<(), NestedError> {
    let attrs = Window::default_attributes()
        .with_inner_size(LogicalSize::new(
            f64::from(opts.size.0),
            f64::from(opts.size.1),
        ))
        .with_title("Livspeil Floor (nested)")
        .with_visible(true);
    let (backend, winit_loop) = winit::init_from_attributes::<GlesRenderer>(attrs)
        .map_err(|err| e("winit/EGL init", err))?;
    let backend: Backend = Rc::new(RefCell::new(backend));

    let mut display: Display<FloorState> =
        Display::new().map_err(|err| e("wayland display", err))?;
    let dh = display.handle();
    let win_size = backend.borrow().window_size();
    let mut state = FloorState::new(&dh, (win_size.w, win_size.h));

    // DMA-BUF: advertise what the renderer can import (D58).
    {
        let formats = backend.borrow_mut().renderer().dmabuf_formats();
        let n = formats.iter().count();
        state.dmabuf_global = Some(state.dmabuf_state.create_global::<FloorState>(&dh, formats));
        let b = backend.clone();
        state.dmabuf_importer = Some(Box::new(move |dmabuf| {
            b.borrow_mut()
                .renderer()
                .import_dmabuf(dmabuf, None)
                .is_ok()
        }));
        tracing::info!("linux-dmabuf: {n} formats advertised");
    }

    let mut renderer_state = {
        let mut b = backend.borrow_mut();
        FloorRenderer::new(b.renderer()).map_err(|err| e("shader compile", err))?
    };

    let mut event_loop: EventLoop<Data> =
        EventLoop::try_new().map_err(|err| e("event loop", err))?;
    let handle = event_loop.handle();

    let socket_name = insert_wayland_sources(&handle, &mut display)?;
    handle
        .insert_source(winit_loop, |event, _, data| handle_winit(data, event))
        .map_err(|err| e("winit source", err))?;

    println!("WAYLAND_DISPLAY={socket_name}");
    println!(
        "run clients with:  WAYLAND_DISPLAY={socket_name} foot   (Ctrl+Alt+Q or closing the window quits)"
    );

    let mut children = Children::default();
    for cmd in &opts.spawn {
        children.spawn(cmd, &socket_name);
    }

    let mut data = Data { state, display };
    let mut stats = FrameStats::default();
    let mut frames = 0usize;
    let mut shot_wait = 0usize;
    let continuous = opts.bench.is_some();
    let keep_drawing = continuous || opts.screenshot.is_some();
    let started = Instant::now();

    while !data.state.quit {
        let timeout = if keep_drawing || data.state.needs_redraw {
            Duration::ZERO
        } else {
            Duration::from_millis(250)
        };
        event_loop
            .dispatch(timeout, &mut data)
            .map_err(|err| e("dispatch", err))?;

        if continuous {
            animate(&mut data.state, frames);
        }
        if !(keep_drawing || data.state.needs_redraw) {
            continue;
        }
        data.state.needs_redraw = false;

        let size = backend.borrow().window_size();
        if size.w <= 0 || size.h <= 0 {
            continue;
        }
        let want_shot = opts.screenshot.is_some()
            && data.state.mapped_count() >= opts.expect_windows
            && opts.expect_windows > 0
            && {
                // Let the clients' second commits and the hover state settle.
                shot_wait += 1;
                shot_wait > 8
            };

        let t0 = Instant::now();
        let mut b = backend.borrow_mut();
        let (renderer, mut fb) = b.bind().map_err(|err| e("bind", err))?;
        let panel = opts.panel.map(|rect| Panel { rect });
        let sync = renderer_state
            .render(
                renderer,
                &mut fb,
                (size.w, size.h),
                Transform::Flipped180,
                &data.state,
                panel,
                opts.fx,
            )
            .map_err(|err| e("render", err))?;

        // GPU-complete time: a 1x1 read-back cannot return before the GPU
        // has executed everything queued for this framebuffer.
        let gpu_probe = if continuous {
            let region = Rectangle::from_size((1, 1).into());
            let _ = renderer.copy_framebuffer(&fb, region, Fourcc::Abgr8888);
            Some(t0.elapsed())
        } else {
            None
        };

        if want_shot && let Some(path) = &opts.screenshot {
            save_png(renderer, &fb, (size.w, size.h), path)?;
            println!("screenshot written to {}", path.display());
            data.state.quit = true;
        }
        drop(fb);
        let _ = sync;
        if let Err(err) = b.submit(None) {
            // A failed swap while shutting down (window already gone) is fine.
            if !data.state.quit {
                return Err(e("swap", err));
            }
        }
        let cpu = t0.elapsed();
        drop(b);
        if continuous {
            stats.push(cpu, gpu_probe);
        }

        // Frame callbacks and cleanup.
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
        if let Err(err) = data.display.flush_clients() {
            tracing::warn!("flush: {err}");
        }

        frames += 1;
        if let Some(n) = opts.bench
            && frames >= n
        {
            data.state.quit = true;
        }
        if started.elapsed() > Duration::from_secs(120) && opts.screenshot.is_some() {
            return Err(BackendError(
                "screenshot timed out: clients never mapped".into(),
            ));
        }
    }

    if opts.bench.is_some() {
        stats.skip_warmup(30);
        print_stats(&stats, &opts);
    }
    Ok(())
}

/// Move the top window on a Lissajous path so every frame changes.
fn animate(state: &mut FloorState, frame: usize) {
    let t = frame as f64 * 0.05;
    if let Some(w) = state.windows.last_mut() {
        w.origin = (
            (300.0 + 120.0 * t.sin()) as i32,
            (160.0 + 60.0 * (t * 1.3).cos()) as i32,
        );
    }
    state.needs_redraw = true;
}

fn print_stats(stats: &FrameStats, opts: &Options) {
    println!(
        "frame times ({}x{}, shadows={} rounded={} blur={}):",
        opts.size.0, opts.size.1, opts.fx.shadows, opts.fx.rounded, opts.fx.blur
    );
    if let Some(c) = stats.cpu() {
        println!("  cpu  (record + swap)        {c}");
    }
    if let Some(g) = stats.gpu() {
        println!("  done (until 1x1 read-back)  {g}");
    }
}

fn save_png(
    renderer: &mut GlesRenderer,
    fb: &<GlesRenderer as smithay::backend::renderer::RendererSuper>::Framebuffer<'_>,
    size: (i32, i32),
    path: &PathBuf,
) -> Result<(), NestedError> {
    let mapping = renderer
        .copy_framebuffer(fb, Rectangle::from_size(size.into()), Fourcc::Abgr8888)
        .map_err(|err| e("copy framebuffer", err))?;
    let flipped = smithay::backend::renderer::TextureMapping::flipped(&mapping);
    let bytes = renderer
        .map_texture(&mapping)
        .map_err(|err| e("map", err))?;
    let (w, h) = (size.0 as usize, size.1 as usize);
    let mut rgba = Vec::with_capacity(bytes.len());
    for row in 0..h {
        let src_row = if flipped { h - 1 - row } else { row };
        rgba.extend_from_slice(&bytes[src_row * w * 4..(src_row + 1) * w * 4]);
    }
    let file = std::fs::File::create(path).map_err(|err| e("create png", err))?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size.0 as u32, size.1 as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|err| e("png header", err))?;
    writer
        .write_image_data(&rgba)
        .map_err(|err| e("png data", err))?;
    Ok(())
}
