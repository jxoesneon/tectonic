// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust TeX quantity scanning: integers, keywords, and bounded numbers.
//!
//! Fulfills Phase 5 Milestone 3 packet 1 in `MIGRATION_ROADMAP.md`:
//! - Ports `scan_left_brace`, `scan_optional_equals`, and `scan_keyword` from
//!   `xetex-xetex0.c` (C ~6500-6591), including the backup token-list
//!   restoration semantics used when a keyword partially matches.
//! - Ports `scan_int` (C ~8071): optional `+`/`-` signs, backtick alphabetic
//!   constants, `'` octal and `"` hexadecimal radixes, `TEX_INFINITY` overflow
//!   saturation, and vacuous-input recovery through `back_input`.
//! - Ports the bounded wrappers: `scan_eight_bit_int`, `scan_usv_num`,
//!   `scan_char_num`, `scan_four_bit_int`, `scan_fifteen_bit_int`,
//!   `scan_math_class_int`, `scan_math_fam_int`, `scan_delimiter_int`,
//!   `scan_register_num`, and `scan_four_bit_int_or_18`.
//! - Provides `get_x_or_protected` and a `scan_something_internal` scaffold
//!   covering the register-backed internal quantities that the current
//!   [`TeXEngineState`] can represent.
//! - Ports `round_decimals`, `xetex_scan_dimen`, `scan_dimen`, `scan_decimal`,
//!   `scan_glue`, `scan_normal_glue`, and `scan_mu_glue` (C ~8221-8585,
//!   ~8969) for Phase 5 Milestone 3 packet 2: optional signs, decimal
//!   fractions, all physical units, the `fil`/`fill`/`filll` infinite
//!   orders, `mu` units, internal quantity coercion, and `MAX_HALFWORD`
//!   overflow recovery.
//! - Ports `add_or_sub`, `quotient`, `fract`, and `scan_expr` (C ~8588-8967)
//!   for Phase 5 Milestone 3 packet 3: the e-TeX `\numexpr`, `\dimexpr`,
//!   `\glueexpr`, and `\muexpr` scanners with parentheses, `+`/`-`/`*`/`/`
//!   operators, the fused multiply-then-divide (`EXPR_SCALE`) rounding, and
//!   glue width/stretch/shrink/order arithmetic. The `LastItem` dispatch of
//!   `scan_something_internal` covers the e-TeX glue conversions and
//!   expression codes 65-70 (C ~7469-7513).
//!
//! Deliberate divergences from the C implementation, deferred to later
//! milestone packets:
//! - TeX's interactive error machinery (`error`, `back_error`, `int_error`)
//!   has no diagnostic channel yet. Missing numbers and missing left braces
//!   follow the C post-error recovery path silently; out-of-range bounded
//!   scans report [`TeXMouthError::InputOutOfBounds`] instead of zeroing the
//!   value through `int_error`. The same convention covers `mu_error`
//!   ("Incompatible glue units"), "Illegal unit of measure" (pt/mu/filll
//!   substituted), and "Dimension too large" (clamped to `MAX_HALFWORD`).
//! - `\primitive` re-lookup inside numbers returns
//!   [`TeXMouthError::UnsupportedPrimitive`]. Non-register internal
//!   quantities are otherwise implemented: `\lastpenalty`, `\lastkern`,
//!   `\lastskip`, `\lastnodetype`, the scalar `LastItem` codes
//!   (`\inputlineno` through `\XeTeXpdfpagecount`), font integers,
//!   `\fontcharwd`-family metrics, `\XeTeXglyphbounds`, parshape elements,
//!   and glue stretch/shrink queries. Layout-engine results (AAT/Graphite/
//!   OpenType feature, variation, and script queries) resolve through
//!   registered per-state `font_engine_values`/`font_engine_named_values`
//!   tables; fonts without an engine take the C `not_*_font_error`
//!   recovery path. The `scan_expr` recoverable diagnostics follow the
//!   same convention:
//!   "Missing ) inserted for expression" pushes the offending token back
//!   and closes the parenthesis level, and "Arithmetic overflow" silently
//!   substitutes zero (zero glue for spec results).
//! - `scan_expr`'s parenthesized-subexpression stack is a local Rust `Vec`
//!   of frames rather than the `expr_node`s that C allocates in dynamic
//!   memory; no engine state escapes the routine.
//! - `true` units cancel magnification exactly as C §476: `prepare_mag`
//!   locks `\mag`, then the scanned value and its fraction are scaled by
//!   `1000/mag` so `1 true in` is a physical inch at any `\magnification`.
//! - `scan_something_internal` covers registers, box dimensions, character
//!   code tables, mode/page/shape quantities, and the implemented e-TeX
//!   `LastItem` codes. The integer,
//!   dimension, and glue scanners append the C epilogue (level coercion,
//!   glue-spec negation, reference counting) explicitly through
//!   `finish_scan_something_internal`.

use crate::scanner::{
    TeXMouthError, BACKED_UP, BIGGEST_USV, CS_TOKEN_FLAG, LEFT_BRACE_TOKEN, MAX_CHAR_VAL,
    MAX_COMMAND, PROTECTED_TOKEN, TOO_BIG_USV,
};
use crate::state::{
    CommandCode, TeXEngineState, TeXMode, CAT_CODE_BASE, CHAR_CLASS_LIMIT, CUR_FONT_LOC,
    DEL_CODE_BASE, FIL, FILLL, HYPHEN_CHAR_CODE, INT_PAR_MAG, LC_CODE_BASE,
    LOCAL_BASE, LOCAL_PAR_SHAPE, LOCAL_XETEX_INTER_CHAR_TOKS, LP_CODE_BASE, MATH_CODE_BASE,
    MAX_HALFWORD, MU_GLUE, NORMAL, PAGE_EMPTY, PAGE_SO_FAR_COUNT, QUAD_CODE,
    SA_BOX_VAL, SA_INTER_CHAR_VAL, SA_TOK_VAL, SF_CODE_BASE, SKEW_CHAR_CODE, SPACE_CODE,
    SPACE_SHRINK_CODE, TEX_NULL, UC_CODE_BASE, X_HEIGHT_CODE,
};

/// Base token of the catcode-11 (letter) command block (`LETTER_TOKEN`).
pub const LETTER_TOKEN: i32 = 11 * MAX_CHAR_VAL;
/// Base token of the catcode-12 (other char) command block (`OTHER_TOKEN`).
pub const OTHER_TOKEN: i32 = 12 * MAX_CHAR_VAL;
/// Token for `0` (`ZERO_TOKEN`).
pub const ZERO_TOKEN: i32 = OTHER_TOKEN + b'0' as i32;
/// Token for a letter `A` (`A_TOKEN`).
pub const A_TOKEN: i32 = LETTER_TOKEN + b'A' as i32;
/// Token for an other-char `A` (`OTHER_A_TOKEN`).
pub const OTHER_A_TOKEN: i32 = OTHER_TOKEN + b'A' as i32;
/// Token for `'` starting an octal constant (`OCTAL_TOKEN`).
pub const OCTAL_TOKEN: i32 = OTHER_TOKEN + b'\'' as i32;
/// Token for `"` starting a hexadecimal constant (`HEX_TOKEN`).
pub const HEX_TOKEN: i32 = OTHER_TOKEN + b'"' as i32;
/// Token for `` ` `` starting an alphabetic constant (`ALPHA_TOKEN`).
pub const ALPHA_TOKEN: i32 = OTHER_TOKEN + b'`' as i32;

/// Largest positive integer TeX accepts (`TEX_INFINITY`).
pub const TEX_INFINITY: i32 = 0x7FFF_FFFF;
/// Largest legacy 16-bit character code (`BIGGEST_CHAR`).
pub const BIGGEST_CHAR: i32 = 0xFFFF;
/// XeTeX's encoded active math character (`ACTIVE_MATH_CHAR`).
pub const ACTIVE_MATH_CHAR: i32 = 0x1F_FFFF;
/// Mask selecting the character payload from a XeTeX math code (`math_char`).
const MATH_CHAR_MASK: u32 = 0x1F_FFFF;
/// Number of math families supported by XeTeX (`NUMBER_MATH_FAMILIES`).
pub const NUMBER_MATH_FAMILIES: i32 = 256;
/// Largest register number accepted by TeX and e-TeX (`max_reg_num`).
///
/// Registers `0..=255` use dense `eqtb` slots; registers `256..=32767`
/// use sparse `sa` elements.
pub const MAX_REGISTER_NUM: i32 = 32_767;

/// `cur_val_level` for plain integers (`INT_VAL`).
pub const INT_VAL: u8 = 0;
/// `cur_val_level` for scaled dimensions (`DIMEN_VAL`).
pub const DIMEN_VAL: u8 = 1;
/// `cur_val_level` for glue specifications (`GLUE_VAL`).
pub const GLUE_VAL: u8 = 2;
/// `cur_val_level` for mu-glue specifications (`MU_VAL`).
pub const MU_VAL: u8 = 3;
/// `cur_val_level` for font identifiers (`IDENT_VAL`).
pub const IDENT_VAL: u8 = 4;
/// `cur_val_level` for token lists (`TOK_VAL`).
pub const TOK_VAL: u8 = 5;

/// `LastItem` subcommand code for `\mutoglue` (`ETEX_GLUE`/`MU_TO_GLUE_CODE`).
pub const ETEX_GLUE: i32 = 65;
/// `LastItem` subcommand code for `\gluetomu` (`ETEX_MU`/`GLUE_TO_MU_CODE`).
pub const ETEX_MU: i32 = 66;
/// `LastItem` subcommand code for `\mutoglue` (`MU_TO_GLUE_CODE`).
pub const MU_TO_GLUE_CODE: i32 = ETEX_GLUE;
/// `LastItem` subcommand code for `\gluetomu` (`GLUE_TO_MU_CODE`).
pub const GLUE_TO_MU_CODE: i32 = ETEX_MU;
/// First `LastItem` subcommand code for an e-TeX expression (`ETEX_EXPR`,
/// the `\numexpr` code).
pub const ETEX_EXPR: i32 = 67;
/// Last `LastItem` subcommand code for an e-TeX expression (the `\muexpr`
/// code).
pub const ETEX_EXPR_MU: i32 = ETEX_EXPR + MU_VAL as i32;

/// `LastItem` subcommand code for `\lastnodetype` (`LAST_NODE_TYPE_CODE`).
pub const LAST_NODE_TYPE_CODE: i32 = 3;
/// `LastItem` subcommand code for `\inputlineno` (`INPUT_LINE_NO_CODE`).
pub const INPUT_LINE_NO_CODE: i32 = 4;
/// `LastItem` subcommand code for `\badness` (`BADNESS_CODE`).
pub const BADNESS_CODE: i32 = 5;
/// `LastItem` subcommand code for `\pdflastxpos` (`PDF_LAST_X_POS_CODE`).
pub const PDF_LAST_X_POS_CODE: i32 = 12;
/// `LastItem` subcommand code for `\pdflastypos` (`PDF_LAST_Y_POS_CODE`).
pub const PDF_LAST_Y_POS_CODE: i32 = 13;
/// `LastItem` subcommand code for `\elapsedtime` (`ELAPSED_TIME_CODE`).
pub const ELAPSED_TIME_CODE: i32 = 16;
/// `LastItem` subcommand code for `\shellescape` (`PDF_SHELL_ESCAPE_CODE`).
pub const PDF_SHELL_ESCAPE_CODE: i32 = 17;
/// `LastItem` subcommand code for `\randomseed` (`RANDOM_SEED_CODE`).
pub const RANDOM_SEED_CODE: i32 = 18;
/// First e-TeX integer `LastItem` code (`ETEX_INT`).
pub const ETEX_INT: i32 = 19;
/// `LastItem` subcommand code for `\eTeXversion` (`ETEX_VERSION_CODE`).
pub const ETEX_VERSION_CODE: i32 = ETEX_INT;
/// `LastItem` subcommand code for `\currentgrouplevel`.
pub const CURRENT_GROUP_LEVEL_CODE: i32 = 20;
/// `LastItem` subcommand code for `\currentgrouptype`.
pub const CURRENT_GROUP_TYPE_CODE: i32 = 21;
/// `LastItem` subcommand code for `\currentiflevel`.
pub const CURRENT_IF_LEVEL_CODE: i32 = 22;
/// `LastItem` subcommand code for `\currentiftype`.
pub const CURRENT_IF_TYPE_CODE: i32 = 23;
/// `LastItem` subcommand code for `\currentifbranch`.
pub const CURRENT_IF_BRANCH_CODE: i32 = 24;
/// `LastItem` subcommand code for `\gluestretchorder`.
pub const GLUE_STRETCH_ORDER_CODE: i32 = 25;
/// `LastItem` subcommand code for `\glueshrinkorder`.
pub const GLUE_SHRINK_ORDER_CODE: i32 = 26;
/// First XeTeX integer `LastItem` code (`XETEX_INT`).
pub const XETEX_INT: i32 = 27;
/// `LastItem` subcommand code for `\XeTeXversion` (`XETEX_VERSION_CODE`).
pub const XETEX_VERSION_CODE: i32 = XETEX_INT;
/// `LastItem` subcommand code for `\XeTeXcountglyphs`.
pub const XETEX_COUNT_GLYPHS_CODE: i32 = 28;
/// `LastItem` subcommand code for `\XeTeXcountvariations`.
pub const XETEX_COUNT_VARIATIONS_CODE: i32 = 29;
/// `LastItem` subcommand code for `\XeTeXvariation`.
pub const XETEX_VARIATION_CODE: i32 = 30;
/// `LastItem` subcommand code for `\XeTeXfindvariationbyname`.
pub const XETEX_FIND_VARIATION_BY_NAME_CODE: i32 = 31;
/// `LastItem` subcommand code for `\XeTeXvariationmin`.
pub const XETEX_VARIATION_MIN_CODE: i32 = 32;
/// `LastItem` subcommand code for `\XeTeXvariationmax`.
pub const XETEX_VARIATION_MAX_CODE: i32 = 33;
/// `LastItem` subcommand code for `\XeTeXvariationdefault`.
pub const XETEX_VARIATION_DEFAULT_CODE: i32 = 34;
/// `LastItem` subcommand code for `\XeTeXcountfeatures`.
pub const XETEX_COUNT_FEATURES_CODE: i32 = 35;
/// `LastItem` subcommand code for `\XeTeXfeaturecode`.
pub const XETEX_FEATURE_CODE_CODE: i32 = 36;
/// `LastItem` subcommand code for `\XeTeXfindfeaturebyname`.
pub const XETEX_FIND_FEATURE_BY_NAME_CODE: i32 = 37;
/// `LastItem` subcommand code for `\XeTeXisexclusivefeature`.
pub const XETEX_IS_EXCLUSIVE_FEATURE_CODE: i32 = 38;
/// `LastItem` subcommand code for `\XeTeXcountselectors`.
pub const XETEX_COUNT_SELECTORS_CODE: i32 = 39;
/// `LastItem` subcommand code for `\XeTeXselectorcode`.
pub const XETEX_SELECTOR_CODE_CODE: i32 = 40;
/// `LastItem` subcommand code for `\XeTeXfindselectorbyname`.
pub const XETEX_FIND_SELECTOR_BY_NAME_CODE: i32 = 41;
/// `LastItem` subcommand code for `\XeTeXisdefaultselector`.
pub const XETEX_IS_DEFAULT_SELECTOR_CODE: i32 = 42;
/// `LastItem` subcommand code for `\XeTeXOTcountscripts`.
pub const XETEX_OT_COUNT_SCRIPTS_CODE: i32 = 43;
/// `LastItem` subcommand code for `\XeTeXOTcountlanguages`.
pub const XETEX_OT_COUNT_LANGUAGES_CODE: i32 = 44;
/// `LastItem` subcommand code for `\XeTeXOTcountfeatures`.
pub const XETEX_OT_COUNT_FEATURES_CODE: i32 = 45;
/// `LastItem` subcommand code for `\XeTeXOTscripttag`.
pub const XETEX_OT_SCRIPT_CODE: i32 = 46;
/// `LastItem` subcommand code for `\XeTeXOTlanguagetag`.
pub const XETEX_OT_LANGUAGE_CODE: i32 = 47;
/// `LastItem` subcommand code for `\XeTeXOTfeaturetag`.
pub const XETEX_OT_FEATURE_CODE: i32 = 48;
/// `LastItem` subcommand code for `\XeTeXcharglyph`.
pub const XETEX_MAP_CHAR_TO_GLYPH_CODE: i32 = 49;
/// `LastItem` subcommand code for `\XeTeXglyphindex`.
pub const XETEX_GLYPH_INDEX_CODE: i32 = 50;
/// `LastItem` subcommand code for `\XeTeXfonttype`.
pub const XETEX_FONT_TYPE_CODE: i32 = 51;
/// `LastItem` subcommand code for `\XeTeXfirstfontchar`.
pub const XETEX_FIRST_CHAR_CODE: i32 = 52;
/// `LastItem` subcommand code for `\XeTeXlastfontchar`.
pub const XETEX_LAST_CHAR_CODE: i32 = 53;
/// `LastItem` subcommand code for `\XeTeXpdfpagecount`.
pub const XETEX_PDF_PAGE_COUNT_CODE: i32 = 54;
/// First XeTeX dimension `LastItem` code (`XETEX_DIM`).
pub const XETEX_DIM: i32 = 55;
/// `LastItem` subcommand code for `\XeTeXglyphbounds`.
pub const XETEX_GLYPH_BOUNDS_CODE: i32 = XETEX_DIM;
/// `LastItem` subcommand code for `\fontcharwd`.
pub const FONT_CHAR_WD_CODE: i32 = 56;
/// `LastItem` subcommand code for `\fontcharht`.
pub const FONT_CHAR_HT_CODE: i32 = 57;
/// `LastItem` subcommand code for `\fontchardp`.
pub const FONT_CHAR_DP_CODE: i32 = 58;
/// `LastItem` subcommand code for `\fontcharic`.
pub const FONT_CHAR_IC_CODE: i32 = 59;
/// `LastItem` subcommand code for `\parshapelength`.
pub const PAR_SHAPE_LENGTH_CODE: i32 = 60;
/// `LastItem` subcommand code for `\parshapeindent`.
pub const PAR_SHAPE_INDENT_CODE: i32 = 61;
/// `LastItem` subcommand code for `\parshapedimen`.
pub const PAR_SHAPE_DIMEN_CODE: i32 = 62;
/// `LastItem` subcommand code for `\gluestretch`.
pub const GLUE_STRETCH_CODE: i32 = 63;
/// `LastItem` subcommand code for `\glueshrink`.
pub const GLUE_SHRINK_CODE: i32 = 64;

/// `\eTeXversion` result (`ETEX_VERSION`).
pub const ETEX_VERSION: i32 = 2;
/// `\XeTeXversion` result (`XETEX_VERSION`).
pub const XETEX_VERSION: i32 = 0;

/// `if_limit` code after `\fi` (`FI_CODE`).
const FI_CODE_LIMIT: u8 = 2;
/// `if_limit` code after `\else` (`ELSE_CODE`).
const ELSE_CODE_LIMIT: u8 = 3;
/// `if_limit` code after `\or` inside `\ifcase` (`OR_CODE`).
const OR_CODE_LIMIT: u8 = 4;
/// First `\unless`-prefixed conditional chr (`UNLESS_CODE`).
const UNLESS_CODE: i32 = 32;

/// No pending e-TeX expression operator (`EXPR_NONE`).
const EXPR_NONE: u8 = 0;
/// Pending e-TeX addition (`EXPR_ADD`).
const EXPR_ADD: u8 = 1;
/// Pending e-TeX subtraction (`EXPR_SUB`).
const EXPR_SUB: u8 = 2;
/// Pending e-TeX multiplication (`EXPR_MULT`).
const EXPR_MULT: u8 = 3;
/// Pending e-TeX division (`EXPR_DIV`).
const EXPR_DIV: u8 = 4;
/// Pending e-TeX fused multiply-then-divide (`EXPR_SCALE`).
const EXPR_SCALE: u8 = 5;

/// First command code denoting an internal quantity (`MIN_INTERNAL`).
pub const MIN_INTERNAL: u8 = CommandCode::CharGiven as u8;
/// Last command code denoting an internal quantity (`MAX_INTERNAL`).
pub const MAX_INTERNAL: u8 = CommandCode::Register as u8;

const OTHER_PLUS: i32 = OTHER_TOKEN + b'+' as i32;
const OTHER_MINUS: i32 = OTHER_TOKEN + b'-' as i32;
const OTHER_EQUALS: i32 = OTHER_TOKEN + b'=' as i32;
/// Token for `.` introducing a decimal fraction (`POINT_TOKEN`).
const POINT_TOKEN: i32 = OTHER_TOKEN + b'.' as i32;
/// Token for `,` accepted as a decimal point (`CONTINENTAL_POINT_TOKEN`).
const CONTINENTAL_POINT_TOKEN: i32 = OTHER_TOKEN + b',' as i32;
/// Token for `(` opening an e-TeX subexpression.
const OTHER_LEFT_PAREN: i32 = OTHER_TOKEN + b'(' as i32;
/// Token for `)` closing an e-TeX subexpression.
const OTHER_RIGHT_PAREN: i32 = OTHER_TOKEN + b')' as i32;
/// Token for `*` multiplying e-TeX operands.
const OTHER_TIMES: i32 = OTHER_TOKEN + b'*' as i32;
/// Token for `/` dividing e-TeX operands.
const OTHER_SLASH: i32 = OTHER_TOKEN + b'/' as i32;

/// Saved state of one parenthesized subexpression level in
/// [`TeXEngineState::scan_expr`].
///
/// The C engine stores this frame in an `expr_node` allocated from dynamic
/// memory (`EXPR_NODE_SIZE` words, with `4 * s + r` packed into
/// `mem[q].b16.s0`); a Rust `Vec` models the same stack without touching
/// node memory.
struct ExprFrame {
    /// Expression level of the enclosing expression (C `l`).
    level: u8,
    /// Pending factor operator (C `s`: `EXPR_NONE`, `EXPR_MULT`,
    /// `EXPR_DIV`, or `EXPR_SCALE`).
    factor_op: u8,
    /// Pending term operator (C `r`: `EXPR_NONE`, `EXPR_ADD`, or `EXPR_SUB`).
    term_op: u8,
    /// Accumulated term value (C `e`).
    term: i32,
    /// Accumulated factor value (C `t`).
    factor: i32,
    /// Saved numerator of a pending `EXPR_SCALE` operation (C `n`).
    scale_numerator: i32,
}

impl TeXEngineState {
    /// Scans a mandatory left brace, skipping spaces and `\relax`
    /// (`scan_left_brace`, C ~6500).
    ///
    /// If the next non-space, non-`\relax` token is not a left brace, the C
    /// engine reports "Missing { inserted" and recovers by pushing the
    /// offending token back and fabricating a `{`. Until the diagnostic
    /// channel exists, this port performs the same recovery silently.
    pub fn scan_left_brace(&mut self) -> Result<(), TeXMouthError> {
        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 && self.cur_cmd != CommandCode::Relax as u8
            {
                break;
            }
        }

