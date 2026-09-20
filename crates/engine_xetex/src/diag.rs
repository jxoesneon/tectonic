//! Command/diagnostic name printing and conditional-tracing machinery
//! (`print_cmd_chr`, `show_cur_cmd_chr`, `group_warning`, `if_warning`,
//! `file_warning`, `print_group`).
//!
//! [`TeXEngineState::cmd_chr_text`] is a String-producing port of C's
//! `print_cmd_chr`: every command family whose output is more than a plain
//! `\name` lookup is handled explicitly, while the large mechanical families
//! fall back to the vendored primitive table (`primitive_name_for`) and
//! finally to C's `[unknown command code!]`.

use crate::commands::primitive_name_for;
use crate::expand::{format_scaled_pt, FI_CODE, OR_CODE, UNLESS_CODE};
use crate::scan::{DIMEN_VAL, GLUE_VAL, INT_VAL};
use crate::scanner::{CR_CODE, PROTECTED_TOKEN, SPAN_CODE, TOKEN_LIST};
use crate::state::{
    CommandCode, SaveStackEntry, TeXEngineState, TeXMode, AAT_FONT_FLAG, ABOVE_CODE,
    ADJUSTED_HBOX_GROUP, ALIGN_GROUP, ATOP_CODE, BEGIN_L_CODE, BIN_NOAD, BOTTOM_LEVEL,
    BOT_MARK_CODE, CAT_CODE_BASE, CLOSE_NOAD, CLOSE_NODE, COUNT_BASE, DELIMITED_CODE,
    DEL_CODE_BASE, DIMEN_BASE, DIMEN_VAL_LIMIT, DISC_GROUP, END_L_CODE, ETEX_PEN_BASE,
    FIRST_MARK_CODE, HBOX_GROUP, HEIGHT_OFFSET, HISTORY_SPOTLESS, HISTORY_WARNING_ISSUED,
    INNER_NOAD, INSERT_GROUP, INT_PAR_TRACING_COMMANDS, INT_PAR_TRACING_IFS,
    INT_PAR_TRACING_NESTING, LC_CODE_BASE, LEFT_NOAD, LIMITS, LOCAL_BASE, LOCAL_PAR_SHAPE,
    MATH_CHOICE_GROUP, MATH_CODE_BASE, MATH_FONT_BASE_C, MATH_GROUP, MATH_LEFT_GROUP,
    MATH_SHIFT_GROUP, MU_SKIP_BASE, NO_ALIGN_GROUP, OPEN_NOAD, OPEN_NODE, ORD_NOAD, OTGR_FONT_FLAG,
    OUTPUT_GROUP, OVER_CODE, PDF_SAVE_POS_NODE, PUNCT_NOAD, REL_NOAD, SCRIPT_SIZE,
    SEMI_SIMPLE_GROUP, SF_CODE_BASE, SIMPLE_GROUP, SKIP_BASE, SPECIAL_NODE, SPLIT_BOT_MARK_CODE,
    SPLIT_FIRST_MARK_CODE, TEXT_SIZE, TEX_NULL, TOKS_BASE, UC_CODE_BASE, UNDER_NOAD, VBOX_GROUP,
    VCENTER_GROUP, VTOP_GROUP, WIDTH_OFFSET, WRITE_NODE,
};
use crate::TeXMouthError;

/// `MARKS_CODE` — boundary between single marks and mark registers.
const MARKS_CODE: i32 = 5;
/// `MIDDLE_NOAD` — `\middle` chr on `LEFT_RIGHT` (distinct from the noad type).
const MIDDLE_NOAD: i32 = 1;
/// `lo_mem_stat_max` — `REGISTER` chr values at or below this are types.
const LO_MEM_STAT_MAX: i32 = 19;
/// `\chardef` shorthand code.
const CHAR_DEF_CODE: i32 = 0;
/// `\mathchardef` shorthand code.
const MATH_CHAR_DEF_CODE: i32 = 1;
/// `\countdef` shorthand code.
const COUNT_DEF_CODE: i32 = 2;
/// `\dimendef` shorthand code.
const DIMEN_DEF_CODE: i32 = 3;
/// `\skipdef` shorthand code.
const SKIP_DEF_CODE: i32 = 4;
/// `\muskipdef` shorthand code.
const MU_SKIP_DEF_CODE: i32 = 5;
/// `\charsubdef` shorthand code.
const CHAR_SUB_DEF_CODE: i32 = 7;
/// `\Umathcharnumdef` shorthand code.
const XETEX_MATH_CHAR_NUM_DEF_CODE: i32 = 8;
/// `\Umathchardef` shorthand code.
const XETEX_MATH_CHAR_DEF_CODE: i32 = 9;
/// `SHOW_BOX_CODE` (`\showbox`).
const SHOW_BOX_CODE: i32 = 1;
/// `SHOW_THE_CODE` (`\showthe`).
const SHOW_THE_CODE: i32 = 2;
/// `SHOW_LISTS` (`\showlists`).
const SHOW_LISTS_CODE: i32 = 3;
/// `SHOW_GROUPS` (`\showgroups`).
const SHOW_GROUPS_CODE: i32 = 4;
/// `SHOW_TOKENS` (`\showtokens`).
const SHOW_TOKENS_CODE: i32 = 5;
/// `SHOW_IFS` (`\showifs`).
const SHOW_IFS_CODE: i32 = 6;
/// `IMMEDIATE_CODE` (`\immediate`).
const IMMEDIATE_CODE: i32 = 4;
/// `SET_LANGUAGE_CODE` (`\setlanguage`).
const SET_LANGUAGE_CODE: i32 = 5;
/// `RESET_TIMER_CODE` (`\resettimer`).
const RESET_TIMER_CODE: i32 = 31;
/// `SET_RANDOM_SEED_CODE` (`\setrandomseed`).
const SET_RANDOM_SEED_CODE: i32 = 33;
/// `PIC_FILE_CODE` (`\XeTeXpicfile`).
const PIC_FILE_CODE: i32 = 41;
/// `PDF_FILE_CODE` (`\XeTeXpdffile`).
const PDF_FILE_CODE: i32 = 42;
/// `GLYPH_CODE` (`\XeTeXglyph`).
const GLYPH_CODE: i32 = 43;
/// `XETEX_INPUT_ENCODING_EXTENSION_CODE` (`\XeTeXinputencoding`).
const XETEX_INPUT_ENCODING_EXTENSION_CODE: i32 = 44;
/// `XETEX_DEFAULT_ENCODING_EXTENSION_CODE` (`\XeTeXdefaultencoding`).
const XETEX_DEFAULT_ENCODING_EXTENSION_CODE: i32 = 45;
/// `XETEX_LINEBREAK_LOCALE_EXTENSION_CODE` (`\XeTeXlinebreaklocale`).
const XETEX_LINEBREAK_LOCALE_EXTENSION_CODE: i32 = 46;
/// `NO_LIMITS` (`\nolimits`).
const NO_LIMITS: i32 = 2;
/// `BEGIN_R_CODE` (`\beginR`).
const BEGIN_R_CODE: i32 = 10;

