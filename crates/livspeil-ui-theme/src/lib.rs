// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Livspeil UI theme: design tokens, color roles (OKLCH), elevations,
//! materials, densities, motion profiles, and config files with hot reload.
//!
//! Config files live in `~/.config/livspeil/ui/` (see [`config_dir`]):
//! `motion.toml` ([`MotionConfig::parse`]) and `theme.toml`
//! ([`Theme::parse`]). Both are strict and report errors with line and
//! column. [`ConfigWatcher`] polls them and yields [`ConfigEvent`]s. The
//! formats are documented in `docs/ui-config.md`.

mod color;
mod error;
mod motion;
mod theme_file;
mod tokens;
mod watch;

pub use color::{Color, ColorParseError};
pub use error::{ConfigError, line_col};
pub use motion::{
    MOTION_FILE, MotionClass, MotionConfig, MotionProfile, MotionSpec, PropertyKind, ScopeOverride,
    SpringSpec,
};
pub use theme_file::THEME_FILE;
pub use tokens::{
    COLOR_ROLE_NAMES, ColorRoles, Density, DensityMetrics, Elevation, IconSizes, Material,
    MaterialRecipe, Radii, Scheme, Shadow, ShadowLayer, Spacing, Theme, Tier, Tokens, TypeScale,
    clamp_contrast, material_recipe,
};
pub use watch::{
    Clock, ConfigEvent, ConfigFs, ConfigKind, ConfigWatcher, StdFs, SystemClock, config_dir,
};
