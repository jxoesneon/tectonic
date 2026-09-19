// Copyright 2020-2021 the Tectonic Project
// Licensed under the MIT License.

use crate::c_api::{FloatPoint, GlyphBBox, XeTeXFont, XeTeXLayoutEngine};
use crate::engine::{
    find_graphite_feature_named, GrBreak, LayoutEngine, LayoutFeature, MaybeBorrow,
};
use std::borrow::Cow;
use std::ffi::{CStr, CString};
use std::{ptr, slice};
use tectonic_bridge_graphite2 as gr;

#[no_mangle]
pub unsafe extern "C" fn createLayoutEngine(
    font: XeTeXFont,
    script: u32,
    language: *mut libc::c_char,
    features: *mut LayoutFeature,
    n_features: libc::c_int,
    shapers: *mut *const libc::c_char,
    rgb_value: u32,
    extend: f32,
    slant: f32,
    embolden: f32,
) -> XeTeXLayoutEngine {
    let font = Box::from_raw(font);

    let language = if !language.is_null() {
        Some(Cow::Owned(CStr::from_ptr(language).to_owned()))
    } else {
        None
    };
    let features: Box<[_]> = if !features.is_null() {
        let len = n_features as usize;
        Box::from(slice::from_raw_parts(features, len))
    } else {
        Box::new([])
    };
    let shaper_list = if !shapers.is_null() {
        let mut len = 0;
        while !(*shapers.add(len)).is_null() {
            len += 1;
        }
        len += 1;
        slice::from_raw_parts(shapers, len).to_vec()
    } else {
        Vec::new()
    };

    let this = Box::new(LayoutEngine::new(
        MaybeBorrow::Owned(font),
        script,
        language,
        features,
        shaper_list,
        rgb_value,
        extend,
        slant,
        embolden,
    ));

    Box::into_raw(this)
}

#[no_mangle]
pub unsafe extern "C" fn createLayoutEngineBorrowed(
    font: XeTeXFont,
    script: u32,
    language: *mut libc::c_char,
    features: *mut LayoutFeature,
    n_features: libc::c_int,
    shapers: *mut *const libc::c_char,
    rgb_value: u32,
    extend: f32,
    slant: f32,
    embolden: f32,
) -> XeTeXLayoutEngine {
    let font = &mut *font;

    let language = if !language.is_null() {
        Some(Cow::Borrowed(CStr::from_ptr(language)))
    } else {
        None
    };
    let features: Box<[_]> = if !features.is_null() {
        let len = n_features as usize;
        Box::from(slice::from_raw_parts(features, len))
    } else {
        Box::new([])
    };
    let shaper_list = if !shapers.is_null() {
        let mut len = 0;
        while !(*shapers.add(len)).is_null() {
            len += 1;
        }
        len += 1;
        slice::from_raw_parts(shapers, len).to_vec()
    } else {
        Vec::new()
    };

    let this = Box::new(LayoutEngine::new(
        MaybeBorrow::Borrowed(font),
        script,
        language,
        features,
        shaper_list,
        rgb_value,
        extend,
        slant,
        embolden,
    ));

    Box::into_raw(this)
}

#[no_mangle]
pub unsafe extern "C" fn deleteLayoutEngine(this: XeTeXLayoutEngine) {
    let _ = Box::from_raw(this);
}

#[no_mangle]
pub unsafe extern "C" fn getFont(engine: XeTeXLayoutEngine) -> XeTeXFont {
    (*engine).font_mut()
}

#[no_mangle]
pub unsafe extern "C" fn getExtendFactor(engine: XeTeXLayoutEngine) -> f32 {
    (*engine).extend()
}

#[no_mangle]
pub unsafe extern "C" fn getSlantFactor(engine: XeTeXLayoutEngine) -> f32 {
    (*engine).slant()
}

#[no_mangle]
pub unsafe extern "C" fn getEmboldenFactor(engine: XeTeXLayoutEngine) -> f32 {
    (*engine).embolden()
}

#[no_mangle]
pub unsafe extern "C" fn getPointSize(engine: XeTeXLayoutEngine) -> f32 {
    (*engine).font().point_size()
}

