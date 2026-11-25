// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! The winit application: window, surface, event loop glue.
//!
//! Everything with logic lives in pure modules ([`crate::input`],
//! [`crate::pacing`], [`crate::options`], [`crate::platform`]); this file
//! only moves data between winit, wgpu and the core.

use std::sync::Arc;
use std::time::{Duration, Instant};

use livspeil_ui_core::{App, CursorShape, FrameScheduler, InputEvent, Keymap, Scene};
use livspeil_ui_render::GpuContext;
use livspeil_ui_theme::ConfigWatcher;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowAttributes, WindowId};

use crate::accessibility::Accessibility;
use crate::input::{InputTranslator, KeyInput};
use crate::options::{
    ActionHook, HostOptions, SurfaceOptions, Titlebar, WindowOptions, surface_config,
};
use crate::pacing::{FrameTimes, Pacer, Wake};
use crate::platform::{Clipboard, ImeState, SystemClipboard, cursor_icon, service_clipboard};
use crate::widget_platform::{
    CaretBlink, apply_config_event, role_cursor, scene_without, text_focus,
};
use crate::{HostError, RenderTarget, SceneRenderer};

/// Open a window and run `app` in it until the window is closed.
///
/// Must be called on the main thread (macOS requirement).
///
/// # Errors
/// Event loop, window, surface and device creation failures.
pub fn run<R: SceneRenderer>(app: App, renderer: R, options: HostOptions) -> Result<(), HostError> {
    let event_loop = EventLoop::<HostEvent>::with_user_event()
        .build()
        .map_err(|e| HostError::EventLoop(e.to_string()))?;
    let mut host = Host::new(app, renderer, options);
    host.set_proxy(event_loop.create_proxy());
    event_loop
        .run_app(&mut host)
        .map_err(|e| HostError::EventLoop(e.to_string()))?;
    let t = host.frame_times();
    log::info!(
        "frames: {} drawn, {} skipped; avg {:?}, max {:?}",
        t.frames,
        t.skipped,
        t.average,
        t.max
    );
    match host.fatal.take() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// User events of the host's event loop: requests from the platform
/// accessibility adapter.
#[derive(Debug)]
pub enum HostEvent {
    /// An `accesskit_winit` event (initial tree request, action request,
    /// deactivation).
    Accessibility(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for HostEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        HostEvent::Accessibility(e)
    }
}

struct Gfx {
    // Field order: the surface must drop before the window.
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    gpu: GpuContext,
    window: Arc<Window>,
    accesskit: accesskit_winit::Adapter,
}

/// The desktop host. Usually driven by [`run`]; implements winit's
/// [`ApplicationHandler`] for custom event loops.
pub struct Host<R> {
    app: App,
    renderer: R,
    window_options: WindowOptions,
    surface_options: SurfaceOptions,
    watcher: Option<ConfigWatcher>,
    translator: InputTranslator,
    pacer: Pacer,
    scheduler: FrameScheduler,
    start: Instant,
    gfx: Option<Gfx>,
    events: Vec<InputEvent>,
    ime: ImeState,
    cursor: CursorShape,
    clipboard: Box<dyn Clipboard>,
    last_scene: Option<Scene>,
    times: FrameTimes,
    fatal: Option<HostError>,
    proxy: Option<EventLoopProxy<HostEvent>>,
    a11y: Accessibility,
    action_hook: Option<ActionHook>,
    blink: CaretBlink,
    caret_blink: bool,
    role_cursors: bool,
    base_keymap: Keymap,
}

impl<R> std::fmt::Debug for Host<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Host")
            .field("app", &self.app)
            .field("pacer", &self.pacer)
            .field("times", &self.times)
            .finish_non_exhaustive()
    }
}

const DEFAULT_INTERVAL: Duration = Duration::from_micros(16_667);

impl<R: SceneRenderer> Host<R> {
    /// A host for `app`; the window opens when the event loop resumes.
    pub fn new(app: App, renderer: R, options: HostOptions) -> Self {
        let interval = options
            .config
            .as_ref()
            .map(|_| options.config_poll_interval);
        let base_keymap = app.keymap().clone();
        let a11y = Accessibility::new(options.window.title.clone());
        Host {
            app,
            renderer,
            window_options: options.window,
            surface_options: options.surface,
            watcher: options.config,
            translator: InputTranslator::default(),
            pacer: Pacer::new(interval),
            scheduler: FrameScheduler::new(DEFAULT_INTERVAL),
            start: Instant::now(),
            gfx: None,
            events: Vec::new(),
            ime: ImeState::OFF,
            cursor: CursorShape::Default,
            clipboard: Box::new(SystemClipboard::default()),
            last_scene: None,
            times: FrameTimes::default(),
            fatal: None,
            proxy: None,
            a11y,
            action_hook: options.after_accessibility_action,
            blink: CaretBlink::new(Instant::now()),
            caret_blink: options.caret_blink,
            role_cursors: options.role_cursors,
            base_keymap,
        }
    }

