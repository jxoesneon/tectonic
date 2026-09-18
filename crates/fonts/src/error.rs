// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Error types for font metric parsing, scaling, and registry operations.

use thiserror::Error;

/// High-level font system errors.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum FontError {
    /// An error occurred during TFM (TeX Font Metric) ingestion.
    #[error("TFM parsing error: {0}")]
    Tfm(#[from] TfmError),

    /// An error occurred during OpenType/TrueType parsing or lookup.
    #[error("OpenType font error: {0}")]
    OpenType(String),

    /// An invalid font identifier was queried.
    #[error("font ID {0} is not registered")]
    InvalidFontId(u32),

    /// A requested font name was not found.
    #[error("font '{0}' was not found")]
    NotFound(String),

    /// An I/O error occurred while reading font data.
    #[error("I/O error: {0}")]
    Io(String),
}

/// Errors specific to the TeX Font Metric (TFM) format.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TfmError {
    /// The input data is shorter than the minimum TFM file length (24 bytes for 6 words).
    #[error("TFM file truncated: fewer than 24 bytes")]
    TruncatedHeader,

    /// The file length declared in word 0 does not match the actual byte count.
    #[error("TFM word count mismatch: declared {declared} words ({declared_bytes} bytes), but got {actual_bytes} bytes")]
    FileLengthMismatch {
        /// Declared word count in TFM header.
        declared: usize,
        /// Expected byte count (`declared * 4`).
        declared_bytes: usize,
        /// Actual byte count.
        actual_bytes: usize,
    },

    /// The character range [bc, ec] is invalid according to TeX rules.
    #[error("invalid character code range: bc={bc}, ec={ec} (must satisfy bc <= ec + 1 and ec <= 255)")]
    InvalidCharBounds {
        /// First character code in font.
        bc: u16,
        /// Last character code in font.
        ec: u16,
    },

    /// The declared header length (lh) is smaller than 2 words (checksum and design size).
    #[error("invalid header length: lh={lh} (must be >= 2)")]
    InvalidHeaderLength {
        /// Header length in words.
        lh: usize,
    },

    /// A required table has an invalid length.
    #[error("invalid table length for {name}: {len}")]
    InvalidTableLength {
        /// Name of the invalid table.
        name: &'static str,
        /// Table length in words.
        len: usize,
    },

    /// The design size in the header is invalid (< 65536 scaled points / 1 pt).
    #[error("invalid font design size: {size} sp (must be >= 65536 sp)")]
    InvalidDesignSize {
        /// Design size in scaled points.
        size: i32,
    },

    /// The first entry in width, height, depth, or italic tables must be zero.
    #[error("first entry (index 0) of {table} table must be zero")]
    NonZeroZeroIndex {
        /// Table name ("width", "height", "depth", "italic").
        table: &'static str,
    },

    /// A character info word points to an out-of-bounds table index.
    #[error("invalid character info for code {code}: {reason}")]
    InvalidCharInfo {
        /// Character code.
        code: u8,
        /// Descriptive failure reason.
        reason: &'static str,
    },

    /// A ligature or kerning instruction is malformed or out of bounds.
    #[error("invalid lig/kern instruction at index {index}: {reason}")]
    InvalidLigKern {
        /// Index in lig_kern table.
        index: usize,
        /// Descriptive failure reason.
        reason: &'static str,
    },

    /// An extensible character recipe references invalid character codes.
    #[error("invalid extensible character recipe at index {index}: {reason}")]
    InvalidExtensible {
        /// Index in extensible table.
        index: usize,
        /// Descriptive failure reason.
        reason: &'static str,
    },

    /// Unexpected end of file encountered while parsing TFM structures.
    #[error("unexpected end of file")]
    UnexpectedEof,
}
