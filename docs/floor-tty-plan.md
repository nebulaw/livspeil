<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Floor on bare metal: TTY session plan

Status: plan plus a skeleton (`floor-tty`, `--features tty`) that **compiles
but has never run**. Owner checklist: `docs/floor-spike-checklist.md`.

## Stack (Smithay 0.7 backends)

| Concern | Backend | Notes |
|---|---|---|
| Seat and device access without root | `session::libseat::LibSeatSession` | Talks to logind or seatd. Opens the GPU and input devices for us and hands back fds; it also receives pause/activate when the VT changes. |
| Find the GPU | `udev::primary_gpu`, `all_gpus` | Falls back to the first GPU on the seat. |
| Modesetting and page flips | `drm::DrmDevice`, `DrmSurface`, `GbmBufferedSurface` | One connector, preferred mode, atomic when the driver allows it. A vblank event releases the previous buffer. |
| Buffers | `allocator::gbm::GbmAllocator`, `GbmDevice` | Scanout-capable GBM buffers, bound to the GLES renderer as dmabufs. |
| Renderer | `GpuManager` with `GbmGlesBackend` | Creates EGL and GLES for the GBM device without `unsafe`; the same `FloorRenderer` as the nested build draws into the buffer. |
| Input | `libinput::LibinputInputBackend` | Devices opened through the session. Same `handle_input` as nested. |

## Session lifecycle

1. Start from a text TTY (a login on tty3, say). logind makes that login
   session the active one on `seat0`; `LibSeatSession::new()` takes
   control of it. Running inside another compositor fails by design.
2. Take the GPU through the session, pick the first connected connector
   (print all with `--list`, which is safe: it does not modeset).
3. Create the scanout swapchain. The first queued buffer performs the
   modeset; from then on one frame per vblank, only while something asks
   for a redraw.
4. The Wayland socket (`wayland-N`) is created exactly as in the nested
   build; `--spawn CMD` starts clients with `WAYLAND_DISPLAY` set.

## Way back to the previous session

In order of preference:

1. **Ctrl+Alt+Q** quits the Floor. The session handle drops, libseat
   releases the devices and logind restores the text console.
2. **Ctrl+Alt+F1..F12** (arrives as `XF86Switch_VT_n`) is intercepted by
   the Floor and calls `change_vt`; the session pauses (libinput suspend,
   DRM master dropped) and resumes on switching back (buffers reset, full
   redraw). This is how you return to the Hyprland VT while the Floor
   keeps running.
3. **`--timeout SECS`** (default 120 s): the loop exits by itself. A broken
   build cannot hold the screen forever.
4. From outside: `ssh` in and `pkill floor-tty`, `loginctl terminate-session`,
   or `chvt`; last resort Magic SysRq (see the checklist).

The log goes to a file (`/tmp/floor-tty.log`), not to the console the
Floor is drawing over.

## Skeleton: done and not done

Done: session, GPU selection, device open, connector and mode choice (and
`--list`), EGL/GLES on GBM, scanout swapchain, vblank-driven frames,
libinput, pause and resume on VT switch, software cursor (a small dot),
shortcuts, client spawning, safety timeout.

Not done (in the order I would do them):

1. Run it. Everything above is unverified on hardware.
2. Hotplug and multiple outputs: `UdevBackend` events, one surface and one
   `wl_output` per connector, output layout.
3. Use `DrmCompositor` (direct scanout, overlay and cursor planes) with
   damage tracking instead of one full redraw per frame.
4. Frame scheduling from vblank timestamps, presentation-time, VRR.
5. Hardware cursor plane and client cursor images (`wp_cursor_shape`).
6. Idle, lock and DPMS; `wp_linux_drm_syncobj` explicit sync.
7. A session entry: a `wayland-sessions` desktop file started by a display
   manager (greetd or SDDM) or `systemd --user`, XWayland, environment
   (`XDG_SESSION_TYPE=wayland`, `XDG_CURRENT_DESKTOP`).
8. Crash handling: restart policy so a Floor crash returns to the greeter
   instead of a dead VT.