    /// The event loop proxy the accessibility adapter sends its requests
    /// through ([`run`] sets it; without one there is no adapter).
    pub fn set_proxy(&mut self, proxy: EventLoopProxy<HostEvent>) {
        self.proxy = Some(proxy);
    }

    /// The keymap that `keymap.toml` extends on hot reload (by default the
    /// app's keymap when the host was created).
    pub fn set_base_keymap(&mut self, keymap: Keymap) {
        self.base_keymap = keymap;
    }

    /// Replace the clipboard (tests, sandboxes).
    pub fn set_clipboard(&mut self, clipboard: Box<dyn Clipboard>) {
        self.clipboard = clipboard;
    }

    /// The app.
    pub fn app(&mut self) -> &mut App {
        &mut self.app
    }

    /// Frame time counters.
    #[must_use]
    pub fn frame_times(&self) -> FrameTimes {
        self.times
    }

    fn window_attributes(&self) -> WindowAttributes {
        let o = &self.window_options;
        let mut attrs = Window::default_attributes()
            .with_title(o.title.clone())
            .with_inner_size(LogicalSize::new(o.size.0, o.size.1))
            .with_resizable(o.resizable)
            .with_transparent(o.transparent)
            // Shown after the accessibility adapter is attached.
            .with_visible(false);
        if let Some((w, h)) = o.min_size {
            attrs = attrs.with_min_inner_size(LogicalSize::new(w, h));
        }
        #[cfg(target_os = "macos")]
        {
            use winit::platform::macos::WindowAttributesExtMacOS;
            attrs = match o.titlebar {
                Titlebar::Standard => attrs,
                Titlebar::FullSizeContent => attrs
                    .with_fullsize_content_view(true)
                    .with_titlebar_transparent(true),
                Titlebar::Hidden => attrs
                    .with_fullsize_content_view(true)
                    .with_titlebar_transparent(true)
                    .with_title_hidden(true),
            };
        }
        #[cfg(not(target_os = "macos"))]
        let _: Titlebar = o.titlebar;
        attrs
    }

