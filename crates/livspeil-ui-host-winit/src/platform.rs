// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Cursor shapes, IME placement and the clipboard.

use livspeil_ui_core::CursorShape;
use livspeil_ui_layout::Rect;
use winit::window::CursorIcon;

/// The winit cursor for a core cursor shape; `None` hides the cursor.
#[must_use]
pub fn cursor_icon(shape: CursorShape) -> Option<CursorIcon> {
    Some(match shape {
        CursorShape::Default => CursorIcon::Default,
        CursorShape::Pointer => CursorIcon::Pointer,
        CursorShape::Text => CursorIcon::Text,
        CursorShape::Grab => CursorIcon::Grab,
        CursorShape::Grabbing => CursorIcon::Grabbing,
        CursorShape::Move => CursorIcon::Move,
        CursorShape::NotAllowed => CursorIcon::NotAllowed,
        CursorShape::Crosshair => CursorIcon::Crosshair,
        CursorShape::Wait => CursorIcon::Wait,
        CursorShape::Progress => CursorIcon::Progress,
        CursorShape::ResizeEw => CursorIcon::EwResize,
        CursorShape::ResizeNs => CursorIcon::NsResize,
        CursorShape::ResizeNesw => CursorIcon::NeswResize,
        CursorShape::ResizeNwse => CursorIcon::NwseResize,
        CursorShape::ResizeColumn => CursorIcon::ColResize,
        CursorShape::ResizeRow => CursorIcon::RowResize,
        CursorShape::Help => CursorIcon::Help,
        CursorShape::ZoomIn => CursorIcon::ZoomIn,
        CursorShape::ZoomOut => CursorIcon::ZoomOut,
        CursorShape::None => return None,
    })
}

/// What the window's IME should look like: allowed or not, and where the
/// candidate window goes (logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImeState {
    /// IME enabled (a text input is focused).
    pub allowed: bool,
    /// The caret rectangle, when allowed.
    pub area: Option<Rect>,
}

impl ImeState {
    /// Disabled.
    pub const OFF: ImeState = ImeState {
        allowed: false,
        area: None,
    };

    /// From the app's focused text input.
    #[must_use]
    pub fn of(app: &livspeil_ui_core::App) -> ImeState {
        match app.text_input() {
            Some(t) => ImeState {
                allowed: true,
                area: Some(t.caret),
            },
            None => ImeState::OFF,
        }
    }
}

/// Text clipboard access. The host uses it to serve
/// [`App::set_clipboard_text`](livspeil_ui_core::App::set_clipboard_text)
/// and [`App::request_paste`](livspeil_ui_core::App::request_paste).
pub trait Clipboard {
    /// The clipboard's text, if any.
    fn get_text(&mut self) -> Option<String>;
    /// Replace the clipboard's contents with `text`.
    fn set_text(&mut self, text: &str);
}

/// An in-process clipboard (tests, or when the system one is unavailable).
#[derive(Debug, Default)]
pub struct MemoryClipboard(pub Option<String>);

impl Clipboard for MemoryClipboard {
    fn get_text(&mut self) -> Option<String> {
        self.0.clone()
    }

    fn set_text(&mut self, text: &str) {
        self.0 = Some(text.to_owned());
    }
}

/// The system clipboard through `arboard`, opened on first use; falls back
/// to an in-process clipboard when it cannot be opened (no display).
#[derive(Default)]
pub struct SystemClipboard {
    inner: Option<arboard::Clipboard>,
    failed: bool,
    fallback: MemoryClipboard,
}

impl std::fmt::Debug for SystemClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemClipboard")
            .field("open", &self.inner.is_some())
            .field("failed", &self.failed)
            .finish()
    }
}

impl SystemClipboard {
    fn open(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.inner.is_none() && !self.failed {
            match arboard::Clipboard::new() {
                Ok(c) => self.inner = Some(c),
                Err(e) => {
                    log::warn!("system clipboard unavailable: {e}");
                    self.failed = true;
                }
            }
        }
        self.inner.as_mut()
    }
}

impl Clipboard for SystemClipboard {
    fn get_text(&mut self) -> Option<String> {
        match self.open() {
            Some(c) => c.get_text().ok(),
            None => self.fallback.get_text(),
        }
    }

    fn set_text(&mut self, text: &str) {
        match self.open() {
            Some(c) => {
                if let Err(e) = c.set_text(text) {
                    log::warn!("copying to the clipboard failed: {e}");
                }
            }
            None => self.fallback.set_text(text),
        }
    }
}

/// Serve the app's clipboard requests: write queued copies, and for a
/// paste request return the text to dispatch as
/// [`InputEvent::Text`](livspeil_ui_core::InputEvent::Text).
pub fn service_clipboard(
    app: &mut livspeil_ui_core::App,
    clipboard: &mut dyn Clipboard,
) -> Option<String> {
    if let Some(text) = app.take_clipboard_write() {
        clipboard.set_text(&text);
    }
    if app.take_paste_request() {
        clipboard.get_text().filter(|t| !t.is_empty())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use livspeil_ui_core::App;

    #[test]
    fn cursor_mapping() {
        assert_eq!(cursor_icon(CursorShape::Default), Some(CursorIcon::Default));
        assert_eq!(
            cursor_icon(CursorShape::ResizeEw),
            Some(CursorIcon::EwResize)
        );
        assert_eq!(cursor_icon(CursorShape::None), None);
    }

    #[test]
    fn clipboard_round_trip() {
        let mut app = App::new();
        let mut cb = MemoryClipboard::default();
        assert_eq!(service_clipboard(&mut app, &mut cb), None);
        app.set_clipboard_text("hello");
        app.request_paste();
        assert_eq!(
            service_clipboard(&mut app, &mut cb).as_deref(),
            Some("hello")
        );
        assert_eq!(service_clipboard(&mut app, &mut cb), None);
    }

    #[test]
    fn ime_off_without_text_input() {
        assert_eq!(ImeState::of(&App::new()), ImeState::OFF);
    }
}
