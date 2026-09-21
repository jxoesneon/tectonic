// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust TeX token scanner and token-list input stack.
//!
//! Fulfills the first half of Phase 5 Milestone 2 in `MIGRATION_ROADMAP.md`:
//! - Ports `get_next`, `get_token`, `begin_token_list`, `end_token_list`,
//!   `back_input`, and control-sequence lookup out of `xetex-xetex0.c`.
//! - Preserves TeX token encoding (`command << 21 | character`) and linked
//!   token-list storage inside [`TeXEngineState::mem`].
//! - Keeps all mutable scanner state inside [`TeXEngineState`] (Safety Gate 5).

use std::collections::{HashMap, VecDeque};
use std::fmt;

use crate::primitives::{primitive_defs, PrimitiveExtraInit};
use crate::scan::{
    BIGGEST_CHAR, DIMEN_VAL, ETEX_EXPR, GLUE_TO_MU_CODE, GLUE_VAL, INT_VAL, MU_TO_GLUE_CODE,
    MU_VAL, OTHER_TOKEN,
};
use crate::state::{
    CommandCode, MemoryWord, SaveStackEntry, SaveTarget, TeXEngineState, TeXMemoryError, TeXMode,
    BATCH_MODE, CAT_CODE_BASE, DEL_CODE_BASE, DEPTH_OFFSET, ERROR_STOP_MODE, ETEX_PEN_BASE,
    ETEX_PEN_CLUB, ETEX_PEN_DISPLAY_WIDOW, ETEX_PEN_INTER_LINE, ETEX_PEN_WIDOW, EXPLICIT,
    GLUE_NODE, HEIGHT_OFFSET, LC_CODE_BASE, LEVEL_ONE, LOCAL_BASE, LOCAL_PAR_SHAPE, MATH_CODE_BASE,
    NONSTOP_MODE, RESTORE_OLD_VALUE, RESTORE_ZERO, RIGHT_NOAD, SCROLL_MODE, SF_CODE_BASE,
    SKIP_CODE, SPECIAL_NODE, TEX_NULL, UC_CODE_BASE, WIDTH_OFFSET,
};

/// Largest Unicode scalar value accepted by XeTeX (`BIGGEST_USV`).
pub const BIGGEST_USV: i32 = 0x10FFFF;
/// One past the largest Unicode scalar value (`TOO_BIG_USV`).
pub const TOO_BIG_USV: i32 = 0x110000;
/// Number of codepoints per TeX token command block (`MAX_CHAR_VAL`).
pub const MAX_CHAR_VAL: i32 = 0x200000;
/// Flag added to a control-sequence index to form a control-sequence token.
pub const CS_TOKEN_FLAG: i32 = 0x1FFFFFF;
/// Token for `{` (`LEFT_BRACE_TOKEN`).
pub const LEFT_BRACE_TOKEN: i32 = 0x200000;
/// First token after the left-brace command block.
pub const LEFT_BRACE_LIMIT: i32 = 0x400000;
/// Token for `}` (`RIGHT_BRACE_TOKEN`).
pub const RIGHT_BRACE_TOKEN: i32 = 0x400000;

/// Token base for math shift (`MATH_SHIFT_TOKEN`).
pub const MATH_SHIFT_TOKEN: i32 = 3 * MAX_CHAR_VAL;
/// First token after the right-brace command block.
pub const RIGHT_BRACE_LIMIT: i32 = 0x600000;
/// Token for a space (`SPACE_TOKEN`).
pub const SPACE_TOKEN: i32 = 0x1400020;
/// Token for an other-character zero (`ZERO_TOKEN`).
pub const ZERO_TOKEN: i32 = 12 * MAX_CHAR_VAL + b'0' as i32;
/// Token marking a macro parameter reference (`OUT_PARAM_TOKEN`).
pub const OUT_PARAM_TOKEN: i32 = 0xA00000;
/// First macro parameter-match token (`MATCH_TOKEN`).
pub const MATCH_TOKEN: i32 = 0x1A00000;
/// Token terminating a macro parameter template (`END_MATCH_TOKEN`).
pub const END_MATCH_TOKEN: i32 = 0x1C00000;
/// Token marking a protected macro definition (`PROTECTED_TOKEN`).
pub const PROTECTED_TOKEN: i32 = END_MATCH_TOKEN + 1;
/// Marker returned through `cur_chr` when `\noexpand` suppresses expansion.
pub const NO_EXPAND_FLAG: i32 = BIGGEST_USV + 2;

/// Token-list input state (`TOKEN_LIST`).
pub const TOKEN_LIST: u16 = 0;
/// Mid-line scanner state (`MID_LINE`).
pub const MID_LINE: u16 = 1;
/// Blank-skipping scanner state (`SKIP_BLANKS`).
pub const SKIP_BLANKS: u16 = 17;
/// New-line scanner state (`NEW_LINE`).
pub const NEW_LINE: u16 = 33;

/// Token-list kind: parameter substitution (`PARAMETER`).
pub const PARAMETER: u16 = 0;
/// Token-list kind: upper alignment template (`U_TEMPLATE`).
pub const U_TEMPLATE: u16 = 1;
/// Token-list kind: lower alignment template (`V_TEMPLATE`).
pub const V_TEMPLATE: u16 = 2;
/// Token-list kind: one pushed-back token (`BACKED_UP`).
pub const BACKED_UP: u16 = 3;
/// Token-list kind: backed-up character (`BACKED_UP_CHAR`).
pub const BACKED_UP_CHAR: u16 = 4;
/// Token-list kind: inserted recovery text (`INSERTED`).
pub const INSERTED: u16 = 5;
/// Token-list kind: macro replacement text (`MACRO`).
pub const MACRO: u16 = 6;
/// Token-list kind: output routine (`OUTPUT_TEXT`).
pub const OUTPUT_TEXT: u16 = 7;
/// Token-list kind: `\everypar` (`EVERY_PAR_TEXT`).
pub const EVERY_PAR_TEXT: u16 = 8;
/// Token-list kind: `\everymath` (`EVERY_MATH_TEXT`).
pub const EVERY_MATH_TEXT: u16 = 9;
/// Token-list kind: `\everydisplay` (`EVERY_DISPLAY_TEXT`).
pub const EVERY_DISPLAY_TEXT: u16 = 10;
/// Token-list kind: `\everyhbox` (`EVERY_HBOX_TEXT`).
pub const EVERY_HBOX_TEXT: u16 = 11;
/// Token-list kind: `\everyvbox` (`EVERY_VBOX_TEXT`).
pub const EVERY_VBOX_TEXT: u16 = 12;
/// Token-list kind: `\everyjob` (`EVERY_JOB_TEXT`).
pub const EVERY_JOB_TEXT: u16 = 13;
/// Token-list kind: `\everycr` (`EVERY_CR_TEXT`).
pub const EVERY_CR_TEXT: u16 = 14;
/// Token-list kind: mark text (`MARK_TEXT`).
pub const MARK_TEXT: u16 = 15;
/// Token-list kind: `\everyeof` (`EVERY_EOF_TEXT`).
pub const EVERY_EOF_TEXT: u16 = 16;
/// Token-list kind: inter-character text (`INTER_CHAR_TEXT`).
pub const INTER_CHAR_TEXT: u16 = 17;
/// Token-list kind: `\write` text (`WRITE_TEXT`).
pub const WRITE_TEXT: u16 = 18;
/// Token-list kind: Tectonic coda text (`TECTONIC_CODA_TEXT`).
pub const TECTONIC_CODA_TEXT: u16 = 19;

/// Scanner status: normal scanning (`NORMAL`).
pub const NORMAL: u8 = 0;
/// Scanner status: skipping conditional text (`SKIPPING`).
pub const SKIPPING: u8 = 1;
/// Scanner status: scanning a definition (`DEFINING`).
pub const DEFINING: u8 = 2;
/// Scanner status: matching macro arguments (`MATCHING`).
pub const MATCHING: u8 = 3;
/// Scanner status: scanning an alignment preamble (`ALIGNING`).
pub const ALIGNING: u8 = 4;
/// Scanner status: absorbing general text (`ABSORBING`).
pub const ABSORBING: u8 = 5;

/// Category code for escape characters.
pub const CAT_ESCAPE: u8 = 0;
/// Category code for left braces.
pub const CAT_LEFT_BRACE: u8 = 1;
/// Category code for right braces.
pub const CAT_RIGHT_BRACE: u8 = 2;
/// Category code for math shifts.
pub const CAT_MATH_SHIFT: u8 = 3;
/// Category code for alignment tabs.
pub const CAT_TAB_MARK: u8 = 4;
/// Category code for carriage returns / line ends.
pub const CAT_CAR_RET: u8 = 5;
/// Category code for macro parameter marks.
pub const CAT_MAC_PARAM: u8 = 6;
/// Category code for superscript marks.
pub const CAT_SUP_MARK: u8 = 7;
/// Category code for subscript marks.
pub const CAT_SUB_MARK: u8 = 8;
/// Category code for ignored characters.
pub const CAT_IGNORE: u8 = 9;
/// Category code for spaces.
pub const CAT_SPACER: u8 = 10;
/// Category code for letters.
pub const CAT_LETTER: u8 = 11;
/// Category code for other characters.
pub const CAT_OTHER_CHAR: u8 = 12;
/// Category code for active characters.
pub const CAT_ACTIVE_CHAR: u8 = 13;
/// Category code for comments.
pub const CAT_COMMENT: u8 = 14;
/// Category code for invalid characters.
pub const CAT_INVALID_CHAR: u8 = 15;

/// TeX command code for `OUT_PARAM` tokens inside token lists.
pub const OUT_PARAM: u8 = 5;
/// TeX command code for `CAR_RET` characters in input buffers.
pub const CAR_RET: u8 = 5;
/// Maximum non-expandable TeX command code (`MAX_COMMAND`).
pub const MAX_COMMAND: u8 = 102;

/// Reserved control-sequence id for an undefined sequence.
pub const UNDEFINED_CS_ID: i32 = 1;
/// Reserved control-sequence id for `\csname\endcsname` (`NULL_CS`).
pub const NULL_CS_ID: i32 = 2;
/// Reserved control-sequence id for `\par`.
pub const PAR_CS_ID: i32 = 3;
/// Reserved control-sequence id for frozen `\endv`.
pub const FROZEN_ENDV_ID: i32 = 4;
/// Reserved control-sequence id for frozen `\relax`.
pub const FROZEN_RELAX_ID: i32 = 5;
/// Reserved control-sequence id for frozen `\noexpand` deferral.
pub const FROZEN_DONT_EXPAND_ID: i32 = 6;
/// Reserved control-sequence id for frozen `\primitive` markers.
pub const FROZEN_PRIMITIVE_ID: i32 = 7;
/// Reserved control-sequence id for frozen `\fi`.
pub const FROZEN_FI_ID: i32 = 8;
/// Reserved control-sequence id inserted by `get_r_token` error recovery.
pub const FROZEN_PROTECTION_ID: i32 = 9;
/// Reserved control-sequence id for frozen `\cr`.
pub const FROZEN_CR_ID: i32 = 10;
/// Reserved control-sequence id for frozen `\endgroup`.
pub const FROZEN_END_GROUP_ID: i32 = 11;
/// Reserved control-sequence id for frozen `\right`.
pub const FROZEN_RIGHT_ID: i32 = 12;
/// Reserved control-sequence id for frozen `endtemplate`.
pub const FROZEN_END_TEMPLATE_ID: i32 = 13;
/// Reserved control-sequence id for the outer `endwrite` macro.
pub const END_WRITE_ID: i32 = 14;
/// Reserved control-sequence id for frozen `\special`.
pub const FROZEN_SPECIAL_ID: i32 = 15;
/// Reserved control-sequence id for frozen `\nullfont`.
pub const FROZEN_NULL_FONT_ID: i32 = 16;

/// `\span` subcommand marker (`SPAN_CODE`).
pub const SPAN_CODE: i32 = 0x110001;
/// `\cr` subcommand marker (`CR_CODE`).
pub const CR_CODE: i32 = 0x110002;
/// `\crcr` subcommand marker (`CR_CR_CODE`).
pub const CR_CR_CODE: i32 = 0x110003;

/// Constructs an `InputOutOfBounds` error for index `i`.
pub(crate) fn oob(i: i32) -> TeXMouthError {
    TeXMouthError::InputOutOfBounds(i)
}

/// Errors produced by the pure-Rust token scanner and expansion input stack.
#[derive(Debug)]
pub enum TeXMouthError {
    /// Dynamic TeX memory operation failed.
    Memory(TeXMemoryError),
    /// The input stack or input buffer was indexed outside valid bounds.
    InputOutOfBounds(i32),
    /// The scanner reached end-of-input with no enclosing input frame.
    EndOfInput,
    /// A forbidden outer control sequence was read while scanning restricted text.
    ForbiddenControlSequence(i32),
    /// An invalid input character was encountered.
    InvalidCharacter(i32),
    /// A right brace occurred where TeX expected ordinary argument text.
    ExtraRightBrace,
    /// A macro argument ended before its parameter text was satisfied.
    RunawayArgument,
    /// A `\par` token occurred inside a non-long macro argument.
    ParagraphInShortArgument,
    /// `\csname` was not terminated by `\endcsname`.
    MissingEndcsname,
    /// An undefined control sequence reached the expansion dispatcher.
    UndefinedControlSequence(i32),
    /// A primitive dispatch case has not yet been implemented.
    UnsupportedPrimitive(&'static str),
    /// The configured expansion-depth limit was exceeded.
    ExpansionDepthExceeded,
    /// The input-stack capacity was exceeded.
    InputStackOverflow,
    /// The macro parameter-stack capacity was exceeded.
    ParameterStackOverflow,
    /// A macro parameter template was malformed.
    MalformedMacroParameters,
    /// A token did not match macro parameter text.
    MacroArgumentMismatch {
        /// Token required by the macro parameter text.
        expected: i32,
        /// Token actually read from the input.
        found: i32,
    },
    /// `\input` referenced a source that is not registered.
    MissingInputFile(String),
    /// A single input line exceeded `buf_size` (`buffer_overflow`).
    BufferOverflow(usize),
    /// A format file failed a load-time consistency check (`bad_fmt`).
    BadFormatFile,
}

impl fmt::Display for TeXMouthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Memory(err) => write!(f, "{err}"),
            Self::InputOutOfBounds(i) => write!(f, "input index {i} out of bounds"),
            Self::EndOfInput => write!(f, "end of TeX input"),
            Self::ForbiddenControlSequence(cs) => {
                write!(f, "forbidden outer control sequence {cs}")
            }
            Self::InvalidCharacter(c) => write!(f, "invalid input character U+{c:04X}"),
            Self::ExtraRightBrace => write!(f, "macro argument has an extra }}"),
            Self::RunawayArgument => write!(f, "runaway macro argument"),
            Self::ParagraphInShortArgument => {
                write!(
                    f,
                    "paragraph ended before a short macro argument was complete"
                )
            }
            Self::MissingEndcsname => write!(f, "missing \\endcsname inserted"),
            Self::UndefinedControlSequence(cs) => {
                write!(f, "undefined control sequence {cs}")
            }
            Self::UnsupportedPrimitive(name) => write!(f, "unsupported primitive {name}"),
            Self::ExpansionDepthExceeded => write!(f, "TeX expansion depth exceeded"),
            Self::InputStackOverflow => write!(f, "TeX input stack overflow"),
            Self::ParameterStackOverflow => write!(f, "TeX parameter stack overflow"),
            Self::MalformedMacroParameters => write!(f, "malformed macro parameter template"),
            Self::MacroArgumentMismatch { expected, found } => write!(
                f,
                "macro argument token {found:#X} does not match expected token {expected:#X}"
            ),
            Self::MissingInputFile(name) => write!(f, "input source {name:?} is not registered"),
            Self::BufferOverflow(size) => {
                write!(f, "unable to read an entire line (buf_size={size})")
            }
            Self::BadFormatFile => write!(f, "fatal format file error"),
        }
    }
}

impl std::error::Error for TeXMouthError {}

impl From<TeXMemoryError> for TeXMouthError {
    fn from(err: TeXMemoryError) -> Self {
        Self::Memory(err)
    }
}

/// A single input frame, mirroring C `input_state_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputState {
    /// Tokenizer state: `TOKEN_LIST`, `MID_LINE`, `SKIP_BLANKS`, or `NEW_LINE`.
    pub state: u16,
    /// File index or token-list kind.
    pub index: u16,
    /// Start position in `buffer`, or token-list start node.
    pub start: i32,
    /// Next buffer position, or current token-list node.
    pub loc: i32,
    /// Last valid buffer position, or macro parameter base.
    pub limit: i32,
    /// File/string name, or macro warning index.
    pub name: i32,
    /// SyncTeX tag associated with this input line.
    pub synctex_tag: i32,
}

/// One open file-like source, preserving its current line buffer and queued
/// input lines while a nested `\input` frame is active.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileInputSource {
    /// Source name used to open this input level.
    pub name: String,
    /// TeX-visible source name id; real input files use names above 17.
    pub name_id: i32,
    /// Current input-line buffer for this source.
    pub buffer: Vec<i32>,
    /// Encoded lines not yet consumed by this source.
    pub lines: VecDeque<Vec<i32>>,
    /// Input encoding mode selected by `\XeTeXinputencoding`.
    pub input_mode: i32,
    /// Interned input-encoding name selected by `\XeTeXinputencoding`.
    pub input_encoding: i32,
}

impl FileInputSource {
    /// Creates a file input source from encoded TeX input lines.
    pub fn new(name: impl Into<String>, name_id: i32, lines: VecDeque<Vec<i32>>) -> Self {
        Self {
            name: name.into(),
            name_id,
            buffer: Vec::new(),
            lines,
            input_mode: 0,
            input_encoding: 0,
        }
    }
}

