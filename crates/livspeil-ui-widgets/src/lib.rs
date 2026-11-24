// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI widgets (UI spec §9): the first real widget set, headless
//! and tested with the core's `TestHost`.

mod button;
mod env;
pub mod keymap;
mod label;
mod number_field;
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
pub use env::{Clipboard, Env, MemoryClipboard, WidgetRoot};
pub use keymap::{WIDGET_KEYMAP, keymap};
pub use label::{Label, Tone};
pub use number_field::{NumberField, Scrub, scrub_precision};
pub use segmented::{SEGMENT_INSET, SegmentedControl};
pub use select::Select;
pub use semantics::{NumericValue, Role, SemanticAction, Semantics, Toggled};
pub use slider::{SLIDER_THUMB, Slider};
pub use style::step_decimals;
pub use style::{ClickFn, DISABLED_OPACITY};
pub use text_field::{TextField, TextFn};
pub use toggle::{CheckState, Checkbox, Radio, RadioGroup, Toggle};
pub use tooltip::{TOOLTIP_DELAY, Tooltip, place as place_tooltip};
