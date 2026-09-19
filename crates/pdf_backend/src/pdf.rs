// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Declarative PDF document writer utilizing `pdf-writer`.

use pdf_writer::types::{CidFontType, FontFlags, SystemInfo};
use pdf_writer::{Finish, Name, Pdf, Rect, Ref, Str};
use std::collections::HashMap;
use crate::fonts::FontRegistry;
use crate::units::PdfRect;

/// Manages PDF object IDs and builds the final PDF document.
pub struct PdfDocumentBuilder {
    pdf: Pdf,
    catalog_id: Ref,
    pages_id: Ref,
    next_id: i32,
    page_ids: Vec<Ref>,
    content_ids: Vec<Ref>,
    font_ids: HashMap<String, Ref>,
    page_boxes: Vec<PdfRect>,
}

impl Default for PdfDocumentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfDocumentBuilder {
    /// Initializes a new PDF document builder.
    pub fn new() -> Self {
        Self {
            pdf: Pdf::new(),
            catalog_id: Ref::new(1),
            pages_id: Ref::new(2),
            next_id: 3,
            page_ids: Vec::new(),
            content_ids: Vec::new(),
            font_ids: HashMap::new(),
            page_boxes: Vec::new(),
        }
    }

    /// Allocates the next available PDF object reference ID.
    pub fn alloc_ref(&mut self) -> Ref {
        let id = self.next_id;
        self.next_id += 1;
        Ref::new(id)
    }

    /// Begins a new page with the specified width and height in points (`bp`).
    /// Returns the allocated page object ID and content stream ID.
    pub fn begin_page(&mut self, width_bp: f64, height_bp: f64) -> (Ref, Ref) {
        let page_id = self.alloc_ref();
        let content_id = self.alloc_ref();

        self.page_ids.push(page_id);
        self.content_ids.push(content_id);
        self.page_boxes.push(PdfRect::new(0.0, 0.0, width_bp, height_bp));

        (page_id, content_id)
    }

    /// Writes a page content stream.
    pub fn write_page_content(&mut self, content_id: Ref, content: &[u8]) {
        self.pdf.stream(content_id, content);
    }

    /// Registers fonts from the font registry into the PDF writer.
    pub fn write_fonts(&mut self, registry: &FontRegistry) {
        for font in registry.iter() {
            if self.font_ids.contains_key(&font.pdf_name) {
                continue;
            }

            if font.is_native {
                let cid_font_ref = self.alloc_ref();
                let descriptor_ref = self.alloc_ref();
                let font_ref = self.alloc_ref();
                self.font_ids.insert(font.pdf_name.clone(), font_ref);

                // 1. Type 0 composite font
                {
                    let mut type0 = self.pdf.type0_font(font_ref);
                    type0.base_font(Name(font.ps_name.as_bytes()));
                    type0.encoding_predefined(Name(b"Identity-H"));
                    type0.descendant_font(cid_font_ref);
                }

                // 2. Descendant CIDFont
                {
                    let mut cid = self.pdf.cid_font(cid_font_ref);
                    cid.subtype(CidFontType::Type2);
                    cid.base_font(Name(font.ps_name.as_bytes()));
                    cid.system_info(SystemInfo {
                        registry: Str(b"Adobe"),
                        ordering: Str(b"Identity"),
                        supplement: 0,
                    });
                    cid.font_descriptor(descriptor_ref);
                    cid.default_width(1000.0);
                    cid.cid_to_gid_map_predefined(Name(b"Identity"));
                }

                // 3. Font descriptor
                {
                    let mut desc = self.pdf.font_descriptor(descriptor_ref);
                    desc.name(Name(font.ps_name.as_bytes()));
                    desc.flags(FontFlags::NON_SYMBOLIC);
                    desc.italic_angle(0.0);
                }
            } else {
                let font_ref = self.alloc_ref();
                self.font_ids.insert(font.pdf_name.clone(), font_ref);

                // Write Type1 font dictionary
                let mut font_dict = self.pdf.type1_font(font_ref);
                font_dict.base_font(Name(font.ps_name.as_bytes()));
            }
        }
    }

    /// Finalizes the PDF structure (Catalog, Pages tree, Page objects) and produces
    /// the serialized PDF byte stream.
    pub fn finish(mut self, registry: &FontRegistry) -> Vec<u8> {
        self.write_fonts(registry);

        // 1. Catalog dictionary
        self.pdf.catalog(self.catalog_id).pages(self.pages_id);

        // 2. Pages dictionary
        let mut pages = self.pdf.pages(self.pages_id);
        pages.count(self.page_ids.len() as i32);
        pages.kids(self.page_ids.iter().copied());
        pages.finish();

        // 3. Page dictionaries
        for i in 0..self.page_ids.len() {
            let page_id = self.page_ids[i];
            let content_id = self.content_ids[i];
            let bbox = self.page_boxes[i];

            let mut page = self.pdf.page(page_id);
            page.parent(self.pages_id);
            page.media_box(Rect::new(
                bbox.x as f32,
                bbox.y as f32,
                (bbox.x + bbox.width) as f32,
                (bbox.y + bbox.height) as f32,
            ));
            page.contents(content_id);

            let mut resources = page.resources();
            let mut font_dict = resources.fonts();
            for (name, font_ref) in &self.font_ids {
                font_dict.pair(Name(name.as_bytes()), *font_ref);
            }
            font_dict.finish();
            resources.finish();
            page.finish();
        }

        self.pdf.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pdf_writer::Content;
    use crate::units::{DEFAULT_PAGE_HEIGHT_BP, DEFAULT_PAGE_WIDTH_BP};

    #[test]
    fn test_pdf_document_builder_empty() {
        let builder = PdfDocumentBuilder::new();
        let registry = FontRegistry::new();
        let bytes = builder.finish(&registry);
        assert!(!bytes.is_empty());
        assert!(bytes.starts_with(b"%PDF-"));
    }

    #[test]
    fn test_pdf_document_builder_with_page() {
        let mut builder = PdfDocumentBuilder::new();
        let (_page_id, content_id) = builder.begin_page(DEFAULT_PAGE_WIDTH_BP, DEFAULT_PAGE_HEIGHT_BP);

        let mut content = Content::new();
        content.rect(72.0, 72.0, 100.0, 200.0);
        content.fill_even_odd();
        builder.write_page_content(content_id, content.as_bytes());

        let mut registry = FontRegistry::new();
        registry.define_tex_font(0, "cmr10", 655360);

        let bytes = builder.finish(&registry);
        assert!(bytes.starts_with(b"%PDF-"));
        let pdf_str = String::from_utf8_lossy(&bytes);
        assert!(pdf_str.contains("/Type /Catalog"));
        assert!(pdf_str.contains("/Type /Pages"));
        assert!(pdf_str.contains("/Type /Page"));
    }

    #[test]
    fn test_pdf_document_builder_default_and_duplicate_font() {
        let mut builder = PdfDocumentBuilder::default();
        let mut registry = FontRegistry::new();
        registry.define_tex_font(0, "cmr10", 655360);
        builder.write_fonts(&registry);
        // Second call exercises the continue branch for already registered fonts
        builder.write_fonts(&registry);
        let bytes = builder.finish(&registry);
        assert!(bytes.starts_with(b"%PDF-"));
    }
}