        if self.cur_cmd != CommandCode::LeftBrace as u8 {
            // Post-error recovery path: push the offending token back and
            // fabricate the mandatory left brace.
            self.back_input()?;
            self.cur_tok = LEFT_BRACE_TOKEN + b'{' as i32;
            self.cur_cmd = CommandCode::LeftBrace as u8;
            self.cur_chr = b'{' as i32;
            self.align_state += 1;
        }
        Ok(())
    }

    /// Scans an optional `=` sign after skipping spaces
    /// (`scan_optional_equals`, C ~6525).
    pub fn scan_optional_equals(&mut self) -> Result<(), TeXMouthError> {
        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 {
                break;
            }
        }
        if self.cur_tok != OTHER_EQUALS {
            self.back_input()?;
        }
        Ok(())
    }

    /// Scans an optional keyword, matching letters case-insensitively
    /// (`scan_keyword`, C ~6536).
    ///
    /// Returns `true` when the keyword was consumed. On a mismatch the
    /// offending token is pushed back and any partially matched prefix is
    /// restored to the input as a `BACKED_UP` token list, exactly as the C
    /// implementation does through `BACKUP_HEAD`. (The prefix is collected
    /// in a scratch vector rather than the shared `backup_head` cell so the
    /// routine cannot disturb tokens already stored in `mem`.)
    ///
    /// Matching follows the C comparison `cur_chr == c || cur_chr == c - 32`,
    /// so uppercase input letters match lowercase keyword characters.
    pub fn scan_keyword(&mut self, s: &str) -> Result<bool, TeXMouthError> {
        let single_character = s.chars().count() == 1;
        let save_cur_cs = self.cur_cs;
        let mut matched: Vec<i32> = Vec::new();

        for c in s.chars().map(|c| c as i32) {
            loop {
                self.get_x_token()?;
                if self.cur_cs == 0 && (self.cur_chr == c || self.cur_chr == c - 32) {
                    matched.push(self.cur_tok);
                    break;
                }
                if self.cur_cmd != CommandCode::Spacer as u8 || !matched.is_empty() {
                    self.back_input()?;
                    if !matched.is_empty() {
                        let head = self.store_token_list(&matched)?;
                        self.begin_token_list(head, BACKED_UP)?;
                    }
                    if single_character {
                        self.cur_cs = save_cur_cs;
                    }
                    return Ok(false);
                }
                // A space before any matched character is skipped, matching C.
            }
        }

        Ok(true)
    }

    /// Reads one non-expandable token, but does not expand `\protected`
    /// macros (`get_x_or_protected`, C ~7001).
    pub fn get_x_or_protected(&mut self) -> Result<(), TeXMouthError> {
        loop {
            self.get_token()?;
            if self.cur_cmd <= MAX_COMMAND {
                return Ok(());
            }
            if (CommandCode::Call as u8..CommandCode::EndTemplate as u8).contains(&self.cur_cmd) {
                let definition = self.mem[self.checked_engine_mem_index(self.cur_chr)?].s1;
                let first = self.mem[self.checked_engine_mem_index(definition)?].s0;
                if first == PROTECTED_TOKEN {
                    return Ok(());
                }
            }
            self.expand()?;
        }
    }

    /// Scans an integer, possibly signed, radix-prefixed, an alphabetic
    /// constant, or an internal quantity (`scan_int`, C ~8071).
    ///
    /// After this routine [`TeXEngineState::cur_val`] holds the result and
    /// [`TeXEngineState::radix`] records the radix actually used (0 for
    /// alphabetic constants and internal quantities).
    pub fn scan_int(&mut self) -> Result<i32, TeXMouthError> {
        self.radix = 0;
        let mut ok_so_far = true;
        let mut negative = false;

        loop {
            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8 {
                    break;
                }
            }
            if self.cur_tok == OTHER_MINUS {
                negative = !negative;
                self.cur_tok = OTHER_PLUS;
            }
            if self.cur_tok != OTHER_PLUS {
                break;
            }
        }

        loop {
            if self.cur_tok == ALPHA_TOKEN {
                self.get_token()?;

                if self.cur_tok < CS_TOKEN_FLAG {
                    self.cur_val = self.cur_chr;
                    if self.cur_cmd <= CommandCode::RightBrace as u8 {
                        if self.cur_cmd == CommandCode::RightBrace as u8 {
                            self.align_state += 1;
                        } else {
                            self.align_state -= 1;
                        }
                    }
                } else {
                    self.cur_val = self.alphabetic_constant(self.cur_tok - CS_TOKEN_FLAG);
                }

                if self.cur_val > BIGGEST_USV || self.cur_val < 0 {
                    // "Improper alphabetic constant": the C engine reports an
                    // error and recovers with the value of `0`, pushing the
                    // offending token back for re-reading (`back_error`).
                    self.back_input()?;
                    self.cur_val = b'0' as i32;
                } else {
                    self.get_x_token()?;
                    if self.cur_cmd != CommandCode::Spacer as u8 {
                        self.back_input()?;
                    }
                }
                self.cur_val_level = INT_VAL;
            } else if self.cur_tok == CS_TOKEN_FLAG + crate::scanner::FROZEN_PRIMITIVE_ID {
                self.get_token()?;
                if let Some((command, character)) = self.prim_lookup_cs(self.cur_cs) {
                    self.cur_cmd = command;
                    self.cur_chr = character;
                    self.cur_cs = 0;
                    self.cur_tok = command as i32 * MAX_CHAR_VAL + character;
                } else {
                    self.cur_cmd = CommandCode::Relax as u8;
                    self.cur_chr = 0;
                    self.cur_cs = crate::scanner::FROZEN_RELAX_ID;
                    self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
                }
                continue;
            } else if (MIN_INTERNAL..=MAX_INTERNAL).contains(&self.cur_cmd) {
                // Register-backed quantities run the generic C epilogue here;
                // e-TeX `LastItem` results return false because they already
                // applied their owned-result epilogue internally.
                if self.scan_something_internal(INT_VAL, false)? {
                    self.finish_scan_something_internal(INT_VAL, false)?;
                }
            } else {
                self.radix = 10;
                let mut m = 0x0CCC_CCCC;

                if self.cur_tok == OCTAL_TOKEN {
                    self.radix = 8;
                    m = 0x1000_0000;
                    self.get_x_token()?;
                } else if self.cur_tok == HEX_TOKEN {
                    self.radix = 16;
                    m = 0x0800_0000;
                    self.get_x_token()?;
                }

                let mut vacuous = true;
                self.cur_val = 0;

                loop {
                    let d = if self.cur_tok >= ZERO_TOKEN
                        && self.cur_tok <= ZERO_TOKEN + 9
                        && self.cur_tok < ZERO_TOKEN + i32::from(self.radix)
                    {
                        self.cur_tok - ZERO_TOKEN
                    } else if self.radix == 16 {
                        if self.cur_tok >= A_TOKEN && self.cur_tok <= A_TOKEN + 5 {
                            self.cur_tok - A_TOKEN + 10
                        } else if self.cur_tok >= OTHER_A_TOKEN && self.cur_tok <= OTHER_A_TOKEN + 5
                        {
                            self.cur_tok - OTHER_A_TOKEN + 10
                        } else {
                            break;
                        }
                    } else {
                        break;
                    };

                    vacuous = false;

                    if self.cur_val >= m && (self.cur_val > m || d > 7 || self.radix != 10) {
                        if ok_so_far {
                            // "Number too big": saturate like the C engine and
                            // keep absorbing digits without further complaint.
                            self.cur_val = TEX_INFINITY;
                            ok_so_far = false;
                        }
                    } else {
                        self.cur_val = self.cur_val * i32::from(self.radix) + d;
                    }

                    self.get_x_token()?;
                }

                if vacuous {
                    // "Missing number, treated as zero": push the offending
                    // token back and proceed with the zero already in cur_val.
                    self.back_input()?;
                } else if self.cur_cmd != CommandCode::Spacer as u8 {
                    self.back_input()?;
                }
                self.cur_val_level = INT_VAL;
            }
            break;
        }

        if negative {
            self.cur_val = -self.cur_val;
        }
        Ok(self.cur_val)
    }

    /// Reads an internal quantity into `cur_val`/`cur_val_level`
    /// (`scan_something_internal`, C ~7112; register-backed subset plus the
    /// e-TeX `LastItem` codes).
    ///
    /// Supported: `CharGiven`/`MathGiven` character values, `\count`-style
    /// `AssignInt`, `\dimen`-style `AssignDimen`, `AssignGlue`,
    /// `AssignMuGlue`, `AssignToks`/`ToksRegister`, `AssignFontDimen`,
    /// `AssignFontInt`, font-valued `DefFamily`/`SetFont`/`DefFont`
    /// idents, the generic `Register` command with `cur_chr` selecting
    /// the value level (0-3), `SetBoxDimen` box measurements, and the
    /// full `LastItem` code range: tail-list quantities (0-3), scalar and
    /// font integers (`INPUT_LINE_NO_CODE`..`XETEX_PDF_PAGE_COUNT_CODE`),
    /// dimensions (`XETEX_GLYPH_BOUNDS_CODE`..`GLUE_SHRINK_CODE`), the
    /// e-TeX glue conversions `\mutoglue`/`\gluetomu` (65-66), and the
    /// expression primitives `\numexpr`/`\dimexpr`/`\glueexpr`/`\muexpr`
    /// (67-70).
    ///
    /// Returns `true` when the caller still has to run the generic C
    /// epilogue through [`TeXEngineState::finish_scan_something_internal`].
    /// The e-TeX `LastItem` codes return `false`: like C ~7491-7514, they
    /// apply a dedicated epilogue inline (level coercion with
    /// `delete_glue_ref`, and negation through a `new_spec` copy), because
    /// their results are *owned* glue references rather than borrowed
    /// register pointers.
    pub fn scan_something_internal(
        &mut self,
        level: u8,
        negative: bool,
    ) -> Result<bool, TeXMouthError> {
        let m = self.cur_chr;

        match self.cur_cmd {
            c if c == CommandCode::CharGiven as u8
                || c == CommandCode::MathGiven as u8
                || c == CommandCode::XetexMathGiven as u8 =>
            {
                self.cur_val = m;
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::AssignInt as u8 => {
                let index = usize::try_from(m)
                    .ok()
                    .filter(|&index| index < self.eqtb.len())
                    .ok_or_else(|| crate::scanner::oob(m))?;
                self.cur_val = self.eqtb[index].s1;
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::AssignDimen as u8 => {
                let index = usize::try_from(m)
                    .ok()
                    .filter(|&index| index < self.eqtb.len())
                    .ok_or_else(|| crate::scanner::oob(m))?;
                self.cur_val = self.eqtb[index].s1;
                self.cur_val_level = DIMEN_VAL;
            }
            c if c == CommandCode::AssignGlue as u8 || c == CommandCode::AssignMuGlue as u8 => {
                let index = usize::try_from(m)
                    .ok()
                    .filter(|&index| index < self.eqtb.len())
                    .ok_or_else(|| crate::scanner::oob(m))?;
                self.cur_val = self.eqtb[index].s1;
                self.cur_val_level = if c == CommandCode::AssignGlue as u8 {
                    GLUE_VAL
                } else {
                    MU_VAL
                };
            }
            c if c == CommandCode::SetAux as u8 => {
                if i32::from(self.mode.abs()) != m {
                    // C reports an improper mode and recovers with zero.
                    self.cur_val = 0;
                    self.cur_val_level = if level == TOK_VAL { INT_VAL } else { DIMEN_VAL };
                } else if m == TeXMode::Vmode as i32 {
                    self.cur_val = self.aux.s1;
                    self.cur_val_level = DIMEN_VAL;
                } else {
                    self.cur_val = self.aux.s0;
                    self.cur_val_level = INT_VAL;
                }
            }
            c if c == CommandCode::SetPrevGraf as u8 => {
                self.cur_val =
                    if self.mode == 0 {
                        0
                    } else {
                        let p = self.innermost_vmode_index().ok_or(
                            TeXMouthError::UnsupportedPrimitive("prevgraf outside vertical mode"),
                        )?;
                        if p == self.nest_ptr {
                            self.prev_graf
                        } else {
                            self.nest[p].prev_graf
                        }
                    };
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::SetPageInt as u8 => {
                self.cur_val = match m {
                    0 => self.dead_cycles,
                    2 => i32::from(self.interaction),
                    _ => self.insert_penalties,
                };
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::SetPageDimen as u8 => {
                let index = usize::try_from(m)
                    .ok()
                    .filter(|&index| index < PAGE_SO_FAR_COUNT)
                    .ok_or_else(|| crate::scanner::oob(m))?;
                self.cur_val = if self.page_contents == PAGE_EMPTY && !self.output_active {
                    if index == 0 {
                        MAX_HALFWORD
                    } else {
                        0
                    }
                } else {
                    self.page_so_far[index]
                };
                self.cur_val_level = DIMEN_VAL;
            }
            c if c == CommandCode::SetShape as u8 => {
                let index = usize::try_from(m)
                    .ok()
                    .filter(|&index| index < self.eqtb.len())
                    .ok_or_else(|| crate::scanner::oob(m))?;
                self.cur_val = if index > LOCAL_BASE + LOCAL_PAR_SHAPE {
                    self.scan_int()?;
                    let shape = self.eqtb[index].s1;
                    if shape == TEX_NULL || self.cur_val < 0 {
                        0
                    } else {
                        let shape_index = self.checked_engine_mem_index(shape)?;
                        let count = self.mem[shape_index + 1].s1;
                        let selected = self.cur_val.min(count);
                        let selected = shape
                            .checked_add(selected)
                            .and_then(|p| p.checked_add(1))
                            .ok_or_else(|| crate::scanner::oob(shape))?;
                        self.mem[self.checked_engine_mem_index(selected)?].s1
                    }
                } else {
                    let shape = self.eqtb[LOCAL_BASE + LOCAL_PAR_SHAPE].s1;
                    if shape == TEX_NULL {
                        0
                    } else {
                        self.mem[self.checked_engine_mem_index(shape)?].s0
                    }
                };
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::DefCode as u8 => {
                self.scan_usv_num()?;
                let character =
                    u32::try_from(self.cur_val).map_err(|_| crate::scanner::oob(self.cur_val))?;
                self.cur_val = match m {
                    CAT_CODE_BASE => i32::from(self.cat_code(character)),
                    LC_CODE_BASE => self.lc_code(character),
                    UC_CODE_BASE => self.uc_code(character),
                    SF_CODE_BASE => self.sf_code(character) % 65_536,
                    MATH_CODE_BASE => {
                        let value = self.math_code(character);
                        let math_char = value & 0x1F_FFFF;
                        if math_char == ACTIVE_MATH_CHAR {
                            0x8000
                        } else {
                            let class = (value >> 21) & 0x07;
                            let family = (value >> 24) & 0xFF;
                            if family > 15 || math_char > 255 {
                                0
                            } else {
                                class * 0x1000 + family * 0x100 + math_char
                            }
                        }
                    }
                    DEL_CODE_BASE => {
                        let value = self.del_code(character);
                        if value >= 0x4000_0000 {
                            0
                        } else {
                            value
                        }
                    }
                    _ => {
                        return Err(TeXMouthError::UnsupportedPrimitive(
                            "character code internal quantity",
                        ))
                    }
                };
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::XetexDefCode as u8 => {
                self.scan_usv_num()?;
                let character =
                    u32::try_from(self.cur_val).map_err(|_| crate::scanner::oob(self.cur_val))?;
                self.cur_val = match m {
                    SF_CODE_BASE => self.sf_code(character) / 65_536,
                    MATH_CODE_BASE => self.math_code(character),
                    DEL_CODE_BASE => self.del_code(character),
                    x if x == MATH_CODE_BASE + 1 || x == DEL_CODE_BASE + 1 => 0,
                    _ => {
                        return Err(TeXMouthError::UnsupportedPrimitive(
                            "XeTeX character code internal quantity",
                        ))
                    }
                };
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::AssignToks as u8 || c == CommandCode::ToksRegister as u8 => {
                if level != TOK_VAL {
                    // C reports "Missing number, treated as zero" and
                    // recovers with a dimen-level zero.
                    self.cur_val = 0;
                    self.cur_val_level = DIMEN_VAL;
                } else if c == CommandCode::AssignToks as u8 {
                    if m == (LOCAL_BASE + LOCAL_XETEX_INTER_CHAR_TOKS) as i32 {
                        self.scan_char_class_not_ignored()?;
                        let first_class = self.cur_val;
                        self.scan_char_class_not_ignored()?;
                        let key = first_class
                            .checked_mul(CHAR_CLASS_LIMIT)
                            .and_then(|key| key.checked_add(self.cur_val))
                            .ok_or_else(|| crate::scanner::oob(self.cur_val))?;
                        let element = self.find_sa_element(SA_INTER_CHAR_VAL, key, false)?;
                        self.cur_val = if element == TEX_NULL {
                            TEX_NULL
                        } else {
                            self.sparse_ptr_value(element)?
                        };
                    } else {
                        let index = usize::try_from(m)
                            .ok()
                            .filter(|&index| index < self.eqtb.len())
                            .ok_or_else(|| crate::scanner::oob(m))?;
                        self.cur_val = self.eqtb[index].s1;
                    }
                    self.cur_val_level = TOK_VAL;
                } else if m != 0 {
                    // A `\toksdef`-style control sequence bound to a sparse
                    // element carries the element pointer in `cur_chr`.
                    self.cur_val = self.sparse_ptr_value(m)?;
                    self.cur_val_level = TOK_VAL;
                } else {
                    let register = self.scan_register_num()?;
                    self.cur_val = if register > 255 {
                        let element = self.find_sa_element(SA_TOK_VAL, register, false)?;
                        if element == TEX_NULL {
                            TEX_NULL
                        } else {
                            self.sparse_ptr_value(element)?
                        }
                    } else {
                        self.toks(register as usize)?
                    };
                    self.cur_val_level = TOK_VAL;
                }
            }
            c if c == CommandCode::AssignFontDimen as u8 => {
                // C ~7395: `find_font_dimen(false)` selects the fmem cell; the
                // error sentinel reads the zeroed `font_info[fmem_ptr]` slot.
                match self.find_font_dimen(false)? {
                    Some((font, index)) => {
                        self.cur_val = self
                            .font_record(font)
                            .and_then(|record| record.params.get(index).copied())
                            .unwrap_or(0);
                    }
                    None => self.cur_val = 0,
                }
                self.cur_val_level = DIMEN_VAL;
            }
            c if c == CommandCode::AssignFontInt as u8 => {
                // C ~7402: `\hyphenchar`, `\skewchar`, `\lpcode`, `\rpcode`.
                self.scan_font_ident()?;
                let font = self.cur_val;
                match m {
                    HYPHEN_CHAR_CODE => {
                        self.cur_val = self.font_record(font).map_or(0, |r| r.hyphen_char);
                    }
                    SKEW_CHAR_CODE => {
                        self.cur_val = self.font_record(font).map_or(0, |r| r.skew_char);
                    }
                    _ => {
                        if self.is_native_font(font) {
                            self.scan_glyph_number(font)?;
                        } else {
                            self.scan_char_num()?;
                        }
                        let key = u32::try_from(self.cur_val)
                            .map_err(|_| crate::scanner::oob(self.cur_val))?;
                        self.cur_val = self.get_cp_code(font, key, m == LP_CODE_BASE);
                    }
                }
                self.cur_val_level = INT_VAL;
            }
            c if c == CommandCode::DefFamily as u8
                || c == CommandCode::SetFont as u8
                || c == CommandCode::DefFont as u8 =>
            {
                // C ~7211: font-valued quantities are token-level idents.
                if level != TOK_VAL {
                    // "Missing number, treated as zero" recovery.
                    self.cur_val = 0;
                    self.cur_val_level = DIMEN_VAL;
                } else {
                    self.back_input()?;
                    self.scan_font_ident()?;
                    self.cur_val = self.font_ident_cs(self.cur_val);
                    self.cur_val_level = IDENT_VAL;
                }
            }
            c if c == CommandCode::Register as u8 => {
                if !(0..=19).contains(&m) {
                    self.cur_val_level = self.sparse_element_class(m)? as u8;
                    self.cur_val = if self.cur_val_level < GLUE_VAL {
                        self.sparse_word_value(m)?
                    } else {
                        self.sparse_ptr_value(m)?
                    };
                } else {
                    let register = self.scan_register_num()?;
                    self.cur_val_level = m as u8;
                    if register > 255 {
                        let element = self.find_sa_element(m, register, false)?;
                        self.cur_val = if element == TEX_NULL {
                            0
                        } else if self.cur_val_level < GLUE_VAL {
                            self.sparse_word_value(element)?
                        } else {
                            self.sparse_ptr_value(element)?
                        };
                    } else {
                        self.cur_val = match m {
                            0 => self.count(register as usize)?,
                            1 => self.dimen(register as usize)?,
                            2 => self.skip(register as usize)?,
                            _ => self.mu_skip(register as usize)?,
                        };
                    }
                }
            }
            c if c == CommandCode::SetBoxDimen as u8 => {
                let register = self.scan_register_num()?;
                let q = if register > 255 {
                    let element = self.find_sa_element(SA_BOX_VAL, register, false)?;
                    if element == TEX_NULL {
                        TEX_NULL
                    } else {
                        self.sparse_ptr_value(element)?
                    }
                } else {
                    self.box_reg(register as usize)?
                };
                self.cur_val = if q == TEX_NULL {
                    0
                } else {
                    let index = q.checked_add(m).ok_or_else(|| crate::scanner::oob(q))?;
                    self.mem[self.checked_engine_mem_index(index)?].s1
                };
                self.cur_val_level = DIMEN_VAL;
            }
            c if c == CommandCode::LastItem as u8 => {
                // C ~7469: codes below `INPUT_LINE_NO_CODE` inspect the list
                // tail, integer codes run to `XETEX_DIM`, and dimension codes
                // run to `GLUE_SHRINK_CODE`; all of them produce borrowed
                // values that flow through the generic epilogue. Only the
                // e-TeX glue/mu/expression codes (`m >= ETEX_GLUE`) produce
                // owned results and run the dedicated epilogue (coercion
                // with delete_glue_ref, negation through a new_spec copy).
                match m {
                    m if (0..=LAST_NODE_TYPE_CODE).contains(&m) => {
                        self.scan_last_item_tail(m)?;
                    }
                    m if (INPUT_LINE_NO_CODE..XETEX_DIM).contains(&m) => {
                        self.scan_last_item_int(m)?;
                    }
                    m if (XETEX_DIM..=GLUE_SHRINK_CODE).contains(&m) => {
                        self.scan_last_item_dimen(m)?;
                    }
                    MU_TO_GLUE_CODE => {
                        self.scan_mu_glue()?;
                        self.cur_val_level = GLUE_VAL;
                    }
                    GLUE_TO_MU_CODE => {
                        self.scan_normal_glue()?;
                        self.cur_val_level = MU_VAL;
                    }
                    ETEX_EXPR..=ETEX_EXPR_MU => {
                        self.cur_val_level = (m - ETEX_EXPR) as u8;
                        self.scan_expr()?;
                    }
                    _ => {
                        return Err(TeXMouthError::UnsupportedPrimitive(
                            "last-item internal quantity",
                        ))
                    }
                }

                if m < ETEX_GLUE {
                    return Ok(true);
                }

                while self.cur_val_level > level {
                    if self.cur_val_level == GLUE_VAL {
                        let spec = self.cur_val;
                        self.cur_val = self.glue_spec_width(spec)?;
                        self.delete_glue_ref(spec)?;
                    } else if self.cur_val_level == MU_VAL {
                        self.mu_error();
                    }
                    self.cur_val_level -= 1;
                }

                if negative {
                    if self.cur_val_level >= GLUE_VAL {
                        let spec = self.cur_val;
                        self.cur_val = self.new_spec(spec)?;
                        self.delete_glue_ref(spec)?;
                        let q = self.cur_val;
                        self.mem[(q + 1) as usize].s1 = -self.mem[(q + 1) as usize].s1;
                        self.mem[(q + 2) as usize].s1 = -self.mem[(q + 2) as usize].s1;
                        self.mem[(q + 3) as usize].s1 = -self.mem[(q + 3) as usize].s1;
                    } else {
                        self.cur_val = self.cur_val.wrapping_neg();
                    }
                }
                return Ok(false);
            }
            _ => return Err(TeXMouthError::UnsupportedPrimitive("internal quantity")),
        }
        Ok(true)
    }

    /// Reads a tail-list `LastItem` value (`\lastpenalty`, `\lastkern`,
    /// `\lastskip`, `\lastnodetype`; C ~7948-8003).
    fn scan_last_item_tail(&mut self, m: i32) -> Result<(), TeXMouthError> {
        self.cur_val = 0;
        let mut tx = self.tail;
        if tx < self.hi_mem_min
            && self.node_type(tx)? == crate::state::MATH_NODE
            && i32::from(self.mem[tx as usize].b16_s0()) == crate::state::END_M_CODE
        {
            let mut r = self.head;
            let mut q;
            loop {
                q = r;
                r = self.mem[q as usize].s1;
                if r == tx {
                    break;
                }
            }
            tx = q;
        }
        if m == LAST_NODE_TYPE_CODE {
            self.cur_val_level = INT_VAL;
            if tx == self.head || self.mode == 0 {
                self.cur_val = -1;
            }
        } else {
            self.cur_val_level = m as u8;
        }
        if tx < self.hi_mem_min && self.mode != 0 {
            match m {
                m if m == i32::from(INT_VAL) => {
                    if self.node_type(tx)? == crate::state::PENALTY_NODE {
                        self.cur_val = self.mem[tx as usize + 1].s1;
                    }
                }
                m if m == i32::from(DIMEN_VAL) => {
                    if self.node_type(tx)? == crate::state::KERN_NODE {
                        self.cur_val = self.mem[tx as usize + 1].s1;
                    }
                }
                m if m == LAST_NODE_TYPE_CODE => {
                    let kind = self.node_type(tx)?;
                    self.cur_val = if kind <= crate::state::UNSET_NODE {
                        i32::from(kind) + 1
                    } else {
                        i32::from(crate::state::UNSET_NODE) + 2
                    };
                }
                _ => {
                    if self.node_type(tx)? == crate::state::GLUE_NODE {
                        self.cur_val = self.mem[tx as usize + 1].s0;
                        if self.mem[tx as usize].b16_s0() == MU_GLUE {
                            self.cur_val_level = MU_VAL;
                        }
                    }
                }
            }
        } else if self.mode == TeXMode::Vmode as i16 && tx == self.head {
            match m {
                m if m == i32::from(INT_VAL) => {
                    self.cur_val = self.last_penalty;
                }
                m if m == i32::from(DIMEN_VAL) => {
                    self.cur_val = self.last_kern;
                }
                m if m == LAST_NODE_TYPE_CODE => {
                    self.cur_val = self.last_node_type;
                }
                _ => {
                    if self.last_glue != MAX_HALFWORD {
                        self.cur_val = self.last_glue;
                    }
                }
            }
        }
        Ok(())
    }

    /// Reads an integer-level `LastItem` value (`INPUT_LINE_NO_CODE` through
    /// `XETEX_PDF_PAGE_COUNT_CODE`; C ~7620-7934).
    fn scan_last_item_int(&mut self, m: i32) -> Result<(), TeXMouthError> {
        match m {
            INPUT_LINE_NO_CODE => self.cur_val = self.line,
            BADNESS_CODE => self.cur_val = self.last_badness,
            ELAPSED_TIME_CODE => self.cur_val = self.get_microinterval(),
            RANDOM_SEED_CODE => self.cur_val = self.random_seed,
            PDF_SHELL_ESCAPE_CODE => self.cur_val = i32::from(self.shell_escape_enabled),
            ETEX_VERSION_CODE => self.cur_val = ETEX_VERSION,
            XETEX_VERSION_CODE => self.cur_val = XETEX_VERSION,
            XETEX_COUNT_GLYPHS_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                self.cur_val = if self.is_native_font(n) {
                    self.font_record(n).map_or(0, |record| record.glyph_count)
                } else {
                    0
                };
            }
            XETEX_COUNT_FEATURES_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                self.cur_val = if self.is_aat_font(n) || self.using_graphite(n) {
                    self.font_engine_get(n, m - XETEX_INT, &[])
                } else {
                    0
                };
            }
            XETEX_VARIATION_CODE
            | XETEX_VARIATION_MIN_CODE
            | XETEX_VARIATION_MAX_CODE
            | XETEX_VARIATION_DEFAULT_CODE
            | XETEX_COUNT_VARIATIONS_CODE => {
                self.scan_font_ident()?;
                self.cur_val = 0;
            }
            XETEX_FEATURE_CODE_CODE
            | XETEX_IS_EXCLUSIVE_FEATURE_CODE
            | XETEX_COUNT_SELECTORS_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.is_aat_font(n) || self.using_graphite(n) {
                    self.scan_int()?;
                    let k = self.cur_val;
                    self.cur_val = self.font_engine_get(n, m - XETEX_INT, &[k]);
                } else {
                    self.font_query_error(m, n, "not an AAT or Graphite font");
                    self.cur_val = -1;
                }
            }
            XETEX_SELECTOR_CODE_CODE | XETEX_IS_DEFAULT_SELECTOR_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.is_aat_font(n) || self.using_graphite(n) {
                    self.scan_int()?;
                    let k = self.cur_val;
                    self.scan_int()?;
                    self.cur_val = self.font_engine_get(n, m - XETEX_INT, &[k, self.cur_val]);
                } else {
                    self.font_query_error(m, n, "not an AAT or Graphite font");
                    self.cur_val = -1;
                }
            }
            XETEX_FIND_VARIATION_BY_NAME_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.is_aat_font(n) {
                    self.scan_and_pack_name()?;
                    let name = self.file_name_string();
                    self.cur_val = self.font_engine_named(n, m - XETEX_INT, -1, &name);
                } else {
                    self.font_query_error(m, n, "not an AAT font");
                    self.cur_val = -1;
                }
            }
            XETEX_FIND_FEATURE_BY_NAME_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.is_aat_font(n) || self.using_graphite(n) {
                    self.scan_and_pack_name()?;
                    let name = self.file_name_string();
                    self.cur_val = self.font_engine_named(n, m - XETEX_INT, -1, &name);
                } else {
                    self.font_query_error(m, n, "not an AAT or Graphite font");
                    self.cur_val = -1;
                }
            }
            XETEX_FIND_SELECTOR_BY_NAME_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.is_aat_font(n) || self.using_graphite(n) {
                    self.scan_int()?;
                    let k = self.cur_val;
                    self.scan_and_pack_name()?;
                    let name = self.file_name_string();
                    self.cur_val = self.font_engine_named(n, m - XETEX_INT, k, &name);
                } else {
                    self.font_query_error(m, n, "not an AAT or Graphite font");
                    self.cur_val = -1;
                }
            }
            XETEX_OT_COUNT_SCRIPTS_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                self.cur_val = if self.using_open_type(n) {
                    self.font_engine_get(n, m - XETEX_INT, &[])
                } else {
                    0
                };
            }
            XETEX_OT_COUNT_LANGUAGES_CODE | XETEX_OT_SCRIPT_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.using_open_type(n) {
                    self.scan_int()?;
                    self.cur_val = self.font_engine_get(n, m - XETEX_INT, &[self.cur_val]);
                } else {
                    self.font_query_error(m, n, "not an OpenType Layout font");
                    self.cur_val = -1;
                }
            }
            XETEX_OT_COUNT_FEATURES_CODE | XETEX_OT_LANGUAGE_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.using_open_type(n) {
                    self.scan_int()?;
                    let k = self.cur_val;
                    self.scan_int()?;
                    self.cur_val = self.font_engine_get(n, m - XETEX_INT, &[k, self.cur_val]);
                } else {
                    self.font_query_error(m, n, "not an OpenType Layout font");
                    self.cur_val = -1;
                }
            }
            XETEX_OT_FEATURE_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                if self.using_open_type(n) {
                    self.scan_int()?;
                    let k = self.cur_val;
                    self.scan_int()?;
                    let kk = self.cur_val;
                    self.scan_int()?;
                    self.cur_val = self.font_engine_get(n, m - XETEX_INT, &[k, kk, self.cur_val]);
                } else {
                    self.font_query_error(m, n, "not an OpenType Layout font");
                    self.cur_val = -1;
                }
            }
            XETEX_MAP_CHAR_TO_GLYPH_CODE => {
                let font = self.eqtb[CUR_FONT_LOC].s1;
                if self.is_native_font(font) {
                    self.scan_int()?;
                    self.cur_val = self.map_char_to_glyph(font, self.cur_val.max(0) as u32);
                } else {
                    self.font_query_error(m, font, "not a native platform font");
                    self.cur_val = 0;
                }
            }
            XETEX_GLYPH_INDEX_CODE => {
                let font = self.eqtb[CUR_FONT_LOC].s1;
                if self.is_native_font(font) {
                    self.scan_and_pack_name()?;
                    let name = self.file_name_string();
                    self.cur_val = self.map_glyph_to_index(font, &name);
                } else {
                    self.font_query_error(m, font, "not a native platform font");
                    self.cur_val = 0;
                }
            }
            XETEX_FONT_TYPE_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                self.cur_val = self.native_font_type(n);
            }
            XETEX_FIRST_CHAR_CODE | XETEX_LAST_CHAR_CODE => {
                self.scan_font_ident()?;
                let n = self.cur_val;
                self.cur_val = self.get_font_char_range(n, m == XETEX_FIRST_CHAR_CODE);
            }
            PDF_LAST_X_POS_CODE => self.cur_val = self.pdf_last_x_pos,
            PDF_LAST_Y_POS_CODE => self.cur_val = self.pdf_last_y_pos,
            XETEX_PDF_PAGE_COUNT_CODE => {
                self.scan_and_pack_name()?;
                self.cur_val = self.count_pdf_file_pages();
            }
            CURRENT_GROUP_LEVEL_CODE => self.cur_val = i32::from(self.cur_level) - 1,
            CURRENT_GROUP_TYPE_CODE => self.cur_val = i32::from(self.cur_group),
            CURRENT_IF_LEVEL_CODE => {
                let mut q = self.cond_ptr;
                self.cur_val = 0;
                while q != TEX_NULL {
                    self.cur_val += 1;
                    q = self.mem[self.checked_engine_mem_index(q)?].s1;
                }
            }
            CURRENT_IF_TYPE_CODE => {
                self.cur_val = if self.cond_ptr == TEX_NULL {
                    0
                } else if i32::from(self.cur_if) < UNLESS_CODE {
                    i32::from(self.cur_if) + 1
                } else {
                    -(i32::from(self.cur_if) - 31)
                };
            }
            CURRENT_IF_BRANCH_CODE => {
                self.cur_val = if self.if_limit == OR_CODE_LIMIT || self.if_limit == ELSE_CODE_LIMIT
                {
                    1
                } else if self.if_limit == FI_CODE_LIMIT {
                    -1
                } else {
                    0
                };
            }
            GLUE_STRETCH_ORDER_CODE | GLUE_SHRINK_ORDER_CODE => {
                self.scan_normal_glue()?;
                let q = self.cur_val;
                let index = self.checked_engine_mem_index(q)?;
                self.cur_val = if m == GLUE_STRETCH_ORDER_CODE {
                    i32::from(self.mem[index].b16_s1())
                } else {
                    i32::from(self.mem[index].b16_s0())
                };
                self.delete_glue_ref(q).map_err(TeXMouthError::Memory)?;
            }
            _ => {
                return Err(TeXMouthError::UnsupportedPrimitive(
                    "last-item internal quantity",
                ))
            }
        }
        self.cur_val_level = INT_VAL;
        Ok(())
    }

    /// Reads a dimension-level `LastItem` value (`XETEX_GLYPH_BOUNDS_CODE`
    /// through `GLUE_SHRINK_CODE`; C ~7517-7623).
    fn scan_last_item_dimen(&mut self, m: i32) -> Result<(), TeXMouthError> {
        match m {
            XETEX_GLYPH_BOUNDS_CODE => {
                let font = self.eqtb[CUR_FONT_LOC].s1;
                if self.is_native_font(font) {
                    self.scan_int()?;
                    let n = self.cur_val;
                    if !(1..=4).contains(&n) {
                        self.transcript.push(format!(
                            "! \\\\XeTeXglyphbounds requires an edge index from 1 to 4; I don't know anything about edge {n}"
                        ));
                        self.error_count += 1;
                        self.cur_val = 0;
                    } else {
                        self.scan_int()?;
                        self.cur_val = self.get_glyph_bounds(font, n, self.cur_val.max(0) as u32);
                    }
                } else {
                    self.font_query_error(m, font, "not a native platform font");
                    self.cur_val = 0;
                }
            }
            FONT_CHAR_WD_CODE | FONT_CHAR_HT_CODE | FONT_CHAR_DP_CODE | FONT_CHAR_IC_CODE => {
                self.scan_font_ident()?;
                let q = self.cur_val;
                self.scan_usv_num()?;
                let character = self.cur_val.max(0) as u32;
                self.cur_val = if self.is_native_font(q) {
                    match m {
                        FONT_CHAR_WD_CODE => self.get_native_char_wd(q, character),
                        FONT_CHAR_HT_CODE => self.get_native_char_height_depth(q, character).0,
                        FONT_CHAR_DP_CODE => self.get_native_char_height_depth(q, character).1,
                        _ => self.get_native_char_ic(q, character),
                    }
                } else {
                    match self.font_record(q) {
                        Some(record) if record.bc <= character && character <= record.ec => {
                            let metric = self.font_char_metric(q, character);
                            match m {
                                FONT_CHAR_WD_CODE => metric.width,
                                FONT_CHAR_HT_CODE => metric.height,
                                FONT_CHAR_DP_CODE => metric.depth,
                                _ => metric.italic,
                            }
                        }
                        _ => 0,
                    }
                };
            }
            PAR_SHAPE_LENGTH_CODE | PAR_SHAPE_INDENT_CODE | PAR_SHAPE_DIMEN_CODE => {
                let mut q = m - PAR_SHAPE_LENGTH_CODE;
                self.scan_int()?;
                let shape = self.eqtb[LOCAL_BASE + LOCAL_PAR_SHAPE].s1;
                if shape == TEX_NULL || self.cur_val <= 0 {
                    self.cur_val = 0;
                } else {
                    if q == 2 {
                        q = self.cur_val % 2;
                        self.cur_val = (self.cur_val + q) / 2;
                    }
                    let shape_index = self.checked_engine_mem_index(shape)?;
                    let count = self.mem[shape_index].s0;
                    if self.cur_val > count {
                        self.cur_val = count;
                    }
                    let cell = shape
                        .checked_add(2 * self.cur_val)
                        .and_then(|p| p.checked_sub(q))
                        .ok_or_else(|| crate::scanner::oob(shape))?;
                    self.cur_val = self.mem[self.checked_engine_mem_index(cell)?].s1;
                }
            }
            GLUE_STRETCH_CODE | GLUE_SHRINK_CODE => {
                self.scan_normal_glue()?;
                let q = self.cur_val;
                let index =
                    self.checked_engine_mem_index(q + if m == GLUE_STRETCH_CODE { 2 } else { 3 })?;
                self.cur_val = self.mem[index].s1;
                self.delete_glue_ref(q).map_err(TeXMouthError::Memory)?;
            }
            _ => {
                return Err(TeXMouthError::UnsupportedPrimitive(
                    "last-item dimension quantity",
                ))
            }
        }
        self.cur_val_level = DIMEN_VAL;
        Ok(())
    }

    /// Records a C `not_*_font_error` diagnostic for a `LastItem` font query.
    fn font_query_error(&mut self, m: i32, font: i32, suffix: &str) {
        let primitive =
            crate::commands::primitive_name_for(CommandCode::LastItem, m).unwrap_or("lastitem");
        let name = self
            .font_record(font)
            .map(|record| {
                record
                    .name
                    .iter()
                    .filter_map(|&unit| char::from_u32(unit as u32))
                    .collect::<String>()
            })
            .unwrap_or_default();
        self.transcript
            .push(format!("! Cannot use \\{primitive} with {name}; {suffix}"));
        self.error_count += 1;
    }

    /// Scans an integer constrained to `0..=255` (`scan_eight_bit_int`).
    pub fn scan_eight_bit_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, 255)
    }

    /// Scans an integer constrained to a Unicode scalar value
    /// (`scan_usv_num`, `0..=0x10FFFF`).
    pub fn scan_usv_num(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, BIGGEST_USV)
    }

    /// Scans an integer constrained to a 16-bit character code
    /// (`scan_char_num`, `0..=65535`).
    pub fn scan_char_num(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, BIGGEST_CHAR)
    }

    /// Scans a packed XeTeX math character code (`scan_xetex_math_char_int`).
    ///
    /// Like C, an encoded active math char normalizes to `ACTIVE_MATH_CHAR`;
    /// other codes whose character payload exceeds `BIGGEST_USV` recover as
    /// zero.
    pub fn scan_xetex_math_char_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_int()?;
        let math_char = self.cur_val as u32 & MATH_CHAR_MASK;
        if math_char == ACTIVE_MATH_CHAR as u32 {
            if self.cur_val != ACTIVE_MATH_CHAR {
                self.cur_val = ACTIVE_MATH_CHAR;
            }
        } else if math_char > BIGGEST_USV as u32 {
            self.cur_val = 0;
        }
        Ok(self.cur_val)
    }

    /// Scans an integer constrained to `0..=15` (`scan_four_bit_int`).
    pub fn scan_four_bit_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, 15)
    }

    /// Scans an integer constrained to `0..=32767` (`scan_fifteen_bit_int`).
    pub fn scan_fifteen_bit_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, 0x7FFF)
    }

    /// Scans an integer constrained to a math class `0..=7`
    /// (`scan_math_class_int`).
    pub fn scan_math_class_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, 7)
    }

    /// Scans an integer constrained to a math family `0..=255`
    /// (`scan_math_fam_int`).
    pub fn scan_math_fam_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, NUMBER_MATH_FAMILIES - 1)
    }

    /// Scans an integer constrained to a XeTeX character class
    /// `0..CHAR_CLASS_LIMIT` (`scan_char_class`).
    pub fn scan_char_class(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, CHAR_CLASS_LIMIT - 1)
    }

    /// Scans a font identifier (`scan_font_ident`, C ~7029).
    ///
    /// A `\font`-defined control sequence (`SetFont`) yields its payload;
    /// `\nullfont`/`\font` (`DefFont`) yields the current font; a
    /// `\textfont`-family primitive (`DefFamily`) scans a math-family number
    /// and reads the family slot. Anything else follows C's
    /// "Missing font identifier" recovery and yields `nullfont`.
    pub fn scan_font_ident(&mut self) -> Result<i32, TeXMouthError> {
        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 {
                break;
            }
        }

        let font = if self.cur_cmd == CommandCode::DefFont as u8 {
            self.eqtb[CUR_FONT_LOC].s1
        } else if self.cur_cmd == CommandCode::SetFont as u8 {
            self.cur_chr
        } else if self.cur_cmd == CommandCode::DefFamily as u8 {
            let base = Self::c_eqtb_pos_to_compact(self.cur_chr);
            self.scan_math_fam_int()?;
            usize::try_from(base + self.cur_val)
                .ok()
                .filter(|&index| index < self.eqtb.len())
                .map_or(0, |index| self.eqtb[index].s1)
        } else {
            // C reports "Missing font identifier" and recovers with FONT_BASE.
            self.back_input()?;
            0
        };
        self.cur_val = font;
        Ok(font)
    }

    /// Selects a `\fontdimen` cell (`find_font_dimen`, C ~7060).
    ///
    /// Returns `Some((font, param_index))` when the cell exists or was newly
    /// allocated on the most recently loaded font, and `None` on the C error
    /// path ("Font ... has only N fontdimen parameters").
    pub fn find_font_dimen(
        &mut self,
        writing: bool,
    ) -> Result<Option<(i32, usize)>, TeXMouthError> {
        self.scan_int()?;
        let n = self.cur_val;
        self.scan_font_ident()?;
        let font = self.cur_val;
        if n <= 0 {
            self.font_dimen_error(font)?;
            return Ok(None);
        }

        if writing && (SPACE_CODE..=SPACE_SHRINK_CODE).contains(&n) {
            let glue = self
                .font_record_mut(font)
                .map(|record| std::mem::replace(&mut record.glue, TEX_NULL));
            if let Some(glue) = glue.filter(|&glue| glue != TEX_NULL) {
                self.delete_glue_ref(glue).map_err(TeXMouthError::Memory)?;
            }
        }

        let param_count = self
            .font_record(font)
            .map_or(0, |record| record.params.len().saturating_sub(1));
        let n = usize::try_from(n).map_err(|_| crate::scanner::oob(n))?;
        if n > param_count {
            if font < self.font_ptr() {
                self.font_dimen_error(font)?;
                return Ok(None);
            }
            let Some(record) = self.font_record_mut(font) else {
                self.font_dimen_error(font)?;
                return Ok(None);
            };
            while record.params.len() <= n {
                record.params.push(0);
            }
        }
        Ok(Some((font, n)))
    }

    /// Records C's "Font <name> has only <n> fontdimen parameters" diagnostic.
    fn font_dimen_error(&mut self, font: i32) -> Result<(), TeXMouthError> {
        let count = self
            .font_record(font)
            .map_or(0, |record| record.params.len().saturating_sub(1));
        let name = self
            .font_record(font)
            .map(|record| {
                record
                    .cs_name
                    .iter()
                    .filter_map(|&unit| char::from_u32(unit as u32))
                    .collect::<String>()
            })
            .unwrap_or_default();
        self.transcript.push(format!(
            "! Font {name} has only {count} fontdimen parameters"
        ));
        self.error_count += 1;
        Ok(())
    }

    /// Scans a glyph selector for native fonts (`scan_glyph_number`, C ~6603):
    /// `/name` maps a glyph name, `u<char>` maps a Unicode scalar, otherwise a
    /// plain integer is scanned.
    pub fn scan_glyph_number(&mut self, f: i32) -> Result<(), TeXMouthError> {
        if self.scan_keyword("/")? {
            self.scan_file_name()?;
            let name = self.file_name_string();
            self.cur_val = self.map_glyph_to_index(f, &name);
            self.cur_val_level = INT_VAL;
        } else if self.scan_keyword("u")? {
            self.scan_char_num()?;
            self.cur_val = self.map_char_to_glyph(f, self.cur_val.max(0) as u32);
            self.cur_val_level = INT_VAL;
        } else {
            self.scan_int()?;
        }
        Ok(())
    }

    /// Scans a XeTeX inter-character class (`scan_char_class_not_ignored`).
    ///
    /// The C scanner shares the same `0..CHAR_CLASS_LIMIT` bounds as
    /// `scan_char_class`; it differs only in diagnostic text.
    pub fn scan_char_class_not_ignored(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_char_class()
    }

    /// Scans an integer constrained to a delimiter code `0..=0x7FFFFFF`
    /// (`scan_delimiter_int`).
    pub fn scan_delimiter_int(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, 0x7FF_FFFF)
    }

    /// Scans an integer constrained to a dense register number `0..=255`
    /// (`scan_register_num`).
    pub fn scan_register_num(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_bounded_int(0, MAX_REGISTER_NUM)
    }

    /// Scans an integer constrained to `0..=15` or exactly 18
    /// (`scan_four_bit_int_or_18`).
    pub fn scan_four_bit_int_or_18(&mut self) -> Result<i32, TeXMouthError> {
        let value = self.scan_int()?;
        if !(0..=15).contains(&value) && value != 18 {
            return Err(crate::scanner::oob(value));
        }
        Ok(value)
    }

    /// Shared implementation for the bounded `scan_*_int` wrappers.
    ///
    /// The C engine reports a recoverable error and substitutes zero; this
    /// port surfaces the range violation as a structured error until the
    /// diagnostic/recovery channel exists.
    fn scan_bounded_int(&mut self, min: i32, max: i32) -> Result<i32, TeXMouthError> {
        let value = self.scan_int()?;
        if !(min..=max).contains(&value) {
            return Err(crate::scanner::oob(value));
        }
        Ok(value)
    }

    /// Resolves the value of `` ` ``-style alphabetic constants written with
    /// a control sequence (`cur_tok >= CS_TOKEN_FLAG`).
    ///
    /// In the C layout, control-sequence indices below `SINGLE_BASE` denote
    /// active characters and indices `SINGLE_BASE + c` denote one-character
    /// names, so the constant is `cs - ACTIVE_BASE` or `cs - SINGLE_BASE`.
    /// The compact [`crate::scanner::ControlSequenceTable`] stores the name
    /// directly, so a one-codepoint name (including active characters, whose
    /// stored name is the character itself) yields that codepoint; anything
    /// else is an improper constant reported via `TOO_BIG_USV`.
    fn alphabetic_constant(&self, cs: i32) -> i32 {
        let name = self.cs_table.name(cs);
        if name.len() == 1 && name[0] >= 0 {
            name[0]
        } else {
            TOO_BIG_USV
        }
    }

    /// Scans a TeX dimension or math dimension (`xetex_scan_dimen`, C ~8235).
    ///
    /// `mu` requires `mu` units (or `mu`-valued internal quantities), `inf`
    /// additionally accepts `fil`/`fill`/`filll` infinite-glue units,
    /// `shortcut` skips the sign/number scan entirely (the caller already
    /// placed the integer part in `cur_val`), and `requires_units` selects
    /// between a dimension (`true`) and a bare decimal constant (`false`).
    /// The result is stored in `cur_val` and returned.
    ///
    /// The C `goto` labels are modeled with flags: `attach_sign` (internal
    /// dimen/mu results skip the sign/normalization pass), `found` (an
    /// internal quantity used as the unit skips both the fraction attach
    /// and the trailing-space consumption), and `skip_fraction` (`sp`
    /// jumps to `done`, discarding the scanned fraction).
    pub fn xetex_scan_dimen(
        &mut self,
        mu: bool,
        inf: bool,
        shortcut: bool,
        requires_units: bool,
    ) -> Result<i32, TeXMouthError> {
        let mut f: i32 = 0;
        self.arith_error = false;
        self.cur_order = NORMAL as i16;
        let mut negative = false;
        let mut attach_sign = false;

        if !shortcut {
            loop {
                loop {
                    self.get_x_token()?;
                    if self.cur_cmd != CommandCode::Spacer as u8 {
                        break;
                    }
                }
                if self.cur_tok == OTHER_MINUS {
                    negative = !negative;
                    self.cur_tok = OTHER_PLUS;
                }
                if self.cur_tok != OTHER_PLUS {
                    break;
                }
            }

            if (MIN_INTERNAL..=MAX_INTERNAL).contains(&self.cur_cmd) {
                if mu {
                    if self.scan_something_internal(MU_VAL, false)? {
                        self.finish_scan_something_internal(MU_VAL, false)?;
                    }
                    if self.cur_val_level >= GLUE_VAL {
                        let v = self.glue_spec_width(self.cur_val)?;
                        self.delete_glue_ref(self.cur_val)?;
                        self.cur_val = v;
                    }
                    if self.cur_val_level == MU_VAL {
                        attach_sign = true;
                    } else if self.cur_val_level != INT_VAL {
                        self.mu_error();
                    }
                } else {
                    if self.scan_something_internal(DIMEN_VAL, false)? {
                        self.finish_scan_something_internal(DIMEN_VAL, false)?;
                    }
                    if self.cur_val_level == DIMEN_VAL {
                        attach_sign = true;
                    }
                }
            } else {
                self.back_input()?;

                if self.cur_tok == CONTINENTAL_POINT_TOKEN {
                    self.cur_tok = POINT_TOKEN;
                }
                if self.cur_tok != POINT_TOKEN {
                    self.scan_int()?;
                } else {
                    self.radix = 10;
                    self.cur_val = 0;
                }

                if self.cur_tok == CONTINENTAL_POINT_TOKEN {
                    self.cur_tok = POINT_TOKEN;
                }
                if self.radix == 10 && self.cur_tok == POINT_TOKEN {
                    // C collects the digits into a linked list and then into
                    // the shared `dig` array; a local buffer is equivalent.
                    let mut k = 0usize;
                    let mut dig = [0i32; 17];
                    self.get_token()?;

                    loop {
                        self.get_x_token()?;
                        if !(ZERO_TOKEN..=ZERO_TOKEN + 9).contains(&self.cur_tok) {
                            break;
                        }
                        if k < 17 {
                            dig[k] = self.cur_tok - ZERO_TOKEN;
                            k += 1;
                        }
                    }

                    f = round_decimals(k, &dig);
                    if self.cur_cmd != CommandCode::Spacer as u8 {
                        self.back_input()?;
                    }
                }
            }
        }

        if !attach_sign {
            if self.cur_val < 0 {
                negative = !negative;
                self.cur_val = self.cur_val.wrapping_neg();
            }

            if requires_units {
                let mut found = false;
                let mut skip_fraction = false;

                if inf && self.scan_keyword("fil")? {
                    self.cur_order = FIL as i16;
                    while self.scan_keyword("l")? {
                        if self.cur_order == FILLL as i16 {
                            // C: "Illegal unit of measure (replaced with
                            // filll)"; silent recovery keeps FILLL.
                        } else {
                            self.cur_order += 1;
                        }
                    }
                } else {
                    let save_cur_val = self.cur_val;
                    loop {
                        self.get_x_token()?;
                        if self.cur_cmd != CommandCode::Spacer as u8 {
                            break;
                        }
                    }

                    if (MIN_INTERNAL..=MAX_INTERNAL).contains(&self.cur_cmd) {
                        if mu {
                            if self.scan_something_internal(MU_VAL, false)? {
                                self.finish_scan_something_internal(MU_VAL, false)?;
                            }
                            if self.cur_val_level >= GLUE_VAL {
                                let v = self.glue_spec_width(self.cur_val)?;
                                self.delete_glue_ref(self.cur_val)?;
                                self.cur_val = v;
                            }
                            if self.cur_val_level != MU_VAL {
                                self.mu_error();
                            }
                        } else {
                            if self.scan_something_internal(DIMEN_VAL, false)? {
                                self.finish_scan_something_internal(DIMEN_VAL, false)?;
                            }
                        }
                        let v = self.cur_val;
                        let addend = self.xn_over_d(v, f, 65536);
                        self.cur_val = self.mult_and_add(save_cur_val, v, addend, MAX_HALFWORD);
                        found = true;
                    } else {
                        self.back_input()?;
                        let font_param = if !mu && self.scan_keyword("em")? {
                            Some(QUAD_CODE)
                        } else if !mu && self.scan_keyword("ex")? {
                            Some(X_HEIGHT_CODE)
                        } else {
                            None
                        };
                        if let Some(code) = font_param {
                            // C ~8386: `v` is the current font's quad or
                            // x_height, then the `found` epilogue.
                            let v = self
                                .font_record(self.eqtb[CUR_FONT_LOC].s1)
                                .and_then(|record| record.params.get(code as usize).copied())
                                .unwrap_or(0);
                            let addend = self.xn_over_d(v, f, 65536);
                            self.cur_val = self.mult_and_add(save_cur_val, v, addend, MAX_HALFWORD);
                            found = true;
                            self.get_x_token()?;
                            if self.cur_cmd != CommandCode::Spacer as u8 {
                                self.back_input()?;
                            }
                        } else {
                            skip_fraction = self.scan_dimen_units(mu, &mut f)?;
                        }
                    }
                }

                if !found {
                    if !skip_fraction {
                        if self.cur_val >= 16384 {
                            self.arith_error = true;
                        } else {
                            self.cur_val = self.cur_val * 65536 + f;
                        }
                    }
                    // done: one optional space follows the unit.
                    self.get_x_token()?;
                    if self.cur_cmd != CommandCode::Spacer as u8 {
                        self.back_input()?;
                    }
                }
            } else if self.cur_val >= 16384 {
                self.arith_error = true;
            } else {
                self.cur_val = self.cur_val * 65536 + f;
            }
        }

        // attach_sign:
        if self.arith_error || self.cur_val.unsigned_abs() >= 0x4000_0000 {
            // C: "Dimension too large"; recovered with the largest legal
            // dimension (about 19 feet).
            self.cur_val = MAX_HALFWORD;
            self.arith_error = false;
        }
        if negative {
            self.cur_val = self.cur_val.wrapping_neg();
        }
        Ok(self.cur_val)
    }

    /// Scans the explicit unit keywords of `xetex_scan_dimen` (C ~8446,
    /// `not_found` through `done2`).
    ///
    /// Returns `true` when the scan should `goto done` (the `sp` case,
    /// which discards the fraction); `false` selects `attach_fraction`.
    fn scan_dimen_units(&mut self, mu: bool, f: &mut i32) -> Result<bool, TeXMouthError> {
        if mu {
            if !self.scan_keyword("mu")? {
                // C: "Illegal unit of measure (mu inserted)"; silent
                // recovery attaches the fraction anyway.
            }
            return Ok(false);
        }

        if self.scan_keyword("true")? {
            // C §476: `true` locks \mag and cancels the magnification, so
            // `1 true in` is a physical inch regardless of \magnification.
            self.prepare_mag()?;
            let mag = self.int_par(INT_PAR_MAG)?;
            if mag != 1000 {
                self.cur_val = self.xn_over_d(self.cur_val, 1000, mag);
                *f = (1000 * *f + 65536 * self.tex_remainder) / mag;
                self.cur_val += *f / 65536;
                *f %= 65536;
            }
        }

        if self.scan_keyword("pt")? {
            return Ok(false);
        }

        let (num, denom) = if self.scan_keyword("in")? {
            (7227, 100)
        } else if self.scan_keyword("pc")? {
            (12, 1)
        } else if self.scan_keyword("cm")? {
            (7227, 254)
        } else if self.scan_keyword("mm")? {
            (7227, 2540)
        } else if self.scan_keyword("bp")? {
            (7227, 7200)
        } else if self.scan_keyword("dd")? {
            (1238, 1157)
        } else if self.scan_keyword("cc")? {
            (14856, 1157)
        } else if self.scan_keyword("sp")? {
            return Ok(true);
        } else {
            // C: "Illegal unit of measure (pt inserted)"; silent recovery
            // attaches the fraction as if `pt` were given.
            return Ok(false);
        };

        self.cur_val = self.xn_over_d(self.cur_val, num, denom);
        *f = (num * *f + 65536 * self.tex_remainder) / denom;
        self.cur_val += *f / 65536;
        *f %= 65536;
        Ok(false)
    }

    /// Scans a dimension (`scan_dimen`, C ~8518).
    pub fn scan_dimen(
        &mut self,
        mu: bool,
        inf: bool,
        shortcut: bool,
    ) -> Result<i32, TeXMouthError> {
        self.xetex_scan_dimen(mu, inf, shortcut, true)
    }

    /// Scans a bare decimal constant without units (`scan_decimal`, C ~8523).
    pub fn scan_decimal(&mut self) -> Result<i32, TeXMouthError> {
        self.xetex_scan_dimen(false, false, false, false)
    }

    /// Scans a glue or mu-glue specification (`scan_glue`, C ~8529).
    ///
    /// Returns the glue-spec node in `cur_val`. An internal quantity of
    /// glue/mu level is returned directly; an integer-level quantity is
    /// reinterpreted as the width through the `shortcut` dimension scan,
    /// exactly as C does.
    pub fn scan_glue(&mut self, level: u8) -> Result<i32, TeXMouthError> {
        let mu = level == MU_VAL;
        let mut negative = false;

        loop {
            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8 {
                    break;
                }
            }
            if self.cur_tok == OTHER_MINUS {
                negative = !negative;
                self.cur_tok = OTHER_PLUS;
            }
            if self.cur_tok != OTHER_PLUS {
                break;
            }
        }

        if (MIN_INTERNAL..=MAX_INTERNAL).contains(&self.cur_cmd) {
            if self.scan_something_internal(level, negative)? {
                self.finish_scan_something_internal(level, negative)?;
            }
            if self.cur_val_level >= GLUE_VAL {
                if self.cur_val_level != level {
                    self.mu_error();
                }
                return Ok(self.cur_val);
            }

            if self.cur_val_level == INT_VAL {
                self.scan_dimen(mu, false, true)?;
            } else if level == MU_VAL {
                self.mu_error();
            }
        } else {
            self.back_input()?;
            self.scan_dimen(mu, false, false)?;
            if negative {
                self.cur_val = self.cur_val.wrapping_neg();
            }
        }

        let q = self.new_spec(0)?;
        self.mem[(q + 1) as usize].s1 = self.cur_val;

        if self.scan_keyword("plus")? {
            self.scan_dimen(mu, true, false)?;
            self.mem[(q + 2) as usize].s1 = self.cur_val;
            self.mem[q as usize].set_b16_s1(self.cur_order as u16);
        }

        if self.scan_keyword("minus")? {
            self.scan_dimen(mu, true, false)?;
            self.mem[(q + 3) as usize].s1 = self.cur_val;
            self.mem[q as usize].set_b16_s0(self.cur_order as u16);
        }

        self.cur_val = q;
        Ok(q)
    }

    /// Scans ordinary glue (`scan_normal_glue`, C ~8969).
    pub fn scan_normal_glue(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_glue(GLUE_VAL)
    }

    /// Scans math glue (`scan_mu_glue`, C ~8974).
    pub fn scan_mu_glue(&mut self) -> Result<i32, TeXMouthError> {
        self.scan_glue(MU_VAL)
    }

    /// Computes `x + y`, negating `y` first when `negative` is set, and
    /// flags `arith_error` when the magnitude would exceed `max_answer`
    /// (`add_or_sub`, C ~8588).
    fn add_or_sub(&mut self, x: i32, y: i32, max_answer: i32, negative: bool) -> i32 {
        let y = if negative { y.wrapping_neg() } else { y };
        if x >= 0 {
            if y <= max_answer - x {
                x + y
            } else {
                self.arith_error = true;
                0
            }
        } else if y >= -max_answer - x {
            x + y
        } else {
            self.arith_error = true;
            0
        }
    }

    /// Computes `n / d` rounded to the nearest integer, with ties going
    /// away from zero (`quotient`, C ~8612). A zero divisor flags
    /// `arith_error` and yields 0.
    fn quotient(&mut self, n: i32, d: i32) -> i32 {
        if d == 0 {
            self.arith_error = true;
            return 0;
        }
        let mut negative = false;
        let mut d = d;
        let mut n = n;
        if d < 0 {
            d = d.wrapping_neg();
            negative = true;
        }
        if n < 0 {
            n = n.wrapping_neg();
            negative = !negative;
        }
        let mut a = n / d;
        n -= a * d;
        d = n - d;
        if d + n >= 0 {
            a += 1;
        }
        if negative {
            a = a.wrapping_neg();
        }
        a
    }

    /// Computes `x * n / d` without intermediate overflow, rounding to the
    /// nearest integer (`fract`, C ~8643). A zero divisor or a result
    /// exceeding `max_answer` flags `arith_error` and yields 0.
    pub(crate) fn fract(&mut self, x: i32, n: i32, d: i32, max_answer: i32) -> i32 {
        if d == 0 {
            self.arith_error = true;
            return 0;
        }
        let mut negative = false;
        let mut x = x;
        let mut n = n;
        let mut d = d;
        if d < 0 {
            d = d.wrapping_neg();
            negative = true;
        }
        if x < 0 {
            x = x.wrapping_neg();
            negative = !negative;
        } else if x == 0 {
            return 0;
        }
        if n < 0 {
            n = n.wrapping_neg();
            negative = !negative;
        }
        let mut t = n / d;
        if t > max_answer / x {
            self.arith_error = true;
            return 0;
        }
        let mut a = t * x;
        n -= t * d;
        if n == 0 {
            return if negative { a.wrapping_neg() } else { a };
        }
        t = x / d;
        if t > (max_answer - a) / n {
            self.arith_error = true;
            return 0;
        }
        a += t * n;
        x -= t * d;
        if x == 0 {
            return if negative { a.wrapping_neg() } else { a };
        }
        if x < n {
            std::mem::swap(&mut x, &mut n);
        }
        let mut f = 0;
        let mut r = (d / 2) - d;
        let h = r.wrapping_neg();
        loop {
            if n & 1 != 0 {
                r += x;
                if r >= 0 {
                    r -= d;
                    f += 1;
                }
            }
            n /= 2;
            if n == 0 {
                break;
            }
            if x < h {
                x += x;
            } else {
                t = x - d;
                x += t;
                f += n;
                if x < n {
                    if x == 0 {
                        break;
                    }
                    std::mem::swap(&mut x, &mut n);
                }
            }
        }
        if f > max_answer - a {
            self.arith_error = true;
            return 0;
        }
        a += f;
        if negative {
            a = a.wrapping_neg();
        }
        a
    }

    /// Scans an e-TeX expression (`scan_expr`, C ~8737).
    ///
    /// `cur_val_level` selects the expression level on entry, as assigned
    /// by the `LastItem` dispatch: `INT_VAL` for `\numexpr`, `DIMEN_VAL`
    /// for `\dimexpr`, `GLUE_VAL` for `\glueexpr`, and `MU_VAL` for
    /// `\muexpr`. The result replaces `cur_val`/`cur_val_level`.
    ///
    /// The scan supports `(`...`)` subexpressions, `+`/`-` terms, `*`/`/`
    /// factors, and the fused `EXPR_SCALE` multiply-then-divide rounding;
    /// factor operands after `*`/`/` are always scanned as integers.
    /// Parenthesized state lives in a local `Vec<ExprFrame>` rather than
    /// C's `expr_node`s. The recoverable C diagnostics follow the module
    /// convention: a missing `)` pushes the offending token back and
    /// closes the level, and "Arithmetic overflow" substitutes zero (zero
    /// glue for spec results) while restoring the incoming `arith_error`.
    pub fn scan_expr(&mut self) -> Result<(), TeXMouthError> {
        self.expand_depth_count += 1;
        if self.expand_depth_count >= self.expand_depth {
            self.expand_depth_count -= 1;
            return Err(TeXMouthError::ExpansionDepthExceeded);
        }

        let result = self.scan_expr_inner();
        if result.is_err() {
            self.expand_depth_count -= 1;
        }
        result
    }

    /// Inner `scan_expr` body once the expansion-depth guard has run.
    fn scan_expr_inner(&mut self) -> Result<(), TeXMouthError> {
        let mut l = self.cur_val_level;
        let saved_arith_error = self.arith_error;
        let mut expr_error = false;
        let mut parens: Vec<ExprFrame> = Vec::new();

        let mut r;
        let mut e;
        let mut s;
        let mut t;
        let mut n;
        let mut f;

        'restart: loop {
            r = EXPR_NONE;
            e = 0;
            s = EXPR_NONE;
            t = 0;
            n = 0;

            'operand: loop {
                // C `continue_`: after a `*`/`/`/`EXPR_SCALE` operator the
                // next operand is always an integer factor.
                let operand_level = if s == EXPR_NONE { l } else { INT_VAL };
                loop {
                    self.get_x_token()?;
                    if self.cur_cmd != CommandCode::Spacer as u8 {
                        break;
                    }
                }

                if self.cur_tok == OTHER_LEFT_PAREN {
                    parens.push(ExprFrame {
                        level: l,
                        factor_op: s,
                        term_op: r,
                        term: e,
                        factor: t,
                        scale_numerator: n,
                    });
                    l = operand_level;
                    continue 'restart;
                }

                self.back_input()?;
                match operand_level {
                    INT_VAL => {
                        self.scan_int()?;
                        // scan_int deliberately skips the generic
                        // internal-quantity epilogue; expressions need the
                        // C coercion so that a glue register contributes
                        // its width as an integer factor.
                        self.finish_scan_something_internal(INT_VAL, false)?;
                    }
                    DIMEN_VAL => {
                        self.scan_dimen(false, false, false)?;
                    }
                    GLUE_VAL => {
                        self.scan_normal_glue()?;
                    }
                    _ => {
                        self.scan_mu_glue()?;
                    }
                }
                f = self.cur_val;

                // C `found`:
                'found: loop {
                    loop {
                        self.get_x_token()?;
                        if self.cur_cmd != CommandCode::Spacer as u8 {
                            break;
                        }
                    }

                    let mut o = if self.cur_tok == OTHER_PLUS {
                        EXPR_ADD
                    } else if self.cur_tok == OTHER_MINUS {
                        EXPR_SUB
                    } else if self.cur_tok == OTHER_TIMES {
                        EXPR_MULT
                    } else if self.cur_tok == OTHER_SLASH {
                        EXPR_DIV
                    } else {
                        if parens.is_empty() {
                            // The expression is over: a `\relax` terminator
                            // is consumed, anything else is pushed back.
                            if self.cur_cmd != CommandCode::Relax as u8 {
                                self.back_input()?;
                            }
                        } else if self.cur_tok != OTHER_RIGHT_PAREN {
                            // C: "Missing ) inserted for expression" +
                            // back_error: recover by pushing the token back
                            // and closing the parenthesis level.
                            self.back_input()?;
                        }
                        EXPR_NONE
                    };

                    self.arith_error = expr_error;
                    if l == INT_VAL || s > EXPR_SUB {
                        // `TEX_INFINITY` is `i32::MAX`; only `i32::MIN`
                        // can escape the C bounds check in this port.
                        if f < -TEX_INFINITY {
                            self.arith_error = true;
                            f = 0;
                        }
                    } else if l == DIMEN_VAL {
                        if f.unsigned_abs() > MAX_HALFWORD as u32 {
                            self.arith_error = true;
                            f = 0;
                        }
                    } else {
                        let width = self.mem[(f + 1) as usize].s1;
                        let stretch = self.mem[(f + 2) as usize].s1;
                        let shrink = self.mem[(f + 3) as usize].s1;
                        if width.unsigned_abs() > MAX_HALFWORD as u32
                            || stretch.unsigned_abs() > MAX_HALFWORD as u32
                            || shrink.unsigned_abs() > MAX_HALFWORD as u32
                        {
                            self.arith_error = true;
                            self.delete_glue_ref(f)?;
                            f = self.new_spec(TEX_NULL)?;
                        }
                    }

                    match s {
                        EXPR_NONE => {
                            if l >= GLUE_VAL && o != EXPR_NONE {
                                // The operation will mutate the operand, so
                                // evaluate it in a fresh spec copy.
                                t = self.new_spec(f)?;
                                self.delete_glue_ref(f)?;
                                if self.mem[(t + 2) as usize].s1 == 0 {
                                    self.mem[t as usize].set_b16_s1(NORMAL);
                                }
                                if self.mem[(t + 3) as usize].s1 == 0 {
                                    self.mem[t as usize].set_b16_s0(NORMAL);
                                }
                            } else {
                                t = f;
                            }
                        }
                        EXPR_MULT => {
                            if o == EXPR_DIV {
                                // Fuse `t * f / <next>` into one `fract`.
                                n = f;
                                o = EXPR_SCALE;
                            } else if l == INT_VAL {
                                t = self.mult_and_add(t, f, 0, TEX_INFINITY);
                            } else if l == DIMEN_VAL {
                                t = self.mult_and_add(t, f, 0, MAX_HALFWORD);
                            } else {
                                for offset in 1..=3 {
                                    let part = self.mem[(t + offset) as usize].s1;
                                    self.mem[(t + offset) as usize].s1 =
                                        self.mult_and_add(part, f, 0, MAX_HALFWORD);
                                }
                            }
                        }
                        EXPR_DIV => {
                            if l < GLUE_VAL {
                                t = self.quotient(t, f);
                            } else {
                                for offset in 1..=3 {
                                    let part = self.mem[(t + offset) as usize].s1;
                                    self.mem[(t + offset) as usize].s1 = self.quotient(part, f);
                                }
                            }
                        }
                        EXPR_SCALE => {
                            if l == INT_VAL {
                                t = self.fract(t, n, f, TEX_INFINITY);
                            } else if l == DIMEN_VAL {
                                t = self.fract(t, n, f, MAX_HALFWORD);
                            } else {
                                for offset in 1..=3 {
                                    let part = self.mem[(t + offset) as usize].s1;
                                    self.mem[(t + offset) as usize].s1 =
                                        self.fract(part, n, f, MAX_HALFWORD);
                                }
                            }
                        }
                        _ => unreachable!("scan_expr operator state"),
                    }

                    if o > EXPR_SUB {
                        s = o;
                    } else {
                        s = EXPR_NONE;
                        if r == EXPR_NONE {
                            e = t;
                        } else if l == INT_VAL {
                            e = self.add_or_sub(e, t, TEX_INFINITY, r == EXPR_SUB);
                        } else if l == DIMEN_VAL {
                            e = self.add_or_sub(e, t, MAX_HALFWORD, r == EXPR_SUB);
                        } else {
                            let subtract = r == EXPR_SUB;
                            let e_idx = e as usize;
                            let t_idx = t as usize;
                            self.mem[e_idx + 1].s1 = self.add_or_sub(
                                self.mem[e_idx + 1].s1,
                                self.mem[t_idx + 1].s1,
                                MAX_HALFWORD,
                                subtract,
                            );
                            let e_stretch_order = self.mem[e_idx].b16_s1();
                            let t_stretch_order = self.mem[t_idx].b16_s1();
                            if e_stretch_order == t_stretch_order {
                                self.mem[e_idx + 2].s1 = self.add_or_sub(
                                    self.mem[e_idx + 2].s1,
                                    self.mem[t_idx + 2].s1,
                                    MAX_HALFWORD,
                                    subtract,
                                );
                            } else if e_stretch_order < t_stretch_order
                                && self.mem[t_idx + 2].s1 != 0
                            {
                                self.mem[e_idx + 2].s1 = self.mem[t_idx + 2].s1;
                                self.mem[e_idx].set_b16_s1(t_stretch_order);
                            }
                            let e_shrink_order = self.mem[e_idx].b16_s0();
                            let t_shrink_order = self.mem[t_idx].b16_s0();
                            if e_shrink_order == t_shrink_order {
                                self.mem[e_idx + 3].s1 = self.add_or_sub(
                                    self.mem[e_idx + 3].s1,
                                    self.mem[t_idx + 3].s1,
                                    MAX_HALFWORD,
                                    subtract,
                                );
                            } else if e_shrink_order < t_shrink_order && self.mem[t_idx + 3].s1 != 0
                            {
                                self.mem[e_idx + 3].s1 = self.mem[t_idx + 3].s1;
                                self.mem[e_idx].set_b16_s0(t_shrink_order);
                            }
                            self.delete_glue_ref(t)?;
                            if self.mem[e_idx + 2].s1 == 0 {
                                self.mem[e_idx].set_b16_s1(NORMAL);
                            }
                            if self.mem[e_idx + 3].s1 == 0 {
                                self.mem[e_idx].set_b16_s0(NORMAL);
                            }
                        }
                        r = o;
                    }
                    expr_error = self.arith_error;

                    if o != EXPR_NONE {
                        continue 'operand;
                    }
                    if let Some(frame) = parens.pop() {
                        f = e;
                        e = frame.term;
                        t = frame.factor;
                        n = frame.scale_numerator;
                        s = frame.factor_op;
                        r = frame.term_op;
                        l = frame.level;
                        continue 'found;
                    }

                    self.expand_depth_count -= 1;
                    if expr_error {
                        // C: "Arithmetic overflow" — silent recovery
                        // substitutes zero (the zero glue spec for
                        // glue-valued results).
                        if l >= GLUE_VAL {
                            self.delete_glue_ref(e)?;
                            e = TEX_NULL;
                        } else {
                            e = 0;
                        }
                    }
                    self.arith_error = saved_arith_error;
                    self.cur_val = e;
                    self.cur_val_level = l;
                    return Ok(());
                }
            }
        }
    }

    /// Returns the width word `mem[p + 1].s1` of glue specification `p`.
    fn glue_spec_width(&self, p: i32) -> Result<i32, TeXMouthError> {
        let idx = self.checked_engine_mem_index(p)?;
        self.mem
            .get(idx + 1)
            .map(|word| word.s1)
            .ok_or_else(|| crate::scanner::oob(p))
    }

    /// C `mu_error` ("Incompatible glue units"): a recoverable error that
    /// assumes `1mu = 1pt` and continues scanning. Until the diagnostic
    /// channel exists the recovery is applied silently.
    fn mu_error(&mut self) {}

    /// Applies the `scan_something_internal` epilogue that C runs after its
    /// main dispatch (C ~8047): coerces higher-level results down to
    /// `level` (glue yields its width, mu reports `mu_error`), negates
    /// glue results through a fresh spec copy, and bumps the glue-spec
    /// reference count for non-negated glue/mu results.
    ///
    /// Kept separate from [`TeXEngineState::scan_something_internal`],
    /// which stays a register-fetch scaffold for `scan_int`, so that the
    /// dimension/glue scanners get the exact C coercion and ownership
    /// semantics.
    fn finish_scan_something_internal(
        &mut self,
        level: u8,
        negative: bool,
    ) -> Result<(), TeXMouthError> {
        while self.cur_val_level > level {
            if self.cur_val_level == GLUE_VAL {
                self.cur_val = self.glue_spec_width(self.cur_val)?;
            } else if self.cur_val_level == MU_VAL {
                self.mu_error();
            }
            self.cur_val_level -= 1;
        }

        if negative {
            if self.cur_val_level >= GLUE_VAL {
                let spec = self.new_spec(self.cur_val)?;
                self.mem[(spec + 1) as usize].s1 = -self.mem[(spec + 1) as usize].s1;
                self.mem[(spec + 2) as usize].s1 = -self.mem[(spec + 2) as usize].s1;
                self.mem[(spec + 3) as usize].s1 = -self.mem[(spec + 3) as usize].s1;
                self.cur_val = spec;
            } else {
                self.cur_val = self.cur_val.wrapping_neg();
            }
        } else if (GLUE_VAL..=MU_VAL).contains(&self.cur_val_level) {
            self.add_glue_ref(self.cur_val)?;
        }
        Ok(())
    }
}

