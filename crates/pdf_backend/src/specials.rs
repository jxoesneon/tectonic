// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Parser and dispatcher for TeX and XeTeX DVI/XDV `\special` commands.

/// Parsed special command representation.
#[derive(Clone, Debug, PartialEq)]
pub enum ParsedSpecial<'a> {
    /// Sets the paper dimensions for the page/document.
    PaperSize {
        /// Page width in PDF points.
        width_bp: f64,
        /// Page height in PDF points.
        height_bp: f64,
    },
    /// Pushes a color onto the color stack.
    ColorPush(Color),
    /// Pops the top color off the color stack.
    ColorPop,
    /// Raw PDF code to insert at page beginning (bop).
    PdfBop(&'a str),
    /// Raw PDF code to insert at page end (eop).
    PdfEop(&'a str),
    /// Generic PDF code insertion.
    PdfContent(&'a str),
    /// Other unhandled or comment specials.
    Other(&'a str),
}

/// Color representation for PDF graphics state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Color {
    /// Grayscale (0.0 = black, 1.0 = white).
    Gray(f32),
    /// RGB color components in [0.0, 1.0].
    Rgb(f32, f32, f32),
    /// CMYK color components in [0.0, 1.0].
    Cmyk(f32, f32, f32, f32),
}

impl Color {
    /// Emits the PDF operator string for setting stroke and fill color.
    pub fn to_pdf_operators(&self) -> (String, String) {
        match *self {
            Color::Gray(g) => (format!("{g:.4} G"), format!("{g:.4} g")),
            Color::Rgb(r, g, b) => (
                format!("{r:.4} {g:.4} {b:.4} RG"),
                format!("{r:.4} {g:.4} {b:.4} rg"),
            ),
            Color::Cmyk(c, m, y, k) => (
                format!("{c:.4} {m:.4} {y:.4} {k:.4} K"),
                format!("{c:.4} {m:.4} {y:.4} {k:.4} k"),
            ),
        }
    }
}

/// Parses a dimension string with unit (e.g., "8.5in", "210mm", "100pt", "500bp").
/// Returns the value converted to PDF points (`bp`).
pub fn parse_dimen_to_bp(s: &str) -> Option<f64> {
    let s = s.trim();
    if s.ends_with("in") {
        let val: f64 = s[..s.len() - 2].trim().parse().ok()?;
        Some(val * 72.0)
    } else if s.ends_with("bp") {
        s[..s.len() - 2].trim().parse().ok()
    } else if s.ends_with("pt") {
        let val: f64 = s[..s.len() - 2].trim().parse().ok()?;
        Some(val * 72.0 / 72.27)
    } else if s.ends_with("mm") {
        let val: f64 = s[..s.len() - 2].trim().parse().ok()?;
        Some(val * 72.0 / 25.4)
    } else if s.ends_with("cm") {
        let val: f64 = s[..s.len() - 2].trim().parse().ok()?;
        Some(val * 72.0 / 2.54)
    } else {
        // Fallback: try raw number as bp
        s.parse().ok()
    }
}

/// Parses a raw special byte slice into a structured [`ParsedSpecial`].
pub fn parse_special(bytes: &[u8]) -> ParsedSpecial<'_> {
    let Ok(s) = std::str::from_utf8(bytes) else {
        return ParsedSpecial::Other("");
    };
    let s = s.trim();

    // 1. Check "papersize=WIDTH,HEIGHT"
    if let Some(rest) = s.strip_prefix("papersize=") {
        let parts: Vec<&str> = rest.split(',').collect();
        if parts.len() == 2 {
            if let (Some(w), Some(h)) = (parse_dimen_to_bp(parts[0]), parse_dimen_to_bp(parts[1])) {
                return ParsedSpecial::PaperSize {
                    width_bp: w,
                    height_bp: h,
                };
            }
        }
    }

    // 2. Check "pdf:pagesize width W height H"
    if let Some(rest) = s.strip_prefix("pdf:pagesize") {
        let mut width = None;
        let mut height = None;
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        let mut i = 0;
        while i + 1 < tokens.len() {
            if tokens[i] == "width" {
                width = parse_dimen_to_bp(tokens[i + 1]);
                i += 2;
            } else if tokens[i] == "height" {
                height = parse_dimen_to_bp(tokens[i + 1]);
                i += 2;
            } else {
                i += 1;
            }
        }
        if let (Some(w), Some(h)) = (width, height) {
            return ParsedSpecial::PaperSize {
                width_bp: w,
                height_bp: h,
            };
        }
    }

    // 3. Check "color push ..." and "color pop"
    if s == "color pop" {
        return ParsedSpecial::ColorPop;
    }
    if let Some(rest) = s.strip_prefix("color push") {
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        if !tokens.is_empty() {
            match tokens[0] {
                "gray" if tokens.len() >= 2 => {
                    if let Ok(g) = tokens[1].parse::<f32>() {
                        return ParsedSpecial::ColorPush(Color::Gray(g.clamp(0.0, 1.0)));
                    }
                }
                "rgb" if tokens.len() >= 4 => {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        tokens[1].parse::<f32>(),
                        tokens[2].parse::<f32>(),
                        tokens[3].parse::<f32>(),
                    ) {
                        return ParsedSpecial::ColorPush(Color::Rgb(
                            r.clamp(0.0, 1.0),
                            g.clamp(0.0, 1.0),
                            b.clamp(0.0, 1.0),
                        ));
                    }
                }
                "cmyk" if tokens.len() >= 5 => {
                    if let (Ok(c), Ok(m), Ok(y), Ok(k)) = (
                        tokens[1].parse::<f32>(),
                        tokens[2].parse::<f32>(),
                        tokens[3].parse::<f32>(),
                        tokens[4].parse::<f32>(),
                    ) {
                        return ParsedSpecial::ColorPush(Color::Cmyk(
                            c.clamp(0.0, 1.0),
                            m.clamp(0.0, 1.0),
                            y.clamp(0.0, 1.0),
                            k.clamp(0.0, 1.0),
                        ));
                    }
                }
                _ => {}
            }
        }
    }

    // 4. Check "pdf:bop <raw>" and "pdf:eop <raw>"
    if let Some(rest) = s.strip_prefix("pdf:bop") {
        return ParsedSpecial::PdfBop(rest.trim());
    }
    if let Some(rest) = s.strip_prefix("pdf:eop") {
        return ParsedSpecial::PdfEop(rest.trim());
    }
    if let Some(rest) = s.strip_prefix("pdf:content") {
        return ParsedSpecial::PdfContent(rest.trim());
    }

    ParsedSpecial::Other(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dimen_to_bp() {
        assert_eq!(parse_dimen_to_bp("8.5in"), Some(8.5 * 72.0));
        assert_eq!(parse_dimen_to_bp("100bp"), Some(100.0));
        assert_eq!(parse_dimen_to_bp("72.27pt"), Some(72.0));
        assert_eq!(parse_dimen_to_bp("25.4mm"), Some(72.0));
        assert_eq!(parse_dimen_to_bp("2.54cm"), Some(72.0));
    }

    #[test]
    fn test_parse_papersize_special() {
        let sp = b"papersize=8.5in,11in";
        assert_eq!(
            parse_special(sp),
            ParsedSpecial::PaperSize {
                width_bp: 612.0,
                height_bp: 792.0,
            }
        );

        let sp_pdf = b"pdf:pagesize width 210mm height 297mm";
        let res = parse_special(sp_pdf);
        if let ParsedSpecial::PaperSize { width_bp, height_bp } = res {
            assert!((width_bp - (210.0 * 72.0 / 25.4)).abs() < 1e-4);
            assert!((height_bp - (297.0 * 72.0 / 25.4)).abs() < 1e-4);
        } else {
            panic!("Expected PaperSize, got {:?}", res);
        }
    }

    #[test]
    fn test_parse_color_specials() {
        assert_eq!(
            parse_special(b"color push rgb 1.0 0.5 0.0"),
            ParsedSpecial::ColorPush(Color::Rgb(1.0, 0.5, 0.0))
        );
        assert_eq!(
            parse_special(b"color push gray 0.75"),
            ParsedSpecial::ColorPush(Color::Gray(0.75))
        );
        assert_eq!(
            parse_special(b"color push cmyk 0.1 0.2 0.3 0.4"),
            ParsedSpecial::ColorPush(Color::Cmyk(0.1, 0.2, 0.3, 0.4))
        );
        assert_eq!(parse_special(b"color pop"), ParsedSpecial::ColorPop);
    }

    #[test]
    fn test_parse_pdf_bop_eop() {
        assert_eq!(
            parse_special(b"pdf:bop q 1 0 0 1 0 0 cm"),
            ParsedSpecial::PdfBop("q 1 0 0 1 0 0 cm")
        );
        assert_eq!(parse_special(b"pdf:eop Q"), ParsedSpecial::PdfEop("Q"));
    }

    #[test]
    fn test_color_operators() {
        let c = Color::Rgb(1.0, 0.0, 0.0);
        let (stroke, fill) = c.to_pdf_operators();
        assert_eq!(stroke, "1.0000 0.0000 0.0000 RG");
        assert_eq!(fill, "1.0000 0.0000 0.0000 rg");

        let cg = Color::Gray(0.5);
        let (stroke_g, fill_g) = cg.to_pdf_operators();
        assert_eq!(stroke_g, "0.5000 G");
        assert_eq!(fill_g, "0.5000 g");

        let cc = Color::Cmyk(0.1, 0.2, 0.3, 0.4);
        let (stroke_c, fill_c) = cc.to_pdf_operators();
        assert_eq!(stroke_c, "0.1000 0.2000 0.3000 0.4000 K");
        assert_eq!(fill_c, "0.1000 0.2000 0.3000 0.4000 k");
    }

    #[test]
    fn test_parse_special_edge_cases() {
        // Non-UTF8 bytes
        assert_eq!(parse_special(b"\xFF\xFE\xFD"), ParsedSpecial::Other(""));

        // pdf:content
        assert_eq!(
            parse_special(b"pdf:content 1 0 0 1 10 10 cm"),
            ParsedSpecial::PdfContent("1 0 0 1 10 10 cm")
        );

        // Unknown specials
        assert_eq!(
            parse_special(b"unknown:special something"),
            ParsedSpecial::Other("unknown:special something")
        );

        // Malformed papersize
        assert_eq!(
            parse_special(b"papersize=invalid"),
            ParsedSpecial::Other("papersize=invalid")
        );
        assert_eq!(
            parse_special(b"papersize=invalid,also_invalid"),
            ParsedSpecial::Other("papersize=invalid,also_invalid")
        );

        // Malformed pdf:pagesize
        assert_eq!(
            parse_special(b"pdf:pagesize unknown foo"),
            ParsedSpecial::Other("pdf:pagesize unknown foo")
        );

        // Malformed colors
        assert_eq!(
            parse_special(b"color push unknown"),
            ParsedSpecial::Other("color push unknown")
        );
        assert_eq!(
            parse_special(b"color push gray"),
            ParsedSpecial::Other("color push gray")
        );
        assert_eq!(
            parse_special(b"color push gray not_a_number"),
            ParsedSpecial::Other("color push gray not_a_number")
        );
        assert_eq!(
            parse_special(b"color push rgb 1 2"),
            ParsedSpecial::Other("color push rgb 1 2")
        );
        assert_eq!(
            parse_special(b"color push rgb 1 bad 3"),
            ParsedSpecial::Other("color push rgb 1 bad 3")
        );
        assert_eq!(
            parse_special(b"color push cmyk 1 2 3"),
            ParsedSpecial::Other("color push cmyk 1 2 3")
        );
        assert_eq!(
            parse_special(b"color push cmyk 1 2 3 bad"),
            ParsedSpecial::Other("color push cmyk 1 2 3 bad")
        );
        assert_eq!(
            parse_special(b"color push"),
            ParsedSpecial::Other("color push")
        );

        // parse_dimen_to_bp edge cases
        assert_eq!(parse_dimen_to_bp("invalid"), None);
        assert_eq!(parse_dimen_to_bp("500"), Some(500.0));
        assert_eq!(parse_dimen_to_bp("10cm"), Some(10.0 * 72.0 / 2.54));
    }
}
