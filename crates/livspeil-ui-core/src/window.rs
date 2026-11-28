// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Window requests from the UI to its host: start a window move (a custom
//! titlebar's drag region), zoom (double-click on it), minimize.
//!
//! Like the other platform requests (`platform.rs`) these are side
//! channels: they never make the app need a frame. A host reads them with
//! [`App::take_window_requests`] after dispatching input and applies them
//! (winit: `drag_window`, `set_maximized`, `set_minimized`).
//!
//! The title is state rather than a request: [`App::set_window_title`]
//! replaces one the host has not yet taken.
//!
//! [`Element::window_drag_region`] marks an element as a drag region: a
//! primary press on it (or on a descendant that does not handle pointer
//! input itself) asks for [`WindowRequest::Drag`], a double press for
//! [`WindowRequest::Zoom`].

use std::time::Duration;

use livspeil_ui_layout::Point;

use crate::app::{App, EventCx};
use crate::element::{Element, ElementId, PointerFilter};
use crate::input::{MouseButton, PointerKind};

/// Two presses within this time (and [`DOUBLE_PRESS_SLOP`]) are a double press.
pub const DOUBLE_PRESS_TIME: Duration = Duration::from_millis(400);
/// Largest distance (px) between the two presses of a double press.
pub const DOUBLE_PRESS_SLOP: f32 = 4.0;

/// A request to the host window.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowRequest {
    /// Start moving the window with the pressed pointer (a drag region).
    Drag,
    /// Toggle the zoomed (maximized) state, as a double-click on the
    /// titlebar does.
    Zoom,
    /// Minimize the window.
    Minimize,
    /// Close the window now. The app asks for this once it is ready to, so
    /// a host does not ask it again.
    Close,
}

#[derive(Debug, Default)]
struct WindowState {
    requests: Vec<WindowRequest>,
    title: Option<String>,
}

/// State slot for the window requests (see `platform.rs`: id 0 never names
/// an element). A different type than the platform state, so a separate
/// entry in the same slot.
const SLOT: ElementId = ElementId(0);

impl App {
    /// Ask the host for a window action.
    pub fn request_window(&mut self, request: WindowRequest) {
        let s = self.element_state_mut::<WindowState>(SLOT);
        if !s.requests.contains(&request) {
            s.requests.push(request);
        }
    }

    /// Retitle the window.
    pub fn set_window_title(&mut self, title: impl Into<String>) {
        self.element_state_mut::<WindowState>(SLOT).title = Some(title.into());
    }

    /// The title set since the last call, clearing it (hosts).
    pub fn take_window_title(&mut self) -> Option<String> {
        self.element_state_mut::<WindowState>(SLOT).title.take()
    }

    /// The pending window requests in order, clearing them (hosts).
    pub fn take_window_requests(&mut self) -> Vec<WindowRequest> {
        std::mem::take(&mut self.element_state_mut::<WindowState>(SLOT).requests)
    }
}

/// Per-element state of a drag region: the last press.
#[derive(Debug, Default)]
struct LastPress {
    at: Option<(Duration, Point)>,
}

/// Whether a press on `target` inside the region `region` belongs to the
/// region: no element from the target up to (excluding) the region takes
/// pointer input itself (click, drag or pointer-down handlers, or focus).
fn press_is_for_region(cx: &EventCx<'_>, target: ElementId, region: ElementId) -> bool {
    let mut cur = Some(target);
    while let Some(id) = cur {
        if id == region {
            return true;
        }
        let Some(node) = cx.tree.nodes.get(&id) else {
            return false;
        };
        if node.tab_index.is_some() {
            return false;
        }
        if let Some(h) = &node.handlers
            && (h.click.is_some()
                || h.drag.is_some()
                || h.pointer.iter().any(|(f, _, _)| *f == PointerFilter::Down))
        {
            return false;
        }
        cur = node.parent;
    }
    false
}

impl Element {
    /// Make this element a window drag region (custom titlebars): a
    /// primary press on it, or on a descendant that does not handle the
    /// pointer itself, requests [`WindowRequest::Drag`]; a double press
    /// requests [`WindowRequest::Zoom`].
    #[must_use]
    pub fn window_drag_region(self) -> Self {
        self.on_pointer_down(|ev, cx| {
            if ev.kind != PointerKind::Down(MouseButton::Left) {
                return;
            }
            let (target, region) = (cx.target(), cx.element_id());
            if !press_is_for_region(cx, target, region) {
                return;
            }
            let last = cx.state::<LastPress>().at.replace((ev.time, ev.position));
            let double = last.is_some_and(|(t, p)| {
                ev.time.saturating_sub(t) <= DOUBLE_PRESS_TIME
                    && (ev.position - p).length() <= DOUBLE_PRESS_SLOP
            });
            if double {
                cx.state::<LastPress>().at = None;
                cx.request_window(WindowRequest::Zoom);
            } else {
                cx.request_window(WindowRequest::Drag);
            }
        })
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
                .key("bar")
                .size(300.0, 28.0)
                .window_drag_region()
                .child(div().key("label").size(50.0, 28.0))
                .child(div().key("button").size(50.0, 28.0).on_click(|_, _| {}))
        }
    }

    fn host() -> TestHost {
        let mut app = App::new();
        let v = app.new_view(V);
        app.set_root(v);
        let mut host = TestHost::new(app, Size::new(300.0, 100.0));
        host.run_until_idle(10);
        host
    }

    #[test]
    fn press_drags_double_press_zooms() {
        let mut h = host();
        assert!(h.app.take_window_requests().is_empty());
        h.click(200.0, 10.0);
        assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Drag]);
        h.advance(Duration::from_millis(100));
        h.click(201.0, 10.0);
        assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Zoom]);
        // Long after: a plain drag again.
        h.advance(Duration::from_millis(600));
        h.click(201.0, 10.0);
        assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Drag]);
        assert!(!h.app.needs_frame());
    }

    #[test]
    fn passive_children_drag_controls_do_not() {
        let mut h = host();
        h.click(20.0, 10.0); // label: passive
        assert_eq!(h.app.take_window_requests(), vec![WindowRequest::Drag]);
        h.advance(Duration::from_millis(600));
        h.click(70.0, 10.0); // button: handles clicks
        assert!(h.app.take_window_requests().is_empty());
    }
}
