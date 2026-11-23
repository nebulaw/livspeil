// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Input event types, keystrokes and gesture velocity tracking.

use std::fmt;
use std::time::Duration;

use livspeil_ui_layout::{Point, Vec2};

/// Modifier keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    /// Shift.
    pub shift: bool,
    /// Control.
    pub ctrl: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Command / Super / Meta.
    pub cmd: bool,
}

impl Modifiers {
    /// No modifiers.
    pub const NONE: Modifiers = Modifiers {
        shift: false,
        ctrl: false,
        alt: false,
        cmd: false,
    };
    /// Shift only.
    pub const SHIFT: Modifiers = Modifiers {
        shift: true,
        ctrl: false,
        alt: false,
        cmd: false,
    };
}

/// Pointer buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// Primary.
    Left,
    /// Secondary.
    Right,
    /// Middle.
    Middle,
    /// Other buttons.
    Other(u16),
}

/// Scroll amount.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollDelta {
    /// Precise (trackpad) deltas in logical pixels.
    Pixels(Vec2),
    /// Coarse wheel deltas in lines.
    Lines(Vec2),
}

impl ScrollDelta {
    /// The delta in pixels, converting lines at `line_height`.
    #[must_use]
    pub fn pixels(&self, line_height: f32) -> Vec2 {
        match *self {
            ScrollDelta::Pixels(v) => v,
            ScrollDelta::Lines(v) => v * line_height,
        }
    }
}

/// What a pointer event is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PointerKind {
    /// Moved.
    Move,
    /// Button pressed.
    Down(MouseButton),
    /// Button released.
    Up(MouseButton),
    /// Scrolled.
    Scroll(ScrollDelta),
}

/// A pointer event in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    /// Kind.
    pub kind: PointerKind,
    /// Position in logical pixels.
    pub position: Point,
    /// Modifiers held.
    pub modifiers: Modifiers,
    /// Host timestamp (monotonic), used for velocity.
    pub time: Duration,
}

/// Named (non-character) keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(missing_docs)]
pub enum NamedKey {
    Enter,
    Tab,
    Space,
    Escape,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
}

/// A key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A named key.
    Named(NamedKey),
    /// A character key, lowercase for letters.
    Char(char),
}

impl Key {
    /// Parse a key name as used in keymaps (`a`, `tab`, `left`, `f5`, ...).
    #[must_use]
    pub fn parse(name: &str) -> Option<Key> {
        let named = match name {
            "enter" | "return" => NamedKey::Enter,
            "tab" => NamedKey::Tab,
            "space" => NamedKey::Space,
            "escape" | "esc" => NamedKey::Escape,
            "backspace" => NamedKey::Backspace,
            "delete" => NamedKey::Delete,
            "left" => NamedKey::Left,
            "right" => NamedKey::Right,
            "up" => NamedKey::Up,
            "down" => NamedKey::Down,
            "home" => NamedKey::Home,
            "end" => NamedKey::End,
            "pageup" => NamedKey::PageUp,
            "pagedown" => NamedKey::PageDown,
            _ => {
                if let Some(n) = name.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
                    && (1..=24).contains(&n)
                {
                    return Some(Key::Named(NamedKey::F(n)));
                }
                let mut chars = name.chars();
                return match (chars.next(), chars.next()) {
                    (Some(c), None) if !c.is_whitespace() && c != '-' || name == "-" => {
                        Some(Key::Char(c.to_ascii_lowercase()))
                    }
                    _ => None,
                };
            }
        };
        Some(Key::Named(named))
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Char(c) => write!(f, "{c}"),
            Key::Named(NamedKey::F(n)) => write!(f, "f{n}"),
            Key::Named(n) => {
                let s = format!("{n:?}").to_ascii_lowercase();
                f.write_str(&s)
            }
        }
    }
}

/// A key event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyEvent {
    /// The key.
    pub key: Key,
    /// Modifiers.
    pub modifiers: Modifiers,
    /// Pressed (true) or released.
    pub pressed: bool,
    /// Auto-repeat.
    pub repeat: bool,
}

impl KeyEvent {
    /// The keystroke of this event.
    #[must_use]
    pub fn keystroke(&self) -> Keystroke {
        Keystroke {
            key: self.key,
            modifiers: self.modifiers,
        }
    }
}

/// Input method editor events (composition for CJK and others).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImeEvent {
    /// IME became active.
    Enabled,
    /// Composition text with an optional selected byte range.
    Preedit {
        /// Text being composed.
        text: String,
        /// Cursor/selection within `text`.
        cursor: Option<(usize, usize)>,
    },
    /// Final text.
    Commit(String),
    /// IME became inactive.
    Disabled,
}

/// A key plus modifiers, as written in keymaps: `ctrl-shift-z`, `tab`, `cmd-,`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Keystroke {
    /// Key.
    pub key: Key,
    /// Modifiers.
    pub modifiers: Modifiers,
}

