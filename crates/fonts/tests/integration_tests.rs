// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Comprehensive integration tests for `tectonic_fonts` verifying against canonical TeX fonts and real OTF files.

use std::path::PathBuf;
use tectonic_fonts::{
    FontMetricsProvider, FontRegistry, OpenTypeFont, TfmError, TfmFont,
};

fn get_asset_path(filename: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/assets")
        .join(filename)
}

fn get_otf_asset_path(filename: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/bridge_harfbuzz/harfbuzz/test/api/fonts")
        .join(filename)
}

#[test]
fn test_canonical_cmr10_tfm() {
    let path = get_asset_path("cmr10.tfm");
    let data = std::fs::read(&path).expect("failed to read cmr10.tfm");

    let tfm = TfmFont::from_bytes(&data).expect("cmr10.tfm should parse successfully");

    // Standard CMR10 properties:
    // bc=0, ec=127 (7-bit font)
    assert_eq!(tfm.bc, 0);
    assert_eq!(tfm.ec, 127);

    // Design size: 10 pt = 655360 sp
    assert_eq!(tfm.header.design_size_sp, 655360);
    assert_eq!(tfm.header.coding_scheme.as_deref(), Some("TeX text"));
    assert_eq!(tfm.header.font_family.as_deref(), Some("CMR"));
    assert!(tfm.header.seven_bit_safe);

    // Standard character 'A' (ASCII 65):
    let char_a = tfm.char_info(b'A').expect("char 'A' should exist");
    assert!(char_a.exists());
    let width_fix = tfm.char_width_fix(b'A').expect("width should exist");
    assert!(width_fix.0 > 0);

    // Scale to design size (10pt = 655360 sp)
    let scaled = tfm.scale(655360);
    let metrics_a = scaled.glyph_metrics(b'A' as u32).expect("metrics for 'A'");
    assert!(metrics_a.width_sp > 0);
    assert!(metrics_a.height_sp > 0);
    assert_eq!(metrics_a.depth_sp, 0); // Capital 'A' has no depth below baseline
    assert!(metrics_a.width_bp() > 0.0);

    // Font dimensions for CMR10:
    assert_eq!(scaled.dimensions().design_size_sp, 655360);
    assert_eq!(scaled.dimensions().scaled_size_sp, 655360);
    assert!(scaled.dimensions().space_sp > 0);
    assert!(scaled.dimensions().quad_sp > 0);
    assert!(scaled.dimensions().x_height_sp > 0);
}

#[test]
fn test_canonical_cmbx10_tfm() {
    let path = get_asset_path("cmbx10.tfm");
    let data = std::fs::read(&path).expect("failed to read cmbx10.tfm");

    let tfm = TfmFont::from_bytes(&data).expect("cmbx10.tfm should parse successfully");
    assert_eq!(tfm.header.font_family.as_deref(), Some("CMBX"));
    assert_eq!(tfm.header.design_size_sp, 655360);

    let scaled = tfm.scale(655360);
    let metrics_a = scaled.glyph_metrics(b'A' as u32).expect("metrics for 'A'");
    // Bold 'A' is wider than regular 'A'
    let cmr10_data = std::fs::read(get_asset_path("cmr10.tfm")).unwrap();
    let cmr10_scaled = TfmFont::from_bytes(&cmr10_data).unwrap().scale(655360);
    let cmr10_a = cmr10_scaled.glyph_metrics(b'A' as u32).unwrap();

    assert!(metrics_a.width_sp > cmr10_a.width_sp);
}

#[test]
fn test_canonical_cmti10_tfm() {
    let path = get_asset_path("cmti10.tfm");
    let data = std::fs::read(&path).expect("failed to read cmti10.tfm");

    let tfm = TfmFont::from_bytes(&data).expect("cmti10.tfm should parse successfully");
    assert_eq!(tfm.header.font_family.as_deref(), Some("CMTI"));

    let scaled = tfm.scale(655360);
    // Italic text font should have non-zero slant and italic corrections
    assert!(scaled.dimensions().slant_ratio > 0);

    let metrics_f = scaled.glyph_metrics(b'f' as u32).expect("metrics for 'f'");
    assert!(metrics_f.italic_correction_sp > 0);
}

#[test]
fn test_canonical_cmex10_math_extension_tfm() {
    let path = get_asset_path("cmex10.tfm");
    let data = std::fs::read(&path).expect("failed to read cmex10.tfm");

    let tfm = TfmFont::from_bytes(&data).expect("cmex10.tfm should parse successfully");
    assert_eq!(
        tfm.header.coding_scheme.as_deref(),
        Some("TeX math extension")
    );

    // Math extension fonts have extensible recipes
    assert!(!tfm.extensibles.is_empty());

    let scaled = tfm.scale(655360);
    assert!(scaled.has_glyph(0x20));
}

#[test]
fn test_canonical_cmmi10_math_italic_tfm() {
    let path = get_asset_path("cmmi10.tfm");
    let data = std::fs::read(&path).expect("failed to read cmmi10.tfm");

    let tfm = TfmFont::from_bytes(&data).expect("cmmi10.tfm should parse successfully");
    assert_eq!(
        tfm.header.coding_scheme.as_deref(),
        Some("TeX math italic")
    );
    assert_eq!(tfm.header.font_family.as_deref(), Some("CMMI"));
}