    fn create_gfx(&mut self, event_loop: &ActiveEventLoop) -> Result<Gfx, HostError> {
        let window = Arc::new(
            event_loop
                .create_window(self.window_attributes())
                .map_err(|e| HostError::Window(e.to_string()))?,
        );
        let proxy = self
            .proxy
            .clone()
            .ok_or(HostError::EventLoop("no event loop proxy".to_owned()))?;
        let accesskit = accesskit_winit::Adapter::with_event_loop_proxy(event_loop, &window, proxy);
        window.set_visible(true);
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(
                Box::new(event_loop.owned_display_handle()),
            ));
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| HostError::Surface(e.to_string()))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| HostError::NoAdapter(e.to_string()))?;
        let adapter_info = adapter.get_info();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("livspeil-ui-host-winit"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .map_err(|e| HostError::Device(e.to_string()))?;
        log::info!(
            "adapter: {} ({:?})",
            adapter_info.name,
            adapter_info.backend
        );
        let size = window.inner_size();
        let config = surface_config(
            &surface.get_capabilities(&adapter),
            (size.width, size.height),
            self.surface_options,
            self.window_options.transparent,
        )?;
        if size.width > 0 && size.height > 0 {
            surface.configure(&device, &config);
        }
        Ok(Gfx {
            surface,
            config,
            gpu: GpuContext {
                device,
                queue,
                adapter_info,
            },
            window,
            accesskit,
        })
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.pacer.set_size(size.width, size.height);
        self.app.set_viewport(self.translator.size_to_logical(size));
        let Some(gfx) = &mut self.gfx else {
            return;
        };
        if size.width == 0 || size.height == 0 {
            return;
        }
        gfx.config.width = size.width;
        gfx.config.height = size.height;
        gfx.surface.configure(&gfx.gpu.device, &gfx.config);
        // Draw at the new size right away (live resize).
        if self.pacer.want_redraw(true) {
            gfx.window.request_redraw();
        }
    }

    fn reconfigure(&mut self) {
        if let Some(gfx) = &mut self.gfx {
            let size = gfx.window.inner_size();
            if size.width > 0 && size.height > 0 {
                gfx.config.width = size.width;
                gfx.config.height = size.height;
                gfx.surface.configure(&gfx.gpu.device, &gfx.config);
            }
            // Re-render the last scene into the new swapchain.
            gfx.window.request_redraw();
        }
    }

    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn redraw(&mut self) {
        let started = Instant::now();
        if !self.pacer.redraw_started() || self.gfx.is_none() {
            self.times.skip();
            return;
        }
        let mut needs_next = false;
        if self.scheduler.should_draw(&self.app) {
            let dt = self.scheduler.begin(self.now());
            let frame = self.app.draw_frame(dt);
            self.scheduler.end(frame.needs_next_frame);
            needs_next = frame.needs_next_frame;
            self.last_scene = Some(frame.scene);
            self.after_frame();
        }
        // A system redraw (expose) with nothing new re-renders the last scene.
        let Some(scene) = &self.last_scene else {
            self.times.skip();
            return;
        };
        let blinked;
        let scene = match self.blink.hidden_element() {
            Some(caret) => {
                blinked = scene_without(scene, caret);
                &blinked
            }
            None => scene,
        };
        let Some(gfx) = &self.gfx else {
            return;
        };
        let (texture, suboptimal) = match gfx.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => (t, false),
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => (t, true),
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded
            | wgpu::CurrentSurfaceTexture::Validation => {
                self.times.skip();
                self.request_next(needs_next);
                return;
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.times.skip();
                self.reconfigure();
                return;
            }
        };
        let view = texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        #[allow(clippy::cast_possible_truncation)]
        let target = RenderTarget {
            texture: &texture.texture,
            view: &view,
            format: gfx.config.format,
            usage: gfx.config.usage,
            size_px: (gfx.config.width, gfx.config.height),
            scale: self.translator.scale() as f32,
        };
        if let Err(e) = self.renderer.render(&gfx.gpu, &target, scene) {
            log::error!("{e}");
        }
        gfx.window.pre_present_notify();
        gfx.gpu.queue.present(texture);
        self.times.record(started.elapsed());
        if suboptimal {
            self.reconfigure();
        }
        self.request_next(needs_next);
    }

    /// After a frame: send accessibility changes, follow the caret.
    fn after_frame(&mut self) {
        let scale = self.translator.scale();
        if let Some(update) = self.a11y.update(&self.app, scale)
            && let Some(gfx) = &mut self.gfx
        {
            gfx.accesskit.update_if_active(|| update);
        }
        self.follow_caret();
    }

    fn follow_caret(&mut self) {
        if !self.caret_blink {
            return;
        }
        let caret = text_focus(&self.app).and_then(|t| {
            let id = t.caret?;
            Some((id, self.app.bounds(id)?))
        });
        if self.blink.set_caret(Instant::now(), caret) {
            self.request_redraw();
        }
    }

    fn request_redraw(&mut self) {
        if self.pacer.want_redraw(true)
            && let Some(gfx) = &self.gfx
        {
            gfx.window.request_redraw();
        }
    }

    fn accessibility_event(&mut self, event: accesskit_winit::WindowEvent) {
        let scale = self.translator.scale();
        match event {
            accesskit_winit::WindowEvent::InitialTreeRequested => {
                let update = self.a11y.initial_tree(&self.app, scale);
                if let Some(gfx) = &mut self.gfx {
                    gfx.accesskit.update_if_active(|| update);
                }
            }
            accesskit_winit::WindowEvent::ActionRequested(request) => {
                match Accessibility::handle_action(&mut self.app, request) {
                    Ok(_) => {}
                    Err(e) => log::warn!("accessibility: {e}"),
                }
                if let Some(hook) = &mut self.action_hook {
                    (hook.0)(&mut self.app);
                }
                self.blink.activity(Instant::now());
            }
            accesskit_winit::WindowEvent::AccessibilityDeactivated => self.a11y.deactivate(),
        }
    }

    fn request_next(&mut self, needs_next: bool) {
        if self.pacer.frame_done(needs_next)
            && let Some(gfx) = &self.gfx
        {
            gfx.window.request_redraw();
        }
    }

    fn dispatch_events(&mut self) {
        if self.events.is_empty() {
            return;
        }
        let mut moved = false;
        let mut pointer = false;
        let mut activity = false;
        for e in self.events.drain(..) {
            match &e {
                InputEvent::Pointer(p) => {
                    pointer = true;
                    moved |= p.kind == livspeil_ui_core::PointerKind::Move;
                    activity |= p.kind != livspeil_ui_core::PointerKind::Move;
                }
                InputEvent::PointerLeft => pointer = true,
                _ => activity = true,
            }
            self.app.dispatch(e);
        }
        if activity {
            self.blink.activity(Instant::now());
        }
        if pointer {
            let requested = self.app.take_cursor_request();
            let shape = match requested {
                Some(s) => Some(s),
                // Keep the shape during a press or drag.
                None if moved && self.translator.buttons_held() => None,
                None if moved => Some(match self.translator.pointer_position() {
                    Some(p) if self.role_cursors => role_cursor(&self.app, p),
                    _ => CursorShape::Default,
                }),
                None => None,
            };
            if let Some(shape) = shape {
                self.set_cursor(shape);
            }
        }
    }

    fn set_cursor(&mut self, shape: CursorShape) {
        if shape == self.cursor {
            return;
        }
        self.cursor = shape;
        if let Some(gfx) = &self.gfx {
            match cursor_icon(shape) {
                Some(icon) => {
                    gfx.window.set_cursor_visible(true);
                    gfx.window.set_cursor(icon);
                }
                None => gfx.window.set_cursor_visible(false),
            }
        }
    }

    fn sync_ime(&mut self) {
        let want = match text_focus(&self.app) {
            Some(t) => ImeState {
                allowed: true,
                area: Some(t.area),
            },
            None => ImeState::OFF,
        };
        if want == self.ime {
            return;
        }
        let Some(gfx) = &self.gfx else {
            return;
        };
        if want.allowed != self.ime.allowed {
            gfx.window.set_ime_allowed(want.allowed);
        }
        if let Some(r) = want.area
            && want.area != self.ime.area
        {
            gfx.window.set_ime_cursor_area(
                LogicalPosition::new(f64::from(r.x), f64::from(r.y)),
                LogicalSize::new(f64::from(r.width.max(1.0)), f64::from(r.height.max(1.0))),
            );
        }
        self.ime = want;
    }

    fn poll_config(&mut self) {
        if !self.pacer.config_due(Instant::now()) {
            return;
        }
        let Some(watcher) = &mut self.watcher else {
            return;
        };
        for event in watcher.poll_now() {
            if let Err(e) = apply_config_event(&mut self.app, &self.base_keymap, event) {
                log::warn!("config: {e}");
            }
        }
    }
}

