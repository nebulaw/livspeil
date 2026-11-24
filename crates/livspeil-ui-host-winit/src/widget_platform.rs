// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Platform needs of widgets, derived from the retained tree without a
//! window (tested headlessly):
//!
//! - [`text_focus`]: the focused text input and its caret, for the IME
//!   candidate window and caret blinking. Widgets may report their caret
//!   with `App::set_text_input`; otherwise a focused element with the
//!   `TextInput` role (or a `SpinButton` being edited) and its descendant
//!   keyed `caret` are used.
//! - [`role_cursor`]: the cursor shape for the element under the pointer
//!   from its role (pointing hand for buttons and other clickable controls,
//!   I-beam for text, east-west resize for sliders and scrub fields) when
//!   no handler asked for one.
//! - [`CaretBlink`]: the caret blink timer. It runs only while a caret is
//!   shown, restarts (caret visible) on input and caret moves, and stops
//!   with the caret visible after [`CaretBlink::IDLE`] without input, so an
//!   idle window wakes up for nothing.
//! - [`apply_config_event`]: hot reload that keeps the app's own key
//!   bindings (widget keymap, app shortcuts) under `keymap.toml`.

use std::time::{Duration, Instant};

use livspeil_ui_core::{App, CursorShape, ElementId, Keymap, PaintCmd, Role, Scene};
use livspeil_ui_layout::{Point, Rect};
use livspeil_ui_theme::{ConfigError, ConfigEvent};

/// Key of the caret element drawn by text widgets (selections are keyed
/// `selection-<n>`).
pub const CARET_KEY: &str = "caret";

/// The focused text input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextFocus {
    /// The element that accepts text.
    pub field: ElementId,
    /// The caret element, while a caret (not a selection) is drawn.
    pub caret: Option<ElementId>,
    /// Where the IME candidate window goes (window logical pixels): the
    /// caret, else the field.
    pub area: Rect,
}

fn find_descendant(app: &App, root: ElementId, label: &str) -> Option<ElementId> {
    find_where(app, root, |l| l == label)
}

fn find_where(app: &App, root: ElementId, f: impl Fn(&str) -> bool) -> Option<ElementId> {
    let mut stack = app.children_of(root);
    while let Some(id) = stack.pop() {
        if app.label(id).is_some_and(|l| f(&l)) {
            return Some(id);
        }
        stack.extend(app.children_of(id));
    }
    None
}

/// The focused text input, if any (see the module docs).
#[must_use]
pub fn text_focus(app: &App) -> Option<TextFocus> {
    if let Some(t) = app.text_input() {
        let caret = find_descendant(app, t.element, CARET_KEY);
        return Some(TextFocus {
            field: t.element,
            caret,
            area: t.caret,
        });
    }
    let field = app.focused()?;
    let node = app.semantic_node(field)?;
    if node.is_disabled() {
        return None;
    }
    let caret = find_descendant(app, field, CARET_KEY);
    match node.role() {
        Some(Role::TextInput) => {}
        // A number field is a text input only while it is being edited
        // (it draws a caret or a selection).
        Some(Role::SpinButton)
            if caret.is_some()
                || find_where(app, field, |l| l.starts_with("selection-")).is_some() => {}
        _ => return None,
    }
    let area = caret.and_then(|c| app.bounds(c)).unwrap_or(node.bounds);
    Some(TextFocus { field, caret, area })
}

/// The cursor shape a role asks for, if any.
#[must_use]
pub fn cursor_for_role(role: Role) -> Option<CursorShape> {
    Some(match role {
        Role::Button
        | Role::Link
        | Role::CheckBox
        | Role::Switch
        | Role::RadioButton
        | Role::ComboBox
        | Role::ListBoxOption
        | Role::MenuItem
        | Role::Tab => CursorShape::Pointer,
        Role::TextInput => CursorShape::Text,
        Role::Slider | Role::SpinButton => CursorShape::ResizeEw,
        _ => return None,
    })
}

