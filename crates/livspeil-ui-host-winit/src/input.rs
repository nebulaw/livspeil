// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! winit → `livspeil-ui-core` input translation.
//!
//! [`InputTranslator`] is pure: it takes the public parts of winit events
//! (which tests can build without a window) and appends core
//! [`InputEvent`]s to a caller-owned buffer, so steady-state translation
//! does not allocate.

use std::time::Duration;

use livspeil_ui_core::{
    ImeEvent, InputEvent, Key, KeyEvent, Modifiers, MouseButton, NamedKey, PointerEvent,
    PointerKind, ScrollDelta,
};
use livspeil_ui_layout::{Point, Vec2};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, MouseScrollDelta};
use winit::keyboard::{self, KeyCode, ModifiersState, PhysicalKey, SmolStr};

/// The public fields of a winit `KeyEvent` (which cannot be built outside
/// winit), so translation can be tested without a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyInput {
    /// The key with the layout and modifiers applied.
    pub logical: keyboard::Key,
    /// The key's position on the keyboard.
    pub physical: PhysicalKey,
    /// Pressed or released.
    pub state: ElementState,
    /// Auto-repeat.
    pub repeat: bool,
    /// The text the key produces, if any.
    pub text: Option<SmolStr>,
}

impl From<&winit::event::KeyEvent> for KeyInput {
    fn from(e: &winit::event::KeyEvent) -> Self {
        KeyInput {
            logical: e.logical_key.clone(),
            physical: e.physical_key,
            state: e.state,
            repeat: e.repeat,
            text: e.text.clone(),
        }
    }
}

/// Translates winit window events into core input events, tracking the
/// pointer position, held buttons, modifiers, scale factor and IME
/// composition.
#[derive(Clone, Debug)]
pub struct InputTranslator {
    scale: f64,
    position: Option<Point>,
    modifiers: Modifiers,
    buttons: Vec<MouseButton>,
    composing: bool,
}

