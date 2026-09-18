// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust TeX Font Metric (TFM) parser and data structures.

#![allow(clippy::chunks_exact_to_as_chunks)]

pub mod char_info;
pub mod header;
pub mod scaler;
pub mod tables;

use std::collections::HashSet;

use byteorder::{BigEndian, ByteOrder};

use crate::error::TfmError;
use crate::metrics::{FontDimensions, FontMetricsProvider, GlyphMetrics};
use self::char_info::{CharInfo, CharTag};
use self::header::TfmHeader;
use self::scaler::TfmScaler;
use self::tables::{ExtensibleRecipe, FixWord, FontParams, LigKernStep};

/// A fully parsed in-memory TeX Font Metric (TFM) structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TfmFont {
    /// Parsed TFM header information.
    pub header: TfmHeader,
    /// Smallest character code present in the font.
    pub bc: u8,
    /// Largest character code present in the font.
    pub ec: u8,
    /// Character info table for character codes `bc..=ec`.
    pub char_info: Vec<CharInfo>,
    /// Width table (index 0 is always 0.0).
    pub widths: Vec<FixWord>,
    /// Height table (index 0 is always 0.0).
    pub heights: Vec<FixWord>,
    /// Depth table (index 0 is always 0.0).
    pub depths: Vec<FixWord>,
    /// Italic correction table (index 0 is always 0.0).
    pub italics: Vec<FixWord>,
    /// Ligature and kerning program table.
    pub lig_kern: Vec<LigKernStep>,
    /// Kerning table.
    pub kerns: Vec<FixWord>,
    /// Extensible character recipes table.
    pub extensibles: Vec<ExtensibleRecipe>,
    /// Font parameter dimensions.
    pub params: FontParams,
}

impl TfmFont {
    /// Parses a TFM file from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self, TfmError> {
        if data.len() < 24 {
            return Err(TfmError::TruncatedHeader);
        }

        // Read the first 12 16-bit integers
        let lf = BigEndian::read_u16(&data[0..2]) as usize;
        let lh = BigEndian::read_u16(&data[2..4]) as usize;
        let bc = BigEndian::read_u16(&data[4..6]);
        let ec = BigEndian::read_u16(&data[6..8]);
        let nw = BigEndian::read_u16(&data[8..10]) as usize;
        let nh = BigEndian::read_u16(&data[10..12]) as usize;
        let nd = BigEndian::read_u16(&data[12..14]) as usize;
        let ni = BigEndian::read_u16(&data[14..16]) as usize;
        let nl = BigEndian::read_u16(&data[16..18]) as usize;
        let nk = BigEndian::read_u16(&data[18..20]) as usize;
        let ne = BigEndian::read_u16(&data[20..22]) as usize;
        let np = BigEndian::read_u16(&data[22..24]) as usize;

        // Validation per TeX specification (read_font_info in xetex0.c)
        if bc > ec + 1 || ec > 255 {
            return Err(TfmError::InvalidCharBounds { bc, ec });
        }

        let (bc, ec) = if bc > 255 {
            // TeX convention for empty font range
            (1, 0)
        } else {
            (bc as u8, ec as u8)
        };

        let num_chars = if bc <= ec {
            (ec - bc + 1) as usize
        } else {
            0
        };

        let expected_words = 6 + lh + num_chars + nw + nh + nd + ni + nl + nk + ne + np;
        if lf != expected_words {
            return Err(TfmError::FileLengthMismatch {
                declared: lf,
                declared_bytes: lf * 4,
                actual_bytes: data.len(),
            });
        }

        if data.len() < lf * 4 {
            return Err(TfmError::TruncatedHeader);
        }

        if nw == 0 || nh == 0 || nd == 0 || ni == 0 {
            return Err(TfmError::InvalidTableLength {
                name: "nw/nh/nd/ni",
                len: 0,
            });
        }

        // Header
        let header_start = 24;
        let header_end = header_start + lh * 4;
        let header = TfmHeader::parse(&data[header_start..header_end], lh)?;