/// Rounds the decimal fraction digits collected by `xetex_scan_dimen` into
/// a `scaled_t` fraction of one point (`round_decimals`, C ~8221).
fn round_decimals(mut k: usize, dig: &[i32; 17]) -> i32 {
    let mut a = 0;
    while k > 0 {
        k -= 1;
        a = (a + dig[k] * 0x20000) / 10;
    }
    (a + 1) / 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{CAT_OTHER_CHAR, END_MATCH_TOKEN, INSERTED, SPACE_TOKEN};

    fn state() -> TeXEngineState {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state
    }

    fn scanned(source: &str) -> i32 {
        let mut state = state();
        state.set_source_text(source);
        state.scan_int().unwrap()
    }

    fn next_tok(state: &mut TeXEngineState) -> i32 {
        state.next_expanded_token().unwrap()
    }

    fn letter(character: u8) -> i32 {
        LETTER_TOKEN + i32::from(character)
    }

    fn other(character: u8) -> i32 {
        OTHER_TOKEN + i32::from(character)
    }

    #[test]
    fn scans_plain_decimal_integers() {
        assert_eq!(scanned("1234"), 1234);
        assert_eq!(scanned("0"), 0);
        assert_eq!(scanned("007"), 7);
    }

    #[test]
    fn decimal_scan_pushes_back_a_non_space_terminator() {
        let mut state = state();
        state.set_source_text("12x");
        assert_eq!(state.scan_int().unwrap(), 12);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn decimal_scan_consumes_a_space_terminator() {
        let mut state = state();
        state.set_source_text("12 x");
        assert_eq!(state.scan_int().unwrap(), 12);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn skips_spaces_and_optional_signs() {
        assert_eq!(scanned("  -  42"), -42);
        assert_eq!(scanned("+17"), 17);
        assert_eq!(scanned("+ +17"), 17);
        assert_eq!(scanned("-+3"), -3);
        assert_eq!(scanned("--8"), 8);
        assert_eq!(scanned("- -8"), 8);
        assert_eq!(scanned("- 42"), -42);
    }

    #[test]
    fn octal_prefix_scans_base_eight() {
        let mut state = state();
        state.set_source_text("'17");
        assert_eq!(state.scan_int().unwrap(), 15);
        assert_eq!(state.radix, 8);
        assert_eq!(scanned("'123"), 83);
        assert_eq!(scanned("-'10"), -8);
    }

    #[test]
    fn octal_prefix_without_octal_digits_is_vacuous() {
        let mut state = state();
        state.set_source_text("'9");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(state.radix, 8);
        assert_eq!(next_tok(&mut state), other(b'9'));
    }

    #[test]
    fn hex_prefix_scans_base_sixteen() {
        let mut state = state();
        state.set_source_text("\"FF");
        assert_eq!(state.scan_int().unwrap(), 255);
        assert_eq!(state.radix, 16);
        assert_eq!(scanned("\"10"), 16);
        assert_eq!(scanned("\"7FFFFFFF"), i32::MAX);
    }

    #[test]
    fn hex_digits_accept_other_catcode_letters() {
        let mut state = state();
        state.set_cat_code(u32::from(b'A'), CAT_OTHER_CHAR);
        state.set_cat_code(u32::from(b'B'), CAT_OTHER_CHAR);
        state.set_source_text("\"AB");
        assert_eq!(state.scan_int().unwrap(), 0xAB);
    }

    #[test]
    fn lowercase_letters_are_not_hex_digits() {
        let mut state = state();
        state.set_source_text("\"f");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(next_tok(&mut state), letter(b'f'));
    }

    #[test]
    fn backtick_reads_a_character_constant() {
        assert_eq!(scanned("`a"), 97);
        assert_eq!(scanned("`A"), 65);
        assert_eq!(scanned("-`a"), -97);
        // `` `- `` is the constant for the character `-` itself.
        assert_eq!(scanned("`-"), i32::from(b'-'));
    }

    #[test]
    fn backtick_reads_a_single_character_control_sequence() {
        let mut state = state();
        state.set_source_text("`\\a");
        assert_eq!(state.scan_int().unwrap(), 97);
    }

    #[test]
    fn backtick_rejects_multicharacter_control_sequences() {
        let mut state = state();
        state.set_source_text("`\\ab");
        // "Improper alphabetic constant" recovers with the value of `0`.
        assert_eq!(state.scan_int().unwrap(), i32::from(b'0'));
    }

    #[test]
    fn backtick_brace_constant_preserves_align_state() {
        let mut state = state();
        let align_state = state.align_state;
        state.set_source_text("`{x");
        assert_eq!(state.scan_int().unwrap(), i32::from(b'{'));
        assert_eq!(state.align_state, align_state);
    }

    #[test]
    fn missing_number_recovers_with_zero_and_pushes_back() {
        let mut state = state();
        state.set_source_text("x");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn missing_number_at_end_of_input_recovers_with_zero() {
        let mut state = state();
        state.set_source_text("");
        assert_eq!(state.scan_int().unwrap(), 0);
    }

    #[test]
    fn missing_number_after_sign_recovers_with_zero() {
        let mut state = state();
        state.set_source_text("-x");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn oversized_numbers_saturate_to_tex_infinity() {
        assert_eq!(scanned("99999999999"), TEX_INFINITY);
        assert_eq!(scanned("2147483648"), TEX_INFINITY);
        assert_eq!(scanned("\"FFFFFFFF"), TEX_INFINITY);
        assert_eq!(scanned("\"80000000"), TEX_INFINITY);
        assert_eq!(scanned("'20000000000"), TEX_INFINITY);
        // Boundary values that exactly fit are not saturated.
        assert_eq!(scanned("2147483647"), TEX_INFINITY);
        assert_eq!(scanned("'17777777777"), TEX_INFINITY);
        // Saturation continues absorbing digits without further error.
        assert_eq!(scanned("99999999999999999999"), TEX_INFINITY);
        assert_eq!(scanned("-99999999999"), -TEX_INFINITY);
    }

    #[test]
    fn scan_optional_equals_consumes_an_equals_sign() {
        let mut state = state();
        state.set_source_text(" =x");
        state.scan_optional_equals().unwrap();
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn scan_optional_equals_restores_a_non_equals_token() {
        let mut state = state();
        state.set_source_text("x");
        state.scan_optional_equals().unwrap();
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn keyword_matching_consumes_the_keyword() {
        let mut state = state();
        state.set_source_text("ptx");
        assert!(state.scan_keyword("pt").unwrap());
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn keyword_matching_is_case_insensitive() {
        let mut state = state();
        state.set_source_text("PT");
        assert!(state.scan_keyword("pt").unwrap());
    }

    #[test]
    fn keyword_mismatch_restores_all_scanned_tokens() {
        let mut state = state();
        state.set_source_text("px");
        assert!(!state.scan_keyword("pt").unwrap());
        assert_eq!(next_tok(&mut state), letter(b'p'));
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn keyword_mismatch_at_first_character_only_pushes_back() {
        let mut state = state();
        state.set_source_text("tp");
        assert!(!state.scan_keyword("pt").unwrap());
        assert_eq!(next_tok(&mut state), letter(b't'));
        assert_eq!(next_tok(&mut state), letter(b'p'));
    }

    #[test]
    fn keyword_scans_skip_spaces_before_the_first_character() {
        let mut state = state();
        state.set_source_text("  pt x");
        assert!(state.scan_keyword("pt").unwrap());
        assert_eq!(next_tok(&mut state), SPACE_TOKEN);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn keyword_mismatch_on_an_interior_space_restores_input() {
        // In the C semantics, spaces are only skipped before the first
        // matched character; a space inside the keyword fails the match and
        // the entire scanned text is restored.
        let mut state = state();
        state.set_source_text("p t");
        assert!(!state.scan_keyword("pt").unwrap());
        assert_eq!(next_tok(&mut state), letter(b'p'));
        assert_eq!(next_tok(&mut state), SPACE_TOKEN);
        assert_eq!(next_tok(&mut state), letter(b't'));
    }

    #[test]
    fn left_brace_scans_a_real_brace() {
        let mut state = state();
        state.set_source_text("{x");
        state.scan_left_brace().unwrap();
        assert_eq!(state.cur_cmd, CommandCode::LeftBrace as u8);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn left_brace_skips_spaces_and_relax() {
        let mut state = state();
        state.set_source_text(" \\relax {x");
        state.scan_left_brace().unwrap();
        assert_eq!(state.cur_cmd, CommandCode::LeftBrace as u8);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn left_brace_fabricates_a_missing_brace() {
        let mut state = state();
        state.set_source_text("x{");
        state.scan_left_brace().unwrap();
        assert_eq!(state.cur_cmd, CommandCode::LeftBrace as u8);
        assert_eq!(state.cur_tok, LEFT_BRACE_TOKEN + i32::from(b'{'));
        // The offending token was pushed back for re-reading.
        assert_eq!(next_tok(&mut state), letter(b'x'));
        assert_eq!(next_tok(&mut state), LEFT_BRACE_TOKEN + i32::from(b'{'));
    }

    #[test]
    fn bounded_wrappers_accept_in_range_values() {
        assert_eq!(scanned_bounded("255", |s| s.scan_eight_bit_int()), 255);
        assert_eq!(scanned_bounded("0", |s| s.scan_eight_bit_int()), 0);
        assert_eq!(scanned_bounded("1114111", |s| s.scan_usv_num()), 0x10FFFF);
        assert_eq!(scanned_bounded("65535", |s| s.scan_char_num()), 0xFFFF);
        assert_eq!(scanned_bounded("15", |s| s.scan_four_bit_int()), 15);
        assert_eq!(
            scanned_bounded("32767", |s| s.scan_fifteen_bit_int()),
            0x7FFF
        );
        assert_eq!(scanned_bounded("7", |s| s.scan_math_class_int()), 7);
        assert_eq!(scanned_bounded("255", |s| s.scan_math_fam_int()), 255);
        assert_eq!(
            scanned_bounded("134217727", |s| s.scan_delimiter_int()),
            0x7FF_FFFF
        );
        assert_eq!(scanned_bounded("32767", |s| s.scan_register_num()), 32_767);
        assert_eq!(scanned_bounded("18", |s| s.scan_four_bit_int_or_18()), 18);
    }

    #[test]
    fn bounded_wrappers_reject_out_of_range_values() {
        assert_bounded_err("256", |s| s.scan_eight_bit_int(), 256);
        assert_bounded_err("-1", |s| s.scan_eight_bit_int(), -1);
        assert_bounded_err("1114112", |s| s.scan_usv_num(), 0x110000);
        assert_bounded_err("65536", |s| s.scan_char_num(), 65536);
        assert_bounded_err("16", |s| s.scan_four_bit_int(), 16);
        assert_bounded_err("32768", |s| s.scan_fifteen_bit_int(), 32768);
        assert_bounded_err("8", |s| s.scan_math_class_int(), 8);
        assert_bounded_err("256", |s| s.scan_math_fam_int(), 256);
        assert_bounded_err("134217728", |s| s.scan_delimiter_int(), 0x800_0000);
        assert_bounded_err("32768", |s| s.scan_register_num(), 32_768);
        assert_bounded_err("17", |s| s.scan_four_bit_int_or_18(), 17);
        assert_bounded_err("19", |s| s.scan_four_bit_int_or_18(), 19);
    }

    fn scanned_bounded<F>(source: &str, f: F) -> i32
    where
        F: FnOnce(&mut TeXEngineState) -> Result<i32, TeXMouthError>,
    {
        let mut state = state();
        state.set_source_text(source);
        f(&mut state).unwrap()
    }

    fn assert_bounded_err<F>(source: &str, f: F, expected: i32)
    where
        F: FnOnce(&mut TeXEngineState) -> Result<i32, TeXMouthError>,
    {
        let mut state = state();
        state.set_source_text(source);
        match f(&mut state) {
            Err(TeXMouthError::InputOutOfBounds(value)) => assert_eq!(value, expected),
            other => panic!("expected InputOutOfBounds({expected}), got {other:?}"),
        }
    }

    #[test]
    fn internal_count_register_reads_eqtb_value() {
        let mut state = state();
        state.set_count(7, 99).unwrap();
        state.set_source_text("\\count7");
        assert_eq!(state.scan_int().unwrap(), 99);
        assert_eq!(state.cur_val_level, INT_VAL);
    }

    #[test]
    fn internal_count_register_negates_like_a_number() {
        let mut state = state();
        state.set_count(7, 99).unwrap();
        state.set_source_text("-\\count7");
        assert_eq!(state.scan_int().unwrap(), -99);
    }

    #[test]
    fn internal_dimen_register_coerces_to_int_level() {
        let mut state = state();
        state.set_dimen(3, 65_536).unwrap();
        state.set_source_text("\\dimen3");
        assert_eq!(state.scan_int().unwrap(), 65_536);
        assert_eq!(state.cur_val_level, INT_VAL);
    }

    #[test]
    fn primitive_relookup_reaches_an_internal_register() {
        let mut state = state();
        state.set_count(0, 42).unwrap();
        state.set_source_text("\\primitive\\count0");
        assert_eq!(state.scan_int().unwrap(), 42);
    }

    #[test]
    fn last_penalty_and_last_kern_read_the_list_tail() {
        let mut penalty_state = state();
        penalty_state.mode = TeXMode::Hmode as i16;
        let penalty = penalty_state.new_penalty(7).unwrap();
        penalty_state.mem[penalty_state.tail as usize].s1 = penalty;
        penalty_state.tail = penalty;
        penalty_state.define_cs("lastpenalty", CommandCode::LastItem, 0);
        penalty_state.set_source_text("\\lastpenalty");
        assert_eq!(penalty_state.scan_int().unwrap(), 7);

        let mut kern_state = state();
        kern_state.mode = TeXMode::Hmode as i16;
        let kern = kern_state.new_kern(3 * 65_536).unwrap();
        kern_state.mem[kern_state.tail as usize].s1 = kern;
        kern_state.tail = kern;
        kern_state.define_cs("lastkern", CommandCode::LastItem, 1);
        kern_state.set_source_text("\\lastkern");
        assert_eq!(
            kern_state.scan_dimen(false, false, false).unwrap(),
            3 * 65_536
        );
    }

    #[test]
    fn last_values_use_page_metadata_on_the_empty_outer_vlist() {
        let mut state = state();
        state.mode = TeXMode::Vmode as i16;
        state.tail = state.head;
        state.last_penalty = 9;
        state.last_kern = 4 * 65_536;
        state.define_cs("lastpenalty", CommandCode::LastItem, 0);
        state.set_source_text("\\lastpenalty");
        assert_eq!(state.scan_int().unwrap(), 9);

        state.define_cs("lastkern", CommandCode::LastItem, 1);
        state.set_source_text("\\lastkern");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 4 * 65_536);
    }

    #[test]
    fn get_x_or_protected_expands_unprotected_macros() {
        let mut state = state();
        state
            .define_simple_macro("a", &[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        state.set_source_text("\\a");
        state.get_x_or_protected().unwrap();
        assert_eq!(state.cur_tok, letter(b'x'));
    }

    #[test]
    fn get_x_or_protected_stops_at_protected_macros() {
        let mut state = state();
        state
            .define_macro(
                "p",
                &[PROTECTED_TOKEN, END_MATCH_TOKEN],
                &[LETTER_TOKEN + i32::from(b'x')],
                CommandCode::Call,
            )
            .unwrap();
        let p_cs = state.cs_table.lookup_str("p").unwrap();
        state.set_source_text("\\p");
        state.get_x_or_protected().unwrap();
        assert_eq!(state.cur_tok, CS_TOKEN_FLAG + p_cs);
        assert_eq!(state.cur_cmd, CommandCode::Call as u8);
    }

    #[test]
    fn scan_int_inside_inserted_token_lists() {
        let mut state = state();
        let head = state.store_token_list(&[other(b'4'), other(b'2')]).unwrap();
        state.begin_token_list(head, INSERTED).unwrap();
        assert_eq!(state.scan_int().unwrap(), 42);
        // The end-of-list relax token is pushed back, so one BACKED_UP frame
        // remains until it is read again.
        assert_eq!(state.input_ptr, 1);
    }

    #[test]
    fn cs_token_after_number_is_pushed_back() {
        let mut state = state();
        state.set_source_text("5\\relax");
        assert_eq!(state.scan_int().unwrap(), 5);
        let relax = state.cs_table.lookup_str("relax").unwrap();
        assert_eq!(next_tok(&mut state), CS_TOKEN_FLAG + relax);
    }

    // =====================================================================
    // Dimension and glue scanning (Phase 5 Milestone 3 packet 2)
    // =====================================================================

    use crate::state::{FILL, GLUE_SPEC_SIZE};

    fn glue_state() -> TeXEngineState {
        state()
    }

    fn scanned_dimen(source: &str) -> i32 {
        let mut state = state();
        state.set_source_text(source);
        state.scan_dimen(false, false, false).unwrap()
    }

    fn glue_spec(state: &mut TeXEngineState, width: i32, stretch: i32, shrink: i32) -> i32 {
        let p = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[(p + 1) as usize].s1 = width;
        state.mem[(p + 2) as usize].s1 = stretch;
        state.mem[(p + 3) as usize].s1 = shrink;
        p
    }

    #[test]
    fn scans_whole_dimensions() {
        assert_eq!(scanned_dimen("10pt"), 10 * 65536);
        assert_eq!(scanned_dimen("0pt"), 0);
        assert_eq!(scanned_dimen("16383pt"), 16383 * 65536);
        assert_eq!(scanned_dimen("2 pt"), 2 * 65536);
    }

    #[test]
    fn scans_fractional_dimensions() {
        assert_eq!(scanned_dimen("1.5pt"), 98304);
        assert_eq!(scanned_dimen(".5pt"), 32768);
        assert_eq!(scanned_dimen("0.25pt"), 16384);
        assert_eq!(scanned_dimen("0.1pt"), 6554);
        assert_eq!(scanned_dimen("10.pt"), 10 * 65536);
        // Continental decimal point.
        assert_eq!(scanned_dimen("1,5pt"), 98304);
    }

    #[test]
    fn fraction_digits_beyond_seventeen_are_dropped() {
        let mut a = state();
        a.set_source_text("0.12345678901234567pt");
        let first = a.scan_dimen(false, false, false).unwrap();
        let mut b = state();
        b.set_source_text("0.12345678901234567888888pt");
        let second = b.scan_dimen(false, false, false).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn physical_unit_conversions_match_tex() {
        assert_eq!(scanned_dimen("1pt"), 65536);
        assert_eq!(scanned_dimen("1in"), 4736286);
        assert_eq!(scanned_dimen("1pc"), 786432);
        assert_eq!(scanned_dimen("1cm"), 1864679);
        assert_eq!(scanned_dimen("1mm"), 186467);
        assert_eq!(scanned_dimen("1bp"), 65781);
        assert_eq!(scanned_dimen("1dd"), 70124);
        assert_eq!(scanned_dimen("1cc"), 841489);
        assert_eq!(scanned_dimen("1sp"), 1);
        // Fractions combine with unit ratios before rounding.
        assert_eq!(scanned_dimen("1.5in"), 7104430);
    }

    #[test]
    fn sp_discards_the_decimal_fraction() {
        // C jumps to `done` for `sp`, skipping the fraction attach.
        assert_eq!(scanned_dimen("1.9sp"), 1);
        assert_eq!(scanned_dimen("0.5sp"), 0);
        assert_eq!(scanned_dimen("2.7sp"), 2);
    }

    #[test]
    fn dimension_signs() {
        assert_eq!(scanned_dimen("-1.5pt"), -98304);
        assert_eq!(scanned_dimen("+3pt"), 196608);
        assert_eq!(scanned_dimen("--2pt"), 131072);
        assert_eq!(scanned_dimen("- -1pt"), 65536);
        assert_eq!(scanned_dimen(" - 1.5pt "), -98304);
    }

    #[test]
    fn nondecimal_radix_has_no_fraction() {
        assert_eq!(scanned_dimen("'20pt"), 16 * 65536);
        assert_eq!(scanned_dimen("\"10pt"), 16 * 65536);
    }

    #[test]
    fn internal_dimen_supplies_the_dimension() {
        let mut state = state();
        state.set_dimen(0, 327_680).unwrap();
        state.set_source_text("\\dimen0");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 327_680);
    }

    #[test]
    fn internal_dimen_respects_scanned_sign() {
        let mut a = state();
        a.set_dimen(0, 327_680).unwrap();
        a.set_source_text("-\\dimen0");
        assert_eq!(a.scan_dimen(false, false, false).unwrap(), -327_680);

        let mut b = state();
        b.set_dimen(0, -327_680).unwrap();
        b.set_source_text("-\\dimen0");
        assert_eq!(b.scan_dimen(false, false, false).unwrap(), 327_680);
    }

    #[test]
    fn internal_count_is_treated_as_a_decimal() {
        let mut a = state();
        a.set_count(0, 5).unwrap();
        a.set_source_text("\\count0pt");
        assert_eq!(a.scan_dimen(false, false, false).unwrap(), 5 * 65536);

        // With no unit, the C engine recovers silently with `pt`.
        let mut b = state();
        b.set_count(0, 5).unwrap();
        b.set_source_text("\\count0");
        assert_eq!(b.scan_dimen(false, false, false).unwrap(), 5 * 65536);
    }

    #[test]
    fn internal_skip_coerces_to_its_width() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 42, 1, 2);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\skip0");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 42);
        // Coercion leaves the register-owned spec untouched.
        assert_eq!(state.mem[spec as usize].s1, TEX_NULL);
    }

    #[test]
    fn internal_muskip_in_dimen_context_recovers_silently() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 42, 0, 0);
        state.set_mu_skip(0, spec).unwrap();
        state.set_source_text("\\muskip0");
        // mu_error recovery still yields the mu-glue width, as in C.
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 42);
    }

    #[test]
    fn internal_dimen_as_unit_multiplies_the_value() {
        let mut a = state();
        a.set_dimen(0, 98_304).unwrap();
        a.set_source_text("2\\dimen0");
        assert_eq!(a.scan_dimen(false, false, false).unwrap(), 196_608);

        let mut b = state();
        b.set_dimen(0, 65_536).unwrap();
        b.set_source_text("1.5\\dimen0");
        assert_eq!(b.scan_dimen(false, false, false).unwrap(), 98_304);
    }

    #[test]
    fn unit_scan_consumes_one_optional_space() {
        let mut a = state();
        a.set_source_text("1ptx");
        assert_eq!(a.scan_dimen(false, false, false).unwrap(), 65536);
        assert_eq!(next_tok(&mut a), letter(b'x'));

        let mut b = state();
        b.set_source_text("1pt x");
        assert_eq!(b.scan_dimen(false, false, false).unwrap(), 65536);
        assert_eq!(next_tok(&mut b), letter(b'x'));

        // A run of spaces collapses into one SPACE_TOKEN at lex time; the
        // `done` step consumes it and `x` remains.
        let mut c = state();
        c.set_source_text("1pt  x");
        assert_eq!(c.scan_dimen(false, false, false).unwrap(), 65536);
        assert_eq!(next_tok(&mut c), letter(b'x'));
    }

    #[test]
    fn missing_unit_recovers_as_pt_and_restores_input() {
        let mut state = state();
        state.set_source_text("5x");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 5 * 65536);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn true_unit_cancels_magnification() {
        // C §476: `true` locks \mag and scales the dimension by 1000/mag,
        // so `2truept` at \mag=2000 is one physical point.
        let mut a = state();
        a.set_source_text("2truept");
        a.eqtb[crate::state::INT_PAR_BASE + INT_PAR_MAG].s1 = 2000;
        assert_eq!(a.scan_dimen(false, false, false).unwrap(), 65536);
        assert_eq!(a.mag_set, 2000);

        // Fractions scale too: `1truept` at \mag=2000 is exactly 0.5pt.
        let mut b = state();
        b.set_source_text("1truept");
        b.eqtb[crate::state::INT_PAR_BASE + INT_PAR_MAG].s1 = 2000;
        assert_eq!(b.scan_dimen(false, false, false).unwrap(), 32768);

        // mag == 1000 leaves the value untouched.
        assert_eq!(scanned_dimen("1truept"), 65536);
        assert_eq!(scanned_dimen("2.5truein"), scanned_dimen("2.5in"));
    }

    #[test]
    fn em_and_ex_scale_by_the_current_font() {
        let mut a = state();
        a.set_source_text("1em");
        a.eqtb[CUR_FONT_LOC].s1 = 1;
        a.font_table.push(crate::state::FontRecord {
            params: vec![0, 0, 0, 0, 0, 0, 10 * 65536, 0, 0],
            ..Default::default()
        });
        assert_eq!(a.scan_dimen(false, false, false).unwrap(), 10 * 65536);

        let mut b = state();
        b.set_source_text("1ex");
        b.eqtb[CUR_FONT_LOC].s1 = 1;
        b.font_table.push(crate::state::FontRecord {
            params: vec![0, 0, 0, 0, 0, 6 * 65536, 0, 0, 0],
            ..Default::default()
        });
        assert_eq!(b.scan_dimen(false, false, false).unwrap(), 6 * 65536);
    }

    #[test]
    fn dimension_overflow_recovers_with_max_halfword() {
        assert_eq!(scanned_dimen("16384pt"), MAX_HALFWORD);
        assert_eq!(scanned_dimen("-16384pt"), -MAX_HALFWORD);
        assert_eq!(scanned_dimen("20000pt"), MAX_HALFWORD);
        assert_eq!(scanned_dimen("99999999pt"), MAX_HALFWORD);
        // Just below the overflow point stays exact.
        assert_eq!(scanned_dimen("16383.9999pt"), 16383 * 65536 + 65529);
    }

    #[test]
    fn mu_dimensions_accept_mu() {
        let mut a = state();
        a.set_source_text("1mu");
        assert_eq!(a.scan_dimen(true, false, false).unwrap(), 65536);

        let mut b = state();
        b.set_source_text("1.5mu");
        assert_eq!(b.scan_dimen(true, false, false).unwrap(), 98304);

        let mut c = state();
        c.set_source_text("-2mu");
        assert_eq!(c.scan_dimen(true, false, false).unwrap(), -131072);
    }

    #[test]
    fn mu_dimensions_recover_from_missing_mu() {
        // "Illegal unit of measure (mu inserted)" recovers silently and
        // pushes the offending letters back for re-reading.
        let mut state = state();
        state.set_source_text("1pt");
        assert_eq!(state.scan_dimen(true, false, false).unwrap(), 65536);
        assert_eq!(next_tok(&mut state), letter(b'p'));
        assert_eq!(next_tok(&mut state), letter(b't'));
    }

    #[test]
    fn mu_internal_muskip_supplies_the_width() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 77, 0, 0);
        state.set_mu_skip(0, spec).unwrap();
        state.set_source_text("\\muskip0");
        assert_eq!(state.scan_dimen(true, false, false).unwrap(), 77);
    }

    #[test]
    fn mu_internal_count_scans_mu_units() {
        let mut state = state();
        state.set_count(0, 3).unwrap();
        state.set_source_text("\\count0mu");
        assert_eq!(state.scan_dimen(true, false, false).unwrap(), 3 * 65536);
    }

    #[test]
    fn skip_in_mu_dimension_recovers_as_decimal_width() {
        // C quirk: a \skip width scanned in a mu context falls through
        // mu_error into the units scan, so the sp width is re-scaled as a
        // decimal integer part.
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 42, 0, 0);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\skip0");
        assert_eq!(state.scan_dimen(true, false, false).unwrap(), 42 * 65536);
    }

    #[test]
    fn scan_decimal_needs_no_units() {
        let mut a = state();
        a.set_source_text("1.5pt");
        assert_eq!(a.scan_decimal().unwrap(), 98304);
        // The unit letters are left in the input.
        assert_eq!(next_tok(&mut a), letter(b'p'));

        let mut b = state();
        b.set_source_text("-2.5");
        assert_eq!(b.scan_decimal().unwrap(), -163840);

        let mut c = state();
        c.set_source_text("16384");
        assert_eq!(c.scan_decimal().unwrap(), MAX_HALFWORD);
    }

    #[test]
    fn normal_glue_width_only() {
        let mut state = glue_state();
        state.set_source_text("1pt");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 65536);
        assert_eq!(state.mem[(q + 2) as usize].s1, 0);
        assert_eq!(state.mem[(q + 3) as usize].s1, 0);
        assert_eq!(state.mem[q as usize].b16_s1(), 0);
        assert_eq!(state.mem[q as usize].b16_s0(), 0);
        assert_eq!(state.mem[q as usize].s1, TEX_NULL);
    }

    #[test]
    fn normal_glue_plus_and_minus() {
        let mut state = glue_state();
        state.set_source_text("1pt plus 2pt minus 3pt");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 65536);
        assert_eq!(state.mem[(q + 2) as usize].s1, 131072);
        assert_eq!(state.mem[(q + 3) as usize].s1, 196608);
        assert_eq!(state.mem[q as usize].b16_s1(), 0);
        assert_eq!(state.mem[q as usize].b16_s0(), 0);
    }

    #[test]
    fn glue_fil_orders() {
        for (source, order) in [
            ("0pt plus 1fil", FIL),
            ("0pt plus 1fill", FILL),
            ("0pt plus 1filll", FILLL),
            // A fourth `l` recovers silently at FILLL, as in C.
            ("0pt plus 1fillll", FILLL),
        ] {
            let mut state = glue_state();
            state.set_source_text(source);
            let q = state.scan_normal_glue().unwrap();
            assert_eq!(state.mem[(q + 2) as usize].s1, 65536, "{source}");
            assert_eq!(state.mem[q as usize].b16_s1(), order, "{source}");
        }
    }

    #[test]
    fn glue_fil_order_on_shrink() {
        let mut state = glue_state();
        state.set_source_text("0pt minus 1fill");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 3) as usize].s1, 65536);
        assert_eq!(state.mem[q as usize].b16_s0(), FILL);
    }

    #[test]
    fn glue_signs_negate_the_width() {
        let mut state = glue_state();
        state.set_source_text("-1pt plus -2pt");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, -65536);
        assert_eq!(state.mem[(q + 2) as usize].s1, -131072);
    }

    #[test]
    fn internal_glue_returns_the_spec() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 65536, 100, 200);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\skip0");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(q, spec);
        // The scan took a reference for the caller.
        assert_eq!(state.mem[spec as usize].s1, 1);
    }

    #[test]
    fn negated_internal_glue_copies_the_spec() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 65536, 100, 200);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("-\\skip0");
        let q = state.scan_normal_glue().unwrap();
        assert_ne!(q, spec);
        assert_eq!(state.mem[(q + 1) as usize].s1, -65536);
        assert_eq!(state.mem[(q + 2) as usize].s1, -100);
        assert_eq!(state.mem[(q + 3) as usize].s1, -200);
        // The original spec is untouched.
        assert_eq!(state.mem[(spec + 1) as usize].s1, 65536);
        assert_eq!(state.mem[spec as usize].s1, TEX_NULL);
    }

    #[test]
    fn internal_dimen_becomes_glue_width() {
        let mut state = glue_state();
        state.set_dimen(0, 123_456).unwrap();
        state.set_source_text("\\dimen0 plus 1pt");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 123_456);
        assert_eq!(state.mem[(q + 2) as usize].s1, 65536);
    }

    #[test]
    fn internal_count_scans_a_dimension_for_glue_width() {
        let mut state = glue_state();
        state.set_count(0, 7).unwrap();
        state.set_source_text("\\count0pt");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 7 * 65536);
    }

    #[test]
    fn mu_glue_scans_mu_units() {
        let mut state = glue_state();
        state.set_source_text("1mu plus 2mu minus 3mu");
        let q = state.scan_mu_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 65536);
        assert_eq!(state.mem[(q + 2) as usize].s1, 131072);
        assert_eq!(state.mem[(q + 3) as usize].s1, 196608);
    }

    #[test]
    fn mu_glue_recovers_from_non_mu_units() {
        let mut state = glue_state();
        state.set_source_text("1pt");
        let q = state.scan_mu_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 65536);
    }

    #[test]
    fn internal_muskip_returns_the_spec() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 1000, 50, 60);
        state.set_mu_skip(0, spec).unwrap();
        state.set_source_text("\\muskip0");
        let q = state.scan_mu_glue().unwrap();
        assert_eq!(q, spec);
        assert_eq!(state.mem[spec as usize].s1, 1);
    }

    #[test]
    fn skip_in_mu_glue_recovers_silently() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 1000, 50, 60);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\skip0");
        // mu_error recovery still returns the spec, as in C.
        let q = state.scan_mu_glue().unwrap();
        assert_eq!(q, spec);
    }

    #[test]
    fn glue_keywords_leave_the_following_token() {
        let mut state = glue_state();
        state.set_source_text("1ptx");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(state.mem[(q + 1) as usize].s1, 65536);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn expr_arithmetic_matches_c_reference() {
        // Differential vectors produced by the verbatim C `add_or_sub`,
        // `quotient`, and `fract` (xetex-xetex0.c ~8588-8735), including
        // the etrip.tex rounding edge cases.
        const CASES: &[(char, i32, i32, i32, i32, bool)] = &[
            ('q', 5, 2, 0, 3, false),
            ('q', 3, 2, 0, 2, false),
            ('q', -5, 2, 0, -3, false),
            ('q', 5, -2, 0, -3, false),
            ('q', -5, -2, 0, 3, false),
            ('q', 1, 0, 0, 0, true),
            ('q', 0, 0, 0, 0, true),
            ('q', -1, 0, 0, 0, true),
            ('q', 1073741823, 2147483647, 0, 0, false),
            ('q', 1073741824, 2147483647, 0, 1, false),
            ('q', 1073741822, 2147483646, 0, 0, false),
            ('q', 1073741823, 2147483646, 0, 1, false),
            ('q', 32, 5, 0, 6, false),
            ('q', 33, 5, 0, 7, false),
            ('q', 25, 4, 0, 6, false),
            ('q', 26, 4, 0, 7, false),
            ('q', 7, 3, 0, 2, false),
            ('q', -7, 3, 0, -2, false),
            ('q', 7, -3, 0, -2, false),
            ('q', -7, -3, 0, 2, false),
            ('q', 2147483647, 1, 0, 2147483647, false),
            ('q', 2147483647, 2147483647, 0, 1, false),
            ('f', 2, 3, 4, 2, false),
            ('f', 1, 2, 3, 1, false),
            ('f', 7, 3, 4, 5, false),
            ('f', 0, 5, 7, 0, false),
            ('f', 1, 2147483647, 2, 1073741824, false),
            ('f', 1073741822, 2147483646, 2147483645, 1073741822, false),
            ('f', 1073741822, 2147483646, 2147483647, 1073741822, false),
            ('f', 536854526, 536887294, 536870910, 536870909, false),
            ('f', 536854527, 536887295, 536870911, 536870910, false),
            ('f', 536854528, 536887296, 536870912, 536870912, false),
            ('f', 536854529, 536887297, 536870913, 536870913, false),
            ('f', 5, 7, 0, 0, true),
            ('f', -5, 7, 3, -12, false),
            ('f', 5, -7, 3, -12, false),
            ('f', 5, 7, -3, -12, false),
            ('f', -5, -7, -3, -12, false),
            ('f', 1, 1, 2147483647, 0, false),
            ('a', 5, 3, 2147483647, 8, false),
            ('s', 5, 3, 2147483647, 2, false),
            ('a', 2147483646, 1, 2147483647, 2147483647, false),
            ('s', 2147483646, 1, 2147483647, 2147483645, false),
            ('a', 2147483646, 2, 2147483647, 0, true),
            ('s', 2147483646, 2, 2147483647, 2147483644, false),
            ('a', -2147483646, -1, 2147483647, -2147483647, false),
            ('s', -2147483646, -1, 2147483647, -2147483645, false),
            ('a', -2147483646, -2, 2147483647, 0, true),
            ('s', -2147483646, -2, 2147483647, -2147483644, false),
            ('a', 0, 2147483647, 2147483647, 2147483647, false),
            ('s', 0, 2147483647, 2147483647, -2147483647, false),
            ('a', 2147483647, 0, 2147483647, 2147483647, false),
            ('s', 2147483647, 0, 2147483647, 2147483647, false),
            ('a', -2147483647, -1, 2147483647, 0, true),
            ('s', -2147483647, -1, 2147483647, -2147483646, false),
            ('a', 1073741823, 1, 1073741823, 0, true),
            ('s', 1073741823, 1, 1073741823, 1073741822, false),
            ('a', -1073741823, -1, 1073741823, 0, true),
            ('s', -1073741823, -1, 1073741823, -1073741822, false),
            ('g', -595578497, -2040063278, 1037451516, 0, true),
            ('f', -21939312, -1188800801, -1548172822, -16846615, false),
            ('g', -633528409, 758918510, 1033660084, -465139792, false),
            ('g', -700342398, 388662378, -335343206, 811696009, false),
            ('q', 5, -2147483646, 0, 0, false),
            ('q', -345659740, 100, 0, -3456597, false),
            ('q', -332368623, 5, 0, -66473725, false),
            ('f', -634752019, -1749143278, -515708290, 0, true),
            ('q', -65536, 1319105919, 0, 0, false),
            ('g', 899472998, 155598470, -1611359367, -86856244, false),
            ('f', -735297560, 223512818, -888292542, 185016109, false),
            ('g', 479468784, 332224960, -1321610451, -120528328, false),
            ('g', -775004718, -1950669414, 692620698, 0, true),
            ('q', 2084010572, 1574035378, 0, 1, false),
            ('f', 120145721, -200101228, 582760240, -41254198, false),
            ('f', -375147799, -557568503, -621607596, -336499422, false),
            ('q', 579727331, -1096028725, 0, -1, false),
            ('f', 556006903, -988066572, 2102486759, -261296216, false),
            ('g', -130501850, 792912176, -754699904, 137109473, false),
            ('q', 1, -424493989, 0, 0, false),
            ('q', 2147483646, -795952425, 0, -3, false),
            ('g', 1070440113, -448257583, 1652201550, -290420317, false),
            ('g', 897011881, -1533854844, -1009831969, 0, true),
            ('q', -2147483647, 358771184, 0, -6, false),
            ('f', -131766344, 2135997925, 2136907761, -131710242, false),
            ('q', -1676544203, -1874634224, 0, 1, false),
            ('f', 936517122, 125045162, -1067668243, -109684760, false),
            ('g', -1024431216, 774311336, 947993018, -836745301, false),
            ('q', 605426323, -1073741823, 0, -1, false),
            ('q', -1073741822, 1772223087, 0, -1, false),
            ('g', 208068776, 1467458680, 596783528, 511629958, false),
            ('g', -219425143, -1491043970, -541535891, -604156699, false),
            ('q', -755243973, -65536, 0, 11524, false),
            ('f', -45301961, -1898696933, -1112948054, -77285453, false),
            ('g', -735482873, -1779604886, 996035536, 0, true),
            ('f', -776476344, 2052927745, 1119277766, -1424177160, false),
            ('g', -533604528, -1596046498, 686124557, 0, true),
            ('f', -364526933, -1009074108, 118857988, 0, true),
            ('g', 743621791, 1994423489, -1237817310, 0, true),
            ('g', -209804577, 914608871, -808672357, 237289090, false),
            ('f', 530087025, -265858142, 1716329420, -82110083, false),
            ('g', 865376399, -1627774568, -1082737122, 0, true),
            ('q', 0, -1201657162, 0, 0, false),
            ('q', -1894622752, 1, 0, -1894622752, false),
            ('f', -769411821, 60800081, -1125261522, 41572826, false),
            ('f', 1011097575, -1227343557, 168508468, 0, true),
            ('q', 327326531, -1073741824, 0, 0, false),
            ('f', -255937441, -1742357419, -1731168980, -257591549, false),
            ('g', 777608915, -625787168, -328227310, 0, true),
            ('f', 932113843, 1562666925, 983873307, 1480458370, false),
            ('q', -1724782098, 65536, 0, -26318, false),
            ('q', 5, -1545405260, 0, 0, false),
            ('q', 7, -1823708014, 0, 0, false),
            ('g', -653227035, -1930208423, 653457337, 0, true),
            ('g', -1010357336, 2014323588, -1746923080, 0, true),
            ('q', -1073741824, 1073741823, 0, -1, false),
            ('q', 0, -193237574, 0, 0, false),
            ('f', 1016495993, -1482636328, -1331878515, 1131555070, false),
            ('f', -138723604, 2013091399, -1896299817, 147267479, false),
            ('g', -811931061, 1065046940, -800561221, 0, true),
            ('q', -1073741824, -1471315227, 0, 1, false),
            ('g', -729665709, 1509004753, -1349371497, 815986573, false),
            ('q', -1137290602, 2147483646, 0, -1, false),
            ('q', -1795367596, -1027503518, 0, 2, false),
            ('g', 275667610, -1122335266, -1006677998, 307339071, false),
            ('f', -511623899, 737403901, 624921129, -603713719, false),
            ('f', 890022773, -789512949, 1842619708, -381350802, false),
            ('q', -2147483646, 2130718029, 0, -1, false),
            ('q', 25274572, -65536, 0, -386, false),
            ('q', -560376878, -2147483647, 0, 0, false),
            ('g', 225559398, 479652333, 2078134141, 52061168, false),
            ('g', -1040141840, 720966962, 1361951899, -550612619, false),
            ('g', 212080106, 1854378079, 701749192, 560423445, false),
            ('q', -1570707697, 2, 0, -785353849, false),
            ('g', -406098134, -977756966, -937349668, -423604225, false),
            ('g', -169094355, 934637224, -674824021, 234197174, false),
            ('q', 1515749002, -1929304049, 0, -1, false),
            ('g', 745502259, 1414539293, -959151116, 0, true),
            ('q', 6, 100, 0, 0, false),
            ('q', -2105952602, 1073741822, 0, -2, false),
            ('q', 1437074667, -1511425673, 0, -1, false),
            ('q', -65536, 1549205809, 0, 0, false),
            ('f', -171469972, 781970487, -1075761592, 124641425, false),
            ('g', -632246589, -628445493, 1203090109, 330259983, false),
            ('g', 671635775, 2035114501, 518237227, 0, true),
            ('g', -409940291, 1828641690, 1846324918, -406014077, false),
            ('q', 1334754394, -2041027014, 0, -1, false),
            ('g', 352983878, 1212840745, 1850340138, 231370017, false),
            ('f', -7871646, -1001561844, -1463734126, -5386183, false),
            ('g', -609474649, -504479461, 1598444177, 192354195, false),
            ('q', -1192138111, -170496300, 0, 7, false),
            ('f', -96226630, -1190034280, -2045845668, -55973425, false),
            ('g', -244255689, -436085295, -737609299, -144407499, false),
            ('f', -775581968, 2005273425, 1173928983, -1324827934, false),
            ('f', 434419313, 607659446, 40397068, 0, true),
            ('f', 348536024, 1886509178, -2028940239, -324068889, false),
            ('q', -1025572916, 2076833914, 0, 0, false),
            ('q', -1073741823, -6, 0, 178956971, false),
            ('f', 800555633, 456163542, 2079777136, 175588185, false),
            ('g', -577046951, -493081884, 1715859754, 165824390, false),
            ('g', -257359699, -1053458803, -1956844949, -138548453, false),
            ('g', 799110965, -2140219676, 85557237, 0, true),
            ('g', -227482374, -583164329, -295075420, -449578640, false),
            ('q', 1807197061, 1718188083, 0, 1, false),
            ('g', 254625686, 716799562, -393449450, -463885717, false),
            ('f', 654554678, 847003687, -877654239, -631695491, false),
            ('g', -527044874, -1323536713, -341680385, 0, true),
            ('g', 554674751, 761574765, 1065729807, 396372786, false),
            ('q', -854913766, -2145740149, 0, 0, false),
            ('f', -171014079, -301128119, 1227324828, 41958858, false),
            ('g', 310308090, -150374582, -250027163, 186629520, false),
            ('f', -155901286, 47991951, -115164477, 64968010, false),
            ('g', -344922843, 682258777, -1783289591, 131962099, false),
            ('f', 365880795, -1746388693, 1367618341, -467213742, false),
            ('q', -1292227096, 7, 0, -184603871, false),
            ('f', -760946960, -191486107, -367473075, -396520945, false),
            ('g', -238648852, 937788546, 843335080, -265377506, false),
            ('f', 1049591957, -430987676, -1099578443, 411395113, false),
            ('q', -321494637, 1073741822, 0, 0, false),
            ('g', 921484758, -1931797339, 246556026, 0, true),
            ('q', -1626252426, 1073741824, 0, -2, false),
            ('g', 289042514, 1932905380, 1096290451, 509620265, false),
            ('f', 759095772, 1419783764, 1747427770, 616764750, false),
            ('g', 841442534, 1705995834, -1463905655, -980594243, false),
            ('g', 964969352, -214448742, -1034264416, 200080811, false),
            ('q', -1119930471, 1073741822, 0, -1, false),
            ('f', -66599564, -980476134, -705029447, -92619228, false),
            ('f', -727663413, -1553171576, -1499670889, -753622770, false),
            ('q', -2, -1871622831, 0, 0, false),
            ('f', 347382914, 183013468, -146353989, -434397123, false),
            ('f', -806312612, -1259136707, 1429756687, 710091316, false),
            ('f', 599048331, 1738768002, 1158764032, 898894029, false),
            ('g', -989852730, 1532152409, 1634907942, -927639536, false),
            ('g', 559982260, -98871289, -2122160187, 26089533, false),
            ('f', 208789758, 1088667390, -472503750, -481059888, false),
            ('f', -126535348, -50553990, -1205074880, -5308273, false),
            ('f', 798159454, -61667136, -2022822981, 24332434, false),
            ('f', 369923566, 725306680, 769408532, 348719857, false),
            ('f', -364880110, 1462460681, -140058243, 0, true),
            ('q', 146559900, 700078631, 0, 0, false),
            ('q', -164503907, 1073741823, 0, 0, false),
            ('f', -164667812, -194564902, -743691590, -43080461, false),
            ('f', 554592868, -952380583, 1082373947, -487986135, false),
            ('f', 9756141, 1438431441, -1795699058, -7815085, false),
            ('f', -990491258, 1069551049, 169303551, 0, true),
            ('q', -644415421, 651781283, 0, -1, false),
            ('q', -1493006453, -1073741822, 0, 1, false),
            ('q', -1211333061, 1146318621, 0, -1, false),
            ('q', 937837180, -811581658, 0, -1, false),
            ('g', -963453853, 1842056375, -807637476, 0, true),
            ('g', 538189289, -443865820, 1896101766, -125986819, false),
            ('g', -221921911, -1821081273, 395557301, 1021692269, false),
            ('g', -30711663, -1709820883, 846937489, 62001557, false),
            ('f', -553814295, 1272286199, 2078095668, -339065374, false),
            ('g', -897351234, -656254839, 140639769, 0, true),
            ('f', 517847999, -1851316726, 25601399, 0, true),
            ('g', 391834924, -2093143089, 1501415886, -546262079, false),
            ('f', 1031658477, -1694197660, -285514830, 0, true),
            ('f', 900823637, 890383865, -1490391665, -538166477, false),
            ('f', -317239351, 1004143171, 93401555, 0, true),
            ('g', 86253222, 497839748, 1323705553, 32439452, false),
            ('g', 1002864159, -150865988, -276705456, 546783914, false),
            ('g', 79008247, -763196031, 1510596513, -39917198, false),
            ('q', -1775269340, -519736981, 0, 3, false),
            ('q', 65536, 5, 0, 13107, false),
            ('f', 35873339, -685739449, -946359606, 25994097, false),
            ('g', 112822673, 239620644, -2103884949, -12849867, false),
            ('g', -253160718, -1779778776, -1110867311, -405602063, false),
            ('g', 671853667, -49057767, 236980005, -139081948, false),
            ('q', -39422075, -1747822546, 0, 0, false),
            ('q', 7, 2147483646, 0, 0, false),
            ('f', 958896499, 229664583, 132808694, 1658208947, false),
            ('f', 753783133, 2133015904, 1056284647, 1522157323, false),
            ('g', 346995995, -636468950, 871255356, -253487310, false),
            ('f', 89835362, -830547928, -1067713078, 69880734, false),
            ('q', 5, -1073741822, 0, 0, false),
            ('g', -278529286, -1324821431, -1218118693, -302927432, false),
            ('g', 1005875266, -959926785, 964585220, -1001017422, false),
            ('g', 141728333, 2063777468, -1715727297, -170479155, false),
            ('q', -65536, 0, 0, 0, true),
            ('g', -530072148, -969325444, -1951995206, -263224222, false),
            ('q', -39175696, 318103419, 0, 0, false),
            ('f', 982320042, -255684544, -684162786, 367111537, false),
            ('q', 1553588300, -1, 0, -1553588300, false),
            ('f', 1038293585, -1829303144, 330803335, 0, true),
            ('g', -843492607, -1495821100, -1506684916, -837410679, false),
            ('g', 231221790, -1781634457, 2121362197, -194192538, false),
            ('q', 6, -2147483646, 0, 0, false),
            ('g', -104422966, 1184094738, 96948300, 0, true),
            ('f', 861203055, 1754512958, -246011409, 0, true),
            ('f', 768068295, -835885498, 294761316, 0, true),
            ('g', -815054640, 470861171, 1976152136, -194204472, false),
            ('g', -647536890, 1922385263, 1128447276, 0, true),
            ('q', -1798786124, -2147483647, 0, 1, false),
            ('q', -1073741823, -2147483646, 0, 1, false),
            ('f', 177207312, -2007296516, -1153300794, 308425713, false),
            ('f', 140524338, 871824910, 1544499152, 79321907, false),
            ('f', -768027088, 804812110, -1144936056, 539870762, false),
            ('f', -224114766, -321525670, -1654523196, -43552517, false),
            ('g', -108192602, 634086307, -1507579071, 45505704, false),
            ('f', -462841624, -1840780721, -1891340208, -450468898, false),
            ('q', 1809008150, 1073741824, 0, 2, false),
            ('g', 232319877, 857849634, -418722723, -475960607, false),
            ('f', 1047092493, -267323622, -1801988519, 155335372, false),
            ('g', -902541060, 1672558589, -291934657, 0, true),
            ('g', 310618823, 445446717, -1072161681, -129051558, false),
            ('q', -1073741822, 1936112340, 0, -1, false),
            ('g', 83799214, 327478316, -1974689920, -13897081, false),
            ('q', 1073741822, 100, 0, 10737418, false),
            ('q', 1349671772, -128804364, 0, -10, false),
            ('f', 357806846, -768548930, 730717877, -376331382, false),
            ('q', -730987686, 1912091710, 0, 0, false),
            ('g', -916128833, -194021230, -1769269722, -100464299, false),
            ('f', 10302978, -759033310, -1649604173, 4740715, false),
            ('f', -1068798037, 677146187, 1586421260, -456204499, false),
            ('g', 910431485, -372598829, -1914678873, 177171070, false),
            ('q', -1925812740, -2, 0, 962906370, false),
            ('f', 807952454, 1635723746, 851477397, 1552110507, false),
            ('f', -552220746, -2023490351, 2032150093, 549867529, false),
            ('g', -45967044, 901010379, -1467254602, 28227401, false),
            ('f', -1015010448, 224418103, -395176966, 576416995, false),
            ('q', 2, 2, 0, 1, false),
            ('g', -412440882, -6977155, 1858230749, 1548604, false),
            ('g', 179794030, 38105013, 882236239, 7765555, false),
            ('f', 710775595, 1437628385, -75163749, 0, true),
            ('f', -26925565, -185600698, 220181825, 22696713, false),
            ('q', -2147483646, 1629193869, 0, -1, false),
            ('q', -6, -2136465943, 0, 0, false),
            ('g', 208545791, 1449448037, 373394670, 809535625, false),
            ('g', 1029247430, 1568782326, -1509271197, -1069831042, false),
            ('f', 1006488594, -665187589, -719664917, 930299234, false),
            ('g', 546363031, -191793419, 1859086291, -56365772, false),
            ('f', -263486064, 2064911300, 848249311, -641409718, false),
            ('q', 1183454511, -1073741822, 0, -1, false),
            ('f', 563633492, -489288044, 2097722157, -131465994, false),
            ('g', -420454449, -20063312, 1996405775, 4225448, false),
            ('q', 2147483646, 2, 0, 1073741823, false),
            ('f', -645483342, 111459045, 1762833600, -40812109, false),
            ('f', -1007824392, 955388964, -1528340849, 630006260, false),
            ('f', -410765425, -1826098755, -131036879, 0, true),
            ('f', 380367601, 529395096, 827659965, 243294047, false),
            ('f', -728790156, 1510820448, -736305202, 1495400368, false),
            ('g', 558386099, 1950514700, -787434349, 0, true),
            ('g', 1021976645, 1599372881, 176162446, 0, true),
            ('q', -1759307363, -1715808338, 0, 1, false),
            ('q', 1732693177, -6, 0, -288782196, false),
            ('q', -65536, 7, 0, -9362, false),
            ('g', 696006947, 283158876, 780826644, 252399872, false),
            ('q', -1, 786557035, 0, 0, false),
            ('f', -459055886, -1150241886, -166859844, 0, true),
            ('g', 17082549, -173643390, -1050863797, 2822699, false),
            ('g', -1033380173, 1713299089, 1308389089, 0, true),
            ('f', 161626761, 762832944, 200137491, 616047585, false),
            ('q', -65536, 1984399750, 0, 0, false),
            ('g', 421874, -185388796, 1483069083, -52736, false),
            ('f', -218111340, 2144793139, -495238750, 944602387, false),
            ('f', -615669506, -1128692911, -509536470, -1363792089, false),
            ('g', 467976017, 321314814, -876721408, -171511298, false),
            ('g', 194264624, -2053253794, 2052755915, -194311741, false),
            ('g', 626230909, -968599371, -2112703147, 287104634, false),
            ('g', -863098397, 1763586383, 457012414, 0, true),
            ('g', 1059685621, 1429532190, 1735833735, 872695740, false),
            ('f', -85390850, 459840649, 1296787882, -30279573, false),
            ('f', -133124506, 586769316, -1330872964, 58693337, false),
            ('g', 2692076, 763492542, 1097696238, 1872449, false),
            ('g', -486279482, 550821438, -1730167747, 154813407, false),
            ('g', -904423138, -820627039, 1239572597, 598749991, false),
            ('f', -930442260, 341766885, -580416590, 547872611, false),
            ('g', -509332401, -1760363753, 1759338075, 509629337, false),
            ('f', 329506138, 1062922215, -362911455, -965082224, false),
            ('q', -6, 2025032331, 0, 0, false),
            ('g', 96754238, 1419369114, -1440770725, -95317024, false),
            ('f', 995778842, 2007399317, 1316358062, 1518527386, false),
            ('f', 381029116, 1306668829, -1652833516, -301227476, false),
            ('f', -750333527, -1542936867, 1091572327, 1060596016, false),
            ('q', 2061336227, -576491749, 0, -4, false),
            ('f', -1013961635, -1011642699, 157076890, 0, true),
            ('q', -1073741822, -552016509, 0, 2, false),
            ('g', -69484386, -122505910, -2040003601, -4172663, false),
            ('g', 335034411, 1783510552, 472799455, 0, true),
            ('q', 863619870, -1646364311, 0, -1, false),
            ('q', -31677023, -2147483647, 0, 0, false),
            ('q', -65536, 1939270199, 0, 0, false),
            ('q', -1214317458, 7, 0, -173473923, false),
            ('f', -647016966, 2065568828, 797490669, -1675829108, false),
            ('f', 466750616, -1887422986, -437486898, 2013673656, false),
            ('f', -258520277, 2010933217, -1622811650, 320349569, false),
            ('f', -680007477, 699086411, -1236496101, 384460563, false),
            ('g', -982012247, -1930250596, 1231773404, 0, true),
            ('f', -27621405, 2099469808, -1606644698, 36094045, false),
            ('g', -192390586, -1852805295, 1416556805, 251639959, false),
            ('g', -184007696, 371012539, -1219979648, 55959263, false),
            ('q', -2, 412237996, 0, 0, false),
            ('f', -452311688, 2093913847, -1589171690, 595971922, false),
            ('g', 2809756, 1282107110, -1398024216, -2576785, false),
            ('q', -2036758735, -6, 0, 339459789, false),
            ('q', -2079704402, -2, 0, 1039852201, false),
            ('g', 734270014, 112036493, -1659363606, -49576257, false),
            ('g', -800762139, -102022807, -222156265, -367741154, false),
            ('f', -605358420, -206135140, 16487063, 0, true),
            ('a', 494380126, -851991239, 2147483647, -357611113, false),
            ('a', 2141413822, 1491635899, 1073741823, 0, true),
            ('a', 1759651651, -1800491172, 2147483647, -40839521, false),
            ('s', -1605494900, 911189322, 1073741823, 0, true),
            ('s', 452804629, 866636000, 2147483647, -413831371, false),
            ('a', -1166111212, -199720695, 1073741823, 0, true),
            ('s', -805098699, -1052740308, 2147483647, 247641609, false),
            ('s', -1859148270, 1424949418, 2147483647, 0, true),
            ('s', 1025621009, 1339038909, 2147483647, -313417900, false),
            ('a', 730201433, -383073991, 1073741823, 347127442, false),
            ('a', 2011756773, 431258068, 1073741823, 0, true),
            ('a', -1233777435, 673177283, 2147483647, -560600152, false),
            ('a', 241658369, 243695420, 1073741823, 485353789, false),
            ('a', -1372602505, -687947847, 2147483647, -2060550352, false),
            ('a', 1588548237, -426127975, 2147483647, 1162420262, false),
            ('s', 130922145, -531740981, 1073741823, 662663126, false),
            ('a', -1471415966, 413277980, 2147483647, -1058137986, false),
            ('s', 506111148, 747189948, 1073741823, -241078800, false),
            ('a', 695521401, -248918329, 1073741823, 446603072, false),
            ('s', -1658641178, -301018648, 1073741823, 0, true),
            ('a', -822832447, -1200364161, 2147483647, -2023196608, false),
            ('s', -1269493001, 1912652385, 2147483647, 0, true),
            ('s', -904317695, -1632154794, 1073741823, 727837099, false),
            ('a', -1392880208, 874761438, 1073741823, -518118770, false),
            ('a', 1039559589, -273934733, 1073741823, 765624856, false),
            ('s', 1555250675, 325383120, 1073741823, 0, true),
            ('a', -904025294, -407883133, 1073741823, 0, true),
            ('s', 507344141, 1802897353, 2147483647, -1295553212, false),
            ('a', 1479549057, -1102447351, 1073741823, 377101706, false),
            ('a', -689285197, -1425359139, 2147483647, -2114644336, false),
            ('a', -646928295, 1353073662, 2147483647, 706145367, false),
            ('a', -1518510510, -1300207711, 2147483647, 0, true),
            ('a', -25267175, 534816967, 1073741823, 509549792, false),
            ('s', -258716709, -133482921, 1073741823, -125233788, false),
            ('s', 392644864, -1828837185, 1073741823, 0, true),
            ('s', -1985961738, -914497171, 2147483647, -1071464567, false),
            ('s', 1700277782, 652136920, 1073741823, 1048140862, false),
            ('a', 5872531, 1965902578, 1073741823, 0, true),
            ('a', 1449023965, 627611078, 2147483647, 2076635043, false),
            ('s', 891728429, 2108768926, 2147483647, -1217040497, false),
            ('a', -932464961, 420416603, 1073741823, -512048358, false),
            ('a', -731472615, -487203029, 1073741823, 0, true),
            ('a', -715704584, -645014804, 1073741823, 0, true),
            ('a', 1881205138, 1252678590, 2147483647, 0, true),
            ('a', -593443800, -590138519, 2147483647, -1183582319, false),
            ('a', 1143132222, -1322827448, 2147483647, -179695226, false),
            ('a', -1816062124, -529484127, 2147483647, 0, true),
            ('s', -267690657, -382859338, 2147483647, 115168681, false),
            ('a', 475003718, 1218913257, 2147483647, 1693916975, false),
            ('s', 1439301063, 2080653064, 1073741823, -641352001, false),
            ('a', 1194533636, 449526090, 1073741823, 0, true),
            ('a', 606359331, 1606820046, 2147483647, 0, true),
            ('s', -923147312, 1130078186, 1073741823, 0, true),
            ('s', -268553293, -2032936045, 2147483647, 1764382752, false),
            ('s', 8620488, -1134824537, 2147483647, 1143445025, false),
            ('s', -1183097007, -1520206653, 2147483647, 337109646, false),
            ('a', 1422108722, -132670977, 2147483647, 1289437745, false),
            ('a', -2065862572, -329618285, 1073741823, 0, true),
            ('s', -854049094, -1942401337, 1073741823, 1088352243, false),
            ('a', -1877728975, 154849842, 2147483647, -1722879133, false),
            ('a', -896479988, -632150298, 1073741823, 0, true),
            ('s', 2038625504, 232580889, 1073741823, 0, true),
            ('a', -544162090, 1062216692, 1073741823, 518054602, false),
            ('a', -666568196, 550859464, 1073741823, -115708732, false),
            ('s', -1957063326, 197627521, 2147483647, 0, true),
            ('s', -1568296209, 1905269630, 2147483647, 0, true),
            ('s', -1281656920, 1250304549, 2147483647, 0, true),
            ('s', 1861314196, 66072007, 1073741823, 0, true),
            ('s', 630932863, -918646707, 2147483647, 1549579570, false),
            ('s', -974027239, 1285665884, 1073741823, 0, true),
            ('s', 1044998636, -486857001, 2147483647, 1531855637, false),
            ('a', 824927872, -324271476, 1073741823, 500656396, false),
            ('s', 195311052, -767891053, 2147483647, 963202105, false),
            ('a', 691142804, -831093958, 1073741823, -139951154, false),
            ('s', 815033642, -637600836, 2147483647, 1452634478, false),
            ('s', -279428413, -1901132619, 1073741823, 1621704206, false),
            ('a', -2013735696, -265545695, 2147483647, 0, true),
            ('a', 623469823, -592058724, 2147483647, 31411099, false),
            ('s', 769065673, 1927682399, 1073741823, -1158616726, false),
            ('s', 1129939210, 317491631, 1073741823, 812447579, false),
            ('a', -950520197, 1977918577, 2147483647, 1027398380, false),
            ('s', 264882283, -230755226, 1073741823, 495637509, false),
            ('s', -1287724220, -1829432550, 2147483647, 541708330, false),
            ('a', 2135725782, 2089822264, 1073741823, 0, true),
            ('a', -1184373238, 2083628439, 2147483647, 899255201, false),
            ('s', 1247142795, -2034594605, 1073741823, 0, true),
            ('s', 221821912, -1777190967, 1073741823, 0, true),
            ('a', 940038026, 110362013, 1073741823, 1050400039, false),
            ('a', -1828947321, 74560688, 2147483647, -1754386633, false),
            ('a', -1593291303, 1581456902, 2147483647, -11834401, false),
            ('s', -841278666, -1451840846, 2147483647, 610562180, false),
            ('a', -1808782935, -968725435, 1073741823, 0, true),
            ('a', -1485224315, 451568175, 2147483647, -1033656140, false),
            ('s', -1798634140, 309826932, 2147483647, -2108461072, false),
            ('s', 195753796, -849999299, 2147483647, 1045753095, false),
            ('s', -870014893, -1890159112, 1073741823, 1020144219, false),
            ('s', -67640877, -1271211478, 2147483647, 1203570601, false),
            ('s', -435008844, 1119547616, 2147483647, -1554556460, false),
            ('a', -1109227431, -1621256049, 1073741823, 0, true),
            ('s', -1542418790, -1930135569, 1073741823, 387716779, false),
            ('s', -804383296, 1130097708, 1073741823, 0, true),
            ('s', -822312376, -2047942188, 1073741823, 1225629812, false),
            ('s', 2004403830, -631810759, 2147483647, 0, true),
            ('s', -126328512, 157585579, 2147483647, -283914091, false),
            ('a', -415520579, -1953169028, 2147483647, 0, true),
            ('a', -907948781, 214134587, 2147483647, -693814194, false),
            ('s', 2040465156, -455680065, 1073741823, 0, true),
            ('a', 1095067079, 39180897, 2147483647, 1134247976, false),
            ('s', -1980349435, 917369995, 1073741823, 0, true),
            ('s', -1547128869, -779407583, 1073741823, -767721286, false),
            ('a', 402544325, 270257647, 1073741823, 672801972, false),
            ('s', -565398905, 746492796, 1073741823, 0, true),
            ('s', -109134201, 333437191, 2147483647, -442571392, false),
            ('a', 1465301660, 776216097, 2147483647, 0, true),
            ('a', -1328071177, -253237418, 1073741823, 0, true),
            ('a', -1271775546, 976684579, 2147483647, -295090967, false),
            ('s', 261506693, -529033565, 2147483647, 790540258, false),
            ('a', 830274337, 472775319, 2147483647, 1303049656, false),
            ('a', -865439500, 2071292195, 1073741823, 1205852695, false),
            ('s', -47231400, -86743647, 1073741823, 39512247, false),
            ('s', 2062273817, 1367420791, 2147483647, 694853026, false),
            ('s', -1966809467, -1808054813, 1073741823, -158754654, false),
            ('s', 1062310754, -1228417286, 1073741823, 0, true),
            ('a', -1557900783, 372934167, 2147483647, -1184966616, false),
            ('a', 133322198, 592467428, 2147483647, 725789626, false),
            ('a', -372367794, 933545753, 2147483647, 561177959, false),
            ('a', 1842969114, 1625214013, 1073741823, 0, true),
            ('a', -913775592, 1154165071, 2147483647, 240389479, false),
            ('s', -1395586145, 1802317805, 1073741823, 0, true),
            ('s', -1986567622, 441449484, 1073741823, 0, true),
            ('a', 104447297, 18093304, 1073741823, 122540601, false),
            ('s', 1326345515, -727206245, 1073741823, 0, true),
            ('s', 938872031, -1317862219, 1073741823, 0, true),
            ('a', 65629940, -1774947749, 2147483647, -1709317809, false),
            ('a', 2068822154, -96833898, 2147483647, 1971988256, false),
            ('s', -1027419413, -1135927572, 1073741823, 108508159, false),
            ('a', -2098367538, 660065040, 2147483647, -1438302498, false),
            ('s', 720635895, -148627391, 2147483647, 869263286, false),
            ('s', 1158958488, 817290975, 1073741823, 341667513, false),
            ('s', 492312552, -1573810096, 2147483647, 2066122648, false),
            ('a', 454145328, 875168854, 2147483647, 1329314182, false),
            ('s', 23566751, -1503872551, 1073741823, 0, true),
            ('s', 357116546, 44561798, 2147483647, 312554748, false),
            ('s', -363248690, 274496477, 1073741823, -637745167, false),
            ('a', 958980671, -446896888, 2147483647, 512083783, false),
            ('a', 336344323, -1952965427, 1073741823, -1616621104, false),
            ('s', 1693155328, -119953270, 1073741823, 0, true),
            ('s', 1991930971, 1680765050, 1073741823, 311165921, false),
            ('a', -534098345, 168793352, 1073741823, -365304993, false),
            ('a', 160023979, 123877783, 2147483647, 283901762, false),
            ('s', 1100282239, 2055101844, 1073741823, -954819605, false),
            ('s', 137153822, 1388822523, 2147483647, -1251668701, false),
            ('a', -660738896, -963577067, 2147483647, -1624315963, false),
            ('a', -2080285459, -1262369814, 2147483647, 0, true),
            ('a', 1598640183, -1957488334, 1073741823, -358848151, false),
            ('s', 170767078, 1185684228, 2147483647, -1014917150, false),
            ('s', 326891419, 1367442385, 2147483647, -1040550966, false),
            ('s', 1616017209, 91272607, 1073741823, 0, true),
            ('a', -965706538, 767725944, 1073741823, -197980594, false),
            ('s', 2028471131, 735910543, 1073741823, 0, true),
            ('s', -394023826, -859472371, 2147483647, 465448545, false),
            ('a', -1179636182, 128813862, 1073741823, -1050822320, false),
            ('a', -1052285690, -553717937, 2147483647, -1606003627, false),
            ('a', -994449918, 442046836, 2147483647, -552403082, false),
            ('a', -1215404784, 1748192677, 1073741823, 532787893, false),
            ('a', 1037128112, 1092342212, 2147483647, 2129470324, false),
            ('s', -931753645, 638753308, 1073741823, 0, true),
            ('s', 1644164800, -146011398, 1073741823, 0, true),
            ('a', 1779535963, -555318123, 1073741823, 0, true),
            ('a', -1520750366, -353999486, 2147483647, -1874749852, false),
            ('s', -730723027, 1431781953, 1073741823, 0, true),
            ('s', 1381226854, -40391665, 2147483647, 1421618519, false),
            ('a', -1420492934, -50181011, 2147483647, -1470673945, false),
            ('a', -2120065327, 561359299, 2147483647, -1558706028, false),
            ('a', -2108221359, 24622689, 1073741823, 0, true),
            ('a', -52164999, 221807399, 2147483647, 169642400, false),
            ('a', 2050758184, 747465853, 2147483647, 0, true),
            ('a', 1795506116, 104734506, 2147483647, 1900240622, false),
            ('s', -625170715, 1266741896, 2147483647, -1891912611, false),
            ('a', 1972669012, 847123522, 1073741823, 0, true),
            ('s', 1114912747, -1745902047, 1073741823, 0, true),
            ('s', 226350962, -1411396799, 1073741823, 0, true),
            ('a', 2035126219, 1800245058, 2147483647, 0, true),
            ('s', 286835712, -983139256, 2147483647, 1269974968, false),
            ('a', -2006171553, -796782171, 2147483647, 0, true),
            ('a', -1817039636, -722714829, 2147483647, 0, true),
            ('a', 2037203192, -1916859217, 1073741823, 120343975, false),
            ('s', 1454596580, 1174909417, 2147483647, 279687163, false),
            ('s', -212385307, -1167636724, 1073741823, 955251417, false),
            ('a', -515083747, -1831774843, 1073741823, 0, true),
            ('a', -474069987, -598287940, 2147483647, -1072357927, false),
            ('a', 1902499705, -237357639, 1073741823, 0, true),
            ('s', -1578370391, -1342642050, 1073741823, -235728341, false),
            ('s', -1703843897, -1762573662, 2147483647, 58729765, false),
            ('s', 1562157671, -2094481098, 1073741823, 0, true),
            ('a', 1199022207, 90643288, 1073741823, 0, true),
            ('a', -1551170121, -854134711, 1073741823, 0, true),
            ('s', -1875753293, -280563279, 1073741823, 0, true),
            ('s', -1817108903, 537015594, 2147483647, 0, true),
            ('s', -1688040803, -838987497, 1073741823, -849053306, false),
        ];
        for &(op, a, b, c, expected, error) in CASES {
            let mut state = TeXEngineState::new(128);
            state.arith_error = false;
            let got = match op {
                'a' => state.add_or_sub(a, b, c, false),
                's' => state.add_or_sub(a, b, c, true),
                'q' => state.quotient(a, b),
                'f' => state.fract(a, b, c, TEX_INFINITY),
                'g' => state.fract(a, b, c, MAX_HALFWORD),
                _ => unreachable!(),
            };
            assert_eq!(
                (got, state.arith_error),
                (expected, error),
                "{op} {a} {b} {c}"
            );
        }
    }

    // =====================================================================
    // e-TeX expression scanning (Phase 5 Milestone 3 packet 3)
    // =====================================================================

    fn scanned_numexpr(source: &str) -> i32 {
        let mut state = state();
        state.set_source_text(source);
        state.scan_int().unwrap()
    }

    fn scanned_dimexpr(source: &str) -> i32 {
        let mut state = state();
        state.set_source_text(source);
        state.scan_dimen(false, false, false).unwrap()
    }

    fn spec_parts(state: &TeXEngineState, q: i32) -> (i32, i32, i32, u16, u16) {
        (
            state.mem[(q + 1) as usize].s1,
            state.mem[(q + 2) as usize].s1,
            state.mem[(q + 3) as usize].s1,
            state.mem[q as usize].b16_s1(),
            state.mem[q as usize].b16_s0(),
        )
    }

    fn scanned_glueexpr(source: &str) -> (i32, i32, i32, u16, u16) {
        let mut state = glue_state();
        state.set_source_text(source);
        let q = state.scan_normal_glue().unwrap();
        spec_parts(&state, q)
    }

    fn scanned_muexpr(source: &str) -> (i32, i32, i32, u16, u16) {
        let mut state = glue_state();
        state.set_source_text(source);
        let q = state.scan_mu_glue().unwrap();
        spec_parts(&state, q)
    }

    #[test]
    fn numexpr_scans_a_plain_integer() {
        assert_eq!(scanned_numexpr("\\numexpr 5\\relax"), 5);
        // A missing terminator runs to end-of-input, which yields the
        // synthetic `\relax` the C engine also sees.
        assert_eq!(scanned_numexpr("\\numexpr 42"), 42);
    }

    #[test]
    fn numexpr_applies_tex_precedence() {
        assert_eq!(scanned_numexpr("\\numexpr 2+3*4\\relax"), 14);
        assert_eq!(scanned_numexpr("\\numexpr (2+3)*4\\relax"), 20);
        assert_eq!(scanned_numexpr("\\numexpr 2*(3+4)\\relax"), 14);
        assert_eq!(scanned_numexpr("\\numexpr 10-4-3\\relax"), 3);
        assert_eq!(scanned_numexpr("\\numexpr 2+3-1\\relax"), 4);
        assert_eq!(scanned_numexpr("\\numexpr 8/2/2\\relax"), 2);
        assert_eq!(scanned_numexpr("\\numexpr 8/(2/2)\\relax"), 8);
        assert_eq!(scanned_numexpr("\\numexpr 100/10/5\\relax"), 2);
        assert_eq!(scanned_numexpr("\\numexpr 100/(10/5)\\relax"), 50);
    }

    #[test]
    fn numexpr_scans_signed_and_nested_operands() {
        assert_eq!(scanned_numexpr("\\numexpr -2+5\\relax"), 3);
        assert_eq!(scanned_numexpr("\\numexpr 2*-3\\relax"), -6);
        // C's scan_int consumes the leading minus, finds no digit, and
        // pushes the parenthesis back during missing-number recovery.
        let mut paren_state = state();
        paren_state.set_source_text("\\numexpr -(2+3)\\relax");
        assert_eq!(paren_state.scan_int().unwrap(), 0);
        assert_eq!(next_tok(&mut paren_state), other(b'('));
        assert_eq!(scanned_numexpr("\\numexpr ((2+3)*(4-1))\\relax"), 15);
        assert_eq!(scanned_numexpr("\\numexpr 2*(3+(4*5))\\relax"), 46);
        // Spaces may appear between operands and operators.
        assert_eq!(scanned_numexpr("\\numexpr  2 +  3 * ( 1 + 1 ) \\relax"), 8);
    }

    #[test]
    fn numexpr_consumes_a_relax_terminator() {
        let mut state = state();
        state.set_source_text("\\numexpr 2+3\\relax x");
        assert_eq!(state.scan_int().unwrap(), 5);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn numexpr_pushes_back_a_non_operator_terminator() {
        let mut state = state();
        state.set_source_text("\\numexpr 2+3x");
        assert_eq!(state.scan_int().unwrap(), 5);
        assert_eq!(next_tok(&mut state), letter(b'x'));
    }

    #[test]
    fn numexpr_pushes_back_a_stray_right_paren() {
        let mut state = state();
        state.set_source_text("\\numexpr 2+3)x");
        assert_eq!(state.scan_int().unwrap(), 5);
        assert_eq!(next_tok(&mut state), other(b')'));
    }

    #[test]
    fn numexpr_recovers_from_a_missing_right_paren() {
        // C reports "Missing ) inserted for expression" and pushes the
        // offending token back; the port performs the same recovery.
        let mut state = state();
        state.set_source_text("\\numexpr (2+3x");
        assert_eq!(state.scan_int().unwrap(), 5);
        assert_eq!(next_tok(&mut state), letter(b'x'));

        // `\relax` inside parentheses both closes the level and
        // terminates the expression, as in C.
        assert_eq!(scanned_numexpr("\\numexpr (2+3\\relax x"), 5);
    }

    #[test]
    fn numexpr_recovers_from_empty_operand() {
        // `\numexpr\relax` scans a missing number, recovers with 0, and
        // the terminator ends the expression.
        assert_eq!(scanned_numexpr("\\numexpr\\relax"), 0);
    }

    #[test]
    fn numexpr_overflow_recovers_with_zero() {
        let mut state = state();
        state.set_source_text("\\numexpr \"7FFFFFFE+2\\relax");
        assert_eq!(state.scan_int().unwrap(), 0);
        assert!(!state.arith_error);

        assert_eq!(
            scanned_numexpr("\\numexpr \"7FFFFFFE+1\\relax"),
            0x7FFF_FFFF
        );
        assert_eq!(scanned_numexpr("\\numexpr -\"80000000-1\\relax"), 0);
        assert_eq!(scanned_numexpr("\\numexpr \"10000*\"8000\\relax"), 0);
        // Sub-expression overflows propagate out of the parens.
        assert_eq!(scanned_numexpr("\\numexpr (\"7FFFFFFE+2)*1\\relax"), 0);
    }

    #[test]
    fn numexpr_division_by_zero_recovers_with_zero() {
        assert_eq!(scanned_numexpr("\\numexpr 1/0\\relax"), 0);
        assert_eq!(scanned_numexpr("\\numexpr 0/0\\relax"), 0);
        assert_eq!(scanned_numexpr("\\numexpr 5*3/0\\relax"), 0);
    }

    #[test]
    fn numexpr_quotient_rounds_to_nearest() {
        // C `quotient` rounds halves away from zero after sign folding.
        assert_eq!(scanned_numexpr("\\numexpr 5/2\\relax"), 3);
        assert_eq!(scanned_numexpr("\\numexpr 3/2\\relax"), 2);
        assert_eq!(scanned_numexpr("\\numexpr -5/2\\relax"), -3);
        assert_eq!(scanned_numexpr("\\numexpr 5/-2\\relax"), -3);
        assert_eq!(scanned_numexpr("\\numexpr -5/-2\\relax"), 3);
        assert_eq!(scanned_numexpr("\\numexpr 7/3\\relax"), 2);
    }

    #[test]
    fn numexpr_fused_scale_uses_fract_rounding() {
        // `a*b/c` is evaluated as one `fract`, not `quotient(a*b, c)`.
        assert_eq!(scanned_numexpr("\\numexpr 2*3/4\\relax"), 2);
        assert_eq!(scanned_numexpr("\\numexpr 1*2/3\\relax"), 1);
        assert_eq!(scanned_numexpr("\\numexpr 7*3/4\\relax"), 5);
        assert_eq!(scanned_numexpr("\\numexpr 5*2/3\\relax"), 3);
        // The intermediate product may exceed TEX_INFINITY only via the
        // fused path: 1*max/2 rounds down without overflow.
        assert_eq!(
            scanned_numexpr("\\numexpr 1*\"7FFFFFFF/2\\relax"),
            0x4000_0000
        );
    }

    #[test]
    fn dimexpr_scans_dimension_terms() {
        assert_eq!(scanned_dimexpr("\\dimexpr 1pt+2pt\\relax"), 3 * 65536);
        assert_eq!(scanned_dimexpr("\\dimexpr 1.5pt*2\\relax"), 3 * 65536);
        assert_eq!(scanned_dimexpr("\\dimexpr (1pt+2pt)*3\\relax"), 9 * 65536);
        assert_eq!(scanned_dimexpr("\\dimexpr 1pt-2pt\\relax"), -65536);
        assert_eq!(scanned_dimexpr("\\dimexpr 1pt*3/2\\relax"), 98304);
        // A bare \dimexpr result is a valid internal dimension too.
        assert_eq!(scanned_numexpr("\\numexpr \\dimexpr 1pt\\relax"), 65536);
    }

    #[test]
    fn dimexpr_factors_are_integers() {
        let mut state = glue_state();
        // `\skip0` contributes its width in sp as the integer factor.
        let spec = glue_spec(&mut state, 100, 0, 0);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\dimexpr 1pt*\\skip0\\relax");
        assert_eq!(state.scan_dimen(false, false, false).unwrap(), 100 * 65536);
    }

    #[test]
    fn dimexpr_clamps_to_max_halfword() {
        // Operand scanning clamps like scan_dimen ("Dimension too large").
        assert_eq!(scanned_dimexpr("\\dimexpr 16384pt\\relax"), MAX_HALFWORD);
        // Arithmetic overflow recovers to zero.
        assert_eq!(scanned_dimexpr("\\dimexpr \"3FFFFFFEsp+2sp\\relax"), 0);
        assert_eq!(
            scanned_dimexpr("\\dimexpr \"3FFFFFFEsp+1sp\\relax"),
            MAX_HALFWORD
        );
    }

    #[test]
    fn glueexpr_scans_and_combines_glue() {
        assert_eq!(
            scanned_glueexpr("\\glueexpr 1pt plus 2pt minus 3pt\\relax"),
            (65536, 131072, 196608, 0, 0)
        );
        assert_eq!(
            scanned_glueexpr("\\glueexpr 1pt plus 1pt + 2pt plus 2pt\\relax"),
            (196608, 196608, 0, 0, 0)
        );
        assert_eq!(
            scanned_glueexpr("\\glueexpr 5pt minus 2pt - 1pt minus 1pt\\relax"),
            (262144, 0, 65536, 0, 0)
        );
    }

    #[test]
    fn glueexpr_scales_and_divides_specs() {
        assert_eq!(
            scanned_glueexpr("\\glueexpr (1pt plus 1fil)*2\\relax"),
            (131072, 131072, 0, FIL, 0)
        );
        assert_eq!(
            scanned_glueexpr("\\glueexpr (2pt plus 2fil)/2\\relax"),
            (65536, 65536, 0, FIL, 0)
        );
        // Fused scale applies `fract` to each spec word.
        assert_eq!(
            scanned_glueexpr("\\glueexpr (1pt plus 1fil)*3/2\\relax"),
            (98304, 98304, 0, FIL, 0)
        );
    }

    #[test]
    fn glueexpr_merges_orders_like_tex() {
        // The stronger non-zero order wins; equal orders combine.
        assert_eq!(
            scanned_glueexpr("\\glueexpr 0pt plus 1fil + 0pt plus 1fill\\relax"),
            (0, 65536, 0, FILL, 0)
        );
        assert_eq!(
            scanned_glueexpr("\\glueexpr 0pt plus 1fill - 0pt plus 1fil\\relax"),
            (0, 65536, 0, FILL, 0)
        );
        assert_eq!(
            scanned_glueexpr("\\glueexpr 0pt plus 3fil - 0pt plus 1fil\\relax"),
            (0, 131072, 0, FIL, 0)
        );
        // A zeroed stretch/shrink resets the order to NORMAL.
        assert_eq!(
            scanned_glueexpr("\\glueexpr 1pt plus 1fil - 0pt plus 1fil\\relax"),
            (65536, 0, 0, 0, 0)
        );
        assert_eq!(
            scanned_glueexpr("\\glueexpr 0pt minus 1filll - 0pt minus 1fill\\relax"),
            (0, 0, 65536, 0, FILLL)
        );
    }

    #[test]
    fn glueexpr_preserves_register_specs() {
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 111, 22, 33);
        state.mem[spec as usize].set_b16_s1(FIL);
        state.set_skip(0, spec).unwrap();

        // A bare `\skip0` operand yields the register spec itself.
        state.set_source_text("\\glueexpr \\skip0\\relax");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(q, spec);
        assert_eq!(spec_parts(&state, q), (111, 22, 33, FIL, 0));

        // With an operator the arithmetic runs on a copy: the register
        // spec is untouched and the result is a fresh node.
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 111, 22, 33);
        state.mem[spec as usize].set_b16_s1(FIL);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\glueexpr \\skip0+0pt\\relax");
        let q = state.scan_normal_glue().unwrap();
        assert_ne!(q, spec);
        assert_eq!(spec_parts(&state, q), (111, 22, 33, FIL, 0));
        assert_eq!(spec_parts(&state, spec), (111, 22, 33, FIL, 0));
        assert_eq!(state.mem[spec as usize].s1, 0);
    }

    #[test]
    fn glueexpr_overflow_yields_zero_glue() {
        assert_eq!(
            scanned_glueexpr("\\glueexpr \"3FFFFFFEsp+2sp\\relax"),
            (0, 0, 0, 0, 0)
        );
    }

    #[test]
    fn muexpr_scans_mu_glue() {
        assert_eq!(
            scanned_muexpr("\\muexpr 1mu plus 2mu\\relax"),
            (65536, 131072, 0, 0, 0)
        );
        assert_eq!(
            scanned_muexpr("\\muexpr (5mu minus 1mu)*2\\relax"),
            (655360, 0, 131072, 0, 0)
        );
        assert_eq!(scanned_muexpr("\\muexpr \\muskip0\\relax"), (0, 0, 0, 0, 0));
    }

    #[test]
    fn mutoglue_and_gluetomu_convert() {
        let mut state = glue_state();
        state.set_source_text("\\mutoglue 1mu plus 2mu\\relax");
        let q = state.scan_normal_glue().unwrap();
        assert_eq!(spec_parts(&state, q), (65536, 131072, 0, 0, 0));

        let mut state = glue_state();
        state.set_source_text("\\gluetomu 1pt minus 1pt\\relax");
        let q = state.scan_mu_glue().unwrap();
        assert_eq!(spec_parts(&state, q), (65536, 0, 65536, 0, 0));
    }

    #[test]
    fn expr_scans_internal_quantities() {
        let mut int_state = state();
        int_state.set_count(1, 41).unwrap();
        int_state.set_dimen(1, 7 * 65536).unwrap();
        int_state.set_source_text("\\numexpr \\count1+1\\relax");
        assert_eq!(int_state.scan_int().unwrap(), 42);

        let mut dimen_state = state();
        dimen_state.set_dimen(1, 7 * 65536).unwrap();
        dimen_state.set_source_text("\\numexpr \\dimen1\\relax");
        assert_eq!(dimen_state.scan_int().unwrap(), 7 * 65536);

        // `\count` inside a dimexpr coefficient position.
        let mut factor_state = state();
        factor_state.set_count(1, 3).unwrap();
        factor_state.set_source_text("\\dimexpr \\count1pt*2\\relax");
        assert_eq!(
            factor_state.scan_dimen(false, false, false).unwrap(),
            6 * 65536
        );
    }

    #[test]
    fn expr_nested_levels_and_parens() {
        // etrip.tex:765-869 style nests: expr inside expr, mixed levels.
        assert_eq!(
            scanned_numexpr("\\numexpr \\numexpr 1+2\\relax*3\\relax"),
            9
        );
        assert_eq!(
            scanned_numexpr("\\numexpr 2*\\numexpr 4-1\\relax\\relax"),
            6
        );
        assert_eq!(
            scanned_dimexpr("\\dimexpr \\dimexpr 1pt\\relax+1pt\\relax"),
            131072
        );
        // A glue register width feeds an integer factor position.
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 100, 0, 0);
        state.set_skip(0, spec).unwrap();
        state.set_source_text("\\numexpr 2*\\skip0\\relax");
        assert_eq!(state.scan_int().unwrap(), 200);
        // And a muskip width does the same through the mu_error path.
        let mut state = glue_state();
        let spec = glue_spec(&mut state, 60, 0, 0);
        state.set_mu_skip(0, spec).unwrap();
        state.set_source_text("\\numexpr \\muskip0\\relax");
        assert_eq!(state.scan_int().unwrap(), 60);
    }
}