/// C `read_open` state for TeX read streams.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReadOpen {
    /// The stream is closed (`\ifeof` is true).
    #[default]
    Closed,
    /// The stream has just been opened but no line has been read.
    JustOpen,
    /// The stream has successfully read at least one line.
    Normal,
}

/// A named or active control sequence in the compact pure-Rust equivalents table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlSequence {
    name: Vec<i32>,
    command: u8,
    character: i32,
    level: u16,
    active: bool,
}

impl ControlSequence {
    fn new(name: Vec<i32>, command: u8, character: i32, level: u16, active: bool) -> Self {
        Self {
            name,
            command,
            character,
            level,
            active,
        }
    }

    /// Returns the control-sequence name as Unicode scalar values.
    pub fn name(&self) -> &[i32] {
        &self.name
    }

    /// Returns the TeX command code assigned to this sequence.
    pub fn command(&self) -> u8 {
        self.command
    }

    /// Returns the `cur_chr`/register payload assigned to this sequence.
    pub fn character(&self) -> i32 {
        self.character
    }

    /// Returns whether this sequence came from an active character.
    pub fn is_active(&self) -> bool {
        self.active
    }
}

/// Compact pure-Rust control-sequence lookup table.
///
/// This preserves the observable semantics of `id_lookup`: unknown names are
/// created during normal scanning, resolve to `undefined_cs` while absorbing
/// tokens, and receive stable control-sequence ids for token encoding.
#[derive(Clone, Debug)]
pub struct ControlSequenceTable {
    sequences: Vec<ControlSequence>,
    lookup: HashMap<Vec<i32>, i32>,
    active_lookup: HashMap<i32, i32>,
    primitive_lookup: HashMap<Vec<i32>, (u8, i32)>,
}