/// The cursor for the element under `point`, from the role of the nearest
/// ancestor (or itself) that has one: [`cursor_for_role`]. Disabled
/// controls show the arrow; a number field being edited shows the I-beam.
#[must_use]
pub fn role_cursor(app: &App, point: Point) -> CursorShape {
    let mut cur = app.hit_test(point);
    while let Some(id) = cur {
        let Some(node) = app.semantic_node(id) else {
            break;
        };
        if let Some(shape) = node.role().and_then(cursor_for_role) {
            if node.is_disabled() {
                return CursorShape::Default;
            }
            if node.role() == Some(Role::SpinButton)
                && text_focus(app).is_some_and(|t| t.field == id)
            {
                return CursorShape::Text;
            }
            return shape;
        }
        cur = node.parent;
    }
    CursorShape::Default
}

/// The caret blink state machine (no window; see the module docs).
#[derive(Clone, Debug)]
pub struct CaretBlink {
    interval: Duration,
    idle: Duration,
    caret: Option<(ElementId, Rect)>,
    visible: bool,
    next: Option<Instant>,
    last_activity: Instant,
}

impl CaretBlink {
    /// Half a blink period (macOS uses about 530 ms).
    pub const INTERVAL: Duration = Duration::from_millis(530);
    /// Blinking stops (caret shown) after this long without input.
    pub const IDLE: Duration = Duration::from_secs(15);

    /// A stopped blinker.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self::with_timing(now, Self::INTERVAL, Self::IDLE)
    }

    /// A stopped blinker with custom timing.
    #[must_use]
    pub fn with_timing(now: Instant, interval: Duration, idle: Duration) -> Self {
        CaretBlink {
            interval,
            idle,
            caret: None,
            visible: true,
            next: None,
            last_activity: now,
        }
    }

    /// Input arrived: show the caret and restart the period.
    pub fn activity(&mut self, now: Instant) {
        self.last_activity = now;
        if self.caret.is_some() {
            self.visible = true;
            self.next = Some(now + self.interval);
        }
    }

    /// The caret after a frame or event batch (`None`: no caret shown).
    /// A new or moved caret is shown and the period restarts. Returns
    /// whether the caret's visibility changed (redraw).
    pub fn set_caret(&mut self, now: Instant, caret: Option<(ElementId, Rect)>) -> bool {
        if caret == self.caret {
            return false;
        }
        let was_hidden = !self.visible;
        self.caret = caret;
        self.visible = true;
        self.last_activity = now;
        self.next = caret.map(|_| now + self.interval);
        was_hidden
    }

    /// Advance to `now`: toggles when due. Returns whether the caret's
    /// visibility changed (redraw).
    pub fn poll(&mut self, now: Instant) -> bool {
        let Some(due) = self.next else {
            return false;
        };
        if now < due {
            return false;
        }
        if now.saturating_duration_since(self.last_activity) >= self.idle {
            // Idle: stop with the caret shown.
            self.next = None;
            let changed = !self.visible;
            self.visible = true;
            return changed;
        }
        self.visible = !self.visible;
        self.next = Some(now + self.interval);
        true
    }

    /// When the next toggle is due (`None`: not blinking).
    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.next
    }

    /// Whether the timer runs.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.next.is_some()
    }

    /// Whether the caret is shown.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// The caret element to leave out of the frame, while hidden.
    #[must_use]
    pub fn hidden_element(&self) -> Option<ElementId> {
        if self.visible {
            None
        } else {
            self.caret.map(|(id, _)| id)
        }
    }
}

/// `scene` without the paint commands of `element` (quads and text runs).
#[must_use]
pub fn scene_without(scene: &Scene, element: ElementId) -> Scene {
    Scene {
        commands: scene
            .commands
            .iter()
            .filter(|c| match c {
                PaintCmd::Quad(q) => q.element != element,
                PaintCmd::Text(t) => t.element != element,
                _ => true,
            })
            .cloned()
            .collect(),
    }
}

/// Apply a hot-reload event to `app`. A `keymap` file extends `base` (the
/// app's keymap when the host started: defaults, widget keymap and app
/// shortcuts) instead of the bare defaults; removing the file restores
/// `base`. Other events go to `App::apply_config_event`.
///
/// # Errors
/// The event's error or a keymap parse error (nothing changes).
pub fn apply_config_event(
    app: &mut App,
    base: &Keymap,
    event: ConfigEvent,
) -> Result<(), ConfigError> {
    match event {
        ConfigEvent::Raw {
            tag: "keymap",
            contents,
        } => {
            let mut keymap = base.clone();
            if let Some(text) = contents {
                keymap.extend(&Keymap::parse(&text)?);
            }
            app.set_keymap(keymap);
            Ok(())
        }
        other => app.apply_config_event(other),
    }
}

