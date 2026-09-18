// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Lightweight pure-Rust PDF trailer, /Pages tree, and bounding box parser.
//! Correctly resolves MediaBox/CropBox/ArtBox/TrimBox/BleedBox inheritance and /Rotate.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use thiserror::Error;

/// Error type encountered while probing PDF documents.
#[derive(Debug, Error)]
pub enum PdfProbeError {
    /// Standard I/O error occurred while reading the PDF file.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// File does not begin with the `%PDF-` header magic.
    #[error("Not a valid PDF file")]
    InvalidPdf,
    /// Specified page number was not found or out of bounds.
    #[error("Page {0} not found in PDF")]
    PageNotFound(usize),
    /// Requested bounding box was not found or could not be determined.
    #[error("Bounding box not found or invalid")]
    BoxNotFound,
}

/// PDF page boundary box types matching XeTeX conventions.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum PdfBoxType {
    /// CropBox boundary.
    Crop = 1,
    /// MediaBox boundary.
    Media = 2,
    /// BleedBox boundary.
    Bleed = 3,
    /// TrimBox boundary.
    Trim = 4,
    /// ArtBox boundary.
    Art = 5,
}

impl From<i32> for PdfBoxType {
    fn from(val: i32) -> Self {
        match val {
            2 => PdfBoxType::Media,
            3 => PdfBoxType::Bleed,
            4 => PdfBoxType::Trim,
            5 => PdfBoxType::Art,
            _ => PdfBoxType::Crop,
        }
    }
}

/// Probe the total page count of a PDF document at `path`.
pub fn probe_pdf_page_count<P: AsRef<Path>>(path: P) -> Result<usize, PdfProbeError> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    probe_pdf_page_count_bytes(&data)
}

/// Probe boundary box coordinates `(llx, lly, urx, ury)` for a page at `path`.
pub fn probe_pdf_box<P: AsRef<Path>>(
    path: P,
    page_num: usize,
    box_type: i32,
) -> Result<(f64, f64, f64, f64), PdfProbeError> {
    let mut file = File::open(path)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    probe_pdf_box_bytes(&data, page_num, PdfBoxType::from(box_type))
}

/// Probe the total page count from raw PDF bytes.
pub fn probe_pdf_page_count_bytes(data: &[u8]) -> Result<usize, PdfProbeError> {
    if !data.starts_with(b"%PDF-") {
        return Err(PdfProbeError::InvalidPdf);
    }
    let text = String::from_utf8_lossy(data);
    let doc = PdfDocIndex::parse(&text)?;
    Ok(doc.page_count)
}

/// Probe boundary box coordinates `(x0, y0, x1, y1)` from raw PDF bytes.
pub fn probe_pdf_box_bytes(
    data: &[u8],
    page_num: usize,
    box_type: PdfBoxType,
) -> Result<(f64, f64, f64, f64), PdfProbeError> {
    if !data.starts_with(b"%PDF-") {
        return Err(PdfProbeError::InvalidPdf);
    }
    let text = String::from_utf8_lossy(data);
    let doc = PdfDocIndex::parse(&text)?;
    let page_info = doc.get_page(page_num)?;

    let raw_box = match box_type {
        PdfBoxType::Media => page_info.media_box,
        PdfBoxType::Crop => page_info.crop_box.or(page_info.media_box),
        PdfBoxType::Bleed => page_info.bleed_box.or(page_info.crop_box).or(page_info.media_box),
        PdfBoxType::Trim => page_info.trim_box.or(page_info.crop_box).or(page_info.media_box),
        PdfBoxType::Art => page_info.art_box.or(page_info.crop_box).or(page_info.media_box),
    }
    .ok_or(PdfProbeError::BoxNotFound)?;

    let media = page_info.media_box.unwrap_or(raw_box);

    // Clamping to MediaBox as in xdvipdfmx/dpx-pdfdoc.c
    let mut llx = raw_box.0.max(media.0);
    let mut lly = raw_box.1.max(media.1);
    let mut urx = raw_box.2.min(media.2);
    let mut ury = raw_box.3.min(media.3);

    if llx > urx {
        std::mem::swap(&mut llx, &mut urx);
    }
    if lly > ury {
        std::mem::swap(&mut lly, &mut ury);
    }

    let orig_w = urx - llx;
    let orig_h = ury - lly;
    let rot = ((page_info.rotate % 360) + 360) % 360;

    // Apply rotation transformation matrix
    let (x0, y0, x1, y1) = match rot {
        90 | 270 => (llx, lly, llx + orig_h, lly + orig_w),
        _ => (llx, lly, llx + orig_w, lly + orig_h),
    };

    Ok((x0, y0, x1, y1))
}

