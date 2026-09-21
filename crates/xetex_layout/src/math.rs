// Copyright 2026 the Tectonic Project
// Licensed under the MIT License.

//! Pure-Rust OpenType MATH table querying via `ttf-parser`.

use crate::c_api::XeTeXFont;
use ttf_parser::GlyphId;

/// A part of an OpenType math glyph assembly.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct XeTeXMathGlyphPart {
    /// Glyph ID of the part.
    pub glyph: u32,
    /// Length of the start connector in font units.
    pub start_connector_length: i32,
    /// Length of the end connector in font units.
    pub end_connector_length: i32,
    /// Full advance measurement in font units.
    pub full_advance: i32,
    /// Part flags (e.g. 1 if extender).
    pub flags: u32,
}

/// A glyph assembly recipe consisting of multiple glyph parts.
#[repr(C)]
#[derive(Debug)]
pub struct XeTeXGlyphAssembly {
    /// Number of parts in the assembly.
    pub count: libc::c_uint,
    /// Pointer to array of assembly parts.
    pub parts: *mut XeTeXMathGlyphPart,
}

/// Query an OpenType math constant by its 0-based constant index.
///
/// # Safety
/// `font` must be a valid pointer to a `Font` instance.
#[no_mangle]
pub unsafe extern "C" fn ttxl_get_ot_math_constant(font: XeTeXFont, constant: u32) -> i32 {
    let Some(face) = (*font).ttf_face() else {
        return 0;
    };
    let Some(math) = face.tables().math else {
        return 0;
    };
    let Some(c) = math.constants else {
        return 0;
    };
    match constant {
        0 => c.script_percent_scale_down() as i32,
        1 => c.script_script_percent_scale_down() as i32,
        2 => c.delimited_sub_formula_min_height() as i32,
        3 => c.display_operator_min_height() as i32,
        4 => c.math_leading().value as i32,
        5 => c.axis_height().value as i32,
        6 => c.accent_base_height().value as i32,
        7 => c.flattened_accent_base_height().value as i32,
        8 => c.subscript_shift_down().value as i32,
        9 => c.subscript_top_max().value as i32,
        10 => c.subscript_baseline_drop_min().value as i32,
        11 => c.superscript_shift_up().value as i32,
        12 => c.superscript_shift_up_cramped().value as i32,
        13 => c.superscript_bottom_min().value as i32,
        14 => c.superscript_baseline_drop_max().value as i32,
        15 => c.sub_superscript_gap_min().value as i32,
        16 => c.superscript_bottom_max_with_subscript().value as i32,
        17 => c.space_after_script().value as i32,
        18 => c.upper_limit_gap_min().value as i32,
        19 => c.upper_limit_baseline_rise_min().value as i32,
        20 => c.lower_limit_gap_min().value as i32,
        21 => c.lower_limit_baseline_drop_min().value as i32,
        22 => c.stack_top_shift_up().value as i32,
        23 => c.stack_top_display_style_shift_up().value as i32,
        24 => c.stack_bottom_shift_down().value as i32,
        25 => c.stack_bottom_display_style_shift_down().value as i32,
        26 => c.stack_gap_min().value as i32,
        27 => c.stack_display_style_gap_min().value as i32,
        28 => c.stretch_stack_top_shift_up().value as i32,
        29 => c.stretch_stack_bottom_shift_down().value as i32,
        30 => c.stretch_stack_gap_above_min().value as i32,
        31 => c.stretch_stack_gap_below_min().value as i32,
        32 => c.fraction_numerator_shift_up().value as i32,
        33 => c.fraction_numerator_display_style_shift_up().value as i32,
        34 => c.fraction_denominator_shift_down().value as i32,
        35 => c.fraction_denominator_display_style_shift_down().value as i32,
        36 => c.fraction_numerator_gap_min().value as i32,
        37 => c.fraction_num_display_style_gap_min().value as i32,
        38 => c.fraction_rule_thickness().value as i32,
        39 => c.fraction_denominator_gap_min().value as i32,
        40 => c.fraction_denom_display_style_gap_min().value as i32,
        41 => c.skewed_fraction_horizontal_gap().value as i32,
        42 => c.skewed_fraction_vertical_gap().value as i32,
        43 => c.overbar_vertical_gap().value as i32,
        44 => c.overbar_rule_thickness().value as i32,
        45 => c.overbar_extra_ascender().value as i32,
        46 => c.underbar_vertical_gap().value as i32,
        47 => c.underbar_rule_thickness().value as i32,
        48 => c.underbar_extra_descender().value as i32,
        49 => c.radical_vertical_gap().value as i32,
        50 => c.radical_display_style_vertical_gap().value as i32,
        51 => c.radical_rule_thickness().value as i32,
        52 => c.radical_extra_ascender().value as i32,
        53 => c.radical_kern_before_degree().value as i32,
        54 => c.radical_kern_after_degree().value as i32,
        55 => c.radical_degree_bottom_raise_percent() as i32,
        _ => 0,
    }
}

