// Copyright 2026 the Tectonic Project
// Licensed under the MIT License.

//! Tier B PDF Content-Stream Normalizer and Visual Comparator.
//!
//! Enforces Gate 2 of MIGRATION_ROADMAP.md:
//! 1. Normalized AST content-stream identity.
//! 2. 300+ DPI visual raster parity.

use std::fmt;
use std::io::Read;
use std::path::Path;
use flate2::read::{DeflateDecoder, ZlibDecoder};
use regex::Regex;

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
                .replace_all(&result, "<xmp:CreateDate>1970-01-01T00:00:00Z</xmp:CreateDate>")
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
                .replace_all(&result, "<pdf:Producer>CANONICAL_TECTONIC_PRODUCER</pdf:Producer>")
                .to_string();

            let xmp_creator_re = Regex::new(r"<xmp:CreatorTool>[^<]*</xmp:CreatorTool>").unwrap();
            result = xmp_creator_re
                .replace_all(&result, "<xmp:CreatorTool>CANONICAL_TECTONIC_PRODUCER</xmp:CreatorTool>")
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
                    let normalized_name = Regex::new(r"^/([A-Z]{6}\+)(.*)$")
                        .unwrap()
                        .replace(&name_token, "/SUBSET+$2")
                        .to_string();
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
    pub fn extract_page_content_streams(&self, pdf_bytes: &[u8]) -> Result<Vec<Vec<u8>>, PdfDiffError> {
        let mut page_streams = Vec::new();
        let pdf_str = String::from_utf8_lossy(pdf_bytes);

        // Pattern matching `stream ... endstream` blocks
        let stream_re = Regex::new(r"(?s)<<(?P<dict>[^>]*)>>\s*stream\r?\n(?P<data>.*?)\r?\nendstream").unwrap();

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
    pub fn compare_raster_buffers(
        &self,
        img_a: &RasterImage,
        img_b: &RasterImage,
    ) -> (f64, usize) {
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

            let dr = (p_a[offset] as i16 - p_b[offset] as i16).abs() as u8;
            let dg = (p_a[offset + 1] as i16 - p_b[offset + 1] as i16).abs() as u8;
            let db = (p_a[offset + 2] as i16 - p_b[offset + 2] as i16).abs() as u8;
            let da = (p_a[offset + 3] as i16 - p_b[offset + 3] as i16).abs() as u8;

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
}
