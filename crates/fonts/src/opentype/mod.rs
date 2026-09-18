// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust OpenType / TrueType font ingestion and metric scaling using `ttf-parser`.

use std::collections::HashMap;
use std::sync::Arc;

use crate::error::FontError;
use crate::metrics::{FontDimensions, FontMetricsProvider, GlyphMetrics};
use crate::tfm::scaler::round_xn_over_d;

/// An unscaled in-memory OpenType or TrueType font.
#[derive(Clone)]
pub struct OpenTypeFont {
    /// Raw font data bytes held in an `Arc` for cheap cloning across threads.
    data: Arc<Vec<u8>>,
    /// Face index within font collection (TTC/OTC).
    face_index: u32,
    /// Font units per em.
    pub units_per_em: u16,
    /// Typographic ascender in font units.
    pub ascender: i16,
    /// Typographic descender in font units.
    pub descender: i16,
    /// Typographic line gap in font units.
    pub line_gap: i16,
    /// PostScript name extracted from font `name` table.
    pub postscript_name: String,
    /// Family name extracted from font `name` table if present.
    pub family_name: Option<String>,
}

impl std::fmt::Debug for OpenTypeFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenTypeFont")
            .field("face_index", &self.face_index)
            .field("units_per_em", &self.units_per_em)
            .field("ascender", &self.ascender)
            .field("descender", &self.descender)
            .field("postscript_name", &self.postscript_name)
            .field("family_name", &self.family_name)
            .finish()
    }
}

impl OpenTypeFont {
    /// Ingests an OpenType or TrueType font from bytes and face index.
    pub fn from_bytes(data: Vec<u8>, face_index: u32) -> Result<Self, FontError> {
        let face = ttf_parser::Face::parse(&data, face_index)
            .map_err(|e| FontError::OpenType(format!("failed to parse OpenType face: {e:?}")))?;

        let units_per_em = face.units_per_em();
        let ascender = face.ascender();
        let descender = face.descender();
        let line_gap = face.line_gap();

        let mut postscript_name = None;
        let mut family_name = None;

        for name in face.names() {
            if name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME && postscript_name.is_none() {
                if let Some(s) = name.to_string() {
                    postscript_name = Some(s);
                }
            } else if name.name_id == ttf_parser::name_id::FAMILY && family_name.is_none() {
                if let Some(s) = name.to_string() {
                    family_name = Some(s);
                }
            }
        }

        let postscript_name = postscript_name.unwrap_or_else(|| "UnknownFont".to_string());

        Ok(Self {
            data: Arc::new(data),
            face_index,
            units_per_em,
            ascender,
            descender,
            line_gap,
            postscript_name,
            family_name,
        })
    }

    /// Borrows the internal font data bytes.
    pub fn font_data(&self) -> &[u8] {
        &self.data
    }

    /// Returns the face index within the font collection.
    pub fn face_index(&self) -> u32 {
        self.face_index
    }

    /// Scales the OpenType font to a target size `at_size_sp` in scaled points (`sp`).
    pub fn scale(&self, at_size_sp: i32) -> Result<ScaledOpenType, FontError> {
        let face = ttf_parser::Face::parse(&self.data, self.face_index)
            .map_err(|e| FontError::OpenType(format!("failed to re-parse OpenType face: {e:?}")))?;

        let upem = self.units_per_em as i32;

        // Calculate standard space (U+0020) width
        let space_advance = face
            .glyph_index(' ')
            .and_then(|gid| face.glyph_hor_advance(gid))
            .unwrap_or(self.units_per_em / 3);
        let space_sp = round_xn_over_d(space_advance as i32, at_size_sp, upem);

        // Calculate x-height from lowercase 'x'
        let x_height_sp = face
            .glyph_index('x')
            .and_then(|gid| face.glyph_bounding_box(gid))
            .map(|bbox| round_xn_over_d(bbox.y_max as i32, at_size_sp, upem))
            .unwrap_or_else(|| round_xn_over_d(self.ascender as i32, at_size_sp, upem) / 2);

        // Quad width = 1 em = at_size_sp
        let quad_sp = at_size_sp;

        let dimensions = FontDimensions {
            design_size_sp: at_size_sp,
            scaled_size_sp: at_size_sp,
            slant_ratio: 0,
            space_sp,
            space_stretch_sp: space_sp / 2,
            space_shrink_sp: space_sp / 3,
            x_height_sp,
            quad_sp,
            extra_space_sp: space_sp / 3,
        };

        Ok(ScaledOpenType {
            font: self.clone(),
            dimensions,
            at_size_sp,
            cached_glyphs: std::sync::RwLock::new(HashMap::new()),
        })
    }
}

