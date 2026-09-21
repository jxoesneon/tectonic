// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! XDV event pipeline that converts XDV commands into a PDF document.

use pdf_writer::Ref;
use tectonic_xdv::{FileType, XdvError, XdvEvents};

use crate::fonts::FontRegistry;
use crate::pdf::PdfDocumentBuilder;
use crate::specials::{parse_special, Color, ParsedSpecial};
use crate::units::{dvi_to_pdf_coords, PdfRect, DEFAULT_PAGE_HEIGHT_BP, DEFAULT_PAGE_WIDTH_BP};

/// Pipeline errors.
#[derive(thiserror::Error, Debug)]
pub enum PipelineError {
    /// XDV parsing error.
    #[error("XDV error: {0}")]
    Xdv(#[from] XdvError),
    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// General pipeline error.
    #[error("{0}")]
    Other(String),
}

/// The stateful XDV-to-PDF event processor.
pub struct XdvToPdfPipeline {
    builder: PdfDocumentBuilder,
    registry: FontRegistry,
    current_content: Option<Vec<u8>>,
    current_content_id: Option<Ref>,
    page_width_bp: f64,
    page_height_bp: f64,
    page_count: usize,
    active_font_num: Option<i32>,
    color_stack: Vec<Color>,
    eop_actions: Vec<String>,
}

impl Default for XdvToPdfPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl XdvToPdfPipeline {
    /// Initializes a new pipeline with default page dimensions (US Letter).
    pub fn new() -> Self {
        Self {
            builder: PdfDocumentBuilder::new(),
            registry: FontRegistry::new(),
            current_content: None,
            current_content_id: None,
            page_width_bp: DEFAULT_PAGE_WIDTH_BP,
            page_height_bp: DEFAULT_PAGE_HEIGHT_BP,
            page_count: 0,
            active_font_num: None,
            color_stack: Vec::new(),
            eop_actions: Vec::new(),
        }
    }

    /// Sets the initial default page dimensions in PDF points (`bp`).
    pub fn set_page_dimensions(&mut self, width_bp: f64, height_bp: f64) {
        self.page_width_bp = width_bp;
        self.page_height_bp = height_bp;
    }

    /// Flushes any active page content stream into the PDF builder.
    fn flush_active_page(&mut self) {
        if let (Some(mut content), Some(content_id)) =
            (self.current_content.take(), self.current_content_id.take())
        {
            // Append any pending end-of-page actions
            for eop in &self.eop_actions {
                content.extend_from_slice(eop.as_bytes());
                content.push(b'\n');
            }
            self.eop_actions.clear();

            self.builder.write_page_content(content_id, &content);
        }
    }

    /// Finishes processing and produces the complete PDF document bytes.
    pub fn finish(mut self) -> Result<Vec<u8>, PipelineError> {
        self.flush_active_page();
        Ok(self.builder.finish(&self.registry))
    }
}

/// Escapes special PDF string literal characters: `\`, `(`, `)`.
fn escape_pdf_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

impl XdvEvents for XdvToPdfPipeline {
    type Error = PipelineError;

    fn handle_header(&mut self, _filetype: FileType, _comment: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }

    fn handle_begin_page(
        &mut self,
        _counters: &[i32],
        _previous_bop: i32,
    ) -> Result<(), Self::Error> {
        self.flush_active_page();

        let (_page_id, content_id) = self
            .builder
            .begin_page(self.page_width_bp, self.page_height_bp);
        self.current_content = Some(Vec::new());
        self.current_content_id = Some(content_id);
        self.page_count += 1;
        self.active_font_num = None;
        self.color_stack.clear();
        self.eop_actions.clear();

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
        self.registry.define_native_font(
            font_num, name, size, face_index, color_rgba, extend, slant, embolden,
        );
        Ok(())
    }

    fn handle_rule(&mut self, x: i32, y: i32, height: i32, width: i32) -> Result<(), Self::Error> {
        if height <= 0 || width <= 0 {
            return Ok(());
        }

        if let Some(content) = self.current_content.as_mut() {
            let rect = PdfRect::from_dvi_rule(x, y, width, height, self.page_height_bp);
            let s = format!(
                "{:.4} {:.4} {:.4} {:.4} re\nf\n",
                rect.x, rect.y, rect.width, rect.height
            );
            content.extend_from_slice(s.as_bytes());
        }

        Ok(())
    }