/// `\if`-family primitive names indexed by `chr % UNLESS_CODE`
/// (`print_cmd_chr`'s `IF_TEST` arm).
const IF_NAMES: [&str; 22] = [
    "if",
    "ifcat",
    "ifnum",
    "ifdim",
    "ifodd",
    "ifvmode",
    "ifhmode",
    "ifmmode",
    "ifinner",
    "ifvoid",
    "ifhbox",
    "ifvbox",
    "ifx",
    "ifeof",
    "iftrue",
    "iffalse",
    "ifcase",
    "ifdefined",
    "ifcsname",
    "iffontchar",
    "ifincsname",
    "ifprimitive",
];

/// Mode name for `show_cur_cmd_chr` (`print_mode`): positive modes print the
/// outer variant, negative modes the inner variant.
fn mode_text(mode: i16) -> &'static str {
    match mode {
        x if x == TeXMode::Vmode as i16 => "vertical mode",
        x if x == TeXMode::Hmode as i16 => "horizontal mode",
        x if x == TeXMode::Mmode as i16 => "display math mode",
        x if x == TeXMode::InternalVmode as i16 => "internal vertical mode",
        x if x == TeXMode::RestrictedHmode as i16 => "restricted horizontal mode",
        x if x == TeXMode::InlineMmode as i16 => "math mode",
        _ => "no mode",
    }
}

/// Group name noun for `print_group` — C reads the current `cur_group`,
/// expanding `semi`/`adjusted`/`no`/`choice`/`shift`/`left` qualifiers.
fn group_noun(group: u8) -> &'static str {
    match group {
        x if x == BOTTOM_LEVEL => "bottom level",
        x if x == SEMI_SIMPLE_GROUP => "semi simple",
        x if x == ADJUSTED_HBOX_GROUP => "adjusted hbox",
        x if x == NO_ALIGN_GROUP => "no align",
        x if x == MATH_CHOICE_GROUP => "math choice",
        x if x == MATH_SHIFT_GROUP => "math shift",
        x if x == MATH_LEFT_GROUP => "math left",
        x if x == SIMPLE_GROUP => "simple",
        x if x == HBOX_GROUP => "hbox",
        x if x == VBOX_GROUP => "vbox",
        x if x == VTOP_GROUP => "vtop",
        x if x == ALIGN_GROUP => "align",
        x if x == OUTPUT_GROUP => "output",
        x if x == DISC_GROUP => "disc",
        x if x == INSERT_GROUP => "insert",
        x if x == VCENTER_GROUP => "vcenter",
        x if x == MATH_GROUP => "math",
        _ => "unknown",
    }
}

