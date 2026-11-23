// SPDX-FileCopyrightText: 2026 Livspeil contributors
// SPDX-License-Identifier: MPL-2.0

//! Font collection and layout scratch space.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use parley::fontique::{
    Blob, Collection, CollectionOptions, FallbackKey, FamilyId, GenericFamily, Script, SourceCache,
};
use parley::{FontContext, LayoutContext};

use crate::error::TextError;
use crate::style::ColorRole;

/// Family name of the vendored Inter variable font (its name table says
/// "Inter Variable", which also keeps it apart from a system-installed
/// static "Inter").
pub const UI_FAMILY: &str = "Inter Variable";

/// A bundled font file.
#[cfg(feature = "bundled-fonts")]
struct Bundled {
    label: &'static str,
    data: &'static [u8],
}

#[cfg(feature = "bundled-fonts")]
const BUNDLED: &[Bundled] = &[
    Bundled {
        label: "InterVariable.ttf",
        data: include_bytes!("../../../assets/fonts/inter/InterVariable.ttf"),
    },
    Bundled {
        label: "InterVariable-Italic.ttf",
        data: include_bytes!("../../../assets/fonts/inter/InterVariable-Italic.ttf"),
    },
    Bundled {
        label: "NotoSansGeorgian-Variable.ttf",
        data: include_bytes!(
            "../../../assets/fonts/noto-sans-georgian/NotoSansGeorgian-Variable.ttf"
        ),
    },
    Bundled {
        label: "NotoSansArabic-Variable.ttf",
        data: include_bytes!("../../../assets/fonts/noto-sans-arabic/NotoSansArabic-Variable.ttf"),
    },
];

/// Script fallbacks that bundled fonts provide, tried before any system
/// fallback: (ISO 15924 script, family name).
const SCRIPT_FALLBACKS: &[([u8; 4], &str)] = &[
    (*b"Geor", "Noto Sans Georgian"),
    (*b"Arab", "Noto Sans Arabic"),
];

/// Fonts plus layout scratch space. Create one per UI thread and reuse it:
/// font discovery and the shaping caches live here.
pub struct FontSystem {
    pub(crate) fonts: FontContext,
    pub(crate) layout: LayoutContext<ColorRole>,
    system_fonts: bool,
}

impl fmt::Debug for FontSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FontSystem")
            .field("system_fonts", &self.system_fonts)
            .finish_non_exhaustive()
    }
}

impl FontSystem {
    /// An empty collection. With `system_fonts` the platform's fonts are
    /// available as named families and as fallback.
    #[must_use]
    pub fn empty(system_fonts: bool) -> Self {
        let collection = Collection::new(CollectionOptions {
            shared: false,
            system_fonts,
        });
        Self {
            fonts: FontContext {
                collection,
                source_cache: SourceCache::default(),
            },
            layout: LayoutContext::new(),
            system_fonts,
        }
    }

    /// Bundled fonts plus system fonts (for fallback: CJK, emoji, other
    /// scripts). Inter and the bundled Georgian/Arabic faces take
    /// precedence over system faces.
    #[cfg(feature = "bundled-fonts")]
    #[must_use]
    pub fn new() -> Self {
        let mut fs = Self::empty(true);
        fs.register_bundled();
        fs
    }

    /// Deterministic test mode: only bundled fonts, no system lookup.
    /// Scripts without a bundled face (e.g. CJK) shape to `.notdef`.
    #[cfg(feature = "bundled-fonts")]
    #[must_use]
    pub fn test() -> Self {
        let mut fs = Self::empty(false);
        fs.register_bundled();
        fs
    }

    #[cfg(feature = "bundled-fonts")]
    fn register_bundled(&mut self) {
        for font in BUNDLED {
            // Bundled data is known-good; an empty result cannot happen for
            // these files (checked by tests), so ignoring it is fine.
            let _ = self.register_font_data(font.data.to_vec(), font.label);
        }
    }

    /// Whether system fonts were requested.
    #[must_use]
    pub fn has_system_fonts(&self) -> bool {
        self.system_fonts
    }

    /// Register every face found in `data` (TTF/OTF/TTC). Returns the
    /// number of faces added.
    ///
    /// # Errors
    /// [`TextError::NoFontsInData`] when no face could be parsed.
    pub fn register_font_data(&mut self, data: Vec<u8>, label: &str) -> Result<usize, TextError> {
        let added = self
            .fonts
            .collection
            .register_fonts(Blob::new(Arc::new(data)), None);
        let faces = added.iter().map(|(_, fonts)| fonts.len()).sum();
        if faces == 0 {
            return Err(TextError::NoFontsInData {
                label: label.to_owned(),
            });
        }
        self.refresh_fallbacks();
        Ok(faces)
    }

    /// Read and register a font file.
    ///
    /// # Errors
    /// I/O errors and files without usable faces.
    pub fn load_font_file(&mut self, path: &Path) -> Result<usize, TextError> {
        let data = std::fs::read(path).map_err(|source| TextError::FontIo {
            path: path.to_path_buf(),
            source,
        })?;
        self.register_font_data(data, &path.display().to_string())
    }

    /// Names of all known families, sorted (includes system families when
    /// enabled).
    #[must_use]
    pub fn family_names(&mut self) -> Vec<String> {
        let mut names: Vec<String> = self
            .fonts
            .collection
            .family_names()
            .map(str::to_owned)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    fn family(&mut self, name: &str) -> Option<FamilyId> {
        self.fonts.collection.family_id(name)
    }

    /// Put registered faces first in the generic sans-serif list and in the
    /// script fallback lists they cover, keeping system entries after them.
    fn refresh_fallbacks(&mut self) {
        if let Some(ui) = self.family(UI_FAMILY) {
            let mut list = vec![ui];
            list.extend(
                self.fonts
                    .collection
                    .generic_families(GenericFamily::SansSerif)
                    .filter(|id| *id != ui),
            );
            self.fonts
                .collection
                .set_generic_families(GenericFamily::SansSerif, list.into_iter());
        }
        for (script, name) in SCRIPT_FALLBACKS {
            let Some(id) = self.family(name) else {
                continue;
            };
            let key = FallbackKey::new(Script::from_bytes(*script), None);
            let mut list = vec![id];
            list.extend(
                self.fonts
                    .collection
                    .fallback_families(key)
                    .filter(|f| *f != id),
            );
            self.fonts.collection.set_fallbacks(key, list.into_iter());
        }
    }
}

#[cfg(feature = "bundled-fonts")]
impl Default for FontSystem {
    fn default() -> Self {
        Self::new()
    }
}