    fn handle_text_and_glyphs(
        &mut self,
        font_num: i32,
        text: &str,
        _width: i32,
        _glyphs: &[u16],
        x: &[i32],
        y: &[i32],
    ) -> Result<(), Self::Error> {
        if x.is_empty() || y.is_empty() {
            return Ok(());
        }

        let Some(font) = self.registry.get(font_num) else {
            return Ok(());
        };

        let font_pdf_name = font.pdf_name.clone();
        let font_size_bp = font.size_bp;

        if let Some(content) = self.current_content.as_mut() {
            let (pdf_x, pdf_y) = dvi_to_pdf_coords(x[0], y[0], self.page_height_bp);
            let escaped = escape_pdf_string(text);
            let s = format!(
                "BT\n/{} {:.4} Tf\n1 0 0 1 {:.4} {:.4} Tm\n({}) Tj\nET\n",
                font_pdf_name, font_size_bp, pdf_x, pdf_y, escaped
            );
            content.extend_from_slice(s.as_bytes());
        }

        self.active_font_num = Some(font_num);
        Ok(())
    }

    fn handle_glyph_run(
        &mut self,
        font_num: i32,
        glyphs: &[u16],
        x: &[i32],
        y: &[i32],
    ) -> Result<(), Self::Error> {
        if x.is_empty() || y.is_empty() {
            return Ok(());
        }

        let Some(font) = self.registry.get(font_num) else {
            return Ok(());
        };

        let font_pdf_name = font.pdf_name.clone();
        let font_size_bp = font.size_bp;

        if let Some(content) = self.current_content.as_mut() {
            content.extend_from_slice(b"BT\n");
            let tf = format!("/{} {:.4} Tf\n", font_pdf_name, font_size_bp);
            content.extend_from_slice(tf.as_bytes());

            if glyphs.is_empty() {
                let (pdf_x, pdf_y) = dvi_to_pdf_coords(x[0], y[0], self.page_height_bp);
                let s = format!("1 0 0 1 {:.4} {:.4} Tm\n", pdf_x, pdf_y);
                content.extend_from_slice(s.as_bytes());
            } else {
                for (i, &gid) in glyphs.iter().enumerate() {
                    let gx = x.get(i).copied().unwrap_or(x[0]);
                    let gy = y.get(i).copied().unwrap_or(y[0]);
                    let (pdf_x, pdf_y) = dvi_to_pdf_coords(gx, gy, self.page_height_bp);
                    let op = format!("1 0 0 1 {:.4} {:.4} Tm\n<{:04x}> Tj\n", pdf_x, pdf_y, gid);
                    content.extend_from_slice(op.as_bytes());
                }
            }

            content.extend_from_slice(b"ET\n");
        }

        self.active_font_num = Some(font_num);
        Ok(())
    }

    fn handle_char_run(&mut self, font_num: i32, chars: &[i32]) -> Result<(), Self::Error> {
        if !self.registry.contains(font_num) {
            self.registry.define_tex_font(font_num, "cmr10", 655360);
        }
        let text: String = chars
            .iter()
            .filter_map(|&c| char::from_u32(c as u32))
            .collect();
        if let Some(content) = self.current_content.as_mut() {
            let font = self.registry.get(font_num).unwrap();
            let font_pdf_name = font.pdf_name.clone();
            let font_size_bp = font.size_bp;
            let escaped = escape_pdf_string(&text);
            let s = format!(
                "BT\n/{} {:.4} Tf\n({}) Tj\nET\n",
                font_pdf_name, font_size_bp, escaped
            );
            content.extend_from_slice(s.as_bytes());
        }
        Ok(())
    }