/// Query an OpenType math variant glyph for a given glyph index.
///
/// # Safety
/// `font` and `adv` must be valid non-null pointers.
#[no_mangle]
pub unsafe extern "C" fn ttxl_get_ot_math_variant(
    font: XeTeXFont,
    glyph: u32,
    variant_index: i32,
    adv: *mut i32,
    horiz: bool,
) -> u32 {
    *adv = -1;
    let Some(face) = (*font).ttf_face() else {
        return glyph;
    };
    let Some(math) = face.tables().math else {
        return glyph;
    };
    let Some(variants) = math.variants else {
        return glyph;
    };
    let constructions = if horiz {
        variants.horizontal_constructions
    } else {
        variants.vertical_constructions
    };
    if let Some(c) = constructions.get(GlyphId(glyph as u16)) {
        if let Some(v) = c.variants.get(variant_index as u16) {
            *adv = v.advance_measurement as i32;
            return v.variant_glyph.0 as u32;
        }
    }
    glyph
}

/// Retrieve an OpenType math glyph assembly recipe for horizontal or vertical growth.
///
/// # Safety
/// `font` must be a valid pointer to a `Font` instance.
#[no_mangle]
pub unsafe extern "C" fn ttxl_get_ot_assembly_ptr(
    font: XeTeXFont,
    glyph: u32,
    horiz: bool,
) -> *mut XeTeXGlyphAssembly {
    let Some(face) = (*font).ttf_face() else {
        return std::ptr::null_mut();
    };
    let Some(math) = face.tables().math else {
        return std::ptr::null_mut();
    };
    let Some(variants) = math.variants else {
        return std::ptr::null_mut();
    };
    let constructions = if horiz {
        variants.horizontal_constructions
    } else {
        variants.vertical_constructions
    };
    if let Some(c) = constructions.get(GlyphId(glyph as u16)) {
        if let Some(assembly) = c.assembly {
            let count = assembly.parts.len() as usize;
            if count > 0 {
                let mut parts_vec: Vec<XeTeXMathGlyphPart> = Vec::with_capacity(count);
                for part in assembly.parts {
                    parts_vec.push(XeTeXMathGlyphPart {
                        glyph: part.glyph_id.0 as u32,
                        start_connector_length: part.start_connector_length as i32,
                        end_connector_length: part.end_connector_length as i32,
                        full_advance: part.full_advance as i32,
                        flags: if part.part_flags.extender() { 1 } else { 0 },
                    });
                }
                let mut parts_slice = parts_vec.into_boxed_slice();
                let parts_ptr = parts_slice.as_mut_ptr();
                std::mem::forget(parts_slice);

                let asm_box = Box::new(XeTeXGlyphAssembly {
                    count: count as libc::c_uint,
                    parts: parts_ptr,
                });
                return Box::into_raw(asm_box);
            }
        }
    }
    std::ptr::null_mut()
}

