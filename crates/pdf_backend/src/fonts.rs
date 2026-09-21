// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Font registration, mapping, and PDF dictionary generation.

use crate::units::sp_to_bp;
use std::collections::HashMap;

/// A font registered during XDV processing.
#[derive(Clone, Debug)]
pub struct FontEntry {
    /// The unique integer identifier assigned in XDV (e.g. 0, 1, ...).
    pub font_num: i32,
    /// The font name or file path.
    pub name: String,
    /// The PostScript name of the font.
    pub ps_name: String,
    /// Font size in PDF points (`bp`).
    pub size_bp: f64,
    /// PDF resource name assigned to this font (e.g. "F1", "F2").
    pub pdf_name: String,
    /// Face index within a font collection (TTC/OTC).
    pub face_index: u32,
    /// Font color RGBA if specified.
    pub color_rgba: Option<u32>,
    /// Slant factor (fixed point 16.16) if specified.
    pub slant: Option<u32>,
    /// Extend factor (fixed point 16.16) if specified.
    pub extend: Option<u32>,
    /// Whether this font was defined as a native OpenType/TrueType font.
    pub is_native: bool,
}

/// Registry of fonts used in a document.
#[derive(Default, Debug)]
pub struct FontRegistry {
    fonts: HashMap<i32, FontEntry>,
    next_pdf_id: usize,
}

impl FontRegistry {
    /// Creates a new empty font registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a native OpenType/TrueType font.
    #[allow(clippy::too_many_arguments)]
    pub fn define_native_font(
        &mut self,
        font_num: i32,
        name: &str,
        size_sp: i32,
        face_index: u32,
        color_rgba: Option<u32>,
        extend: Option<u32>,
        slant: Option<u32>,
        _embolden: Option<u32>,
    ) -> &FontEntry {
        let pdf_name = format!("F{}", self.next_pdf_id + 1);
        self.next_pdf_id += 1;

        // Clean name for PostScript name
        let ps_name = sanitize_ps_name(name);
        let size_bp = sp_to_bp(size_sp);

        let entry = FontEntry {
            font_num,
            name: name.to_string(),
            ps_name,
            size_bp,
            pdf_name,
            face_index,
            color_rgba,
            slant,
            extend,
            is_native: true,
        };

        self.fonts.insert(font_num, entry);
        self.fonts.get(&font_num).unwrap()
    }

    /// Registers a traditional TeX font (e.g. CMR10).
    pub fn define_tex_font(&mut self, font_num: i32, name: &str, size_sp: i32) -> &FontEntry {
        let pdf_name = format!("F{}", self.next_pdf_id + 1);
        self.next_pdf_id += 1;

        let ps_name = sanitize_ps_name(name);
        let size_bp = sp_to_bp(size_sp);

        let entry = FontEntry {
            font_num,
            name: name.to_string(),
            ps_name,
            size_bp,
            pdf_name,
            face_index: 0,
            color_rgba: None,
            slant: None,
            extend: None,
            is_native: false,
        };

        self.fonts.insert(font_num, entry);
        self.fonts.get(&font_num).unwrap()
    }

    /// Looks up a registered font by its XDV font number.
    pub fn get(&self, font_num: i32) -> Option<&FontEntry> {
        self.fonts.get(&font_num)
    }

    /// Returns an iterator over all registered fonts.
    pub fn iter(&self) -> impl Iterator<Item = &FontEntry> {
        self.fonts.values()
    }

    /// Returns true if the given font number is registered.
    pub fn contains(&self, font_num: i32) -> bool {
        self.fonts.contains_key(&font_num)
    }

    /// Returns the number of registered fonts.
    pub fn len(&self) -> usize {
        self.fonts.len()
    }

    /// Returns true if no fonts have been registered.
    pub fn is_empty(&self) -> bool {
        self.fonts.is_empty()
    }
}

/// Sanitizes a font name into a valid PostScript name without spaces or forbidden characters.
fn sanitize_ps_name(name: &str) -> String {
    let base = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .rsplit('\\')
        .next()
        .unwrap_or(name);
    let mut ps = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            ps.push(c);
        } else if c == '.' {
            break; // Strip extension
        }
    }
    if ps.is_empty() {
        "DefaultFont".to_string()
    } else {
        ps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_font_registry() {
        let mut reg = FontRegistry::new();
        assert!(reg.is_empty());

        let f1 = reg.define_native_font(
            0,
            "LiberationSans-Regular.ttf",
            655360, // 10 pt
            0,
            None,
            None,
            None,
            None,
        );
        assert_eq!(f1.pdf_name, "F1");
        assert_eq!(f1.ps_name, "LiberationSans-Regular");
        assert!((f1.size_bp - (10.0 * 72.0 / 72.27)).abs() < 1e-4);

        let lookup = reg.get(0).expect("font 0 should exist");
        assert_eq!(lookup.name, "LiberationSans-Regular.ttf");
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn test_sanitize_ps_name() {
        assert_eq!(
            sanitize_ps_name("/usr/share/fonts/NimbusRom.otf"),
            "NimbusRom"
        );
        assert_eq!(sanitize_ps_name("cmr10"), "cmr10");
        assert_eq!(sanitize_ps_name("Font With Spaces!"), "FontWithSpaces");
        assert_eq!(sanitize_ps_name(""), "DefaultFont");
    }
}
