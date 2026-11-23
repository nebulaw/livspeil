// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Requests from the UI to its host platform: the pointer cursor shape, the
//! focused text input (for IME enablement and candidate window placement)
//! and the clipboard.
//!
//! These are side channels, not frame state: setting them never makes the
//! app need a frame. A host (for example `livspeil-ui-host-winit`) reads
//! them after dispatching input and applies them to the window.
//!
//! - **Cursor**: during a pointer event, handlers call
//!   [`App::request_cursor`]; the first request wins, so with bubbling
//!   handlers the innermost element decides. [`Element::cursor`] installs
//!   such a handler. After a pointer move with no request the host shows the
//!   default arrow.
//! - **Text input**: a text field calls [`App::set_text_input`] (or
//!   [`EventCx::set_text_caret`]) with its caret rectangle when it gains
//!   focus or the caret moves. [`App::text_input`] reports it only while that
//!   element is focused, so stale state after focus moves away is ignored.
//! - **Clipboard**: [`App::set_clipboard_text`] queues a copy;
//!   [`App::request_paste`] asks the host to read the clipboard and deliver
//!   its text as [`InputEvent::Text`](crate::InputEvent::Text) to the focused
//!   element.

use livspeil_ui_layout::Rect;

use crate::app::{App, EventCx};
use crate::element::{Element, ElementId};

/// Pointer cursor shapes (a subset of the CSS cursor names).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// The platform's default arrow.
    #[default]
    Default,
    /// A pointing hand (links).
    Pointer,
    /// An I-beam (text).
    Text,
    /// Something can be grabbed.
    Grab,
    /// Something is being dragged.
    Grabbing,
    /// Move in any direction.
    Move,
    /// The action is not allowed.
    NotAllowed,
    /// Precise selection.
    Crosshair,
    /// Busy; the UI does not respond.
    Wait,
    /// Busy in the background; the UI still responds.
    Progress,
    /// Resize horizontally (east-west).
    ResizeEw,
    /// Resize vertically (north-south).
    ResizeNs,
    /// Resize along the north-east / south-west diagonal.
    ResizeNesw,
    /// Resize along the north-west / south-east diagonal.
    ResizeNwse,
    /// Resize a column.
    ResizeColumn,
    /// Resize a row.
    ResizeRow,
    /// Help is available.
    Help,
    /// Zoom in.
    ZoomIn,
    /// Zoom out.
    ZoomOut,
    /// Hide the cursor.
    None,
}

/// The focused text input, as reported by the widget that owns it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextInputState {
    /// The element that accepts text.
    pub element: ElementId,
    /// The caret (or the selection's end) in window logical pixels. The
    /// host places the IME candidate window next to it.
    pub caret: Rect,
}

#[derive(Debug, Default)]
struct PlatformState {
    cursor: Option<CursorShape>,
    text_input: Option<TextInputState>,
    clipboard_write: Option<String>,
    paste: bool,
}

/// Element state slot holding the app-wide platform requests. Element ids
/// are FNV hashes, so 0 never names an element in the tree (and the state is
/// therefore never pruned).
const SLOT: ElementId = ElementId(0);

impl App {
    fn platform(&self) -> Option<&PlatformState> {
        self.element_state::<PlatformState>(SLOT)
    }

    fn platform_mut(&mut self) -> &mut PlatformState {
        self.element_state_mut::<PlatformState>(SLOT)
    }

    /// Ask for a cursor shape for the current pointer event. The first
    /// request since the host last took it wins.
    pub fn request_cursor(&mut self, shape: CursorShape) {
        let p = self.platform_mut();
        if p.cursor.is_none() {
            p.cursor = Some(shape);
        }
    }

    /// The pending cursor request, clearing it (hosts).
    pub fn take_cursor_request(&mut self) -> Option<CursorShape> {
        self.platform_mut().cursor.take()
    }

    /// Report (or with `None`, withdraw) the text input that accepts text.
    pub fn set_text_input(&mut self, state: Option<TextInputState>) {
        self.platform_mut().text_input = state;
    }

