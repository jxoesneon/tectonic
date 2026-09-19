// Copyright 2020-2021 the Tectonic Project
// Licensed under the MIT License.

use crate::c_api::{Fixed, OTTag, RawPlatformFontRef, XeTeXFont};
use crate::font::{larger_script_list, Font};
use crate::utils::{d_to_fix, fix_to_d, raw_to_rs};
use std::ffi::{CStr, CString};
use std::ptr;
#[cfg(target_os = "macos")]
use tectonic_mac_core::{sys::CTFontRef, CTFont, CoreType};

#[no_mangle]
pub unsafe extern "C" fn hasFontTable(font: XeTeXFont, table_tag: OTTag) -> bool {
    (*font).has_font_table(table_tag)
}

#[no_mangle]
pub unsafe extern "C" fn getSlant(font: XeTeXFont) -> Fixed {
    let angle = (*font).italic_angle() as f64;
    d_to_fix(f64::tan(-angle * std::f64::consts::PI / 180.0))
}

#[no_mangle]
pub unsafe extern "C" fn countGlyphs(font: XeTeXFont) -> libc::c_uint {
    (*font).num_glyphs() as libc::c_uint
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphWidth(font: XeTeXFont, gid: u32) -> f32 {
    (*font).get_glyph_width(gid)
}

#[no_mangle]
pub unsafe extern "C" fn setFontLayoutDir(font: XeTeXFont, vertical: libc::c_int) {
    (*font).set_layout_dir_vertical(vertical != 0)
}

#[no_mangle]
pub unsafe extern "C" fn getIndScript(font: XeTeXFont, index: libc::c_uint) -> u32 {
    if let Some(face) = (*font).ttf_face() {
        if let Some(scripts) = larger_script_list(&face) {
            if let Some(script) = scripts.get(index as u16) {
                return u32::from_be_bytes(script.tag.to_bytes());
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn getIndLanguage(
    font: XeTeXFont,
    script: u32,
    index: libc::c_uint,
) -> u32 {
    if let Some(face) = (*font).ttf_face() {
        if let Some(scripts) = larger_script_list(&face) {
            let tag = ttf_parser::Tag::from_bytes(&script.to_be_bytes());
            if let Some(script_rec) = scripts.find(tag) {
                if let Some(lang) = script_rec.languages.get(index as u16) {
                    return u32::from_be_bytes(lang.tag.to_bytes());
                }
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn getIndFeature(
    font: XeTeXFont,
    script: u32,
    language: u32,
    mut index: libc::c_uint,
) -> u32 {
    if let Some(face) = (*font).ttf_face() {
        let script_tag = ttf_parser::Tag::from_bytes(&script.to_be_bytes());
        let lang_tag = if language != 0 {
            Some(ttf_parser::Tag::from_bytes(&language.to_be_bytes()))
        } else {
            None
        };
        for table in [face.tables().gsub, face.tables().gpos].into_iter().flatten() {
            if let Some(s) = table.scripts.find(script_tag) {
                let feat_indices = if let Some(lang_tag) = lang_tag {
                    s.languages.find(lang_tag).map(|l| l.feature_indices)
                } else {
                    s.default_language.map(|d| d.feature_indices)
                };
                if let Some(indices) = feat_indices {
                    let len = indices.len() as libc::c_uint;
                    if index < len {
                        if let Some(feat_idx) = indices.get(index as u16) {
                            if let Some(feat) = table.features.get(feat_idx) {
                                return u32::from_be_bytes(feat.tag.to_bytes());
                            }
                        }
                        return 0;
                    }
                    index -= len;
                }
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn getGlyphName(
    font: XeTeXFont,
    gid: u16,
    len: *mut libc::c_int,
) -> *const libc::c_char {
    match (*font).get_glyph_name(gid) {
        Some(out) => {
            *len = out.as_bytes().len() as libc::c_int;
            CString::into_raw(out)
        }
        None => {
            *len = 0;
            ptr::null()
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn freeGlyphName(name: *mut libc::c_char) {
    if !name.is_null() {
        let _ = CString::from_raw(name);
    }
}

#[no_mangle]
pub unsafe extern "C" fn ttxl_font_units_to_points(font: XeTeXFont, units: f32) -> f32 {
    (*font).units_to_points(units as f64) as f32
}

#[no_mangle]
pub unsafe extern "C" fn ttxl_font_points_to_units(font: XeTeXFont, points: f32) -> f32 {
    (*font).points_to_units(points as f64) as f32
}

#[no_mangle]
pub unsafe extern "C" fn ttxl_font_get_point_size(font: XeTeXFont) -> f32 {
    (*font).point_size()
}

#[no_mangle]
pub unsafe extern "C" fn createFont(font_ref: RawPlatformFontRef, point_size: Fixed) -> XeTeXFont {
    let font_ref = match raw_to_rs(font_ref) {
        Some(fr) => fr,
        None => return ptr::null_mut(),
    };

    match Font::new(font_ref, fix_to_d(point_size) as f32) {
        Err(_) => ptr::null_mut(),
        Ok(out) => Box::into_raw(Box::new(out)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn createFontFromFile(
    filename: *const libc::c_char,
    index: libc::c_int,
    point_size: Fixed,
) -> XeTeXFont {
    let filename = if filename.is_null() {
        None
    } else {
        Some(CStr::from_ptr(filename).to_str().unwrap())
    };

    match Font::new_path_index(filename, index as usize, fix_to_d(point_size) as f32) {
        Err(_) => ptr::null_mut(),
        Ok(out) => Box::into_raw(Box::new(out)),
    }
}

#[no_mangle]
pub unsafe extern "C" fn deleteFont(font: XeTeXFont) {
    let _ = Box::from_raw(font);
}

#[no_mangle]
pub unsafe extern "C" fn countScripts(font: XeTeXFont) -> libc::c_uint {
    if let Some(face) = (*font).ttf_face() {
        if let Some(scripts) = larger_script_list(&face) {
            return scripts.len() as libc::c_uint;
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn countLanguages(font: XeTeXFont, script: u32) -> libc::c_uint {
    if let Some(face) = (*font).ttf_face() {
        if let Some(scripts) = larger_script_list(&face) {
            let tag = ttf_parser::Tag::from_bytes(&script.to_be_bytes());
            if let Some(script_rec) = scripts.find(tag) {
                return script_rec.languages.len() as libc::c_uint;
            }
        }
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn countFeatures(
    font: XeTeXFont,
    script: u32,
    language: u32,
) -> libc::c_uint {
    if let Some(face) = (*font).ttf_face() {
        let script_tag = ttf_parser::Tag::from_bytes(&script.to_be_bytes());
        let lang_tag = if language != 0 {
            Some(ttf_parser::Tag::from_bytes(&language.to_be_bytes()))
        } else {
            None
        };
        let mut count = 0;
        for table in [face.tables().gsub, face.tables().gpos].into_iter().flatten() {
            if let Some(s) = table.scripts.find(script_tag) {
                if let Some(lang_tag) = lang_tag {
                    if let Some(l) = s.languages.find(lang_tag) {
                        count += l.feature_indices.len();
                    }
                } else if let Some(dflt) = s.default_language {
                    count += dflt.feature_indices.len();
                }
            }
        }
        return count as libc::c_uint;
    }
    0
}

#[cfg(target_os = "macos")]
#[no_mangle]
pub unsafe extern "C" fn getFileNameFromCTFont(
    ct_font: CTFontRef,
    index: *mut u32,
) -> *const libc::c_char {
    use std::ptr::NonNull;
    crate::font::get_file_name_from_ct_font(
        &CTFont::new_borrowed(NonNull::new(ct_font.cast_mut()).unwrap()),
        &mut *index,
    )
    .map(CString::into_raw)
    .unwrap_or(ptr::null_mut())
}
