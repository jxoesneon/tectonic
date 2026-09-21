// Copyright 2020-2021 the Tectonic Project
// Licensed under the MIT License.

//! Base engine for laying out fonts using `rustybuzz` and `bridge_graphite2`.

use crate::font::Font;
use std::borrow::Cow;
use std::ffi::{CStr, CString};
use std::ops::{Deref, DerefMut};
use tectonic_bridge_graphite2 as gr;

/// Item that may be borrowed or owned. Similar to `Cow`, but with mutable references.
pub enum MaybeBorrow<'a, T> {
    /// Owned item
    Owned(Box<T>),
    /// Borrowed item
    Borrowed(&'a mut T),
}

impl<T> Deref for MaybeBorrow<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            MaybeBorrow::Owned(val) => val,
            MaybeBorrow::Borrowed(val) => val,
        }
    }
}

impl<T> DerefMut for MaybeBorrow<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            MaybeBorrow::Owned(val) => val,
            MaybeBorrow::Borrowed(val) => val,
        }
    }
}

pub(crate) struct GrBreak {
    pub(crate) segment: gr::Segment,
    pub(crate) slot: gr::Slot,
    pub(crate) text_len: libc::c_uint,
}

/// An OpenType feature setting for text shaping.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LayoutFeature {
    /// 4-byte OpenType feature tag.
    pub tag: u32,
    /// Value assigned to the feature.
    pub value: u32,
    /// Start index in text buffer.
    pub start: libc::c_uint,
    /// End index in text buffer.
    pub end: libc::c_uint,
}

/// Glyph positioning offsets and advances.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShapedPosition {
    /// Horizontal advance.
    pub x_advance: f32,
    /// Vertical advance.
    pub y_advance: f32,
    /// Horizontal offset.
    pub x_offset: f32,
    /// Vertical offset.
    pub y_offset: f32,
}

/// Base layout engine. Combines all the information needed to shape a font for TeX.
#[repr(C)]
pub struct LayoutEngine {
    font: MaybeBorrow<'static, Font>,
    script: u32,
    pub(crate) language: Option<String>,
    pub(crate) features: Box<[LayoutFeature]>,
    /// the requested shapers
    shaper_list: Vec<CString>,
    /// the actually used shaper
    shaper: Option<CString>,
    rgb_value: u32,
    extend: f32,
    slant: f32,
    embolden: f32,
    pub(crate) shaped_glyphs: Vec<u32>,
    pub(crate) shaped_positions: Vec<ShapedPosition>,
    pub(crate) gr_breaking: Option<GrBreak>,
}