/// Free an OpenType glyph assembly structure previously returned by `ttxl_get_ot_assembly_ptr`.
///
/// # Safety
/// `a` must be null or a pointer previously returned by `ttxl_get_ot_assembly_ptr`.
#[no_mangle]
pub unsafe extern "C" fn ttxl_free_ot_assembly(a: *mut XeTeXGlyphAssembly) {
    if !a.is_null() {
        let asm_val = Box::from_raw(a);
        if !asm_val.parts.is_null() && asm_val.count > 0 {
            let _ = Vec::from_raw_parts(
                asm_val.parts,
                asm_val.count as usize,
                asm_val.count as usize,
            );
        }
    }
}

/// Query the italics correction for an OpenType math glyph.
///
/// # Safety
/// `font` must be a valid pointer to a `Font` instance.
#[no_mangle]
pub unsafe extern "C" fn ttxl_get_ot_math_ital_corr(font: XeTeXFont, glyph: u32) -> i32 {
    let Some(face) = (*font).ttf_face() else {
        return 0;
    };
    let Some(math) = face.tables().math else {
        return 0;
    };
    let Some(glyph_info) = math.glyph_info else {
        return 0;
    };
    glyph_info
        .italic_corrections
        .and_then(|ic| ic.get(GlyphId(glyph as u16)))
        .map(|v| v.value as i32)
        .unwrap_or(0)
}

/// Query the top accent attachment position for an OpenType math glyph.
///
/// # Safety
/// `font` must be a valid pointer to a `Font` instance.
#[no_mangle]
pub unsafe extern "C" fn ttxl_get_ot_math_accent_pos(font: XeTeXFont, glyph: u32) -> i32 {
    let Some(face) = (*font).ttf_face() else {
        return 0x7fffffff;
    };
    let Some(math) = face.tables().math else {
        return 0x7fffffff;
    };
    let Some(glyph_info) = math.glyph_info else {
        return 0x7fffffff;
    };
    glyph_info
        .top_accent_attachments
        .and_then(|ta| ta.get(GlyphId(glyph as u16)))
        .map(|v| v.value as i32)
        .unwrap_or(0x7fffffff)
}

/// Query the minimum connector overlap for OpenType math glyph assemblies.
///
/// # Safety
/// `font` must be a valid pointer to a `Font` instance.
#[no_mangle]
pub unsafe extern "C" fn ttxl_ot_min_connector_overlap(font: XeTeXFont) -> i32 {
    let Some(face) = (*font).ttf_face() else {
        return 0;
    };
    let Some(math) = face.tables().math else {
        return 0;
    };
    let Some(variants) = math.variants else {
        return 0;
    };
    variants.min_connector_overlap as i32
}

/// Query math kerning between two glyphs across a specified boundary and height.
///
/// # Safety
/// `font` must be a valid pointer to a `Font` instance.
#[no_mangle]
pub unsafe extern "C" fn ttxl_get_ot_math_kern(
    font: XeTeXFont,
    glyph: u32,
    side: i32,
    height: i32,
) -> i32 {
    let Some(face) = (*font).ttf_face() else {
        return 0;
    };
    let Some(math) = face.tables().math else {
        return 0;
    };
    let Some(glyph_info) = math.glyph_info else {
        return 0;
    };
    let Some(kern_infos) = glyph_info.kern_infos else {
        return 0;
    };
    let Some(ki) = kern_infos.get(GlyphId(glyph as u16)) else {
        return 0;
    };
    let kern_table = match side {
        0 => ki.top_right,    // HB_OT_MATH_KERN_TOP_RIGHT
        1 => ki.top_left,     // HB_OT_MATH_KERN_TOP_LEFT
        2 => ki.bottom_right, // HB_OT_MATH_KERN_BOTTOM_RIGHT
        3 => ki.bottom_left,  // HB_OT_MATH_KERN_BOTTOM_LEFT
        _ => None,
    };
    let Some(kern) = kern_table else {
        return 0;
    };
    let count = kern.count();
    for i in 0..count {
        if let Some(h) = kern.height(i) {
            if h.value as i32 >= height {
                return kern.kern(i).map(|k| k.value as i32).unwrap_or(0);
            }
        }
    }
    kern.kern(count).map(|k| k.value as i32).unwrap_or(0)
}