        // Char Info
        let char_info_start = header_end;
        let char_info_end = char_info_start + num_chars * 4;
        let mut char_infos = Vec::with_capacity(num_chars);
        for chunk in data[char_info_start..char_info_end].chunks_exact(4) {
            char_infos.push(CharInfo::from_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }

        // Validate char info indices
        for (i, ci) in char_infos.iter().enumerate() {
            let code = bc + i as u8;
            if ci.width_index as usize >= nw {
                return Err(TfmError::InvalidCharInfo {
                    code,
                    reason: "width index out of bounds",
                });
            }
            if ci.height_index as usize >= nh {
                return Err(TfmError::InvalidCharInfo {
                    code,
                    reason: "height index out of bounds",
                });
            }
            if ci.depth_index as usize >= nd {
                return Err(TfmError::InvalidCharInfo {
                    code,
                    reason: "depth index out of bounds",
                });
            }
            if ci.italic_index as usize >= ni {
                return Err(TfmError::InvalidCharInfo {
                    code,
                    reason: "italic index out of bounds",
                });
            }

            match ci.tag {
                CharTag::LigKern => {
                    if ci.remainder as usize >= nl {
                        return Err(TfmError::InvalidCharInfo {
                            code,
                            reason: "lig_kern remainder out of bounds",
                        });
                    }
                }
                CharTag::Extensible => {
                    if ci.remainder as usize >= ne {
                        return Err(TfmError::InvalidCharInfo {
                            code,
                            reason: "extensible remainder out of bounds",
                        });
                    }
                }
                CharTag::List => {
                    // Check list cycles
                    let mut curr = ci.remainder;
                    let mut visited = HashSet::new();
                    visited.insert(code);
                    while curr >= bc && curr <= ec {
                        if !visited.insert(curr) {
                            return Err(TfmError::InvalidCharInfo {
                                code,
                                reason: "cyclic character list tag",
                            });
                        }
                        let next_ci = &char_infos[(curr - bc) as usize];
                        if next_ci.tag == CharTag::List {
                            curr = next_ci.remainder;
                        } else {
                            break;
                        }
                    }
                }
                CharTag::Vanilla => {}
            }
        }

        // Helper to read table of FixWords
        let mut offset = char_info_end;
        let read_fix_table = |start: usize, count: usize, data: &[u8]| -> Vec<FixWord> {
            let mut words = Vec::with_capacity(count);
            for chunk in data[start..start + count * 4].chunks_exact(4) {
                words.push(FixWord(BigEndian::read_i32(chunk)));
            }
            words
        };

        // Width table
        let widths = read_fix_table(offset, nw, data);
        if widths[0] != FixWord::ZERO {
            return Err(TfmError::NonZeroZeroIndex { table: "width" });
        }
        offset += nw * 4;

        // Height table
        let heights = read_fix_table(offset, nh, data);
        if heights[0] != FixWord::ZERO {
            return Err(TfmError::NonZeroZeroIndex { table: "height" });
        }
        offset += nh * 4;

        // Depth table
        let depths = read_fix_table(offset, nd, data);
        if depths[0] != FixWord::ZERO {
            return Err(TfmError::NonZeroZeroIndex { table: "depth" });
        }
        offset += nd * 4;

        // Italic table
        let italics = read_fix_table(offset, ni, data);
        if italics[0] != FixWord::ZERO {
            return Err(TfmError::NonZeroZeroIndex { table: "italic" });
        }
        offset += ni * 4;

        // Lig/Kern table
        let mut lig_kern = Vec::with_capacity(nl);
        for chunk in data[offset..offset + nl * 4].chunks_exact(4) {
            lig_kern.push(LigKernStep::from_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        offset += nl * 4;

        // Kern table
        let kerns = read_fix_table(offset, nk, data);
        offset += nk * 4;

        // Extensible table
        let mut extensibles = Vec::with_capacity(ne);
        for chunk in data[offset..offset + ne * 4].chunks_exact(4) {
            extensibles.push(ExtensibleRecipe::from_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        offset += ne * 4;

        // Param table
        let param_words = read_fix_table(offset, np, data);
        let params = FontParams::from_words(&param_words);

        Ok(Self {
            header,
            bc,
            ec,
            char_info: char_infos,
            widths,
            heights,
            depths,
            italics,
            lig_kern,
            kerns,
            extensibles,
            params,
        })
    }

    /// Queries the `CharInfo` word for a character code, if it is within range.
    pub fn char_info(&self, ch: u8) -> Option<&CharInfo> {
        if ch >= self.bc && ch <= self.ec {
            self.char_info.get((ch - self.bc) as usize)
        } else {
            None
        }
    }

    /// Queries the unscaled `FixWord` advance width for character `ch`.
    pub fn char_width_fix(&self, ch: u8) -> Option<FixWord> {
        self.char_info(ch).map(|ci| self.widths[ci.width_index as usize])
    }

    /// Queries the unscaled `FixWord` height for character `ch`.
    pub fn char_height_fix(&self, ch: u8) -> Option<FixWord> {
        self.char_info(ch).map(|ci| self.heights[ci.height_index as usize])
    }

    /// Queries the unscaled `FixWord` depth for character `ch`.
    pub fn char_depth_fix(&self, ch: u8) -> Option<FixWord> {
        self.char_info(ch).map(|ci| self.depths[ci.depth_index as usize])
    }

    /// Queries the unscaled `FixWord` italic correction for character `ch`.
    pub fn char_italic_fix(&self, ch: u8) -> Option<FixWord> {
        self.char_info(ch).map(|ci| self.italics[ci.italic_index as usize])
    }

    /// Scales this TFM font to a target size `at_size_sp` in scaled points.
    ///
    /// If `at_size_sp <= 0`, scales using font design size.
    pub fn scale(&self, at_size_sp: i32) -> ScaledTfm {
        let actual_size = if at_size_sp > 0 {
            at_size_sp
        } else {
            self.header.design_size_sp
        };

        let scaler = TfmScaler::new(actual_size);

        let dimensions = FontDimensions {
            design_size_sp: self.header.design_size_sp,
            scaled_size_sp: actual_size,
            slant_ratio: self.params.slant.0,
            space_sp: self.params.space.scale(&scaler),
            space_stretch_sp: self.params.space_stretch.scale(&scaler),
            space_shrink_sp: self.params.space_shrink.scale(&scaler),
            x_height_sp: self.params.x_height.scale(&scaler),
            quad_sp: self.params.quad.scale(&scaler),
            extra_space_sp: self.params.extra_space.scale(&scaler),
        };

        let mut metrics = [None; 256];
        if self.bc <= self.ec {
            for code in self.bc..=self.ec {
                if let Some(ci) = self.char_info(code) {
                    if ci.exists() {
                        let w = self.widths[ci.width_index as usize].scale(&scaler);
                        let h = self.heights[ci.height_index as usize].scale(&scaler);
                        let d = self.depths[ci.depth_index as usize].scale(&scaler);
                        let i = self.italics[ci.italic_index as usize].scale(&scaler);
                        metrics[code as usize] = Some(GlyphMetrics::new(w, h, d, i));
                    }
                }
            }
        }

        ScaledTfm {
            header: self.header.clone(),
            bc: self.bc,
            ec: self.ec,
            dimensions,
            metrics,
        }
    }
}

/// A scaled TFM font ready for layout measurement and PDF output.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaledTfm {
    /// Font header.
    pub header: TfmHeader,
    /// First character code.
    pub bc: u8,
    /// Last character code.
    pub ec: u8,
    /// Calculated global font dimensions in scaled points (`sp`).
    pub dimensions: FontDimensions,
    /// Pre-scaled character metrics indexed by byte code.
    pub metrics: [Option<GlyphMetrics>; 256],
}

impl FontMetricsProvider for ScaledTfm {
    fn dimensions(&self) -> &FontDimensions {
        &self.dimensions
    }

    fn glyph_metrics(&self, char_code: u32) -> Option<GlyphMetrics> {
        if char_code < 256 {
            self.metrics[char_code as usize]
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_minimal_tfm() -> Vec<u8> {
        let lh = 2;
        let bc = 65; // 'A'
        let ec = 65;
        let nw = 2;
        let nh = 2;
        let nd = 2;
        let ni = 2;
        let nl = 1;
        let nk = 1;
        let ne = 1;
        let np = 7;
        let lf = 6 + lh + (ec - bc + 1) + nw + nh + nd + ni + nl + nk + ne + np;

        let mut data = vec![0u8; lf * 4];
        BigEndian::write_u16(&mut data[0..2], lf as u16);
        BigEndian::write_u16(&mut data[2..4], lh as u16);
        BigEndian::write_u16(&mut data[4..6], bc as u16);
        BigEndian::write_u16(&mut data[6..8], ec as u16);
        BigEndian::write_u16(&mut data[8..10], nw as u16);
        BigEndian::write_u16(&mut data[10..12], nh as u16);
        BigEndian::write_u16(&mut data[12..14], nd as u16);
        BigEndian::write_u16(&mut data[14..16], ni as u16);
        BigEndian::write_u16(&mut data[16..18], nl as u16);
        BigEndian::write_u16(&mut data[18..20], nk as u16);
        BigEndian::write_u16(&mut data[20..22], ne as u16);
        BigEndian::write_u16(&mut data[22..24], np as u16);

        // Header
        BigEndian::write_u32(&mut data[24..28], 0x12345678);
        BigEndian::write_u32(&mut data[28..32], 0x00A0_0000); // 10pt

        // Char info for 'A' (offset 32)
        // width=1, height=1, depth=1, italic=1, tag=Vanilla (0)
        data[32] = 1;
        data[33] = (1 << 4) | 1;
        data[34] = 1 << 2;
        data[35] = 0;

        // Width table (offset 36..44)
        BigEndian::write_i32(&mut data[36..40], 0);
        BigEndian::write_i32(&mut data[40..44], 1048576); // 1.0

        // Height table (offset 44..52)
        BigEndian::write_i32(&mut data[44..48], 0);
        BigEndian::write_i32(&mut data[48..52], 524288);

        // Depth table (offset 52..60)
        BigEndian::write_i32(&mut data[52..56], 0);
        BigEndian::write_i32(&mut data[56..60], 262144);

        // Italic table (offset 60..68)
        BigEndian::write_i32(&mut data[60..64], 0);
        BigEndian::write_i32(&mut data[64..68], 65536);

        // Lig/kern table (offset 68..72)
        data[68] = 128; // stop

        data
    }

    #[test]
    fn test_tfm_truncated() {
        assert_eq!(TfmFont::from_bytes(&[0; 10]), Err(TfmError::TruncatedHeader));

        // Declares 27 words but only provides 30 bytes (< 108 bytes)
        let mut short_data = build_minimal_tfm();
        short_data.truncate(30);
        assert_eq!(TfmFont::from_bytes(&short_data), Err(TfmError::TruncatedHeader));
    }

    #[test]
    fn test_tfm_valid_synthetic() {
        let data = build_minimal_tfm();
        let tfm = TfmFont::from_bytes(&data).expect("minimal TFM must parse");
        assert_eq!(tfm.bc, 65);
        assert_eq!(tfm.ec, 65);

        assert!(tfm.char_info(65).is_some());
        assert_eq!(tfm.char_info(64), None);
        assert_eq!(tfm.char_info(66), None);

        assert_eq!(tfm.char_width_fix(65), Some(FixWord(1048576)));
        assert_eq!(tfm.char_height_fix(65), Some(FixWord(524288)));
        assert_eq!(tfm.char_depth_fix(65), Some(FixWord(262144)));
        assert_eq!(tfm.char_italic_fix(65), Some(FixWord(65536)));

        // Scale at design size (0 means design size)
        let scaled_default = tfm.scale(0);
        assert_eq!(scaled_default.dimensions.scaled_size_sp, 655360);

        let gm = scaled_default.glyph_metrics(65).expect("glyph 'A'");
        assert_eq!(gm.width_sp, 655360);
        assert_eq!(scaled_default.glyph_metrics(500), None);
    }

    #[test]
    fn test_tfm_empty_font_range() {
        let mut data = build_minimal_tfm();
        // TeX convention for empty font range: bc = 256, ec = 255
        BigEndian::write_u16(&mut data[4..6], 256);
        BigEndian::write_u16(&mut data[6..8], 255);
        // Recalculate lf: num_chars = 0
        let lf = 6 + 2 + 0 + 2 + 2 + 2 + 2 + 1 + 1 + 1 + 7;
        BigEndian::write_u16(&mut data[0..2], lf as u16);

        // Remove char_info word
        let mut adjusted = data[0..32].to_vec();
        adjusted.extend_from_slice(&data[36..]);
        adjusted.truncate(lf * 4);
        let tfm = TfmFont::from_bytes(&adjusted).expect("empty range font");
        assert_eq!(tfm.bc, 1);
        assert_eq!(tfm.ec, 0);
    }

    #[test]
    fn test_tfm_invalid_table_lengths() {
        let mut data = build_minimal_tfm();
        BigEndian::write_u16(&mut data[8..10], 0); // nw = 0
        // Update lf so it passes FileLengthMismatch and reaches nw == 0 check
        let lf = BigEndian::read_u16(&data[0..2]) - 2; // nw decreased by 2
        BigEndian::write_u16(&mut data[0..2], lf);
        data.truncate(lf as usize * 4);
        assert_eq!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidTableLength {
                name: "nw/nh/nd/ni",
                len: 0,
            })
        );
    }

    #[test]
    fn test_tfm_cyclic_list_tag() {
        let mut data = build_minimal_tfm();
        // Set char 'A' (65) to have CharTag::List (2) pointing to itself (remainder = 65)
        data[34] = (1 << 2) | 2; // tag = List
        data[35] = 65; // remainder = 65 ('A')
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("cyclic")
        ));
    }

    #[test]
    fn test_tfm_empty_char_in_scale() {
        let mut data = build_minimal_tfm();
        // Set width_index = 0 for char 'A', so ci.exists() == false
        data[32] = 0;
        let tfm = TfmFont::from_bytes(&data).expect("tfm with unallocated char");
        let scaled = tfm.scale(655360);
        assert_eq!(scaled.glyph_metrics(65), None);
    }

    #[test]
    fn test_tfm_invalid_char_info_indices() {
        // Invalid width index
        let mut data = build_minimal_tfm();
        data[32] = 5; // nw is 2, index 5 is out of bounds
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("width")
        ));