/// The host clipboard as the widgets' `Env` clipboard (feature `widgets`):
/// copy and paste in text fields reach the system clipboard.
#[cfg(feature = "widgets")]
pub struct WidgetClipboard {
    inner: std::cell::RefCell<Box<dyn crate::Clipboard>>,
}

#[cfg(feature = "widgets")]
impl WidgetClipboard {
    /// Wrap a host clipboard.
    #[must_use]
    pub fn new(clipboard: Box<dyn crate::Clipboard>) -> Self {
        WidgetClipboard {
            inner: std::cell::RefCell::new(clipboard),
        }
    }

    /// The system clipboard (in-process fallback without a display).
    #[must_use]
    pub fn system() -> Self {
        Self::new(Box::new(crate::SystemClipboard::default()))
    }

    /// Install the system clipboard into `env`.
    pub fn install_system(env: &livspeil_ui_widgets::Env) {
        env.set_clipboard(Box::new(Self::system()));
    }
}

#[cfg(feature = "widgets")]
impl std::fmt::Debug for WidgetClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WidgetClipboard").finish_non_exhaustive()
    }
}

#[cfg(feature = "widgets")]
impl livspeil_ui_widgets::Clipboard for WidgetClipboard {
    fn read_text(&self) -> Option<String> {
        self.inner.try_borrow_mut().ok()?.get_text()
    }

    fn write_text(&mut self, text: &str) {
        self.inner.get_mut().set_text(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32) -> Rect {
        Rect::new(x, 0.0, 1.0, 16.0)
    }

    #[test]
    fn blink_runs_only_with_a_caret() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut b = CaretBlink::with_timing(t0, ms(500), ms(5000));
        assert!(!b.is_running());
        assert!(!b.poll(t0 + ms(600)));
        assert_eq!(b.deadline(), None);
        let caret = Some((ElementId(7), rect(0.0)));
        assert!(!b.set_caret(t0, caret));
        assert_eq!(b.deadline(), Some(t0 + ms(500)));
        assert!(!b.poll(t0 + ms(499)));
        assert!(b.poll(t0 + ms(500)));
        assert!(!b.is_visible());
        assert_eq!(b.hidden_element(), Some(ElementId(7)));
        assert!(b.poll(t0 + ms(1000)));
        assert!(b.is_visible());
        // Typing shows the caret and restarts the period.
        b.poll(t0 + ms(1500));
        assert!(!b.is_visible());
        b.activity(t0 + ms(1600));
        assert!(b.is_visible());
        assert_eq!(b.deadline(), Some(t0 + ms(2100)));
        // Unfocused: stops with the caret shown.
        b.poll(t0 + ms(2100));
        assert!(b.set_caret(t0 + ms(2200), None));
        assert!(!b.is_running());
        assert!(b.is_visible());
    }

    #[test]
    fn blink_stops_when_idle() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut b = CaretBlink::with_timing(t0, ms(500), ms(2000));
        b.set_caret(t0, Some((ElementId(1), rect(0.0))));
        let mut t = t0;
        let mut toggles = 0;
        while b.is_running() {
            t = b.deadline().unwrap();
            if b.poll(t) {
                toggles += 1;
            }
        }
        assert!(t >= t0 + ms(2000) && t < t0 + ms(2600), "{:?}", t - t0);
        assert!(b.is_visible());
        assert!(toggles >= 3);
        // A moved caret restarts it.
        b.set_caret(t, Some((ElementId(1), rect(5.0))));
        assert!(b.is_running());
    }

    #[test]
    fn roles_map_to_cursors() {
        assert_eq!(cursor_for_role(Role::Button), Some(CursorShape::Pointer));
        assert_eq!(cursor_for_role(Role::Switch), Some(CursorShape::Pointer));
        assert_eq!(cursor_for_role(Role::TextInput), Some(CursorShape::Text));
        assert_eq!(cursor_for_role(Role::Slider), Some(CursorShape::ResizeEw));
        assert_eq!(
            cursor_for_role(Role::SpinButton),
            Some(CursorShape::ResizeEw)
        );
        assert_eq!(cursor_for_role(Role::Label), None);
        assert_eq!(cursor_for_role(Role::Window), None);
    }
}
