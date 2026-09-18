// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Tier A XDV Bitwise & Opcode Comparator for differential testing.
//!
//! Provides strict 1:1 bitwise verification, fine-grained opcode event trace
//! extraction, and structured parameter divergence reporting.

use byteorder::{BigEndian, ByteOrder};
use std::fmt::{self, Display, Formatter};
use std::io::{self, Read};
use tectonic_xdv::{FileType, XdvError, XdvEvents, XdvParser};
use thiserror::Error;

/// 2D coordinate in TeX scaled points (1 pt = 65,536 sp).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Coordinate {
    pub x: i32, // h in TeX
    pub y: i32, // v in TeX
}

impl Coordinate {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Converts scaled points to floating-point printer points (1 pt = 65536 sp).
    pub fn to_pt(self) -> (f64, f64) {
        (self.x as f64 / 65536.0, self.y as f64 / 65536.0)
    }
}

impl Display for Coordinate {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let (pt_x, pt_y) = self.to_pt();
        write!(
            f,
            "({}, {}) sp [{:.2} pt, {:.2} pt]",
            self.x, self.y, pt_x, pt_y
        )
    }
}

/// A structured opcode event emitted during XDV stream execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum XdvOpcodeEvent {
    Preamble {
        file_type: String,
        comment: Vec<u8>,
    },
    DefineFont {
        font_num: i32,
        checksum: u32,
        scale_factor: u32,
        design_size: u32,
        name: String,
    },
    DefineNativeFont {
        font_num: i32,
        name: String,
        size: i32,
        face_index: u32,
        color_rgba: Option<u32>,
        extend: Option<u32>,
        slant: Option<u32>,
        embolden: Option<u32>,
    },
    BeginPage {
        page_num: i32,
        counters: [i32; 10],
    },
    EndPage {
        page_num: i32,
    },
    SetFont {
        font_num: i32,
    },
    SetChar {
        char_code: i32,
        font_num: i32,
        coord: Coordinate,
    },
    PutGlyph {
        glyph_id: u16,
        font_num: i32,
        coord: Coordinate,
        width: i32,
    },
    SetRule {
        height: i32,
        width: i32,
        coord: Coordinate,
    },
    PutRule {
        height: i32,
        width: i32,
        coord: Coordinate,
    },
    MoveRight {
        delta: i32,
        new_coord: Coordinate,
    },
    MoveDown {
        delta: i32,
        new_coord: Coordinate,
    },
    Push {
        depth: usize,
        coord: Coordinate,
    },
    Pop {
        depth: usize,
        coord: Coordinate,
    },
    Special {
        contents: Vec<u8>,
        coord: Coordinate,
    },
    Postamble,
    DoublePostamble,
}

impl XdvOpcodeEvent {
    pub fn opcode_name(&self) -> &'static str {
        match self {
            XdvOpcodeEvent::Preamble { .. } => "preamble",
            XdvOpcodeEvent::DefineFont { .. } => "define_font",
            XdvOpcodeEvent::DefineNativeFont { .. } => "define_native_font",
            XdvOpcodeEvent::BeginPage { .. } => "begin_page",
            XdvOpcodeEvent::EndPage { .. } => "end_page",
            XdvOpcodeEvent::SetFont { .. } => "set_font",
            XdvOpcodeEvent::SetChar { .. } => "set_char",
            XdvOpcodeEvent::PutGlyph { .. } => "put_glyph",
            XdvOpcodeEvent::SetRule { .. } => "set_rule",
            XdvOpcodeEvent::PutRule { .. } => "put_rule",
            XdvOpcodeEvent::MoveRight { .. } => "move_right",
            XdvOpcodeEvent::MoveDown { .. } => "move_down",
            XdvOpcodeEvent::Push { .. } => "push",
            XdvOpcodeEvent::Pop { .. } => "pop",
            XdvOpcodeEvent::Special { .. } => "special",
            XdvOpcodeEvent::Postamble => "postamble",
            XdvOpcodeEvent::DoublePostamble => "double_postamble",
        }
    }

    pub fn coordinate(&self) -> Coordinate {
        match self {
            XdvOpcodeEvent::SetChar { coord, .. }
            | XdvOpcodeEvent::PutGlyph { coord, .. }
            | XdvOpcodeEvent::SetRule { coord, .. }
            | XdvOpcodeEvent::PutRule { coord, .. }
            | XdvOpcodeEvent::MoveRight { new_coord: coord, .. }
            | XdvOpcodeEvent::MoveDown { new_coord: coord, .. }
            | XdvOpcodeEvent::Push { coord, .. }
            | XdvOpcodeEvent::Pop { coord, .. }
            | XdvOpcodeEvent::Special { coord, .. } => *coord,
            _ => Coordinate::default(),
        }
    }
}

/// Detailed divergence categorization between two XDV opcode traces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivergenceKind {
    OpcodeMismatch {
        expected: String,
        actual: String,
    },
    GlyphPlacementMismatch {
        glyph_id: u16,
        font_num: i32,
        expected_coord: Coordinate,
        actual_coord: Coordinate,
        delta_x: i32,
        delta_y: i32,
    },
    FontDefinitionMismatch {
        font_num: i32,
        expected_name: String,
        actual_name: String,
        expected_size: i32,
        actual_size: i32,
    },
    CharMismatch {
        font_num: i32,
        expected_char: i32,
        actual_char: i32,
        coord: Coordinate,
    },
    RuleMismatch {
        expected_height: i32,
        expected_width: i32,
        actual_height: i32,
        actual_width: i32,
        coord: Coordinate,
    },
    SpecialMismatch {
        expected: String,
        actual: String,
        coord: Coordinate,
    },
    StackDepthMismatch {
        expected_depth: usize,
        actual_depth: usize,
    },
    MovementMismatch {
        direction: &'static str,
        expected_delta: i32,
        actual_delta: i32,
        expected_coord: Coordinate,
        actual_coord: Coordinate,
    },
    UnexpectedEndOfStream {
        expected_remaining: usize,
        next_expected: String,
    },
    TrailingOpcodes {
        actual_remaining: usize,
        next_actual: String,
    },
    Bitwise(BitwiseDivergence),
}

impl Display for DivergenceKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            DivergenceKind::OpcodeMismatch { expected, actual } => {
                write!(f, "Opcode mismatch: expected `{expected}`, found `{actual}`")
            }
            DivergenceKind::GlyphPlacementMismatch {
                glyph_id,
                font_num,
                expected_coord,
                actual_coord,
                delta_x,
                delta_y,
            } => {
                write!(
                    f,
                    "Glyph {glyph_id} (font {font_num}) placement mismatch: expected {expected_coord}, actual {actual_coord} (delta dx={delta_x} sp, dy={delta_y} sp)"
                )
            }
            DivergenceKind::FontDefinitionMismatch {
                font_num,
                expected_name,
                actual_name,
                expected_size,
                actual_size,
            } => {
                write!(
                    f,
                    "Font {font_num} definition mismatch: expected `{expected_name}` (size {expected_size} sp), actual `{actual_name}` (size {actual_size} sp)"
                )
            }
            DivergenceKind::CharMismatch {
                font_num,
                expected_char,
                actual_char,
                coord,
            } => {
                write!(
                    f,
                    "Character mismatch in font {font_num} at {coord}: expected {expected_char} ({:?}), actual {actual_char} ({:?})",
                    *expected_char as u8 as char, *actual_char as u8 as char
                )
            }
            DivergenceKind::RuleMismatch {
                expected_height,
                expected_width,
                actual_height,
                actual_width,
                coord,
            } => {
                write!(
                    f,
                    "Rule dimensions mismatch at {coord}: expected (w={expected_width}, h={expected_height}), actual (w={actual_width}, h={actual_height})"
                )
            }
            DivergenceKind::SpecialMismatch {
                expected,
                actual,
                coord,
            } => {
                write!(
                    f,
                    "\\special content mismatch at {coord}: expected `{expected}`, actual `{actual}`"
                )
            }
            DivergenceKind::StackDepthMismatch {
                expected_depth,
                actual_depth,
            } => {
                write!(
                    f,
                    "Stack depth mismatch: expected {expected_depth}, actual {actual_depth}"
                )
            }
            DivergenceKind::MovementMismatch {
                direction,
                expected_delta,
                actual_delta,
                expected_coord,
                actual_coord,
            } => {
                write!(
                    f,
                    "Movement ({direction}) mismatch: expected delta {expected_delta} -> {expected_coord}, actual delta {actual_delta} -> {actual_coord}"
                )
            }
            DivergenceKind::UnexpectedEndOfStream {
                expected_remaining,
                next_expected,
            } => {
                write!(
                    f,
                    "Actual stream ended prematurely ({expected_remaining} events remaining, next expected: `{next_expected}`)"
                )
            }
            DivergenceKind::TrailingOpcodes {
                actual_remaining,
                next_actual,
            } => {
                write!(
                    f,
                    "Actual stream contains {actual_remaining} extra events after expected stream completed (first extra: `{next_actual}`)"
                )
            }
            DivergenceKind::Bitwise(b) => write!(f, "{b}"),
        }
    }
}

/// Structured divergence report indicating exact opcode index, page number, and coordinate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XdvDivergence {
    pub opcode_index: usize,
    pub page_number: i32,
    pub coordinate: Coordinate,
    pub kind: DivergenceKind,
}

impl Display for XdvDivergence {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[XDV Divergence] Opcode #{} on Page {} at {}: {}",
            self.opcode_index, self.page_number, self.coordinate, self.kind
        )
    }
}

impl std::error::Error for XdvDivergence {}

/// Diagnostic report for bitwise byte mismatches with neighborhood hex dumps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitwiseDivergence {
    pub byte_offset: u64,
    pub expected_byte: u8,
    pub actual_byte: u8,
    pub expected_len: usize,
    pub actual_len: usize,
    pub hex_diff: String,
}

