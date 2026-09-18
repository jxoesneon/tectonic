// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Centralized, re-entrant font registry managing TFM and OpenType fonts.

use std::collections::HashMap;

use crate::error::FontError;
use crate::metrics::{FontDimensions, FontMetricsProvider, GlyphMetrics, sp_to_bp};
use crate::opentype::{OpenTypeFont, ScaledOpenType};
use crate::tfm::{ScaledTfm, TfmFont};

/// A loaded, scaled font ready for layout measurement and rendering.
#[derive(Debug)]
pub enum LoadedFont {
    /// Traditional TeX Font Metric font.
    Tfm {
        /// Assigned numeric font identifier.
        font_id: u32,
        /// Canonical font name (e.g. "cmr10").
        name: String,
        /// PostScript font name.
        postscript_name: String,
        /// Scaled font instance.
        scaled: Box<ScaledTfm>,
    },
    /// Native OpenType / TrueType font.
    OpenType {
        /// Assigned numeric font identifier.
        font_id: u32,
        /// Canonical font name or file path.
        name: String,
        /// PostScript font name.
        postscript_name: String,
        /// Scaled OpenType font instance.
        scaled: ScaledOpenType,
    },
}

impl LoadedFont {
    /// Returns the unique numeric font identifier.
    pub fn font_id(&self) -> u32 {
        match self {
            Self::Tfm { font_id, .. } => *font_id,
            Self::OpenType { font_id, .. } => *font_id,
        }
    }

    /// Returns the primary name of the font.
    pub fn name(&self) -> &str {
        match self {
            Self::Tfm { name, .. } => name,
            Self::OpenType { name, .. } => name,
        }
    }

    /// Returns the PostScript name of the font.
    pub fn postscript_name(&self) -> &str {
        match self {
            Self::Tfm { postscript_name, .. } => postscript_name,
            Self::OpenType { postscript_name, .. } => postscript_name,
        }
    }

    /// Returns the scaled font size in scaled points (`sp`).
    pub fn size_sp(&self) -> i32 {
        match self {
            Self::Tfm { scaled, .. } => scaled.dimensions.scaled_size_sp,
            Self::OpenType { scaled, .. } => scaled.dimensions.scaled_size_sp,
        }
    }

    /// Returns the scaled font size in PDF points (`bp`).
    pub fn size_bp(&self) -> f64 {
        sp_to_bp(self.size_sp())
    }

    /// Returns `true` if this font is a traditional TFM font.
    pub fn is_tfm(&self) -> bool {
        matches!(self, Self::Tfm { .. })
    }

    /// Returns `true` if this font is a native OpenType/TrueType font.
    pub fn is_opentype(&self) -> bool {
        matches!(self, Self::OpenType { .. })
    }
}

impl FontMetricsProvider for LoadedFont {
    fn dimensions(&self) -> &FontDimensions {
        match self {
            Self::Tfm { scaled, .. } => scaled.dimensions(),
            Self::OpenType { scaled, .. } => scaled.dimensions(),
        }
    }

    fn glyph_metrics(&self, char_code: u32) -> Option<GlyphMetrics> {
        match self {
            Self::Tfm { scaled, .. } => scaled.glyph_metrics(char_code),
            Self::OpenType { scaled, .. } => scaled.glyph_metrics(char_code),
        }
    }
}

/// A re-entrant, thread-safe registry of fonts active in a compilation document.
#[derive(Debug, Default)]
pub struct FontRegistry {
    fonts: HashMap<u32, LoadedFont>,
    name_to_id: HashMap<String, u32>,
}

impl FontRegistry {
    /// Creates a new empty font registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads and registers a TFM font from raw bytes.
    pub fn register_tfm(
        &mut self,
        font_id: u32,
        name: &str,
        tfm_bytes: &[u8],
        at_size_sp: i32,
    ) -> Result<&LoadedFont, FontError> {
        let tfm = TfmFont::from_bytes(tfm_bytes)?;
        let scaled = tfm.scale(at_size_sp);
        let postscript_name = sanitize_ps_name(name);

        let loaded = LoadedFont::Tfm {
            font_id,
            name: name.to_string(),
            postscript_name,
            scaled: Box::new(scaled),
        };

        self.fonts.insert(font_id, loaded);
        self.name_to_id.insert(name.to_string(), font_id);
        Ok(self.fonts.get(&font_id).unwrap())
    }

