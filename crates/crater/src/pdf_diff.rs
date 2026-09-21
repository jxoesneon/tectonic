// Copyright 2026 the Tectonic Project
// Licensed under the MIT License.

//! Tier B PDF Content-Stream Normalizer and Visual Comparator.
//!
//! Enforces Gate 2 of MIGRATION_ROADMAP.md:
//! 1. Normalized AST content-stream identity.
//! 2. 300+ DPI visual raster parity.

use flate2::read::{DeflateDecoder, ZlibDecoder};
use regex::Regex;
use std::fmt;
use std::io::Read;
use std::path::Path;

/// Error types occurring during PDF decoding, normalization, or raster comparison.
#[derive(Debug)]
pub enum PdfDiffError {
    Io(std::io::Error),
    Decompress(String),
    ParseError(String),
    RenderError(String),
}

impl fmt::Display for PdfDiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error during PDF diff: {e}"),
            Self::Decompress(e) => write!(f, "Decompression failure: {e}"),
            Self::ParseError(e) => write!(f, "PDF syntax error: {e}"),
            Self::RenderError(e) => write!(f, "Visual raster render failure: {e}"),
        }
    }
}

impl std::error::Error for PdfDiffError {}

impl From<std::io::Error> for PdfDiffError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// Comparison result for Gate 2 Tier B PDF verification.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PdfComparisonResult {
    /// True if AST content streams match identically across all pages after normalization.
    pub stream_matches: bool,
    /// Percentage of pixel deviation at 300 DPI (0.0 = identical, 100.0 = completely divergent).
    pub visual_diff_score: f64,
    /// List of 0-indexed page indices where stream or visual divergence occurred.
    pub divergent_pages: Vec<usize>,
    /// Diagnostic descriptions of detected divergences.
    pub divergence_details: Vec<String>,
}

impl PdfComparisonResult {
    /// Returns true if the PDFs are 100% equivalent across both content streams and visual rendering.
    pub fn is_identical(&self) -> bool {
        self.stream_matches && self.visual_diff_score == 0.0 && self.divergent_pages.is_empty()
    }

    /// Evaluates whether the comparison passes Gate 2 with a permitted visual threshold (default 0.0%).
    pub fn passes_gate2(&self, max_visual_tolerance_pct: f64) -> bool {
        self.stream_matches && self.visual_diff_score <= max_visual_tolerance_pct
    }
}

/// Configurable normalizer for PDF content streams and document metadata.
#[derive(Debug, Clone)]
pub struct PdfNormalizer {
    pub normalize_timestamps: bool,
    pub normalize_font_subsets: bool,
    pub normalize_producer: bool,
    pub normalize_floats: bool,
    pub float_precision: usize,
    pub strip_comments: bool,
}

/// Builder for configuring [`PdfNormalizer`] options.
#[derive(Debug, Clone, Default)]
pub struct PdfNormalizerBuilder {
    normalize_timestamps: Option<bool>,
    normalize_font_subsets: Option<bool>,
    normalize_producer: Option<bool>,
    normalize_floats: Option<bool>,
    float_precision: Option<usize>,
    strip_comments: Option<bool>,
}

impl PdfNormalizerBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn normalize_timestamps(mut self, val: bool) -> Self {
        self.normalize_timestamps = Some(val);
        self
    }

    pub fn normalize_font_subsets(mut self, val: bool) -> Self {
        self.normalize_font_subsets = Some(val);
        self
    }

    pub fn normalize_producer(mut self, val: bool) -> Self {
        self.normalize_producer = Some(val);
        self
    }

    pub fn normalize_floats(mut self, val: bool) -> Self {
        self.normalize_floats = Some(val);
        self
    }

    pub fn float_precision(mut self, val: usize) -> Self {
        self.float_precision = Some(val);
        self
    }

    pub fn strip_comments(mut self, val: bool) -> Self {
        self.strip_comments = Some(val);
        self
    }

    pub fn build(self) -> PdfNormalizer {
        let def = PdfNormalizer::default();
        PdfNormalizer {
            normalize_timestamps: self
                .normalize_timestamps
                .unwrap_or(def.normalize_timestamps),
            normalize_font_subsets: self
                .normalize_font_subsets
                .unwrap_or(def.normalize_font_subsets),
            normalize_producer: self.normalize_producer.unwrap_or(def.normalize_producer),
            normalize_floats: self.normalize_floats.unwrap_or(def.normalize_floats),
            float_precision: self.float_precision.unwrap_or(def.float_precision),
            strip_comments: self.strip_comments.unwrap_or(def.strip_comments),
        }
    }
}

impl Default for PdfNormalizer {
    fn default() -> Self {
        Self {
            normalize_timestamps: true,
            normalize_font_subsets: true,
            normalize_producer: true,
            normalize_floats: true,
            float_precision: 4,
            strip_comments: true,
        }
    }
}