/// A scaled OpenType font instance supporting thread-safe glyph queries and caching.
pub struct ScaledOpenType {
    /// Underlying unscaled font.
    pub font: OpenTypeFont,
    /// Calculated global font dimensions in scaled points (`sp`).
    pub dimensions: FontDimensions,
    /// Target scaled size in scaled points (`sp`).
    pub at_size_sp: i32,
    /// Internal glyph metric cache for fast layout lookups.
    cached_glyphs: std::sync::RwLock<HashMap<u32, Option<GlyphMetrics>>>,
}

impl std::fmt::Debug for ScaledOpenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScaledOpenType")
            .field("font", &self.font)
            .field("dimensions", &self.dimensions)
            .field("at_size_sp", &self.at_size_sp)
            .finish()
    }
}

impl ScaledOpenType {
    /// Computes glyph metrics for a Unicode codepoint.
    fn compute_glyph_metrics(&self, char_code: u32) -> Option<GlyphMetrics> {
        let ch = char::from_u32(char_code)?;
        let face = ttf_parser::Face::parse(&self.font.data, self.font.face_index).ok()?;
        let gid = face.glyph_index(ch)?;

        let upem = self.font.units_per_em as i32;
        let advance = face.glyph_hor_advance(gid).unwrap_or(0) as i32;
        let width_sp = round_xn_over_d(advance, self.at_size_sp, upem);

        let (height_sp, depth_sp) = if let Some(bbox) = face.glyph_bounding_box(gid) {
            let h = if bbox.y_max > 0 {
                round_xn_over_d(bbox.y_max as i32, self.at_size_sp, upem)
            } else {
                0
            };
            let d = if bbox.y_min < 0 {
                round_xn_over_d((-bbox.y_min) as i32, self.at_size_sp, upem)
            } else {
                0
            };
            (h, d)
        } else {
            (0, 0)
        };

        Some(GlyphMetrics::new(width_sp, height_sp, depth_sp, 0))
    }
}

impl FontMetricsProvider for ScaledOpenType {
    fn dimensions(&self) -> &FontDimensions {
        &self.dimensions
    }

    fn glyph_metrics(&self, char_code: u32) -> Option<GlyphMetrics> {
        // Check cache first
        if let Ok(cache) = self.cached_glyphs.read() {
            if let Some(metrics) = cache.get(&char_code) {
                return *metrics;
            }
        }

        let computed = self.compute_glyph_metrics(char_code);
        if let Ok(mut cache) = self.cached_glyphs.write() {
            cache.insert(char_code, computed);
        }
        computed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opentype_invalid() {
        assert!(matches!(
            OpenTypeFont::from_bytes(vec![0; 16], 0),
            Err(FontError::OpenType(_))
        ));
    }

    #[test]
    fn test_opentype_valid_methods() {
        let font_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/bridge_harfbuzz/harfbuzz/test/api/fonts/cmunrm.otf");
        if !font_path.exists() {
            return;
        }

        let data = std::fs::read(&font_path).expect("read cmunrm.otf");
        let otf = OpenTypeFont::from_bytes(data, 0).expect("parse cmunrm.otf");

        assert_eq!(otf.face_index(), 0);
        assert!(!otf.font_data().is_empty());
        let debug_otf = format!("{:?}", otf);
        assert!(debug_otf.contains("OpenTypeFont"));
        assert!(debug_otf.contains("CMUSerif-Roman"));

        let scaled = otf.scale(655360).expect("scale otf");
        let debug_scaled = format!("{:?}", scaled);
        assert!(debug_scaled.contains("ScaledOpenType"));

        // Space character (usually has no bounding box, tests bbox None path)
        let space_metrics = scaled.glyph_metrics(b' ' as u32);
        assert!(space_metrics.is_some());

        // Test cache hit on second query
        let first_a = scaled.glyph_metrics(b'A' as u32);
        let second_a = scaled.glyph_metrics(b'A' as u32);
        assert_eq!(first_a, second_a);
    }
}
