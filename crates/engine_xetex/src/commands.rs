// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust primitive command dispatch for the migrated XeTeX stomach.
//!
//! This module ports the first bounded slice of C `prefixed_command`
//! (`xetex-ini.c` ~1316) and the mode-independent group/after-token cases from
//! `main_control` (`xetex-xetex0.c` ~17128-17493). Implemented dispatch covers
//! `\global`, `\long`, `\outer`, and `\protected`; `\def`, `\gdef`, `\edef`,
//! and `\xdef`; `\let` and `\futurelet`; dense `\count`, `\dimen`, `\skip`,
//! `\muskip`, and `\toks` assignments; the `AssignInt`, `AssignDimen`,
//! `AssignGlue`, `AssignMuGlue`, and `AssignToks` primitives; dense
//! `\advance`, `\multiply`, and `\divide` register arithmetic; the shorthand
//! `\chardef`, `\mathchardef`, XeTeX math-char, and dense register-definition
//! forms; the `\openin`, `\closein`, `\read`, and `\readline` stream
//! operations backed by registered in-memory sources; the dense
//! character-code table assignments used by `\catcode`, `\lccode`, `\uccode`,
//! `\sfcode`, `\mathcode`, `\delcode`, and the XeTeX variants; semantic state
//! assignments such as `\prevdepth`, `\spacefactor`, `\prevgraf`, page
//! quantities, dense box dimensions, shape arrays, interaction modes, box
//! construction/packaging, box-register ownership, rules, moves, leaders,
//! `\setbox`, and `\vsplit`.
//!
//! Deliberately deferred cases include font assignments and font loading,
//! hyphenation data, paragraph construction, alignments, math lists, output
//! routines, and other mode-dependent stomach paths.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::expand::format_scaled_pt;
use crate::primitives::primitive_defs;
use crate::scan::NUMBER_MATH_FAMILIES;
use crate::scan::{
    ACTIVE_MATH_CHAR, BIGGEST_CHAR, DIMEN_VAL, GLUE_VAL, INT_VAL, MU_VAL, OTHER_TOKEN, TEX_INFINITY,
};
use crate::scanner::{
    TeXMouthError, ALIGNING, BACKED_UP, BACKED_UP_CHAR, CR_CODE, CR_CR_CODE, CS_TOKEN_FLAG,
    EVERY_CR_TEXT, EVERY_HBOX_TEXT, EVERY_PAR_TEXT, EVERY_VBOX_TEXT, FROZEN_END_GROUP_ID,
    FROZEN_END_TEMPLATE_ID, FROZEN_RIGHT_ID, INSERTED, INTER_CHAR_TEXT, LEFT_BRACE_TOKEN,
    MATH_SHIFT_TOKEN, MAX_CHAR_VAL, MAX_COMMAND, NORMAL as SCAN_NORMAL, OUTPUT_TEXT,
    PROTECTED_TOKEN, RIGHT_BRACE_TOKEN, SPAN_CODE, TOKEN_LIST, TOO_BIG_USV, U_TEMPLATE, V_TEMPLATE,
};
use crate::scanner::{EVERY_DISPLAY_TEXT, EVERY_MATH_TEXT};
use crate::state::{
    badness, half, tex_round, CommandCode, LigKernInsn, MemoryWord, SaveStackEntry, TeXEngineState,
    TeXMemoryError, TeXMode, WriteStream, ABOVE_CODE, ACCENT_NOAD, ACCENT_NOAD_SIZE, ACC_KERN,
    ACTIVE_NODE_SIZE_EXTENDED, ACTIVE_NODE_SIZE_NORMAL, ADDITIONAL, ADJUSTED_HBOX_GROUP,
    ADJUST_NODE, AFTER, ALIGN_GROUP, ALIGN_STACK_NODE_SIZE, AWFUL_BAD, A_LEADERS, BATCH_MODE,
    BEFORE, BEGIN_L_CODE, BEGIN_M_CODE, BIGGEST_LANG, BOTTOM_ACC, BOTTOM_LEVEL, BOT_MARK_CODE,
    BOX_BASE, BOX_CODE, BOX_FLAG, BOX_NODE_SIZE, CAT_CODE_BASE, CHAR_CLASS_LIMIT, CLOSE_NODE,
    CODE_TABLE_CAT, CODE_TABLE_DEL, CODE_TABLE_LC, CODE_TABLE_MATH, CODE_TABLE_SF, CODE_TABLE_UC,
    COND_MATH_GLUE, COPY_CODE, COUNT_BASE, CUR_FONT_LOC, DECENT_FIT, DEFAULT_CODE, DEFAULT_RULE,
    DELIMITED_CODE, DELTA_NODE, DELTA_NODE_SIZE, DEL_CODE_BASE, DIMEN_BASE, DIMEN_PAR_BASE,
    DIMEN_PAR_BOX_MAX_DEPTH, DIMEN_PAR_DISPLAY_INDENT, DIMEN_PAR_DISPLAY_WIDTH,
    DIMEN_PAR_EMERGENCY_STRETCH, DIMEN_PAR_HANG_INDENT, DIMEN_PAR_HSIZE, DIMEN_PAR_MATH_SURROUND,
    DIMEN_PAR_MAX_DEPTH, DIMEN_PAR_OVERFULL_RULE, DIMEN_PAR_PAR_INDENT, DIMEN_PAR_PRE_DISPLAY_SIZE,
    DIMEN_PAR_VFUZZ, DIMEN_PAR_VSIZE, DISC_GROUP, DISC_NODE, DISPLAY_STYLE, EDGE_NODE,
    EJECT_PENALTY, END_L_CODE, END_M_CODE, ERROR_STOP_MODE, ETEX_PEN_BASE, ETEX_PEN_CLUB,
    ETEX_PEN_DISPLAY_WIDOW, ETEX_PEN_INTER_LINE, ETEX_PEN_WIDOW, EXACTLY, EXPLICIT, FAMILY_BASE,
    FIL, FILL, FILLL, FIRE_UP_DONE, FIRE_UP_INIT, FIRST_MARK_CODE, FIXED_ACC, FRACTION_NOAD,
    FRACTION_NOAD_SIZE, GLOBAL_BOX_FLAG, GLUE_NODE, GLUE_PAR_ABOVE_DISPLAY_SHORT_SKIP,
    GLUE_PAR_ABOVE_DISPLAY_SKIP, GLUE_PAR_BASE, GLUE_PAR_BELOW_DISPLAY_SHORT_SKIP,
    GLUE_PAR_BELOW_DISPLAY_SKIP, GLUE_PAR_LEFT_SKIP, GLUE_PAR_PAR_FILL_SKIP, GLUE_PAR_PAR_SKIP,
    GLUE_PAR_RIGHT_SKIP, GLUE_PAR_SPACE_SKIP, GLUE_PAR_SPLIT_TOP_SKIP, GLUE_PAR_TAB_SKIP,
    GLUE_PAR_TOP_SKIP, GLUE_PAR_XETEX_LINEBREAK_SKIP, GLUE_PAR_XSPACE_SKIP, GLUE_SPEC_SIZE,
    GLYPH_NODE, GLYPH_NODE_SIZE, HBOX_GROUP, HLIST_NODE, HYPHENATED, HYPHEN_CHAR_CODE,
    IGNORE_DEPTH, INF_BAD, INF_PENALTY, INNER_NOAD, INSERTING, INSERT_GROUP, INS_NODE,
    INS_NODE_SIZE, INT_PAR_ADJ_DEMERITS, INT_PAR_BASE, INT_PAR_BROKEN_PENALTY,
    INT_PAR_CLUB_PENALTY, INT_PAR_CUR_FAM, INT_PAR_DISPLAY_WIDOW_PENALTY,
    INT_PAR_DOUBLE_HYPHEN_DEMERITS, INT_PAR_ESCAPE_CHAR, INT_PAR_EX_HYPHEN_PENALTY,
    INT_PAR_FINAL_HYPHEN_DEMERITS, INT_PAR_FLOATING_PENALTY, INT_PAR_GLOBAL_DEFS,
    INT_PAR_HANG_AFTER, INT_PAR_HOLDING_INSERTS, INT_PAR_HYPHEN_PENALTY,
    INT_PAR_INTER_LINE_PENALTY, INT_PAR_LANGUAGE, INT_PAR_LAST_LINE_FIT, INT_PAR_LEFT_HYPHEN_MIN,
    INT_PAR_LINE_PENALTY, INT_PAR_LOOSENESS, INT_PAR_MAX_DEAD_CYCLES, INT_PAR_OUTPUT_PENALTY,
    INT_PAR_POST_DISPLAY_PENALTY, INT_PAR_PRETOLERANCE, INT_PAR_PRE_DISPLAY_DIRECTION,
    INT_PAR_PRE_DISPLAY_PENALTY, INT_PAR_RIGHT_HYPHEN_MIN, INT_PAR_SAVING_VDISCARDS,
    INT_PAR_SHOW_BOX_BREADTH, INT_PAR_SHOW_BOX_DEPTH, INT_PAR_SUPPRESS_FONTNOTFOUND_ERROR,
    INT_PAR_TEXXET, INT_PAR_TOLERANCE, INT_PAR_TRACING_LOST_CHARS, INT_PAR_TRACING_ONLINE,
    INT_PAR_UC_HYPH, INT_PAR_VBADNESS, INT_PAR_WIDOW_PENALTY, INT_PAR_XETEX_DASH_BREAK,
    INT_PAR_XETEX_DEFAULT_INPUT_ENCODING, INT_PAR_XETEX_DEFAULT_INPUT_MODE,
    INT_PAR_XETEX_INTERWORD_SPACE_SHAPING, INT_PAR_XETEX_INTER_CHAR_TOKENS,
    INT_PAR_XETEX_LINEBREAK_LOCALE, INT_PAR_XETEX_LINEBREAK_PENALTY, INT_PAR_XETEX_PROTRUDE_CHARS,
    INT_PAR_XETEX_TRACING_FONTS, INT_PAR_XETEX_UPWARDS, KERN_NODE, LANGUAGE_NODE, LAST_BOX_CODE,
    LC_CODE_BASE, LEADER_FLAG, LEFT_NOAD, LEFT_TO_RIGHT, LIGATURE_NODE, LIG_TAG, LOCAL_BASE,
    LOCAL_ERR_HELP, LOCAL_EVERY_CR, LOCAL_EVERY_DISPLAY, LOCAL_EVERY_HBOX, LOCAL_EVERY_MATH,
    LOCAL_EVERY_PAR, LOCAL_EVERY_VBOX, LOCAL_OUTPUT_ROUTINE, LOCAL_PAR_SHAPE,
    LOCAL_XETEX_INTER_CHAR_TOKS, LOOSE_FIT, LP_CODE_BASE, L_CODE, MARGIN_KERN_NODE, MARK_NODE,
    MATH_CHAR, MATH_CHOICE_GROUP, MATH_CODE_BASE, MATH_FIELD_EMPTY, MATH_FONT_BASE_C, MATH_GROUP,
    MATH_LEFT_GROUP, MATH_NODE, MATH_SHIFT_GROUP, MAX_HALFWORD, MAX_HYPH_CHAR, MEDIUM_NODE_SIZE,
    MIN_HALFWORD, MSKIP_CODE, MU_GLUE, MU_SKIP_BASE, NATIVE_WORD_NODE, NATIVE_WORD_NODE_AT,
    NON_ADDRESS, NORMAL, NO_ALIGN_GROUP, NULL_FLAG, OPEN_NODE, OPEN_NODE_SIZE, OP_NOAD, ORD_NOAD,
    OUTPUT_GROUP, OVER_CODE, PAGE_BOX_THERE, PAGE_EMPTY, PAGE_INSERTS_ONLY, PAGE_INS_NODE_SIZE,
    PAGE_SO_FAR_COUNT, PASSIVE_NODE_SIZE, PDF_NODE, PDF_SAVE_POS_NODE, PENALTY_NODE, PIC_NODE,
    PIC_NODE_SIZE, QUAD_CODE, RADICAL_NOAD, RADICAL_NOAD_SIZE, RIGHT_NOAD, RIGHT_TO_LEFT,
    RULE_NODE, RULE_NODE_SIZE, R_CODE, SA_BOX_VAL, SA_INTER_CHAR_VAL, SA_MARK_VAL, SA_TOK_VAL,
    SCRIPT_SCRIPT_SIZE, SCRIPT_SIZE, SELECTOR_NO_PRINT, SELECTOR_TERM_ONLY, SEMI_SIMPLE_GROUP,
    SF_CODE_BASE, SHIP_OUT_FLAG, SHRINKING, SIMPLE_GROUP, SKIP_BASE, SKIP_CODE, SLANT_CODE,
    SMALL_NODE_SIZE, SPACE_ADJUSTMENT, SPACE_CODE, SPACE_SHRINK_CODE, SPACE_STRETCH_CODE,
    SPAN_NODE_SIZE, SPECIAL_NODE, STRETCHING, SUB_BOX, SUB_MLIST, TEXT_SIZE, TEXT_STYLE, TEX_NULL,
    TIGHT_FIT, TOKS_BASE, TOO_BIG_CHAR, TOP_MARK_CODE, TOTAL_MATHEX_PARAMS, TOTAL_MATHSY_PARAMS,
    UC_CODE_BASE, UNHYPHENATED, UNSET_NODE, VBOX_GROUP, VCENTER_GROUP, VERY_LOOSE_FIT, VLIST_NODE,
    VSPLIT_CODE, VTOP_CODE, VTOP_GROUP, WHATSIT_NODE, WRITE_NODE, WRITE_NODE_SIZE, X_HEIGHT_CODE,
};
use crate::state::{
    OtMathAssembly, BIN_NOAD, CHOICE_NODE, CLOSE_NOAD, DIMEN_PAR_DELIMITER_SHORTFALL,
    DIMEN_PAR_NULL_DELIMITER_SPACE, DIMEN_PAR_SCRIPT_SPACE, EXT_TAG, GLUE_PAR_MED_MU_SKIP,
    GLUE_PAR_THICK_MU_SKIP, GLUE_PAR_THIN_MU_SKIP, INT_PAR_BIN_OP_PENALTY,
    INT_PAR_DELIMITER_FACTOR, INT_PAR_REL_PENALTY, LIMITS, LIST_TAG, MATH_TEXT_CHAR, NOAD_SIZE,
    OPEN_NOAD, OVER_NOAD, PUNCT_NOAD, REL_NOAD, SCRIPT_STYLE, STYLE_NODE, STYLE_NODE_SIZE, SUB_CMD,
    SUP_CMD, UNDER_NOAD, VCENTER_NOAD,
};

/// Largest command code that cannot legally follow a prefix
/// (`MAX_NON_PREFIXED_COMMAND`).
const MAX_NON_PREFIXED_COMMAND: u8 = CommandCode::LastItem as u8;

/// `cur_chr` used by `\let` (`NORMAL`).
const NORMAL_LET: i32 = 0;

/// Clamps a hyphenation minimum like C `norm_min`.
fn norm_min(h: i32) -> i32 {
    h.clamp(1, 63)
}

/// Finds the vendored primitive name for a command/payload pair.
pub(crate) fn primitive_name_for(command: CommandCode, character: i32) -> Option<&'static str> {
    primitive_defs()
        .ok()?
        .into_iter()
        .find(|def| def.command == command && def.chr == character)
        .map(|def| def.name)
}

/// Formats a semantic-list mode (`print_mode`).
fn mode_text(mode: i16) -> &'static str {
    match mode {
        x if x == TeXMode::Vmode as i16 => "vertical mode",
        x if x == TeXMode::Hmode as i16 => "horizontal mode",
        x if x == TeXMode::Mmode as i16 => "display math mode",
        x if x == TeXMode::InternalVmode as i16 => "internal vertical mode",
        x if x == TeXMode::RestrictedHmode as i16 => "restricted horizontal mode",
        x if x == TeXMode::InlineMmode as i16 => "math mode",
        _ => "unknown mode",
    }
}

/// Formats a group code's noun (`print_group`).
fn group_text(group: u8) -> &'static str {
    match group {
        BOTTOM_LEVEL => "bottom level",
        SIMPLE_GROUP => "simple",
        HBOX_GROUP => "hbox",
        ADJUSTED_HBOX_GROUP => "adjusted hbox",
        VBOX_GROUP => "vbox",
        VTOP_GROUP => "vtop",
        ALIGN_GROUP => "align",
        NO_ALIGN_GROUP => "no align",
        OUTPUT_GROUP => "output",
        DISC_GROUP => "disc",
        INSERT_GROUP => "insert",
        VCENTER_GROUP => "vcenter",
        MATH_GROUP => "math",
        MATH_CHOICE_GROUP => "math choice",
        SEMI_SIMPLE_GROUP => "semi simple",
        MATH_SHIFT_GROUP => "math shift",
        MATH_LEFT_GROUP => "math left",
        _ => "unknown",
    }
}

/// Formats a fixed-point number without a unit, preserving TeX's `.0`.
fn scaled_number(value: i32) -> String {
    format_scaled_pt(value)
        .strip_suffix("pt")
        .unwrap_or_default()
        .to_string()
}

/// Formats a fixed-point number with a unit.
fn scaled_text(value: i32, unit: &str) -> String {
    format!("{}{}", scaled_number(value), unit)
}

/// Returns the unit suffix for a glue order (`print_glue`/`print_spec`).
fn glue_order_unit(order: u16, unit: &'static str) -> &'static str {
    match order {
        NORMAL => {
            if unit == "mu" {
                "mu"
            } else {
                "pt"
            }
        }
        FIL => "fil",
        FILL => "fill",
        FILLL => "filll",
        _ => unit,
    }
}

/// Formats a glue amount and order (`print_glue`).
fn glue_amount_text(value: i32, order: u16) -> String {
    format!("{}{}", scaled_number(value), glue_order_unit(order, "pt"))
}

/// Formats a rule dimension, using `*` for C's null/default rule marker.
fn rule_dimen_text(value: i32) -> String {
    if value == DEFAULT_RULE {
        "*".to_string()
    } else {
        scaled_text(value, "pt")
    }
}

/// Formats a read/write stream number for whatsit diagnostics
/// (`print_write_whatsit`): `<16` prints the number, `16` prints `*`, higher
/// prints `-`.
fn stream_text(stream: i32) -> String {
    if stream < 16 {
        stream.to_string()
    } else if stream == 16 {
        "*".to_string()
    } else {
        "-".to_string()
    }
}

/// Returns the `' in <mode>` suffix for `you_cant` diagnostics
/// (`print_in_mode`).
fn in_mode_text(m: i16) -> &'static str {
    if m > 0 {
        match i32::from(m) / 103 {
            0 => "' in vertical mode",
            1 => "' in horizontal mode",
            _ => "' in display math mode",
        }
    } else if m == 0 {
        "' in no mode"
    } else {
        match i32::from(-m) / 103 {
            0 => "' in internal vertical mode",
            1 => "' in restricted horizontal mode",
            _ => "' in math mode",
        }
    }
}

/// Returns the `\...style` diagnostic name for a math style code
/// (`print_style`).
fn style_text(c: i32) -> &'static str {
    match c / 2 {
        0 => "\\displaystyle",
        1 => "\\textstyle",
        2 => "\\scriptstyle",
        3 => "\\scriptscriptstyle",
        _ => "Unknown style!",
    }
}

/// `SHORTHAND_DEF` subcommand codes (`xetex_format.h`).
const CHAR_DEF_CODE: i32 = 0;
const MATH_CHAR_DEF_CODE: i32 = 1;
const COUNT_DEF_CODE: i32 = 2;
const DIMEN_DEF_CODE: i32 = 3;
const SKIP_DEF_CODE: i32 = 4;
const MU_SKIP_DEF_CODE: i32 = 5;
const TOKS_DEF_CODE: i32 = 6;
const CHAR_SUB_DEF_CODE: i32 = 7;
const XETEX_MATH_CHAR_NUM_DEF_CODE: i32 = 8;
const XETEX_MATH_CHAR_DEF_CODE: i32 = 9;

/// `XRAY` subcommand codes (`xetex_format.h`).
const SHOW_CODE: i32 = 0;
const SHOW_BOX_CODE: i32 = 1;
const SHOW_THE_CODE: i32 = 2;
const SHOW_LISTS_CODE: i32 = 3;
const SHOW_GROUPS_CODE: i32 = 4;
const SHOW_TOKENS_CODE: i32 = 5;
const SHOW_IFS_CODE: i32 = 6;

/// `EXTENSION` subcommand codes (`xetex_format.h`).
const IMMEDIATE_CODE: i32 = 4;
const SET_LANGUAGE_CODE: i32 = 5;
const RESET_TIMER_CODE: i32 = 31;
const SET_RANDOM_SEED_CODE: i32 = 33;
const PIC_FILE_CODE: i32 = 41;
const PDF_FILE_CODE: i32 = 42;
const GLYPH_CODE: i32 = 43;
const XETEX_INPUT_ENCODING_EXTENSION_CODE: i32 = 44;
const XETEX_DEFAULT_ENCODING_EXTENSION_CODE: i32 = 45;
const XETEX_LINEBREAK_LOCALE_EXTENSION_CODE: i32 = 46;

/// XeTeX input encoding modes (`xetex-xetexd.h`).
const XETEX_INPUT_MODE_AUTO: i32 = 0;
const XETEX_INPUT_MODE_UTF8: i32 = 1;
const XETEX_INPUT_MODE_UTF16BE: i32 = 2;
const XETEX_INPUT_MODE_UTF16LE: i32 = 3;
const XETEX_INPUT_MODE_RAW: i32 = 4;
const XETEX_INPUT_MODE_ICUMAPPING: i32 = 5;

/// Glue/kern subtype codes used by diagnostic list printing.
const C_LEADERS_CODE: u16 = 101;
const X_LEADERS_CODE: u16 = 102;
const COND_MATH_GLUE_CODE: u16 = 98;
const ACC_KERN_CODE: u16 = 2;
const SPACE_ADJUSTMENT_CODE: u16 = 3;
const DLIST_CODE: u16 = 2;

/// Maximum legacy `\sfcode` value.
const MAX_SF_CODE: i32 = 0x7FFF;
/// Maximum legacy `\delcode` value.
const MAX_DEL_CODE: i32 = 0xFF_FFFF;
/// Flag stored in an extended XeTeX delimiter code.
const EXTENDED_DEL_CODE_FLAG: i32 = 0x4000_0000;

#[derive(Clone, Copy)]
enum TokenListTarget {
    Dense(usize),
    Sparse(i32),
}

/// Resolved picture path and `(x, y, w, h)` bounds in TeX points
/// (`find_pic_file`'s `*path` + `*bounds` out-params).
type PicFile = (String, (f64, f64, f64, f64));

fn transform_point(point: (f64, f64), transform: (f64, f64, f64, f64, f64, f64)) -> (f64, f64) {
    (
        point.0 * transform.0 + point.1 * transform.2 + transform.4,
        point.0 * transform.1 + point.1 * transform.3 + transform.5,
    )
}

fn concat_transform(
    left: (f64, f64, f64, f64, f64, f64),
    right: (f64, f64, f64, f64, f64, f64),
) -> (f64, f64, f64, f64, f64, f64) {
    (
        left.0 * right.0 + left.1 * right.2,
        left.0 * right.1 + left.1 * right.3,
        left.2 * right.0 + left.3 * right.2,
        left.2 * right.1 + left.3 * right.3,
        left.4 * right.0 + left.5 * right.2 + right.4,
        left.4 * right.1 + left.5 * right.3 + right.5,
    )
}

fn bounds_of_corners(corners: &[(f64, f64); 4]) -> (f64, f64, f64, f64) {
    let (mut xmin, mut ymin) = corners[0];
    let (mut xmax, mut ymax) = corners[0];
    for &(x, y) in corners.iter().skip(1) {
        xmin = xmin.min(x);
        xmax = xmax.max(x);
        ymin = ymin.min(y);
        ymax = ymax.max(y);
    }
    (xmin, xmax, ymin, ymax)
}

fn requested_scale(x_req: f64, y_req: f64, width: f64, height: f64) -> (f64, f64) {
    if x_req == 0.0 {
        let scale = y_req / height;
        (scale, scale)
    } else if y_req == 0.0 {
        let scale = x_req / width;
        (scale, scale)
    } else {
        (x_req / width, y_req / height)
    }
}

fn d2fix(value: f64) -> i32 {
    (value * 65_536.0).round() as i32
}

fn write_pic_path(words: &mut [crate::state::MemoryWord], path: &[u8]) {
    for (index, &byte) in path.iter().enumerate() {
        let word = 9 + index / 8;
        let shift = (index % 4) * 8;
        let value = i32::from(byte) << shift;
        if (index % 8) < 4 {
            words[word].s0 |= value;
        } else {
            words[word].s1 |= value;
        }
    }
}

impl TeXEngineState {
    /// Reads one expanded token and dispatches the implemented mode-independent
    /// command cases (`main_control`'s `big_switch` prefix).
    pub fn dispatch_x_command(&mut self) -> Result<(), TeXMouthError> {
        self.get_x_token()?;
        self.dispatch_current_command()
    }

    /// Dispatches the currently scanned command for the implemented
    /// mode-independent stomach cases.
    pub fn dispatch_current_command(&mut self) -> Result<(), TeXMouthError> {
        // C `main_control` reswitch: `\tracingcommands` reports every command
        // that reaches the stomach, including recursive re-dispatches.
        if self.int_par(crate::state::INT_PAR_TRACING_COMMANDS)? > 0 {
            self.show_cur_cmd_chr()?;
        }
        let command = CommandCode::from_u8(self.cur_cmd);

        // C's first `switch (abs(mode) + cur_cmd)` cases (C ~16953):
        // HMODE characters enter `main_loop`; `HMODE + NO_BOUNDARY`
        // peeks at the next token and sets `cancel_boundary`.
        if self.mode.abs() == TeXMode::Hmode as i16 {
            match command {
                Some(CommandCode::Letter | CommandCode::OtherChar | CommandCode::CharGiven) => {
                    return self.main_loop_char();
                }
                Some(CommandCode::CharNum) => {
                    self.scan_usv_num()?;
                    self.cur_chr = self.cur_val;
                    return self.main_loop_char();
                }
                Some(CommandCode::NoBoundary) => {
                    self.get_x_token()?;
                    if self.is_char_cmd() || self.cur_cmd == CommandCode::CharNum as u8 {
                        self.cancel_boundary = true;
                    }
                    return self.dispatch_current_command();
                }
                _ => {}
            }
        }

        // Shared hmode end-of-word inter-char injection (C ~16975):
        // runs before the second-mode switch for every non-character
        // command in horizontal mode.
        if self.mode.abs() == TeXMode::Hmode as i16
            && self.int_par(INT_PAR_XETEX_INTER_CHAR_TOKENS)? > 0
            && self.space_class != CHAR_CLASS_LIMIT
            && self.prev_class != CHAR_CLASS_LIMIT - 1
        {
            self.prev_class = CHAR_CLASS_LIMIT - 1;
            if self.inter_char_tokens(
                self.space_class * CHAR_CLASS_LIMIT + CHAR_CLASS_LIMIT - 1,
                false,
                false,
            )? {
                return Ok(());
            }
        }

        if self.mode.abs() == TeXMode::Vmode as i16
            && matches!(
                command,
                Some(
                    CommandCode::Letter
                        | CommandCode::OtherChar
                        | CommandCode::CharNum
                        | CommandCode::CharGiven
                        | CommandCode::MathShift
                        | CommandCode::UnHbox
                        | CommandCode::Vrule
                        | CommandCode::Accent
                        | CommandCode::Discretionary
                        | CommandCode::Hskip
                        | CommandCode::Valign
                        | CommandCode::ExSpace
                        | CommandCode::NoBoundary,
                )
            )
        {
            self.back_input()?;
            self.new_graf(true)?;
            self.get_x_token()?;
            return self.dispatch_current_command();
        }
        if self.mode == TeXMode::Hmode as i16
            && matches!(
                command,
                Some(
                    CommandCode::Stop
                        | CommandCode::Vskip
                        | CommandCode::Hrule
                        | CommandCode::UnVbox
                        | CommandCode::Halign,
                )
            )
        {
            // C `head_for_vmode`: pushes the pending command back, then a
            // `\par` on top so the paragraph ends first.
            return self.head_for_vmode();
        }

        match command {
            Some(CommandCode::Relax) => Ok(()),
            Some(CommandCode::Spacer) => {
                // C `HMODE + SPACER` (C ~17005): space factor 1000 takes
                // `append_normal_space`; otherwise `app_space`.
                if self.mode.abs() == TeXMode::Hmode as i16 {
                    if self.aux.s0 == 1000 {
                        self.append_normal_space()
                    } else {
                        self.app_space()
                    }
                } else {
                    Ok(())
                }
            }
            Some(CommandCode::ExSpace) => {
                // C `HMODE + EX_SPACE` / `MMODE + EX_SPACE` (C ~17013).
                if self.mode.abs() == TeXMode::Hmode as i16
                    || self.mode.abs() == TeXMode::Mmode as i16
                {
                    self.append_normal_space()
                } else {
                    Ok(())
                }
            }
            Some(CommandCode::ItalCorr) => {
                // C `HMODE + ITAL_CORR` (C ~17238); MMODE appends a zero
                // kern; `VMODE + ITAL_CORR` → `report_illegal_case`.
                if self.mode.abs() == TeXMode::Mmode as i16 {
                    let kern = self.new_kern(0)?;
                    self.set_link(self.tail, kern)?;
                    self.tail = kern;
                    Ok(())
                } else if self.mode.abs() == TeXMode::Hmode as i16 {
                    self.append_italic_correction()
                } else {
                    self.report_illegal_case();
                    Ok(())
                }
            }
            Some(CommandCode::Discretionary) => {
                if self.mode.abs() == TeXMode::Hmode as i16
                    || self.mode.abs() == TeXMode::Mmode as i16
                {
                    self.append_discretionary()
                } else {
                    Err(TeXMouthError::UnsupportedPrimitive(
                        "mode-incompatible discretionary",
                    ))
                }
            }
            Some(CommandCode::Accent) => {
                if self.mode.abs() == TeXMode::Hmode as i16 {
                    self.make_accent()
                } else if self.mode.abs() == TeXMode::Mmode as i16 {
                    // C `MMODE + ACCENT` (C ~17397) routes into `math_ac`,
                    // which reports the `\accent` → `\mathaccent` fix.
                    self.math_ac()
                } else {
                    self.report_illegal_case();
                    Ok(())
                }
            }
            Some(CommandCode::NoBoundary) => {
                // `MMODE + NO_BOUNDARY` is ignored; HMODE/VMODE paths are
                // handled above the second switch.
                Ok(())
            }
            Some(CommandCode::IgnoreSpaces) => {
                if self.cur_chr != 0 {
                    return Err(TeXMouthError::UnsupportedPrimitive("frozen \\primitive"));
                }
                loop {
                    self.get_x_token()?;
                    if self.cur_cmd != CommandCode::Spacer as u8 {
                        break;
                    }
                }
                self.dispatch_current_command()
            }
            Some(
                CommandCode::ToksRegister
                | CommandCode::AssignToks
                | CommandCode::AssignInt
                | CommandCode::AssignDimen
                | CommandCode::AssignGlue
                | CommandCode::AssignMuGlue
                | CommandCode::Register
                | CommandCode::Advance
                | CommandCode::Multiply
                | CommandCode::Divide
                | CommandCode::Prefix
                | CommandCode::Let
                | CommandCode::ShorthandDef
                | CommandCode::ReadToCs
                | CommandCode::DefCode
                | CommandCode::XetexDefCode
                | CommandCode::SetAux
                | CommandCode::SetPrevGraf
                | CommandCode::SetPageDimen
                | CommandCode::SetPageInt
                | CommandCode::SetBoxDimen
                | CommandCode::SetShape
                | CommandCode::SetInteraction
                | CommandCode::SetBox
                | CommandCode::DefFamily
                | CommandCode::SetFont
                | CommandCode::DefFont
                | CommandCode::HyphData
                | CommandCode::AssignFontDimen
                | CommandCode::AssignFontInt
                | CommandCode::Def,
            ) => self.prefixed_command(),
            Some(CommandCode::MakeBox) => self.begin_box(0),
            Some(CommandCode::LeaderShip) => {
                self.scan_box(LEADER_FLAG - i32::from(A_LEADERS as u16) + self.cur_chr)
            }
            Some(CommandCode::UnHbox | CommandCode::UnVbox) => {
                let legal = if self.cur_cmd == CommandCode::UnVbox as u8 {
                    self.mode.abs() == TeXMode::Vmode as i16
                } else {
                    self.mode.abs() == TeXMode::Hmode as i16
                        || self.mode.abs() == TeXMode::Mmode as i16
                };
                if !legal {
                    // C `MMODE + UN_VBOX` → `insert_dollar_sign`;
                    // `HMODE + UN_VBOX` in restricted hmode → `head_for_vmode`
                    // → `off_save`. Other combos are unreachable here.
                    if self.mode.abs() == TeXMode::Mmode as i16 {
                        return self.insert_dollar_sign();
                    }
                    return self.head_for_vmode();
                }
                self.unpackage()
            }
            Some(CommandCode::Hmove | CommandCode::Vmove) => {
                let legal = if self.cur_cmd == CommandCode::Hmove as u8 {
                    self.mode == TeXMode::Vmode as i16
                } else {
                    self.mode.abs() == TeXMode::Hmode as i16
                        || self.mode.abs() == TeXMode::Mmode as i16
                };
                if !legal {
                    return Err(TeXMouthError::UnsupportedPrimitive("box movement"));
                }
                let direction = self.cur_chr;
                self.scan_dimen(false, false, false)?;
                let context = if direction == 0 {
                    self.cur_val
                } else {
                    -self.cur_val
                };
                self.scan_box(context)
            }
            Some(CommandCode::Hrule | CommandCode::Vrule) => {
                let legal = if self.cur_cmd == CommandCode::Hrule as u8 {
                    self.mode.abs() == TeXMode::Vmode as i16
                } else {
                    self.mode.abs() == TeXMode::Hmode as i16
                        || self.mode.abs() == TeXMode::Mmode as i16
                };
                if !legal {
                    // C `MMODE + HRULE` → `insert_dollar_sign`;
                    // `HMODE + HRULE` in restricted hmode → `head_for_vmode`
                    // (the leaders diagnostic).
                    if self.mode.abs() == TeXMode::Mmode as i16 {
                        return self.insert_dollar_sign();
                    }
                    return self.head_for_vmode();
                }
                let rule = self.scan_rule_spec()?;
                self.mem[self.tail as usize].s1 = rule;
                self.tail = rule;
                if self.mode.abs() == TeXMode::Vmode as i16 {
                    self.aux.s1 = IGNORE_DEPTH;
                } else if self.mode.abs() == TeXMode::Hmode as i16 {
                    self.aux.s0 = 1000;
                }
                Ok(())
            }
            Some(CommandCode::Hskip | CommandCode::Vskip | CommandCode::Mskip) => {
                let legal = match CommandCode::from_u8(self.cur_cmd) {
                    Some(CommandCode::Hskip) => {
                        self.mode.abs() == TeXMode::Hmode as i16
                            || self.mode.abs() == TeXMode::Mmode as i16
                    }
                    Some(CommandCode::Vskip) => self.mode.abs() == TeXMode::Vmode as i16,
                    Some(CommandCode::Mskip) => self.mode.abs() == TeXMode::Mmode as i16,
                    _ => false,
                };
                if !legal {
                    // C `MMODE + VSKIP` → `insert_dollar_sign`;
                    // `HMODE + VSKIP` in restricted hmode → `head_for_vmode`;
                    // `NON_MATH(MSKIP)` → `insert_dollar_sign`.
                    if self.cur_cmd == CommandCode::Mskip as u8
                        || self.mode.abs() == TeXMode::Mmode as i16
                    {
                        return self.insert_dollar_sign();
                    }
                    return self.head_for_vmode();
                }
                self.append_glue()
            }
            Some(CommandCode::Kern) => self.append_kern(),
            Some(CommandCode::Mkern) => {
                if self.mode.abs() != TeXMode::Mmode as i16 {
                    return self.insert_dollar_sign();
                }
                self.append_kern()
            }
            Some(CommandCode::BreakPenalty) => self.append_penalty(),
            Some(CommandCode::RemoveItem) => self.delete_last(),
            Some(CommandCode::Insert) => self.begin_insert_or_adjust(),
            Some(CommandCode::Vadjust) => {
                if self.mode.abs() != TeXMode::Hmode as i16
                    && self.mode.abs() != TeXMode::Mmode as i16
                {
                    // C `VMODE + VADJUST` → `report_illegal_case`.
                    self.report_illegal_case();
                    return Ok(());
                }
                self.begin_insert_or_adjust()
            }
            Some(CommandCode::StartPar) => {
                if self.mode == TeXMode::Vmode as i16 {
                    self.new_graf(self.cur_chr > 0)
                } else if self.mode.abs() == TeXMode::Hmode as i16
                    || self.mode.abs() == TeXMode::Mmode as i16
                {
                    self.indent_in_hmode()
                } else {
                    Err(TeXMouthError::UnsupportedPrimitive("paragraph start"))
                }
            }
            Some(CommandCode::ParEnd) => {
                if self.mode == TeXMode::Vmode as i16 {
                    self.normal_paragraph()?;
                    if self.mode > 0 {
                        self.build_page()?;
                    }
                    Ok(())
                } else if self.mode.abs() == TeXMode::Hmode as i16 {
                    // C `HMODE + PAR_END` (C ~17198): a negative
                    // `align_state` means the preamble is still open.
                    if self.align_state < 0 {
                        self.off_save()?;
                    }
                    self.end_graf()?;
                    if self.mode == TeXMode::Vmode as i16 {
                        self.build_page()?;
                    }
                    Ok(())
                } else {
                    // C `MMODE + PAR_END` → `insert_dollar_sign`.
                    self.insert_dollar_sign()
                }
            }
            Some(CommandCode::Stop) => {
                if self.mode == TeXMode::Vmode as i16 {
                    if self.its_all_over()? {
                        self.job_stopped = true;
                        if !self.dvi_finalized {
                            self.finalize_dvi_file()?;
                            self.dvi_finalized = true;
                        }
                        self.final_cleanup()?;
                    }
                    Ok(())
                } else if self.mode.abs() == TeXMode::Mmode as i16 {
                    self.insert_dollar_sign()
                } else {
                    self.head_for_vmode()
                }
            }
            Some(CommandCode::LeftBrace) => {
                if self.mode.abs() == TeXMode::Mmode as i16 {
                    // C `MMODE + LEFT_BRACE` (C ~17316): appends a noad and
                    // rescans the group as a math field.
                    let noad = self.new_noad()?;
                    self.mem[self.tail as usize].s1 = noad;
                    self.tail = noad;
                    self.back_input()?;
                    return self.scan_math(self.tail + 1);
                }
                self.new_save_level(SIMPLE_GROUP)?;
                Ok(())
            }
            Some(CommandCode::BeginGroup) => {
                self.new_save_level(SEMI_SIMPLE_GROUP)?;
                Ok(())
            }
            Some(CommandCode::EndGroup) => {
                if self.cur_group == SEMI_SIMPLE_GROUP {
                    self.unsave()
                } else {
                    self.off_save()
                }
            }
            Some(CommandCode::RightBrace) => self.handle_right_brace(),
            Some(CommandCode::AfterAssignment) => {
                self.get_token()?;
                self.after_token = self.cur_tok;
                Ok(())
            }
            Some(CommandCode::AfterGroup) => {
                self.get_token()?;
                self.save_for_after(self.cur_tok)?;
                Ok(())
            }
            Some(CommandCode::Mark) => self.make_mark(),
            Some(CommandCode::InStream) => self.open_or_close_in(),
            Some(CommandCode::Extension) => self.do_extension(),
            Some(CommandCode::Message) => self.issue_message(),
            Some(CommandCode::CaseShift) => self.shift_case(),
            Some(CommandCode::Xray) => self.show_whatever(),
            Some(CommandCode::Halign) => {
                if self.mode.abs() == TeXMode::Vmode as i16 {
                    // C `VMODE + HALIGN` (C ~17269).
                    self.init_align()
                } else if self.mode.abs() == TeXMode::Mmode as i16 {
                    // C `MMODE + HALIGN` (C ~17283).
                    if !self.privileged() {
                        Ok(())
                    } else if self.cur_group == MATH_SHIFT_GROUP {
                        self.init_align()
                    } else {
                        self.off_save()
                    }
                } else {
                    // Restricted hmode: C `HMODE + HALIGN` is part of the
                    // `head_for_vmode` group (C ~17210).
                    self.head_for_vmode()
                }
            }
            Some(CommandCode::Valign) => {
                if self.mode.abs() == TeXMode::Hmode as i16 {
                    // C `HMODE + VALIGN` (C ~17273): `cur_chr > 0` is the
                    // TeX--XeT `\beginL`/`\beginR` box direction node.
                    if self.cur_chr > 0 {
                        if self
                            .etex_enabled(self.int_par(INT_PAR_TEXXET)? > 0, "\\valign direction")
                        {
                            let math = self.new_math(0, self.cur_chr)?;
                            self.mem[self.tail as usize].s1 = math;
                            self.tail = math;
                        }
                        Ok(())
                    } else {
                        self.init_align()
                    }
                } else {
                    // C `MMODE + VALIGN` → `insert_dollar_sign`; vertical
                    // mode is redirected by `new_graf` above.
                    self.insert_dollar_sign()
                }
            }
            Some(CommandCode::Endv) => {
                if self.mode.abs() == TeXMode::Mmode as i16 {
                    // C `MMODE + ENDV` → `insert_dollar_sign` (C ~17094).
                    self.insert_dollar_sign()
                } else {
                    // C `VMODE + ENDV` / `HMODE + ENDV` (C ~17292).
                    self.do_endv()
                }
            }
            Some(CommandCode::CarRet | CommandCode::TabMark) => self.align_error(),
            Some(CommandCode::NoAlign) => self.no_align_error(),
            Some(CommandCode::Omit) => self.omit_error(),
            Some(CommandCode::EndCsName) => {
                self.cs_error();
                Ok(())
            }
            Some(CommandCode::MacParam | CommandCode::LastItem) => {
                // C `ANY_MODE(MAC_PARAM)` / `ANY_MODE(LAST_ITEM)` →
                // `report_illegal_case`.
                self.report_illegal_case();
                Ok(())
            }
            Some(
                CommandCode::Letter
                | CommandCode::OtherChar
                | CommandCode::CharGiven
                | CommandCode::CharNum
                | CommandCode::SupMark
                | CommandCode::SubMark
                | CommandCode::MathCharNum
                | CommandCode::MathGiven
                | CommandCode::XetexMathGiven
                | CommandCode::MathComp
                | CommandCode::DelimNum
                | CommandCode::LeftRight
                | CommandCode::Above
                | CommandCode::Radical
                | CommandCode::MathStyle
                | CommandCode::MathChoice
                | CommandCode::Vcenter
                | CommandCode::NonScript
                | CommandCode::LimitSwitch
                | CommandCode::MathAccent,
            ) => {
                if self.mode.abs() != TeXMode::Mmode as i16 {
                    // C `NON_MATH(...)` → `insert_dollar_sign` (C ~17072).
                    return self.insert_dollar_sign();
                }
                self.math_list_command()
            }
            Some(CommandCode::MathShift) => {
                if self.mode.abs() == TeXMode::Mmode as i16 {
                    // C `MMODE + MATH_SHIFT` (C ~17441).
                    if self.cur_group == MATH_SHIFT_GROUP {
                        self.after_math()
                    } else {
                        self.off_save()
                    }
                } else {
                    // C `HMODE + MATH_SHIFT` (C ~17303); vertical mode was
                    // redirected by `new_graf` above.
                    self.init_math()
                }
            }
            Some(CommandCode::EqNo) => {
                if self.mode.abs() == TeXMode::Mmode as i16 {
                    // C `MMODE + EQ_NO` (C ~17307).
                    if !self.privileged() {
                        Ok(())
                    } else if self.cur_group == MATH_SHIFT_GROUP {
                        self.start_eq_no()
                    } else {
                        self.off_save()
                    }
                } else {
                    // C `NON_MATH(EQ_NO)` → `report_illegal_case`.
                    self.report_illegal_case();
                    Ok(())
                }
            }
            _ => Err(TeXMouthError::UnsupportedPrimitive("main command")),
        }
    }

    /// Executes a prefixed or directly assignable primitive
    /// (`prefixed_command`, bounded to the implemented cases).
    ///
    /// This preserves C's prefix parity (`a`), `\globaldefs` coercion,
    /// `PROTECTED_TOKEN` insertion, local/global definition split, and
    /// `after_token` reinsertion epilogue. Unsupported C switch cases return
    /// [`TeXMouthError::UnsupportedPrimitive`] rather than invoking C's
    /// `confusion("prefix")` abort path.
    pub fn prefixed_command(&mut self) -> Result<(), TeXMouthError> {
        let mut a = 0i32;

        while self.cur_cmd == CommandCode::Prefix as u8 {
            let prefix = self.cur_chr;
            if prefix <= 0 {
                return Err(TeXMouthError::UnsupportedPrimitive("prefix code"));
            }
            if (a / prefix) % 2 == 0 {
                a += prefix;
            }

            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8
                    && self.cur_cmd != CommandCode::Relax as u8
                {
                    break;
                }
            }

            if self.cur_cmd <= MAX_NON_PREFIXED_COMMAND {
                // C reports "You can't use a prefix with ..." and returns via
                // back_error; the token is left in the input for recovery.
                self.back_input()?;
                return Ok(());
            }

            if self.int_par(crate::state::INT_PAR_TRACING_COMMANDS)? > 2 {
                self.show_cur_cmd_chr()?;
            }
        }

        let mut protected_marker = TEX_NULL;
        if a >= 8 {
            protected_marker = PROTECTED_TOKEN;
            a -= 8;
        }

        // C emits "I'll pretend you didn't say \long or \outer or \protected
        // here." and continues. There is no diagnostic channel yet, so the
        // recovery path intentionally continues silently.
        if self.cur_cmd != CommandCode::Def as u8 && (a % 4 != 0 || protected_marker != TEX_NULL) {
            // Recovery only.
        }

        let global_defs = self.int_par(INT_PAR_GLOBAL_DEFS)?;
        if global_defs < 0 {
            if a >= 4 {
                a -= 4;
            }
        } else if global_defs > 0 && a < 4 {
            a += 4;
        }

        match CommandCode::from_u8(self.cur_cmd) {
            Some(CommandCode::Def) => self.prefixed_def(a, protected_marker)?,
            Some(CommandCode::Let) => self.prefixed_let(a)?,
            Some(CommandCode::ShorthandDef) => self.prefixed_shorthand_def(a)?,
            Some(CommandCode::ReadToCs) => self.prefixed_read_to_cs(a)?,
            Some(CommandCode::DefCode | CommandCode::XetexDefCode) => self.prefixed_def_code(a)?,
            Some(CommandCode::AssignInt) => {
                let p = self.checked_eqtb_payload(self.cur_chr)?;
                self.scan_optional_equals()?;
                self.scan_int()?;
                self.define_word(p, a >= 4)?;
            }
            Some(CommandCode::AssignDimen) => {
                let p = self.checked_eqtb_payload(self.cur_chr)?;
                self.scan_optional_equals()?;
                self.scan_dimen(false, false, false)?;
                self.define_word(p, a >= 4)?;
            }
            Some(CommandCode::AssignGlue | CommandCode::AssignMuGlue) => {
                let p = self.checked_eqtb_payload(self.cur_chr)?;
                let level = if self.cur_cmd == CommandCode::AssignMuGlue as u8 {
                    MU_VAL
                } else {
                    GLUE_VAL
                };
                self.scan_optional_equals()?;
                self.scan_glue(level)?;
                self.trap_zero_glue()?;
                self.define_glue(p, a >= 4)?;
            }
            Some(
                CommandCode::Register
                | CommandCode::Advance
                | CommandCode::Multiply
                | CommandCode::Divide,
            ) => self.do_register_command(a)?,
            Some(CommandCode::ToksRegister | CommandCode::AssignToks) => {
                self.prefixed_assign_toks(a)?;
            }
            Some(CommandCode::SetAux) => self.alter_aux()?,
            Some(CommandCode::SetPrevGraf) => self.alter_prev_graf()?,
            Some(CommandCode::SetPageDimen) => self.alter_page_so_far()?,
            Some(CommandCode::SetPageInt) => self.alter_integer()?,
            Some(CommandCode::SetBoxDimen) => self.alter_box_dimen()?,
            Some(CommandCode::SetShape) => self.prefixed_set_shape(a)?,
            Some(CommandCode::HyphData) => self.prefixed_hyph_data()?,
            Some(CommandCode::SetInteraction) => self.new_interaction(),
            Some(CommandCode::SetBox) => {
                self.scan_register_num()?;
                let context = if a >= 4 {
                    GLOBAL_BOX_FLAG + self.cur_val
                } else {
                    BOX_FLAG + self.cur_val
                };
                self.scan_optional_equals()?;
                self.scan_box(context)?;
            }
            Some(CommandCode::SetFont) => {
                // C ~1386: `eq_define(CUR_FONT_LOC, DATA, cur_chr)`.
                self.define_eqtb(CUR_FONT_LOC, CommandCode::Data as u16, self.cur_chr, a >= 4)?;
            }
            Some(CommandCode::DefFamily) => {
                // C ~1891: `p = cur_chr; scan_math_fam_int; scan_font_ident`.
                let base = FAMILY_BASE as i32 + (self.cur_chr - MATH_FONT_BASE_C);
                self.scan_math_fam_int()?;
                let p = usize::try_from(base + self.cur_val)
                    .ok()
                    .filter(|&index| index < self.eqtb.len())
                    .ok_or(TeXMouthError::InputOutOfBounds(base + self.cur_val))?;
                self.scan_optional_equals()?;
                self.scan_font_ident()?;
                self.define_eqtb(p, CommandCode::Data as u16, self.cur_val, a >= 4)?;
            }
            Some(CommandCode::AssignFontDimen) => {
                // C ~2015: `find_font_dimen(true); scan_dimen; font_info[k]`.
                if let Some((font, index)) = self.find_font_dimen(true)? {
                    self.scan_optional_equals()?;
                    self.scan_dimen(false, false, false)?;
                    let value = self.cur_val;
                    if let Some(record) = self.font_record_mut(font) {
                        if index < record.params.len() {
                            record.params[index] = value;
                        }
                    }
                } else {
                    // C's error path still scans the value before continuing.
                    self.scan_optional_equals()?;
                    self.scan_dimen(false, false, false)?;
                }
            }
            Some(CommandCode::AssignFontInt) => self.prefixed_assign_font_int()?,
            Some(CommandCode::DefFont) => self.new_font(a)?,
            _ => return Err(TeXMouthError::UnsupportedPrimitive("prefixed command")),
        }

        if self.after_token != TEX_NULL {
            self.cur_tok = self.after_token;
            self.back_input()?;
            self.after_token = TEX_NULL;
        }
        Ok(())
    }

    /// Scans a packaging specification (`scan_spec`) and opens its group.
    ///
    /// Raw context slots occupy the save-stack positions immediately before the
    /// new level's line/boundary words, matching the C layout consumed by
    /// `package` and `fin_align`.
    pub fn scan_spec(&mut self, group: u8, three_codes: bool) -> Result<(), TeXMouthError> {
        self.check_save_stack().map_err(TeXMouthError::Memory)?;
        let saved = if three_codes {
            match self.save_stack.get(self.save_ptr) {
                Some(SaveStackEntry::RawValue(value)) => *value,
                _ => {
                    return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                        self.save_ptr as i32,
                    )))
                }
            }
        } else {
            0
        };

        let spec_code = if self.scan_keyword("to")? {
            EXACTLY
        } else {
            if !self.scan_keyword("spread")? {
                self.cur_val = 0;
                ADDITIONAL
            } else {
                self.scan_dimen(false, false, false)?;
                ADDITIONAL
            }
        };
        if spec_code == EXACTLY {
            self.scan_dimen(false, false, false)?;
        }

        if three_codes {
            self.push_save_entry(SaveStackEntry::RawValue(saved))
                .map_err(TeXMouthError::Memory)?;
        }
        self.push_save_entry(SaveStackEntry::RawValue(spec_code))
            .map_err(TeXMouthError::Memory)?;
        self.push_save_entry(SaveStackEntry::RawValue(self.cur_val))
            .map_err(TeXMouthError::Memory)?;
        self.new_save_level(group).map_err(TeXMouthError::Memory)?;
        self.scan_left_brace()
    }

    /// Defines a macro for the `DEF` branch (`\def`, `\gdef`, `\edef`,
    /// `\xdef`).
    fn prefixed_def(&mut self, mut a: i32, protected_marker: i32) -> Result<(), TeXMouthError> {
        let definition_kind = self.cur_chr;
        if definition_kind % 2 != 0 && a < 4 && self.int_par(INT_PAR_GLOBAL_DEFS)? >= 0 {
            a += 4;
        }

        let xpand = definition_kind >= 2;
        self.get_r_token()?;
        let target = self.cur_cs;
        self.scan_toks(true, xpand)?;

        if protected_marker != TEX_NULL {
            let marker = self.get_avail()?;
            let marker_idx = self.checked_engine_mem_index(marker)?;
            let def_idx = self.checked_engine_mem_index(self.def_ref)?;
            self.mem[marker_idx].s0 = protected_marker;
            self.mem[marker_idx].s1 = self.mem[def_idx].s1;
            self.mem[def_idx].s1 = marker;
        }

        let command = match a % 4 {
            0 => CommandCode::Call,
            1 => CommandCode::LongCall,
            2 => CommandCode::OuterCall,
            _ => CommandCode::LongOuterCall,
        };
        if a >= 4 {
            self.geq_define_cs(target, command, self.def_ref)?;
        } else {
            self.eq_define_cs(target, command, self.def_ref)?;
        }
        Ok(())
    }

    /// Defines a control sequence for the `LET` branch (`\let`,
    /// `\futurelet`).
    fn prefixed_let(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let future = self.cur_chr != NORMAL_LET;
        self.get_r_token()?;
        let target = self.cur_cs;

        if future {
            self.get_token()?;
            let first = self.cur_tok;
            self.get_token()?;
            self.back_input()?;
            self.cur_tok = first;
            self.back_input()?;
        } else {
            loop {
                self.get_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8 {
                    break;
                }
            }
            if self.cur_tok == OTHER_TOKEN + i32::from(b'=') {
                self.get_token()?;
                if self.cur_cmd == CommandCode::Spacer as u8 {
                    self.get_token()?;
                }
            }
        }

        if self.cur_cmd >= CommandCode::Call as u8 {
            let payload = self.checked_engine_mem_index(self.cur_chr)?;
            self.mem[payload].s0 = self.mem[payload].s0.wrapping_add(1);
        } else if matches!(
            CommandCode::from_u8(self.cur_cmd),
            Some(CommandCode::Register | CommandCode::ToksRegister)
        ) && (self.cur_chr < 0 || self.cur_chr > 19)
        {
            let payload = self.checked_engine_mem_index(self.cur_chr + 1)?;
            self.mem[payload].s0 = self.mem[payload].s0.wrapping_add(1);
        }

        let command = CommandCode::from_u8(self.cur_cmd)
            .ok_or(TeXMouthError::UnsupportedPrimitive("let source command"))?;
        if a >= 4 {
            self.geq_define_cs(target, command, self.cur_chr)?;
        } else {
            self.eq_define_cs(target, command, self.cur_chr)?;
        }
        Ok(())
    }

    /// Executes the `SHORTHAND_DEF` branch (`\chardef`, `\mathchardef`,
    /// XeTeX math-char definitions, and dense register-definition forms).
    fn prefixed_shorthand_def(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let definition_kind = self.cur_chr;
        if definition_kind == CHAR_SUB_DEF_CODE {
            return Err(TeXMouthError::UnsupportedPrimitive("charsubdef"));
        }

        self.get_r_token()?;
        let target = self.cur_cs;
        self.define_cs_meaning(target, CommandCode::Relax, TOO_BIG_USV, a >= 4)?;
        self.scan_optional_equals()?;

        let (command, payload) = match definition_kind {
            CHAR_DEF_CODE => {
                self.scan_usv_num()?;
                (CommandCode::CharGiven, self.cur_val)
            }
            MATH_CHAR_DEF_CODE => {
                self.scan_fifteen_bit_int()?;
                (CommandCode::MathGiven, self.cur_val)
            }
            XETEX_MATH_CHAR_NUM_DEF_CODE => {
                self.scan_xetex_math_char_int()?;
                (CommandCode::XetexMathGiven, self.cur_val)
            }
            XETEX_MATH_CHAR_DEF_CODE => {
                self.scan_math_class_int()?;
                let mut value = (self.cur_val & 0x07) << 21;
                self.scan_math_fam_int()?;
                value += (self.cur_val & 0xFF) << 24;
                self.scan_usv_num()?;
                value += self.cur_val;
                (CommandCode::XetexMathGiven, value)
            }
            COUNT_DEF_CODE..=TOKS_DEF_CODE => {
                self.scan_register_num()?;
                if self.cur_val > 255 {
                    let sparse_kind = if definition_kind == TOKS_DEF_CODE {
                        SA_TOK_VAL
                    } else {
                        definition_kind - COUNT_DEF_CODE
                    };
                    let element = self.find_sa_element(sparse_kind, self.cur_val, true)?;
                    let idx = self.checked_engine_mem_index(element + 1)?;
                    self.mem[idx].s0 = self.mem[idx].s0.wrapping_add(1);
                    if sparse_kind == SA_TOK_VAL {
                        (CommandCode::ToksRegister, element)
                    } else {
                        (CommandCode::Register, element)
                    }
                } else {
                    let register = usize::try_from(self.cur_val)
                        .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?;
                    match definition_kind {
                        COUNT_DEF_CODE => (CommandCode::AssignInt, (COUNT_BASE + register) as i32),
                        DIMEN_DEF_CODE => {
                            (CommandCode::AssignDimen, (DIMEN_BASE + register) as i32)
                        }
                        SKIP_DEF_CODE => (CommandCode::AssignGlue, (SKIP_BASE + register) as i32),
                        MU_SKIP_DEF_CODE => {
                            (CommandCode::AssignMuGlue, (MU_SKIP_BASE + register) as i32)
                        }
                        TOKS_DEF_CODE => (CommandCode::AssignToks, (TOKS_BASE + register) as i32),
                        _ => unreachable!(),
                    }
                }
            }
            _ => return Err(TeXMouthError::UnsupportedPrimitive("shorthand definition")),
        };

        self.define_cs_meaning(target, command, payload, a >= 4)
    }

    /// Executes the `READ_TO_CS` branch (`\read` and `\readline`).
    fn prefixed_read_to_cs(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let j = self.cur_chr;
        self.scan_int()?;
        let n = self.cur_val;
        self.scan_keyword("to")?;

        self.get_r_token()?;
        let target = self.cur_cs;
        self.read_toks(n, target, j)?;
        self.define_cs_meaning(target, CommandCode::Call, self.cur_val, a >= 4)
    }

    /// Executes the `DEF_CODE`/`XETEX_DEF_CODE` branch for the dense
    /// per-character code tables.
    fn prefixed_def_code(&mut self, a: i32) -> Result<(), TeXMouthError> {
        if self.cur_cmd == CommandCode::XetexDefCode as u8 {
            self.prefixed_xetex_def_code(a)
        } else {
            self.prefixed_tex_def_code(a)
        }
    }

    /// Executes a legacy character-code assignment (`\catcode`, `\lccode`,
    /// `\uccode`, `\sfcode`, `\mathcode`, or `\delcode`).
    fn prefixed_tex_def_code(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let base = self.cur_chr;
        let (table, maximum) = match base {
            CAT_CODE_BASE => (CODE_TABLE_CAT, 15),
            LC_CODE_BASE => (CODE_TABLE_LC, TOO_BIG_USV),
            UC_CODE_BASE => (CODE_TABLE_UC, TOO_BIG_USV),
            SF_CODE_BASE => (CODE_TABLE_SF, MAX_SF_CODE),
            MATH_CODE_BASE => (CODE_TABLE_MATH, 0x8000),
            DEL_CODE_BASE => (CODE_TABLE_DEL, MAX_DEL_CODE),
            _ => return Err(TeXMouthError::UnsupportedPrimitive("character code table")),
        };

        self.scan_usv_num()?;
        let character = u32::try_from(self.cur_val)
            .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?;
        self.scan_optional_equals()?;
        self.scan_int()?;
        let mut value = self.cur_val;

        if (value < 0 && base != DEL_CODE_BASE) || value > maximum {
            value = 0;
        }

        if base == SF_CODE_BASE {
            let character_class = self.sf_code(character) / 65_536;
            value += character_class * 65_536;
        } else if base == MATH_CODE_BASE {
            value = if value == 0x8000 {
                ACTIVE_MATH_CHAR
            } else {
                ((value / 4096) & 0x07) << 21
                    | (((value % 4096) / 256) & 0xFF) << 24
                    | (value % 256)
            };
        }

        self.define_character_code(table, character, value, a >= 4)
            .map_err(TeXMouthError::from)
    }

    /// Executes a XeTeX character-code assignment (`\XeTeXcharclass`,
    /// `\Umathcodenum`, `\Umathcode`, `\Udelcodenum`, or `\Udelcode`).
    fn prefixed_xetex_def_code(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let base = self.cur_chr;
        self.scan_usv_num()?;
        let character = u32::try_from(self.cur_val)
            .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?;
        self.scan_optional_equals()?;

        let (table, value) = match base {
            SF_CODE_BASE => {
                let space_factor = self.sf_code(character) % 65_536;
                self.scan_char_class()?;
                (CODE_TABLE_SF, self.cur_val * 65_536 + space_factor)
            }
            MATH_CODE_BASE => {
                self.scan_xetex_math_char_int()?;
                (CODE_TABLE_MATH, self.cur_val)
            }
            x if x == MATH_CODE_BASE + 1 => {
                self.scan_math_class_int()?;
                let mut value = (self.cur_val & 0x07) << 21;
                self.scan_math_fam_int()?;
                value += (self.cur_val & 0xFF) << 24;
                self.scan_usv_num()?;
                value += self.cur_val;
                (CODE_TABLE_MATH, value)
            }
            DEL_CODE_BASE => {
                self.scan_int()?;
                (CODE_TABLE_DEL, self.cur_val)
            }
            x if x == DEL_CODE_BASE + 1 => {
                let mut value = EXTENDED_DEL_CODE_FLAG;
                self.scan_math_fam_int()?;
                value += self.cur_val * 0x20_0000;
                self.scan_usv_num()?;
                value += self.cur_val;
                (CODE_TABLE_DEL, value)
            }
            _ => return Err(TeXMouthError::UnsupportedPrimitive("XeTeX character code")),
        };

        self.define_character_code(table, character, value, a >= 4)
            .map_err(TeXMouthError::from)
    }

    /// Executes `alter_aux` for `\prevdepth` and `\spacefactor`.
    fn alter_aux(&mut self) -> Result<(), TeXMouthError> {
        let c = self.cur_chr;
        if c != i32::from(self.mode.abs()) {
            // C reports an illegal case and performs no assignment.
            return Ok(());
        }

        self.scan_optional_equals()?;
        if c == TeXMode::Vmode as i32 {
            self.scan_dimen(false, false, false)?;
            self.aux.s1 = self.cur_val;
        } else {
            self.scan_int()?;
            if (1..=32_767).contains(&self.cur_val) {
                self.aux.s0 = self.cur_val;
            }
        }
        Ok(())
    }

    /// Returns the index of the innermost vertical-mode list, where
    /// `nest_ptr` denotes the current `cur_list` rather than a stored record.
    pub(crate) fn innermost_vmode_index(&self) -> Option<usize> {
        let mut p = self.nest_ptr;
        loop {
            let mode = if p == self.nest_ptr {
                self.mode
            } else {
                self.nest.get(p)?.mode
            };
            if mode.abs() == TeXMode::Vmode as i16 {
                return Some(p);
            }
            if p == 0 {
                return None;
            }
            p -= 1;
        }
    }

    /// Executes `alter_prev_graf`, assigning the innermost vertical list.
    fn alter_prev_graf(&mut self) -> Result<(), TeXMouthError> {
        let p = self
            .innermost_vmode_index()
            .ok_or(TeXMouthError::UnsupportedPrimitive(
                "prevgraf outside vertical mode",
            ))?;
        self.scan_optional_equals()?;
        self.scan_int()?;
        if self.cur_val < 0 {
            // C reports "Bad \prevgraf" and leaves the old value intact.
            return Ok(());
        }
        if p == self.nest_ptr {
            self.prev_graf = self.cur_val;
        } else {
            self.nest[p].prev_graf = self.cur_val;
        }
        Ok(())
    }

    /// Executes `alter_page_so_far` for the eight page dimension quantities.
    fn alter_page_so_far(&mut self) -> Result<(), TeXMouthError> {
        let c = usize::try_from(self.cur_chr)
            .ok()
            .filter(|&c| c < PAGE_SO_FAR_COUNT)
            .ok_or(TeXMouthError::InputOutOfBounds(self.cur_chr))?;
        self.scan_optional_equals()?;
        self.scan_dimen(false, false, false)?;
        self.page_so_far[c] = self.cur_val;
        Ok(())
    }

    /// Executes `alter_integer` for `\deadcycles`, `\insertpenalties`, and
    /// `\interactionmode`.
    fn alter_integer(&mut self) -> Result<(), TeXMouthError> {
        let c = self.cur_chr;
        self.scan_optional_equals()?;
        self.scan_int()?;
        match c {
            0 => self.dead_cycles = self.cur_val,
            2 => {
                if (i32::from(BATCH_MODE)..=i32::from(ERROR_STOP_MODE)).contains(&self.cur_val) {
                    self.cur_chr = self.cur_val;
                    self.new_interaction();
                }
            }
            _ => self.insert_penalties = self.cur_val,
        }
        Ok(())
    }

    /// Executes `alter_box_dimen` for dense and sparse `\wd`, `\dp`, and
    /// `\ht` box registers.
    fn alter_box_dimen(&mut self) -> Result<(), TeXMouthError> {
        let c = self.cur_chr;
        self.scan_register_num()?;
        let b = if self.cur_val > 255 {
            let element = self.find_sa_element(SA_BOX_VAL, self.cur_val, false)?;
            if element == TEX_NULL {
                TEX_NULL
            } else {
                self.sparse_ptr_value(element)?
            }
        } else {
            let register = usize::try_from(self.cur_val)
                .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?;
            self.box_reg(register)?
        };
        self.scan_optional_equals()?;
        self.scan_dimen(false, false, false)?;
        if b != TEX_NULL {
            let index = self.checked_engine_mem_index(
                b.checked_add(c).ok_or(TeXMouthError::InputOutOfBounds(b))?,
            )?;
            self.mem[index].s1 = self.cur_val;
        }
        Ok(())
    }

    /// Executes the `SET_SHAPE` branch for `\parshape` and the e-TeX penalty
    /// arrays.
    fn prefixed_set_shape(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let q = self.checked_eqtb_payload(self.cur_chr)?;
        self.scan_optional_equals()?;
        self.scan_int()?;
        let count = self.cur_val;

        let p = if count <= 0 {
            TEX_NULL
        } else if q > LOCAL_BASE + LOCAL_PAR_SHAPE {
            let pair_words = count
                .checked_div(2)
                .and_then(|n| n.checked_add(1))
                .ok_or(TeXMouthError::InputOutOfBounds(count))?;
            let size = pair_words
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or(TeXMouthError::InputOutOfBounds(count))?;
            let p = self.get_node(size)?;
            let base = self.checked_engine_mem_index(p)?;
            self.mem[base].s0 = pair_words;
            self.mem[base + 1].s1 = count;
            for j in 0..count {
                self.scan_int()?;
                self.mem[base + 2 + j as usize].s1 = self.cur_val;
            }
            if count % 2 == 0 {
                self.mem[base + count as usize + 2].s1 = 0;
            }
            p
        } else {
            let size = count
                .checked_mul(2)
                .and_then(|n| n.checked_add(1))
                .ok_or(TeXMouthError::InputOutOfBounds(count))?;
            let p = self.get_node(size)?;
            let base = self.checked_engine_mem_index(p)?;
            self.mem[base].s0 = count;
            for j in 1..=count {
                self.scan_dimen(false, false, false)?;
                self.mem[base + (2 * j - 1) as usize].s1 = self.cur_val;
                self.scan_dimen(false, false, false)?;
                self.mem[base + (2 * j) as usize].s1 = self.cur_val;
            }
            p
        };

        self.define_eqtb(q, CommandCode::ShapeRef as u16, p, a >= 4)
    }

    /// Executes `new_interaction`, updating the interaction and selector modes.
    fn new_interaction(&mut self) {
        self.interaction = u8::try_from(self.cur_chr).unwrap_or(ERROR_STOP_MODE);
        self.selector = if self.interaction == BATCH_MODE {
            SELECTOR_NO_PRINT
        } else {
            SELECTOR_TERM_ONLY
        };
        if self.log_opened {
            self.selector += 2;
        }
    }

    /// Executes the `ASSIGN_FONT_INT` branch (`\hyphenchar`, `\skewchar`,
    /// `\lpcode`, `\rpcode`; C `prefixed_command` ~2023).
    fn prefixed_assign_font_int(&mut self) -> Result<(), TeXMouthError> {
        let n = self.cur_chr;
        self.scan_font_ident()?;
        let font = self.cur_val;

        if n < LP_CODE_BASE {
            self.scan_optional_equals()?;
            self.scan_int()?;
            let value = self.cur_val;
            if let Some(record) = self.font_record_mut(font) {
                if n == HYPHEN_CHAR_CODE {
                    record.hyphen_char = value;
                } else {
                    record.skew_char = value;
                }
            }
            return Ok(());
        }

        if self.is_native_font(font) {
            self.scan_glyph_number(font)?;
        } else {
            self.scan_char_num()?;
        }
        let key = u32::try_from(self.cur_val)
            .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?;
        self.scan_optional_equals()?;
        self.scan_int()?;
        self.set_cp_code(font, key, n == LP_CODE_BASE, self.cur_val);
        Ok(())
    }

    /// Executes `\font` (`new_font`, C ~16040).
    ///
    /// Scans the target control sequence, installs a provisional `nullfont`
    /// definition, reads the file name plus the `at`/`scaled` size
    /// specification, deduplicates against previously loaded fonts, loads the
    /// record through the state-owned `font_sources` registry, and defines the
    /// control sequence with `SetFont` semantics.
    fn new_font(&mut self, a: i32) -> Result<(), TeXMouthError> {
        self.get_r_token()?;
        let target = self.cur_cs;
        let cs_name = self.cs_table.name(target).to_vec();

        // C installs a provisional nullfont meaning before scanning the name.
        if a >= 4 {
            self.geq_define_cs(target, CommandCode::SetFont, 0)?;
        } else {
            self.eq_define_cs(target, CommandCode::SetFont, 0)?;
        }

        self.scan_optional_equals()?;
        self.scan_file_name()?;
        self.name_in_progress = true;
        let s = if self.scan_keyword("at")? {
            self.scan_dimen(false, false, false)?;
            let s = self.cur_val;
            if s <= 0 || s >= 0x800_0000 {
                self.transcript.push(format!(
                    "! Improper `at' size ({}pt), replaced by 10pt",
                    format_scaled_pt(s)
                ));
                self.error_count += 1;
                10 * 65_536
            } else {
                s
            }
        } else if self.scan_keyword("scaled")? {
            self.scan_int()?;
            if self.cur_val <= 0 || self.cur_val > 32_768 {
                self.transcript
                    .push("! Illegal magnification has been changed to 1000".to_string());
                self.error_count += 1;
                -1000
            } else {
                -self.cur_val
            }
        } else {
            -1000
        };
        self.name_in_progress = false;

        let font = match self.find_existing_font(s)? {
            Some(font) => font,
            None => self.read_font_info(s)?,
        };

        if a >= 4 {
            self.geq_define_cs(target, CommandCode::SetFont, font)?;
        } else {
            self.eq_define_cs(target, CommandCode::SetFont, font)?;
        }
        if let Some(record) = self.font_record_mut(font) {
            record.cs_name = cs_name;
        }
        self.font_ident_cs(font);
        Ok(())
    }

    /// Implements the C `new_font` deduplication loop over previously loaded
    /// fonts: matching name and area (or empty area against a native font),
    /// then the requested size rule (`s > 0` compares `font_size`, otherwise
    /// `xn_over_d(dsize, -s, 1000)`), and the packed `area+name+ext` fallback
    /// for native fonts.
    fn find_existing_font(&mut self, s: i32) -> Result<Option<i32>, TeXMouthError> {
        let cur_name = self.cur_name.clone();
        let cur_area = self.cur_area.clone();
        let cur_ext = self.cur_ext.clone();
        let mut packed = cur_area.clone();
        packed.extend(&cur_name);
        packed.extend(&cur_ext);

        for font in 1..=self.font_ptr() {
            let Some((record_size, record_dsize, matches)) = self.font_record(font).map(|r| {
                let native = self.is_native_font(font);
                let name_area =
                    r.name == cur_name && ((cur_area.is_empty() && native) || r.area == cur_area);
                let packed_native = native && r.name == packed;
                (r.size, r.dsize, name_area || packed_native)
            }) else {
                continue;
            };
            if !matches {
                continue;
            }
            if s > 0 {
                if record_size == s {
                    return Ok(Some(font));
                }
            } else if record_size == self.xn_over_d(record_dsize, -s, 1000) {
                return Ok(Some(font));
            }
        }
        Ok(None)
    }

    /// Loads a font record (`read_font_info`), following C's dispatch:
    /// `quoted_filename` (or any name carrying `[`/`:`/`/` native syntax)
    /// goes straight to `load_native_font`; otherwise the registered
    /// `font_sources`/`input_byte_sources` TFM seam is tried first and a
    /// miss falls back to `load_native_font` before the `bad_tfm` error.
    /// `\XeTeXtracingfonts` diagnostics and the `:mapping=` suffix handling
    /// mirror the C path.
    fn read_font_info(&mut self, s: i32) -> Result<i32, TeXMouthError> {
        let key: String = self
            .cur_area
            .iter()
            .chain(&self.cur_name)
            .filter_map(|&unit| char::from_u32(unit as u32))
            .collect();
        let name_key: String = self
            .cur_name
            .iter()
            .filter_map(|&unit| char::from_u32(unit as u32))
            .collect();

        if self.int_par(INT_PAR_XETEX_TRACING_FONTS)? > 0 {
            let size_text = if s < 0 {
                format!(" scaled {}", -s)
            } else {
                format!(" at {}pt", s as f64 / 65_536.0)
            };
            self.transcript
                .push(format!("Requested font \"{key}\"{size_text}"));
        }

        // `name_of_file` carries the packed name for the native loader and
        // the `:mapping=` check.
        self.name_of_file = key.bytes().collect();
        self.name_length = self.name_of_file.len() as i32;

        let native_syntax = self.quoted_filename
            || name_key.starts_with('[')
            || name_key.contains(':')
            || name_key.contains('/');
        if native_syntax {
            let font = self.load_native_font(s)?;
            if font != 0 {
                return Ok(font);
            }
        }

        self.check_for_tfm_font_mapping();

        let Some(mut record) = self
            .font_sources
            .get(&key)
            .or_else(|| self.font_sources.get(&name_key))
            .cloned()
        else {
            if !native_syntax {
                let font = self.load_native_font(s)?;
                if font != 0 {
                    return Ok(font);
                }
            }
            if self.int_par(INT_PAR_SUPPRESS_FONTNOTFOUND_ERROR)? <= 0 {
                self.transcript.push(format!(
                    "! Font ={key} not loadable: Metric (TFM) file or installed font not found"
                ));
                self.error_count += 1;
            }
            return Ok(0);
        };

        record.name = self.cur_name.clone();
        record.area = self.cur_area.clone();
        record.ext = self.cur_ext.clone();
        record.size = if s > 0 {
            s
        } else {
            self.xn_over_d(record.dsize, -s, 1000)
        };

        let font = self.font_table.len() as i32;
        self.font_table.push(record);
        self.font_id_cs.push(-1);
        // `font_mapping[f] = load_tfm_font_mapping()`.
        if let Some(name) = self.load_tfm_font_mapping() {
            self.font_table[font as usize].mapping_name = Some(name);
        }
        Ok(font)
    }

    /// Records C's `not_native_font_error` diagnostic and continues (`error`).
    pub fn not_native_font_error(&mut self, cmd_text: &str, font: i32) {
        let name = self
            .font_record(font)
            .map(|record| {
                record
                    .cs_name
                    .iter()
                    .filter_map(|&unit| char::from_u32(unit as u32))
                    .collect::<String>()
            })
            .unwrap_or_else(|| "nullfont".to_string());
        self.transcript.push(format!(
            "! Cannot use {cmd_text} with {name}; not a native platform font"
        ));
        self.error_count += 1;
    }

    /// Executes `do_register_command` (`\count`, `\dimen`, `\skip`,
    /// `\muskip`, `\advance`, `\multiply`, and `\divide`) for dense `eqtb`
    /// registers and sparse e-TeX `sa` elements.
    fn do_register_command(&mut self, a: i32) -> Result<(), TeXMouthError> {
        enum RegisterTarget {
            Dense(usize),
            Sparse(i32),
        }

        let operation = self.cur_cmd;
        let mut quantity = self.cur_chr;
        let mut target = None;

        if operation != CommandCode::Register as u8 {
            self.get_x_token()?;
            if (CommandCode::AssignInt as u8..=CommandCode::AssignMuGlue as u8)
                .contains(&self.cur_cmd)
            {
                target = Some(RegisterTarget::Dense(
                    self.checked_eqtb_payload(self.cur_chr)?,
                ));
                quantity = i32::from(self.cur_cmd - CommandCode::AssignInt as u8);
            } else if self.cur_cmd == CommandCode::Register as u8 {
                quantity = self.cur_chr;
                if !(0..=19).contains(&quantity) {
                    target = Some(RegisterTarget::Sparse(quantity));
                    quantity = self.sparse_element_class(quantity)?;
                }
            } else {
                // C reports "You can't use ... after \advance" and resumes
                // without changing a register.
                return Ok(());
            }
        }

        if target.is_none() {
            if !(0..=19).contains(&quantity) {
                target = Some(RegisterTarget::Sparse(quantity));
                quantity = self.sparse_element_class(quantity)?;
            } else {
                if !(0..=3).contains(&quantity) {
                    return Err(TeXMouthError::UnsupportedPrimitive("register class"));
                }
                self.scan_register_num()?;
                let register = self.cur_val;
                target = Some(if register > 255 {
                    let element = self.find_sa_element(quantity, register, true)?;
                    RegisterTarget::Sparse(element)
                } else {
                    let register = usize::try_from(register)
                        .map_err(|_| TeXMouthError::InputOutOfBounds(register))?;
                    RegisterTarget::Dense(match quantity as u8 {
                        INT_VAL => COUNT_BASE + register,
                        DIMEN_VAL => DIMEN_BASE + register,
                        GLUE_VAL => SKIP_BASE + register,
                        MU_VAL => MU_SKIP_BASE + register,
                        _ => unreachable!(),
                    })
                });
            }
        }
        let target = target.unwrap();
        let quantity = quantity as u8;

        let current_word = match target {
            RegisterTarget::Dense(target) if quantity < GLUE_VAL => self.eqtb[target].s1,
            RegisterTarget::Sparse(target) if quantity < GLUE_VAL => {
                self.sparse_word_value(target)?
            }
            _ => 0,
        };
        let current_glue = match target {
            RegisterTarget::Dense(target) if quantity >= GLUE_VAL => self.eqtb[target].s1,
            RegisterTarget::Sparse(target) if quantity >= GLUE_VAL => {
                self.sparse_ptr_value(target)?
            }
            _ => TEX_NULL,
        };

        if operation == CommandCode::Register as u8 {
            self.scan_optional_equals()?;
        } else {
            self.scan_keyword("by")?;
        }
        self.arith_error = false;

        if operation < CommandCode::Multiply as u8 {
            if quantity < GLUE_VAL {
                if quantity == INT_VAL {
                    self.scan_int()?;
                } else {
                    self.scan_dimen(false, false, false)?;
                }
                if operation == CommandCode::Advance as u8 {
                    self.cur_val = self.cur_val.wrapping_add(current_word);
                }
            } else {
                self.scan_glue(quantity)?;
                if operation == CommandCode::Advance as u8 {
                    self.advance_glue(current_glue)?;
                }
            }
        } else {
            self.scan_int()?;
            let operand = self.cur_val;
            if quantity < GLUE_VAL {
                if operation == CommandCode::Multiply as u8 {
                    let max_answer = if quantity == INT_VAL {
                        TEX_INFINITY
                    } else {
                        crate::state::MAX_HALFWORD
                    };
                    self.cur_val = self.mult_and_add(current_word, operand, 0, max_answer);
                } else {
                    self.cur_val = self.x_over_n(current_word, operand);
                }
            } else {
                let result = self.new_spec(current_glue).map_err(TeXMouthError::Memory)?;
                let result_idx = self.checked_engine_mem_index(result)?;
                for offset in 1..=3 {
                    let value = self.mem[result_idx + offset].s1;
                    self.mem[result_idx + offset].s1 = if operation == CommandCode::Multiply as u8 {
                        self.mult_and_add(value, operand, 0, crate::state::MAX_HALFWORD)
                    } else {
                        self.x_over_n(value, operand)
                    };
                }
                self.cur_val = result;
            }
        }

        if self.arith_error {
            if quantity >= GLUE_VAL {
                self.delete_glue_ref(self.cur_val)
                    .map_err(TeXMouthError::Memory)?;
            }
            return Ok(());
        }

        match target {
            RegisterTarget::Dense(target) if quantity < GLUE_VAL => {
                self.define_word(target, a >= 4)
            }
            RegisterTarget::Dense(target) => {
                self.trap_zero_glue()?;
                self.define_glue(target, a >= 4)
            }
            RegisterTarget::Sparse(target) if quantity < GLUE_VAL => if a >= 4 {
                self.gsa_w_def(target, self.cur_val)
            } else {
                self.sa_w_def(target, self.cur_val)
            }
            .map_err(TeXMouthError::Memory),
            RegisterTarget::Sparse(target) => {
                self.trap_zero_glue()?;
                if a >= 4 {
                    self.gsa_def(target, self.cur_val)
                } else {
                    self.sa_def(target, self.cur_val)
                }
                .map_err(TeXMouthError::Memory)
            }
        }
    }

    /// Adds the current glue value `s` to the just-scanned glue spec in
    /// `cur_val`, preserving TeX's stretch/shrink order precedence.
    fn advance_glue(&mut self, s: i32) -> Result<(), TeXMouthError> {
        let scanned = self.cur_val;
        let result = self.new_spec(scanned).map_err(TeXMouthError::Memory)?;
        self.delete_glue_ref(scanned)
            .map_err(TeXMouthError::Memory)?;

        let q = self.checked_engine_mem_index(result)?;
        let r = self.checked_engine_mem_index(s)?;
        self.mem[q + 1].s1 = self.mem[q + 1].s1.wrapping_add(self.mem[r + 1].s1);

        let r_stretch_order = self.mem[r].b16_s1();
        if self.mem[q + 2].s1 == 0 {
            self.mem[q].set_b16_s1(crate::state::NORMAL);
        }
        if self.mem[q].b16_s1() == r_stretch_order {
            self.mem[q + 2].s1 = self.mem[q + 2].s1.wrapping_add(self.mem[r + 2].s1);
        } else if self.mem[q].b16_s1() < r_stretch_order && self.mem[r + 2].s1 != 0 {
            self.mem[q + 2].s1 = self.mem[r + 2].s1;
            self.mem[q].set_b16_s1(r_stretch_order);
        }

        let r_shrink_order = self.mem[r].b16_s0();
        if self.mem[q + 3].s1 == 0 {
            self.mem[q].set_b16_s0(crate::state::NORMAL);
        }
        if self.mem[q].b16_s0() == r_shrink_order {
            self.mem[q + 3].s1 = self.mem[q + 3].s1.wrapping_add(self.mem[r + 3].s1);
        } else if self.mem[q].b16_s0() < r_shrink_order && self.mem[r + 3].s1 != 0 {
            self.mem[q + 3].s1 = self.mem[r + 3].s1;
            self.mem[q].set_b16_s0(r_shrink_order);
        }

        self.cur_val = result;
        Ok(())
    }

    /// Executes `TOKS_REGISTER` and `ASSIGN_TOKS` assignments.
    fn prefixed_assign_toks(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let command_cs = self.cur_cs;
        let mut p = self.cur_chr;
        let mut sparse = false;

        if self.cur_cmd == CommandCode::ToksRegister as u8 {
            if p == 0 {
                self.scan_register_num()?;
                if self.cur_val > 255 {
                    p = self.find_sa_element(SA_TOK_VAL, self.cur_val, true)?;
                    sparse = true;
                } else {
                    p = (TOKS_BASE + self.cur_val as usize) as i32;
                }
            } else {
                sparse = true;
            }
        } else if p == (LOCAL_BASE + LOCAL_XETEX_INTER_CHAR_TOKS) as i32 {
            self.scan_char_class_not_ignored()?;
            let first_class = self.cur_val;
            self.scan_char_class_not_ignored()?;
            let key = first_class
                .checked_mul(crate::state::CHAR_CLASS_LIMIT)
                .and_then(|key| key.checked_add(self.cur_val))
                .ok_or(TeXMouthError::InputOutOfBounds(self.cur_val))?;
            p = self.find_sa_element(SA_INTER_CHAR_VAL, key, true)?;
            sparse = true;
        }

        let target = if sparse {
            TokenListTarget::Sparse(p)
        } else {
            TokenListTarget::Dense(
                usize::try_from(p).map_err(|_| TeXMouthError::InputOutOfBounds(p))?,
            )
        };

        self.scan_optional_equals()?;
        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 && self.cur_cmd != CommandCode::Relax as u8
            {
                break;
            }
        }

        if self.cur_cmd != CommandCode::LeftBrace as u8 {
            let mut copied_existing = false;
            let copied = match CommandCode::from_u8(self.cur_cmd) {
                Some(CommandCode::ToksRegister) => {
                    copied_existing = true;
                    if self.cur_chr == 0 {
                        self.scan_register_num()?;
                        if self.cur_val > 255 {
                            let element = self.find_sa_element(SA_TOK_VAL, self.cur_val, false)?;
                            if element == TEX_NULL {
                                TEX_NULL
                            } else {
                                self.sparse_ptr_value(element)?
                            }
                        } else {
                            self.toks(self.cur_val as usize)
                                .map_err(TeXMouthError::Memory)?
                        }
                    } else {
                        self.sparse_ptr_value(self.cur_chr)?
                    }
                }
                Some(CommandCode::AssignToks) => {
                    copied_existing = true;
                    if self.cur_chr == (LOCAL_BASE + LOCAL_XETEX_INTER_CHAR_TOKS) as i32 {
                        self.scan_char_class_not_ignored()?;
                        let first_class = self.cur_val;
                        self.scan_char_class_not_ignored()?;
                        let key = first_class
                            .checked_mul(crate::state::CHAR_CLASS_LIMIT)
                            .and_then(|key| key.checked_add(self.cur_val))
                            .ok_or(TeXMouthError::InputOutOfBounds(self.cur_val))?;
                        let element = self.find_sa_element(SA_INTER_CHAR_VAL, key, false)?;
                        if element == TEX_NULL {
                            TEX_NULL
                        } else {
                            self.sparse_ptr_value(element)?
                        }
                    } else {
                        let source = self.checked_eqtb_payload(self.cur_chr)?;
                        self.eqtb[source].s1
                    }
                }
                _ => TEX_NULL,
            };

            if copied_existing {
                if copied == TEX_NULL {
                    return self.assign_token_list_target(target, TEX_NULL, a >= 4);
                }
                let copied = self.token_list_reference(copied)?;
                let idx = self.checked_engine_mem_index(copied)?;
                self.mem[idx].s0 = self.mem[idx].s0.wrapping_add(1);
                return self.assign_token_list_target(target, copied, a >= 4);
            }
        }

        self.back_input()?;
        self.cur_cs = command_cs;
        let tail = self.scan_toks(false, false)?;
        let value = self.def_ref;
        let def_idx = self.checked_engine_mem_index(value)?;
        if self.mem[def_idx].s1 == TEX_NULL {
            self.mem[def_idx].s1 = self.avail;
            self.avail = value;
            self.assign_token_list_target(target, TEX_NULL, a >= 4)
        } else {
            if matches!(target, TokenListTarget::Dense(target) if target == LOCAL_BASE + LOCAL_OUTPUT_ROUTINE)
            {
                let q_idx = self.checked_engine_mem_index(tail)?;
                self.mem[q_idx].s1 = self.get_avail().map_err(TeXMouthError::Memory)?;
                let right_brace = self.mem[q_idx].s1;
                let right_idx = self.checked_engine_mem_index(right_brace)?;
                self.mem[right_idx].s0 = RIGHT_BRACE_TOKEN + i32::from(b'}');
                let left_brace = self.get_avail().map_err(TeXMouthError::Memory)?;
                let left_idx = self.checked_engine_mem_index(left_brace)?;
                self.mem[left_idx].s0 = LEFT_BRACE_TOKEN + i32::from(b'{');
                self.mem[left_idx].s1 = self.mem[def_idx].s1;
                self.mem[def_idx].s1 = left_brace;
            }
            self.assign_token_list_target(target, value, a >= 4)
        }
    }

    /// Appends a mark node to the current semantic list (`make_mark`).
    fn make_mark(&mut self) -> Result<(), TeXMouthError> {
        let mark_class = if self.cur_chr == 0 {
            0
        } else {
            self.scan_register_num()?;
            self.cur_val
        };
        self.scan_toks(false, true)?;
        let node = self.get_node(SMALL_NODE_SIZE)?;
        let node_idx = self.checked_engine_mem_index(node)?;
        self.mem[node_idx + 1].s0 = mark_class;
        self.set_node_type(node, MARK_NODE)?;
        self.mem[node_idx].set_b16_s0(0);
        self.mem[node_idx + 1].s1 = self.def_ref;
        let tail_idx = self.checked_engine_mem_index(self.tail)?;
        self.mem[tail_idx].s1 = node;
        self.tail = node;
        Ok(())
    }

    /// Emits an expanded general text as `\message` or `\errmessage`
    /// (`issue_message`).
    fn issue_message(&mut self) -> Result<(), TeXMouthError> {
        let error_message = self.cur_chr != 0;
        self.scan_toks(false, true)?;
        let def_ref = self.def_ref;
        let first = self.mem[self.checked_engine_mem_index(def_ref)?].s1;
        let text = self.token_list_to_string(first)?;
        self.flush_list(def_ref)?;

        if !error_message {
            self.transcript.push(text);
            return Ok(());
        }

        self.transcript.push(format!("! {text}"));
        let err_help = self.local(LOCAL_ERR_HELP)?;
        if err_help != TEX_NULL {
            let help = self.stored_token_list_start(err_help)?;
            let help = self.token_list_to_string(help)?;
            self.transcript.push(help);
        } else if self.long_help_seen {
            self.transcript
                .push("(That was another \\errmessage.)".to_string());
        } else {
            if self.interaction < ERROR_STOP_MODE {
                self.long_help_seen = true;
            }
            self.transcript.extend([
                "This error message was generated by an \\errmessage".to_string(),
                "command, so I can't give any explicit help.".to_string(),
                "Pretend that you're Hercule Poirot: Examine all clues,".to_string(),
                "and deduce the truth by order and method.".to_string(),
            ]);
        }
        self.error_count = self.error_count.saturating_add(1);
        Ok(())
    }

    /// Applies `\lowercase` or `\uppercase` to a scanned token list and pushes
    /// the transformed list back as input (`shift_case`).
    fn shift_case(&mut self) -> Result<(), TeXMouthError> {
        let table = match self.cur_chr {
            LC_CODE_BASE => CODE_TABLE_LC,
            UC_CODE_BASE => CODE_TABLE_UC,
            _ => return Err(TeXMouthError::UnsupportedPrimitive("case-shift table")),
        };
        self.scan_toks(false, false)?;
        let def_ref = self.def_ref;
        let mut p = self.mem[self.checked_engine_mem_index(def_ref)?].s1;
        while p != TEX_NULL {
            let p_idx = self.checked_engine_mem_index(p)?;
            let token = self.mem[p_idx].s0;
            if (0..CS_TOKEN_FLAG).contains(&token) {
                let character = token % MAX_CHAR_VAL;
                let mapped = u32::try_from(character).ok().map(|c| {
                    if table == CODE_TABLE_LC {
                        self.lc_code(c)
                    } else {
                        self.uc_code(c)
                    }
                });
                if let Some(mapped) = mapped.filter(|&mapped| mapped != 0) {
                    self.mem[p_idx].s0 = token - character + mapped;
                }
            }
            p = self.mem[p_idx].s1;
        }

        let list = self.mem[self.checked_engine_mem_index(def_ref)?].s1;
        self.begin_token_list(list, BACKED_UP)?;
        let def_idx = self.checked_engine_mem_index(def_ref)?;
        self.mem[def_idx].s1 = self.avail;
        self.avail = def_ref;
        Ok(())
    }

    /// Executes an `EXTENSION` command (`do_extension`).
    pub fn do_extension(&mut self) -> Result<(), TeXMouthError> {
        match self.cur_chr {
            x if x == i32::from(OPEN_NODE) => {
                let node = self.new_write_whatsit(3, OPEN_NODE)?;
                self.scan_optional_equals()?;
                self.scan_and_pack_name()?;
                let idx = self.checked_engine_mem_index(node)?;
                self.mem[idx + 1].s1 = self.intern_string(&self.cur_name.clone());
                self.mem[idx + 2].s0 = self.intern_string(&self.cur_area.clone());
                self.mem[idx + 2].s1 = self.intern_string(&self.cur_ext.clone());
                Ok(())
            }
            x if x == i32::from(WRITE_NODE) => {
                let saved_cs = self.cur_cs;
                let node = self.new_write_whatsit(SMALL_NODE_SIZE, WRITE_NODE)?;
                self.cur_cs = saved_cs;
                self.scan_toks(false, false)?;
                let idx = self.checked_engine_mem_index(node)?;
                self.mem[idx + 1].s1 = self.def_ref;
                Ok(())
            }
            x if x == i32::from(CLOSE_NODE) => {
                let node = self.new_write_whatsit(SMALL_NODE_SIZE, CLOSE_NODE)?;
                let idx = self.checked_engine_mem_index(node)?;
                self.mem[idx + 1].s1 = TEX_NULL;
                Ok(())
            }
            x if x == i32::from(SPECIAL_NODE) => {
                let node = self.new_whatsit(SPECIAL_NODE, SMALL_NODE_SIZE)?;
                self.scan_toks(false, true)?;
                let idx = self.checked_engine_mem_index(node)?;
                self.mem[idx + 1].s0 = TEX_NULL;
                self.mem[idx + 1].s1 = self.def_ref;
                Ok(())
            }
            IMMEDIATE_CODE => {
                self.get_x_token()?;
                if self.cur_cmd == CommandCode::Extension as u8
                    && self.cur_chr <= i32::from(CLOSE_NODE)
                {
                    let p = self.tail;
                    self.do_extension()?;
                    self.out_what(self.tail)?;
                    self.flush_node_list(self.tail)?;
                    self.tail = p;
                    let idx = self.checked_engine_mem_index(p)?;
                    self.mem[idx].s1 = TEX_NULL;
                } else {
                    self.back_input()?;
                }
                Ok(())
            }
            SET_LANGUAGE_CODE => {
                if self.mode.abs() != TeXMode::Hmode as i16 {
                    return Err(TeXMouthError::UnsupportedPrimitive(
                        "mode-incompatible setlanguage",
                    ));
                }
                let node = self.new_whatsit(LANGUAGE_NODE, SMALL_NODE_SIZE)?;
                self.scan_int()?;
                let language = if (1..=BIGGEST_LANG).contains(&self.cur_val) {
                    self.cur_val
                } else {
                    0
                };
                self.aux.s1 = language;
                let left_min = norm_min(self.int_par(INT_PAR_LEFT_HYPHEN_MIN)?) as u16;
                let right_min = norm_min(self.int_par(INT_PAR_RIGHT_HYPHEN_MIN)?) as u16;
                let idx = self.checked_engine_mem_index(node)?;
                self.mem[idx + 1].s1 = language;
                self.mem[idx + 1].set_b16_s1(left_min);
                self.mem[idx + 1].set_b16_s0(right_min);
                Ok(())
            }
            x if x == i32::from(PDF_SAVE_POS_NODE) => {
                self.new_whatsit(PDF_SAVE_POS_NODE, SMALL_NODE_SIZE)?;
                Ok(())
            }
            RESET_TIMER_CODE => {
                let elapsed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                self.epochseconds = elapsed.as_secs() as i64;
                self.microseconds = elapsed.subsec_micros() as i32;
                Ok(())
            }
            SET_RANDOM_SEED_CODE => {
                self.scan_int()?;
                self.random_seed = self.cur_val.abs();
                self.rand.init(self.random_seed);
                Ok(())
            }
            PIC_FILE_CODE => self.load_picture(false),
            PDF_FILE_CODE => self.load_picture(true),
            GLYPH_CODE => self.extension_glyph(),
            XETEX_INPUT_ENCODING_EXTENSION_CODE => self.extension_input_encoding(),
            XETEX_DEFAULT_ENCODING_EXTENSION_CODE => self.extension_default_encoding(),
            XETEX_LINEBREAK_LOCALE_EXTENSION_CODE => {
                self.scan_file_name()?;
                let value = if self.cur_name.is_empty() {
                    0
                } else {
                    self.intern_string(&self.cur_name.clone())
                };
                self.set_int_par(INT_PAR_XETEX_LINEBREAK_LOCALE, value)?;
                Ok(())
            }
            _ => Err(TeXMouthError::UnsupportedPrimitive("extension subtype")),
        }
    }

    /// Allocates and appends a whatsit node (`new_whatsit`).
    pub(crate) fn new_whatsit(&mut self, subtype: u16, size: i32) -> Result<i32, TeXMouthError> {
        let node = self.get_node(size)?;
        self.set_node_type(node, WHATSIT_NODE)?;
        self.set_node_subtype(node, subtype)?;
        let tail = self.checked_engine_mem_index(self.tail)?;
        self.mem[tail].s1 = node;
        self.tail = node;
        Ok(node)
    }

    /// Allocates a write-family whatsit and scans its normalized stream
    /// (`new_write_whatsit`).
    fn new_write_whatsit(&mut self, size: i32, subtype: u16) -> Result<i32, TeXMouthError> {
        let node = self.new_whatsit(subtype, size)?;
        if size != SMALL_NODE_SIZE {
            self.scan_four_bit_int()?;
        } else {
            self.scan_int()?;
            if self.cur_val < 0 {
                self.cur_val = 17;
            } else if self.cur_val > 15 && self.cur_val != 18 {
                self.cur_val = 16;
            }
        }
        let idx = self.checked_engine_mem_index(node)?;
        self.mem[idx + 1].s0 = self.cur_val;
        Ok(node)
    }

    /// Scans a file name and interns its three packed components
    /// (`scan_and_pack_name`).
    pub(crate) fn scan_and_pack_name(&mut self) -> Result<(), TeXMouthError> {
        self.scan_file_name()
    }

    /// Executes a write-family or special whatsit at shipout/immediate time
    /// (`out_what`).
    pub fn out_what(&mut self, p: i32) -> Result<(), TeXMouthError> {
        if self.node_type(p)? != WHATSIT_NODE {
            return Err(TeXMouthError::UnsupportedPrimitive("non-whatsit output"));
        }
        match self.node_subtype(p)? {
            OPEN_NODE => {
                let stream = self.word_at(p, 1)?.s0;
                if !(0..16).contains(&stream) {
                    return Err(TeXMouthError::InputOutOfBounds(stream));
                }
                let name = self.string_text(self.word_at(p, 1)?.s1).unwrap_or_default();
                let index = stream as usize;
                self.write_open[index] = true;
                self.write_streams[index] = Some(WriteStream {
                    name,
                    text: String::new(),
                });
            }
            WRITE_NODE => {
                self.write_out(p)?;
            }
            CLOSE_NODE => {
                let stream = self.word_at(p, 1)?.s0;
                if !(0..16).contains(&stream) {
                    return Err(TeXMouthError::InputOutOfBounds(stream));
                }
                self.write_open[stream as usize] = false;
            }
            SPECIAL_NODE => {
                self.special_out(p)?;
            }
            LANGUAGE_NODE => {}
            _ => {
                return Err(self.confusion("ext4"));
            }
        }
        Ok(())
    }

    /// Emits a language whatsit when `\language` differs from `clang`
    /// (`fix_language`).
    pub fn fix_language(&mut self) -> Result<(), TeXMouthError> {
        let language = self.int_par(INT_PAR_LANGUAGE)?;
        let language = if (1..=BIGGEST_LANG).contains(&language) {
            language
        } else {
            0
        };
        if language != self.aux.s1 {
            let node = self.new_whatsit(LANGUAGE_NODE, SMALL_NODE_SIZE)?;
            self.aux.s1 = language;
            let left_min = norm_min(self.int_par(INT_PAR_LEFT_HYPHEN_MIN)?) as u16;
            let right_min = norm_min(self.int_par(INT_PAR_RIGHT_HYPHEN_MIN)?) as u16;
            let idx = self.checked_engine_mem_index(node)?;
            self.mem[idx + 1].s1 = language;
            self.mem[idx + 1].set_b16_s1(left_min);
            self.mem[idx + 1].set_b16_s0(right_min);
        }
        Ok(())
    }

    /// Executes `\XeTeXglyph` (`GLYPH_CODE`).
    fn extension_glyph(&mut self) -> Result<(), TeXMouthError> {
        if self.mode.abs() == TeXMode::Vmode as i16 {
            self.back_input()?;
            return self.new_graf(true);
        }
        if self.mode.abs() == TeXMode::Mmode as i16 {
            return Err(TeXMouthError::UnsupportedPrimitive(
                "mode-incompatible XeTeXglyph",
            ));
        }
        let font = self.current_font();
        if !self.is_native_font(font) {
            return Err(TeXMouthError::UnsupportedPrimitive("non-native font glyph"));
        }
        let node = self.new_whatsit(GLYPH_NODE, 5)?;
        self.scan_int()?;
        if !(0..=65_535).contains(&self.cur_val) {
            self.cur_val = 0;
        }
        let idx = self.checked_engine_mem_index(node)?;
        self.mem[idx + 4].set_b16_s2(font as u16);
        self.mem[idx + 4].set_b16_s1(self.cur_val as u16);
        let metric = self.font_char_metric(font, self.cur_val.max(0) as u32);
        self.mem[idx + 1].s1 = metric.width;
        self.mem[idx + 2].s1 = metric.depth;
        self.mem[idx + 3].s1 = metric.height;
        Ok(())
    }

    /// Scans a picture/PDF file and its scaling options (`load_picture`).
    fn load_picture(&mut self, is_pdf: bool) -> Result<(), TeXMouthError> {
        if self.mode.abs() == TeXMode::Mmode as i16 {
            return Err(TeXMouthError::UnsupportedPrimitive(
                "mode-incompatible picture file",
            ));
        }
        self.scan_file_name()?;
        self.pack_name_of_file();
        let mut page = 0;
        let mut pdf_box_type = 0;
        if is_pdf {
            if self.scan_keyword("page")? {
                self.scan_int()?;
                page = self.cur_val;
            }
            // C `pdfbox_none` (6): stored in the node, but bounds are probed
            // with `pdfbox_crop`.
            pdf_box_type = if self.scan_keyword("crop")? {
                1
            } else if self.scan_keyword("media")? {
                2
            } else if self.scan_keyword("bleed")? {
                3
            } else if self.scan_keyword("trim")? {
                4
            } else if self.scan_keyword("art")? {
                5
            } else {
                6
            };
        }

        let probe_box = if pdf_box_type == 6 { 1 } else { pdf_box_type };
        let result = self.find_pic_file(probe_box, page);
        let (x, y, width, height) = result.as_ref().map(|r| r.1).unwrap_or_default();
        let mut corners = [
            (x, y),
            (x, y + height),
            (x + width, y + height),
            (x + width, y),
        ];
        let mut transform = (1.0, 0.0, 0.0, 1.0, 0.0, 0.0);
        let mut x_size_req = 0.0;
        let mut y_size_req = 0.0;

        loop {
            let scale = if self.scan_keyword("scaled")? {
                self.scan_int()?;
                Some((self.cur_val as f64 / 1000.0, self.cur_val as f64 / 1000.0))
            } else if self.scan_keyword("xscaled")? {
                self.scan_int()?;
                Some((self.cur_val as f64 / 1000.0, 1.0))
            } else if self.scan_keyword("yscaled")? {
                self.scan_int()?;
                Some((1.0, self.cur_val as f64 / 1000.0))
            } else {
                None
            };
            if let Some((sx, sy)) = scale {
                if x_size_req == 0.0 && y_size_req == 0.0 {
                    transform = concat_transform(transform, (sx, 0.0, 0.0, sy, 0.0, 0.0));
                    for corner in &mut corners {
                        *corner = transform_point(*corner, (sx, 0.0, 0.0, sy, 0.0, 0.0));
                    }
                }
                continue;
            }
            if self.scan_keyword("width")? {
                self.scan_dimen(false, false, false)?;
                if self.cur_val <= 0 {
                    self.report_error(
                        &format!(
                            "Improper image size ({}pt) will be ignored",
                            format_scaled_pt(self.cur_val)
                        ),
                        &[
                            "I can't scale images to zero or negative sizes,",
                            "so I'm ignoring this.",
                        ],
                    );
                } else {
                    x_size_req = self.cur_val as f64 / 65_536.0;
                }
                continue;
            }
            if self.scan_keyword("height")? {
                self.scan_dimen(false, false, false)?;
                if self.cur_val <= 0 {
                    self.report_error(
                        &format!(
                            "Improper image size ({}pt) will be ignored",
                            format_scaled_pt(self.cur_val)
                        ),
                        &[
                            "I can't scale images to zero or negative sizes,",
                            "so I'm ignoring this.",
                        ],
                    );
                } else {
                    y_size_req = self.cur_val as f64 / 65_536.0;
                }
                continue;
            }
            if self.scan_keyword("rotated")? {
                self.scan_decimal()?;
                let angle = self.cur_val as f64 / 65_536.0 * std::f64::consts::PI / 180.0;
                if x_size_req != 0.0 || y_size_req != 0.0 {
                    let (xmin, xmax, ymin, ymax) = bounds_of_corners(&corners);
                    let (sx, sy) =
                        requested_scale(x_size_req, y_size_req, xmax - xmin, ymax - ymin);
                    let scaling = (sx, 0.0, 0.0, sy, 0.0, 0.0);
                    transform = concat_transform(transform, scaling);
                    for corner in &mut corners {
                        *corner = transform_point(*corner, scaling);
                    }
                    x_size_req = 0.0;
                    y_size_req = 0.0;
                }
                let rotation = (
                    angle.cos(),
                    angle.sin(),
                    -angle.sin(),
                    angle.cos(),
                    0.0,
                    0.0,
                );
                transform = concat_transform(transform, rotation);
                for corner in &mut corners {
                    *corner = transform_point(*corner, rotation);
                }
                // C replaces the corners with the rotated image's axis-aligned
                // bounding box (`SET_POINT` ×4); the translation to the origin
                // lives in `t` only, so chained rotations re-inflate the AABB.
                let (xmin, xmax, ymin, ymax) = bounds_of_corners(&corners);
                corners = [(xmin, ymin), (xmin, ymax), (xmax, ymax), (xmax, ymin)];
                continue;
            }
            break;
        }

        if x_size_req != 0.0 || y_size_req != 0.0 {
            let (xmin, xmax, ymin, ymax) = bounds_of_corners(&corners);
            let (sx, sy) = requested_scale(x_size_req, y_size_req, xmax - xmin, ymax - ymin);
            let scaling = (sx, 0.0, 0.0, sy, 0.0, 0.0);
            transform = concat_transform(transform, scaling);
            for corner in &mut corners {
                *corner = transform_point(*corner, scaling);
            }
        }
        let (xmin, xmax, ymin, ymax) = bounds_of_corners(&corners);
        transform = concat_transform(
            transform,
            (
                1.0,
                0.0,
                0.0,
                1.0,
                -(xmin as i32 as f64) * 72.0 / 72.27,
                -(ymin as i32 as f64) * 72.0 / 72.27,
            ),
        );

        match result {
            Ok((pic_path, _bounds)) => {
                let path_bytes = pic_path.as_bytes();
                let size = 9 + (path_bytes.len() as i32 + 7) / 8;
                let node = self.new_whatsit(if is_pdf { PDF_NODE } else { PIC_NODE }, size)?;
                let idx = self.checked_engine_mem_index(node)?;
                self.mem[idx + 4].set_b16_s1(path_bytes.len() as u16);
                self.mem[idx + 4].set_b16_s0(page as u16);
                self.mem[idx + 8].set_b16_s1(pdf_box_type as u16);
                self.mem[idx + 1].s1 = d2fix(xmax - xmin);
                self.mem[idx + 2].s1 = 0;
                self.mem[idx + 3].s1 = d2fix(ymax - ymin);
                self.mem[idx + 5].s0 = d2fix(transform.0);
                self.mem[idx + 5].s1 = d2fix(transform.1);
                self.mem[idx + 6].s0 = d2fix(transform.2);
                self.mem[idx + 6].s1 = d2fix(transform.3);
                self.mem[idx + 7].s0 = d2fix(transform.4);
                self.mem[idx + 7].s1 = d2fix(transform.5);
                write_pic_path(&mut self.mem[idx..], path_bytes);
            }
            Err(_) => {
                // Tectonic's probe layer never yields C's -43
                // (file-not-found) code, so the help text is always the
                // unrecognized-format variant.
                self.report_error(
                    &format!(
                        "Unable to load picture or PDF file '{}'",
                        self.packed_file_name()
                    ),
                    &[
                        "The requested image couldn't be read because",
                        "it was not a recognized image format.",
                    ],
                );
            }
        }
        Ok(())
    }

    /// Opens `name_of_file` through the registered byte sources and probes its
    /// bounds (`find_pic_file` + `pdf_get_rect`/`get_image_size_in_inches`).
    ///
    /// `picture_bounds` entries act as an embedder override; otherwise the
    /// file's registered bytes are probed. `pdf_box_type != 0` selects the PDF
    /// path (bp→pt at 72.27/72 after page clamping); 0 selects the raster
    /// image path (pixels/DPI → inches → pt at 72.27). Returns the resolved
    /// path and `(x, y, w, h)` in TeX points, or `Err` on open/probe failure.
    fn find_pic_file(&mut self, pdf_box_type: i32, page: i32) -> Result<PicFile, i32> {
        let name = String::from_utf8_lossy(&self.name_of_file).into_owned();
        if let Some(&bounds) = self.picture_bounds.get(&name) {
            return Ok((name, bounds));
        }
        let bytes = self.input_byte_sources.get(&name).cloned().ok_or(1)?;
        if pdf_box_type != 0 {
            // `pdf_get_rect`: clamp the page into [1, total] (negative counts
            // back from the last page), probe the requested box, convert bp→pt.
            let total =
                tectonic_image_probe::probe_pdf_page_count_bytes(&bytes).map_err(|_| -1)? as i32;
            if total <= 0 {
                return Err(-1);
            }
            let mut page_num = page;
            if page_num > total {
                page_num = total;
            }
            if page_num < 0 {
                page_num += total + 1;
            }
            if page_num < 1 {
                page_num = 1;
            }
            let (x0, y0, x1, y1) = tectonic_image_probe::probe_pdf_box_bytes(
                &bytes,
                page_num as usize,
                tectonic_image_probe::PdfBoxType::from(pdf_box_type),
            )
            .map_err(|_| -1)?;
            const BP_TO_PT: f64 = 72.27 / 72.0;
            Ok((
                name,
                (
                    BP_TO_PT * x0,
                    BP_TO_PT * y0,
                    BP_TO_PT * (x1 - x0),
                    BP_TO_PT * (y1 - y0),
                ),
            ))
        } else {
            // `get_image_size_in_inches`: pixel dimensions ÷ DPI (72 when the
            // format carries no resolution), then inches → pt at 72.27.
            let info = tectonic_image_probe::probe_image_bytes(&bytes).map_err(|_| 1)?;
            let w_px = info.width as f64;
            let h_px = info.height as f64;
            let wd = if info.x_dpi > 0.0 {
                w_px / info.x_dpi
            } else {
                w_px / 72.0
            };
            let ht = if info.y_dpi > 0.0 {
                h_px / info.y_dpi
            } else {
                h_px / 72.0
            };
            Ok((name, (0.0, 0.0, wd * 72.27, ht * 72.27)))
        }
    }

    /// Applies `\XeTeXinputencoding` to the current in-memory input source.
    fn extension_input_encoding(&mut self) -> Result<(), TeXMouthError> {
        self.scan_and_pack_name()?;
        let name = self.packed_file_name();
        let (mode, encoding) = self.encoding_mode_and_info(&name);
        if mode == XETEX_INPUT_MODE_AUTO {
            self.transcript
                .push("! Encoding mode `auto' is not valid for \\XeTeXinputencoding".to_string());
            return Ok(());
        }
        if let Some(source) = self.file_sources.get_mut(self.in_open) {
            source.input_mode = mode;
            source.input_encoding = encoding;
        }
        // C `set_input_file_encoding(input_file[in_open], mode, encodingData)`.
        if let Some(mut f) = self.input_file.get_mut(self.in_open).and_then(Option::take) {
            self.set_input_file_encoding(&mut f, mode, encoding);
            self.input_file[self.in_open] = Some(f);
        }
        Ok(())
    }

    /// Applies `\XeTeXdefaultencoding`.
    fn extension_default_encoding(&mut self) -> Result<(), TeXMouthError> {
        self.scan_and_pack_name()?;
        let name = self.packed_file_name();
        let (mode, encoding) = self.encoding_mode_and_info(&name);
        self.set_int_par(INT_PAR_XETEX_DEFAULT_INPUT_MODE, mode)?;
        self.set_int_par(INT_PAR_XETEX_DEFAULT_INPUT_ENCODING, encoding)?;
        Ok(())
    }

    /// Maps a packed encoding name to C's `(mode, info)` result.
    fn encoding_mode_and_info(&mut self, name: &str) -> (i32, i32) {
        let mode = if name.eq_ignore_ascii_case("auto") {
            XETEX_INPUT_MODE_AUTO
        } else if name.eq_ignore_ascii_case("utf8") {
            XETEX_INPUT_MODE_UTF8
        } else if name.eq_ignore_ascii_case("utf16") || name.eq_ignore_ascii_case("utf16le") {
            XETEX_INPUT_MODE_UTF16LE
        } else if name.eq_ignore_ascii_case("utf16be") {
            XETEX_INPUT_MODE_UTF16BE
        } else if name.eq_ignore_ascii_case("bytes") {
            XETEX_INPUT_MODE_RAW
        } else {
            XETEX_INPUT_MODE_ICUMAPPING
        };
        let info = if mode == XETEX_INPUT_MODE_ICUMAPPING {
            self.intern_string(&name.chars().map(|c| c as i32).collect::<Vec<_>>())
        } else {
            0
        };
        (mode, info)
    }

    /// Executes `\hyphenation` or INITEX `\patterns` (`HYPH_DATA`).
    fn prefixed_hyph_data(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_chr == 1 {
            if !self.in_initex_mode {
                while self.cur_cmd != CommandCode::RightBrace as u8 {
                    self.get_token()?;
                }
                return Err(TeXMouthError::UnsupportedPrimitive(
                    "patterns can be loaded only by INITEX",
                ));
            }
            return self.new_patterns();
        }
        self.new_hyph_exceptions()
    }

    /// Emits a `\show*` diagnostic (`show_whatever`).
    fn show_whatever(&mut self) -> Result<(), TeXMouthError> {
        let show_ok = match self.cur_chr {
            SHOW_CODE => {
                self.get_token()?;
                let meaning = self.meaning_of_current()?;
                let line = if self.cur_cs != 0 {
                    format!("> {}={meaning}", self.control_sequence_text(self.cur_cs)?)
                } else {
                    format!("> {meaning}")
                };
                self.transcript.push(line);
                false
            }
            SHOW_BOX_CODE => {
                self.scan_register_num()?;
                let register = self.cur_val;
                let boxed = if register < 256 {
                    self.box_reg(register as usize)?
                } else {
                    let element = self.find_sa_element(SA_BOX_VAL, register, false)?;
                    if element == TEX_NULL {
                        TEX_NULL
                    } else {
                        self.sparse_ptr_value(element)?
                    }
                };
                self.transcript.push(format!("> \\box{register}="));
                if boxed == TEX_NULL {
                    self.transcript.push("void".to_string());
                } else {
                    self.show_box(boxed)?;
                }
                true
            }
            SHOW_THE_CODE | SHOW_TOKENS_CODE => {
                let tokens = self.the_token_vec()?;
                let text = if tokens.is_empty() {
                    String::new()
                } else {
                    let head = self.store_token_list(&tokens)?;
                    let text = self.token_list_to_string(head)?;
                    self.flush_list(head)?;
                    text
                };
                self.transcript.push(format!("> {text}"));
                false
            }
            SHOW_LISTS_CODE => {
                self.show_activities()?;
                true
            }
            SHOW_GROUPS_CODE => {
                self.show_save_groups()?;
                true
            }
            SHOW_IFS_CODE => {
                self.show_ifs()?;
                true
            }
            _ => return Err(TeXMouthError::UnsupportedPrimitive("show command")),
        };

        if show_ok {
            self.transcript.push("! OK".to_string());
        }
        if self.interaction < ERROR_STOP_MODE {
            self.error_count = self.error_count.saturating_sub(1);
        } else if self.int_par(INT_PAR_TRACING_ONLINE)? > 0 {
            self.transcript.extend([
                "This isn't an error message; I'm just \\showing something.".to_string(),
                "Type `I\\show...' to show more (e.g., \\show\\cs,".to_string(),
                "\\showthe\\count10, \\showbox255, \\showlists).".to_string(),
            ]);
        } else {
            self.transcript.extend([
                "This isn't an error message; I'm just \\showing something.".to_string(),
                "Type `I\\show...' to show more (e.g., \\show\\cs,".to_string(),
                "\\showthe\\count10, \\showbox255, \\showlists).".to_string(),
                "And type `I\\tracingonline=1\\show...' to show boxes and".to_string(),
                "lists on your terminal as well as in the transcript file.".to_string(),
            ]);
        }
        Ok(())
    }

    /// Formats the meaning of the current token or control sequence
    /// (`print_meaning`): `print_cmd_chr` followed by `:` and the replacement
    /// token list for macros and mark registers.
    pub(crate) fn meaning_of_current(&self) -> Result<String, TeXMouthError> {
        let mut text = self.cmd_chr_text(self.cur_cmd, self.cur_chr)?;
        if self.cur_cmd >= CommandCode::Call as u8 {
            let start = self.stored_token_list_start(self.cur_chr)?;
            let mut tokens = start;
            if start != TEX_NULL {
                let idx = self.checked_engine_mem_index(start)?;
                if self.mem[idx].s0 == PROTECTED_TOKEN {
                    tokens = self.mem[idx].s1;
                }
            }
            text.push(':');
            text.push_str(&self.token_list_to_string(tokens)?);
        } else if self.cur_cmd == CommandCode::TopBotMark as u8 && self.cur_chr < 5 {
            text.push(':');
            if let Some(&head) = self.cur_mark.get(self.cur_chr as usize) {
                let start = self.stored_token_list_start(head)?;
                text.push_str(&self.token_list_to_string(start)?);
            }
        }
        Ok(text)
    }

    /// Emits the semantic-list nest (`show_activities`).
    fn show_activities(&mut self) -> Result<(), TeXMouthError> {
        self.transcript.push(String::new());
        for p in (0..=self.nest_ptr).rev() {
            let (mode, head, prev_graf, mode_line, aux) = if p == self.nest_ptr {
                (
                    self.mode,
                    self.head,
                    self.prev_graf,
                    self.mode_line,
                    self.aux,
                )
            } else {
                let record = self.nest[p];
                (
                    record.mode,
                    record.head,
                    record.prev_graf,
                    record.mode_line,
                    record.aux,
                )
            };
            let mut line = format!(
                "### {} entered at line {}",
                mode_text(mode),
                mode_line.abs()
            );
            if mode == TeXMode::Hmode as i16 && prev_graf != 0x0083_0000 {
                line.push_str(&format!(
                    " (language{}:hyphenmin{},{})",
                    prev_graf % 65_536,
                    prev_graf / 0x40_0000,
                    (prev_graf / 65_536) % 64
                ));
            }
            if mode_line < 0 {
                line.push_str(" (\\output routine)");
            }
            self.transcript.push(line);

            if p == 0 {
                let page_head = self.page_head();
                if self.link(page_head)? != TEX_NULL {
                    self.transcript.push("### current page:".to_string());
                    if self.output_active {
                        self.transcript
                            .push(" (held over for next output)".to_string());
                    }
                    let first = self.link(page_head)?;
                    self.show_box(first)?;
                    if self.page_contents > PAGE_EMPTY {
                        self.transcript
                            .push(format!("total height {}", self.page_totals_text()));
                        self.transcript.push(format!(
                            " goal height {}",
                            scaled_number(self.page_so_far[0])
                        ));
                    }
                }
                let contrib_head = self.contrib_head();
                if self.link(contrib_head)? != TEX_NULL {
                    self.transcript
                        .push("### recent contributions:".to_string());
                }
            }

            let list = self.link(head)?;
            self.show_box(list)?;
            match mode.abs() / 103 {
                0 => {
                    let prevdepth = if aux.s1 <= IGNORE_DEPTH {
                        "ignored".to_string()
                    } else {
                        scaled_text(aux.s1, "pt")
                    };
                    let mut line = format!("prevdepth {prevdepth}");
                    if prev_graf != 0 {
                        line.push_str(&format!(
                            ", prevgraf {prev_graf} {}",
                            if prev_graf == 1 { "line" } else { "lines" }
                        ));
                    }
                    self.transcript.push(line);
                }
                1 => {
                    let mut line = format!("spacefactor {}", aux.s0);
                    if mode > 0 && aux.s1 > 0 {
                        line.push_str(&format!(", current language {}", aux.s1));
                    }
                    self.transcript.push(line);
                }
                2 if aux.s1 != TEX_NULL => {
                    self.transcript
                        .push("this will begin denominator of:".to_string());
                    self.show_box(aux.s1)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Emits the active group chain (`show_save_groups`).
    pub(crate) fn show_save_groups(&mut self) -> Result<(), TeXMouthError> {
        self.transcript.push(String::new());
        let mut nest_index = self.nest_ptr;
        let mut group = self.cur_group;
        let mut boundary = self.cur_boundary;
        let mut level = self.cur_level.saturating_sub(1);
        let mut align_state = 1;

        loop {
            let mut heading = group_text(group).to_string();
            if group != BOTTOM_LEVEL {
                heading.push_str(&format!(" group (level {level})"));
                let entered = self.save_line_before(boundary);
                if entered != 0 {
                    heading.push_str(&format!(" entered at line {entered}"));
                }
            }
            if group == BOTTOM_LEVEL {
                self.transcript.push(format!("### {heading}"));
                break;
            }

            let mut enclosing_mode;
            loop {
                enclosing_mode = self.nest_mode_at(nest_index);
                if nest_index > 0 {
                    nest_index -= 1;
                } else {
                    enclosing_mode = TeXMode::Vmode as i16;
                }
                if enclosing_mode != TeXMode::Hmode as i16 {
                    break;
                }
            }

            let detail = match group {
                SIMPLE_GROUP => {
                    nest_index += 1;
                    "{".to_string()
                }
                HBOX_GROUP | ADJUSTED_HBOX_GROUP | VBOX_GROUP | VTOP_GROUP | VCENTER_GROUP => {
                    self.packaging_group_detail(group, boundary)?
                }
                ALIGN_GROUP => self.align_group_detail(
                    boundary,
                    &mut nest_index,
                    &mut align_state,
                    enclosing_mode,
                )?,
                NO_ALIGN_GROUP => {
                    nest_index += 1;
                    align_state = -1;
                    "\\noalign{".to_string()
                }
                OUTPUT_GROUP => "\\output".to_string(),
                MATH_GROUP => "{".to_string(),
                DISC_GROUP | MATH_CHOICE_GROUP => {
                    let name = if group == DISC_GROUP {
                        "\\discretionary"
                    } else {
                        "\\mathchoice"
                    };
                    let count = self.raw_save_value(boundary - 2).unwrap_or(0).clamp(0, 3);
                    format!("{name}{}{}", "{}".repeat(count as usize), "{")
                }
                INSERT_GROUP => {
                    let class = self.raw_save_value(boundary - 3).unwrap_or(0);
                    if class == 255 {
                        "\\vadjust{".to_string()
                    } else {
                        format!("\\insert{class}{{")
                    }
                }
                SEMI_SIMPLE_GROUP => {
                    nest_index += 1;
                    "\\begingroup".to_string()
                }
                MATH_SHIFT_GROUP => "$".to_string(),
                MATH_LEFT_GROUP => "\\left".to_string(),
                _ => String::new(),
            };
            self.transcript.push(format!("### {heading} ({detail})"));

            let index = usize::try_from(boundary)
                .ok()
                .filter(|index| *index < self.save_stack.len());
            let Some(SaveStackEntry::Boundary {
                group: parent,
                boundary: previous,
            }) = index.map(|index| self.save_stack[index])
            else {
                break;
            };
            group = parent;
            boundary = previous;
            level = level.saturating_sub(1);
        }
        Ok(())
    }

    /// Returns the current-or-stacked semantic mode at a C-style nest index.
    fn nest_mode_at(&self, index: usize) -> i16 {
        if index == self.nest_ptr {
            self.mode
        } else {
            self.nest
                .get(index)
                .map(|record| record.mode)
                .unwrap_or(TeXMode::Vmode as i16)
        }
    }

    /// Returns the source-line slot immediately preceding a boundary.
    fn save_line_before(&self, boundary: i32) -> i32 {
        usize::try_from(boundary - 1)
            .ok()
            .and_then(|index| self.save_stack.get(index))
            .and_then(|entry| match entry {
                SaveStackEntry::Line(line) => Some(*line),
                _ => None,
            })
            .unwrap_or(0)
    }

    /// Returns a raw packaging/insert context slot before a boundary.
    fn raw_save_value(&self, index: i32) -> Option<i32> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.save_stack.get(index))
            .and_then(|entry| match entry {
                SaveStackEntry::RawValue(value) => Some(*value),
                _ => None,
            })
    }

    /// Formats the descriptive text inside a packaging group's parentheses.
    fn packaging_group_detail(&self, group: u8, boundary: i32) -> Result<String, TeXMouthError> {
        let name = match group {
            HBOX_GROUP | ADJUSTED_HBOX_GROUP => "hbox",
            VBOX_GROUP => "vbox",
            VTOP_GROUP => "vtop",
            _ => "vcenter",
        };
        let mut detail = String::new();
        if let Some(context) = self.raw_save_value(boundary - 4) {
            if context != 0 {
                if context < BOX_FLAG {
                    let horizontal = self.mode.abs() == TeXMode::Vmode as i16;
                    let command = match (horizontal, context > 0) {
                        (true, true) => "\\moveleft",
                        (true, false) => "\\moveright",
                        (false, true) => "\\raise",
                        (false, false) => "\\lower",
                    };
                    detail.push_str(&format!("{command}{}pt", scaled_number(context.abs())));
                } else if context < SHIP_OUT_FLAG {
                    let mut register = context;
                    if register >= GLOBAL_BOX_FLAG {
                        detail.push_str("\\global");
                        register -= GLOBAL_BOX_FLAG - BOX_FLAG;
                    }
                    detail.push_str(&format!("\\setbox{}=", register - BOX_FLAG));
                } else {
                    detail.push_str("\\shipout");
                }
            }
        }
        detail.push_str(&format!("\\{name}"));
        if let Some(width) = self.raw_save_value(boundary - 2) {
            if width != 0 {
                let spec = self.raw_save_value(boundary - 3).unwrap_or(ADDITIONAL);
                detail.push_str(&format!(
                    " {}{}pt",
                    if spec == EXACTLY { "to" } else { "spread" },
                    scaled_number(width)
                ));
            }
        }
        detail.push('{');
        Ok(detail)
    }

    /// Formats the descriptive text inside an alignment group's parentheses.
    fn align_group_detail(
        &self,
        boundary: i32,
        nest_index: &mut usize,
        align_state: &mut i32,
        enclosing_mode: i16,
    ) -> Result<String, TeXMouthError> {
        if *align_state == 0 {
            *align_state = 1;
            return Ok(if enclosing_mode == TeXMode::InternalVmode as i16 {
                "\\halign".to_string()
            } else {
                "\\valign".to_string()
            });
        }
        let detail = if *align_state == 1 {
            "align entry".to_string()
        } else {
            "\\cr".to_string()
        };
        if *nest_index >= (*align_state).max(0) as usize {
            *nest_index -= (*align_state).max(0) as usize;
        }
        *align_state = 0;
        let _ = boundary;
        Ok(detail)
    }

    /// Emits the active conditional stack (`showifs`).
    fn show_ifs(&mut self) -> Result<(), TeXMouthError> {
        self.transcript.push(String::new());
        if self.cond_ptr == TEX_NULL {
            self.transcript
                .push("### no active conditionals".to_string());
            return Ok(());
        }

        let mut p = self.cond_ptr;
        let mut count = 0;
        while p != TEX_NULL {
            count += 1;
            p = self.link(p)?;
        }

        p = self.cond_ptr;
        let mut conditional = self.cur_if;
        let mut line_number = self.if_line;
        let mut limit = self.if_limit;
        while p != TEX_NULL {
            let mut line = format!("### level {count}: {}", self.if_name(conditional)?);
            if limit == 2 {
                line.push_str(" \\else");
            }
            if line_number != 0 {
                line.push_str(&format!(" entered on line {line_number}"));
            }
            self.transcript.push(line);
            count -= 1;
            let idx = self.checked_engine_mem_index(p)?;
            conditional = self.mem[idx].b16_s0() as i16;
            line_number = self.mem[idx + 1].s1;
            limit = self.mem[idx].b16_s1() as u8;
            p = self.mem[idx].s1;
        }
        Ok(())
    }

    /// Formats a conditional command name for `showifs`.
    fn if_name(&self, conditional: i16) -> Result<String, TeXMouthError> {
        let unless = conditional >= 32;
        let code = if unless {
            conditional - 32
        } else {
            conditional
        };
        let name = primitive_name_for(CommandCode::IfTest, i32::from(code)).unwrap_or("if");
        Ok(if unless {
            format!("\\unless\\{name}")
        } else {
            format!("\\{name}")
        })
    }

    /// Emits a semantic node list rooted at `p` (`show_box`).
    fn show_box(&mut self, p: i32) -> Result<(), TeXMouthError> {
        let mut breadth = self.int_par(INT_PAR_SHOW_BOX_BREADTH)?;
        if breadth <= 0 {
            breadth = 5;
        }
        let depth = self.int_par(INT_PAR_SHOW_BOX_DEPTH)?;
        self.show_node_list(p, "", depth, breadth)
    }

    /// Recursively formats one semantic node list (`show_node_list`).
    fn show_node_list(
        &mut self,
        mut p: i32,
        indent: &str,
        depth_threshold: i32,
        breadth_max: i32,
    ) -> Result<(), TeXMouthError> {
        if indent.len() as i32 > depth_threshold {
            if p != TEX_NULL {
                self.transcript.push(format!("{indent} []"));
            }
            return Ok(());
        }

        let mut count = 0;
        while p != TEX_NULL {
            if count >= breadth_max {
                self.transcript.push(format!("{indent}etc."));
                return Ok(());
            }
            count += 1;
            let line = self.node_line(p, indent, depth_threshold)?;
            self.transcript.push(line);
            self.show_node_children(p, indent, depth_threshold, breadth_max)?;
            p = self.link(p)?;
        }
        Ok(())
    }

    /// Formats one node's diagnostic text without its children.
    fn node_line(
        &mut self,
        p: i32,
        indent: &str,
        depth_threshold: i32,
    ) -> Result<String, TeXMouthError> {
        if self.is_char_node(p) {
            return Ok(format!("{indent}{}", self.font_and_char_text(p)?));
        }

        let node_type = self.node_type(p)?;
        let subtype = self.node_subtype(p)?;
        let text = match node_type {
            HLIST_NODE | VLIST_NODE | UNSET_NODE => {
                let kind = match node_type {
                    HLIST_NODE => "h",
                    VLIST_NODE => "v",
                    _ => "unset",
                };
                let mut text = format!(
                    "\\{kind}box({}+{})x{}",
                    scaled_text(self.word_at(p, 3)?.s1, "pt"),
                    scaled_text(self.word_at(p, 2)?.s1, "pt"),
                    scaled_text(self.word_at(p, 1)?.s1, "pt")
                );
                if node_type == UNSET_NODE {
                    if subtype != 0 {
                        text.push_str(&format!(" ({} columns)", subtype + 1));
                    }
                    let stretch = self.word_at(p, 6)?.s1;
                    if stretch != 0 {
                        text.push_str(&format!(
                            ", stretch {}",
                            glue_amount_text(stretch, self.word_at(p, 5)?.b16_s0())
                        ));
                    }
                    let shrink = self.word_at(p, 4)?.s1;
                    if shrink != 0 {
                        text.push_str(&format!(
                            ", shrink {}",
                            glue_amount_text(shrink, self.word_at(p, 5)?.b16_s1())
                        ));
                    }
                } else {
                    let glue_set = self.word_at(p, 6)?.gr();
                    let sign = self.word_at(p, 5)?.b16_s1();
                    if glue_set != 0.0 && sign != NORMAL {
                        text.push_str(", glue set ");
                        if sign == 2 {
                            text.push_str("- ");
                        }
                        let amount = if glue_set.abs() > 20_000.0 {
                            if glue_set > 0.0 { ">" } else { "< -" }.to_string()
                                + &glue_amount_text(20_000 * 65_536, self.word_at(p, 5)?.b16_s0())
                        } else {
                            glue_amount_text(
                                (65_536.0 * glue_set).round() as i32,
                                self.word_at(p, 5)?.b16_s0(),
                            )
                        };
                        text.push_str(&amount);
                    }
                    let shifted = self.word_at(p, 4)?.s1;
                    if shifted != 0 {
                        text.push_str(&format!(", shifted {}", scaled_text(shifted, "pt")));
                    }
                    if node_type == HLIST_NODE && subtype == DLIST_CODE {
                        text.push_str(", display");
                    }
                }
                text
            }
            RULE_NODE => format!(
                "\\rule({}+{})x{}",
                rule_dimen_text(self.word_at(p, 3)?.s1),
                rule_dimen_text(self.word_at(p, 2)?.s1),
                rule_dimen_text(self.word_at(p, 1)?.s1)
            ),
            INS_NODE => format!(
                "\\insert{}, natural size {}; split({},{}); float cost {}",
                subtype,
                scaled_text(self.word_at(p, 3)?.s1, "pt"),
                self.glue_spec_text(self.word_at(p, 4)?.s1, "pt")?,
                scaled_text(self.word_at(p, 2)?.s1, "pt"),
                self.word_at(p, 1)?.s1
            ),
            WHATSIT_NODE => match subtype {
                OPEN_NODE => format!(
                    "\\openout{}={}",
                    stream_text(self.word_at(p, 1)?.s0),
                    self.file_name_text(
                        self.word_at(p, 1)?.s1,
                        self.word_at(p, 2)?.s0,
                        self.word_at(p, 2)?.s1
                    )
                ),
                WRITE_NODE => {
                    let mark = self.word_at(p, 1)?.s1;
                    format!(
                        "\\write{}{}",
                        stream_text(self.word_at(p, 1)?.s0),
                        self.mark_text(mark)?
                    )
                }
                CLOSE_NODE => format!("\\closeout{}", stream_text(self.word_at(p, 1)?.s0)),
                SPECIAL_NODE => {
                    let mark = self.word_at(p, 1)?.s1;
                    format!("\\special{}", self.mark_text(mark)?)
                }
                LANGUAGE_NODE => format!(
                    "\\setlanguage{} (hyphenmin {},{})",
                    self.word_at(p, 1)?.s1,
                    self.word_at(p, 1)?.b16_s1(),
                    self.word_at(p, 1)?.b16_s0()
                ),
                PDF_SAVE_POS_NODE => "\\pdfsavepos".to_string(),
                NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT => {
                    let font = self.native_node_font(p)?;
                    let cs = self.font_ident_cs(font);
                    let mut text = format!("\\{} ", self.cs_name_text(cs));
                    for unit in self.native_word_text(p)? {
                        text.push_str(&self.display_character(i32::from(unit)));
                    }
                    text
                }
                GLYPH_NODE => {
                    let font = self.native_node_font(p)?;
                    let cs = self.font_ident_cs(font);
                    format!(
                        "\\{} glyph#{}",
                        self.cs_name_text(cs),
                        self.word_at(p, 4)?.b16_s1()
                    )
                }
                PIC_NODE | PDF_NODE => {
                    let name = if subtype == PIC_NODE {
                        "XeTeXpicfile"
                    } else {
                        "XeTeXpdffile"
                    };
                    let path_len = i32::from(self.word_at(p, 4)?.b16_s1());
                    let mut text = format!("\\{name}( ");
                    for i in 0..path_len {
                        let word = self.word_at(p + PIC_NODE_SIZE + i / 8, 0)?;
                        let half = if i % 8 < 4 { word.s0 } else { word.s1 };
                        let byte = (half >> ((i % 4) * 8)) & 0xFF;
                        text.push(char::from_u32(byte as u32).unwrap_or('\u{FFFD}'));
                    }
                    text.push('"');
                    text
                }
                _ => "whatsit?".to_string(),
            },
            GLUE_NODE => {
                if i32::from(subtype) >= A_LEADERS {
                    let leader = match subtype {
                        C_LEADERS_CODE => "cleaders",
                        X_LEADERS_CODE => "xleaders",
                        _ => "leaders",
                    };
                    format!(
                        "\\{leader} {}",
                        self.glue_spec_text(self.word_at(p, 1)?.s0, "pt")?
                    )
                } else if subtype == COND_MATH_GLUE_CODE {
                    "\\glue(\\nonscript)".to_string()
                } else {
                    format!(
                        "\\glue {}",
                        self.glue_spec_text(
                            self.word_at(p, 1)?.s0,
                            if subtype == MU_GLUE { "mu" } else { "pt" }
                        )?
                    )
                }
            }
            KERN_NODE => {
                if subtype == MU_GLUE {
                    format!("\\mkern{}", scaled_text(self.word_at(p, 1)?.s1, "mu"))
                } else {
                    let mut text = format!("\\kern{}", scaled_text(self.word_at(p, 1)?.s1, "pt"));
                    if subtype == ACC_KERN_CODE {
                        text.push_str(" (for accent)");
                    } else if subtype == SPACE_ADJUSTMENT_CODE {
                        text.push_str(" (space adjustment)");
                    }
                    text
                }
            }
            MARGIN_KERN_NODE => format!(
                "\\kern{} ({} margin)",
                scaled_text(self.word_at(p, 1)?.s1, "pt"),
                if subtype == 0 { "left" } else { "right" }
            ),
            MATH_NODE => {
                if subtype > BEFORE as u16 {
                    let boundary = match subtype {
                        x if x > END_M_CODE as u16 => "R",
                        x if x > BEGIN_M_CODE as u16 => "L",
                        _ => "M",
                    };
                    format!(
                        "\\{}{}",
                        if subtype % 2 == 1 { "end" } else { "begin" },
                        boundary
                    )
                } else {
                    let mut text = format!(
                        "\\math{}",
                        if subtype == BEFORE as u16 {
                            "on"
                        } else {
                            "off"
                        }
                    );
                    let surrounded = self.word_at(p, 1)?.s1;
                    if surrounded != 0 {
                        text.push_str(&format!(", surrounded {}", scaled_text(surrounded, "pt")));
                    }
                    text
                }
            }
            PENALTY_NODE => format!("\\penalty {}", self.word_at(p, 1)?.s1),
            DISC_NODE => {
                let mut text = "\\discretionary".to_string();
                if subtype > 0 {
                    text.push_str(&format!(" replacing {subtype}"));
                }
                text
            }
            MARK_NODE => {
                let mark = self.word_at(p, 1)?.s1;
                let class = self.word_at(p, 1)?.s0;
                format!(
                    "\\mark{}{}",
                    if class == 0 {
                        String::new()
                    } else {
                        format!("s{class}")
                    },
                    self.mark_text(mark)?
                )
            }
            ADJUST_NODE => format!("\\vadjust{}", if subtype != 0 { " pre" } else { "" }),
            LIGATURE_NODE => {
                let mut text = format!("{} (ligature ", self.font_and_char_text(p + 1)?);
                if subtype > 1 {
                    text.push('|');
                }
                self.font_in_short_display = i32::from(self.word_at(p + 1, 0)?.b16_s1());
                text.push_str(&self.short_display_text(self.word_at(p + 1, 0)?.s1)?);
                if subtype % 2 == 1 {
                    text.push('|');
                }
                text.push(')');
                text
            }
            STYLE_NODE => style_text(i32::from(subtype)).to_string(),
            CHOICE_NODE => "\\mathchoice".to_string(),
            FRACTION_NOAD => {
                let thickness = self.word_at(p + 1, 0)?.s1;
                let mut text = format!(
                    "\\fraction, thickness {}",
                    if thickness == DEFAULT_CODE {
                        "= default".to_string()
                    } else {
                        scaled_text(thickness, "pt")
                    }
                );
                if self.delimiter_nonzero(p + 4)? {
                    text.push_str(&format!(", left-delimiter {}", self.delimiter_text(p + 4)?));
                }
                if self.delimiter_nonzero(p + 5)? {
                    text.push_str(&format!(
                        ", right-delimiter {}",
                        self.delimiter_text(p + 5)?
                    ));
                }
                if indent.len() as i32 >= depth_threshold {
                    text.push_str(self.subsidiary_overflow_suffix(p + 2)?);
                    text.push_str(self.subsidiary_overflow_suffix(p + 3)?);
                }
                text
            }
            ORD_NOAD | OP_NOAD | BIN_NOAD | REL_NOAD | OPEN_NOAD | CLOSE_NOAD | PUNCT_NOAD
            | INNER_NOAD | RADICAL_NOAD | OVER_NOAD | UNDER_NOAD | VCENTER_NOAD | ACCENT_NOAD
            | LEFT_NOAD | RIGHT_NOAD => {
                let mut text = match node_type {
                    ORD_NOAD => "\\mathord".to_string(),
                    OP_NOAD => "\\mathop".to_string(),
                    BIN_NOAD => "\\mathbin".to_string(),
                    REL_NOAD => "\\mathrel".to_string(),
                    OPEN_NOAD => "\\mathopen".to_string(),
                    CLOSE_NOAD => "\\mathclose".to_string(),
                    PUNCT_NOAD => "\\mathpunct".to_string(),
                    INNER_NOAD => "\\mathinner".to_string(),
                    OVER_NOAD => "\\overline".to_string(),
                    UNDER_NOAD => "\\underline".to_string(),
                    VCENTER_NOAD => "\\vcenter".to_string(),
                    RADICAL_NOAD => {
                        format!("\\radical{}", self.delimiter_text(p + 4)?)
                    }
                    ACCENT_NOAD => {
                        format!("\\accent{}", self.fam_and_char_text(p + 4)?)
                    }
                    LEFT_NOAD => format!("\\left{}", self.delimiter_text(p + 1)?),
                    _ => format!(
                        "\\{}{}",
                        if subtype == NORMAL { "right" } else { "middle" },
                        self.delimiter_text(p + 1)?
                    ),
                };
                if node_type < LEFT_NOAD {
                    if subtype != NORMAL {
                        text.push_str(if subtype == LIMITS {
                            "\\limits"
                        } else {
                            "\\nolimits"
                        });
                    }
                    if indent.len() as i32 >= depth_threshold {
                        text.push_str(self.subsidiary_overflow_suffix(p + 1)?);
                    }
                }
                if indent.len() as i32 >= depth_threshold {
                    text.push_str(self.subsidiary_overflow_suffix(p + 2)?);
                    text.push_str(self.subsidiary_overflow_suffix(p + 3)?);
                }
                text
            }
            _ => "Unknown node type!".to_string(),
        };
        Ok(format!("{indent}{text}"))
    }

    /// Emits child lists belonging to one semantic node.
    fn show_node_children(
        &mut self,
        p: i32,
        indent: &str,
        depth_threshold: i32,
        breadth_max: i32,
    ) -> Result<(), TeXMouthError> {
        if self.is_char_node(p) {
            return Ok(());
        }
        let child_indent = format!("{indent}.");
        match self.node_type(p)? {
            HLIST_NODE | VLIST_NODE | UNSET_NODE => self.show_node_list(
                self.word_at(p, 5)?.s1,
                &child_indent,
                depth_threshold,
                breadth_max,
            ),
            INS_NODE => self.show_node_list(
                self.word_at(p, 4)?.s0,
                &child_indent,
                depth_threshold,
                breadth_max,
            ),
            GLUE_NODE if i32::from(self.node_subtype(p)?) >= A_LEADERS => self.show_node_list(
                self.word_at(p, 1)?.s1,
                &child_indent,
                depth_threshold,
                breadth_max,
            ),
            DISC_NODE => {
                self.show_node_list(
                    self.word_at(p, 1)?.s0,
                    &child_indent,
                    depth_threshold,
                    breadth_max,
                )?;
                self.show_node_list(
                    self.word_at(p, 1)?.s1,
                    &format!("{indent}|"),
                    depth_threshold,
                    breadth_max,
                )
            }
            ADJUST_NODE => self.show_node_list(
                self.word_at(p, 1)?.s1,
                &child_indent,
                depth_threshold,
                breadth_max,
            ),
            CHOICE_NODE => {
                self.show_node_list(
                    self.word_at(p + 1, 0)?.s0,
                    &format!("{indent}D"),
                    depth_threshold,
                    breadth_max,
                )?;
                self.show_node_list(
                    self.word_at(p + 1, 0)?.s1,
                    &format!("{indent}T"),
                    depth_threshold,
                    breadth_max,
                )?;
                self.show_node_list(
                    self.word_at(p + 2, 0)?.s0,
                    &format!("{indent}S"),
                    depth_threshold,
                    breadth_max,
                )?;
                self.show_node_list(
                    self.word_at(p + 2, 0)?.s1,
                    &format!("{indent}s"),
                    depth_threshold,
                    breadth_max,
                )
            }
            ORD_NOAD | OP_NOAD | BIN_NOAD | REL_NOAD | OPEN_NOAD | CLOSE_NOAD | PUNCT_NOAD
            | INNER_NOAD | RADICAL_NOAD | OVER_NOAD | UNDER_NOAD | VCENTER_NOAD | ACCENT_NOAD
            | LEFT_NOAD | RIGHT_NOAD => {
                if indent.len() as i32 >= depth_threshold {
                    return Ok(());
                }
                let node_type = self.node_type(p)?;
                if node_type < LEFT_NOAD {
                    self.show_subsidiary_field(p + 1, '.', indent, depth_threshold, breadth_max)?;
                }
                self.show_subsidiary_field(p + 2, '^', indent, depth_threshold, breadth_max)?;
                self.show_subsidiary_field(p + 3, '_', indent, depth_threshold, breadth_max)
            }
            FRACTION_NOAD => {
                if indent.len() as i32 >= depth_threshold {
                    return Ok(());
                }
                self.show_subsidiary_field(p + 2, '\\', indent, depth_threshold, breadth_max)?;
                self.show_subsidiary_field(p + 3, '/', indent, depth_threshold, breadth_max)
            }
            _ => Ok(()),
        }
    }

    /// Displays one subsidiary math field at `q` (`print_subsidiary_data` +
    /// `show_info` for the non-overflow case).
    fn show_subsidiary_field(
        &mut self,
        q: i32,
        marker: char,
        indent: &str,
        depth_threshold: i32,
        breadth_max: i32,
    ) -> Result<(), TeXMouthError> {
        let field_indent = format!("{indent}{marker}");
        match self.word_at(q, 0)?.s1 {
            MATH_CHAR => self
                .transcript
                .push(format!("{field_indent}{}", self.fam_and_char_text(q)?)),
            SUB_BOX => {
                self.show_node_list(
                    self.word_at(q, 0)?.s0,
                    &field_indent,
                    depth_threshold,
                    breadth_max,
                )?;
            }
            SUB_MLIST => {
                let head = self.word_at(q, 0)?.s0;
                if head == TEX_NULL {
                    self.transcript.push(format!("{field_indent}{{}}"));
                } else {
                    self.show_node_list(head, &field_indent, depth_threshold, breadth_max)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Returns `" []"` when the subsidiary field at `q` is non-empty
    /// (`print_subsidiary_data`'s over-threshold branch).
    fn subsidiary_overflow_suffix(&self, q: i32) -> Result<&'static str, TeXMouthError> {
        Ok(if self.word_at(q, 0)?.s1 != MATH_FIELD_EMPTY {
            " []"
        } else {
            ""
        })
    }

    /// Formats `\fam<n> <char>` for a `MATH_CHAR`/`MATH_TEXT_CHAR` field
    /// (`print_fam_and_char`).
    fn fam_and_char_text(&self, q: i32) -> Result<String, TeXMouthError> {
        let word = self.word_at(q, 0)?;
        let family = i32::from(word.b16_s1() % 256) % 256;
        let c = i32::from(word.b16_s0()) + (i32::from(word.b16_s1()) / 256) * 65536;
        Ok(format!("\\fam{family} {}", self.display_character(c)))
    }

    /// Formats `\<font-cs> <char>` for a ligature char node (`print_font_and_char`).
    fn font_and_char_text(&mut self, q: i32) -> Result<String, TeXMouthError> {
        if q > self.mem_top() {
            return Ok("\\CLOBBERED.".to_string());
        }
        let word = self.word_at(q, 0)?;
        let font = i32::from(word.b16_s1());
        let name = if font > self.font_ptr() {
            "*".to_string()
        } else {
            let cs = self.font_ident_cs(font);
            format!("\\{}", self.cs_name_text(cs))
        };
        Ok(format!(
            "{name} {}",
            self.display_character(i32::from(word.b16_s0()))
        ))
    }

    /// Whether any component of the delimiter word at `q` is nonzero.
    fn delimiter_nonzero(&self, q: i32) -> Result<bool, TeXMouthError> {
        let word = self.word_at(q, 0)?;
        Ok(word.b16_s0() != 0 || word.b16_s1() != 0 || word.b16_s2() != 0 || word.b16_s3() != 0)
    }

    /// Packs and renders a delimiter word (`print_delimiter`): a signed
    /// integer when negative, hex otherwise.
    fn delimiter_text(&self, q: i32) -> Result<String, TeXMouthError> {
        let word = self.word_at(q, 0)?;
        let s0 = i32::from(word.b16_s0());
        let s1 = i32::from(word.b16_s1());
        let s2 = i32::from(word.b16_s2());
        let s3 = i32::from(word.b16_s3());
        let small_char = s2 + (s3 / 256) * 65536;
        let large_char = s0 + (s1 / 256) * 65536;
        let a = ((s3 % 256) * 256 + small_char)
            .wrapping_mul(4096)
            .wrapping_add((s1 % 256) * 256 + large_char);
        Ok(if a < 0 {
            a.to_string()
        } else {
            format!("{a:X}")
        })
    }

    /// Renders a node list inline for ligature diagnostics (`short_display`).
    fn short_display_text(&mut self, mut p: i32) -> Result<String, TeXMouthError> {
        let mut text = String::new();
        while p > 0 {
            if self.is_char_node(p) {
                if p <= self.mem_top() {
                    let word = self.word_at(p, 0)?;
                    let font = i32::from(word.b16_s1());
                    if font != self.font_in_short_display {
                        if font > self.font_ptr() {
                            text.push('*');
                        } else {
                            let cs = self.font_ident_cs(font);
                            text.push('\\');
                            text.push_str(&self.cs_name_text(cs));
                        }
                        text.push(' ');
                        self.font_in_short_display = font;
                    }
                    text.push_str(&self.display_character(i32::from(word.b16_s0())));
                }
            } else {
                match self.node_type(p)? {
                    HLIST_NODE | VLIST_NODE | INS_NODE | MARK_NODE | ADJUST_NODE | UNSET_NODE => {
                        text.push_str("[]")
                    }
                    WHATSIT_NODE => {
                        if self.is_native_word_node(p)? {
                            let font = self.native_node_font(p)?;
                            if font != self.font_in_short_display {
                                let cs = self.font_ident_cs(font);
                                text.push('\\');
                                text.push_str(&self.cs_name_text(cs));
                                text.push(' ');
                                self.font_in_short_display = font;
                            }
                            for unit in self.native_word_text(p)? {
                                text.push_str(&self.display_character(i32::from(unit)));
                            }
                        } else {
                            text.push_str("[]");
                        }
                    }
                    RULE_NODE => text.push('|'),
                    GLUE_NODE => {
                        if self.word_at(p + 1, 0)?.s0 != 0 {
                            text.push(' ');
                        }
                    }
                    MATH_NODE => {
                        if i32::from(self.node_subtype(p)?) >= L_CODE {
                            text.push_str("[]");
                        } else {
                            text.push('$');
                        }
                    }
                    LIGATURE_NODE => {
                        text.push_str(&self.short_display_text(self.word_at(p + 1, 0)?.s1)?);
                    }
                    DISC_NODE => {
                        text.push_str(&self.short_display_text(self.word_at(p + 1, 0)?.s0)?);
                        text.push_str(&self.short_display_text(self.word_at(p + 1, 0)?.s1)?);
                        let mut n = i32::from(self.node_subtype(p)?);
                        while n > 0 {
                            if self.word_at(p, 0)?.s1 != TEX_NULL {
                                p = self.link(p)?;
                            }
                            n -= 1;
                        }
                    }
                    _ => {}
                }
            }
            p = self.link(p)?;
        }
        Ok(text)
    }

    /// Renders a file-name triple to a string, mirroring `print_file_name`'s
    /// quoting of names containing spaces or quotes.
    fn file_name_text(&self, n: i32, a: i32, e: i32) -> String {
        let part = |s: i32| -> String {
            if s == 0 {
                return String::new();
            }
            self.string_text(s).unwrap_or_default()
        };
        let (name, area, ext) = (part(n), part(a), part(e));
        let joined = format!("{area}{name}{ext}");
        let mut quote = 0i32;
        let mut must_quote = false;
        for ch in joined.chars() {
            if ch == ' ' {
                must_quote = true;
            } else if ch == '"' || ch == '\'' {
                must_quote = true;
                if quote == 0 {
                    quote = 73 - ch as i32;
                }
            }
        }
        if must_quote {
            if quote == 0 {
                quote = i32::from(b'"');
            }
            let q = char::from_u32(quote as u32).unwrap_or('"');
            format!("{q}{joined}{q}")
        } else {
            joined
        }
    }

    /// Renders the current page's total plus stretch/shrink components
    /// (`print_totals`).
    fn page_totals_text(&self) -> String {
        let mut text = scaled_number(self.page_so_far[1]);
        if self.page_so_far[2] != 0 {
            text.push_str(&format!(" plus {}", scaled_number(self.page_so_far[2])));
        }
        if self.page_so_far[3] != 0 {
            text.push_str(&format!(" plus {}fil", scaled_number(self.page_so_far[3])));
        }
        if self.page_so_far[4] != 0 {
            text.push_str(&format!(" plus {}fill", scaled_number(self.page_so_far[4])));
        }
        if self.page_so_far[5] != 0 {
            text.push_str(&format!(
                " plus {}filll",
                scaled_number(self.page_so_far[5])
            ));
        }
        if self.page_so_far[6] != 0 {
            text.push_str(&format!(" minus {}", scaled_number(self.page_so_far[6])));
        }
        text
    }

    /// Formats a mark or write token-list reference.
    fn mark_text(&self, reference: i32) -> Result<String, TeXMouthError> {
        if reference == TEX_NULL {
            return Ok(String::new());
        }
        if reference < self.hi_mem_min || reference > self.mem_top() {
            return Ok("{\\CLOBBERED.}".to_string());
        }
        let start = self.stored_token_list_start(reference)?;
        Ok(format!("{{{}}}", self.token_list_to_string(start)?))
    }

    /// Formats a glue specification (`print_spec`).
    fn glue_spec_text(&self, spec: i32, unit: &'static str) -> Result<String, TeXMouthError> {
        if spec == TEX_NULL {
            return Ok(scaled_text(0, unit));
        }
        let word = self.word_at(spec, 0)?;
        let mut text = scaled_text(self.word_at(spec, 1)?.s1, unit);
        let stretch = self.word_at(spec, 2)?.s1;
        let stretch_order = word.b16_s1();
        if stretch != 0 || stretch_order != NORMAL {
            text.push_str(&format!(
                " plus {}{}",
                scaled_number(stretch),
                glue_order_unit(stretch_order, unit)
            ));
        }
        let shrink = self.word_at(spec, 3)?.s1;
        let shrink_order = word.b16_s0();
        if shrink != 0 || shrink_order != NORMAL {
            text.push_str(&format!(
                " minus {}{}",
                scaled_number(shrink),
                glue_order_unit(shrink_order, unit)
            ));
        }
        Ok(text)
    }

    /// Formats a control sequence with the current escape character.
    fn control_sequence_text(&self, cs: i32) -> Result<String, TeXMouthError> {
        if self.cs_table.is_active(cs) {
            return Ok(self
                .cs_table
                .name(cs)
                .first()
                .and_then(|&c| char::from_u32(c.max(0) as u32))
                .map(|c| c.to_string())
                .unwrap_or_default());
        }
        let mut text = String::new();
        let escape = self.int_par(INT_PAR_ESCAPE_CHAR)?;
        if (0..MAX_CHAR_VAL).contains(&escape) {
            if let Some(c) = char::from_u32(escape as u32) {
                text.push(c);
            }
        }
        for &unit in self.cs_table.name(cs) {
            if let Some(c) = char::from_u32(unit.max(0) as u32) {
                text.push(c);
            }
        }
        Ok(text)
    }

    /// Formats a token character for `\show` meanings.
    pub(crate) fn display_character(&self, character: i32) -> String {
        char::from_u32(character.max(0) as u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("\"{character:X}"))
    }

    /// Renders a control sequence's name characters for diagnostics.
    pub(crate) fn cs_name_text(&self, cs: i32) -> String {
        self.cs_table
            .name(cs)
            .iter()
            .filter_map(|&unit| char::from_u32(unit.max(0) as u32))
            .collect()
    }

    /// Assigns a token-list value to a dense `eqtb` target or a sparse `sa`
    /// element.
    fn assign_token_list_target(
        &mut self,
        target: TokenListTarget,
        value: i32,
        global: bool,
    ) -> Result<(), TeXMouthError> {
        match target {
            TokenListTarget::Dense(target) => {
                if value == TEX_NULL {
                    self.define_eqtb(target, CommandCode::UndefinedCs as u16, TEX_NULL, global)
                } else {
                    self.define_eqtb(target, CommandCode::Call as u16, value, global)
                }
            }
            TokenListTarget::Sparse(target) => if global {
                self.gsa_def(target, value)
            } else {
                self.sa_def(target, value)
            }
            .map_err(TeXMouthError::Memory),
        }
    }

    /// Resets paragraph-shape state before a vertical box (`normal_paragraph`).
    fn normal_paragraph(&mut self) -> Result<(), TeXMouthError> {
        if self.int_par(INT_PAR_LOOSENESS)? != 0 {
            self.eq_word_define(INT_PAR_BASE + INT_PAR_LOOSENESS, 0)?;
        }
        if self.dimen_par(DIMEN_PAR_HANG_INDENT)? != 0 {
            self.eq_word_define(DIMEN_PAR_BASE + DIMEN_PAR_HANG_INDENT, 0)?;
        }
        if self.int_par(INT_PAR_HANG_AFTER)? != 1 {
            self.eq_word_define(INT_PAR_BASE + INT_PAR_HANG_AFTER, 1)?;
        }
        if self.local(LOCAL_PAR_SHAPE)? != TEX_NULL {
            self.eq_define(
                LOCAL_BASE + LOCAL_PAR_SHAPE,
                CommandCode::ShapeRef as u16,
                TEX_NULL,
            )?;
        }
        if self.eqtb[ETEX_PEN_BASE + ETEX_PEN_INTER_LINE].s1 != TEX_NULL {
            self.eq_define(
                ETEX_PEN_BASE + ETEX_PEN_INTER_LINE,
                CommandCode::ShapeRef as u16,
                TEX_NULL,
            )?;
        }
        Ok(())
    }

    /// Scans a rule specification (`scan_rule_spec`).
    fn scan_rule_spec(&mut self) -> Result<i32, TeXMouthError> {
        let q = self.new_rule()?;
        if self.cur_cmd == CommandCode::Vrule as u8 {
            self.mem[q as usize + 1].s1 = DEFAULT_RULE;
        } else {
            self.mem[q as usize + 3].s1 = DEFAULT_RULE;
            self.mem[q as usize + 2].s1 = 0;
        }
        loop {
            if self.scan_keyword("width")? {
                self.scan_dimen(false, false, false)?;
                self.mem[q as usize + 1].s1 = self.cur_val;
            } else if self.scan_keyword("height")? {
                self.scan_dimen(false, false, false)?;
                self.mem[q as usize + 3].s1 = self.cur_val;
            } else if self.scan_keyword("depth")? {
                self.scan_dimen(false, false, false)?;
                self.mem[q as usize + 2].s1 = self.cur_val;
            } else {
                return Ok(q);
            }
        }
    }

    /// Scans one box context token and invokes `begin_box` (`scan_box`).
    fn scan_box(&mut self, box_context: i32) -> Result<(), TeXMouthError> {
        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 && self.cur_cmd != CommandCode::Relax as u8
            {
                break;
            }
        }

        if self.cur_cmd == CommandCode::MakeBox as u8 {
            self.begin_box(box_context)
        } else if box_context >= LEADER_FLAG
            && matches!(
                CommandCode::from_u8(self.cur_cmd),
                Some(CommandCode::Hrule | CommandCode::Vrule)
            )
        {
            self.cur_box = self.scan_rule_spec()?;
            self.box_end(box_context)
        } else {
            self.back_input()?;
            Err(TeXMouthError::UnsupportedPrimitive("<box> expected"))
        }
    }

    /// Starts or consumes a box according to `cur_chr` (`begin_box`).
    fn begin_box(&mut self, box_context: i32) -> Result<(), TeXMouthError> {
        match self.cur_chr {
            BOX_CODE => {
                self.scan_register_num()?;
                self.cur_box = if self.cur_val < 256 {
                    self.box_reg(self.cur_val as usize)?
                } else {
                    let element = self.find_sa_element(SA_BOX_VAL, self.cur_val, false)?;
                    if element == TEX_NULL {
                        TEX_NULL
                    } else {
                        self.sparse_ptr_value(element)?
                    }
                };
                if self.cur_val < 256 {
                    self.set_box_reg(self.cur_val as usize, TEX_NULL)?;
                } else {
                    let element = self.find_sa_element(SA_BOX_VAL, self.cur_val, false)?;
                    if element != TEX_NULL {
                        let idx = self.checked_engine_mem_index(element)?;
                        self.mem[idx + 1].s1 = TEX_NULL;
                        self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
                        self.delete_sa_ref(element)?;
                    }
                }
            }
            COPY_CODE => {
                self.scan_register_num()?;
                let q = if self.cur_val < 256 {
                    self.box_reg(self.cur_val as usize)?
                } else {
                    let element = self.find_sa_element(SA_BOX_VAL, self.cur_val, false)?;
                    if element == TEX_NULL {
                        TEX_NULL
                    } else {
                        self.sparse_ptr_value(element)?
                    }
                };
                self.cur_box = self.copy_node_list(q)?;
            }
            LAST_BOX_CODE => {
                self.cur_box = TEX_NULL;
                if self.mode.abs() == TeXMode::Mmode as i16 {
                    return Err(TeXMouthError::UnsupportedPrimitive(
                        "\\lastbox in math mode",
                    ));
                }
                if self.mode == TeXMode::Vmode as i16 && self.head == self.tail {
                    return self.box_end(box_context);
                }
                let mut tx = self.tail;
                if tx < self.hi_mem_min
                    && self.node_type(tx)? == MATH_NODE
                    && i32::from(self.node_subtype(tx)?) == END_M_CODE
                {
                    let mut r = self.head;
                    while self.link(r)? != tx {
                        r = self.link(r)?;
                    }
                    tx = r;
                }
                if tx < self.hi_mem_min && matches!(self.node_type(tx)?, HLIST_NODE | VLIST_NODE) {
                    let mut q = self.head;
                    let mut p = TEX_NULL;
                    let mut r;
                    let mut found_math;
                    loop {
                        r = p;
                        p = q;
                        found_math = false;
                        if q < self.hi_mem_min {
                            match self.node_type(q)? {
                                DISC_NODE => {
                                    for _ in 1..=i32::from(self.node_subtype(q)?) {
                                        p = self.link(p)?;
                                    }
                                    if p == tx {
                                        return self.box_end(box_context);
                                    }
                                }
                                MATH_NODE if i32::from(self.node_subtype(q)?) == BEGIN_M_CODE => {
                                    found_math = true;
                                }
                                _ => {}
                            }
                        }
                        q = self.link(p)?;
                        if q == tx {
                            break;
                        }
                    }
                    q = self.link(tx)?;
                    self.mem[p as usize].s1 = q;
                    self.mem[tx as usize].s1 = TEX_NULL;
                    if q == TEX_NULL {
                        if found_math {
                            return Err(TeXMouthError::UnsupportedPrimitive("lastbox tail"));
                        }
                        self.tail = p;
                    } else if found_math {
                        self.tail = r;
                        self.mem[r as usize].s1 = TEX_NULL;
                        self.flush_node_list(p)?;
                    }
                    self.cur_box = tx;
                    self.mem[tx as usize + 4].s1 = 0;
                }
            }
            VSPLIT_CODE => {
                self.scan_register_num()?;
                let n = self.cur_val;
                if !self.scan_keyword("to")? {
                    // C reports "Missing `to' inserted" and continues at the
                    // dimension scanner.
                }
                self.scan_dimen(false, false, false)?;
                self.cur_box = self.vsplit(n, self.cur_val)?;
            }
            _ => {
                let mut k = self.cur_chr - 4;
                if self.save_stack.len() <= self.save_ptr {
                    self.save_stack.push(SaveStackEntry::RawValue(box_context));
                } else {
                    self.save_stack[self.save_ptr] = SaveStackEntry::RawValue(box_context);
                }
                if k == i32::from(TeXMode::Hmode as i16) {
                    if box_context < BOX_FLAG && self.mode.abs() == TeXMode::Vmode as i16 {
                        self.scan_spec(ADJUSTED_HBOX_GROUP, true)?;
                    } else {
                        self.scan_spec(HBOX_GROUP, true)?;
                    }
                } else {
                    if k == i32::from(TeXMode::Vmode as i16) {
                        self.scan_spec(VBOX_GROUP, true)?;
                    } else {
                        self.scan_spec(VTOP_GROUP, true)?;
                        k = i32::from(TeXMode::Vmode as i16);
                    }
                    self.normal_paragraph()?;
                }
                self.push_nest()?;
                self.mode = -(k as i16);
                if k == i32::from(TeXMode::Vmode as i16) {
                    self.aux.s1 = IGNORE_DEPTH;
                    let every = self.local(LOCAL_EVERY_VBOX)?;
                    if every != TEX_NULL {
                        self.begin_token_list(every, EVERY_VBOX_TEXT)?;
                    }
                } else {
                    self.aux.s0 = 1000;
                    let every = self.local(LOCAL_EVERY_HBOX)?;
                    if every != TEX_NULL {
                        self.begin_token_list(every, EVERY_HBOX_TEXT)?;
                    }
                }
                return Ok(());
            }
        }
        self.box_end(box_context)
    }

    /// Moves a box's contents into the current list (`unpackage`).
    fn unpackage(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_chr > COPY_CODE {
            self.mem[self.tail as usize].s1 = self.disc_ptr[self.cur_chr as usize];
            self.disc_ptr[self.cur_chr as usize] = TEX_NULL;
        } else {
            let copy = self.cur_chr;
            self.scan_register_num()?;
            let register = self.cur_val;
            let p = if register < 256 {
                self.box_reg(register as usize)?
            } else {
                let element = self.find_sa_element(SA_BOX_VAL, register, false)?;
                if element == TEX_NULL {
                    TEX_NULL
                } else {
                    self.sparse_ptr_value(element)?
                }
            };
            if p == TEX_NULL {
                return Ok(());
            }
            let node_type = self.node_type(p)?;
            let incompatible = self.mode.abs() == TeXMode::Mmode as i16
                || (self.mode.abs() == TeXMode::Vmode as i16 && node_type != VLIST_NODE)
                || (self.mode.abs() == TeXMode::Hmode as i16 && node_type != HLIST_NODE);
            if incompatible {
                return Err(TeXMouthError::UnsupportedPrimitive("incompatible unbox"));
            }
            if copy == COPY_CODE {
                self.mem[self.tail as usize].s1 =
                    self.copy_node_list(self.mem[p as usize + 5].s1)?;
            } else {
                self.mem[self.tail as usize].s1 = self.mem[p as usize + 5].s1;
                if register < 256 {
                    self.set_box_reg(register as usize, TEX_NULL)?;
                } else {
                    let element = self.find_sa_element(SA_BOX_VAL, register, false)?;
                    if element != TEX_NULL {
                        let idx = self.checked_engine_mem_index(element)?;
                        self.mem[idx + 1].s1 = TEX_NULL;
                        self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
                        self.delete_sa_ref(element)?;
                    }
                }
                self.free_node(p, BOX_NODE_SIZE)?;
            }
        }

        while self.link(self.tail)? != TEX_NULL {
            let r = self.link(self.tail)?;
            if !self.is_char_node(r) && self.node_type(r)? == MARGIN_KERN_NODE {
                self.mem[self.tail as usize].s1 = self.link(r)?;
                self.free_node(r, MEDIUM_NODE_SIZE)?;
            }
            self.tail = self.link(self.tail)?;
        }
        Ok(())
    }

    /// Appends the glue selected by the current `\hskip`/`\vskip`/`\mskip`
    /// token (`append_glue`).
    fn append_glue(&mut self) -> Result<(), TeXMouthError> {
        let s = self.cur_chr;
        match s {
            0..=3 => {
                let spec = self.get_node(4)?;
                let spec_idx = spec as usize;
                self.mem[spec_idx].s1 = TEX_NULL;
                self.mem[spec_idx + 1].s1 = 0;
                self.mem[spec_idx + 2].s1 = 65_536;
                self.mem[spec_idx + 3].s1 = if s == 2 { 65_536 } else { 0 };
                self.mem[spec_idx].set_b16_s1(match s {
                    0 | 2 | 3 => FIL,
                    _ => FILL,
                });
                self.mem[spec_idx].set_b16_s0(if s == 2 { FIL } else { NORMAL });
                if s == 3 {
                    self.mem[spec_idx + 2].s1 = -65_536;
                }
                self.cur_val = spec;
            }
            SKIP_CODE => {
                self.scan_glue(GLUE_VAL)?;
            }
            MSKIP_CODE => {
                self.scan_glue(MU_VAL)?;
            }
            _ => return Err(TeXMouthError::UnsupportedPrimitive("implicit glue")),
        };
        let p = self.new_glue(self.cur_val)?;
        self.mem[self.tail as usize].s1 = p;
        self.tail = p;
        if s >= SKIP_CODE {
            self.mem[self.cur_val as usize].s1 -= 1;
            if s > SKIP_CODE {
                self.set_node_subtype(self.tail, MU_GLUE)?;
            }
        }
        Ok(())
    }

    /// Appends a kern or math kern (`append_kern`).
    fn append_kern(&mut self) -> Result<(), TeXMouthError> {
        let s = self.cur_chr;
        if s == i32::from(MU_GLUE) {
            self.scan_glue(MU_VAL)?;
            self.cur_val = self.mem[self.cur_val as usize].s1;
        } else {
            self.scan_dimen(false, false, false)?;
        }
        let p = self.new_kern(self.cur_val)?;
        if s == i32::from(MU_GLUE) {
            self.set_node_subtype(p, MU_GLUE)?;
        }
        self.mem[self.tail as usize].s1 = p;
        self.tail = p;
        Ok(())
    }

    /// Scans and appends a penalty node (`append_penalty`).
    fn append_penalty(&mut self) -> Result<(), TeXMouthError> {
        self.scan_int()?;
        let p = self.new_penalty(self.cur_val)?;
        self.mem[self.tail as usize].s1 = p;
        self.tail = p;
        if self.mode.abs() == TeXMode::Vmode as i16 {
            self.aux.s1 = IGNORE_DEPTH;
        } else if self.mode.abs() == TeXMode::Hmode as i16 {
            self.aux.s0 = 1000;
        }
        if self.mode == TeXMode::Vmode as i16 {
            self.build_page()?;
        }
        Ok(())
    }

    /// Removes the last glue, kern, or penalty node (`delete_last`).
    fn delete_last(&mut self) -> Result<(), TeXMouthError> {
        if self.mode == TeXMode::Vmode as i16 && self.tail == self.head {
            if self.cur_chr != GLUE_NODE as i32 {
                return Err(TeXMouthError::UnsupportedPrimitive(
                    "remove item on the outer vertical list",
                ));
            }
            return Ok(());
        }

        let mut tx = self.tail;
        if !self.is_char_node(tx)
            && self.node_type(tx)? == MATH_NODE
            && self.node_subtype(tx)? == END_M_CODE as u16
        {
            let mut r = self.head;
            let mut q;
            loop {
                q = r;
                r = self.link(q)?;
                if r == tx {
                    break;
                }
            }
            tx = q;
        }
        if self.is_char_node(tx) || self.node_type(tx)? != self.cur_chr as u16 {
            return Ok(());
        }

        let mut q = self.head;
        let mut p = TEX_NULL;
        let mut r;
        let mut found_math;
        loop {
            r = p;
            p = q;
            found_math = false;
            if !self.is_char_node(q) {
                if self.node_type(q)? == DISC_NODE {
                    for _ in 1..=self.node_subtype(q)? {
                        p = self.link(p)?;
                    }
                    if p == tx {
                        return Ok(());
                    }
                } else if self.node_type(q)? == MATH_NODE
                    && self.node_subtype(q)? == BEGIN_M_CODE as u16
                {
                    found_math = true;
                }
            }
            q = self.link(p)?;
            if q == tx {
                break;
            }
        }

        q = self.link(tx)?;
        self.set_link(p, q)?;
        self.set_link(tx, TEX_NULL)?;
        if q == TEX_NULL {
            if found_math {
                return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(p)));
            }
            self.tail = p;
        } else if found_math {
            self.tail = r;
            self.set_link(r, TEX_NULL)?;
            self.flush_node_list(p)?;
        }
        self.flush_node_list(tx)?;
        Ok(())
    }

    /// Switches outer horizontal mode back to restricted/outer vertical mode
    /// (`head_for_vmode`).
    fn head_for_vmode(&mut self) -> Result<(), TeXMouthError> {
        if self.mode < 0 {
            if self.cur_cmd != CommandCode::Hrule as u8 {
                return self.off_save();
            }
            self.report_error(
                "You can't use `\\hrule' here except with leaders",
                &[
                    "To put a horizontal rule in an hbox or an alignment,",
                    "you should use \\leaders or \\hrulefill (see The TeXbook).",
                ],
            );
            return Ok(());
        }
        // C `head_for_vmode` for an outer paragraph: push `\par` back so the
        // pending command re-dispatches after `end_graf`.
        self.back_input()?;
        self.cur_tok = self.par_token;
        self.back_input()?;
        self.cur_input.index = INSERTED;
        Ok(())
    }

    /// Finishes a paragraph or closes an empty horizontal list (`end_graf`).
    fn end_graf(&mut self) -> Result<(), TeXMouthError> {
        if self.mode == TeXMode::Hmode as i16 {
            if self.head == self.tail {
                self.pop_nest()?;
            } else {
                self.line_break(false)?;
            }

            if self.error_count == 0 {
                let mut pg = 0;
                if self.head == self.contrib_head() && self.nest_ptr == 1 {
                    self.build_page()?;
                }
                if self.etex_aux != TEX_NULL {
                    self.flush_list(self.etex_aux)?;
                    self.etex_aux = TEX_NULL;
                    pg = 1;
                }
                if pg != 0 && self.error_count == 0 {
                    self.build_page()?;
                }
            }
        }
        Ok(())
    }

    /// Records a recoverable TeX error in the transcript and increments the
    /// error counter, mirroring C `error()` with its help lines.
    pub(crate) fn report_error(&mut self, message: &str, help: &[&str]) {
        self.transcript.push(format!("! {message}"));
        self.transcript
            .extend(help.iter().map(|line| line.to_string()));
        self.error_count += 1;
    }

    /// Reports a recoverable error that also back-inputs the current token
    /// (`ins_error`).
    pub(crate) fn ins_error(&mut self, message: &str, help: &[&str]) -> Result<(), TeXMouthError> {
        self.back_input()?;
        self.report_error(message, help);
        Ok(())
    }

    /// Pushes the current alignment context (`push_alignment`).
    fn push_alignment(&mut self) -> Result<(), TeXMouthError> {
        let p = self.get_node(ALIGN_STACK_NODE_SIZE)?;
        self.mem[p as usize].s1 = self.align_ptr;
        self.mem[p as usize].s0 = self.cur_align;
        self.mem[(p + 1) as usize].s0 = self.mem[self.align_head() as usize].s1;
        self.mem[(p + 1) as usize].s1 = self.cur_span;
        self.mem[(p + 2) as usize].s1 = self.cur_loop;
        self.mem[(p + 3) as usize].s1 = self.align_state;
        self.mem[(p + 4) as usize].s0 = self.cur_head;
        self.mem[(p + 4) as usize].s1 = self.cur_tail;
        self.mem[(p + 5) as usize].s0 = self.cur_pre_head;
        self.mem[(p + 5) as usize].s1 = self.cur_pre_tail;
        self.align_ptr = p;
        self.cur_head = self.get_avail()?;
        self.cur_pre_head = self.get_avail()?;
        Ok(())
    }

    /// Restores the enclosing alignment context (`pop_alignment`).
    fn pop_alignment(&mut self) -> Result<(), TeXMouthError> {
        self.mem[self.cur_head as usize].s1 = self.avail;
        self.avail = self.cur_head;
        self.mem[self.cur_pre_head as usize].s1 = self.avail;
        self.avail = self.cur_pre_head;
        let p = self.align_ptr;
        self.cur_tail = self.mem[(p + 4) as usize].s1;
        self.cur_head = self.mem[(p + 4) as usize].s0;
        self.cur_pre_tail = self.mem[(p + 5) as usize].s1;
        self.cur_pre_head = self.mem[(p + 5) as usize].s0;
        self.align_state = self.mem[(p + 3) as usize].s1;
        self.cur_loop = self.mem[(p + 2) as usize].s1;
        self.cur_span = self.mem[(p + 1) as usize].s1;
        let align_head = self.align_head() as usize;
        self.mem[align_head].s1 = self.mem[(p + 1) as usize].s0;
        self.cur_align = self.mem[p as usize].s0;
        self.align_ptr = self.mem[p as usize].s1;
        self.free_node(p, ALIGN_STACK_NODE_SIZE)?;
        Ok(())
    }

    /// Scans one preamble token, resolving `\span` chains and a `\tabskip`
    /// assignment inside the preamble (`get_preamble_token`).
    fn get_preamble_token(&mut self) -> Result<(), TeXMouthError> {
        loop {
            self.get_token()?;
            while self.cur_chr == SPAN_CODE && self.cur_cmd == CommandCode::TabMark as u8 {
                self.get_token()?;
                if self.cur_cmd > MAX_COMMAND {
                    self.expand()?;
                    self.get_token()?;
                }
            }
            if self.cur_cmd == CommandCode::Endv as u8 {
                return Err(TeXMouthError::UnsupportedPrimitive(
                    "interwoven alignment preambles",
                ));
            }
            if self.cur_cmd == CommandCode::AssignGlue as u8
                && self.cur_chr == (GLUE_PAR_BASE + GLUE_PAR_TAB_SKIP) as i32
            {
                self.scan_optional_equals()?;
                self.scan_glue(GLUE_VAL)?;
                if self.int_par(INT_PAR_GLOBAL_DEFS)? > 0 {
                    self.geq_define(
                        GLUE_PAR_BASE + GLUE_PAR_TAB_SKIP,
                        CommandCode::GlueRef as u16,
                        self.cur_val,
                    )?;
                } else {
                    self.eq_define(
                        GLUE_PAR_BASE + GLUE_PAR_TAB_SKIP,
                        CommandCode::GlueRef as u16,
                        self.cur_val,
                    )?;
                }
            } else {
                return Ok(());
            }
        }
    }

    /// Begins an alignment (`init_align`).
    fn init_align(&mut self) -> Result<(), TeXMouthError> {
        let save_cs_ptr = self.cur_cs;
        self.push_alignment()?;
        self.align_state = -1_000_000;

        if self.mode == TeXMode::Mmode as i16 && (self.tail != self.head || self.aux.s1 != TEX_NULL)
        {
            self.report_error(
                "Improper \\halign inside $$'s",
                &[
                    "Displays can use special alignments (like \\eqalignno)",
                    "only if nothing but the alignment itself is between $$'s.",
                    "So I've deleted the formulas that preceded this alignment.",
                ],
            );
            self.flush_math()?;
        }

        self.push_nest()?;

        if self.mode == TeXMode::Mmode as i16 {
            self.mode = -(TeXMode::Vmode as i16);
            self.aux.s1 = self.nest[self.nest_ptr - 2].aux.s1;
        } else if self.mode > 0 {
            self.mode = -self.mode;
        }

        self.scan_spec(ALIGN_GROUP, false)?;
        let align_head = self.align_head() as usize;
        self.mem[align_head].s1 = TEX_NULL;
        self.cur_align = self.align_head();
        self.cur_loop = TEX_NULL;
        self.scanner_status = ALIGNING;
        self.warning_index = save_cs_ptr;
        self.align_state = -1_000_000;

        let hold_head = self.hold_head();
        loop {
            let skip = self.new_param_glue(GLUE_PAR_TAB_SKIP)?;
            self.mem[self.cur_align as usize].s1 = skip;
            self.cur_align = skip;
            if self.cur_cmd == CommandCode::CarRet as u8 {
                break;
            }

            let mut p = hold_head;
            self.mem[p as usize].s1 = TEX_NULL;

            'u_part: loop {
                self.get_preamble_token()?;
                if self.cur_cmd == CommandCode::MacParam as u8 {
                    break 'u_part;
                }
                if self.cur_cmd <= CommandCode::CarRet as u8
                    && self.cur_cmd >= CommandCode::TabMark as u8
                    && self.align_state == -1_000_000
                {
                    if p == hold_head
                        && self.cur_loop == TEX_NULL
                        && self.cur_cmd == CommandCode::TabMark as u8
                    {
                        self.cur_loop = self.cur_align;
                    } else {
                        self.back_input()?;
                        self.report_error(
                            "Missing # inserted in alignment preamble",
                            &[
                                "There should be exactly one # between &'s, when an",
                                "\\halign or \\valign is being set up. In this case you had",
                                "none, so I've put one in; maybe that will work.",
                            ],
                        );
                        break 'u_part;
                    }
                } else if self.cur_cmd != CommandCode::Spacer as u8 || p != hold_head {
                    let q = self.get_avail()?;
                    self.mem[p as usize].s1 = q;
                    p = q;
                    self.mem[p as usize].s0 = self.cur_tok;
                }
            }

            let entry = self.new_null_box()?;
            self.mem[self.cur_align as usize].s1 = entry;
            self.cur_align = entry;
            self.mem[self.cur_align as usize].s0 = self.end_span();
            self.mem[(self.cur_align + 1) as usize].s1 = NULL_FLAG;
            self.mem[(self.cur_align + 3) as usize].s1 = self.mem[hold_head as usize].s1;
            let mut p = hold_head;
            self.mem[p as usize].s1 = TEX_NULL;

            'v_part: loop {
                self.get_preamble_token()?;
                if self.cur_cmd <= CommandCode::CarRet as u8
                    && self.cur_cmd >= CommandCode::TabMark as u8
                    && self.align_state == -1_000_000
                {
                    break 'v_part;
                }
                if self.cur_cmd == CommandCode::MacParam as u8 {
                    self.report_error(
                        "Only one # is allowed per tab",
                        &[
                            "There should be exactly one # between &'s, when an",
                            "\\halign or \\valign is being set up. In this case you had",
                            "more than one, so I'm ignoring all but the first.",
                        ],
                    );
                    continue 'v_part;
                }
                let q = self.get_avail()?;
                self.mem[p as usize].s1 = q;
                p = q;
                self.mem[p as usize].s0 = self.cur_tok;
            }

            let q = self.get_avail()?;
            self.mem[p as usize].s1 = q;
            p = q;
            self.mem[p as usize].s0 = CS_TOKEN_FLAG + FROZEN_END_TEMPLATE_ID;
            self.mem[(self.cur_align + 2) as usize].s1 = self.mem[hold_head as usize].s1;
        }

        self.scanner_status = SCAN_NORMAL;
        self.new_save_level(ALIGN_GROUP)?;

        let every_cr = self.local(LOCAL_EVERY_CR)?;
        if every_cr != TEX_NULL {
            self.begin_token_list(every_cr, EVERY_CR_TEXT)?;
        }

        self.align_peek()
    }

    /// Starts a column entry's inner list (`init_span`).
    fn init_span(&mut self, p: i32) -> Result<(), TeXMouthError> {
        self.push_nest()?;
        if self.mode == -(TeXMode::Hmode as i16) {
            self.aux.s0 = 1000;
        } else {
            self.aux.s1 = IGNORE_DEPTH;
            self.normal_paragraph()?;
        }
        self.cur_span = p;
        Ok(())
    }

    /// Starts a new alignment row (`init_row`).
    fn init_row(&mut self) -> Result<(), TeXMouthError> {
        self.push_nest()?;
        self.mode = (-(TeXMode::Hmode as i16) - 1) - self.mode;
        if self.mode == -(TeXMode::Hmode as i16) {
            self.aux.s0 = 0;
        } else {
            self.aux.s1 = 0;
        }
        let first = self.mem[self.align_head() as usize].s1;
        let spec = self.word_at(first + 1, 0)?.s0;
        let glue = self.new_glue(spec)?;
        self.mem[self.tail as usize].s1 = glue;
        self.tail = glue;
        self.mem[self.tail as usize].set_b16_s0((GLUE_PAR_TAB_SKIP + 1) as u16);
        self.cur_align = self.mem[first as usize].s1;
        self.cur_tail = self.cur_head;
        self.cur_pre_tail = self.cur_pre_head;
        self.init_span(self.cur_align)
    }

    /// Starts a new alignment column (`init_col`).
    fn init_col(&mut self) -> Result<(), TeXMouthError> {
        self.mem[(self.cur_align + 5) as usize].s0 = self.cur_cmd as i32;
        if self.cur_cmd == CommandCode::Omit as u8 {
            self.align_state = 0;
        } else {
            self.back_input()?;
            let template = self.mem[(self.cur_align + 3) as usize].s1;
            self.begin_token_list(template, U_TEMPLATE)?;
        }
        Ok(())
    }

    /// Finishes a column entry, returning `true` when the row should end
    /// (`fin_col`).
    fn fin_col(&mut self) -> Result<bool, TeXMouthError> {
        if self.cur_align == TEX_NULL {
            return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                self.cur_align,
            )));
        }
        let q = self.mem[self.cur_align as usize].s1;
        if q == TEX_NULL || self.align_state < 500_000 {
            return Err(TeXMouthError::UnsupportedPrimitive(
                "interwoven alignment preambles",
            ));
        }
        let mut p = self.mem[q as usize].s1;
        if p == TEX_NULL && self.mem[(self.cur_align + 5) as usize].s0 < CR_CODE {
            if self.cur_loop != TEX_NULL {
                let entry = self.new_null_box()?;
                self.mem[q as usize].s1 = entry;
                p = entry;
                self.mem[p as usize].s0 = self.end_span();
                self.mem[(p + 1) as usize].s1 = NULL_FLAG;
                self.cur_loop = self.link(self.cur_loop)?;
                let hold_head = self.hold_head();
                let mut qq = hold_head;
                let mut r = self.mem[(self.cur_loop + 3) as usize].s1;
                while r != TEX_NULL {
                    let cell = self.get_avail()?;
                    self.mem[qq as usize].s1 = cell;
                    qq = cell;
                    self.mem[qq as usize].s0 = self.mem[r as usize].s0;
                    r = self.link(r)?;
                }
                self.mem[qq as usize].s1 = TEX_NULL;
                self.mem[(p + 3) as usize].s1 = self.mem[hold_head as usize].s1;
                qq = hold_head;
                r = self.mem[(self.cur_loop + 2) as usize].s1;
                while r != TEX_NULL {
                    let cell = self.get_avail()?;
                    self.mem[qq as usize].s1 = cell;
                    qq = cell;
                    self.mem[qq as usize].s0 = self.mem[r as usize].s0;
                    r = self.link(r)?;
                }
                self.mem[qq as usize].s1 = TEX_NULL;
                self.mem[(p + 2) as usize].s1 = self.mem[hold_head as usize].s1;
                self.cur_loop = self.link(self.cur_loop)?;
                let spec = self.word_at(self.cur_loop + 1, 0)?.s0;
                let glue = self.new_glue(spec)?;
                self.mem[p as usize].s1 = glue;
                self.mem[glue as usize].set_b16_s0((GLUE_PAR_TAB_SKIP + 1) as u16);
            } else {
                self.mem[(self.cur_align + 5) as usize].s0 = CR_CODE;
                self.report_error(
                    "Extra alignment tab has been changed to \\cr",
                    &[
                        "You have given more \\span or & marks than there were",
                        "in the preamble to the \\halign or \\valign now in progress.",
                        "So I'll assume that you meant to type \\cr instead.",
                    ],
                );
            }
        }
        if self.mem[(self.cur_align + 5) as usize].s0 != SPAN_CODE {
            self.unsave()?;
            self.new_save_level(ALIGN_GROUP)?;
            let (u, w);
            if self.mode == -(TeXMode::Hmode as i16) {
                self.adjust_tail = self.cur_tail;
                self.pre_adjust_tail = self.cur_pre_tail;
                let packed = self.hpack(self.mem[self.head as usize].s1, 0, ADDITIONAL)?;
                w = self.mem[(packed + 1) as usize].s1;
                u = packed;
                self.cur_tail = self.adjust_tail;
                self.adjust_tail = TEX_NULL;
                self.cur_pre_tail = self.pre_adjust_tail;
                self.pre_adjust_tail = TEX_NULL;
            } else {
                let packed = self.vpackage(self.mem[self.head as usize].s1, 0, ADDITIONAL, 0)?;
                w = self.mem[(packed + 3) as usize].s1;
                u = packed;
            }
            let mut n = 0;
            if self.cur_span != self.cur_align {
                let mut q2 = self.cur_span;
                loop {
                    n += 1;
                    let next = self.mem[q2 as usize].s1;
                    q2 = self.mem[next as usize].s1;
                    if q2 == self.cur_align {
                        break;
                    }
                }
                if n > u16::MAX as i32 {
                    return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(n)));
                }
                let mut q2 = self.cur_span;
                while self.mem[self.mem[q2 as usize].s0 as usize].s1 < n {
                    q2 = self.mem[q2 as usize].s0;
                }
                if self.mem[self.mem[q2 as usize].s0 as usize].s1 > n {
                    let s = self.get_node(SPAN_NODE_SIZE)?;
                    self.mem[s as usize].s0 = self.mem[q2 as usize].s0;
                    self.mem[s as usize].s1 = n;
                    self.mem[q2 as usize].s0 = s;
                    self.mem[(s + 1) as usize].s1 = w;
                } else {
                    let span = self.mem[q2 as usize].s0;
                    if self.mem[(span + 1) as usize].s1 < w {
                        self.mem[(span + 1) as usize].s1 = w;
                    }
                }
            } else if w > self.mem[(self.cur_align + 1) as usize].s1 {
                self.mem[(self.cur_align + 1) as usize].s1 = w;
            }
            self.set_node_type(u, UNSET_NODE)?;
            self.mem[u as usize].set_b16_s0(n as u16);
            let o = if self.total_stretch[FILLL as usize] != 0 {
                FILLL
            } else if self.total_stretch[FILL as usize] != 0 {
                FILL
            } else if self.total_stretch[FIL as usize] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.mem[(u + 5) as usize].set_b16_s0(o);
            self.mem[(u + 6) as usize].s1 = self.total_stretch[o as usize];
            let o = if self.total_shrink[FILLL as usize] != 0 {
                FILLL
            } else if self.total_shrink[FILL as usize] != 0 {
                FILL
            } else if self.total_shrink[FIL as usize] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.mem[(u + 5) as usize].set_b16_s1(o);
            self.mem[(u + 4) as usize].s1 = self.total_shrink[o as usize];
            self.pop_nest()?;
            self.mem[self.tail as usize].s1 = u;
            self.tail = u;

            let spec = self
                .word_at(self.mem[self.cur_align as usize].s1 + 1, 0)?
                .s0;
            let glue = self.new_glue(spec)?;
            self.mem[self.tail as usize].s1 = glue;
            self.tail = glue;
            self.mem[self.tail as usize].set_b16_s0((GLUE_PAR_TAB_SKIP + 1) as u16);
            if self.mem[(self.cur_align + 5) as usize].s0 >= CR_CODE {
                return Ok(true);
            }
            self.init_span(p)?;
        }
        self.align_state = 1_000_000;
        loop {
            self.get_x_or_protected()?;
            if self.cur_cmd != CommandCode::Spacer as u8 {
                break;
            }
        }
        self.cur_align = p;
        self.init_col()?;
        Ok(false)
    }

    /// Finishes an alignment row (`fin_row`).
    fn fin_row(&mut self) -> Result<(), TeXMouthError> {
        let p;
        if self.mode == -(TeXMode::Hmode as i16) {
            p = self.hpack(self.mem[self.head as usize].s1, 0, ADDITIONAL)?;
            self.pop_nest()?;
            if self.cur_pre_head != self.cur_pre_tail {
                self.mem[self.tail as usize].s1 = self.mem[self.cur_pre_head as usize].s1;
                self.tail = self.cur_pre_tail;
            }
            self.append_to_vlist(p)?;
            if self.cur_head != self.cur_tail {
                self.mem[self.tail as usize].s1 = self.mem[self.cur_head as usize].s1;
                self.tail = self.cur_tail;
            }
        } else {
            p = self.vpackage(self.mem[self.head as usize].s1, 0, ADDITIONAL, MAX_HALFWORD)?;
            self.pop_nest()?;
            self.mem[self.tail as usize].s1 = p;
            self.tail = p;
            self.aux.s0 = 1000;
        }
        self.set_node_type(p, UNSET_NODE)?;
        self.mem[(p + 6) as usize].s1 = 0;
        let every_cr = self.local(LOCAL_EVERY_CR)?;
        if every_cr != TEX_NULL {
            self.begin_token_list(every_cr, EVERY_CR_TEXT)?;
        }
        self.align_peek()
    }

    /// Finalizes an alignment, packaging columns into rows and reattaching
    /// the result (`fin_align`).
    fn fin_align(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_group != ALIGN_GROUP {
            return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                self.cur_group as i32,
            )));
        }
        self.unsave()?;
        if self.cur_group != ALIGN_GROUP {
            return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                self.cur_group as i32,
            )));
        }
        self.unsave()?;
        let o = if self.nest_mode_at(self.nest_ptr - 1) == TeXMode::Mmode as i16 {
            self.dimen_par(DIMEN_PAR_DISPLAY_INDENT)?
        } else {
            0
        };
        let mut q = self.mem[self.mem[self.align_head() as usize].s1 as usize].s1;
        loop {
            self.flush_list(self.mem[(q + 3) as usize].s1)?;
            self.flush_list(self.mem[(q + 2) as usize].s1)?;
            let p = self.mem[self.mem[q as usize].s1 as usize].s1;
            if self.mem[(q + 1) as usize].s1 == NULL_FLAG {
                self.mem[(q + 1) as usize].s1 = 0;
                let r = self.mem[q as usize].s1;
                let s = self.word_at(r + 1, 0)?.s0;
                if s != 0 {
                    self.mem[0].s1 += 1;
                    self.delete_glue_ref(s)?;
                    self.mem[(r + 1) as usize].s0 = 0;
                }
            }
            if self.mem[q as usize].s0 != self.end_span() {
                let t = self.mem[(q + 1) as usize].s1
                    + self.mem[(self.word_at(self.mem[q as usize].s1 + 1, 0)?.s0 + 1) as usize].s1;
                let mut r = self.mem[q as usize].s0;
                let mut s = self.end_span();
                self.mem[s as usize].s0 = p;
                let mut n = 1;
                loop {
                    self.mem[(r + 1) as usize].s1 -= t;
                    let u = self.mem[r as usize].s0;
                    while self.mem[r as usize].s1 > n {
                        s = self.mem[s as usize].s0;
                        n = self.mem[self.mem[s as usize].s0 as usize].s1 + 1;
                    }
                    if self.mem[r as usize].s1 < n {
                        self.mem[r as usize].s0 = self.mem[s as usize].s0;
                        self.mem[s as usize].s0 = r;
                        self.mem[r as usize].s1 -= 1;
                        s = r;
                    } else {
                        let span = self.mem[s as usize].s0;
                        if self.mem[(r + 1) as usize].s1 > self.mem[(span + 1) as usize].s1 {
                            self.mem[(span + 1) as usize].s1 = self.mem[(r + 1) as usize].s1;
                        }
                        self.free_node(r, SPAN_NODE_SIZE)?;
                    }
                    r = u;
                    if r == self.end_span() {
                        break;
                    }
                }
            }
            self.set_node_type(q, UNSET_NODE)?;
            self.mem[q as usize].set_b16_s0(0);
            self.mem[(q + 3) as usize].s1 = 0;
            self.mem[(q + 2) as usize].s1 = 0;
            self.mem[(q + 5) as usize].set_b16_s0(NORMAL);
            self.mem[(q + 5) as usize].set_b16_s1(NORMAL);
            self.mem[(q + 6) as usize].s1 = 0;
            self.mem[(q + 4) as usize].s1 = 0;
            q = p;
            if q == TEX_NULL {
                break;
            }
        }
        self.save_ptr -= 2;
        self.pack_begin_line = -self.mode_line;
        let raw = |index: usize| -> Result<i32, TeXMouthError> {
            match self.save_stack.get(index) {
                Some(SaveStackEntry::RawValue(value)) => Ok(*value),
                _ => Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                    index as i32,
                ))),
            }
        };
        let spec_code = raw(self.save_ptr)?;
        let spec_width = raw(self.save_ptr + 1)?;
        let p;
        if self.mode == -(TeXMode::Vmode as i16) {
            let rule_save = self.dimen_par(DIMEN_PAR_OVERFULL_RULE)?;
            self.set_dimen_par(DIMEN_PAR_OVERFULL_RULE, 0)?;
            p = self.hpack(
                self.mem[self.align_head() as usize].s1,
                spec_width,
                spec_code,
            )?;
            self.set_dimen_par(DIMEN_PAR_OVERFULL_RULE, rule_save)?;
        } else {
            let mut q2 = self.mem[self.mem[self.align_head() as usize].s1 as usize].s1;
            loop {
                self.mem[(q2 + 3) as usize].s1 = self.mem[(q2 + 1) as usize].s1;
                self.mem[(q2 + 1) as usize].s1 = 0;
                q2 = self.mem[self.mem[q2 as usize].s1 as usize].s1;
                if q2 == TEX_NULL {
                    break;
                }
            }
            p = self.vpackage(
                self.mem[self.align_head() as usize].s1,
                spec_width,
                spec_code,
                MAX_HALFWORD,
            )?;
            q2 = self.mem[self.mem[self.align_head() as usize].s1 as usize].s1;
            loop {
                self.mem[(q2 + 1) as usize].s1 = self.mem[(q2 + 3) as usize].s1;
                self.mem[(q2 + 3) as usize].s1 = 0;
                q2 = self.mem[self.mem[q2 as usize].s1 as usize].s1;
                if q2 == TEX_NULL {
                    break;
                }
            }
        }
        self.pack_begin_line = 0;

        let mut q = self.mem[self.head as usize].s1;
        let mut s = self.head;
        while q != TEX_NULL {
            if !self.is_char_node(q) {
                if self.node_type(q)? == UNSET_NODE {
                    if self.mode == -(TeXMode::Vmode as i16) {
                        self.set_node_type(q, HLIST_NODE)?;
                        self.mem[(q + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
                        if self.nest_mode_at(self.nest_ptr - 1) == TeXMode::Mmode as i16 {
                            self.mem[q as usize].set_b16_s0(DLIST_CODE);
                        }
                    } else {
                        self.set_node_type(q, VLIST_NODE)?;
                        self.mem[(q + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                    }
                    let p_stretch = self.mem[(p + 5) as usize].b16_s0();
                    let p_shrink = self.mem[(p + 5) as usize].b16_s1();
                    self.mem[(q + 5) as usize].set_b16_s0(p_stretch);
                    self.mem[(q + 5) as usize].set_b16_s1(p_shrink);
                    let glue_set = self.mem[(p + 6) as usize].gr();
                    self.mem[(q + 6) as usize].set_gr(glue_set);
                    self.mem[(q + 4) as usize].s1 = o;
                    let q_list = self.mem[(q + 5) as usize].s1;
                    let p_list = self.mem[(p + 5) as usize].s1;
                    let mut r = self.mem[q_list as usize].s1;
                    let mut s2 = self.mem[p_list as usize].s1;
                    let hold_head = self.hold_head();
                    while r != TEX_NULL {
                        let mut n = i32::from(self.mem[r as usize].b16_s0());
                        let mut t = self.mem[(s2 + 1) as usize].s1;
                        let w = t;
                        let mut u = hold_head;
                        self.mem[r as usize].set_b16_s0(0);
                        while n > 0 {
                            n -= 1;
                            s2 = self.link(s2)?;
                            let v = self.mem[(s2 + 1) as usize].s0;
                            let glue = self.new_glue(v)?;
                            self.mem[u as usize].s1 = glue;
                            u = glue;
                            self.mem[u as usize].set_b16_s0((GLUE_PAR_TAB_SKIP + 1) as u16);
                            t += self.word_at(v, 1)?.s1;
                            if self.mem[(p + 5) as usize].b16_s1() == STRETCHING {
                                if self.mem[v as usize].b16_s1()
                                    == self.mem[(p + 5) as usize].b16_s0()
                                {
                                    t += tex_round(
                                        self.mem[(p + 6) as usize].gr()
                                            * self.word_at(v, 2)?.s1 as f64,
                                    );
                                }
                            } else if self.mem[(p + 5) as usize].b16_s1() == SHRINKING
                                && self.mem[v as usize].b16_s0()
                                    == self.mem[(p + 5) as usize].b16_s0()
                            {
                                t -= tex_round(
                                    self.mem[(p + 6) as usize].gr() * self.word_at(v, 3)?.s1 as f64,
                                );
                            }
                            s2 = self.link(s2)?;
                            let cell = self.new_null_box()?;
                            self.mem[u as usize].s1 = cell;
                            u = cell;
                            t += self.mem[(s2 + 1) as usize].s1;
                            if self.mode == -(TeXMode::Vmode as i16) {
                                self.mem[(u + 1) as usize].s1 = self.mem[(s2 + 1) as usize].s1;
                            } else {
                                self.set_node_type(u, VLIST_NODE)?;
                                self.mem[(u + 3) as usize].s1 = self.mem[(s2 + 1) as usize].s1;
                            }
                        }
                        if self.mode == -(TeXMode::Vmode as i16) {
                            self.mem[(r + 3) as usize].s1 = self.mem[(q + 3) as usize].s1;
                            self.mem[(r + 2) as usize].s1 = self.mem[(q + 2) as usize].s1;
                            if t == self.mem[(r + 1) as usize].s1 {
                                self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
                                self.mem[(r + 5) as usize].set_b16_s0(NORMAL);
                                self.mem[(r + 6) as usize].set_gr(0.0);
                            } else if t > self.mem[(r + 1) as usize].s1 {
                                self.mem[(r + 5) as usize].set_b16_s1(STRETCHING);
                                if self.mem[(r + 6) as usize].s1 == 0 {
                                    self.mem[(r + 6) as usize].set_gr(0.0);
                                } else {
                                    let glue_set = (t - self.mem[(r + 1) as usize].s1) as f64
                                        / self.mem[(r + 6) as usize].s1 as f64;
                                    self.mem[(r + 6) as usize].set_gr(glue_set);
                                }
                            } else {
                                let shrink_o = self.mem[(r + 5) as usize].b16_s1();
                                self.mem[(r + 5) as usize].set_b16_s0(shrink_o);
                                self.mem[(r + 5) as usize].set_b16_s1(SHRINKING);
                                if self.mem[(r + 4) as usize].s1 == 0 {
                                    self.mem[(r + 6) as usize].set_gr(0.0);
                                } else if self.mem[(r + 5) as usize].b16_s0() == NORMAL
                                    && self.mem[(r + 1) as usize].s1 - t
                                        > self.mem[(r + 4) as usize].s1
                                {
                                    self.mem[(r + 6) as usize].set_gr(1.0);
                                } else {
                                    let glue_set = (self.mem[(r + 1) as usize].s1 - t) as f64
                                        / self.mem[(r + 4) as usize].s1 as f64;
                                    self.mem[(r + 6) as usize].set_gr(glue_set);
                                }
                            }
                            self.mem[(r + 1) as usize].s1 = w;
                            self.set_node_type(r, HLIST_NODE)?;
                        } else {
                            self.mem[(r + 1) as usize].s1 = self.mem[(q + 1) as usize].s1;
                            if t == self.mem[(r + 3) as usize].s1 {
                                self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
                                self.mem[(r + 5) as usize].set_b16_s0(NORMAL);
                                self.mem[(r + 6) as usize].set_gr(0.0);
                            } else if t > self.mem[(r + 3) as usize].s1 {
                                self.mem[(r + 5) as usize].set_b16_s1(STRETCHING);
                                if self.mem[(r + 6) as usize].s1 == 0 {
                                    self.mem[(r + 6) as usize].set_gr(0.0);
                                } else {
                                    let glue_set = (t - self.mem[(r + 3) as usize].s1) as f64
                                        / self.mem[(r + 6) as usize].s1 as f64;
                                    self.mem[(r + 6) as usize].set_gr(glue_set);
                                }
                            } else {
                                let shrink_o = self.mem[(r + 5) as usize].b16_s1();
                                self.mem[(r + 5) as usize].set_b16_s0(shrink_o);
                                self.mem[(r + 5) as usize].set_b16_s1(SHRINKING);
                                if self.mem[(r + 4) as usize].s1 == 0 {
                                    self.mem[(r + 6) as usize].set_gr(0.0);
                                } else if self.mem[(r + 5) as usize].b16_s0() == NORMAL
                                    && self.mem[(r + 3) as usize].s1 - t
                                        > self.mem[(r + 4) as usize].s1
                                {
                                    self.mem[(r + 6) as usize].set_gr(1.0);
                                } else {
                                    let glue_set = (self.mem[(r + 3) as usize].s1 - t) as f64
                                        / self.mem[(r + 4) as usize].s1 as f64;
                                    self.mem[(r + 6) as usize].set_gr(glue_set);
                                }
                            }
                            self.mem[(r + 3) as usize].s1 = w;
                            self.set_node_type(r, VLIST_NODE)?;
                        }
                        self.mem[(r + 4) as usize].s1 = 0;
                        if u != hold_head {
                            self.mem[u as usize].s1 = self.mem[r as usize].s1;
                            self.mem[r as usize].s1 = self.mem[hold_head as usize].s1;
                            r = u;
                        }
                        r = self.mem[self.mem[r as usize].s1 as usize].s1;
                        s2 = self.mem[self.mem[s2 as usize].s1 as usize].s1;
                    }
                } else if self.node_type(q)? == RULE_NODE {
                    if self.mem[(q + 1) as usize].s1 == NULL_FLAG {
                        self.mem[(q + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
                    }
                    if self.mem[(q + 3) as usize].s1 == NULL_FLAG {
                        self.mem[(q + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                    }
                    if self.mem[(q + 2) as usize].s1 == NULL_FLAG {
                        self.mem[(q + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
                    }
                    if o != 0 {
                        let r = self.mem[q as usize].s1;
                        self.mem[q as usize].s1 = TEX_NULL;
                        q = self.hpack(q, 0, ADDITIONAL)?;
                        self.mem[(q + 4) as usize].s1 = o;
                        self.mem[q as usize].s1 = r;
                        self.mem[s as usize].s1 = q;
                    }
                }
            }
            s = q;
            q = self.link(q)?;
        }
        self.flush_node_list(p)?;
        self.pop_alignment()?;
        let aux_save = self.aux;
        let p = self.mem[self.head as usize].s1;
        let q = self.tail;
        self.pop_nest()?;
        if self.mode == TeXMode::Mmode as i16 {
            self.do_assignments()?;
            if self.cur_cmd != CommandCode::MathShift as u8 {
                self.back_input()?;
                self.report_error(
                    "Missing $$ inserted",
                    &[
                        "Displays can use special alignments (like \\eqalignno)",
                        "only if nothing but the alignment itself is between $$'s.",
                    ],
                );
            } else {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::MathShift as u8 {
                    self.back_input()?;
                    self.report_error(
                        "Display math should end with $$",
                        &[
                            "The `$' that I just saw supposedly matches a previous `$$'.",
                            "So I shall assume that you typed `$$' both times.",
                        ],
                    );
                }
            }
            self.flush_node_list(self.etex_aux)?;
            self.pop_nest()?;
            let pen = self.new_penalty(self.int_par(INT_PAR_PRE_DISPLAY_PENALTY)?)?;
            self.mem[self.tail as usize].s1 = pen;
            self.tail = pen;
            let skip = self.new_param_glue(GLUE_PAR_ABOVE_DISPLAY_SKIP)?;
            self.mem[self.tail as usize].s1 = skip;
            self.tail = skip;
            self.mem[self.tail as usize].s1 = p;
            if p != TEX_NULL {
                self.tail = q;
            }
            let pen = self.new_penalty(self.int_par(INT_PAR_POST_DISPLAY_PENALTY)?)?;
            self.mem[self.tail as usize].s1 = pen;
            self.tail = pen;
            let skip = self.new_param_glue(GLUE_PAR_BELOW_DISPLAY_SKIP)?;
            self.mem[self.tail as usize].s1 = skip;
            self.tail = skip;
            self.aux.s1 = aux_save.s1;
            self.resume_after_display()?;
        } else {
            self.aux = aux_save;
            self.mem[self.tail as usize].s1 = p;
            if p != TEX_NULL {
                self.tail = q;
            }
            if self.mode == TeXMode::Vmode as i16 {
                self.build_page()?;
            }
        }
        Ok(())
    }

    /// Peeks at the next token after `\cr` or a row end, dispatching
    /// `\noalign`, `}`, `\crcr`, or the next row (`align_peek`).
    fn align_peek(&mut self) -> Result<(), TeXMouthError> {
        loop {
            self.align_state = 1_000_000;
            loop {
                self.get_x_or_protected()?;
                if self.cur_cmd != CommandCode::Spacer as u8 {
                    break;
                }
            }
            if self.cur_cmd == CommandCode::NoAlign as u8 {
                self.scan_left_brace()?;
                self.new_save_level(NO_ALIGN_GROUP)?;
                if self.mode == -(TeXMode::Vmode as i16) {
                    self.normal_paragraph()?;
                }
                return Ok(());
            } else if self.cur_cmd == CommandCode::RightBrace as u8 {
                return self.fin_align();
            } else if self.cur_cmd == CommandCode::CarRet as u8 && self.cur_chr == CR_CR_CODE {
                continue;
            } else {
                self.init_row()?;
                return self.init_col();
            }
        }
    }

    /// Reports a `&`/`\cr` outside an alignment column (`align_error`).
    fn align_error(&mut self) -> Result<(), TeXMouthError> {
        if self.align_state.abs() > 2 {
            self.report_error(
                "Misplaced alignment tab character or \\cr",
                &[
                    "I can't figure out why you would want to use a tab mark",
                    "or \\cr or \\span just now. If something like a right brace",
                    "up above has ended a previous alignment prematurely,",
                    "you're probably due for more error messages, and you",
                    "might try typing `S' now just to see what is salvageable.",
                ],
            );
            return Ok(());
        }

        self.back_input()?;
        if self.align_state < 0 {
            self.align_state += 1;
            self.cur_tok = LEFT_BRACE_TOKEN + i32::from(b'{');
            self.report_error(
                "Missing { inserted",
                &[
                    "I've put in what seems to be necessary to fix",
                    "the current column of the current alignment.",
                    "Try to go on, since this might almost work.",
                ],
            );
        } else {
            self.align_state -= 1;
            self.cur_tok = RIGHT_BRACE_TOKEN + i32::from(b'}');
            self.report_error(
                "Missing } inserted",
                &[
                    "I've put in what seems to be necessary to fix",
                    "the current column of the current alignment.",
                    "Try to go on, since this might almost work.",
                ],
            );
        }
        Ok(())
    }

    /// Reports `\noalign` outside an alignment row boundary (`no_align_error`).
    fn no_align_error(&mut self) -> Result<(), TeXMouthError> {
        self.report_error(
            "Misplaced \\noalign",
            &[
                "I expect to see \\noalign only after the \\cr of",
                "an alignment. Proceed, and I'll ignore this case.",
            ],
        );
        Ok(())
    }

    /// Reports `\omit` outside an alignment column start (`omit_error`).
    fn omit_error(&mut self) -> Result<(), TeXMouthError> {
        self.report_error(
            "Misplaced \\omit",
            &[
                "I expect to see \\omit only after tab marks or the \\cr of",
                "an alignment. Proceed, and I'll ignore this case.",
            ],
        );
        Ok(())
    }

    /// Handles the `\endtemplate`/`\endv` boundary inside an alignment
    /// (`do_endv`).
    fn do_endv(&mut self) -> Result<(), TeXMouthError> {
        let mut base_ptr = self.input_ptr;
        if self.input_stack.len() <= base_ptr {
            self.input_stack.push(self.cur_input);
        } else {
            self.input_stack[base_ptr] = self.cur_input;
        }
        while self.input_stack[base_ptr].index != V_TEMPLATE
            && self.input_stack[base_ptr].loc == TEX_NULL
            && self.input_stack[base_ptr].state == TOKEN_LIST
        {
            if base_ptr == 0 {
                break;
            }
            base_ptr -= 1;
        }
        if self.input_stack[base_ptr].index != V_TEMPLATE
            || self.input_stack[base_ptr].loc != TEX_NULL
            || self.input_stack[base_ptr].state != TOKEN_LIST
        {
            return Err(TeXMouthError::UnsupportedPrimitive(
                "interwoven alignment preambles",
            ));
        }
        if self.cur_group == ALIGN_GROUP {
            self.end_graf()?;
            if self.fin_col()? {
                self.fin_row()?;
            }
            Ok(())
        } else {
            self.off_save()
        }
    }

    /// Pushes a missing group-closing token when a command appears in the
    /// wrong group context (`off_save`).
    fn off_save(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_group == BOTTOM_LEVEL {
            let cmd = self
                .cmd_chr_text(self.cur_cmd, self.cur_chr)
                .unwrap_or_default();
            self.report_error(
                &format!("Extra {cmd}"),
                &["Things are pretty mixed up, but I think the worst is over."],
            );
            return Ok(());
        }

        self.back_input()?;
        let p = self.get_avail()?;
        let temp_head = self.temp_head();
        self.mem[temp_head as usize].s1 = p;

        let missing = match self.cur_group {
            SEMI_SIMPLE_GROUP => {
                self.mem[p as usize].s0 = CS_TOKEN_FLAG + FROZEN_END_GROUP_ID;
                "\\endgroup"
            }
            MATH_SHIFT_GROUP => {
                self.mem[p as usize].s0 = MATH_SHIFT_TOKEN + i32::from(b'$');
                "$"
            }
            MATH_LEFT_GROUP => {
                self.mem[p as usize].s0 = CS_TOKEN_FLAG + FROZEN_RIGHT_ID;
                let q = self.get_avail()?;
                self.mem[p as usize].s1 = q;
                self.mem[q as usize].s0 = OTHER_TOKEN + i32::from(b'.');
                "\\right."
            }
            _ => {
                self.mem[p as usize].s0 = RIGHT_BRACE_TOKEN + i32::from(b'}');
                "}"
            }
        };

        let list = self.mem[temp_head as usize].s1;
        self.begin_token_list(list, INSERTED)?;
        self.report_error(
            &format!("Missing {missing} inserted"),
            &[
                "I've inserted something that you may have forgotten.",
                "(See the <inserted text> above.)",
                "With luck, this will get me unwedged. But if you",
                "really didn't forget anything, try typing `2' now; then",
                "my insertion and my current dilemma will both disappear.",
            ],
        );
        Ok(())
    }

    /// Reports that the current command is illegal in the current mode
    /// (`you_cant`/`report_illegal_case`).
    fn report_illegal_case(&mut self) {
        let cmd = self
            .cmd_chr_text(self.cur_cmd, self.cur_chr)
            .unwrap_or_default();
        self.report_error(
            &format!("You can't use `{cmd}'{}", in_mode_text(self.mode)),
            &[
                "Sorry, but I'm not programmed to handle this case;",
                "I'll just pretend that you didn't ask for it.",
                "If you're in the wrong mode, you might be able to",
                "return to the right one by typing `I}' or `I$' or `I\\par'.",
            ],
        );
    }

    /// Returns `true` in outer modes; otherwise reports the illegal-case
    /// error and returns `false` (`privileged`).
    fn privileged(&mut self) -> bool {
        if self.mode > 0 {
            return true;
        }
        self.report_illegal_case();
        false
    }

    /// Terminates the run when the page and current list are empty
    /// (`its_all_over`); otherwise feeds a forced `\vfill` page break.
    fn its_all_over(&mut self) -> Result<bool, TeXMouthError> {
        if !self.privileged() {
            return Ok(false);
        }
        if self.page_tail == self.page_head() && self.head == self.tail && self.dead_cycles == 0 {
            return Ok(true);
        }
        self.back_input()?;
        let null_box = self.new_null_box()?;
        self.mem[self.tail as usize].s1 = null_box;
        self.tail = null_box;
        self.mem[(self.tail + 1) as usize].s1 = self.dimen_par(DIMEN_PAR_HSIZE)?;
        let fill_spec = self.get_node(GLUE_SPEC_SIZE)?;
        self.mem[fill_spec as usize].s1 = TEX_NULL;
        self.mem[fill_spec as usize].set_b16_s1(FILL);
        self.mem[fill_spec as usize].set_b16_s0(NORMAL);
        self.mem[(fill_spec + 2) as usize].s1 = 65_536;
        let glue = self.new_glue(fill_spec)?;
        self.mem[self.tail as usize].s1 = glue;
        self.tail = glue;
        let penalty = self.new_penalty(NULL_FLAG)?;
        self.mem[self.tail as usize].s1 = penalty;
        self.tail = penalty;
        self.build_page()?;
        Ok(false)
    }

    /// Reports a stray `\endcsname` (`cs_error`).
    fn cs_error(&mut self) {
        self.report_error(
            "Extra \\endcsname",
            &["I'm ignoring this, since I wasn't doing a \\csname."],
        );
    }

    /// Inserts a `$` when a math-mode command appears outside math mode
    /// (`insert_dollar_sign`).
    fn insert_dollar_sign(&mut self) -> Result<(), TeXMouthError> {
        self.back_input()?;
        self.cur_tok = MATH_SHIFT_TOKEN + i32::from(b'$');
        self.ins_error(
            "Missing $ inserted",
            &[
                "I've inserted a begin-math/end-math symbol since I think",
                "you left one out. Proceed, with fingers crossed.",
            ],
        )
    }

    /// Checks an e-TeX feature flag, reporting an error when disabled
    /// (`eTeX_enabled`).
    fn etex_enabled(&mut self, enabled: bool, what: &str) -> bool {
        if !enabled {
            self.report_error(
                &format!("Improper {what}"),
                &["Sorry, this optional e-TeX feature has been disabled."],
            );
        }
        enabled
    }

    /// Flushes the current math list (`flush_math`).
    fn flush_math(&mut self) -> Result<(), TeXMouthError> {
        self.flush_node_list(self.mem[self.head as usize].s1)?;
        self.flush_node_list(self.aux.s1)?;
        self.mem[self.head as usize].s1 = TEX_NULL;
        self.tail = self.head;
        self.aux.s1 = TEX_NULL;
        Ok(())
    }

    /// Runs pending assignments after display math (`do_assignments`).
    fn do_assignments(&mut self) -> Result<(), TeXMouthError> {
        loop {
            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8
                    && self.cur_cmd != CommandCode::Relax as u8
                {
                    break;
                }
            }
            if self.cur_cmd <= MAX_NON_PREFIXED_COMMAND {
                return Ok(());
            }
            self.set_box_allowed = false;
            self.prefixed_command()?;
            self.set_box_allowed = true;
        }
    }

    /// Resumes horizontal mode after display math (`resume_after_display`).
    fn resume_after_display(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_group != MATH_SHIFT_GROUP {
            return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                self.cur_group as i32,
            )));
        }
        self.unsave()?;
        self.prev_graf += 3;
        self.push_nest()?;
        self.mode = TeXMode::Hmode as i16;
        self.aux.s0 = 1000;
        let language = self.int_par(INT_PAR_LANGUAGE)?;
        self.cur_lang = if !(1..=BIGGEST_LANG).contains(&language) {
            0
        } else {
            language
        };
        self.aux.s1 = self.cur_lang;
        self.prev_graf = (norm_min(self.int_par(INT_PAR_LEFT_HYPHEN_MIN)?) * 64
            + norm_min(self.int_par(INT_PAR_RIGHT_HYPHEN_MIN)?))
            * 65_536
            + self.cur_lang;
        self.get_x_token()?;
        if self.cur_cmd != CommandCode::Spacer as u8 {
            self.back_input()?;
        }
        if self.nest_ptr == 1 {
            self.build_page()?;
        }
        Ok(())
    }

    /// Pushes a math-mode nest level and opens a save group (`push_math`).
    fn push_math(&mut self, c: u8) -> Result<(), TeXMouthError> {
        self.push_nest()?;
        self.mode = -(TeXMode::Mmode as i16);
        self.aux.s1 = TEX_NULL;
        self.new_save_level(c)?;
        Ok(())
    }

    /// Ends the current math list, resolving a pending `\over`-style
    /// fraction or `\left` scaffolding (`fin_mlist`).
    fn fin_mlist(&mut self, p: i32) -> Result<i32, TeXMouthError> {
        let q;
        if self.aux.s1 != TEX_NULL {
            self.mem[(self.aux.s1 + 3) as usize].s1 = SUB_MLIST;
            self.mem[(self.aux.s1 + 3) as usize].s0 = self.mem[self.head as usize].s1;
            if p == TEX_NULL {
                q = self.aux.s1;
            } else {
                q = self.mem[(self.aux.s1 + 2) as usize].s0;
                if self.mem[q as usize].b16_s1() != LEFT_NOAD || self.etex_aux == TEX_NULL {
                    return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(q)));
                }
                self.mem[(self.aux.s1 + 2) as usize].s0 = self.mem[self.etex_aux as usize].s1;
                self.mem[self.etex_aux as usize].s1 = self.aux.s1;
                self.mem[self.aux.s1 as usize].s1 = p;
            }
        } else {
            self.mem[self.tail as usize].s1 = p;
            q = self.mem[self.head as usize].s1;
        }
        self.pop_nest()?;
        Ok(q)
    }

    /// Scans one math field into `p` (`scan_math`).
    fn scan_math(&mut self, p: i32) -> Result<(), TeXMouthError> {
        'restart: loop {
            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8
                    && self.cur_cmd != CommandCode::Relax as u8
                {
                    break;
                }
            }
            let c;
            'reswitch: loop {
                match CommandCode::from_u8(self.cur_cmd) {
                    Some(CommandCode::Letter | CommandCode::OtherChar | CommandCode::CharGiven) => {
                        c = self.math_code(self.cur_chr.max(0) as u32);
                        if TeXEngineState::math_char(c) == crate::scan::ACTIVE_MATH_CHAR {
                            self.cur_cs = self.cur_chr + 1;
                            let cell = self.eqtb[self.cur_cs as usize];
                            self.cur_cmd = cell.b16_s1() as u8;
                            self.cur_chr = cell.s1;
                            self.x_token()?;
                            self.back_input()?;
                            continue 'restart;
                        }
                    }
                    Some(CommandCode::CharNum) => {
                        self.scan_char_num()?;
                        self.cur_chr = self.cur_val;
                        self.cur_cmd = CommandCode::CharGiven as u8;
                        continue 'reswitch;
                    }
                    Some(CommandCode::MathCharNum) => {
                        if self.cur_chr == 2 {
                            self.scan_math_class_int()?;
                            let mut cv = TeXEngineState::set_class(self.cur_val);
                            self.scan_math_fam_int()?;
                            cv += TeXEngineState::set_family(self.cur_val);
                            self.scan_usv_num()?;
                            c = cv + self.cur_val;
                        } else if self.cur_chr == 1 {
                            self.scan_xetex_math_char_int()?;
                            c = self.cur_val;
                        } else {
                            self.scan_fifteen_bit_int()?;
                            c = TeXEngineState::set_class(self.cur_val / 4096)
                                + TeXEngineState::set_family((self.cur_val % 4096) / 256)
                                + (self.cur_val % 256);
                        }
                    }
                    Some(CommandCode::MathGiven) => {
                        c = TeXEngineState::set_class(self.cur_chr / 4096)
                            + TeXEngineState::set_family((self.cur_chr % 4096) / 256)
                            + (self.cur_chr % 256);
                    }
                    Some(CommandCode::XetexMathGiven) => {
                        c = self.cur_chr;
                    }
                    Some(CommandCode::DelimNum) => {
                        if self.cur_chr == 1 {
                            self.scan_math_class_int()?;
                            let mut cv = TeXEngineState::set_class(self.cur_val);
                            self.scan_math_fam_int()?;
                            cv += TeXEngineState::set_family(self.cur_val);
                            self.scan_usv_num()?;
                            c = cv + self.cur_val;
                        } else {
                            self.scan_delimiter_int()?;
                            let dv = self.cur_val / 4096;
                            c = TeXEngineState::set_class(dv / 4096)
                                + TeXEngineState::set_family((dv % 4096) / 256)
                                + (dv % 256);
                        }
                    }
                    _ => {
                        self.back_input()?;
                        self.scan_left_brace()?;
                        self.push_save_entry(SaveStackEntry::RawValue(p))?;
                        self.push_math(MATH_GROUP)?;
                        return Ok(());
                    }
                }
                break 'reswitch;
            }
            self.mem[p as usize].s1 = MATH_CHAR;
            self.mem[p as usize].set_b16_s0((c % 65536) as u16);
            let cur_fam = self.int_par(INT_PAR_CUR_FAM)?;
            let fam = if TeXEngineState::math_class(c) == 7
                && (0..NUMBER_MATH_FAMILIES).contains(&cur_fam)
            {
                cur_fam
            } else {
                TeXEngineState::math_fam(c)
            };
            self.mem[p as usize]
                .set_b16_s1((fam + (TeXEngineState::math_char(c) / 65536) * 256) as u16);
            return Ok(());
        }
    }

    /// Installs a math character of code `c` as a noad (`set_math_char`).
    fn set_math_char(&mut self, c: i32) -> Result<(), TeXMouthError> {
        if TeXEngineState::math_char(c) == crate::scan::ACTIVE_MATH_CHAR {
            self.cur_cs = self.cur_chr + 1;
            let cell = self.eqtb[self.cur_cs as usize];
            self.cur_cmd = cell.b16_s1() as u8;
            self.cur_chr = cell.s1;
            self.x_token()?;
            self.back_input()?;
            return Ok(());
        }
        let p = self.new_noad()?;
        self.mem[(p + 1) as usize].s1 = MATH_CHAR;
        let ch = TeXEngineState::math_char(c);
        self.mem[(p + 1) as usize].set_b16_s0((ch % 65536) as u16);
        self.mem[(p + 1) as usize].set_b16_s1(TeXEngineState::math_fam(c) as u16);
        if TeXEngineState::math_class(c) == 7 {
            let cur_fam = self.int_par(INT_PAR_CUR_FAM)?;
            if (0..NUMBER_MATH_FAMILIES).contains(&cur_fam) {
                self.mem[(p + 1) as usize].set_b16_s1(cur_fam as u16);
            }
            self.mem[p as usize].set_b16_s1(ORD_NOAD);
        } else {
            self.mem[p as usize]
                .set_b16_s1((ORD_NOAD as i32 + TeXEngineState::math_class(c)) as u16);
        }
        let fam = self.mem[(p + 1) as usize].b16_s1() as i32;
        self.mem[(p + 1) as usize].set_b16_s1((fam + (ch / 65536) * 256) as u16);
        self.mem[self.tail as usize].s1 = p;
        self.tail = p;
        Ok(())
    }

    /// Scans a delimiter value into the four b16 slots of `p`
    /// (`scan_delimiter`); `r` selects the `\delimiter`-class integer path.
    fn scan_delimiter(&mut self, p: i32, r: bool) -> Result<(), TeXMouthError> {
        if r {
            if self.cur_chr == 1 {
                self.cur_val1 = 0x4000_0000;
                self.scan_math_fam_int()?;
                self.cur_val1 += self.cur_val * 0x20_0000;
                self.scan_usv_num()?;
                self.cur_val += self.cur_val1;
            } else {
                self.scan_delimiter_int()?;
            }
        } else {
            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8
                    && self.cur_cmd != CommandCode::Relax as u8
                {
                    break;
                }
            }
            match CommandCode::from_u8(self.cur_cmd) {
                Some(CommandCode::Letter | CommandCode::OtherChar) => {
                    self.cur_val = self.del_code(self.cur_chr.max(0) as u32);
                }
                Some(CommandCode::DelimNum) => {
                    if self.cur_chr == 1 {
                        self.cur_val1 = 0x4000_0000;
                        self.scan_math_class_int()?;
                        self.scan_math_fam_int()?;
                        self.cur_val1 += self.cur_val * 0x2_0000;
                        self.scan_usv_num()?;
                        self.cur_val += self.cur_val1;
                    } else {
                        self.scan_delimiter_int()?;
                    }
                }
                _ => {
                    self.cur_val = -1;
                }
            }
        }
        if self.cur_val < 0 {
            self.report_error(
                "Missing delimiter (. inserted)",
                &[
                    "I was expecting to see something like `(' or `\\{' or",
                    "`\\}' here. If you typed, e.g., `{' instead of `\\{', you",
                    "should probably delete the `{' by typing `1' now, so that",
                    "braces don't get unbalanced. Otherwise just proceed.",
                    "Acceptable delimiters are characters whose \\delcode is",
                    "nonnegative, or you can use `\\delimiter <delimiter code>'.",
                ],
            );
            self.back_input()?;
            self.cur_val = 0;
        }
        if self.cur_val >= 0x4000_0000 {
            self.mem[p as usize].set_b16_s3(
                (((self.cur_val % 0x20_0000) / 0x1_0000) * 0x100
                    + (self.cur_val / 0x20_0000) % 0x100) as u16,
            );
            self.mem[p as usize].set_b16_s2((self.cur_val % 0x1_0000) as u16);
            self.mem[p as usize].set_b16_s1(0);
            self.mem[p as usize].set_b16_s0(0);
        } else {
            self.mem[p as usize].set_b16_s3(((self.cur_val / 0x10_0000) % 16) as u16);
            self.mem[p as usize].set_b16_s2(((self.cur_val / 0x1000) % 0x100) as u16);
            self.mem[p as usize].set_b16_s1(((self.cur_val / 0x100) % 16) as u16);
            self.mem[p as usize].set_b16_s0((self.cur_val % 0x100) as u16);
        }
        Ok(())
    }

    /// `\limits`/`\nolimits` on the previous noad (`math_limit_switch`).
    fn math_limit_switch(&mut self) -> Result<(), TeXMouthError> {
        if self.head != self.tail && self.mem[self.tail as usize].b16_s1() == OP_NOAD {
            self.mem[self.tail as usize].set_b16_s0(self.cur_chr as u16);
            return Ok(());
        }
        self.report_error(
            "Limit controls must follow a math operator",
            &["I'm ignoring this misplaced \\limits or \\nolimits command."],
        );
        Ok(())
    }

    /// Appends a radical noad (`math_radical`).
    fn math_radical(&mut self) -> Result<(), TeXMouthError> {
        let node = self.get_node(RADICAL_NOAD_SIZE)?;
        self.mem[self.tail as usize].s1 = node;
        self.tail = node;
        self.mem[node as usize].set_b16_s1(RADICAL_NOAD);
        self.mem[node as usize].set_b16_s0(NORMAL);
        for offset in 1..=3 {
            self.mem[(node + offset) as usize] = MemoryWord::default();
        }
        self.scan_delimiter(self.tail + 4, true)?;
        self.scan_math(self.tail + 1)
    }

    /// Appends an accent noad (`math_ac`, including `\Umathaccent`
    /// `fixed`/`bottom` modifiers).
    fn math_ac(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_cmd == CommandCode::Accent as u8 {
            self.report_error(
                "Please use \\mathaccent for accents in math mode",
                &[
                    "I'm changing \\accent to \\mathaccent here; wish me luck.",
                    "(Accents are not the same in formulas as they are in text.)",
                ],
            );
        }
        let node = self.get_node(ACCENT_NOAD_SIZE)?;
        self.mem[self.tail as usize].s1 = node;
        self.tail = node;
        self.mem[node as usize].set_b16_s1(ACCENT_NOAD);
        self.mem[node as usize].set_b16_s0(NORMAL);
        for offset in 1..=3 {
            self.mem[(node + offset) as usize] = MemoryWord::default();
        }
        self.mem[(self.tail + 4) as usize].s1 = MATH_CHAR;
        if self.cur_chr == 1 {
            if self.scan_keyword("fixed")? {
                self.mem[self.tail as usize].set_b16_s0(FIXED_ACC as u16);
            } else if self.scan_keyword("bottom")? {
                if self.scan_keyword("fixed")? {
                    self.mem[self.tail as usize].set_b16_s0((BOTTOM_ACC + 1) as u16);
                } else {
                    self.mem[self.tail as usize].set_b16_s0(BOTTOM_ACC as u16);
                }
            }
            self.scan_math_class_int()?;
            let mut c = TeXEngineState::set_class(self.cur_val);
            self.scan_math_fam_int()?;
            c += TeXEngineState::set_family(self.cur_val);
            self.scan_usv_num()?;
            self.cur_val += c;
        } else {
            self.scan_fifteen_bit_int()?;
            self.cur_val = TeXEngineState::set_class(self.cur_val / 4096)
                + TeXEngineState::set_family((self.cur_val % 4096) / 256)
                + (self.cur_val % 256);
        }
        self.mem[(self.tail + 4) as usize].set_b16_s0((self.cur_val % 65536) as u16);
        let cur_fam = self.int_par(INT_PAR_CUR_FAM)?;
        let fam = if TeXEngineState::math_class(self.cur_val) == 7
            && (0..NUMBER_MATH_FAMILIES).contains(&cur_fam)
        {
            cur_fam
        } else {
            TeXEngineState::math_fam(self.cur_val)
        };
        self.mem[(self.tail + 4) as usize]
            .set_b16_s1((fam + (TeXEngineState::math_char(self.cur_val) / 65536) * 256) as u16);
        self.scan_math(self.tail + 1)
    }

    /// Appends a `\mathchoice` node and enters the first choice group
    /// (`append_choices`).
    fn append_choices(&mut self) -> Result<(), TeXMouthError> {
        let node = self.new_choice()?;
        self.mem[self.tail as usize].s1 = node;
        self.tail = node;
        self.push_save_entry(SaveStackEntry::RawValue(0))?;
        self.push_math(MATH_CHOICE_GROUP)?;
        self.scan_left_brace()
    }

    /// Installs the mlist `p` into the current `\mathchoice` slot and opens
    /// the next group (`build_choices`).
    fn build_choices(&mut self) -> Result<(), TeXMouthError> {
        self.unsave()?;
        let p = self.fin_mlist(TEX_NULL)?;
        let slot = self.save_ptr - 1;
        let count = match self.save_stack.get(slot) {
            Some(SaveStackEntry::RawValue(value)) => *value,
            _ => {
                return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                    slot as i32,
                )))
            }
        };
        match count {
            0 => self.mem[(self.tail + 1) as usize].s0 = p,
            1 => self.mem[(self.tail + 1) as usize].s1 = p,
            2 => self.mem[(self.tail + 2) as usize].s0 = p,
            _ => {
                self.mem[(self.tail + 2) as usize].s1 = p;
                self.save_ptr -= 1;
                self.save_stack.pop();
                return Ok(());
            }
        }
        if let Some(entry) = self.save_stack.get_mut(slot) {
            *entry = SaveStackEntry::RawValue(count + 1);
        }
        self.push_math(MATH_CHOICE_GROUP)?;
        self.scan_left_brace()
    }

    /// Handles `_`/`^` script attachment (`sub_sup`).
    fn sub_sup(&mut self) -> Result<(), TeXMouthError> {
        let mut t = MATH_FIELD_EMPTY;
        let mut p = TEX_NULL;
        if self.tail != self.head {
            let tail_type = self.mem[self.tail as usize].b16_s1();
            if (ORD_NOAD..LEFT_NOAD).contains(&tail_type) {
                p = self.tail + 2 + i32::from(self.cur_cmd) - 7;
                t = self.mem[p as usize].s1;
            }
        }
        if p == TEX_NULL || t != MATH_FIELD_EMPTY {
            let node = self.new_noad()?;
            self.mem[self.tail as usize].s1 = node;
            self.tail = node;
            p = self.tail + 2 + i32::from(self.cur_cmd) - 7;
            if t != MATH_FIELD_EMPTY {
                if self.cur_cmd == CommandCode::SupMark as u8 {
                    self.report_error(
                        "Double superscript",
                        &["I treat `x^1^2' essentially like `x^1{}^2'."],
                    );
                } else {
                    self.report_error(
                        "Double subscript",
                        &["I treat `x_1_2' essentially like `x_1{}_2'."],
                    );
                }
            }
        }
        self.scan_math(p)
    }

    /// Handles `\over`-family fraction specs (`math_fraction`).
    fn math_fraction(&mut self) -> Result<(), TeXMouthError> {
        let c = self.cur_chr;
        if self.aux.s1 != TEX_NULL {
            if c >= DELIMITED_CODE {
                self.scan_delimiter(self.lig_trick(), false)?;
                self.scan_delimiter(self.lig_trick(), false)?;
            }
            if c % DELIMITED_CODE == ABOVE_CODE {
                self.scan_dimen(false, false, false)?;
            }
            self.report_error(
                "Ambiguous; you need another { and }",
                &[
                    "I'm ignoring this fraction specification, since I don't",
                    "know whether a construction like `x \\over y \\over z'",
                    "means `{x \\over y} \\over z' or `x \\over {y \\over z}'.",
                ],
            );
            return Ok(());
        }
        let aux = self.get_node(FRACTION_NOAD_SIZE)?;
        self.aux.s1 = aux;
        self.mem[aux as usize].set_b16_s1(FRACTION_NOAD);
        self.mem[aux as usize].set_b16_s0(NORMAL);
        self.mem[(aux + 2) as usize].s1 = SUB_MLIST;
        self.mem[(aux + 2) as usize].s0 = self.mem[self.head as usize].s1;
        self.mem[(aux + 3) as usize] = MemoryWord::default();
        for offset in 4..=5 {
            self.mem[(aux + offset) as usize] = MemoryWord::default();
        }
        self.mem[self.head as usize].s1 = TEX_NULL;
        self.tail = self.head;
        if c >= DELIMITED_CODE {
            self.scan_delimiter(aux + 4, false)?;
            self.scan_delimiter(aux + 5, false)?;
        }
        match c % DELIMITED_CODE {
            x if x == ABOVE_CODE => {
                self.scan_dimen(false, false, false)?;
                self.mem[(aux + 1) as usize].s1 = self.cur_val;
            }
            x if x == OVER_CODE => {
                self.mem[(aux + 1) as usize].s1 = DEFAULT_CODE;
            }
            _ => {
                self.mem[(aux + 1) as usize].s1 = 0;
            }
        }
        Ok(())
    }

    /// Handles `\left`/`\middle`/`\right` (`math_left_right`).
    fn math_left_right(&mut self) -> Result<(), TeXMouthError> {
        let t = self.cur_chr;
        if t != LEFT_NOAD as i32 && self.cur_group != MATH_LEFT_GROUP {
            if self.cur_group == MATH_SHIFT_GROUP {
                self.scan_delimiter(self.lig_trick(), false)?;
                if t == 1 {
                    self.report_error(
                        "Extra \\middle",
                        &["I'm ignoring a \\middle that had no matching \\left."],
                    );
                } else {
                    self.report_error(
                        "Extra \\right",
                        &["I'm ignoring a \\right that had no matching \\left."],
                    );
                }
            } else {
                self.off_save()?;
            }
            return Ok(());
        }
        let p = self.new_noad()?;
        self.mem[p as usize].set_b16_s1(t as u16);
        self.scan_delimiter(p + 1, false)?;
        if t == 1 {
            self.mem[p as usize].set_b16_s1(RIGHT_NOAD);
            self.mem[p as usize].set_b16_s0(1);
        }
        let q;
        if t == LEFT_NOAD as i32 {
            q = p;
        } else {
            q = self.fin_mlist(p)?;
            self.unsave()?;
        }
        if t != RIGHT_NOAD as i32 {
            self.push_math(MATH_LEFT_GROUP)?;
            self.mem[self.head as usize].s1 = q;
            self.tail = p;
            self.etex_aux = p;
        } else {
            let node = self.new_noad()?;
            self.mem[self.tail as usize].s1 = node;
            self.tail = node;
            self.mem[node as usize].set_b16_s1(INNER_NOAD);
            self.mem[(node + 1) as usize].s1 = SUB_MLIST;
            self.mem[(node + 1) as usize].s0 = q;
        }
        Ok(())
    }

    /// Copies a partial hlist into a reversed display prefix (`just_copy`).
    fn just_copy(&mut self, mut p: i32, mut h: i32, t: i32) -> Result<(), TeXMouthError> {
        while p != TEX_NULL {
            let mut words = 1;
            let r;
            let mut ligature_copied = false;
            if self.is_char_node(p) {
                r = self.get_avail()?;
            } else {
                match self.node_type(p)? {
                    x if x == HLIST_NODE || x == VLIST_NODE => {
                        r = self.get_node(BOX_NODE_SIZE)?;
                        // SyncTeX: preserve the source node's tag/line
                        // (`SYNCTEX_tag/line(r, BOX_NODE_SIZE)`).
                        self.mem[(r + 7) as usize] = self.mem[(p + 7) as usize];
                        self.mem[(r + 6) as usize] = self.mem[(p + 6) as usize];
                        self.mem[(r + 5) as usize] = self.mem[(p + 5) as usize];
                        words = 5;
                        self.mem[(r + 5) as usize].s1 = TEX_NULL;
                    }
                    x if x == RULE_NODE => {
                        r = self.get_node(RULE_NODE_SIZE)?;
                        words = RULE_NODE_SIZE;
                    }
                    x if x == LIGATURE_NODE => {
                        r = self.get_avail()?;
                        self.mem[r as usize] = self.mem[(p + 1) as usize];
                        ligature_copied = true;
                    }
                    x if x == KERN_NODE || x == MATH_NODE => {
                        words = MEDIUM_NODE_SIZE;
                        r = self.get_node(words)?;
                    }
                    x if x == GLUE_NODE => {
                        r = self.get_node(MEDIUM_NODE_SIZE)?;
                        // SyncTeX: preserve the source node's tag/line
                        // (`SYNCTEX_tag/line(r, MEDIUM_NODE_SIZE)`).
                        self.mem[(r + 2) as usize] = self.mem[(p + 2) as usize];
                        let spec = self.mem[(p + 1) as usize].s0;
                        self.add_glue_ref(spec)?;
                        self.mem[(r + 1) as usize].s0 = spec;
                        self.mem[(r + 1) as usize].s1 = TEX_NULL;
                    }
                    x if x == WHATSIT_NODE => match self.node_subtype(p)? {
                        OPEN_NODE => {
                            r = self.get_node(OPEN_NODE_SIZE)?;
                            words = OPEN_NODE_SIZE;
                        }
                        WRITE_NODE | SPECIAL_NODE => {
                            r = self.get_node(WRITE_NODE_SIZE)?;
                            let tokens = self.mem[(p + 1) as usize].s1;
                            if tokens != TEX_NULL {
                                self.mem[tokens as usize].s0 += 1;
                            }
                            words = WRITE_NODE_SIZE;
                        }
                        CLOSE_NODE | LANGUAGE_NODE => {
                            r = self.get_node(SMALL_NODE_SIZE)?;
                            words = SMALL_NODE_SIZE;
                        }
                        NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT => {
                            words = self.word_at(p, 4)?.b16_s3() as i32;
                            r = self.get_node(words)?;
                            let mut w = words;
                            while w > 0 {
                                w -= 1;
                                self.mem[(r + w) as usize] = self.mem[(p + w) as usize];
                            }
                        }
                        GLYPH_NODE => {
                            r = self.get_node(GLYPH_NODE_SIZE)?;
                            words = GLYPH_NODE_SIZE;
                        }
                        PIC_NODE | PDF_NODE => {
                            let path_len = self.word_at(p, 4)?.b16_s1() as i32;
                            words = PIC_NODE_SIZE + (path_len + 7) / 8;
                            r = self.get_node(words)?;
                        }
                        PDF_SAVE_POS_NODE => {
                            r = self.get_node(SMALL_NODE_SIZE)?;
                        }
                        _ => {
                            return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(p)));
                        }
                    },
                    _ => {
                        p = self.mem[p as usize].s1;
                        continue;
                    }
                }
            }
            if !ligature_copied {
                while words > 0 {
                    words -= 1;
                    self.mem[(r + words) as usize] = self.mem[(p + words) as usize];
                }
            }
            self.mem[h as usize].s1 = r;
            h = r;
            p = self.mem[p as usize].s1;
        }
        self.mem[h as usize].s1 = t;
        Ok(())
    }

    /// Reverses an hlist segment for TeX--XeT typesetting (`just_reverse`).
    fn just_reverse(&mut self, mut p: i32) -> Result<(), TeXMouthError> {
        let mut m = MIN_HALFWORD;
        let mut n = MIN_HALFWORD;
        let temp_head = self.temp_head();
        let mut q;
        if self.mem[temp_head as usize].s1 == TEX_NULL {
            self.just_copy(self.mem[p as usize].s1, temp_head, TEX_NULL)?;
            q = self.mem[temp_head as usize].s1;
        } else {
            q = self.mem[p as usize].s1;
            self.mem[p as usize].s1 = TEX_NULL;
            self.flush_node_list(self.mem[temp_head as usize].s1)?;
        }
        let t = self.new_edge(self.cur_dir, 0)?;
        let mut l = t;
        self.cur_dir = 1 - self.cur_dir;
        'outer: while q != TEX_NULL {
            if self.is_char_node(q) {
                loop {
                    p = q;
                    q = self.mem[p as usize].s1;
                    self.mem[p as usize].s1 = l;
                    l = p;
                    if !self.is_char_node(q) {
                        break;
                    }
                }
                if q == TEX_NULL {
                    break;
                }
            }
            p = q;
            q = self.mem[p as usize].s1;
            if self.node_type(p)? == MATH_NODE {
                let subtype = self.mem[p as usize].b16_s0() as i32;
                if subtype % 2 == 1 {
                    if self.word_at(self.lr_ptr, 0)?.s0 != L_CODE * (subtype / L_CODE) + 3 {
                        self.set_node_type(p, KERN_NODE)?;
                        self.lr_problems += 1;
                    } else {
                        let temp = self.lr_ptr;
                        self.lr_ptr = self.mem[temp as usize].s1;
                        self.free_avail(temp)?;
                        if n > MIN_HALFWORD {
                            n -= 1;
                            self.mem[p as usize].set_b16_s0((subtype - 1) as u16);
                        } else if m > MIN_HALFWORD {
                            m -= 1;
                            self.set_node_type(p, KERN_NODE)?;
                        } else {
                            self.mem[(t + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
                            self.mem[t as usize].s1 = q;
                            self.free_node(p, MEDIUM_NODE_SIZE)?;
                            break 'outer;
                        }
                    }
                } else {
                    let temp = self.get_avail()?;
                    self.mem[temp as usize].s0 = L_CODE * (subtype / L_CODE) + 3;
                    self.mem[temp as usize].s1 = self.lr_ptr;
                    self.lr_ptr = temp;
                    if n > MIN_HALFWORD || (subtype / R_CODE) != self.cur_dir {
                        n += 1;
                        self.mem[p as usize].set_b16_s0((subtype + 1) as u16);
                    } else {
                        self.set_node_type(p, KERN_NODE)?;
                        m += 1;
                    }
                }
            }
            self.mem[p as usize].s1 = l;
            l = p;
        }
        self.mem[temp_head as usize].s1 = l;
        Ok(())
    }

    /// Enters math mode (`init_math`): the `$$` display path measures the
    /// so-far line width and installs display parameters; the inline path
    /// simply pushes a math group.
    fn init_math(&mut self) -> Result<(), TeXMouthError> {
        self.get_token()?;
        if self.cur_cmd == CommandCode::MathShift as u8 && self.mode > 0 {
            let mut j = TEX_NULL;
            let mut w = -MAX_HALFWORD;
            let x;
            if self.head == self.tail {
                self.pop_nest()?;
                if self.etex_aux == TEX_NULL {
                    x = 0;
                } else if self.mem[self.etex_aux as usize].s0 >= R_CODE {
                    x = -1;
                } else {
                    x = 1;
                }
            } else {
                let mut v;
                self.line_break(true)?;
                j = if self.glue_par(GLUE_PAR_RIGHT_SKIP)? == 0 {
                    self.new_kern(0)?
                } else {
                    self.new_param_glue(GLUE_PAR_RIGHT_SKIP)?
                };
                let p = if self.glue_par(GLUE_PAR_LEFT_SKIP)? == 0 {
                    self.new_kern(0)?
                } else {
                    self.new_param_glue(GLUE_PAR_LEFT_SKIP)?
                };
                self.mem[p as usize].s1 = j;
                j = self.new_null_box()?;
                self.mem[(j + 1) as usize].s1 = self.mem[(self.just_box + 1) as usize].s1;
                self.mem[(j + 4) as usize].s1 = self.mem[(self.just_box + 4) as usize].s1;
                self.mem[(j + 5) as usize].s1 = p;
                let jb5 = self.mem[(self.just_box + 5) as usize];
                let jb6 = self.mem[(self.just_box + 6) as usize];
                self.mem[(j + 5) as usize].set_b16_s0(jb5.b16_s0());
                self.mem[(j + 5) as usize].set_b16_s1(jb5.b16_s1());
                self.mem[(j + 6) as usize].set_gr(jb6.gr());
                v = self.mem[(self.just_box + 4) as usize].s1;
                if self.etex_aux == TEX_NULL {
                    x = 0;
                } else if self.mem[self.etex_aux as usize].s0 >= R_CODE {
                    x = -1;
                } else {
                    x = 1;
                }
                let mut p;
                let temp_head = self.temp_head();
                if x >= 0 {
                    p = self.mem[(self.just_box + 5) as usize].s1;
                    self.mem[temp_head as usize].s1 = TEX_NULL;
                } else {
                    v = -v - self.mem[(self.just_box + 1) as usize].s1;
                    p = self.new_math(0, BEGIN_L_CODE)?;
                    self.mem[temp_head as usize].s1 = p;
                    let end = self.new_math(0, END_L_CODE)?;
                    self.just_copy(self.mem[(self.just_box + 5) as usize].s1, p, end)?;
                    self.cur_dir = RIGHT_TO_LEFT;
                }
                let quad = self
                    .font_char_metric(self.cur_font(), QUAD_CODE as u32)
                    .width;
                v += 2 * quad;
                if self.int_par(INT_PAR_TEXXET)? > 0 {
                    let temp = self.get_avail()?;
                    self.mem[temp as usize].s0 = BEFORE;
                    self.mem[temp as usize].s1 = self.lr_ptr;
                    self.lr_ptr = temp;
                }
                'measure: while p != TEX_NULL {
                    let d;
                    'reswitch: loop {
                        if self.is_char_node(p) {
                            let f = self.mem[p as usize].b16_s1() as i32;
                            let ch = self.mem[p as usize].b16_s0() as i32;
                            let eff = self.effective_char(f, ch);
                            d = self.font_char_metric(f, eff.max(0) as u32).width;
                            break 'reswitch;
                        }
                        match self.node_type(p)? {
                            t if t == HLIST_NODE || t == VLIST_NODE || t == RULE_NODE => {
                                d = self.mem[(p + 1) as usize].s1;
                                break 'reswitch;
                            }
                            t if t == LIGATURE_NODE => {
                                let garbage = self.lig_trick();
                                self.mem[garbage as usize] = self.mem[(p + 1) as usize];
                                self.mem[garbage as usize].s1 = self.mem[p as usize].s1;
                                p = garbage;
                                self.xtx_ligature_present = true;
                                continue 'reswitch;
                            }
                            t if t == KERN_NODE => {
                                d = self.mem[(p + 1) as usize].s1;
                                break 'reswitch;
                            }
                            t if t == MARGIN_KERN_NODE => {
                                d = self.mem[(p + 1) as usize].s1;
                                break 'reswitch;
                            }
                            t if t == MATH_NODE => {
                                d = self.mem[(p + 1) as usize].s1;
                                if self.int_par(INT_PAR_TEXXET)? > 0 {
                                    let subtype = self.mem[p as usize].b16_s0() as i32;
                                    if subtype % 2 == 1 {
                                        let expected = L_CODE * (subtype / L_CODE) + 3;
                                        if self.word_at(self.lr_ptr, 0)?.s0 == expected {
                                            let temp = self.lr_ptr;
                                            self.lr_ptr = self.mem[temp as usize].s1;
                                            self.free_avail(temp)?;
                                        } else if subtype > L_CODE {
                                            w = MAX_HALFWORD;
                                            break 'measure;
                                        }
                                    } else {
                                        let temp = self.get_avail()?;
                                        self.mem[temp as usize].s0 =
                                            L_CODE * (subtype / L_CODE) + 3;
                                        self.mem[temp as usize].s1 = self.lr_ptr;
                                        self.lr_ptr = temp;
                                        if (subtype / R_CODE) != self.cur_dir {
                                            self.just_reverse(p)?;
                                            p = self.temp_head();
                                        }
                                    }
                                } else if self.mem[p as usize].b16_s0() as i32 >= L_CODE {
                                    w = MAX_HALFWORD;
                                    break 'measure;
                                }
                                break 'reswitch;
                            }
                            t if t == EDGE_NODE => {
                                d = self.mem[(p + 1) as usize].s1;
                                self.cur_dir = self.mem[p as usize].b16_s0() as i32;
                                break 'reswitch;
                            }
                            t if t == GLUE_NODE => {
                                let spec = self.mem[(p + 1) as usize].s0;
                                d = self.mem[(spec + 1) as usize].s1;
                                let sign = self.mem[(self.just_box + 5) as usize].b16_s1();
                                let order = self.mem[(self.just_box + 5) as usize].b16_s0();
                                if sign == STRETCHING {
                                    if order == self.mem[spec as usize].b16_s1()
                                        && self.mem[(spec + 2) as usize].s1 != 0
                                    {
                                        v = MAX_HALFWORD;
                                    }
                                } else if sign == SHRINKING
                                    && order == self.mem[spec as usize].b16_s0()
                                    && self.mem[(spec + 3) as usize].s1 != 0
                                {
                                    v = MAX_HALFWORD;
                                }
                                if self.mem[p as usize].b16_s0() as i32 >= A_LEADERS {
                                    // Leader glue terminates the measure loop.
                                    if v < MAX_HALFWORD {
                                        w = v + d;
                                    } else {
                                        w = MAX_HALFWORD;
                                        break 'measure;
                                    }
                                    p = self.mem[p as usize].s1;
                                    continue 'measure;
                                }
                                break 'reswitch;
                            }
                            t if t == WHATSIT_NODE => {
                                let subtype = self.mem[p as usize].b16_s0();
                                if subtype == NATIVE_WORD_NODE
                                    || subtype == NATIVE_WORD_NODE_AT
                                    || subtype == GLYPH_NODE
                                    || subtype == PIC_NODE
                                    || subtype == PDF_NODE
                                {
                                    d = self.mem[(p + 1) as usize].s1;
                                    break 'reswitch;
                                }
                                d = 0;
                                break 'reswitch;
                            }
                            _ => {
                                d = 0;
                                break 'reswitch;
                            }
                        }
                    }
                    if v < MAX_HALFWORD {
                        v += d;
                        w = v;
                    } else {
                        w = MAX_HALFWORD;
                        break 'measure;
                    }
                    p = self.mem[p as usize].s1;
                }
                if self.int_par(INT_PAR_TEXXET)? > 0 {
                    while self.lr_ptr != TEX_NULL {
                        let temp = self.lr_ptr;
                        self.lr_ptr = self.mem[temp as usize].s1;
                        self.free_avail(temp)?;
                    }
                    if self.lr_problems != 0 {
                        w = MAX_HALFWORD;
                        self.lr_problems = 0;
                    }
                }
                self.cur_dir = LEFT_TO_RIGHT;
                self.flush_node_list(self.mem[self.temp_head() as usize].s1)?;
            }
            let l;
            let s;
            if self.local(LOCAL_PAR_SHAPE)? == TEX_NULL {
                let hang_indent = self.dimen_par(DIMEN_PAR_HANG_INDENT)?;
                let hang_after = self.int_par(INT_PAR_HANG_AFTER)?;
                if hang_indent != 0
                    && ((hang_after >= 0 && self.prev_graf + 2 > hang_after)
                        || self.prev_graf + 1 < -hang_after)
                {
                    l = self.dimen_par(DIMEN_PAR_HSIZE)? - hang_indent.abs();
                    s = if hang_indent > 0 { hang_indent } else { 0 };
                } else {
                    l = self.dimen_par(DIMEN_PAR_HSIZE)?;
                    s = 0;
                }
            } else {
                let par_shape = self.local(LOCAL_PAR_SHAPE)?;
                let n = self.mem[par_shape as usize].s0;
                let slot = if self.prev_graf + 2 >= n {
                    par_shape + 2 * n
                } else {
                    par_shape + 2 * (self.prev_graf + 2)
                };
                s = self.mem[(slot - 1) as usize].s1;
                l = self.mem[slot as usize].s1;
            }
            self.push_math(MATH_SHIFT_GROUP)?;
            self.mode = TeXMode::Mmode as i16;
            self.eq_word_define(INT_PAR_BASE + INT_PAR_CUR_FAM, -1)?;
            self.eq_word_define(DIMEN_PAR_BASE + DIMEN_PAR_PRE_DISPLAY_SIZE, w)?;
            self.etex_aux = j;
            self.eq_word_define(INT_PAR_BASE + INT_PAR_PRE_DISPLAY_DIRECTION, x)?;
            self.eq_word_define(DIMEN_PAR_BASE + DIMEN_PAR_DISPLAY_WIDTH, l)?;
            self.eq_word_define(DIMEN_PAR_BASE + DIMEN_PAR_DISPLAY_INDENT, s)?;
            if self.semantic_pagination_enabled {
                self.tt_insert_special("tdux:cs dmath")?;
            }
            let every_display = self.local(LOCAL_EVERY_DISPLAY)?;
            if every_display != TEX_NULL {
                self.begin_token_list(every_display, EVERY_DISPLAY_TEXT)?;
            }
            if self.nest_ptr == 1 {
                self.build_page()?;
            }
        } else {
            self.back_input()?;
            self.push_math(MATH_SHIFT_GROUP)?;
            self.eq_word_define(INT_PAR_BASE + INT_PAR_CUR_FAM, -1)?;
            if self.insert_src_special_every_math {
                self.insert_src_special()?;
            }
            if self.semantic_pagination_enabled {
                self.tt_insert_special("tdux:cs math")?;
            }
            let every_math = self.local(LOCAL_EVERY_MATH)?;
            if every_math != TEX_NULL {
                self.begin_token_list(every_math, EVERY_MATH_TEXT)?;
            }
        }
        Ok(())
    }

    /// Enters an `\eqno`/`\leqno` equation-number group (`start_eq_no`).
    fn start_eq_no(&mut self) -> Result<(), TeXMouthError> {
        self.push_save_entry(SaveStackEntry::RawValue(self.cur_chr))?;
        self.push_math(MATH_SHIFT_GROUP)?;
        self.eq_word_define(INT_PAR_BASE + INT_PAR_CUR_FAM, -1)?;
        if self.insert_src_special_every_math {
            self.insert_src_special()?;
        }
        let every_math = self.local(LOCAL_EVERY_MATH)?;
        if every_math != TEX_NULL {
            self.begin_token_list(every_math, EVERY_MATH_TEXT)?;
        }
        Ok(())
    }

    /// Appends a display line box to the enclosing vertical list, with
    /// TeX--XeT direction handling (`app_display`).
    fn app_display(&mut self, mut j: i32, mut b: i32, mut d: i32) -> Result<(), TeXMouthError> {
        let mut s = self.dimen_par(DIMEN_PAR_DISPLAY_INDENT)?;
        let x = self.int_par(INT_PAR_PRE_DISPLAY_DIRECTION)?;
        if x == 0 {
            self.mem[(b + 4) as usize].s1 = s + d;
        } else {
            let z = self.dimen_par(DIMEN_PAR_DISPLAY_WIDTH)?;
            let mut p = b;
            let mut e;
            if x > 0 {
                e = z - d - self.mem[(p + 1) as usize].s1;
            } else {
                e = d;
                d = z - e - self.mem[(p + 1) as usize].s1;
            }
            if j != TEX_NULL {
                b = self.copy_node_list(j)?;
                self.mem[(b + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                self.mem[(b + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
                s -= self.mem[(b + 4) as usize].s1;
                d += s;
                e += self.mem[(b + 1) as usize].s1 - z - s;
            }
            let mut q;
            if self.mem[p as usize].b16_s0() as i32 == DLIST_CODE as i32 {
                q = p;
            } else {
                let mut r = self.mem[(p + 5) as usize].s1;
                self.free_node(p, BOX_NODE_SIZE)?;
                if r == TEX_NULL {
                    return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(p)));
                }
                if x > 0 {
                    p = r;
                    loop {
                        q = r;
                        r = self.mem[r as usize].s1;
                        if r == TEX_NULL {
                            break;
                        }
                    }
                } else {
                    p = TEX_NULL;
                    q = r;
                    loop {
                        let t = self.mem[r as usize].s1;
                        self.mem[r as usize].s1 = p;
                        p = r;
                        r = t;
                        if r == TEX_NULL {
                            break;
                        }
                    }
                }
            }
            let r;
            let t;
            if j == TEX_NULL {
                r = self.new_kern(0)?;
                t = self.new_kern(0)?;
            } else {
                r = self.mem[(b + 5) as usize].s1;
                t = self.mem[r as usize].s1;
            }
            let mut u = self.new_math(0, END_M_CODE)?;
            if self.node_type(t)? == GLUE_NODE {
                j = self.new_skip_param(GLUE_PAR_RIGHT_SKIP)?;
                self.mem[q as usize].s1 = j;
                self.mem[j as usize].s1 = u;
                j = self.mem[(t + 1) as usize].s0;
                let spec = j;
                let temp = self.temp_ptr;
                let so1 = self.mem[spec as usize].b16_s1();
                let so0 = self.mem[spec as usize].b16_s0();
                self.mem[temp as usize].set_b16_s1(so1);
                self.mem[temp as usize].set_b16_s0(so0);
                self.mem[(temp + 1) as usize].s1 = e - self.mem[(spec + 1) as usize].s1;
                self.mem[(temp + 2) as usize].s1 = -self.mem[(spec + 2) as usize].s1;
                self.mem[(temp + 3) as usize].s1 = -self.mem[(spec + 3) as usize].s1;
                self.mem[u as usize].s1 = t;
            } else {
                self.mem[(t + 1) as usize].s1 = e;
                self.mem[t as usize].s1 = u;
                self.mem[q as usize].s1 = t;
            }
            u = self.new_math(0, BEGIN_M_CODE)?;
            if self.node_type(r)? == GLUE_NODE {
                j = self.new_skip_param(GLUE_PAR_LEFT_SKIP)?;
                self.mem[u as usize].s1 = j;
                self.mem[j as usize].s1 = p;
                j = self.mem[(r + 1) as usize].s0;
                let spec = j;
                let temp = self.temp_ptr;
                let so1 = self.mem[spec as usize].b16_s1();
                let so0 = self.mem[spec as usize].b16_s0();
                self.mem[temp as usize].set_b16_s1(so1);
                self.mem[temp as usize].set_b16_s0(so0);
                self.mem[(temp + 1) as usize].s1 = d - self.mem[(spec + 1) as usize].s1;
                self.mem[(temp + 2) as usize].s1 = -self.mem[(spec + 2) as usize].s1;
                self.mem[(temp + 3) as usize].s1 = -self.mem[(spec + 3) as usize].s1;
                self.mem[r as usize].s1 = u;
            } else {
                self.mem[(r + 1) as usize].s1 = d;
                self.mem[r as usize].s1 = p;
                self.mem[u as usize].s1 = r;
                if j == TEX_NULL {
                    b = self.hpack(u, 0, ADDITIONAL)?;
                    self.mem[(b + 4) as usize].s1 = s;
                } else {
                    self.mem[(b + 5) as usize].s1 = u;
                }
            }
        }
        self.append_to_vlist(b)?;
        Ok(())
    }

    /// Finishes a math formula (`after_math`): inline math appends the
    /// converted list between `\mathsurround` kerns; display math packages
    /// the formula, equation number, and display skips.
    fn after_math(&mut self) -> Result<(), TeXMouthError> {
        let mut l = false;
        let mut danger = false;
        let mut j = TEX_NULL;
        if self.mode == TeXMode::Mmode as i16 {
            j = self.etex_aux;
        }
        if self.math_fonts_insufficient(2, TOTAL_MATHSY_PARAMS) {
            self.report_error(
                "Math formula deleted: Insufficient symbol fonts",
                &[
                    "Sorry, but I can't typeset math unless \\textfont 2",
                    "and \\scriptfont 2 and \\scriptscriptfont 2 have all",
                    "the \\fontdimen values needed in math symbol fonts.",
                ],
            );
            self.flush_math()?;
            danger = true;
        } else if self.math_fonts_insufficient(3, TOTAL_MATHEX_PARAMS) {
            self.report_error(
                "Math formula deleted: Insufficient extension fonts",
                &[
                    "Sorry, but I can't typeset math unless \\textfont 3",
                    "and \\scriptfont 3 and \\scriptscriptfont 3 have all",
                    "the \\fontdimen values needed in math extension fonts.",
                ],
            );
            self.flush_math()?;
            danger = true;
        }
        let mut m = self.mode;
        let mut p = self.fin_mlist(TEX_NULL)?;
        let mut a = TEX_NULL;
        if self.mode == -m {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::MathShift as u8 {
                self.back_input()?;
                self.report_error(
                    "Display math should end with $$",
                    &[
                        "The `$' that I just saw supposedly matches a previous `$$'.",
                        "So I shall assume that you typed `$$' both times.",
                    ],
                );
            }
            self.cur_mlist = p;
            self.cur_style = TEXT_STYLE;
            self.mlist_penalties = false;
            self.mlist_to_hlist()?;
            a = self.hpack(self.mem[self.temp_head() as usize].s1, 0, ADDITIONAL)?;
            self.mem[a as usize].set_b16_s0(DLIST_CODE);
            self.unsave()?;
            self.save_ptr -= 1;
            if let Some(SaveStackEntry::RawValue(1)) = self.save_stack.get(self.save_ptr) {
                l = true;
            }
            danger = false;
            if self.mode == TeXMode::Mmode as i16 {
                j = self.etex_aux;
            }
            if self.math_fonts_insufficient(2, TOTAL_MATHSY_PARAMS) {
                self.report_error(
                    "Math formula deleted: Insufficient symbol fonts",
                    &[
                        "Sorry, but I can't typeset math unless \\textfont 2",
                        "and \\scriptfont 2 and \\scriptscriptfont 2 have all",
                        "the \\fontdimen values needed in math symbol fonts.",
                    ],
                );
                self.flush_math()?;
                danger = true;
            } else if self.math_fonts_insufficient(3, TOTAL_MATHEX_PARAMS) {
                self.report_error(
                    "Math formula deleted: Insufficient extension fonts",
                    &[
                        "Sorry, but I can't typeset math unless \\textfont 3",
                        "and \\scriptfont 3 and \\scriptscriptfont 3 have all",
                        "the \\fontdimen values needed in math extension fonts.",
                    ],
                );
                self.flush_math()?;
                danger = true;
            }
            m = self.mode;
            p = self.fin_mlist(TEX_NULL)?;
        }
        if m < 0 {
            let before = self.new_math(self.dimen_par(DIMEN_PAR_MATH_SURROUND)?, BEFORE)?;
            self.mem[self.tail as usize].s1 = before;
            self.tail = before;
            self.cur_mlist = p;
            self.cur_style = TEXT_STYLE;
            self.mlist_penalties = self.mode > 0;
            self.mlist_to_hlist()?;
            self.mem[self.tail as usize].s1 = self.mem[self.temp_head() as usize].s1;
            while self.mem[self.tail as usize].s1 != TEX_NULL {
                self.tail = self.mem[self.tail as usize].s1;
            }
            let after = self.new_math(self.dimen_par(DIMEN_PAR_MATH_SURROUND)?, AFTER)?;
            self.mem[self.tail as usize].s1 = after;
            self.tail = after;
            self.aux.s0 = 1000;
            if self.semantic_pagination_enabled {
                self.tt_insert_special("tdux:ce math")?;
            }
            self.unsave()?;
        } else {
            if a == TEX_NULL {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::MathShift as u8 {
                    self.back_input()?;
                    self.report_error(
                        "Display math should end with $$",
                        &[
                            "The `$' that I just saw supposedly matches a previous `$$'.",
                            "So I shall assume that you typed `$$' both times.",
                        ],
                    );
                }
            }
            self.cur_mlist = p;
            self.cur_style = DISPLAY_STYLE;
            self.mlist_penalties = false;
            self.mlist_to_hlist()?;
            p = self.mem[self.temp_head() as usize].s1;
            self.adjust_tail = self.adjust_head();
            self.pre_adjust_tail = self.pre_adjust_head();
            let mut b = self.hpack(p, 0, ADDITIONAL)?;
            p = self.mem[(b + 5) as usize].s1;
            let t = self.adjust_tail;
            self.adjust_tail = TEX_NULL;
            let pre_t = self.pre_adjust_tail;
            self.pre_adjust_tail = TEX_NULL;
            let mut w = self.mem[(b + 1) as usize].s1;
            let z = self.dimen_par(DIMEN_PAR_DISPLAY_WIDTH)?;
            let s = self.dimen_par(DIMEN_PAR_DISPLAY_INDENT)?;
            let mut s = s;
            if self.int_par(INT_PAR_PRE_DISPLAY_DIRECTION)? < 0 {
                s = -s - z;
            }
            let mut e = 0;
            let mut q = 0;
            if !(a == TEX_NULL || danger) {
                e = self.mem[(a + 1) as usize].s1;
                q = e + self.math_quad(TEXT_SIZE)?;
            }
            if w + q > z {
                if e != 0
                    && (w - self.total_shrink[NORMAL as usize] + q <= z
                        || self.total_shrink[FIL as usize] != 0
                        || self.total_shrink[FILL as usize] != 0
                        || self.total_shrink[FILLL as usize] != 0)
                {
                    self.free_node(b, BOX_NODE_SIZE)?;
                    b = self.hpack(p, z - q, EXACTLY)?;
                } else {
                    e = 0;
                    if w > z {
                        self.free_node(b, BOX_NODE_SIZE)?;
                        b = self.hpack(p, z, EXACTLY)?;
                    }
                }
                w = self.mem[(b + 1) as usize].s1;
            }
            self.mem[b as usize].set_b16_s0(DLIST_CODE);
            let mut d = half(z - w);
            if e > 0 && d < 2 * e {
                d = half(z - w - e);
                if p != TEX_NULL && !self.is_char_node(p) && self.node_type(p)? == GLUE_NODE {
                    d = 0;
                }
            }
            let pen = self.new_penalty(self.int_par(INT_PAR_PRE_DISPLAY_PENALTY)?)?;
            self.mem[self.tail as usize].s1 = pen;
            self.tail = pen;
            let (g1, mut g2);
            if d + s <= self.dimen_par(DIMEN_PAR_PRE_DISPLAY_SIZE)? || l {
                g1 = GLUE_PAR_ABOVE_DISPLAY_SKIP;
                g2 = GLUE_PAR_BELOW_DISPLAY_SKIP;
            } else {
                g1 = GLUE_PAR_ABOVE_DISPLAY_SHORT_SKIP;
                g2 = GLUE_PAR_BELOW_DISPLAY_SHORT_SKIP;
            }
            if l && e == 0 {
                self.app_display(j, a, 0)?;
                let pen = self.new_penalty(INF_PENALTY)?;
                self.mem[self.tail as usize].s1 = pen;
                self.tail = pen;
            } else {
                let skip = self.new_param_glue(g1)?;
                self.mem[self.tail as usize].s1 = skip;
                self.tail = skip;
            }
            if e != 0 {
                let r = self.new_kern(z - w - e - d)?;
                if l {
                    self.mem[a as usize].s1 = r;
                    self.mem[r as usize].s1 = b;
                    b = a;
                    d = 0;
                } else {
                    self.mem[b as usize].s1 = r;
                    self.mem[r as usize].s1 = a;
                }
                b = self.hpack(b, 0, ADDITIONAL)?;
            }
            self.app_display(j, b, d)?;
            if a != TEX_NULL && e == 0 && !l {
                let pen = self.new_penalty(INF_PENALTY)?;
                self.mem[self.tail as usize].s1 = pen;
                self.tail = pen;
                self.app_display(j, a, z - self.mem[(a + 1) as usize].s1)?;
                g2 = 0;
            }
            if t != self.adjust_head() {
                self.mem[self.tail as usize].s1 = self.mem[self.adjust_head() as usize].s1;
                self.tail = t;
            }
            if pre_t != self.pre_adjust_head() {
                self.mem[self.tail as usize].s1 = self.mem[self.pre_adjust_head() as usize].s1;
                self.tail = pre_t;
            }
            let pen = self.new_penalty(self.int_par(INT_PAR_POST_DISPLAY_PENALTY)?)?;
            self.mem[self.tail as usize].s1 = pen;
            self.tail = pen;
            if g2 > 0 {
                let skip = self.new_param_glue(g2)?;
                self.mem[self.tail as usize].s1 = skip;
                self.tail = skip;
            }
            self.flush_node_list(j)?;
            if self.semantic_pagination_enabled {
                self.tt_insert_special("tdux:ce dmath")?;
            }
            self.resume_after_display()?;
        }
        Ok(())
    }

    /// Returns true when the given math font family lacks the parameter
    /// set TeX requires (`font_params`/`isOpenTypeMathFont` check).
    fn math_fonts_insufficient(&self, fam: i32, total: usize) -> bool {
        for offset in [TEXT_SIZE, SCRIPT_SIZE, SCRIPT_SCRIPT_SIZE] {
            let f = self.math_font(fam + offset);
            if self.font_params_count(f) < total && !self.is_ot_math_font(f) {
                return true;
            }
        }
        false
    }

    /// One math em of `size_code` (`math_quad`): `font_info[6 +
    /// param_base]` for TFM fonts, `get_native_mathsy_param(f, 6)` for
    /// OpenType MATH fonts.
    fn math_quad(&self, size_code: i32) -> Result<i32, TeXMouthError> {
        Ok(self.mathsy_param(self.math_font(2 + size_code), 6))
    }

    /// Dispatches the `MMODE` command arm selected by `dispatch_current_command`.
    fn math_list_command(&mut self) -> Result<(), TeXMouthError> {
        match CommandCode::from_u8(self.cur_cmd) {
            Some(CommandCode::Letter | CommandCode::OtherChar | CommandCode::CharGiven) => {
                let c = self.math_code(self.cur_chr.max(0) as u32);
                self.set_math_char(c)
            }
            Some(CommandCode::CharNum) => {
                self.scan_char_num()?;
                self.cur_chr = self.cur_val;
                let c = self.math_code(self.cur_chr.max(0) as u32);
                self.set_math_char(c)
            }
            Some(CommandCode::MathCharNum) => {
                if self.cur_chr == 2 {
                    self.scan_math_class_int()?;
                    let mut t = TeXEngineState::set_class(self.cur_val);
                    self.scan_math_fam_int()?;
                    t += TeXEngineState::set_family(self.cur_val);
                    self.scan_usv_num()?;
                    self.set_math_char(t + self.cur_val)
                } else if self.cur_chr == 1 {
                    self.scan_xetex_math_char_int()?;
                    self.set_math_char(self.cur_val)
                } else {
                    self.scan_fifteen_bit_int()?;
                    let c = TeXEngineState::set_class(self.cur_val / 4096)
                        + TeXEngineState::set_family((self.cur_val % 4096) / 256)
                        + (self.cur_val % 256);
                    self.set_math_char(c)
                }
            }
            Some(CommandCode::MathGiven) => {
                let c = TeXEngineState::set_class(self.cur_chr / 4096)
                    + TeXEngineState::set_family((self.cur_chr % 4096) / 256)
                    + (self.cur_chr % 256);
                self.set_math_char(c)
            }
            Some(CommandCode::XetexMathGiven) => self.set_math_char(self.cur_chr),
            Some(CommandCode::DelimNum) => {
                if self.cur_chr == 1 {
                    self.scan_math_class_int()?;
                    let mut t = TeXEngineState::set_class(self.cur_val);
                    self.scan_math_fam_int()?;
                    t += TeXEngineState::set_family(self.cur_val);
                    self.scan_usv_num()?;
                    self.set_math_char(t + self.cur_val)
                } else {
                    self.scan_delimiter_int()?;
                    let dv = self.cur_val / 4096;
                    let c = TeXEngineState::set_class(dv / 4096)
                        + TeXEngineState::set_family((dv % 4096) / 256)
                        + (dv % 256);
                    self.set_math_char(c)
                }
            }
            Some(CommandCode::MathComp) => {
                let node = self.new_noad()?;
                self.mem[self.tail as usize].s1 = node;
                self.tail = node;
                self.mem[node as usize].set_b16_s1(self.cur_chr as u16);
                self.scan_math(self.tail + 1)
            }
            Some(CommandCode::LimitSwitch) => self.math_limit_switch(),
            Some(CommandCode::Radical) => self.math_radical(),
            Some(CommandCode::Accent | CommandCode::MathAccent) => self.math_ac(),
            Some(CommandCode::Vcenter) => {
                self.scan_spec(VCENTER_GROUP, false)?;
                self.normal_paragraph()?;
                self.push_nest()?;
                self.mode = -(TeXMode::Vmode as i16);
                self.aux.s1 = IGNORE_DEPTH;
                if self.insert_src_special_every_vbox {
                    self.insert_src_special()?;
                }
                let every_vbox = self.local(LOCAL_EVERY_VBOX)?;
                if every_vbox != TEX_NULL {
                    self.begin_token_list(every_vbox, EVERY_VBOX_TEXT)?;
                }
                Ok(())
            }
            Some(CommandCode::MathStyle) => {
                let node = self.new_style(self.cur_chr)?;
                self.mem[self.tail as usize].s1 = node;
                self.tail = node;
                Ok(())
            }
            Some(CommandCode::NonScript) => {
                let glue = self.new_glue(0)?;
                self.mem[self.tail as usize].s1 = glue;
                self.tail = glue;
                self.mem[glue as usize].set_b16_s0(COND_MATH_GLUE);
                Ok(())
            }
            Some(CommandCode::MathChoice) => self.append_choices(),
            Some(CommandCode::SubMark | CommandCode::SupMark) => self.sub_sup(),
            Some(CommandCode::Above) => self.math_fraction(),
            Some(CommandCode::LeftRight) => self.math_left_right(),
            _ => Err(TeXMouthError::UnsupportedPrimitive("math list command")),
        }
    }

    /// Converts the collected math list to a horizontal list
    /// (`mlist_to_hlist`); the conversion engine is WP-13b-ii.
    /// Returns a `Confusion`-equivalent internal error (`confusion`),
    /// emitting the C `confusion` diagnostic through the print machinery.
    pub(crate) fn confusion(&mut self, context: &'static str) -> TeXMouthError {
        self.print_confusion(context);
        TeXMouthError::UnsupportedPrimitive(match context {
            "mlist1" => "internal inconsistency: mlist1",
            "mlist2" => "internal inconsistency: mlist2",
            "mlist3" => "internal inconsistency: mlist3",
            "mlist4" => "internal inconsistency: mlist4",
            "vcenter" => "internal inconsistency: vcenter",
            "LR2" => "internal inconsistency: LR2",
            "LR3" => "internal inconsistency: LR3",
            "vlistout" => "internal inconsistency: vlistout",
            "ext4" => "internal inconsistency: ext4",
            "dvi length exceeds 0x7FFFFFFF" => "dvi length exceeds 0x7FFFFFFF",
            _ => "internal inconsistency",
        })
    }

    /// Symbol-font parameter accessor (`math_x_height`).
    fn math_x_height(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 5)
    }

    /// `num1` math parameter.
    fn num1(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 8)
    }

    /// `num2` math parameter.
    fn num2(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 9)
    }

    /// `num3` math parameter.
    fn num3(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 10)
    }

    /// `denom1` math parameter.
    fn denom1(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 11)
    }

    /// `denom2` math parameter.
    fn denom2(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 12)
    }

    /// `sup1` math parameter.
    fn sup1(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 13)
    }

    /// `sup2` math parameter.
    fn sup2(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 14)
    }

    /// `sup3` math parameter.
    fn sup3(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 15)
    }

    /// `sub1` math parameter.
    fn sub1(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 16)
    }

    /// `sub2` math parameter.
    fn sub2(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 17)
    }

    /// `sup_drop` math parameter.
    fn sup_drop(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 18)
    }

    /// `sub_drop` math parameter.
    fn sub_drop(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 19)
    }

    /// `delim1` math parameter.
    fn delim1(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 20)
    }

    /// `delim2` math parameter.
    fn delim2(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 21)
    }

    /// `axis_height` math parameter.
    fn axis_height(&self, size_code: i32) -> i32 {
        self.mathsy_param(self.math_font(2 + size_code), 22)
    }

    /// `default_rule_thickness` extension-font parameter.
    fn default_rule_thickness(&self) -> i32 {
        self.mathex_param(self.math_font(3 + self.cur_size), 8)
    }

    /// `big_op_spacing1` extension-font parameter.
    fn big_op_spacing1(&self) -> i32 {
        self.mathex_param(self.math_font(3 + self.cur_size), 9)
    }

    /// `big_op_spacing2` extension-font parameter.
    fn big_op_spacing2(&self) -> i32 {
        self.mathex_param(self.math_font(3 + self.cur_size), 10)
    }

    /// `big_op_spacing3` extension-font parameter.
    fn big_op_spacing3(&self) -> i32 {
        self.mathex_param(self.math_font(3 + self.cur_size), 11)
    }

    /// `big_op_spacing4` extension-font parameter.
    fn big_op_spacing4(&self) -> i32 {
        self.mathex_param(self.math_font(3 + self.cur_size), 12)
    }

    /// `big_op_spacing5` extension-font parameter.
    fn big_op_spacing5(&self) -> i32 {
        self.mathex_param(self.math_font(3 + self.cur_size), 13)
    }

    /// Creates a fraction bar rule of thickness `t` (`fraction_rule`).
    fn fraction_rule(&mut self, t: i32) -> Result<i32, TeXMouthError> {
        let p = self.new_rule()?;
        self.mem[(p + 3) as usize].s1 = t;
        self.mem[(p + 2) as usize].s1 = 0;
        Ok(p)
    }

    /// Stacks kern `k`, a rule of thickness `t`, and kern `t` over box `b`
    /// (`overbar`).
    fn overbar(&mut self, b: i32, k: i32, t: i32) -> Result<i32, TeXMouthError> {
        let p = self.new_kern(k)?;
        self.mem[p as usize].s1 = b;
        let q = self.fraction_rule(t)?;
        self.mem[q as usize].s1 = p;
        let p = self.new_kern(t)?;
        self.mem[p as usize].s1 = q;
        Ok(self.vpackage(p, 0, ADDITIONAL, MAX_HALFWORD)?)
    }

    /// Scales glue spec `g` by `m` mu (`math_glue`).
    fn math_glue(&mut self, g: i32, m: i32) -> Result<i32, TeXMouthError> {
        let mut n = self.x_over_n(m, 65536);
        let mut f = self.tex_remainder;
        if f < 0 {
            n -= 1;
            f += 65536;
        }
        let gw1 = self.mem[(g + 1) as usize].s1;
        let gw2 = self.mem[(g + 2) as usize].s1;
        let gw3 = self.mem[(g + 3) as usize].s1;
        let gs0 = self.mem[g as usize].b16_s0();
        let gs1 = self.mem[g as usize].b16_s1();
        let p = self.get_node(GLUE_SPEC_SIZE)?;
        let part = self.xn_over_d(gw1, f, 65536);
        self.mem[(p + 1) as usize].s1 = self.mult_and_add(n, gw1, part, MAX_HALFWORD);
        self.mem[p as usize].set_b16_s1(gs1);
        if self.mem[p as usize].b16_s1() == NORMAL {
            let part = self.xn_over_d(gw2, f, 65536);
            self.mem[(p + 2) as usize].s1 = self.mult_and_add(n, gw2, part, MAX_HALFWORD);
        } else {
            self.mem[(p + 2) as usize].s1 = gw2;
        }
        self.mem[p as usize].set_b16_s0(gs0);
        if self.mem[p as usize].b16_s0() == NORMAL {
            let part = self.xn_over_d(gw3, f, 65536);
            self.mem[(p + 3) as usize].s1 = self.mult_and_add(n, gw3, part, MAX_HALFWORD);
        } else {
            self.mem[(p + 3) as usize].s1 = gw3;
        }
        Ok(p)
    }

    /// Scales kern `p` by `m` mu when it is a `MU_GLUE` kern (`math_kern`).
    fn math_kern(&mut self, p: i32, m: i32) {
        if self.mem[p as usize].b16_s0() == MU_GLUE {
            let mut n = self.x_over_n(m, 65536);
            let mut f = self.tex_remainder;
            if f < 0 {
                n -= 1;
                f += 65536;
            }
            let w = self.mem[(p + 1) as usize].s1;
            let part = self.xn_over_d(w, f, 65536);
            let scaled = self.mult_and_add(n, w, part, MAX_HALFWORD);
            self.mem[(p + 1) as usize].s1 = scaled;
            self.mem[p as usize].set_b16_s0(EXPLICIT);
        }
    }

    /// Converts a noad field to an hbox (`clean_box`).
    fn clean_box(&mut self, p: i32, s: i32) -> Result<i32, TeXMouthError> {
        let mut q = TEX_NULL;
        let found = match self.mem[p as usize].s1 {
            MATH_CHAR => {
                self.cur_mlist = self.new_noad()?;
                let src = self.mem[p as usize];
                self.mem[(self.cur_mlist + 1) as usize] = src;
                false
            }
            SUB_BOX => {
                q = self.mem[p as usize].s0;
                true
            }
            SUB_MLIST => {
                self.cur_mlist = self.mem[p as usize].s0;
                false
            }
            _ => {
                q = self.new_null_box()?;
                true
            }
        };
        if !found {
            let save_style = self.cur_style;
            self.cur_style = s;
            self.mlist_penalties = false;
            self.mlist_to_hlist()?;
            q = self.mem[self.temp_head() as usize].s1;
            self.cur_style = save_style;
            self.set_cur_size_and_mu();
        }
        let mut x = if self.is_char_node(q) || q == TEX_NULL {
            self.hpack(q, 0, ADDITIONAL)?
        } else if self.mem[q as usize].s1 == TEX_NULL
            && self.mem[q as usize].b16_s1() <= VLIST_NODE
            && self.mem[(q + 4) as usize].s1 == 0
        {
            q
        } else {
            self.hpack(q, 0, ADDITIONAL)?
        };
        let q2 = self.mem[(x + 5) as usize].s1;
        if self.is_char_node(q2) {
            let r = self.mem[q2 as usize].s1;
            if r != TEX_NULL
                && self.mem[r as usize].s1 == TEX_NULL
                && !self.is_char_node(r)
                && self.mem[r as usize].b16_s1() == KERN_NODE
            {
                self.free_node(r, MEDIUM_NODE_SIZE)?;
                self.mem[q2 as usize].s1 = TEX_NULL;
            }
        }
        x = x.max(0);
        Ok(x)
    }

    /// Resolves the character and font of a math field (`fetch`).
    fn fetch(&mut self, a: i32) -> Result<(), TeXMouthError> {
        let word = self.mem[a as usize];
        let mut cur_c = i32::from(word.b16_s0());
        self.math_cur_f = self.math_font(i32::from(word.b16_s1()) % 256 + self.cur_size);
        cur_c += i32::from(word.b16_s1()) / 256 * 65536;
        self.math_cur_c = cur_c;
        if self.math_cur_f == 0 {
            // FONT_BASE: undefined family (C §749).
            let size_name = match self.cur_size {
                TEXT_SIZE => "textfont",
                SCRIPT_SIZE => "scriptfont",
                _ => "scriptscriptfont",
            };
            self.report_error(
                &format!(
                    "\\{} {} is undefined (character {})",
                    size_name,
                    i32::from(word.b16_s1()) % 256,
                    cur_c
                ),
                &[
                    "Somewhere in the math formula just ended, you used the",
                    "stated character from an undefined font family. For example,",
                    "plain TeX doesn't allow \\it or \\sl in subscripts. Proceed,",
                    "and I'll try to forget that I needed that character.",
                ],
            );
            self.math_cur_i = LigKernInsn::default();
            self.mem[a as usize].s1 = MATH_FIELD_EMPTY;
        } else if self.is_aat_font(self.math_cur_f)
            || self.using_open_type(self.math_cur_f)
            || self.is_native_font(self.math_cur_f)
        {
            self.math_cur_i = LigKernInsn::default();
        } else {
            let (bc, ec) = self
                .font_record(self.math_cur_f)
                .map_or((0, 0), |r| (r.bc as i32, r.ec as i32));
            if cur_c >= bc && cur_c <= ec {
                self.math_cur_i = self.math_char_info(self.math_cur_f, cur_c);
            } else {
                self.math_cur_i = LigKernInsn::default();
            }
            if self.math_cur_i.skip == 0 {
                self.char_warning(self.math_cur_f, cur_c);
                self.mem[a as usize].s1 = MATH_FIELD_EMPTY;
                self.math_cur_i = LigKernInsn::default();
            }
        }
        Ok(())
    }

    /// Wraps the nucleus in an overbar (`make_over`).
    fn make_over(&mut self, q: i32) -> Result<(), TeXMouthError> {
        let t = self.default_rule_thickness();
        let b = self.clean_box(q + 1, 2 * (self.cur_style / 2) + 1)?;
        let bar = self.overbar(b, 3 * t, t)?;
        self.mem[(q + 1) as usize].s0 = bar;
        self.mem[(q + 1) as usize].s1 = SUB_BOX;
        Ok(())
    }

    /// Wraps the nucleus in an underbar (`make_under`).
    fn make_under(&mut self, q: i32) -> Result<(), TeXMouthError> {
        let x = self.clean_box(q + 1, self.cur_style)?;
        let t = self.default_rule_thickness();
        let p = self.new_kern(3 * t)?;
        self.mem[x as usize].s1 = p;
        let rule = self.fraction_rule(t)?;
        self.mem[p as usize].s1 = rule;
        let y = self.vpackage(x, 0, ADDITIONAL, MAX_HALFWORD)?;
        let delta = self.mem[(y + 3) as usize].s1 + self.mem[(y + 2) as usize].s1 + t;
        let xh = self.mem[(x + 3) as usize].s1;
        self.mem[(y + 3) as usize].s1 = xh;
        self.mem[(y + 2) as usize].s1 = delta - xh;
        self.mem[(q + 1) as usize].s0 = y;
        self.mem[(q + 1) as usize].s1 = SUB_BOX;
        Ok(())
    }

    /// Recenters a `\vcenter` vbox about the math axis (`make_vcenter`).
    fn make_vcenter(&mut self, q: i32) -> Result<(), TeXMouthError> {
        let v = self.mem[(q + 1) as usize].s0;
        if self.mem[v as usize].b16_s1() != VLIST_NODE {
            return Err(self.confusion("vcenter"));
        }
        let delta = self.mem[(v + 3) as usize].s1 + self.mem[(v + 2) as usize].s1;
        let half_d = delta / 2;
        self.mem[(v + 3) as usize].s1 = self.axis_height(self.cur_size) + half_d;
        self.mem[(v + 2) as usize].s1 = delta - self.mem[(v + 3) as usize].s1;
        Ok(())
    }

    /// Builds a radical sign (`make_radical`).
    fn make_radical(&mut self, q: i32) -> Result<(), TeXMouthError> {
        let f =
            self.math_font(i32::from(self.mem[(q + 4) as usize].b16_s3()) % 256 + self.cur_size);
        let ot_math = self.is_ot_math_font(f);
        let rule_thickness = if ot_math {
            self.ot_math_constant(f, 51) // RADICALRULETHICKNESS
        } else {
            self.default_rule_thickness()
        };
        let x = self.clean_box(q + 1, 2 * (self.cur_style / 2) + 1)?;
        let mut clr;
        if ot_math {
            clr = if self.cur_style < TEXT_STYLE {
                self.ot_math_constant(f, 50) // RADICALDISPLAYSTYLEVERTICALGAP
            } else {
                self.ot_math_constant(f, 49) // RADICALVERTICALGAP
            };
        } else if self.cur_style < TEXT_STYLE {
            clr = rule_thickness + self.math_x_height(self.cur_size).abs() / 4;
        } else {
            clr = rule_thickness + rule_thickness.abs() / 4;
        }
        let total =
            self.mem[(x + 3) as usize].s1 + self.mem[(x + 2) as usize].s1 + clr + rule_thickness;
        let y = self.var_delimiter(q + 4, self.cur_size, total)?;
        if ot_math {
            let h = self.mem[(y + 3) as usize].s1 + self.mem[(y + 2) as usize].s1 - rule_thickness;
            self.mem[(y + 2) as usize].s1 = h;
            self.mem[(y + 3) as usize].s1 = rule_thickness;
        }
        let delta = self.mem[(y + 2) as usize].s1
            - (self.mem[(x + 3) as usize].s1 + self.mem[(x + 2) as usize].s1 + clr);
        if delta > 0 {
            clr += delta / 2;
        }
        self.mem[(y + 4) as usize].s1 = -(self.mem[(x + 3) as usize].s1 + clr);
        let xh = self.mem[(y + 3) as usize].s1;
        let ob = self.overbar(x, clr, xh)?;
        self.mem[y as usize].s1 = ob;
        let packed = self.hpack(y, 0, ADDITIONAL)?;
        self.mem[(q + 1) as usize].s0 = packed;
        self.mem[(q + 1) as usize].s1 = SUB_BOX;
        Ok(())
    }

    /// Top-accent attachment position for an OT math nucleus
    /// (`compute_ot_math_accent_pos`).
    fn compute_ot_math_accent_pos(&mut self, p: i32) -> Result<i32, TeXMouthError> {
        if self.mem[(p + 1) as usize].s1 == MATH_CHAR {
            self.fetch(p + 1)?;
            let node = self.new_native_character(self.math_cur_f, self.math_cur_c)?;
            let g = self.get_native_glyph(node, 0);
            Ok(self.ot_math_accent_pos(self.math_cur_f, g))
        } else if self.mem[(p + 1) as usize].s1 == SUB_MLIST {
            let r = self.mem[(p + 1) as usize].s0;
            if r != TEX_NULL && self.mem[r as usize].b16_s1() == ACCENT_NOAD {
                self.compute_ot_math_accent_pos(r)
            } else {
                Ok(TEX_INFINITY)
            }
        } else {
            Ok(TEX_INFINITY)
        }
    }

    /// Builds a math accent (`make_math_accent`).
    fn make_math_accent(&mut self, q: i32) -> Result<(), TeXMouthError> {
        self.fetch(q + 4)?;
        let mut x = TEX_NULL;
        let mut c = self.math_cur_c;
        let f = self.math_cur_f;
        let mut s = 0;
        let mut h = 0;
        let mut w = 0;
        if self.is_aat_font(f) || self.using_open_type(f) {
            let st = self.mem[q as usize].b16_s0();
            if st != BOTTOM_ACC as u16 && st != (BOTTOM_ACC + 1) as u16 {
                s = self.compute_ot_math_accent_pos(q)?;
            }
            x = self.clean_box(q + 1, 2 * (self.cur_style / 2) + 1)?;
            w = self.mem[(x + 1) as usize].s1;
            h = self.mem[(x + 3) as usize].s1;
        } else if self.math_cur_i.skip > 0 {
            let mut i = self.math_cur_i;
            if self.mem[(q + 1) as usize].s1 == MATH_CHAR {
                self.fetch(q + 1)?;
                if i32::from(self.math_cur_i.op) % 4 == LIG_TAG as i32 {
                    // §768: chase the lig/kern program for the skew-char kern.
                    let base_f = self.math_cur_f;
                    let mut a = self.math_cur_i.remainder;
                    let mut insn = self.lig_kern_insn(base_f, a);
                    if insn.skip > 128 {
                        a = 256 * i32::from(insn.op) + insn.remainder + 32768 - 256 * 128;
                        insn = self.lig_kern_insn(base_f, a);
                    }
                    let skew = self.font_record(base_f).map_or(-1, |r| r.skew_char);
                    loop {
                        if insn.next_char == skew {
                            if i32::from(insn.op) >= 128 && insn.skip <= 128 {
                                s = self.kern_value(base_f, insn.op, insn.remainder);
                            }
                            break;
                        }
                        if insn.skip >= 128 {
                            break;
                        }
                        a += i32::from(insn.skip) + 1;
                        insn = self.lig_kern_insn(base_f, a);
                    }
                }
            }
            x = self.clean_box(q + 1, 2 * (self.cur_style / 2) + 1)?;
            w = self.mem[(x + 1) as usize].s1;
            h = self.mem[(x + 3) as usize].s1;
            // §767: grow the accent through the LIST_TAG chain.
            loop {
                if i32::from(i.op) % 4 != LIST_TAG as i32 {
                    break;
                }
                let y = i.remainder;
                let iy = self.math_char_info(f, y);
                if iy.skip == 0 || self.font_char_metric(f, y.max(0) as u32).width > w {
                    break;
                }
                i = iy;
                c = y;
            }
        }
        if x != TEX_NULL {
            let mut delta;
            if self.is_ot_math_font(f) {
                let st = self.mem[q as usize].b16_s0();
                if st == BOTTOM_ACC as u16 || st == (BOTTOM_ACC + 1) as u16 {
                    delta = 0;
                } else {
                    let base = self.ot_math_constant(f, 6); // ACCENTBASEHEIGHT
                    delta = if h < base { h } else { base };
                }
            } else {
                let xh = self
                    .font_record(f)
                    .and_then(|r| r.params.get(X_HEIGHT_CODE as usize))
                    .copied()
                    .unwrap_or(0);
                delta = if h < xh { h } else { xh };
            }
            if (self.mem[(q + 2) as usize].s1 != MATH_FIELD_EMPTY
                || self.mem[(q + 3) as usize].s1 != MATH_FIELD_EMPTY)
                && self.mem[(q + 1) as usize].s1 == MATH_CHAR
            {
                // §769: fold scripts under the accent.
                self.flush_node_list(x)?;
                x = self.new_noad()?;
                self.mem[(x + 1) as usize] = self.mem[(q + 1) as usize];
                self.mem[(x + 2) as usize] = self.mem[(q + 2) as usize];
                self.mem[(x + 3) as usize] = self.mem[(q + 3) as usize];
                self.mem[(q + 2) as usize].s1 = MATH_FIELD_EMPTY;
                self.mem[(q + 2) as usize].s0 = TEX_NULL;
                self.mem[(q + 3) as usize].s1 = MATH_FIELD_EMPTY;
                self.mem[(q + 3) as usize].s0 = TEX_NULL;
                self.mem[(q + 1) as usize].s1 = SUB_MLIST;
                self.mem[(q + 1) as usize].s0 = x;
                x = self.clean_box(q + 1, self.cur_style)?;
                delta += self.mem[(x + 3) as usize].s1 - h;
                h = self.mem[(x + 3) as usize].s1;
            }
            let y = self.char_box(f, c)?;
            if self.is_aat_font(f) || self.using_open_type(f) {
                let mut p = self.get_node(GLYPH_NODE_SIZE)?;
                self.mem[p as usize].set_b16_s1(WHATSIT_NODE);
                self.mem[p as usize].set_b16_s0(GLYPH_NODE);
                self.mem[(p + 4) as usize].set_b16_s2(f as u16);
                let z = self.mem[(y + 5) as usize].s1;
                let glyph = self.get_native_glyph(z, 0);
                self.mem[(p + 4) as usize].set_b16_s1(glyph.max(0) as u16);
                self.set_native_glyph_metrics(p, true);
                let zsize = i32::from(self.mem[(z + 4) as usize].b16_s3());
                self.free_node(z, zsize)?;
                self.mem[(y + 5) as usize].s1 = p;
                if self.mem[q as usize].b16_s0() % 2 == 1 {
                    self.set_native_glyph_metrics(p, true);
                } else {
                    let mut a = 0;
                    let mut w2;
                    loop {
                        let (g, adv) = self.ot_math_variant(f, c, a, 1);
                        w2 = adv;
                        if w2 > 0 && w2 <= w {
                            self.mem[(p + 4) as usize].set_b16_s1(g.max(0) as u16);
                            self.set_native_glyph_metrics(p, true);
                            a += 1;
                        }
                        if w2 < 0 || w2 >= w {
                            break;
                        }
                    }
                    if w2 < 0 {
                        if let Some(asm) = self.ot_math_assembly(f, c, 1) {
                            self.free_node(p, GLYPH_NODE_SIZE)?;
                            p = self.build_opentype_assembly(f, &asm, w, true)?;
                            self.mem[(y + 5) as usize].s1 = p;
                        }
                    } else {
                        self.set_native_glyph_metrics(p, true);
                    }
                }
                self.mem[(y + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
                self.mem[(y + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                self.mem[(y + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
                let st = self.mem[q as usize].b16_s0();
                if st == BOTTOM_ACC as u16 || st == (BOTTOM_ACC + 1) as u16 {
                    if self.mem[(y + 3) as usize].s1 < 0 {
                        self.mem[(y + 3) as usize].s1 = 0;
                    }
                } else if self.mem[(y + 2) as usize].s1 < 0 {
                    self.mem[(y + 2) as usize].s1 = 0;
                }
                let sa = if self.is_glyph_node(p) {
                    let pos =
                        self.ot_math_accent_pos(f, i32::from(self.mem[(p + 4) as usize].b16_s1()));
                    if pos == TEX_INFINITY {
                        self.mem[(y + 1) as usize].s1 / 2
                    } else {
                        pos
                    }
                } else {
                    self.mem[(y + 1) as usize].s1 / 2
                };
                if st == BOTTOM_ACC as u16 || st == (BOTTOM_ACC + 1) as u16 || s == TEX_INFINITY {
                    s = w / 2;
                }
                self.mem[(y + 4) as usize].s1 = s - sa;
            } else {
                self.mem[(y + 4) as usize].s1 = s + (w - self.mem[(y + 1) as usize].s1) / 2;
            }
            self.mem[(y + 1) as usize].s1 = 0;
            let st = self.mem[q as usize].b16_s0();
            if st == BOTTOM_ACC as u16 || st == (BOTTOM_ACC + 1) as u16 {
                self.mem[x as usize].s1 = y;
                let y = self.vpackage(x, 0, ADDITIONAL, MAX_HALFWORD)?;
                self.mem[(y + 4) as usize].s1 = -(h - self.mem[(y + 3) as usize].s1);
                self.mem[(y + 1) as usize].s1 = self.mem[(x + 1) as usize].s1;
                self.mem[(q + 1) as usize].s0 = y;
            } else {
                let p = self.new_kern(-delta)?;
                self.mem[p as usize].s1 = x;
                self.mem[y as usize].s1 = p;
                let y = self.vpackage(y, 0, ADDITIONAL, MAX_HALFWORD)?;
                if self.mem[(y + 3) as usize].s1 < h {
                    // §765: raise the accentee height if needed.
                    let p = self.new_kern(h - self.mem[(y + 3) as usize].s1)?;
                    let list = self.mem[(y + 5) as usize].s1;
                    self.mem[p as usize].s1 = list;
                    self.mem[(y + 5) as usize].s1 = p;
                    self.mem[(y + 3) as usize].s1 = h;
                }
                self.mem[(y + 1) as usize].s1 = self.mem[(x + 1) as usize].s1;
                self.mem[(q + 1) as usize].s0 = y;
            }
            self.mem[(q + 1) as usize].s1 = SUB_BOX;
        }
        Ok(())
    }

    /// Builds a fraction noad (`make_fraction`).
    fn make_fraction(&mut self, q: i32) -> Result<(), TeXMouthError> {
        if self.mem[(q + 1) as usize].s1 == DEFAULT_CODE {
            self.mem[(q + 1) as usize].s1 = self.default_rule_thickness();
        }
        let mut x = self.clean_box(q + 2, self.cur_style + 2 - 2 * (self.cur_style / 6))?;
        let mut z = self.clean_box(
            q + 3,
            2 * (self.cur_style / 2) + 3 - 2 * (self.cur_style / 6),
        )?;
        if self.mem[(x + 1) as usize].s1 < self.mem[(z + 1) as usize].s1 {
            let w = self.mem[(z + 1) as usize].s1;
            x = self.rebox(x, w)?;
        } else {
            let w = self.mem[(x + 1) as usize].s1;
            z = self.rebox(z, w)?;
        }
        let mut shift_up;
        let mut shift_down;
        if self.cur_style < TEXT_STYLE {
            shift_up = self.num1(self.cur_size);
            shift_down = self.denom1(self.cur_size);
        } else {
            shift_down = self.denom2(self.cur_size);
            shift_up = if self.mem[(q + 1) as usize].s1 != 0 {
                self.num2(self.cur_size)
            } else {
                self.num3(self.cur_size)
            };
        }
        let delta;
        if self.mem[(q + 1) as usize].s1 == 0 {
            // §772: rule-free fraction.
            let clr = if self.is_ot_math_font(self.math_cur_f) {
                if self.cur_style < TEXT_STYLE {
                    self.ot_math_constant(self.math_cur_f, 27) // STACKDISPLAYSTYLEGAPMIN
                } else {
                    self.ot_math_constant(self.math_cur_f, 26) // STACKGAPMIN
                }
            } else if self.cur_style < TEXT_STYLE {
                7 * self.default_rule_thickness()
            } else {
                3 * self.default_rule_thickness()
            };
            let extra = (clr
                - ((shift_up - self.mem[(x + 2) as usize].s1)
                    - (self.mem[(z + 3) as usize].s1 - shift_down)))
                / 2;
            if extra > 0 {
                shift_up += extra;
                shift_down += extra;
            }
            delta = 0;
        } else {
            // §773: ruled fraction.
            let delta1;
            let delta2;
            if self.is_ot_math_font(self.math_cur_f) {
                delta = self.mem[(q + 1) as usize].s1 / 2;
                let clr = if self.cur_style < TEXT_STYLE {
                    self.ot_math_constant(self.math_cur_f, 37) // FRACTIONNUMDISPLAYSTYLEGAPMIN
                } else {
                    self.ot_math_constant(self.math_cur_f, 36) // FRACTIONNUMERATORGAPMIN
                };
                delta1 = clr
                    - ((shift_up - self.mem[(x + 2) as usize].s1)
                        - (self.axis_height(self.cur_size) + delta));
                let clr = if self.cur_style < TEXT_STYLE {
                    self.ot_math_constant(self.math_cur_f, 40) // FRACTIONDENOMDISPLAYSTYLEGAPMIN
                } else {
                    self.ot_math_constant(self.math_cur_f, 39) // FRACTIONDENOMINATORGAPMIN
                };
                delta2 = clr
                    - ((self.axis_height(self.cur_size) - delta)
                        - (self.mem[(z + 3) as usize].s1 - shift_down));
            } else {
                let clr = if self.cur_style < TEXT_STYLE {
                    3 * self.mem[(q + 1) as usize].s1
                } else {
                    self.mem[(q + 1) as usize].s1
                };
                delta = self.mem[(q + 1) as usize].s1 / 2;
                delta1 = clr
                    - ((shift_up - self.mem[(x + 2) as usize].s1)
                        - (self.axis_height(self.cur_size) + delta));
                delta2 = clr
                    - ((self.axis_height(self.cur_size) - delta)
                        - (self.mem[(z + 3) as usize].s1 - shift_down));
            }
            if delta1 > 0 {
                shift_up += delta1;
            }
            if delta2 > 0 {
                shift_down += delta2;
            }
        }
        let v = self.new_null_box()?;
        self.mem[v as usize].set_b16_s1(VLIST_NODE);
        self.mem[(v + 3) as usize].s1 = shift_up + self.mem[(x + 3) as usize].s1;
        self.mem[(v + 2) as usize].s1 = self.mem[(z + 2) as usize].s1 + shift_down;
        self.mem[(v + 1) as usize].s1 = self.mem[(x + 1) as usize].s1;
        let mut p;
        if self.mem[(q + 1) as usize].s1 == 0 {
            p = self.new_kern(
                (shift_up - self.mem[(x + 2) as usize].s1)
                    - (self.mem[(z + 3) as usize].s1 - shift_down),
            )?;
            self.mem[p as usize].s1 = z;
        } else {
            let y = self.fraction_rule(self.mem[(q + 1) as usize].s1)?;
            p = self.new_kern(
                (self.axis_height(self.cur_size) - delta)
                    - (self.mem[(z + 3) as usize].s1 - shift_down),
            )?;
            self.mem[y as usize].s1 = p;
            self.mem[p as usize].s1 = z;
            p = self.new_kern(
                (shift_up - self.mem[(x + 2) as usize].s1)
                    - (self.axis_height(self.cur_size) + delta),
            )?;
            self.mem[p as usize].s1 = y;
        }
        self.mem[x as usize].s1 = p;
        self.mem[(v + 5) as usize].s1 = x; // §774
        let delta_delim = if self.cur_style < TEXT_STYLE {
            self.delim1(self.cur_size)
        } else {
            self.delim2(self.cur_size)
        };
        let x = self.var_delimiter(q + 4, self.cur_size, delta_delim)?;
        self.mem[x as usize].s1 = v;
        let z = self.var_delimiter(q + 5, self.cur_size, delta_delim)?;
        self.mem[v as usize].s1 = z;
        self.mem[(q + 1) as usize].s1 = self.hpack(x, 0, ADDITIONAL)?; // §775
        Ok(())
    }

    /// Converts an operator noad and returns its italic correction
    /// (`make_op`).
    fn make_op(&mut self, q: i32) -> Result<i32, TeXMouthError> {
        let mut delta = 0;
        if self.mem[q as usize].b16_s0() == NORMAL && self.cur_style < TEXT_STYLE {
            self.mem[q as usize].set_b16_s0(LIMITS);
        }
        if self.mem[(q + 1) as usize].s1 == MATH_CHAR {
            self.fetch(q + 1)?;
            if !(self.using_open_type(self.math_cur_f)) {
                if self.cur_style < TEXT_STYLE
                    && i32::from(self.math_cur_i.op) % 4 == LIST_TAG as i32
                {
                    let c = self.math_cur_i.remainder;
                    let i = self.math_char_info(self.math_cur_f, c);
                    if i.skip > 0 {
                        self.math_cur_c = c;
                        self.math_cur_i = i;
                        self.mem[(q + 1) as usize].set_b16_s0(c.max(0) as u16);
                    }
                }
                delta = self
                    .effective_char_metric(self.math_cur_f, self.math_cur_c)
                    .italic;
            }
            let x = self.clean_box(q + 1, self.cur_style)?;
            if self.is_ot_math_font(self.math_cur_f) {
                let mut p = self.mem[(x + 5) as usize].s1;
                if self.is_glyph_node(p) {
                    if self.cur_style < TEXT_STYLE {
                        let mut h1 = self.ot_math_constant(self.math_cur_f, 3); // DISPLAYOPERATORMINHEIGHT
                        let tot =
                            (self.mem[(p + 3) as usize].s1 + self.mem[(p + 2) as usize].s1) * 5 / 4;
                        if h1 < tot {
                            h1 = tot;
                        }
                        let c = i32::from(self.mem[(p + 4) as usize].b16_s1());
                        let mut n = 0;
                        let mut h2;
                        loop {
                            let (g, adv) = self.ot_math_variant(self.math_cur_f, c, n, 0);
                            h2 = adv;
                            if h2 > 0 {
                                self.mem[(p + 4) as usize].set_b16_s1(g.max(0) as u16);
                                self.set_native_glyph_metrics(p, true);
                            }
                            n += 1;
                            if h2 < 0 || h2 >= h1 {
                                break;
                            }
                        }
                        if h2 < 0 {
                            if let Some(asm) = self.ot_math_assembly(self.math_cur_f, c, 0) {
                                self.free_node(p, GLYPH_NODE_SIZE)?;
                                p =
                                    self.build_opentype_assembly(self.math_cur_f, &asm, h1, false)?;
                                self.mem[(x + 5) as usize].s1 = p;
                                delta = 0;
                            }
                        } else {
                            self.set_native_glyph_metrics(p, true);
                        }
                    }
                    if delta != 0 || self.mem[(x + 5) as usize].s1 == p {
                        delta = self.ot_math_ital_corr(
                            self.math_cur_f,
                            i32::from(self.mem[(p + 4) as usize].b16_s1()),
                        );
                    }
                    self.mem[(x + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
                    self.mem[(x + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                    self.mem[(x + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
                }
            }
            if self.mem[(q + 3) as usize].s1 != MATH_FIELD_EMPTY
                && self.mem[q as usize].b16_s0() != LIMITS
            {
                self.mem[(x + 1) as usize].s1 -= delta;
            }
            self.mem[(x + 4) as usize].s1 =
                (self.mem[(x + 3) as usize].s1 - self.mem[(x + 2) as usize].s1) / 2
                    - self.axis_height(self.cur_size);
            self.mem[(q + 1) as usize].s1 = SUB_BOX;
            self.mem[(q + 1) as usize].s0 = x;
        }
        let save_f = self.math_cur_f;
        if self.mem[q as usize].b16_s0() == LIMITS {
            // §777: stack scripts above/below.
            let mut x =
                self.clean_box(q + 2, 2 * (self.cur_style / 4) + 4 + (self.cur_style % 2))?;
            let mut y = self.clean_box(q + 1, self.cur_style)?;
            let mut z = self.clean_box(q + 3, 2 * (self.cur_style / 4) + 5)?;
            let v = self.new_null_box()?;
            self.mem[v as usize].set_b16_s1(VLIST_NODE);
            let mut vw = self.mem[(y + 1) as usize].s1;
            if self.mem[(x + 1) as usize].s1 > vw {
                vw = self.mem[(x + 1) as usize].s1;
            }
            if self.mem[(z + 1) as usize].s1 > vw {
                vw = self.mem[(z + 1) as usize].s1;
            }
            self.mem[(v + 1) as usize].s1 = vw;
            x = self.rebox(x, vw)?;
            y = self.rebox(y, vw)?;
            z = self.rebox(z, vw)?;
            self.mem[(x + 4) as usize].s1 = delta / 2;
            self.mem[(z + 4) as usize].s1 = -(delta / 2);
            self.mem[(v + 3) as usize].s1 = self.mem[(y + 3) as usize].s1;
            self.mem[(v + 2) as usize].s1 = self.mem[(y + 2) as usize].s1;
            self.math_cur_f = save_f;
            if self.mem[(q + 2) as usize].s1 == MATH_FIELD_EMPTY {
                self.free_node(x, BOX_NODE_SIZE)?;
                self.mem[(v + 5) as usize].s1 = y;
            } else {
                let mut shift_up = self.big_op_spacing3() - self.mem[(x + 2) as usize].s1;
                if shift_up < self.big_op_spacing1() {
                    shift_up = self.big_op_spacing1();
                }
                let p = self.new_kern(shift_up)?;
                self.mem[p as usize].s1 = y;
                self.mem[x as usize].s1 = p;
                let p = self.new_kern(self.big_op_spacing5())?;
                self.mem[p as usize].s1 = x;
                self.mem[(v + 5) as usize].s1 = p;
                self.mem[(v + 3) as usize].s1 += self.big_op_spacing5()
                    + self.mem[(x + 3) as usize].s1
                    + self.mem[(x + 2) as usize].s1
                    + shift_up;
            }
            if self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY {
                self.free_node(z, BOX_NODE_SIZE)?;
            } else {
                let mut shift_down = self.big_op_spacing4() - self.mem[(z + 3) as usize].s1;
                if shift_down < self.big_op_spacing2() {
                    shift_down = self.big_op_spacing2();
                }
                let p = self.new_kern(shift_down)?;
                self.mem[y as usize].s1 = p;
                self.mem[p as usize].s1 = z;
                let p = self.new_kern(self.big_op_spacing5())?;
                self.mem[z as usize].s1 = p;
                self.mem[(v + 2) as usize].s1 += self.big_op_spacing5()
                    + self.mem[(z + 3) as usize].s1
                    + self.mem[(z + 2) as usize].s1
                    + shift_down;
            }
            self.mem[(q + 1) as usize].s1 = v;
        }
        Ok(delta)
    }

    /// Resolves italic-correction kerning and ligatures between adjacent
    /// ordinary noads (`make_ord`).
    fn make_ord(&mut self, q: i32) -> Result<(), TeXMouthError> {
        'restart: loop {
            if self.mem[(q + 3) as usize].s1 != MATH_FIELD_EMPTY
                || self.mem[(q + 2) as usize].s1 != MATH_FIELD_EMPTY
                || self.mem[(q + 1) as usize].s1 != MATH_CHAR
            {
                return Ok(());
            }
            let p = self.mem[q as usize].s1;
            if p == TEX_NULL {
                return Ok(());
            }
            let ptype = self.mem[p as usize].b16_s1();
            if !(ORD_NOAD..=PUNCT_NOAD).contains(&ptype) {
                return Ok(());
            }
            if self.mem[(p + 1) as usize].s1 != MATH_CHAR {
                return Ok(());
            }
            if i32::from(self.mem[(p + 1) as usize].b16_s1()) % 256
                != i32::from(self.mem[(q + 1) as usize].b16_s1()) % 256
            {
                return Ok(());
            }
            self.mem[(q + 1) as usize].s1 = MATH_TEXT_CHAR;
            self.fetch(q + 1)?;
            if i32::from(self.math_cur_i.op) % 4 != LIG_TAG as i32 {
                return Ok(());
            }
            let f = self.math_cur_f;
            let mut a = self.math_cur_i.remainder;
            let cur_c = i32::from(self.mem[(p + 1) as usize].b16_s0());
            let mut insn = self.lig_kern_insn(f, a);
            if insn.skip > 128 {
                a = 256 * i32::from(insn.op) + insn.remainder + 32768 - 256 * 128;
                insn = self.lig_kern_insn(f, a);
            }
            loop {
                if insn.next_char == cur_c && insn.skip <= 128 {
                    if i32::from(insn.op) >= 128 {
                        let kv = self.kern_value(f, insn.op, insn.remainder);
                        let pk = self.new_kern(kv)?;
                        self.mem[pk as usize].s1 = self.mem[q as usize].s1;
                        self.mem[q as usize].s1 = pk;
                        return Ok(());
                    }
                    match insn.op {
                        1 | 5 => {
                            self.mem[(q + 1) as usize].set_b16_s0(insn.remainder.max(0) as u16);
                        }
                        2 | 6 => {
                            self.mem[(p + 1) as usize].set_b16_s0(insn.remainder.max(0) as u16);
                        }
                        3 | 7 | 11 => {
                            let r = self.new_noad()?;
                            self.mem[(r + 1) as usize].set_b16_s0(insn.remainder.max(0) as u16);
                            let fam = self.mem[(q + 1) as usize].b16_s1() % 256;
                            self.mem[(r + 1) as usize].set_b16_s1(fam);
                            let qlink = self.mem[q as usize].s1;
                            self.mem[q as usize].s1 = r;
                            self.mem[r as usize].s1 = qlink;
                            self.mem[(r + 1) as usize].s1 = if insn.op < 11 {
                                MATH_CHAR
                            } else {
                                MATH_TEXT_CHAR
                            };
                        }
                        _ => {
                            let plink = self.mem[p as usize].s1;
                            self.mem[q as usize].s1 = plink;
                            self.mem[(q + 1) as usize].set_b16_s0(insn.remainder.max(0) as u16);
                            self.mem[(q + 3) as usize] = self.mem[(p + 3) as usize];
                            self.mem[(q + 2) as usize] = self.mem[(p + 2) as usize];
                            self.free_node(p, NOAD_SIZE)?;
                        }
                    }
                    if insn.op > 3 {
                        return Ok(());
                    }
                    self.mem[(q + 1) as usize].s1 = MATH_CHAR;
                    continue 'restart;
                }
                if insn.skip >= 128 {
                    return Ok(());
                }
                a += i32::from(insn.skip) + 1;
                insn = self.lig_kern_insn(f, a);
            }
        }
    }

    /// Appends a kern of `delta` to the new hlist at `q + 1`
    /// (`attach_hkern_to_new_hlist`).
    fn attach_hkern_to_new_hlist(&mut self, q: i32, delta: i32) -> Result<i32, TeXMouthError> {
        let z = self.new_kern(delta)?;
        if self.mem[(q + 1) as usize].s1 == TEX_NULL {
            self.mem[(q + 1) as usize].s1 = z;
        } else {
            let mut y = self.mem[(q + 1) as usize].s1;
            while self.mem[y as usize].s1 != TEX_NULL {
                y = self.mem[y as usize].s1;
            }
            self.mem[y as usize].s1 = z;
        }
        Ok(self.mem[(q + 1) as usize].s1)
    }

    /// Scans a script mlist for its first character nucleus and resolves
    /// the OT math script glyph (C's duplicated `make_scripts` scan).
    /// Returns `(script_c, script_g, script_f)`.
    fn scan_script_glyph(&mut self, q_field: i32) -> Result<(i32, i32, i32), TeXMouthError> {
        let mut script_head = q_field;
        let mut this_math_style = 2 * (self.cur_style / 4) + 5;
        if self.mem[script_head as usize].s1 == SUB_MLIST {
            let mut script_ptr = self.mem[script_head as usize].s0;
            script_head = TEX_NULL;
            while self.is_valid_mem_ptr(script_ptr) {
                match self.mem[script_ptr as usize].b16_s1() {
                    t if t == KERN_NODE || t == GLUE_NODE || t == CHOICE_NODE => {}
                    STYLE_NODE => {
                        this_math_style = i32::from(self.mem[script_ptr as usize].b16_s0());
                    }
                    ORD_NOAD | OP_NOAD | BIN_NOAD | REL_NOAD | OPEN_NOAD | CLOSE_NOAD
                    | PUNCT_NOAD => {
                        script_head = script_ptr + 1;
                        script_ptr = TEX_NULL;
                    }
                    _ => script_ptr = TEX_NULL,
                }
                if self.is_valid_mem_ptr(script_ptr) {
                    if self.mem[script_ptr as usize].b16_s1() == CHOICE_NODE {
                        script_ptr = match this_math_style / 2 {
                            0 => self.mem[(script_ptr + 1) as usize].s0,
                            1 => self.mem[(script_ptr + 1) as usize].s1,
                            2 => self.mem[(script_ptr + 2) as usize].s0,
                            _ => self.mem[(script_ptr + 2) as usize].s1,
                        };
                    } else {
                        script_ptr = self.mem[script_ptr as usize].s1;
                    }
                }
            }
        }
        let mut script_c = TEX_NULL;
        let mut script_g = 0;
        let mut script_f = 0;
        if self.is_valid_mem_ptr(script_head) && self.mem[script_head as usize].s1 == MATH_CHAR {
            let save_f = self.math_cur_f;
            let saved_style = self.cur_style;
            self.cur_style = this_math_style;
            self.set_cur_size_and_mu();
            self.fetch(script_head)?;
            if self.is_ot_math_font(self.math_cur_f) {
                script_c = self.new_native_character(self.math_cur_f, self.math_cur_c)?;
                script_g = self.get_native_glyph(script_c, 0);
                script_f = self.math_cur_f;
            }
            self.math_cur_f = save_f;
            self.cur_style = saved_style;
            self.set_cur_size_and_mu();
        }
        Ok((script_c, script_g, script_f))
    }

    /// Attaches super- and subscripts to a noad (`make_scripts`).
    fn make_scripts(&mut self, q: i32, delta: i32) -> Result<(), TeXMouthError> {
        let p = self.mem[(q + 1) as usize].s1;
        let mut sup_kern = 0;
        let mut sub_kern = 0;
        let mut shift_up = 0;
        let mut shift_down = 0;
        if !(self.is_char_node(p) || self.is_glyph_node(p)) {
            let z = self.hpack(p, 0, ADDITIONAL)?;
            let t = if self.cur_style < SCRIPT_STYLE {
                SCRIPT_SIZE
            } else {
                SCRIPT_SCRIPT_SIZE
            };
            shift_up = self.mem[(z + 3) as usize].s1 - self.sup_drop(t);
            shift_down = self.mem[(z + 2) as usize].s1 + self.sub_drop(t);
            self.free_node(z, BOX_NODE_SIZE)?;
        }
        let x;
        if self.mem[(q + 2) as usize].s1 == MATH_FIELD_EMPTY {
            // §801: subscript only.
            let (_sc, sg, sf) = self.scan_script_glyph(q + 3)?;
            let save_f = self.math_cur_f;
            let xx = self.clean_box(q + 3, 2 * (self.cur_style / 4) + 5)?;
            self.math_cur_f = save_f;
            self.mem[(xx + 1) as usize].s1 += self.dimen_par(DIMEN_PAR_SCRIPT_SPACE)?;
            let s1 = self.sub1(self.cur_size);
            if shift_down < s1 {
                shift_down = s1;
            }
            let clr = if self.is_ot_math_font(self.math_cur_f) {
                self.mem[(xx + 3) as usize].s1 - self.ot_math_constant(self.math_cur_f, 9)
            // SUBSCRIPTTOPMAX
            } else {
                self.mem[(xx + 3) as usize].s1 - (self.math_x_height(self.cur_size) * 4).abs() / 5
            };
            if shift_down < clr {
                shift_down = clr;
            }
            self.mem[(xx + 4) as usize].s1 = shift_down;
            // §787
            if self.is_ot_math_font(self.math_cur_f) && self.is_glyph_node(p) {
                sub_kern = self.ot_math_kern(
                    i32::from(self.mem[(p + 4) as usize].b16_s2()),
                    i32::from(self.mem[(p + 4) as usize].b16_s1()),
                    sf,
                    sg,
                    SUB_CMD,
                    shift_down,
                );
                if sub_kern != 0 {
                    self.attach_hkern_to_new_hlist(q, sub_kern)?;
                }
            }
            x = xx;
        } else {
            let (_sc, sup_g, sup_f) = self.scan_script_glyph(q + 2)?;
            let save_f = self.math_cur_f;
            let xx = self.clean_box(q + 2, 2 * (self.cur_style / 4) + 4 + (self.cur_style % 2))?;
            self.math_cur_f = save_f;
            self.mem[(xx + 1) as usize].s1 += self.dimen_par(DIMEN_PAR_SCRIPT_SPACE)?;
            let clr = if self.cur_style % 2 == 1 {
                self.sup3(self.cur_size)
            } else if self.cur_style < TEXT_STYLE {
                self.sup1(self.cur_size)
            } else {
                self.sup2(self.cur_size)
            };
            if shift_up < clr {
                shift_up = clr;
            }
            let clr = if self.is_ot_math_font(self.math_cur_f) {
                self.mem[(xx + 2) as usize].s1 + self.ot_math_constant(self.math_cur_f, 13)
            // SUPERSCRIPTBOTTOMMIN
            } else {
                self.mem[(xx + 2) as usize].s1 + self.math_x_height(self.cur_size).abs() / 4
            };
            if shift_up < clr {
                shift_up = clr;
            }
            // §788
            if self.is_ot_math_font(self.math_cur_f)
                && self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY
                && self.is_glyph_node(p)
            {
                sup_kern = self.ot_math_kern(
                    i32::from(self.mem[(p + 4) as usize].b16_s2()),
                    i32::from(self.mem[(p + 4) as usize].b16_s1()),
                    sup_f,
                    sup_g,
                    SUP_CMD,
                    shift_up,
                );
                if sup_kern != 0 {
                    self.attach_hkern_to_new_hlist(q, sup_kern)?;
                }
            }
            if self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY {
                self.mem[(xx + 4) as usize].s1 = -shift_up;
                x = xx;
            } else {
                // §786: both scripts.
                let (_sc, sub_g, sub_f) = self.scan_script_glyph(q + 3)?;
                let save_f = self.math_cur_f;
                let y = self.clean_box(q + 3, 2 * (self.cur_style / 4) + 5)?;
                self.math_cur_f = save_f;
                self.mem[(y + 1) as usize].s1 += self.dimen_par(DIMEN_PAR_SCRIPT_SPACE)?;
                let s2 = self.sub2(self.cur_size);
                if shift_down < s2 {
                    shift_down = s2;
                }
                let mut clr = if self.is_ot_math_font(self.math_cur_f) {
                    self.ot_math_constant(self.math_cur_f, 15) // SUBSUPERSCRIPTGAPMIN
                        - ((shift_up - self.mem[(xx + 2) as usize].s1)
                            - (self.mem[(y + 3) as usize].s1 - shift_down))
                } else {
                    4 * self.default_rule_thickness()
                        - ((shift_up - self.mem[(xx + 2) as usize].s1)
                            - (self.mem[(y + 3) as usize].s1 - shift_down))
                };
                if clr > 0 {
                    shift_down += clr;
                    clr = if self.is_ot_math_font(self.math_cur_f) {
                        self.ot_math_constant(self.math_cur_f, 16) // SUPERSCRIPTBOTTOMMAXWITHSUBSCRIPT
                            - (shift_up - self.mem[(xx + 2) as usize].s1)
                    } else {
                        (self.math_x_height(self.cur_size) * 4).abs() / 5
                            - (shift_up - self.mem[(xx + 2) as usize].s1)
                    };
                    if clr > 0 {
                        shift_up += clr;
                        shift_down -= clr;
                    }
                }
                if self.is_ot_math_font(self.math_cur_f) {
                    if self.is_glyph_node(p) {
                        sub_kern = self.ot_math_kern(
                            i32::from(self.mem[(p + 4) as usize].b16_s2()),
                            i32::from(self.mem[(p + 4) as usize].b16_s1()),
                            sub_f,
                            sub_g,
                            SUB_CMD,
                            shift_down,
                        );
                        if sub_kern != 0 {
                            self.attach_hkern_to_new_hlist(q, sub_kern)?;
                        }
                    }
                    if self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY && self.is_glyph_node(p) {
                        sup_kern = self.ot_math_kern(
                            i32::from(self.mem[(p + 4) as usize].b16_s2()),
                            i32::from(self.mem[(p + 4) as usize].b16_s1()),
                            sup_f,
                            sup_g,
                            SUP_CMD,
                            shift_up,
                        );
                        if sup_kern != 0 {
                            self.attach_hkern_to_new_hlist(q, sup_kern)?;
                        }
                    }
                } else {
                    sup_kern = 0;
                    sub_kern = 0;
                }
                self.mem[(xx + 4) as usize].s1 = sup_kern + delta - sub_kern;
                let pk = self.new_kern(
                    (shift_up - self.mem[(xx + 2) as usize].s1)
                        - (self.mem[(y + 3) as usize].s1 - shift_down),
                )?;
                self.mem[xx as usize].s1 = pk;
                self.mem[pk as usize].s1 = y;
                let packed = self.vpackage(xx, 0, ADDITIONAL, MAX_HALFWORD)?;
                self.mem[(packed + 4) as usize].s1 = shift_down;
                x = packed;
            }
        }
        if self.mem[(q + 1) as usize].s1 == TEX_NULL {
            self.mem[(q + 1) as usize].s1 = x;
        } else {
            let mut pp = self.mem[(q + 1) as usize].s1;
            while self.mem[pp as usize].s1 != TEX_NULL {
                pp = self.mem[pp as usize].s1;
            }
            self.mem[pp as usize].s1 = x;
        }
        Ok(())
    }

    /// Sizes a `\left`/`\middle`/`\right` delimiter (`make_left_right`).
    fn make_left_right(
        &mut self,
        q: i32,
        style: i32,
        max_d: i32,
        max_h: i32,
    ) -> Result<i32, TeXMouthError> {
        self.cur_style = style;
        self.set_cur_size_and_mu();
        let delta2 = max_d + self.axis_height(self.cur_size);
        let mut delta1 = max_h + max_d - delta2;
        if delta2 > delta1 {
            delta1 = delta2;
        }
        let mut delta = (delta1 / 500) * self.int_par(INT_PAR_DELIMITER_FACTOR)?;
        let delta2 = delta1 + delta1 - self.dimen_par(DIMEN_PAR_DELIMITER_SHORTFALL)?;
        if delta < delta2 {
            delta = delta2;
        }
        self.mem[(q + 1) as usize].s1 = self.var_delimiter(q + 1, self.cur_size, delta)?;
        Ok(i32::from(self.mem[q as usize].b16_s1()) - (LEFT_NOAD as i32 - 20))
    }

    /// Converts an mlist to an hlist (`mlist_to_hlist`, C §726–762).
    fn mlist_to_hlist(&mut self) -> Result<(), TeXMouthError> {
        const OFFSET_TABLE: [&str; 8] = [
            "02340001", "22*40001", "33**3**3", "44*04004", "00*00000", "02340001", "11*11111",
            "12341011",
        ];
        let mlist = self.cur_mlist;
        let penalties = self.mlist_penalties;
        let style = self.cur_style;
        let mut q = mlist;
        let mut r = TEX_NULL;
        let mut r_type = OP_NOAD as i32;
        let mut max_h = 0;
        let mut max_d = 0;
        self.set_cur_size_and_mu();
        // Pass 1: convert noads (C §753).
        while q != TEX_NULL {
            let mut delta = 0;
            let mut flow = 0u8; // 0 = process nucleus; 2 = lab82; 3 = lab80; 4 = lab81
            'reswitch: loop {
                match self.mem[q as usize].b16_s1() {
                    BIN_NOAD => match r_type {
                        t if t == BIN_NOAD as i32
                            || t == OP_NOAD as i32
                            || t == REL_NOAD as i32
                            || t == OPEN_NOAD as i32
                            || t == PUNCT_NOAD as i32
                            || t == LEFT_NOAD as i32 =>
                        {
                            self.mem[q as usize].set_b16_s1(ORD_NOAD);
                            continue 'reswitch;
                        }
                        _ => {}
                    },
                    t if t == REL_NOAD || t == CLOSE_NOAD || t == PUNCT_NOAD || t == RIGHT_NOAD => {
                        if r_type == BIN_NOAD as i32 {
                            self.mem[r as usize].set_b16_s1(ORD_NOAD); // §755
                        }
                        if t == RIGHT_NOAD {
                            flow = 3;
                            break 'reswitch;
                        }
                    }
                    LEFT_NOAD => {
                        flow = 3;
                        break 'reswitch;
                    }
                    FRACTION_NOAD => {
                        self.make_fraction(q)?;
                        flow = 2;
                        break 'reswitch;
                    }
                    OP_NOAD => {
                        delta = self.make_op(q)?;
                        if self.mem[q as usize].b16_s0() == LIMITS {
                            flow = 2;
                            break 'reswitch;
                        }
                    }
                    ORD_NOAD => self.make_ord(q)?,
                    t if t == OPEN_NOAD || t == INNER_NOAD => {}
                    RADICAL_NOAD => self.make_radical(q)?,
                    OVER_NOAD => self.make_over(q)?,
                    UNDER_NOAD => self.make_under(q)?,
                    ACCENT_NOAD => self.make_math_accent(q)?,
                    VCENTER_NOAD => self.make_vcenter(q)?,
                    STYLE_NODE => {
                        self.cur_style = i32::from(self.mem[q as usize].b16_s0());
                        self.set_cur_size_and_mu();
                        flow = 4;
                        break 'reswitch;
                    }
                    CHOICE_NODE => {
                        let mut p = match self.cur_style / 2 {
                            0 => {
                                let p = self.mem[(q + 1) as usize].s0;
                                self.mem[(q + 1) as usize].s0 = TEX_NULL;
                                p
                            }
                            1 => {
                                let p = self.mem[(q + 1) as usize].s1;
                                self.mem[(q + 1) as usize].s1 = TEX_NULL;
                                p
                            }
                            2 => {
                                let p = self.mem[(q + 2) as usize].s0;
                                self.mem[(q + 2) as usize].s0 = TEX_NULL;
                                p
                            }
                            _ => {
                                let p = self.mem[(q + 2) as usize].s1;
                                self.mem[(q + 2) as usize].s1 = TEX_NULL;
                                p
                            }
                        };
                        let d = self.mem[(q + 1) as usize].s0;
                        self.flush_node_list(d)?;
                        let d = self.mem[(q + 1) as usize].s1;
                        self.flush_node_list(d)?;
                        let d = self.mem[(q + 2) as usize].s0;
                        self.flush_node_list(d)?;
                        let d = self.mem[(q + 2) as usize].s1;
                        self.flush_node_list(d)?;
                        self.mem[q as usize].set_b16_s1(STYLE_NODE);
                        self.mem[q as usize].set_b16_s0(self.cur_style as u16);
                        self.mem[(q + 1) as usize].s1 = 0;
                        self.mem[(q + 2) as usize].s1 = 0;
                        if p != TEX_NULL {
                            let z = self.mem[q as usize].s1;
                            self.mem[q as usize].s1 = p;
                            while self.mem[p as usize].s1 != TEX_NULL {
                                p = self.mem[p as usize].s1;
                            }
                            self.mem[p as usize].s1 = z;
                        }
                        flow = 4;
                        break 'reswitch;
                    }
                    t if t == INS_NODE
                        || t == MARK_NODE
                        || t == ADJUST_NODE
                        || t == WHATSIT_NODE
                        || t == PENALTY_NODE
                        || t == DISC_NODE =>
                    {
                        flow = 4;
                        break 'reswitch;
                    }
                    RULE_NODE => {
                        if self.mem[(q + 3) as usize].s1 > max_h {
                            max_h = self.mem[(q + 3) as usize].s1;
                        }
                        if self.mem[(q + 2) as usize].s1 > max_d {
                            max_d = self.mem[(q + 2) as usize].s1;
                        }
                        flow = 4;
                        break 'reswitch;
                    }
                    GLUE_NODE => {
                        if self.mem[q as usize].b16_s0() == MU_GLUE {
                            let x = self.mem[(q + 1) as usize].s0;
                            let y = self.math_glue(x, self.cur_mu)?;
                            self.delete_glue_ref(x)?;
                            self.mem[(q + 1) as usize].s0 = y;
                            self.mem[q as usize].set_b16_s0(NORMAL);
                        } else if self.cur_size != TEXT_SIZE
                            && self.mem[q as usize].b16_s0() == COND_MATH_GLUE
                        {
                            let p = self.mem[q as usize].s1;
                            if p != TEX_NULL {
                                let pt = self.mem[p as usize].b16_s1();
                                if pt == GLUE_NODE || pt == KERN_NODE {
                                    let plink = self.mem[p as usize].s1;
                                    self.mem[q as usize].s1 = plink;
                                    self.mem[p as usize].s1 = TEX_NULL;
                                    self.flush_node_list(p)?;
                                }
                            }
                        }
                        flow = 4;
                        break 'reswitch;
                    }
                    KERN_NODE => {
                        self.math_kern(q, self.cur_mu);
                        flow = 4;
                        break 'reswitch;
                    }
                    _ => return Err(self.confusion("mlist1")),
                }
                break 'reswitch;
            }
            if flow == 0 {
                // Nucleus conversion (C §754).
                let p;
                match self.mem[(q + 1) as usize].s1 {
                    MATH_CHAR | MATH_TEXT_CHAR => {
                        self.fetch(q + 1)?;
                        if self.is_aat_font(self.math_cur_f)
                            || self.using_open_type(self.math_cur_f)
                            || self.is_native_font(self.math_cur_f)
                        {
                            let z = self.new_native_character(self.math_cur_f, self.math_cur_c)?;
                            let p2 = self.get_node(GLYPH_NODE_SIZE)?;
                            self.mem[p2 as usize].set_b16_s1(WHATSIT_NODE);
                            self.mem[p2 as usize].set_b16_s0(GLYPH_NODE);
                            self.mem[(p2 + 4) as usize].set_b16_s2(self.math_cur_f.max(0) as u16);
                            let g = self.get_native_glyph(z, 0);
                            self.mem[(p2 + 4) as usize].set_b16_s1(g.max(0) as u16);
                            self.set_native_glyph_metrics(p2, true);
                            let zsize = i32::from(self.mem[(z + 4) as usize].b16_s3());
                            self.free_node(z, zsize)?;
                            delta = self.ot_math_ital_corr(
                                self.math_cur_f,
                                i32::from(self.mem[(p2 + 4) as usize].b16_s1()),
                            );
                            if self.mem[(q + 1) as usize].s1 == MATH_TEXT_CHAR
                                && !self.is_ot_math_font(self.math_cur_f)
                            {
                                delta = 0;
                            }
                            if self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY && delta != 0 {
                                let k = self.new_kern(delta)?;
                                self.mem[p2 as usize].s1 = k;
                                delta = 0;
                            }
                            p = p2;
                        } else if self.math_cur_i.skip > 0 {
                            delta = self
                                .effective_char_metric(self.math_cur_f, self.math_cur_c)
                                .italic;
                            let p2 = self.new_character(self.math_cur_f, self.math_cur_c)?;
                            if self.mem[(q + 1) as usize].s1 == MATH_TEXT_CHAR {
                                let space = self
                                    .font_record(self.math_cur_f)
                                    .and_then(|r| r.params.get(SPACE_CODE as usize))
                                    .copied()
                                    .unwrap_or(0);
                                if space != 0 {
                                    delta = 0;
                                }
                            }
                            if self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY && delta != 0 {
                                let k = self.new_kern(delta)?;
                                self.mem[p2 as usize].s1 = k;
                                delta = 0;
                            }
                            p = p2;
                        } else {
                            p = TEX_NULL;
                        }
                    }
                    MATH_FIELD_EMPTY => p = TEX_NULL,
                    SUB_BOX => p = self.mem[(q + 1) as usize].s0,
                    SUB_MLIST => {
                        self.cur_mlist = self.mem[(q + 1) as usize].s0;
                        let save_style = self.cur_style;
                        self.mlist_penalties = false;
                        self.mlist_to_hlist()?;
                        self.cur_style = save_style;
                        self.set_cur_size_and_mu();
                        p = self.hpack(self.mem[self.temp_head() as usize].s1, 0, ADDITIONAL)?;
                    }
                    _ => return Err(self.confusion("mlist2")),
                }
                self.mem[(q + 1) as usize].s1 = p;
                if self.mem[(q + 3) as usize].s1 == MATH_FIELD_EMPTY
                    && self.mem[(q + 2) as usize].s1 == MATH_FIELD_EMPTY
                {
                    flow = 2;
                } else {
                    self.make_scripts(q, delta)?;
                    flow = 2;
                }
            }
            if flow == 2 {
                // lab82: check dimensions.
                let z = self.hpack(self.mem[(q + 1) as usize].s1, 0, ADDITIONAL)?;
                if self.mem[(z + 3) as usize].s1 > max_h {
                    max_h = self.mem[(z + 3) as usize].s1;
                }
                if self.mem[(z + 2) as usize].s1 > max_d {
                    max_d = self.mem[(z + 2) as usize].s1;
                }
                self.free_node(z, BOX_NODE_SIZE)?;
            }
            if flow <= 3 {
                // lab80: done_with_noad.
                r = q;
                r_type = i32::from(self.mem[r as usize].b16_s1());
                if r_type == RIGHT_NOAD as i32 {
                    r_type = LEFT_NOAD as i32;
                    self.cur_style = style;
                    self.set_cur_size_and_mu();
                }
            }
            // lab81: done_with_node.
            q = self.mem[q as usize].s1;
        }
        if r_type == BIN_NOAD as i32 {
            self.mem[r as usize].set_b16_s1(ORD_NOAD); // §755
        }
        // Pass 2: spacing and penalties (C §760–762).
        let mut p = self.temp_head();
        self.mem[p as usize].s1 = TEX_NULL;
        let mut q = mlist;
        let mut r_type = 0i32;
        self.cur_style = style;
        self.set_cur_size_and_mu();
        'pass2: while q != TEX_NULL {
            let mut t = ORD_NOAD as i32;
            let mut s = NOAD_SIZE;
            let mut pen = INF_PENALTY;
            let mut delete_q_only = false;
            match self.mem[q as usize].b16_s1() {
                t2 if t2 == OP_NOAD
                    || t2 == OPEN_NOAD
                    || t2 == CLOSE_NOAD
                    || t2 == PUNCT_NOAD
                    || t2 == INNER_NOAD =>
                {
                    t = t2 as i32;
                }
                BIN_NOAD => {
                    t = BIN_NOAD as i32;
                    pen = self.int_par(INT_PAR_BIN_OP_PENALTY)?;
                }
                REL_NOAD => {
                    t = REL_NOAD as i32;
                    pen = self.int_par(INT_PAR_REL_PENALTY)?;
                }
                t2 if t2 == ORD_NOAD
                    || t2 == VCENTER_NOAD
                    || t2 == OVER_NOAD
                    || t2 == UNDER_NOAD => {}
                RADICAL_NOAD => s = RADICAL_NOAD_SIZE,
                ACCENT_NOAD => s = ACCENT_NOAD_SIZE,
                FRACTION_NOAD => s = FRACTION_NOAD_SIZE,
                t2 if t2 == LEFT_NOAD || t2 == RIGHT_NOAD => {
                    t = self.make_left_right(q, style, max_d, max_h)?;
                }
                STYLE_NODE => {
                    self.cur_style = i32::from(self.mem[q as usize].b16_s0());
                    s = STYLE_NODE_SIZE;
                    self.set_cur_size_and_mu();
                    delete_q_only = true;
                }
                t2 if t2 == WHATSIT_NODE
                    || t2 == PENALTY_NODE
                    || t2 == RULE_NODE
                    || t2 == DISC_NODE
                    || t2 == ADJUST_NODE
                    || t2 == INS_NODE
                    || t2 == MARK_NODE
                    || t2 == GLUE_NODE
                    || t2 == KERN_NODE =>
                {
                    self.mem[p as usize].s1 = q;
                    p = q;
                    q = self.mem[q as usize].s1;
                    self.mem[p as usize].s1 = TEX_NULL;
                    continue 'pass2;
                }
                _ => return Err(self.confusion("mlist3")),
            }
            if !delete_q_only {
                if r_type > 0 {
                    let row = OFFSET_TABLE[(r_type - ORD_NOAD as i32) as usize];
                    let entry = row.as_bytes()[(t - ORD_NOAD as i32) as usize] as char;
                    let mut x = match entry {
                        '0' => 0,
                        '1' => {
                            if self.cur_style < SCRIPT_STYLE {
                                GLUE_PAR_THIN_MU_SKIP
                            } else {
                                0
                            }
                        }
                        '2' => GLUE_PAR_THIN_MU_SKIP,
                        '3' => {
                            if self.cur_style < SCRIPT_STYLE {
                                GLUE_PAR_MED_MU_SKIP
                            } else {
                                0
                            }
                        }
                        '4' => {
                            if self.cur_style < SCRIPT_STYLE {
                                GLUE_PAR_THICK_MU_SKIP
                            } else {
                                0
                            }
                        }
                        _ => return Err(self.confusion("mlist4")),
                    };
                    if x != 0 {
                        let spec = self.eqtb[GLUE_PAR_BASE + x].s1;
                        let y = self.math_glue(spec, self.cur_mu)?;
                        let z = self.new_glue(y)?;
                        self.mem[y as usize].s1 = TEX_NULL;
                        self.mem[p as usize].s1 = z;
                        p = z;
                        self.mem[z as usize].set_b16_s0((x + 1) as u16);
                        x = 0;
                        let _ = x;
                    }
                }
                if self.mem[(q + 1) as usize].s1 != TEX_NULL {
                    self.mem[p as usize].s1 = self.mem[(q + 1) as usize].s1;
                    loop {
                        p = self.mem[p as usize].s1;
                        if self.mem[p as usize].s1 == TEX_NULL {
                            break;
                        }
                    }
                }
                if penalties && self.mem[q as usize].s1 != TEX_NULL && pen < INF_PENALTY {
                    let ntype = self.mem[self.mem[q as usize].s1 as usize].b16_s1();
                    if ntype != PENALTY_NODE && ntype != REL_NOAD {
                        let z = self.new_penalty(pen)?;
                        self.mem[p as usize].s1 = z;
                        p = z;
                    }
                }
                if self.mem[q as usize].b16_s1() == RIGHT_NOAD {
                    t = OPEN_NOAD as i32;
                }
                r_type = t;
            }
            // lab83: delete_q.
            let rr = q;
            q = self.mem[q as usize].s1;
            self.free_node(rr, s)?;
        }
        Ok(())
    }

    /// Selects or builds a delimiter of at least height `v` (`var_delimiter`).
    fn var_delimiter(&mut self, d: i32, s: i32, v: i32) -> Result<i32, TeXMouthError> {
        let mut f = 0; // FONT_BASE
        let mut w = 0;
        let mut large_attempt = false;
        let word = self.mem[d as usize];
        let mut z = i32::from(word.b16_s3()) % 256;
        let mut x = i32::from(word.b16_s2()) + i32::from(word.b16_s3()) / 256 * 65536;
        let mut c: i32 = 0;
        let mut ot_assembly: Option<OtMathAssembly> = None;
        let mut recipe_q: LigKernInsn = LigKernInsn::default();
        let b;
        'search: loop {
            if z != 0 || x != 0 {
                let mut zz = z + s + 256;
                loop {
                    zz -= 256;
                    let g = self.math_font(zz);
                    if g != 0 {
                        // §734
                        if self.using_open_type(g) {
                            let mx = self.map_char_to_glyph(g, x.max(0) as u32);
                            f = g;
                            c = mx;
                            w = 0;
                            let mut n = 0;
                            loop {
                                let (y, u) = self.ot_math_variant(g, mx, n, 0);
                                if u > w {
                                    c = y;
                                    w = u;
                                    if u >= v {
                                        break 'search;
                                    }
                                }
                                n += 1;
                                if u < 0 {
                                    break;
                                }
                            }
                            if let Some(asm) = self.ot_math_assembly(g, mx, 0) {
                                ot_assembly = Some(asm);
                                break 'search;
                            }
                        } else {
                            let mut y = x;
                            if y >= self.font_record(g).map_or(0, |r| r.bc as i32)
                                && y <= self.font_record(g).map_or(0, |r| r.ec as i32)
                            {
                                loop {
                                    let qi = self.math_char_info(g, y);
                                    if qi.skip > 0 {
                                        if i32::from(qi.op) % 4 == EXT_TAG as i32 {
                                            f = g;
                                            c = y;
                                            recipe_q = qi;
                                            break 'search;
                                        }
                                        let m = self.effective_char_metric(g, y);
                                        let u = m.height + m.depth;
                                        if u > w {
                                            f = g;
                                            c = y;
                                            w = u;
                                            if u >= v {
                                                break 'search;
                                            }
                                        }
                                        if i32::from(qi.op) % 4 == LIST_TAG as i32 {
                                            y = qi.remainder;
                                            continue;
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }
                    if zz < SCRIPT_SIZE {
                        break;
                    }
                }
            }
            if large_attempt {
                break;
            }
            large_attempt = true;
            z = i32::from(self.mem[d as usize].b16_s1()) % 256;
            x = i32::from(self.mem[d as usize].b16_s0())
                + i32::from(self.mem[d as usize].b16_s1()) / 256 * 65536;
        }
        // found:
        if f != 0 {
            if !self.using_open_type(f) {
                // §736
                if i32::from(recipe_q.op) % 4 == EXT_TAG as i32 {
                    // §739: assemble the extensible.
                    let bb = self.new_null_box()?;
                    self.mem[bb as usize].set_b16_s1(VLIST_NODE);
                    let r = self.exten_recipe(f, recipe_q.remainder);
                    // C recipe order: r.s0 = rep, r.s1 = top, r.s2 = bot, r.s3 = mid.
                    let mut cc = r[0];
                    let u = self.height_plus_depth(f, cc);
                    let mut w2 = 0;
                    let qq = self.effective_char_metric(f, cc);
                    self.mem[(bb + 1) as usize].s1 = qq.width + qq.italic;
                    cc = r[1];
                    if cc != 0 {
                        w2 += self.height_plus_depth(f, cc);
                    }
                    cc = r[2];
                    if cc != 0 {
                        w2 += self.height_plus_depth(f, cc);
                    }
                    cc = r[3];
                    if cc != 0 {
                        w2 += self.height_plus_depth(f, cc);
                    }
                    let mut n = 0;
                    if u > 0 {
                        while w2 < v {
                            w2 += u;
                            n += 1;
                            if r[2] != 0 {
                                w2 += u;
                            }
                        }
                    }
                    let cc = r[1];
                    if cc != 0 {
                        self.stack_into_box(bb, f, cc)?;
                    }
                    for _ in 0..n {
                        self.stack_into_box(bb, f, r[0])?;
                    }
                    let cc = r[2];
                    if cc != 0 {
                        self.stack_into_box(bb, f, cc)?;
                        for _ in 0..n {
                            self.stack_into_box(bb, f, r[0])?;
                        }
                    }
                    let cc = r[3];
                    if cc != 0 {
                        self.stack_into_box(bb, f, cc)?;
                    }
                    self.mem[(bb + 2) as usize].s1 = w2 - self.mem[(bb + 3) as usize].s1;
                    b = bb;
                } else {
                    b = self.char_box(f, c)?;
                }
            } else if let Some(asm) = ot_assembly {
                b = self.build_opentype_assembly(f, &asm, v, false)?;
            } else {
                let bb = self.new_null_box()?;
                self.mem[bb as usize].set_b16_s1(VLIST_NODE);
                let p = self.get_node(GLYPH_NODE_SIZE)?;
                self.mem[p as usize].set_b16_s1(WHATSIT_NODE);
                self.mem[p as usize].set_b16_s0(GLYPH_NODE);
                self.mem[(p + 4) as usize].set_b16_s2(f.max(0) as u16);
                self.mem[(p + 4) as usize].set_b16_s1(c.max(0) as u16);
                self.set_native_glyph_metrics(p, true);
                self.mem[(bb + 5) as usize].s1 = p;
                self.mem[(bb + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
                self.mem[(bb + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                self.mem[(bb + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
                b = bb;
            }
        } else {
            let bb = self.new_null_box()?;
            self.mem[(bb + 1) as usize].s1 = self.dimen_par(DIMEN_PAR_NULL_DELIMITER_SPACE)?;
            b = bb;
        }
        self.mem[(b + 4) as usize].s1 =
            (self.mem[(b + 3) as usize].s1 - self.mem[(b + 2) as usize].s1) / 2
                - self.axis_height(s);
        Ok(b)
    }

    /// Boxes a single character or glyph (`char_box`).
    fn char_box(&mut self, f: i32, c: i32) -> Result<i32, TeXMouthError> {
        let b = self.new_null_box()?;
        if self.is_aat_font(f) || self.using_open_type(f) || self.is_native_font(f) {
            let p = self.new_native_character(f, c)?;
            self.mem[(b + 5) as usize].s1 = p;
            self.mem[(b + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
            self.mem[(b + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
            if self.mem[(p + 2) as usize].s1 < 0 {
                self.mem[(b + 2) as usize].s1 = 0;
            } else {
                self.mem[(b + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
            }
        } else {
            let e = self.effective_char(f, c);
            let q = self.font_char_metric(f, e.max(0) as u32);
            self.mem[(b + 1) as usize].s1 = q.width + q.italic;
            self.mem[(b + 3) as usize].s1 = q.height;
            self.mem[(b + 2) as usize].s1 = q.depth;
            let p = self.get_avail()?;
            self.mem[p as usize].set_b16_s0(e.max(0) as u16);
            self.mem[p as usize].set_b16_s1(f.max(0) as u16);
            self.mem[(b + 5) as usize].s1 = p;
        }
        Ok(b)
    }

    /// Stacks one more `char_box` piece into extensible box `b`
    /// (`stack_into_box`).
    fn stack_into_box(&mut self, b: i32, f: i32, c: i32) -> Result<(), TeXMouthError> {
        let p = self.char_box(f, c)?;
        let list = self.mem[(b + 5) as usize].s1;
        self.mem[p as usize].s1 = list;
        self.mem[(b + 5) as usize].s1 = p;
        self.mem[(b + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
        Ok(())
    }

    /// `height_plus_depth`: height + depth of `effective_char(f, c)`.
    fn height_plus_depth(&mut self, f: i32, c: i32) -> i32 {
        let q = self.effective_char_metric(f, c);
        q.height + q.depth
    }

    /// Appends glyph `g` to box `b` (`stack_glyph_into_box`).
    fn stack_glyph_into_box(&mut self, b: i32, f: i32, g: i32) -> Result<(), TeXMouthError> {
        let p = self.get_node(GLYPH_NODE_SIZE)?;
        self.mem[p as usize].set_b16_s1(WHATSIT_NODE);
        self.mem[p as usize].set_b16_s0(GLYPH_NODE);
        self.mem[(p + 4) as usize].set_b16_s2(f.max(0) as u16);
        self.mem[(p + 4) as usize].set_b16_s1(g.max(0) as u16);
        self.set_native_glyph_metrics(p, true);
        if self.mem[b as usize].b16_s1() == HLIST_NODE {
            let q = self.mem[(b + 5) as usize].s1;
            if q == TEX_NULL {
                self.mem[(b + 5) as usize].s1 = p;
            } else {
                let mut q = q;
                while self.mem[q as usize].s1 != TEX_NULL {
                    q = self.mem[q as usize].s1;
                }
                self.mem[q as usize].s1 = p;
                if self.mem[(b + 3) as usize].s1 < self.mem[(p + 3) as usize].s1 {
                    self.mem[(b + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
                }
                if self.mem[(b + 2) as usize].s1 < self.mem[(p + 2) as usize].s1 {
                    self.mem[(b + 2) as usize].s1 = self.mem[(p + 2) as usize].s1;
                }
            }
        } else {
            let list = self.mem[(b + 5) as usize].s1;
            self.mem[p as usize].s1 = list;
            self.mem[(b + 5) as usize].s1 = p;
            self.mem[(b + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
            if self.mem[(b + 1) as usize].s1 < self.mem[(p + 1) as usize].s1 {
                self.mem[(b + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
            }
        }
        Ok(())
    }

    /// Inserts variable glue into box `b` (`stack_glue_into_box`).
    fn stack_glue_into_box(&mut self, b: i32, min: i32, max: i32) -> Result<(), TeXMouthError> {
        let q = self.new_spec(0)?;
        self.mem[(q + 1) as usize].s1 = min;
        self.mem[(q + 2) as usize].s1 = max - min;
        let p = self.new_glue(q)?;
        if self.mem[b as usize].b16_s1() == HLIST_NODE {
            let q = self.mem[(b + 5) as usize].s1;
            if q == TEX_NULL {
                self.mem[(b + 5) as usize].s1 = p;
            } else {
                let mut q = q;
                while self.mem[q as usize].s1 != TEX_NULL {
                    q = self.mem[q as usize].s1;
                }
                self.mem[q as usize].s1 = p;
            }
        } else {
            let list = self.mem[(b + 5) as usize].s1;
            self.mem[p as usize].s1 = list;
            self.mem[(b + 5) as usize].s1 = p;
            self.mem[(b + 3) as usize].s1 = self.mem[(p + 3) as usize].s1;
            self.mem[(b + 1) as usize].s1 = self.mem[(p + 1) as usize].s1;
        }
        Ok(())
    }

    /// Builds an extensible glyph assembly to size `s`
    /// (`build_opentype_assembly`).
    fn build_opentype_assembly(
        &mut self,
        f: i32,
        a: &OtMathAssembly,
        s: i32,
        horiz: bool,
    ) -> Result<i32, TeXMouthError> {
        let b = self.new_null_box()?;
        if horiz {
            self.mem[b as usize].set_b16_s1(HLIST_NODE);
        } else {
            self.mem[b as usize].set_b16_s1(VLIST_NODE);
        }
        let min_o = self.ot_min_connector_overlap(f);
        let mut n = -1i32;
        let mut no_extenders = true;
        let mut s_max;
        loop {
            n += 1;
            s_max = 0;
            let mut prev_o = 0;
            for part in &a.parts {
                if part.is_extender {
                    no_extenders = false;
                    for _ in 0..n {
                        let mut o = part.start_connector;
                        if min_o < o {
                            o = min_o;
                        }
                        if prev_o < o {
                            o = prev_o;
                        }
                        s_max = s_max - o + part.full_advance;
                        prev_o = part.end_connector;
                    }
                } else {
                    let mut o = part.start_connector;
                    if min_o < o {
                        o = min_o;
                    }
                    if prev_o < o {
                        o = prev_o;
                    }
                    s_max = s_max - o + part.full_advance;
                    prev_o = part.end_connector;
                }
            }
            if s_max >= s || no_extenders {
                break;
            }
        }
        let mut prev_o = 0;
        for part in &a.parts {
            if part.is_extender {
                for _ in 0..n {
                    let mut o = part.start_connector;
                    if prev_o < o {
                        o = prev_o;
                    }
                    let oo = o;
                    if min_o < o {
                        o = min_o;
                    }
                    if oo > 0 {
                        self.stack_glue_into_box(b, -oo, -o)?;
                    }
                    self.stack_glyph_into_box(b, f, part.glyph as i32)?;
                    prev_o = part.end_connector;
                }
            } else {
                let mut o = part.start_connector;
                if prev_o < o {
                    o = prev_o;
                }
                let oo = o;
                if min_o < o {
                    o = min_o;
                }
                if oo > 0 {
                    self.stack_glue_into_box(b, -oo, -o)?;
                }
                self.stack_glyph_into_box(b, f, part.glyph as i32)?;
                prev_o = part.end_connector;
            }
        }
        let mut p = self.mem[(b + 5) as usize].s1;
        let mut nat = 0;
        let mut str_ = 0;
        while p != TEX_NULL {
            if self.mem[p as usize].b16_s1() == WHATSIT_NODE {
                if horiz {
                    nat += self.mem[(p + 1) as usize].s1;
                } else {
                    nat += self.mem[(p + 3) as usize].s1 + self.mem[(p + 2) as usize].s1;
                }
            } else if self.mem[p as usize].b16_s1() == GLUE_NODE {
                let spec = self.mem[(p + 1) as usize].s0;
                nat += self.mem[(spec + 1) as usize].s1;
                str_ += self.mem[(spec + 2) as usize].s1;
            }
            p = self.mem[p as usize].s1;
        }
        if s > nat && str_ > 0 {
            let mut o = s - nat;
            if o > str_ {
                o = str_;
            }
            self.mem[(b + 5) as usize].set_b16_s0(NORMAL);
            self.mem[(b + 5) as usize].set_b16_s1(STRETCHING);
            let glue_set = o as f64 / str_ as f64;
            self.mem[(b + 6) as usize].set_gr(glue_set);
            if horiz {
                self.mem[(b + 1) as usize].s1 = nat + tex_round(str_ as f64 * glue_set);
            } else {
                self.mem[(b + 3) as usize].s1 = nat + tex_round(str_ as f64 * glue_set);
            }
        } else if horiz {
            self.mem[(b + 1) as usize].s1 = nat;
        } else {
            self.mem[(b + 3) as usize].s1 = nat;
        }
        Ok(b)
    }

    /// Repacks box `b` to width `w`, centering on `\hss` glue (`rebox`).
    fn rebox(&mut self, mut b: i32, w: i32) -> Result<i32, TeXMouthError> {
        if self.mem[(b + 1) as usize].s1 != w && self.mem[(b + 5) as usize].s1 != TEX_NULL {
            if self.mem[b as usize].b16_s1() == VLIST_NODE {
                b = self.hpack(b, 0, ADDITIONAL)?;
            }
            let mut p = self.mem[(b + 5) as usize].s1;
            if self.is_char_node(p) && self.mem[p as usize].s1 == TEX_NULL {
                let f = i32::from(self.mem[p as usize].b16_s1());
                let c = i32::from(self.mem[p as usize].b16_s0());
                let v = self.effective_char_metric(f, c).width;
                if v != self.mem[(b + 1) as usize].s1 {
                    let k = self.new_kern(self.mem[(b + 1) as usize].s1 - v)?;
                    self.mem[p as usize].s1 = k;
                }
            }
            self.free_node(b, BOX_NODE_SIZE)?;
            let spec = self.fil_glue()?;
            let glue = self.new_glue(spec)?;
            self.mem[glue as usize].s1 = p;
            while self.mem[p as usize].s1 != TEX_NULL {
                p = self.mem[p as usize].s1;
            }
            let spec = self.fil_glue()?;
            let tail = self.new_glue(spec)?;
            self.mem[p as usize].s1 = tail;
            Ok(self.hpack(b, w, EXACTLY)?)
        } else {
            self.mem[(b + 1) as usize].s1 = w;
            Ok(b)
        }
    }

    /// Starts a new paragraph (`new_graf`).
    fn new_graf(&mut self, indented: bool) -> Result<(), TeXMouthError> {
        self.prev_graf = 0;
        if self.mode == TeXMode::Vmode as i16 || self.head != self.tail {
            let parskip = self.new_param_glue(GLUE_PAR_PAR_SKIP)?;
            self.mem[self.tail as usize].s1 = parskip;
            self.tail = parskip;
        }
        self.push_nest()?;
        self.mode = TeXMode::Hmode as i16;
        self.aux.s0 = 1000;
        let language = self.int_par(INT_PAR_LANGUAGE)?;
        self.cur_lang = if !(1..=BIGGEST_LANG).contains(&language) {
            0
        } else {
            language
        };
        self.aux.s1 = self.cur_lang;
        self.prev_graf = (norm_min(self.int_par(INT_PAR_LEFT_HYPHEN_MIN)?) * 64
            + norm_min(self.int_par(INT_PAR_RIGHT_HYPHEN_MIN)?))
            * 65_536
            + self.cur_lang;
        if indented {
            let indent_box = self.new_null_box()?;
            self.mem[self.head as usize].s1 = indent_box;
            self.tail = indent_box;
            self.mem[indent_box as usize + 1].s1 = self.dimen_par(DIMEN_PAR_PAR_INDENT)?;
            if self.insert_src_special_every_par {
                self.insert_src_special()?;
            }
        }
        let every = self.local(LOCAL_EVERY_PAR)?;
        if every != TEX_NULL {
            self.begin_token_list(every, EVERY_PAR_TEXT)?;
        }
        if self.nest_ptr == 1 {
            self.build_page()?;
        }
        Ok(())
    }

    /// Handles `\indent` in horizontal or math mode (`indent_in_hmode`).
    fn indent_in_hmode(&mut self) -> Result<(), TeXMouthError> {
        let cur_chr = self.cur_chr;
        let glue = self.new_param_glue(if self.aux.s0 != 1000 {
            GLUE_PAR_SPACE_SKIP
        } else {
            GLUE_PAR_XSPACE_SKIP
        })?;
        self.mem[self.tail as usize].s1 = glue;
        self.tail = glue;
        if self.mode.abs() == TeXMode::Hmode as i16 {
            self.cur_lang = self.int_par(INT_PAR_LANGUAGE)?.clamp(0, BIGGEST_LANG);
        }
        if cur_chr == 0 {
            return Ok(());
        }
        self.push_nest()?;
        self.mode = -(TeXMode::Hmode as i16);
        self.aux.s0 = 1000;
        let indent = self.dimen_par(DIMEN_PAR_PAR_INDENT)?;
        let kern = if indent != 0 {
            Some(self.new_kern(indent)?)
        } else {
            None
        };
        let every = self.local(LOCAL_EVERY_PAR)?;
        if every != TEX_NULL {
            self.begin_token_list(every, EVERY_PAR_TEXT)?;
        }
        if let Some(kern) = kern {
            self.mem[self.tail as usize].s1 = kern;
            self.tail = kern;
        }
        Ok(())
    }

    /// Returns whether `cur_cmd` delivers a character directly
    /// (LETTER/OTHER_CHAR/CHAR_GIVEN).
    fn is_char_cmd(&self) -> bool {
        self.cur_cmd == CommandCode::Letter as u8
            || self.cur_cmd == CommandCode::OtherChar as u8
            || self.cur_cmd == CommandCode::CharGiven as u8
    }

    /// Updates `space_factor` from `sf_code(cur_chr)` (`main_s` logic,
    /// C ~17952).
    fn update_space_factor(&mut self) {
        let main_s = self.sf_code(self.cur_chr.max(0) as u32) % 65536;
        if main_s == 1000 {
            self.aux.s0 = 1000;
        } else if main_s < 1000 {
            if main_s > 0 {
                self.aux.s0 = main_s;
            }
        } else if self.aux.s0 < 1000 {
            self.aux.s0 = 1000;
        } else {
            self.aux.s0 = main_s;
        }
    }

    /// Backs up the pending token and starts the registered inter-char
    /// token list (`find_sa_element` hit in `collect_native`,
    /// `main_loop_lookahead_1`, `append_normal_space`, and the shared
    /// hmode end-of-word injection). Returns `true` when a token list was
    /// inserted (the caller `goto`s `big_switch`/`collected`).
    ///
    /// `force_other` matches the mid-word call sites, which force
    /// `cur_cmd` to `OTHER_CHAR` for anything but `LETTER`; end-of-word
    /// call sites keep the pending command and only normalize `CHAR_NUM`.
    fn inter_char_tokens(
        &mut self,
        n: i32,
        backed_up_char: bool,
        force_other: bool,
    ) -> Result<bool, TeXMouthError> {
        let p = self.find_sa_element(SA_INTER_CHAR_VAL, n, false)?;
        if p == TEX_NULL || self.sparse_ptr_value(p)? == TEX_NULL {
            return Ok(false);
        }
        if self.cur_cs == 0 {
            if (force_other && self.cur_cmd != CommandCode::Letter as u8)
                || self.cur_cmd == CommandCode::CharNum as u8
            {
                self.cur_cmd = CommandCode::OtherChar as u8;
            }
            self.cur_tok = i32::from(self.cur_cmd) * MAX_CHAR_VAL + self.cur_chr;
        } else {
            self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        }
        self.back_input()?;
        if backed_up_char {
            self.cur_input.index = BACKED_UP_CHAR;
        }
        self.begin_token_list(self.sparse_ptr_value(p)?, INTER_CHAR_TEXT)?;
        Ok(true)
    }

    /// Processes a scanned character in horizontal mode (`main_loop`,
    /// C ~17509). `cur_chr` holds the character code.
    fn main_loop_char(&mut self) -> Result<(), TeXMouthError> {
        // C `main_loop` head: a fresh nonempty-mode list records its source.
        if self.head == self.tail && self.mode > 0 && self.insert_src_special_auto {
            self.append_src_special()?;
        }
        self.prev_class = CHAR_CLASS_LIMIT - 1;
        let main_f = self.eqtb[CUR_FONT_LOC].s1;
        if self.is_native_font(main_f) {
            self.main_loop_native(main_f)
        } else {
            self.main_loop_tfm(main_f)
        }
    }

    /// Native-font word collection (`collect_native`/`collected`,
    /// C ~17536-17951).
    fn main_loop_native(&mut self, main_f: i32) -> Result<(), TeXMouthError> {
        const LIMIT: i32 = CHAR_CLASS_LIMIT;
        if self.mode > 0 && self.int_par(INT_PAR_LANGUAGE)? != self.aux.s1 {
            self.fix_language()?;
        }

        let inter_char = self.int_par(INT_PAR_XETEX_INTER_CHAR_TOKENS)? > 0;
        let hyphen = self
            .font_record(main_f)
            .map_or(i32::from(b'-'), |r| r.hyphen_char);
        let dash_break = self.int_par(INT_PAR_XETEX_DASH_BREAK)? > 0;
        let mut native_text: Vec<u16> = Vec::new();
        let mut main_h = 0i32;
        let mut is_hyph = false;
        let mut space_class: i32;

        // `collect_native:`
        loop {
            self.update_space_factor();
            self.cur_ptr = TEX_NULL;
            space_class = self.sf_code(self.cur_chr.max(0) as u32) / 65536;

            if inter_char && space_class != LIMIT {
                if self.prev_class == LIMIT - 1 {
                    if !(self.cur_input.state == TOKEN_LIST
                        && self.cur_input.index == BACKED_UP_CHAR)
                        && self.inter_char_tokens((LIMIT - 1) * LIMIT + space_class, true, true)?
                    {
                        return Ok(());
                    }
                } else if self.inter_char_tokens(
                    self.prev_class * LIMIT + space_class,
                    true,
                    true,
                )? {
                    self.prev_class = LIMIT - 1;
                    break;
                }
                self.prev_class = space_class;
            }

            if self.cur_chr > 65535 {
                native_text.push(((self.cur_chr - 65536) / 1024 + 0xD800) as u16);
                native_text.push(((self.cur_chr - 65536) % 1024 + 0xDC00) as u16);
            } else {
                native_text.push(self.cur_chr.max(0) as u16);
            }

            is_hyph = self.cur_chr == hyphen
                || (dash_break && (self.cur_chr == 8212 || self.cur_chr == 8211));
            if main_h == 0 && is_hyph {
                main_h = native_text.len() as i32;
            }

            self.get_next()?;
            if !self.is_char_cmd() {
                self.x_token()?;
                if !self.is_char_cmd() {
                    if self.cur_cmd == CommandCode::CharNum as u8 {
                        self.scan_usv_num()?;
                        self.cur_chr = self.cur_val;
                        continue;
                    }
                    if inter_char && space_class != LIMIT && self.prev_class != LIMIT - 1 {
                        self.prev_class = LIMIT - 1;
                        if self.inter_char_tokens(space_class * LIMIT + LIMIT - 1, false, false)? {
                            break;
                        }
                    }
                    break;
                }
            }
        }

        // `collected:`
        self.native_word_emit(main_f, native_text, main_h, is_hyph)
    }

    /// Emits collected UTF-16 text as native word nodes and
    /// discretionaries (`collected`, C ~17643-17951).
    fn native_word_emit(
        &mut self,
        main_f: i32,
        mut native_text: Vec<u16>,
        mut main_h: i32,
        is_hyph: bool,
    ) -> Result<(), TeXMouthError> {
        let hyphen = self
            .font_record(main_f)
            .map_or(i32::from(b'-'), |r| r.hyphen_char);
        let dash_break = self.int_par(INT_PAR_XETEX_DASH_BREAK)? > 0;

        // `apply_mapping` (`font_mapping[main_f]`).
        let mapped = self.apply_text_mapping(main_f, &native_text);
        if mapped != native_text {
            native_text = mapped;
            main_h = 0;
            for (i, &u) in native_text.iter().enumerate() {
                if main_h == 0
                    && (i32::from(u) == hyphen || (dash_break && (u == 8212 || u == 8211)))
                {
                    main_h = (i + 1) as i32;
                }
            }
        }

        // `char_warning` for unmapped characters (`tracing_lost_chars`).
        if self.int_par(INT_PAR_TRACING_LOST_CHARS)? > 0 {
            let mut i = 0usize;
            while i < native_text.len() {
                let mut scalar = i32::from(native_text[i]);
                i += 1;
                if (0xD800..0xDC00).contains(&scalar) && i < native_text.len() {
                    scalar = 65536 + (scalar - 0xD800) * 1024 + i32::from(native_text[i]) - 0xDC00;
                    i += 1;
                }
                if self.map_char_to_glyph(main_f, scalar.max(0) as u32) == 0 {
                    self.char_warning(main_f, scalar);
                }
            }
        }

        let mut main_k = native_text.len() as i32;
        let mut main_pp = self.tail;

        if self.mode == TeXMode::Hmode as i16 {
            // `main_ppp` = predecessor of `main_pp`, skipping past the
            // character chain of any DISC_NODE (`subtype` = chain length).
            let mut main_ppp = self.head;
            while main_ppp != main_pp && self.link(main_ppp)? != main_pp {
                if !self.is_char_node(main_ppp) && self.node_type(main_ppp)? == DISC_NODE {
                    let count = i32::from(self.word_at(main_ppp, 0)?.b16_s0());
                    for _ in 1..=count {
                        main_ppp = self.link(main_ppp)?;
                    }
                }
                if main_ppp != main_pp {
                    main_ppp = self.link(main_ppp)?;
                }
            }

            let mut temp_ptr = 0i32;
            loop {
                if main_h == 0 {
                    main_h = main_k;
                }
                if self.is_native_word_with_font(main_pp, main_f)?
                    && main_ppp != main_pp
                    && !self.is_char_node(main_ppp)
                    && self.node_type(main_ppp)? != DISC_NODE
                {
                    main_k = main_h + self.native_node_length(main_pp)?;
                    let save_native_len = native_text.len() as i32;
                    for i in 0..self.native_node_length(main_pp)? {
                        native_text.push(self.native_word_text_at(main_pp, i)?);
                    }
                    for i in 0..main_h {
                        native_text.push(native_text[(temp_ptr + i) as usize]);
                    }
                    self.do_locale_linebreaks(&native_text, save_native_len, main_k)?;
                    native_text.truncate(save_native_len as usize);
                    main_k = native_text.len() as i32 - main_h - temp_ptr;
                    temp_ptr = main_h;
                    main_h = 0;
                    while main_h < main_k
                        && i32::from(native_text[(temp_ptr + main_h) as usize]) != hyphen
                        && (!dash_break
                            || (native_text[(temp_ptr + main_h) as usize] != 8212
                                && native_text[(temp_ptr + main_h) as usize] != 8211))
                    {
                        main_h += 1;
                    }
                    if main_h < main_k {
                        main_h += 1;
                    }
                    self.set_link(main_ppp, self.link(main_pp)?)?;
                    self.set_link(main_pp, TEX_NULL)?;
                    self.flush_node_list(main_pp)?;
                    main_pp = self.tail;
                    while self.link(main_ppp)? != main_pp {
                        main_ppp = self.link(main_ppp)?;
                    }
                } else {
                    self.do_locale_linebreaks(&native_text, temp_ptr, main_h)?;
                    temp_ptr += main_h;
                    main_k -= main_h;
                    main_h = 0;
                    while main_h < main_k
                        && i32::from(native_text[(temp_ptr + main_h) as usize]) != hyphen
                        && (!dash_break
                            || (native_text[(temp_ptr + main_h) as usize] != 8212
                                && native_text[(temp_ptr + main_h) as usize] != 8211))
                    {
                        main_h += 1;
                    }
                    if main_h < main_k {
                        main_h += 1;
                    }
                }

                if main_k > 0 || is_hyph {
                    let disc = self.new_disc()?;
                    self.set_link(self.tail, disc)?;
                    self.tail = disc;
                    main_pp = self.tail;
                }
                if main_k == 0 {
                    break;
                }
            }
        } else {
            // Non-HMODE tail merge (C ~17800): concatenate into the
            // previous native word or append a new one.
            let mut main_ppp = self.head;
            while main_ppp != main_pp && self.link(main_ppp)? != main_pp {
                if !self.is_char_node(main_ppp) && self.node_type(main_ppp)? == DISC_NODE {
                    let count = i32::from(self.word_at(main_ppp, 0)?.b16_s0());
                    for _ in 1..=count {
                        main_ppp = self.link(main_ppp)?;
                    }
                }
                if main_ppp != main_pp {
                    main_ppp = self.link(main_ppp)?;
                }
            }
            if self.is_native_word_with_font(main_pp, main_f)?
                && main_ppp != main_pp
                && !self.is_char_node(main_ppp)
                && self.node_type(main_ppp)? != DISC_NODE
            {
                let prev_len = self.native_node_length(main_pp)?;
                let p = self.new_native_word_node(main_f, main_k + prev_len)?;
                self.set_link(main_pp, p)?;
                self.tail = p;
                for i in 0..prev_len {
                    let u = self.native_word_text_at(main_pp, i)?;
                    self.set_native_word_text_at(p, i, u)?;
                }
                for i in 0..main_k {
                    self.set_native_word_text_at(p, prev_len + i, native_text[i as usize])?;
                }
                self.set_native_metrics(p)?;
                let mut q = self.head;
                if q != main_pp {
                    while self.link(q)? != main_pp {
                        q = self.link(q)?;
                    }
                }
                self.set_link(q, self.link(main_pp)?)?;
                self.set_link(main_pp, TEX_NULL)?;
                self.flush_node_list(main_pp)?;
            } else {
                let p = self.new_native_word_node(main_f, main_k)?;
                self.set_link(main_pp, p)?;
                self.tail = p;
                for i in 0..main_k {
                    self.set_native_word_text_at(p, i, native_text[i as usize])?;
                }
                self.set_native_metrics(p)?;
            }
        }

        if self.int_par(INT_PAR_XETEX_INTERWORD_SPACE_SHAPING)? > 0 {
            self.interword_space_shaping_merge(main_f)?;
        }
        Ok(())
    }

    /// Splits `native_text[s..s+len]` into `NATIVE_WORD_NODE`s at ICU
    /// locale break offsets, inserting `xetex_linebreak_penalty` /
    /// `xetex_linebreak_skip` between pieces (`do_locale_linebreaks`,
    /// C ~11493).
    fn do_locale_linebreaks(
        &mut self,
        native_text: &[u16],
        s: i32,
        len: i32,
    ) -> Result<(), TeXMouthError> {
        let main_f = self.eqtb[CUR_FONT_LOC].s1;
        if len <= 0 {
            return Ok(());
        }
        if self.int_par(INT_PAR_XETEX_LINEBREAK_LOCALE)? == 0 || len == 1 {
            let p = self.new_native_word_node(main_f, len)?;
            self.set_link(self.tail, p)?;
            self.tail = p;
            for i in 0..len {
                self.set_native_word_text_at(p, i, native_text[(s + i) as usize])?;
            }
            self.set_native_metrics(p)?;
            return Ok(());
        }

        let use_skip = self.glue_par(GLUE_PAR_XETEX_LINEBREAK_SKIP)? != TEX_NULL;
        let penalty = self.int_par(INT_PAR_XETEX_LINEBREAK_PENALTY)?;
        let use_penalty = penalty != 0 || !use_skip;
        let locale = self.int_par(INT_PAR_XETEX_LINEBREAK_LOCALE)?;
        let text: Vec<u16> = native_text[s as usize..(s + len) as usize].to_vec();
        let offsets = self
            .locale_break_offsets
            .get(&(locale, text.clone()))
            .cloned()
            .unwrap_or_else(|| crate::unicode::linebreak_offsets(&text));

        let mut prev_offs = 0i32;
        for offs in offsets {
            if offs <= 0 || offs > len {
                continue;
            }
            if prev_offs != 0 {
                if use_penalty {
                    let p = self.new_penalty(penalty)?;
                    self.set_link(self.tail, p)?;
                    self.tail = p;
                }
                if use_skip {
                    let p = self.new_param_glue(GLUE_PAR_XETEX_LINEBREAK_SKIP)?;
                    self.set_link(self.tail, p)?;
                    self.tail = p;
                }
            }
            let p = self.new_native_word_node(main_f, offs - prev_offs)?;
            self.set_link(self.tail, p)?;
            self.tail = p;
            for i in prev_offs..offs {
                self.set_native_word_text_at(p, i - prev_offs, native_text[(s + i) as usize])?;
            }
            self.set_native_metrics(p)?;
            prev_offs = offs;
        }
        Ok(())
    }

    /// Merges a trailing space glue back into the previous native word
    /// when `\XeTeXinterwordspaceshaping` is on (C ~17871-17940).
    fn interword_space_shaping_merge(&mut self, main_f: i32) -> Result<(), TeXMouthError> {
        let mut main_p = self.head;
        let mut main_pp = TEX_NULL;
        while main_p != self.tail {
            if self.is_native_word_node(main_p)? {
                main_pp = main_p;
            }
            main_p = self.link(main_p)?;
        }
        if main_pp == TEX_NULL || self.native_node_font(main_pp)? != main_f {
            return Ok(());
        }
        let mut main_p = self.link(main_pp)?;
        while !self.is_char_node(main_p)
            && (self.node_type(main_p)? == PENALTY_NODE
                || self.node_type(main_p)? == INS_NODE
                || self.node_type(main_p)? == MARK_NODE
                || self.node_type(main_p)? == ADJUST_NODE
                || (self.node_type(main_p)? == WHATSIT_NODE
                    && self.node_subtype(main_p)? <= LANGUAGE_NODE))
        {
            main_p = self.link(main_p)?;
        }
        if self.is_char_node(main_p) || self.node_type(main_p)? != GLUE_NODE {
            return Ok(());
        }
        let mut main_ppp = self.link(main_p)?;
        while !self.is_char_node(main_ppp)
            && (self.node_type(main_ppp)? == PENALTY_NODE
                || self.node_type(main_ppp)? == INS_NODE
                || self.node_type(main_ppp)? == MARK_NODE
                || self.node_type(main_ppp)? == ADJUST_NODE
                || (self.node_type(main_ppp)? == WHATSIT_NODE
                    && self.node_subtype(main_ppp)? <= LANGUAGE_NODE))
        {
            main_ppp = self.link(main_ppp)?;
        }
        if main_ppp != self.tail || !self.is_native_word_node(main_ppp)? {
            return Ok(());
        }
        let len_pp = self.native_node_length(main_pp)?;
        let len_tail = self.native_node_length(main_ppp)?;
        let temp_ptr = self.new_native_word_node(main_f, len_pp + 1 + len_tail)?;
        let mut k = 0;
        for t in 0..len_pp {
            let u = self.native_word_text_at(main_pp, t)?;
            self.set_native_word_text_at(temp_ptr, k, u)?;
            k += 1;
        }
        self.set_native_word_text_at(temp_ptr, k, u16::from(b' '))?;
        k += 1;
        for t in 0..len_tail {
            let u = self.native_word_text_at(main_ppp, t)?;
            self.set_native_word_text_at(temp_ptr, k, u)?;
            k += 1;
        }
        self.set_native_metrics(temp_ptr)?;
        let t = self.word_at(temp_ptr, 1)?.s1
            - self.word_at(main_pp, 1)?.s1
            - self.word_at(main_ppp, 1)?.s1;
        let temp_size = i32::from(self.word_at(temp_ptr, 4)?.b16_s3());
        self.free_node(temp_ptr, temp_size)?;
        if let Some(font_glue) = self.font_record(main_f).map(|r| r.glue) {
            if font_glue != TEX_NULL {
                let glue_width = self.word_at(font_glue, 1)?.s1;
                if t != glue_width {
                    let kern = self.new_kern(t - glue_width)?;
                    self.set_node_subtype(kern, SPACE_ADJUSTMENT)?;
                    self.set_link(kern, self.link(main_p)?)?;
                    self.set_link(main_p, kern)?;
                }
            }
        }
        Ok(())
    }

    /// TFM branch of `main_loop` with the ligature/kern program
    /// (C ~17952-18073 plus the shared wrapup/move/lig labels).
    fn main_loop_tfm(&mut self, main_f: i32) -> Result<(), TeXMouthError> {
        const LIMIT: i32 = CHAR_CLASS_LIMIT;
        let inter_char = self.int_par(INT_PAR_XETEX_INTER_CHAR_TOKENS)? > 0;

        // Entry: space factor + inter-char injection for the first char.
        self.update_space_factor();
        self.cur_ptr = TEX_NULL;
        let space_class = self.sf_code(self.cur_chr.max(0) as u32) / 65536;
        if inter_char && space_class != LIMIT {
            if self.prev_class == LIMIT - 1 {
                if !(self.cur_input.state == TOKEN_LIST && self.cur_input.index == BACKED_UP_CHAR)
                    && self.inter_char_tokens((LIMIT - 1) * LIMIT + space_class, true, true)?
                {
                    return Ok(());
                }
            } else if self.inter_char_tokens(self.prev_class * LIMIT + space_class, true, true)? {
                self.prev_class = LIMIT - 1;
                return Ok(());
            }
            self.prev_class = space_class;
        }

        let (bchar_label, mut bchar, false_bchar) = self
            .font_record(main_f)
            .map_or((NON_ADDRESS, TOO_BIG_CHAR, TOO_BIG_CHAR), |r| {
                (r.bchar_label, r.bchar, r.false_bchar)
            });

        if self.mode > 0 && self.int_par(INT_PAR_LANGUAGE)? != self.aux.s1 {
            self.fix_language()?;
        }

        let hyphen = self
            .font_record(main_f)
            .map_or(i32::from(b'-'), |r| r.hyphen_char);

        // Ligature program state (`main_loop` locals).
        let mut lig_stack = self.get_avail()?;
        self.mem[lig_stack as usize].set_b16_s1(main_f as u16);
        let mut cur_l = self.cur_chr;
        self.mem[lig_stack as usize].set_b16_s0(cur_l as u16);
        let mut cur_q = self.tail;
        let mut ins_disc = false;
        let mut lft_hit = false;
        let mut rt_hit = false;
        let mut ligature_present = false;
        let mut cur_r = TOO_BIG_CHAR;

        let mut main_k = if self.cancel_boundary {
            self.cancel_boundary = false;
            NON_ADDRESS
        } else {
            bchar_label
        };

        // `main_i` equivalents are queried on demand through
        // `char_tag`/`char_lig_kern_start`/`effective_char_metric`.

        // `goto main_loop_move_2` or `main_lig_loop_1` for boundary char.
        #[derive(Clone, Copy, PartialEq)]
        enum Step {
            Wrapup,
            Move,
            Move1,
            MoveLig,
            Move2,
            Lookahead,
            LigLoop,
            LigLoop1,
            LigLoop2 { k: i32 },
            Done,
        }
        let mut step = if main_k == NON_ADDRESS {
            Step::Move2
        } else {
            cur_r = cur_l;
            cur_l = TOO_BIG_CHAR;
            Step::LigLoop1
        };

        while step != Step::Done {
            match step {
                Step::Wrapup => {
                    // `main_loop_wrapup` (C ~18039).
                    if cur_l < TOO_BIG_CHAR {
                        if self.link(cur_q)? > TEX_NULL
                            && self.mem[self.tail as usize].b16_s0() as i32 == hyphen
                        {
                            ins_disc = true;
                        }
                        if ligature_present {
                            let main_p = self.new_ligature(main_f, cur_l, self.link(cur_q)?)?;
                            if lft_hit {
                                self.set_node_subtype(main_p, 2)?;
                                lft_hit = false;
                            }
                            if rt_hit && lig_stack == TEX_NULL {
                                let sub = i32::from(self.word_at(main_p, 0)?.b16_s0()) + 1;
                                self.set_node_subtype(main_p, sub as u16)?;
                                rt_hit = false;
                            }
                            self.set_link(cur_q, main_p)?;
                            self.tail = main_p;
                            ligature_present = false;
                        }
                        if ins_disc {
                            ins_disc = false;
                            if self.mode > 0 {
                                let disc = self.new_disc()?;
                                self.set_link(self.tail, disc)?;
                                self.tail = disc;
                            }
                        }
                    }
                    step = Step::Move;
                }
                Step::Move => {
                    // `main_loop_move` (C ~18073): an empty stack means
                    // `goto reswitch` — the lookahead token is already in
                    // `cur_cmd`/`cur_chr` and must be dispatched, not
                    // re-read.
                    if lig_stack == TEX_NULL {
                        return self.dispatch_current_command();
                    }
                    cur_q = self.tail;
                    cur_l = i32::from(self.mem[lig_stack as usize].b16_s0());
                    step = Step::Move1;
                }
                Step::Move1 => {
                    // `main_loop_move_1`: a non-char lig_stack item means a
                    // deferred ligature/kern to move into the list.
                    if !self.is_char_node(lig_stack) {
                        step = Step::MoveLig;
                        continue;
                    }
                    step = Step::Move2;
                }
                Step::MoveLig => {
                    // `main_loop_move_lig` (C ~18369).
                    let main_p = self.lig_ptr(lig_stack)?;
                    if main_p > TEX_NULL {
                        self.set_link(self.tail, main_p)?;
                        self.tail = self.link(self.tail)?;
                    }
                    let temp = lig_stack;
                    lig_stack = self.link(temp)?;
                    self.free_node(temp, SMALL_NODE_SIZE)?;
                    self.effective_char(main_f, cur_l);
                    ligature_present = true;
                    if lig_stack == TEX_NULL {
                        if main_p > TEX_NULL {
                            step = Step::Lookahead;
                            continue;
                        }
                        cur_r = bchar;
                    } else {
                        cur_r = i32::from(self.mem[lig_stack as usize].b16_s0());
                    }
                    step = Step::LigLoop;
                }
                Step::Move2 => {
                    // `main_loop_move_2` (C ~18084).
                    let eff = self.effective_char(main_f, self.cur_chr);
                    let (bc, ec) = self
                        .font_record(main_f)
                        .map_or((0, 0), |r| (r.bc as i32, r.ec as i32));
                    if eff > ec || eff < bc || !self.char_exists(main_f, eff.max(0) as u32) {
                        self.char_warning(main_f, self.cur_chr);
                        self.set_link(lig_stack, self.avail)?;
                        self.avail = lig_stack;
                        return Ok(()); // goto big_switch
                    }
                    self.set_link(self.tail, lig_stack)?;
                    self.tail = lig_stack;
                    step = Step::Lookahead;
                }
                Step::Lookahead => {
                    // `main_loop_lookahead` (C ~18101).
                    self.get_next()?;
                    let mut have_char = self.is_char_cmd();
                    if !have_char {
                        self.x_token()?;
                        have_char = self.is_char_cmd();
                    }
                    if !have_char && self.cur_cmd == CommandCode::CharNum as u8 {
                        self.scan_char_num()?;
                        self.cur_chr = self.cur_val;
                        have_char = true;
                    }
                    if have_char {
                        // `main_loop_lookahead_1` (C ~18130).
                        self.update_space_factor();
                        self.cur_ptr = TEX_NULL;
                        let sc = self.sf_code(self.cur_chr.max(0) as u32) / 65536;
                        if inter_char && sc != LIMIT {
                            if self.prev_class == LIMIT - 1 {
                                if !(self.cur_input.state == TOKEN_LIST
                                    && self.cur_input.index == BACKED_UP_CHAR)
                                    && self.inter_char_tokens(
                                        (LIMIT - 1) * LIMIT + sc,
                                        true,
                                        true,
                                    )?
                                {
                                    return Ok(());
                                }
                            } else if self.inter_char_tokens(
                                self.prev_class * LIMIT + sc,
                                true,
                                true,
                            )? {
                                self.prev_class = LIMIT - 1;
                                return Ok(());
                            }
                            self.prev_class = sc;
                        }
                        lig_stack = self.get_avail()?;
                        self.mem[lig_stack as usize].set_b16_s1(main_f as u16);
                        cur_r = self.cur_chr;
                        self.mem[lig_stack as usize].set_b16_s0(cur_r as u16);
                        if cur_r == false_bchar {
                            cur_r = TOO_BIG_CHAR;
                        }
                        step = Step::LigLoop;
                    } else {
                        if self.cur_cmd == CommandCode::NoBoundary as u8 {
                            bchar = TOO_BIG_CHAR;
                        }
                        cur_r = bchar;
                        lig_stack = TEX_NULL;
                        step = Step::LigLoop;
                    }
                }
                Step::LigLoop => {
                    // `main_lig_loop` (C ~18199).
                    if self.char_tag(main_f, cur_l) != LIG_TAG || cur_r == TOO_BIG_CHAR {
                        step = Step::Wrapup;
                        continue;
                    }
                    main_k = self.char_lig_kern_start(main_f, cur_l);
                    let main_j = self.lig_kern_insn(main_f, main_k);
                    step = if main_j.skip <= 128 {
                        Step::LigLoop2 { k: main_k }
                    } else {
                        Step::LigLoop1
                    };
                    if let Step::LigLoop1 = step {
                        main_k = 256 * i32::from(main_j.op) + main_j.remainder + 32768 - 256 * 128;
                    }
                }
                Step::LigLoop1 => {
                    step = Step::LigLoop2 { k: main_k };
                }
                Step::LigLoop2 { k } => {
                    // `main_lig_loop_2` (C ~18216).
                    let main_j = self.lig_kern_insn(main_f, k);
                    if main_j.next_char == cur_r && main_j.skip <= 128 {
                        // Instruction matches.
                        if main_j.op >= 128 {
                            // Kern (C ~18220): wrap up current char, then
                            // append the kern node.
                            if cur_l < TOO_BIG_CHAR {
                                if self.link(cur_q)? > TEX_NULL
                                    && self.mem[self.tail as usize].b16_s0() as i32 == hyphen
                                {
                                    ins_disc = true;
                                }
                                if ligature_present {
                                    let main_p =
                                        self.new_ligature(main_f, cur_l, self.link(cur_q)?)?;
                                    if lft_hit {
                                        self.set_node_subtype(main_p, 2)?;
                                        lft_hit = false;
                                    }
                                    if rt_hit && lig_stack == TEX_NULL {
                                        let sub = i32::from(self.word_at(main_p, 0)?.b16_s0()) + 1;
                                        self.set_node_subtype(main_p, sub as u16)?;
                                        rt_hit = false;
                                    }
                                    self.set_link(cur_q, main_p)?;
                                    self.tail = main_p;
                                    ligature_present = false;
                                }
                                if ins_disc {
                                    ins_disc = false;
                                    if self.mode > 0 {
                                        let disc = self.new_disc()?;
                                        self.set_link(self.tail, disc)?;
                                        self.tail = disc;
                                    }
                                }
                            }
                            let kern = self.new_kern(self.kern_value(
                                main_f,
                                main_j.op,
                                main_j.remainder,
                            ))?;
                            self.set_link(self.tail, kern)?;
                            self.tail = kern;
                            step = Step::Move;
                            continue;
                        }

                        if cur_l == TOO_BIG_CHAR {
                            lft_hit = true;
                        } else if lig_stack == TEX_NULL {
                            rt_hit = true;
                        }

                        match main_j.op {
                            1 | 5 => {
                                cur_l = main_j.remainder;
                                self.effective_char(main_f, cur_l);
                                ligature_present = true;
                            }
                            2 | 6 => {
                                cur_r = main_j.remainder;
                                if lig_stack == TEX_NULL {
                                    lig_stack = self.new_lig_item(cur_r)?;
                                    bchar = TOO_BIG_CHAR;
                                } else if self.is_char_node(lig_stack) {
                                    let main_p = lig_stack;
                                    lig_stack = self.new_lig_item(cur_r)?;
                                    self.mem[(lig_stack + 1) as usize].s1 = main_p;
                                } else {
                                    self.mem[lig_stack as usize].set_b16_s0(cur_r as u16);
                                }
                            }
                            3 => {
                                cur_r = main_j.remainder;
                                let main_p = lig_stack;
                                lig_stack = self.new_lig_item(cur_r)?;
                                self.mem[lig_stack as usize].s1 = main_p;
                            }
                            7 | 11 => {
                                if cur_l < TOO_BIG_CHAR {
                                    if self.link(cur_q)? > TEX_NULL
                                        && self.mem[self.tail as usize].b16_s0() as i32 == hyphen
                                    {
                                        ins_disc = true;
                                    }
                                    if ligature_present {
                                        let main_p =
                                            self.new_ligature(main_f, cur_l, self.link(cur_q)?)?;
                                        if lft_hit {
                                            self.set_node_subtype(main_p, 2)?;
                                            lft_hit = false;
                                        }
                                        self.set_link(cur_q, main_p)?;
                                        self.tail = main_p;
                                    }
                                    if ins_disc {
                                        ins_disc = false;
                                        if self.mode > 0 {
                                            let disc = self.new_disc()?;
                                            self.set_link(self.tail, disc)?;
                                            self.tail = disc;
                                        }
                                    }
                                }
                                cur_q = self.tail;
                                cur_l = main_j.remainder;
                                self.effective_char(main_f, cur_l);
                                ligature_present = true;
                            }
                            _ => {
                                cur_l = main_j.remainder;
                                ligature_present = true;
                                if lig_stack == TEX_NULL {
                                    step = Step::Wrapup;
                                    continue;
                                }
                                step = Step::Move1;
                                continue;
                            }
                        }

                        if main_j.op > 4 && main_j.op != 7 {
                            step = Step::Wrapup;
                            continue;
                        }
                        if cur_l < TOO_BIG_CHAR {
                            step = Step::LigLoop;
                            continue;
                        }
                        main_k = bchar_label;
                        step = Step::LigLoop2 { k: main_k };
                    } else {
                        // No match: advance or terminate the program.
                        if main_j.skip == 0 {
                            main_k = k + 1;
                        } else {
                            if main_j.skip >= 128 {
                                step = Step::Wrapup;
                                continue;
                            }
                            main_k = k + i32::from(main_j.skip) + 1;
                        }
                        step = Step::LigLoop2 { k: main_k };
                    }
                }
                Step::Done => break,
            }
        }
        Ok(())
    }

    /// Inserts an italic-correction kern after a letter/`\char`
    /// (`\/` in HMODE, `append_italic_correction`, C ~15110).
    fn append_italic_correction(&mut self) -> Result<(), TeXMouthError> {
        if self.tail == self.head {
            return Ok(());
        }
        let p = self.tail;
        if self.is_char_node(p) {
            let f = i32::from(self.mem[p as usize].b16_s1());
            let c = i32::from(self.mem[p as usize].b16_s0());
            let eff = self.effective_char(f, c);
            let kern = self.new_kern(self.font_char_metric(f, eff.max(0) as u32).italic)?;
            self.set_link(self.tail, kern)?;
            self.tail = kern;
            self.set_node_subtype(kern, EXPLICIT)?;
        } else if self.node_type(p)? == LIGATURE_NODE {
            let f = self.lig_font(p)?;
            let c = self.lig_char(p)?;
            let eff = self.effective_char(f, c);
            let kern = self.new_kern(self.font_char_metric(f, eff.max(0) as u32).italic)?;
            self.set_link(self.tail, kern)?;
            self.tail = kern;
            self.set_node_subtype(kern, EXPLICIT)?;
        } else if self.node_type(p)? == WHATSIT_NODE {
            match self.node_subtype(p)? {
                NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT => {
                    let ic = self.get_native_italic_correction(p)?;
                    let kern = self.new_kern(ic)?;
                    self.set_link(self.tail, kern)?;
                    self.tail = kern;
                    self.set_node_subtype(kern, EXPLICIT)?;
                }
                GLYPH_NODE => {
                    let ic = self.get_native_glyph_italic_correction(p)?;
                    let kern = self.new_kern(ic)?;
                    self.set_link(self.tail, kern)?;
                    self.tail = kern;
                    self.set_node_subtype(kern, EXPLICIT)?;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Appends a `\discretionary` / `-` (`append_discretionary`,
    /// C ~15140).
    fn append_discretionary(&mut self) -> Result<(), TeXMouthError> {
        let disc = self.new_disc()?;
        self.set_link(self.tail, disc)?;
        self.tail = disc;
        if self.cur_chr == 1 {
            let c = self
                .font_record(self.eqtb[CUR_FONT_LOC].s1)
                .map_or(-1, |r| r.hyphen_char);
            if (0..TOO_BIG_CHAR).contains(&c) {
                let p = self.new_character(self.eqtb[CUR_FONT_LOC].s1, c)?;
                let idx = self.checked_engine_mem_index(self.tail + 1)?;
                self.mem[idx].s0 = p;
            }
        } else {
            self.push_save_entry(SaveStackEntry::RawValue(0))?;
            self.new_save_level(DISC_GROUP)?;
            self.scan_left_brace()?;
            self.push_nest()?;
            self.mode = -(TeXMode::Hmode as i16);
            self.aux.s0 = 1000;
        }
        Ok(())
    }

    /// Finishes one `\discretionary` sublist (`build_discretionary`,
    /// C ~15174). The `RawValue` under the boundary counts completed
    /// lists: 0 = pre_break, 1 = post_break, 2 = replacement text.
    fn build_discretionary(&mut self) -> Result<(), TeXMouthError> {
        self.unsave()?;
        let mut q = self.head;
        let mut p = self.link(q)?;
        let mut n = 0i32;
        while p != TEX_NULL {
            if !self.is_char_node(p)
                && self.node_type(p)? > RULE_NODE
                && self.node_type(p)? != KERN_NODE
                && self.node_type(p)? != LIGATURE_NODE
                && !(self.node_type(p)? == WHATSIT_NODE
                    && matches!(
                        self.node_subtype(p)?,
                        NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT | GLYPH_NODE
                    ))
            {
                self.transcript
                    .push("! Improper discretionary list".to_string());
                self.error_count += 1;
                self.flush_node_list(p)?;
                self.set_link(q, TEX_NULL)?;
                break;
            }
            q = p;
            p = self.link(q)?;
            n += 1;
        }

        let p = self.link(self.head)?;
        self.pop_nest()?;
        let slot = self.save_ptr.saturating_sub(1);
        let Some(SaveStackEntry::RawValue(count)) = self.save_stack.get(slot).copied() else {
            return Err(TeXMouthError::MalformedMacroParameters);
        };
        match count {
            0 => {
                let idx = self.checked_engine_mem_index(self.tail + 1)?;
                self.mem[idx].s0 = p;
            }
            1 => {
                let idx = self.checked_engine_mem_index(self.tail + 1)?;
                self.mem[idx].s1 = p;
            }
            _ => {
                if n > 0 && self.mode.abs() == TeXMode::Mmode as i16 {
                    self.transcript
                        .push("! Illegal math \\discretionary".to_string());
                    self.error_count += 1;
                    self.flush_node_list(p)?;
                    n = 0;
                } else {
                    self.set_link(self.tail, p)?;
                }
                if n <= i32::from(u16::MAX) {
                    self.set_node_subtype(self.tail, n as u16)?;
                } else {
                    self.transcript
                        .push("! Discretionary list is too long".to_string());
                    self.error_count += 1;
                }
                if n > 0 {
                    self.tail = q;
                }
                self.save_ptr -= 1;
                return Ok(());
            }
        }
        if let Some(entry) = self.save_stack.get_mut(slot) {
            *entry = SaveStackEntry::RawValue(count + 1);
        }
        self.new_save_level(DISC_GROUP)?;
        self.scan_left_brace()?;
        self.push_nest()?;
        self.mode = -(TeXMode::Hmode as i16);
        self.aux.s0 = 1000;
        Ok(())
    }

    /// `\accent` (`make_accent`, C ~15279).
    fn make_accent(&mut self) -> Result<(), TeXMouthError> {
        self.scan_char_num()?;
        let accent_char = self.cur_val;
        let mut f = self.eqtb[CUR_FONT_LOC].s1;
        let mut p = self.new_character(f, accent_char)?;
        if p == TEX_NULL {
            return Ok(());
        }

        let font_param = |record: Option<&crate::state::FontRecord>, code: i32| {
            record
                .and_then(|r| usize::try_from(code).ok().and_then(|i| r.params.get(i)))
                .copied()
                .unwrap_or(0)
        };
        let x = font_param(self.font_record(f), X_HEIGHT_CODE);
        let s = font_param(self.font_record(f), SLANT_CODE) as f64 / 65536.0;
        let (a, lsb, rsb) = if self.is_native_font(f) {
            let width = self.word_at(p, 1)?.s1;
            if width == 0 {
                let gid = self.map_char_to_glyph(f, accent_char.max(0) as u32).max(0) as u32;
                (
                    width,
                    self.get_glyph_bounds(f, 1, gid),
                    self.get_glyph_bounds(f, 3, gid),
                )
            } else {
                (width, 0, 0)
            }
        } else {
            let eff = self.effective_char(f, i32::from(self.mem[p as usize].b16_s0()));
            (self.font_char_metric(f, eff.max(0) as u32).width, 0, 0)
        };

        // `do_assignments`: execute pending prefixed commands until a
        // character token or another non-assignment command arrives.
        loop {
            self.get_x_token()?;
            if self.cur_cmd <= MAX_NON_PREFIXED_COMMAND {
                break;
            }
            self.prefixed_command()?;
        }

        let mut q = TEX_NULL;
        f = self.eqtb[CUR_FONT_LOC].s1;
        if self.is_char_cmd() {
            q = self.new_character(f, self.cur_chr)?;
            self.cur_val = self.cur_chr;
        } else if self.cur_cmd == CommandCode::CharNum as u8 {
            self.scan_char_num()?;
            q = self.new_character(f, self.cur_val)?;
        } else {
            self.back_input()?;
        }

        if q != TEX_NULL {
            let t = font_param(self.font_record(f), SLANT_CODE) as f64 / 65536.0;
            let (w, h) = if self.is_native_font(f) {
                let (h, _) = self.get_native_char_height_depth(f, self.cur_val.max(0) as u32);
                (self.word_at(q, 1)?.s1, h)
            } else {
                let eff = self.effective_char(f, i32::from(self.mem[q as usize].b16_s0()));
                let i = self.font_char_metric(f, eff.max(0) as u32);
                (i.width, i.height)
            };
            if h != x {
                p = self.hpack(p, 0, ADDITIONAL)?;
                self.mem[(p + 4) as usize].s1 = x - h;
            }
            let delta = if self.is_native_font(f) && a == 0 {
                tex_round((w - lsb + rsb) as f64 / 2.0 + h as f64 * t - x as f64 * s)
            } else {
                tex_round((w - a) as f64 / 2.0 + h as f64 * t - x as f64 * s)
            };
            let r = self.new_kern(delta)?;
            self.set_node_subtype(r, ACC_KERN)?;
            self.set_link(self.tail, r)?;
            self.set_link(r, p)?;
            let trailing = self.new_kern(-a - delta)?;
            self.set_node_subtype(trailing, ACC_KERN)?;
            self.tail = trailing;
            self.set_link(p, trailing)?;
            p = q;
        }
        self.set_link(self.tail, p)?;
        self.tail = p;
        self.aux.s0 = 1000;
        Ok(())
    }

    /// `\ ` in horizontal mode (`append_normal_space`,
    /// C ~18389-18430).
    fn append_normal_space(&mut self) -> Result<(), TeXMouthError> {
        const LIMIT: i32 = CHAR_CLASS_LIMIT;
        let inter_char = self.int_par(INT_PAR_XETEX_INTER_CHAR_TOKENS)? > 0;
        if inter_char && self.space_class != LIMIT && self.prev_class != LIMIT - 1 {
            self.prev_class = LIMIT - 1;
            if self.inter_char_tokens(self.space_class * LIMIT + LIMIT - 1, false, false)? {
                return Ok(());
            }
        }

        let temp_ptr = if self.glue_par(GLUE_PAR_SPACE_SKIP)? == TEX_NULL {
            let f = self.eqtb[CUR_FONT_LOC].s1;
            let font_glue = self.font_record(f).map(|r| r.glue);
            let main_p = match font_glue {
                Some(g) if g != TEX_NULL => g,
                _ => {
                    let p = self.new_spec(0)?;
                    let (space, stretch, shrink) = self.font_record(f).map_or((0, 0, 0), |r| {
                        (
                            r.params.get(SPACE_CODE as usize).copied().unwrap_or(0),
                            r.params
                                .get(SPACE_STRETCH_CODE as usize)
                                .copied()
                                .unwrap_or(0),
                            r.params
                                .get(SPACE_SHRINK_CODE as usize)
                                .copied()
                                .unwrap_or(0),
                        )
                    });
                    self.mem[(p + 1) as usize].s1 = space;
                    self.mem[(p + 2) as usize].s1 = stretch;
                    self.mem[(p + 3) as usize].s1 = shrink;
                    if let Some(record) = self.font_record_mut(f) {
                        record.glue = p;
                    }
                    p
                }
            };
            self.new_glue(main_p)?
        } else {
            self.new_param_glue(GLUE_PAR_SPACE_SKIP)?
        };
        self.set_link(self.tail, temp_ptr)?;
        self.tail = temp_ptr;
        Ok(())
    }

    /// Inter-word space after a `Spacer` token in HMODE (`app_space`,
    /// C ~14165).
    fn app_space(&mut self) -> Result<(), TeXMouthError> {
        let q = if self.aux.s0 >= 2000 && self.glue_par(GLUE_PAR_XSPACE_SKIP)? != TEX_NULL {
            self.new_param_glue(GLUE_PAR_XSPACE_SKIP)?
        } else if self.glue_par(GLUE_PAR_SPACE_SKIP)? != TEX_NULL {
            let main_p = self.glue_par(GLUE_PAR_SPACE_SKIP)?;
            let temp = self.mem[main_p as usize + 2].s1;
            let temp2 = self.mem[main_p as usize + 3].s1;
            self.mem[main_p as usize + 2].s1 = self
                .font_record(self.eqtb[CUR_FONT_LOC].s1)
                .and_then(|r| r.params.get(SPACE_STRETCH_CODE as usize))
                .copied()
                .unwrap_or(0);
            self.mem[main_p as usize + 3].s1 = self
                .font_record(self.eqtb[CUR_FONT_LOC].s1)
                .and_then(|r| r.params.get(SPACE_SHRINK_CODE as usize))
                .copied()
                .unwrap_or(0);
            let q = self.new_glue(main_p)?;
            self.mem[main_p as usize + 2].s1 = temp;
            self.mem[main_p as usize + 3].s1 = temp2;
            q
        } else {
            self.append_normal_space()?;
            return Ok(());
        };
        self.set_link(self.tail, q)?;
        self.tail = q;
        Ok(())
    }

    /// Starts an `\insert` or `\vadjust` vertical list
    /// (`begin_insert_or_adjust`).
    fn begin_insert_or_adjust(&mut self) -> Result<(), TeXMouthError> {
        let insert_class = if self.cur_cmd == CommandCode::Vadjust as u8 {
            255
        } else {
            self.scan_eight_bit_int()?;
            if self.cur_val == 255 {
                0
            } else {
                self.cur_val
            }
        };
        let pre_adjust =
            if self.cur_cmd == CommandCode::Vadjust as u8 && self.scan_keyword("pre")? {
                1
            } else {
                0
            };
        self.push_save_entry(SaveStackEntry::RawValue(insert_class))?;
        self.push_save_entry(SaveStackEntry::RawValue(pre_adjust))?;
        self.new_save_level(INSERT_GROUP)?;
        self.scan_left_brace()?;
        self.normal_paragraph()?;
        self.push_nest()?;
        self.mode = -1;
        self.aux.s1 = IGNORE_DEPTH;
        Ok(())
    }

    /// Closes an insertion or adjustment group (`denda`).
    fn denda(&mut self) -> Result<(), TeXMouthError> {
        self.end_graf()?;
        let split_top = self.glue_par(crate::state::GLUE_PAR_SPLIT_TOP_SKIP)?;
        self.add_glue_ref(split_top)?;
        let max_depth = self.dimen_par(crate::state::DIMEN_PAR_SPLIT_MAX_DEPTH)?;
        let floating_penalty = self.int_par(INT_PAR_FLOATING_PENALTY)?;
        self.unsave()?;
        self.save_ptr -= 2;
        let insert_class = match self.save_stack.get(self.save_ptr) {
            Some(SaveStackEntry::RawValue(value)) => *value,
            _ => {
                return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                    self.save_ptr as i32,
                )))
            }
        };
        let pre_adjust = match self.save_stack.get(self.save_ptr + 1) {
            Some(SaveStackEntry::RawValue(value)) => *value,
            _ => {
                return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                    self.save_ptr as i32 + 1,
                )))
            }
        };
        let boxed = self.vpackage(self.mem[self.head as usize].s1, 0, ADDITIONAL, MAX_HALFWORD)?;
        self.pop_nest()?;

        if insert_class < 255 {
            let ins = self.get_node(INS_NODE_SIZE)?;
            self.mem[self.tail as usize].s1 = ins;
            self.tail = ins;
            self.set_node_type(ins, INS_NODE)?;
            self.set_node_subtype(ins, insert_class as u16)?;
            self.mem[ins as usize + 3].s1 =
                self.mem[boxed as usize + 3].s1 + self.mem[boxed as usize + 2].s1;
            self.mem[ins as usize + 4].s0 = self.mem[boxed as usize + 5].s1;
            self.mem[ins as usize + 4].s1 = split_top;
            self.mem[ins as usize + 2].s1 = max_depth;
            self.mem[ins as usize + 1].s1 = floating_penalty;
        } else {
            let adjust = self.get_node(SMALL_NODE_SIZE)?;
            self.mem[self.tail as usize].s1 = adjust;
            self.tail = adjust;
            self.set_node_type(adjust, ADJUST_NODE)?;
            self.set_node_subtype(adjust, pre_adjust as u16)?;
            self.mem[adjust as usize + 1].s1 = self.mem[boxed as usize + 5].s1;
            self.delete_glue_ref(split_top)?;
        }
        self.free_node(boxed, BOX_NODE_SIZE)?;
        if self.nest_ptr == 0 {
            self.build_page()?;
        }
        Ok(())
    }

    /// Initializes current-page measurements when the first contribution is
    /// retained (`freeze_page_specs`).
    fn freeze_page_specs(&mut self, contents: u8) -> Result<(), TeXMouthError> {
        self.page_contents = contents;
        self.page_so_far[0] = self.dimen_par(DIMEN_PAR_VSIZE)?;
        self.page_max_depth = self.dimen_par(DIMEN_PAR_MAX_DEPTH)?;
        self.page_so_far[7] = 0;
        self.page_so_far[1..7].fill(0);
        self.least_page_cost = MAX_HALFWORD;
        Ok(())
    }

    /// Discards a box register after an output/box diagnostic (`box_error`).
    fn box_error(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let register = usize::try_from(n).map_err(|_| TeXMouthError::InputOutOfBounds(n))?;
        let boxed = self.box_reg(register)?;
        if boxed != TEX_NULL {
            self.flush_node_list(boxed)?;
            self.set_box_reg(register, TEX_NULL)?;
        }
        Ok(())
    }

    /// Verifies that an insertion class targets a vertical box (`ensure_vbox`).
    fn ensure_vbox(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let register = usize::try_from(n).map_err(|_| TeXMouthError::InputOutOfBounds(n))?;
        let boxed = self.box_reg(register)?;
        if boxed != TEX_NULL && self.node_type(boxed)? == HLIST_NODE {
            self.box_error(n)?;
        }
        Ok(())
    }

    /// Returns whether a page node can legally terminate a break candidate
    /// (`is_non_discardable_node`).
    fn is_non_discardable_node(&self, p: i32) -> Result<bool, TeXMouthError> {
        Ok(!self.is_char_node(p) && self.node_type(p)? < MATH_NODE)
    }

    /// Prepares and fires the output routine for the selected breakpoint
    /// (`fire_up`).
    fn fire_up(&mut self, c: i32) -> Result<(), TeXMouthError> {
        if self.best_page_break != TEX_NULL
            && self.node_type(self.best_page_break)? == crate::state::PENALTY_NODE
        {
            let penalty = self.mem[self.best_page_break as usize + 1].s1;
            self.geq_word_define(INT_PAR_BASE + INT_PAR_OUTPUT_PENALTY, penalty)?;
            self.mem[self.best_page_break as usize + 1].s1 = INF_PENALTY;
        } else {
            self.geq_word_define(INT_PAR_BASE + INT_PAR_OUTPUT_PENALTY, INF_PENALTY)?;
        }

        let mark_root = self.sa_root[SA_MARK_VAL as usize];
        if mark_root != TEX_NULL && self.do_marks(FIRE_UP_INIT, 0, mark_root)? {
            self.sa_root[SA_MARK_VAL as usize] = TEX_NULL;
        }
        if self.cur_mark[BOT_MARK_CODE as usize] != TEX_NULL {
            if self.cur_mark[TOP_MARK_CODE as usize] != TEX_NULL {
                self.delete_token_ref(self.cur_mark[TOP_MARK_CODE as usize])?;
            }
            self.cur_mark[TOP_MARK_CODE as usize] = self.cur_mark[BOT_MARK_CODE as usize];
            let top = self.cur_mark[TOP_MARK_CODE as usize];
            let top_idx = self.checked_engine_mem_index(top)?;
            self.mem[top_idx].s0 += 1;
            self.delete_token_ref(self.cur_mark[FIRST_MARK_CODE as usize])?;
            self.cur_mark[FIRST_MARK_CODE as usize] = TEX_NULL;
        }

        if c == self.best_page_break {
            self.best_page_break = TEX_NULL;
        }
        if self.box_reg(255)? != TEX_NULL {
            return Err(TeXMouthError::UnsupportedPrimitive("\\box255 is not void"));
        }
        self.insert_penalties = 0;
        let saved_split_top_skip = self.glue_par(GLUE_PAR_SPLIT_TOP_SKIP)?;
        let process_inserts =
            self.int_par(INT_PAR_HOLDING_INSERTS)? <= 0 && !self.semantic_pagination_enabled;
        let page_ins_head = self.page_ins_head();

        if process_inserts {
            let mut r = self.link(page_ins_head)?;
            while r != page_ins_head {
                if self.mem[r as usize + 2].s0 != TEX_NULL {
                    let n = usize::from(self.node_subtype(r)?);
                    self.ensure_vbox(n as i32)?;
                    if self.box_reg(n)? == TEX_NULL {
                        let null_box = self.new_null_box()?;
                        self.set_box_reg(n, null_box)?;
                    }
                    let mut p = self.box_reg(n)? + 5;
                    while self.link(p)? != TEX_NULL {
                        p = self.link(p)?;
                    }
                    self.mem[r as usize + 2].s1 = p;
                }
                r = self.link(r)?;
            }
        }

        let hold_head = self.hold_head();
        let mut q = hold_head;
        self.set_link(q, TEX_NULL)?;
        let mut prev_p = self.page_head();
        let mut p = self.link(prev_p)?;
        while p != self.best_page_break {
            if self.node_type(p)? == INS_NODE {
                if process_inserts {
                    let mut r = self.link(page_ins_head)?;
                    while self.node_subtype(r)? != self.node_subtype(p)? {
                        r = self.link(r)?;
                    }
                    let mut wait = self.mem[r as usize + 2].s0 == TEX_NULL;
                    if !wait {
                        let mut s = self.mem[r as usize + 2].s1;
                        self.set_link(s, self.mem[p as usize + 4].s0)?;
                        if self.mem[r as usize + 2].s0 == p {
                            if self.node_type(r)? == crate::state::SPLIT_UP
                                && self.mem[r as usize + 1].s0 == p
                                && self.mem[r as usize + 1].s1 != TEX_NULL
                            {
                                while self.link(s)? != self.mem[r as usize + 1].s1 {
                                    s = self.link(s)?;
                                }
                                self.set_link(s, TEX_NULL)?;
                                self.set_glue_par(
                                    GLUE_PAR_SPLIT_TOP_SKIP,
                                    self.mem[p as usize + 4].s1,
                                )?;
                                self.mem[p as usize + 4].s0 =
                                    self.prune_page_top(self.mem[r as usize + 1].s1, false)?;
                                if self.mem[p as usize + 4].s0 != TEX_NULL {
                                    self.temp_ptr = self.vpackage(
                                        self.mem[p as usize + 4].s0,
                                        0,
                                        ADDITIONAL,
                                        MAX_HALFWORD,
                                    )?;
                                    self.mem[p as usize + 3].s1 =
                                        self.mem[self.temp_ptr as usize + 3].s1
                                            + self.mem[self.temp_ptr as usize + 2].s1;
                                    self.free_node(self.temp_ptr, BOX_NODE_SIZE)?;
                                    wait = true;
                                }
                            }
                            self.mem[r as usize + 2].s0 = TEX_NULL;
                            let n = usize::from(self.node_subtype(r)?);
                            self.temp_ptr = self.mem[self.box_reg(n)? as usize + 5].s1;
                            self.free_node(self.box_reg(n)?, BOX_NODE_SIZE)?;
                            let repackaged =
                                self.vpackage(self.temp_ptr, 0, ADDITIONAL, MAX_HALFWORD)?;
                            self.set_box_reg(n, repackaged)?;
                        } else {
                            while self.link(s)? != TEX_NULL {
                                s = self.link(s)?;
                            }
                            self.mem[r as usize + 2].s1 = s;
                        }
                    }

                    self.set_link(prev_p, self.link(p)?)?;
                    self.set_link(p, TEX_NULL)?;
                    if wait {
                        self.set_link(q, p)?;
                        q = p;
                        self.insert_penalties += 1;
                    } else {
                        self.delete_glue_ref(self.mem[p as usize + 4].s1)?;
                        self.free_node(p, INS_NODE_SIZE)?;
                    }
                    p = prev_p;
                }
            } else if self.node_type(p)? == MARK_NODE {
                let mark_class = self.mem[p as usize + 1].s0;
                let mark = self.mem[p as usize + 1].s1;
                if mark_class != 0 {
                    let element = self.find_sa_element(SA_MARK_VAL, mark_class, true)?;
                    let element_idx = self.checked_engine_mem_index(element)?;
                    let mark_idx = self.checked_engine_mem_index(mark)?;
                    if self.mem[element_idx + 2].s1 == TEX_NULL {
                        self.mem[element_idx + 2].s1 = mark;
                        self.mem[mark_idx].s0 += 1;
                    }
                    if self.mem[element_idx + 3].s0 != TEX_NULL {
                        self.delete_token_ref(self.mem[element_idx + 3].s0)?;
                    }
                    let element_idx = self.checked_engine_mem_index(element)?;
                    self.mem[element_idx + 3].s0 = mark;
                    self.mem[mark_idx].s0 += 1;
                } else {
                    let mark_idx = self.checked_engine_mem_index(mark)?;
                    if self.cur_mark[FIRST_MARK_CODE as usize] == TEX_NULL {
                        self.cur_mark[FIRST_MARK_CODE as usize] = mark;
                        self.mem[mark_idx].s0 += 1;
                    }
                    if self.cur_mark[BOT_MARK_CODE as usize] != TEX_NULL {
                        self.delete_token_ref(self.cur_mark[BOT_MARK_CODE as usize])?;
                    }
                    self.cur_mark[BOT_MARK_CODE as usize] = mark;
                    self.mem[mark_idx].s0 += 1;
                }
            }
            prev_p = p;
            p = self.link(prev_p)?;
        }
        self.set_glue_par(GLUE_PAR_SPLIT_TOP_SKIP, saved_split_top_skip)?;

        let contrib_head = self.contrib_head();
        let page_head = self.page_head();
        if p != TEX_NULL {
            if self.link(contrib_head)? == TEX_NULL {
                if self.nest_ptr == 0 {
                    self.tail = self.page_tail;
                } else {
                    self.nest[0].tail = self.page_tail;
                }
            }
            self.set_link(self.page_tail, self.link(contrib_head)?)?;
            self.set_link(contrib_head, p)?;
            self.set_link(prev_p, TEX_NULL)?;
        }

        let saved_vbadness = self.int_par(INT_PAR_VBADNESS)?;
        let saved_vfuzz = self.dimen_par(DIMEN_PAR_VFUZZ)?;
        self.set_int_par(INT_PAR_VBADNESS, INF_BAD)?;
        self.set_dimen_par(DIMEN_PAR_VFUZZ, MAX_HALFWORD)?;
        let page_box = self.vpackage(
            self.link(page_head)?,
            self.best_size,
            EXACTLY,
            self.page_max_depth,
        )?;
        self.set_box_reg(255, page_box)?;
        self.set_int_par(INT_PAR_VBADNESS, saved_vbadness)?;
        self.set_dimen_par(DIMEN_PAR_VFUZZ, saved_vfuzz)?;

        if self.last_glue != MAX_HALFWORD {
            self.delete_glue_ref(self.last_glue)?;
        }
        self.page_contents = PAGE_EMPTY;
        self.page_tail = page_head;
        self.set_link(page_head, TEX_NULL)?;
        self.last_glue = MAX_HALFWORD;
        self.last_penalty = 0;
        self.last_kern = 0;
        self.last_node_type = -1;
        self.page_so_far[7] = 0;
        self.page_max_depth = 0;
        if q != hold_head {
            self.set_link(page_head, self.link(hold_head)?)?;
            self.page_tail = q;
        }

        let mut r = self.link(page_ins_head)?;
        while r != page_ins_head {
            let next = self.link(r)?;
            self.free_node(r, PAGE_INS_NODE_SIZE)?;
            r = next;
        }
        self.set_link(page_ins_head, page_ins_head)?;

        let mark_root = self.sa_root[SA_MARK_VAL as usize];
        if mark_root != TEX_NULL && self.do_marks(FIRE_UP_DONE, 0, mark_root)? {
            self.sa_root[SA_MARK_VAL as usize] = TEX_NULL;
        }
        if self.cur_mark[TOP_MARK_CODE as usize] != TEX_NULL
            && self.cur_mark[FIRST_MARK_CODE as usize] == TEX_NULL
        {
            self.cur_mark[FIRST_MARK_CODE as usize] = self.cur_mark[TOP_MARK_CODE as usize];
            let first = self.cur_mark[FIRST_MARK_CODE as usize];
            let first_idx = self.checked_engine_mem_index(first)?;
            self.mem[first_idx].s0 += 1;
        }

        let output_routine = self.local(LOCAL_OUTPUT_ROUTINE)?;
        if output_routine != TEX_NULL {
            if self.dead_cycles >= self.int_par(INT_PAR_MAX_DEAD_CYCLES)? {
                return Err(TeXMouthError::UnsupportedPrimitive("output loop"));
            }
            self.output_active = true;
            self.dead_cycles += 1;
            self.push_nest()?;
            self.mode = -(TeXMode::Vmode as i16);
            self.aux.s1 = IGNORE_DEPTH;
            self.mode_line = -self.line;
            self.begin_token_list(output_routine, OUTPUT_TEXT)?;
            self.new_save_level(OUTPUT_GROUP)?;
            self.normal_paragraph()?;
            self.scan_left_brace()?;
            return Ok(());
        }

        if self.link(page_head)? != TEX_NULL {
            if self.link(contrib_head)? == TEX_NULL {
                if self.nest_ptr == 0 {
                    self.tail = self.page_tail;
                } else {
                    self.nest[0].tail = self.page_tail;
                }
            } else {
                self.set_link(self.page_tail, self.link(contrib_head)?)?;
            }
            self.set_link(contrib_head, self.link(page_head)?)?;
            self.set_link(page_head, TEX_NULL)?;
            self.page_tail = page_head;
        }
        self.flush_node_list(self.disc_ptr[LAST_BOX_CODE as usize])?;
        self.disc_ptr[LAST_BOX_CODE as usize] = TEX_NULL;
        let shipped = self.box_reg(255)?;
        self.ship_out(shipped)?;
        self.set_box_reg(255, TEX_NULL)?;
        Ok(())
    }

    /// Runs the contribution transfer/output cycle (`build_page`).
    fn build_page(&mut self) -> Result<(), TeXMouthError> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum PageStep {
            Contribute,
            UpdateHeights,
            Discard,
        }

        let contrib_head = self.contrib_head();
        if self.link(contrib_head)? == TEX_NULL || self.output_active {
            return Ok(());
        }

        while self.link(contrib_head)? != TEX_NULL {
            let p = self.link(contrib_head)?;
            let p_type = self.node_type(p)?;

            if self.last_glue != MAX_HALFWORD {
                self.delete_glue_ref(self.last_glue)?;
            }
            self.last_penalty = 0;
            self.last_kern = 0;
            self.last_node_type = i32::from(p_type) + 1;
            if p_type == GLUE_NODE {
                self.last_glue = self.mem[p as usize + 1].s0;
                self.add_glue_ref(self.last_glue)?;
            } else {
                self.last_glue = MAX_HALFWORD;
                if p_type == crate::state::PENALTY_NODE {
                    self.last_penalty = self.mem[p as usize + 1].s1;
                } else if p_type == crate::state::KERN_NODE {
                    self.last_kern = self.mem[p as usize + 1].s1;
                }
            }

            let mut pi = INF_PENALTY;
            let mut step = PageStep::Contribute;
            match p_type {
                HLIST_NODE | VLIST_NODE | RULE_NODE => {
                    if self.page_contents < PAGE_BOX_THERE {
                        if self.page_contents == PAGE_EMPTY {
                            self.freeze_page_specs(PAGE_BOX_THERE)?;
                        } else {
                            self.page_contents = PAGE_BOX_THERE;
                        }
                        let q = self.new_skip_param(GLUE_PAR_TOP_SKIP)?;
                        let spec = self.mem[q as usize + 1].s0;
                        self.temp_ptr = spec;
                        let spec_width = self.mem[spec as usize + 1].s1;
                        let height = self.mem[p as usize + 3].s1;
                        self.mem[spec as usize + 1].s1 = if spec_width > height {
                            spec_width - height
                        } else {
                            0
                        };
                        self.set_link(q, p)?;
                        self.set_link(contrib_head, q)?;
                        continue;
                    }
                    self.page_so_far[1] += self.page_so_far[7] + self.mem[p as usize + 3].s1;
                    self.page_so_far[7] = self.mem[p as usize + 2].s1;
                    step = PageStep::Contribute;
                }
                WHATSIT_NODE => {
                    let subtype = self.node_subtype(p)?;
                    if subtype == PIC_NODE || subtype == PDF_NODE {
                        self.page_so_far[1] += self.page_so_far[7] + self.mem[p as usize + 3].s1;
                        self.page_so_far[7] = self.mem[p as usize + 2].s1;
                    }
                    step = PageStep::Contribute;
                }
                GLUE_NODE => {
                    if self.page_contents < PAGE_BOX_THERE {
                        step = PageStep::Discard;
                    } else if self.is_non_discardable_node(self.page_tail)? {
                        pi = 0;
                        step = PageStep::UpdateHeights;
                    } else {
                        step = PageStep::UpdateHeights;
                    }
                }
                crate::state::KERN_NODE => {
                    if self.page_contents < PAGE_BOX_THERE {
                        step = PageStep::Discard;
                    } else if self.link(p)? == TEX_NULL {
                        return Ok(());
                    } else {
                        if self.node_type(self.link(p)?)? == GLUE_NODE {
                            pi = 0;
                        }
                        step = PageStep::UpdateHeights;
                    }
                }
                crate::state::PENALTY_NODE => {
                    if self.page_contents < PAGE_BOX_THERE {
                        step = PageStep::Discard;
                    } else {
                        pi = self.mem[p as usize + 1].s1;
                    }
                }
                MARK_NODE => {
                    step = PageStep::Contribute;
                }
                INS_NODE => {
                    if self.page_contents == PAGE_EMPTY {
                        self.freeze_page_specs(PAGE_INSERTS_ONLY)?;
                    }
                    let n = self.node_subtype(p)?;
                    let mut r = self.page_ins_head();
                    while n >= self.node_subtype(self.link(r)?)? {
                        r = self.link(r)?;
                    }
                    if self.node_subtype(r)? != n {
                        let q = self.get_node(PAGE_INS_NODE_SIZE)?;
                        self.set_link(q, self.link(r)?)?;
                        self.set_link(r, q)?;
                        r = q;
                        self.set_node_subtype(r, n)?;
                        self.set_node_type(r, INSERTING)?;
                        self.ensure_vbox(i32::from(n))?;
                        let boxed = self.box_reg(usize::from(n))?;
                        self.mem[r as usize + 3].s1 = if boxed == TEX_NULL {
                            0
                        } else {
                            self.mem[boxed as usize + 3].s1 + self.mem[boxed as usize + 2].s1
                        };
                        self.mem[r as usize + 2].s0 = TEX_NULL;
                        let spec = self.skip(usize::from(n))?;
                        let count = self.count(usize::from(n))?;
                        let h = if count == 1000 {
                            self.mem[r as usize + 3].s1
                        } else {
                            self.x_over_n(self.mem[r as usize + 3].s1, 1000) * count
                        };
                        self.page_so_far[0] -= h + self.mem[spec as usize + 1].s1;
                        let stretch_order = usize::from(self.mem[spec as usize].b16_s1());
                        self.page_so_far[2 + stretch_order] += self.mem[spec as usize + 2].s1;
                        self.page_so_far[6] += self.mem[spec as usize + 3].s1;
                    }

                    if self.node_type(r)? == crate::state::SPLIT_UP {
                        self.insert_penalties += self.mem[p as usize + 1].s1;
                    } else {
                        self.mem[r as usize + 2].s1 = p;
                        let delta = self.page_so_far[0] - self.page_so_far[1] - self.page_so_far[7]
                            + self.page_so_far[6];
                        let count = self.count(usize::from(n))?;
                        let p_height = self.mem[p as usize + 3].s1;
                        let h = if count == 1000 {
                            p_height
                        } else {
                            self.x_over_n(p_height, 1000) * count
                        };
                        if (h <= 0 || h <= delta)
                            && p_height + self.mem[r as usize + 3].s1
                                <= self.dimen(usize::from(n))?
                        {
                            self.page_so_far[0] -= h;
                            self.mem[r as usize + 3].s1 += p_height;
                        } else {
                            let mut w = if count <= 0 {
                                MAX_HALFWORD
                            } else {
                                let available =
                                    self.page_so_far[0] - self.page_so_far[1] - self.page_so_far[7];
                                if count == 1000 {
                                    available
                                } else {
                                    self.x_over_n(available, count) * 1000
                                }
                            };
                            let remaining =
                                self.dimen(usize::from(n))? - self.mem[r as usize + 3].s1;
                            if w > remaining {
                                w = remaining;
                            }
                            let q = self.vert_break(
                                self.mem[p as usize + 4].s0,
                                w,
                                self.mem[p as usize + 2].s1,
                            )?;
                            self.mem[r as usize + 3].s1 += self.best_height_plus_depth;
                            if count != 1000 {
                                self.best_height_plus_depth =
                                    self.x_over_n(self.best_height_plus_depth, 1000) * count;
                            }
                            self.page_so_far[0] -= self.best_height_plus_depth;
                            self.set_node_type(r, crate::state::SPLIT_UP)?;
                            self.mem[r as usize + 1].s1 = q;
                            self.mem[r as usize + 1].s0 = p;
                            if q == TEX_NULL {
                                self.insert_penalties += EJECT_PENALTY;
                            } else if self.node_type(q)? == crate::state::PENALTY_NODE {
                                self.insert_penalties += self.mem[q as usize + 1].s1;
                            }
                        }
                    }
                    step = PageStep::Contribute;
                }
                _ => {
                    return Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(p)));
                }
            }

            if pi < INF_PENALTY {
                let mut b = if self.page_so_far[1] < self.page_so_far[0] {
                    if self.page_so_far[3] != 0
                        || self.page_so_far[4] != 0
                        || self.page_so_far[5] != 0
                    {
                        0
                    } else {
                        badness(
                            self.page_so_far[0] - self.page_so_far[1],
                            self.page_so_far[2],
                        )
                    }
                } else if self.page_so_far[1] - self.page_so_far[0] > self.page_so_far[6] {
                    MAX_HALFWORD
                } else {
                    badness(
                        self.page_so_far[1] - self.page_so_far[0],
                        self.page_so_far[6],
                    )
                };
                if b >= MAX_HALFWORD {
                    b = MAX_HALFWORD;
                }
                let mut c = if b < MAX_HALFWORD {
                    if pi <= EJECT_PENALTY {
                        pi
                    } else if b < INF_BAD {
                        b + pi + self.insert_penalties
                    } else {
                        100_000
                    }
                } else {
                    b
                };
                if self.insert_penalties >= 10_000 {
                    c = MAX_HALFWORD;
                }
                if c <= self.least_page_cost {
                    self.best_page_break = p;
                    self.best_size = self.page_so_far[0];
                    self.least_page_cost = c;
                    let mut r = self.link(self.page_ins_head())?;
                    while r != self.page_ins_head() {
                        self.mem[r as usize + 2].s0 = self.mem[r as usize + 2].s1;
                        r = self.link(r)?;
                    }
                }
                if c == MAX_HALFWORD || pi <= EJECT_PENALTY {
                    self.fire_up(p)?;
                    if self.output_active {
                        return Ok(());
                    }
                    break;
                }
            }

            if !(GLUE_NODE..=crate::state::KERN_NODE).contains(&p_type) {
                step = PageStep::Contribute;
            }
            if step == PageStep::UpdateHeights {
                let width = if p_type == crate::state::KERN_NODE {
                    self.mem[p as usize + 1].s1
                } else {
                    let spec = self.mem[p as usize + 1].s0;
                    let stretch_order = usize::from(self.mem[spec as usize].b16_s1());
                    self.page_so_far[2 + stretch_order] += self.mem[spec as usize + 2].s1;
                    self.page_so_far[6] += self.mem[spec as usize + 3].s1;
                    if self.mem[spec as usize].b16_s0() != NORMAL
                        && self.mem[spec as usize + 3].s1 != 0
                    {
                        let replacement = self.new_spec(spec)?;
                        self.mem[replacement as usize].set_b16_s0(NORMAL);
                        self.delete_glue_ref(spec)?;
                        self.mem[p as usize + 1].s0 = replacement;
                    }
                    self.mem[self.mem[p as usize + 1].s0 as usize + 1].s1
                };
                self.page_so_far[1] += self.page_so_far[7] + width;
                self.page_so_far[7] = 0;
                step = PageStep::Contribute;
            }

            match step {
                PageStep::Contribute => {
                    if self.page_so_far[7] > self.page_max_depth {
                        self.page_so_far[1] += self.page_so_far[7] - self.page_max_depth;
                        self.page_so_far[7] = self.page_max_depth;
                    }
                    self.set_link(self.page_tail, p)?;
                    self.page_tail = p;
                    self.set_link(contrib_head, self.link(p)?)?;
                    self.set_link(p, TEX_NULL)?;
                }
                PageStep::Discard => {
                    self.set_link(contrib_head, self.link(p)?)?;
                    self.set_link(p, TEX_NULL)?;
                    if self.int_par(INT_PAR_SAVING_VDISCARDS)? <= 0 {
                        self.flush_node_list(p)?;
                    } else {
                        if self.disc_ptr[LAST_BOX_CODE as usize] == TEX_NULL {
                            self.disc_ptr[LAST_BOX_CODE as usize] = p;
                        } else {
                            let tail = self.disc_ptr[COPY_CODE as usize];
                            self.set_link(tail, p)?;
                        }
                        self.disc_ptr[COPY_CODE as usize] = p;
                    }
                }
                PageStep::UpdateHeights => unreachable!(),
            }
        }

        if self.nest_ptr == 0 {
            self.tail = contrib_head;
        } else {
            self.nest[0].tail = contrib_head;
        }
        Ok(())
    }

    /// Replaces a glue spec having infinite shrink order and nonzero
    /// shrink with a finite copy (`finite_shrink`), issuing the
    /// "Infinite glue shrinkage" diagnostic once per paragraph.
    fn finite_shrink(&mut self, p: i32) -> Result<i32, TeXMouthError> {
        if self.lb_no_shrink_error_yet {
            self.lb_no_shrink_error_yet = false;
            self.transcript
                .push("! Infinite glue shrinkage found in a paragraph.".to_string());
            self.error_count += 1;
        }
        let q = self.new_spec(p)?;
        self.mem[q as usize].set_b16_s0(NORMAL);
        self.delete_glue_ref(p)?;
        Ok(q)
    }

    /// Width contribution of one node during breakpoint arithmetic:
    /// characters and ligatures use `effective_char` metrics; boxes,
    /// rules, kerns, and XeTeX whatsits use `BOX_width`.
    fn lb_node_width(&mut self, p: i32) -> Result<i32, TeXMouthError> {
        if self.is_char_node(p) {
            let f = i32::from(self.mem[p as usize].b16_s1());
            let eff = self.effective_char(f, i32::from(self.mem[p as usize].b16_s0()));
            return Ok(self.font_char_metric(f, eff.max(0) as u32).width);
        }
        match self.node_type(p)? {
            LIGATURE_NODE => {
                let f = self.lig_font(p)?;
                self.xtx_ligature_present = true;
                let eff = self.effective_char(f, self.lig_char(p)?);
                Ok(self.font_char_metric(f, eff.max(0) as u32).width)
            }
            HLIST_NODE | VLIST_NODE | RULE_NODE | KERN_NODE => Ok(self.word_at(p, 1)?.s1),
            WHATSIT_NODE => {
                let subtype = self.node_subtype(p)?;
                if matches!(
                    subtype,
                    NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT | GLYPH_NODE | PIC_NODE | PDF_NODE
                ) {
                    Ok(self.word_at(p, 1)?.s1)
                } else {
                    Err(TeXMemoryError::CorruptedNode(p).into())
                }
            }
            _ => Err(TeXMemoryError::CorruptedNode(p).into()),
        }
    }

    /// Tests whether `lb_cur_p` is a feasible breakpoint for each active
    /// node, recording the best breaks per fitness class (`try_break`).
    fn try_break(&mut self, mut pi: i32, break_type: u16) -> Result<(), TeXMouthError> {
        if self.semantic_pagination_enabled && self.lb_cur_p != TEX_NULL {
            return Ok(());
        }
        if pi.abs() >= INF_PENALTY {
            if pi > 0 {
                return Ok(());
            }
            pi = EJECT_PENALTY;
        }

        let active_list = self.active_list();
        let mut no_break_yet = true;
        let mut prev_r = active_list;
        let mut prev_prev_r = TEX_NULL;
        let mut old_l: i32 = 0;
        let mut line_width = 0;
        let mut g = 0;
        self.lb_cur_active_width = self.active_width;

        loop {
            let r = self.link(prev_r)?;

            if self.node_type(r)? == DELTA_NODE {
                for i in 1usize..=6 {
                    self.lb_cur_active_width[i] += self.word_at(r, i as i32)?.s1;
                }
                prev_prev_r = prev_r;
                prev_r = r;
                continue;
            }

            let l = self.word_at(r, 1)?.s0;

            if l > old_l {
                if self.lb_minimum_demerits < AWFUL_BAD
                    && (old_l != self.lb_easy_line || r == active_list)
                {
                    if no_break_yet {
                        no_break_yet = false;
                        self.lb_break_width = self.lb_background;
                        let mut s = self.lb_cur_p;
                        if break_type > UNHYPHENATED && self.lb_cur_p != TEX_NULL {
                            let mut t = i32::from(self.mem[self.lb_cur_p as usize].b16_s0());
                            let mut v = self.lb_cur_p;
                            s = self.word_at(self.lb_cur_p, 1)?.s1;
                            while t > 0 {
                                t -= 1;
                                v = self.link(v)?;
                                let w = self.lb_node_width(v)?;
                                self.lb_break_width[1] -= w;
                            }
                            while s != TEX_NULL {
                                let w = self.lb_node_width(s)?;
                                self.lb_break_width[1] += w;
                                s = self.link(s)?;
                            }
                            self.lb_break_width[1] += self.lb_disc_width;
                            if self.word_at(self.lb_cur_p, 1)?.s1 == TEX_NULL {
                                s = self.link(v)?;
                            }
                        }
                        while s != TEX_NULL {
                            if self.is_char_node(s) {
                                break;
                            }
                            match self.node_type(s)? {
                                GLUE_NODE => {
                                    let v = self.word_at(s, 1)?.s0;
                                    self.lb_break_width[1] -= self.word_at(v, 1)?.s1;
                                    let order = usize::from(self.mem[v as usize].b16_s1());
                                    self.lb_break_width[2 + order] -= self.word_at(v, 2)?.s1;
                                    self.lb_break_width[6] -= self.word_at(v, 3)?.s1;
                                }
                                PENALTY_NODE => {}
                                MATH_NODE => self.lb_break_width[1] -= self.word_at(s, 1)?.s1,
                                KERN_NODE => {
                                    if self.node_subtype(s)? != EXPLICIT {
                                        break;
                                    }
                                    self.lb_break_width[1] -= self.word_at(s, 1)?.s1;
                                }
                                _ => break,
                            }
                            s = self.link(s)?;
                        }
                    }

                    if self.node_type(prev_r)? == DELTA_NODE {
                        for i in 1usize..=6 {
                            self.mem[(prev_r + i as i32) as usize].s1 +=
                                self.lb_break_width[i] - self.lb_cur_active_width[i];
                        }
                    } else if prev_r == active_list {
                        self.active_width = self.lb_break_width;
                    } else {
                        let q = self.get_node(DELTA_NODE_SIZE)?;
                        self.set_link(q, r)?;
                        self.set_node_type(q, DELTA_NODE)?;
                        self.set_node_subtype(q, 0)?;
                        for i in 1usize..=6 {
                            self.mem[(q + i as i32) as usize].s1 =
                                self.lb_break_width[i] - self.lb_cur_active_width[i];
                        }
                        self.set_link(prev_r, q)?;
                        prev_prev_r = prev_r;
                        prev_r = q;
                    }

                    let adj = self.int_par(INT_PAR_ADJ_DEMERITS)?.abs();
                    if adj >= MAX_HALFWORD - self.lb_minimum_demerits {
                        self.lb_minimum_demerits = AWFUL_BAD - 1;
                    } else {
                        self.lb_minimum_demerits += adj;
                    }

                    for fit_class in VERY_LOOSE_FIT..=TIGHT_FIT {
                        let fc = usize::from(fit_class);
                        if self.lb_minimal_demerits[fc] <= self.lb_minimum_demerits {
                            let q = self.get_node(PASSIVE_NODE_SIZE)?;
                            self.set_link(q, self.lb_passive)?;
                            self.lb_passive = q;
                            self.mem[(q + 1) as usize].s1 = self.lb_cur_p;
                            self.mem[(q + 1) as usize].s0 = self.lb_best_place[fc];

                            let q = self.get_node(self.lb_active_node_size)?;
                            self.mem[(q + 1) as usize].s1 = self.lb_passive;
                            self.mem[(q + 1) as usize].s0 = self.lb_best_pl_line[fc] + 1;
                            self.mem[q as usize].set_b16_s0(fit_class);
                            self.set_node_type(q, break_type)?;
                            self.mem[(q + 2) as usize].s1 = self.lb_minimal_demerits[fc];
                            if self.lb_do_last_line_fit {
                                self.mem[(q + 3) as usize].s1 = self.lb_best_pl_short[fc];
                                self.mem[(q + 4) as usize].s1 = self.lb_best_pl_glue[fc];
                            }
                            self.set_link(q, r)?;
                            self.set_link(prev_r, q)?;
                            prev_r = q;
                        }
                        self.lb_minimal_demerits[fc] = MAX_HALFWORD;
                    }
                    self.lb_minimum_demerits = MAX_HALFWORD;

                    if r != active_list {
                        let q = self.get_node(DELTA_NODE_SIZE)?;
                        self.set_link(q, r)?;
                        self.set_node_type(q, DELTA_NODE)?;
                        self.set_node_subtype(q, 0)?;
                        for i in 1usize..=6 {
                            self.mem[(q + i as i32) as usize].s1 =
                                self.lb_cur_active_width[i] - self.lb_break_width[i];
                        }
                        self.set_link(prev_r, q)?;
                        prev_prev_r = prev_r;
                        prev_r = q;
                    }
                }

                if r == active_list {
                    return Ok(());
                }

                if l > self.lb_easy_line {
                    line_width = self.lb_second_width;
                    old_l = MAX_HALFWORD - 1;
                } else {
                    old_l = l;
                    if l > self.lb_last_special_line {
                        line_width = self.lb_second_width;
                    } else if self.local(LOCAL_PAR_SHAPE)? == TEX_NULL {
                        line_width = self.lb_first_width;
                    } else {
                        line_width = self.mem[(self.local(LOCAL_PAR_SHAPE)? + 2 * l) as usize].s1;
                    }
                }
            }

            let mut artificial_demerits = false;
            let mut shortfall;
            if self.semantic_pagination_enabled {
                line_width = self.lb_cur_active_width[1];
                artificial_demerits = true;
                shortfall = 0;
            } else {
                shortfall = line_width - self.lb_cur_active_width[1];
                if self.int_par(INT_PAR_XETEX_PROTRUDE_CHARS)? > 1 {
                    shortfall += self.total_pw(r, self.lb_cur_p)?;
                }
            }

            let mut b: i32 = 0;
            let mut fit_class: u16 = DECENT_FIT;
            let mut skip_to_found = false;
            if shortfall > 0 {
                if self.lb_cur_active_width[3] != 0
                    || self.lb_cur_active_width[4] != 0
                    || self.lb_cur_active_width[5] != 0
                {
                    if self.lb_do_last_line_fit {
                        if self.lb_cur_p == TEX_NULL {
                            let a_short = self.mem[(r + 3) as usize].s1;
                            let a_glue = self.mem[(r + 4) as usize].s1;
                            if a_short != 0
                                && a_glue > 0
                                && self.lb_cur_active_width[3] == self.lb_fill_width[0]
                                && self.lb_cur_active_width[4] == self.lb_fill_width[1]
                                && self.lb_cur_active_width[5] == self.lb_fill_width[2]
                            {
                                let mut gg = if a_short > 0 {
                                    self.lb_cur_active_width[2]
                                } else {
                                    self.lb_cur_active_width[6]
                                };
                                if gg > 0 {
                                    self.arith_error = false;
                                    gg = self.fract(gg, a_short, a_glue, MAX_HALFWORD);
                                    let llf = self.int_par(INT_PAR_LAST_LINE_FIT)?;
                                    if llf < 1000 {
                                        gg = self.fract(gg, llf, 1000, MAX_HALFWORD);
                                    }
                                    if self.arith_error {
                                        gg = if a_short > 0 {
                                            MAX_HALFWORD
                                        } else {
                                            -MAX_HALFWORD
                                        };
                                    }
                                    if gg > 0 {
                                        if gg > shortfall {
                                            gg = shortfall;
                                        }
                                        if gg > 7_230_584 && self.lb_cur_active_width[2] < 1_663_497
                                        {
                                            b = INF_BAD;
                                            fit_class = VERY_LOOSE_FIT;
                                        } else {
                                            b = badness(gg, self.lb_cur_active_width[2]);
                                            fit_class = if b > 12 {
                                                if b > 99 {
                                                    VERY_LOOSE_FIT
                                                } else {
                                                    LOOSE_FIT
                                                }
                                            } else {
                                                DECENT_FIT
                                            };
                                        }
                                        skip_to_found = true;
                                    } else if gg < 0 {
                                        if -gg > self.lb_cur_active_width[6] {
                                            gg = -self.lb_cur_active_width[6];
                                        }
                                        b = badness(-gg, self.lb_cur_active_width[6]);
                                        fit_class = if b > 12 { TIGHT_FIT } else { DECENT_FIT };
                                        skip_to_found = true;
                                    }
                                }
                            }
                        }
                        shortfall = 0;
                    }
                } else {
                    if shortfall > 7_230_584 && self.lb_cur_active_width[2] < 1_663_497 {
                        b = INF_BAD;
                        fit_class = VERY_LOOSE_FIT;
                    } else {
                        b = badness(shortfall, self.lb_cur_active_width[2]);
                        if b > 12 {
                            fit_class = if b > 99 { VERY_LOOSE_FIT } else { LOOSE_FIT };
                        }
                    }
                }
            } else {
                b = if -shortfall > self.lb_cur_active_width[6] {
                    INF_BAD + 1
                } else {
                    badness(-shortfall, self.lb_cur_active_width[6])
                };
                fit_class = if b > 12 { TIGHT_FIT } else { DECENT_FIT };
            }

            if !skip_to_found && self.lb_do_last_line_fit {
                if self.lb_cur_p == TEX_NULL {
                    shortfall = 0;
                }
                g = if shortfall > 0 {
                    self.lb_cur_active_width[2]
                } else if shortfall < 0 {
                    self.lb_cur_active_width[6]
                } else {
                    0
                };
            }

            let node_r_stays_active;
            let mut deactivate = false;
            if b > INF_BAD || pi == EJECT_PENALTY {
                if self.lb_final_pass
                    && self.lb_minimum_demerits == AWFUL_BAD
                    && self.link(r)? == active_list
                    && prev_r == active_list
                {
                    artificial_demerits = true;
                    node_r_stays_active = false;
                } else if b > self.lb_threshold {
                    deactivate = true;
                    node_r_stays_active = false;
                } else {
                    node_r_stays_active = false;
                }
            } else {
                prev_r = r;
                if b > self.lb_threshold {
                    continue;
                }
                node_r_stays_active = true;
            }

            if !deactivate {
                let d = if artificial_demerits {
                    0
                } else {
                    let mut d = self.int_par(INT_PAR_LINE_PENALTY)? + b;
                    d = if d.abs() >= 10000 { 100_000_000 } else { d * d };
                    if pi != 0 {
                        if pi > 0 {
                            d += pi * pi;
                        } else if pi > EJECT_PENALTY {
                            d -= pi * pi;
                        }
                    }
                    if break_type == HYPHENATED && self.node_type(r)? == HYPHENATED {
                        if self.lb_cur_p != TEX_NULL {
                            d += self.int_par(INT_PAR_DOUBLE_HYPHEN_DEMERITS)?;
                        } else {
                            d += self.int_par(INT_PAR_FINAL_HYPHEN_DEMERITS)?;
                        }
                    }
                    if (i32::from(fit_class) - i32::from(self.mem[r as usize].b16_s0())).abs() > 1 {
                        d += self.int_par(INT_PAR_ADJ_DEMERITS)?;
                    }
                    d
                };
                let d = d + self.mem[(r + 2) as usize].s1;
                if d <= self.lb_minimal_demerits[usize::from(fit_class)] {
                    self.lb_minimal_demerits[usize::from(fit_class)] = d;
                    self.lb_best_place[usize::from(fit_class)] = self.mem[(r + 1) as usize].s1;
                    self.lb_best_pl_line[usize::from(fit_class)] = l;
                    if self.lb_do_last_line_fit {
                        self.lb_best_pl_short[usize::from(fit_class)] = shortfall;
                        self.lb_best_pl_glue[usize::from(fit_class)] = g;
                    }
                    if d < self.lb_minimum_demerits {
                        self.lb_minimum_demerits = d;
                    }
                }
                if node_r_stays_active {
                    continue;
                }
            }

            // deactivate node r
            self.set_link(prev_r, self.link(r)?)?;
            self.free_node(r, self.lb_active_node_size)?;
            if prev_r == active_list {
                let r2 = self.link(active_list)?;
                if self.node_type(r2)? == DELTA_NODE {
                    for i in 1usize..=6 {
                        self.active_width[i] += self.word_at(r2, i as i32)?.s1;
                    }
                    self.lb_cur_active_width = self.active_width;
                    let next = self.link(r2)?;
                    self.set_link(active_list, next)?;
                    self.free_node(r2, DELTA_NODE_SIZE)?;
                }
            } else if self.node_type(prev_r)? == DELTA_NODE {
                let r2 = self.link(prev_r)?;
                if r2 == active_list {
                    for i in 1usize..=6 {
                        self.lb_cur_active_width[i] -= self.word_at(prev_r, i as i32)?.s1;
                    }
                    self.set_link(prev_prev_r, active_list)?;
                    self.free_node(prev_r, DELTA_NODE_SIZE)?;
                    prev_r = prev_prev_r;
                } else if self.node_type(r2)? == DELTA_NODE {
                    for i in 1usize..=6 {
                        let dv = self.word_at(r2, i as i32)?.s1;
                        self.lb_cur_active_width[i] += dv;
                        self.mem[(prev_r + i as i32) as usize].s1 += dv;
                    }
                    let next = self.link(r2)?;
                    self.set_link(prev_r, next)?;
                    self.free_node(r2, DELTA_NODE_SIZE)?;
                }
            }
        }
    }

    /// Character-protrusion width of node `p` on `side` (`char_pw`);
    /// records `lb_last_leftmost_char`/`lb_last_rightmost_char`.
    fn char_pw(&mut self, p: i32, side: i32) -> Result<i32, TeXMouthError> {
        if side == 0 {
            self.lb_last_leftmost_char = TEX_NULL;
        } else {
            self.lb_last_rightmost_char = TEX_NULL;
        }
        if p == TEX_NULL {
            return Ok(0);
        }
        if !self.is_char_node(p) && self.node_type(p)? == WHATSIT_NODE {
            let subtype = self.node_subtype(p)?;
            if subtype == NATIVE_WORD_NODE || subtype == NATIVE_WORD_NODE_AT {
                if self.has_native_glyph_info(p) {
                    let f = i32::from(self.word_at(p, 4)?.b16_s2());
                    let quad = self
                        .font_record(f)
                        .and_then(|record| {
                            usize::try_from(QUAD_CODE)
                                .ok()
                                .and_then(|i| record.params.get(i))
                                .copied()
                        })
                        .unwrap_or(0);
                    let code = self.real_get_native_word_cp(p, side);
                    return Ok(self.round_xn_over_d(quad, code, 1000));
                }
                return Ok(0);
            }
            if subtype == GLYPH_NODE {
                let f = i32::from(self.word_at(p, 4)?.b16_s2());
                let glyph = u32::from(self.word_at(p, 4)?.b16_s1());
                let quad = self
                    .font_record(f)
                    .and_then(|record| {
                        usize::try_from(QUAD_CODE)
                            .ok()
                            .and_then(|i| record.params.get(i))
                            .copied()
                    })
                    .unwrap_or(0);
                let code = self.get_cp_code(f, glyph, side == 0);
                return Ok(self.round_xn_over_d(quad, code, 1000));
            }
        }
        let mut node = p;
        if !self.is_char_node(node) {
            if self.node_type(node)? == LIGATURE_NODE {
                node += 1;
            } else {
                return Ok(0);
            }
        }
        let f = i32::from(self.mem[node as usize].b16_s1());
        let c = self.get_cp_code(f, u32::from(self.mem[node as usize].b16_s0()), side == 0);
        if side == 0 {
            self.lb_last_leftmost_char = node;
        } else {
            self.lb_last_rightmost_char = node;
        }
        if c == 0 {
            return Ok(0);
        }
        let quad = self
            .font_record(f)
            .and_then(|record| {
                usize::try_from(QUAD_CODE)
                    .ok()
                    .and_then(|i| record.params.get(i))
                    .copied()
            })
            .unwrap_or(0);
        Ok(self.round_xn_over_d(quad, c, 1000))
    }

    /// Pushes `p` onto the protrusion hlist stack (`push_node`).
    fn push_hlist_node(&mut self, p: i32) {
        self.lb_hlist_stack.push(p);
    }

    /// Pops the protrusion hlist stack (`pop_node`).
    fn pop_hlist_node(&mut self) -> Result<i32, TeXMouthError> {
        self.lb_hlist_stack
            .pop()
            .ok_or_else(|| TeXMemoryError::CorruptedNode(-1).into())
    }

    /// Whether node `p` is discardable for protrusion walking
    /// (`find_protchar_*` predicates).
    fn is_protchar_discardable(&self, l: i32) -> Result<bool, TeXMouthError> {
        if self.is_char_node(l) {
            return Ok(false);
        }
        let ty = self.node_type(l)?;
        Ok(
            matches!(ty, INS_NODE | MARK_NODE | ADJUST_NODE | PENALTY_NODE)
                || (ty == DISC_NODE
                    && self.word_at(l, 1)?.s0 == TEX_NULL
                    && self.word_at(l, 1)?.s1 == TEX_NULL
                    && self.mem[l as usize].b16_s0() == 0)
                || (ty == MATH_NODE && self.word_at(l, 1)?.s1 == 0)
                || (ty == KERN_NODE
                    && (self.word_at(l, 1)?.s1 == 0 || self.mem[l as usize].b16_s0() == NORMAL))
                || (ty == GLUE_NODE && self.word_at(l, 1)?.s0 == 0)
                || (ty == HLIST_NODE
                    && self.word_at(l, 1)?.s1 == 0
                    && self.word_at(l, 3)?.s1 == 0
                    && self.word_at(l, 2)?.s1 == 0
                    && self.word_at(l, 5)?.s1 == TEX_NULL),
        )
    }

    /// Finds the leftmost protrudable character from `l`
    /// (`find_protchar_left`).
    fn find_protchar_left(&mut self, mut l: i32, d: bool) -> Result<i32, TeXMouthError> {
        if self.link(l)? != TEX_NULL
            && self.node_type(l)? == HLIST_NODE
            && self.word_at(l, 1)?.s1 == 0
            && self.word_at(l, 3)?.s1 == 0
            && self.word_at(l, 2)?.s1 == 0
            && self.word_at(l, 5)?.s1 == TEX_NULL
        {
            l = self.link(l)?;
        } else if d {
            while self.link(l)? != TEX_NULL
                && !(self.is_char_node(l)
                    || (!self.is_char_node(l) && self.node_type(l)? < MATH_NODE))
            {
                l = self.link(l)?;
            }
        }
        self.lb_hlist_stack.clear();
        loop {
            let t = l;
            while self.node_type(l)? == HLIST_NODE && self.word_at(l, 5)?.s1 != TEX_NULL {
                self.push_hlist_node(l);
                l = self.word_at(l, 5)?.s1;
            }
            while self.is_protchar_discardable(l)? {
                while self.link(l)? == TEX_NULL && !self.lb_hlist_stack.is_empty() {
                    l = self.pop_hlist_node()?;
                }
                if self.link(l)? != TEX_NULL {
                    l = self.link(l)?;
                } else if self.lb_hlist_stack.is_empty() {
                    return Ok(l);
                }
            }
            if t == l {
                return Ok(l);
            }
        }
    }

    /// Finds the rightmost protrudable character between `l` and `r`
    /// (`find_protchar_right`).
    fn find_protchar_right(&mut self, mut l: i32, mut r: i32) -> Result<i32, TeXMouthError> {
        if r == TEX_NULL {
            return Ok(TEX_NULL);
        }
        self.lb_hlist_stack.clear();
        loop {
            let t = r;
            while self.node_type(r)? == HLIST_NODE && self.word_at(r, 5)?.s1 != TEX_NULL {
                self.push_hlist_node(l);
                self.push_hlist_node(r);
                l = self.word_at(r, 5)?.s1;
                r = l;
                while self.link(r)? != TEX_NULL {
                    r = self.link(r)?;
                }
            }
            while self.is_protchar_discardable(r)? {
                while r == l && !self.lb_hlist_stack.is_empty() {
                    r = self.pop_hlist_node()?;
                    l = self.pop_hlist_node()?;
                }
                if r != l && r != TEX_NULL {
                    r = self.prev_rightmost(l, r)?;
                } else if r == l && self.lb_hlist_stack.is_empty() {
                    return Ok(r);
                }
            }
            if t == r {
                return Ok(r);
            }
        }
    }

    /// Total protrusion width relevant to a break from active node `q`
    /// to `p` (`total_pw`).
    fn total_pw(&mut self, q: i32, p: i32) -> Result<i32, TeXMouthError> {
        let mut l = if self.word_at(q, 1)?.s1 == TEX_NULL {
            self.lb_first_p
        } else {
            self.word_at(self.word_at(q, 1)?.s1, 1)?.s1
        };
        let mut r = self.prev_rightmost(self.lb_global_prev_p, p)?;
        if p != TEX_NULL && self.node_type(p)? == DISC_NODE && self.word_at(p, 1)?.s0 != TEX_NULL {
            r = self.word_at(p, 1)?.s0;
            while self.link(r)? != TEX_NULL {
                r = self.link(r)?;
            }
        } else {
            r = self.find_protchar_right(l, r)?;
        }
        let mut resolve_left = true;
        if l != TEX_NULL && self.node_type(l)? == DISC_NODE {
            if self.word_at(l, 1)?.s1 != TEX_NULL {
                l = self.word_at(l, 1)?.s1;
                resolve_left = false;
            } else {
                let mut n = self.mem[l as usize].b16_s0();
                l = self.link(l)?;
                while n > 0 {
                    if self.link(l)? != TEX_NULL {
                        l = self.link(l)?;
                    }
                    n -= 1;
                }
            }
        }
        if resolve_left {
            l = self.find_protchar_left(l, true)?;
        }
        Ok(self.char_pw(l, 0)? + self.char_pw(r, 1)?)
    }

    /// Replays the font's ligature/kern program over `lb_hu[j..=n]`,
    /// depositing rebuilt characters, ligatures, and kerns on
    /// `HOLD_HEAD` (`reconstitute`). Returns the last consumed index.
    fn reconstitute(
        &mut self,
        mut j: i32,
        n: i32,
        mut bchar: i32,
        mut hchar: i32,
    ) -> Result<i32, TeXMouthError> {
        let hold_head = self.hold_head();
        self.lb_hyphen_passed = 0;
        let mut t = hold_head;
        let mut w = 0i32;
        self.set_link(t, TEX_NULL)?;
        self.lb_cur_l = self.lb_hu[j as usize];
        self.lb_cur_q = t;
        if j == 0 {
            self.lb_ligature_present = self.lb_init_lig;
            let mut p = self.lb_init_list;
            if self.lb_ligature_present {
                self.lb_lft_hit = self.lb_init_lft;
            }
            while p > TEX_NULL {
                let cell = self.get_avail()?;
                self.set_link(t, cell)?;
                t = cell;
                let ch = self.mem[p as usize].b16_s0();
                self.mem[t as usize].set_b16_s1(self.lb_hf as u16);
                self.mem[t as usize].set_b16_s0(ch);
                p = self.link(p)?;
            }
        } else if self.lb_cur_l < TOO_BIG_CHAR {
            let cell = self.get_avail()?;
            self.set_link(t, cell)?;
            t = cell;
            self.mem[t as usize].set_b16_s1(self.lb_hf as u16);
            self.mem[t as usize].set_b16_s0(self.lb_cur_l as u16);
        }
        self.lb_lig_stack = TEX_NULL;
        let mut cur_r = if j < n {
            self.lb_hu[(j + 1) as usize]
        } else {
            bchar
        };
        let mut cur_rh = if self.lb_hyf[j as usize] % 2 != 0 {
            hchar
        } else {
            TOO_BIG_CHAR
        };

        'outer: loop {
            'cont: loop {
                let mut k: i32;
                let mut q: LigKernInsn;
                if self.lb_cur_l == TOO_BIG_CHAR {
                    k = self
                        .font_record(self.lb_hf)
                        .map_or(NON_ADDRESS, |record| record.bchar_label);
                    if k == NON_ADDRESS {
                        break 'cont;
                    }
                    q = self.lig_kern_insn(self.lb_hf, k);
                } else {
                    let e = self.effective_char(self.lb_hf, self.lb_cur_l).max(0) as u32;
                    let (tag, start) = self.font_record(self.lb_hf).map_or((0, 0), |record| {
                        (
                            record.char_tags.get(&e).copied().unwrap_or(0),
                            record.char_remainders.get(&e).copied().unwrap_or(0),
                        )
                    });
                    if tag != LIG_TAG {
                        break 'cont;
                    }
                    k = start;
                    q = self.lig_kern_insn(self.lb_hf, k);
                    if q.skip > 128 {
                        k = 256 * i32::from(q.op) + q.remainder + 32768 - 256 * 128;
                        q = self.lig_kern_insn(self.lb_hf, k);
                    }
                }
                let test_char = if cur_rh < TOO_BIG_CHAR { cur_rh } else { cur_r };
                loop {
                    if q.next_char == test_char && q.skip <= 128 {
                        if cur_rh < TOO_BIG_CHAR {
                            self.lb_hyphen_passed = j;
                            hchar = TOO_BIG_CHAR;
                            cur_rh = TOO_BIG_CHAR;
                            continue 'cont;
                        }
                        if hchar < TOO_BIG_CHAR && self.lb_hyf[j as usize] % 2 != 0 {
                            self.lb_hyphen_passed = j;
                            hchar = TOO_BIG_CHAR;
                        }
                        if q.op < 128 {
                            if self.lb_cur_l == TOO_BIG_CHAR {
                                self.lb_lft_hit = true;
                            }
                            if j == n && self.lb_lig_stack == TEX_NULL {
                                self.lb_rt_hit = true;
                            }
                            match q.op {
                                1 | 5 => {
                                    self.lb_cur_l = q.remainder;
                                    self.lb_ligature_present = true;
                                }
                                2 | 6 => {
                                    cur_r = q.remainder;
                                    if self.lb_lig_stack > TEX_NULL {
                                        self.mem[self.lb_lig_stack as usize]
                                            .set_b16_s0(cur_r as u16);
                                    } else {
                                        self.lb_lig_stack = self.new_lig_item(cur_r)?;
                                        if j == n {
                                            bchar = TOO_BIG_CHAR;
                                        } else {
                                            let p = self.get_avail()?;
                                            self.mem[(self.lb_lig_stack + 1) as usize].s1 = p;
                                            let h = self.lb_hu[(j + 1) as usize];
                                            self.mem[p as usize].set_b16_s0(h as u16);
                                            self.mem[p as usize].set_b16_s1(self.lb_hf as u16);
                                        }
                                    }
                                }
                                3 => {
                                    cur_r = q.remainder;
                                    let p = self.lb_lig_stack;
                                    self.lb_lig_stack = self.new_lig_item(cur_r)?;
                                    self.set_link(self.lb_lig_stack, p)?;
                                }
                                7 | 11 => {
                                    if self.lb_ligature_present {
                                        let p = self.new_ligature(
                                            self.lb_hf,
                                            self.lb_cur_l,
                                            self.link(self.lb_cur_q)?,
                                        )?;
                                        if self.lb_lft_hit {
                                            self.set_node_subtype(p, 2)?;
                                            self.lb_lft_hit = false;
                                        }
                                        self.set_link(self.lb_cur_q, p)?;
                                        t = p;
                                        self.lb_ligature_present = false;
                                    }
                                    self.lb_cur_q = t;
                                    self.lb_cur_l = q.remainder;
                                    self.lb_ligature_present = true;
                                }
                                _ => {
                                    self.lb_cur_l = q.remainder;
                                    self.lb_ligature_present = true;
                                    if self.lb_lig_stack > TEX_NULL {
                                        let chain = self.mem[(self.lb_lig_stack + 1) as usize].s1;
                                        if chain > TEX_NULL {
                                            self.set_link(t, chain)?;
                                            t = self.link(t)?;
                                            j += 1;
                                        }
                                        let p = self.lb_lig_stack;
                                        self.lb_lig_stack = self.link(p)?;
                                        self.free_node(p, SMALL_NODE_SIZE)?;
                                        if self.lb_lig_stack == TEX_NULL {
                                            cur_r = if j < n {
                                                self.lb_hu[(j + 1) as usize]
                                            } else {
                                                bchar
                                            };
                                            cur_rh = if self.lb_hyf[j as usize] % 2 != 0 {
                                                hchar
                                            } else {
                                                TOO_BIG_CHAR
                                            };
                                        } else {
                                            cur_r = i32::from(
                                                self.mem[self.lb_lig_stack as usize].b16_s0(),
                                            );
                                        }
                                    } else if j == n {
                                        break 'cont;
                                    } else {
                                        let cell = self.get_avail()?;
                                        self.set_link(t, cell)?;
                                        t = cell;
                                        self.mem[t as usize].set_b16_s1(self.lb_hf as u16);
                                        self.mem[t as usize].set_b16_s0(cur_r as u16);
                                        j += 1;
                                        cur_r = if j < n {
                                            self.lb_hu[(j + 1) as usize]
                                        } else {
                                            bchar
                                        };
                                        cur_rh = if self.lb_hyf[j as usize] % 2 != 0 {
                                            hchar
                                        } else {
                                            TOO_BIG_CHAR
                                        };
                                    }
                                }
                            }
                            if q.op > 4 && q.op != 7 {
                                break 'cont;
                            }
                            continue 'cont;
                        }
                        w = self.kern_value(self.lb_hf, q.op, q.remainder);
                        break 'cont;
                    }
                    if q.skip >= 128 {
                        if cur_rh == TOO_BIG_CHAR {
                            break 'cont;
                        }
                        cur_rh = TOO_BIG_CHAR;
                        continue 'cont;
                    }
                    k += i32::from(q.skip) + 1;
                    q = self.lig_kern_insn(self.lb_hf, k);
                }
            }

            // done: flush pending ligature and kern, then requeue the
            // ligature stack if it is nonempty.
            if self.lb_ligature_present {
                let p = self.new_ligature(self.lb_hf, self.lb_cur_l, self.link(self.lb_cur_q)?)?;
                if self.lb_lft_hit {
                    self.set_node_subtype(p, 2)?;
                    self.lb_lft_hit = false;
                }
                if self.lb_rt_hit && self.lb_lig_stack == TEX_NULL {
                    let sub = i32::from(self.word_at(p, 0)?.b16_s0()) + 1;
                    self.set_node_subtype(p, sub as u16)?;
                    self.lb_rt_hit = false;
                }
                self.set_link(self.lb_cur_q, p)?;
                t = p;
                self.lb_ligature_present = false;
            }
            if w != 0 {
                let kern = self.new_kern(w)?;
                self.set_link(t, kern)?;
                t = self.link(t)?;
                w = 0;
                self.mem[(t + 2) as usize].s0 = 0;
            }
            if self.lb_lig_stack > TEX_NULL {
                self.lb_cur_q = t;
                self.lb_cur_l = i32::from(self.mem[self.lb_lig_stack as usize].b16_s0());
                self.lb_ligature_present = true;
                let chain = self.mem[(self.lb_lig_stack + 1) as usize].s1;
                if chain > TEX_NULL {
                    self.set_link(t, chain)?;
                    t = self.link(t)?;
                    j += 1;
                }
                let p = self.lb_lig_stack;
                self.lb_lig_stack = self.link(p)?;
                self.free_node(p, SMALL_NODE_SIZE)?;
                if self.lb_lig_stack == TEX_NULL {
                    cur_r = if j < n {
                        self.lb_hu[(j + 1) as usize]
                    } else {
                        bchar
                    };
                    cur_rh = if self.lb_hyf[j as usize] % 2 != 0 {
                        hchar
                    } else {
                        TOO_BIG_CHAR
                    };
                } else {
                    cur_r = i32::from(self.mem[self.lb_lig_stack as usize].b16_s0());
                }
                continue 'outer;
            }
            return Ok(j);
        }
    }

    /// Breaks the current paragraph into lines (`line_break`). On entry
    /// the hlist is at `head`; on return the lines have been appended to
    /// the enclosing vertical list and `just_box` names the last one.
    /// `d` is set when the paragraph precedes display math.
    fn line_break(&mut self, d: bool) -> Result<(), TeXMouthError> {
        self.pack_begin_line = self.mode_line;

        let temp_head = self.temp_head();
        let contrib = self.link(self.head)?;
        self.set_link(temp_head, contrib)?;

        // Remove trailing space or glue; append the infinite penalty and
        // `\parfillskip` (`line_break` preamble, C ~99).
        if self.is_char_node(self.tail) || self.node_type(self.tail)? != GLUE_NODE {
            let pen = self.new_penalty(INF_PENALTY)?;
            self.set_link(self.tail, pen)?;
            self.tail = pen;
        } else {
            self.set_node_type(self.tail, PENALTY_NODE)?;
            let glue = self.word_at(self.tail, 1)?.s0;
            self.delete_glue_ref(glue)?;
            let leader = self.word_at(self.tail, 1)?.s1;
            if leader != TEX_NULL {
                self.flush_node_list(leader)?;
            }
            self.mem[(self.tail + 1) as usize].s1 = INF_PENALTY;
        }
        let fill = self.new_param_glue(GLUE_PAR_PAR_FILL_SKIP)?;
        self.set_link(self.tail, fill)?;
        self.lb_last_line_fill = fill;

        self.lb_init_cur_lang = self.prev_graf % 65536;
        self.lb_init_l_hyf = self.prev_graf / 0x040_0000;
        self.lb_init_r_hyf = (self.prev_graf / 65536) % 64;

        self.pop_nest()?;

        self.lb_no_shrink_error_yet = true;

        for glue_index in [GLUE_PAR_LEFT_SKIP, GLUE_PAR_RIGHT_SKIP] {
            let spec = self.glue_par(glue_index)?;
            if self.mem[spec as usize].b16_s0() != NORMAL && self.word_at(spec, 3)?.s1 != 0 {
                let replacement = self.finite_shrink(spec)?;
                self.eqtb[GLUE_PAR_BASE + glue_index].s1 = replacement;
            }
        }

        let q = self.glue_par(GLUE_PAR_LEFT_SKIP)?;
        let r = self.glue_par(GLUE_PAR_RIGHT_SKIP)?;
        self.lb_background = [0; 7];
        self.lb_background[1] = self.word_at(q, 1)?.s1 + self.word_at(r, 1)?.s1;
        let q_order = usize::from(self.mem[q as usize].b16_s1());
        self.lb_background[2 + q_order] = self.word_at(q, 2)?.s1;
        let r_order = usize::from(self.mem[r as usize].b16_s1());
        self.lb_background[2 + r_order] += self.word_at(r, 2)?.s1;
        self.lb_background[6] = self.word_at(q, 3)?.s1 + self.word_at(r, 3)?.s1;

        // `\lastlinefit` setup (C ~1631).
        self.lb_do_last_line_fit = false;
        self.lb_active_node_size = ACTIVE_NODE_SIZE_NORMAL;
        if self.int_par(INT_PAR_LAST_LINE_FIT)? > 0 {
            let q = self.word_at(self.lb_last_line_fill, 1)?.s0;
            if self.word_at(q, 2)?.s1 > 0
                && self.mem[q as usize].b16_s1() != NORMAL
                && self.lb_background[3..6] == [0; 3]
            {
                self.lb_do_last_line_fit = true;
                self.lb_active_node_size = ACTIVE_NODE_SIZE_EXTENDED;
                self.lb_fill_width = [0; 3];
                self.lb_fill_width[usize::from(self.mem[q as usize].b16_s1()) - 1] =
                    self.word_at(q, 2)?.s1;
            }
        }

        self.lb_minimum_demerits = AWFUL_BAD;
        self.lb_minimal_demerits = [AWFUL_BAD; 4];

        // `\parshape`/`\hangindent` geometry (C ~877).
        let par_shape = self.local(LOCAL_PAR_SHAPE)?;
        if par_shape == TEX_NULL {
            if self.dimen_par(DIMEN_PAR_HANG_INDENT)? == 0 {
                self.lb_last_special_line = 0;
                self.lb_second_width = self.dimen_par(DIMEN_PAR_HSIZE)?;
                self.lb_second_indent = 0;
            } else {
                self.lb_last_special_line = self.int_par(INT_PAR_HANG_AFTER)?.abs();
                if self.int_par(INT_PAR_HANG_AFTER)? < 0 {
                    self.lb_first_width = self.dimen_par(DIMEN_PAR_HSIZE)?
                        - self.dimen_par(DIMEN_PAR_HANG_INDENT)?.abs();
                    self.lb_first_indent = self.dimen_par(DIMEN_PAR_HANG_INDENT)?.max(0);
                    self.lb_second_width = self.dimen_par(DIMEN_PAR_HSIZE)?;
                    self.lb_second_indent = 0;
                } else {
                    self.lb_first_width = self.dimen_par(DIMEN_PAR_HSIZE)?;
                    self.lb_first_indent = 0;
                    self.lb_second_width = self.dimen_par(DIMEN_PAR_HSIZE)?
                        - self.dimen_par(DIMEN_PAR_HANG_INDENT)?.abs();
                    self.lb_second_indent = self.dimen_par(DIMEN_PAR_HANG_INDENT)?.max(0);
                }
            }
        } else {
            self.lb_last_special_line = self.mem[par_shape as usize].s0 - 1;
            self.lb_second_width =
                self.mem[(par_shape + 2 * (self.lb_last_special_line + 1)) as usize].s1;
            self.lb_second_indent =
                self.mem[(par_shape + 2 * self.lb_last_special_line + 1) as usize].s1;
        }

        if self.int_par(INT_PAR_LOOSENESS)? == 0 {
            self.lb_easy_line = self.lb_last_special_line;
        } else {
            self.lb_easy_line = MAX_HALFWORD;
        }

        // Pass management (C ~892).
        self.lb_threshold = self.int_par(INT_PAR_PRETOLERANCE)?;
        if self.lb_threshold >= 0 {
            self.lb_second_pass = false;
            self.lb_final_pass = false;
        } else {
            self.lb_threshold = self.int_par(INT_PAR_TOLERANCE)?;
            self.lb_second_pass = true;
            self.lb_final_pass = self.dimen_par(DIMEN_PAR_EMERGENCY_STRETCH)? <= 0;
        }

        let active_list = self.active_list();
        loop {
            if self.lb_threshold > INF_BAD {
                self.lb_threshold = INF_BAD;
            }
            if self.lb_second_pass {
                self.cur_lang = self.lb_init_cur_lang;
                self.lb_l_hyf = self.lb_init_l_hyf;
                self.lb_r_hyf = self.lb_init_r_hyf;
            }

            // Initial active node (C ~893).
            let q = self.get_node(self.lb_active_node_size)?;
            self.set_node_type(q, UNHYPHENATED)?;
            self.mem[q as usize].set_b16_s0(DECENT_FIT);
            self.set_link(q, active_list)?;
            self.mem[(q + 1) as usize].s1 = TEX_NULL;
            self.mem[(q + 1) as usize].s0 = self.prev_graf + 1;
            self.mem[(q + 2) as usize].s1 = 0;
            self.set_link(active_list, q)?;
            if self.lb_do_last_line_fit {
                self.mem[(q + 3) as usize].s1 = 0;
                self.mem[(q + 4) as usize].s1 = 0;
            }

            self.active_width = self.lb_background;
            self.lb_passive = TEX_NULL;
            self.lb_font_in_short_display = 0;
            self.lb_cur_p = self.link(temp_head)?;
            let mut auto_breaking = true;
            let mut prev_p = self.lb_cur_p;
            self.lb_global_prev_p = self.lb_cur_p;
            self.lb_first_p = self.lb_cur_p;

            'walk: while self.lb_cur_p != TEX_NULL && self.link(active_list)? != active_list {
                if self.is_char_node(self.lb_cur_p) {
                    prev_p = self.lb_cur_p;
                    self.lb_global_prev_p = self.lb_cur_p;
                    while self.is_char_node(self.lb_cur_p) {
                        let f = i32::from(self.mem[self.lb_cur_p as usize].b16_s1());
                        let eff = self.effective_char(
                            f,
                            i32::from(self.mem[self.lb_cur_p as usize].b16_s0()),
                        );
                        self.active_width[1] += self.font_char_metric(f, eff.max(0) as u32).width;
                        self.lb_cur_p = self.link(self.lb_cur_p)?;
                    }
                    if self.lb_cur_p == TEX_NULL {
                        break 'walk;
                    }
                }

                let mut skip_advance = false;
                match self.node_type(self.lb_cur_p)? {
                    HLIST_NODE | VLIST_NODE | RULE_NODE => {
                        self.active_width[1] += self.word_at(self.lb_cur_p, 1)?.s1;
                    }
                    WHATSIT_NODE => {
                        let subtype = self.node_subtype(self.lb_cur_p)?;
                        if subtype == LANGUAGE_NODE {
                            self.cur_lang = self.mem[(self.lb_cur_p + 1) as usize].s1;
                            self.lb_l_hyf = i32::from(self.word_at(self.lb_cur_p, 1)?.b16_s1());
                            self.lb_r_hyf = i32::from(self.word_at(self.lb_cur_p, 1)?.b16_s0());
                        } else if matches!(
                            subtype,
                            NATIVE_WORD_NODE
                                | NATIVE_WORD_NODE_AT
                                | GLYPH_NODE
                                | PIC_NODE
                                | PDF_NODE
                        ) {
                            self.active_width[1] += self.word_at(self.lb_cur_p, 1)?.s1;
                        }
                    }
                    GLUE_NODE => {
                        if auto_breaking {
                            let breaks = self.is_char_node(prev_p)
                                || (!self.is_char_node(prev_p)
                                    && self.node_type(prev_p)? < MATH_NODE)
                                || (self.node_type(prev_p)? == KERN_NODE
                                    && self.node_subtype(prev_p)? != EXPLICIT);
                            if breaks {
                                self.try_break(0, UNHYPHENATED)?;
                            }
                        }
                        let q = self.word_at(self.lb_cur_p, 1)?.s0;
                        if self.mem[q as usize].b16_s0() != NORMAL && self.word_at(q, 3)?.s1 != 0 {
                            let q = self.finite_shrink(q)?;
                            self.mem[(self.lb_cur_p + 1) as usize].s0 = q;
                        }
                        let q = self.word_at(self.lb_cur_p, 1)?.s0;
                        self.active_width[1] += self.word_at(q, 1)?.s1;
                        let order = usize::from(self.mem[q as usize].b16_s1());
                        self.active_width[2 + order] += self.word_at(q, 2)?.s1;
                        self.active_width[6] += self.word_at(q, 3)?.s1;

                        if self.lb_second_pass && auto_breaking {
                            self.lb_hyphenate_next_word(self.lb_cur_p)?;
                        }
                    }
                    KERN_NODE => {
                        if self.node_subtype(self.lb_cur_p)? == EXPLICIT {
                            let next = self.link(self.lb_cur_p)?;
                            if !self.is_char_node(next)
                                && auto_breaking
                                && self.node_type(next)? == GLUE_NODE
                            {
                                self.try_break(0, UNHYPHENATED)?;
                            }
                        }
                        self.active_width[1] += self.word_at(self.lb_cur_p, 1)?.s1;
                    }
                    LIGATURE_NODE => {
                        let f = self.lig_font(self.lb_cur_p)?;
                        self.xtx_ligature_present = true;
                        let eff = self.effective_char(f, self.lig_char(self.lb_cur_p)?);
                        self.active_width[1] += self.font_char_metric(f, eff.max(0) as u32).width;
                    }
                    DISC_NODE => {
                        let mut s = self.word_at(self.lb_cur_p, 1)?.s0;
                        self.lb_disc_width = 0;
                        if s == TEX_NULL {
                            let pen = self.int_par(INT_PAR_EX_HYPHEN_PENALTY)?;
                            self.try_break(pen, HYPHENATED)?;
                        } else {
                            while s != TEX_NULL {
                                self.lb_disc_width += self.lb_node_width(s)?;
                                s = self.link(s)?;
                            }
                            self.active_width[1] += self.lb_disc_width;
                            let pen = self.int_par(INT_PAR_HYPHEN_PENALTY)?;
                            self.try_break(pen, HYPHENATED)?;
                            self.active_width[1] -= self.lb_disc_width;
                        }
                        let mut count = i32::from(self.mem[self.lb_cur_p as usize].b16_s0());
                        let mut s = self.link(self.lb_cur_p)?;
                        while count > 0 {
                            self.active_width[1] += self.lb_node_width(s)?;
                            count -= 1;
                            s = self.link(s)?;
                        }
                        prev_p = self.lb_cur_p;
                        self.lb_global_prev_p = self.lb_cur_p;
                        self.lb_cur_p = s;
                        skip_advance = true;
                    }
                    MATH_NODE => {
                        let subtype = i32::from(self.node_subtype(self.lb_cur_p)?);
                        if subtype < L_CODE {
                            auto_breaking = subtype % 2 != 0;
                        }
                        let next = self.link(self.lb_cur_p)?;
                        if !self.is_char_node(next)
                            && auto_breaking
                            && self.node_type(next)? == GLUE_NODE
                        {
                            self.try_break(0, UNHYPHENATED)?;
                        }
                        self.active_width[1] += self.word_at(self.lb_cur_p, 1)?.s1;
                    }
                    PENALTY_NODE => {
                        let pen = self.word_at(self.lb_cur_p, 1)?.s1;
                        self.try_break(pen, UNHYPHENATED)?;
                    }
                    MARK_NODE | INS_NODE | ADJUST_NODE => {}
                    _ => return Err(TeXMemoryError::CorruptedNode(self.lb_cur_p).into()),
                }

                if skip_advance {
                    continue 'walk;
                }
                prev_p = self.lb_cur_p;
                self.lb_global_prev_p = self.lb_cur_p;
                self.lb_cur_p = self.link(self.lb_cur_p)?;
            }

            let mut finished = false;
            if self.lb_cur_p == TEX_NULL {
                self.try_break(EJECT_PENALTY, HYPHENATED)?;
                if self.link(active_list)? != active_list {
                    let mut r = self.link(active_list)?;
                    self.lb_fewest_demerits = MAX_HALFWORD;
                    loop {
                        if self.node_type(r)? != DELTA_NODE {
                            let demerits = self.mem[(r + 2) as usize].s1;
                            if demerits < self.lb_fewest_demerits {
                                self.lb_fewest_demerits = demerits;
                                self.lb_best_bet = r;
                            }
                        }
                        r = self.link(r)?;
                        if r == active_list {
                            break;
                        }
                    }
                    self.lb_best_line = self.mem[(self.lb_best_bet + 1) as usize].s0;

                    if self.int_par(INT_PAR_LOOSENESS)? == 0 {
                        finished = true;
                    } else {
                        let mut r = self.link(active_list)?;
                        self.lb_actual_looseness = 0;
                        let looseness = self.int_par(INT_PAR_LOOSENESS)?;
                        loop {
                            if self.node_type(r)? != DELTA_NODE {
                                self.lb_line_diff =
                                    self.mem[(r + 1) as usize].s0 - self.lb_best_line;
                                let demerits = self.mem[(r + 2) as usize].s1;
                                if (self.lb_line_diff < self.lb_actual_looseness
                                    && looseness <= self.lb_line_diff)
                                    || (self.lb_line_diff > self.lb_actual_looseness
                                        && looseness >= self.lb_line_diff)
                                {
                                    self.lb_best_bet = r;
                                    self.lb_actual_looseness = self.lb_line_diff;
                                    self.lb_fewest_demerits = demerits;
                                } else if self.lb_line_diff == self.lb_actual_looseness
                                    && demerits < self.lb_fewest_demerits
                                {
                                    self.lb_best_bet = r;
                                    self.lb_fewest_demerits = demerits;
                                }
                            }
                            r = self.link(r)?;
                            if r == active_list {
                                break;
                            }
                        }
                        self.lb_best_line = self.mem[(self.lb_best_bet + 1) as usize].s0;
                        if self.lb_actual_looseness == looseness || self.lb_final_pass {
                            finished = true;
                        }
                    }
                }
            }
            if finished {
                break;
            }

            // Clean up the active and passive lists (C ~894).
            let mut q = self.link(active_list)?;
            while q != active_list {
                let next = self.link(q)?;
                if self.node_type(q)? == DELTA_NODE {
                    self.free_node(q, DELTA_NODE_SIZE)?;
                } else {
                    self.free_node(q, self.lb_active_node_size)?;
                }
                q = next;
            }
            let mut q = self.lb_passive;
            while q != TEX_NULL {
                let next = self.link(q)?;
                self.free_node(q, PASSIVE_NODE_SIZE)?;
                q = next;
            }

            if !self.lb_second_pass {
                self.lb_threshold = self.int_par(INT_PAR_TOLERANCE)?;
                self.lb_second_pass = true;
                self.lb_final_pass = self.dimen_par(DIMEN_PAR_EMERGENCY_STRETCH)? <= 0;
            } else {
                self.lb_background[2] += self.dimen_par(DIMEN_PAR_EMERGENCY_STRETCH)?;
                self.lb_final_pass = true;
            }
        }

        if self.lb_do_last_line_fit {
            let best_shortfall = self.mem[(self.lb_best_bet + 3) as usize].s1;
            if best_shortfall == 0 {
                self.lb_do_last_line_fit = false;
            } else {
                let spec = self.word_at(self.lb_last_line_fill, 1)?.s0;
                let q = self.new_spec(spec)?;
                self.delete_glue_ref(spec)?;
                self.mem[(q + 1) as usize].s1 +=
                    best_shortfall - self.mem[(self.lb_best_bet + 4) as usize].s1;
                self.mem[(q + 2) as usize].s1 = 0;
                self.mem[(self.lb_last_line_fill + 1) as usize].s0 = q;
            }
        }

        self.post_line_break(d)?;

        let mut q = self.link(active_list)?;
        while q != active_list {
            let next = self.link(q)?;
            if self.node_type(q)? == DELTA_NODE {
                self.free_node(q, DELTA_NODE_SIZE)?;
            } else {
                self.free_node(q, self.lb_active_node_size)?;
            }
            q = next;
        }
        let mut q = self.lb_passive;
        while q != TEX_NULL {
            let next = self.link(q)?;
            self.free_node(q, PASSIVE_NODE_SIZE)?;
            q = next;
        }

        self.pack_begin_line = 0;
        Ok(())
    }

    /// Walks the passive breakpoint chain backward, packaging each line
    /// and appending it to the vertical list (`post_line_break`).
    fn post_line_break(&mut self, d: bool) -> Result<(), TeXMouthError> {
        let mut lr_ptr = self.etex_aux;
        let temp_head = self.temp_head();

        // Reverse the passive chain (C ~907).
        let mut q = self.mem[(self.lb_best_bet + 1) as usize].s1;
        self.lb_cur_p = TEX_NULL;
        loop {
            let r = q;
            q = self.mem[(q + 1) as usize].s0;
            self.mem[(r + 1) as usize].s0 = self.lb_cur_p;
            self.lb_cur_p = r;
            if q == TEX_NULL {
                break;
            }
        }

        let mut cur_line = self.prev_graf + 1;

        loop {
            // TeX--XeT LR stack insertion (C ~1494).
            if self.int_par(INT_PAR_TEXXET)? > 0 {
                let mut q = self.mem[temp_head as usize].s1;
                if lr_ptr != TEX_NULL {
                    let mut temp = lr_ptr;
                    let mut r = q;
                    loop {
                        let s = self.new_math(0, self.mem[temp as usize].s0 - 1)?;
                        self.mem[s as usize].s1 = r;
                        r = s;
                        temp = self.mem[temp as usize].s1;
                        if temp == TEX_NULL {
                            break;
                        }
                    }
                    self.mem[temp_head as usize].s1 = r;
                }
                while q != self.mem[(self.lb_cur_p + 1) as usize].s1 {
                    if !self.is_char_node(q) && self.node_type(q)? == MATH_NODE {
                        let subtype = i32::from(self.mem[q as usize].b16_s0());
                        if subtype % 2 != 0 {
                            let expected = L_CODE * (subtype / L_CODE) + 3;
                            if lr_ptr != TEX_NULL && self.mem[lr_ptr as usize].s0 == expected {
                                let temp = lr_ptr;
                                lr_ptr = self.mem[temp as usize].s1;
                                self.mem[temp as usize].s1 = self.avail;
                                self.avail = temp;
                            }
                        } else {
                            let temp = self.get_avail()?;
                            self.mem[temp as usize].s0 = L_CODE * (subtype / L_CODE) + 3;
                            self.mem[temp as usize].s1 = lr_ptr;
                            lr_ptr = temp;
                        }
                    }
                    q = self.mem[q as usize].s1;
                }
            }

            // Modify the end of the line for the break (C ~910).
            let mut q = self.mem[(self.lb_cur_p + 1) as usize].s1;
            let mut disc_break = false;
            let mut post_disc_break = false;
            let mut glue_break = false;

            if q == TEX_NULL {
                q = temp_head;
                while self.mem[q as usize].s1 != TEX_NULL {
                    q = self.mem[q as usize].s1;
                }
            } else if self.node_type(q)? == GLUE_NODE {
                let spec = self.word_at(q, 1)?.s0;
                self.delete_glue_ref(spec)?;
                let right_skip = self.glue_par(GLUE_PAR_RIGHT_SKIP)?;
                self.mem[(q + 1) as usize].s0 = right_skip;
                self.mem[q as usize].set_b16_s0((GLUE_PAR_RIGHT_SKIP + 1) as u16);
                self.mem[right_skip as usize].s1 += 1;
                glue_break = true;
            } else if self.node_type(q)? == DISC_NODE {
                let mut t = i32::from(self.mem[q as usize].b16_s0());
                let mut r;
                if t == 0 {
                    r = self.mem[q as usize].s1;
                } else {
                    let mut rr = q;
                    while t > 1 {
                        rr = self.mem[rr as usize].s1;
                        t -= 1;
                    }
                    let s = self.mem[rr as usize].s1;
                    let r2 = self.mem[s as usize].s1;
                    self.mem[s as usize].s1 = TEX_NULL;
                    self.flush_node_list(self.mem[q as usize].s1)?;
                    self.mem[q as usize].set_b16_s0(0);
                    r = r2;
                }
                if self.mem[(q + 1) as usize].s1 != TEX_NULL {
                    let mut s = self.mem[(q + 1) as usize].s1;
                    while self.mem[s as usize].s1 != TEX_NULL {
                        s = self.mem[s as usize].s1;
                    }
                    self.mem[s as usize].s1 = r;
                    r = self.mem[(q + 1) as usize].s1;
                    self.mem[(q + 1) as usize].s1 = TEX_NULL;
                    post_disc_break = true;
                }
                if self.mem[(q + 1) as usize].s0 != TEX_NULL {
                    let mut s = self.mem[(q + 1) as usize].s0;
                    self.mem[q as usize].s1 = s;
                    while self.mem[s as usize].s1 != TEX_NULL {
                        s = self.mem[s as usize].s1;
                    }
                    self.mem[(q + 1) as usize].s0 = TEX_NULL;
                    q = s;
                }
                self.mem[q as usize].s1 = r;
                disc_break = true;
            } else if self.node_type(q)? == KERN_NODE || self.node_type(q)? == MATH_NODE {
                self.mem[(q + 1) as usize].s1 = 0;
                if self.node_type(q)? == MATH_NODE && self.int_par(INT_PAR_TEXXET)? > 0 {
                    let subtype = i32::from(self.mem[q as usize].b16_s0());
                    if subtype % 2 != 0 {
                        let expected = L_CODE * (subtype / L_CODE) + 3;
                        if lr_ptr != TEX_NULL && self.mem[lr_ptr as usize].s0 == expected {
                            let temp = lr_ptr;
                            lr_ptr = self.mem[temp as usize].s1;
                            self.mem[temp as usize].s1 = self.avail;
                            self.avail = temp;
                        }
                    } else {
                        let temp = self.get_avail()?;
                        self.mem[temp as usize].s0 = L_CODE * (subtype / L_CODE) + 3;
                        self.mem[temp as usize].s1 = lr_ptr;
                        lr_ptr = temp;
                    }
                }
            }

            if self.int_par(INT_PAR_XETEX_PROTRUDE_CHARS)? > 0 {
                let (p, ptmp) =
                    if disc_break && (self.is_char_node(q) || self.node_type(q)? != DISC_NODE) {
                        (q, q)
                    } else {
                        let p = self.prev_rightmost(self.mem[temp_head as usize].s1, q)?;
                        let p2 = self.find_protchar_right(self.mem[temp_head as usize].s1, p)?;
                        (p2, p)
                    };
                let w = self.char_pw(p, 1)?;
                if w != 0 {
                    let k = self.new_margin_kern(-w, 1)?;
                    self.mem[k as usize].s1 = self.mem[ptmp as usize].s1;
                    self.mem[ptmp as usize].s1 = k;
                    if ptmp == q {
                        q = self.mem[q as usize].s1;
                    }
                }
            }

            if !glue_break {
                let r = self.new_param_glue(GLUE_PAR_RIGHT_SKIP)?;
                self.mem[r as usize].s1 = self.mem[q as usize].s1;
                self.mem[q as usize].s1 = r;
                q = r;
            }

            // TeX--XeT LR closures before the line end (C ~1496).
            if self.int_par(INT_PAR_TEXXET)? > 0 && lr_ptr != TEX_NULL {
                let mut s = temp_head;
                let mut r = self.mem[s as usize].s1;
                while r != q {
                    s = r;
                    r = self.mem[s as usize].s1;
                }
                let mut r = lr_ptr;
                while r != TEX_NULL {
                    let temp = self.new_math(0, self.mem[r as usize].s0)?;
                    self.mem[s as usize].s1 = temp;
                    s = temp;
                    r = self.mem[r as usize].s1;
                }
                self.mem[s as usize].s1 = q;
            }

            // Detach the line at q (C ~916).
            let r = self.mem[q as usize].s1;
            self.mem[q as usize].s1 = TEX_NULL;
            let mut q = self.mem[temp_head as usize].s1;
            self.mem[temp_head as usize].s1 = r;

            if self.int_par(INT_PAR_XETEX_PROTRUDE_CHARS)? > 0 {
                let p = self.find_protchar_left(q, false)?;
                let w = self.char_pw(p, 0)?;
                if w != 0 {
                    let k = self.new_margin_kern(-w, 0)?;
                    self.mem[k as usize].s1 = q;
                    q = k;
                }
            }

            if self.glue_par(GLUE_PAR_LEFT_SKIP)? != TEX_NULL {
                let r = self.new_param_glue(GLUE_PAR_LEFT_SKIP)?;
                self.mem[r as usize].s1 = q;
                q = r;
            }

            let (cur_width, cur_indent);
            if cur_line > self.lb_last_special_line {
                cur_width = self.lb_second_width;
                cur_indent = self.lb_second_indent;
            } else if self.local(LOCAL_PAR_SHAPE)? == TEX_NULL {
                cur_width = self.lb_first_width;
                cur_indent = self.lb_first_indent;
            } else {
                let shape = self.local(LOCAL_PAR_SHAPE)?;
                cur_width = self.mem[(shape + 2 * cur_line) as usize].s1;
                cur_indent = self.mem[(shape + 2 * cur_line - 1) as usize].s1;
            }

            self.adjust_tail = self.adjust_head();
            self.pre_adjust_tail = self.pre_adjust_head();

            if self.semantic_pagination_enabled {
                self.just_box = self.hpack(q, 0, ADDITIONAL)?;
            } else {
                self.just_box = self.hpack(q, cur_width, EXACTLY)?;
            }
            self.mem[(self.just_box + 4) as usize].s1 = cur_indent;

            let pre_adjust_head = self.pre_adjust_head();
            if pre_adjust_head != self.pre_adjust_tail {
                self.mem[self.tail as usize].s1 = self.mem[pre_adjust_head as usize].s1;
                self.tail = self.pre_adjust_tail;
            }
            self.pre_adjust_tail = TEX_NULL;
            self.append_to_vlist(self.just_box)?;

            let adjust_head = self.adjust_head();
            if adjust_head != self.adjust_tail {
                self.mem[self.tail as usize].s1 = self.mem[adjust_head as usize].s1;
                self.tail = self.adjust_tail;
            }
            self.adjust_tail = TEX_NULL;

            // Penalties between lines (C ~919).
            if cur_line + 1 != self.lb_best_line {
                let mut pen: i32;
                let q = self.eqtb[ETEX_PEN_BASE + ETEX_PEN_INTER_LINE].s1;
                if q != TEX_NULL {
                    let mut r = cur_line;
                    if r > self.mem[(q + 1) as usize].s1 {
                        r = self.mem[(q + 1) as usize].s1;
                    }
                    pen = self.mem[(q + 1 + r) as usize].s1;
                } else {
                    pen = self.int_par(INT_PAR_INTER_LINE_PENALTY)?;
                }
                let q = self.eqtb[ETEX_PEN_BASE + ETEX_PEN_CLUB].s1;
                if q != TEX_NULL {
                    let mut r = cur_line - self.prev_graf;
                    if r > self.mem[(q + 1) as usize].s1 {
                        r = self.mem[(q + 1) as usize].s1;
                    }
                    pen += self.mem[(q + 1 + r) as usize].s1;
                } else if cur_line == self.prev_graf + 1 {
                    pen += self.int_par(INT_PAR_CLUB_PENALTY)?;
                }
                let q = self.eqtb[ETEX_PEN_BASE
                    + if d {
                        ETEX_PEN_DISPLAY_WIDOW
                    } else {
                        ETEX_PEN_WIDOW
                    }]
                .s1;
                if q != TEX_NULL {
                    let mut r = self.lb_best_line - cur_line - 1;
                    if r > self.mem[(q + 1) as usize].s1 {
                        r = self.mem[(q + 1) as usize].s1;
                    }
                    pen += self.mem[(q + 1 + r) as usize].s1;
                } else if cur_line + 2 == self.lb_best_line {
                    pen += if d {
                        self.int_par(INT_PAR_DISPLAY_WIDOW_PENALTY)?
                    } else {
                        self.int_par(INT_PAR_WIDOW_PENALTY)?
                    };
                }
                if disc_break {
                    pen += self.int_par(INT_PAR_BROKEN_PENALTY)?;
                }
                if pen != 0 {
                    let r = self.new_penalty(pen)?;
                    self.mem[self.tail as usize].s1 = r;
                    self.tail = r;
                }
            }

            cur_line += 1;
            self.lb_cur_p = self.mem[(self.lb_cur_p + 1) as usize].s0;

            if self.lb_cur_p != TEX_NULL && !post_disc_break {
                // Prune discardable nodes before the next breakpoint
                // (C ~908).
                let mut r = temp_head;
                loop {
                    let q = self.mem[r as usize].s1;
                    if q == self.mem[(self.lb_cur_p + 1) as usize].s1 {
                        break;
                    }
                    if self.is_char_node(q) {
                        break;
                    }
                    if !self.is_char_node(q) && self.node_type(q)? < MATH_NODE {
                        break;
                    }
                    if self.node_type(q)? == KERN_NODE
                        && self.node_subtype(q)? != EXPLICIT
                        && self.node_subtype(q)? != SPACE_ADJUSTMENT
                    {
                        break;
                    }
                    r = q;
                    if self.node_type(q)? == MATH_NODE && self.int_par(INT_PAR_TEXXET)? > 0 {
                        let subtype = i32::from(self.mem[q as usize].b16_s0());
                        if subtype % 2 != 0 {
                            let expected = L_CODE * (subtype / L_CODE) + 3;
                            if lr_ptr != TEX_NULL && self.mem[lr_ptr as usize].s0 == expected {
                                let temp = lr_ptr;
                                lr_ptr = self.mem[temp as usize].s1;
                                self.mem[temp as usize].s1 = self.avail;
                                self.avail = temp;
                            }
                        } else {
                            let temp = self.get_avail()?;
                            self.mem[temp as usize].s0 = L_CODE * (subtype / L_CODE) + 3;
                            self.mem[temp as usize].s1 = lr_ptr;
                            lr_ptr = temp;
                        }
                    }
                }
                if r != temp_head {
                    self.mem[r as usize].s1 = TEX_NULL;
                    self.flush_node_list(self.mem[temp_head as usize].s1)?;
                    self.mem[temp_head as usize].s1 = q;
                }
            }

            if self.lb_cur_p == TEX_NULL {
                break;
            }
        }

        if cur_line != self.lb_best_line || self.mem[temp_head as usize].s1 != TEX_NULL {
            return Err(TeXMemoryError::CorruptedNode(cur_line).into());
        }

        self.prev_graf = self.lb_best_line - 1;
        self.etex_aux = lr_ptr;
        Ok(())
    }

    /// Collects the hyphenatable word following glue node `cur_p`,
    /// fills `lb_hu`/`lb_hc`/`lb_hn`, and invokes `hyphenate` when the
    /// surrounding nodes permit it (the C 924 hyphenation block).
    fn lb_hyphenate_next_word(&mut self, cur_p: i32) -> Result<(), TeXMouthError> {
        let mut prev_s = cur_p;
        let mut s = self.link(prev_s)?;
        if s == TEX_NULL {
            return Ok(());
        }

        // Skip to node ha: the first hyphenatable character (C ~930).
        let mut c: i32 = 0;
        let mut found = false;
        while !found {
            let mut cont = false;
            if self.is_char_node(s) {
                c = i32::from(self.mem[s as usize].b16_s0());
                self.lb_hf = i32::from(self.mem[s as usize].b16_s1());
            } else {
                match self.node_type(s)? {
                    LIGATURE_NODE => {
                        if self.lig_ptr(s)? == TEX_NULL {
                            cont = true;
                        } else {
                            let q = self.lig_ptr(s)?;
                            c = i32::from(self.mem[q as usize].b16_s0());
                            self.lb_hf = i32::from(self.mem[q as usize].b16_s1());
                        }
                    }
                    KERN_NODE if self.node_subtype(s)? == NORMAL => cont = true,
                    MATH_NODE if i32::from(self.node_subtype(s)?) >= L_CODE => cont = true,
                    WHATSIT_NODE => {
                        let subtype = self.node_subtype(s)?;
                        if subtype == NATIVE_WORD_NODE || subtype == NATIVE_WORD_NODE_AT {
                            let len = self.native_node_length(s)?;
                            let mut l = 0;
                            while l < len {
                                let cc = self.get_native_usv(s, l)?;
                                let cc_lc = self.lc_code(cc.max(0) as u32);
                                if cc_lc != 0 {
                                    self.lb_hf = i32::from(self.word_at(s, 4)?.b16_s2());
                                    prev_s = s;
                                    if cc_lc == cc || self.int_par(INT_PAR_UC_HYPH)? > 0 {
                                        c = cc;
                                        found = true;
                                    } else {
                                        return Ok(());
                                    }
                                    break;
                                }
                                if cc >= 65536 {
                                    l += 1;
                                }
                                l += 1;
                            }
                        }
                        if subtype == LANGUAGE_NODE && !found {
                            self.cur_lang = self.mem[(s + 1) as usize].s1;
                            self.lb_l_hyf = i32::from(self.word_at(s, 1)?.b16_s1());
                            self.lb_r_hyf = i32::from(self.word_at(s, 1)?.b16_s0());
                        }
                        cont = !found;
                    }
                    _ => return Ok(()),
                }
            }
            if !found && !cont {
                let hc0 = self.lc_code(c.max(0) as u32);
                if hc0 == 0 {
                    // keep scanning (_continue)
                } else if hc0 == c || self.int_par(INT_PAR_UC_HYPH)? > 0 {
                    found = true;
                } else {
                    return Ok(());
                }
            }
            if !found {
                prev_s = s;
                s = self.link(prev_s)?;
            }
        }

        self.lb_hyf_char = self
            .font_record(self.lb_hf)
            .map_or(-1, |record| record.hyphen_char);
        if self.lb_hyf_char < 0 || self.lb_hyf_char > BIGGEST_CHAR {
            return Ok(());
        }
        self.lb_ha = prev_s;
        let native_ha = !self.is_char_node(prev_s)
            && self.node_type(prev_s)? == WHATSIT_NODE
            && matches!(
                self.node_subtype(prev_s)?,
                NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT
            );
        if self.lb_l_hyf + self.lb_r_hyf > self.max_hyphenatable_length()? {
            return Ok(());
        }

        if native_ha {
            // Nodes following a native-word ha must permit hyphenation
            // (C ~926).
            s = self.link(self.lb_ha)?;
            loop {
                if !self.is_char_node(s) {
                    match self.node_type(s)? {
                        LIGATURE_NODE => {}
                        KERN_NODE if self.node_subtype(s)? != NORMAL => break,
                        WHATSIT_NODE | GLUE_NODE | PENALTY_NODE | INS_NODE | MARK_NODE
                        | ADJUST_NODE => break,
                        _ => return Ok(()),
                    }
                }
                s = self.link(s)?;
            }

            // Collect the word's characters into lb_hu/lb_hc (C ~927).
            self.lb_hn = 0;
            'restart: loop {
                let len = self.native_node_length(self.lb_ha)?;
                let mut l = 0;
                while l < len {
                    let cc = self.get_native_usv(self.lb_ha, l)?;
                    let hc0 = self.lc_code(cc.max(0) as u32);
                    if hc0 == 0 {
                        if self.lb_hn > 0 {
                            // Split the tail off as a new native word.
                            let q = self.new_native_word_node(self.lb_hf, len - l)?;
                            self.set_node_subtype(q, self.node_subtype(self.lb_ha)?)?;
                            for i in l..len {
                                let v = self.native_word_text_at(self.lb_ha, i)?;
                                self.set_native_word_text_at(q, i - l, v)?;
                            }
                            self.set_native_metrics(q)?;
                            self.set_link(q, self.link(self.lb_ha)?)?;
                            self.set_link(self.lb_ha, q)?;
                            self.mem[(self.lb_ha + 4) as usize].set_b16_s1(l as u16);
                            self.set_native_metrics(self.lb_ha)?;
                            break 'restart;
                        }
                    } else if self.lb_hn == 0 && l > 0 {
                        let q = self.new_native_word_node(self.lb_hf, len - l)?;
                        self.set_node_subtype(q, self.node_subtype(self.lb_ha)?)?;
                        for i in l..len {
                            let v = self.native_word_text_at(self.lb_ha, i)?;
                            self.set_native_word_text_at(q, i - l, v)?;
                        }
                        self.set_native_metrics(q)?;
                        self.set_link(q, self.link(self.lb_ha)?)?;
                        self.set_link(self.lb_ha, q)?;
                        self.mem[(self.lb_ha + 4) as usize].set_b16_s1(l as u16);
                        self.set_native_metrics(self.lb_ha)?;
                        self.lb_ha = self.link(self.lb_ha)?;
                        continue 'restart;
                    } else if self.lb_hn == self.max_hyphenatable_length()? {
                        break 'restart;
                    } else {
                        self.lb_hn += 1;
                        if cc < 65536 {
                            self.lb_hu[self.lb_hn as usize] = cc;
                            self.lb_hc[self.lb_hn as usize] = hc0;
                        } else {
                            self.lb_hu[self.lb_hn as usize] = (cc - 65536) / 1024 + 0xD800;
                            self.lb_hc[self.lb_hn as usize] = (hc0 - 65536) / 1024 + 0xD800;
                            self.lb_hn += 1;
                            self.lb_hu[self.lb_hn as usize] = cc % 1024 + 0xDC00;
                            self.lb_hc[self.lb_hn as usize] = hc0 % 1024 + 0xDC00;
                            l += 1;
                        }
                        self.lb_hyf_bchar = TOO_BIG_CHAR;
                    }
                    l += 1;
                }
                break;
            }
        } else {
            // Skip to hb collecting letters into lb_hu/lb_hc (C ~931).
            self.lb_hn = 0;
            'collect: loop {
                if self.is_char_node(s) {
                    if i32::from(self.mem[s as usize].b16_s1()) != self.lb_hf {
                        break 'collect;
                    }
                    self.lb_hyf_bchar = i32::from(self.mem[s as usize].b16_s0());
                    let cc = self.lb_hyf_bchar;
                    let hc0 = self.lc_code(cc.max(0) as u32);
                    if hc0 == 0 || hc0 > MAX_HYPH_CHAR {
                        break 'collect;
                    }
                    if self.lb_hn == self.max_hyphenatable_length()? {
                        break 'collect;
                    }
                    self.lb_hb = s;
                    self.lb_hn += 1;
                    self.lb_hu[self.lb_hn as usize] = cc;
                    self.lb_hc[self.lb_hn as usize] = hc0;
                    self.lb_hyf_bchar = TOO_BIG_CHAR;
                } else if self.node_type(s)? == LIGATURE_NODE {
                    if self.lig_font(s)? != self.lb_hf {
                        break 'collect;
                    }
                    let mut j = self.lb_hn;
                    let mut q = self.lig_ptr(s)?;
                    if q > TEX_NULL {
                        self.lb_hyf_bchar = i32::from(self.mem[q as usize].b16_s0());
                    }
                    while q > TEX_NULL {
                        let cc = i32::from(self.mem[q as usize].b16_s0());
                        let hc0 = self.lc_code(cc.max(0) as u32);
                        if hc0 == 0 || hc0 > MAX_HYPH_CHAR {
                            break 'collect;
                        }
                        if j == self.max_hyphenatable_length()? {
                            break 'collect;
                        }
                        j += 1;
                        self.lb_hu[j as usize] = cc;
                        self.lb_hc[j as usize] = hc0;
                        q = self.link(q)?;
                    }
                    self.lb_hb = s;
                    self.lb_hn = j;
                    self.lb_hyf_bchar = if self.node_subtype(s)? % 2 != 0 {
                        self.font_record(self.lb_hf)
                            .map_or(TOO_BIG_CHAR, |record| record.bchar)
                    } else {
                        TOO_BIG_CHAR
                    };
                } else if self.node_type(s)? == KERN_NODE && self.node_subtype(s)? == NORMAL {
                    self.lb_hb = s;
                    self.lb_hyf_bchar = self
                        .font_record(self.lb_hf)
                        .map_or(TOO_BIG_CHAR, |record| record.bchar);
                } else {
                    break 'collect;
                }
                s = self.link(s)?;
            }
        }

        // Nodes after hb must permit hyphenation (C ~933).
        if self.lb_hn < self.lb_l_hyf + self.lb_r_hyf {
            return Ok(());
        }
        loop {
            if !self.is_char_node(s) {
                match self.node_type(s)? {
                    LIGATURE_NODE => {}
                    KERN_NODE if self.node_subtype(s)? != NORMAL => break,
                    WHATSIT_NODE | GLUE_NODE | PENALTY_NODE | INS_NODE | MARK_NODE
                    | ADJUST_NODE => break,
                    MATH_NODE if i32::from(self.node_subtype(s)?) >= L_CODE => break,
                    _ => return Ok(()),
                }
            }
            s = self.link(s)?;
        }

        self.hyphenate()
    }

    /// Scores the collected word via exceptions and patterns, then
    /// splices discretionary nodes into the list (`hyphenate`).
    fn hyphenate(&mut self) -> Result<(), TeXMouthError> {
        let hold_head = self.hold_head();
        for j in 0..=self.lb_hn.max(0) {
            self.lb_hyf[j as usize] = 0;
        }

        // Exception dictionary lookup (C ~934); the dictionary key is
        // the lowercased word followed by the language.
        let mut key: Vec<i32> = (1..=self.lb_hn.max(0))
            .map(|i| self.lb_hc[i as usize])
            .collect();
        key.push(self.cur_lang);
        let mut found = false;
        if let Some(positions) = self.hyph_exceptions.get(&key).cloned() {
            for p in positions {
                if p >= 0 && (p as usize) < self.lb_hyf.len() {
                    self.lb_hyf[p as usize] = 1;
                }
            }
            found = true;
        }

        if !found {
            // Pattern pass (C ~937). The word is padded as
            // `hc[0]=0, hc[hn+1]=0, hc[hn+2]=max_hyph_char`; pattern
            // letters (including '.', which compares as lc_code 0) are
            // matched consecutively and gap values raise `hyf`.
            let lang = self.cur_lang;
            let mut any = false;
            let mut hc_ext: Vec<i32> = Vec::with_capacity(self.lb_hn.max(0) as usize + 3);
            hc_ext.push(0);
            for i in 1..=self.lb_hn.max(0) {
                hc_ext.push(self.lb_hc[i as usize]);
            }
            hc_ext.push(0);
            hc_ext.push(MAX_HYPH_CHAR);
            let patterns: Vec<(Vec<i32>, Vec<i32>)> = self
                .hyph_patterns
                .iter()
                .filter(|(k, _)| k.last() == Some(&lang))
                .map(|(k, v)| (k[..k.len() - 1].to_vec(), v.clone()))
                .collect();
            for (pat, digits) in &patterns {
                any = true;
                let mut letters: Vec<i32> = Vec::new();
                let mut vals: Vec<i32> = vec![0];
                for (i, &pc) in pat.iter().enumerate() {
                    let dv = digits.get(i).copied().unwrap_or(0);
                    if dv != 0 {
                        if let Some(v) = vals.last_mut() {
                            *v = dv;
                        }
                    } else {
                        letters.push(pc);
                        vals.push(0);
                    }
                }
                let m = letters.len() as i32;
                let mut o = 0;
                while o <= self.lb_hn - self.lb_r_hyf + 1 {
                    if o + m > hc_ext.len() as i32 {
                        break;
                    }
                    let mut matched = true;
                    for (k, &pl) in letters.iter().enumerate() {
                        if self.lc_code(pl.max(0) as u32) != hc_ext[(o + k as i32) as usize] {
                            matched = false;
                            break;
                        }
                    }
                    if matched {
                        for k in 0..=m {
                            let idx = (o + k) as usize;
                            if idx < self.lb_hyf.len() && vals[k as usize] > self.lb_hyf[idx] {
                                self.lb_hyf[idx] = vals[k as usize];
                            }
                        }
                    }
                    o += 1;
                }
            }
            if !any {
                return Ok(());
            }
        }

        for j in 0..self.lb_l_hyf.max(0) {
            if (j as usize) < self.lb_hyf.len() {
                self.lb_hyf[j as usize] = 0;
            }
        }
        for j in 0..self.lb_r_hyf.max(0) {
            let idx = self.lb_hn - j;
            if idx >= 0 && (idx as usize) < self.lb_hyf.len() {
                self.lb_hyf[idx as usize] = 0;
            }
        }

        let mut j = self.lb_l_hyf;
        let end = self.lb_hn - self.lb_r_hyf;
        while j <= end {
            if self.lb_hyf[j as usize] % 2 != 0 {
                break;
            }
            j += 1;
        }
        if j > end {
            return Ok(());
        }

        // found1: reconstruct the word with discretionary nodes.
        if !self.is_char_node(self.lb_ha)
            && self.node_type(self.lb_ha)? == WHATSIT_NODE
            && matches!(
                self.node_subtype(self.lb_ha)?,
                NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT
            )
        {
            // Native word: split into in-list fragments separated by
            // disc nodes carrying a native hyphen character.
            let mut s = self.lb_cur_p;
            while self.link(s)? != self.lb_ha {
                s = self.link(s)?;
            }
            let mut hyphen_passed = 0;
            let mut j = self.lb_l_hyf;
            while j <= self.lb_hn - self.lb_r_hyf {
                if self.lb_hyf[j as usize] % 2 != 0 {
                    let q = self.new_native_word_node(self.lb_hf, j - hyphen_passed)?;
                    self.set_node_subtype(q, self.node_subtype(self.lb_ha)?)?;
                    for i in 0..(j - hyphen_passed) {
                        let v = self.native_word_text_at(self.lb_ha, i + hyphen_passed)?;
                        self.set_native_word_text_at(q, i, v)?;
                    }
                    self.set_native_metrics(q)?;
                    self.set_link(s, q)?;
                    s = q;
                    let d = self.new_disc()?;
                    let hyf_node = self.new_native_character(self.lb_hf, self.lb_hyf_char)?;
                    self.mem[(d + 1) as usize].s0 = hyf_node;
                    self.set_link(s, d)?;
                    s = d;
                    hyphen_passed = j;
                }
                j += 1;
            }
            let hn = self.native_node_length(self.lb_ha)?;
            let q = self.new_native_word_node(self.lb_hf, hn - hyphen_passed)?;
            self.set_node_subtype(q, self.node_subtype(self.lb_ha)?)?;
            for i in 0..(hn - hyphen_passed) {
                let v = self.native_word_text_at(self.lb_ha, i + hyphen_passed)?;
                self.set_native_word_text_at(q, i, v)?;
            }
            self.set_native_metrics(q)?;
            self.set_link(s, q)?;
            s = q;
            let q = self.link(self.lb_ha)?;
            self.set_link(s, q)?;
            self.set_link(self.lb_ha, TEX_NULL)?;
            self.flush_node_list(self.lb_ha)?;
            return Ok(());
        }

        // TFM path (C ~938): detach the word, then replay
        // `reconstitute` per break position.
        let q = self.link(self.lb_hb)?;
        self.set_link(self.lb_hb, TEX_NULL)?;
        let r = self.link(self.lb_ha)?;
        self.set_link(self.lb_ha, TEX_NULL)?;
        let bchar = self.lb_hyf_bchar;
        let mut s: i32;
        let mut j: i32;
        let init_list;
        let mut init_lig = false;
        let mut init_lft = false;
        self.lb_hu[0] = MAX_HYPH_CHAR;

        if self.is_char_node(self.lb_ha) {
            if i32::from(self.mem[self.lb_ha as usize].b16_s1()) != self.lb_hf {
                s = self.lb_ha;
                j = 0;
                self.lb_hu[0] = MAX_HYPH_CHAR;
                init_lig = false;
                init_list = TEX_NULL;
            } else {
                init_list = self.lb_ha;
                init_lig = false;
                self.lb_hu[0] = i32::from(self.mem[self.lb_ha as usize].b16_s0());
                s = self.lb_cur_p;
                while self.link(s)? != self.lb_ha {
                    s = self.link(s)?;
                }
                j = 0;
            }
        } else if self.node_type(self.lb_ha)? == LIGATURE_NODE {
            if self.lig_font(self.lb_ha)? != self.lb_hf {
                s = self.lb_ha;
                j = 0;
                self.lb_hu[0] = MAX_HYPH_CHAR;
                init_lig = false;
                init_list = TEX_NULL;
            } else {
                init_list = self.lig_ptr(self.lb_ha)?;
                init_lig = true;
                init_lft = self.node_subtype(self.lb_ha)? > 1;
                self.lb_hu[0] = self.lig_char(self.lb_ha)?;
                if init_list == TEX_NULL && init_lft {
                    self.lb_hu[0] = MAX_HYPH_CHAR;
                    init_lig = false;
                }
                self.free_node(self.lb_ha, SMALL_NODE_SIZE)?;
                s = self.lb_cur_p;
                while self.link(s)? != self.lb_ha {
                    s = self.link(s)?;
                }
                j = 0;
            }
        } else {
            if !self.is_char_node(r)
                && self.node_type(r)? == LIGATURE_NODE
                && self.node_subtype(r)? > 1
            {
                s = self.lb_ha;
                j = 0;
                self.lb_hu[0] = MAX_HYPH_CHAR;
                init_lig = false;
                init_list = TEX_NULL;
            } else {
                j = 1;
                s = self.lb_ha;
                init_list = TEX_NULL;
            }
        }

        self.lb_init_list = init_list;
        self.lb_init_lig = init_lig;
        self.lb_init_lft = init_lft;
        self.lb_hyphen_passed = 0;

        // common_ending: replay the lig/kern program, inserting a disc
        // node at each permissible break.
        self.flush_node_list(r)?;
        let mut l;
        loop {
            l = j;
            j = self.reconstitute(j, self.lb_hn, bchar, self.lb_hyf_char)? + 1;
            if self.lb_hyphen_passed == 0 {
                self.set_link(s, self.link(hold_head)?)?;
                while self.link(s)? > TEX_NULL {
                    s = self.link(s)?;
                }
                if self.lb_hyf[(j - 1) as usize] % 2 != 0 {
                    l = j;
                    self.lb_hyphen_passed = j - 1;
                    self.set_link(hold_head, TEX_NULL)?;
                }
            }
            if self.lb_hyphen_passed > 0 {
                loop {
                    let r = self.get_node(SMALL_NODE_SIZE)?;
                    self.set_link(r, self.link(hold_head)?)?;
                    self.set_node_type(r, DISC_NODE)?;
                    let mut major_tail = r;
                    let mut r_count = 0;
                    while self.link(major_tail)? > TEX_NULL {
                        major_tail = self.link(major_tail)?;
                        r_count += 1;
                    }
                    let mut i = self.lb_hyphen_passed;
                    self.lb_hyf[i as usize] = 0;
                    let mut minor_tail = TEX_NULL;
                    self.mem[(r + 1) as usize].s0 = TEX_NULL;
                    let hyf_node = self.new_character(self.lb_hf, self.lb_hyf_char)?;
                    let mut c = 0;
                    if hyf_node != TEX_NULL {
                        i += 1;
                        c = self.lb_hu[i as usize];
                        self.lb_hu[i as usize] = self.lb_hyf_char;
                        self.mem[hyf_node as usize].s1 = self.avail;
                        self.avail = hyf_node;
                    }
                    while l <= i {
                        l = self.reconstitute(
                            l,
                            i,
                            self.font_record(self.lb_hf)
                                .map_or(TOO_BIG_CHAR, |record| record.bchar),
                            TOO_BIG_CHAR,
                        )? + 1;
                        if self.link(hold_head)? > TEX_NULL {
                            if minor_tail == TEX_NULL {
                                self.mem[(r + 1) as usize].s0 = self.link(hold_head)?;
                            } else {
                                self.set_link(minor_tail, self.link(hold_head)?)?;
                            }
                            minor_tail = self.link(hold_head)?;
                            while self.link(minor_tail)? > TEX_NULL {
                                minor_tail = self.link(minor_tail)?;
                            }
                        }
                    }
                    if hyf_node != TEX_NULL {
                        self.lb_hu[i as usize] = c;
                        l = i;
                    }
                    minor_tail = TEX_NULL;
                    self.mem[(r + 1) as usize].s1 = TEX_NULL;
                    let mut c_loc = 0;
                    let mut c2 = 0;
                    if self
                        .font_record(self.lb_hf)
                        .map_or(NON_ADDRESS, |record| record.bchar_label)
                        != NON_ADDRESS
                    {
                        l -= 1;
                        c2 = self.lb_hu[l as usize];
                        c_loc = l;
                        self.lb_hu[l as usize] = MAX_HYPH_CHAR;
                    }
                    while l < j {
                        loop {
                            l = self.reconstitute(l, self.lb_hn, bchar, TOO_BIG_CHAR)? + 1;
                            if c_loc > 0 {
                                self.lb_hu[c_loc as usize] = c2;
                                c_loc = 0;
                            }
                            if self.link(hold_head)? > TEX_NULL {
                                if minor_tail == TEX_NULL {
                                    self.mem[(r + 1) as usize].s1 = self.link(hold_head)?;
                                } else {
                                    self.set_link(minor_tail, self.link(hold_head)?)?;
                                }
                                minor_tail = self.link(hold_head)?;
                                while self.link(minor_tail)? > TEX_NULL {
                                    minor_tail = self.link(minor_tail)?;
                                }
                            }
                            if l >= j {
                                break;
                            }
                        }
                        while l > j {
                            j = self.reconstitute(j, self.lb_hn, bchar, TOO_BIG_CHAR)? + 1;
                            self.set_link(major_tail, self.link(hold_head)?)?;
                            while self.link(major_tail)? > TEX_NULL {
                                major_tail = self.link(major_tail)?;
                                r_count += 1;
                            }
                        }
                    }
                    if r_count > 127 {
                        self.set_link(s, self.link(r)?)?;
                        self.set_link(r, TEX_NULL)?;
                        self.flush_node_list(r)?;
                    } else {
                        self.set_link(s, r)?;
                        self.mem[r as usize].set_b16_s0(r_count as u16);
                    }
                    s = major_tail;
                    self.lb_hyphen_passed = j - 1;
                    self.set_link(hold_head, TEX_NULL)?;
                    if self.lb_hyf[(j - 1) as usize] % 2 == 0 {
                        break;
                    }
                }
            }
            if j > self.lb_hn {
                break;
            }
        }
        self.set_link(s, q)?;
        self.flush_list(self.lb_init_list)?;
        Ok(())
    }

    /// Completes `cur_box` according to its saved context (`box_end`).
    fn box_end(&mut self, box_context: i32) -> Result<(), TeXMouthError> {
        if box_context < BOX_FLAG {
            if self.cur_box != TEX_NULL {
                self.mem[self.cur_box as usize + 4].s1 = box_context;
                if self.mode.abs() == TeXMode::Vmode as i16 {
                    if self.pre_adjust_tail != TEX_NULL {
                        let head = self.pre_adjust_head();
                        if head != self.pre_adjust_tail {
                            self.mem[self.tail as usize].s1 = self.mem[head as usize].s1;
                            self.tail = self.pre_adjust_tail;
                        }
                        self.pre_adjust_tail = TEX_NULL;
                    }
                    self.append_to_vlist(self.cur_box)?;
                    if self.adjust_tail != TEX_NULL {
                        let head = self.adjust_head();
                        if head != self.adjust_tail {
                            self.mem[self.tail as usize].s1 = self.mem[head as usize].s1;
                            self.tail = self.adjust_tail;
                        }
                        self.adjust_tail = TEX_NULL;
                    }
                    if self.mode > 0 {
                        self.build_page()?;
                    }
                } else {
                    if self.mode.abs() == TeXMode::Hmode as i16 {
                        self.aux.s0 = 1000;
                    } else {
                        let p = self.new_noad()?;
                        self.mem[p as usize + 1].s1 = SUB_BOX;
                        self.mem[p as usize + 1].s0 = self.cur_box;
                        self.cur_box = p;
                    }
                    self.mem[self.tail as usize].s1 = self.cur_box;
                    self.tail = self.cur_box;
                }
            }
            return Ok(());
        }

        if box_context < SHIP_OUT_FLAG {
            let (n, global) = if box_context < GLOBAL_BOX_FLAG {
                (box_context - BOX_FLAG, false)
            } else {
                (box_context - GLOBAL_BOX_FLAG, true)
            };
            self.cur_val = n;
            if n < 256 {
                let target = BOX_BASE + n as usize;
                if global {
                    self.geq_define(target, CommandCode::BoxRef as u16, self.cur_box)?;
                } else {
                    self.eq_define(target, CommandCode::BoxRef as u16, self.cur_box)?;
                }
            } else {
                let element = self.find_sa_element(SA_BOX_VAL, n, true)?;
                if global {
                    self.gsa_def(element, self.cur_box)?;
                } else {
                    self.sa_def(element, self.cur_box)?;
                }
            }
            return Ok(());
        }

        if self.cur_box == TEX_NULL {
            return Ok(());
        }
        if box_context == SHIP_OUT_FLAG {
            return self.ship_out(self.cur_box);
        }

        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 && self.cur_cmd != CommandCode::Relax as u8
            {
                break;
            }
        }
        if (self.cur_cmd == CommandCode::Hskip as u8 && self.mode.abs() != TeXMode::Vmode as i16)
            || (self.cur_cmd == CommandCode::Vskip as u8
                && self.mode.abs() == TeXMode::Vmode as i16)
        {
            self.append_glue()?;
            self.mem[self.tail as usize]
                .set_b16_s0((box_context - (LEADER_FLAG - i32::from(A_LEADERS as u16))) as u16);
            self.mem[self.tail as usize + 1].s1 = self.cur_box;
            Ok(())
        } else {
            self.back_input()?;
            self.flush_node_list(self.cur_box)?;
            Err(TeXMouthError::UnsupportedPrimitive(
                "leaders not followed by glue",
            ))
        }
    }

    /// Closes a packaging group and packages its list (`package`).
    fn package(&mut self, c: i32) -> Result<(), TeXMouthError> {
        let max_depth = self.dimen_par(DIMEN_PAR_BOX_MAX_DEPTH)?;
        let saved_upwards = self.int_par(INT_PAR_XETEX_UPWARDS)?;
        self.unsave()?;
        self.save_ptr -= 3;
        let inner_upwards = self.int_par(INT_PAR_XETEX_UPWARDS)?;
        self.set_int_par(INT_PAR_XETEX_UPWARDS, saved_upwards)?;

        let raw = |index: usize| -> Result<i32, TeXMouthError> {
            match self.save_stack.get(index) {
                Some(SaveStackEntry::RawValue(value)) => Ok(*value),
                _ => Err(TeXMouthError::Memory(TeXMemoryError::CorruptedNode(
                    index as i32,
                ))),
            }
        };
        let saved_context = raw(self.save_ptr)?;
        let spec_code = raw(self.save_ptr + 1)?;
        let spec_width = raw(self.save_ptr + 2)?;

        let list = self.mem[self.head as usize].s1;
        self.cur_box = if self.mode == -(TeXMode::Hmode as i16) {
            self.hpack(list, spec_width, spec_code)?
        } else {
            let boxed = self.vpackage(list, spec_width, spec_code, max_depth)?;
            if c == VTOP_CODE {
                let first = self.mem[boxed as usize + 5].s1;
                let mut h = 0;
                if first != TEX_NULL && self.node_type(first)? <= RULE_NODE {
                    h = self.mem[first as usize + 3].s1;
                }
                self.mem[boxed as usize + 2].s1 =
                    self.mem[boxed as usize + 2].s1 - h + self.mem[boxed as usize + 3].s1;
                self.mem[boxed as usize + 3].s1 = h;
            }
            boxed
        };
        self.set_int_par(INT_PAR_XETEX_UPWARDS, inner_upwards)?;
        self.pop_nest()?;
        self.box_end(saved_context)
    }

    /// Closes a right-brace group for simple and packaging groups
    /// (`handle_right_brace`).
    /// Reports an extra group-closing token inside a semi-simple, math-shift,
    /// or `\left` group and recovers (`extra_right_brace`).
    fn extra_right_brace(&mut self) -> Result<(), TeXMouthError> {
        let closing = match self.cur_group {
            SEMI_SIMPLE_GROUP => "\\endgroup",
            MATH_SHIFT_GROUP => "$",
            MATH_LEFT_GROUP => "\\right",
            _ => "",
        };
        self.report_error(
            &format!("Extra }}, or forgotten {}", closing),
            &[
                "I've deleted a group-closing symbol because it seems to be",
                "spurious, as in `$x}$'. But perhaps the } is legitimate and",
                "you forgot something else, as in `\\hbox{$x}'. In such cases",
                "the way to recover is to insert both the forgotten and the",
                "deleted material, e.g., by typing `I$}'.",
            ],
        );
        self.align_state += 1;
        Ok(())
    }

    fn handle_right_brace(&mut self) -> Result<(), TeXMouthError> {
        match self.cur_group {
            SIMPLE_GROUP => self.unsave(),
            HBOX_GROUP => self.package(0),
            VBOX_GROUP => {
                self.end_graf()?;
                self.package(0)
            }
            ADJUSTED_HBOX_GROUP => {
                self.adjust_tail = self.adjust_head();
                self.pre_adjust_tail = self.pre_adjust_head();
                self.package(0)
            }
            VTOP_GROUP => {
                self.end_graf()?;
                self.package(VTOP_CODE)
            }
            INSERT_GROUP => self.denda(),
            DISC_GROUP => self.build_discretionary(),
            OUTPUT_GROUP => {
                if self.cur_input.loc != TEX_NULL
                    || (self.cur_input.index != OUTPUT_TEXT
                        && self.cur_input.index != crate::scanner::BACKED_UP)
                {
                    while self.cur_input.loc != TEX_NULL {
                        self.get_token()?;
                    }
                }
                self.end_token_list()?;
                self.end_graf()?;
                self.unsave()?;
                self.output_active = false;
                self.insert_penalties = 0;
                if self.box_reg(255)? != TEX_NULL {
                    self.box_error(255)?;
                }
                if self.tail != self.head {
                    self.mem[self.page_tail as usize].s1 = self.mem[self.head as usize].s1;
                    self.page_tail = self.tail;
                }
                let page_head = self.page_head();
                let contrib_head = self.contrib_head();
                if self.link(page_head)? != TEX_NULL {
                    if self.link(contrib_head)? == TEX_NULL {
                        self.nest[0].tail = self.page_tail;
                    }
                    let pending = self.link(contrib_head)?;
                    self.mem[self.page_tail as usize].s1 = pending;
                    let held_page = self.link(page_head)?;
                    self.set_link(contrib_head, held_page)?;
                    self.set_link(page_head, TEX_NULL)?;
                    self.page_tail = page_head;
                }
                self.flush_node_list(self.disc_ptr[LAST_BOX_CODE as usize])?;
                self.disc_ptr[LAST_BOX_CODE as usize] = TEX_NULL;
                self.pop_nest()?;
                self.build_page()
            }
            SEMI_SIMPLE_GROUP | MATH_SHIFT_GROUP | MATH_LEFT_GROUP => self.extra_right_brace(),
            ALIGN_GROUP => {
                self.back_input()?;
                self.cur_tok = CS_TOKEN_FLAG + crate::scanner::FROZEN_CR_ID;
                self.ins_error(
                    "Missing \\cr inserted",
                    &["I'm guessing that you meant to end an alignment here."],
                )
            }
            NO_ALIGN_GROUP => {
                self.end_graf()?;
                self.unsave()?;
                self.align_peek()
            }
            VCENTER_GROUP => {
                self.end_graf()?;
                self.unsave()?;
                self.save_ptr -= 2;
                let width = self.raw_save_value(self.save_ptr as i32 + 1).unwrap_or(0);
                let spec_code = self.raw_save_value(self.save_ptr as i32).unwrap_or(0);
                let p = self.vpackage(
                    self.mem[self.head as usize].s1,
                    width,
                    spec_code,
                    MAX_HALFWORD,
                )?;
                self.pop_nest()?;
                let q = self.new_noad()?;
                self.mem[self.tail as usize].s1 = q;
                self.tail = q;
                self.mem[q as usize].set_b16_s1(VCENTER_NOAD);
                self.mem[(q + 1) as usize].s1 = SUB_BOX;
                self.mem[(q + 1) as usize].s0 = p;
                Ok(())
            }
            MATH_CHOICE_GROUP => self.build_choices(),
            MATH_GROUP => {
                self.unsave()?;
                self.save_ptr -= 1;
                let addr = self
                    .raw_save_value(self.save_ptr as i32)
                    .unwrap_or(TEX_NULL);
                self.mem[addr as usize].s1 = SUB_MLIST;
                let p = self.fin_mlist(TEX_NULL)?;
                self.mem[addr as usize].s0 = p;
                if p != TEX_NULL && self.mem[p as usize].s1 == TEX_NULL {
                    if self.mem[p as usize].b16_s1() == ORD_NOAD {
                        if self.mem[(p + 3) as usize].s1 == MATH_FIELD_EMPTY
                            && self.mem[(p + 2) as usize].s1 == MATH_FIELD_EMPTY
                        {
                            self.mem[addr as usize] = self.mem[(p + 1) as usize];
                            self.free_node(p, NOAD_SIZE)?;
                        }
                    } else if self.mem[p as usize].b16_s1() == ACCENT_NOAD
                        && addr == self.tail + 1
                        && self.mem[self.tail as usize].b16_s1() == ORD_NOAD
                    {
                        // §1222: migrate scripts onto the accent nucleus.
                        let mut qq = self.head;
                        while self.mem[qq as usize].s1 != self.tail {
                            qq = self.mem[qq as usize].s1;
                        }
                        self.mem[qq as usize].s1 = p;
                        let tail = self.tail;
                        self.free_node(tail, NOAD_SIZE)?;
                        self.tail = p;
                    }
                }
                Ok(())
            }
            BOTTOM_LEVEL => Ok(()), // C reports "Too many }'s" and continues.
            _ => Err(TeXMouthError::UnsupportedPrimitive("right-brace group")),
        }
    }

    /// Collapses a newly scanned all-zero glue spec to the compact zero value
    /// (`trap_zero_glue`).
    fn trap_zero_glue(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_val == TEX_NULL {
            return Ok(());
        }
        let spec = self.checked_engine_mem_index(self.cur_val)?;
        if self.mem[spec + 1].s1 == 0 && self.mem[spec + 2].s1 == 0 && self.mem[spec + 3].s1 == 0 {
            self.delete_glue_ref(self.cur_val)?;
            self.cur_val = TEX_NULL;
        }
        Ok(())
    }

    /// Applies a local or global control-sequence definition.
    fn define_cs_meaning(
        &mut self,
        cs: i32,
        command: CommandCode,
        payload: i32,
        global: bool,
    ) -> Result<(), TeXMouthError> {
        if global {
            self.geq_define_cs(cs, command, payload)
        } else {
            self.eq_define_cs(cs, command, payload)
        }
    }

    /// Applies a local or global word-valued definition.
    fn define_word(&mut self, p: usize, global: bool) -> Result<(), TeXMouthError> {
        if global {
            self.geq_word_define(p, self.cur_val)?;
        } else {
            self.eq_word_define(p, self.cur_val)?;
        }
        Ok(())
    }

    /// Applies a local or global glue-valued definition.
    fn define_glue(&mut self, p: usize, global: bool) -> Result<(), TeXMouthError> {
        if global {
            self.geq_define(p, CommandCode::GlueRef as u16, self.cur_val)?;
        } else {
            self.eq_define(p, CommandCode::GlueRef as u16, self.cur_val)?;
        }
        Ok(())
    }

    /// Applies a local or global `eqtb` definition.
    fn define_eqtb(
        &mut self,
        p: usize,
        command: u16,
        value: i32,
        global: bool,
    ) -> Result<(), TeXMouthError> {
        if global {
            self.geq_define(p, command, value)?;
        } else {
            self.eq_define(p, command, value)?;
        }
        Ok(())
    }

    /// Converts a non-negative primitive `cur_chr` payload into an `eqtb` index.
    fn checked_eqtb_payload(&self, payload: i32) -> Result<usize, TeXMouthError> {
        let index =
            usize::try_from(payload).map_err(|_| TeXMouthError::InputOutOfBounds(payload))?;
        if index >= self.eqtb.len() {
            return Err(TeXMouthError::InputOutOfBounds(payload));
        }
        Ok(index)
    }

    /// Returns the first token node for either compact token-list storage form.
    ///
    /// Reference-counted lists use a `def_ref`-style head whose `s0` is a small
    /// count and whose `s1` links to the first token. Direct compact lists use
    /// the first token node itself.
    pub(crate) fn stored_token_list_start(&self, p: i32) -> Result<i32, TeXMouthError> {
        if p == TEX_NULL {
            return Ok(TEX_NULL);
        }
        let idx = self.checked_engine_mem_index(p)?;
        if self.mem[idx].s0 < MAX_CHAR_VAL {
            Ok(self.mem[idx].s1)
        } else {
            Ok(p)
        }
    }

    /// Returns a reference-counted head for a token-list value.
    ///
    /// The compact Rust `set_toks` API may store the first token node directly;
    /// `scan_toks` and C-style `eqtb` token assignments store a `def_ref` head
    /// whose `s0` field is the reference count. This normalizes both forms to
    /// the latter representation.
    fn token_list_reference(&mut self, p: i32) -> Result<i32, TeXMouthError> {
        let idx = self.checked_engine_mem_index(p)?;
        if self.mem[idx].s0 < MAX_CHAR_VAL {
            return Ok(p);
        }
        let reference = self.get_avail()?;
        let reference_idx = self.checked_engine_mem_index(reference)?;
        self.mem[reference_idx].s0 = TEX_NULL;
        self.mem[reference_idx].s1 = p;
        Ok(reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::{
        XETEX_COUNT_FEATURES_CODE, XETEX_FIND_FEATURE_BY_NAME_CODE, XETEX_INT,
        XETEX_OT_COUNT_SCRIPTS_CODE, XETEX_OT_FEATURE_CODE,
    };
    use crate::scanner::{
        CS_TOKEN_FLAG, END_MATCH_TOKEN, LEFT_BRACE_TOKEN, MATCH_TOKEN, OUT_PARAM_TOKEN,
        RIGHT_BRACE_TOKEN, SPACE_TOKEN,
    };
    use crate::state::{
        TeXMode, ADJUST_NODE, BOX_FLAG, CHAR_CLASS_LIMIT, FIL, GLUE_NODE, HBOX_GROUP, INSERT_GROUP,
        INS_NODE, INT_PAR_END_LINE_CHAR, KERN_NODE, LEVEL_ONE, LEVEL_ZERO, LOCAL_BASE,
        LOCAL_EVERY_EOF, LOCAL_PAR_SHAPE, PAGE_BOX_THERE, PENALTY_NODE, SELECTOR_LOG_ONLY,
        SELECTOR_TERM_ONLY, XXX1,
    };

    const LETTER_TOKEN: i32 = 11 * MAX_CHAR_VAL;

    fn engine() -> TeXEngineState {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state
    }

    fn cs(state: &TeXEngineState, name: &str) -> i32 {
        state.cs_table.lookup_str(name).unwrap()
    }

    fn definition_tokens(state: &TeXEngineState, cs: i32) -> Vec<i32> {
        let reference = state.cs_character(cs);
        let start = state.stored_token_list_start(reference).unwrap();
        state.token_list_to_vec(start).unwrap()
    }

    fn glue_parts(state: &TeXEngineState, spec: i32) -> (i32, i32, i32, u16, u16) {
        let spec = state.checked_engine_mem_index(spec).unwrap();
        (
            state.mem[spec + 1].s1,
            state.mem[spec + 2].s1,
            state.mem[spec + 3].s1,
            state.mem[spec].b16_s1(),
            state.mem[spec].b16_s0(),
        )
    }

    #[test]
    fn def_scans_template_and_replacement() {
        let mut state = engine();
        state.set_source_text("\\def\\foo#1{#1!}");
        state.dispatch_x_command().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Call as u8);
        assert_eq!(
            definition_tokens(&state, foo),
            vec![
                MATCH_TOKEN + i32::from(b'#'),
                END_MATCH_TOKEN,
                OUT_PARAM_TOKEN + 1,
                OTHER_TOKEN + i32::from(b'!'),
            ]
        );
    }

    #[test]
    fn local_def_is_restored_by_group_exit() {
        let mut state = engine();
        state.new_save_level(SIMPLE_GROUP).unwrap();
        state.set_source_text("\\def\\foo{x}");
        state.dispatch_x_command().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Call as u8);
        assert_eq!(state.cs_table.level(foo).unwrap(), LEVEL_ONE + 1);

        state.unsave().unwrap();
        assert_eq!(state.cs_command(foo), CommandCode::UndefinedCs as u8);
        assert_eq!(state.cs_table.level(foo).unwrap(), LEVEL_ZERO);
    }

    #[test]
    fn begingroup_and_endgroup_dispatch_save_levels() {
        let mut state = engine();
        state.set_source_text("\\begingroup\\def\\foo{x}\\endgroup");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::UndefinedCs as u8);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
    }

    #[test]
    fn gdef_survives_group_exit() {
        let mut state = engine();
        state.new_save_level(SIMPLE_GROUP).unwrap();
        state.set_source_text("\\gdef\\foo{x}");
        state.dispatch_x_command().unwrap();
        state.unsave().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Call as u8);
        assert_eq!(state.cs_table.level(foo).unwrap(), LEVEL_ONE);
    }

    #[test]
    fn globaldefs_overrides_definition_scope() {
        let mut state = engine();
        state.set_int_par(INT_PAR_GLOBAL_DEFS, 1).unwrap();
        state.new_save_level(SIMPLE_GROUP).unwrap();
        state.set_source_text("\\def\\foo{x}");
        state.dispatch_x_command().unwrap();
        state.unsave().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Call as u8);
        assert_eq!(state.cs_table.level(foo).unwrap(), LEVEL_ONE);

        let mut negative_state = engine();
        negative_state.set_int_par(INT_PAR_GLOBAL_DEFS, -1).unwrap();
        negative_state.new_save_level(SIMPLE_GROUP).unwrap();
        negative_state.set_source_text("\\gdef\\foo{x}");
        negative_state.dispatch_x_command().unwrap();
        negative_state.unsave().unwrap();

        let foo = cs(&negative_state, "foo");
        assert_eq!(
            negative_state.cs_command(foo),
            CommandCode::UndefinedCs as u8
        );
        assert_eq!(negative_state.cs_table.level(foo).unwrap(), LEVEL_ZERO);
    }

    #[test]
    fn prefixes_accumulate_parity_and_protection() {
        let mut state = engine();
        state
            .define_simple_macro("foo", &[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        state.set_source_text("\\long\\outer\\global\\protected\\edef\\bar{\\foo}");
        state.dispatch_x_command().unwrap();

        let bar = cs(&state, "bar");
        assert_eq!(state.cs_command(bar), CommandCode::LongOuterCall as u8);
        assert_eq!(state.cs_table.level(bar).unwrap(), LEVEL_ONE);
        assert_eq!(
            definition_tokens(&state, bar),
            vec![
                PROTECTED_TOKEN,
                END_MATCH_TOKEN,
                LETTER_TOKEN + i32::from(b'x'),
            ]
        );
    }

    #[test]
    fn prefix_on_nonprefixed_command_is_pushed_back() {
        let mut state = engine();
        state.set_source_text("\\global X");
        state.dispatch_x_command().unwrap();

        state.get_token().unwrap();
        assert_eq!(state.cur_cmd, CommandCode::Letter as u8);
        assert_eq!(state.cur_chr, i32::from(b'X'));
    }

    #[test]
    fn let_copies_control_sequence_and_character_meanings() {
        let mut state = engine();
        state.set_source_text("\\let\\a=\\relax \\let\\b=x");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let a = cs(&state, "a");
        assert_eq!(state.cs_command(a), CommandCode::Relax as u8);
        let b = cs(&state, "b");
        assert_eq!(state.cs_command(b), CommandCode::Letter as u8);
        assert_eq!(state.cs_character(b), i32::from(b'x'));
    }

    #[test]
    fn futurelet_looks_past_the_source_token() {
        let mut state = engine();
        let foo = state
            .define_simple_macro("foo", &[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        state.set_source_text("\\futurelet\\a\\foo X");
        state.dispatch_x_command().unwrap();

        let a = cs(&state, "a");
        assert_eq!(state.cs_command(a), CommandCode::Letter as u8);
        assert_eq!(state.cs_character(a), i32::from(b'X'));

        state.get_token().unwrap();
        assert_eq!(state.cur_cs, foo);
        state.get_token().unwrap();
        assert_eq!(state.cur_cmd, CommandCode::Letter as u8);
        assert_eq!(state.cur_chr, i32::from(b'X'));
    }

    #[test]
    fn integer_dimension_and_glue_assignments_write_eqtb() {
        let mut state = engine();
        state.set_source_text(
            "\\endlinechar=10 \\count3=42 \\dimen2=1.5pt \\skip1=2pt plus 3fil minus 4pt \\muskip1=2mu",
        );
        for _ in 0..5 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.int_par(INT_PAR_END_LINE_CHAR).unwrap(), 10);
        assert_eq!(state.count(3).unwrap(), 42);
        assert_eq!(state.dimen(2).unwrap(), 98_304);

        let skip = state.skip(1).unwrap();
        let spec = state.checked_engine_mem_index(skip).unwrap();
        assert_eq!(state.mem[spec + 1].s1, 2 * 65_536);
        assert_eq!(state.mem[spec + 2].s1, 3 * 65_536);
        assert_eq!(state.mem[spec + 3].s1, 4 * 65_536);
        assert_eq!(state.mem[spec].b16_s1(), FIL);

        let muskip = state.mu_skip(1).unwrap();
        let spec = state.checked_engine_mem_index(muskip).unwrap();
        assert_eq!(state.mem[spec + 1].s1, 2 * 65_536);
    }

    #[test]
    fn global_register_assignment_survives_group_exit() {
        let mut state = engine();
        state.set_source_text("{\\global\\count3=42}");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.count(3).unwrap(), 42);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
    }

    #[test]
    fn advance_multiply_and_divide_dense_word_registers() {
        let mut state = engine();
        state.set_source_text(
            "\\count1=40 \\advance\\count1 by 2 \\multiply\\count1 by 3 \\divide\\count1 by 6\
             \\dimen0=1pt \\advance\\dimen0 by .5pt \\multiply\\dimen0 by 2 \\divide\\dimen0 by 3",
        );
        for _ in 0..8 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.count(1).unwrap(), 21);
        assert_eq!(state.dimen(0).unwrap(), 65_536);
    }

    #[test]
    fn advance_accepts_internal_assignable_quantities() {
        let mut state = engine();
        state.set_source_text(
            "\\count2=5 \\endlinechar=7 \\advance\\endlinechar by 2 \\advance\\count1 by \\count2",
        );
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.int_par(INT_PAR_END_LINE_CHAR).unwrap(), 9);
        assert_eq!(state.count(1).unwrap(), 5);
    }

    #[test]
    fn global_advance_survives_group_exit() {
        let mut state = engine();
        state.set_source_text("\\count0=1 {\\global\\advance\\count0 by 5}");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.count(0).unwrap(), 6);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
    }

    #[test]
    fn advance_glue_preserves_order_precedence() {
        let mut state = engine();
        state.set_source_text(
            "\\skip0=1pt plus 2pt minus 3pt \\advance\\skip0 by 4pt plus 5pt minus 6fil",
        );
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let (width, stretch, shrink, stretch_order, shrink_order) =
            glue_parts(&state, state.skip(0).unwrap());
        assert_eq!(width, 5 * 65_536);
        assert_eq!(stretch, 7 * 65_536);
        assert_eq!(shrink, 6 * 65_536);
        assert_eq!(stretch_order, crate::state::NORMAL);
        assert_eq!(shrink_order, FIL);
    }

    #[test]
    fn multiply_and_divide_glue_scale_all_components() {
        let mut state = engine();
        state.set_source_text(
            "\\skip0=2pt plus 3pt minus 4pt \\multiply\\skip0 by 3 \\divide\\skip0 by 2",
        );
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(
            glue_parts(&state, state.skip(0).unwrap()),
            (
                3 * 65_536,
                294_912,
                6 * 65_536,
                crate::state::NORMAL,
                crate::state::NORMAL,
            )
        );
    }

    #[test]
    fn divide_by_zero_recovers_without_assignment() {
        let mut state = engine();
        state.set_source_text("\\count0=7 \\divide\\count0 by 0");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.count(0).unwrap(), 7);
        assert!(state.arith_error);
    }

    #[test]
    fn multiply_overflow_recovers_without_assignment() {
        let mut state = engine();
        state.set_source_text("\\count0=7 \\multiply\\count0 by 1073741824");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.count(0).unwrap(), 7);
        assert!(state.arith_error);
    }

    #[test]
    fn shorthand_char_and_math_definitions_install_given_codes() {
        let mut state = engine();
        state.set_source_text("\\chardef\\A=65 \\mathchardef\\M=\"2141");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let a = cs(&state, "A");
        assert_eq!(state.cs_command(a), CommandCode::CharGiven as u8);
        assert_eq!(state.cs_character(a), 65);

        let m = cs(&state, "M");
        assert_eq!(state.cs_command(m), CommandCode::MathGiven as u8);
        assert_eq!(state.cs_character(m), 0x2141);
    }

    #[test]
    fn xetex_math_shorthand_definitions_pack_math_codes() {
        let mut state = engine();
        state.set_source_text("\\Umathchardef\\X=1 2 65 \\Umathcharnumdef\\N=\"2200041");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let expected = (1 << 21) + (2 << 24) + 65;
        for name in ["X", "N"] {
            let cs = cs(&state, name);
            assert_eq!(state.cs_command(cs), CommandCode::XetexMathGiven as u8);
            assert_eq!(state.cs_character(cs), expected);
        }
    }

    #[test]
    fn dense_register_shorthands_create_assignable_aliases() {
        let mut state = engine();
        state.set_source_text(
            "\\countdef\\c=7 \\c=12 \\dimendef\\d=2 \\d=1pt\
             \\skipdef\\s=1 \\s=2pt plus 3pt \\muskipdef\\m=1 \\m=4mu \\toksdef\\t=0 \\t={x}",
        );
        for _ in 0..10 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.count(7).unwrap(), 12);
        assert_eq!(state.dimen(2).unwrap(), 65_536);
        assert_eq!(
            glue_parts(&state, state.skip(1).unwrap()),
            (
                2 * 65_536,
                3 * 65_536,
                0,
                crate::state::NORMAL,
                crate::state::NORMAL
            )
        );
        assert_eq!(glue_parts(&state, state.mu_skip(1).unwrap()).0, 4 * 65_536);
        assert_eq!(
            state
                .token_list_to_vec(
                    state
                        .stored_token_list_start(state.toks(0).unwrap())
                        .unwrap()
                )
                .unwrap(),
            vec![LETTER_TOKEN + i32::from(b'x')]
        );
    }

    #[test]
    fn global_shorthand_definition_survives_group_exit() {
        let mut state = engine();
        state.set_source_text("{\\global\\countdef\\c=3}\\c=9");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        let c = cs(&state, "c");
        assert_eq!(state.cs_command(c), CommandCode::AssignInt as u8);
        assert_eq!(state.cs_character(c), (COUNT_BASE + 3) as i32);
        assert_eq!(state.count(3).unwrap(), 9);
    }

    #[test]
    fn toks_assignments_store_reference_counted_lists() {
        let mut state = engine();
        state.set_source_text("\\toks0={x}\\toks1=\\toks0 \\everyeof={z}");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        let toks0 = state.toks(0).unwrap();
        assert_eq!(
            state
                .token_list_to_vec(state.stored_token_list_start(toks0).unwrap())
                .unwrap(),
            vec![LETTER_TOKEN + i32::from(b'x')]
        );
        assert_eq!(state.toks(1).unwrap(), toks0);
        assert_eq!(
            state.mem[state.checked_engine_mem_index(toks0).unwrap()].s0,
            1
        );

        let every_eof = state.eqtb[LOCAL_BASE + LOCAL_EVERY_EOF].s1;
        assert_eq!(
            state
                .token_list_to_vec(state.stored_token_list_start(every_eof).unwrap())
                .unwrap(),
            vec![LETTER_TOKEN + i32::from(b'z')]
        );

        state.set_source_text("\\the\\toks1");
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + i32::from(b'x')
        );
    }

    #[test]
    fn empty_toks_assignment_defines_undefined_null() {
        let mut state = engine();
        state.set_source_text("\\toks0={}");
        state.dispatch_x_command().unwrap();

        let entry = state.eqtb[TOKS_BASE];
        assert_eq!(entry.b16_s1(), CommandCode::UndefinedCs as u16);
        assert_eq!(entry.s1, TEX_NULL);
    }

    #[test]
    fn afterassignment_reinserts_token_after_assignment() {
        let mut state = engine();
        let x = LETTER_TOKEN + i32::from(b'x');
        state.define_simple_macro("foo", &[x]).unwrap();
        state.set_source_text("\\afterassignment\\foo\\endlinechar=1");

        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        assert_eq!(state.after_token, TEX_NULL);
        assert_eq!(state.next_expanded_token().unwrap(), x);
    }

    #[test]
    fn aftergroup_reinserts_token_after_unsave() {
        let mut state = engine();
        let foo = state
            .define_simple_macro("foo", &[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        state.set_source_text("{\\aftergroup\\foo}");

        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        state.get_token().unwrap();
        assert_eq!(state.cur_tok, CS_TOKEN_FLAG + foo);
    }

    #[test]
    fn openin_and_read_define_tokenized_control_sequence() {
        let mut state = engine();
        state.register_input_source("read-data", "{a} b");
        state.set_source_text("\\openin0=read-data \\read0 to\\foo");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Call as u8);
        assert_eq!(
            definition_tokens(&state, foo),
            vec![
                END_MATCH_TOKEN,
                LEFT_BRACE_TOKEN + i32::from(b'{'),
                LETTER_TOKEN + i32::from(b'a'),
                RIGHT_BRACE_TOKEN + i32::from(b'}'),
                SPACE_TOKEN,
                LETTER_TOKEN + i32::from(b'b'),
                SPACE_TOKEN,
            ]
        );
        assert!(!state.read_stream_eof(0));
    }

    #[test]
    fn readline_preserves_raw_other_character_tokens() {
        let mut state = engine();
        state.register_input_source("raw-read", "{a} #");
        state.set_source_text("\\openin1=raw-read \\readline1 to\\raw");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let raw = cs(&state, "raw");
        assert_eq!(state.cs_command(raw), CommandCode::Call as u8);
        assert_eq!(
            definition_tokens(&state, raw),
            vec![
                END_MATCH_TOKEN,
                OTHER_TOKEN + i32::from(b'{'),
                OTHER_TOKEN + i32::from(b'a'),
                OTHER_TOKEN + i32::from(b'}'),
                SPACE_TOKEN,
                OTHER_TOKEN + i32::from(b'#'),
                OTHER_TOKEN + i32::from(b'\r'),
            ]
        );
    }

    #[test]
    fn read_consumes_stream_lines_until_eof() {
        let mut state = engine();
        state.register_input_source("two-lines", "a\nb");
        state.set_source_text("\\openin0=two-lines \\read0 to\\a \\read0 to\\b \\read0 to\\empty");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        let a = cs(&state, "a");
        let b = cs(&state, "b");
        let empty = cs(&state, "empty");
        assert_eq!(
            definition_tokens(&state, a),
            vec![END_MATCH_TOKEN, LETTER_TOKEN + i32::from(b'a'), SPACE_TOKEN]
        );
        assert_eq!(
            definition_tokens(&state, b),
            vec![END_MATCH_TOKEN, LETTER_TOKEN + i32::from(b'b'), SPACE_TOKEN]
        );
        assert_eq!(definition_tokens(&state, empty), vec![END_MATCH_TOKEN]);
        assert!(state.read_stream_eof(0));
    }

    #[test]
    fn global_read_definition_survives_group_exit() {
        let mut state = engine();
        state.register_input_source("global-read", "x");
        state.set_source_text("\\openin0=global-read {\\global\\read0 to\\foo}");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Call as u8);
        assert_eq!(state.cs_table.level(foo).unwrap(), LEVEL_ONE);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
    }

    #[test]
    fn closein_and_missing_openin_leave_stream_at_eof() {
        let mut state = engine();
        state.register_input_source("close-me", "x");
        state.set_source_text("\\openin0=close-me \\closein0 \\openin1=missing");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert!(state.read_stream_eof(0));
        assert!(state.read_stream_eof(1));
    }

    #[test]
    fn legacy_code_tables_assign_values() {
        let mut state = engine();
        assert_eq!(state.lc_code(u32::from(b'A')), u32::from(b'a') as i32);
        assert_eq!(state.uc_code(u32::from(b'a')), u32::from(b'A') as i32);
        assert_eq!(state.sf_code(u32::from(b'A')), 999);
        assert_eq!(state.math_code(u32::from(b'0')), (7 << 21) + b'0' as i32);
        assert_eq!(state.del_code(u32::from(b'.')), 0);

        state.set_source_text(
            "\\catcode`A=11 \\lccode`B=98 \\uccode`b=66 \\sfcode`C=1001\
             \\mathcode`x=\"715D \\delcode`(=\"123456 \\mathcode`z=\"8000",
        );
        for _ in 0..7 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.cat_code(u32::from(b'A')), 11);
        assert_eq!(state.lc_code(u32::from(b'B')), 98);
        assert_eq!(state.uc_code(u32::from(b'b')), 66);
        assert_eq!(state.sf_code(u32::from(b'C')), 1001);
        assert_eq!(
            state.math_code(u32::from(b'x')),
            (7 << 21) + (1 << 24) + 0x5D
        );
        assert_eq!(state.math_code(u32::from(b'z')), ACTIVE_MATH_CHAR);
        assert_eq!(state.del_code(u32::from(b'(')), 0x123456);
    }

    #[test]
    fn code_tables_work_as_internal_integer_quantities() {
        let mut state = engine();
        state.set_source_text(
            "\\catcode`A=12 \\count0=\\catcode`A \\count1=\\sfcode`A\
             \\count2=\\mathcode`x \\count3=\\delcode`.",
        );
        for _ in 0..5 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.count(0).unwrap(), 12);
        assert_eq!(state.count(1).unwrap(), 999);
        assert_eq!(state.count(2).unwrap(), 0x7178);
        assert_eq!(state.count(3).unwrap(), 0);
    }

    #[test]
    fn xetex_code_tables_work_as_internal_integer_quantities() {
        let mut state = engine();
        state.set_source_text(
            "\\XeTeXcharclass`A=3 \\count4=\\XeTeXcharclass`A\
             \\count5=\\Umathcodenum`B \\count6=\\Umathcode`B",
        );
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.count(4).unwrap(), 3);
        assert_eq!(
            state.count(5).unwrap(),
            (7 << 21) + (1 << 24) + i32::from(b'B')
        );
        assert_eq!(state.count(6).unwrap(), 0);
    }

    #[test]
    fn invalid_legacy_code_values_are_replaced_by_zero() {
        let mut state = engine();
        state.set_source_text("\\catcode`A=16 \\delcode`B=16777216 \\delcode`C=-1");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.cat_code(u32::from(b'A')), 0);
        assert_eq!(state.del_code(u32::from(b'B')), 0);
        assert_eq!(state.del_code(u32::from(b'C')), -1);
    }

    #[test]
    fn sfcode_and_xetex_char_class_share_the_packed_table() {
        let mut state = engine();
        state.set_source_text("\\XeTeXcharclass`A=3 \\sfcode`A=1002 \\sfcode`A=1003");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.sf_code(u32::from(b'A')), 3 * 65_536 + 1003);
    }

    #[test]
    fn xetex_math_code_forms_pack_the_same_value() {
        let mut state = engine();
        state.set_source_text("\\Umathcodenum`A=\"2200041 \\Umathcode`B=1 2 66");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.math_code(u32::from(b'A')), (1 << 21) + (2 << 24) + 65);
        assert_eq!(state.math_code(u32::from(b'B')), (1 << 21) + (2 << 24) + 66);
    }

    #[test]
    fn xetex_delimiter_code_forms_store_numeric_and_packed_values() {
        let mut state = engine();
        state.set_source_text("\\Udelcodenum`A=12345 \\Udelcode`B=2 40");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.del_code(u32::from(b'A')), 12_345);
        assert_eq!(
            state.del_code(u32::from(b'B')),
            0x4000_0000 + 2 * 0x20_0000 + 40
        );
    }

    #[test]
    fn character_code_definitions_restore_on_group_exit() {
        let mut state = engine();
        state.set_source_text("{\\catcode`0=11 \\global\\lccode`B=98}");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.cat_code(u32::from(b'0')), 12);
        assert_eq!(state.lc_code(u32::from(b'B')), 98);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);

        state.set_source_text("{\\catcode`0=11 \\global\\catcode`0=13}");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }
        assert_eq!(state.cat_code(u32::from(b'0')), 13);
    }

    #[test]
    fn xetex_character_class_rejects_the_limit() {
        let mut state = engine();
        state.set_source_text("\\XeTeXcharclass`A=4096");
        assert!(matches!(
            state.dispatch_x_command(),
            Err(TeXMouthError::InputOutOfBounds(4096))
        ));
    }

    #[test]
    fn aux_quantities_assign_and_read_in_their_modes() {
        let mut state = engine();
        state.set_source_text("\\prevdepth=1.5pt \\dimen0=\\prevdepth");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.aux.s1, 98_304);
        assert_eq!(state.dimen(0).unwrap(), 98_304);

        state.mode = TeXMode::Hmode as i16;
        state.set_source_text("\\spacefactor=1200 \\count0=\\spacefactor");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.aux.s0, 1_200);
        assert_eq!(state.count(0).unwrap(), 1_200);
    }

    #[test]
    fn improper_aux_and_invalid_spacefactor_recover_without_assignment() {
        let mut state = engine();
        state.set_source_text("\\spacefactor=100");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.aux.s0, 0);

        let mut state = engine();
        state.mode = TeXMode::Hmode as i16;
        state.aux.s0 = 777;
        state.set_source_text("\\spacefactor=0");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.aux.s0, 777);
    }

    #[test]
    fn prevgraf_assigns_and_reads_the_innermost_vertical_list() {
        let mut state = engine();
        state.prev_graf = 3;
        state.push_nest().unwrap();
        state.mode = TeXMode::Hmode as i16;
        state.set_source_text("\\prevgraf=5 \\count0=\\prevgraf");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.nest[0].prev_graf, 5);
        assert_eq!(state.count(0).unwrap(), 5);

        state.pop_nest().unwrap();
        assert_eq!(state.prev_graf, 5);
    }

    #[test]
    fn page_dimensions_and_integers_assign_and_read() {
        let mut state = engine();
        state.set_source_text(
            "\\deadcycles=3 \\insertpenalties=7 \\interactionmode=1\
             \\pagegoal=10pt \\pagetotal=2pt",
        );
        for _ in 0..5 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.dead_cycles, 3);
        assert_eq!(state.insert_penalties, 7);
        assert_eq!(state.interaction, 1);
        assert_eq!(state.selector, SELECTOR_TERM_ONLY);
        assert_eq!(state.page_so_far[0], 10 * 65_536);
        assert_eq!(state.page_so_far[1], 2 * 65_536);

        state.page_contents = PAGE_BOX_THERE;
        state.set_source_text("\\dimen0=\\pagegoal \\count0=\\interactionmode");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        assert_eq!(state.dimen(0).unwrap(), 10 * 65_536);
        assert_eq!(state.count(0).unwrap(), 1);
    }

    #[test]
    fn interaction_primitives_and_invalid_interactionmode_update_selector() {
        let mut state = engine();
        state.set_source_text("\\batchmode \\scrollmode \\interactionmode=9");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.interaction, 2);
        assert_eq!(state.selector, SELECTOR_TERM_ONLY);

        state.log_opened = true;
        state.set_source_text("\\batchmode");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.interaction, 0);
        assert_eq!(state.selector, SELECTOR_LOG_ONLY);
    }

    #[test]
    fn box_dimensions_assign_and_read_dense_registers() {
        let mut state = engine();
        let b = state.new_null_box().unwrap();
        state.set_box_reg(1, b).unwrap();
        state.set_source_text("\\wd1=2pt \\dp1=3pt \\ht1=4pt \\dimen0=\\wd1");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        let b = b as usize;
        assert_eq!(state.mem[b + 1].s1, 2 * 65_536);
        assert_eq!(state.mem[b + 2].s1, 3 * 65_536);
        assert_eq!(state.mem[b + 3].s1, 4 * 65_536);
        assert_eq!(state.dimen(0).unwrap(), 2 * 65_536);
    }

    #[test]
    fn parshape_stores_dimension_pairs_and_reads_its_length() {
        let mut state = engine();
        state.set_source_text("\\parshape=2 1pt 2pt 3pt 4pt \\count0=\\parshape");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let p = state.eqtb[LOCAL_BASE + LOCAL_PAR_SHAPE].s1 as usize;
        assert_eq!(state.mem[p].s0, 2);
        assert_eq!(state.mem[p + 1].s1, 65_536);
        assert_eq!(state.mem[p + 2].s1, 2 * 65_536);
        assert_eq!(state.mem[p + 3].s1, 3 * 65_536);
        assert_eq!(state.mem[p + 4].s1, 4 * 65_536);
        assert_eq!(state.count(0).unwrap(), 2);
    }

    #[test]
    fn etex_penalty_arrays_store_values_and_support_indexed_reads() {
        let mut state = engine();
        state.set_source_text(
            "\\interlinepenalties=3 10 20 30 \\count0=\\interlinepenalties 2\
             \\clubpenalties=2 5 6",
        );
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        let inter = state.eqtb[crate::state::ETEX_PEN_BASE].s1 as usize;
        assert_eq!(state.mem[inter].s0, 2);
        assert_eq!(state.mem[inter + 1].s1, 3);
        assert_eq!(state.mem[inter + 2].s1, 10);
        assert_eq!(state.mem[inter + 3].s1, 20);
        assert_eq!(state.mem[inter + 4].s1, 30);
        assert_eq!(state.count(0).unwrap(), 20);

        let club = state.eqtb[crate::state::ETEX_PEN_BASE + 1].s1 as usize;
        assert_eq!(state.mem[club].s0, 2);
        assert_eq!(state.mem[club + 1].s1, 2);
        assert_eq!(state.mem[club + 2].s1, 5);
        assert_eq!(state.mem[club + 3].s1, 6);
        assert_eq!(state.mem[club + 4].s1, 0);
    }

    #[test]
    fn local_shape_assignment_restores_the_previous_shape() {
        let mut state = engine();
        state.set_source_text("\\parshape=1 1pt 2pt {\\parshape=2 3pt 4pt 5pt 6pt}");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        let restored = state.eqtb[LOCAL_BASE + LOCAL_PAR_SHAPE].s1 as usize;
        assert_eq!(state.mem[restored].s0, 1);
        assert_eq!(state.mem[restored + 1].s1, 65_536);
        assert_eq!(state.mem[restored + 2].s1, 2 * 65_536);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
    }

    #[test]
    fn sparse_count_registers_assign_restore_and_read() {
        let mut state = engine();
        state.set_source_text("\\count300=5\\begingroup\\count300=7\\endgroup");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        let element = state.find_sa_element(0, 300, false).unwrap();
        assert_ne!(element, TEX_NULL);
        assert_eq!(state.sparse_word_value(element).unwrap(), 5);

        state.set_source_text("\\global\\count300=9\\begingroup\\count300=1\\endgroup");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }
        let element = state.find_sa_element(0, 300, false).unwrap();
        assert_eq!(state.sparse_word_value(element).unwrap(), 9);
    }

    #[test]
    fn sparse_register_aliases_support_assignment_and_arithmetic() {
        let mut state = engine();
        state.set_source_text("\\countdef\\foo=300\\foo=42\\advance\\foo by 1");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::Register as u8);
        let element = state.cs_character(foo);
        assert_eq!(state.sparse_word_value(element).unwrap(), 43);

        state.set_source_text("\\foo");
        assert_eq!(state.scan_int().unwrap(), 43);
    }

    #[test]
    fn sparse_glue_registers_assign_and_advance() {
        let mut state = engine();
        state.set_source_text("\\skip300=1pt plus 2pt\\advance\\skip300 by 3pt");
        for _ in 0..2 {
            state.dispatch_x_command().unwrap();
        }

        let element = state.find_sa_element(2, 300, false).unwrap();
        assert_eq!(
            glue_parts(&state, state.sparse_ptr_value(element).unwrap()),
            (4 * 65_536, 2 * 65_536, 0, 0, 0)
        );
    }

    #[test]
    fn sparse_toks_copy_references_alias_and_restore() {
        let mut state = engine();
        state.set_source_text("\\toks300={a b}\\toks301=\\toks300");
        for _ in 0..2 {
            state.dispatch_x_command().unwrap();
        }

        let first = state.find_sa_element(5, 300, false).unwrap();
        let second = state.find_sa_element(5, 301, false).unwrap();
        let first_list = state.sparse_ptr_value(first).unwrap();
        assert_eq!(
            state
                .token_list_to_vec(state.stored_token_list_start(first_list).unwrap())
                .unwrap(),
            vec![
                11 * MAX_CHAR_VAL + i32::from(b'a'),
                SPACE_TOKEN,
                11 * MAX_CHAR_VAL + i32::from(b'b')
            ]
        );
        assert_eq!(state.sparse_ptr_value(second).unwrap(), first_list);
        assert_eq!(state.mem[first_list as usize].s0, 1);

        state.set_source_text("\\toksdef\\baz=300\\baz={x}");
        for _ in 0..2 {
            state.dispatch_x_command().unwrap();
        }
        assert_ne!(state.sparse_ptr_value(first).unwrap(), first_list);

        state.set_source_text("\\begingroup\\toks301={y}\\endgroup");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }
        assert_eq!(state.sparse_ptr_value(second).unwrap(), first_list);
    }

    #[test]
    fn interchartoks_assign_and_copy_through_sparse_elements() {
        let mut state = engine();
        state.set_source_text("\\XeTeXinterchartoks 1 2={i}\\toks300=\\XeTeXinterchartoks 1 2");
        for _ in 0..2 {
            state.dispatch_x_command().unwrap();
        }

        let key = CHAR_CLASS_LIMIT + 2;
        let inter = state.find_sa_element(6, key, false).unwrap();
        let toks = state.find_sa_element(5, 300, false).unwrap();
        assert_eq!(
            state.sparse_ptr_value(toks).unwrap(),
            state.sparse_ptr_value(inter).unwrap()
        );
    }

    #[test]
    fn marks_command_appends_a_classed_mark_node() {
        let mut state = engine();
        state.set_source_text("\\marks3{x}");
        state.dispatch_x_command().unwrap();

        let node = state.tail as usize;
        assert_eq!(state.mem[node].b16_s1(), MARK_NODE);
        assert_eq!(state.mem[node + 1].s0, 3);
        assert_eq!(
            state
                .token_list_to_vec(
                    state
                        .stored_token_list_start(state.mem[node + 1].s1)
                        .unwrap(),
                )
                .unwrap(),
            vec![LETTER_TOKEN + i32::from(b'x')]
        );
    }

    #[test]
    fn sparse_mark_classes_expand_top_first_and_bottom_marks() {
        let mut state = engine();
        let element = state.find_sa_element(7, 3, true).unwrap();
        let first_list = state
            .store_token_list(&[LETTER_TOKEN + i32::from(b'f')])
            .unwrap();
        let first = state.token_list_reference(first_list).unwrap();
        let bot_list = state
            .store_token_list(&[LETTER_TOKEN + i32::from(b'b')])
            .unwrap();
        let bot = state.token_list_reference(bot_list).unwrap();
        let element = element as usize;
        state.mem[element + 1].s1 = first;
        state.mem[element + 2].s0 = bot;

        state.set_source_text("\\firstmarks3\\botmarks3");
        state.get_x_token().unwrap();
        assert_eq!(state.cur_tok, LETTER_TOKEN + i32::from(b'f'));
        state.get_x_token().unwrap();
        assert_eq!(state.cur_tok, LETTER_TOKEN + i32::from(b'b'));
    }

    #[test]
    fn destroy_marks_deletes_references_and_prunes_the_sparse_trie() {
        let mut state = engine();
        let element = state.find_sa_element(7, 3, true).unwrap();
        let first_list = state
            .store_token_list(&[LETTER_TOKEN + i32::from(b'f')])
            .unwrap();
        let first = state.token_list_reference(first_list).unwrap();
        let bot_list = state
            .store_token_list(&[LETTER_TOKEN + i32::from(b'b')])
            .unwrap();
        let bot = state.token_list_reference(bot_list).unwrap();
        let element = element as usize;
        state.mem[element + 1].s1 = first;
        state.mem[element + 2].s0 = bot;

        let root = state.sa_root[7];
        assert!(state.do_marks(3, 0, root).unwrap());
        state.sa_root[7] = TEX_NULL;
        assert_eq!(state.find_sa_element(7, 3, false).unwrap(), TEX_NULL);
    }

    #[test]
    fn message_stores_the_expanded_general_text() {
        let mut state = engine();
        state.set_source_text("\\message{Hello}");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.transcript, vec!["Hello"]);
        assert_eq!(state.error_count, 0);
    }

    #[test]
    fn errmessage_uses_errhelp_and_remembers_long_help() {
        let mut state = engine();
        state.interaction = BATCH_MODE;
        state.set_source_text("\\errmessage{first}\\errmessage{second}");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.transcript[0], "! first");
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("\\errmessage")));
        assert_eq!(
            state.transcript.last().unwrap(),
            "(That was another \\errmessage.)"
        );
        assert_eq!(state.error_count, 2);
        assert!(state.long_help_seen);
    }

    #[test]
    fn errmessage_reads_the_errhelp_token_list() {
        let mut state = engine();
        state.set_source_text("\\errhelp={use a clue}\\errmessage{bad}");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.transcript[0], "! bad");
        assert_eq!(state.transcript[1], "use a clue");
    }

    #[test]
    fn lowercase_maps_character_tokens_and_preserves_control_sequences() {
        let mut state = engine();
        state.set_source_text("\\lowercase{Ab\\x}");
        state.dispatch_x_command().unwrap();

        state.get_token().unwrap();
        assert_eq!(state.cur_tok, LETTER_TOKEN + i32::from(b'a'));
        state.get_token().unwrap();
        assert_eq!(state.cur_tok, LETTER_TOKEN + i32::from(b'b'));
        state.get_token().unwrap();
        assert!(state.cur_tok >= CS_TOKEN_FLAG);
    }

    #[test]
    fn uppercase_maps_character_tokens() {
        let mut state = engine();
        state.set_source_text("\\uppercase{ab}");
        state.dispatch_x_command().unwrap();

        state.get_token().unwrap();
        assert_eq!(state.cur_tok, LETTER_TOKEN + i32::from(b'A'));
        state.get_token().unwrap();
        assert_eq!(state.cur_tok, LETTER_TOKEN + i32::from(b'B'));
    }

    #[test]
    fn scan_spec_preserves_box_context_and_raw_save_slots() {
        let mut state = engine();
        state
            .save_stack
            .push(SaveStackEntry::RawValue(BOX_FLAG + 3));
        state.set_source_text("to 2pt{x}");
        state.scan_spec(HBOX_GROUP, true).unwrap();

        assert_eq!(state.cur_group, HBOX_GROUP);
        assert_eq!(state.save_ptr, 5);
        assert_eq!(state.save_stack[0], SaveStackEntry::RawValue(BOX_FLAG + 3));
        assert_eq!(state.save_stack[1], SaveStackEntry::RawValue(EXACTLY));
        assert_eq!(state.save_stack[2], SaveStackEntry::RawValue(2 * 65_536));

        state.unsave().unwrap();
        assert_eq!(state.save_ptr, 3);
        state.save_ptr -= 3;
        assert_eq!(state.save_ptr, 0);
    }

    #[test]
    fn scan_spec_defaults_to_zero_additional_spread() {
        let mut state = engine();
        state.set_source_text("{x}");
        state.scan_spec(HBOX_GROUP, false).unwrap();

        assert_eq!(state.save_ptr, 4);
        assert_eq!(state.save_stack[0], SaveStackEntry::RawValue(ADDITIONAL));
        assert_eq!(state.save_stack[1], SaveStackEntry::RawValue(0));
        state.unsave().unwrap();
        assert_eq!(state.save_ptr, 2);
    }

    #[test]
    fn setbox_empty_hbox_closes_packaging_group() {
        let mut state = engine();
        state.set_source_text("\\setbox0=\\hbox{}");

        state.dispatch_x_command().unwrap();
        assert_eq!(state.cur_group, HBOX_GROUP);
        assert_eq!(state.mode, -(TeXMode::Hmode as i16));

        state.dispatch_x_command().unwrap();
        let boxed = state.box_reg(0).unwrap();
        assert_ne!(boxed, TEX_NULL);
        assert_eq!(state.node_type(boxed).unwrap(), HLIST_NODE);
        assert_eq!(state.mem[boxed as usize + 5].s1, TEX_NULL);
        assert_eq!(state.mem[boxed as usize + 1].s1, 0);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
        assert_eq!(state.mode, TeXMode::Vmode as i16);
        assert_eq!(state.save_ptr, 0);
    }

    #[test]
    fn box_code_transfers_register_ownership() {
        let mut state = engine();
        let boxed = state.new_null_box().unwrap();
        state.mem[boxed as usize + 1].s1 = 7;
        state.set_box_reg(3, boxed).unwrap();
        state.mode = -(TeXMode::Vmode as i16);
        state.aux.s1 = IGNORE_DEPTH;
        state.cur_cmd = CommandCode::MakeBox as u8;
        state.cur_chr = BOX_CODE;
        state.set_source_text("3");

        state.begin_box(0).unwrap();

        assert_eq!(state.box_reg(3).unwrap(), TEX_NULL);
        assert_eq!(state.cur_box, boxed);
        assert_eq!(state.tail, boxed);
        assert_eq!(state.mem[boxed as usize + 4].s1, 0);
    }

    #[test]
    fn leaders_accept_rule_and_attach_following_glue() {
        let mut state = engine();
        state.mode = TeXMode::Hmode as i16;
        state.set_source_text("\\leaders\\hrule width 5pt\\hskip 3pt");

        state.dispatch_x_command().unwrap();

        let glue = state.tail;
        assert_eq!(state.node_type(glue).unwrap(), GLUE_NODE);
        assert_eq!(state.node_subtype(glue).unwrap(), A_LEADERS as u16);
        let leader = state.mem[glue as usize + 1].s1;
        assert_eq!(state.node_type(leader).unwrap(), RULE_NODE);
        assert_eq!(state.mem[leader as usize + 1].s1, 5 * 65_536);
        let spec = state.mem[glue as usize + 1].s0;
        assert_eq!(state.mem[spec as usize + 1].s1, 3 * 65_536);
    }

    #[test]
    fn vsplit_moves_the_split_remainder_back_to_the_register() {
        let mut state = engine();
        let first = state.new_null_box().unwrap();
        state.mem[first as usize + 1].s1 = 10;
        state.mem[first as usize + 3].s1 = 5 * 65_536;
        let penalty = state.new_penalty(0).unwrap();
        let second = state.new_null_box().unwrap();
        state.mem[second as usize + 1].s1 = 12;
        state.mem[second as usize + 3].s1 = 7 * 65_536;
        state.mem[first as usize].s1 = penalty;
        state.mem[penalty as usize].s1 = second;
        let source = state.vpackage(first, 0, ADDITIONAL, MAX_HALFWORD).unwrap();
        state.set_box_reg(0, source).unwrap();
        state.mode = -(TeXMode::Vmode as i16);
        state.aux.s1 = IGNORE_DEPTH;
        state.cur_cmd = CommandCode::MakeBox as u8;
        state.cur_chr = VSPLIT_CODE;
        state.set_source_text("0to6pt");

        state.begin_box(0).unwrap();

        assert_eq!(state.node_type(state.cur_box).unwrap(), VLIST_NODE);
        assert_eq!(state.mem[state.cur_box as usize + 3].s1, 6 * 65_536);
        let remainder = state.box_reg(0).unwrap();
        assert_eq!(state.node_type(remainder).unwrap(), VLIST_NODE);
        let first_remainder = state.mem[remainder as usize + 5].s1;
        assert_eq!(state.node_type(first_remainder).unwrap(), GLUE_NODE);
        assert_eq!(state.tail, state.cur_box);
    }

    #[test]
    fn kern_penalty_and_unpenalty_update_the_active_list() {
        let mut state = engine();
        state.mode = TeXMode::Hmode as i16;
        state.aux.s0 = 1000;
        state.set_source_text("\\kern2pt\\penalty5\\unpenalty");

        state.dispatch_x_command().unwrap();
        let kern = state.tail;
        assert_eq!(state.node_type(kern).unwrap(), KERN_NODE);
        assert_eq!(state.mem[kern as usize + 1].s1, 2 * 65_536);

        state.dispatch_x_command().unwrap();
        let penalty = state.tail;
        assert_eq!(state.node_type(penalty).unwrap(), PENALTY_NODE);
        assert_eq!(state.mem[penalty as usize + 1].s1, 5);

        state.dispatch_x_command().unwrap();
        assert_eq!(state.tail, kern);
        assert_eq!(state.link(kern).unwrap(), TEX_NULL);
    }

    #[test]
    fn noindent_starts_paragraph_with_parskip_and_hyphen_minima() {
        let mut state = engine();
        state.set_int_par(INT_PAR_SAVING_VDISCARDS, 1).unwrap();
        state.set_source_text("\\noindent");

        state.dispatch_x_command().unwrap();

        assert_eq!(state.mode, TeXMode::Hmode as i16);
        assert_eq!(state.prev_graf, (2 * 64 + 3) * 65_536);
        assert_eq!(state.head, state.tail);
        let outer_tail = state.nest[state.nest_ptr - 1].tail;
        assert_eq!(outer_tail, state.contrib_head());
        assert_eq!(state.link(state.contrib_head()).unwrap(), TEX_NULL);
        let discarded = state.disc_ptr[LAST_BOX_CODE as usize];
        assert_eq!(state.node_type(discarded).unwrap(), GLUE_NODE);
        assert_eq!(state.node_subtype(discarded).unwrap(), 3);
    }

    #[test]
    fn insert_packages_empty_vertical_material_as_an_ins_node() {
        let mut state = engine();
        state.set_source_text("\\insert3{}");

        state.dispatch_x_command().unwrap();
        assert_eq!(state.cur_group, INSERT_GROUP);
        assert_eq!(state.mode, -1);

        state.dispatch_x_command().unwrap();
        let ins = state.link(state.page_head()).unwrap();
        assert_eq!(state.node_type(ins).unwrap(), INS_NODE);
        assert_eq!(state.node_subtype(ins).unwrap(), 3);
        assert_eq!(state.cur_group, BOTTOM_LEVEL);
        assert_eq!(state.save_ptr, 0);
    }

    #[test]
    fn vadjust_pre_packages_empty_material_as_a_pre_adjust_node() {
        let mut state = engine();
        state.push_nest().unwrap();
        state.mode = TeXMode::Hmode as i16;
        state.aux.s0 = 1000;
        state.set_source_text("\\vadjust pre{}");

        state.dispatch_x_command().unwrap();
        assert_eq!(state.cur_group, INSERT_GROUP);
        assert_eq!(state.mode, -1);

        state.dispatch_x_command().unwrap();
        let adjust = state.tail;
        assert_eq!(state.node_type(adjust).unwrap(), ADJUST_NODE);
        assert_eq!(state.node_subtype(adjust).unwrap(), 1);
        assert_eq!(state.mode, TeXMode::Hmode as i16);
        assert_eq!(state.save_ptr, 0);
    }

    #[test]
    fn build_page_inserts_topskip_before_the_first_box() {
        let mut state = engine();
        state.set_dimen_par(DIMEN_PAR_VSIZE, 100 * 65_536).unwrap();
        let boxed = state.new_null_box().unwrap();
        state.mem[boxed as usize + 1].s1 = 10 * 65_536;
        state.mem[boxed as usize + 2].s1 = 2 * 65_536;
        state.mem[boxed as usize + 3].s1 = 8 * 65_536;
        let contrib = state.contrib_head();
        state.set_link(contrib, boxed).unwrap();

        state.build_page().unwrap();

        let page_head = state.page_head();
        let top_skip = state.link(page_head).unwrap();
        assert_eq!(state.page_contents, PAGE_BOX_THERE);
        assert_eq!(state.node_type(top_skip).unwrap(), GLUE_NODE);
        assert_eq!(
            state.node_subtype(top_skip).unwrap(),
            GLUE_PAR_TOP_SKIP as u16 + 1
        );
        assert_eq!(state.link(top_skip).unwrap(), boxed);
        assert_eq!(state.link(boxed).unwrap(), TEX_NULL);
        assert_eq!(state.link(contrib).unwrap(), TEX_NULL);
    }

    #[test]
    fn build_page_saves_discarded_vertical_glue() {
        let mut state = engine();
        state.set_int_par(INT_PAR_SAVING_VDISCARDS, 1).unwrap();
        let glue = state.new_param_glue(GLUE_PAR_LEFT_SKIP).unwrap();
        let contrib = state.contrib_head();
        state.set_link(contrib, glue).unwrap();

        state.build_page().unwrap();

        assert_eq!(state.page_contents, PAGE_EMPTY);
        assert_eq!(state.disc_ptr[LAST_BOX_CODE as usize], glue);
        assert_eq!(state.disc_ptr[COPY_CODE as usize], glue);
        assert_eq!(state.link(contrib).unwrap(), TEX_NULL);
        assert_eq!(state.link(glue).unwrap(), TEX_NULL);
    }

    #[test]
    fn build_page_fires_default_output_and_records_the_shipped_page() {
        let mut state = engine();
        state.set_dimen_par(DIMEN_PAR_VSIZE, 100 * 65_536).unwrap();
        let boxed = state.new_null_box().unwrap();
        state.mem[boxed as usize + 1].s1 = 10 * 65_536;
        state.mem[boxed as usize + 2].s1 = 2 * 65_536;
        state.mem[boxed as usize + 3].s1 = 8 * 65_536;
        let penalty = state.new_penalty(EJECT_PENALTY).unwrap();
        state.mem[boxed as usize].s1 = penalty;
        let contrib = state.contrib_head();
        state.set_link(contrib, boxed).unwrap();

        state.build_page().unwrap();

        assert_eq!(state.shipped_pages.len(), 1);
        assert_eq!(state.box_reg(255).unwrap(), TEX_NULL);
        assert_eq!(state.page_contents, PAGE_EMPTY);
        assert_eq!(state.link(state.page_head()).unwrap(), TEX_NULL);
        assert_eq!(state.link(contrib).unwrap(), penalty);

        state.build_page().unwrap();
        assert_eq!(state.link(contrib).unwrap(), TEX_NULL);
    }

    #[test]
    fn show_reports_primitive_character_and_macro_meanings() {
        let mut state = engine();
        state.set_source_text("\\show\\relax\\show A\\def\\foo#1{#1!}\\show\\foo");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.transcript[0], "> \\relax=\\relax");
        assert_eq!(state.transcript[6], "> the letter A");
        assert_eq!(state.transcript[12], "> \\foo=macro:#1->#1!");
        assert!(!state.transcript.iter().any(|line| line == "! OK"));
    }

    #[test]
    fn showbox_reports_a_void_register_and_the_common_ok_tail() {
        let mut state = engine();
        state.set_source_text("\\showbox0");
        state.dispatch_x_command().unwrap();

        assert_eq!(state.transcript[0], "> \\box0=");
        assert_eq!(state.transcript[1], "void");
        assert_eq!(state.transcript[2], "! OK");
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("\\tracingonline=1")));
    }

    #[test]
    fn showbox_displays_a_packed_box() {
        let mut state = engine();
        state.set_source_text("\\setbox0=\\hbox{}\\showbox0");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.transcript[0], "> \\box0=");
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("\\hbox(0.0pt+0.0pt)x0.0pt")));
        assert!(state.transcript.iter().any(|line| line == "! OK"));
    }

    #[test]
    fn showthe_and_showtokens_display_their_token_lists_without_ok() {
        let mut state = engine();
        state.set_source_text("\\showthe\\count0\\showtokens{ab}");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.transcript[0], "> 0");
        assert!(state.transcript.iter().any(|line| line == "> ab"));
        assert!(!state.transcript.iter().any(|line| line == "! OK"));
    }

    #[test]
    fn showlists_reports_the_current_nest_and_page_state() {
        let mut state = engine();
        state.set_source_text("\\showlists");
        state.dispatch_x_command().unwrap();

        assert!(state
            .transcript
            .iter()
            .any(|line| line == "### vertical mode entered at line 0"));
        assert!(state
            .transcript
            .iter()
            .any(|line| line.starts_with("prevdepth ")));
        assert!(state.transcript.iter().any(|line| line == "! OK"));
    }

    #[test]
    fn showgroups_reports_the_group_stack() {
        let mut state = engine();
        state.set_source_text("\\begingroup\\showgroups");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(
            state.transcript[1],
            "### semi simple group (level 1) entered at line 1 (\\begingroup)"
        );
        assert!(state
            .transcript
            .iter()
            .any(|line| line == "### bottom level"));
        assert!(state.transcript.iter().any(|line| line == "! OK"));
    }

    #[test]
    fn showifs_reports_empty_and_active_conditionals() {
        let mut state = engine();
        state.set_source_text("\\showifs");
        state.dispatch_x_command().unwrap();
        assert!(state
            .transcript
            .iter()
            .any(|line| line == "### no active conditionals"));

        let mut state = engine();
        state.set_source_text("\\ifnum1=1 \\showifs");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        assert!(state
            .transcript
            .iter()
            .any(|line| line.starts_with("### level 1: \\ifnum")));
    }

    #[test]
    fn openout_builds_an_open_whatsit_and_out_what_opens_the_stream() {
        let mut state = engine();
        state.set_source_text("\\openout2=out-file");
        state.dispatch_x_command().unwrap();

        let node = state.link(state.contrib_head()).unwrap();
        assert_eq!(state.node_type(node).unwrap(), WHATSIT_NODE);
        assert_eq!(state.node_subtype(node).unwrap(), OPEN_NODE);
        assert_eq!(state.mem[node as usize + 1].s0, 2);
        assert_eq!(
            state.string_text(state.mem[node as usize + 1].s1).unwrap(),
            "out-file"
        );

        state.out_what(node).unwrap();
        assert!(state.write_stream_open(2));
        assert_eq!(state.write_streams[2].as_ref().unwrap().name, "out-file");
    }

    #[test]
    fn deferred_write_normalizes_the_stream_and_keeps_its_token_list() {
        let mut state = engine();
        state.set_source_text("\\write-2{abc}");
        state.dispatch_x_command().unwrap();

        let node = state.link(state.contrib_head()).unwrap();
        assert_eq!(state.node_subtype(node).unwrap(), WRITE_NODE);
        assert_eq!(state.mem[node as usize + 1].s0, 17);
        let reference = state
            .stored_token_list_start(state.mem[node as usize + 1].s1)
            .unwrap();
        assert_eq!(state.token_list_to_string(reference).unwrap(), "abc");

        state.out_what(node).unwrap();
        assert_eq!(state.transcript, vec!["abc"]);
    }

    #[test]
    fn immediate_write_outputs_and_restores_the_list_tail() {
        let mut state = engine();
        state.set_source_text("\\immediate\\write16{abc}");
        let old_tail = state.tail;

        state.dispatch_x_command().unwrap();

        assert_eq!(state.transcript, vec!["abc"]);
        assert_eq!(state.tail, old_tail);
        assert_eq!(state.link(old_tail).unwrap(), TEX_NULL);
    }

    #[test]
    fn immediate_open_write_and_close_update_state_owned_streams() {
        let mut state = engine();
        state.set_source_text(
            "\\immediate\\openout1=journal \\immediate\\write1{line}\\immediate\\closeout1",
        );
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert!(!state.write_stream_open(1));
        let stream = state.write_streams[1].as_ref().unwrap();
        assert_eq!(stream.name, "journal");
        assert_eq!(stream.text, "line\n");
    }

    #[test]
    fn setlanguage_and_fix_language_emit_language_whatsits() {
        let mut state = engine();
        state.push_nest().unwrap();
        state.mode = TeXMode::Hmode as i16;
        state.set_int_par(INT_PAR_LEFT_HYPHEN_MIN, 4).unwrap();
        state.set_int_par(INT_PAR_RIGHT_HYPHEN_MIN, 5).unwrap();
        state.set_source_text("\\setlanguage7");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        assert_eq!(state.node_subtype(node).unwrap(), LANGUAGE_NODE);
        assert_eq!(state.mem[node as usize + 1].s1, 7);
        assert_eq!(state.mem[node as usize + 1].b16_s1(), 4);
        assert_eq!(state.mem[node as usize + 1].b16_s0(), 5);

        state.set_int_par(INT_PAR_LANGUAGE, 8).unwrap();
        state.fix_language().unwrap();
        assert_eq!(state.node_subtype(state.tail).unwrap(), LANGUAGE_NODE);
        assert_eq!(state.mem[state.tail as usize + 1].s1, 8);
    }

    #[test]
    fn utility_extensions_update_timer_random_and_special_state() {
        let mut state = engine();
        state.set_source_text("\\resettimer\\setrandomseed-42\\pdfsavepos\\special{pdf:literal}");
        for _ in 0..4 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.random_seed, 42);
        let special = state.tail;
        assert_eq!(state.node_subtype(special).unwrap(), SPECIAL_NODE);
        state.out_what(special).unwrap();
        // XXX1 opcode + one-byte length + payload bytes.
        let mut expected = vec![XXX1, b"pdf:literal".len() as u8];
        expected.extend_from_slice(b"pdf:literal");
        assert_eq!(&state.dvi_buf[..expected.len()], expected.as_slice());
        assert!(state.epochseconds > 0 || state.microseconds >= 0);
    }

    #[test]
    fn encoding_and_locale_extensions_update_state_slots() {
        let mut state = engine();
        state.set_source_text(
            "\\XeTeXinputencoding{utf8}\\XeTeXdefaultencoding{latin1}\\XeTeXlinebreaklocale{en_US}",
        );
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.file_sources[state.in_open].input_mode, 1);
        assert_eq!(
            state
                .int_par(crate::state::INT_PAR_XETEX_DEFAULT_INPUT_MODE)
                .unwrap(),
            5
        );
        assert_eq!(
            state
                .int_par(crate::state::INT_PAR_XETEX_LINEBREAK_LOCALE)
                .unwrap(),
            state.intern_string(&"en_US".chars().map(|c| c as i32).collect::<Vec<_>>())
        );
    }

    #[test]
    fn native_glyph_extension_builds_a_glyph_whatsit() {
        let mut state = engine();
        state.push_nest().unwrap();
        state.mode = TeXMode::Hmode as i16;
        state.eqtb[CUR_FONT_LOC].s1 = 1;
        state.font_area.insert(1, 0xFFFE);
        state.set_font_char_metric(
            1,
            65,
            crate::state::FontCharMetric {
                width: 11,
                height: 7,
                depth: 2,
                italic: 0,
            },
        );
        state.set_source_text("\\XeTeXglyph65");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        assert_eq!(state.node_subtype(node).unwrap(), GLYPH_NODE);
        assert_eq!(state.mem[node as usize + 4].b16_s2(), 1);
        assert_eq!(state.mem[node as usize + 4].b16_s1(), 65);
        assert_eq!(state.mem[node as usize + 1].s1, 11);
        assert_eq!(state.mem[node as usize + 3].s1, 7);
    }

    #[test]
    fn pdf_picture_extension_scans_options_and_packs_the_path() {
        let mut state = engine();
        state
            .picture_bounds
            .insert("image.pdf".to_string(), (0.0, 0.0, 10.0, 20.0));
        state.set_source_text("\\XeTeXpdffile{image.pdf} page 2 crop width 5pt");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        assert_eq!(state.node_subtype(node).unwrap(), PDF_NODE);
        assert_eq!(state.mem[node as usize + 4].b16_s0(), 2);
        assert_eq!(state.mem[node as usize + 4].b16_s1(), 9);
        assert_eq!(state.mem[node as usize + 8].b16_s1(), 1);
        assert_eq!(state.mem[node as usize + 1].s1, 5 * 65_536);
    }

    fn png_fixture(width: u32, height: u32) -> Vec<u8> {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&width.to_be_bytes());
        png.extend_from_slice(&height.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]);
        png.extend_from_slice(&[0; 4]);
        png.extend_from_slice(&0u32.to_be_bytes());
        png.extend_from_slice(b"IEND");
        png.extend_from_slice(&[0; 4]);
        png
    }

    fn two_page_pdf_fixture() -> Vec<u8> {
        b"%PDF-1.4\n\
          1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n\
          2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n\
          3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /CropBox [10 20 300 400] >>\nendobj\n\
          4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>\nendobj\n\
          trailer\n<< /Root 1 0 R >>\n"
            .to_vec()
    }

    #[test]
    fn picfile_probes_registered_raster_bytes() {
        let mut state = engine();
        state.register_input_byte_source("img.png", png_fixture(100, 50));
        state.set_source_text("\\XeTeXpicfile{img.png}");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        assert_eq!(state.node_subtype(node).unwrap(), PIC_NODE);
        assert_eq!(state.error_count, 0);
        // 100px at 72dpi → 100.375pt; 50px → 50.1875pt.
        assert_eq!(state.mem[node as usize + 1].s1, d2fix(100.375));
        assert_eq!(state.mem[node as usize + 3].s1, d2fix(50.1875));
        assert_eq!(state.mem[node as usize + 4].b16_s1(), 7);
    }

    #[test]
    fn picfile_missing_or_unrecognized_reports_error_without_node() {
        let mut state = engine();
        state.set_source_text("\\XeTeXpicfile{missing.png}");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.error_count, 1);
        assert_eq!(state.tail, state.head);
        assert!(state
            .transcript
            .iter()
            .any(|l| l.contains("Unable to load picture or PDF file 'missing.png'")));

        let mut state = engine();
        state.register_input_byte_source("bad.png", b"not an image".to_vec());
        state.set_source_text("\\XeTeXpicfile{bad.png}");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.error_count, 1);
        assert_eq!(state.tail, state.head);

        // `\XeTeXpicfile` on a PDF fails the raster probe, as in C.
        let mut state = engine();
        state.register_input_byte_source("doc.pdf", two_page_pdf_fixture());
        state.set_source_text("\\XeTeXpicfile{doc.pdf}");
        state.dispatch_x_command().unwrap();
        assert_eq!(state.error_count, 1);
        assert_eq!(state.tail, state.head);
    }

    #[test]
    fn pdffile_probes_crop_box_and_stores_pdfbox_none() {
        let mut state = engine();
        state.register_input_byte_source("doc.pdf", two_page_pdf_fixture());
        state.set_source_text("\\XeTeXpdffile{doc.pdf}");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        const BP_TO_PT: f64 = 72.27 / 72.0;
        assert_eq!(state.node_subtype(node).unwrap(), PDF_NODE);
        assert_eq!(state.error_count, 0);
        assert_eq!(state.mem[node as usize + 8].b16_s1(), 6);
        // pdfbox_none probes the CropBox [10 20 300 400] → 290×380 bp.
        assert_eq!(state.mem[node as usize + 1].s1, d2fix(290.0 * BP_TO_PT));
        assert_eq!(state.mem[node as usize + 3].s1, d2fix(380.0 * BP_TO_PT));
    }

    #[test]
    fn pdffile_media_keyword_selects_the_media_box() {
        let mut state = engine();
        state.register_input_byte_source("doc.pdf", two_page_pdf_fixture());
        state.set_source_text("\\XeTeXpdffile{doc.pdf} media");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        const BP_TO_PT: f64 = 72.27 / 72.0;
        assert_eq!(state.mem[node as usize + 8].b16_s1(), 2);
        assert_eq!(state.mem[node as usize + 1].s1, d2fix(612.0 * BP_TO_PT));
        assert_eq!(state.mem[node as usize + 3].s1, d2fix(792.0 * BP_TO_PT));
    }

    #[test]
    fn pdffile_clamps_out_of_range_and_negative_pages() {
        for opts in ["page 99", "page -1", "page -2"] {
            let mut state = engine();
            state.register_input_byte_source("doc.pdf", two_page_pdf_fixture());
            state.set_source_text(&format!("\\XeTeXpdffile{{doc.pdf}} {opts} media"));

            state.dispatch_x_command().unwrap();

            let node = state.tail;
            const BP_TO_PT: f64 = 72.27 / 72.0;
            // page 99 clamps to 2; -1 resolves to 2; -2 resolves to 1.
            let (w, h) = if opts == "page -2" {
                (612.0 * BP_TO_PT, 792.0 * BP_TO_PT)
            } else {
                (200.0 * BP_TO_PT, 200.0 * BP_TO_PT)
            };
            assert_eq!(state.mem[node as usize + 1].s1, d2fix(w), "{opts}");
            assert_eq!(state.mem[node as usize + 3].s1, d2fix(h), "{opts}");
        }
    }

    #[test]
    fn improper_image_size_reports_error_and_is_ignored() {
        let mut state = engine();
        state.register_input_byte_source("img.png", png_fixture(100, 50));
        state.set_source_text("\\XeTeXpicfile{img.png} width -5pt");

        state.dispatch_x_command().unwrap();

        assert_eq!(state.error_count, 1);
        assert!(state
            .transcript
            .iter()
            .any(|l| l.starts_with("! Improper image size")));
        let node = state.tail;
        assert_eq!(state.node_subtype(node).unwrap(), PIC_NODE);
        // The invalid width is ignored → natural probed size.
        assert_eq!(state.mem[node as usize + 1].s1, d2fix(100.375));
    }

    #[test]
    fn rotated_picfile_snaps_corners_to_the_aabb() {
        // Two chained 30° rotations: C snaps the corners to the rotated
        // AABB each pass (≈143.8×137.1pt), so width ends up larger than
        // height — corner-shifting semantics would instead produce the
        // 60°-rotated original (≈93.7×112.0pt).
        let mut state = engine();
        state.register_input_byte_source("img.png", png_fixture(100, 50));
        state.set_source_text("\\XeTeXpicfile{img.png} rotated 30 rotated 30");

        state.dispatch_x_command().unwrap();

        let node = state.tail;
        let w = state.mem[node as usize + 1].s1;
        let h = state.mem[node as usize + 3].s1;
        assert!(w > h && w > d2fix(120.0));
    }

    #[test]
    fn pdf_page_count_probes_registered_bytes() {
        let mut state = engine();
        state.register_input_byte_source("doc.pdf", two_page_pdf_fixture());
        state.cur_name = "doc.pdf".chars().map(|c| c as i32).collect();
        assert_eq!(state.count_pdf_file_pages(), 2);

        state.register_pdf_page_count("doc.pdf", 9);
        assert_eq!(state.count_pdf_file_pages(), 9);
    }

    #[test]
    fn hyphenation_records_exceptions_for_the_current_language() {
        let mut state = engine();
        state.set_int_par(INT_PAR_LANGUAGE, 3).unwrap();
        state.set_source_text("\\hyphenation{as-sert ex-am-ple}");

        state.dispatch_x_command().unwrap();

        let mut key = "assert".chars().map(|c| c as i32).collect::<Vec<_>>();
        key.push(3);
        assert_eq!(state.hyph_exceptions[&key], vec![2]);
        let mut key = "example".chars().map(|c| c as i32).collect::<Vec<_>>();
        key.push(3);
        assert_eq!(state.hyph_exceptions[&key], vec![2, 4]);
    }

    #[test]
    fn patterns_are_initex_only_and_store_hyphenation_digits() {
        let mut state = engine();
        state.set_source_text("\\patterns{a1b}");
        assert!(state.dispatch_x_command().is_err());

        let mut state = engine();
        state.in_initex_mode = true;
        state.set_source_text("\\patterns{a1b .c2d}");
        state.dispatch_x_command().unwrap();

        let mut key = "a1b".chars().map(|c| c as i32).collect::<Vec<_>>();
        key.push(0);
        assert_eq!(state.hyph_patterns[&key], vec![0, 1, 0]);
        let mut key = vec![0, i32::from(b'c'), i32::from(b'2'), i32::from(b'd')];
        key.push(0);
        assert_eq!(state.hyph_patterns[&key], vec![0, 0, 2, 0]);
    }

    fn font_engine() -> TeXEngineState {
        let mut state = engine();
        state.register_font_source("cmr10", crate::state::FontRecord::default());
        state
    }

    #[test]
    fn font_loads_a_registered_source_and_selects_the_current_font() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\foo");

        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.font_ptr(), 1);
        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::SetFont as u8);
        assert_eq!(state.cs_character(foo), 1);
        assert_eq!(state.current_font(), 1);
        assert_eq!(state.font_record(1).unwrap().size, 10 * 65_536);
        assert_eq!(
            state.font_record(1).unwrap().cs_name,
            vec![i32::from(b'f'), i32::from(b'o'), i32::from(b'o')]
        );
    }

    #[test]
    fn font_at_and_scaled_forms_set_the_requested_size() {
        let mut state = font_engine();
        state.set_source_text("\\font\\a=cmr10 at 12pt \\font\\b=cmr10 scaled 2000");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.font_record(1).unwrap().size, 12 * 65_536);
        assert_eq!(state.font_record(2).unwrap().size, 20 * 65_536);
    }

    #[test]
    fn font_deduplicates_identical_requests() {
        let mut state = font_engine();
        state.set_source_text("\\font\\a=cmr10 \\font\\b=cmr10 \\font\\c=cmr10 at 15pt");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.font_ptr(), 2);
        assert_eq!(state.cs_character(cs(&state, "b")), 1);
        assert_eq!(state.cs_character(cs(&state, "c")), 2);
    }

    #[test]
    fn font_missing_source_reports_and_substitutes_nullfont() {
        let mut state = engine();
        state.set_source_text("\\font\\foo=missing-font");
        state.dispatch_x_command().unwrap();

        assert_eq!(state.cs_character(cs(&state, "foo")), 0);
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("not loadable")));

        let mut suppressed = engine();
        suppressed
            .set_int_par(INT_PAR_SUPPRESS_FONTNOTFOUND_ERROR, 1)
            .unwrap();
        suppressed.set_source_text("\\font\\foo=missing-font");
        suppressed.dispatch_x_command().unwrap();
        assert_eq!(suppressed.cs_character(cs(&suppressed, "foo")), 0);
        assert!(!suppressed
            .transcript
            .iter()
            .any(|line| line.contains("not loadable")));
    }

    #[test]
    fn font_improper_at_size_recovers_with_ten_points() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 at -2pt");
        state.dispatch_x_command().unwrap();

        assert_eq!(state.font_record(1).unwrap().size, 10 * 65_536);
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("Improper `at' size")));
    }

    #[test]
    fn textfont_family_slots_assign_and_restore_with_groups() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\foo \\textfont3=\\foo");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.eqtb[FAMILY_BASE + 3].s1, 1);
        assert_eq!(
            state.eqtb[FAMILY_BASE + 3].b16_s1(),
            CommandCode::Data as u16
        );
        assert_eq!(state.current_font(), 1);

        state.new_save_level(SIMPLE_GROUP).unwrap();
        state.set_source_text("\\nullfont\\textfont3=\\nullfont");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        assert_eq!(state.eqtb[FAMILY_BASE + 3].s1, 0);
        assert_eq!(state.current_font(), 0);

        state.unsave().unwrap();
        assert_eq!(state.eqtb[FAMILY_BASE + 3].s1, 1);
        assert_eq!(state.current_font(), 1);
    }

    #[test]
    fn script_and_scriptscript_family_offsets_map_the_compact_region() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\scriptfont2=\\foo \\scriptscriptfont5=\\foo");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.eqtb[FAMILY_BASE + 256 + 2].s1, 1);
        assert_eq!(state.eqtb[FAMILY_BASE + 512 + 5].s1, 1);
    }

    #[test]
    fn fontdimen_writes_extends_last_font_and_reads_back() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\fontdimen6\\foo=12pt \\fontdimen8\\foo=3pt");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        let record = state.font_record(1).unwrap();
        assert_eq!(record.params[6], 12 * 65_536);
        assert_eq!(record.params[8], 3 * 65_536);

        state.set_source_text("\\fontdimen6\\foo");
        state.scan_dimen(false, false, false).unwrap();
        assert_eq!(state.cur_val, 12 * 65_536);
    }

    #[test]
    fn fontdimen_rejects_zero_and_non_tail_fonts() {
        let mut state = font_engine();
        state.set_source_text("\\font\\a=cmr10 \\font\\b=cmr10 at 11pt");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        state.set_source_text("\\fontdimen9\\a=1pt");
        state.dispatch_x_command().unwrap();
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("has only")));

        state.set_source_text("\\fontdimen0\\b=1pt");
        state.dispatch_x_command().unwrap();
    }

    #[test]
    fn hyphenchar_and_skewchar_assign_and_read() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\hyphenchar\\foo=65 \\skewchar\\foo=66");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        let record = state.font_record(1).unwrap();
        assert_eq!(record.hyphen_char, 65);
        assert_eq!(record.skew_char, 66);

        state.set_source_text("\\hyphenchar\\foo");
        assert_eq!(state.scan_int().unwrap(), 65);
        state.set_source_text("\\skewchar\\foo");
        assert_eq!(state.scan_int().unwrap(), 66);
    }

    #[test]
    fn lpcode_and_rpcode_use_character_codes_for_tfm_fonts() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\lpcode\\foo`A=250 \\rpcode\\foo`A=300");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }

        assert_eq!(state.get_cp_code(1, u32::from(b'A'), true), 250);
        assert_eq!(state.get_cp_code(1, u32::from(b'A'), false), 300);

        state.set_source_text("\\lpcode\\foo`A");
        assert_eq!(state.scan_int().unwrap(), 250);
    }

    #[test]
    fn lpcode_scans_glyph_names_and_unicode_for_native_fonts() {
        let mut state = font_engine();
        let mut record = crate::state::FontRecord::default();
        record.glyph_names.insert("A.gid".to_string(), 42);
        record.char_to_glyph.insert(u32::from(b'A'), 7);
        state.register_font_source("natfont", record);
        state.set_source_text("\\font\\nat=natfont \\lpcode\\nat/A.gid =11 \\rpcode\\nat u`A=13");
        state.dispatch_x_command().unwrap();
        state.font_area.insert(1, 0xFFFE);
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        assert_eq!(state.get_cp_code(1, 42, true), 11);
        assert_eq!(state.get_cp_code(1, 7, false), 13);
    }

    #[test]
    fn global_font_definition_survives_group_exit() {
        let mut state = font_engine();
        state.new_save_level(SIMPLE_GROUP).unwrap();
        state.set_source_text("\\global\\font\\foo=cmr10 \\foo");
        state.dispatch_x_command().unwrap();
        state.unsave().unwrap();

        let foo = cs(&state, "foo");
        assert_eq!(state.cs_command(foo), CommandCode::SetFont as u8);
        assert_eq!(state.cs_character(foo), 1);
    }

    #[test]
    fn nullfont_selects_font_zero_and_missing_ident_recovers() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\foo \\nullfont");
        for _ in 0..3 {
            state.dispatch_x_command().unwrap();
        }
        assert_eq!(state.current_font(), 0);

        state.set_source_text("x");
        assert_eq!(state.scan_font_ident().unwrap(), 0);
    }

    /// Registers `natfont` as a native (`OTGR`) font and selects it as the
    /// current font.
    fn native_engine() -> TeXEngineState {
        let mut state = font_engine();
        let mut record = crate::state::FontRecord {
            params: vec![0; 9],
            bc: 32,
            ec: 126,
            glyph_count: 500,
            letter_space: 5,
            ..Default::default()
        };
        record.params[crate::state::QUAD_CODE as usize] = 100;
        record.params[crate::state::X_HEIGHT_CODE as usize] = 45;
        record.params[crate::state::CAP_HEIGHT_CODE as usize] = 90;
        record.char_to_glyph.insert(u32::from(b'A'), 7);
        record.char_to_glyph.insert(u32::from(b'B'), 8);
        record.glyph_names.insert("A.gid".to_string(), 7);
        record.glyph_id_names.insert(7, "A.gid".to_string());
        record.glyph_metrics.insert(
            7,
            crate::state::NativeGlyphMetric {
                width: 50,
                height: 50,
                depth: 20,
                lsb: -30,
                rsb: -40,
            },
        );
        record.glyph_metrics.insert(
            8,
            crate::state::NativeGlyphMetric {
                width: 60,
                height: 44,
                depth: 0,
                lsb: 10,
                rsb: 12,
            },
        );
        state.register_font_source("natfont", record);
        state.set_source_text("\\font\\nat=natfont \\nat");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.font_area.insert(1, crate::state::OTGR_FONT_FLAG);
        state
    }

    #[test]
    fn fontchar_queries_read_tfm_metrics_and_clamp_to_char_range() {
        let mut state = font_engine();
        state.set_source_text("\\font\\foo=cmr10 \\foo");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.font_table[1].bc = 0;
        state.font_table[1].ec = 255;
        state.set_font_char_metric(
            1,
            u32::from(b'A'),
            crate::state::FontCharMetric {
                width: 10,
                height: 20,
                depth: 30,
                italic: 40,
            },
        );

        state.set_source_text(
            "\\fontcharwd\\foo`A \\fontcharht\\foo`A \\fontchardp\\foo`A \\fontcharic\\foo`A",
        );
        assert_eq!(state.scan_int().unwrap(), 10);
        assert_eq!(state.scan_int().unwrap(), 20);
        assert_eq!(state.scan_int().unwrap(), 30);
        assert_eq!(state.scan_int().unwrap(), 40);

        state.set_source_text("\\fontcharwd\\foo256");
        assert_eq!(state.scan_int().unwrap(), 0);
    }

    #[test]
    fn fontchar_queries_use_native_glyph_metrics_with_snap_zones() {
        let mut state = native_engine();
        state.set_source_text("\\fontcharwd\\nat`A \\fontcharht\\nat`A \\fontchardp\\nat`A");
        assert_eq!(state.scan_int().unwrap(), 50);
        assert_eq!(state.scan_int().unwrap(), 50);
        assert_eq!(state.scan_int().unwrap(), 20);

        // height 44 snaps to x-height 45 (fuzz = quad/25 = 4).
        state.set_source_text("\\fontcharht\\nat`B");
        assert_eq!(state.scan_int().unwrap(), 45);

        // Italic correction: rsb < 0 yields letter_space - rsb.
        state.set_source_text("\\fontcharic\\nat`A");
        assert_eq!(state.scan_int().unwrap(), 45);
        // Nonnegative rsb yields letter_space.
        state.set_source_text("\\fontcharic\\nat`B");
        assert_eq!(state.scan_int().unwrap(), 5);
    }

    #[test]
    fn xetex_glyphbounds_reports_native_edges() {
        let mut state = native_engine();
        state.set_source_text(
            "\\XeTeXglyphbounds1 7 \\XeTeXglyphbounds2 7 \\XeTeXglyphbounds3 7 \\XeTeXglyphbounds4 7",
        );
        assert_eq!(state.scan_int().unwrap(), -30);
        assert_eq!(state.scan_int().unwrap(), 50);
        assert_eq!(state.scan_int().unwrap(), -40);
        assert_eq!(state.scan_int().unwrap(), 20);

        state.set_source_text("\\XeTeXglyphbounds0 7");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("edge index from 1 to 4")));

        let mut tfm = font_engine();
        tfm.set_source_text("\\font\\foo=cmr10 \\foo \\XeTeXglyphbounds1 7");
        tfm.dispatch_x_command().unwrap();
        tfm.dispatch_x_command().unwrap();
        assert_eq!(tfm.scan_int().unwrap(), 0);
        assert!(tfm
            .transcript
            .iter()
            .any(|line| line.contains("not a native platform font")));
    }

    #[test]
    fn xetex_charglyph_glyphindex_and_fonttype_queries() {
        let mut state = native_engine();
        state.font_table[1].font_type = 2;

        state.set_source_text("\\XeTeXcharglyph`A \\XeTeXglyphindex A.gid ");
        assert_eq!(state.scan_int().unwrap(), 7);
        assert_eq!(state.scan_int().unwrap(), 7);

        state.set_source_text("\\XeTeXfonttype\\nat");
        assert_eq!(state.scan_int().unwrap(), 2);
        state.font_area.insert(1, crate::state::AAT_FONT_FLAG);
        state.set_source_text("\\XeTeXfonttype\\nat");
        assert_eq!(state.scan_int().unwrap(), 1);

        state.set_source_text("\\XeTeXfirstfontchar\\nat \\XeTeXlastfontchar\\nat");
        assert_eq!(state.scan_int().unwrap(), 32);
        assert_eq!(state.scan_int().unwrap(), 126);

        state.set_source_text("\\XeTeXcountglyphs\\nat");
        assert_eq!(state.scan_int().unwrap(), 500);

        // A TFM font answers 0 for native-only counts and its bc/ec range.
        let mut tfm = font_engine();
        tfm.font_table.push(crate::state::FontRecord {
            bc: 3,
            ec: 99,
            ..Default::default()
        });
        tfm.define_cs("bar", CommandCode::SetFont, 1);
        tfm.set_source_text(
            "\\XeTeXcountglyphs\\bar \\XeTeXfirstfontchar\\bar \\XeTeXlastfontchar\\bar",
        );
        assert_eq!(tfm.scan_int().unwrap(), 0);
        assert_eq!(tfm.scan_int().unwrap(), 3);
        assert_eq!(tfm.scan_int().unwrap(), 99);
        tfm.set_source_text("\\XeTeXfonttype\\bar");
        assert_eq!(tfm.scan_int().unwrap(), 0);
    }

    #[test]
    fn xetex_engine_queries_use_registered_tables_or_error_paths() {
        let mut state = native_engine();
        state.font_table[1].font_type = 2;
        state.register_font_engine_value(1, XETEX_OT_COUNT_SCRIPTS_CODE - XETEX_INT, &[], 3);
        state.register_font_engine_value(1, XETEX_OT_FEATURE_CODE - XETEX_INT, &[2, 3, 4], 12_345);
        state.register_font_engine_named(
            1,
            XETEX_FIND_FEATURE_BY_NAME_CODE - XETEX_INT,
            -1,
            "feat",
            77,
        );

        state.set_source_text("\\XeTeXOTcountscripts\\nat");
        assert_eq!(state.scan_int().unwrap(), 3);
        state.set_source_text("\\XeTeXOTfeaturetag\\nat 2 3 4");
        assert_eq!(state.scan_int().unwrap(), 12_345);

        // `\XeTeXfindfeaturebyname` accepts AAT or Graphite engines only.
        state.font_table[1].font_type = 3;
        state.set_source_text("\\XeTeXfindfeaturebyname\\nat feat ");
        assert_eq!(state.scan_int().unwrap(), 77);

        // Variation codes always yield zero, like C's unconditional path.
        state.set_source_text("\\XeTeXcountvariations\\nat \\XeTeXvariation\\nat");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(state.scan_int().unwrap(), 0);

        // Feature queries on a non-AAT/Graphite font record the C error and
        // recover with -1.
        state.font_table[1].font_type = 2;
        state.set_source_text("\\XeTeXfeaturecode\\nat 0");
        assert_eq!(state.scan_int().unwrap(), -1);
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("not an AAT or Graphite font")));

        // An OpenType feature query on a Graphite font consults the table.
        state.font_table[1].font_type = 3;
        state.register_font_engine_value(1, XETEX_COUNT_FEATURES_CODE - XETEX_INT, &[], 9);
        state.set_source_text("\\XeTeXcountfeatures\\nat");
        assert_eq!(state.scan_int().unwrap(), 9);
    }

    #[test]
    fn last_item_scalars_read_engine_state() {
        let mut state = engine();
        state.set_source_text(
            "\\inputlineno \\badness \\randomseed \\shellescape \\eTeXversion \\XeTeXversion \\pdflastxpos \\pdflastypos",
        );
        state.line = 42;
        state.last_badness = 17;
        state.random_seed = 99;
        state.pdf_last_x_pos = 1234;
        state.pdf_last_y_pos = -567;
        assert_eq!(state.scan_int().unwrap(), 42);
        assert_eq!(state.scan_int().unwrap(), 17);
        assert_eq!(state.scan_int().unwrap(), 99);
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(state.scan_int().unwrap(), 2);
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(state.scan_int().unwrap(), 1234);
        assert_eq!(state.scan_int().unwrap(), -567);

        state.shell_escape_enabled = true;
        state.set_source_text("\\shellescape");
        assert_eq!(state.scan_int().unwrap(), 1);

        // \lastnodetype outside a list follows the C empty tail path.
        state.set_source_text("\\lastnodetype");
        assert_eq!(state.scan_int().unwrap(), -1);
    }

    #[test]
    fn current_group_and_if_quantities_report_scanner_state() {
        let mut state = engine();
        state.set_source_text("\\currentgrouplevel \\currentgrouptype");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(state.scan_int().unwrap(), 0);

        state.new_save_level(SIMPLE_GROUP).unwrap();
        state.set_source_text("\\currentgrouplevel \\currentgrouptype");
        assert_eq!(state.scan_int().unwrap(), 1);
        assert_eq!(state.scan_int().unwrap(), 1);

        state.set_source_text("\\ifnum1=1 \\currentiflevel \\currentiftype \\currentifbranch\\fi");
        assert_eq!(state.scan_int().unwrap(), 1);
        assert_eq!(state.scan_int().unwrap(), 3);
        assert_eq!(state.scan_int().unwrap(), 1);
        state.unsave().unwrap();
    }

    #[test]
    fn glue_stretch_and_shrink_queries_read_spec_fields() {
        let mut state = engine();
        state.set_source_text("\\gluestretchorder 1pt plus 2fill \\glueshrinkorder 1pt minus 3pt");
        assert_eq!(state.scan_int().unwrap(), 2);
        assert_eq!(state.scan_int().unwrap(), 0);

        state.set_source_text("\\gluestretch 1pt plus 2pt minus 3pt \\glueshrink 1pt minus 4pt");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 2 * 65_536);
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 4 * 65_536);
    }

    #[test]
    fn parshape_element_queries_follow_the_c_pair_layout() {
        let mut state = engine();
        state.set_source_text("\\parshape=2 1pt 2pt 3pt 4pt");
        state.dispatch_x_command().unwrap();

        state.set_source_text(
            "\\parshapelength1 \\parshapeindent1 \\parshapedimen3 \\parshapedimen4",
        );
        assert_eq!(state.scan_int().unwrap(), 2 * 65_536);
        assert_eq!(state.scan_int().unwrap(), 65_536);
        assert_eq!(state.scan_int().unwrap(), 3 * 65_536);
        assert_eq!(state.scan_int().unwrap(), 4 * 65_536);
    }

    #[test]
    fn xetex_pdfpagecount_reads_registered_counts() {
        let mut state = engine();
        state.register_pdf_page_count("book.pdf", 12);
        state.set_source_text(
            "\\XeTeXpdfpagecount \"book.pdf\" \\XeTeXpdfpagecount \"missing.pdf\"",
        );
        assert_eq!(state.scan_int().unwrap(), 12);
        assert_eq!(state.scan_int().unwrap(), 0);
    }

    #[test]
    fn em_and_ex_units_use_the_current_font_parameters() {
        let mut state = font_engine();
        let mut record = crate::state::FontRecord::default();
        record.params[crate::state::QUAD_CODE as usize] = 10 * 65_536;
        record.params[crate::state::X_HEIGHT_CODE as usize] = 5 * 65_536;
        state.register_font_source("big", record);
        state.set_source_text("\\font\\foo=big \\foo");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        state.set_source_text("1em 2ex 0.5em");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 10 * 65_536);
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 10 * 65_536);
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 5 * 65_536);
    }

    fn dispatch_all(state: &mut TeXEngineState, max: usize) {
        for _ in 0..max {
            if state.dispatch_x_command().is_err() {
                break;
            }
        }
    }

    fn tfm_engine_with_metrics() -> TeXEngineState {
        let mut state = font_engine();
        for &ch in b"filab-" {
            state.set_font_char_metric(
                1,
                u32::from(ch),
                crate::state::FontCharMetric {
                    width: 10,
                    height: 7,
                    depth: 0,
                    italic: 2,
                },
            );
        }
        state.set_font_char_metric(
            1,
            0xFB01, // fi ligature char
            crate::state::FontCharMetric {
                width: 20,
                height: 7,
                depth: 0,
                italic: 0,
            },
        );
        state.set_source_text("\\font\\foo=cmr10 \\foo");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.bc = 0;
            font.ec = 0x10FFFF;
        }
        state
    }

    #[test]
    fn hmode_tfm_char_appends_a_character_node() {
        let mut state = tfm_engine_with_metrics();
        state.set_source_text("a");

        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();

        let mut p = state.link(state.head).unwrap();
        if state.node_type(p).unwrap() == HLIST_NODE {
            p = state.link(p).unwrap();
        }
        assert!(state.is_char_node(p));
        assert_eq!(state.mem[p as usize].b16_s1(), 1);
        assert_eq!(state.mem[p as usize].b16_s0(), u16::from(b'a'));
    }

    #[test]
    fn hmode_tfm_ligature_program_replaces_fi() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.char_tags
                .insert(u32::from(b'f'), crate::state::LIG_TAG);
            font.char_remainders.insert(u32::from(b'f'), 0);
            font.lig_kern.push(crate::state::LigKernInsn {
                skip: 0,
                next_char: i32::from(b'i'),
                op: 0,
                remainder: 0xFB01,
            });
        }
        state.set_source_text("fi");

        dispatch_all(&mut state, 12);

        let mut p = state.link(state.head).unwrap();
        if state.node_type(p).unwrap() == HLIST_NODE {
            p = state.link(p).unwrap();
        }
        assert_eq!(state.node_type(p).unwrap(), LIGATURE_NODE);
        assert_eq!(state.mem[p as usize + 1].b16_s0(), 0xFB01u16);
        let mut chars = Vec::new();
        let mut q = state.lig_ptr(p).unwrap();
        while q > 0 {
            chars.push(state.mem[q as usize].b16_s0());
            q = state.link(q).unwrap();
        }
        assert_eq!(chars, vec![u16::from(b'f'), u16::from(b'i')]);
        // op 0 (`=:`) consumes both chars; nothing follows the ligature
        // except the paragraph-end glue.
        let after = state.link(p).unwrap();
        assert!(
            after == TEX_NULL || !state.is_char_node(after),
            "consumed 'i' must not remain a separate char node"
        );
    }

    #[test]
    fn hmode_tfm_kern_program_appends_a_kern_node() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.char_tags
                .insert(u32::from(b'a'), crate::state::LIG_TAG);
            font.char_remainders.insert(u32::from(b'a'), 0);
            font.kerns.push(77);
            font.lig_kern.push(crate::state::LigKernInsn {
                skip: 0,
                next_char: i32::from(b'f'),
                op: 128,
                remainder: 0,
            });
        }
        state.set_source_text("af");

        dispatch_all(&mut state, 12);

        let mut kinds = Vec::new();
        let mut p = state.link(state.head).unwrap();
        while p > 0 {
            kinds.push(if state.is_char_node(p) {
                -1
            } else {
                i32::from(state.node_type(p).unwrap())
            });
            p = state.link(p).unwrap();
        }
        let kern_pos = kinds
            .iter()
            .position(|&t| t == i32::from(KERN_NODE))
            .unwrap();
        assert_eq!(kinds[kern_pos - 1], -1);
        assert_eq!(kinds[kern_pos + 1], -1);
    }

    #[test]
    fn hmode_native_font_collects_a_native_word_node() {
        let mut state = native_engine();
        state.set_source_text("AB");

        dispatch_all(&mut state, 12);

        let mut p = state.link(state.head).unwrap();
        if state.node_type(p).unwrap() == HLIST_NODE {
            p = state.link(p).unwrap();
        }
        assert_eq!(state.node_type(p).unwrap(), WHATSIT_NODE);
        assert_eq!(state.node_subtype(p).unwrap(), NATIVE_WORD_NODE);
        assert_eq!(state.mem[p as usize + 4].b16_s2(), 1);
        assert_eq!(state.native_node_length(p).unwrap(), 2);
        assert_eq!(state.native_word_text_at(p, 0).unwrap(), u16::from(b'A'));
        assert_eq!(state.native_word_text_at(p, 1).unwrap(), u16::from(b'B'));
        assert_eq!(state.mem[p as usize + 1].s1, 110);
    }

    #[test]
    fn hmode_discretionary_builds_three_sublists() {
        let mut state = tfm_engine_with_metrics();
        state.set_source_text("a\\discretionary{-}{}{}b");

        dispatch_all(&mut state, 12);

        let mut found = false;
        let mut p = state.link(state.head).unwrap();
        while p > 0 {
            if state.node_type(p).unwrap() == DISC_NODE {
                found = true;
                let pre = state.mem[p as usize + 1].s0;
                assert!(pre > 0, "pre-break list should hold the '-' char");
                assert!(state.is_char_node(pre));
                assert_eq!(state.mem[pre as usize].b16_s0(), u16::from(b'-'));
            }
            p = state.link(p).unwrap();
        }
        assert!(found);
    }

    #[test]
    fn hmode_noboundary_suppresses_the_boundary_ligature() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.bchar_label = 0;
            font.lig_kern.push(crate::state::LigKernInsn {
                skip: 0,
                next_char: i32::from(b'a'),
                op: 0,
                remainder: i32::from(b'X'),
            });
        }
        state.set_source_text("\\noboundary a");

        dispatch_all(&mut state, 12);

        let mut p = state.link(state.head).unwrap();
        if state.node_type(p).unwrap() == HLIST_NODE {
            p = state.link(p).unwrap();
        }
        assert!(
            state.is_char_node(p),
            "\\noboundary must suppress the boundary ligature program"
        );
        assert_eq!(state.mem[p as usize].b16_s0(), u16::from(b'a'));
    }

    #[test]
    fn hmode_italic_correction_appends_a_kern_after_chars() {
        let mut state = tfm_engine_with_metrics();
        state.set_source_text("a\\/");

        dispatch_all(&mut state, 12);

        let mut found = false;
        let mut p = state.link(state.head).unwrap();
        while p > 0 {
            if state.node_type(p).unwrap() == KERN_NODE && state.mem[p as usize + 1].s1 == 2 {
                found = true;
            }
            p = state.link(p).unwrap();
        }
        assert!(found, "italic correction kern not found in list");
    }

    #[test]
    fn hmode_inter_char_tokens_expand_between_characters() {
        let mut state = tfm_engine_with_metrics();
        state.eqtb[crate::state::INT_PAR_BASE + INT_PAR_XETEX_INTER_CHAR_TOKENS].s1 = 1;
        state.set_source_text("\\XeTeXcharclass`\\a=1");
        state.dispatch_x_command().unwrap();
        state.set_source_text("\\XeTeXcharclass`\\b=2");
        state.dispatch_x_command().unwrap();
        state.set_source_text("\\XeTeXinterchartoks 1 2={\\kern1pt}");
        state.dispatch_x_command().unwrap();
        state.set_source_text("ab");

        dispatch_all(&mut state, 12);

        let mut kinds = Vec::new();
        let mut p = state.link(state.head).unwrap();
        while p > 0 {
            kinds.push(if state.is_char_node(p) {
                -1
            } else {
                i32::from(state.node_type(p).unwrap())
            });
            p = state.link(p).unwrap();
        }
        let kern_pos = kinds
            .iter()
            .position(|&t| t == i32::from(KERN_NODE))
            .expect("inter-char tokens should inject a kern between a and b");
        assert_eq!(kinds[kern_pos - 1], -1);
        assert_eq!(kinds[kern_pos + 1], -1);
    }
    /// Runs `line_break` on the current horizontal list and walks the
    /// enclosing contribution list, returning the line boxes, interline
    /// glue and penalties it produced.
    fn break_and_collect(state: &mut TeXEngineState) -> Vec<i32> {
        state.line_break(false).unwrap();
        let mut nodes = Vec::new();
        let mut p = state.link(state.contrib_head()).unwrap();
        while p != TEX_NULL {
            nodes.push(p);
            p = state.link(p).unwrap();
        }
        nodes
    }

    #[test]
    fn line_break_packs_a_single_line() {
        let mut state = tfm_engine_with_metrics();
        state.set_dimen_par(DIMEN_PAR_HSIZE, 1000).unwrap();
        state.set_source_text("ab");
        dispatch_all(&mut state, 8);

        let lines = break_and_collect(&mut state);
        let boxes: Vec<i32> = lines
            .iter()
            .copied()
            .filter(|&p| state.node_type(p).unwrap() == HLIST_NODE)
            .collect();
        assert_eq!(boxes.len(), 1);
        // The line box is packed to `\hsize`.
        assert_eq!(state.mem[(boxes[0] + 1) as usize].s1, 1000);
        assert_eq!(state.prev_graf, 1);
    }

    #[test]
    fn line_break_wraps_words_across_lines() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.params[crate::state::SPACE_STRETCH_CODE as usize] = 50;
        }
        state.set_dimen_par(DIMEN_PAR_HSIZE, 30).unwrap();
        state.set_int_par(INT_PAR_PRETOLERANCE, 10000).unwrap();
        state.set_source_text("ab ab ab");
        dispatch_all(&mut state, 20);

        let lines = break_and_collect(&mut state);
        let boxes: Vec<i32> = lines
            .iter()
            .copied()
            .filter(|&p| state.node_type(p).unwrap() == HLIST_NODE)
            .collect();
        // Three words of width 20 each at `\hsize` 30 produce three
        // lines, and `prev_graf` reflects the surviving line count.
        assert_eq!(boxes.len(), 3);
        assert_eq!(state.prev_graf, 3);
    }

    #[test]
    fn line_break_hanging_indent_shifts_the_first_line() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.params[crate::state::SPACE_STRETCH_CODE as usize] = 50;
        }
        state.set_dimen_par(DIMEN_PAR_HSIZE, 30).unwrap();
        state.set_dimen_par(DIMEN_PAR_HANG_INDENT, 10).unwrap();
        state.set_int_par(INT_PAR_HANG_AFTER, -1).unwrap();
        state.set_int_par(INT_PAR_PRETOLERANCE, 10000).unwrap();
        state.set_source_text("ab ab");
        dispatch_all(&mut state, 14);

        let lines = break_and_collect(&mut state);
        let boxes: Vec<i32> = lines
            .iter()
            .copied()
            .filter(|&p| state.node_type(p).unwrap() == HLIST_NODE)
            .collect();
        assert_eq!(boxes.len(), 2);
        assert_eq!(state.mem[(boxes[0] + 4) as usize].s1, 10);
        assert_eq!(state.mem[(boxes[1] + 4) as usize].s1, 0);
    }

    #[test]
    fn line_break_second_pass_hyphenates_with_exceptions() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.hyphen_char = i32::from(b'-');
            font.params[crate::state::SPACE_CODE as usize] = 10;
            font.params[crate::state::SPACE_STRETCH_CODE as usize] = 50;
        }
        state.set_dimen_par(DIMEN_PAR_HSIZE, 1000).unwrap();
        state.set_int_par(INT_PAR_PRETOLERANCE, -1).unwrap();
        state.set_int_par(INT_PAR_TOLERANCE, 10000).unwrap();
        state.set_int_par(INT_PAR_LEFT_HYPHEN_MIN, 1).unwrap();
        state.set_int_par(INT_PAR_RIGHT_HYPHEN_MIN, 1).unwrap();
        state
            .hyph_exceptions
            .insert(vec![i32::from(b'a'), i32::from(b'b'), 0], vec![1]);
        state.set_source_text("ab ab");
        dispatch_all(&mut state, 12);

        // The exception "a-b" must leave a discretionary inside the
        // packed line; per C semantics only the word *following* the
        // inter-word glue is hyphenated, so the second "ab" carries it.
        let lines = break_and_collect(&mut state);
        let line = lines
            .iter()
            .copied()
            .find(|&p| state.node_type(p).unwrap() == HLIST_NODE)
            .expect("expected a line box");
        let mut found_disc = false;
        let mut p = state.mem[(line + 5) as usize].s1;
        while p != TEX_NULL {
            if !state.is_char_node(p) && state.node_type(p).unwrap() == DISC_NODE {
                found_disc = true;
                break;
            }
            p = state.link(p).unwrap();
        }
        assert!(found_disc, "hyphenated word must contain a disc node");
    }

    #[test]
    fn line_break_inserts_interline_penalties() {
        let mut state = tfm_engine_with_metrics();
        {
            let font = state.font_table.get_mut(1).unwrap();
            font.params[crate::state::SPACE_STRETCH_CODE as usize] = 50;
        }
        state.set_dimen_par(DIMEN_PAR_HSIZE, 30).unwrap();
        state.set_int_par(INT_PAR_PRETOLERANCE, 10000).unwrap();
        state.set_int_par(INT_PAR_INTER_LINE_PENALTY, 7).unwrap();
        state.set_source_text("ab ab");
        dispatch_all(&mut state, 14);

        let lines = break_and_collect(&mut state);
        let penalties: Vec<i32> = lines
            .iter()
            .copied()
            .filter(|&p| state.node_type(p).unwrap() == PENALTY_NODE)
            .collect();
        assert_eq!(penalties.len(), 1);
        assert_eq!(state.mem[(penalties[0] + 1) as usize].s1, 7);
    }
    // ---- WP-13a: alignment lifecycle and main-loop error paths ----

    #[test]
    fn halign_single_column_packs_row_into_vlist() {
        let mut state = tfm_engine_with_metrics();
        state.set_source_text("\\halign{#\\cr a\\cr}");
        dispatch_all(&mut state, 40);

        let contrib = state.contrib_head();
        let row = state.mem[contrib as usize].s1;
        assert_eq!(
            state.node_type(row).unwrap(),
            HLIST_NODE,
            "halign row should package into an hlist"
        );
        assert!(
            state.error_count == 0,
            "clean halign: {:?}",
            state.transcript
        );
    }

    #[test]
    fn halign_two_columns_repeats_templates() {
        let mut state = tfm_engine_with_metrics();
        state.set_source_text("\\halign{#&#\\cr a&b\\cr}");
        dispatch_all(&mut state, 60);

        // Material appended in vertical mode passes through build_page into
        // the page list (C fin_align -> build_page); find the first box there.
        let ph = state.page_head();
        let mut row = state.mem[ph as usize].s1;
        while row != TEX_NULL
            && state.node_type(row).unwrap() != HLIST_NODE
            && state.node_type(row).unwrap() != VLIST_NODE
        {
            row = state.link(row).unwrap();
        }
        assert_ne!(
            row, TEX_NULL,
            "alignment must append a row to the page list"
        );
        // The packaged row contains: tabskip glue, cell box, tabskip glue,
        // cell box, tabskip glue (C §800/§824).
        let mut cells = 0;
        let mut glues = 0;
        let mut p = state.mem[(row + 5) as usize].s1;
        while p != TEX_NULL {
            match state.node_type(p).unwrap() {
                HLIST_NODE => cells += 1,
                GLUE_NODE => glues += 1,
                _ => {}
            }
            p = state.link(p).unwrap();
        }
        assert_eq!(cells, 2, "two columns must produce two cell boxes");
        assert!(glues >= 3, "tabskip glue must surround the cells");
        assert!(
            state.error_count == 0,
            "clean halign: {:?}",
            state.transcript
        );
    }

    #[test]
    fn halign_omit_bypasses_the_column_template() {
        let mut state = tfm_engine_with_metrics();
        // Wrap the template in \itshape-like markers via a kern so \omit is
        // observable: without \omit the kern would surround the cell.
        state.set_source_text("\\halign{#\\cr \\omit a\\cr}");
        dispatch_all(&mut state, 60);
        assert_eq!(
            state.error_count, 0,
            r"\omit at column start is legal: {:?}",
            state.transcript
        );
        let ph = state.page_head();
        let mut row = state.mem[ph as usize].s1;
        while row != TEX_NULL
            && state.node_type(row).unwrap() != HLIST_NODE
            && state.node_type(row).unwrap() != VLIST_NODE
        {
            row = state.link(row).unwrap();
        }
        assert_eq!(state.node_type(row).unwrap(), HLIST_NODE);
    }

    #[test]
    fn misplaced_cr_reports_alignment_error() {
        let mut state = engine();
        state.set_source_text("\\cr");
        dispatch_all(&mut state, 4);
        assert!(
            state
                .transcript
                .iter()
                .any(|line| line.contains("Misplaced")),
            "expected Misplaced error, got {:?}",
            state.transcript
        );
        assert!(state.error_count > 0);
    }

    #[test]
    fn tab_mark_outside_alignment_inserts_brace() {
        let mut state = engine();
        // `&` in vertical mode is a TabMark token → align_error path.
        state.set_source_text("&x");
        dispatch_all(&mut state, 8);
        assert!(
            state
                .transcript
                .iter()
                .any(|line| line.contains("Misplaced") || line.contains("Missing")),
            "expected alignment error, got {:?}",
            state.transcript
        );
    }

    #[test]
    fn noalign_and_omit_outside_alignment_report_errors() {
        let mut state = engine();
        state.set_source_text("\\noalign{}\\omit x");
        dispatch_all(&mut state, 12);
        let misplaced = state
            .transcript
            .iter()
            .filter(|line| line.contains("Misplaced"))
            .count();
        assert_eq!(misplaced, 2, "both errors expected: {:?}", state.transcript);
    }

    #[test]
    fn stray_endv_runs_off_save() {
        let mut state = tfm_engine_with_metrics();
        // ENDV arriving from a macro inside a braced group within a cell:
        // a V_TEMPLATE input sits below the exhausted macro list, but
        // cur_group is the simple group, so C runs off_save.
        state.set_source_text("\\def\\x{\\cr}\\halign{#\\cr a{\\x}b\\cr}");
        dispatch_all(&mut state, 80);
        assert!(
            state
                .transcript
                .iter()
                .any(|line| line.contains("Missing") && line.contains("inserted")
                    || line.starts_with("! Extra ")),
            "expected off_save diagnostic, got {:?}",
            state.transcript
        );
    }

    #[test]
    fn stray_endgroup_runs_off_save() {
        let mut state = engine();
        state.set_source_text("\\endgroup");
        dispatch_all(&mut state, 8);
        assert!(
            state
                .transcript
                .iter()
                .any(|line| line == "! Extra \\endgroup"),
            "expected off_save diagnostic, got {:?}",
            state.transcript
        );
    }

    #[test]
    fn stray_endcsname_reports_extra() {
        let mut state = engine();
        state.define_cs("endc", CommandCode::EndCsName, 0);
        state.set_source_text("\\endc");
        dispatch_all(&mut state, 8);
        assert!(
            state
                .transcript
                .iter()
                .any(|line| line.contains("Extra \\endcsname")),
            "expected cs_error diagnostic, got {:?}",
            state.transcript
        );
    }

    #[test]
    fn end_stops_the_job_when_lists_are_empty() {
        let mut state = engine();
        state.set_source_text("\\end");
        dispatch_all(&mut state, 8);
        assert!(state.job_stopped, r"\end on empty lists ends the job");
    }

    #[test]
    fn end_forces_pending_material_out() {
        let mut state = tfm_engine_with_metrics();
        state.set_source_text("a\\end");
        dispatch_all(&mut state, 30);
        // Material in the vertical/contribution list prevents termination;
        // its_all_over appends \\vfill + \\null penalty and builds a page.
        assert!(
            !state.job_stopped || !state.shipped_pages.is_empty(),
            r"\end with pending material must force output: transcript {:?}",
            state.transcript
        );
    }

    #[test]
    fn sup_mark_in_vertical_mode_inserts_dollar() {
        let mut state = engine();
        state.set_source_text("^a");
        dispatch_all(&mut state, 8);
        assert!(
            state
                .transcript
                .iter()
                .any(|line| line.contains("Missing $ inserted")),
            "expected insert_dollar_sign diagnostic, got {:?}",
            state.transcript
        );
    }

    #[test]
    fn halign_in_restricted_hmode_runs_off_save() {
        let mut state = tfm_engine_with_metrics();
        // \\hbox puts the engine in restricted hmode; \\halign inside it
        // hits the C `HMODE + HALIGN` → head_for_vmode → off_save path.
        state.set_source_text("\\hbox{\\halign{#\\cr}}");
        dispatch_all(&mut state, 30);
        assert!(
            state.error_count > 0,
            "restricted-hmode \\halign must report an error: {:?}",
            state.transcript
        );
    }

    #[test]
    fn valign_in_hmode_builds_rows() {
        let mut state = tfm_engine_with_metrics();
        // \valign inside a paragraph runs init_align in hmode.
        state.set_source_text("a\\valign{#\\cr b\\cr}");
        dispatch_all(&mut state, 60);
        assert_eq!(
            state.error_count, 0,
            "hmode \\valign is legal: {:?}",
            state.transcript
        );
    }

    /// Engine with a 24-parameter math font installed in every family slot so
    /// `init_math`'s `math_fonts_insufficient` checks pass.
    fn math_engine() -> TeXEngineState {
        let mut state = font_engine();
        let mut record = crate::state::FontRecord {
            params: vec![0; 24],
            bc: 32,
            ec: 126,
            ..Default::default()
        };
        record.params[crate::state::QUAD_CODE as usize] = 1000;
        for i in 7..23 {
            record.params[i] = 10 * i as i32;
        }
        state.register_font_source("math", record);
        state.set_source_text("\\font\\mathf=math ");
        state.dispatch_x_command().unwrap();
        for fam in 0..4i32 {
            for off in [0i32, 256, 512] {
                state
                    .eq_word_define(crate::state::FAMILY_BASE + (fam + off) as usize, 1)
                    .unwrap();
            }
        }
        // Ordinary mathcodes (class 0, fam 0) for the test letters.
        for &ch in b"xab" {
            let word = crate::state::MemoryWord {
                s1: i32::from(ch),
                ..Default::default()
            };
            state.code_tables[crate::state::CODE_TABLE_MATH as usize].insert(u32::from(ch), word);
        }
        for &ch in b"xab2(" {
            state.set_font_char_metric(
                1,
                u32::from(ch),
                crate::state::FontCharMetric {
                    width: 10,
                    height: 7,
                    depth: 0,
                    italic: 2,
                },
            );
        }
        state.set_font_char_metric(
            1,
            0x70,
            crate::state::FontCharMetric {
                width: 10,
                height: 10,
                depth: 0,
                italic: 0,
            },
        );
        // \delcode`( = small variant fam 0 char '('.
        let word = crate::state::MemoryWord {
            s1: 0x28,
            ..Default::default()
        };
        state.code_tables[crate::state::CODE_TABLE_DEL as usize].insert(u32::from(b'('), word);
        state
    }

    /// Collects the node types of the current list (`link(head)…`).
    fn head_node_types(state: &TeXEngineState) -> Vec<(u16, u16)> {
        let mut out = Vec::new();
        let mut p = state.mem[state.head as usize].s1;
        while p != crate::state::TEX_NULL {
            out.push((state.node_type(p).unwrap(), state.node_subtype(p).unwrap()));
            p = state.mem[p as usize].s1;
        }
        out
    }

    #[test]
    fn inline_math_wraps_formula_in_math_nodes() {
        let mut state = math_engine();
        state.set_source_text("$x$");
        dispatch_all(&mut state, 30);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        // VMODE `$` starts a paragraph (\parindent hbox), so the math nodes
        // may follow a leading box; find the MATH(BEFORE)…MATH(AFTER) region.
        let nodes = head_node_types(&state);
        let open = nodes.iter().position(|&n| n == (MATH_NODE, BEFORE as u16));
        let close = nodes.iter().position(|&n| n == (MATH_NODE, AFTER as u16));
        assert!(open.is_some() && close.is_some(), "nodes: {nodes:?}");
        assert!(close.unwrap() > open.unwrap() + 1, "nodes: {nodes:?}");
    }

    #[test]
    fn superscript_converts_noad_to_script_box() {
        let mut state = math_engine();
        state.set_source_text("$x^2$");
        dispatch_all(&mut state, 30);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        let nodes = head_node_types(&state);
        assert!(
            nodes
                .iter()
                .any(|&(t, _)| t == HLIST_NODE || t == VLIST_NODE),
            "scripts produce a boxed nucleus: {nodes:?}"
        );
    }

    #[test]
    fn fraction_produces_vcentered_box() {
        let mut state = math_engine();
        state.set_source_text("${a\\over b}$");
        dispatch_all(&mut state, 40);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        let nodes = head_node_types(&state);
        assert!(
            nodes
                .iter()
                .any(|&(t, _)| t == VLIST_NODE || t == HLIST_NODE),
            "\\over builds a boxed fraction nucleus (hpack'ed vcenter): {nodes:?}"
        );
    }

    #[test]
    fn radical_wraps_contents_in_radical_box() {
        let mut state = math_engine();
        state.set_source_text("$\\radical\"270370{x}$");
        dispatch_all(&mut state, 30);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        let nodes = head_node_types(&state);
        assert!(
            nodes
                .iter()
                .any(|&(t, _)| t == VLIST_NODE || t == HLIST_NODE),
            "\\radical produces a stacked box: {nodes:?}"
        );
    }

    #[test]
    fn left_right_delimiters_convert_to_hlist() {
        let mut state = math_engine();
        state.set_source_text("$\\left. x \\right.$");
        dispatch_all(&mut state, 40);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        let nodes = head_node_types(&state);
        assert!(
            nodes
                .iter()
                .any(|&(t, _)| t == HLIST_NODE || t == VLIST_NODE),
            "\\left..\\right. emits an inner box: {nodes:?}"
        );
    }

    #[test]
    fn mathchoice_selects_the_current_style_branch() {
        let mut state = math_engine();
        state.set_source_text("$\\mathchoice{a}{b}{x}{a}$");
        dispatch_all(&mut state, 40);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        let nodes = head_node_types(&state);
        let open = nodes.iter().position(|&n| n == (MATH_NODE, BEFORE as u16));
        let close = nodes.iter().position(|&n| n == (MATH_NODE, AFTER as u16));
        assert!(open.is_some() && close.is_some(), "nodes: {nodes:?}");
        assert!(close.unwrap() > open.unwrap() + 1, "nodes: {nodes:?}");
    }

    #[test]
    fn display_math_appends_display_list() {
        let mut state = math_engine();
        state.set_source_text("$$x$$");
        dispatch_all(&mut state, 40);
        assert_eq!(state.error_count, 0, "transcript: {:?}", state.transcript);
        // Display math contributes [penalty, skips, display hbox, penalty,
        // skip] to the enclosing vertical list (contrib_head).
        let mut found_box = false;
        let mut p = state.mem[state.contrib_head() as usize].s1;
        while p != crate::state::TEX_NULL {
            if state.node_type(p).unwrap() == HLIST_NODE {
                found_box = true;
            }
            p = state.mem[p as usize].s1;
        }
        assert!(found_box, "display math contributes a display hbox");
    }

    #[test]
    fn over_outside_math_recovers_with_inserted_dollar() {
        let mut state = math_engine();
        state.set_source_text("\\over");
        dispatch_all(&mut state, 30);
        assert!(
            state.error_count > 0 || state.transcript.iter().any(|l| l.contains("Missing")),
            "\\over in vmode must report the missing math shift: {:?}",
            state.transcript
        );
    }

    #[test]
    fn show_node_list_displays_math_noad_and_subsidiaries() {
        let mut state = engine();
        let p = state.new_noad().unwrap();
        state.set_node_type(p, OP_NOAD).unwrap();
        state.set_node_subtype(p, LIMITS).unwrap();
        let nucleus = (p + 1) as usize;
        state.mem[nucleus].s1 = MATH_CHAR;
        state.mem[nucleus].set_b16_s0(u16::from(b'x'));
        state.mem[nucleus].set_b16_s1(0);
        let sup = (p + 2) as usize;
        state.mem[sup].s1 = SUB_MLIST;
        state.mem[sup].s0 = crate::state::TEX_NULL;

        state.show_node_list(p, "", 3, 10).unwrap();

        assert_eq!(
            state.transcript,
            vec![
                "\\mathop\\limits".to_string(),
                ".\\fam0 x".to_string(),
                "^{}".to_string(),
            ],
            "transcript: {:?}",
            state.transcript
        );
    }

    #[test]
    fn show_node_list_displays_fraction_and_delimiters() {
        let mut state = engine();
        let p = state.get_node(crate::state::FRACTION_NOAD_SIZE).unwrap();
        state.set_node_type(p, FRACTION_NOAD).unwrap();
        state.mem[(p + 1) as usize].s1 = DEFAULT_CODE;
        let num = (p + 2) as usize;
        state.mem[num].s1 = MATH_CHAR;
        state.mem[num].set_b16_s0(u16::from(b'a'));
        let den = (p + 3) as usize;
        state.mem[den].s1 = MATH_FIELD_EMPTY;
        let delim = (p + 4) as usize;
        state.mem[delim].set_b16_s0(0x41);
        state.mem[delim].set_b16_s1(0);
        state.mem[delim].set_b16_s2(0);
        state.mem[delim].set_b16_s3(0);

        state.show_node_list(p, "", 3, 10).unwrap();

        assert_eq!(
            state.transcript,
            vec![
                "\\fraction, thickness = default, left-delimiter 41".to_string(),
                "\\\\fam0 a".to_string(),
            ],
            "transcript: {:?}",
            state.transcript
        );
    }

    #[test]
    fn show_node_list_displays_style_and_choice() {
        let mut state = engine();
        let style = state.new_style(DISPLAY_STYLE).unwrap();
        let choice = state.new_choice().unwrap();
        state.mem[style as usize].s1 = choice;

        state.show_node_list(style, "", 3, 10).unwrap();

        assert_eq!(
            state.transcript,
            vec!["\\displaystyle".to_string(), "\\mathchoice".to_string()],
            "transcript: {:?}",
            state.transcript
        );
    }

    #[test]
    fn show_node_list_displays_choice_branches_with_markers() {
        let mut state = engine();
        let choice = state.new_choice().unwrap();
        let noad = state.new_noad().unwrap();
        state.mem[(choice + 1) as usize].s0 = noad;

        state.show_node_list(choice, "", 3, 10).unwrap();

        assert_eq!(
            state.transcript,
            vec!["\\mathchoice".to_string(), "D\\mathord".to_string()],
            "transcript: {:?}",
            state.transcript
        );
    }

    #[test]
    fn node_line_displays_ligature_with_short_display() {
        let mut state = engine();
        let inner = state.get_avail().unwrap();
        state.mem[inner as usize].set_b16_s0(u16::from(b'i'));
        state.mem[inner as usize].set_b16_s1(99);
        state.mem[inner as usize].s1 = crate::state::TEX_NULL;
        let lig = state.new_ligature(5, i32::from(b'f'), inner).unwrap();
        state.set_node_subtype(lig, 3).unwrap();

        let line = state.node_line(lig, "", 3).unwrap();

        assert_eq!(line, "* f (ligature |* i|)");
    }

    #[test]
    fn char_node_line_shows_font_and_char() {
        let mut state = engine();
        let p = state.get_avail().unwrap();
        state.mem[p as usize].set_b16_s0(u16::from(b'x'));
        state.mem[p as usize].set_b16_s1(7);
        state.mem[p as usize].s1 = crate::state::TEX_NULL;

        let line = state.node_line(p, ".", 3).unwrap();

        assert_eq!(line, ".* x");
    }

    #[test]
    fn subsidiary_overflow_marks_nonempty_fields_inline() {
        let mut state = engine();
        let p = state.new_noad().unwrap();
        let nucleus = (p + 1) as usize;
        state.mem[nucleus].s1 = MATH_CHAR;
        state.mem[nucleus].set_b16_s0(u16::from(b'y'));
        let sub = (p + 3) as usize;
        state.mem[sub].s1 = SUB_MLIST;
        state.mem[sub].s0 = crate::state::TEX_NULL;

        let line = state.node_line(p, "", 0).unwrap();

        assert_eq!(line, "\\mathord [] []");
    }

    #[test]
    fn page_totals_text_includes_glue_components() {
        let mut state = engine();
        state.page_so_far[1] = 10 * 65536;
        state.page_so_far[2] = 2 * 65536;
        state.page_so_far[3] = 65536;
        state.page_so_far[4] = 3 * 65536;
        state.page_so_far[6] = 65536;

        assert_eq!(
            state.page_totals_text(),
            "10.0 plus 2.0 plus 1.0fil plus 3.0fill minus 1.0"
        );
    }
}