        // Invalid height index
        let mut data = build_minimal_tfm();
        data[33] = (5 << 4) | 1; // nh is 2, height 5 is out of bounds
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("height")
        ));

        // Invalid depth index
        let mut data = build_minimal_tfm();
        data[33] = (1 << 4) | 5; // nd is 2, depth 5 is out of bounds
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("depth")
        ));

        // Invalid italic index
        let mut data = build_minimal_tfm();
        data[34] = 5 << 2; // ni is 2, italic 5 is out of bounds
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("italic")
        ));

        // Invalid lig_kern remainder
        let mut data = build_minimal_tfm();
        data[34] = (1 << 2) | 1; // tag = LigKern
        data[35] = 10; // nl is 1, remainder 10 is out of bounds
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("lig_kern")
        ));

        // Invalid extensible remainder
        let mut data = build_minimal_tfm();
        data[34] = (1 << 2) | 3; // tag = Extensible
        data[35] = 10; // ne is 1, remainder 10 is out of bounds
        assert!(matches!(
            TfmFont::from_bytes(&data),
            Err(TfmError::InvalidCharInfo { reason, .. }) if reason.contains("extensible")
        ));
    }

    #[test]
    fn test_tfm_nonzero_zero_index() {
        let mut data = build_minimal_tfm();
        BigEndian::write_i32(&mut data[36..40], 1234); // width[0] != 0
        assert_eq!(
            TfmFont::from_bytes(&data),
            Err(TfmError::NonZeroZeroIndex { table: "width" })
        );

        let mut data = build_minimal_tfm();
        BigEndian::write_i32(&mut data[44..48], 1234); // height[0] != 0
        assert_eq!(
            TfmFont::from_bytes(&data),
            Err(TfmError::NonZeroZeroIndex { table: "height" })
        );

        let mut data = build_minimal_tfm();
        BigEndian::write_i32(&mut data[52..56], 1234); // depth[0] != 0
        assert_eq!(
            TfmFont::from_bytes(&data),
            Err(TfmError::NonZeroZeroIndex { table: "depth" })
        );

        let mut data = build_minimal_tfm();
        BigEndian::write_i32(&mut data[60..64], 1234); // italic[0] != 0
        assert_eq!(
            TfmFont::from_bytes(&data),
            Err(TfmError::NonZeroZeroIndex { table: "italic" })
        );
    }
}
