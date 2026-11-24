// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI widgets (UI spec §9): the first real widget set, headless
//! and tested with the core's `TestHost`. Widget list, states and keyboard
//! behavior: `docs/widgets.md`.
//!
//! - Stateless builders (render with the theme, report changes through
//!   callbacks): [`Button`], [`IconButton`], [`Toggle`], [`Checkbox`],
//!   [`Radio`], [`RadioGroup`], [`SegmentedControl`], [`Label`],
//!   [`Tooltip`].
//! - Views with editing or popup state: [`Slider`], [`NumberField`],
//!   [`TextField`], [`Select`].
//! - [`Env`] holds fonts, the clipboard and focus tracking, and installs the
//!   widget keymap; [`Env::root`] wraps the content in a [`WidgetRoot`]
//!   whose overlay layer hosts popups and tooltips.
//! - Every widget describes itself with [`Semantics`] (`semantics()`), and
//!   declares it on its element with the core's accessibility builders.
//!
//! ```
//! use livspeil_ui_core::{App, Element, Model, Render, TestHost, ViewContext, div};
//! use livspeil_ui_layout::Size;
//! use livspeil_ui_widgets::{Button, Env};
//!
//! struct Counter {
//!     count: Model<i32>,
//! }
//!
//! impl Render for Counter {
//!     fn render(&mut self, cx: &mut ViewContext<'_, Self>) -> Element {
//!         let count = self.count;
//!         let theme = cx.theme().clone();
//!         div().p(16.0).child(
//!             Button::new("Increment")
//!                 .primary()
//!                 .on_click(move |_, cx| {
//!                     count.update(cx, |c, cx| {
//!                         *c += 1;
//!                         cx.notify();
//!                     });
//!                 })
//!                 .render(&theme),
//!         )
//!     }
//! }
//!
//! let mut app = App::new();
//! let env = Env::headless(&mut app);
//! let count = app.new_model(0);
//! let content = app.new_view(Counter { count });
//! let root = env.root(&mut app, content);
//! app.set_root(root);
//! let mut host = TestHost::new(app, Size::new(400.0, 300.0));
//! host.run_until_idle(100);
//! let b = host.app.bounds(host.app.find("button:Increment").unwrap()).unwrap().center();
//! host.click(b.x, b.y);
//! assert_eq!(*count.read(&host.app), 1);
//! ```

mod button;
mod dock;
mod env;
pub mod keymap;
mod label;
mod menu;
mod number_field;
mod scroll;
mod segmented;
mod select;
pub mod semantics;
mod slider;
mod style;
mod text_box;
mod text_field;
mod toggle;
mod tooltip;

pub use button::{Button, ButtonVariant, IconButton};
pub use dock::{DockView, PANEL_SCOPE, Panel, PanelMaterial, SlotFn};
pub use env::{Clipboard, Env, MemoryClipboard, WidgetRoot};
pub use keymap::{WIDGET_KEYMAP, keymap};
pub use label::{Label, Tone};
pub use menu::{
    ContextMenu, Menu, MenuAnchor, MenuBar, MenuCallback, MenuEntry, MenuItem, MenuPopup,
    SUBMENU_DELAY, close_menu, format_keystroke, open_menu, place_below, place_menu, place_submenu,
    shortcut_for,
};
pub use number_field::{NumberField, Scrub, scrub_precision};
pub use scroll::{
    OVERSCAN, SCROLL_SCOPE, ScrollAxis, ScrollContent, ScrollInfo, ScrollPhysics, ScrollView,
    Scroller, VirtualList, rubber_band, scroll_actions, visible_range,
};
pub use segmented::{SEGMENT_INSET, SegmentedControl};
pub use select::Select;
pub use semantics::{NumericValue, Role, SemanticAction, Semantics, Toggled};
pub use slider::{SLIDER_THUMB, Slider};
pub use style::step_decimals;
pub use style::{ClickFn, DISABLED_OPACITY};
pub use text_field::{TextField, TextFn};
pub use toggle::{CheckState, Checkbox, Radio, RadioGroup, Toggle};
pub use tooltip::{TOOLTIP_DELAY, Tooltip, place as place_tooltip};