impl LayoutEngine {
    /// Create a new layout engine from the needed information
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        font: MaybeBorrow<'static, Font>,
        script: u32,
        language: Option<Cow<'static, CStr>>,
        features: Box<[LayoutFeature]>,
        shaper_list: Vec<CString>,
        rgb_value: u32,
        extend: f32,
        slant: f32,
        embolden: f32,
    ) -> LayoutEngine {
        let lang_str = language.and_then(|l| l.to_str().ok().map(|s| s.to_string()));

        LayoutEngine {
            font,
            script,
            language: lang_str,
            features,
            shaper_list,
            shaper: None,
            rgb_value,
            extend,
            slant,
            embolden,
            shaped_glyphs: Vec::new(),
            shaped_positions: Vec::new(),
            gr_breaking: None,
        }
    }

    /// Get the font script used
    pub fn script(&self) -> u32 {
        self.script
    }

    /// Get the extend
    pub fn extend(&self) -> f32 {
        self.extend
    }

    /// Get the slant
    pub fn slant(&self) -> f32 {
        self.slant
    }

    /// Get the embolden
    pub fn embolden(&self) -> f32 {
        self.embolden
    }

    /// Get the RGB
    pub fn rgb(&self) -> u32 {
        self.rgb_value
    }

    /// Get the font
    pub fn font(&self) -> &Font {
        &self.font
    }

    /// Get a mutable reference to the font
    pub fn font_mut(&mut self) -> &mut Font {
        &mut self.font
    }

    /// Shaped glyph ids from the last `layout_chars` call (`getGlyphs`).
    pub fn shaped_glyph_ids(&self) -> &[u32] {
        &self.shaped_glyphs
    }

    /// Per-glyph advances in points (`getGlyphAdvances`).
    pub fn glyph_advances_points(&self) -> Vec<f32> {
        self.shaped_positions
            .iter()
            .map(|pos| {
                let advance = if self.font.layout_dir_vertical() {
                    pos.y_advance
                } else {
                    pos.x_advance
                };
                self.font.units_to_points(advance as f64) as f32
            })
            .collect()
    }

    /// Cumulative glyph positions in points, `n + 1` entries where the last
    /// is the final pen position (`getGlyphPositions`, including the
    /// extend/slant transform).
    pub fn glyph_positions_points(&self) -> Vec<(f32, f32)> {
        let mut x: f32 = 0.0;
        let mut y: f32 = 0.0;
        let mut out = Vec::with_capacity(self.shaped_positions.len() + 1);
        if self.font.layout_dir_vertical() {
            for pos in &self.shaped_positions {
                out.push((
                    -self.font.units_to_points((x + pos.y_offset) as f64) as f32,
                    self.font.units_to_points((y - pos.x_offset) as f64) as f32,
                ));
                x += pos.y_advance;
                y += pos.x_advance;
            }
            out.push((
                -self.font.units_to_points(x as f64) as f32,
                self.font.units_to_points(y as f64) as f32,
            ));
        } else {
            for pos in &self.shaped_positions {
                out.push((
                    self.font.units_to_points((x + pos.x_offset) as f64) as f32,
                    -self.font.units_to_points((y + pos.y_offset) as f64) as f32,
                ));
                x += pos.x_advance;
                y += pos.y_advance;
            }
            out.push((
                self.font.units_to_points(x as f64) as f32,
                self.font.units_to_points(y as f64) as f32,
            ));
        }
        if self.extend != 1.0 || self.slant != 0.0 {
            for pos in out.iter_mut() {
                pos.0 = pos.0 * self.extend - pos.1 * self.slant;
            }
        }
        out
    }

    /// Get the default direction
    pub fn default_dir(&self) -> u8 {
        pub const UBIDI_DEFAULT_LTR: u8 = 0xFE;
        pub const UBIDI_DEFAULT_RTL: u8 = 0xFF;

        if self.script != 0 {
            let tag_bytes = self.script.to_be_bytes();
            let script_tag = ttf_parser::Tag::from_bytes(&tag_bytes);
            if let Some(script) = rustybuzz::Script::from_iso15924_tag(script_tag) {
                // Arabic, Hebrew, Syriac, Thaana, NKo, Samaritan, Mandaic are RTL
                let is_rtl = matches!(
                    script,
                    rustybuzz::script::ARABIC
                        | rustybuzz::script::HEBREW
                        | rustybuzz::script::SYRIAC
                        | rustybuzz::script::THAANA
                        | rustybuzz::script::NKO
                        | rustybuzz::script::SAMARITAN
                        | rustybuzz::script::MANDAIC
                );
                if is_rtl {
                    return UBIDI_DEFAULT_RTL;
                }
            }
        }
        UBIDI_DEFAULT_LTR
    }

    /// Check whether we're using the graphite shaper
    pub fn used_graphite(&self) -> bool {
        self.shaper
            .as_ref()
            .is_some_and(|s| s.to_bytes() == b"graphite2")
    }

    /// Check whether we're using the OpenType shaper
    pub fn used_ot(&self) -> bool {
        self.shaper.as_ref().is_some_and(|s| s.to_bytes() == b"ot")
    }

    /// Get the number of characters in the given slice
    pub fn layout_chars(&mut self, chars: &[u16], rtl: bool) -> usize {
        self.shaped_glyphs.clear();
        self.shaped_positions.clear();

        // Check if Graphite is requested and font has Graphite data
        let wants_graphite = self
            .shaper_list
            .iter()
            .any(|s| s.as_bytes() == b"graphite2");

        if wants_graphite {
            if let Some(gr_face_ptr) = self.font.gr_face_ptr() {
                // SAFETY: gr_face_ptr is guaranteed non-null and valid for the lifetime of self.font
                let gr_face = unsafe { gr::FaceRef::from_raw(gr_face_ptr) };
                if self.layout_chars_graphite(chars, rtl, gr_face) {
                    self.shaper = Some(CString::new("graphite2").unwrap());
                    return self.shaped_glyphs.len();
                }
            }
        }

        self.layout_chars_rustybuzz(chars, rtl);
        self.shaper = Some(CString::new("ot").unwrap());
        self.shaped_glyphs.len()
    }

    fn layout_chars_rustybuzz(&mut self, chars: &[u16], rtl: bool) {
        let Some(font_data) = self.font.data() else {
            return;
        };
        let Some(face) = rustybuzz::Face::from_slice(font_data, self.font.index()) else {
            return;
        };

        let direction = if self.font.layout_dir_vertical() {
            rustybuzz::Direction::TopToBottom
        } else if rtl {
            rustybuzz::Direction::RightToLeft
        } else {
            rustybuzz::Direction::LeftToRight
        };

        let mut buffer = rustybuzz::UnicodeBuffer::new();
        for (cluster, ch) in std::char::decode_utf16(chars.iter().copied()).enumerate() {
            let c = ch.unwrap_or('\u{FFFD}');
            buffer.add(c, cluster as u32);
        }
        buffer.set_direction(direction);

        if self.script != 0 {
            let tag_bytes = self.script.to_be_bytes();
            let script_tag = ttf_parser::Tag::from_bytes(&tag_bytes);
            if let Some(script) = rustybuzz::Script::from_iso15924_tag(script_tag) {
                buffer.set_script(script);
            }
        }

        if let Some(ref lang_str) = self.language {
            if let Ok(lang) = core::str::FromStr::from_str(lang_str) {
                buffer.set_language(lang);
            }
        }

        buffer.guess_segment_properties();

        let mut rb_features = Vec::with_capacity(self.features.len());
        for f in self.features.iter() {
            let tag_bytes = f.tag.to_be_bytes();
            rb_features.push(rustybuzz::Feature::new(
                ttf_parser::Tag::from_bytes(&tag_bytes),
                f.value,
                f.start as usize..f.end as usize,
            ));
        }

        let glyph_buffer = rustybuzz::shape(&face, &rb_features, buffer);

        for (info, pos) in glyph_buffer
            .glyph_infos()
            .iter()
            .zip(glyph_buffer.glyph_positions().iter())
        {
            self.shaped_glyphs.push(info.glyph_id);
            self.shaped_positions.push(ShapedPosition {
                x_advance: pos.x_advance as f32,
                y_advance: pos.y_advance as f32,
                x_offset: pos.x_offset as f32,
                y_offset: pos.y_offset as f32,
            });
        }
    }

    fn layout_chars_graphite(
        &mut self,
        chars: &[u16],
        rtl: bool,
        gr_face: gr::FaceRef<'_>,
    ) -> bool {
        let lang_tag = self
            .language
            .as_deref()
            .map(|s| {
                let bytes = s.as_bytes();
                let mut tag = [b' '; 4];
                for (i, b) in bytes.iter().take(4).enumerate() {
                    tag[i] = *b;
                }
                u32::from_be_bytes(tag)
            })
            .unwrap_or(0);

        let mut gr_feature_values = gr_face.feature_val_for_lang(lang_tag);
        for f in self.features.iter().rev() {
            if let Some(fref) = gr_face.find_feature_ref(f.tag) {
                let _ = fref.set_feat_value(gr_feature_values.as_mut(), f.value as u16);
            }
        }

        let gr_seg =
            gr::Segment::new_unscaled(gr_face, self.script, gr_feature_values.as_ref(), chars, rtl);

        let Some(gr_seg) = gr_seg else {
            return false;
        };

        let seg_ref = gr_seg.as_ref();
        let glyph_count = seg_ref.n_slots();
        if glyph_count == 0 {
            return true;
        }

        #[derive(Default, Clone, Copy)]
        struct GrCluster {
            base_char: u32,
            num_chars: u32,
            base_glyph: usize,
            num_glyphs: u32,
            advance: f32,
        }

        let mut clusters = vec![GrCluster::default(); chars.len().max(1)];
        let mut gids = Vec::with_capacity(glyph_count);

        let mut ci = 0usize;
        let mut curradv = if rtl {
            let first = seg_ref.first_slot();
            first.origin().0
        } else {
            0.0
        };

        if rtl {
            clusters[0].advance = seg_ref.advance_x() - curradv;
        } else {
            clusters[0].advance = 0.0;
        }

        let mut slot = Some(seg_ref.first_slot());
        let mut ic = 0usize;

        while let Some(s) = slot {
            let before = s.before();
            let after = s.after();
            gids.push(s.gid() as u32);

            while clusters[ci].base_char > before && ci > 0 {
                let c_cur = clusters[ci];
                clusters[ci - 1].num_chars += c_cur.num_chars;
                clusters[ci - 1].num_glyphs += c_cur.num_glyphs;
                clusters[ci - 1].advance += c_cur.advance;
                ci -= 1;
            }

            if s.can_insert_before()
                && clusters[ci].num_chars > 0
                && before >= clusters[ci].base_char + clusters[ci].num_chars
            {
                let next_base_char = clusters[ci].base_char + clusters[ci].num_chars;
                ci += 1;
                if ci >= clusters.len() {
                    clusters.resize(ci + 1, GrCluster::default());
                }
                clusters[ci].base_char = next_base_char;
                clusters[ci].num_chars = before - next_base_char;
                clusters[ci].base_glyph = ic;
                clusters[ci].num_glyphs = 0;
                if rtl {
                    clusters[ci].advance = curradv - s.origin().0;
                    curradv -= clusters[ci].advance;
                } else {
                    let origin_x = s.origin().0;
                    clusters[ci].advance = 0.0;
                    clusters[ci - 1].advance += origin_x - curradv;
                    curradv = origin_x;
                }
            }
            clusters[ci].num_glyphs += 1;

            if clusters[ci].base_char + clusters[ci].num_chars < after + 1 {
                clusters[ci].num_chars = after + 1 - clusters[ci].base_char;
            }

            slot = seg_ref.next(&s);
            ic += 1;
        }

        if rtl {
            clusters[ci].advance += curradv;
        } else {
            clusters[ci].advance += seg_ref.advance_x() - curradv;
        }
        ci += 1;

        let mut glyph_advances = vec![0.0f32; glyph_count];
        let mut glyph_cluster_adv = vec![0.0f32; glyph_count];
        for c in &clusters[..ci] {
            if c.num_glyphs > 0 && c.base_glyph < glyph_count {
                glyph_advances[c.base_glyph] = c.advance;
            }
            for j in 0..c.num_glyphs {
                let idx = c.base_glyph + j as usize;
                if idx < glyph_count {
                    glyph_cluster_adv[idx] = c.advance;
                }
            }
        }

        if !rtl {
            let mut curradvx = 0.0f32;
            let mut curradvy = 0.0f32;
            let mut slot = Some(seg_ref.first_slot());
            let mut i = 0;
            while let Some(s) = slot {
                let x_adv = glyph_advances[i];
                let y_adv = s.advance_y_unscaled(gr_face);
                let x_off = s.origin().0 - curradvx;
                let y_off = s.origin().1 - curradvy;
                curradvx += x_adv;
                curradvy += y_adv;

                self.shaped_glyphs.push(gids[i]);
                self.shaped_positions.push(ShapedPosition {
                    x_advance: x_adv,
                    y_advance: y_adv,
                    x_offset: x_off,
                    y_offset: y_off,
                });

                slot = seg_ref.next(&s);
                i += 1;
            }
        } else {
            let mut curradvx = seg_ref.advance_x();
            let mut curradvy = 0.0f32;
            let mut slot = Some(seg_ref.first_slot());
            let mut i = 0;
            let mut temp_positions = Vec::with_capacity(glyph_count);
            while let Some(s) = slot {
                let x_adv = glyph_advances[i];
                let y_adv = s.advance_y_unscaled(gr_face);
                curradvx -= x_adv;
                curradvy -= y_adv;
                let x_off = s.origin().0 - glyph_cluster_adv[i] - curradvx + x_adv;
                let y_off = s.origin().1 - curradvy;

                temp_positions.push(ShapedPosition {
                    x_advance: x_adv,
                    y_advance: y_adv,
                    x_offset: x_off,
                    y_offset: y_off,
                });

                slot = seg_ref.next(&s);
                i += 1;
            }

            for (gid, pos) in gids.into_iter().rev().zip(temp_positions.into_iter().rev()) {
                self.shaped_glyphs.push(gid);
                self.shaped_positions.push(pos);
            }
        }

        true
    }
}

