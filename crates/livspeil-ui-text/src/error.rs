// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Error type of the text crate.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Errors from font loading, layout and atlas operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum TextError {
    /// A font file could not be read.
    FontIo { path: PathBuf, source: io::Error },
    /// Font data contained no usable font.
    NoFontsInData { label: String },
    /// A byte offset was out of range or not on a character boundary.
    InvalidOffset { offset: usize, len: usize },
    /// A glyph does not fit into the atlas even when empty.
    GlyphTooLarge { width: u32, height: u32 },
    /// The atlas is full; the caller should clear it (or grow it) and
    /// re-prepare the frame.
    AtlasFull,
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextError::FontIo { path, source } => {
                write!(f, "cannot read font {}: {source}", path.display())
            }
            TextError::NoFontsInData { label } => write!(f, "no usable font in {label}"),
            TextError::InvalidOffset { offset, len } => write!(
                f,
                "byte offset {offset} is not a character boundary of a {len}-byte text"
            ),
            TextError::GlyphTooLarge { width, height } => {
                write!(
                    f,
                    "glyph of {width}x{height} px does not fit into the atlas"
                )
            }
            TextError::AtlasFull => write!(f, "glyph atlas is full"),
        }
    }
}

impl std::error::Error for TextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TextError::FontIo { source, .. } => Some(source),
            _ => None,
        }
    }
}