#[no_mangle]
pub unsafe extern "C" fn getAscentAndDescent(
    engine: XeTeXLayoutEngine,
    ascent: *mut f32,
    descent: *mut f32,
) {
    *ascent = (*engine).font().ascent();
    *descent = (*engine).font().descent();
}

#[no_mangle]
pub unsafe extern "C" fn getCapAndXHeight(
    engine: XeTeXLayoutEngine,
    capheight: *mut f32,
    xheight: *mut f32,
) {
    *capheight = (*engine).font().cap_height();
    *xheight = (*engine).font().x_height();
}

#[no_mangle]
pub unsafe extern "C" fn getDefaultDirection(engine: XeTeXLayoutEngine) -> libc::c_int {
    (*engine).default_dir() as libc::c_int
}

#[no_mangle]
pub unsafe extern "C" fn getRgbValue(engine: XeTeXLayoutEngine) -> u32 {
    (*engine).rgb()
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphBounds(
    engine: XeTeXLayoutEngine,
    glyph_id: u32,
    bbox: *mut GlyphBBox,
) {
    *bbox = (*engine).font_mut().get_glyph_bounds(glyph_id as u16);
    if (*engine).extend() != 0.0 {
        (*bbox).x_min *= (*engine).extend();
        (*bbox).x_max *= (*engine).extend();
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphWidthFromEngine(engine: XeTeXLayoutEngine, glyph_id: u32) -> f32 {
    (*engine).extend() * (*engine).font().get_glyph_width(glyph_id)
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphHeightDepth(
    engine: XeTeXLayoutEngine,
    glyph_id: u32,
    height: *mut f32,
    depth: *mut f32,
) {
    (*engine)
        .font_mut()
        .get_glyph_height_depth(glyph_id as u16, height.as_mut(), depth.as_mut());
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphSidebearings(
    engine: XeTeXLayoutEngine,
    glyph_id: u32,
    lsb: *mut f32,
    rsb: *mut f32,
) {
    (*engine)
        .font_mut()
        .get_glyph_sidebearings(glyph_id as u16, lsb.as_mut(), rsb.as_mut());
    if (*engine).extend() != 0.0 {
        *lsb *= (*engine).extend();
        *rsb *= (*engine).extend();
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphItalCorr(engine: XeTeXLayoutEngine, glyph_id: u32) -> f32 {
    (*engine).extend() * (*engine).font_mut().get_glyph_ital_corr(glyph_id as u16)
}

#[no_mangle]
pub unsafe extern "C" fn mapCharToGlyph(engine: XeTeXLayoutEngine, char_code: u32) -> u32 {
    (*engine).font().map_char_to_glyph(char_code) as u32
}

#[no_mangle]
pub unsafe extern "C" fn getFontCharRange(
    engine: XeTeXLayoutEngine,
    req_first: libc::c_int,
) -> libc::c_int {
    if req_first != 0 {
        (*engine).font().first_char_code() as libc::c_int
    } else {
        (*engine).font().last_char_code() as libc::c_int
    }
}

#[no_mangle]
pub unsafe extern "C" fn mapGlyphToIndex(
    engine: XeTeXLayoutEngine,
    glyph_name: *const libc::c_char,
) -> libc::c_int {
    (*engine)
        .font()
        .map_glyph_to_index(CStr::from_ptr(glyph_name)) as libc::c_int
}

#[no_mangle]
pub unsafe extern "C" fn usingGraphite(engine: XeTeXLayoutEngine) -> bool {
    (*engine).used_graphite()
}

#[no_mangle]
pub unsafe extern "C" fn usingOpenType(engine: XeTeXLayoutEngine) -> bool {
    (*engine).used_ot()
}

#[no_mangle]
pub unsafe extern "C" fn isOpenTypeMathFont(engine: XeTeXLayoutEngine) -> bool {
    (*engine)
        .font()
        .ttf_face()
        .map(|f| f.tables().math.is_some())
        .unwrap_or(false)
}

#[no_mangle]
pub unsafe extern "C" fn layoutChars(
    engine: XeTeXLayoutEngine,
    chars: *mut u16,
    offset: i32,
    count: i32,
    max: i32,
    rtl: bool,
) -> libc::c_int {
    let chars = slice::from_raw_parts(chars, max as usize);
    let engine = &mut *engine;
    engine.layout_chars(&chars[offset as usize..(offset + count) as usize], rtl) as libc::c_int
}

#[no_mangle]
pub unsafe extern "C" fn getFontFilename(
    engine: XeTeXLayoutEngine,
    index: *mut u32,
) -> *const libc::c_char {
    (*engine).font().filename(&mut *index).to_owned().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn freeFontFilename(filename: *const libc::c_char) {
    if !filename.is_null() {
        let _ = CString::from_raw(filename.cast_mut());
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphs(engine: XeTeXLayoutEngine, glyphs: *mut u32) {
    for (idx, &glyph) in (*engine).shaped_glyphs.iter().enumerate() {
        *glyphs.add(idx) = glyph;
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphAdvances(engine: XeTeXLayoutEngine, advances: *mut f32) {
    let engine = &*engine;
    for (i, pos) in engine.shaped_positions.iter().enumerate() {
        let advance = if engine.font().layout_dir_vertical() {
            pos.y_advance
        } else {
            pos.x_advance
        };

        *advances.add(i) = engine.font().units_to_points(advance as f64) as f32;
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphPositions(engine: XeTeXLayoutEngine, positions: *mut FloatPoint) {
    let engine = &mut *engine;
    let positions_slice = &engine.shaped_positions;

    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    let font = engine.font();

    if font.layout_dir_vertical() {
        for (i, pos) in positions_slice.iter().enumerate() {
            (*positions.add(i)).x = -font.units_to_points((x + pos.y_offset) as f64) as f32;
            (*positions.add(i)).y = font.units_to_points((y - pos.x_offset) as f64) as f32;
            x += pos.y_advance;
            y += pos.x_advance;
        }

        (*positions.add(positions_slice.len())).x = -font.units_to_points(x as f64) as f32;
        (*positions.add(positions_slice.len())).y = font.units_to_points(y as f64) as f32;
    } else {
        for (i, pos) in positions_slice.iter().enumerate() {
            (*positions.add(i)).x = font.units_to_points((x + pos.x_offset) as f64) as f32;
            (*positions.add(i)).y = -font.units_to_points((y + pos.y_offset) as f64) as f32;
            x += pos.x_advance;
            y += pos.y_advance;
        }
        (*positions.add(positions_slice.len())).x = font.units_to_points(x as f64) as f32;
        (*positions.add(positions_slice.len())).y = -font.units_to_points(y as f64) as f32;
    }

    if engine.extend() != 1.0 || engine.slant() != 0.0 {
        for i in 0..=positions_slice.len() {
            let pos = &mut *positions.add(i);
            pos.x = pos.x * engine.extend() - pos.y * engine.slant();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn countGraphiteFeatures(engine: XeTeXLayoutEngine) -> u32 {
    match (*engine).font().gr_face() {
        Some(face) => face.num_feature_refs() as u32,
        None => 0,
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGraphiteFeatureCode(engine: XeTeXLayoutEngine, index: u32) -> u32 {
    crate::engine::get_graphite_feature_code(&*engine, index).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn countGraphiteFeatureSettings(
    engine: XeTeXLayoutEngine,
    feature_id: u32,
) -> u32 {
    crate::engine::count_graphite_feature_settings(&*engine, feature_id).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn getGraphiteFeatureSettingCode(
    engine: XeTeXLayoutEngine,
    feature_id: u32,
    index: u32,
) -> u32 {
    crate::engine::get_graphite_feature_setting_code(&*engine, feature_id, index).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn getGraphiteFeatureDefaultSetting(
    engine: XeTeXLayoutEngine,
    feature_id: u32,
) -> u32 {
    crate::engine::get_graphite_feature_default_setting(&*engine, feature_id).unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn getGraphiteFeatureLabel(
    engine: XeTeXLayoutEngine,
    feature_id: u32,
) -> *const libc::c_char {
    match crate::engine::get_graphite_feature_label(&*engine, feature_id) {
        Some(label) => label.into_raw().cast(),
        None => ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn getGraphiteFeatureSettingLabel(
    engine: XeTeXLayoutEngine,
    feature_id: u32,
    setting_id: u32,
) -> *const libc::c_char {
    match crate::engine::get_graphite_feature_setting_label(&*engine, feature_id, setting_id) {
        Some(label) => label.into_raw().cast(),
        None => ptr::null(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn findGraphiteFeature(
    engine: XeTeXLayoutEngine,
    s: *const libc::c_char,
    e: *const libc::c_char,
    f: *mut u32,
    v: *mut libc::c_int,
) -> bool {
    let len = e.byte_offset_from(s).unsigned_abs();
    let str = slice::from_raw_parts(s.cast(), len);
    crate::engine::find_graphite_feature(&*engine, str, &mut *f, &mut *v)
}

#[no_mangle]
pub unsafe extern "C" fn findGraphiteFeatureNamed(
    engine: XeTeXLayoutEngine,
    name: *const libc::c_char,
    namelength: libc::c_int,
) -> libc::c_long {
    let name = if !name.is_null() && namelength > 0 {
        slice::from_raw_parts(name.cast::<u8>(), namelength as usize)
    } else {
        return -1;
    };
    find_graphite_feature_named(&*engine, name)
        .map(|i| i as libc::c_long)
        .unwrap_or(-1)
}

#[no_mangle]
pub unsafe extern "C" fn findGraphiteFeatureSettingNamed(
    engine: XeTeXLayoutEngine,
    id: u32,
    name: *const libc::c_char,
    namelength: libc::c_int,
) -> libc::c_long {
    let name = if !name.is_null() && namelength > 0 {
        slice::from_raw_parts(name.cast(), namelength as usize)
    } else {
        return -1;
    };

    crate::engine::find_graphite_feature_setting_named(&*engine, id, name)
        .map(|i| i as libc::c_long)
        .unwrap_or(-1)
}

#[no_mangle]
pub unsafe extern "C" fn initGraphiteBreaking(
    engine: XeTeXLayoutEngine,
    txt_ptr: *const u16,
    txt_len: libc::c_uint,
) -> bool {
    let engine = &mut *engine;

    engine.gr_breaking = None;

    let Some(gr_face) = engine.font().gr_face() else {
        return false;
    };
    let Some(gr_font) = gr::Font::new(engine.font().point_size(), gr_face) else {
        return false;
    };

    let lang = engine
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
    let mut gr_feature_values = gr_face.feature_val_for_lang(lang);

    let features = &engine.features;
    for i in (0..engine.features.len()).rev() {
        let fref = gr_face.find_feature_ref(features[i].tag);
        if let Some(fref) = fref {
            let _ = fref.set_feat_value(gr_feature_values.as_mut(), features[i].value as u16);
        }
    }

    let text = slice::from_raw_parts(txt_ptr, txt_len as usize);
    let gr_seg = gr::Segment::new(
        gr_font.as_ref(),
        gr_face,
        engine.script(),
        gr_feature_values.as_ref(),
        text,
    );

    let Some(gr_seg) = gr_seg else {
        return false;
    };

    engine.gr_breaking = Some(GrBreak {
        slot: gr_seg.as_ref().first_slot(),
        segment: gr_seg,
        text_len: txt_len,
    });

    true
}

#[no_mangle]
pub unsafe extern "C" fn findNextGraphiteBreak(engine: XeTeXLayoutEngine) -> libc::c_int {
    let engine = &mut *engine;
    let Some(breaking) = &mut engine.gr_breaking else {
        return -1;
    };

    let segment = breaking.segment.as_ref();
    if breaking.slot != segment.last_slot() {
        let mut s = segment.next(&breaking.slot);
        let mut ret = -1;

        while let Some(slot) = s {
            let ci = segment.cinfo(segment.index(&slot));
            let bw = ci.break_weight();
            if (gr::BREAK_BEFORE_WORD..gr::BREAK_NONE).contains(&bw) {
                breaking.slot = slot.clone();
                ret = ci.base() as libc::c_int;
            } else if (gr::BREAK_NONE + 1..=gr::BREAK_WORD).contains(&bw) {
                breaking.slot = segment.next(&slot).unwrap();
                ret = (ci.base() + 1) as libc::c_int;
            }

            if ret != -1 {
                break;
            }

            s = segment.next(&slot);
        }

        if ret == -1 {
            breaking.slot = segment.last_slot();
            breaking.text_len as libc::c_int
        } else {
            ret
        }
    } else {
        -1
    }
}