impl Default for ControlSequenceTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ControlSequenceTable {
    /// Creates a table containing the reserved control sequences needed by the
    /// scanner and expansion dispatcher.
    pub fn new() -> Self {
        let mut table = Self {
            sequences: Vec::with_capacity(512),
            lookup: HashMap::new(),
            active_lookup: HashMap::new(),
            primitive_lookup: HashMap::new(),
        };

        table.push_reserved(
            Vec::new(),
            CommandCode::Relax as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 0: no cs
        table.push_reserved(
            vec![-1],
            CommandCode::UndefinedCs as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 1: undefined sentinel
        table.push_reserved(
            Vec::new(),
            CommandCode::Relax as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 2: null cs
        table.lookup.insert(Vec::new(), NULL_CS_ID);
        table.push_reserved(
            vec![i32::from(b'p'), i32::from(b'a'), i32::from(b'r')],
            CommandCode::ParEnd as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 3: \par
        table.lookup.insert(
            vec![i32::from(b'p'), i32::from(b'a'), i32::from(b'r')],
            PAR_CS_ID,
        );
        table.push_reserved(
            vec![-2],
            CommandCode::Endv as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 4: frozen endv
        table.push_reserved(
            vec![-3],
            CommandCode::Relax as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 5: frozen relax
        table.push_reserved(
            vec![-4],
            CommandCode::DontExpand as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 6: frozen dont_expand
        table.push_reserved(
            vec![-5],
            CommandCode::IgnoreSpaces as u8,
            1,
            crate::state::LEVEL_ONE,
            false,
        ); // id 7: frozen primitive
        table.push_reserved(
            vec![-6],
            CommandCode::FiOrElse as u8,
            2,
            crate::state::LEVEL_ONE,
            false,
        ); // id 8: frozen fi
        table.push_reserved(
            "inaccessible".chars().map(|c| c as i32).collect(),
            CommandCode::UndefinedCs as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 9: frozen protection recovery cs
        table.push_reserved(
            "cr".chars().map(|c| c as i32).collect(),
            CommandCode::CarRet as u8,
            CR_CODE,
            crate::state::LEVEL_ONE,
            false,
        ); // id 10: frozen \cr
        table.push_reserved(
            "endgroup".chars().map(|c| c as i32).collect(),
            CommandCode::EndGroup as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 11: frozen \endgroup
        table.push_reserved(
            "right".chars().map(|c| c as i32).collect(),
            CommandCode::LeftRight as u8,
            i32::from(RIGHT_NOAD),
            crate::state::LEVEL_ONE,
            false,
        ); // id 12: frozen \right
        table.push_reserved(
            "endtemplate".chars().map(|c| c as i32).collect(),
            CommandCode::EndTemplate as u8,
            TEX_NULL,
            crate::state::LEVEL_ONE,
            false,
        ); // id 13: frozen endtemplate
        table.push_reserved(
            "endwrite".chars().map(|c| c as i32).collect(),
            CommandCode::OuterCall as u8,
            TEX_NULL,
            crate::state::LEVEL_ONE,
            false,
        ); // id 14: endwrite
        table.push_reserved(
            "special".chars().map(|c| c as i32).collect(),
            CommandCode::Extension as u8,
            i32::from(SPECIAL_NODE),
            crate::state::LEVEL_ONE,
            false,
        ); // id 15: frozen \special
        table.push_reserved(
            "nullfont".chars().map(|c| c as i32).collect(),
            CommandCode::SetFont as u8,
            0,
            crate::state::LEVEL_ONE,
            false,
        ); // id 16: frozen \nullfont

        table
    }

    fn push_reserved(
        &mut self,
        name: Vec<i32>,
        command: u8,
        character: i32,
        level: u16,
        active: bool,
    ) -> i32 {
        self.sequences.push(ControlSequence::new(
            name, command, character, level, active,
        ));
        (self.sequences.len() - 1) as i32
    }

    /// Returns the number of allocated control-sequence ids.
    pub fn len(&self) -> usize {
        self.sequences.len()
    }

    /// Returns whether the table contains no user-defined sequences.
    pub fn is_empty(&self) -> bool {
        self.sequences.len() <= 17
    }

    /// Looks up an existing named control sequence without creating one.
    pub fn lookup_units(&self, name: &[i32]) -> Option<i32> {
        self.lookup.get(name).copied()
    }

    /// Looks up an existing named control sequence encoded from a Rust string.
    pub fn lookup_str(&self, name: &str) -> Option<i32> {
        let units: Vec<i32> = name.chars().map(|c| c as i32).collect();
        self.lookup_units(&units)
    }

    /// Records the immutable primitive meaning for `name` (`prim_eqtb`).
    pub fn register_primitive_units(&mut self, name: &[i32], command: u8, character: i32) {
        self.primitive_lookup
            .insert(name.to_vec(), (command, character));
    }

    /// Returns the original primitive meaning of `name`, independent of later
    /// control-sequence redefinitions (`prim_lookup`).
    pub fn prim_lookup_units(&self, name: &[i32]) -> Option<(u8, i32)> {
        self.primitive_lookup.get(name).copied()
    }

    /// Returns the original primitive meaning of `name` (`prim_lookup`).
    pub fn prim_lookup_str(&self, name: &str) -> Option<(u8, i32)> {
        let units: Vec<i32> = name.chars().map(|c| c as i32).collect();
        self.prim_lookup_units(&units)
    }

    /// Returns the number of installed primitive meanings.
    pub fn primitive_count(&self) -> usize {
        self.primitive_lookup.len()
    }

    /// Returns the command code assigned to a control-sequence id.
    pub fn command(&self, cs: i32) -> u8 {
        usize::try_from(cs)
            .ok()
            .and_then(|index| self.sequences.get(index))
            .map_or(CommandCode::Relax as u8, |seq| seq.command)
    }

    /// Returns the `cur_chr` payload assigned to a control-sequence id.
    pub fn character(&self, cs: i32) -> i32 {
        usize::try_from(cs)
            .ok()
            .and_then(|index| self.sequences.get(index))
            .map_or(0, |seq| seq.character)
    }

    /// Returns the grouping level assigned to a control-sequence id.
    pub fn level(&self, cs: i32) -> Result<u16, TeXMemoryError> {
        usize::try_from(cs)
            .ok()
            .and_then(|index| self.sequences.get(index))
            .map(|seq| seq.level)
            .ok_or(TeXMemoryError::OutOfBounds(cs, self.sequences.len()))
    }

    /// Restores a control sequence's command, payload, and level.
    pub fn restore(
        &mut self,
        cs: i32,
        command: u8,
        character: i32,
        level: u16,
    ) -> Result<(), TeXMemoryError> {
        let len = self.sequences.len();
        let index = usize::try_from(cs)
            .ok()
            .filter(|index| *index < len)
            .ok_or(TeXMemoryError::OutOfBounds(cs, len))?;
        let seq = &mut self.sequences[index];
        seq.command = command;
        seq.character = character;
        seq.level = level;
        Ok(())
    }

    /// Returns the name of a control-sequence id.
    pub fn name(&self, cs: i32) -> &[i32] {
        usize::try_from(cs)
            .ok()
            .and_then(|index| self.sequences.get(index))
            .map_or(&[], |seq| seq.name.as_slice())
    }

    /// Returns whether a control-sequence id came from an active character.
    pub fn is_active(&self, cs: i32) -> bool {
        usize::try_from(cs)
            .ok()
            .and_then(|index| self.sequences.get(index))
            .is_some_and(ControlSequence::is_active)
    }

    /// Assigns a command code and payload to an existing control sequence.
    pub fn set(&mut self, cs: i32, command: u8, character: i32) -> Result<(), TeXMouthError> {
        let index = usize::try_from(cs)
            .ok()
            .filter(|index| *index < self.sequences.len())
            .ok_or(TeXMouthError::UndefinedControlSequence(cs))?;
        let seq = &mut self.sequences[index];
        seq.command = command;
        seq.character = character;
        Ok(())
    }

    /// Defines or updates a named control sequence.
    pub fn define_units(&mut self, name: &[i32], command: u8, character: i32) -> i32 {
        let cs = self.lookup_or_insert(name);
        if let Some(seq) = self.sequences.get_mut(cs as usize) {
            seq.command = command;
            seq.character = character;
            seq.level = crate::state::LEVEL_ONE;
        }
        cs
    }

    /// Defines or updates a named control sequence from a Rust string.
    pub fn define_str(&mut self, name: &str, command: CommandCode, character: i32) -> i32 {
        let units: Vec<i32> = name.chars().map(|c| c as i32).collect();
        self.define_units(&units, command as u8, character)
    }

    /// Defines or updates an active-character control sequence.
    pub fn define_active(&mut self, character: i32, command: CommandCode, payload: i32) -> i32 {
        if let Some(cs) = self.active_lookup.get(&character) {
            if let Some(seq) = self.sequences.get_mut(*cs as usize) {
                seq.command = command as u8;
                seq.character = payload;
                seq.level = crate::state::LEVEL_ONE;
            }
            return *cs;
        }

        let cs = self.push_reserved(
            vec![character],
            command as u8,
            payload,
            crate::state::LEVEL_ONE,
            true,
        );
        self.active_lookup.insert(character, cs);
        cs
    }

    /// Looks up an active-character control sequence without creating one.
    pub fn lookup_active(&self, character: i32) -> Option<i32> {
        self.active_lookup.get(&character).copied()
    }

    fn lookup_or_insert(&mut self, name: &[i32]) -> i32 {
        if let Some(cs) = self.lookup.get(name) {
            return *cs;
        }

        let cs = self.push_reserved(
            name.to_vec(),
            CommandCode::UndefinedCs as u8,
            0,
            crate::state::LEVEL_ZERO,
            false,
        );
        self.lookup.insert(name.to_vec(), cs);
        cs
    }

    fn lookup_or_undefined(&mut self, name: &[i32], create: bool) -> i32 {
        if create {
            self.lookup_or_insert(name)
        } else {
            self.lookup_units(name).unwrap_or(UNDEFINED_CS_ID)
        }
    }
}

fn is_lower_hex(c: i32) -> bool {
    matches!(c, 0x30..=0x39 | 0x61..=0x66)
}

fn push_character(c: i32, output: &mut String) {
    if let Ok(c) = u32::try_from(c) {
        if let Some(c) = char::from_u32(c) {
            output.push(c);
            return;
        }
    }
    output.push('\u{FFFD}');
}

fn push_units(units: &[i32], output: &mut String) {
    let mut i = 0;
    while i < units.len() {
        let c = units[i];
        if (0xD800..=0xDBFF).contains(&c) && i + 1 < units.len() {
            let low = units[i + 1];
            if (0xDC00..=0xDFFF).contains(&low) {
                let scalar = 0x10000 + ((c - 0xD800) << 10) + low - 0xDC00;
                push_character(scalar, output);
                i += 2;
                continue;
            }
        }
        push_character(c, output);
        i += 1;
    }
}

fn hex_value(c: i32) -> i32 {
    if matches!(c, 0x30..=0x39) {
        c - i32::from(b'0')
    } else {
        c - i32::from(b'a') + 10
    }
}

impl TeXEngineState {
    /// Loads raw UTF-16/USV buffer units and makes them the current input line.
    pub fn set_source_units(&mut self, units: &[i32]) {
        self.source_lines.clear();
        self.set_current_source_line(units.to_vec());
    }

    /// Loads a Rust string as TeX input, one TeX input line at a time.
    pub fn set_source_text(&mut self, source: &str) {
        if source.is_empty() {
            self.source_lines.clear();
            self.set_current_source_line(Vec::new());
            return;
        }

        let mut lines = Self::encode_source_text(source);
        let first_line = lines.pop_front().unwrap_or_default();
        self.source_lines = lines;
        self.set_current_source_line(first_line);
    }

    fn set_current_source_line(&mut self, units: Vec<i32>) {
        self.buffer = units;
        self.input_stack.clear();
        self.input_ptr = 0;
        self.max_in_stack = 0;
        self.in_open = 0;
        self.file_sources.clear();
        self.file_sources.push(FileInputSource::default());
        self.input_file.clear();
        self.eof_seen.clear();
        self.eof_seen.push(false);
        self.line_stack.clear();
        self.line_stack.push(0);
        self.grp_stack.clear();
        self.grp_stack.push(self.cur_boundary);
        self.if_stack.clear();
        self.if_stack.push(self.cond_ptr);
        self.cur_cs = 0;
        self.cur_tok = 0;
        self.scanner_status = NORMAL;
        self.warning_index = 0;
        self.def_ref = TEX_NULL;
        self.param_ptr = 0;
        self.pstack = [TEX_NULL; 9];
        self.align_state = 1_000_000;
        self.force_eof = false;
        self.expand_depth_count = 0;
        self.is_in_csname = false;
        self.long_state = CommandCode::Call as u8;
        self.cond_ptr = TEX_NULL;
        self.if_limit = 0;
        self.cur_if = 0;
        self.if_line = 0;
        self.skip_line = 0;
        self.name_in_progress = false;
        self.cur_input = InputState {
            state: MID_LINE,
            index: 0,
            start: 0,
            loc: 0,
            limit: self.buffer.len() as i32 - 1,
            name: 1,
            synctex_tag: 0,
        };
        self.first = 0;
        self.last = self.cur_input.limit;
        self.line = 1;
    }

    fn apply_end_line_char(&mut self) {
        match self.int_par(crate::state::INT_PAR_END_LINE_CHAR) {
            Ok(end_line_char) if !(0..=255).contains(&end_line_char) => {
                self.cur_input.limit -= 1;
            }
            Ok(end_line_char) => {
                let limit = self.cur_input.limit.max(0) as usize;
                if limit == self.buffer.len() {
                    self.buffer.push(end_line_char);
                } else if limit < self.buffer.len() {
                    self.buffer[limit] = end_line_char;
                }
            }
            Err(_) => self.cur_input.limit -= 1,
        }
    }

    fn load_next_source_line(&mut self, advance_line: bool) -> Result<bool, TeXMouthError> {
        if let Some(mut f) = self.input_file.get_mut(self.in_open).and_then(Option::take) {
            self.first = 0;
            let got = self.input_line(&mut f);
            self.input_file[self.in_open] = Some(f);
            if got? {
                self.cur_input.start = 0;
                self.cur_input.loc = 0;
                self.cur_input.limit = self.last;
                self.cur_input.state = NEW_LINE;
                self.apply_end_line_char();
                self.first = self.cur_input.limit + 1;
                self.last = self.cur_input.limit;
                if advance_line {
                    self.line += 1;
                }
                return Ok(true);
            }
            return Ok(false);
        }

        let Some(units) = self.source_lines.pop_front() else {
            return Ok(false);
        };
        self.buffer = units;
        self.cur_input.start = 0;
        self.cur_input.loc = 0;
        self.cur_input.limit = self.buffer.len() as i32 - 1;
        self.cur_input.state = NEW_LINE;
        self.apply_end_line_char();
        self.first = self.cur_input.limit + 1;
        self.last = self.cur_input.limit;
        if advance_line {
            self.line += 1;
        }
        Ok(true)
    }

    fn encode_source_text(source: &str) -> VecDeque<Vec<i32>> {
        let mut lines: Vec<&str> = source.split('\n').collect();
        if source.ends_with('\n') {
            lines.pop();
        }
        lines
            .into_iter()
            .map(|line| {
                let mut units: Vec<i32> = line.encode_utf16().map(i32::from).collect();
                units.push(i32::from(b'\r'));
                units
            })
            .collect()
    }

    /// Registers an in-memory TeX source for `\input` lookups.
    ///
    /// Sources are stored as raw UTF-8 bytes so that opening them runs the
    /// same `u_open_in`/`input_line` decode path C applies to real files.
    pub fn register_input_source(&mut self, name: impl Into<String>, source: &str) {
        self.input_byte_sources
            .insert(name.into(), source.as_bytes().to_vec());
    }

    /// Registers an already-encoded TeX source for `\input` lookups.
    pub fn register_input_source_units(
        &mut self,
        name: impl Into<String>,
        lines: VecDeque<Vec<i32>>,
    ) {
        self.input_sources.insert(name.into(), lines);
    }

    /// Pushes a file-reading input frame (`begin_file_reading`).
    ///
    /// The active source context is parked at the current `in_open` level so a
    /// nested input file can replace `buffer`/`source_lines` and restore the
    /// parent source when its own EOF frame is popped.
    pub fn begin_file_reading(&mut self) -> Result<(), TeXMouthError> {
        if self.in_open >= self.max_in_open {
            return Err(TeXMouthError::InputStackOverflow);
        }
        if self.input_ptr >= self.input_stack_capacity {
            return Err(TeXMouthError::InputStackOverflow);
        }

        if self.file_sources.len() <= self.in_open {
            self.file_sources
                .resize_with(self.in_open + 1, FileInputSource::default);
        }
        self.file_sources[self.in_open].buffer = std::mem::take(&mut self.buffer);
        self.file_sources[self.in_open].lines = std::mem::take(&mut self.source_lines);

        if self.input_stack.len() <= self.input_ptr {
            self.input_stack.push(self.cur_input);
        } else {
            self.input_stack[self.input_ptr] = self.cur_input;
        }
        self.input_ptr += 1;
        self.max_in_stack = self.max_in_stack.max(self.input_ptr);

        self.in_open += 1;
        if self.file_sources.len() <= self.in_open {
            self.file_sources
                .resize_with(self.in_open + 1, FileInputSource::default);
        }
        if self.eof_seen.len() <= self.in_open {
            self.eof_seen.resize(self.in_open + 1, false);
        }
        if self.line_stack.len() <= self.in_open {
            self.line_stack.resize(self.in_open + 1, 0);
        }
        if self.grp_stack.len() <= self.in_open {
            self.grp_stack.resize(self.in_open + 1, TEX_NULL);
        }
        if self.if_stack.len() <= self.in_open {
            self.if_stack.resize(self.in_open + 1, TEX_NULL);
        }
        if self.input_file.len() <= self.in_open {
            self.input_file.resize_with(self.in_open + 1, || None);
        }

        self.eof_seen[self.in_open] = false;
        self.grp_stack[self.in_open] = self.cur_boundary;
        self.if_stack[self.in_open] = self.cond_ptr;
        self.line_stack[self.in_open] = self.line;
        self.cur_input = InputState {
            state: MID_LINE,
            index: self.in_open as u16,
            start: self.first,
            loc: self.first,
            limit: self.first - 1,
            name: 0,
            synctex_tag: 0,
        };
        Ok(())
    }

    /// Attaches a registered source to the current file-reading frame and
    /// preloads its first input line (`start_input` file-open path).
    ///
    /// Byte sources open through `u_open_in` exactly like C's `a_open_in`;
    /// registered unit-line sources remain as a compatibility fallback.
    pub fn open_file_source(&mut self, name: &str) -> Result<(), TeXMouthError> {
        self.open_file_source_fmt(name, crate::input::TTBC_FILE_FORMAT_TEX)
    }

    /// `open_file_source` with an explicit Tectonic file format:
    /// `TTBC_FILE_FORMAT_TECTONIC_PRIMARY` opens route through the
    /// `primary_input_resolver` (`input_open_primary`), matching C's
    /// `start_input(primary_input_name)` path.
    pub fn open_file_source_fmt(&mut self, name: &str, filefmt: i32) -> Result<(), TeXMouthError> {
        let byte_name = if filefmt == crate::input::TTBC_FILE_FORMAT_TECTONIC_PRIMARY {
            match self.primary_input_resolver.clone() {
                Some(primary) => {
                    if self.input_byte_sources.contains_key(name) {
                        Some(name.to_string())
                    } else {
                        primary.resolve(name).map(|bytes| {
                            self.input_byte_sources.insert(name.to_string(), bytes);
                            name.to_string()
                        })
                    }
                }
                None => self.input_byte_source_name(name),
            }
        } else {
            self.input_byte_source_name(name)
        };
        let lines = if byte_name.is_none() {
            self.input_source_lines(name)
        } else {
            None
        };
        if byte_name.is_none() && lines.is_none() {
            return Err(TeXMouthError::MissingInputFile(name.to_string()));
        }

        let name_id = self.next_source_name;
        self.next_source_name += 1;
        self.file_sources[self.in_open] =
            FileInputSource::new(name, name_id, lines.unwrap_or_default());
        self.buffer = std::mem::take(&mut self.file_sources[self.in_open].buffer);
        self.source_lines = std::mem::take(&mut self.file_sources[self.in_open].lines);
        self.cur_input.name = name_id;
        self.cur_input.state = NEW_LINE;
        self.line = 1;
        // The name as opened, consumed by SyncTeX's `get_current_name` (C
        // receives the filesystem path from the I/O provider; under
        // virtualized I/O the resolved byte-source name is the equivalent).
        self.abspath_of_input_file = match &byte_name {
            Some(resolved) => resolved.clone(),
            None => name.to_string(),
        };
        // The name as opened, for the `start_input` transcript echo —
        // `u_open_in` may overwrite `abspath_of_input_file` with provider
        // metadata, so capture it here.
        let opened_name = self.abspath_of_input_file.clone();

        if let Some(resolved) = byte_name {
            self.name_of_file = resolved.into_bytes();
            self.name_length = self.name_of_file.len() as i32;
            let f = self.u_open_in(filefmt, crate::input::INPUT_MODE_AUTO, 0);
            if self.input_file.len() <= self.in_open {
                self.input_file.resize_with(self.in_open + 1, || None);
            }
            self.input_file[self.in_open] = f;
        }

        // C `open_input` tail: assign the frame's sync tag and record the
        // tag↔name mapping.
        self.synctex_start_input();

        // C `start_input` tail: the first opened file names the job and
        // opens the transcript (`job_name = cur_name; open_log_file()`).
        if self.job_name.is_none() {
            let stem: String = self
                .cur_name
                .iter()
                .filter_map(|&u| char::from_u32(u as u32))
                .collect();
            self.job_name = Some(stem);
            self.open_log_file()?;
        }

        // C `start_input` transcript echo: wrap if the name would overflow
        // the line, else separate with a space, then `(name` and bump
        // `open_parens`. C prints `full_source_filename_stack[in_open]` =
        // `maketexstring(name_of_input_file)`, the name as opened.
        let full_name = opened_name;
        let name_len = full_name.chars().count() as i32;
        if self.term_offset + name_len > self.max_print_line - 2 {
            self.print_ln();
        } else if self.term_offset > 0 || self.file_offset > 0 {
            self.print_char(i32::from(b' '))?;
        }
        self.print_char(i32::from(b'('))?;
        self.open_parens += 1;
        self.print_str_text(&full_name)?;

        if !self.load_next_source_line(false)? {
            self.cur_input.limit = self.cur_input.start - 1;
            self.cur_input.loc = self.cur_input.start;
            self.first = self.cur_input.start;
            self.last = self.cur_input.limit;
        }
        Ok(())
    }

    /// Opens a `\scantokens` pseudo-file input frame (`pseudo_start`).
    ///
    /// Pseudo-files use TeX name ids 18 and 19; real in-memory sources begin
    /// at id 20. The first pseudo line is deliberately not preloaded: the
    /// scanner's end-of-line path calls the shared line loader, which also
    /// applies `\endlinechar` exactly as C does after `pseudo_input`.
    pub fn open_pseudo_source(&mut self, lines: VecDeque<Vec<i32>>) -> Result<(), TeXMouthError> {
        self.begin_file_reading()?;
        self.source_lines = lines;
        self.buffer.clear();
        self.line = 0;
        let traced = self
            .int_par(crate::state::INT_PAR_TRACING_SCAN_TOKENS)
            .unwrap_or(0)
            > 0;
        self.cur_input.name = if traced { 19 } else { 18 };
        if traced {
            // C `pseudo_start`: traced pseudo-files echo `( ` and join the
            // `open_parens` accounting like a real file frame.
            if self.term_offset > self.max_print_line - 3 {
                self.print_ln();
            } else if self.term_offset > 0 || self.file_offset > 0 {
                self.print_char(i32::from(b' '))?;
            }
            self.print_cstr("( ")?;
            self.open_parens += 1;
        }
        self.cur_input.state = MID_LINE;
        self.cur_input.limit = self.cur_input.start;
        self.cur_input.loc = self.cur_input.limit + 1;
        self.last = self.cur_input.limit;
        Ok(())
    }

    /// Scans and opens `\input`'s file name (`start_input`, in-memory source
    /// backend).
    pub fn start_input(&mut self) -> Result<(), TeXMouthError> {
        self.scan_file_name()?;
        let name = self.file_name_string();
        self.begin_file_reading()?;
        self.open_file_source(&name)
    }

    /// Reads one line from a numbered TeX read stream into the current input
    /// buffer (`input_line(read_file[m])`).
    fn input_line_from_read_stream(&mut self, m: usize) -> Result<bool, TeXMouthError> {
        if let Some(mut f) = self.read_file.get_mut(m).and_then(Option::take) {
            self.first = 0;
            let got = self.input_line(&mut f);
            self.read_file[m] = Some(f);
            if !got? {
                return Ok(false);
            }
            self.cur_input.start = 0;
            self.cur_input.loc = 0;
            self.cur_input.limit = self.last;
            self.cur_input.state = NEW_LINE;
            self.apply_end_line_char();
            self.first = self.cur_input.limit + 1;
            self.last = self.cur_input.limit;
            return Ok(true);
        }

        let Some(source) = self.read_sources.get_mut(m).and_then(Option::as_mut) else {
            return Ok(false);
        };
        let Some(units) = source.lines.pop_front() else {
            return Ok(false);
        };

        self.buffer = units;
        self.cur_input.start = 0;
        self.cur_input.loc = 0;
        self.cur_input.limit = self.buffer.len() as i32 - 1;
        self.cur_input.state = NEW_LINE;
        self.apply_end_line_char();
        self.first = self.cur_input.limit + 1;
        self.last = self.cur_input.limit;
        Ok(true)
    }

    fn close_read_stream(&mut self, m: usize) {
        if let Some(state) = self.read_open.get_mut(m) {
            *state = ReadOpen::Closed;
        }
        if let Some(source) = self.read_sources.get_mut(m) {
            *source = None;
        }
        if let Some(f) = self.read_file.get_mut(m) {
            *f = None;
        }
    }

    pub(crate) fn input_source_lines(&self, name: &str) -> Option<VecDeque<Vec<i32>>> {
        self.input_sources.get(name).cloned().or_else(|| {
            if name
                .rsplit('/')
                .next()
                .is_some_and(|stem| !stem.contains('.'))
            {
                self.input_sources.get(&format!("{name}.tex")).cloned()
            } else {
                None
            }
        })
    }

    /// Scans a token list from a TeX read stream (`read_toks`).
    ///
    /// `j == 1` implements `\readline`'s raw other-character conversion; the
    /// normal path tokenizes with the current category-code table.
    pub fn read_toks(&mut self, n: i32, r: i32, j: i32) -> Result<(), TeXMouthError> {
        self.scanner_status = DEFINING;
        self.warning_index = r;
        self.def_ref = self.get_avail()?;
        let def_idx = self.checked_mem_index(self.def_ref)?;
        self.mem[def_idx].s0 = TEX_NULL;
        let mut p = self.def_ref;
        self.append_scanned_token(&mut p, END_MATCH_TOKEN)?;

        let m = if !(0..=15).contains(&n) {
            16
        } else {
            usize::try_from(n).map_err(|_| crate::scanner::oob(n))?
        };
        let saved_align_state = self.align_state;
        self.align_state = 1_000_000;

        loop {
            self.begin_file_reading()?;
            self.cur_input.name = (m + 1) as i32;

            match self.read_open.get(m).copied().unwrap_or(ReadOpen::Closed) {
                ReadOpen::Closed => {
                    self.end_file_reading()?;
                    return Err(TeXMouthError::UnsupportedPrimitive("terminal \\read"));
                }
                ReadOpen::JustOpen => {
                    if self.input_line_from_read_stream(m)? {
                        self.read_open[m] = ReadOpen::Normal;
                    } else {
                        self.close_read_stream(m);
                    }
                }
                ReadOpen::Normal => {
                    if !self.input_line_from_read_stream(m)? {
                        self.close_read_stream(m);
                        if self.align_state != 1_000_000 {
                            self.runaway()?;
                            self.report_error(
                                "File ended within \\read",
                                &["This \\read has unbalanced braces."],
                            );
                            self.align_state = 1_000_000;
                            self.cur_input.limit = 0;
                        }
                    }
                }
            }

            if self.read_open.get(m).copied() == Some(ReadOpen::Closed) {
                self.buffer.clear();
                self.cur_input.start = 0;
                self.cur_input.loc = 0;
                self.cur_input.limit = -1;
                self.cur_input.state = NEW_LINE;
                self.first = 0;
                self.last = -1;
            }

            if j == 1 {
                while self.cur_input.loc <= self.cur_input.limit {
                    self.cur_chr = self.buffer_at(self.cur_input.loc)?;
                    self.cur_input.loc += 1;
                    self.cur_tok = if self.cur_chr == i32::from(b' ') {
                        SPACE_TOKEN
                    } else {
                        OTHER_TOKEN + self.cur_chr
                    };
                    self.append_scanned_token(&mut p, self.cur_tok)?;
                }
            } else {
                loop {
                    self.get_token()?;
                    if self.cur_tok == 0 {
                        break;
                    }
                    if self.align_state < 1_000_000 {
                        while self.cur_tok != 0 {
                            self.get_token()?;
                        }
                        self.align_state = 1_000_000;
                        break;
                    }
                    self.append_scanned_token(&mut p, self.cur_tok)?;
                }
            }

            self.end_file_reading()?;
            if self.align_state == 1_000_000 {
                break;
            }
        }

        self.cur_val = self.def_ref;
        self.scanner_status = NORMAL;
        self.align_state = saved_align_state;
        Ok(())
    }

    /// Opens a named source directly, bypassing file-name token scanning.
    pub fn start_input_source(&mut self, name: &str) -> Result<(), TeXMouthError> {
        // C `start_input(primary_input_name)`: the primary name runs
        // through the same begin/more/end_name trifecta so `cur_name`
        // holds the stem — `job_name` derives from it exactly like a
        // scanned `\input` name.
        self.begin_name();
        let saved_stop_at_space = self.stop_at_space;
        self.stop_at_space = false;
        for c in name.chars() {
            self.more_name(c as i32);
        }
        self.stop_at_space = saved_stop_at_space;
        self.end_name();
        self.begin_file_reading()?;
        self.open_file_source_fmt(name, crate::input::TTBC_FILE_FORMAT_TECTONIC_PRIMARY)
    }

    /// Scans a TeX file name (`scan_file_name`), supporting unbraced names,
    /// quotes, and a bounded braced-name subset pending full `scan_toks`.
    pub fn scan_file_name(&mut self) -> Result<(), TeXMouthError> {
        let save_warning_index = self.warning_index;
        self.warning_index = self.cur_cs;

        loop {
            self.get_x_token()?;
            if self.cur_cmd != CommandCode::Spacer as u8 && self.cur_cmd != CommandCode::Relax as u8
            {
                break;
            }
        }
        self.back_input()?;

        self.begin_name();
        if self.cur_cmd == CommandCode::LeftBrace as u8 {
            self.scan_file_name_braced()?;
        } else {
            self.name_in_progress = true;
            loop {
                self.get_x_token()?;
                if self.cur_cmd != CommandCode::Spacer as u8 {
                    break;
                }
            }

            loop {
                if self.cur_cmd > CommandCode::OtherChar as u8 || self.cur_chr > BIGGEST_CHAR {
                    self.back_input()?;
                    break;
                }
                if !self.more_name(self.cur_chr) {
                    break;
                }
                self.get_x_token()?;
            }
        }

        self.end_name();
        self.name_in_progress = false;
        self.warning_index = save_warning_index;
        Ok(())
    }

    fn scan_file_name_braced(&mut self) -> Result<(), TeXMouthError> {
        let save_stop_at_space = self.stop_at_space;
        self.get_x_token()?;
        if self.cur_cmd != CommandCode::LeftBrace as u8 {
            return Err(TeXMouthError::MacroArgumentMismatch {
                expected: LEFT_BRACE_TOKEN + i32::from(b'{'),
                found: self.cur_tok,
            });
        }

        let mut depth = 1_i32;
        while depth > 0 {
            self.get_x_token()?;
            match self.cur_cmd {
                c if c == CommandCode::LeftBrace as u8 => {
                    depth += 1;
                    self.more_name(self.cur_chr);
                }
                c if c == CommandCode::RightBrace as u8 => {
                    depth -= 1;
                    if depth > 0 {
                        self.more_name(self.cur_chr);
                    }
                }
                c if c <= CommandCode::OtherChar as u8 && self.cur_chr <= BIGGEST_CHAR => {
                    self.more_name(self.cur_chr);
                }
                _ => {
                    return Err(TeXMouthError::UnsupportedPrimitive(
                        "braced file name token",
                    ))
                }
            }
        }
        self.stop_at_space = save_stop_at_space;
        Ok(())
    }

    pub(crate) fn begin_name(&mut self) {
        self.area_delimiter = 0;
        self.ext_delimiter = 0;
        self.quoted_filename = false;
        self.file_name_quote_char = 0;
        self.cur_name.clear();
        self.cur_area.clear();
        self.cur_ext.clear();
    }

    pub(crate) fn more_name(&mut self, c: i32) -> bool {
        if self.stop_at_space && self.file_name_quote_char == 0 && c == i32::from(b' ') {
            return false;
        }
        if self.stop_at_space && self.file_name_quote_char != 0 && c == self.file_name_quote_char {
            self.file_name_quote_char = 0;
            return true;
        }
        if self.stop_at_space
            && self.file_name_quote_char == 0
            && (c == i32::from(b'"') || c == i32::from(b'\''))
        {
            self.file_name_quote_char = c;
            self.quoted_filename = true;
            return true;
        }

        self.cur_name.push(c);
        if c == i32::from(b'/') || c == i32::from(b'\\') {
            self.area_delimiter = self.cur_name.len();
            self.ext_delimiter = 0;
        } else if c == i32::from(b'.') {
            self.ext_delimiter = self.cur_name.len();
        }
        true
    }

    pub(crate) fn end_name(&mut self) {
        let name = std::mem::take(&mut self.cur_name);
        if self.area_delimiter == 0 {
            self.cur_area.clear();
        } else {
            self.cur_area = name[..self.area_delimiter].to_vec();
        }

        let stem_start = self.area_delimiter;
        let stem_end = if self.ext_delimiter == 0 {
            name.len()
        } else {
            self.ext_delimiter - 1
        };
        self.cur_name = name[stem_start..stem_end].to_vec();
        if self.ext_delimiter == 0 {
            self.cur_ext.clear();
        } else {
            self.cur_ext = name[self.ext_delimiter - 1..].to_vec();
        }
    }

    /// Packs the scanned file-name parts into text (`pack_file_name`).
    pub fn file_name_string(&self) -> String {
        self.cur_area
            .iter()
            .chain(&self.cur_name)
            .chain(&self.cur_ext)
            .filter_map(|&unit| char::from_u32(unit as u32))
            .collect()
    }

    /// Allocates a token-list reference cell suitable for token variables such
    /// as `\everyeof`.
    pub fn store_token_ref(&mut self, tokens: &[i32]) -> Result<i32, TeXMouthError> {
        let list = self.store_token_list(tokens)?;
        let reference = match self.get_avail() {
            Ok(reference) => reference,
            Err(err) => {
                self.flush_list(list)?;
                return Err(err.into());
            }
        };
        self.mem[reference as usize].s0 = TEX_NULL;
        self.mem[reference as usize].s1 = list;
        Ok(reference)
    }

    /// Looks up a control-sequence name in `buffer[j..j+l]` (`id_lookup`).
    pub fn id_lookup(&mut self, j: i32, l: i32) -> Result<i32, TeXMouthError> {
        if l < 0
            || j < 0
            || j.checked_add(l)
                .is_none_or(|end| end as usize > self.buffer.len())
        {
            return Err(crate::scanner::oob(j));
        }

        let name = self.buffer[j as usize..(j + l) as usize].to_vec();
        Ok(self
            .cs_table
            .lookup_or_undefined(&name, !self.no_new_control_sequence))
    }

    /// Looks up a control-sequence name supplied as Unicode scalar values.
    pub fn lookup_cs_units(&mut self, name: &[i32], create: bool) -> i32 {
        self.cs_table.lookup_or_undefined(name, create)
    }

    /// Returns the command code assigned to `cs`.
    pub fn cs_command(&self, cs: i32) -> u8 {
        self.cs_table.command(cs)
    }

    /// Returns the `cur_chr` payload assigned to `cs`.
    pub fn cs_character(&self, cs: i32) -> i32 {
        self.cs_table.character(cs)
    }

    /// Returns the original primitive meaning of `name` (`prim_lookup`).
    pub fn prim_lookup(&self, name: &str) -> Option<(u8, i32)> {
        self.cs_table.prim_lookup_str(name)
    }

    /// Returns the original primitive meaning addressed by a control-sequence
    /// id (`prim_lookup` after `get_token`/`get_next`).
    pub fn prim_lookup_cs(&self, cs: i32) -> Option<(u8, i32)> {
        self.cs_table.prim_lookup_units(self.cs_table.name(cs))
    }

    /// Assigns `cur_cmd`/`cur_chr` from a control-sequence id.
    pub fn set_current_cs(&mut self, cs: i32) {
        self.cur_cs = cs;
        self.cur_cmd = self.cs_command(cs);
        self.cur_chr = self.cs_character(cs);
    }

    /// Defines or updates a named control sequence (`eq_define`-style helper).
    pub fn define_cs(&mut self, name: &str, command: CommandCode, character: i32) -> i32 {
        self.cs_table.define_str(name, command, character)
    }

    fn cs_save_word(&self, cs: i32) -> Result<MemoryWord, TeXMouthError> {
        let mut word = MemoryWord::default();
        word.set_b16_s0(self.cs_table.level(cs)?);
        word.set_b16_s1(u16::from(self.cs_table.command(cs)));
        word.s1 = self.cs_table.character(cs);
        Ok(word)
    }

    fn destroy_cs_word(&mut self, word: MemoryWord) -> Result<(), TeXMouthError> {
        self.eq_destroy(word)?;
        Ok(())
    }

    /// Defines a control sequence at the current group level (`eq_define` for
    /// the compact control-sequence table).
    pub fn eq_define_cs(
        &mut self,
        cs: i32,
        command: CommandCode,
        character: i32,
    ) -> Result<(), TeXMouthError> {
        let old = self.cs_save_word(cs)?;
        if old.b16_s1() == command as u16 && old.s1 == character {
            self.destroy_cs_word(old)?;
            return Ok(());
        }
        if old.b16_s0() == self.cur_level {
            self.destroy_cs_word(old)?;
        } else if self.cur_level > LEVEL_ONE {
            self.cs_eq_save(cs, old.b16_s0(), old)?;
        }
        self.cs_table
            .restore(cs, command as u8, character, self.cur_level)?;
        Ok(())
    }

    /// Defines a control sequence globally (`geq_define` for the compact
    /// control-sequence table).
    pub fn geq_define_cs(
        &mut self,
        cs: i32,
        command: CommandCode,
        character: i32,
    ) -> Result<(), TeXMouthError> {
        let old = self.cs_save_word(cs)?;
        self.destroy_cs_word(old)?;
        Ok(self
            .cs_table
            .restore(cs, command as u8, character, LEVEL_ONE)?)
    }

    fn restore_eqtb_word(&mut self, p: usize, saved: MemoryWord) -> Result<(), TeXMouthError> {
        if p >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(p as i32, self.eqtb.len()).into());
        }
        if self.eqtb[p].b16_s0() == LEVEL_ONE {
            self.eq_destroy(saved)?;
        } else {
            self.eq_destroy(self.eqtb[p])?;
            self.eqtb[p] = saved;
        }
        Ok(())
    }

    fn restore_cs_word(&mut self, cs: i32, saved: MemoryWord) -> Result<(), TeXMouthError> {
        let current = self.cs_save_word(cs)?;
        if current.b16_s0() == LEVEL_ONE {
            self.destroy_cs_word(saved)?;
        } else {
            self.destroy_cs_word(current)?;
            self.cs_table
                .restore(cs, saved.b16_s1() as u8, saved.s1, saved.b16_s0())?;
        }
        Ok(())
    }

    fn zero_restore_word(&self) -> MemoryWord {
        let mut word = MemoryWord::default();
        word.set_b16_s1(CommandCode::UndefinedCs as u16);
        word
    }

    /// Closes the innermost save level (`unsave`).
    pub fn unsave(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_level <= LEVEL_ONE {
            return Err(TeXMemoryError::SaveStackUnderflow.into());
        }
        self.cur_level -= 1;
        let mut inserted = false;

        loop {
            if self.save_ptr == 0 || self.save_ptr > self.save_stack.len() {
                return Err(TeXMemoryError::CorruptedNode(self.save_ptr as i32).into());
            }
            self.save_ptr -= 1;
            match self.save_stack[self.save_ptr] {
                SaveStackEntry::Boundary { group, boundary } => {
                    // C `unsave`'s `done:` warns when the group being closed
                    // was opened in a different file than the one active now.
                    if self.grp_stack.get(self.in_open).copied() == Some(self.cur_boundary) {
                        self.group_warning(boundary)?;
                    }
                    self.cur_group = group;
                    self.cur_boundary = boundary;
                    self.save_ptr -= 1;
                    return Ok(());
                }
                SaveStackEntry::Line(_)
                | SaveStackEntry::SavedWord(_)
                | SaveStackEntry::RawValue(_) => {
                    return Err(TeXMemoryError::CorruptedNode(self.save_ptr as i32).into());
                }
                SaveStackEntry::Restore {
                    kind,
                    level: _,
                    target,
                } => {
                    let saved = if kind == RESTORE_OLD_VALUE {
                        if self.save_ptr == 0 {
                            return Err(TeXMemoryError::CorruptedNode(self.save_ptr as i32).into());
                        }
                        self.save_ptr -= 1;
                        match self.save_stack[self.save_ptr] {
                            SaveStackEntry::SavedWord(word) => word,
                            _ => {
                                return Err(
                                    TeXMemoryError::CorruptedNode(self.save_ptr as i32).into()
                                );
                            }
                        }
                    } else if kind == RESTORE_ZERO {
                        self.zero_restore_word()
                    } else {
                        return Err(TeXMemoryError::CorruptedNode(self.save_ptr as i32).into());
                    };

                    match target {
                        SaveTarget::Eqtb(p) => self.restore_eqtb_word(p, saved)?,
                        SaveTarget::ControlSequence(cs) => self.restore_cs_word(cs, saved)?,
                        SaveTarget::CharacterCode { table, character } => {
                            self.restore_character_code(table, character, saved)?
                        }
                    }
                }
                SaveStackEntry::InsertToken(token) => {
                    let saved_token = self.cur_tok;
                    self.cur_tok = token;
                    if inserted {
                        let p = self.get_avail()?;
                        self.mem[p as usize].s0 = self.cur_tok;
                        self.mem[p as usize].s1 = self.cur_input.loc;
                        self.cur_input.loc = p;
                        self.cur_input.start = p;
                        if self.cur_tok < RIGHT_BRACE_LIMIT {
                            if self.cur_tok < LEFT_BRACE_LIMIT {
                                self.align_state -= 1;
                            } else {
                                self.align_state += 1;
                            }
                        }
                    } else {
                        self.back_input()?;
                        inserted = true;
                    }
                    self.cur_tok = saved_token;
                }
                SaveStackEntry::RestoreSa { level, chain } => {
                    self.sa_restore().map_err(TeXMouthError::Memory)?;
                    self.sa_chain = chain;
                    self.sa_level = level;
                }
            }
        }
    }

    /// Defines or updates an active-character control sequence.
    pub fn define_active_cs(&mut self, character: char, command: CommandCode, payload: i32) -> i32 {
        self.cs_table
            .define_active(character as i32, command, payload)
    }

    /// Installs every primitive from the vendored XeTeX format table.
    pub fn install_core_primitives(&mut self) {
        self.no_new_control_sequence = false;
        for def in primitive_defs().expect("vendored XeTeX primitive table is valid") {
            let units: Vec<i32> = def.name.chars().map(|c| c as i32).collect();
            let cs = self
                .cs_table
                .define_units(&units, def.command as u8, def.chr);
            self.cs_table
                .register_primitive_units(&units, def.command as u8, def.chr);
            match def.extra_init {
                PrimitiveExtraInit::None => {}
                PrimitiveExtraInit::Par => {
                    self.par_loc = cs;
                    self.par_token = CS_TOKEN_FLAG + cs;
                }
                PrimitiveExtraInit::Write => self.write_loc = cs,
                PrimitiveExtraInit::Frozen(symbol) => {
                    self.install_frozen_primitive(symbol, def.command as u8, def.chr);
                }
            }
        }
        self.no_new_control_sequence = true;
    }

    fn install_frozen_primitive(&mut self, symbol: &str, command: u8, character: i32) {
        let target = match symbol {
            "FROZEN_RELAX" => FROZEN_RELAX_ID,
            "FROZEN_CR" => FROZEN_CR_ID,
            "FROZEN_END_GROUP" => FROZEN_END_GROUP_ID,
            "FROZEN_RIGHT" => FROZEN_RIGHT_ID,
            "FROZEN_FI" => FROZEN_FI_ID,
            "FROZEN_SPECIAL" => FROZEN_SPECIAL_ID,
            "FROZEN_NULL_FONT" => FROZEN_NULL_FONT_ID,
            _ => return,
        };
        self.cs_table
            .restore(target, command, character, LEVEL_ONE)
            .expect("reserved frozen control-sequence id exists");
    }

    /// Installs the original bounded primitive subset.
    #[allow(dead_code)]
    fn install_core_primitives_legacy(&mut self) {
        self.define_cs("par", CommandCode::ParEnd, 0);
        self.define_cs("relax", CommandCode::Relax, 0);
        self.define_cs("long", CommandCode::Prefix, 1);
        self.define_cs("outer", CommandCode::Prefix, 2);
        self.define_cs("global", CommandCode::Prefix, 4);
        self.define_cs("protected", CommandCode::Prefix, 8);
        self.define_cs("def", CommandCode::Def, 0);
        self.define_cs("gdef", CommandCode::Def, 1);
        self.define_cs("edef", CommandCode::Def, 2);
        self.define_cs("xdef", CommandCode::Def, 3);
        self.define_cs("let", CommandCode::Let, 0);
        self.define_cs("futurelet", CommandCode::Let, 1);
        self.define_cs("read", CommandCode::ReadToCs, 0);
        self.define_cs("readline", CommandCode::ReadToCs, 1);
        self.define_cs("openin", CommandCode::InStream, 1);
        self.define_cs("closein", CommandCode::InStream, 0);
        self.define_cs("chardef", CommandCode::ShorthandDef, 0);
        self.define_cs("mathchardef", CommandCode::ShorthandDef, 1);
        self.define_cs("countdef", CommandCode::ShorthandDef, 2);
        self.define_cs("dimendef", CommandCode::ShorthandDef, 3);
        self.define_cs("skipdef", CommandCode::ShorthandDef, 4);
        self.define_cs("muskipdef", CommandCode::ShorthandDef, 5);
        self.define_cs("toksdef", CommandCode::ShorthandDef, 6);
        self.define_cs("Umathcharnumdef", CommandCode::ShorthandDef, 8);
        self.define_cs("XeTeXmathcharnumdef", CommandCode::ShorthandDef, 8);
        self.define_cs("Umathchardef", CommandCode::ShorthandDef, 9);
        self.define_cs("XeTeXmathchardef", CommandCode::ShorthandDef, 9);
        self.define_cs("catcode", CommandCode::DefCode, CAT_CODE_BASE);
        self.define_cs("lccode", CommandCode::DefCode, LC_CODE_BASE);
        self.define_cs("uccode", CommandCode::DefCode, UC_CODE_BASE);
        self.define_cs("sfcode", CommandCode::DefCode, SF_CODE_BASE);
        self.define_cs("mathcode", CommandCode::DefCode, MATH_CODE_BASE);
        self.define_cs("delcode", CommandCode::DefCode, DEL_CODE_BASE);
        self.define_cs("XeTeXcharclass", CommandCode::XetexDefCode, SF_CODE_BASE);
        self.define_cs("Umathcodenum", CommandCode::XetexDefCode, MATH_CODE_BASE);
        self.define_cs("Umathcode", CommandCode::XetexDefCode, MATH_CODE_BASE + 1);
        self.define_cs("Udelcodenum", CommandCode::XetexDefCode, DEL_CODE_BASE);
        self.define_cs("Udelcode", CommandCode::XetexDefCode, DEL_CODE_BASE + 1);
        self.define_cs("char", CommandCode::CharNum, 0);
        self.define_cs("noboundary", CommandCode::NoBoundary, 0);
        self.define_cs("/", CommandCode::ItalCorr, 0);
        self.define_cs("accent", CommandCode::Accent, 0);
        self.define_cs("discretionary", CommandCode::Discretionary, 0);
        self.define_cs("-", CommandCode::Discretionary, 1);
        self.define_cs("unskip", CommandCode::RemoveItem, i32::from(GLUE_NODE));
        self.define_cs("kern", CommandCode::Kern, i32::from(EXPLICIT));
        self.define_cs("hskip", CommandCode::Hskip, SKIP_CODE);
        self.define_cs("vskip", CommandCode::Vskip, SKIP_CODE);
        self.define_cs("indent", CommandCode::StartPar, 1);
        self.define_cs("noindent", CommandCode::StartPar, 0);
        self.define_cs("vrule", CommandCode::Vrule, 0);
        self.define_cs("hrule", CommandCode::Hrule, 0);
        self.define_cs("prevdepth", CommandCode::SetAux, TeXMode::Vmode as i32);
        self.define_cs("spacefactor", CommandCode::SetAux, TeXMode::Hmode as i32);
        self.define_cs("prevgraf", CommandCode::SetPrevGraf, 0);
        self.define_cs("pagegoal", CommandCode::SetPageDimen, 0);
        self.define_cs("pagetotal", CommandCode::SetPageDimen, 1);
        self.define_cs("pagestretch", CommandCode::SetPageDimen, 2);
        self.define_cs("pagefilstretch", CommandCode::SetPageDimen, 3);
        self.define_cs("pagefillstretch", CommandCode::SetPageDimen, 4);
        self.define_cs("pagefilllstretch", CommandCode::SetPageDimen, 5);
        self.define_cs("pageshrink", CommandCode::SetPageDimen, 6);
        self.define_cs("pagedepth", CommandCode::SetPageDimen, 7);
        self.define_cs("deadcycles", CommandCode::SetPageInt, 0);
        self.define_cs("insertpenalties", CommandCode::SetPageInt, 1);
        self.define_cs("interactionmode", CommandCode::SetPageInt, 2);
        self.define_cs("wd", CommandCode::SetBoxDimen, WIDTH_OFFSET);
        self.define_cs("dp", CommandCode::SetBoxDimen, DEPTH_OFFSET);
        self.define_cs("ht", CommandCode::SetBoxDimen, HEIGHT_OFFSET);
        self.define_cs(
            "parshape",
            CommandCode::SetShape,
            (LOCAL_BASE + LOCAL_PAR_SHAPE) as i32,
        );
        self.define_cs(
            "interlinepenalties",
            CommandCode::SetShape,
            (ETEX_PEN_BASE + ETEX_PEN_INTER_LINE) as i32,
        );
        self.define_cs(
            "clubpenalties",
            CommandCode::SetShape,
            (ETEX_PEN_BASE + ETEX_PEN_CLUB) as i32,
        );
        self.define_cs(
            "widowpenalties",
            CommandCode::SetShape,
            (ETEX_PEN_BASE + ETEX_PEN_WIDOW) as i32,
        );
        self.define_cs(
            "displaywidowpenalties",
            CommandCode::SetShape,
            (ETEX_PEN_BASE + ETEX_PEN_DISPLAY_WIDOW) as i32,
        );
        self.define_cs(
            "batchmode",
            CommandCode::SetInteraction,
            i32::from(BATCH_MODE),
        );
        self.define_cs(
            "nonstopmode",
            CommandCode::SetInteraction,
            i32::from(NONSTOP_MODE),
        );
        self.define_cs(
            "scrollmode",
            CommandCode::SetInteraction,
            i32::from(SCROLL_MODE),
        );
        self.define_cs(
            "errorstopmode",
            CommandCode::SetInteraction,
            i32::from(ERROR_STOP_MODE),
        );
        self.define_cs("begingroup", CommandCode::BeginGroup, 0);
        self.define_cs("endgroup", CommandCode::EndGroup, 0);
        self.define_cs("afterassignment", CommandCode::AfterAssignment, 0);
        self.define_cs("aftergroup", CommandCode::AfterGroup, 0);
        self.define_cs("expandafter", CommandCode::ExpandAfter, 0);
        self.define_cs("unless", CommandCode::ExpandAfter, 1);
        self.define_cs("noexpand", CommandCode::NoExpand, 0);
        self.define_cs("primitive", CommandCode::NoExpand, 1);
        self.define_cs("csname", CommandCode::CsName, 0);
        self.define_cs("endcsname", CommandCode::EndCsName, 0);
        self.define_cs("the", CommandCode::The, 0);
        self.define_cs("unexpanded", CommandCode::The, 1);
        self.define_cs("detokenize", CommandCode::The, 5);
        self.define_cs("input", CommandCode::Input, 0);
        self.define_cs("endinput", CommandCode::Input, 1);
        self.define_cs("scantokens", CommandCode::Input, 2);
        self.define_cs("number", CommandCode::Convert, 0);
        self.define_cs("romannumeral", CommandCode::Convert, 1);
        self.define_cs("string", CommandCode::Convert, 2);
        self.define_cs("meaning", CommandCode::Convert, 3);
        self.define_cs("count", CommandCode::Register, 0);
        self.define_cs("dimen", CommandCode::Register, 1);
        self.define_cs("skip", CommandCode::Register, 2);
        self.define_cs("muskip", CommandCode::Register, 3);
        self.define_cs("toks", CommandCode::ToksRegister, 0);
        self.define_cs("advance", CommandCode::Advance, 0);
        self.define_cs("multiply", CommandCode::Multiply, 0);
        self.define_cs("divide", CommandCode::Divide, 0);
        self.define_cs(
            "endlinechar",
            CommandCode::AssignInt,
            (crate::state::INT_PAR_BASE + crate::state::INT_PAR_END_LINE_CHAR) as i32,
        );
        self.define_cs(
            "newlinechar",
            CommandCode::AssignInt,
            (crate::state::INT_PAR_BASE + crate::state::INT_PAR_NEW_LINE_CHAR) as i32,
        );
        self.define_cs(
            "globaldefs",
            CommandCode::AssignInt,
            (crate::state::INT_PAR_BASE + crate::state::INT_PAR_GLOBAL_DEFS) as i32,
        );
        self.define_cs(
            "everyeof",
            CommandCode::AssignToks,
            (crate::state::LOCAL_BASE + crate::state::LOCAL_EVERY_EOF) as i32,
        );
        self.define_cs("if", CommandCode::IfTest, 0);
        self.define_cs("ifcat", CommandCode::IfTest, 1);
        self.define_cs("ifnum", CommandCode::IfTest, 2);
        self.define_cs("ifdim", CommandCode::IfTest, 3);
        self.define_cs("ifodd", CommandCode::IfTest, 4);
        self.define_cs("ifvmode", CommandCode::IfTest, 5);
        self.define_cs("ifhmode", CommandCode::IfTest, 6);
        self.define_cs("ifmmode", CommandCode::IfTest, 7);
        self.define_cs("ifinner", CommandCode::IfTest, 8);
        self.define_cs("ifvoid", CommandCode::IfTest, 9);
        self.define_cs("ifhbox", CommandCode::IfTest, 10);
        self.define_cs("ifvbox", CommandCode::IfTest, 11);
        self.define_cs("ifx", CommandCode::IfTest, 12);
        self.define_cs("ifeof", CommandCode::IfTest, 13);
        self.define_cs("iftrue", CommandCode::IfTest, 14);
        self.define_cs("iffalse", CommandCode::IfTest, 15);
        self.define_cs("ifcase", CommandCode::IfTest, 16);
        self.define_cs("ifdefined", CommandCode::IfTest, 17);
        self.define_cs("ifcsname", CommandCode::IfTest, 18);
        self.define_cs("iffontchar", CommandCode::IfTest, 19);
        self.define_cs("ifincsname", CommandCode::IfTest, 20);
        self.define_cs("ifprimitive", CommandCode::IfTest, 21);
        self.define_cs("fi", CommandCode::FiOrElse, 2);
        self.define_cs("else", CommandCode::FiOrElse, 3);
        self.define_cs("or", CommandCode::FiOrElse, 4);
        self.define_cs("topmark", CommandCode::TopBotMark, 0);
        self.define_cs("firstmark", CommandCode::TopBotMark, 1);
        self.define_cs("botmark", CommandCode::TopBotMark, 2);
        self.define_cs("splitfirstmark", CommandCode::TopBotMark, 3);
        self.define_cs("splitbotmark", CommandCode::TopBotMark, 4);
        // e-TeX glue conversions and expression primitives (LastItem).
        self.define_cs("mutoglue", CommandCode::LastItem, MU_TO_GLUE_CODE);
        self.define_cs("gluetomu", CommandCode::LastItem, GLUE_TO_MU_CODE);
        // e-TeX expression primitives (LastItem, ETEX_EXPR + level).
        self.define_cs(
            "numexpr",
            CommandCode::LastItem,
            ETEX_EXPR + i32::from(INT_VAL),
        );
        self.define_cs(
            "dimexpr",
            CommandCode::LastItem,
            ETEX_EXPR + i32::from(DIMEN_VAL),
        );
        self.define_cs(
            "glueexpr",
            CommandCode::LastItem,
            ETEX_EXPR + i32::from(GLUE_VAL),
        );
        self.define_cs(
            "muexpr",
            CommandCode::LastItem,
            ETEX_EXPR + i32::from(MU_VAL),
        );
    }

    /// Allocates a linked token list in high memory and returns its head node.
    pub fn store_token_list(&mut self, tokens: &[i32]) -> Result<i32, TeXMouthError> {
        let mut head = TEX_NULL;
        let mut tail = TEX_NULL;

        for &token in tokens {
            let node = self.get_avail()?;
            self.mem[node as usize].s0 = token;
            self.mem[node as usize].s1 = TEX_NULL;
            if head == TEX_NULL {
                head = node;
            } else {
                self.mem[tail as usize].s1 = node;
            }
            tail = node;
        }

        Ok(head)
    }

    /// Copies a linked token list into a Rust vector.
    pub fn token_list_to_vec(&self, mut p: i32) -> Result<Vec<i32>, TeXMouthError> {
        let mut tokens = Vec::new();
        while p != TEX_NULL {
            let idx = self.checked_mem_index(p)?;
            tokens.push(self.mem[idx].s0);
            p = self.mem[idx].s1;
        }
        Ok(tokens)
    }

    /// Converts a linked token list to TeX's `show_token_list` text form.
    pub fn token_list_to_string(&self, mut p: i32) -> Result<String, TeXMouthError> {
        let mut output = String::new();
        let mut match_chr = i32::from(b'#');
        let mut parameter_digit = i32::from(b'0');

        while p != TEX_NULL {
            let idx = self.checked_mem_index(p)?;
            let token = self.mem[idx].s0;
            if token >= CS_TOKEN_FLAG {
                self.push_cs_text(token - CS_TOKEN_FLAG, &mut output)?;
            } else if token < 0 {
                self.push_escape_text("BAD.", &mut output)?;
            } else {
                let command = token / MAX_CHAR_VAL;
                let character = token % MAX_CHAR_VAL;
                match command {
                    1 | 2 | 3 | 4 | 7 | 8 | 10 | 11 | 12 => {
                        push_character(character, &mut output);
                    }
                    6 => {
                        push_character(character, &mut output);
                        push_character(character, &mut output);
                    }
                    5 => {
                        push_character(match_chr, &mut output);
                        if character <= 9 {
                            push_character(character + i32::from(b'0'), &mut output);
                        } else {
                            output.push('!');
                            break;
                        }
                    }
                    13 => {
                        match_chr = character;
                        push_character(character, &mut output);
                        parameter_digit += 1;
                        push_character(parameter_digit, &mut output);
                        if parameter_digit > i32::from(b'9') {
                            break;
                        }
                    }
                    14 if character == 0 => output.push_str("->"),
                    _ => self.push_escape_text("BAD.", &mut output)?,
                }
            }
            p = self.mem[idx].s1;
        }
        Ok(output)
    }

    /// Appends TeX's printable form of a control sequence (`print_cs`).
    fn push_cs_text(&self, cs: i32, output: &mut String) -> Result<(), TeXMouthError> {
        if self.cs_table.is_active(cs) {
            if let Some(&character) = self.cs_table.name(cs).first() {
                push_character(character, output);
            }
            return Ok(());
        }
        if cs == NULL_CS_ID {
            self.push_escape_text("csname", output)?;
            self.push_escape_text("endcsname", output)?;
            output.push(' ');
            return Ok(());
        }

        let name = self.cs_table.name(cs);
        if name.is_empty() || name.iter().any(|&c| c < 0) {
            self.push_escape_text("NONEXISTENT.", output)?;
            output.push(' ');
            return Ok(());
        }

        self.push_escape_units(name, output)?;
        if name.len() > 1 {
            output.push(' ');
        } else if name.len() == 1 {
            let character =
                u32::try_from(name[0]).map_err(|_| TeXMouthError::InvalidCharacter(name[0]))?;
            if self.cat_code(character) == CommandCode::Letter as u8 {
                output.push(' ');
            }
        }
        Ok(())
    }

    fn push_escape_text(&self, text: &str, output: &mut String) -> Result<(), TeXMouthError> {
        self.push_escape_char(output)?;
        output.push_str(text);
        Ok(())
    }

    fn push_escape_units(&self, name: &[i32], output: &mut String) -> Result<(), TeXMouthError> {
        self.push_escape_char(output)?;
        push_units(name, output);
        Ok(())
    }

    fn push_escape_char(&self, output: &mut String) -> Result<(), TeXMouthError> {
        let escape = self.int_par(crate::state::INT_PAR_ESCAPE_CHAR)?;
        if (0..=BIGGEST_USV).contains(&escape) {
            push_character(escape, output);
        }
        Ok(())
    }

    /// Pushes the current input and starts a token-list input frame (`begin_token_list`).
    pub fn begin_token_list(&mut self, p: i32, kind: u16) -> Result<(), TeXMouthError> {
        if self.input_ptr >= self.input_stack_capacity {
            return Err(TeXMouthError::InputStackOverflow);
        }

        if self.input_stack.len() <= self.input_ptr {
            self.input_stack.push(self.cur_input);
        } else {
            self.input_stack[self.input_ptr] = self.cur_input;
        }
        self.input_ptr += 1;
        self.max_in_stack = self.max_in_stack.max(self.input_ptr);

        self.cur_input.state = TOKEN_LIST;
        self.cur_input.index = kind;
        self.cur_input.start = p;

        if kind >= MACRO {
            let idx = self.checked_mem_index(p)?;
            self.mem[idx].s0 += 1;
            if kind == MACRO {
                self.cur_input.limit = self.param_ptr as i32;
            } else {
                self.cur_input.loc = self.mem[idx].s1;
                if self
                    .int_par(crate::state::INT_PAR_TRACING_MACROS)
                    .unwrap_or(0)
                    > 1
                {
                    let name = match kind {
                        MARK_TEXT => "\\mark".to_string(),
                        WRITE_TEXT => "\\write".to_string(),
                        _ => self
                            .cmd_chr_text(
                                CommandCode::AssignToks as u8,
                                (crate::state::LOCAL_BASE + crate::state::LOCAL_OUTPUT_ROUTINE)
                                    as i32
                                    + i32::from(kind)
                                    - i32::from(OUTPUT_TEXT),
                            )
                            .unwrap_or_default(),
                    };
                    let shown = self.token_list_display_string(p)?;
                    self.transcript.push(format!("{name}->{shown}"));
                }
            }
        } else {
            self.cur_input.loc = p;
        }

        Ok(())
    }

    /// Ends the current token-list input frame (`end_token_list`).
    pub fn end_token_list(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_input.index >= BACKED_UP {
            if self.cur_input.index <= INSERTED {
                self.flush_list(self.cur_input.start)?;
            } else {
                self.delete_token_ref(self.cur_input.start)?;
                if self.cur_input.index == MACRO {
                    while self.param_ptr as i32 > self.cur_input.limit {
                        self.param_ptr -= 1;
                        let p = self.param_stack[self.param_ptr];
                        self.flush_list(p)?;
                    }
                }
            }
        } else if self.cur_input.index == U_TEMPLATE {
            if self.align_state > 500_000 {
                self.align_state = 0;
            } else {
                return Err(TeXMouthError::MalformedMacroParameters);
            }
        }

        if self.input_ptr == 0 {
            return Err(TeXMouthError::EndOfInput);
        }
        self.input_ptr -= 1;
        self.cur_input = self.input_stack[self.input_ptr];
        Ok(())
    }

    /// Pushes `cur_tok` back onto the input stream (`back_input`).
    pub fn back_input(&mut self) -> Result<(), TeXMouthError> {
        while self.cur_input.state == TOKEN_LIST
            && self.cur_input.loc == TEX_NULL
            && self.cur_input.index != V_TEMPLATE
        {
            self.end_token_list()?;
        }

        let p = self.get_avail()?;
        self.mem[p as usize].s0 = self.cur_tok;
        if self.cur_tok < RIGHT_BRACE_LIMIT {
            if self.cur_tok < LEFT_BRACE_LIMIT {
                self.align_state -= 1;
            } else {
                self.align_state += 1;
            }
        }

        if self.input_ptr >= self.input_stack_capacity {
            return Err(TeXMouthError::InputStackOverflow);
        }
        if self.input_stack.len() <= self.input_ptr {
            self.input_stack.push(self.cur_input);
        } else {
            self.input_stack[self.input_ptr] = self.cur_input;
        }
        self.input_ptr += 1;
        self.max_in_stack = self.max_in_stack.max(self.input_ptr);

        self.cur_input.state = TOKEN_LIST;
        self.cur_input.start = p;
        self.cur_input.index = BACKED_UP;
        self.cur_input.loc = p;
        Ok(())
    }

    /// Deletes one token-list reference (`delete_token_ref`).
    pub fn delete_token_ref(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        if self.mem[idx].s0 == TEX_NULL {
            self.flush_list(p)?;
        } else {
            self.mem[idx].s0 -= 1;
        }
        Ok(())
    }

    /// Pushes a sequence of tokens back so that `tokens[0]` is read next.
    pub fn unget_tokens(&mut self, tokens: &[i32]) -> Result<(), TeXMouthError> {
        for &token in tokens.iter().rev() {
            self.cur_tok = token;
            self.back_input()?;
        }
        Ok(())
    }

    /// Reads one unexpanded token (`get_token`).
    pub fn get_token(&mut self) -> Result<(), TeXMouthError> {
        self.no_new_control_sequence = false;
        self.get_next()?;
        self.no_new_control_sequence = true;
        if self.cur_cs == 0 {
            self.cur_tok = self.cur_cmd as i32 * MAX_CHAR_VAL + self.cur_chr;
        } else {
            self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        }
        Ok(())
    }

    /// Reads the control sequence that receives a definition (`get_r_token`).
    ///
    /// Spaces are skipped. If a non-control-sequence token is read, TeX pushes
    /// that token back, inserts the inaccessible frozen control sequence, and
    /// retries; this preserves the C recovery behavior used by `\def`-like
    /// primitives.
    pub fn get_r_token(&mut self) -> Result<(), TeXMouthError> {
        loop {
            loop {
                self.get_token()?;
                if self.cur_tok != SPACE_TOKEN {
                    break;
                }
            }

            let valid_cs = usize::try_from(self.cur_cs)
                .ok()
                .is_some_and(|index| self.cur_cs != 0 && index < self.cs_table.len());
            if valid_cs {
                return Ok(());
            }

            if self.cur_cs == 0 {
                self.back_input()?;
            }
            self.cur_tok = CS_TOKEN_FLAG + FROZEN_PROTECTION_ID;
            self.back_input()?;
            self.cur_input.index = INSERTED;
        }
    }

    /// Scans a balanced token list into `def_ref` (`scan_toks`).
    ///
    /// With `macro_def`, the leading parameter text is translated into TeX's
    /// `MATCH_TOKEN`/`END_MATCH_TOKEN` representation before the replacement
    /// text is scanned. With `xpand`, expandable tokens are expanded while
    /// scanning and `\the` results are spliced directly into the output list.
    /// The return value is the final allocated token cell, matching the C
    /// function's `p` result.
    pub fn scan_toks(&mut self, macro_def: bool, xpand: bool) -> Result<i32, TeXMouthError> {
        self.scanner_status = if macro_def { DEFINING } else { ABSORBING };
        self.warning_index = self.cur_cs;
        self.def_ref = self.get_avail()?;
        let def_ref = self.checked_mem_index(self.def_ref)?;
        self.mem[def_ref].s0 = TEX_NULL;
        self.mem[def_ref].s1 = TEX_NULL;

        let mut p = self.def_ref;
        let mut hash_brace = 0;
        let mut t = ZERO_TOKEN;
        let mut missing_left_brace = false;

        if macro_def {
            'parameter_text: loop {
                self.get_token()?;
                if self.cur_tok < RIGHT_BRACE_LIMIT {
                    self.append_scanned_token(&mut p, END_MATCH_TOKEN)?;
                    missing_left_brace = self.cur_cmd == CommandCode::RightBrace as u8;
                    break 'parameter_text;
                }

                if self.cur_cmd == CommandCode::MacParam as u8 {
                    let match_token = MATCH_TOKEN + self.cur_chr;
                    self.get_token()?;
                    if self.cur_tok < LEFT_BRACE_LIMIT {
                        hash_brace = self.cur_tok;
                        self.append_scanned_token(&mut p, self.cur_tok)?;
                        self.append_scanned_token(&mut p, END_MATCH_TOKEN)?;
                        break 'parameter_text;
                    }

                    if t == ZERO_TOKEN + 9 {
                        continue;
                    }

                    t += 1;
                    if self.cur_tok != t {
                        self.back_input()?;
                    }
                    self.cur_tok = match_token;
                }

                self.append_scanned_token(&mut p, self.cur_tok)?;
            }

            if missing_left_brace {
                self.align_state += 1;
                self.scanner_status = NORMAL;
                if hash_brace != 0 {
                    self.append_scanned_token(&mut p, hash_brace)?;
                }
                return Ok(p);
            }
        } else {
            self.scan_left_brace()?;
        }

        let mut unbalance = 1i32;
        loop {
            if xpand {
                loop {
                    self.get_next()?;
                    if self.cur_cmd >= CommandCode::Call as u8
                        && self.is_protected_call(self.cur_chr)?
                    {
                        self.cur_cmd = CommandCode::Relax as u8;
                        self.cur_chr = NO_EXPAND_FLAG;
                    }
                    if self.cur_cmd <= MAX_COMMAND {
                        break;
                    }
                    if self.cur_cmd != CommandCode::The as u8 {
                        self.expand()?;
                    } else {
                        for token in self.the_token_vec()? {
                            self.append_scanned_token(&mut p, token)?;
                        }
                    }
                }
                self.x_token()?;
            } else {
                self.get_token()?;
            }

            if self.cur_tok < RIGHT_BRACE_LIMIT {
                if self.cur_cmd < CommandCode::RightBrace as u8 {
                    unbalance += 1;
                } else {
                    unbalance -= 1;
                    if unbalance == 0 {
                        break;
                    }
                }
            } else if self.cur_cmd == CommandCode::MacParam as u8 && macro_def {
                let parameter_mark = self.cur_tok;
                if xpand {
                    self.get_x_token()?;
                } else {
                    self.get_token()?;
                }

                if self.cur_cmd != CommandCode::MacParam as u8 {
                    if self.cur_tok <= ZERO_TOKEN || self.cur_tok > t {
                        self.back_input()?;
                        self.cur_tok = parameter_mark;
                    } else {
                        self.cur_tok = (OUT_PARAM_TOKEN - 48) + self.cur_chr;
                    }
                }
            }

            self.append_scanned_token(&mut p, self.cur_tok)?;
        }

        self.scanner_status = NORMAL;
        if hash_brace != 0 {
            self.append_scanned_token(&mut p, hash_brace)?;
        }
        Ok(p)
    }

    fn append_scanned_token(&mut self, p: &mut i32, token: i32) -> Result<(), TeXMouthError> {
        let q = self.get_avail()?;
        let p_index = self.checked_mem_index(*p)?;
        self.mem[p_index].s1 = q;
        let q_index = self.checked_mem_index(q)?;
        self.mem[q_index].s0 = token;
        self.mem[q_index].s1 = TEX_NULL;
        *p = q;
        Ok(())
    }

    fn is_protected_call(&self, reference: i32) -> Result<bool, TeXMouthError> {
        if reference == TEX_NULL {
            return Ok(false);
        }
        let first = self.mem[self.checked_mem_index(reference)?].s1;
        if first == TEX_NULL {
            return Ok(false);
        }
        Ok(self.mem[self.checked_mem_index(first)?].s0 == PROTECTED_TOKEN)
    }

    /// Reads the next lexical token (`get_next`).
    pub fn get_next(&mut self) -> Result<(), TeXMouthError> {
        'restart: loop {
            self.cur_cs = 0;

            if self.cur_input.state != TOKEN_LIST {
                'texswitch: loop {
                    if self.cur_input.loc <= self.cur_input.limit {
                        self.cur_chr = self.buffer_at(self.cur_input.loc)?;
                        self.cur_input.loc += 1;

                        if (0xD800..0xDC00).contains(&self.cur_chr)
                            && self.cur_input.loc <= self.cur_input.limit
                        {
                            let lower = self.buffer_at(self.cur_input.loc)?;
                            if (0xDC00..0xE000).contains(&lower) {
                                self.cur_input.loc += 1;
                                self.cur_chr =
                                    65536 + (self.cur_chr - 0xD800) * 1024 + (lower - 0xDC00);
                            }
                        }

                        'reswitch: loop {
                            self.cur_cmd = self.cat_code(self.cur_chr.max(0) as u32);
                            match (self.cur_input.state, self.cur_cmd) {
                                (_, CAT_IGNORE) => continue 'texswitch,
                                (SKIP_BLANKS, CAT_SPACER) | (NEW_LINE, CAT_SPACER) => {
                                    continue 'texswitch;
                                }
                                (_, CAT_ESCAPE) => {
                                    self.scan_control_sequence()?;
                                    self.check_outer_validity()?;
                                    break 'texswitch;
                                }
                                (_, CAT_ACTIVE_CHAR) => {
                                    let cs = self.active_cs_id(self.cur_chr);
                                    self.set_current_cs(cs);
                                    self.cur_input.state = MID_LINE;
                                    self.check_outer_validity()?;
                                    break 'texswitch;
                                }
                                (_, CAT_SUP_MARK) => {
                                    if self.expand_sup_mark()? {
                                        continue 'reswitch;
                                    }
                                    self.cur_input.state = MID_LINE;
                                    break 'texswitch;
                                }
                                (_, CAT_INVALID_CHAR) => {
                                    return Err(TeXMouthError::InvalidCharacter(self.cur_chr));
                                }
                                (MID_LINE, CAT_SPACER) => {
                                    self.cur_input.state = SKIP_BLANKS;
                                    self.cur_chr = i32::from(b' ');
                                    break 'texswitch;
                                }
                                (MID_LINE, CAT_CAR_RET) => {
                                    self.cur_input.loc = self.cur_input.limit + 1;
                                    self.cur_cmd = CommandCode::Spacer as u8;
                                    self.cur_chr = i32::from(b' ');
                                    break 'texswitch;
                                }
                                (_, CAT_COMMENT) | (SKIP_BLANKS, CAT_CAR_RET) => {
                                    self.cur_input.loc = self.cur_input.limit + 1;
                                    continue 'texswitch;
                                }
                                (NEW_LINE, CAT_CAR_RET) => {
                                    self.cur_input.loc = self.cur_input.limit + 1;
                                    self.set_current_cs(self.par_loc);
                                    self.check_outer_validity()?;
                                    break 'texswitch;
                                }
                                (MID_LINE, CAT_LEFT_BRACE) => {
                                    self.align_state += 1;
                                    break 'texswitch;
                                }
                                (SKIP_BLANKS, CAT_LEFT_BRACE) | (NEW_LINE, CAT_LEFT_BRACE) => {
                                    self.cur_input.state = MID_LINE;
                                    self.align_state += 1;
                                    break 'texswitch;
                                }
                                (MID_LINE, CAT_RIGHT_BRACE) => {
                                    self.align_state -= 1;
                                    break 'texswitch;
                                }
                                (SKIP_BLANKS, CAT_RIGHT_BRACE) | (NEW_LINE, CAT_RIGHT_BRACE) => {
                                    self.cur_input.state = MID_LINE;
                                    self.align_state -= 1;
                                    break 'texswitch;
                                }
                                (SKIP_BLANKS | NEW_LINE, CAT_MATH_SHIFT)
                                | (SKIP_BLANKS | NEW_LINE, CAT_TAB_MARK)
                                | (SKIP_BLANKS | NEW_LINE, CAT_MAC_PARAM)
                                | (SKIP_BLANKS | NEW_LINE, CAT_SUB_MARK)
                                | (SKIP_BLANKS | NEW_LINE, CAT_LETTER)
                                | (SKIP_BLANKS | NEW_LINE, CAT_OTHER_CHAR) => {
                                    self.cur_input.state = MID_LINE;
                                    break 'texswitch;
                                }
                                _ => break 'texswitch,
                            }
                        }
                    } else {
                        self.cur_input.state = NEW_LINE;

                        if self.cur_input.name > 17 {
                            self.first = self.cur_input.start;
                            if !self.force_eof && self.load_next_source_line(true)? {
                                continue 'restart;
                            }

                            if !self.force_eof {
                                let index = self.cur_input.index as usize;
                                let every_eof = self.local(crate::state::LOCAL_EVERY_EOF)?;
                                if every_eof != TEX_NULL
                                    && !self.eof_seen.get(index).copied().unwrap_or(false)
                                {
                                    self.cur_input.limit = self.first - 1;
                                    if self.eof_seen.len() <= index {
                                        self.eof_seen.resize(index + 1, false);
                                    }
                                    self.eof_seen[index] = true;
                                    self.begin_token_list(every_eof, EVERY_EOF_TEXT)?;
                                    continue 'restart;
                                }
                                self.force_eof = true;
                            }

                            if self.force_eof {
                                // C warns about groups/conditionals still
                                // open at end-of-file when `\tracingnesting`.
                                if self.int_par(crate::state::INT_PAR_TRACING_NESTING)? > 0
                                    && (self.grp_stack.get(self.in_open).copied()
                                        != Some(self.cur_boundary)
                                        || self.if_stack.get(self.in_open).copied()
                                            != Some(self.cond_ptr))
                                {
                                    self.file_warning()?;
                                }
                                self.line += 1;
                                // C `get_next` file close: real files
                                // (name_field >= 19) echo `)` and drop
                                // `open_parens`.
                                if self.cur_input.name >= 19 {
                                    self.print_char(i32::from(b')'))?;
                                    self.open_parens -= 1;
                                }
                                self.force_eof = false;
                                self.end_file_reading()?;
                                self.check_outer_validity()?;
                                continue 'restart;
                            }
                        }

                        if self.load_next_source_line(true)? {
                            continue 'restart;
                        }

                        if self.cur_input.name != 0 {
                            self.cur_cmd = CommandCode::Relax as u8;
                            self.cur_chr = 0;
                            return Ok(());
                        }

                        if self.input_ptr > 0 {
                            self.end_file_reading()?;
                            continue 'restart;
                        }
                        return Err(TeXMouthError::EndOfInput);
                    }
                }
            } else if self.cur_input.loc != TEX_NULL {
                let loc_idx = self.checked_mem_index(self.cur_input.loc)?;
                let token = self.mem[loc_idx].s0;
                self.cur_input.loc = self.mem[loc_idx].s1;

                if token >= CS_TOKEN_FLAG {
                    let cs = token - CS_TOKEN_FLAG;
                    self.set_current_cs(cs);
                    if self.cur_cmd >= CommandCode::OuterCall as u8 {
                        if self.cur_cmd == CommandCode::DontExpand as u8 {
                            let next_idx = self.checked_mem_index(self.cur_input.loc)?;
                            let deferred_cs = self.mem[next_idx].s0 - CS_TOKEN_FLAG;
                            self.cur_input.loc = TEX_NULL;
                            self.set_current_cs(deferred_cs);
                            if self.cur_cmd > MAX_COMMAND {
                                self.cur_cmd = CommandCode::Relax as u8;
                                self.cur_chr = NO_EXPAND_FLAG;
                            }
                        } else {
                            self.check_outer_validity()?;
                        }
                    }
                } else {
                    self.cur_cmd = (token / MAX_CHAR_VAL) as u8;
                    self.cur_chr = token % MAX_CHAR_VAL;
                    match self.cur_cmd {
                        x if x == CommandCode::LeftBrace as u8 => self.align_state += 1,
                        x if x == CommandCode::RightBrace as u8 => self.align_state -= 1,
                        OUT_PARAM => {
                            let param_idx = self.cur_input.limit + self.cur_chr - 1;
                            if param_idx < 0 || param_idx as usize >= self.param_stack.len() {
                                return Err(TeXMouthError::ParameterStackOverflow);
                            }
                            let p = self.param_stack[param_idx as usize];
                            self.begin_token_list(p, PARAMETER)?;
                            continue 'restart;
                        }
                        _ => {}
                    }
                }
            } else {
                self.end_token_list()?;
                continue 'restart;
            }

            if self.cur_cmd <= CAR_RET
                && self.cur_cmd >= CommandCode::TabMark as u8
                && self.align_state == 0
            {
                if self.scanner_status == ALIGNING || self.cur_align == TEX_NULL {
                    return Err(TeXMouthError::MalformedMacroParameters);
                }

                // §818: `&`, `\cr`, or `\span` at brace level zero closes the
                // current alignment entry and feeds its v-template.
                let stored = self.mem[(self.cur_align + 5) as usize].s0;
                self.mem[(self.cur_align + 5) as usize].s0 = self.cur_chr;
                if stored == CommandCode::Omit as i32 {
                    self.begin_token_list(self.omit_template(), V_TEMPLATE)?;
                } else {
                    let template = self.mem[(self.cur_align + 2) as usize].s1;
                    self.begin_token_list(template, V_TEMPLATE)?;
                }
                self.align_state = 1_000_000;
                continue 'restart;
            }

            return Ok(());
        }
    }

    fn checked_mem_index(&self, p: i32) -> Result<usize, TeXMouthError> {
        if p < 0 || p as usize >= self.mem.len() {
            Err(crate::scanner::oob(p))
        } else {
            Ok(p as usize)
        }
    }

    fn buffer_at(&self, i: i32) -> Result<i32, TeXMouthError> {
        self.buffer
            .get(i.max(0) as usize)
            .copied()
            .ok_or_else(|| crate::scanner::oob(i))
    }

    fn buffer_at_or_zero(&self, i: i32) -> i32 {
        if i < 0 {
            0
        } else {
            self.buffer.get(i as usize).copied().unwrap_or(0)
        }
    }

    fn active_cs_id(&mut self, character: i32) -> i32 {
        if let Some(cs) = self.cs_table.lookup_active(character) {
            cs
        } else if self.no_new_control_sequence {
            UNDEFINED_CS_ID
        } else {
            self.cs_table
                .define_active(character, CommandCode::UndefinedCs, 0)
        }
    }

    /// C `check_outer_validity` (§350–§354): when an `\outer` control
    /// sequence or end-of-file arrives while the scanner is inside a
    /// restricted context, TeX reports the runaway, pushes back the
    /// offending control sequence, and inserts a recovery token so the
    /// run can continue.
    fn check_outer_validity(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_cmd < CommandCode::OuterCall as u8 || self.scanner_status == NORMAL {
            return Ok(());
        }

        if self.cur_cs != 0 {
            if self.cur_input.state == TOKEN_LIST
                || self.cur_input.name < 1
                || self.cur_input.name > 17
            {
                let p = self.get_avail()?;
                self.mem[p as usize].s0 = CS_TOKEN_FLAG + self.cur_cs;
                self.begin_token_list(p, BACKED_UP)?;
            }
            self.cur_cmd = CommandCode::Spacer as u8;
            self.cur_chr = i32::from(b' ');
        }

        if self.scanner_status > SKIPPING {
            self.runaway()?;
            let headline = if self.cur_cs == 0 {
                "File ended"
            } else {
                self.cur_cs = 0;
                "Forbidden control sequence found"
            };

            let (kind, head) = match self.scanner_status {
                DEFINING => ("definition", RIGHT_BRACE_TOKEN + i32::from(b'}')),
                MATCHING => {
                    self.long_state = CommandCode::OuterCall as u8;
                    ("use", self.par_token)
                }
                ALIGNING => ("preamble", RIGHT_BRACE_TOKEN + i32::from(b'}')),
                _ => ("text", RIGHT_BRACE_TOKEN + i32::from(b'}')),
            };

            let p = self.get_avail()?;
            self.mem[p as usize].s0 = head;
            let list = if self.scanner_status == ALIGNING {
                let q = self.get_avail()?;
                self.mem[q as usize].s0 = CS_TOKEN_FLAG + FROZEN_CR_ID;
                self.mem[q as usize].s1 = p;
                self.align_state = -1_000_000;
                q
            } else {
                p
            };
            self.begin_token_list(list, INSERTED)?;

            let name = self.cs_name_text(self.warning_index);
            self.report_error(
                &format!("{headline} while scanning {kind} of \\{name}"),
                &[
                    "I suspect you have forgotten a `}', causing me",
                    "to read past where you wanted me to stop.",
                    "I'll try to recover; but if the error is serious,",
                    "you'd better type `E' or `X' now and fix your file.",
                ],
            );
        } else {
            let if_name = self
                .cmd_chr_text(CommandCode::IfTest as u8, i32::from(self.cur_if))
                .unwrap_or_default();
            let skipped = if self.cur_cs != 0 {
                self.cur_cs = 0;
                "A forbidden control sequence occurred in skipped text."
            } else {
                "The file ended while I was skipping conditional text."
            };
            self.cur_tok = CS_TOKEN_FLAG + FROZEN_FI_ID;
            self.ins_error(
                &format!(
                    "Incomplete {if_name}; all text was ignored after line {}",
                    self.skip_line
                ),
                &[
                    skipped,
                    "This kind of error happens when you say `\\if...' and forget",
                    "the matching `\\fi'. I've inserted a `\\fi'; this might work.",
                ],
            )?;
        }
        Ok(())
    }

    fn scan_control_sequence(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_input.loc > self.cur_input.limit {
            self.set_current_cs(NULL_CS_ID);
            return Ok(());
        }

        'start_cs: loop {
            let mut k = self.cur_input.loc;
            self.cur_chr = self.buffer_at(k)?;
            let mut cat = self.cat_code(self.cur_chr.max(0) as u32);
            k += 1;

            self.cur_input.state = if cat == CAT_LETTER || cat == CAT_SPACER {
                SKIP_BLANKS
            } else {
                MID_LINE
            };

            if cat == CAT_LETTER && k <= self.cur_input.limit {
                while k <= self.cur_input.limit {
                    self.cur_chr = self.buffer_at(k)?;
                    cat = self.cat_code(self.cur_chr.max(0) as u32);
                    k += 1;
                    if cat != CAT_LETTER {
                        break;
                    }
                }

                if cat == CAT_SUP_MARK
                    && self.buffer_at_or_zero(k) == self.cur_chr
                    && k < self.cur_input.limit
                    && self.expand_sup_in_cs(k, self.cur_chr)?
                {
                    continue 'start_cs;
                }

                if cat != CAT_LETTER {
                    k -= 1;
                }

                if k > self.cur_input.loc + 1 {
                    let len = k - self.cur_input.loc;
                    self.cur_cs = self.id_lookup(self.cur_input.loc, len)?;
                    self.cur_input.loc = k;
                    self.cur_cmd = self.cs_command(self.cur_cs);
                    self.cur_chr = self.cs_character(self.cur_cs);
                    return Ok(());
                }
            } else if cat == CAT_SUP_MARK
                && self.buffer_at_or_zero(k) == self.cur_chr
                && k < self.cur_input.limit
                && self.expand_sup_in_cs(k, self.cur_chr)?
            {
                continue 'start_cs;
            }

            self.cur_cs = self.id_lookup(self.cur_input.loc, 1)?;
            self.cur_input.loc += 1;
            self.cur_cmd = self.cs_command(self.cur_cs);
            self.cur_chr = self.cs_character(self.cur_cs);
            return Ok(());
        }
    }

    fn expand_sup_in_cs(&mut self, k: i32, sup_chr: i32) -> Result<bool, TeXMouthError> {
        let mut sup_count = 2;
        while sup_count < 6
            && k + 2 * sup_count - 2 <= self.cur_input.limit
            && self.buffer_at_or_zero(k + sup_count - 1) == sup_chr
        {
            sup_count += 1;
        }

        if k + 2 * sup_count - 2 > self.cur_input.limit {
            if self.buffer_at_or_zero(k + 1) < 128 && k < self.cur_input.limit {
                let c = self.buffer_at(k + 1)?;
                self.replace_buffer_span(k - 1, 2, if c < 64 { c + 64 } else { c - 64 })?;
                return Ok(true);
            }
            return Ok(false);
        }

        let mut all_hex = true;
        for d in 1..=sup_count {
            if !is_lower_hex(self.buffer_at_or_zero(k + sup_count - 2 + d)) {
                all_hex = false;
                break;
            }
        }

        if !all_hex {
            let c = self.buffer_at_or_zero(k + 1);
            if c < 128 {
                self.replace_buffer_span(k - 1, 2, if c < 64 { c + 64 } else { c - 64 })?;
                return Ok(true);
            }
            return Ok(false);
        }

        let mut value = 0;
        for d in 1..=sup_count {
            value = 16 * value + hex_value(self.buffer_at(k + sup_count - 2 + d)?);
        }

        if value > BIGGEST_USV {
            return Ok(false);
        }

        self.replace_buffer_span(k - 1, 2 * sup_count - 1, value)?;
        Ok(true)
    }

    fn expand_sup_mark(&mut self) -> Result<bool, TeXMouthError> {
        let loc = self.cur_input.loc;
        if self.buffer_at_or_zero(loc) != self.cur_chr || loc >= self.cur_input.limit {
            return Ok(false);
        }

        let mut sup_count = 2;
        while sup_count < 6
            && loc + 2 * sup_count - 2 <= self.cur_input.limit
            && self.buffer_at_or_zero(loc + sup_count - 1) == self.cur_chr
        {
            sup_count += 1;
        }

        if loc + 2 * sup_count - 2 > self.cur_input.limit {
            let c = self.buffer_at_or_zero(loc + 1);
            if loc < self.cur_input.limit && c < 128 {
                self.cur_input.loc += 2;
                self.cur_chr = if c < 64 { c + 64 } else { c - 64 };
                return Ok(true);
            }
            return Ok(false);
        }

        let mut all_hex = true;
        for d in 1..=sup_count {
            if !is_lower_hex(self.buffer_at_or_zero(loc + sup_count - 2 + d)) {
                all_hex = false;
                break;
            }
        }

        if !all_hex {
            let c = self.buffer_at(loc + 1)?;
            if c < 128 {
                self.cur_input.loc += 2;
                self.cur_chr = if c < 64 { c + 64 } else { c - 64 };
                return Ok(true);
            }
            return Ok(false);
        }

        let mut value = 0;
        for d in 1..=sup_count {
            value = 16 * value + hex_value(self.buffer_at(loc + sup_count - 2 + d)?);
        }

        if value > BIGGEST_USV {
            self.cur_chr = self.buffer_at(loc)?;
            return Ok(false);
        }

        self.cur_input.loc += 2 * sup_count - 1;
        self.cur_chr = value;
        Ok(true)
    }

    fn replace_buffer_span(
        &mut self,
        at: i32,
        remove: i32,
        value: i32,
    ) -> Result<(), TeXMouthError> {
        if at < 0 || at as usize >= self.buffer.len() || remove < 0 {
            return Err(crate::scanner::oob(at));
        }

        let at = at as usize;
        let remove = remove as usize;
        if at + remove >= self.buffer.len() {
            return Err(crate::scanner::oob(at as i32));
        }

        self.buffer[at] = value;
        self.buffer.drain(at + 1..=at + remove);
        self.cur_input.limit -= remove as i32;
        self.last = self.cur_input.limit;
        Ok(())
    }

    pub(crate) fn end_file_reading(&mut self) -> Result<(), TeXMouthError> {
        if self.input_ptr == 0 {
            return Err(TeXMouthError::EndOfInput);
        }

        let index = self.cur_input.index as usize;
        if index < self.line_stack.len() {
            self.line = self.line_stack[index];
        }
        self.first = self.cur_input.start;

        if self.cur_input.name > 17 && index < self.input_file.len() {
            // C `u_close(input_file[cur_input.index])` for real input files.
            self.input_file[index] = None;
        }

        self.input_ptr -= 1;
        self.cur_input = self.input_stack[self.input_ptr];
        self.in_open = self.in_open.saturating_sub(1);

        if self.file_sources.len() <= self.in_open {
            self.file_sources
                .resize_with(self.in_open + 1, FileInputSource::default);
        }
        let parent = std::mem::take(&mut self.file_sources[self.in_open]);
        self.buffer = parent.buffer;
        self.source_lines = parent.lines;
        Ok(())
    }
}

#[allow(dead_code)]
fn _assert_memory_word_layout(word: MemoryWord) -> (i32, i32) {
    (word.s0, word.s1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::TeXEngineState;

    const LETTER_TOKEN: i32 = 11 * MAX_CHAR_VAL;
    const OTHER_TOKEN: i32 = 12 * MAX_CHAR_VAL;

    fn state() -> TeXEngineState {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state
    }

    fn token(state: &mut TeXEngineState) -> i32 {
        state.get_token().unwrap();
        state.cur_tok
    }

    #[test]
    fn scans_character_tokens_and_collapses_spaces() {
        let mut state = state();
        state.set_source_text("a  b");

        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'a'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'b'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn queued_input_lines_preserve_line_state_and_blank_line_pars() {
        let mut state = state();
        state.set_source_text("a\n\nb");

        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'a'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), state.par_token);
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'b'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.line, 3);
    }

    #[test]
    fn comments_discard_the_remainder_of_the_line() {
        let mut state = state();
        state.set_source_text("a% ignored entirely");

        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'a'));
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn braces_are_tokens_and_update_align_state() {
        let mut state = state();
        let initial_align_state = state.align_state;
        state.set_source_text("{x}");

        assert_eq!(token(&mut state), LEFT_BRACE_TOKEN + i32::from(b'{'));
        assert_eq!(state.align_state, initial_align_state + 1);
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'x'));
        assert_eq!(token(&mut state), RIGHT_BRACE_TOKEN + i32::from(b'}'));
        assert_eq!(state.align_state, initial_align_state);
    }

    #[test]
    fn scans_multiletter_control_sequences_and_skips_following_blanks() {
        let mut state = state();
        state.set_source_text("\\foo  x");

        assert_eq!(
            token(&mut state),
            CS_TOKEN_FLAG + state.cs_table.lookup_str("foo").unwrap()
        );
        assert_eq!(state.cur_cmd, CommandCode::UndefinedCs as u8);
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'x'));
    }

    #[test]
    fn scans_single_character_control_sequences() {
        let mut state = state();
        state.set_source_text("\\a b");

        assert_eq!(
            token(&mut state),
            CS_TOKEN_FLAG + state.cs_table.lookup_str("a").unwrap()
        );
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'b'));
    }

    #[test]
    fn escape_before_the_endline_character_scans_a_control_symbol() {
        let mut state = state();
        state.set_source_text("\\");

        assert_eq!(token(&mut state), CS_TOKEN_FLAG + state.cur_cs);
        let cs = state.cs_table.lookup_units(&[i32::from(b'\r')]).unwrap();
        assert_eq!(state.cur_cs, cs);
        assert_eq!(state.cur_cmd, CommandCode::UndefinedCs as u8);
    }

    #[test]
    fn active_characters_resolve_through_the_active_table() {
        let mut state = state();
        state.set_cat_code(u32::from(b'~'), CAT_ACTIVE_CHAR);
        let active = state.define_active_cs('~', CommandCode::Relax, 7);
        state.set_source_text("~");

        assert_eq!(token(&mut state), CS_TOKEN_FLAG + active);
        assert_eq!(state.cur_cmd, CommandCode::Relax as u8);
        assert_eq!(state.cur_chr, 7);
    }

    #[test]
    fn old_style_double_caret_expands_ascii_control_characters() {
        let mut state = state();
        state.set_cat_code(1, CAT_LETTER);
        state.set_source_text("^^A");

        assert_eq!(token(&mut state), LETTER_TOKEN + 1);
    }

    #[test]
    fn extended_caret_hex_expands_unicode_scalars() {
        let mut state = state();
        state.set_source_text("^^41 ^^^^0041");

        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'A'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'A'));
    }

    #[test]
    fn old_style_caret_fallback_expands_the_first_non_hex_character() {
        let mut state = state();
        state.set_source_text("^^4A");

        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b't'));
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'A'));
    }

    #[test]
    fn utf16_surrogate_pairs_decode_to_one_character_token() {
        let mut state = state();
        state.set_source_units(&[0xD83D, 0xDE00]);

        assert_eq!(token(&mut state), OTHER_TOKEN + 0x1F600);
    }

    #[test]
    fn token_list_input_reads_linked_tokens_then_restores_input() {
        let mut state = state();
        let head = state
            .store_token_list(&[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        state.begin_token_list(head, INSERTED).unwrap();

        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'x'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.input_ptr, 0);
    }

    #[test]
    fn no_new_control_sequence_suppresses_name_creation() {
        let mut state = state();
        state.no_new_control_sequence = true;
        state.set_source_text("\\unknown");

        state.get_next().unwrap();
        assert_eq!(state.cur_cs, UNDEFINED_CS_ID);
        assert_eq!(state.cur_cmd, CommandCode::UndefinedCs as u8);
        assert!(state.cs_table.lookup_str("unknown").is_none());
    }

    #[test]
    fn invalid_character_reports_structured_error() {
        let mut state = state();
        state.set_cat_code(7, CAT_INVALID_CHAR);
        state.set_source_units(&[7]);

        let err = state.get_token().unwrap_err();
        assert!(matches!(err, TeXMouthError::InvalidCharacter(7)));
    }

    #[test]
    fn input_reads_a_registered_source_and_restores_the_parent() {
        let mut state = state();
        state.register_input_source("child", "c");
        state.set_source_text("a \\input child b");

        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'a' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'c' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'b' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(state.next_expanded_token().unwrap(), 0);
        assert_eq!(state.in_open, 0);
        assert_eq!(state.input_ptr, 0);
    }

    #[test]
    fn nested_input_restores_each_parent_source_in_order() {
        let mut state = state();
        state.register_input_source("child", "c\\input grand d");
        state.register_input_source("grand.tex", "g");
        state.set_source_text("a \\input child b");

        let tokens = (0..7)
            .map(|_| state.next_expanded_token().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            tokens,
            vec![
                LETTER_TOKEN + b'a' as i32,
                SPACE_TOKEN,
                LETTER_TOKEN + b'c' as i32,
                LETTER_TOKEN + b'g' as i32,
                SPACE_TOKEN,
                LETTER_TOKEN + b'd' as i32,
                SPACE_TOKEN,
            ]
        );
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'b' as i32
        );
        assert_eq!(state.in_open, 0);
    }

    #[test]
    fn input_accepts_braced_and_quoted_names() {
        let mut state = state();
        state.register_input_source("child", "c");
        state.register_input_source("other name", "o");
        state.set_source_text("\\input{child}\\input \"other name\"");

        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'c' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'o' as i32
        );
    }

    #[test]
    fn endinput_forces_eof_at_the_end_of_the_line() {
        let mut state = state();
        state.register_input_source("child", "c\\endinput ignored");
        state.set_source_text("a \\input child b");

        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'a' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'c' as i32
        );
        for expected in b"ignored" {
            assert_eq!(
                state.next_expanded_token().unwrap(),
                LETTER_TOKEN + i32::from(*expected)
            );
        }
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'b' as i32
        );
        assert_eq!(state.in_open, 0);
    }

    #[test]
    fn everyeof_is_inserted_once_at_file_end() {
        let mut state = state();
        let every_eof = state
            .store_token_ref(&[LETTER_TOKEN + b'E' as i32])
            .unwrap();
        state
            .set_local(crate::state::LOCAL_EVERY_EOF, every_eof)
            .unwrap();
        state.register_input_source("child", "c");
        state.set_source_text("a \\input child b");

        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'a' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'c' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), SPACE_TOKEN);
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'E' as i32
        );
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'b' as i32
        );
        assert!(state.eof_seen.get(1).copied().unwrap_or(false));
    }

    #[test]
    fn disabled_endlinechar_removes_the_endline_token() {
        let mut state = state();
        state
            .set_int_par(crate::state::INT_PAR_END_LINE_CHAR, -1)
            .unwrap();
        state.register_input_source("child", "c\nd");
        state.set_source_text("\\input child");

        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'c' as i32
        );
        assert_eq!(
            state.next_expanded_token().unwrap(),
            LETTER_TOKEN + b'd' as i32
        );
        assert_eq!(state.next_expanded_token().unwrap(), 0);
    }

    #[test]
    fn input_reports_a_missing_registered_source() {
        let mut state = state();
        state.set_source_text("\\input missing");

        assert!(matches!(
            state.next_expanded_token(),
            Err(TeXMouthError::MissingInputFile(name)) if name == "missing"
        ));
    }

    #[test]
    fn push_and_pop_nest_preserve_the_parent_list_state() {
        let mut state = state();
        let parent_head = state.get_avail().unwrap();
        let parent_tail = state.get_avail().unwrap();
        state.head = parent_head;
        state.tail = parent_tail;
        state.mode = crate::state::TeXMode::Hmode as i16;
        state.prev_graf = 3;
        state.mode_line = 11;
        state.etex_aux = 123;
        state.aux.s1 = 456;
        state.line = 17;

        state.push_nest().unwrap();
        assert_eq!(state.nest_ptr, 1);
        assert_ne!(state.head, parent_head);
        assert_eq!(state.tail, state.head);
        assert_eq!(state.prev_graf, 0);
        assert_eq!(state.mode_line, 17);
        assert_eq!(state.etex_aux, TEX_NULL);

        state.mode = crate::state::TeXMode::Mmode as i16;
        state.tail = state.get_avail().unwrap();
        state.prev_graf = 9;
        state.mode_line = 21;
        state.aux.s1 = 999;

        state.pop_nest().unwrap();
        assert_eq!(state.nest_ptr, 0);
        assert_eq!(state.mode, crate::state::TeXMode::Hmode as i16);
        assert_eq!(state.head, parent_head);
        assert_eq!(state.tail, parent_tail);
        assert_eq!(state.prev_graf, 3);
        assert_eq!(state.mode_line, 11);
        assert_eq!(state.etex_aux, 123);
        assert_eq!(state.aux.s1, 456);
    }

    #[test]
    fn nest_stack_reports_underflow_and_overflow() {
        let mut state = state();
        assert!(matches!(
            state.pop_nest(),
            Err(TeXMemoryError::NestStackUnderflow)
        ));

        state.nest_size = 1;
        state.push_nest().unwrap();
        assert!(matches!(
            state.push_nest(),
            Err(TeXMemoryError::NestStackOverflow)
        ));
    }

    #[test]
    fn unsave_restores_nested_local_eqtb_values() {
        let mut state = state();
        let p = crate::state::COUNT_BASE;
        state.eqtb[p].set_b16_s0(LEVEL_ONE);
        state.eqtb[p].set_b16_s1(CommandCode::AssignInt as u16);
        state.eqtb[p].s1 = 5;

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state.eq_word_define(p, 42).unwrap();
        state.new_save_level(crate::state::HBOX_GROUP).unwrap();
        state.eq_word_define(p, 84).unwrap();
        assert_eq!(state.eqtb[p].s1, 84);
        assert_eq!(state.eqtb[p].b16_s0(), state.cur_level);

        state.unsave().unwrap();
        assert_eq!(state.eqtb[p].s1, 42);
        assert_eq!(state.eqtb[p].b16_s0(), LEVEL_ONE + 1);
        assert_eq!(state.cur_group, crate::state::SIMPLE_GROUP);

        state.unsave().unwrap();
        assert_eq!(state.eqtb[p].s1, 5);
        assert_eq!(state.eqtb[p].b16_s0(), LEVEL_ONE);
        assert_eq!(state.cur_group, crate::state::BOTTOM_LEVEL);
        assert_eq!(state.save_ptr, 0);
    }

    #[test]
    fn eq_define_restores_command_and_payload() {
        let mut state = state();
        let p = crate::state::LOCAL_BASE;
        state.eqtb[p].set_b16_s0(LEVEL_ONE);
        state.eqtb[p].set_b16_s1(CommandCode::AssignToks as u16);
        state.eqtb[p].s1 = 111;

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state.eq_define(p, CommandCode::Data as u16, 222).unwrap();
        assert_eq!(state.eqtb[p].b16_s1(), CommandCode::Data as u16);
        assert_eq!(state.eqtb[p].s1, 222);

        state.unsave().unwrap();
        assert_eq!(state.eqtb[p].b16_s0(), LEVEL_ONE);
        assert_eq!(state.eqtb[p].b16_s1(), CommandCode::AssignToks as u16);
        assert_eq!(state.eqtb[p].s1, 111);
    }

    #[test]
    fn global_eqtb_definitions_survive_unsave() {
        let mut state = state();
        let global = crate::state::COUNT_BASE;
        let local = crate::state::COUNT_BASE + 1;
        for p in [global, local] {
            state.eqtb[p].set_b16_s0(LEVEL_ONE);
            state.eqtb[p].set_b16_s1(CommandCode::AssignInt as u16);
            state.eqtb[p].s1 = p as i32;
        }

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state
            .geq_define(global, CommandCode::AssignInt as u16, 900)
            .unwrap();
        state.eq_word_define(local, 700).unwrap();
        state.unsave().unwrap();

        assert_eq!(state.eqtb[global].s1, 900);
        assert_eq!(state.eqtb[global].b16_s0(), LEVEL_ONE);
        assert_eq!(state.eqtb[local].s1, local as i32);
        assert_eq!(state.eqtb[local].b16_s0(), LEVEL_ONE);
    }

    #[test]
    fn level_zero_eqtb_values_restore_as_undefined() {
        // `\count` registers are LEVEL_ONE from `initialize` like C's
        // `xeq_level`; use a slot outside every initialized region so the
        // entry is genuinely LEVEL_ZERO and saves as `RESTORE_ZERO`.
        let mut state = state();
        let p = crate::state::FAMILY_BASE + crate::state::FAMILY_COUNT + 16;
        assert_eq!(state.eqtb[p].b16_s0(), crate::state::LEVEL_ZERO);
        state.eqtb[p].s1 = 123;

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state.eq_word_define(p, 77).unwrap();
        state.unsave().unwrap();

        assert_eq!(state.eqtb[p].b16_s0(), crate::state::LEVEL_ZERO);
        assert_eq!(state.eqtb[p].b16_s1(), CommandCode::UndefinedCs as u16);
        assert_eq!(state.eqtb[p].s1, 0);
    }

    #[test]
    fn control_sequence_definitions_restore_level_command_and_payload() {
        let mut state = state();
        let name = "localcs".chars().map(|c| c as i32).collect::<Vec<_>>();
        let cs = state.cs_table.lookup_or_insert(&name);

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state.eq_define_cs(cs, CommandCode::Relax, 123).unwrap();
        assert_eq!(state.cs_table.level(cs).unwrap(), LEVEL_ONE + 1);
        assert_eq!(state.cs_table.command(cs), CommandCode::Relax as u8);
        assert_eq!(state.cs_table.character(cs), 123);

        state.unsave().unwrap();
        assert_eq!(state.cs_table.level(cs).unwrap(), crate::state::LEVEL_ZERO);
        assert_eq!(state.cs_table.command(cs), CommandCode::UndefinedCs as u8);
        assert_eq!(state.cs_table.character(cs), 0);
    }

    #[test]
    fn global_control_sequence_definitions_survive_unsave() {
        let mut state = state();
        let name = "globalcs".chars().map(|c| c as i32).collect::<Vec<_>>();
        let cs = state.cs_table.lookup_or_insert(&name);

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state
            .geq_define_cs(cs, CommandCode::CharGiven, 321)
            .unwrap();
        state.unsave().unwrap();

        assert_eq!(state.cs_table.level(cs).unwrap(), LEVEL_ONE);
        assert_eq!(state.cs_table.command(cs), CommandCode::CharGiven as u8);
        assert_eq!(state.cs_table.character(cs), 321);
    }

    #[test]
    fn csname_definitions_are_local_to_the_current_group() {
        let mut state = state();
        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state.set_source_text("\\csname grouped\\endcsname");

        let cs = state.next_expanded_token().unwrap() - CS_TOKEN_FLAG;
        assert_eq!(state.cs_table.command(cs), CommandCode::Relax as u8);
        assert_eq!(state.cs_table.level(cs).unwrap(), LEVEL_ONE + 1);

        state.unsave().unwrap();
        assert_eq!(state.cs_table.command(cs), CommandCode::UndefinedCs as u8);
        assert_eq!(state.cs_table.level(cs).unwrap(), crate::state::LEVEL_ZERO);
    }

    #[test]
    fn save_for_after_reinserts_tokens_in_source_order() {
        let mut state = state();
        let first = OTHER_TOKEN + i32::from(b'a');
        let second = OTHER_TOKEN + i32::from(b'b');

        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        state.save_for_after(first).unwrap();
        state.save_for_after(second).unwrap();
        state.unsave().unwrap();

        assert_eq!(token(&mut state), first);
        assert_eq!(token(&mut state), second);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn save_stack_reports_underflow_and_overflow() {
        let mut state = state();
        assert!(matches!(
            state.unsave(),
            Err(TeXMouthError::Memory(TeXMemoryError::SaveStackUnderflow))
        ));

        state.save_size = 8;
        state.new_save_level(crate::state::SIMPLE_GROUP).unwrap();
        assert!(matches!(
            state.new_save_level(crate::state::HBOX_GROUP),
            Err(TeXMemoryError::SaveStackOverflow)
        ));
    }

    #[test]
    fn grouping_level_overflow_is_reported() {
        let mut state = state();
        state.cur_level = u16::MAX;
        assert!(matches!(
            state.new_save_level(crate::state::SIMPLE_GROUP),
            Err(TeXMemoryError::GroupingLevelOverflow)
        ));
    }

    fn scanned_tokens(state: &TeXEngineState) -> Vec<i32> {
        state
            .token_list_to_vec(state.mem[state.def_ref as usize].s1)
            .unwrap()
    }

    #[test]
    fn scan_toks_absorbs_balanced_general_text() {
        let mut state = state();
        state.set_source_text("{a{b}c}");
        let last = state.scan_toks(false, false).unwrap();

        let left = LEFT_BRACE_TOKEN + i32::from(b'{');
        let right = RIGHT_BRACE_TOKEN + i32::from(b'}');
        assert_eq!(
            scanned_tokens(&state),
            vec![
                LETTER_TOKEN + i32::from(b'a'),
                left,
                LETTER_TOKEN + i32::from(b'b'),
                right,
                LETTER_TOKEN + i32::from(b'c'),
            ]
        );
        assert_eq!(state.mem[last as usize].s1, TEX_NULL);
        assert_eq!(state.scanner_status, NORMAL);
    }

    #[test]
    fn scan_toks_builds_macro_parameter_template_and_replacement() {
        let mut state = state();
        state.set_source_text("#1{x#1}");
        state.scan_toks(true, false).unwrap();

        assert_eq!(
            scanned_tokens(&state),
            vec![
                MATCH_TOKEN + i32::from(b'#'),
                END_MATCH_TOKEN,
                LETTER_TOKEN + i32::from(b'x'),
                OUT_PARAM_TOKEN + 1,
            ]
        );
    }

    #[test]
    fn scan_toks_preserves_literal_double_hash_in_replacement() {
        let mut state = state();
        let hash = CommandCode::MacParam as i32 * MAX_CHAR_VAL + i32::from(b'#');
        state.set_source_text("#1{a##b}");
        state.scan_toks(true, false).unwrap();

        assert_eq!(
            scanned_tokens(&state),
            vec![
                MATCH_TOKEN + i32::from(b'#'),
                END_MATCH_TOKEN,
                LETTER_TOKEN + i32::from(b'a'),
                hash,
                LETTER_TOKEN + i32::from(b'b'),
            ]
        );
    }

    #[test]
    fn scan_toks_recovers_out_of_order_parameter_numbers() {
        let mut state = state();
        state.set_source_text("#2{x}");
        state.scan_toks(true, false).unwrap();

        assert_eq!(
            scanned_tokens(&state),
            vec![
                MATCH_TOKEN + i32::from(b'#'),
                OTHER_TOKEN + i32::from(b'2'),
                END_MATCH_TOKEN,
                LETTER_TOKEN + i32::from(b'x'),
            ]
        );
    }

    #[test]
    fn scan_toks_records_hash_brace_at_both_boundaries() {
        let mut state = state();
        let left = LEFT_BRACE_TOKEN + i32::from(b'{');
        state.set_source_text("#{x}");
        state.scan_toks(true, false).unwrap();

        assert_eq!(
            scanned_tokens(&state),
            vec![left, END_MATCH_TOKEN, LETTER_TOKEN + i32::from(b'x'), left,]
        );
    }

    #[test]
    fn scan_toks_recovers_a_missing_left_brace_after_parameters() {
        let mut state = state();
        let align_state = state.align_state;
        state.set_source_text("}");
        state.scan_toks(true, false).unwrap();

        assert_eq!(scanned_tokens(&state), vec![END_MATCH_TOKEN]);
        assert_eq!(state.align_state, align_state);
        assert_eq!(state.scanner_status, NORMAL);
    }

    #[test]
    fn scan_toks_ignores_a_tenth_parameter_like_tex() {
        let mut state = state();
        state.set_source_text("#1#2#3#4#5#6#7#8#9#0{x}");
        state.scan_toks(true, false).unwrap();

        let mut expected = vec![MATCH_TOKEN + i32::from(b'#'); 9];
        expected.extend([END_MATCH_TOKEN, LETTER_TOKEN + i32::from(b'x')]);
        assert_eq!(scanned_tokens(&state), expected);
    }

    #[test]
    fn scan_toks_xpand_preserves_noexpanded_macros() {
        let mut state = state();
        let cs = state
            .define_simple_macro("foo", &[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        state.set_source_text("{\\noexpand\\foo}");
        state.scan_toks(false, true).unwrap();

        assert_eq!(scanned_tokens(&state), vec![CS_TOKEN_FLAG + cs]);
    }

    #[test]
    fn scan_toks_xpand_expands_macros_and_splices_the() {
        let mut state = state();
        let z = LETTER_TOKEN + i32::from(b'z');
        state
            .define_simple_macro("foo", &[LETTER_TOKEN + i32::from(b'x')])
            .unwrap();
        let toks = state.store_token_list(&[z]).unwrap();
        state.set_toks(0, toks).unwrap();
        state.set_source_text("{\\foo\\the\\toks0}");
        state.scan_toks(false, true).unwrap();

        assert_eq!(
            scanned_tokens(&state),
            vec![LETTER_TOKEN + i32::from(b'x'), z]
        );
    }

    #[test]
    fn scan_toks_xpand_preserves_protected_macros() {
        let mut state = state();
        let cs = state
            .define_macro(
                "protected",
                &[PROTECTED_TOKEN, END_MATCH_TOKEN],
                &[LETTER_TOKEN + i32::from(b'p')],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("{\\protected}");
        state.scan_toks(false, true).unwrap();

        assert_eq!(scanned_tokens(&state), vec![CS_TOKEN_FLAG + cs]);
    }

    #[test]
    fn get_r_token_skips_spaces_and_accepts_a_control_sequence() {
        let mut state = state();
        state.set_source_text(" \\foo");

        state.get_r_token().unwrap();
        let cs = state.cs_table.lookup_str("foo").unwrap();
        assert_eq!(state.cur_cs, cs);
        assert_eq!(state.cur_tok, CS_TOKEN_FLAG + cs);
    }

    #[test]
    fn get_r_token_recovers_with_the_inaccessible_control_sequence() {
        let mut state = state();
        state.set_source_text(" x");

        state.get_r_token().unwrap();
        assert_eq!(state.cur_cs, FROZEN_PROTECTION_ID);
        assert_eq!(state.cur_tok, CS_TOKEN_FLAG + FROZEN_PROTECTION_ID);
        assert_eq!(token(&mut state), LETTER_TOKEN + i32::from(b'x'));
    }

    #[test]
    fn installs_the_complete_vendored_primitive_table() {
        let state = state();
        assert_eq!(state.cs_table.primitive_count(), 500);

        let pretolerance = state.cs_table.lookup_str("pretolerance").unwrap();
        assert_eq!(
            state.cs_table.command(pretolerance),
            CommandCode::AssignInt as u8
        );
        assert_eq!(
            state.cs_table.character(pretolerance),
            crate::state::INT_PAR_BASE as i32
        );
        assert_eq!(
            state.prim_lookup("pretolerance"),
            Some((
                CommandCode::AssignInt as u8,
                crate::state::INT_PAR_BASE as i32
            ))
        );

        let parindent = state.cs_table.lookup_str("parindent").unwrap();
        assert_eq!(
            state.cs_table.character(parindent),
            crate::state::DIMEN_PAR_BASE as i32
        );
        let thinmuskip = state.cs_table.lookup_str("thinmuskip").unwrap();
        assert_eq!(
            state.cs_table.command(thinmuskip),
            CommandCode::AssignMuGlue as u8
        );
        assert_eq!(
            state.cs_table.character(thinmuskip),
            crate::state::GLUE_PAR_BASE as i32 + 16
        );
        let everypar = state.cs_table.lookup_str("everypar").unwrap();
        assert_eq!(
            state.cs_table.character(everypar),
            (crate::state::LOCAL_BASE + crate::state::LOCAL_EVERY_PAR) as i32
        );
        let interlinepenalties = state.cs_table.lookup_str("interlinepenalties").unwrap();
        assert_eq!(
            state.cs_table.character(interlinepenalties),
            (crate::state::ETEX_PEN_BASE + crate::state::ETEX_PEN_INTER_LINE) as i32
        );

        assert_eq!(state.write_loc, state.cs_table.lookup_str("write").unwrap());
        assert_eq!(state.par_loc, state.cs_table.lookup_str("par").unwrap());
        assert_eq!(
            state.cs_table.command(FROZEN_RIGHT_ID),
            CommandCode::LeftRight as u8
        );
        assert_eq!(state.cs_table.character(FROZEN_CR_ID), CR_CODE);
        assert_eq!(
            state.cs_table.command(FROZEN_NULL_FONT_ID),
            CommandCode::SetFont as u8
        );
    }

    #[test]
    fn primitive_lookup_preserves_the_original_meaning_after_redefinition() {
        let mut state = state();
        state.define_cs("relax", CommandCode::Call, TEX_NULL);
        assert_eq!(
            state.prim_lookup("relax"),
            Some((CommandCode::Relax as u8, TOO_BIG_USV))
        );
        assert_eq!(state.prim_lookup("notaprimitive"), None);
    }
}

#[cfg(test)]
mod cs_caret_tests {
    use super::*;
    use crate::state::TeXEngineState;

    #[test]
    fn caret_expansion_rewrites_names_inside_control_sequences() {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state.set_source_text("\\^^41bc x");

        state.get_token().unwrap();
        let cs = state.cs_table.lookup_str("Abc").unwrap();
        assert_eq!(state.cur_tok, CS_TOKEN_FLAG + cs);
        state.get_token().unwrap();
        assert_eq!(state.cur_tok, 11 * MAX_CHAR_VAL + i32::from(b'x'));
    }
}