impl Default for InputTranslator {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Map winit modifiers (Super is Command on macOS).
#[must_use]
pub fn modifiers(state: ModifiersState) -> Modifiers {
    Modifiers {
        shift: state.shift_key(),
        ctrl: state.control_key(),
        alt: state.alt_key(),
        cmd: state.super_key(),
    }
}

/// Map a winit mouse button.
#[must_use]
pub fn mouse_button(button: winit::event::MouseButton) -> MouseButton {
    use winit::event::MouseButton as W;
    match button {
        W::Left => MouseButton::Left,
        W::Right => MouseButton::Right,
        W::Middle => MouseButton::Middle,
        W::Back => MouseButton::Other(3),
        W::Forward => MouseButton::Other(4),
        W::Other(n) => MouseButton::Other(n),
    }
}

/// Map a winit named key; `None` for keys the core does not know.
#[must_use]
pub fn named_key(key: keyboard::NamedKey) -> Option<Key> {
    use keyboard::NamedKey as W;
    let n = match key {
        W::Enter => NamedKey::Enter,
        W::Tab => NamedKey::Tab,
        W::Space => NamedKey::Space,
        W::Escape => NamedKey::Escape,
        W::Backspace => NamedKey::Backspace,
        W::Delete => NamedKey::Delete,
        W::ArrowLeft => NamedKey::Left,
        W::ArrowRight => NamedKey::Right,
        W::ArrowUp => NamedKey::Up,
        W::ArrowDown => NamedKey::Down,
        W::Home => NamedKey::Home,
        W::End => NamedKey::End,
        W::PageUp => NamedKey::PageUp,
        W::PageDown => NamedKey::PageDown,
        W::F1 => NamedKey::F(1),
        W::F2 => NamedKey::F(2),
        W::F3 => NamedKey::F(3),
        W::F4 => NamedKey::F(4),
        W::F5 => NamedKey::F(5),
        W::F6 => NamedKey::F(6),
        W::F7 => NamedKey::F(7),
        W::F8 => NamedKey::F(8),
        W::F9 => NamedKey::F(9),
        W::F10 => NamedKey::F(10),
        W::F11 => NamedKey::F(11),
        W::F12 => NamedKey::F(12),
        W::F13 => NamedKey::F(13),
        W::F14 => NamedKey::F(14),
        W::F15 => NamedKey::F(15),
        W::F16 => NamedKey::F(16),
        W::F17 => NamedKey::F(17),
        W::F18 => NamedKey::F(18),
        W::F19 => NamedKey::F(19),
        W::F20 => NamedKey::F(20),
        W::F21 => NamedKey::F(21),
        W::F22 => NamedKey::F(22),
        W::F23 => NamedKey::F(23),
        W::F24 => NamedKey::F(24),
        _ => return None,
    };
    Some(Key::Named(n))
}

/// The US-layout character of a physical key (letters lowercase), used for
/// shortcuts when the layout or Option/Alt turns the logical key into
/// another character (`alt-a` is `å` on a Mac; `ctrl-ф` on a Russian
/// layout should still match `ctrl-a`).
#[must_use]
pub fn physical_char(code: KeyCode) -> Option<char> {
    use KeyCode as K;
    let c = match code {
        K::KeyA => 'a',
        K::KeyB => 'b',
        K::KeyC => 'c',
        K::KeyD => 'd',
        K::KeyE => 'e',
        K::KeyF => 'f',
        K::KeyG => 'g',
        K::KeyH => 'h',
        K::KeyI => 'i',
        K::KeyJ => 'j',
        K::KeyK => 'k',
        K::KeyL => 'l',
        K::KeyM => 'm',
        K::KeyN => 'n',
        K::KeyO => 'o',
        K::KeyP => 'p',
        K::KeyQ => 'q',
        K::KeyR => 'r',
        K::KeyS => 's',
        K::KeyT => 't',
        K::KeyU => 'u',
        K::KeyV => 'v',
        K::KeyW => 'w',
        K::KeyX => 'x',
        K::KeyY => 'y',
        K::KeyZ => 'z',
        K::Digit0 => '0',
        K::Digit1 => '1',
        K::Digit2 => '2',
        K::Digit3 => '3',
        K::Digit4 => '4',
        K::Digit5 => '5',
        K::Digit6 => '6',
        K::Digit7 => '7',
        K::Digit8 => '8',
        K::Digit9 => '9',
        K::Minus => '-',
        K::Equal => '=',
        K::BracketLeft => '[',
        K::BracketRight => ']',
        K::Backslash => '\\',
        K::Semicolon => ';',
        K::Quote => '\'',
        K::Backquote => '`',
        K::Comma => ',',
        K::Period => '.',
        K::Slash => '/',
        _ => return None,
    };
    Some(c)
}

/// The core key for a key event: named keys map directly; character keys
/// use the logical character, unless a shortcut modifier (Ctrl, Alt,
/// Cmd) is held and the logical character is not ASCII, in which case the
/// physical key's US-layout character is used.
#[must_use]
pub fn key(logical: &keyboard::Key, physical: PhysicalKey, mods: Modifiers) -> Option<Key> {
    let from_physical = || match physical {
        PhysicalKey::Code(code) => physical_char(code).map(Key::Char),
        PhysicalKey::Unidentified(_) => None,
    };
    match logical {
        keyboard::Key::Named(n) => named_key(*n).or_else(from_physical),
        keyboard::Key::Character(s) => {
            let mut chars = s.chars();
            let c = match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            };
            let shortcut = mods.ctrl || mods.alt || mods.cmd;
            match c {
                Some(' ') => Some(Key::Named(NamedKey::Space)),
                Some(c) if !(shortcut && !c.is_ascii()) => Some(Key::Char(c.to_ascii_lowercase())),
                _ => from_physical(),
            }
        }
        _ => from_physical(),
    }
}