#[derive(Debug, Default, Clone)]
struct PageAttrs {
    media_box: Option<(f64, f64, f64, f64)>,
    crop_box: Option<(f64, f64, f64, f64)>,
    bleed_box: Option<(f64, f64, f64, f64)>,
    trim_box: Option<(f64, f64, f64, f64)>,
    art_box: Option<(f64, f64, f64, f64)>,
    rotate: i32,
}

struct PdfDocIndex {
    page_count: usize,
    pages: Vec<PageAttrs>,
}

impl PdfDocIndex {
    fn parse(text: &str) -> Result<Self, PdfProbeError> {
        let mut objects: HashMap<u32, &str> = HashMap::new();
        let mut search_idx = 0;

        while let Some(obj_pos) = text[search_idx..].find(" obj") {
            let abs_pos = search_idx + obj_pos;
            let pre = text[..abs_pos].trim_end();
            let tokens: Vec<&str> = pre.split_whitespace().rev().take(2).collect();
            if tokens.len() == 2 {
                if let (Ok(_gen), Ok(num)) = (tokens[0].parse::<u32>(), tokens[1].parse::<u32>()) {
                    let content_start = abs_pos + 4;
                    if let Some(end_pos) = text[content_start..].find("endobj") {
                        let body = &text[content_start..content_start + end_pos];
                        objects.insert(num, body);
                        search_idx = content_start + end_pos + 6;
                        continue;
                    }
                }
            }
            search_idx = abs_pos + 4;
        }

        let mut catalog_id = None;
        for (&id, &body) in &objects {
            if body.contains("/Type /Catalog") || (body.contains("/Type/Catalog")) {
                catalog_id = Some(id);
                break;
            }
        }

        let mut root_pages_id = None;
        if let Some(cat_id) = catalog_id {
            if let Some(body) = objects.get(&cat_id) {
                root_pages_id = extract_ref(body, "/Pages");
            }
        }

        if root_pages_id.is_none() {
            for (&id, &body) in &objects {
                if (body.contains("/Type /Pages") || body.contains("/Type/Pages")) && body.contains("/Kids") {
                    root_pages_id = Some(id);
                    break;
                }
            }
        }

        let mut collected_pages = Vec::new();
        if let Some(pages_id) = root_pages_id {
            let mut inherited = PageAttrs::default();
            collect_pages_tree(pages_id, &objects, &mut inherited, &mut collected_pages, 0);
        }

        if collected_pages.is_empty() {
            // Fallback: scan all /Type /Page objects directly
            for (&_id, &body) in &objects {
                if (body.contains("/Type /Page") || body.contains("/Type/Page")) && !body.contains("/Pages") {
                    let mut attrs = PageAttrs::default();
                    attrs.media_box = extract_box(body, "/MediaBox");
                    attrs.crop_box = extract_box(body, "/CropBox");
                    attrs.bleed_box = extract_box(body, "/BleedBox");
                    attrs.trim_box = extract_box(body, "/TrimBox");
                    attrs.art_box = extract_box(body, "/ArtBox");
                    attrs.rotate = extract_int(body, "/Rotate").unwrap_or(0);
                    collected_pages.push(attrs);
                }
            }
        }

        let page_count = collected_pages.len();
        if page_count == 0 {
            return Err(PdfProbeError::InvalidPdf);
        }

        Ok(PdfDocIndex {
            page_count,
            pages: collected_pages,
        })
    }