impl BitwiseDivergence {
    pub fn new(
        byte_offset: u64,
        expected_byte: u8,
        actual_byte: u8,
        expected: &[u8],
        actual: &[u8],
    ) -> Self {
        let offset = byte_offset as usize;
        let start = offset.saturating_sub(16);
        let end_exp = (offset + 16).min(expected.len());
        let end_act = (offset + 16).min(actual.len());

        let mut hex_diff = String::new();
        hex_diff.push_str(&format!(
            "  Expected [{:#06x}..{:#06x}]:\n    ",
            start, end_exp
        ));
        for i in start..end_exp {
            if i == offset {
                hex_diff.push_str(&format!("[{:02X}] ", expected[i]));
            } else {
                hex_diff.push_str(&format!("{:02X} ", expected[i]));
            }
        }
        hex_diff.push_str(&format!(
            "\n  Actual   [{:#06x}..{:#06x}]:\n    ",
            start, end_act
        ));
        for i in start..end_act {
            if i == offset {
                hex_diff.push_str(&format!("[{:02X}] ", actual[i]));
            } else {
                hex_diff.push_str(&format!("{:02X} ", actual[i]));
            }
        }

        Self {
            byte_offset,
            expected_byte,
            actual_byte,
            expected_len: expected.len(),
            actual_len: actual.len(),
            hex_diff,
        }
    }
}

impl Display for BitwiseDivergence {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Bitwise divergence at byte offset {:#08X} (decimal {}):\n  Expected: {:#02X} ({:?})\n  Actual:   {:#02X} ({:?})\n  Lengths:  expected={} B, actual={} B\n{}",
            self.byte_offset,
            self.byte_offset,
            self.expected_byte,
            self.expected_byte as char,
            self.actual_byte,
            self.actual_byte as char,
            self.expected_len,
            self.actual_len,
            self.hex_diff
        )
    }
}

impl std::error::Error for BitwiseDivergence {}

/// Statistics returned after a successful parity comparison.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub total_opcodes: usize,
    pub pages_checked: usize,
    pub fonts_checked: usize,
    pub glyphs_checked: usize,
    pub rules_checked: usize,
    pub specials_checked: usize,
    pub bitwise_identical: bool,
}