impl InputTranslator {
    /// A translator for a window at `scale` physical pixels per logical pixel.
    #[must_use]
    pub fn new(scale: f64) -> Self {
        InputTranslator {
            scale: if scale > 0.0 { scale } else { 1.0 },
            position: None,
            modifiers: Modifiers::NONE,
            buttons: Vec::new(),
            composing: false,
        }
    }

    /// The scale factor.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// The window's scale factor changed.
    pub fn set_scale(&mut self, scale: f64) {
        if scale > 0.0 {
            // Positions are stored in logical pixels and stay valid.
            self.scale = scale;
        }
    }

    /// Current modifiers.
    #[must_use]
    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    /// Last pointer position in logical pixels.
    #[must_use]
    pub fn pointer_position(&self) -> Option<Point> {
        self.position
    }

    /// Whether a mouse button is held (a press or drag is in progress).
    #[must_use]
    pub fn buttons_held(&self) -> bool {
        !self.buttons.is_empty()
    }

    /// Whether an IME composition (non-empty preedit) is in progress.
    #[must_use]
    pub fn is_composing(&self) -> bool {
        self.composing
    }

    /// Physical pixels to logical pixels.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn to_logical(&self, p: PhysicalPosition<f64>) -> Point {
        Point::new((p.x / self.scale) as f32, (p.y / self.scale) as f32)
    }

    /// A physical size in logical pixels.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn size_to_logical(&self, s: PhysicalSize<u32>) -> livspeil_ui_layout::Size {
        livspeil_ui_layout::Size::new(
            (f64::from(s.width) / self.scale) as f32,
            (f64::from(s.height) / self.scale) as f32,
        )
    }

    fn pointer(&self, kind: PointerKind, position: Point, time: Duration) -> InputEvent {
        InputEvent::Pointer(PointerEvent {
            kind,
            position,
            modifiers: self.modifiers,
            time,
        })
    }

    /// `WindowEvent::ModifiersChanged`.
    pub fn modifiers_changed(&mut self, state: ModifiersState) {
        self.modifiers = modifiers(state);
    }

    /// `WindowEvent::CursorMoved`.
    pub fn cursor_moved(
        &mut self,
        position: PhysicalPosition<f64>,
        time: Duration,
        out: &mut Vec<InputEvent>,
    ) {
        let p = self.to_logical(position);
        self.position = Some(p);
        out.push(self.pointer(PointerKind::Move, p, time));
    }

    /// `WindowEvent::CursorLeft`. While a button is held the pointer is
    /// captured by the window, so leaving is reported only when none is.
    pub fn cursor_left(&mut self, out: &mut Vec<InputEvent>) {
        if self.buttons.is_empty() {
            self.position = None;
            out.push(InputEvent::PointerLeft);
        }
    }

    /// `WindowEvent::MouseInput`. Ignored before the first cursor position
    /// is known.
    pub fn mouse_input(
        &mut self,
        state: ElementState,
        button: winit::event::MouseButton,
        time: Duration,
        out: &mut Vec<InputEvent>,
    ) {
        let Some(p) = self.position else {
            return;
        };
        let b = mouse_button(button);
        match state {
            ElementState::Pressed => {
                if !self.buttons.contains(&b) {
                    self.buttons.push(b);
                }
                out.push(self.pointer(PointerKind::Down(b), p, time));
            }
            ElementState::Released => {
                // A release without a press (pressed outside the window)
                // is dropped.
                if let Some(i) = self.buttons.iter().position(|x| *x == b) {
                    self.buttons.swap_remove(i);
                    out.push(self.pointer(PointerKind::Up(b), p, time));
                }
            }
        }
    }

    /// Release every held button (`Up` events at the last position): after
    /// `Window::drag_window` the window manager owns the pointer and the
    /// release usually never reaches the window. A later release of the
    /// same button is dropped.
    pub fn release_buttons(&mut self, time: Duration, out: &mut Vec<InputEvent>) {
        let held = std::mem::take(&mut self.buttons);
        if let Some(p) = self.position {
            for b in held {
                out.push(self.pointer(PointerKind::Up(b), p, time));
            }
        }
    }

    /// `WindowEvent::MouseWheel`. Line deltas (mouse wheels) stay lines;
    /// pixel deltas (trackpads, precise wheels) become logical pixels. Signs
    /// follow winit: positive moves the content right and down.
    #[allow(clippy::cast_possible_truncation)]
    pub fn mouse_wheel(
        &mut self,
        delta: MouseScrollDelta,
        time: Duration,
        out: &mut Vec<InputEvent>,
    ) {
        let Some(p) = self.position else {
            return;
        };
        let d = match delta {
            MouseScrollDelta::LineDelta(x, y) => ScrollDelta::Lines(Vec2::new(x, y)),
            MouseScrollDelta::PixelDelta(v) => ScrollDelta::Pixels(Vec2::new(
                (v.x / self.scale) as f32,
                (v.y / self.scale) as f32,
            )),
        };
        out.push(self.pointer(PointerKind::Scroll(d), p, time));
    }

    /// `WindowEvent::KeyboardInput` (synthetic events excluded by the
    /// caller). Emits a key event, then on press the produced text (unless
    /// Ctrl or Cmd is held, or it is a control character). While an IME
    /// composition is in progress keys belong to the IME and are dropped.
    pub fn keyboard(&mut self, input: &KeyInput, out: &mut Vec<InputEvent>) {
        let pressed = input.state == ElementState::Pressed;
        if self.composing && pressed {
            return;
        }
        if let Some(key) = key(&input.logical, input.physical, self.modifiers) {
            out.push(InputEvent::Key(KeyEvent {
                key,
                modifiers: self.modifiers,
                pressed,
                repeat: input.repeat,
            }));
        }
        if pressed
            && !self.modifiers.ctrl
            && !self.modifiers.cmd
            && let Some(text) = &input.text
            && !text.is_empty()
            && !text.chars().any(char::is_control)
        {
            out.push(InputEvent::Text(text.to_string()));
        }
    }

    /// `WindowEvent::Ime`. Maps 1:1 to [`ImeEvent`]; preedit cursor
    /// ranges are byte offsets in both. An empty preedit ends the
    /// composition (it is still delivered, so widgets clear their preedit).
    pub fn ime(&mut self, ime: Ime, out: &mut Vec<InputEvent>) {
        let ev = match ime {
            Ime::Enabled => ImeEvent::Enabled,
            Ime::Preedit(text, cursor) => {
                self.composing = !text.is_empty();
                ImeEvent::Preedit { text, cursor }
            }
            Ime::Commit(text) => {
                self.composing = false;
                ImeEvent::Commit(text)
            }
            Ime::Disabled => {
                self.composing = false;
                ImeEvent::Disabled
            }
        };
        out.push(InputEvent::Ime(ev));
    }

    /// `WindowEvent::Focused`. On focus loss held buttons are released (so
    /// drags end), modifiers are cleared (their releases go to another
    /// window) and any composition is dropped.
    pub fn focused(&mut self, focused: bool, time: Duration, out: &mut Vec<InputEvent>) {
        if focused {
            return;
        }
        if let Some(p) = self.position {
            for b in std::mem::take(&mut self.buttons) {
                out.push(self.pointer(PointerKind::Up(b), p, time));
            }
        }
        self.buttons.clear();
        self.modifiers = Modifiers::NONE;
        if self.composing {
            self.composing = false;
            out.push(InputEvent::Ime(ImeEvent::Preedit {
                text: String::new(),
                cursor: None,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NamedKey as WNamed;

    const T: Duration = Duration::from_millis(5);

    fn ki(logical: keyboard::Key, code: KeyCode, text: Option<&str>) -> KeyInput {
        KeyInput {
            logical,
            physical: PhysicalKey::Code(code),
            state: ElementState::Pressed,
            repeat: false,
            text: text.map(SmolStr::new),
        }
    }

    fn ch(s: &str) -> keyboard::Key {
        keyboard::Key::Character(SmolStr::new(s))
    }

    fn translator() -> (InputTranslator, Vec<InputEvent>) {
        (InputTranslator::new(2.0), Vec::new())
    }

    fn ptr(e: &InputEvent) -> &PointerEvent {
        match e {
            InputEvent::Pointer(p) => p,
            other => panic!("not a pointer event: {other:?}"),
        }
    }

    #[test]
    fn pointer_positions_are_logical() {
        let (mut t, mut out) = translator();
        t.cursor_moved(PhysicalPosition::new(200.0, 100.0), T, &mut out);
        let p = ptr(&out[0]);
        assert_eq!(p.kind, PointerKind::Move);
        assert_eq!(p.position, Point::new(100.0, 50.0));
        assert_eq!(p.time, T);
        t.set_scale(1.0);
        t.cursor_moved(PhysicalPosition::new(200.0, 100.0), T, &mut out);
        assert_eq!(ptr(&out[1]).position, Point::new(200.0, 100.0));
    }

    #[test]
    fn buttons_and_modifiers() {
        let (mut t, mut out) = translator();
        // No position yet: dropped.
        t.mouse_input(
            ElementState::Pressed,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        assert!(out.is_empty());
        t.cursor_moved(PhysicalPosition::new(20.0, 20.0), T, &mut out);
        t.modifiers_changed(ModifiersState::SHIFT | ModifiersState::SUPER);
        t.mouse_input(
            ElementState::Pressed,
            winit::event::MouseButton::Right,
            T,
            &mut out,
        );
        t.mouse_input(
            ElementState::Released,
            winit::event::MouseButton::Right,
            T,
            &mut out,
        );
        // Release without press: dropped.
        t.mouse_input(
            ElementState::Released,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        assert_eq!(out.len(), 3);
        let down = ptr(&out[1]);
        assert_eq!(down.kind, PointerKind::Down(MouseButton::Right));
        assert_eq!(down.position, Point::new(10.0, 10.0));
        assert!(down.modifiers.shift && down.modifiers.cmd && !down.modifiers.ctrl);
        assert_eq!(ptr(&out[2]).kind, PointerKind::Up(MouseButton::Right));
        assert_eq!(
            mouse_button(winit::event::MouseButton::Back),
            MouseButton::Other(3)
        );
    }

    #[test]
    fn scroll_lines_and_pixels() {
        let (mut t, mut out) = translator();
        t.cursor_moved(PhysicalPosition::new(0.0, 0.0), T, &mut out);
        t.mouse_wheel(MouseScrollDelta::LineDelta(0.0, -3.0), T, &mut out);
        t.mouse_wheel(
            MouseScrollDelta::PixelDelta(PhysicalPosition::new(4.0, 10.0)),
            T,
            &mut out,
        );
        assert_eq!(
            ptr(&out[1]).kind,
            PointerKind::Scroll(ScrollDelta::Lines(Vec2::new(0.0, -3.0)))
        );
        assert_eq!(
            ptr(&out[2]).kind,
            PointerKind::Scroll(ScrollDelta::Pixels(Vec2::new(2.0, 5.0)))
        );
    }

    #[test]
    fn leaving_is_deferred_while_captured() {
        let (mut t, mut out) = translator();
        t.cursor_moved(PhysicalPosition::new(0.0, 0.0), T, &mut out);
        t.mouse_input(
            ElementState::Pressed,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        t.cursor_left(&mut out);
        assert_eq!(out.len(), 2);
        t.mouse_input(
            ElementState::Released,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        t.cursor_left(&mut out);
        assert_eq!(out.last(), Some(&InputEvent::PointerLeft));
        assert_eq!(t.pointer_position(), None);
    }

    #[test]
    fn keys_and_text() {
        let (mut t, mut out) = translator();
        t.keyboard(&ki(ch("a"), KeyCode::KeyA, Some("a")), &mut out);
        assert_eq!(
            out,
            vec![
                InputEvent::Key(KeyEvent {
                    key: Key::Char('a'),
                    modifiers: Modifiers::NONE,
                    pressed: true,
                    repeat: false,
                }),
                InputEvent::Text("a".into()),
            ]
        );
        out.clear();
        // Shift: uppercase text, lowercase key.
        t.modifiers_changed(ModifiersState::SHIFT);
        t.keyboard(&ki(ch("A"), KeyCode::KeyA, Some("A")), &mut out);
        assert!(
            matches!(out[0], InputEvent::Key(KeyEvent { key: Key::Char('a'), modifiers, .. }) if modifiers.shift)
        );
        assert_eq!(out[1], InputEvent::Text("A".into()));
        out.clear();
        // Named keys; Enter's "\r" text is a control char and dropped.
        t.modifiers_changed(ModifiersState::empty());
        t.keyboard(
            &ki(
                keyboard::Key::Named(WNamed::Enter),
                KeyCode::Enter,
                Some("\r"),
            ),
            &mut out,
        );
        t.keyboard(
            &ki(
                keyboard::Key::Named(WNamed::ArrowLeft),
                KeyCode::ArrowLeft,
                None,
            ),
            &mut out,
        );
        t.keyboard(
            &ki(keyboard::Key::Named(WNamed::F5), KeyCode::F5, None),
            &mut out,
        );
        t.keyboard(
            &ki(
                keyboard::Key::Named(WNamed::Space),
                KeyCode::Space,
                Some(" "),
            ),
            &mut out,
        );
        let keys: Vec<_> = out
            .iter()
            .map(|e| match e {
                InputEvent::Key(k) => format!("{}", k.key),
                InputEvent::Text(s) => format!("text:{s}"),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(keys, ["enter", "left", "f5", "space", "text: "]);
    }

    #[test]
    fn repeat_and_release() {
        let (mut t, mut out) = translator();
        let mut k = ki(ch("x"), KeyCode::KeyX, Some("x"));
        k.repeat = true;
        t.keyboard(&k, &mut out);
        k.state = ElementState::Released;
        k.repeat = false;
        k.text = None;
        t.keyboard(&k, &mut out);
        assert!(matches!(
            out[0],
            InputEvent::Key(KeyEvent {
                repeat: true,
                pressed: true,
                ..
            })
        ));
        assert_eq!(out[1], InputEvent::Text("x".into()));
        assert!(matches!(
            out[2],
            InputEvent::Key(KeyEvent { pressed: false, .. })
        ));
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn shortcuts_use_physical_keys_and_suppress_text() {
        let (mut t, mut out) = translator();
        // Cmd-C: key event, no text.
        t.modifiers_changed(ModifiersState::SUPER);
        t.keyboard(&ki(ch("c"), KeyCode::KeyC, Some("c")), &mut out);
        assert_eq!(out.len(), 1);
        // Option-A on a Mac produces "å": the key is `a`, the text stays.
        t.modifiers_changed(ModifiersState::ALT);
        t.keyboard(&ki(ch("å"), KeyCode::KeyA, Some("å")), &mut out);
        assert!(matches!(
            out[1],
            InputEvent::Key(KeyEvent {
                key: Key::Char('a'),
                ..
            })
        ));
        assert_eq!(out[2], InputEvent::Text("å".into()));
        // Ctrl on a Cyrillic layout.
        t.modifiers_changed(ModifiersState::CONTROL);
        t.keyboard(&ki(ch("ф"), KeyCode::KeyA, Some("ф")), &mut out);
        assert!(matches!(
            out[3],
            InputEvent::Key(KeyEvent {
                key: Key::Char('a'),
                ..
            })
        ));
        assert_eq!(out.len(), 4);
        // Without modifiers the logical character is kept.
        t.modifiers_changed(ModifiersState::empty());
        assert_eq!(
            key(&ch("ф"), PhysicalKey::Code(KeyCode::KeyA), Modifiers::NONE),
            Some(Key::Char('ф'))
        );
    }

    #[test]
    fn ime_sequence_and_composition_blocks_keys() {
        let (mut t, mut out) = translator();
        t.ime(Ime::Enabled, &mut out);
        t.ime(Ime::Preedit("にほ".into(), Some((6, 6))), &mut out);
        assert!(t.is_composing());
        // Keys during composition belong to the IME.
        t.keyboard(&ki(ch("n"), KeyCode::KeyN, Some("n")), &mut out);
        t.ime(Ime::Preedit(String::new(), None), &mut out);
        t.ime(Ime::Commit("日本".into()), &mut out);
        assert!(!t.is_composing());
        t.ime(Ime::Disabled, &mut out);
        assert_eq!(
            out,
            vec![
                InputEvent::Ime(ImeEvent::Enabled),
                InputEvent::Ime(ImeEvent::Preedit {
                    text: "にほ".into(),
                    cursor: Some((6, 6))
                }),
                InputEvent::Ime(ImeEvent::Preedit {
                    text: String::new(),
                    cursor: None
                }),
                InputEvent::Ime(ImeEvent::Commit("日本".into())),
                InputEvent::Ime(ImeEvent::Disabled),
            ]
        );
    }

    #[test]
    fn focus_loss_releases_buttons_and_composition() {
        let (mut t, mut out) = translator();
        t.cursor_moved(PhysicalPosition::new(10.0, 10.0), T, &mut out);
        t.modifiers_changed(ModifiersState::SHIFT);
        t.mouse_input(
            ElementState::Pressed,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        t.ime(Ime::Preedit("a".into(), None), &mut out);
        out.clear();
        t.focused(false, T, &mut out);
        assert_eq!(ptr(&out[0]).kind, PointerKind::Up(MouseButton::Left));
        assert_eq!(
            out[1],
            InputEvent::Ime(ImeEvent::Preedit {
                text: String::new(),
                cursor: None
            })
        );
        assert_eq!(t.modifiers(), Modifiers::NONE);
        assert!(!t.is_composing());
        out.clear();
        t.focused(true, T, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn window_drag_releases_buttons_once() {
        let (mut t, mut out) = translator();
        t.cursor_moved(PhysicalPosition::new(10.0, 10.0), T, &mut out);
        t.mouse_input(
            ElementState::Pressed,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        out.clear();
        t.release_buttons(T, &mut out);
        assert_eq!(out.len(), 1);
        assert_eq!(ptr(&out[0]).kind, PointerKind::Up(MouseButton::Left));
        assert!(!t.buttons_held());
        // The window manager's late release is dropped.
        out.clear();
        t.mouse_input(
            ElementState::Released,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn translated_events_drive_the_core() {
        use livspeil_ui_core::{App, Element, Render, TestHost, ViewContext, div};
        use std::cell::Cell;
        use std::rc::Rc;

        struct V(Rc<Cell<u32>>);
        impl Render for V {
            fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
                let n = self.0.clone();
                div()
                    .key("b")
                    .size(100.0, 100.0)
                    .on_click(move |_, _| n.set(n.get() + 1))
            }
        }
        let clicks = Rc::new(Cell::new(0));
        let mut app = App::new();
        let v = app.new_view(V(clicks.clone()));
        app.set_root(v);
        let mut host = TestHost::new(app, livspeil_ui_layout::Size::new(200.0, 200.0));
        host.run_until_idle(10);
        let (mut t, mut out) = translator();
        t.cursor_moved(PhysicalPosition::new(100.0, 100.0), T, &mut out);
        t.mouse_input(
            ElementState::Pressed,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        t.mouse_input(
            ElementState::Released,
            winit::event::MouseButton::Left,
            T,
            &mut out,
        );
        for e in out.drain(..) {
            host.app.dispatch(e);
        }
        assert_eq!(clicks.get(), 1);
    }
}