    fn handle_special(&mut self, _x: i32, _y: i32, contents: &[u8]) -> Result<(), Self::Error> {
        match parse_special(contents) {
            ParsedSpecial::PaperSize {
                width_bp,
                height_bp,
            } => {
                self.page_width_bp = width_bp;
                self.page_height_bp = height_bp;
            }
            ParsedSpecial::ColorPush(color) => {
                self.color_stack.push(color);
                if let Some(content) = self.current_content.as_mut() {
                    let (stroke, fill) = color.to_pdf_operators();
                    content.extend_from_slice(stroke.as_bytes());
                    content.push(b'\n');
                    content.extend_from_slice(fill.as_bytes());
                    content.push(b'\n');
                }
            }
            ParsedSpecial::ColorPop => {
                self.color_stack.pop();
                let restore_color = self.color_stack.last().copied().unwrap_or(Color::Gray(0.0));
                if let Some(content) = self.current_content.as_mut() {
                    let (stroke, fill) = restore_color.to_pdf_operators();
                    content.extend_from_slice(stroke.as_bytes());
                    content.push(b'\n');
                    content.extend_from_slice(fill.as_bytes());
                    content.push(b'\n');
                }
            }
            ParsedSpecial::PdfBop(raw) => {
                if let Some(content) = self.current_content.as_mut() {
                    content.extend_from_slice(raw.as_bytes());
                    content.push(b'\n');
                }
            }
            ParsedSpecial::PdfEop(raw) => {
                self.eop_actions.push(raw.to_string());
            }
            ParsedSpecial::PdfContent(raw) => {
                if let Some(content) = self.current_content.as_mut() {
                    content.extend_from_slice(raw.as_bytes());
                    content.push(b'\n');
                }
            }
            ParsedSpecial::Other(_) => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_basic_lifecycle() {
        let mut pipeline = XdvToPdfPipeline::new();
        pipeline
            .handle_header(FileType::Xdv, b"test comment")
            .unwrap();
        pipeline.handle_begin_page(&[1], -1).unwrap();

        // Add a rule
        pipeline.handle_rule(0, 0, 65536, 131072).unwrap();

        // Add a native font and text
        pipeline
            .handle_define_native_font("Times-Roman", 1, 655360, 0, None, None, None, None)
            .unwrap();
        pipeline
            .handle_text_and_glyphs(1, "Hello FerroTeX", 655360, &[0], &[0], &[0])
            .unwrap();

        let pdf_bytes = pipeline.finish().unwrap();
        assert!(pdf_bytes.starts_with(b"%PDF-"));
        let pdf_str = String::from_utf8_lossy(&pdf_bytes);
        assert!(pdf_str.contains("/Type /Page"));
        assert!(pdf_str.contains("Hello FerroTeX"));
    }

    #[test]
    fn test_pipeline_color_and_papersize_specials() {
        let mut pipeline = XdvToPdfPipeline::new();
        pipeline
            .handle_special(0, 0, b"papersize=210mm,297mm")
            .unwrap();
        pipeline.handle_begin_page(&[1], -1).unwrap();

        pipeline
            .handle_special(0, 0, b"color push rgb 1 0 0")
            .unwrap();
        pipeline.handle_rule(0, 0, 65536, 65536).unwrap();
        pipeline.handle_special(0, 0, b"color pop").unwrap();

        let pdf_bytes = pipeline.finish().unwrap();
        assert!(pdf_bytes.starts_with(b"%PDF-"));
        let pdf_str = String::from_utf8_lossy(&pdf_bytes);
        assert!(pdf_str.contains("1.0000 0.0000 0.0000 rg"));
    }

    #[test]
    fn test_escape_pdf_string() {
        assert_eq!(
            escape_pdf_string("Hello (World) \\ Test"),
            "Hello \\(World\\) \\\\ Test"
        );
        assert_eq!(
            escape_pdf_string("Line1\r\nLine2\tTab"),
            "Line1\\r\\nLine2\\tTab"
        );
    }

    #[test]
    fn test_pipeline_edge_cases() {
        let mut pipeline = XdvToPdfPipeline::default();
        pipeline.handle_begin_page(&[1], -1).unwrap();

        // 1. Zero/negative rules (should be no-op)
        pipeline.handle_rule(0, 0, 0, 100).unwrap();
        pipeline.handle_rule(0, 0, 100, -50).unwrap();

        // 2. Empty coordinates in text and glyphs (should be no-op)
        pipeline
            .handle_text_and_glyphs(1, "Text", 100, &[0], &[], &[])
            .unwrap();
        pipeline.handle_glyph_run(1, &[0], &[], &[]).unwrap();

        // 3. Unknown font in text and glyphs (should be no-op)
        pipeline
            .handle_text_and_glyphs(999, "Unknown", 100, &[0], &[0], &[0])
            .unwrap();
        pipeline.handle_glyph_run(999, &[0], &[0], &[0]).unwrap();

        // 4. Register font and run glyph run
        pipeline
            .handle_define_native_font("Courier", 2, 655360, 0, None, None, None, None)
            .unwrap();
        pipeline.handle_glyph_run(2, &[0], &[0], &[0]).unwrap();

        // 5. Char run with automatic CMR10 definition
        pipeline.handle_char_run(3, &[72, 105]).unwrap(); // "Hi"

        // 6. Specials: pdf:bop, pdf:eop, pdf:content, unknown
        pipeline.handle_special(0, 0, b"pdf:bop q").unwrap();
        pipeline
            .handle_special(0, 0, b"pdf:content 1 0 0 1 0 0 cm")
            .unwrap();
        pipeline.handle_special(0, 0, b"pdf:eop Q").unwrap();
        pipeline.handle_special(0, 0, b"unhandled_special").unwrap();

        // 7. Finish page and document
        let bytes = pipeline.finish().unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let pdf_str = String::from_utf8_lossy(&bytes);
        assert!(pdf_str.contains("Hi"));
        assert!(pdf_str.contains("Q"));
    }
}