    fn get_page(&self, page_num: usize) -> Result<&PageAttrs, PdfProbeError> {
        if page_num == 0 || page_num > self.pages.len() {
            return Err(PdfProbeError::PageNotFound(page_num));
        }
        Ok(&self.pages[page_num - 1])
    }
}

fn collect_pages_tree(
    node_id: u32,
    objects: &HashMap<u32, &str>,
    inherited: &PageAttrs,
    out_pages: &mut Vec<PageAttrs>,
    depth: usize,
) {
    if depth > 32 {
        return;
    }
    let body = match objects.get(&node_id) {
        Some(b) => *b,
        None => return,
    };

    let mut cur_attrs = inherited.clone();
    if let Some(mb) = extract_box(body, "/MediaBox") {
        cur_attrs.media_box = Some(mb);
    }
    if let Some(cb) = extract_box(body, "/CropBox") {
        cur_attrs.crop_box = Some(cb);
    }
    if let Some(bb) = extract_box(body, "/BleedBox") {
        cur_attrs.bleed_box = Some(bb);
    }
    if let Some(tb) = extract_box(body, "/TrimBox") {
        cur_attrs.trim_box = Some(tb);
    }
    if let Some(ab) = extract_box(body, "/ArtBox") {
        cur_attrs.art_box = Some(ab);
    }
    if let Some(rot) = extract_int(body, "/Rotate") {
        cur_attrs.rotate = rot;
    }

    let is_leaf = body.contains("/Type /Page") && !body.contains("/Kids");
    if is_leaf {
        out_pages.push(cur_attrs);
        return;
    }

    if let Some(kids_start) = body.find("/Kids") {
        let slice = &body[kids_start..];
        if let (Some(l_bracket), Some(r_bracket)) = (slice.find('['), slice.find(']')) {
            let kids_str = &slice[l_bracket + 1..r_bracket];
            let tokens: Vec<&str> = kids_str.split_whitespace().collect();
            let mut i = 0;
            while i + 2 <= tokens.len() {
                if tokens[i + 1] == "0" && (tokens[i + 2] == "R" || tokens[i + 2] == "R,") {
                    if let Ok(kid_id) = tokens[i].parse::<u32>() {
                        collect_pages_tree(kid_id, objects, &cur_attrs, out_pages, depth + 1);
                    }
                    i += 3;
                } else if tokens[i + 2] == "R" {
                    if let Ok(kid_id) = tokens[i].parse::<u32>() {
                        collect_pages_tree(kid_id, objects, &cur_attrs, out_pages, depth + 1);
                    }
                    i += 3;
                } else {
                    i += 1;
                }
            }
        }
    } else if cur_attrs.media_box.is_some() {
        out_pages.push(cur_attrs);
    }
}

fn extract_ref(text: &str, key: &str) -> Option<u32> {
    if let Some(pos) = text.find(key) {
        let slice = &text[pos + key.len()..];
        let tokens: Vec<&str> = slice.split_whitespace().take(3).collect();
        if tokens.len() >= 3 && tokens[2] == "R" {
            return tokens[0].parse::<u32>().ok();
        }
    }
    None
}

fn extract_box(text: &str, key: &str) -> Option<(f64, f64, f64, f64)> {
    let pos = text.find(key)?;
    let slice = &text[pos + key.len()..];
    let start = slice.find('[')?;
    let end = slice[start..].find(']')?;
    let inner = &slice[start + 1..start + end];
    let nums: Vec<f64> = inner
        .split_whitespace()
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();
    if nums.len() == 4 {
        Some((nums[0], nums[1], nums[2], nums[3]))
    } else {
        None
    }
}

fn extract_int(text: &str, key: &str) -> Option<i32> {
    let pos = text.find(key)?;
    let slice = &text[pos + key.len()..];
    let token = slice.split_whitespace().next()?;
    token.trim_matches(|c: char| !c.is_ascii_digit() && c != '-').parse::<i32>().ok()
}