impl Keystroke {
    /// Parse `mod-mod-key`. Modifiers: `ctrl`, `alt`/`option`, `shift`,
    /// `cmd`/`super`/`meta`. A trailing `-` key is written `ctrl--`.
    ///
    /// # Errors
    /// A message naming the unknown part.
    pub fn parse(text: &str) -> Result<Keystroke, String> {
        let lower = text.trim().to_ascii_lowercase();
        if lower.is_empty() {
            return Err("empty keystroke".to_owned());
        }
        let (mods, key) = if let Some(prefix) = lower.strip_suffix("--") {
            (prefix, "-")
        } else if lower == "-" {
            ("", "-")
        } else {
            match lower.rsplit_once('-') {
                Some((m, k)) => (m, k),
                None => ("", lower.as_str()),
            }
        };
        let mut modifiers = Modifiers::NONE;
        for m in mods.split('-').filter(|m| !m.is_empty()) {
            match m {
                "ctrl" | "control" => modifiers.ctrl = true,
                "alt" | "option" => modifiers.alt = true,
                "shift" => modifiers.shift = true,
                "cmd" | "super" | "meta" => modifiers.cmd = true,
                other => return Err(format!("unknown modifier `{other}` in `{text}`")),
            }
        }
        let key = Key::parse(key).ok_or_else(|| format!("unknown key `{key}` in `{text}`"))?;
        Ok(Keystroke { key, modifiers })
    }
}

impl fmt::Display for Keystroke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (on, name) in [
            (m.ctrl, "ctrl-"),
            (m.alt, "alt-"),
            (m.shift, "shift-"),
            (m.cmd, "cmd-"),
        ] {
            if on {
                f.write_str(name)?;
            }
        }
        write!(f, "{}", self.key)
    }
}

/// A click (pointer down and up on the same element without dragging, or
/// keyboard activation).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClickEvent {
    /// Where (the element center for keyboard activation).
    pub position: Point,
    /// Modifiers.
    pub modifiers: Modifiers,
    /// Activated from the keyboard (`ui.activate`).
    pub keyboard: bool,
}

/// Drag gesture phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    /// Moved past the drag threshold.
    Start,
    /// Moved.
    Move,
    /// Released; `velocity` is the release velocity.
    End,
}

/// A drag gesture event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DragEvent {
    /// Phase.
    pub phase: DragPhase,
    /// Where the pointer went down.
    pub start: Point,
    /// Current position.
    pub position: Point,
    /// Movement since the previous drag event.
    pub delta: Vec2,
    /// Pointer velocity (px/s), from the last ~100 ms of samples.
    pub velocity: Vec2,
}

impl DragEvent {
    /// Total movement since the start.
    #[must_use]
    pub fn total(&self) -> Vec2 {
        self.position - self.start
    }
}

const SAMPLES: usize = 16;
const WINDOW: Duration = Duration::from_millis(100);

/// Estimates pointer velocity by a least-squares fit over the samples of
/// the last 100 ms. Fixed-size ring buffer: never allocates.
#[derive(Clone, Copy, Debug, Default)]
pub struct VelocityTracker {
    samples: [(f32, f32, f32); SAMPLES],
    len: usize,
    head: usize,
}

impl VelocityTracker {
    /// Forget all samples.
    pub fn reset(&mut self) {
        self.len = 0;
        self.head = 0;
    }

    /// Add a sample.
    pub fn push(&mut self, time: Duration, position: Point) {
        self.samples[self.head] = (time.as_secs_f32(), position.x, position.y);
        self.head = (self.head + 1) % SAMPLES;
        self.len = (self.len + 1).min(SAMPLES);
    }

    /// Velocity in px/s (zero with fewer than two recent samples).
    #[must_use]
    pub fn velocity(&self) -> Vec2 {
        if self.len < 2 {
            return Vec2::ZERO;
        }
        let newest = self.samples[(self.head + SAMPLES - 1) % SAMPLES].0;
        let window = WINDOW.as_secs_f32();
        let (mut n, mut st, mut sx, mut sy, mut stt, mut stx, mut sty) =
            (0.0_f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        for i in 0..self.len {
            let (t, x, y) = self.samples[(self.head + SAMPLES - 1 - i) % SAMPLES];
            let t = t - newest;
            if -t > window {
                break;
            }
            n += 1.0;
            st += t;
            sx += x;
            sy += y;
            stt += t * t;
            stx += t * x;
            sty += t * y;
        }
        let denom = n * stt - st * st;
        if n < 2.0 || denom.abs() < 1e-9 {
            return Vec2::ZERO;
        }
        Vec2::new((n * stx - st * sx) / denom, (n * sty - st * sy) / denom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keystrokes_parse_and_print() {
        let k = Keystroke::parse("Ctrl-Shift-Z").unwrap();
        assert!(k.modifiers.ctrl && k.modifiers.shift && !k.modifiers.alt);
        assert_eq!(k.key, Key::Char('z'));
        assert_eq!(k.to_string(), "ctrl-shift-z");
        assert_eq!(Keystroke::parse("ctrl--").unwrap().key, Key::Char('-'));
        assert_eq!(
            Keystroke::parse("shift-tab").unwrap().key,
            Key::Named(NamedKey::Tab)
        );
        assert_eq!(
            Keystroke::parse("f12").unwrap().key,
            Key::Named(NamedKey::F(12))
        );
        assert!(Keystroke::parse("hyper-a").is_err());
        assert!(Keystroke::parse("ctrl-banana").is_err());
    }

    #[test]
    fn velocity_fit() {
        let mut v = VelocityTracker::default();
        for i in 0..10 {
            let t = Duration::from_millis(i * 8);
            #[allow(clippy::cast_precision_loss)]
            v.push(t, Vec2::new(i as f32 * 4.0, 100.0));
        }
        let vel = v.velocity();
        assert!((vel.x - 500.0).abs() < 1.0, "{vel:?}");
        assert!(vel.y.abs() < 1e-3);
        // Old samples fall out of the window.
        v.push(Duration::from_millis(1000), Vec2::new(0.0, 0.0));
        assert_eq!(v.velocity(), Vec2::ZERO);
    }
}
