// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Exact unit conversions and coordinate transformations between TeX and PDF.
//!
//! TeX computes all dimensions in 32-bit fixed point scaled points (`sp`),
//! where 1 pt = 65,536 sp and 72.27 pt = 1 in.
//!
//! PDF uses Big Points (`bp`), where 72 bp = 1 in.
//! The exact rational conversion factor is:
//! 1 bp = 72.27 * 65536 / 72 = 4736286.72 / 72 = 65,781.76 sp
//! or sp -> bp: sp * 25 / 1644544.

/// The exact number of scaled points in a TeX point (2^16).
pub const SP_PER_PT: i64 = 65536;

/// Ratio numerator for converting TeX `sp` to PDF `bp`: `7200 / 473628672 = 25 / 1644544`.
pub const SP_TO_BP_NUMERATOR: f64 = 25.0;

/// Ratio denominator for converting TeX `sp` to PDF `bp`: `7200 / 473628672 = 25 / 1644544`.
pub const SP_TO_BP_DENOMINATOR: f64 = 1644544.0;

/// Standard TeX origin offset from page edges in PDF points (1 inch = 72 bp).
pub const TEX_ORIGIN_OFFSET_BP: f64 = 72.0;

/// Default US Letter paper width in PDF points (8.5 inches * 72 bp/in).
pub const DEFAULT_PAGE_WIDTH_BP: f64 = 8.5 * 72.0; // 612.0 bp

/// Default US Letter paper height in PDF points (11.0 inches * 72 bp/in).
pub const DEFAULT_PAGE_HEIGHT_BP: f64 = 11.0 * 72.0; // 792.0 bp

/// Converts TeX scaled points (`sp`) to PDF big points (`bp`).
#[inline]
pub fn sp_to_bp(sp: i32) -> f64 {
    (sp as f64) * SP_TO_BP_NUMERATOR / SP_TO_BP_DENOMINATOR
}

/// Converts TeX points (`pt`) to PDF big points (`bp`).
#[inline]
pub fn pt_to_bp(pt: f64) -> f64 {
    pt * 72.0 / 72.27
}

/// Converts PDF big points (`bp`) to TeX scaled points (`sp`).
#[inline]
pub fn bp_to_sp(bp: f64) -> i32 {
    (bp * SP_TO_BP_DENOMINATOR / SP_TO_BP_NUMERATOR).round() as i32
}

/// Converts DVI coordinate `(x, y)` in `sp` to PDF coordinate `(x, y)` in `bp`.
///
/// DVI coordinates have their origin at (1 in, 1 in) from the top-left of the page,
/// with y increasing downwards.
/// PDF coordinates have their origin at (0, 0) at the bottom-left of the page,
/// with y increasing upwards.
#[inline]
pub fn dvi_to_pdf_coords(dvi_x_sp: i32, dvi_y_sp: i32, page_height_bp: f64) -> (f64, f64) {
    let pdf_x = TEX_ORIGIN_OFFSET_BP + sp_to_bp(dvi_x_sp);
    let pdf_y = page_height_bp - TEX_ORIGIN_OFFSET_BP - sp_to_bp(dvi_y_sp);
    (pdf_x, pdf_y)
}

/// Rectangle bounds in PDF coordinate space (points).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PdfRect {
    /// Lower-left X coordinate.
    pub x: f64,
    /// Lower-left Y coordinate.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

impl PdfRect {
    /// Creates a new rectangle in PDF points.
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// Converts a TeX DVI rule to PDF rectangle coordinates.
    ///
    /// In DVI, a rule at `(x, y)` with dimensions `(w, h)` is positioned such that
    /// `y` is the bottom of the rule and it extends upwards by `h`.
    pub fn from_dvi_rule(dvi_x_sp: i32, dvi_y_sp: i32, width_sp: i32, height_sp: i32, page_height_bp: f64) -> Self {
        let (pdf_x, pdf_y) = dvi_to_pdf_coords(dvi_x_sp, dvi_y_sp, page_height_bp);
        let width_bp = sp_to_bp(width_sp);
        let height_bp = sp_to_bp(height_sp);
        Self {
            x: pdf_x,
            y: pdf_y,
            width: width_bp,
            height: height_bp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unit_conversions() {
        // 1 pt in sp = 65536
        let pt_sp = 65536;
        let bp = sp_to_bp(pt_sp);
        let expected_bp = 72.0 / 72.27;
        assert!((bp - expected_bp).abs() < 1e-10);

        // Round-trip conversion
        let round_trip_sp = bp_to_sp(bp);
        assert_eq!(round_trip_sp, pt_sp);

        // 72.27 pt = 72 bp
        assert!((pt_to_bp(72.27) - 72.0).abs() < 1e-10);
    }

    #[test]
    fn test_dvi_to_pdf_coords() {
        let page_height = 792.0; // 11 inches
        // (0, 0) in DVI should map to (72, 792 - 72) = (72, 720) in PDF
        let (x, y) = dvi_to_pdf_coords(0, 0, page_height);
        assert_eq!(x, 72.0);
        assert_eq!(y, 720.0);
    }

    #[test]
    fn test_pdf_rect_from_dvi_rule() {
        let page_height = 792.0;
        let rule = PdfRect::from_dvi_rule(0, 0, 65536, 131072, page_height);
        assert_eq!(rule.x, 72.0);
        assert_eq!(rule.y, 720.0);
        assert!((rule.width - (72.0 / 72.27)).abs() < 1e-10);
        assert!((rule.height - (144.0 / 72.27)).abs() < 1e-10);
    }
}
