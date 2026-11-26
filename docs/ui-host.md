<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI desktop host (`livspeil-ui-host-winit`)

The desktop host runs a `livspeil-ui-core` `App` in a native window: winit
0.30 for windows and input, a wgpu surface for presentation. macOS comes
first; Linux (X11 and Wayland, both loaded at run time) also works. It
backs the gallery and the Dream Mode app (UI spec, section 11).

## Running the gallery and the example

The component gallery (`crates/livspeil-ui-gallery`) is the fully wired
host app: `cargo run -p livspeil-ui-gallery` (see `README.md`, "Try the
gallery").

```sh
cargo run -p livspeil-ui-host-winit --example hello
```

On macOS this opens a window with a full-size content view under the
native (transparent) titlebar, with a `livspeil-ui-widgets` Button, Toggle
and Slider on a Frost card over two colored shapes. It renders through
Metal, through the library's `PaintRenderer`. On Linux it opens a
decorated Wayland or X11 window and renders through Vulkan (OpenGL ES
when no Vulkan driver is installed); packages and backend selection are
in `README.md`, "On Linux".

Things to try: hover and click the button (the cursor becomes a pointing
hand), click the toggle, drag the slider (east-west resize cursor), press
Tab to move focus, and use the arrow keys on the slider. When nothing moves, the
process uses no CPU and draws no frames.

If `~/.config/livspeil/ui/` exists, `theme.toml`, `motion.toml` and
`keymap.toml` in it are hot-reloaded (see `docs/ui-config.md`).

To build without running (CI, headless machines):

```sh
cargo build -p livspeil-ui-host-winit --examples
```

## Using the host

```rust
let renderer = PaintRenderer::for_theme(PaintBridge::new(FontSystem::new()), app.theme());
renderer.install_measurer(&mut app); // layout and paint share one shaping cache
let env = Env::new(&mut app, FontSystem::new());
WidgetClipboard::install_system(&env); // copy/paste in text fields
// ... mount views ...
let options = HostOptions::default()
    .title("Gallery") // also the accessibility root's name
    .after_accessibility_action(move |app| env.sync_focus(app));
livspeil_ui_host_winit::run(app, renderer, options)?;
```

### `PaintRenderer` (feature `widgets`, on by default)

The standard `SceneRenderer`: the `livspeil-ui-paint` bridge turns each
scene into a display list plus glyph atlas uploads, which it applies to a
`livspeil-ui-render` `Renderer` before encoding and drawing the frame.
It rebuilds the GPU renderer (and re-uploads the whole glyph atlas) when
the surface format changes, and picks a blur path per frame (see "Backdrop
blur paths" below). It
also widens each text run by one pixel (`widen_text_runs`): layout rounds
boxes to whole pixels, and the paint bridge wraps text that exceeds its
box by more than half a pixel, which would break labels laid out at their
measured width. Without the feature the host has no `livspeil-ui-paint`
or `livspeil-ui-widgets` dependency.

- `HostOptions`: `WindowOptions` (title, logical size, minimum size,
  resizable, `transparent` (a hook for later), and the macOS `Titlebar`:
  `Standard`, `FullSizeContent` (the default), `Hidden`), `SurfaceOptions`
  (present mode, FIFO by default; maximum frame latency, 1 by default), an
  optional `ConfigWatcher`, and the poll interval (500 ms).
- With `Titlebar::FullSizeContent` the UI draws under the titlebar. Leave
  `Titlebar::MACOS_INSET` (28 logical px) free at the top.
- `Host` implements winit's `ApplicationHandler` if you need your own event
  loop.

### The rendering boundary

The host does not know how a `Scene` becomes GPU work:

```rust
pub trait SceneRenderer {
    fn render(&mut self, gpu: &GpuContext, target: &RenderTarget<'_>, scene: &Scene)
        -> Result<(), HostError>;
}
```

`RenderTarget` carries the surface `texture`, a `view`, its `format`,
`usage`, `size_px` and `scale` (the window's scale factor). The format is
always a non-sRGB 8-bit format (`Bgra8Unorm` on macOS). The UI composites
gamma-encoded sRGB (UI spec, section 3), so an `*Srgb` surface would
encode twice. The usage includes `TEXTURE_BINDING` when the surface
supports it; blur works either way (see "Backdrop blur paths"). The renderer submits on
`gpu.queue`, and the host presents. Errors are logged and the frame is
skipped. External renderers report failures as
`HostError::Renderer(Box<dyn Error>)`, or as `HostError::Render(RenderError)`.

## Input

`input::InputTranslator` maps winit events to core `InputEvent`s. It is
pure and tested without a window.

| winit | core |
|---|---|
| `CursorMoved` (physical px) | `Pointer(Move)` in logical px (divided by the scale factor) |
| `MouseInput` | `Pointer(Down/Up(button))` at the last position. Back/Forward become `Other(3/4)`. A release without a press is dropped |
| `MouseWheel` `LineDelta` | `Scroll(Lines)` (mouse wheels) |
| `MouseWheel` `PixelDelta` | `Scroll(Pixels)` in logical px (trackpads, precise wheels). Signs follow winit: positive moves the content right and down |
| `CursorLeft` | `PointerLeft`, deferred while a button is held (capture) |
| `ModifiersChanged` | modifiers on every event; Super is `cmd` |
| `KeyboardInput` | `Key` with `repeat`, then on press `Text` (see below). Synthetic events are ignored |
| `Ime` | `Ime(Enabled / Preedit / Commit / Disabled)`, 1:1 |
| `Focused(false)` | `Pointer(Up)` for held buttons, modifiers cleared, an empty `Preedit` if composing |

Keys: named keys map directly (Enter, Tab, Space, Escape, Backspace,
Delete, arrows, Home/End, PageUp/PageDown, F1 to F24). Character keys use the
logical character, lowercased. When Ctrl, Alt/Option or Cmd is held and the
logical character is not ASCII (Option-A gives `å` on a Mac, or a Cyrillic
layout), the physical key's US-layout character is used instead, so
`alt-a` and `ctrl-a` shortcuts match on any layout.

Text: a key press emits `Text` with winit's text unless Ctrl or Cmd is held
or the text is a control character. Option-produced characters are text.

Each frame's pointer events carry a monotonic `time`, which the core uses
for velocity.

## Accessibility

`accesskit_winit` (0.34, AccessKit 0.25) is attached to the window. The
window is created hidden, the adapter is attached, then the window is
shown (an AccessKit requirement). Adapter requests arrive as
`HostEvent::Accessibility` user events through an event loop proxy (the
event loop is `EventLoop<HostEvent>`, and `Host` implements
`ApplicationHandler<HostEvent>`).

The logic is in `Accessibility`, which needs no window and is tested with
real widgets (`tests/glue.rs`):

- Initial tree request (VoiceOver on, Cmd+F5): the full tree from
  `livspeil-ui-a11y`'s `A11yTree`. Before the app has a root, a single
  window node is sent.
- After every frame, while active: only the nodes that changed (or
  nothing). Focus changes are part of the update.
- Action requests (press, focus, blur, increment, decrement, set value,
  scroll into view) go to `livspeil_ui_a11y::handle_action`. Failures are
  logged. `HostOptions::after_accessibility_action` runs afterwards, for
  example so the widgets' `Env` sees a focus change.
- Deactivation stops updates; the next activation sends everything again.
- The window title names the root node unless the app labels it.

## Widget platform glue

`widget_platform` derives what widgets need from the retained tree, with
no window (tested headlessly):

- **Focused text input**: `text_focus(&app)` uses `App::text_input()` if
  a widget reported one, otherwise the focused element with the
  `TextInput` role (or a `SpinButton` being edited, which draws a caret or
  selection) and its descendant keyed `caret`. Its caret rectangle (the
  field's bounds while a selection is shown) places the IME candidate
  window, and IME is allowed only while there is such an input.
- **Caret blink** (`CaretBlink`): a 530 ms timer that runs only while a
  caret is drawn. Input and caret moves show the caret and restart the
  period. After 15 s without input it stops with the caret shown, so an
  idle window does not wake. While hidden, the caret element's paint
  commands are left out of the re-rendered last scene (no app frame is
  drawn for a blink). `HostOptions::caret_blink` turns it off.
- **Cursor shapes**: when no handler requested a cursor, `role_cursor`
  picks one from the nearest ancestor with a role: pointing hand for
  buttons, links, check boxes, switches, radios, tabs, combo boxes,
  options and menu items; I-beam for text inputs (and a number field
  being edited); east-west resize for sliders and number fields (drag and
  scrub); the arrow for disabled controls. While a button is held the
  shape does not change. `HostOptions::role_cursors` turns it off.
- **Clipboard for widgets**: `WidgetClipboard` adapts a host `Clipboard`
  to the widgets' `Env` clipboard; `WidgetClipboard::install_system(&env)`
  installs the system clipboard.
- **Keymap hot reload**: `keymap.toml` extends the app's keymap as it was
  when the host was created (defaults, widget keymap, app shortcuts), not
  the bare defaults; removing the file restores it
  (`Host::set_base_keymap` changes the base).

## IME

- The core reports the focused text input through `App::set_text_input`
  (or `EventCx::set_text_caret(rect)` from a handler). `App::text_input()`
  returns it only while that element has focus. Widgets that do not
  report one are found by role (see "Widget platform glue").
- After each event batch the host calls `Window::set_ime_allowed(true)` when
  a text input is focused (and `false` otherwise). It also calls
  `Window::set_ime_cursor_area` with the caret rectangle, so the candidate
  window sits next to the caret.
- Preedit cursor ranges are byte offsets into the preedit text, in winit and
  in the core. An empty preedit ends the composition, and it is still
  delivered so widgets can clear it.
- While a composition is in progress (non-empty preedit), key presses
  belong to the IME and are not delivered. This avoids, for example,
  Enter confirming a candidate and also submitting a form.
- IME commits arrive as `ImeEvent::Commit`, not as `Text`. On macOS, while
  the IME is allowed, ordinary typing also arrives as commits. Text widgets
  must therefore handle both `on_text_input` and `on_ime`.

## Window chrome and window requests

- **Linux (Wayland and X11)**: the window always has the system's
  decorations, so moving, resizing, snapping, the window menu and the
  title behave as in every other app. On Wayland the compositor draws them
  through `xdg-decoration` when it offers it (KDE, Sway, Hyprland);
  otherwise (GNOME) winit draws client-side decorations
  (`wayland-csd-adwaita-notitle`: no title text, as no fonts are bundled
  for it). `WindowOptions::titlebar` is ignored there, so an app never
  needs to draw its own titlebar on Linux.
- **macOS**: `Titlebar::FullSizeContent` (default) puts the content under
  a transparent native titlebar; `Titlebar::Hidden` also hides the native
  title, for apps that draw `livspeil_ui_widgets::Titlebar`
  (`HostOptions::titlebar`). The gallery uses `Hidden` with its own
  titlebar (`Chrome::native()` picks it on macOS and system chrome
  elsewhere).
- **Window requests**: after dispatching input (and after accessibility
  actions) the host takes `App::take_window_requests()` and applies them
  through `platform::window_action`: `Drag` calls `Window::drag_window`
  while the press is still the current event (Wayland and X11 need that),
  `Zoom` toggles `set_maximized` (like a double-click on a native
  titlebar), `Minimize` calls `set_minimized(true)`. After a successful
  drag the held buttons are released at once (`InputTranslator::
  release_buttons`), because the window manager owns the pointer until the
  move ends and the release usually never arrives; a late release is
  dropped.

## Cursor and clipboard

- Cursor: handlers call `App::request_cursor(shape)` during pointer events.
  The first request wins, so with bubbling the innermost element decides.
  `Element::cursor(shape)` installs such a handler. After a pointer move with
  no request the host shows the shape for the role under the pointer
  (`role_cursor`), or the default arrow. `CursorShape::None` hides the
  cursor. Every core shape maps to a winit cursor (`cursor_icon`),
  including the resize shapes (`ResizeEw`/`Ns`/`Nesw`/`Nwse`, column and
  row resize) that dock splitters request; menu items and check-mark
  items show the pointing hand. On Wayland the cursor theme comes from
  `XCURSOR_THEME`/`XCURSOR_SIZE`.
- Clipboard: `App::set_clipboard_text` queues a copy, and
  `App::request_paste` makes the host read the clipboard and dispatch its
  text as `InputEvent::Text`. The system clipboard uses `arboard`, text
  only. If it cannot be opened, an in-process clipboard is used.
  `Host::set_clipboard` swaps it. Text widgets use their `Env`
  clipboard directly; install `WidgetClipboard` there.

## Frame pacing

Render on demand (UI spec sections 3 and 12): when idle there are no
frames and no timers. The state machine is `pacing::Pacer`, which is pure
and tested by simulating the event loop with the real `FrameScheduler`.

1. Events are translated and dispatched as they arrive.
2. In `about_to_wait` (once per event batch) the host polls config files if
   a poll is due, serves clipboard requests, syncs the IME, then asks
   `Pacer::want_redraw(scheduler.should_draw(&app))`. A redraw is requested
   only if the app needs a frame (something is dirty or animating), the
   window is visible and no redraw is already pending.
3. On `RedrawRequested`: `FrameScheduler::begin`, `App::draw_frame(dt)`,
   `FrameScheduler::end`, then acquire, render, `pre_present_notify` and
   present. If the frame reports `needs_next_frame` (a spring is still
   moving), the next redraw is requested at once. winit delivers it at
   the display's pace, because FIFO presentation blocks on vsync. A
   system redraw (expose) with nothing dirty re-renders the last scene.
4. Occluded or zero-size (minimized) windows draw nothing. When they
   become visible again, the next batch requests a frame if one is needed.
5. Control flow is `Wait`. The only timers are the config poll, only when
   config files are watched, and the caret blink, only while a text input
   shows a caret and has had input in the last 15 s (`WaitUntil` the
   earlier of the two).

Latency: the surface uses a maximum frame latency of 1, and input
handled before a redraw is visible in that frame.

`dt` comes from `FrameScheduler`. The first frame after idle advances one
nominal interval (taken from the monitor's refresh rate, 60 Hz by
default), and stalls are clamped to 50 ms.

Frame times (CPU time from redraw to present) are collected in
`FrameTimes` (count, skipped, last, moving average, maximum), available as
`Host::frame_times()`, and logged at exit through the `log` crate.

## Backdrop blur paths

Backdrop blur samples what was drawn before it, so the frame has to be
somewhere a shader can read. `Renderer::render` picks one of three paths
from the renderer's device check and the target's format and usage (the
pure function `choose_blur_path`, unit-tested):

| Path | When | What happens |
|---|---|---|
| `Direct` | target has `TEXTURE_BINDING` (Vulkan swapchains) | draw into the target, sample it |
| `Offscreen` | render-only target (Metal drawables) | draw into an internal texture, sample that, blit it 1:1 into the target |
| `Fallback(reason)` | blur pipelines or the `Rgba16Float` level format were rejected at `Renderer::new`, or the format is not `Bgra8Unorm`/`Rgba8Unorm`, or the target is not renderable | every backdrop is drawn as a plain opaque quad of its own colour (`DisplayList::without_backdrops`, no blur, no extra tint) |

Why `Offscreen` exists: wgpu-hal 30's Metal `surface_capabilities` lists
`COLOR_TARGET | COPY_SRC | COPY_DST | STORAGE_*` and not `RESOURCE`
(`TEXTURE_BINDING`), so a Metal surface can never be sampled. The host
asks only for what the surface offers, and before this path existed the
renderer silently dropped every backdrop to an opaque quad on macOS.

Fallback is never silent (decision D63): `PaintRenderer` logs one
`warn!` with the reason when the path becomes `Fallback`, `info!` for the
other two, `PaintRenderer::on_blur_path` reports every change, and the
gallery's Materials page shows a note while blur is off.
`Renderer::blur_support()` is `Supported` or `Fallback(reason)`;
`Renderer::render` refuses a frame with backdrops in fallback
(`RenderError::BlurUnavailable`), so callers cannot hide it.

### `--gpu-info`

```sh
cargo run -p livspeil-ui-gallery -- --gpu-info
```

Opens a small window, prints the backend, adapter, driver, surface
formats, usages and present modes, the format capabilities the blur needs,
the renderer's device check and the chosen blur path, then exits (library
entry: `livspeil_ui_host_winit::gpu_info()`). Paste the output into bug
reports. `RUST_LOG=info` also logs the surface capabilities and the blur
path when the app starts.

## Frame HUD and frame log

The host measures every presented frame when `HostOptions::frame_hud` is
set (the gallery sets it). Nothing is measured, and F3 and F4 stay with the
app, when it is `None`. Pure code, unit-tested headlessly: `frame_stats`
(samples, ring buffer, statistics, log path, dump text) and `hud` (options
and the overlay's paint commands).

Per frame, in order: **build** (`App::draw_frame`: render, diff, layout,
core paint list), **acquire** (`Surface::get_current_texture`; blocks on
vsync under FIFO), **paint** (scene to display list and atlas uploads),
**encode** (display list to GPU instance data), **submit** (recording and
submitting the GPU commands; CPU time, the GPU runs asynchronously) and
**present**. The renderer reports paint, encode and submit through
`SceneRenderer::take_timings` (`RenderTimings`, also display list items,
instances, draw calls and bytes uploaded); a renderer that does not
override it reports zeros. The *interval* is the time between two
presents; gaps over 250 ms (the window was idle, as designed) are not
intervals. *Late* frames have an interval over 1.5 times the median
interval.

- **F3** shows or hides the overlay (bottom right; it covers the last 120
  frames). **F4** appends to the frame log: the graphics report, the
  summary of every stored frame (the last 16,384) and a CSV line per frame
  of the last 10 s (`HudOptions::dump_seconds`), and prints the summary to
  stderr. The log is also written once more when the window closes, and
  the file is replaced at each start. Path:
  `$XDG_STATE_HOME/livspeil/gallery-frames.log`, else
  `~/.local/state/livspeil/` (Linux), `~/Library/Logs/livspeil/` (macOS),
  `%LOCALAPPDATA%\livspeil\` (Windows); the gallery prints it at startup.
- The overlay is appended to the scene for the render only and removed
  again, so the app never sees it. It costs a little itself (a summary of
  120 frames and about a dozen text runs per frame); judge CPU times with
  it off if a few tenths of a millisecond matter.
- The overlay and the log also show the **backend**, **adapter**,
  **driver** and driver info, the **present mode** (requested and actual,
  plus what the surface supports), **surface format**, **composite alpha**,
  **frame latency**, **scale factor**, **monitor refresh rate**, surface
  size, redraws that drew nothing, and **swapchain reconfigurations**
  (suboptimal, outdated or lost surfaces: each one recreates the
  swapchain, so a count that grows every frame is a stutter by itself).
- Environment: `LIVSPEIL_PRESENT_MODE` = `fifo`, `mailbox`, `immediate` or
  `auto` (`AutoVsync`; also `autonovsync`), `LIVSPEIL_FRAME_LATENCY` = 1 to
  3 (`desired_maximum_frame_latency`; wgpu asks the swapchain for latency
  plus one images, at least the driver's minimum). The host applies them
  only where the embedding app asks (`SurfaceOptions::with_env`); the
  gallery does. A mode the surface does not support falls back to FIFO, is
  logged, and shows as `present Fifo (asked Mailbox)`. Gallery flags:
  `--hud` (or `LIVSPEIL_HUD=1`) starts with the overlay on,
  `--present-mode=MODE` beats the environment variable.

## Surface details

- Scale factor changes: positions are tracked in logical pixels. The
  following `Resized` reconfigures the surface and updates the viewport.
- `Outdated` or `Lost` surfaces are reconfigured and redrawn, and
  `Suboptimal` surfaces are reconfigured after presenting. `Timeout` and
  `Occluded` skip the frame.
- Alpha: opaque by default. Transparent windows use premultiplied alpha
  when the surface supports it.

## Deferred

- Window-level damage rectangles.
- Close and resize requests from custom titlebars (only move, zoom and
  minimize exist), and a client-drawn titlebar on Linux for setups without
  any decorations (winit always provides them on Wayland).
- Touch and pen input, and gesture events (pinch and rotate). Scroll momentum
  phases are not forwarded.
- Wayland `text-input-v3` specifics beyond what winit provides, and
  `ImePurpose` hints.
- Drag and drop of files.
- Rendering text: `livspeil-ui-paint`.
