// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI core: GPUI-style state and views, a retained element tree,
//! input routing and focus, the animation driver, the frame loop and the
//! renderer-agnostic paint list.
//!
//! ```
//! use livspeil_ui_core::{App, Element, Model, Render, TestHost, ViewContext, div, text};
//! use livspeil_ui_layout::Size;
//!
//! struct Counter {
//!     count: Model<i32>,
//! }
//!
//! impl Render for Counter {
//!     fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
//!         let count = self.count;
//!         let n = *count.read(cx); // observed: changes re-render this view
//!         div()
//!             .flex_col()
//!             .gap(8.0)
//!             .child(text(format!("Count: {n}")))
//!             .child(
//!                 div()
//!                     .key("inc")
//!                     .size(120.0, 32.0)
//!                     .focusable()
//!                     .child(text("Increment"))
//!                     .on_click(move |_, cx| {
//!                         count.update(cx, |c, cx| {
//!                             *c += 1;
//!                             cx.notify();
//!                         });
//!                     }),
//!             )
//!     }
//! }
//!
//! let mut app = App::new();
//! let count = app.new_model(0);
//! let view = app.new_view(Counter { count });
//! app.set_root(view);
//! let mut host = TestHost::new(app, Size::new(400.0, 300.0));
//! host.run_until_idle(100);
//! let b = host.app.bounds(host.app.find("inc").unwrap()).unwrap().center();
//! host.click(b.x, b.y);
//! host.run_until_idle(100);
//! assert_eq!(*count.read(&host.app), 1);
//! ```

mod anim;
mod app;
mod element;
mod entity;
mod host;
mod input;
mod keymap;
mod paint;
mod platform;
mod semantics;
mod text_measure;
mod tree;

pub use anim::{Animatable, Animated, Easing, TransformParts, spring_step};
pub use app::{
    AnimationStats, App, DRAG_THRESHOLD, EventCx, Frame, FrameStats, InputEvent, Subscription,
    pointer,
};
pub use element::{
    Element, ElementId, Phase, PointerFilter, Refinement, SharedString, Transition, div, text,
};
pub use entity::{AnyView, EntityId, Model, ModelContext, Render, View, ViewContext};
pub use host::{FrameScheduler, TestHost};
pub use input::{
    ClickEvent, DragEvent, DragPhase, ImeEvent, Key, KeyEvent, Keystroke, Modifiers, MouseButton,
    NamedKey, PointerEvent, PointerKind, ScrollDelta, VelocityTracker,
};
pub use keymap::{Binding, KEYMAP_FILE, Keymap, actions};
pub use paint::{
    Border, Fill, PaintCmd, PaintStats, Painter, Quad, RecordingPainter, Scene, TextRun,
};
pub use platform::{CursorShape, TextInputState};
pub use semantics::{
    ActionSet, Role, SCROLL_INTO_VIEW, SemanticAction, SemanticActionError, SemanticNode,
    SemanticRequest, SemanticValue, Semantics, Toggled, ValueRequest,
};
pub use text_measure::TextMeasurer;