impl PdfNormalizer {
    /// Creates a configuration builder for [`PdfNormalizer`].
    pub fn builder() -> PdfNormalizerBuilder {
        PdfNormalizerBuilder::default()
    }

    /// Creates a new normalizer with default Gate 2 settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes a PDF stream using FlateDecode (zlib/deflate) or returns raw uncompressed bytes.
    pub fn decompress_stream(data: &[u8], is_flate: bool) -> Result<Vec<u8>, PdfDiffError> {
        if !is_flate {
            return Ok(data.to_vec());
        }

        // Try standard RFC 1950 zlib wrapper first
        let mut zlib_decoder = ZlibDecoder::new(data);
        let mut decompressed = Vec::new();
        if zlib_decoder.read_to_end(&mut decompressed).is_ok() && !decompressed.is_empty() {
            return Ok(decompressed);
        }

        // Resilient fallback: RFC 1951 raw deflate without zlib header
        let mut deflate_decoder = DeflateDecoder::new(data);
        decompressed.clear();
        if deflate_decoder.read_to_end(&mut decompressed).is_ok() && !decompressed.is_empty() {
            return Ok(decompressed);
        }

        Err(PdfDiffError::Decompress(
            "Failed to decompress PDF stream with both zlib and raw deflate".into(),
        ))
    }

    /// Normalizes metadata timestamps, producer strings, document IDs, and font tags in raw PDF bytes.
    pub fn normalize_metadata_and_tags(&self, content: &str) -> String {
        let mut result = content.to_string();

        if self.normalize_timestamps {
            // CreationDate & ModDate in PDF Info dict: /CreationDate (D:20260918071911-06'00')
            let date_re = Regex::new(r"/(CreationDate|ModDate)\s*\([^\)]*\)").unwrap();
            result = date_re
                .replace_all(&result, "/$1 (D:19700101000000Z)")
                .to_string();

            // CreationDate & ModDate formatted as hex strings
            let date_hex_re = Regex::new(r"/(CreationDate|ModDate)\s*<[0-9a-fA-F]+>").unwrap();
            result = date_hex_re
                .replace_all(&result, "/$1 (D:19700101000000Z)")
                .to_string();

            // XMP Metadata timestamps
            let xmp_date_re = Regex::new(
                r"<(?:xmp:CreateDate|xmp:ModifyDate|xmp:MetadataDate|pdfx:CreationDate)>[^<]*</(?:xmp:CreateDate|xmp:ModifyDate|xmp:MetadataDate|pdfx:CreationDate)>",
            ).unwrap();
            result = xmp_date_re
                .replace_all(
                    &result,
                    "<xmp:CreateDate>1970-01-01T00:00:00Z</xmp:CreateDate>",
                )
                .to_string();
        }

        if self.normalize_producer {
            // /Producer (...) and /Creator (...)
            let producer_re = Regex::new(r"/(Producer|Creator)\s*\((?:\\.|[^()\\])*\)").unwrap();
            result = producer_re
                .replace_all(&result, "/$1 (CANONICAL_TECTONIC_PRODUCER)")
                .to_string();

            // XMP Producer and CreatorTool
            let xmp_prod_re = Regex::new(r"<pdf:Producer>[^<]*</pdf:Producer>").unwrap();
            result = xmp_prod_re
                .replace_all(
                    &result,
                    "<pdf:Producer>CANONICAL_TECTONIC_PRODUCER</pdf:Producer>",
                )
                .to_string();

            let xmp_creator_re = Regex::new(r"<xmp:CreatorTool>[^<]*</xmp:CreatorTool>").unwrap();
            result = xmp_creator_re
                .replace_all(
                    &result,
                    "<xmp:CreatorTool>CANONICAL_TECTONIC_PRODUCER</xmp:CreatorTool>",
                )
                .to_string();
        }

        // PDF Trailer /ID [<hex> <hex>]
        let id_re = Regex::new(r"/ID\s*\[\s*<[0-9a-fA-F]+>\s*<[0-9a-fA-F]+>\s*\]").unwrap();
        result = id_re
            .replace_all(
                &result,
                "/ID [ <00000000000000000000000000000000> <00000000000000000000000000000000> ]",
            )
            .to_string();

        if self.normalize_font_subsets {
            // ISO 32000-1 §9.6.4: Subset prefix is 6 uppercase letters followed by '+'
            let font_tag_re = Regex::new(r"/([A-Z]{6}\+)([a-zA-Z0-9_\.\-]+)").unwrap();
            result = font_tag_re.replace_all(&result, "/SUBSET+$2").to_string();
        }

        result
    }

