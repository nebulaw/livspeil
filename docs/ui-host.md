<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI desktop host (`livspeil-ui-host-winit`)

The desktop host runs a `livspeil-ui-core` `App` in a native window: winit
0.30 for windows and input, a wgpu surface for presentation. macOS comes
first; Linux (X11 and Wayland, both loaded at run time) also works. It
backs the gallery and the Dream Mode app (UI spec, section 11).

## Running the example

```sh
cargo run -p livspeil-ui-host-winit --example hello
```

On macOS this opens a window with a full-size content view under the
native (transparent) titlebar, with a `livspeil-ui-widgets` Button, Toggle
and Slider on a Frost card over two colored shapes. It renders through
Metal. The example's `PaintRenderer` is a complete `SceneRenderer`: the
`livspeil-ui-paint` bridge turns each scene into a display list plus glyph
atlas uploads, which it applies to a `livspeil-ui-render` `Renderer` before
encoding and drawing the frame (it re-uploads the whole atlas when the
surface format changes, and falls back to opaque material tints when the
surface cannot be sampled for backdrop blur).

Things to try: hover and click the button (the cursor becomes a pointing
hand), click the toggle, drag the slider (grab cursor), press Tab to move
focus, and use the arrow keys on the slider. When nothing moves, the
process uses no CPU and draws no frames.

If `~/.config/livspeil/ui/` exists, `theme.toml`, `motion.toml` and
`keymap.toml` in it are hot-reloaded (see `docs/ui-config.md`).

To build without running (CI, headless machines):

```sh
cargo build -p livspeil-ui-host-winit --examples
```

## Using the host

```rust
livspeil_ui_host_winit::run(app, my_scene_renderer, HostOptions::default().title("Gallery"))?;
```

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
supports it, which backdrop blur needs. The renderer submits on
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

## IME

- The core reports the focused text input through `App::set_text_input`
  (or `EventCx::set_text_caret(rect)` from a handler). `App::text_input()`
  returns it only while that element has focus.
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

## Cursor and clipboard

- Cursor: handlers call `App::request_cursor(shape)` during pointer events.
  The first request wins, so with bubbling the innermost element decides.
  `Element::cursor(shape)` installs such a handler. After a pointer move with
  no request the host shows the default arrow. `CursorShape::None` hides the
  cursor.
- Clipboard: `App::set_clipboard_text` queues a copy, and
  `App::request_paste` makes the host read the clipboard and dispatch its
  text as `InputEvent::Text`. The system clipboard uses `arboard`, text
  only. If it cannot be opened, an in-process clipboard is used.
  `Host::set_clipboard` swaps it.

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
5. Control flow is `Wait`. The only timer is the config poll
   (`WaitUntil`), and only when config files are watched.

Latency: the surface uses a maximum frame latency of 1, and input
handled before a redraw is visible in that frame.

`dt` comes from `FrameScheduler`. The first frame after idle advances one
nominal interval (taken from the monitor's refresh rate, 60 Hz by
default), and stalls are clamped to 50 ms.

Frame times (CPU time from redraw to present) are collected in
`FrameTimes` (count, skipped, last, moving average, maximum), available as
`Host::frame_times()`, and logged at exit through the `log` crate.

## Surface details

- Scale factor changes: positions are tracked in logical pixels. The
  following `Resized` reconfigures the surface and updates the viewport.
- `Outdated` or `Lost` surfaces are reconfigured and redrawn, and
  `Suboptimal` surfaces are reconfigured after presenting. `Timeout` and
  `Occluded` skip the frame.
- Alpha: opaque by default. Transparent windows use premultiplied alpha
  when the surface supports it.

## Deferred

- Window-level damage rectangles, and a custom titlebar widget (`Titlebar::Hidden` is the hook).
- Touch and pen input, and gesture events (pinch and rotate). Scroll momentum
  phases are not forwarded.
- Wayland `text-input-v3` specifics beyond what winit provides, and
  `ImePurpose` hints.
- Drag and drop of files.
- Rendering text: `livspeil-ui-paint`.