pub(crate) fn get_graphite_feature_code(engine: &LayoutEngine, index: u32) -> Option<u32> {
    let id = engine.font().gr_face()?.feature_ref(index as usize)?.id();
    Some(id)
}

pub(crate) fn count_graphite_feature_settings(
    engine: &LayoutEngine,
    feature_id: u32,
) -> Option<u32> {
    let out = engine
        .font()
        .gr_face()?
        .find_feature_ref(feature_id)?
        .num_values() as u32;
    Some(out)
}

pub(crate) fn get_graphite_feature_setting_code(
    engine: &LayoutEngine,
    feature_id: u32,
    index: u32,
) -> Option<u32> {
    let out = engine
        .font()
        .gr_face()?
        .find_feature_ref(feature_id)?
        .value(index as usize) as u32;
    Some(out)
}

pub(crate) fn get_graphite_feature_default_setting(
    engine: &LayoutEngine,
    feature_id: u32,
) -> Option<u32> {
    let face = engine.font().gr_face()?;
    let feat = face.find_feature_ref(feature_id)?;
    let lang = engine.language.as_deref().map(tag_from_str).unwrap_or(0);
    let feature_values = face.feature_val_for_lang(lang);
    let out = feat.feat_value(feature_values.as_ref()) as u32;
    Some(out)
}

