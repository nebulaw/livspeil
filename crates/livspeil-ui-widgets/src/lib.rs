// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI widgets (UI spec §9): the first real widget set, headless
//! and tested with the core's `TestHost`.

mod button;
mod env;
mod label;
mod segmented;
mod slider;
mod text_box;
mod text_field;
pub mod keymap;
pub mod semantics;
mod style;
mod toggle;
mod tooltip;

pub use button::{Button, ButtonVariant, IconButton};
pub use keymap::{WIDGET_KEYMAP, keymap};
pub use semantics::{NumericValue, Role, SemanticAction, Semantics, Toggled};
pub use style::{ClickFn, DISABLED_OPACITY};
pub use toggle::{CheckState, Checkbox, Radio, RadioGroup, Toggle};
pub use env::{Clipboard, Env, MemoryClipboard, WidgetRoot};
pub use tooltip::{TOOLTIP_DELAY, Tooltip, place as place_tooltip};
pub use label::{Label, Tone};
pub use segmented::{SEGMENT_INSET, SegmentedControl};
pub use slider::{SLIDER_THUMB, Slider};
pub use text_field::{TextFn, TextField};