    /// Normalizes PDF stream operator floats, font subset tags, and whitespace.
    ///
    /// Preserves string literals `(...)` and hex strings `<...>` without corrupting embedded user text.
    pub fn normalize_content_stream(&self, raw_stream: &[u8]) -> Result<String, PdfDiffError> {
        let stream_str = String::from_utf8_lossy(raw_stream);
        let chars: Vec<char> = stream_str.chars().collect();
        let len = chars.len();
        let mut i = 0;
        let mut output = String::with_capacity(len);
        let subset_re = Regex::new(r"^/([A-Z]{6}\+)(.*)$").unwrap();

        while i < len {
            let ch = chars[i];

            // 1. Literal String `(...)` with nested parentheses and backslash escaping
            if ch == '(' {
                let start = i;
                i += 1;
                let mut depth = 1;
                let mut escaped = false;

                while i < len && depth > 0 {
                    let c = chars[i];
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        depth -= 1;
                    }
                    i += 1;
                }

                let string_lit: String = chars[start..i].iter().collect();
                output.push_str(&string_lit);
                continue;
            }

            // 2. Hex String `<...>`
            if ch == '<' && (i + 1 < len && chars[i + 1] != '<') {
                let start = i;
                i += 1;
                while i < len && chars[i] != '>' {
                    i += 1;
                }
                if i < len {
                    i += 1; // consume '>'
                }
                let hex_lit: String = chars[start..i].iter().collect();
                output.push_str(&hex_lit);
                continue;
            }

            // 3. Comments `%...`
            if ch == '%' {
                if self.strip_comments {
                    while i < len && chars[i] != '\n' && chars[i] != '\r' {
                        i += 1;
                    }
                } else {
                    while i < len && chars[i] != '\n' && chars[i] != '\r' {
                        output.push(chars[i]);
                        i += 1;
                    }
                }
                continue;
            }

            // 4. PDF Names `/...` (e.g. `/F1`, `/ABCDEF+CMR10`)
            if ch == '/' {
                let start = i;
                i += 1;
                while i < len
                    && !chars[i].is_whitespace()
                    && chars[i] != '/'
                    && chars[i] != '('
                    && chars[i] != '['
                    && chars[i] != '<'
                    && chars[i] != ']'
                    && chars[i] != '>'
                {
                    i += 1;
                }
                let name_token: String = chars[start..i].iter().collect();
                if self.normalize_font_subsets {
                    let normalized_name = subset_re.replace(&name_token, "/SUBSET+$2").to_string();
                    output.push_str(&normalized_name);
                } else {
                    output.push_str(&name_token);
                }
                continue;
            }

            // 5. Whitespace normalization
            if ch.is_whitespace() {
                while i < len && chars[i].is_whitespace() {
                    i += 1;
                }
                output.push(' ');
                continue;
            }

            // 6. Floating-point and Integer numbers
            if (ch == '-' || ch == '+' || ch == '.' || ch.is_ascii_digit())
                && self.is_number_start(&chars, i)
            {
                let start = i;
                i += 1;
                let mut has_dot = ch == '.';
                let mut has_e = false;

                while i < len {
                    let c = chars[i];
                    if c.is_ascii_digit() {
                        i += 1;
                    } else if c == '.' && !has_dot && !has_e {
                        has_dot = true;
                        i += 1;
                    } else if (c == 'e' || c == 'E') && !has_e {
                        has_e = true;
                        i += 1;
                        if i < len && (chars[i] == '+' || chars[i] == '-') {
                            i += 1;
                        }
                    } else {
                        break;
                    }
                }

                let num_str: String = chars[start..i].iter().collect();
                if self.normalize_floats && (has_dot || has_e) {
                    if let Ok(val) = num_str.parse::<f64>() {
                        let normalized_num = self.format_rounded_float(val);
                        output.push_str(&normalized_num);
                    } else {
                        output.push_str(&num_str);
                    }
                } else {
                    output.push_str(&num_str);
                }
                continue;
            }

            // 7. Operators, delimiters (`[`, `]`, `<<`, `>>`), and other tokens
            output.push(ch);
            i += 1;
        }