impl TeXEngineState {
    /// Returns the display text for a command/payload pair
    /// (`print_cmd_chr`). Escape-sequence names include the leading
    /// backslash.
    pub(crate) fn cmd_chr_text(&self, cmd: u8, chr: i32) -> Result<String, TeXMouthError> {
        let esc = |name: &str| format!("\\{name}");

        match CommandCode::from_u8(cmd) {
            Some(CommandCode::LeftBrace) => {
                return Ok(format!(
                    "begin-group character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::RightBrace) => {
                return Ok(format!(
                    "end-group character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::MathShift) => {
                return Ok(format!(
                    "math shift character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::TabMark) => {
                if chr == SPAN_CODE {
                    return Ok(esc("span"));
                }
                return Ok(format!(
                    "alignment tab character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::CarRet) => {
                return Ok(esc(if chr == CR_CODE { "cr" } else { "crcr" }));
            }
            Some(CommandCode::MacParam) => {
                return Ok(format!(
                    "macro parameter character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::SupMark) => {
                return Ok(format!(
                    "superscript character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::SubMark) => {
                return Ok(format!(
                    "subscript character {}",
                    self.display_character(chr)
                ));
            }
            Some(CommandCode::Endv) => return Ok("end of alignment template".to_string()),
            Some(CommandCode::Spacer) => {
                return Ok(format!("blank space {}", self.display_character(chr)));
            }
            Some(CommandCode::Letter) => {
                return Ok(format!("the letter {}", self.display_character(chr)));
            }
            Some(CommandCode::OtherChar) => {
                return Ok(format!("the character {}", self.display_character(chr)));
            }
            Some(CommandCode::AssignGlue | CommandCode::AssignMuGlue) => {
                let skip_base = SKIP_BASE as i32;
                let mu_base = MU_SKIP_BASE as i32;
                if (skip_base..skip_base + 256).contains(&chr) {
                    return Ok(format!("\\skip{}", chr - skip_base));
                }
                if (mu_base..mu_base + 256).contains(&chr) {
                    return Ok(format!("\\muskip{}", chr - mu_base));
                }
                if let Some(name) = primitive_name_for(CommandCode::AssignGlue, chr)
                    .or_else(|| primitive_name_for(CommandCode::AssignMuGlue, chr))
                {
                    return Ok(esc(name));
                }
                return Ok("[unknown command code!]".to_string());
            }
            Some(CommandCode::AssignToks) => {
                let toks_base = TOKS_BASE as i32;
                if (toks_base..toks_base + 256).contains(&chr) {
                    return Ok(format!("\\toks{}", chr - toks_base));
                }
                // C's local-parameter switch defaults to `\errhelp`.
                let name = primitive_name_for(CommandCode::AssignToks, chr).unwrap_or("errhelp");
                return Ok(esc(name));
            }
            Some(CommandCode::AssignInt) => {
                let count_base = COUNT_BASE as i32;
                if (count_base..count_base + 256).contains(&chr) {
                    return Ok(format!("\\count{}", chr - count_base));
                }
                if let Some(name) = primitive_name_for(CommandCode::AssignInt, chr) {
                    return Ok(esc(name));
                }
                return Ok("[unknown command code!]".to_string());
            }
            Some(CommandCode::AssignDimen) => {
                let dimen_base = DIMEN_BASE as i32;
                if (dimen_base..dimen_base + 256).contains(&chr) {
                    return Ok(format!("\\dimen{}", chr - dimen_base));
                }
                if let Some(name) = primitive_name_for(CommandCode::AssignDimen, chr) {
                    return Ok(esc(name));
                }
                return Ok("[unknown command code!]".to_string());
            }
            Some(CommandCode::SetAux) => {
                return Ok(esc(if chr == i32::from(TeXMode::Vmode as i16) {
                    "prevdepth"
                } else {
                    "spacefactor"
                }));
            }
            Some(CommandCode::SetPageDimen) => {
                let name = match chr {
                    0 => "pagegoal",
                    1 => "pagetotal",
                    2 => "pagestretch",
                    3 => "pagefilstretch",
                    4 => "pagefillstretch",
                    5 => "pagefilllstretch",
                    6 => "pageshrink",
                    _ => "pagedepth",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::SetPageInt) => {
                let name = match chr {
                    0 => "deadcycles",
                    2 => "interactionmode",
                    _ => "insertpenalties",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::SetBoxDimen) => {
                let name = match chr {
                    x if x == WIDTH_OFFSET => "wd",
                    x if x == HEIGHT_OFFSET => "ht",
                    _ => "dp",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::SetShape) => {
                // C's switch has no default: unlisted shapes print nothing.
                let name = match chr {
                    x if x == (LOCAL_BASE + LOCAL_PAR_SHAPE) as i32 => "parshape",
                    x if x == ETEX_PEN_BASE as i32 => "interlinepenalties",
                    x if x == ETEX_PEN_BASE as i32 + 1 => "clubpenalties",
                    x if x == ETEX_PEN_BASE as i32 + 2 => "widowpenalties",
                    x if x == ETEX_PEN_BASE as i32 + 3 => "displaywidowpenalties",
                    _ => return Ok(String::new()),
                };
                return Ok(esc(name));
            }
            Some(CommandCode::DefCode) => {
                let name = match chr {
                    x if x == CAT_CODE_BASE => "catcode",
                    x if x == MATH_CODE_BASE => "mathcode",
                    x if x == LC_CODE_BASE => "lccode",
                    x if x == UC_CODE_BASE => "uccode",
                    x if x == SF_CODE_BASE => "sfcode",
                    _ => "delcode",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::XetexDefCode) => {
                let name = match chr {
                    x if x == SF_CODE_BASE => "XeTeXcharclass",
                    x if x == MATH_CODE_BASE => "Umathcodenum",
                    x if x == MATH_CODE_BASE + 1 => "Umathcode",
                    x if x == DEL_CODE_BASE => "Udelcodenum",
                    _ => "Udelcode",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::DefFamily) => {
                return Ok(esc(self.size_text(chr - MATH_FONT_BASE_C)));
            }
            Some(CommandCode::SetFont) => {
                return self.select_font_text(chr);
            }
            Some(CommandCode::Register) => {
                let (type_code, chr_code) = if !(0..=LO_MEM_STAT_MAX).contains(&chr) {
                    let idx = self.checked_engine_mem_index(chr)?;
                    (i32::from(self.mem[idx].b16_s1() / 64), chr)
                } else {
                    (chr, TEX_NULL)
                };
                let name = match type_code as u8 {
                    x if x == INT_VAL => "count",
                    x if x == DIMEN_VAL => "dimen",
                    x if x == GLUE_VAL => "skip",
                    _ => "muskip",
                };
                let mut text = esc(name);
                if chr_code != TEX_NULL {
                    text.push_str(&self.sa_num_text(chr_code)?);
                }
                return Ok(text);
            }
            Some(CommandCode::ToksRegister) => {
                let mut text = esc("toks");
                if chr != 0 {
                    text.push_str(&self.sa_num_text(chr)?);
                }
                return Ok(text);
            }
            Some(CommandCode::CharGiven) => return Ok(format!("\\char\"{chr:X}")),
            Some(CommandCode::MathGiven) => return Ok(format!("\\mathchar\"{chr:X}")),
            Some(CommandCode::XetexMathGiven) => {
                return Ok(format!(
                    "\\Umathchar\"{:X}\"{:X}\"{:X}",
                    Self::math_class(chr),
                    Self::math_fam(chr),
                    Self::math_char(chr)
                ));
            }
            Some(CommandCode::ShorthandDef) => {
                let name = match chr {
                    CHAR_DEF_CODE => "chardef",
                    MATH_CHAR_DEF_CODE => "mathchardef",
                    XETEX_MATH_CHAR_DEF_CODE => "Umathchardef",
                    XETEX_MATH_CHAR_NUM_DEF_CODE => "Umathcharnumdef",
                    COUNT_DEF_CODE => "countdef",
                    DIMEN_DEF_CODE => "dimendef",
                    SKIP_DEF_CODE => "skipdef",
                    MU_SKIP_DEF_CODE => "muskipdef",
                    CHAR_SUB_DEF_CODE => "charsubdef",
                    _ => "toksdef",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::Prefix) => {
                let name = match chr {
                    1 => "long",
                    2 => "outer",
                    8 => "protected",
                    _ => "global",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::Def) => {
                let name = match chr {
                    0 => "def",
                    1 => "gdef",
                    2 => "edef",
                    _ => "xdef",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::Let) => {
                return Ok(esc(if chr != 0 { "futurelet" } else { "let" }));
            }
            Some(CommandCode::HyphData) => {
                return Ok(esc(if chr == 1 { "patterns" } else { "hyphenation" }));
            }
            Some(CommandCode::AssignFontInt) => {
                let name = match chr {
                    0 => "hyphenchar",
                    1 => "skewchar",
                    x if x == crate::state::LP_CODE_BASE => "lpcode",
                    x if x == crate::state::RP_CODE_BASE => "rpcode",
                    _ => return Ok(String::new()),
                };
                return Ok(esc(name));
            }
            Some(CommandCode::SetInteraction) => {
                let name = match chr {
                    0 => "batchmode",
                    1 => "nonstopmode",
                    2 => "scrollmode",
                    _ => "errorstopmode",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::InStream) => {
                return Ok(esc(if chr == 0 { "closein" } else { "openin" }));
            }
            Some(CommandCode::Message) => {
                return Ok(esc(if chr == 0 { "message" } else { "errmessage" }));
            }
            Some(CommandCode::CaseShift) => {
                return Ok(esc(if chr == LC_CODE_BASE {
                    "lowercase"
                } else {
                    "uppercase"
                }));
            }
            Some(CommandCode::Xray) => {
                let name = match chr {
                    SHOW_BOX_CODE => "showbox",
                    SHOW_THE_CODE => "showthe",
                    SHOW_LISTS_CODE => "showlists",
                    SHOW_GROUPS_CODE => "showgroups",
                    SHOW_TOKENS_CODE => "showtokens",
                    SHOW_IFS_CODE => "showifs",
                    _ => "show",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::UndefinedCs) => return Ok("undefined".to_string()),
            Some(
                CommandCode::Call
                | CommandCode::LongCall
                | CommandCode::OuterCall
                | CommandCode::LongOuterCall,
            ) => {
                let mut n = i32::from(cmd - CommandCode::Call as u8);
                let start = self.stored_token_list_start(chr)?;
                if start != TEX_NULL {
                    let idx = self.checked_engine_mem_index(start)?;
                    if self.mem[idx].s0 == PROTECTED_TOKEN {
                        n += 4;
                    }
                }
                // C concatenates the flag names with no separator and prints
                // a single space before `macro` when any flag is present.
                let mut text = String::new();
                if n / 4 % 2 != 0 {
                    text.push_str("\\protected");
                }
                if n % 2 != 0 {
                    text.push_str("\\long");
                }
                if n / 2 % 2 != 0 {
                    text.push_str("\\outer");
                }
                if n > 0 {
                    text.push(' ');
                }
                text.push_str("macro");
                return Ok(text);
            }
            Some(CommandCode::EndTemplate) => return Ok(esc("outer endtemplate")),
            Some(CommandCode::IfTest) => {
                let mut text = String::new();
                if chr >= UNLESS_CODE {
                    text.push_str("\\unless");
                }
                let code = (chr % UNLESS_CODE) as usize;
                text.push_str(&esc(IF_NAMES.get(code).copied().unwrap_or("if")));
                return Ok(text);
            }
            Some(CommandCode::FiOrElse) => {
                let name = match chr {
                    FI_CODE => "fi",
                    OR_CODE => "or",
                    _ => "else",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::Mark) => {
                let mut text = esc("mark");
                if chr > 0 {
                    text.push('s');
                }
                return Ok(text);
            }
            Some(CommandCode::TopBotMark) => {
                let name = match chr % MARKS_CODE {
                    FIRST_MARK_CODE => "firstmark",
                    BOT_MARK_CODE => "botmark",
                    SPLIT_FIRST_MARK_CODE => "splitfirstmark",
                    SPLIT_BOT_MARK_CODE => "splitbotmark",
                    _ => "topmark",
                };
                let mut text = esc(name);
                if chr >= MARKS_CODE {
                    text.push('s');
                }
                return Ok(text);
            }
            Some(CommandCode::ExpandAfter) => {
                return Ok(esc(if chr == 0 { "expandafter" } else { "unless" }));
            }
            Some(CommandCode::NoExpand) => {
                return Ok(esc(if chr == 0 { "noexpand" } else { "primitive" }));
            }
            Some(CommandCode::IgnoreSpaces) => {
                return Ok(esc(if chr == 0 {
                    "ignorespaces"
                } else {
                    "primitive"
                }));
            }
            Some(CommandCode::Input) => {
                let name = match chr {
                    0 => "input",
                    2 => "scantokens",
                    _ => "endinput",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::Stop) => {
                return Ok(esc(if chr == 1 { "dump" } else { "end" }));
            }
            Some(CommandCode::ReadToCs) => {
                return Ok(esc(if chr == 0 { "read" } else { "readline" }));
            }
            Some(CommandCode::The) => {
                let name = match chr {
                    0 => "the",
                    1 => "unexpanded",
                    _ => "detokenize",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::DelimNum) => {
                return Ok(esc(if chr == 1 { "Udelimiter" } else { "delimiter" }));
            }
            Some(CommandCode::MathCharNum) => {
                let name = match chr {
                    2 => "Umathchar",
                    1 => "Umathcharnum",
                    _ => "mathchar",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::MathAccent) => {
                return Ok(esc(if chr == 1 {
                    "Umathaccent"
                } else {
                    "mathaccent"
                }));
            }
            Some(CommandCode::Radical) => {
                return Ok(esc(if chr == 1 { "Uradical" } else { "radical" }));
            }
            Some(CommandCode::Valign) => {
                let name = match chr {
                    0 => "valign",
                    BEGIN_L_CODE => "beginL",
                    END_L_CODE => "endL",
                    BEGIN_R_CODE => "beginR",
                    _ => "endR",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::LeftRight) => {
                let name = match chr {
                    x if x == i32::from(LEFT_NOAD) => "left",
                    MIDDLE_NOAD => "middle",
                    _ => "right",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::MathComp) => {
                let name = match chr {
                    x if x == i32::from(ORD_NOAD) => "mathord",
                    x if x == i32::from(crate::state::OP_NOAD) => "mathop",
                    x if x == i32::from(BIN_NOAD) => "mathbin",
                    x if x == i32::from(REL_NOAD) => "mathrel",
                    x if x == i32::from(OPEN_NOAD) => "mathopen",
                    x if x == i32::from(CLOSE_NOAD) => "mathclose",
                    x if x == i32::from(PUNCT_NOAD) => "mathpunct",
                    x if x == i32::from(INNER_NOAD) => "mathinner",
                    x if x == i32::from(UNDER_NOAD) => "underline",
                    _ => "overline",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::LimitSwitch) => {
                let name = match chr {
                    x if x == i32::from(LIMITS) => "limits",
                    NO_LIMITS => "nolimits",
                    _ => "displaylimits",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::MathStyle) => {
                return Ok(esc(self.style_text(chr)));
            }
            Some(CommandCode::Above) => {
                let name = match chr {
                    OVER_CODE => "over",
                    ATOP_CODE => "atop",
                    x if x == DELIMITED_CODE + ABOVE_CODE => "abovewithdelims",
                    x if x == DELIMITED_CODE + OVER_CODE => "overwithdelims",
                    x if x == DELIMITED_CODE + ATOP_CODE => "atopwithdelims",
                    _ => "above",
                };
                return Ok(esc(name));
            }
            Some(CommandCode::Extension) => {
                let name = match chr {
                    x if x == i32::from(OPEN_NODE) => "openout",
                    x if x == i32::from(WRITE_NODE) => "write",
                    x if x == i32::from(CLOSE_NODE) => "closeout",
                    x if x == i32::from(SPECIAL_NODE) => "special",
                    IMMEDIATE_CODE => "immediate",
                    SET_LANGUAGE_CODE => "setlanguage",
                    x if x == i32::from(PDF_SAVE_POS_NODE) => "pdfsavepos",
                    RESET_TIMER_CODE => "resettimer",
                    SET_RANDOM_SEED_CODE => "setrandomseed",
                    PIC_FILE_CODE => "XeTeXpicfile",
                    PDF_FILE_CODE => "XeTeXpdffile",
                    GLYPH_CODE => "XeTeXglyph",
                    XETEX_LINEBREAK_LOCALE_EXTENSION_CODE => "XeTeXlinebreaklocale",
                    XETEX_INPUT_ENCODING_EXTENSION_CODE => "XeTeXinputencoding",
                    XETEX_DEFAULT_ENCODING_EXTENSION_CODE => "XeTeXdefaultencoding",
                    _ => return Ok("[unknown extension!]".to_string()),
                };
                return Ok(esc(name));
            }
            Some(CommandCode::LastItem) => {
                if let Some(name) = primitive_name_for(CommandCode::LastItem, chr) {
                    return Ok(esc(name));
                }
                // C's `LAST_ITEM` switch defaults to `\jobname`.
                return Ok(esc("jobname"));
            }
            _ => {}
        }

        if let Some(command) = CommandCode::from_u8(cmd) {
            if let Some(name) = primitive_name_for(command, chr) {
                return Ok(esc(name));
            }
        }
        Ok("[unknown command code!]".to_string())
    }

    /// `select font <name>` text for `SET_FONT` (`print_cmd_chr`'s `SET_FONT`
    /// arm): quotes AAT/OT native names, appending ` at <size>pt` when the
    /// requested size differs from the design size.
    fn select_font_text(&self, font: i32) -> Result<String, TeXMouthError> {
        let record = self
            .font_record(font)
            .ok_or(TeXMouthError::UnsupportedPrimitive("select font"))?;
        let name: String = record
            .name
            .iter()
            .filter_map(|&u| char::from_u32(u.max(0) as u32))
            .collect();
        let mut text = String::from("select font ");
        if matches!(
            self.font_area.get(&font),
            Some(&flag) if flag == AAT_FONT_FLAG || flag == OTGR_FONT_FLAG
        ) {
            let quote = if name.contains('"') { '\'' } else { '"' };
            text.push(quote);
            text.push_str(&name);
            text.push(quote);
        } else {
            text.push_str(&name);
        }
        if record.size != record.dsize {
            text.push_str(" at ");
            text.push_str(&format_scaled_pt(record.size));
            text.push_str("pt");
        }
        Ok(text)
    }

    /// `\textfont`/`\scriptfont`/`\scriptscriptfont` name for a `DEF_FAMILY`
    /// payload (`print_size`).
    fn size_text(&self, s: i32) -> &'static str {
        if s == TEXT_SIZE {
            "textfont"
        } else if s == SCRIPT_SIZE {
            "scriptfont"
        } else {
            "scriptscriptfont"
        }
    }

    /// `\displaystyle`-family name for a `MATH_STYLE` payload
    /// (`print_style`).
    fn style_text(&self, s: i32) -> &'static str {
        match s {
            0 => "displaystyle",
            2 => "textstyle",
            4 => "scriptstyle",
            _ => "scriptscriptstyle",
        }
    }

    /// Sparse-register number text (`print_sa_num`).
    fn sa_num_text(&self, q: i32) -> Result<String, TeXMouthError> {
        let mut q = q;
        let n = if self.mem[q as usize].b16_s1() < DIMEN_VAL_LIMIT as u16 {
            self.mem[(q + 1) as usize].s1
        } else {
            let mut n = i32::from(self.mem[q as usize].b16_s1() % 64);
            q = self.mem[q as usize].s1;
            n += 64 * i32::from(self.mem[q as usize].b16_s1());
            q = self.mem[q as usize].s1;
            n + 64
                * 64
                * (i32::from(self.mem[q as usize].b16_s1())
                    + 64 * i32::from(self.mem[self.mem[q as usize].s1 as usize].b16_s1()))
        };
        Ok(n.to_string())
    }

    /// `\if` primitive name for an `IF_TEST` payload (`print_cmd_chr`'s
    /// `IF_TEST` arm applied to `cur_if`).
    pub(crate) fn if_test_text(&self, if_code: i32) -> Result<String, TeXMouthError> {
        self.cmd_chr_text(CommandCode::IfTest as u8, if_code)
    }

    /// Emits the `{mode: cmd (level n) entered on line l}` trace
    /// (`show_cur_cmd_chr`) through the diagnostic selector.
    pub fn show_cur_cmd_chr(&mut self) -> Result<(), TeXMouthError> {
        self.begin_diagnostic()?;
        self.capture_to_diagnostic(true);
        self.diagnostic_print_file_line();

        self.print_nl_cstr("{")?;
        if i32::from(self.mode) != self.shown_mode {
            self.print_cstr(mode_text(self.mode))?;
            self.print_cstr(": ")?;
            self.shown_mode = i32::from(self.mode);
        }
        let text = self.cmd_chr_text(self.cur_cmd, self.cur_chr)?;
        self.print_cstr(&text)?;
        if self.int_par(INT_PAR_TRACING_IFS)? > 0
            && self.cur_cmd >= CommandCode::IfTest as u8
            && self.cur_cmd <= CommandCode::FiOrElse as u8
        {
            self.print_cstr(": ")?;
            let (mut n, l) = if self.cur_cmd == CommandCode::FiOrElse as u8 {
                let if_name = self.if_test_text(i32::from(self.cur_if))?;
                self.print_cstr(&if_name)?;
                self.print_char(i32::from(b' '))?;
                (0, self.if_line)
            } else {
                (1, self.line)
            };
            let mut p = self.cond_ptr;
            while p != TEX_NULL {
                n += 1;
                let idx = self.checked_engine_mem_index(p)?;
                p = self.mem[idx].s1;
            }
            self.print_cstr("(level ")?;
            self.print_int(n)?;
            self.print_char(i32::from(b')'))?;
            if l != 0 {
                self.print_cstr(" entered on line ")?;
                self.print_int(l)?;
            }
        }
        self.print_char(i32::from(b'}'))?;

        self.capture_to_diagnostic(false);
        self.end_diagnostic(false)
    }

    /// Emits `{true}`/`{false}` when a conditional result is traced
    /// (C `conditional`'s `INTPAR(tracing_commands) > 1` block).
    pub(crate) fn trace_conditional_result(
        &mut self,
        condition: bool,
    ) -> Result<(), TeXMouthError> {
        if self.int_par(INT_PAR_TRACING_COMMANDS)? <= 1 {
            return Ok(());
        }
        self.begin_diagnostic()?;
        self.capture_to_diagnostic(true);
        self.diagnostic_print_file_line();
        self.print_cstr(if condition { "{true}" } else { "{false}" })?;
        self.capture_to_diagnostic(false);
        self.end_diagnostic(false)
    }

    /// Prints `<name> group (level <n>) entered at line <l>`
    /// (`print_group` with `e = true`).
    pub(crate) fn print_group(&mut self, entered: bool) -> Result<(), TeXMouthError> {
        self.print_cstr(group_noun(self.cur_group))?;
        self.print_cstr(" group (level ")?;
        self.print_int(i32::from(self.cur_level))?;
        self.print_char(i32::from(b')'))?;
        let line = match self.save_stack.get(self.save_ptr.wrapping_sub(1)).copied() {
            Some(SaveStackEntry::Line(line)) => line,
            _ => 0,
        };
        if line != 0 {
            if entered {
                self.print_cstr(" entered at line ")?;
            } else {
                self.print_cstr(" at line ")?;
            }
            self.print_int(line)?;
        }
        Ok(())
    }

    /// Warns when a group opened in one file closes in another
    /// (`group_warning`). `restored_boundary` is the boundary the enclosing
    /// `unsave` is about to restore (`save_stack[save_ptr].b32.s1` in C).
    pub(crate) fn group_warning(&mut self, restored_boundary: i32) -> Result<(), TeXMouthError> {
        let tracing_nesting = self.int_par(INT_PAR_TRACING_NESTING)?;
        let mut base_ptr = self.input_ptr;
        let mut warned = false;
        for i in (1..=self.in_open).rev() {
            if self.grp_stack.get(i).copied().unwrap_or(TEX_NULL) != self.cur_boundary {
                break;
            }
            if tracing_nesting > 0 && self.input_source_name_above(&mut base_ptr, i) {
                warned = true;
            }
            if let Some(slot) = self.grp_stack.get_mut(i) {
                *slot = restored_boundary;
            }
        }
        if warned {
            self.print_nl_cstr("Warning: ")?;
            self.capture_to_diagnostic(true);
            self.diagnostic_print_file_line();
            self.print_cstr("end of ")?;
            self.print_group(true)?;
            self.print_cstr(" of a different file")?;
            self.print_ln();
            if tracing_nesting > 1 {
                self.show_context()?;
            }
            self.capture_to_diagnostic(false);
            if self.history == HISTORY_SPOTLESS {
                self.history = HISTORY_WARNING_ISSUED;
            }
        }
        Ok(())
    }

    /// Warns when a conditional opened in one file ends in another
    /// (`if_warning`).
    pub(crate) fn if_warning(&mut self) -> Result<(), TeXMouthError> {
        let tracing_nesting = self.int_par(INT_PAR_TRACING_NESTING)?;
        let mut base_ptr = self.input_ptr;
        let mut warned = false;
        for i in (0..=self.in_open).rev() {
            if self.if_stack.get(i).copied().unwrap_or(TEX_NULL) != self.cond_ptr {
                break;
            }
            if tracing_nesting > 0 && self.input_source_name_above(&mut base_ptr, i) {
                warned = true;
            }
            let link = self.cond_link()?;
            if let Some(slot) = self.if_stack.get_mut(i) {
                *slot = link;
            }
        }
        if warned {
            self.print_nl_cstr("Warning: ")?;
            self.capture_to_diagnostic(true);
            self.diagnostic_print_file_line();
            self.print_cstr("end of ")?;
            let name = self.if_test_text(i32::from(self.cur_if))?;
            self.print_cstr(&name)?;
            if self.if_line != 0 {
                self.print_cstr(" entered on line ")?;
                self.print_int(self.if_line)?;
            }
            self.print_cstr(" of a different file")?;
            self.print_ln();
            if tracing_nesting > 1 {
                self.show_context()?;
            }
            self.capture_to_diagnostic(false);
            if self.history == HISTORY_SPOTLESS {
                self.history = HISTORY_WARNING_ISSUED;
            }
        }
        Ok(())
    }

    /// Warns about incomplete groups and conditionals at end-of-file
    /// (`file_warning`).
    pub(crate) fn file_warning(&mut self) -> Result<(), TeXMouthError> {
        let save_save_ptr = self.save_ptr;
        let save_level = self.cur_level;
        let save_group = self.cur_group;
        self.save_ptr = self.cur_boundary.max(0) as usize;

        while self
            .grp_stack
            .get(self.in_open)
            .copied()
            .unwrap_or(TEX_NULL)
            != self.save_ptr as i32
        {
            self.cur_level = self.cur_level.saturating_sub(1);
            self.print_nl_cstr("Warning: ")?;
            self.capture_to_diagnostic(true);
            self.diagnostic_print_file_line();
            self.print_cstr("end of file when ")?;
            self.print_group(true)?;
            self.print_cstr(" is incomplete")?;
            self.capture_to_diagnostic(false);
            match self.save_stack.get(self.save_ptr).copied() {
                Some(SaveStackEntry::Boundary { group, boundary }) => {
                    self.cur_group = group;
                    self.save_ptr = boundary.max(0) as usize;
                }
                _ => break,
            }
        }

        self.save_ptr = save_save_ptr;
        self.cur_level = save_level;
        self.cur_group = save_group;

        let save_cond = self.cond_ptr;
        let save_limit = self.if_limit;
        let save_cur_if = self.cur_if;
        let save_if_line = self.if_line;

        while self.if_stack.get(self.in_open).copied().unwrap_or(TEX_NULL) != self.cond_ptr {
            self.print_nl_cstr("Warning: ")?;
            self.capture_to_diagnostic(true);
            self.diagnostic_print_file_line();
            self.print_cstr("end of file when ")?;
            let name = self.if_test_text(i32::from(self.cur_if))?;
            self.print_cstr(&name)?;
            if self.if_limit == FI_CODE as u8 {
                self.print_esc_cstr("else")?;
            }
            if self.if_line != 0 {
                self.print_cstr(" entered on line ")?;
                self.print_int(self.if_line)?;
            }
            self.print_cstr(" is incomplete")?;
            self.capture_to_diagnostic(false);
            let idx = self.checked_engine_mem_index(self.cond_ptr)?;
            self.if_line = self.mem[idx + 1].s1;
            self.cur_if = self.mem[idx].b16_s0() as i16;
            self.if_limit = self.mem[idx].b16_s1() as u8;
            self.cond_ptr = self.mem[idx].s1;
        }

        self.cond_ptr = save_cond;
        self.if_limit = save_limit;
        self.cur_if = save_cur_if;
        self.if_line = save_if_line;
        self.print_ln();
        if self.int_par(INT_PAR_TRACING_NESTING)? > 1 {
            self.capture_to_diagnostic(true);
            self.diagnostic_print_file_line();
            self.show_context()?;
            self.capture_to_diagnostic(false);
        }
        if self.history == HISTORY_SPOTLESS {
            self.history = HISTORY_WARNING_ISSUED;
        }
        Ok(())
    }

    /// Whether the input frame responsible for file level `level` has a real
    /// file name — C's `input_stack[base_ptr].name > 17` after skipping token
    /// lists and deeper file frames. `base_ptr` persists across calls like
    /// C's global.
    fn input_source_name_above(&self, base_ptr: &mut usize, level: usize) -> bool {
        loop {
            let Some(frame) = self.input_stack.get(*base_ptr) else {
                return false;
            };
            if frame.state != TOKEN_LIST && usize::from(frame.index) <= level {
                break;
            }
            if *base_ptr == 0 {
                break;
            }
            *base_ptr -= 1;
        }
        self.input_stack
            .get(*base_ptr)
            .is_some_and(|frame| frame.name > 17)
    }

    /// Link of the current conditional node (`mem[cond_ptr].s1`).
    fn cond_link(&self) -> Result<i32, TeXMouthError> {
        if self.cond_ptr == TEX_NULL {
            return Ok(TEX_NULL);
        }
        let idx = self.checked_engine_mem_index(self.cond_ptr)?;
        Ok(self.mem[idx].s1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{FileInputSource, InputState, MID_LINE};
    use crate::state::{INT_PAR_TRACING_ONLINE, SELECTOR_TERM_AND_LOG};

    fn engine() -> TeXEngineState {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state
    }

    fn term_text(state: &TeXEngineState) -> String {
        String::from_utf8_lossy(&state.term_out).into_owned()
    }

    fn log_text(state: &TeXEngineState) -> String {
        String::from_utf8_lossy(&state.log_out).into_owned()
    }

    #[test]
    fn cmd_chr_text_renders_character_commands() {
        let state = engine();
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::Letter as u8, i32::from(b'x'))
                .unwrap(),
            "the letter x"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::OtherChar as u8, i32::from(b'1'))
                .unwrap(),
            "the character 1"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::Spacer as u8, i32::from(b' '))
                .unwrap(),
            "blank space  "
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::LeftBrace as u8, i32::from(b'{'))
                .unwrap(),
            "begin-group character {"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::SupMark as u8, i32::from(b'^'))
                .unwrap(),
            "superscript character ^"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::MacParam as u8, i32::from(b'#'))
                .unwrap(),
            "macro parameter character #"
        );
        assert_eq!(
            state.cmd_chr_text(CommandCode::Endv as u8, 0).unwrap(),
            "end of alignment template"
        );
    }

    #[test]
    fn cmd_chr_text_renders_register_families() {
        let state = engine();
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::AssignInt as u8, COUNT_BASE as i32)
                .unwrap(),
            "\\count0"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::AssignInt as u8, COUNT_BASE as i32 + 42)
                .unwrap(),
            "\\count42"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::AssignGlue as u8, SKIP_BASE as i32 + 3)
                .unwrap(),
            "\\skip3"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::AssignGlue as u8, MU_SKIP_BASE as i32 + 1)
                .unwrap(),
            "\\muskip1"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::AssignToks as u8, TOKS_BASE as i32 + 5)
                .unwrap(),
            "\\toks5"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::AssignDimen as u8, DIMEN_BASE as i32 + 7)
                .unwrap(),
            "\\dimen7"
        );
    }

    #[test]
    fn cmd_chr_text_renders_conditionals() {
        let state = engine();
        assert_eq!(
            state.cmd_chr_text(CommandCode::IfTest as u8, 2).unwrap(),
            "\\ifnum"
        );
        assert_eq!(
            state.cmd_chr_text(CommandCode::IfTest as u8, 16).unwrap(),
            "\\ifcase"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::IfTest as u8, UNLESS_CODE + 2)
                .unwrap(),
            "\\unless\\ifnum"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::FiOrElse as u8, FI_CODE)
                .unwrap(),
            "\\fi"
        );
        assert_eq!(
            state
                .cmd_chr_text(CommandCode::FiOrElse as u8, OR_CODE)
                .unwrap(),
            "\\or"
        );
        assert_eq!(
            state.cmd_chr_text(CommandCode::FiOrElse as u8, 3).unwrap(),
            "\\else"
        );
    }

    #[test]
    fn show_cur_cmd_chr_emits_mode_and_command() {
        let mut state = engine();
        state.set_int_par(INT_PAR_TRACING_ONLINE, 1).unwrap();
        state.selector = SELECTOR_TERM_AND_LOG;
        state.mode = TeXMode::Vmode as i16;
        state.cur_cmd = CommandCode::Letter as u8;
        state.cur_chr = i32::from(b'x');

        state.show_cur_cmd_chr().unwrap();

        let out = term_text(&state);
        assert!(
            out.contains("{vertical mode: the letter x}"),
            "out: {out:?}"
        );
        assert_eq!(state.shown_mode, i32::from(state.mode));

        state.show_cur_cmd_chr().unwrap();
        let out = term_text(&state);
        assert!(out.contains("{the letter x}"), "out: {out:?}");
    }

    #[test]
    fn trace_conditional_result_respects_tracing_commands() {
        let mut state = engine();
        state.set_int_par(INT_PAR_TRACING_ONLINE, 1).unwrap();
        state.selector = SELECTOR_TERM_AND_LOG;
        state.set_int_par(INT_PAR_TRACING_COMMANDS, 1).unwrap();
        state.trace_conditional_result(true).unwrap();
        assert!(!term_text(&state).contains("{true}"));

        state.set_int_par(INT_PAR_TRACING_COMMANDS, 2).unwrap();
        state.trace_conditional_result(false).unwrap();
        assert!(term_text(&state).contains("{false}"));
    }

    #[test]
    fn group_warning_reports_cross_file_group() {
        let mut state = engine();
        state.set_int_par(INT_PAR_TRACING_ONLINE, 1).unwrap();
        state.selector = SELECTOR_TERM_AND_LOG;
        state.set_int_par(INT_PAR_TRACING_NESTING, 1).unwrap();

        state.cur_boundary = 3;
        state.cur_group = HBOX_GROUP;
        state.in_open = 1;
        state.grp_stack = vec![TEX_NULL, state.cur_boundary];
        state.input_stack.push(InputState::default());
        state.input_stack.push(InputState {
            state: MID_LINE,
            index: 1,
            name: 20,
            ..InputState::default()
        });
        state.input_ptr = 1;
        state.file_sources.push(Default::default());
        state.file_sources.push(FileInputSource {
            name: "outer.tex".to_string(),
            ..Default::default()
        });
        state.line = 7;

        state.group_warning(0).unwrap();

        assert!(
            term_text(&state).contains("end of hbox group (level 1) of a different file"),
            "out: {:?}",
            term_text(&state)
        );
        assert_eq!(state.grp_stack[1], 0);
        assert_eq!(state.history, HISTORY_WARNING_ISSUED);
    }

    #[test]
    fn if_warning_reports_cross_file_conditional() {
        let mut state = engine();
        state.set_int_par(INT_PAR_TRACING_ONLINE, 1).unwrap();
        state.selector = SELECTOR_TERM_AND_LOG;
        state.set_int_par(INT_PAR_TRACING_NESTING, 1).unwrap();

        let cond = state.get_node(2).unwrap();
        state.mem[cond as usize].s1 = TEX_NULL;
        state.cond_ptr = cond;
        state.cur_if = 2;
        state.if_line = 11;
        state.in_open = 1;
        state.if_stack = vec![TEX_NULL, cond];
        state.input_stack.push(InputState::default());
        state.input_stack.push(InputState {
            state: MID_LINE,
            index: 1,
            name: 20,
            ..InputState::default()
        });
        state.input_ptr = 1;
        state.file_sources.push(Default::default());
        state.file_sources.push(FileInputSource {
            name: "outer.tex".to_string(),
            ..Default::default()
        });
        state.line = 7;

        state.if_warning().unwrap();

        let out = term_text(&state);
        assert!(
            out.contains("end of \\ifnum entered on line 11 of a different file"),
            "out: {out:?}"
        );
        assert_eq!(state.if_stack[1], TEX_NULL);
        assert_eq!(state.history, HISTORY_WARNING_ISSUED);
    }

    #[test]
    fn file_warning_reports_incomplete_group_and_conditional() {
        let mut state = engine();
        state.set_int_par(INT_PAR_TRACING_ONLINE, 1).unwrap();
        state.selector = SELECTOR_TERM_AND_LOG;
        state.set_int_par(INT_PAR_TRACING_NESTING, 1).unwrap();

        state.in_open = 1;
        state.cur_boundary = 1;
        state.cur_level = 1;
        state.cur_group = HBOX_GROUP;
        state.save_ptr = 1;
        state.grp_stack = vec![TEX_NULL, 0];
        state.save_stack = vec![
            SaveStackEntry::Line(4),
            SaveStackEntry::Boundary {
                group: BOTTOM_LEVEL,
                boundary: 0,
            },
        ];

        let cond = state.get_node(2).unwrap();
        state.mem[cond as usize].set_b16_s0(2);
        state.mem[cond as usize].set_b16_s1(1);
        state.mem[(cond + 1) as usize].s1 = 9;
        state.mem[cond as usize].s1 = TEX_NULL;
        state.cond_ptr = cond;
        state.cur_if = 2;
        state.if_limit = 1;
        state.if_line = 9;
        state.if_stack = vec![TEX_NULL, 0];

        state.file_sources.push(Default::default());
        state.file_sources.push(FileInputSource {
            name: "partial.tex".to_string(),
            ..Default::default()
        });
        state.line = 5;

        state.file_warning().unwrap();

        let out = log_text(&state);
        assert!(
            out.contains("end of file when hbox group (level 0) entered at line 4 is incomplete"),
            "out: {out:?}"
        );
        assert!(
            out.contains("end of file when \\ifnum entered on line 9 is incomplete"),
            "out: {out:?}"
        );
        assert_eq!(state.cond_ptr, cond);
        assert_eq!(state.save_ptr, 1);
        assert_eq!(state.history, HISTORY_WARNING_ISSUED);
    }
}