pub(crate) fn get_graphite_feature_label(
    engine: &LayoutEngine,
    feature_id: u32,
) -> Option<gr::Label> {
    let face = engine.font().gr_face()?;
    let feature = face.find_feature_ref(feature_id)?;
    let lang_id = 0x409;
    let label = feature.label(lang_id)?;
    Some(label)
}

pub(crate) fn get_graphite_feature_setting_label(
    engine: &LayoutEngine,
    feature_id: u32,
    setting_id: u32,
) -> Option<gr::Label> {
    let face = engine.font().gr_face()?;

    let feature = face.find_feature_ref(feature_id)?;
    for i in 0..feature.num_values() {
        if setting_id == feature.value(i) as u32 {
            let lang_id = 0x409;
            return feature.value_label(i, lang_id);
        }
    }

    None
}

fn tag_from_str(s: &str) -> u32 {
    let bytes = s.as_bytes();
    let mut tag = [b' '; 4];
    for (i, b) in bytes.iter().take(4).enumerate() {
        tag[i] = *b;
    }
    u32::from_be_bytes(tag)
}

pub(crate) fn find_graphite_feature(
    engine: &LayoutEngine,
    str: &[u8],
    tag: &mut u32,
    v: &mut libc::c_int,
) -> bool {
    *tag = 0;
    *v = 0;

    let mut idx = 0;
    while str[idx] == b' ' || str[idx] == b'\t' {
        idx += 1;
    }
    while str.get(idx).is_some_and(|c| *c != b'=') {
        idx += 1;
    }

    match find_graphite_feature_named(engine, &str[..idx]) {
        Some(val) => *tag = val,
        None => return false,
    }

    idx += 1;
    while idx < str.len() && (str[idx] == b' ' || str[idx] == b'\t') {
        idx += 1;
    }

    if idx >= str.len() {
        return false;
    }

    *v = find_graphite_feature_setting_named(engine, *tag, &str[idx..])
        .map(|i| i as libc::c_int)
        .unwrap_or(-1);

    *v != -1
}

pub(crate) fn find_graphite_feature_named(engine: &LayoutEngine, name: &[u8]) -> Option<u32> {
    let gr_face = engine.font().gr_face()?;

    let tag = tag_from_str(std::str::from_utf8(name).ok()?);

    for i in 0..gr_face.num_feature_refs() {
        let feature = gr_face.feature_ref(i)?;
        let lang_id = 0x409;
        let label = feature.label(lang_id)?;

        if &label.as_bytes()[..name.len()] == name || feature.id() == tag {
            return Some(feature.id());
        }
    }

    None
}

pub(crate) fn find_graphite_feature_setting_named(
    engine: &LayoutEngine,
    id: u32,
    name: &[u8],
) -> Option<i16> {
    let face = engine.font().gr_face()?;

    let tag = tag_from_str(std::str::from_utf8(name).ok()?);

    let feature = face.find_feature_ref(id)?;
    for i in 0..feature.num_values() {
        let lang_id = 0x409;
        let label = feature.value_label(i, lang_id)?;
        if &label.as_bytes()[..name.len()] == name || feature.id() == tag {
            return Some(feature.value(i));
        }
    }
    None
}