    /// The text input, if its element is focused. Hosts enable the IME
    /// while this is `Some` and place the candidate window at its caret.
    #[must_use]
    pub fn text_input(&self) -> Option<TextInputState> {
        let state = self.platform()?.text_input?;
        (self.focused() == Some(state.element)).then_some(state)
    }

    /// Queue `text` to be copied to the system clipboard.
    pub fn set_clipboard_text(&mut self, text: impl Into<String>) {
        self.platform_mut().clipboard_write = Some(text.into());
    }

    /// The queued clipboard text, clearing it (hosts).
    pub fn take_clipboard_write(&mut self) -> Option<String> {
        self.platform_mut().clipboard_write.take()
    }

    /// Ask the host to paste: it reads the clipboard and dispatches its
    /// text as [`InputEvent::Text`](crate::InputEvent::Text).
    pub fn request_paste(&mut self) {
        self.platform_mut().paste = true;
    }

    /// Whether a paste was requested, clearing the request (hosts).
    pub fn take_paste_request(&mut self) -> bool {
        std::mem::take(&mut self.platform_mut().paste)
    }
}

impl EventCx<'_> {
    /// Report the handling element as the focused text input with its caret
    /// rectangle (window logical pixels).
    pub fn set_text_caret(&mut self, caret: Rect) {
        let element = self.element_id();
        self.set_text_input(Some(TextInputState { element, caret }));
    }
}

impl Element {
    /// Show `shape` while the pointer moves over this element (unless a
    /// descendant asks for another shape).
    #[must_use]
    pub fn cursor(self, shape: CursorShape) -> Self {
        self.on_pointer_move(move |_, cx| cx.request_cursor(shape))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Render, TestHost, ViewContext, div};
    use livspeil_ui_layout::Size;

    struct V;

    impl Render for V {
        fn render(&mut self, _cx: &mut ViewContext<'_, Self>) -> Element {
            div()
                .key("outer")
                .size(200.0, 200.0)
                .cursor(CursorShape::Move)
                .child(
                    div()
                        .key("inner")
                        .size(50.0, 50.0)
                        .focusable()
                        .cursor(CursorShape::Text)
                        .on_pointer_down(|_, cx| {
                            cx.set_text_caret(Rect::new(10.0, 10.0, 1.0, 16.0));
                        }),
                )
        }
    }

    fn host() -> TestHost {
        let mut app = App::new();
        let v = app.new_view(V);
        app.set_root(v);
        let mut host = TestHost::new(app, Size::new(300.0, 300.0));
        host.run_until_idle(10);
        host
    }

    #[test]
    fn innermost_cursor_wins_and_is_taken_once() {
        let mut host = host();
        host.pointer_move(10.0, 10.0);
        assert_eq!(host.app.take_cursor_request(), Some(CursorShape::Text));
        assert_eq!(host.app.take_cursor_request(), None);
        host.pointer_move(150.0, 150.0);
        assert_eq!(host.app.take_cursor_request(), Some(CursorShape::Move));
        host.pointer_move(250.0, 250.0);
        assert_eq!(host.app.take_cursor_request(), None);
    }

    #[test]
    fn text_input_follows_focus() {
        let mut host = host();
        assert_eq!(host.app.text_input(), None);
        host.click(10.0, 10.0);
        let t = host.app.text_input().unwrap();
        assert_eq!(t.caret, Rect::new(10.0, 10.0, 1.0, 16.0));
        host.app.blur();
        assert_eq!(host.app.text_input(), None);
    }

    #[test]
    fn clipboard_requests() {
        let mut app = App::new();
        assert!(!app.take_paste_request());
        app.request_paste();
        assert!(app.take_paste_request());
        assert!(!app.take_paste_request());
        app.set_clipboard_text("hi");
        assert_eq!(app.take_clipboard_write().as_deref(), Some("hi"));
        assert_eq!(app.take_clipboard_write(), None);
        assert!(!app.needs_frame());
    }
}
