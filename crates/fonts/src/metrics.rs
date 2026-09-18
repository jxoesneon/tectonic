// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Unified font metric abstractions for TeX layout and PDF backend rendering.

/// Rational conversion constant: 1 bp = 25 / 1,644,544 sp (exact).
const SP_PER_BP_NUM: f64 = 25.0;
const SP_PER_BP_DENOM: f64 = 1_644_544.0;

/// Converts TeX scaled points (`sp`) to PDF big points (`bp`).
///
/// $1\,\text{bp} = \frac{25}{1,644,544}\,\text{sp} \approx \frac{1}{65536} \times \frac{72}{72.27}\,\text{sp}$.
#[inline]
pub fn sp_to_bp(sp: i32) -> f64 {
    (sp as f64) * SP_PER_BP_NUM / SP_PER_BP_DENOM
}

/// Converts PDF big points (`bp`) to TeX scaled points (`sp`).
#[inline]
pub fn bp_to_sp(bp: f64) -> i32 {
    (bp * SP_PER_BP_DENOM / SP_PER_BP_NUM).round() as i32
}

/// Standard TeX font dimensions in scaled points (`sp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FontDimensions {
    /// Font design size in scaled points (`sp`).
    pub design_size_sp: i32,
    /// Effective scaled size at which the font was loaded in scaled points (`sp`).
    pub scaled_size_sp: i32,
    /// Slant per point (FixWord ratio of dx / dy).
    pub slant_ratio: i32,
    /// Interword space in scaled points (`sp`).
    pub space_sp: i32,
    /// Interword space stretchability in scaled points (`sp`).
    pub space_stretch_sp: i32,
    /// Interword space shrinkability in scaled points (`sp`).
    pub space_shrink_sp: i32,
    /// The x-height (height of lowercase 'x') in scaled points (`sp`).
    pub x_height_sp: i32,
    /// The quad width (width of 1 em) in scaled points (`sp`).
    pub quad_sp: i32,
    /// Extra space at the end of a sentence in scaled points (`sp`).
    pub extra_space_sp: i32,
}

/// Metrics for an individual character or glyph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GlyphMetrics {
    /// Advance width in scaled points (`sp`).
    pub width_sp: i32,
    /// Character height above the baseline in scaled points (`sp`).
    pub height_sp: i32,
    /// Character depth below the baseline in scaled points (`sp`).
    pub depth_sp: i32,
    /// Italic correction in scaled points (`sp`).
    pub italic_correction_sp: i32,
}

impl GlyphMetrics {
    /// Creates a new `GlyphMetrics` instance with the given dimensions in `sp`.
    pub const fn new(width_sp: i32, height_sp: i32, depth_sp: i32, italic_correction_sp: i32) -> Self {
        Self {
            width_sp,
            height_sp,
            depth_sp,
            italic_correction_sp,
        }
    }

    /// Returns advance width in PDF big points (`bp`).
    pub fn width_bp(&self) -> f64 {
        sp_to_bp(self.width_sp)
    }

    /// Returns height above baseline in PDF big points (`bp`).
    pub fn height_bp(&self) -> f64 {
        sp_to_bp(self.height_sp)
    }

    /// Returns depth below baseline in PDF big points (`bp`).
    pub fn depth_bp(&self) -> f64 {
        sp_to_bp(self.depth_sp)
    }

    /// Returns italic correction in PDF big points (`bp`).
    pub fn italic_correction_bp(&self) -> f64 {
        sp_to_bp(self.italic_correction_sp)
    }

    /// Returns the total bounding box height (`height_sp + depth_sp`).
    pub fn total_height_sp(&self) -> i32 {
        self.height_sp.saturating_add(self.depth_sp)
    }
}

/// Common trait for querying font dimensions and character metrics.
pub trait FontMetricsProvider {
    /// Returns the global dimensions of the font.
    fn dimensions(&self) -> &FontDimensions;

    /// Queries character or glyph metrics for a Unicode codepoint or character code.
    fn glyph_metrics(&self, char_code: u32) -> Option<GlyphMetrics>;

    /// Returns `true` if the font defines a glyph for the given codepoint or character code.
    fn has_glyph(&self, char_code: u32) -> bool {
        self.glyph_metrics(char_code).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unit_conversions() {
        // 65536 sp = 1 pt = (72 / 72.27) bp
        let sp = 65536;
        let bp = sp_to_bp(sp);
        let expected_bp = 72.0 / 72.27;
        assert!((bp - expected_bp).abs() < 1e-4);

        let roundtrip = bp_to_sp(bp);
        assert!((roundtrip - sp).abs() <= 1);
    }

    #[test]
    fn test_glyph_metrics() {
        let gm = GlyphMetrics::new(655360, 458752, 131072, 6553);
        assert_eq!(gm.width_sp, 655360);
        assert_eq!(gm.total_height_sp(), 458752 + 131072);
        assert!(gm.width_bp() > 0.0);
        assert!(gm.height_bp() > 0.0);
        assert!(gm.depth_bp() > 0.0);
        assert!(gm.italic_correction_bp() > 0.0);
    }
}