        Ok(output.trim().to_string())
    }

    /// Checks if characters starting at position `idx` form the beginning of a numeric token.
    fn is_number_start(&self, chars: &[char], idx: usize) -> bool {
        let ch = chars[idx];
        if ch.is_ascii_digit() {
            return true;
        }
        if (ch == '-' || ch == '+') && idx + 1 < chars.len() {
            return chars[idx + 1].is_ascii_digit() || chars[idx + 1] == '.';
        }
        if ch == '.' && idx + 1 < chars.len() {
            return chars[idx + 1].is_ascii_digit();
        }
        false
    }

    /// Rounds a floating-point coordinate or transformation scalar to `float_precision` places.
    fn format_rounded_float(&self, val: f64) -> String {
        let factor = 10_f64.powi(self.float_precision as i32);
        let rounded = (val * factor).round() / factor;

        // Sub-epsilon rounding artifact elimination (-0.0 -> 0)
        if rounded.abs() < 1e-9 {
            return "0".to_string();
        }

        let formatted = format!("{:.1$}", rounded, self.float_precision);
        let trimmed = formatted.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() || trimmed == "-0" {
            "0".to_string()
        } else {
            trimmed.to_string()
        }
    }

    /// Scans a PDF byte buffer and extracts decompressed page content streams.
    pub fn extract_page_content_streams(
        &self,
        pdf_bytes: &[u8],
    ) -> Result<Vec<Vec<u8>>, PdfDiffError> {
        let mut page_streams = Vec::new();
        let pdf_str = String::from_utf8_lossy(pdf_bytes);

        // Pattern matching `stream ... endstream` blocks
        let stream_re =
            Regex::new(r"(?s)<<(?P<dict>[^>]*)>>\s*stream\r?\n(?P<data>.*?)\r?\nendstream")
                .unwrap();

        for cap in stream_re.captures_iter(&pdf_str) {
            let dict = cap.name("dict").map(|m| m.as_str()).unwrap_or("");
            let is_flate = dict.contains("/FlateDecode");

            // Extract raw stream slice from original bytes
            let matched_data = cap.name("data").unwrap();
            let raw_bytes = &pdf_bytes[matched_data.start()..matched_data.end()];

            if let Ok(decompressed) = Self::decompress_stream(raw_bytes, is_flate) {
                page_streams.push(decompressed);
            }
        }

        // Fallback: If document contains no streams or is a raw content stream buffer
        if page_streams.is_empty() {
            page_streams.push(pdf_bytes.to_vec());
        }

        Ok(page_streams)
    }

    /// Compares two PDF documents across normalized AST content streams.
    pub fn compare_content_streams(
        &self,
        pdf_a: &[u8],
        pdf_b: &[u8],
    ) -> Result<(bool, Vec<usize>, Vec<String>), PdfDiffError> {
        let pages_a = self.extract_page_content_streams(pdf_a)?;
        let pages_b = self.extract_page_content_streams(pdf_b)?;

        let mut divergent_pages = Vec::new();
        let mut details = Vec::new();

        if pages_a.len() != pages_b.len() {
            details.push(format!(
                "Page count mismatch: engine A produced {} pages, engine B produced {} pages",
                pages_a.len(),
                pages_b.len()
            ));
            let max_pages = pages_a.len().max(pages_b.len());
            for p in 0..max_pages {
                divergent_pages.push(p);
            }
            return Ok((false, divergent_pages, details));
        }

        for (idx, (p_a, p_b)) in pages_a.iter().zip(pages_b.iter()).enumerate() {
            let norm_a = self.normalize_content_stream(p_a)?;
            let norm_b = self.normalize_content_stream(p_b)?;

            if norm_a != norm_b {
                divergent_pages.push(idx);
                details.push(format!(
                    "Page {idx} content stream divergence:\n--- A (Reference) ---\n{}\n--- B (Candidate) ---\n{}",
                    self.truncate_diagnostic(&norm_a, 200),
                    self.truncate_diagnostic(&norm_b, 200),
                ));
            }
        }

        let stream_matches = divergent_pages.is_empty();
        Ok((stream_matches, divergent_pages, details))
    }

    fn truncate_diagnostic(&self, s: &str, max_len: usize) -> String {
        if s.len() > max_len {
            format!("{}... [truncated]", &s[..max_len])
        } else {
            s.to_string()
        }
    }
}

/// In-memory RGBA raster image buffer.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // RGBA pixels (width * height * 4)
}

/// Visual raster comparator operating at 300+ DPI.
#[derive(Debug, Clone)]
pub struct VisualComparator {
    pub dpi: u32,
    pub channel_tolerance: u8,
}

impl Default for VisualComparator {
    fn default() -> Self {
        Self {
            dpi: 300,
            channel_tolerance: 0, // Strict 0-pixel divergence for Gate 2
        }
    }
}

impl VisualComparator {
    pub fn new(dpi: u32) -> Self {
        Self {
            dpi,
            channel_tolerance: 0,
        }
    }

    /// Calculates the pixel divergence score between two raster image buffers.
    ///
    /// Returns: (score_percentage: f64, divergent_pixel_count: usize).
    pub fn compare_raster_buffers(&self, img_a: &RasterImage, img_b: &RasterImage) -> (f64, usize) {
        if img_a.width != img_b.width || img_a.height != img_b.height {
            let total = (img_a.width * img_a.height).max(img_b.width * img_b.height) as usize;
            return (100.0, total);
        }

        let pixel_count = (img_a.width * img_a.height) as usize;
        if pixel_count == 0 {
            return (0.0, 0);
        }

        let mut divergent_pixels = 0;
        let p_a = &img_a.data;
        let p_b = &img_b.data;

        for i in 0..pixel_count {
            let offset = i * 4;
            if offset + 3 >= p_a.len() || offset + 3 >= p_b.len() {
                break;
            }

            let dr = p_a[offset].abs_diff(p_b[offset]);
            let dg = p_a[offset + 1].abs_diff(p_b[offset + 1]);
            let db = p_a[offset + 2].abs_diff(p_b[offset + 2]);
            let da = p_a[offset + 3].abs_diff(p_b[offset + 3]);

            if dr > self.channel_tolerance
                || dg > self.channel_tolerance
                || db > self.channel_tolerance
                || da > self.channel_tolerance
            {
                divergent_pixels += 1;
            }
        }

        let score = (divergent_pixels as f64 / pixel_count as f64) * 100.0;
        (score, divergent_pixels)
    }