    /// Loads and registers an OpenType/TrueType font from raw bytes.
    pub fn register_opentype(
        &mut self,
        font_id: u32,
        name: &str,
        font_bytes: Vec<u8>,
        face_index: u32,
        at_size_sp: i32,
    ) -> Result<&LoadedFont, FontError> {
        let otf = OpenTypeFont::from_bytes(font_bytes, face_index)?;
        let postscript_name = otf.postscript_name.clone();
        let scaled = otf.scale(at_size_sp)?;

        let loaded = LoadedFont::OpenType {
            font_id,
            name: name.to_string(),
            postscript_name,
            scaled,
        };

        self.fonts.insert(font_id, loaded);
        self.name_to_id.insert(name.to_string(), font_id);
        Ok(self.fonts.get(&font_id).unwrap())
    }

    /// Looks up a registered font by its numeric identifier.
    pub fn get(&self, font_id: u32) -> Option<&LoadedFont> {
        self.fonts.get(&font_id)
    }

    /// Looks up a registered font by its primary name.
    pub fn get_by_name(&self, name: &str) -> Option<&LoadedFont> {
        self.name_to_id.get(name).and_then(|id| self.fonts.get(id))
    }

    /// Returns the number of registered fonts.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Returns `true` if the registry contains no fonts.
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }

    /// Iterates over all registered fonts.
    pub fn iter(&self) -> impl Iterator<Item = &LoadedFont> {
        self.fonts.values()
    }
}

/// Sanitizes a font name into a valid PostScript name without spaces or forbidden characters.
pub fn sanitize_ps_name(name: &str) -> String {
    let base = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit('\\')
        .next()
        .unwrap_or(name);
    let mut ps = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            ps.push(c);
        } else if c == '.' {
            break; // Strip extension
        }
    }
    if ps.is_empty() {
        "DefaultFont".to_string()
    } else {
        ps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_ps_name() {
        assert_eq!(sanitize_ps_name("/fonts/cmr10.tfm"), "cmr10");
        assert_eq!(sanitize_ps_name("C:\\Windows\\Fonts\\Arial.ttf"), "Arial");
        assert_eq!(sanitize_ps_name("My Custom Font!"), "MyCustomFont");
        assert_eq!(sanitize_ps_name(""), "DefaultFont");
    }

    #[test]
    fn test_empty_registry() {
        let reg = FontRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
        assert!(reg.get(0).is_none());
        assert!(reg.get_by_name("cmr10").is_none());
    }

    #[test]
    fn test_loaded_font_methods() {
        let mut reg = FontRegistry::new();
        let tfm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/assets/cmr10.tfm");
        let tfm_data = std::fs::read(tfm_path).unwrap();

        let tfm_font = reg.register_tfm(10, "cmr10", &tfm_data, 655360).unwrap();
        assert_eq!(tfm_font.font_id(), 10);
        assert_eq!(tfm_font.name(), "cmr10");
        assert_eq!(tfm_font.postscript_name(), "cmr10");
        assert!(tfm_font.is_tfm());
        assert!(!tfm_font.is_opentype());
        assert_eq!(tfm_font.size_sp(), 655360);
        assert!(tfm_font.size_bp() > 0.0);
        assert_eq!(tfm_font.dimensions().scaled_size_sp, 655360);
        assert!(tfm_font.glyph_metrics(b'A' as u32).is_some());

        let otf_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/bridge_harfbuzz/harfbuzz/test/api/fonts/cmunrm.otf");
        if otf_path.exists() {
            let otf_data = std::fs::read(otf_path).unwrap();
            let otf_font = reg.register_opentype(20, "cmunrm.otf", otf_data, 0, 655360).unwrap();
            assert_eq!(otf_font.font_id(), 20);
            assert_eq!(otf_font.name(), "cmunrm.otf");
            assert_eq!(otf_font.postscript_name(), "CMUSerif-Roman");
            assert!(otf_font.is_opentype());
            assert!(!otf_font.is_tfm());
            assert_eq!(otf_font.size_sp(), 655360);
            assert!(otf_font.size_bp() > 0.0);
            assert_eq!(otf_font.dimensions().scaled_size_sp, 655360);
            assert!(otf_font.glyph_metrics(b'A' as u32).is_some());
        }
    }
}
