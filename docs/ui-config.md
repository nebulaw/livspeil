<!--
SPDX-FileCopyrightText: 2026 Livspeil contributors
SPDX-License-Identifier: CC-BY-4.0
-->

# Livspeil UI configuration files

Livspeil UI reads plain TOML files from `~/.config/livspeil/ui/`
(`$XDG_CONFIG_HOME/livspeil/ui/` if that variable is set). Each file is
optional. A missing key keeps its built-in default, so an empty file is the
same as no file.

Parsing is strict. Unknown keys, wrong types, unknown names and out-of-range
numbers are errors, and each error gives a line and column, for example:

```
motion.toml:2:11: unknown profile `bouncy` (known: calm, fluid, off, snappy)
```

When a file cannot be parsed, the last good configuration stays in effect.

## Hot reload

`ConfigWatcher` (crate `livspeil-ui-theme`) polls the modification time of
each watched file, at most once per interval (default 250 ms). It reads a
file only when the time changes, and it reports a change only when the
contents change. Each change becomes a typed event:

| Event | When |
|---|---|
| `Motion(MotionConfig)` | `motion.toml` changed; on removal it holds the defaults |
| `Theme(Theme)` | `theme.toml` changed; on removal it holds the defaults |
| `Raw { tag, contents }` | another watched file changed (`keymap.toml` uses tag `keymap`); `contents` is `None` on removal |
| `Error { kind, error }` | the file could not be read or parsed |

The host passes the events to `App::apply_config_event`. A new motion
config takes effect at the next retarget, and animations that are already
running finish with their current spring. A new theme re-renders every view.

## `motion.toml`

```toml
profile = "snappy"          # snappy | fluid | calm | off, or a custom profile
speed = 1.0                 # global multiplier; 0 = instant
reduce_motion = false       # replaces movement with fades

[profiles.snappy]
micro    = { response = 0.10, damping = 0.90 }   # hover, press, toggle
standard = { response = 0.16, damping = 0.86 }   # menus, popovers, selection
large    = { response = 0.26, damping = 0.82 }   # panels, window tokens, camera moves

[overrides.windows]
speed = 1.2
```

| Key | Type | Default | Meaning |
|---|---|---|---|
| `profile` | string | `"snappy"` | The active profile. It must name a built-in or defined profile. |
| `speed` | number, 0 to 100 | `1.0` | Spring `response` is divided by `speed`. `0` makes every change instant. |
| `reduce_motion` | bool | `false` | Movement (position, scale) changes instantly. Appearance (color, opacity) uses the Calm spring and never overshoots. |
| `[profiles.<name>]` | table | built-ins | `micro`, `standard`, `large`, each `{ response, damping }`. For a built-in name, a missing class keeps the built-in value. A custom profile must define all three. |
| `[overrides.<scope>]` | table | none | `profile`, `speed` (multiplies the global speed) and `reduce_motion` (replaces the global value) for elements in that motion scope. |

Spring parameters:

- `response` (seconds, 0 to 10): the period of the undamped oscillation, so
  larger values are slower. `0` means instant.
- `damping` (ratio, above 0 and at most 10): `1` is critically damped, with
  no overshoot. Values below `1` bounce.

The built-in profiles are:

| Profile | micro | standard | large |
|---|---|---|---|
| snappy | 0.10 / 0.90 | 0.16 / 0.86 | 0.26 / 0.82 |
| fluid | 0.14 / 0.80 | 0.22 / 0.78 | 0.34 / 0.76 |
| calm | 0.18 / 1.0 | 0.28 / 1.0 | 0.42 / 1.0 |
| off | instant | instant | instant |

Springs are solved in closed form, so frame rate does not change the motion.
When a spring gets a new target, it keeps its current value and velocity.
At most two `large` animations run at once. A third one finishes the oldest.
Tween transitions also follow `speed` and Reduce Motion.

## `theme.toml`

```toml
scheme = "dark"             # dark | light
density = "pro"             # simple | pro
tier = "t1"                 # t0 | t1 | t2 (material quality)
reduce_transparency = false # opaque material tints

spacing = [2, 4, 8, 12, 16, 24, 32]        # exactly 7 values
type_sizes = [11, 12, 13, 15, 17, 22, 28]  # exactly 7 values
icon_sizes = [14, 16, 20, 24]              # exactly 4 values

[radii]                     # any of xs, sm, md, lg, window
md = 9

[metrics.simple]            # any of body, body_large, row_height
row_height = 30
[metrics.pro]
row_height = 22

[elevation]                 # e1..e4: one or two layers "x y blur [spread] [/ alpha]"
e2 = ["0 4 12 / 0.24", "0 1 2 / 0.2"]

[colors.dark]               # per-scheme color roles
accent = "oklch(0.70 0.15 300)"
"surface.0" = "#101014"
```

Sizes must be between 0 and 1000. Colors are `oklch(L C H)`,
`oklch(L C H / A)` (where `L` and `A` may be percentages), `#rrggbb` or
`#rrggbbaa`.

The color roles are `surface.0` to `surface.3`, `text.primary`,
`text.secondary`, `text.tertiary`, `accent`, `danger`, `warning`, `success`
and `focus-ring`. The accent is used with its lightness clamped for contrast
against `surface.1` (a lightness difference of at least 0.30).

Material tiers work as follows: T2 uses the full recipe. T1 halves the blur
and drops the saturation boost. T0, and `reduce_transparency = true`, use an
opaque tint with no blur.

## `keymap.toml`

```toml
[bindings]                  # global
"ctrl-z" = "edit.undo"

[contexts.Slider]           # only when focus is inside an element with key_context("Slider")
"shift-right" = "ui.increment_large"
```

A keystroke is written as `mod-mod-key`. The modifiers are `ctrl`,
`alt`/`option`, `shift` and `cmd`/`super`/`meta`. Keys are single
characters, `enter`, `tab`, `space`, `escape`, `backspace`, `delete`,
arrows (`left`, `right`, `up`, `down`), `home`, `end`, `pageup`, `pagedown`
and `f1` to `f24`. A `-` key is written `ctrl--`.

Action names use lowercase letters, digits, `_`, `.` and `-`. The file
extends the built-in keymap, and its bindings win. Context bindings are
tried from the innermost focused context outwards, and global bindings
come last.

The built-in actions are:

| Action | Default keys |
|---|---|
| `focus.next` / `focus.prev` | `tab` / `shift-tab` |
| `ui.activate` | `space`, `enter` (clicks the focused element if nothing handles it) |
| `ui.cancel` | `escape` |
| `ui.increment` / `ui.decrement` | Slider: `right`/`up` / `left`/`down` |
| `ui.increment_large` / `ui.decrement_large` | Slider: `shift-` arrows, `pageup` / `pagedown` |
| `ui.min` / `ui.max` | Slider: `home` / `end` |