/// Errors occurring during XDV parsing or collection.
#[derive(Debug, Error)]
pub enum XdvParseError {
    #[error("XDV engine error: {0}")]
    Xdv(#[from] XdvError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Malformed opcode stream at offset {offset}: {msg}")]
    Malformed { offset: u64, msg: String },
}

/// Collector implementing `tectonic_xdv::XdvEvents` to validate standard XDV conformance.
#[derive(Default, Debug)]
pub struct XdvEventCollector {
    pub headers: Vec<(FileType, Vec<u8>)>,
    pub pages: Vec<(Vec<i32>, i32)>,
    pub native_fonts: Vec<NativeFontRecord>,
    pub char_runs: Vec<(i32, Vec<i32>)>,
    pub glyph_runs: Vec<GlyphRunRecord>,
    pub rules: Vec<RuleRecord>,
    pub specials: Vec<SpecialRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeFontRecord {
    pub name: String,
    pub font_num: i32,
    pub size: i32,
    pub face_index: u32,
    pub color_rgba: Option<u32>,
    pub extend: Option<u32>,
    pub slant: Option<u32>,
    pub embolden: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphRunRecord {
    pub font_num: i32,
    pub glyphs: Vec<u16>,
    pub x: Vec<i32>,
    pub y: Vec<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleRecord {
    pub x: i32,
    pub y: i32,
    pub height: i32,
    pub width: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecialRecord {
    pub x: i32,
    pub y: i32,
    pub contents: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("XDV parse error: {0}")]
    Xdv(#[from] XdvError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

impl XdvEvents for XdvEventCollector {
    type Error = CollectorError;

    fn handle_header(&mut self, filetype: FileType, comment: &[u8]) -> Result<(), Self::Error> {
        self.headers.push((filetype, comment.to_vec()));
        Ok(())
    }

    fn handle_begin_page(&mut self, counters: &[i32], previous_bop: i32) -> Result<(), Self::Error> {
        self.pages.push((counters.to_vec(), previous_bop));
        Ok(())
    }

    fn handle_define_native_font(
        &mut self,
        name: &str,
        font_num: i32,
        size: i32,
        face_index: u32,
        color_rgba: Option<u32>,
        extend: Option<u32>,
        slant: Option<u32>,
        embolden: Option<u32>,
    ) -> Result<(), Self::Error> {
        self.native_fonts.push(NativeFontRecord {
            name: name.to_string(),
            font_num,
            size,
            face_index,
            color_rgba,
            extend,
            slant,
            embolden,
        });
        Ok(())
    }

    fn handle_char_run(&mut self, font_num: i32, chars: &[i32]) -> Result<(), Self::Error> {
        self.char_runs.push((font_num, chars.to_vec()));
        Ok(())
    }

    fn handle_glyph_run(
        &mut self,
        font_num: i32,
        glyphs: &[u16],
        x: &[i32],
        y: &[i32],
    ) -> Result<(), Self::Error> {
        self.glyph_runs.push(GlyphRunRecord {
            font_num,
            glyphs: glyphs.to_vec(),
            x: x.to_vec(),
            y: y.to_vec(),
        });
        Ok(())
    }

    fn handle_rule(&mut self, x: i32, y: i32, height: i32, width: i32) -> Result<(), Self::Error> {
        self.rules.push(RuleRecord {
            x,
            y,
            height,
            width,
        });
        Ok(())
    }

    fn handle_special(&mut self, x: i32, y: i32, contents: &[u8]) -> Result<(), Self::Error> {
        self.specials.push(SpecialRecord {
            x,
            y,
            contents: contents.to_vec(),
        });
        Ok(())
    }
}

/// The main differential comparison engine for Tier A XDV testing.
pub struct XdvDiffEngine;

impl XdvDiffEngine {
    /// Strict 1:1 bitwise verification with detailed neighborhood hex diagnostics.
    pub fn compare_bitwise(expected: &[u8], actual: &[u8]) -> Result<(), BitwiseDivergence> {
        let min_len = std::cmp::min(expected.len(), actual.len());
        for i in 0..min_len {
            if expected[i] != actual[i] {
                return Err(BitwiseDivergence::new(
                    i as u64,
                    expected[i],
                    actual[i],
                    expected,
                    actual,
                ));
            }
        }

        if expected.len() != actual.len() {
            let offset = min_len as u64;
            let (exp_b, act_b) = if expected.len() > min_len {
                (expected[min_len], 0)
            } else {
                (0, actual[min_len])
            };
            return Err(BitwiseDivergence::new(
                offset, exp_b, act_b, expected, actual,
            ));
        }

        Ok(())
    }

    /// Parses an XDV byte slice into an opcode event trace.
    pub fn trace_opcodes(data: &[u8]) -> Result<Vec<XdvOpcodeEvent>, XdvParseError> {
        let mut parser = OpcodeParser::new(data);
        parser.parse_all()
    }

    /// Verifies standard XDV parsing using `tectonic_xdv::XdvParser`.
    pub fn parse_with_tectonic_xdv<R: Read>(
        stream: R,
    ) -> Result<XdvEventCollector, CollectorError> {
        let (collector, _) = XdvParser::process(stream, XdvEventCollector::default())?;
        Ok(collector)
    }

    /// Compares two opcode event traces opcode-by-opcode with parameter divergence reporting.
    pub fn compare_opcode_traces(
        expected: &[XdvOpcodeEvent],
        actual: &[XdvOpcodeEvent],
    ) -> Result<DiffStats, XdvDivergence> {
        let mut stats = DiffStats::default();
        let mut current_page = 0;

        let min_len = std::cmp::min(expected.len(), actual.len());
        for idx in 0..min_len {
            let exp = &expected[idx];
            let act = &actual[idx];

            if let XdvOpcodeEvent::BeginPage { page_num, .. } = exp {
                current_page = *page_num;
                stats.pages_checked += 1;
            }

            stats.total_opcodes += 1;

            match (exp, act) {
                (
                    XdvOpcodeEvent::Preamble { file_type: e_ft, comment: e_c },
                    XdvOpcodeEvent::Preamble { file_type: a_ft, comment: a_c },
                ) => {
                    if e_ft != a_ft || e_c != a_c {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: Coordinate::default(),
                            kind: DivergenceKind::OpcodeMismatch {
                                expected: format!("Preamble({e_ft}, comment={e_c:?})"),
                                actual: format!("Preamble({a_ft}, comment={a_c:?})"),
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::DefineNativeFont {
                        font_num: e_fn,
                        name: e_name,
                        size: e_sz,
                        face_index: e_fi,
                        color_rgba: e_col,
                        extend: e_ext,
                        slant: e_sl,
                        embolden: e_emb,
                    },
                    XdvOpcodeEvent::DefineNativeFont {
                        font_num: a_fn,
                        name: a_name,
                        size: a_sz,
                        face_index: a_fi,
                        color_rgba: a_col,
                        extend: a_ext,
                        slant: a_sl,
                        embolden: a_emb,
                    },
                ) => {
                    stats.fonts_checked += 1;
                    if e_fn != a_fn
                        || e_name != a_name
                        || e_sz != a_sz
                        || e_fi != a_fi
                        || e_col != a_col
                        || e_ext != a_ext
                        || e_sl != a_sl
                        || e_emb != a_emb
                    {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: Coordinate::default(),
                            kind: DivergenceKind::FontDefinitionMismatch {
                                font_num: *e_fn,
                                expected_name: e_name.clone(),
                                actual_name: a_name.clone(),
                                expected_size: *e_sz,
                                actual_size: *a_sz,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::DefineFont {
                        font_num: e_fn,
                        name: e_name,
                        scale_factor: e_sf,
                        design_size: e_ds,
                        checksum: e_cs,
                    },
                    XdvOpcodeEvent::DefineFont {
                        font_num: a_fn,
                        name: a_name,
                        scale_factor: a_sf,
                        design_size: a_ds,
                        checksum: a_cs,
                    },
                ) => {
                    stats.fonts_checked += 1;
                    if e_fn != a_fn
                        || e_name != a_name
                        || e_sf != a_sf
                        || e_ds != a_ds
                        || e_cs != a_cs
                    {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: Coordinate::default(),
                            kind: DivergenceKind::FontDefinitionMismatch {
                                font_num: *e_fn,
                                expected_name: e_name.clone(),
                                actual_name: a_name.clone(),
                                expected_size: *e_sf as i32,
                                actual_size: *a_sf as i32,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::PutGlyph {
                        glyph_id: e_g,
                        font_num: e_fn,
                        coord: e_coord,
                        width: e_w,
                    },
                    XdvOpcodeEvent::PutGlyph {
                        glyph_id: a_g,
                        font_num: a_fn,
                        coord: a_coord,
                        width: a_w,
                    },
                ) => {
                    stats.glyphs_checked += 1;
                    if e_g != a_g || e_fn != a_fn || e_coord != a_coord || e_w != a_w {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::GlyphPlacementMismatch {
                                glyph_id: *e_g,
                                font_num: *e_fn,
                                expected_coord: *e_coord,
                                actual_coord: *a_coord,
                                delta_x: a_coord.x - e_coord.x,
                                delta_y: a_coord.y - e_coord.y,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::SetChar {
                        char_code: e_c,
                        font_num: e_fn,
                        coord: e_coord,
                    },
                    XdvOpcodeEvent::SetChar {
                        char_code: a_c,
                        font_num: a_fn,
                        coord: a_coord,
                    },
                ) => {
                    if e_c != a_c || e_fn != a_fn || e_coord != a_coord {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::CharMismatch {
                                font_num: *e_fn,
                                expected_char: *e_c,
                                actual_char: *a_c,
                                coord: *e_coord,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::SetRule {
                        height: e_h,
                        width: e_w,
                        coord: e_coord,
                    },
                    XdvOpcodeEvent::SetRule {
                        height: a_h,
                        width: a_w,
                        coord: a_coord,
                    },
                )
                | (
                    XdvOpcodeEvent::PutRule {
                        height: e_h,
                        width: e_w,
                        coord: e_coord,
                    },
                    XdvOpcodeEvent::PutRule {
                        height: a_h,
                        width: a_w,
                        coord: a_coord,
                    },
                ) => {
                    stats.rules_checked += 1;
                    if e_h != a_h || e_w != a_w || e_coord != a_coord {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::RuleMismatch {
                                expected_height: *e_h,
                                expected_width: *e_w,
                                actual_height: *a_h,
                                actual_width: *a_w,
                                coord: *e_coord,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::MoveRight {
                        delta: e_d,
                        new_coord: e_coord,
                    },
                    XdvOpcodeEvent::MoveRight {
                        delta: a_d,
                        new_coord: a_coord,
                    },
                ) => {
                    if e_d != a_d || e_coord != a_coord {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::MovementMismatch {
                                direction: "right",
                                expected_delta: *e_d,
                                actual_delta: *a_d,
                                expected_coord: *e_coord,
                                actual_coord: *a_coord,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::MoveDown {
                        delta: e_d,
                        new_coord: e_coord,
                    },
                    XdvOpcodeEvent::MoveDown {
                        delta: a_d,
                        new_coord: a_coord,
                    },
                ) => {
                    if e_d != a_d || e_coord != a_coord {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::MovementMismatch {
                                direction: "down",
                                expected_delta: *e_d,
                                actual_delta: *a_d,
                                expected_coord: *e_coord,
                                actual_coord: *a_coord,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::Push {
                        depth: e_dp,
                        coord: e_coord,
                    },
                    XdvOpcodeEvent::Push {
                        depth: a_dp,
                        coord: a_coord,
                    },
                )
                | (
                    XdvOpcodeEvent::Pop {
                        depth: e_dp,
                        coord: e_coord,
                    },
                    XdvOpcodeEvent::Pop {
                        depth: a_dp,
                        coord: a_coord,
                    },
                ) => {
                    if e_dp != a_dp || e_coord != a_coord {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::StackDepthMismatch {
                                expected_depth: *e_dp,
                                actual_depth: *a_dp,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::Special {
                        contents: e_c,
                        coord: e_coord,
                    },
                    XdvOpcodeEvent::Special {
                        contents: a_c,
                        coord: a_coord,
                    },
                ) => {
                    stats.specials_checked += 1;
                    if e_c != a_c || e_coord != a_coord {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: *e_coord,
                            kind: DivergenceKind::SpecialMismatch {
                                expected: String::from_utf8_lossy(e_c).into_owned(),
                                actual: String::from_utf8_lossy(a_c).into_owned(),
                                coord: *e_coord,
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::BeginPage { page_num: e_pn, counters: e_cnt },
                    XdvOpcodeEvent::BeginPage { page_num: a_pn, counters: a_cnt },
                ) => {
                    if e_pn != a_pn || e_cnt != a_cnt {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: Coordinate::default(),
                            kind: DivergenceKind::OpcodeMismatch {
                                expected: format!("BeginPage(page={e_pn})"),
                                actual: format!("BeginPage(page={a_pn})"),
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::EndPage { page_num: e_pn },
                    XdvOpcodeEvent::EndPage { page_num: a_pn },
                ) => {
                    if e_pn != a_pn {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: Coordinate::default(),
                            kind: DivergenceKind::OpcodeMismatch {
                                expected: format!("EndPage(page={e_pn})"),
                                actual: format!("EndPage(page={a_pn})"),
                            },
                        });
                    }
                }
                (
                    XdvOpcodeEvent::SetFont { font_num: e_fn },
                    XdvOpcodeEvent::SetFont { font_num: a_fn },
                ) => {
                    if e_fn != a_fn {
                        return Err(XdvDivergence {
                            opcode_index: idx,
                            page_number: current_page,
                            coordinate: Coordinate::default(),
                            kind: DivergenceKind::OpcodeMismatch {
                                expected: format!("SetFont({e_fn})"),
                                actual: format!("SetFont({a_fn})"),
                            },
                        });
                    }
                }
                (XdvOpcodeEvent::Postamble, XdvOpcodeEvent::Postamble)
                | (XdvOpcodeEvent::DoublePostamble, XdvOpcodeEvent::DoublePostamble) => {}
                _ => {
                    return Err(XdvDivergence {
                        opcode_index: idx,
                        page_number: current_page,
                        coordinate: exp.coordinate(),
                        kind: DivergenceKind::OpcodeMismatch {
                            expected: exp.opcode_name().to_string(),
                            actual: act.opcode_name().to_string(),
                        },
                    });
                }
            }
        }

        if expected.len() > actual.len() {
            return Err(XdvDivergence {
                opcode_index: min_len,
                page_number: current_page,
                coordinate: expected[min_len].coordinate(),
                kind: DivergenceKind::UnexpectedEndOfStream {
                    expected_remaining: expected.len() - actual.len(),
                    next_expected: expected[min_len].opcode_name().to_string(),
                },
            });
        }

        if actual.len() > expected.len() {
            return Err(XdvDivergence {
                opcode_index: min_len,
                page_number: current_page,
                coordinate: actual[min_len].coordinate(),
                kind: DivergenceKind::TrailingOpcodes {
                    actual_remaining: actual.len() - expected.len(),
                    next_actual: actual[min_len].opcode_name().to_string(),
                },
            });
        }

        Ok(stats)
    }

    /// Complete differential comparison: runs bitwise, opcode traces, and tectonic_xdv validation.
    pub fn compare_bytes(expected: &[u8], actual: &[u8]) -> Result<DiffStats, XdvDivergence> {
        let bitwise_result = Self::compare_bitwise(expected, actual);

        // Parse both through tectonic_xdv::XdvParser to ensure conforming structure
        if let Err(e) = Self::parse_with_tectonic_xdv(expected) {
            return Err(XdvDivergence {
                opcode_index: 0,
                page_number: 0,
                coordinate: Coordinate::default(),
                kind: DivergenceKind::OpcodeMismatch {
                    expected: "Valid tectonic_xdv parse".into(),
                    actual: format!("Expected stream parse failure: {e}"),
                },
            });
        }

        if let Err(e) = Self::parse_with_tectonic_xdv(actual) {
            return Err(XdvDivergence {
                opcode_index: 0,
                page_number: 0,
                coordinate: Coordinate::default(),
                kind: DivergenceKind::OpcodeMismatch {
                    expected: "Valid tectonic_xdv parse".into(),
                    actual: format!("Actual stream parse failure: {e}"),
                },
            });
        }

        let exp_trace = Self::trace_opcodes(expected).map_err(|e| XdvDivergence {
            opcode_index: 0,
            page_number: 0,
            coordinate: Coordinate::default(),
            kind: DivergenceKind::OpcodeMismatch {
                expected: "Valid opcode trace".into(),
                actual: format!("Failed to parse expected stream: {e}"),
            },
        })?;

        let act_trace = Self::trace_opcodes(actual).map_err(|e| XdvDivergence {
            opcode_index: 0,
            page_number: 0,
            coordinate: Coordinate::default(),
            kind: DivergenceKind::OpcodeMismatch {
                expected: "Valid opcode trace".into(),
                actual: format!("Failed to parse actual stream: {e}"),
            },
        })?;

        let mut stats = Self::compare_opcode_traces(&exp_trace, &act_trace)?;

        match bitwise_result {
            Ok(()) => {
                stats.bitwise_identical = true;
                Ok(stats)
            }
            Err(b) => {
                // If opcodes match perfectly but bitwise diverges (e.g. non-semantic padding),
                // we report the bitwise divergence as Tier A failure.
                Err(XdvDivergence {
                    opcode_index: stats.total_opcodes,
                    page_number: stats.pages_checked as i32,
                    coordinate: Coordinate::default(),
                    kind: DivergenceKind::Bitwise(b),
                })
            }
        }
    }

    /// Stream comparison entry point.
    pub fn compare<R1: Read, R2: Read>(
        mut expected: R1,
        mut actual: R2,
    ) -> Result<DiffStats, XdvDivergence> {
        let mut exp_buf = Vec::new();
        let mut act_buf = Vec::new();

        expected.read_to_end(&mut exp_buf).map_err(|e| XdvDivergence {
            opcode_index: 0,
            page_number: 0,
            coordinate: Coordinate::default(),
            kind: DivergenceKind::OpcodeMismatch {
                expected: "Readable stream".into(),
                actual: format!("I/O read error on expected: {e}"),
            },
        })?;

        actual.read_to_end(&mut act_buf).map_err(|e| XdvDivergence {
            opcode_index: 0,
            page_number: 0,
            coordinate: Coordinate::default(),
            kind: DivergenceKind::OpcodeMismatch {
                expected: "Readable stream".into(),
                actual: format!("I/O read error on actual: {e}"),
            },
        })?;

        Self::compare_bytes(&exp_buf, &act_buf)
    }
}

// ---------------------------------------------------------------------------
// Internal Opcode Parser
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct TeXState {
    h: i32,
    v: i32,
    w: i32,
    x: i32,
    y: i32,
    z: i32,
}

struct OpcodeParser<'a> {
    buf: &'a [u8],
    offset: usize,
    stack: Vec<TeXState>,
    cur_font: i32,
    cur_page: i32,
}

impl<'a> OpcodeParser<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self {
            buf,
            offset: 0,
            stack: vec![TeXState::default()],
            cur_font: 0,
            cur_page: 0,
        }
    }

    fn state(&mut self) -> &mut TeXState {
        self.stack.last_mut().expect("Stack underflow in TeX state")
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.offset)
    }

    fn read_u8(&mut self) -> Result<u8, XdvParseError> {
        if self.offset >= self.buf.len() {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let b = self.buf[self.offset];
        self.offset += 1;
        Ok(b)
    }

    fn read_i8(&mut self) -> Result<i8, XdvParseError> {
        self.read_u8().map(|b| b as i8)
    }

    fn read_u16(&mut self) -> Result<u16, XdvParseError> {
        if self.remaining() < 2 {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let v = BigEndian::read_u16(&self.buf[self.offset..self.offset + 2]);
        self.offset += 2;
        Ok(v)
    }

    fn read_i16(&mut self) -> Result<i16, XdvParseError> {
        if self.remaining() < 2 {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let v = BigEndian::read_i16(&self.buf[self.offset..self.offset + 2]);
        self.offset += 2;
        Ok(v)
    }

    fn read_u24(&mut self) -> Result<u32, XdvParseError> {
        if self.remaining() < 3 {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let v = BigEndian::read_u24(&self.buf[self.offset..self.offset + 3]);
        self.offset += 3;
        Ok(v)
    }

    fn read_i24(&mut self) -> Result<i32, XdvParseError> {
        if self.remaining() < 3 {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let v = BigEndian::read_i24(&self.buf[self.offset..self.offset + 3]);
        self.offset += 3;
        Ok(v)
    }

    fn read_u32(&mut self) -> Result<u32, XdvParseError> {
        if self.remaining() < 4 {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let v = BigEndian::read_u32(&self.buf[self.offset..self.offset + 4]);
        self.offset += 4;
        Ok(v)
    }

    fn read_i32(&mut self) -> Result<i32, XdvParseError> {
        if self.remaining() < 4 {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let v = BigEndian::read_i32(&self.buf[self.offset..self.offset + 4]);
        self.offset += 4;
        Ok(v)
    }

    fn read_compact_signed(&mut self, size: u8) -> Result<i32, XdvParseError> {
        match size {
            0 => Ok(self.read_i8()? as i32),
            1 => Ok(self.read_i16()? as i32),
            2 => Ok(self.read_i24()?),
            3 => self.read_i32(),
            _ => Err(XdvParseError::Malformed {
                offset: self.offset as u64,
                msg: "Invalid compact signed size".into(),
            }),
        }
    }

    fn read_compact_pos(&mut self, size: u8) -> Result<i32, XdvParseError> {
        match size {
            0 => Ok(self.read_u8()? as i32),
            1 => Ok(self.read_u16()? as i32),
            2 => Ok(self.read_u24()? as i32),
            3 => self.read_i32(),
            _ => Err(XdvParseError::Malformed {
                offset: self.offset as u64,
                msg: "Invalid compact pos size".into(),
            }),
        }
    }

    fn read_slice(&mut self, len: usize) -> Result<&'a [u8], XdvParseError> {
        if self.remaining() < len {
            return Err(XdvParseError::Xdv(XdvError::UnexpectedEndOfStream));
        }
        let s = &self.buf[self.offset..self.offset + len];
        self.offset += len;
        Ok(s)
    }

    fn parse_all(&mut self) -> Result<Vec<XdvOpcodeEvent>, XdvParseError> {
        let mut events = Vec::new();

        while self.remaining() > 0 {
            let opcode = self.read_u8()?;

            match opcode {
                // set_char_0 .. set_char_127
                0..=127 => {
                    let st = self.state();
                    let coord = Coordinate::new(st.h, st.v);
                    events.push(XdvOpcodeEvent::SetChar {
                        char_code: opcode as i32,
                        font_num: self.cur_font,
                        coord,
                    });
                }
                // set1 .. set4
                128..=131 => {
                    let char_code = self.read_compact_pos(opcode - 128)?;
                    let st = self.state();
                    let coord = Coordinate::new(st.h, st.v);
                    events.push(XdvOpcodeEvent::SetChar {
                        char_code,
                        font_num: self.cur_font,
                        coord,
                    });
                }
                // set_rule
                132 => {
                    let height = self.read_i32()?;
                    let width = self.read_i32()?;
                    let st = self.state();
                    let coord = Coordinate::new(st.h, st.v);
                    st.h += width;
                    events.push(XdvOpcodeEvent::SetRule {
                        height,
                        width,
                        coord,
                    });
                }
                // put_rule
                137 => {
                    let height = self.read_i32()?;
                    let width = self.read_i32()?;
                    let st = self.state();
                    let coord = Coordinate::new(st.h, st.v);
                    events.push(XdvOpcodeEvent::PutRule {
                        height,
                        width,
                        coord,
                    });
                }
                // nop
                138 => {}
                // bop (beginning of page)
                139 => {
                    let mut counters = [0i32; 10];
                    for c in &mut counters {
                        *c = self.read_i32()?;
                    }
                    let _prev_bop = self.read_i32()?;
                    self.cur_page = counters[0];
                    self.stack.clear();
                    self.stack.push(TeXState::default());
                    self.cur_font = 0;
                    events.push(XdvOpcodeEvent::BeginPage {
                        page_num: self.cur_page,
                        counters,
                    });
                }
                // eop (end of page)
                140 => {
                    events.push(XdvOpcodeEvent::EndPage {
                        page_num: self.cur_page,
                    });
                }
                // push
                141 => {
                    let dup = self.state().clone();
                    self.stack.push(dup);
                    let (h, v) = {
                        let st = self.state();
                        (st.h, st.v)
                    };
                    events.push(XdvOpcodeEvent::Push {
                        depth: self.stack.len(),
                        coord: Coordinate::new(h, v),
                    });
                }
                // pop
                142 => {
                    if self.stack.len() > 1 {
                        self.stack.pop();
                    }
                    let (h, v) = {
                        let st = self.state();
                        (st.h, st.v)
                    };
                    events.push(XdvOpcodeEvent::Pop {
                        depth: self.stack.len(),
                        coord: Coordinate::new(h, v),
                    });
                }
                // right1 .. right4
                143..=146 => {
                    let delta = self.read_compact_signed(opcode - 143)?;
                    let st = self.state();
                    st.h += delta;
                    events.push(XdvOpcodeEvent::MoveRight {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // w0 (right by w)
                147 => {
                    let st = self.state();
                    let delta = st.w;
                    st.h += delta;
                    events.push(XdvOpcodeEvent::MoveRight {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // w1 .. w4
                148..=151 => {
                    let delta = self.read_compact_signed(opcode - 148)?;
                    let st = self.state();
                    st.w = delta;
                    st.h += delta;
                    events.push(XdvOpcodeEvent::MoveRight {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // x0 (right by x)
                152 => {
                    let st = self.state();
                    let delta = st.x;
                    st.h += delta;
                    events.push(XdvOpcodeEvent::MoveRight {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // x1 .. x4
                153..=156 => {
                    let delta = self.read_compact_signed(opcode - 153)?;
                    let st = self.state();
                    st.x = delta;
                    st.h += delta;
                    events.push(XdvOpcodeEvent::MoveRight {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // down1 .. down4
                157..=160 => {
                    let delta = self.read_compact_signed(opcode - 157)?;
                    let st = self.state();
                    st.v += delta;
                    events.push(XdvOpcodeEvent::MoveDown {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // y0 (down by y)
                161 => {
                    let st = self.state();
                    let delta = st.y;
                    st.v += delta;
                    events.push(XdvOpcodeEvent::MoveDown {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // y1 .. y4
                162..=165 => {
                    let delta = self.read_compact_signed(opcode - 162)?;
                    let st = self.state();
                    st.y = delta;
                    st.v += delta;
                    events.push(XdvOpcodeEvent::MoveDown {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // z0 (down by z)
                166 => {
                    let st = self.state();
                    let delta = st.z;
                    st.v += delta;
                    events.push(XdvOpcodeEvent::MoveDown {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // z1 .. z4
                167..=170 => {
                    let delta = self.read_compact_signed(opcode - 167)?;
                    let st = self.state();
                    st.z = delta;
                    st.v += delta;
                    events.push(XdvOpcodeEvent::MoveDown {
                        delta,
                        new_coord: Coordinate::new(st.h, st.v),
                    });
                }
                // fnt_num_0 .. fnt_num_63
                171..=234 => {
                    self.cur_font = (opcode - 171) as i32;
                    events.push(XdvOpcodeEvent::SetFont {
                        font_num: self.cur_font,
                    });
                }
                // fnt1 .. fnt4
                235..=238 => {
                    self.cur_font = self.read_compact_pos(opcode - 235)?;
                    events.push(XdvOpcodeEvent::SetFont {
                        font_num: self.cur_font,
                    });
                }
                // xxx1 .. xxx4 (special)
                239..=242 => {
                    let len = match opcode - 239 {
                        0 => self.read_u8()? as usize,
                        1 => self.read_u16()? as usize,
                        2 => self.read_u24()? as usize,
                        3 => self.read_u32()? as usize,
                        _ => unreachable!(),
                    };
                    let contents = self.read_slice(len)?.to_vec();
                    let st = self.state();
                    events.push(XdvOpcodeEvent::Special {
                        contents,
                        coord: Coordinate::new(st.h, st.v),
                    });
                }
                // fnt_def1 .. fnt_def4
                243..=246 => {
                    let font_num = self.read_compact_pos(opcode - 243)?;
                    let checksum = self.read_u32()?;
                    let scale_factor = self.read_u32()?;
                    let design_size = self.read_u32()?;
                    let area_len = self.read_u8()? as usize;
                    let name_len = self.read_u8()? as usize;
                    let _area = self.read_slice(area_len)?;
                    let name = String::from_utf8_lossy(self.read_slice(name_len)?).into_owned();
                    events.push(XdvOpcodeEvent::DefineFont {
                        font_num,
                        checksum,
                        scale_factor,
                        design_size,
                        name,
                    });
                }
                // pre (preamble)
                247 => {
                    let id_byte = self.read_u8()?;
                    let file_type = if id_byte == 7 {
                        "XDV".to_string()
                    } else if id_byte == 100 {
                        "SPX".to_string()
                    } else {
                        format!("Unknown({id_byte})")
                    };
                    let _num = self.read_u32()?;
                    let _den = self.read_u32()?;
                    let _mag = self.read_u32()?;
                    let comment_len = self.read_u8()? as usize;
                    let comment = self.read_slice(comment_len)?.to_vec();
                    events.push(XdvOpcodeEvent::Preamble { file_type, comment });
                }
                // post (postamble)
                248 => {
                    let _last_bop = self.read_u32()?;
                    let _num = self.read_u32()?;
                    let _den = self.read_u32()?;
                    let _mag = self.read_u32()?;
                    let _max_h = self.read_u32()?;
                    let _max_w = self.read_u32()?;
                    let _max_stack = self.read_u16()?;
                    let _pages = self.read_u16()?;
                    events.push(XdvOpcodeEvent::Postamble);
                }
                // post_post (double postamble)
                249 => {
                    let _post_ptr = self.read_u32()?;
                    let _id_byte = self.read_u8()?;
                    while self.remaining() > 0 && self.buf[self.offset] == 0xDF {
                        self.offset += 1;
                    }
                    events.push(XdvOpcodeEvent::DoublePostamble);
                    break;
                }
                // define_native_font
                252 => {
                    let font_num = self.read_i32()?;
                    let size = self.read_i32()?;
                    let flags = self.read_u16()?;
                    let name_len = self.read_u8()? as usize;
                    let name = String::from_utf8_lossy(self.read_slice(name_len)?).into_owned();
                    let face_index = self.read_u32()?;

                    let color_rgba = if flags & 0x0200 != 0 {
                        Some(self.read_u32()?)
                    } else {
                        None
                    };
                    let extend = if flags & 0x1000 != 0 {
                        Some(self.read_u32()?)
                    } else {
                        None
                    };
                    let slant = if flags & 0x2000 != 0 {
                        Some(self.read_u32()?)
                    } else {
                        None
                    };
                    let embolden = if flags & 0x4000 != 0 {
                        Some(self.read_u32()?)
                    } else {
                        None
                    };

                    events.push(XdvOpcodeEvent::DefineNativeFont {
                        font_num,
                        name,
                        size,
                        face_index,
                        color_rgba,
                        extend,
                        slant,
                        embolden,
                    });
                }
                // set_glyphs (XDV_GLYPHS)
                253 => {
                    let width = self.read_i32()?;
                    let n_glyphs = self.read_u16()? as usize;
                    let mut deltas = Vec::with_capacity(n_glyphs);
                    for _ in 0..n_glyphs {
                        let dx = self.read_i32()?;
                        let dy = self.read_i32()?;
                        deltas.push((dx, dy));
                    }
                    let mut glyph_ids = Vec::with_capacity(n_glyphs);
                    for _ in 0..n_glyphs {
                        glyph_ids.push(self.read_u16()?);
                    }

                    let cur_font = self.cur_font;
                    let st = self.state();
                    for i in 0..n_glyphs {
                        let abs_x = st.h + deltas[i].0;
                        let abs_y = st.v + deltas[i].1;
                        events.push(XdvOpcodeEvent::PutGlyph {
                            glyph_id: glyph_ids[i],
                            font_num: cur_font,
                            coord: Coordinate::new(abs_x, abs_y),
                            width,
                        });
                    }
                    st.h += width;
                }
                // set_text_and_glyphs (XDV_TEXT_AND_GLYPHS)
                254 => {
                    let n_chars = self.read_u16()? as usize;
                    for _ in 0..n_chars {
                        let _ch = self.read_u16()?;
                    }
                    let width = self.read_i32()?;
                    let n_glyphs = self.read_u16()? as usize;
                    let mut deltas = Vec::with_capacity(n_glyphs);
                    for _ in 0..n_glyphs {
                        let dx = self.read_i32()?;
                        let dy = self.read_i32()?;
                        deltas.push((dx, dy));
                    }
                    let mut glyph_ids = Vec::with_capacity(n_glyphs);
                    for _ in 0..n_glyphs {
                        glyph_ids.push(self.read_u16()?);
                    }

                    let cur_font = self.cur_font;
                    let st = self.state();
                    for i in 0..n_glyphs {
                        let abs_x = st.h + deltas[i].0;
                        let abs_y = st.v + deltas[i].1;
                        events.push(XdvOpcodeEvent::PutGlyph {
                            glyph_id: glyph_ids[i],
                            font_num: cur_font,
                            coord: Coordinate::new(abs_x, abs_y),
                            width,
                        });
                    }
                    st.h += width;
                }
                _ => {
                    return Err(XdvParseError::Xdv(XdvError::IllegalOpcode(
                        opcode,
                        self.offset as u64 - 1,
                    )));
                }
            }
        }

        Ok(events)
    }
}

// ---------------------------------------------------------------------------
// Synthetic XDV Stream Builder for Tests & Harness Fixtures
// ---------------------------------------------------------------------------

/// Helper for constructing spec-compliant XDV binary streams.
#[derive(Clone, Debug, Default)]
pub struct XdvBuilder {
    comment: Vec<u8>,
    native_fonts: Vec<NativeFontDef>,
    body_ops: Vec<u8>,
    bop_offsets: Vec<u32>,
    page_count: u16,
}

#[derive(Clone, Debug)]
struct NativeFontDef {
    font_num: i32,
    name: String,
    size: i32,
    flags: u16,
}

impl XdvBuilder {
    pub fn new() -> Self {
        Self {
            comment: b"tectonic".to_vec(),
            native_fonts: Vec::new(),
            body_ops: Vec::new(),
            bop_offsets: Vec::new(),
            page_count: 0,
        }
    }

    pub fn comment(mut self, comment: &[u8]) -> Self {
        self.comment = comment.to_vec();
        self
    }

    pub fn native_font(mut self, font_num: i32, name: &str, size: i32) -> Self {
        self.native_fonts.push(NativeFontDef {
            font_num,
            name: name.to_string(),
            size,
            flags: 0,
        });
        self
    }

    pub fn begin_page(mut self, page_num: i32) -> Self {
        let bop_offset = self.current_stream_length();
        self.bop_offsets.push(bop_offset);
        self.page_count += 1;

        // Opcode 139 (BOP)
        self.body_ops.push(139);
        let mut counters = [0i32; 10];
        counters[0] = page_num;
        for c in counters {
            let mut buf = [0u8; 4];
            BigEndian::write_i32(&mut buf, c);
            self.body_ops.extend_from_slice(&buf);
        }
        let prev_bop = if self.bop_offsets.len() > 1 {
            self.bop_offsets[self.bop_offsets.len() - 2] as i32
        } else {
            -1
        };
        let mut buf = [0u8; 4];
        BigEndian::write_i32(&mut buf, prev_bop);
        self.body_ops.extend_from_slice(&buf);
        self
    }

    pub fn set_font(mut self, font_num: i32) -> Self {
        if (0..64).contains(&font_num) {
            self.body_ops.push(171 + font_num as u8);
        } else {
            self.body_ops.push(235);
            self.body_ops.push(font_num as u8);
        }
        self
    }

    pub fn move_right(mut self, delta: i32) -> Self {
        if (-128..=127).contains(&delta) {
            self.body_ops.push(143);
            self.body_ops.push(delta as i8 as u8);
        } else {
            self.body_ops.push(146);
            let mut buf = [0u8; 4];
            BigEndian::write_i32(&mut buf, delta);
            self.body_ops.extend_from_slice(&buf);
        }
        self
    }

    pub fn move_down(mut self, delta: i32) -> Self {
        if (-128..=127).contains(&delta) {
            self.body_ops.push(157);
            self.body_ops.push(delta as i8 as u8);
        } else {
            self.body_ops.push(160);
            let mut buf = [0u8; 4];
            BigEndian::write_i32(&mut buf, delta);
            self.body_ops.extend_from_slice(&buf);
        }
        self
    }

    pub fn push_stack(mut self) -> Self {
        self.body_ops.push(141);
        self
    }

    pub fn pop_stack(mut self) -> Self {
        self.body_ops.push(142);
        self
    }

    pub fn glyph(mut self, glyph_id: u16, dx: i32, dy: i32, width: i32) -> Self {
        self.body_ops.push(253); // SetGlyphs
        let mut buf4 = [0u8; 4];
        let mut buf2 = [0u8; 2];

        BigEndian::write_i32(&mut buf4, width);
        self.body_ops.extend_from_slice(&buf4);

        BigEndian::write_u16(&mut buf2, 1); // n_glyphs = 1
        self.body_ops.extend_from_slice(&buf2);

        BigEndian::write_i32(&mut buf4, dx);
        self.body_ops.extend_from_slice(&buf4);
        BigEndian::write_i32(&mut buf4, dy);
        self.body_ops.extend_from_slice(&buf4);

        BigEndian::write_u16(&mut buf2, glyph_id);
        self.body_ops.extend_from_slice(&buf2);
        self
    }

    pub fn rule(mut self, height: i32, width: i32) -> Self {
        self.body_ops.push(132); // SetRule
        let mut buf = [0u8; 4];
        BigEndian::write_i32(&mut buf, height);
        self.body_ops.extend_from_slice(&buf);
        BigEndian::write_i32(&mut buf, width);
        self.body_ops.extend_from_slice(&buf);
        self
    }

    pub fn special(mut self, data: &[u8]) -> Self {
        assert!(data.len() <= 255);
        self.body_ops.push(239); // Special1
        self.body_ops.push(data.len() as u8);
        self.body_ops.extend_from_slice(data);
        self
    }

    pub fn end_page(mut self) -> Self {
        self.body_ops.push(140); // EOP
        self
    }

    fn current_stream_length(&self) -> u32 {
        let preamble_len = 1 + 1 + 4 + 4 + 4 + 1 + self.comment.len();
        let mut fonts_len = 0;
        for f in &self.native_fonts {
            fonts_len += 1 + 4 + 4 + 2 + 1 + f.name.len() + 4;
        }
        (preamble_len + fonts_len + self.body_ops.len()) as u32
    }

    pub fn build(self) -> Vec<u8> {
        let mut out = Vec::new();

        // 1. Preamble (opcode 247)
        out.push(247);
        out.push(7); // XDV ID
        let mut u32_buf = [0u8; 4];
        BigEndian::write_u32(&mut u32_buf, 25_400_000);
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 473_628_672);
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 1000); // mag
        out.extend_from_slice(&u32_buf);
        out.push(self.comment.len() as u8);
        out.extend_from_slice(&self.comment);

        // 2. Native Font Definitions (opcode 252)
        for f in &self.native_fonts {
            out.push(252);
            let mut i32_buf = [0u8; 4];
            let mut u16_buf = [0u8; 2];
            BigEndian::write_i32(&mut i32_buf, f.font_num);
            out.extend_from_slice(&i32_buf);
            BigEndian::write_i32(&mut i32_buf, f.size);
            out.extend_from_slice(&i32_buf);
            BigEndian::write_u16(&mut u16_buf, f.flags);
            out.extend_from_slice(&u16_buf);
            out.push(f.name.len() as u8);
            out.extend_from_slice(f.name.as_bytes());
            BigEndian::write_u32(&mut u32_buf, 0); // face_index
            out.extend_from_slice(&u32_buf);
        }

        // 3. Body Operations
        out.extend_from_slice(&self.body_ops);

        // 4. Postamble (opcode 248)
        let postamble_offset = out.len() as u32;
        let last_bop = self.bop_offsets.last().copied().unwrap_or(0);

        out.push(248);
        BigEndian::write_u32(&mut u32_buf, last_bop);
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 25_400_000);
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 473_628_672);
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 1000);
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 100_000); // max height
        out.extend_from_slice(&u32_buf);
        BigEndian::write_u32(&mut u32_buf, 100_000); // max width
        out.extend_from_slice(&u32_buf);

        let mut u16_buf = [0u8; 2];
        BigEndian::write_u16(&mut u16_buf, 5); // max stack depth
        out.extend_from_slice(&u16_buf);
        BigEndian::write_u16(&mut u16_buf, self.page_count); // total pages
        out.extend_from_slice(&u16_buf);

        // Repeat native font definitions in postamble for strict parsers
        for f in &self.native_fonts {
            out.push(252);
            let mut i32_buf = [0u8; 4];
            BigEndian::write_i32(&mut i32_buf, f.font_num);
            out.extend_from_slice(&i32_buf);
            BigEndian::write_i32(&mut i32_buf, f.size);
            out.extend_from_slice(&i32_buf);
            BigEndian::write_u16(&mut u16_buf, f.flags);
            out.extend_from_slice(&u16_buf);
            out.push(f.name.len() as u8);
            out.extend_from_slice(f.name.as_bytes());
            BigEndian::write_u32(&mut u32_buf, 0);
            out.extend_from_slice(&u32_buf);
        }

        // 5. Double Postamble (opcode 249)
        out.push(249);
        BigEndian::write_u32(&mut u32_buf, postamble_offset);
        out.extend_from_slice(&u32_buf);
        out.push(7); // XDV ID
        out.extend_from_slice(&[0xDF, 0xDF, 0xDF, 0xDF, 0xDF]); // Trailing 0xDF padding

        out
    }
}

// ---------------------------------------------------------------------------
// Comprehensive Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_xdv_streams_pass() {
        let xdv = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .push_stack()
            .move_right(100)
            .move_down(200)
            .glyph(65, 0, 0, 65536)
            .rule(65536, 131072)
            .special(b"color push rgb 1 0 0")
            .pop_stack()
            .end_page()
            .build();

        let stats = XdvDiffEngine::compare_bytes(&xdv, &xdv).expect("Identical streams must pass");
        assert!(stats.bitwise_identical);
        assert_eq!(stats.pages_checked, 1);
        assert_eq!(stats.fonts_checked, 2); // 1 in preamble, 1 in postamble
        assert_eq!(stats.glyphs_checked, 1);
        assert_eq!(stats.rules_checked, 1);
        assert_eq!(stats.specials_checked, 1);
    }

    #[test]
    fn test_divergence_glyph_placement_exact_coordinate_diff() {
        let base = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .move_right(100)
            .move_down(200)
            .glyph(65, 0, 0, 65536) // Placed at (100, 200)
            .end_page()
            .build();

        let shifted = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .move_right(150) // Shifted by +50 sp
            .move_down(200)
            .glyph(65, 0, 0, 65536) // Placed at (150, 200)
            .end_page()
            .build();

        let base_trace = XdvDiffEngine::trace_opcodes(&base).unwrap();
        let shifted_trace = XdvDiffEngine::trace_opcodes(&shifted).unwrap();

        let err = XdvDiffEngine::compare_opcode_traces(&base_trace, &shifted_trace)
            .expect_err("Divergent coordinates must fail");

        assert_eq!(err.page_number, 1);
        match err.kind {
            DivergenceKind::MovementMismatch {
                direction,
                expected_delta,
                actual_delta,
                expected_coord,
                actual_coord,
            } => {
                assert_eq!(direction, "right");
                assert_eq!(expected_delta, 100);
                assert_eq!(actual_delta, 150);
                assert_eq!(expected_coord, Coordinate::new(100, 0));
                assert_eq!(actual_coord, Coordinate::new(150, 0));
            }
            DivergenceKind::GlyphPlacementMismatch {
                glyph_id,
                expected_coord,
                actual_coord,
                delta_x,
                delta_y,
                ..
            } => {
                assert_eq!(glyph_id, 65);
                assert_eq!(expected_coord, Coordinate::new(100, 200));
                assert_eq!(actual_coord, Coordinate::new(150, 200));
                assert_eq!(delta_x, 50);
                assert_eq!(delta_y, 0);
            }
            other => panic!("Unexpected divergence kind: {other:?}"),
        }
    }

    #[test]
    fn test_divergence_glyph_placement_delta_in_set_glyphs() {
        let base = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .glyph(65, 10, 20, 65536) // (10, 20)
            .end_page()
            .build();

        let altered = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .glyph(65, 40, 20, 65536) // (40, 20), delta_x = 30
            .end_page()
            .build();

        let base_trace = XdvDiffEngine::trace_opcodes(&base).unwrap();
        let altered_trace = XdvDiffEngine::trace_opcodes(&altered).unwrap();

        let err = XdvDiffEngine::compare_opcode_traces(&base_trace, &altered_trace)
            .expect_err("Mismatched glyph delta must fail");

        assert_eq!(err.page_number, 1);
        match err.kind {
            DivergenceKind::GlyphPlacementMismatch {
                glyph_id,
                expected_coord,
                actual_coord,
                delta_x,
                delta_y,
                ..
            } => {
                assert_eq!(glyph_id, 65);
                assert_eq!(expected_coord, Coordinate::new(10, 20));
                assert_eq!(actual_coord, Coordinate::new(40, 20));
                assert_eq!(delta_x, 30);
                assert_eq!(delta_y, 0);
            }
            other => panic!("Expected GlyphPlacementMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn test_divergence_font_definitions_font_name_mismatch() {
        let base = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .glyph(65, 0, 0, 65536)
            .end_page()
            .build();

        let altered = XdvBuilder::new()
            .native_font(0, "cmbx10", 655360) // Font name mismatch
            .begin_page(1)
            .glyph(65, 0, 0, 65536)
            .end_page()
            .build();

        let base_trace = XdvDiffEngine::trace_opcodes(&base).unwrap();
        let altered_trace = XdvDiffEngine::trace_opcodes(&altered).unwrap();

        let err = XdvDiffEngine::compare_opcode_traces(&base_trace, &altered_trace)
            .expect_err("Font name divergence must fail");

        match err.kind {
            DivergenceKind::FontDefinitionMismatch {
                font_num,
                expected_name,
                actual_name,
                expected_size,
                actual_size,
            } => {
                assert_eq!(font_num, 0);
                assert_eq!(expected_name, "cmr10");
                assert_eq!(actual_name, "cmbx10");
                assert_eq!(expected_size, 655360);
                assert_eq!(actual_size, 655360);
            }
            other => panic!("Expected FontDefinitionMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn test_divergence_rule_mismatch() {
        let base = XdvBuilder::new()
            .begin_page(1)
            .rule(65536, 131072)
            .end_page()
            .build();

        let altered = XdvBuilder::new()
            .begin_page(1)
            .rule(65536, 262144) // Double width
            .end_page()
            .build();

        let base_trace = XdvDiffEngine::trace_opcodes(&base).unwrap();
        let altered_trace = XdvDiffEngine::trace_opcodes(&altered).unwrap();

        let err = XdvDiffEngine::compare_opcode_traces(&base_trace, &altered_trace)
            .expect_err("Rule divergence must fail");

        match err.kind {
            DivergenceKind::RuleMismatch {
                expected_width,
                actual_width,
                ..
            } => {
                assert_eq!(expected_width, 131072);
                assert_eq!(actual_width, 262144);
            }
            other => panic!("Expected RuleMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn test_divergence_special_mismatch() {
        let base = XdvBuilder::new()
            .begin_page(1)
            .special(b"color push rgb 1 0 0")
            .end_page()
            .build();

        let altered = XdvBuilder::new()
            .begin_page(1)
            .special(b"color push rgb 0 1 0")
            .end_page()
            .build();

        let base_trace = XdvDiffEngine::trace_opcodes(&base).unwrap();
        let altered_trace = XdvDiffEngine::trace_opcodes(&altered).unwrap();

        let err = XdvDiffEngine::compare_opcode_traces(&base_trace, &altered_trace)
            .expect_err("Special mismatch must fail");

        match err.kind {
            DivergenceKind::SpecialMismatch { expected, actual, .. } => {
                assert_eq!(expected, "color push rgb 1 0 0");
                assert_eq!(actual, "color push rgb 0 1 0");
            }
            other => panic!("Expected SpecialMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn test_divergence_unexpected_eof() {
        let base = XdvBuilder::new()
            .begin_page(1)
            .rule(10, 10)
            .rule(20, 20)
            .end_page()
            .build();

        let truncated = XdvBuilder::new()
            .begin_page(1)
            .rule(10, 10)
            .end_page()
            .build();

        let base_trace = XdvDiffEngine::trace_opcodes(&base).unwrap();
        let trunc_trace = XdvDiffEngine::trace_opcodes(&truncated).unwrap();

        let err = XdvDiffEngine::compare_opcode_traces(&base_trace, &trunc_trace)
            .expect_err("Truncated stream must fail");

        match err.kind {
            DivergenceKind::UnexpectedEndOfStream { .. }
            | DivergenceKind::OpcodeMismatch { .. } => {}
            other => panic!("Expected UnexpectedEndOfStream or OpcodeMismatch, got: {other:?}"),
        }
    }

    #[test]
    fn test_bitwise_divergence_diagnostics() {
        let buf1 = b"abcdefghijklmnopqrstuvwxyz0123456789";
        let mut buf2 = buf1.to_vec();
        buf2[10] = b'X'; // mutate 'k' to 'X'

        let err = XdvDiffEngine::compare_bitwise(buf1, &buf2)
            .expect_err("Bitwise divergence must be detected");

        assert_eq!(err.byte_offset, 10);
        assert_eq!(err.expected_byte, b'k');
        assert_eq!(err.actual_byte, b'X');
        assert!(err.hex_diff.contains("[6B]")); // 'k' = 0x6B
        assert!(err.hex_diff.contains("[58]")); // 'X' = 0x58
    }

    #[test]
    fn test_tectonic_xdv_parser_integration() {
        let xdv = XdvBuilder::new()
            .native_font(0, "cmr10", 655360)
            .begin_page(1)
            .glyph(65, 0, 0, 65536)
            .rule(100, 200)
            .special(b"test")
            .end_page()
            .build();

        let collector = XdvDiffEngine::parse_with_tectonic_xdv(&xdv[..])
            .expect("tectonic_xdv must parse generated stream cleanly");

        assert_eq!(collector.headers.len(), 1);
        assert_eq!(collector.headers[0].0, FileType::Xdv);
        assert_eq!(collector.pages.len(), 1);
        assert_eq!(collector.pages[0].0[0], 1); // Counter 0 = 1
        assert_eq!(collector.glyph_runs.len(), 1);
        assert_eq!(collector.glyph_runs[0].glyphs, vec![65]);
        assert_eq!(collector.rules.len(), 1);
        assert_eq!(collector.rules[0].height, 100);
        assert_eq!(collector.rules[0].width, 200);
        assert_eq!(collector.specials.len(), 1);
        assert_eq!(collector.specials[0].contents, b"test");
    }

    #[test]
    fn test_bitwise_divergence_length_mismatches() {
        let shorter = b"12345";
        let longer = b"1234567890";

        // 1. Expected longer than actual
        let err1 = XdvDiffEngine::compare_bitwise(longer, shorter).unwrap_err();
        assert_eq!(err1.byte_offset, 5);
        assert_eq!(err1.expected_len, 10);
        assert_eq!(err1.actual_len, 5);
        let msg1 = format!("{err1}");
        assert!(msg1.contains("Bitwise divergence at byte offset"));
        assert!(msg1.contains("expected=10 B, actual=5 B"));

        // 2. Actual longer than expected
        let err2 = XdvDiffEngine::compare_bitwise(shorter, longer).unwrap_err();
        assert_eq!(err2.byte_offset, 5);
        assert_eq!(err2.expected_len, 5);
        assert_eq!(err2.actual_len, 10);
        let msg2 = format!("{err2}");
        assert!(msg2.contains("expected=5 B, actual=10 B"));
    }

    #[test]
    fn test_xdv_opcode_event_names_and_coords() {
        let coord = Coordinate::new(100, 200);

        let events = vec![
            XdvOpcodeEvent::Preamble { file_type: "xdv".into(), comment: b"comment".to_vec() },
            XdvOpcodeEvent::DefineFont { font_num: 1, name: "cmr10".into(), scale_factor: 10, design_size: 10, checksum: 0 },
            XdvOpcodeEvent::DefineNativeFont { font_num: 2, name: "Arial".into(), size: 10, face_index: 0, color_rgba: None, extend: None, slant: None, embolden: None },
            XdvOpcodeEvent::BeginPage { page_num: 1, counters: [1, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
            XdvOpcodeEvent::EndPage { page_num: 1 },
            XdvOpcodeEvent::SetFont { font_num: 1 },
            XdvOpcodeEvent::SetChar { char_code: 65, font_num: 1, coord },
            XdvOpcodeEvent::PutGlyph { glyph_id: 65, font_num: 1, coord, width: 50 },
            XdvOpcodeEvent::SetRule { height: 10, width: 20, coord },
            XdvOpcodeEvent::PutRule { height: 10, width: 20, coord },
            XdvOpcodeEvent::MoveRight { delta: 5, new_coord: coord },
            XdvOpcodeEvent::MoveDown { delta: 10, new_coord: coord },
            XdvOpcodeEvent::Push { depth: 1, coord },
            XdvOpcodeEvent::Pop { depth: 0, coord },
            XdvOpcodeEvent::Special { contents: b"spec".to_vec(), coord },
            XdvOpcodeEvent::Postamble,
            XdvOpcodeEvent::DoublePostamble,
        ];

        let expected_names = [
            "preamble", "define_font", "define_native_font", "begin_page", "end_page",
            "set_font", "set_char", "put_glyph", "set_rule", "put_rule",
            "move_right", "move_down", "push", "pop", "special", "postamble", "double_postamble"
        ];

        for (ev, &name) in events.iter().zip(expected_names.iter()) {
            assert_eq!(ev.opcode_name(), name);
        }

        // Coordinate checks
        assert_eq!(events[0].coordinate(), Coordinate::default());
        assert_eq!(events[6].coordinate(), coord);
        assert_eq!(events[7].coordinate(), coord);
        assert_eq!(events[8].coordinate(), coord);
        assert_eq!(events[9].coordinate(), coord);
        assert_eq!(events[10].coordinate(), coord);
        assert_eq!(events[11].coordinate(), coord);
        assert_eq!(events[12].coordinate(), coord);
        assert_eq!(events[13].coordinate(), coord);
        assert_eq!(events[14].coordinate(), coord);
    }

    #[test]
    fn test_divergence_kind_display_all_variants() {
        let coord = Coordinate::new(10, 20);

        let kinds = vec![
            DivergenceKind::OpcodeMismatch { expected: "push".into(), actual: "pop".into() },
            DivergenceKind::GlyphPlacementMismatch {
                glyph_id: 42,
                font_num: 1,
                expected_coord: coord,
                actual_coord: Coordinate::new(15, 25),
                delta_x: 5,
                delta_y: 5,
            },
            DivergenceKind::FontDefinitionMismatch {
                font_num: 2,
                expected_name: "cmr10".into(),
                actual_name: "cmbx10".into(),
                expected_size: 655360,
                actual_size: 700000,
            },
            DivergenceKind::CharMismatch {
                font_num: 1,
                expected_char: 65,
                actual_char: 66,
                coord,
            },
            DivergenceKind::RuleMismatch {
                expected_height: 100,
                expected_width: 200,
                actual_height: 100,
                actual_width: 250,
                coord,
            },
            DivergenceKind::SpecialMismatch {
                expected: "color push rgb 1 0 0".into(),
                actual: "color push rgb 0 1 0".into(),
                coord,
            },
            DivergenceKind::StackDepthMismatch { expected_depth: 2, actual_depth: 1 },
            DivergenceKind::MovementMismatch {
                direction: "right",
                expected_delta: 10,
                actual_delta: 20,
                expected_coord: coord,
                actual_coord: Coordinate::new(20, 20),
            },
            DivergenceKind::UnexpectedEndOfStream { expected_remaining: 3, next_expected: "end_page".into() },
            DivergenceKind::TrailingOpcodes { actual_remaining: 2, next_actual: "special".into() },
        ];

        for kind in kinds {
            let s = format!("{kind}");
            assert!(!s.is_empty());
        }

        let div = XdvDivergence {
            opcode_index: 7,
            page_number: 1,
            coordinate: coord,
            kind: DivergenceKind::OpcodeMismatch { expected: "A".into(), actual: "B".into() },
        };
        assert!(format!("{div}").contains("Opcode #7 on Page 1"));
    }

    #[test]
    fn test_xdv_builder_methods_and_opcode_trace_divergences() {
        let xdv_full = XdvBuilder::new()
            .comment(b"custom comment")
            .native_font(1, "cmr10", 655360)
            .native_font(2, "Helvetica", 655360)
            .begin_page(1)
            .set_font(1)
            .move_right(1000)
            .move_down(2000)
            .push_stack()
            .glyph(10, 100, 200, 500)
            .rule(50, 150)
            .special(b"spec content")
            .pop_stack()
            .end_page()
            .build();

        let trace1 = XdvDiffEngine::trace_opcodes(&xdv_full).unwrap();
        assert!(!trace1.is_empty());

        // Identical comparison produces clean stats
        let stats = XdvDiffEngine::compare_opcode_traces(&trace1, &trace1).unwrap();
        assert!(stats.total_opcodes > 5);
        assert_eq!(stats.pages_checked, 1);

        // 1. Preamble mismatch
        let mut trace_bad_preamble = trace1.clone();
        trace_bad_preamble[0] = XdvOpcodeEvent::Preamble { file_type: "dvi".into(), comment: b"other".to_vec() };
        let err = XdvDiffEngine::compare_opcode_traces(&trace1, &trace_bad_preamble).unwrap_err();
        assert!(matches!(err.kind, DivergenceKind::OpcodeMismatch { .. }));

        // 2. DefineNativeFont mismatch
        let mut trace_bad_native = trace1.clone();
        trace_bad_native[1] = XdvOpcodeEvent::DefineNativeFont {
            font_num: 1,
            name: "Times-Roman".into(),
            size: 655360,
            face_index: 0,
            color_rgba: None,
            extend: None,
            slant: None,
            embolden: None,
        };
        let err = XdvDiffEngine::compare_opcode_traces(&trace1, &trace_bad_native).unwrap_err();
        assert!(matches!(err.kind, DivergenceKind::FontDefinitionMismatch { .. }));

        // 3. Trailing opcodes
        let trace_short = trace1[..3].to_vec();
        let err = XdvDiffEngine::compare_opcode_traces(&trace_short, &trace1).unwrap_err();
        assert!(matches!(err.kind, DivergenceKind::TrailingOpcodes { .. }));

        // 4. Unexpected end of stream
        let err = XdvDiffEngine::compare_opcode_traces(&trace1, &trace_short).unwrap_err();
        assert!(matches!(err.kind, DivergenceKind::UnexpectedEndOfStream { .. }));
    }

    #[test]
    fn test_xdv_diff_engine_compare_bytes_and_files() {
        let xdv = XdvBuilder::new()
            .begin_page(1)
            .rule(10, 20)
            .end_page()
            .build();

        // 1. Compare identical bytes
        let stats = XdvDiffEngine::compare_bytes(&xdv, &xdv).unwrap();
        assert!(stats.bitwise_identical);
        assert_eq!(stats.pages_checked, 1);

        // 2. Compare files on disk via stream readers
        let temp_dir = tempfile::tempdir().unwrap();
        let p1 = temp_dir.path().join("a.xdv");
        let p2 = temp_dir.path().join("b.xdv");
        std::fs::write(&p1, &xdv).unwrap();
        std::fs::write(&p2, &xdv).unwrap();

        let f1 = std::fs::File::open(&p1).unwrap();
        let f2 = std::fs::File::open(&p2).unwrap();
        let f_stats = XdvDiffEngine::compare(f1, f2).unwrap();
        assert!(f_stats.bitwise_identical);

        // 3. Compare divergent files
        let mut xdv_diverged = xdv.clone();
        xdv_diverged[10] ^= 0xFF;
        std::fs::write(&p2, &xdv_diverged).unwrap();
        let f1_div = std::fs::File::open(&p1).unwrap();
        let f2_div = std::fs::File::open(&p2).unwrap();
        let div = XdvDiffEngine::compare(f1_div, f2_div).unwrap_err();
        assert!(format!("{div}").contains("Page"));
    }

    #[test]
    fn test_xdv_event_collector_char_run_and_collector_error() {
        let mut collector = XdvEventCollector::default();
        use tectonic_xdv::XdvEvents;
        collector.handle_char_run(0, &[65, 66, 67]).unwrap();
        assert_eq!(collector.char_runs.len(), 1);
        assert_eq!(collector.char_runs[0].1, vec![65, 66, 67]);

        let err = CollectorError::Io(std::io::Error::new(std::io::ErrorKind::Other, "test failure"));
        assert!(format!("{err}").contains("test failure"));
    }

    #[test]
    fn test_opcode_traces_detailed_parameter_mismatches() {
        let coord = Coordinate::new(10, 20);

        // 1. DefineFont parameter divergence
        let f1 = vec![XdvOpcodeEvent::DefineFont {
            font_num: 1, name: "cmr10".into(), scale_factor: 655360, design_size: 655360, checksum: 100
        }];
        let f2 = vec![XdvOpcodeEvent::DefineFont {
            font_num: 1, name: "cmr10".into(), scale_factor: 700000, design_size: 655360, checksum: 100
        }];
        let err1 = XdvDiffEngine::compare_opcode_traces(&f1, &f2).unwrap_err();
        assert!(matches!(err1.kind, DivergenceKind::FontDefinitionMismatch { .. }));

        // 2. SetChar parameter divergence
        let c1 = vec![XdvOpcodeEvent::SetChar { char_code: 65, font_num: 1, coord }];
        let c2 = vec![XdvOpcodeEvent::SetChar { char_code: 66, font_num: 1, coord }];
        let err2 = XdvDiffEngine::compare_opcode_traces(&c1, &c2).unwrap_err();
        assert!(matches!(err2.kind, DivergenceKind::CharMismatch { .. }));

        // 3. PutRule / SetRule divergence
        let r1 = vec![XdvOpcodeEvent::PutRule { height: 10, width: 20, coord }];
        let r2 = vec![XdvOpcodeEvent::PutRule { height: 10, width: 25, coord }];
        let err3 = XdvDiffEngine::compare_opcode_traces(&r1, &r2).unwrap_err();
        assert!(matches!(err3.kind, DivergenceKind::RuleMismatch { .. }));

        // 4. MoveDown divergence
        let m1 = vec![XdvOpcodeEvent::MoveDown { delta: 10, new_coord: coord }];
        let m2 = vec![XdvOpcodeEvent::MoveDown { delta: 20, new_coord: Coordinate::new(10, 30) }];
        let err4 = XdvDiffEngine::compare_opcode_traces(&m1, &m2).unwrap_err();
        assert!(matches!(err4.kind, DivergenceKind::MovementMismatch { .. }));

        // 5. Stack depth divergence
        let s1 = vec![XdvOpcodeEvent::Push { depth: 1, coord }];
        let s2 = vec![XdvOpcodeEvent::Push { depth: 2, coord }];
        let err5 = XdvDiffEngine::compare_opcode_traces(&s1, &s2).unwrap_err();
        assert!(matches!(err5.kind, DivergenceKind::StackDepthMismatch { .. }));

        // 6. BeginPage page_num divergence
        let p1 = vec![XdvOpcodeEvent::BeginPage { page_num: 1, counters: [0; 10] }];
        let p2 = vec![XdvOpcodeEvent::BeginPage { page_num: 2, counters: [0; 10] }];
        let err6 = XdvDiffEngine::compare_opcode_traces(&p1, &p2).unwrap_err();
        assert!(matches!(err6.kind, DivergenceKind::OpcodeMismatch { .. }));

        // 7. SetFont font_num divergence
        let sf1 = vec![XdvOpcodeEvent::SetFont { font_num: 1 }];
        let sf2 = vec![XdvOpcodeEvent::SetFont { font_num: 2 }];
        let err7 = XdvDiffEngine::compare_opcode_traces(&sf1, &sf2).unwrap_err();
        assert!(matches!(err7.kind, DivergenceKind::OpcodeMismatch { .. }));
    }

    #[test]
    fn test_xdv_builder_edge_opcodes_and_error_streams() {
        // Builder with font_num >= 64 and small move_down delta
        let xdv = XdvBuilder::new()
            .begin_page(1)
            .set_font(100) // Opcode 235 branch
            .move_down(50) // Small delta opcode 157 branch
            .end_page()
            .build();
        assert!(!xdv.is_empty());

        // Corrupt stream parse failures in compare_bytes
        let corrupt_expected = b"INVALID_HEADER_DATA";
        let valid_actual = xdv.clone();
        let err = XdvDiffEngine::compare_bytes(corrupt_expected, &valid_actual).unwrap_err();
        assert!(format!("{err}").contains("Expected stream parse failure"));

        let err_act = XdvDiffEngine::compare_bytes(&valid_actual, corrupt_expected).unwrap_err();
        assert!(format!("{err_act}").contains("Actual stream parse failure"));
    }

    #[test]
    fn test_xdv_cursor_all_opcodes_and_read_methods() {
        let mut bytes = Vec::new();
        // 1. Preamble (SPX id=100)
        bytes.push(247); // pre
        bytes.push(100); // id_byte = 100 (SPX)
        bytes.extend_from_slice(&1u32.to_be_bytes()); // num
        bytes.extend_from_slice(&1u32.to_be_bytes()); // den
        bytes.extend_from_slice(&1000u32.to_be_bytes()); // mag
        bytes.push(4); // comment len
        bytes.extend_from_slice(b"test");

        // 2. Preamble with unknown ID (e.g. 42)
        bytes.push(247);
        bytes.push(42);
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&1u32.to_be_bytes());
        bytes.extend_from_slice(&1000u32.to_be_bytes());
        bytes.push(0); // comment len 0

        // 3. Begin page
        bytes.push(139); // bop
        for i in 0..10 {
            bytes.extend_from_slice(&(i as i32).to_be_bytes());
        }
        bytes.extend_from_slice(&(-1i32).to_be_bytes()); // prev_bop

        // 4. Movements: w0 (147), w1 (148), w2 (149), w3 (150), w4 (151)
        bytes.push(148); // w1
        bytes.push(10);  // signed 8-bit
        bytes.push(147); // w0 (uses st.w = 10)
        bytes.push(149); // w2
        bytes.extend_from_slice(&20i16.to_be_bytes());
        bytes.push(150); // w3
        bytes.extend_from_slice(&[0, 0, 30]); // i24
        bytes.push(151); // w4
        bytes.extend_from_slice(&40i32.to_be_bytes());

        // 5. Movements: x0 (152), x1 (153), x2 (154), x3 (155), x4 (156)
        bytes.push(153); // x1
        bytes.push(11);
        bytes.push(152); // x0
        bytes.push(154); // x2
        bytes.extend_from_slice(&21i16.to_be_bytes());
        bytes.push(155); // x3
        bytes.extend_from_slice(&[0, 0, 31]);
        bytes.push(156); // x4
        bytes.extend_from_slice(&41i32.to_be_bytes());

        // 6. Movements: y0 (161), y1 (162), y2 (163), y3 (164), y4 (165)
        bytes.push(162); // y1
        bytes.push(12);
        bytes.push(161); // y0
        bytes.push(163); // y2
        bytes.extend_from_slice(&22i16.to_be_bytes());
        bytes.push(164); // y3
        bytes.extend_from_slice(&[0, 0, 32]);
        bytes.push(165); // y4
        bytes.extend_from_slice(&42i32.to_be_bytes());

        // 7. Movements: z0 (166), z1 (167), z2 (168), z3 (169), z4 (170)
        bytes.push(167); // z1
        bytes.push(13);
        bytes.push(166); // z0
        bytes.push(168); // z2
        bytes.extend_from_slice(&23i16.to_be_bytes());
        bytes.push(169); // z3
        bytes.extend_from_slice(&[0, 0, 33]);
        bytes.push(170); // z4
        bytes.extend_from_slice(&43i32.to_be_bytes());

        // 8. Specials: xxx1 (239), xxx2 (240), xxx3 (241), xxx4 (242)
        bytes.push(239); // xxx1
        bytes.push(3);
        bytes.extend_from_slice(b"sp1");

        bytes.push(240); // xxx2
        bytes.extend_from_slice(&3u16.to_be_bytes());
        bytes.extend_from_slice(b"sp2");

        bytes.push(241); // xxx3
        bytes.extend_from_slice(&[0, 0, 3]);
        bytes.extend_from_slice(b"sp3");

        bytes.push(242); // xxx4
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes.extend_from_slice(b"sp4");

        // 9. Font definitions: fnt_def1 (243), fnt_def2 (244)
        bytes.push(243); // fnt_def1
        bytes.push(1);   // font_num
        bytes.extend_from_slice(&123u32.to_be_bytes()); // checksum
        bytes.extend_from_slice(&655360u32.to_be_bytes()); // scale
        bytes.extend_from_slice(&655360u32.to_be_bytes()); // design
        bytes.push(0); // area len
        bytes.push(5); // name len
        bytes.extend_from_slice(b"cmr10");

        // 10. End page (140)
        bytes.push(140);

        let mut cursor = OpcodeParser::new(&bytes);
        let events = cursor.parse_all().unwrap();
        assert!(!events.is_empty());
    }

    #[test]
    fn test_xdv_cursor_read_error_branches() {
        let short_data = [0u8; 1];
        let mut c1 = OpcodeParser::new(&short_data);
        assert!(c1.read_i16().is_err());

        let mut c2 = OpcodeParser::new(&short_data);
        assert!(c2.read_u24().is_err());

        let mut c3 = OpcodeParser::new(&short_data);
        assert!(c3.read_i24().is_err());

        let mut c4 = OpcodeParser::new(&short_data);
        assert!(c4.read_u32().is_err());

        let mut c5 = OpcodeParser::new(&short_data);
        assert!(c5.read_i32().is_err());

        let mut c6 = OpcodeParser::new(&short_data);
        assert!(c6.read_slice(10).is_err());

        let mut c7 = OpcodeParser::new(&short_data);
        assert!(c7.read_compact_signed(5).is_err());

        let mut c8 = OpcodeParser::new(&short_data);
        assert!(c8.read_compact_pos(5).is_err());
    }
}
