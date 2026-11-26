<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Floor spike: owner checklist (Hyprland, RTX 5070 Ti)

Goal: verify the nested build and the TTY build on real hardware and send
back the numbers the renderer decision needs
(`docs/decisions/floor-renderer.md`). Nothing here was run on a real GPU,
display, Hyprland or TTY by the agent that wrote it; expect surprises and
report them.

## 0. Prepare

```sh
sudo pacman -S --needed base-devel libxkbcommon seatd libinput libdrm mesa foot
# NVIDIA: nvidia-open or nvidia-dkms plus nvidia-utils, then check:
cat /sys/module/nvidia_drm/parameters/modeset      # want: Y
git checkout feat/floor-spike
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
cargo test -p livspeil-floor        # no display needed; expect 15 unit + 4 protocol tests
cargo build -p livspeil-floor --bins
cargo build -p livspeil-floor --features tty --bin floor-tty
```

For the TTY test, make sure you can `ssh` into the machine from another
device first, and note your Hyprland session's VT:
`loginctl show-session $XDG_SESSION_ID -p VTNr`.

## 1. Nested build (inside Hyprland)

```sh
RUST_LOG=info cargo run -p livspeil-floor --bin floor-nested -- --spawn foot 2>&1 | tee /tmp/floor-nested.log
```

Expect: a window "Livspeil Floor (nested)" (Hyprland may tile it; make it
floating with `hyprctl dispatch togglefloating`), and a line
`WAYLAND_DISPLAY=wayland-N` with N of 2 or more. Inside it: a dark grey
gradient, a foot window with a neutral titlebar and three grey buttons,
rounded corners, a soft shadow, and a translucent panel at (120,150)
whose backdrop (foot's text) is blurred.

Try: drag a titlebar; click the right-most button (closes the window);
hover buttons (they lighten); type in foot; scroll; Ctrl+Alt+Q quits.
Other clients: `WAYLAND_DISPLAY=wayland-N kitty` or any app.

Send back: the log (look for `GL Renderer`, `linux-dmabuf: N formats`),
what looked wrong, and screenshots:

```sh
cargo run -p livspeil-floor --bin floor-nested -- --spawn foot --screenshot /tmp/floor-foot.png
```

### Frame times

Release build, window left alone; run each and paste the output:

```sh
cargo build --release -p livspeil-floor --bins
F=target/release/floor-nested
$F --bench 600
$F --bench 600 --no-blur
$F --bench 600 --no-blur --no-shadow
$F --bench 600 --no-blur --no-shadow --no-round --no-panel
$F --bench 600 --size 3840x2160
```

While `--bench 3000` runs in another terminal: `nvidia-smi dmon -s u`
(GPU utilisation). Compare with software rendering:
`LIBGL_ALWAYS_SOFTWARE=1 $F --bench 200`. The sandbox numbers
(llvmpipe, 1280x800) are in the decision doc: 30.7 / 16.3 / 10.1 / 10.7 ms
mean CPU time for the four first variants; a real GPU should be far lower.
If bench shows a frame time near your refresh interval, that is vsync from
the host compositor, not the cost; say so.

If EGL fails to initialise, send the full `RUST_LOG=info` log and the
output of `echo $XDG_SESSION_TYPE; nvidia-smi; vulkaninfo --summary`.

## 2. TTY build (bare metal)

Do this only after section 1 works. Save your work first.

1. Switch to a free text VT (Ctrl+Alt+F3) and log in.
2. Safe probe, no modeset:
   ```sh
   cd path/to/livspeil
   target/debug/floor-tty --list
   ```
   Expect: `seat seat0, GPU /dev/dri/cardN`, your connectors and modes, and
   `output <name>: WxH@Hz`. Send this output. If it says it cannot create
   a libseat session, check `systemctl status seatd` or that you are on a
   real logind session (`loginctl`).
3. The real run, with a short safety timeout:
   ```sh
   target/debug/floor-tty --timeout 30 --spawn foot
   ```
   Expect: the screen goes dark grey, a white dot is the pointer (move
   the mouse), a foot window appears with titlebar, shadow and the blurred
   panel; typing works. Try dragging, then Ctrl+Alt+F2 (or your Hyprland
   VT) to switch away and back: the picture should return. Exit with
   Ctrl+Alt+Q or wait 30 s; you land back on the text login.
4. NVIDIA black screen? Check `modeset` is `Y` (kernel parameter
   `nvidia_drm.modeset=1`), try again with `--no-blur`, and send the log.

Send back: `/tmp/floor-tty.log`, the `--list` output, `uname -r`,
`nvidia-smi --query-gpu=name,driver_version --format=csv`,
`lspci -k | grep -A3 -i vga`, and a description (or phone photo) of what
the screen did.

## 3. Recover from a stuck TTY

In this order:

1. Wait for `--timeout` (30 s above, 120 s by default).
2. Ctrl+Alt+Q. Then Ctrl+Alt+F<your Hyprland VT>.
3. From another device: `ssh you@host`, then `pkill floor-tty` (logind or
   seatd then release the devices and restore the console), or
   `loginctl terminate-session <id>`, or `sudo chvt <n>`.
4. Keyboard dead, no ssh: Magic SysRq. Needs `sysctl kernel.sysrq` to
   allow it (set `1` beforehand: `sudo sysctl kernel.sysrq=1`).
   Alt+SysRq+K kills everything on the current VT (the Floor), then switch
   VT. If the machine is wedged: Alt+SysRq then R, E, I, S, U, B one key at
   a time, a few seconds apart (unraw, terminate, kill, sync, remount
   read-only, reboot).
5. Power button, last.

A hung Floor can block VT switching, because the kernel waits for the
session holder to acknowledge the release; that is why steps 3 and 4
exist.

## What to expect to be missing

Title text and button glyphs, window resize/maximise, layer-shell,
XWayland, fractional scaling, hardware cursor, multi-monitor and hotplug,
damage tracking (every frame redraws everything). See "Known gaps" in the
decision doc.