impl<R: SceneRenderer> ApplicationHandler<HostEvent> for Host<R> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.gfx.is_some() {
            return;
        }
        match self.create_gfx(event_loop) {
            Ok(gfx) => {
                let window = gfx.window.clone();
                if let Some(hz) = window
                    .current_monitor()
                    .and_then(|m| m.refresh_rate_millihertz())
                    .filter(|mhz| *mhz > 0)
                {
                    self.scheduler =
                        FrameScheduler::new(Duration::from_secs_f64(1000.0 / f64::from(hz)));
                }
                self.translator = InputTranslator::new(window.scale_factor());
                self.gfx = Some(gfx);
                self.resize(window.inner_size());
                window.set_ime_allowed(false);
                self.poll_config();
                if self.pacer.want_redraw(true) {
                    window.request_redraw();
                }
            }
            Err(e) => {
                log::error!("{e}");
                self.fatal = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let Some(gfx) = &mut self.gfx {
            gfx.accesskit.process_event(&gfx.window, &event);
        }
        let now = self.now();
        let out = &mut self.events;
        let t = &mut self.translator;
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => self.resize(size),
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                t.set_scale(scale_factor);
                // A Resized event with the new physical size follows; force
                // a full repaint for the new pixel density meanwhile.
                self.app.request_frame();
            }
            WindowEvent::Occluded(occluded) => self.pacer.set_occluded(occluded),
            WindowEvent::Focused(focused) => t.focused(focused, now, out),
            WindowEvent::ModifiersChanged(m) => t.modifiers_changed(m.state()),
            WindowEvent::CursorMoved { position, .. } => t.cursor_moved(position, now, out),
            WindowEvent::CursorLeft { .. } => t.cursor_left(out),
            WindowEvent::MouseInput { state, button, .. } => t.mouse_input(state, button, now, out),
            WindowEvent::MouseWheel { delta, .. } => t.mouse_wheel(delta, now, out),
            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => t.keyboard(&KeyInput::from(&event), out),
            WindowEvent::Ime(ime) => t.ime(ime, out),
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
        self.dispatch_events();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostEvent) {
        match event {
            HostEvent::Accessibility(e) => self.accessibility_event(e.window_event),
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_config();
        if self.blink.poll(Instant::now()) {
            self.request_redraw();
        }
        if let Some(text) = service_clipboard(&mut self.app, self.clipboard.as_mut()) {
            self.app.dispatch(InputEvent::Text(text));
        }
        self.sync_ime();
        if self
            .pacer
            .want_redraw(self.scheduler.should_draw(&self.app))
            && let Some(gfx) = &self.gfx
        {
            gfx.window.request_redraw();
        }
        let wake = match (self.pacer.wake(), self.blink.deadline()) {
            (Wake::Never, None) => None,
            (Wake::At(t), None) | (Wake::Never, Some(t)) => Some(t),
            (Wake::At(a), Some(b)) => Some(a.min(b)),
        };
        event_loop.set_control_flow(match wake {
            None => ControlFlow::Wait,
            Some(t) => ControlFlow::WaitUntil(t),
        });
    }
}