#[test]
fn test_canonical_cmsy10_math_symbols_tfm() {
    let path = get_asset_path("cmsy10.tfm");
    let data = std::fs::read(&path).expect("failed to read cmsy10.tfm");

    let tfm = TfmFont::from_bytes(&data).expect("cmsy10.tfm should parse successfully");
    assert_eq!(
        tfm.header.coding_scheme.as_deref(),
        Some("TeX math symbols")
    );
    assert_eq!(tfm.header.font_family.as_deref(), Some("CMSY"));

    // Math symbols fonts have additional parameters (num1..3, denom1..2, sup1..3, sub1..2, etc.)
    assert!(!tfm.params.additional.is_empty());
}

#[test]
fn test_trip_tfm() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/trip/trip.tfm");
    if path.exists() {
        let data = std::fs::read(&path).expect("failed to read trip.tfm");
        let tfm = TfmFont::from_bytes(&data).expect("trip.tfm should parse successfully");
        assert_eq!(tfm.header.design_size_sp, 655360);
    }
}

#[test]
fn test_opentype_cmunrm_font() {
    let path = get_otf_asset_path("cmunrm.otf");
    if !path.exists() {
        eprintln!("cmunrm.otf not found, skipping");
        return;
    }

    let data = std::fs::read(&path).expect("failed to read cmunrm.otf");
    let otf = OpenTypeFont::from_bytes(data, 0).expect("cmunrm.otf should parse successfully");

    assert_eq!(otf.postscript_name, "CMUSerif-Roman");
    assert!(otf.units_per_em > 0);
    assert!(otf.ascender > 0);
    assert!(otf.descender < 0);

    let scaled = otf.scale(655360).expect("scaling should succeed");
    assert_eq!(scaled.dimensions().scaled_size_sp, 655360);
    assert!(scaled.dimensions().space_sp > 0);
    assert!(scaled.dimensions().quad_sp == 655360);

    // Query 'A'
    let gm_a = scaled.glyph_metrics(b'A' as u32).expect("glyph 'A'");
    assert!(gm_a.width_sp > 0);
    assert!(gm_a.height_sp > 0);

    // Query nonexistent character
    assert_eq!(scaled.glyph_metrics(0x10_FFFF), None);
}

#[test]
fn test_unified_font_registry() {
    let mut reg = FontRegistry::new();

    // Register CMR10 as font 0
    let cmr10_data = std::fs::read(get_asset_path("cmr10.tfm")).unwrap();
    let f0 = reg
        .register_tfm(0, "cmr10", &cmr10_data, 655360)
        .expect("register cmr10");
    assert_eq!(f0.font_id(), 0);
    assert_eq!(f0.name(), "cmr10");
    assert_eq!(f0.postscript_name(), "cmr10");
    assert!(f0.is_tfm());
    assert!(!f0.is_opentype());
    assert_eq!(f0.size_sp(), 655360);
    assert!(f0.size_bp() > 0.0);

    // Register CMBX10 as font 1
    let cmbx10_data = std::fs::read(get_asset_path("cmbx10.tfm")).unwrap();
    let f1 = reg
        .register_tfm(1, "cmbx10", &cmbx10_data, 786432) // 12pt
        .expect("register cmbx10");
    assert_eq!(f1.font_id(), 1);
    assert_eq!(f1.size_sp(), 786432);

    // Register OpenType font if available
    let otf_path = get_otf_asset_path("cmunrm.otf");
    if otf_path.exists() {
        let otf_data = std::fs::read(&otf_path).unwrap();
        let f2 = reg
            .register_opentype(2, "cmunrm.otf", otf_data, 0, 655360)
            .expect("register otf");
        assert_eq!(f2.font_id(), 2);
        assert!(f2.is_opentype());
        assert!(!f2.is_tfm());
        assert_eq!(f2.postscript_name(), "CMUSerif-Roman");
    }

    // Lookups
    assert!(reg.get(0).is_some());
    assert!(reg.get(1).is_some());
    assert!(reg.get_by_name("cmr10").is_some());
    assert!(reg.get_by_name("cmbx10").is_some());
    assert!(reg.get_by_name("nonexistent").is_none());

    // Iterator
    let count = reg.iter().count();
    assert_eq!(count, reg.len());
}

#[test]
fn test_tfm_error_handling() {
    // 1. Empty data
    assert_eq!(TfmFont::from_bytes(&[]), Err(TfmError::TruncatedHeader));

    // 2. Data with invalid word count
    let mut bad_words = vec![0u8; 40];
    bad_words[0] = 0;
    bad_words[1] = 50; // declares 50 words but length is 10 words
    assert!(matches!(
        TfmFont::from_bytes(&bad_words),
        Err(TfmError::FileLengthMismatch { .. })
    ));

    // 3. Invalid char bounds (bc > ec + 1)
    let mut bad_bounds = vec![0u8; 100];
    bad_bounds[4] = 0;
    bad_bounds[5] = 100; // bc = 100
    bad_bounds[6] = 0;
    bad_bounds[7] = 50;  // ec = 50
    assert!(matches!(
        TfmFont::from_bytes(&bad_bounds),
        Err(TfmError::InvalidCharBounds { .. })
    ));
}