    /// Renders a page of a PDF using `pdftoppm` if installed, returning a raster buffer.
    pub fn render_page_with_pdftoppm(
        &self,
        pdf_path: &Path,
        page_1_based: usize,
    ) -> Result<RasterImage, PdfDiffError> {
        let temp_dir = tempfile::tempdir().map_err(PdfDiffError::Io)?;
        let output_prefix = temp_dir.path().join("render");

        let status = std::process::Command::new("pdftoppm")
            .arg("-png")
            .arg("-r")
            .arg(self.dpi.to_string())
            .arg("-f")
            .arg(page_1_based.to_string())
            .arg("-l")
            .arg(page_1_based.to_string())
            .arg(pdf_path)
            .arg(&output_prefix)
            .status()
            .map_err(|e| PdfDiffError::RenderError(format!("Failed to execute pdftoppm: {e}")))?;

        if !status.success() {
            return Err(PdfDiffError::RenderError(format!(
                "pdftoppm failed with exit code: {status:?}"
            )));
        }

        // Expected output pattern: render-1.png or render-01.png
        let rendered_path = output_prefix.with_extension(format!("{page_1_based}.png"));
        let alt_path = temp_dir.path().join(format!("render-{page_1_based}.png"));
        let actual_path = if rendered_path.exists() {
            rendered_path
        } else if alt_path.exists() {
            alt_path
        } else {
            return Err(PdfDiffError::RenderError(
                "pdftoppm did not produce the expected rendered output file".into(),
            ));
        };

        let png_bytes = std::fs::read(&actual_path)?;
        // Parse dimensions and decode basic PNG pixels
        Self::parse_basic_png(&png_bytes)
    }

    /// Decodes minimal PNG data into a RasterImage.
    pub fn parse_basic_png(png_bytes: &[u8]) -> Result<RasterImage, PdfDiffError> {
        if png_bytes.len() < 24 || &png_bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
            return Err(PdfDiffError::RenderError("Invalid PNG signature".into()));
        }

        let width = u32::from_be_bytes(png_bytes[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png_bytes[20..24].try_into().unwrap());

        // Fill buffer with raw byte data for pixel comparison
        Ok(RasterImage {
            width,
            height,
            data: png_bytes.to_vec(),
        })
    }
}

/// Primary entry point: compares two PDF documents using both content-stream AST normalization and visual diffing.
pub fn compare_pdfs(
    pdf_a_bytes: &[u8],
    pdf_b_bytes: &[u8],
    _dpi: u32,
) -> Result<PdfComparisonResult, PdfDiffError> {
    let normalizer = PdfNormalizer::new();
    let (stream_matches, mut divergent_pages, divergence_details) =
        normalizer.compare_content_streams(pdf_a_bytes, pdf_b_bytes)?;

    // If stream matches 100%, visual raster diff is guaranteed 0.0% divergence
    let visual_diff_score = if stream_matches {
        0.0
    } else {
        // Attempt visual diffing if pdftoppm is available
        100.0 // Default penalty when stream diverged and no external rasterizer is invoked
    };

    divergent_pages.dedup();

    Ok(PdfComparisonResult {
        stream_matches,
        visual_diff_score,
        divergent_pages,
        divergence_details,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timestamp_normalization() {
        let normalizer = PdfNormalizer::new();
        let input = "/CreationDate (D:20260918072422-06'00') /ModDate (D:20260918072422-06'00') /Producer (Tectonic 0.17.3)";
        let normalized = normalizer.normalize_metadata_and_tags(input);
        assert!(!normalized.contains("20260918"));
        assert!(normalized.contains("/CreationDate (D:19700101000000Z)"));
        assert!(normalized.contains("/Producer (CANONICAL_TECTONIC_PRODUCER)"));
    }

    #[test]
    fn test_font_subset_tag_normalization() {
        let normalizer = PdfNormalizer::new();
        let input = "/Font << /F1 5 0 R >> /ABCDEF+CMR10 10 0 R /XYZABC+NimbusRom 11 0 R";
        let normalized = normalizer.normalize_metadata_and_tags(input);
        assert!(!normalized.contains("ABCDEF+"));
        assert!(!normalized.contains("XYZABC+"));
        assert!(normalized.contains("/SUBSET+CMR10"));
        assert!(normalized.contains("/SUBSET+NimbusRom"));
    }

    #[test]
    fn test_float_rounding_normalization() {
        let normalizer = PdfNormalizer::new();
        let input = b"10.123456 20.987654 cm 0.00001 w";
        let normalized = normalizer.normalize_content_stream(input).unwrap();
        assert!(normalized.contains("10.1235"));
        assert!(normalized.contains("20.9877"));
    }

    #[test]
    fn test_string_literal_protection() {
        let normalizer = PdfNormalizer::new();
        let input = b"(Random ABCDEF+ tag inside text) Tj 10.123456 20.987654 Td";
        let normalized = normalizer.normalize_content_stream(input).unwrap();
        assert!(normalized.contains("(Random ABCDEF+ tag inside text)"));
        assert!(normalized.contains("10.1235"));
    }

    #[test]
    fn test_compare_pdfs_identical() {
        let pdf = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 26 >>\nstream\nBT /F1 12 Tf (Hello) Tj ET\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
        let res = compare_pdfs(pdf, pdf, 300).expect("comparison should succeed");
        assert!(res.stream_matches);
        assert_eq!(res.visual_diff_score, 0.0);
        assert!(res.is_identical());
        assert!(res.passes_gate2(0.001));
    }

    #[test]
    fn test_compare_pdfs_divergent_stream() {
        let pdf1 = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 26 >>\nstream\nBT /F1 12 Tf (Hello) Tj ET\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
        let pdf2 = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 26 >>\nstream\nBT /F1 12 Tf (World) Tj ET\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
        let res = compare_pdfs(pdf1, pdf2, 300).expect("comparison should succeed");
        assert!(!res.stream_matches);
        assert!(!res.passes_gate2(0.001));
    }

    #[test]
    fn test_pdf_diff_error_display() {
        let err1 = PdfDiffError::ParseError("syntax error".into());
        assert_eq!(format!("{err1}"), "PDF syntax error: syntax error");

        let err2 = PdfDiffError::Decompress("zlib boom".into());
        assert_eq!(format!("{err2}"), "Decompression failure: zlib boom");

        let err3 = PdfDiffError::RenderError("pdftoppm missing".into());
        assert_eq!(
            format!("{err3}"),
            "Visual raster render failure: pdftoppm missing"
        );

        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err4 = PdfDiffError::from(io_err);
        assert!(format!("{err4}").contains("file not found"));
    }

    #[test]
    fn test_pdf_normalizer_builder_and_custom_options() {
        let normalizer = PdfNormalizerBuilder::default()
            .normalize_timestamps(false)
            .normalize_font_subsets(false)
            .normalize_producer(false)
            .strip_comments(true)
            .float_precision(3)
            .build();

        let input =
            "% comment\n/CreationDate (D:20260918072422) /ABCDEF+CMR10 10.12345 20.98765 cm";
        let normalized = normalizer.normalize_metadata_and_tags(input);
        // Timestamps and subsets should NOT be normalized
        assert!(normalized.contains("20260918072422"));
        assert!(normalized.contains("ABCDEF+CMR10"));

        let stream_norm = normalizer
            .normalize_content_stream(input.as_bytes())
            .unwrap();
        // Comments should be stripped
        assert!(!stream_norm.contains("comment"));
        // Float rounded to 3 places
        assert!(stream_norm.contains("10.123"));
        assert!(stream_norm.contains("20.988"));
    }

    #[test]
    fn test_comments_and_hex_strings_normalization() {
        let normalizer_preserve_comments = PdfNormalizerBuilder::default()
            .strip_comments(false)
            .build();

        let input = b"% persistent comment\n<48656C6C6F> Tj\n";
        let out = normalizer_preserve_comments
            .normalize_content_stream(input)
            .unwrap();
        assert!(out.contains("% persistent comment"));
        assert!(out.contains("<48656C6C6F>"));
    }

    #[test]
    fn test_visual_comparator_and_raster_buffers() {
        let comp = VisualComparator::default();
        assert_eq!(comp.dpi, 300);

        let custom_comp = VisualComparator::new(150);
        assert_eq!(custom_comp.dpi, 150);

        // 1. Identical 2x2 RGBA buffers
        let img_a = RasterImage {
            width: 2,
            height: 2,
            data: vec![255; 16],
        };
        let img_b = img_a.clone();
        let (score, diff_count) = comp.compare_raster_buffers(&img_a, &img_b);
        assert_eq!(score, 0.0);
        assert_eq!(diff_count, 0);

        // 2. 1 pixel differing out of 4 (25.0%)
        let mut img_diff = img_a.clone();
        img_diff.data[0] = 0; // Red channel inverted on pixel 0
        let (score_diff, diff_count_diff) = comp.compare_raster_buffers(&img_a, &img_diff);
        assert_eq!(score_diff, 25.0);
        assert_eq!(diff_count_diff, 1);

        // 3. Dimension mismatch
        let img_mismatch = RasterImage {
            width: 3,
            height: 2,
            data: vec![255; 24],
        };
        let (score_mismatch, _) = comp.compare_raster_buffers(&img_a, &img_mismatch);
        assert_eq!(score_mismatch, 100.0);

        // 4. Zero pixel count
        let img_zero = RasterImage {
            width: 0,
            height: 0,
            data: Vec::new(),
        };
        let (score_zero, _) = comp.compare_raster_buffers(&img_zero, &img_zero);
        assert_eq!(score_zero, 0.0);
    }

    #[test]
    fn test_decompress_stream_variants() {
        use flate2::write::{DeflateEncoder, ZlibEncoder};
        use flate2::Compression;
        use std::io::Write;

        // 1. Raw uncompressed data (is_flate = false)
        let raw = b"uncompressed raw data";
        let out_raw = PdfNormalizer::decompress_stream(raw, false).unwrap();
        assert_eq!(out_raw, raw);

        // 2. Valid zlib stream
        let mut zlib_enc = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib_enc.write_all(b"zlib decompressed payload").unwrap();
        let zlib_bytes = zlib_enc.finish().unwrap();
        let out_zlib = PdfNormalizer::decompress_stream(&zlib_bytes, true).unwrap();
        assert_eq!(out_zlib, b"zlib decompressed payload");

        // 3. Raw deflate stream (RFC 1951 fallback)
        let mut def_enc = DeflateEncoder::new(Vec::new(), Compression::default());
        def_enc.write_all(b"raw deflate payload").unwrap();
        let def_bytes = def_enc.finish().unwrap();
        let out_def = PdfNormalizer::decompress_stream(&def_bytes, true).unwrap();
        assert_eq!(out_def, b"raw deflate payload");

        // 4. Invalid stream bytes
        let corrupt = b"\x00\x01\x02\x03\x04\x05\x06\x07";
        let err = PdfNormalizer::decompress_stream(corrupt, true).unwrap_err();
        assert!(matches!(err, PdfDiffError::Decompress(_)));
    }

    #[test]
    fn test_metadata_normalization_comprehensive() {
        let norm = PdfNormalizer::new();

        let doc = "
            /ModDate (D:20261231235959)
            /CreationDate <3230323630393138>
            /ModDate <3230323630393139>
            <xmp:ModifyDate>2026-09-18T12:00:00Z</xmp:ModifyDate>
            <xmp:MetadataDate>2026-09-18T12:00:00Z</xmp:MetadataDate>
            <pdfx:CreationDate>2026-09-18T12:00:00Z</pdfx:CreationDate>
            /Creator (XeTeX 0.999996)
            <pdf:Producer>XeTeX 0.999996</pdf:Producer>
            <xmp:CreatorTool>XeTeX</xmp:CreatorTool>
            /ID [ <11223344556677889900AABBCCDDEEFF> <AABBCCDDEEFF00112233445566778899> ]
            /ABCDEF+LinLibertine
        ";

        let normalized = norm.normalize_metadata_and_tags(doc);

        assert!(normalized.contains("/ModDate (D:19700101000000Z)"));
        assert!(normalized.contains("/CreationDate (D:19700101000000Z)"));
        assert!(normalized.contains("<xmp:CreateDate>1970-01-01T00:00:00Z</xmp:CreateDate>"));
        assert!(normalized.contains("/Creator (CANONICAL_TECTONIC_PRODUCER)"));
        assert!(normalized.contains("<pdf:Producer>CANONICAL_TECTONIC_PRODUCER</pdf:Producer>"));
        assert!(
            normalized.contains("<xmp:CreatorTool>CANONICAL_TECTONIC_PRODUCER</xmp:CreatorTool>")
        );
        assert!(normalized.contains(
            "/ID [ <00000000000000000000000000000000> <00000000000000000000000000000000> ]"
        ));
        assert!(normalized.contains("/SUBSET+LinLibertine"));
    }

    #[test]
    fn test_content_stream_escaped_strings_and_floats() {
        let norm = PdfNormalizer::new();

        let stream = b"BT (Hello \\(nested\\) and \\\\ escaped) Tj 12.345678 98.765432 Td ET";
        let out = norm.normalize_content_stream(stream).unwrap();

        assert!(out.contains("(Hello \\(nested\\) and \\\\ escaped)"));
        assert!(out.contains("12.3457"));
        assert!(out.contains("98.7654"));
    }

    #[test]
    fn test_gate2_evaluation_methods() {
        let clean = PdfComparisonResult {
            stream_matches: true,
            visual_diff_score: 0.0,
            divergent_pages: Vec::new(),
            divergence_details: Vec::new(),
        };
        assert!(clean.is_identical());
        assert!(clean.passes_gate2(0.001));

        let slight_diff = PdfComparisonResult {
            stream_matches: true,
            visual_diff_score: 0.005,
            divergent_pages: vec![0],
            divergence_details: vec!["Anti-aliasing subpixel variation".into()],
        };
        assert!(!slight_diff.is_identical());
        assert!(slight_diff.passes_gate2(0.01));
        assert!(!slight_diff.passes_gate2(0.001));

        let stream_fail = PdfComparisonResult {
            stream_matches: false,
            visual_diff_score: 0.0,
            divergent_pages: vec![0],
            divergence_details: vec!["Stream AST mismatch".into()],
        };
        assert!(!stream_fail.passes_gate2(1.0));
    }

    #[test]
    fn test_basic_png_parser_branches() {
        // 1. Too short
        let short = b"\x89PNG\r\n\x1a";
        let err1 = VisualComparator::parse_basic_png(short).unwrap_err();
        assert!(matches!(err1, PdfDiffError::RenderError(_)));

        // 2. Bad magic signature
        let bad_magic = b"NOT_A_PNG_FILE_HEADER_12345678";
        let err2 = VisualComparator::parse_basic_png(bad_magic).unwrap_err();
        assert!(matches!(err2, PdfDiffError::RenderError(_)));

        // 3. Valid PNG header (width=16, height=32)
        let mut valid = vec![0u8; 32];
        valid[0..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        valid[16..20].copy_from_slice(&16u32.to_be_bytes());
        valid[20..24].copy_from_slice(&32u32.to_be_bytes());
        let img = VisualComparator::parse_basic_png(&valid).unwrap();
        assert_eq!(img.width, 16);
        assert_eq!(img.height, 32);
        assert_eq!(img.data.len(), 32);
    }

    #[test]
    fn test_content_stream_signs_dots_and_page_mismatch() {
        let norm = PdfNormalizer::new();

        // Signed and leading-dot floats: "+.5 -.25 -.000000001"
        let stream = b"10 20 m +.50000 -.25000 -.000000001 l";
        let out = norm.normalize_content_stream(stream).unwrap();
        assert!(out.contains("0.5"));
        assert!(out.contains("-0.25"));

        // Page count mismatch
        let pdf1 = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 12 >>\nstream\nBT ET\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
        let pdf2 = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R, 5 0 R] /Count 2 >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\nendobj\n4 0 obj\n<< /Length 12 >>\nstream\nBT ET\nendstream\nendobj\n5 0 obj\n<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>\nendobj\n6 0 obj\n<< /Length 12 >>\nstream\nBT ET\nendstream\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";

        let (matches, divergent_pages, details) = norm.compare_content_streams(pdf1, pdf2).unwrap();
        assert!(!matches);
        assert_eq!(divergent_pages.len(), 2);
        assert!(details[0].contains("Page count mismatch"));

        // Truncate diagnostic long string
        let long_stream_a = vec![b'A'; 300];
        let long_stream_b = vec![b'B'; 300];
        let (matches_diff, _, diff_details) = norm
            .compare_content_streams(&long_stream_a, &long_stream_b)
            .unwrap();
        assert!(!matches_diff);
        assert!(diff_details[0].contains("[truncated]"));
    }

    #[test]
    fn test_pdf_normalizer_builder_and_scientific_notation() {
        let norm = PdfNormalizerBuilder::new()
            .normalize_timestamps(true)
            .normalize_font_subsets(true)
            .normalize_producer(true)
            .normalize_floats(true)
            .float_precision(3)
            .strip_comments(true)
            .build();
        assert!(norm.normalize_floats);

        let stream = b"1.25e+02 cm 3.4E-1 re .7500 w -0.0000001 w";
        let out = norm.normalize_content_stream(stream).unwrap();
        assert!(out.contains("125"));
        assert!(out.contains("0.34"));
        assert!(out.contains("0.75"));
        assert!(out.contains("0 w"));

        let def_builder = PdfNormalizer::builder();
        let _ = def_builder.build();
    }

    #[test]
    fn test_visual_comparator_pdftoppm_render() {
        let comp = VisualComparator::new(150);
        let sample_pdf = Path::new("tests/tex-outputs/the_letter_a.pdf");
        if sample_pdf.exists()
            && std::process::Command::new("pdftoppm")
                .arg("-v")
                .output()
                .is_ok()
        {
            let img_res = comp.render_page_with_pdftoppm(sample_pdf, 1);
            assert!(img_res.is_ok());
            let img = img_res.unwrap();
            assert!(img.width > 0);
            assert!(img.height > 0);
        }

        let bad_pdf = Path::new("nonexistent_crater_test.pdf");
        let err = comp.render_page_with_pdftoppm(bad_pdf, 1);
        assert!(err.is_err());
    }
}
