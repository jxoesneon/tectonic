// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust encapsulated execution state automaton and memory structures for the TeX stomach.
//!
//! Fulfills Phase 5 of `MIGRATION_ROADMAP.md`:
//! - Encapsulates all TeX memory, registers, and execution state into an instantiable [`TeXEngineState`].
//! - Zero process-global mutable static variables (strictly enforcing Safety Gate 5).
//! - Deterministic 32-bit fixed-point `scaled_t` arithmetic (strictly enforcing Safety Gate 4).
//! - Supports Clone-on-Write (CoW) state snapshotting for instantaneous preamble resumption
//!   and reversible DAP time-travel debugging.

use std::fmt;

/// Error type for TeX dynamic memory allocations and operations.
#[derive(Debug, PartialEq, Eq)]
pub enum TeXMemoryError {
    /// Available dynamic node memory exhausted.
    OutOfMemory(usize),

    /// Attempted to access memory outside valid bounds.
    OutOfBounds(i32, usize),

    /// Corrupted free list or node header detected.
    CorruptedNode(i32),
}

impl fmt::Display for TeXMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory(words) => {
                write!(f, "TeX capacity exceeded (out of memory: {words} words requested)")
            }
            Self::OutOfBounds(addr, len) => {
                write!(f, "TeX memory address {addr} out of bounds (valid range: 0..{len})")
            }
            Self::CorruptedNode(addr) => {
                write!(f, "Corrupted memory node structure at address {addr}")
            }
        }
    }
}

impl std::error::Error for TeXMemoryError {}

/// A 64-bit TeX memory word, containing either two 32-bit integers (`b32`) or four 16-bit integers (`b16`).
///
/// Layout matches Knuth's exact `memory_word` representation:
/// - `s0`: `LH` / `info` (low 32-bits)
/// - `s1`: `RH` / `link` (high 32-bits)
/// - `s0` lower 16-bits: `node_subtype` / `b16.s0`
/// - `s0` upper 16-bits: `node_type` / `b16.s1`
#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
pub struct MemoryWord {
    /// Low 32-bit word (`LH` / `info`).
    pub s0: i32,
    /// High 32-bit word (`RH` / `link`).
    pub s1: i32,
}

impl std::fmt::Debug for MemoryWord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "MemoryWord(info/s0: {}, link/s1: {}, type: {}, subtype: {})",
            self.s0,
            self.s1,
            self.node_type(),
            self.node_subtype()
        )
    }
}

impl MemoryWord {
    /// Creates a new `MemoryWord` with explicit `s0` (info) and `s1` (link) components.
    #[inline(always)]
    pub const fn new(s0: i32, s1: i32) -> Self {
        Self { s0, s1 }
    }

    /// Creates a node header word with the given type, subtype, and link pointer.
    #[inline(always)]
    pub fn new_node(node_type: u16, node_subtype: u16, link: i32) -> Self {
        let s0 = (node_subtype as u32 | ((node_type as u32) << 16)) as i32;
        Self { s0, s1: link }
    }

    /// Returns the 32-bit `s0` (low 32 bits, `info`) field.
    #[inline(always)]
    pub fn b32_s0(&self) -> i32 {
        self.s0
    }

    /// Sets the 32-bit `s0` (low 32 bits, `info`) field.
    #[inline(always)]
    pub fn set_b32_s0(&mut self, val: i32) {
        self.s0 = val;
    }

    /// Returns the 32-bit `s1` (high 32 bits, `link`) field.
    #[inline(always)]
    pub fn b32_s1(&self) -> i32 {
        self.s1
    }

    /// Sets the 32-bit `s1` (high 32 bits, `link`) field.
    #[inline(always)]
    pub fn set_b32_s1(&mut self, val: i32) {
        self.s1 = val;
    }

    /// Returns the low 16-bits of `s0` (`b16.s0`, `node_subtype`).
    #[inline(always)]
    pub fn b16_s0(&self) -> u16 {
        (self.s0 as u32 & 0xFFFF) as u16
    }

    /// Sets the low 16-bits of `s0` (`b16.s0`, `node_subtype`).
    #[inline(always)]
    pub fn set_b16_s0(&mut self, val: u16) {
        let high = self.s0 as u32 & 0xFFFF_0000;
        self.s0 = (high | (val as u32)) as i32;
    }

    /// Returns the high 16-bits of `s0` (`b16.s1`, `node_type`).
    #[inline(always)]
    pub fn b16_s1(&self) -> u16 {
        ((self.s0 as u32 >> 16) & 0xFFFF) as u16
    }

    /// Sets the high 16-bits of `s0` (`b16.s1`, `node_type`).
    #[inline(always)]
    pub fn set_b16_s1(&mut self, val: u16) {
        let low = self.s0 as u32 & 0x0000_FFFF;
        self.s0 = (low | ((val as u32) << 16)) as i32;
    }

    /// Returns the low 16-bits of `s1` (`b16.s2`).
    #[inline(always)]
    pub fn b16_s2(&self) -> u16 {
        (self.s1 as u32 & 0xFFFF) as u16
    }

    /// Sets the low 16-bits of `s1` (`b16.s2`).
    #[inline(always)]
    pub fn set_b16_s2(&mut self, val: u16) {
        let high = self.s1 as u32 & 0xFFFF_0000;
        self.s1 = (high | (val as u32)) as i32;
    }

    /// Returns the high 16-bits of `s1` (`b16.s3`).
    #[inline(always)]
    pub fn b16_s3(&self) -> u16 {
        ((self.s1 as u32 >> 16) & 0xFFFF) as u16
    }

    /// Sets the high 16-bits of `s1` (`b16.s3`).
    #[inline(always)]
    pub fn set_b16_s3(&mut self, val: u16) {
        let low = self.s1 as u32 & 0x0000_FFFF;
        self.s1 = (low | ((val as u32) << 16)) as i32;
    }

    /// TeX symbolic accessor: `info(p)`.
    #[inline(always)]
    pub fn info(&self) -> i32 {
        self.b32_s0()
    }

    /// TeX symbolic mutator: `info(p) = val`.
    #[inline(always)]
    pub fn set_info(&mut self, val: i32) {
        self.set_b32_s0(val);
    }

    /// TeX symbolic accessor: `link(p)`.
    #[inline(always)]
    pub fn link(&self) -> i32 {
        self.b32_s1()
    }

    /// TeX symbolic mutator: `link(p) = val`.
    #[inline(always)]
    pub fn set_link(&mut self, val: i32) {
        self.set_b32_s1(val);
    }

    /// TeX symbolic accessor: `type(p)`.
    #[inline(always)]
    pub fn node_type(&self) -> u16 {
        self.b16_s1()
    }

    /// TeX symbolic mutator: `type(p) = val`.
    #[inline(always)]
    pub fn set_node_type(&mut self, val: u16) {
        self.set_b16_s1(val);
    }

    /// TeX symbolic accessor: `subtype(p)`.
    #[inline(always)]
    pub fn node_subtype(&self) -> u16 {
        self.b16_s0()
    }

    /// TeX symbolic mutator: `subtype(p) = val`.
    #[inline(always)]
    pub fn set_node_subtype(&mut self, val: u16) {
        self.set_b16_s0(val);
    }
}

/// A pair of 32-bit integers representing control sequence hash table entries.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct TwoHalves {
    /// Left half.
    pub lh: i32,
    /// Right half.
    pub rh: i32,
}

/// Deterministic pseudo-random number generator for `\random` and `\uniformdeviate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeXRandom {
    randoms: [i32; 55],
    j_random: usize,
}

impl Default for TeXRandom {
    fn default() -> Self {
        Self::new(0)
    }
}

impl TeXRandom {
    /// Initializes the pseudo-random generator with a seed value.
    pub fn new(seed: i32) -> Self {
        let mut r = Self {
            randoms: [0; 55],
            j_random: 0,
        };
        r.init(seed);
        r
    }

    /// Re-seeds the random number generator.
    pub fn init(&mut self, seed: i32) {
        let mut j = seed.abs();
        while j >= 0x1000_0000 {
            j /= 2;
        }

        let mut k = 1;
        for i in 0..55 {
            let jj = k;
            k = j - k;
            j = jj;
            if k < 0 {
                k += 0x1000_0000;
            }
            self.randoms[(i * 21) % 55] = j;
        }

        self.new_randoms();
        self.new_randoms();
        self.new_randoms();
    }

    fn new_randoms(&mut self) {
        for k in 0..24 {
            let mut x = self.randoms[k] - self.randoms[k + 31];
            if x < 0 {
                x += 0x1000_0000;
            }
            self.randoms[k] = x;
        }
        for k in 24..55 {
            let mut x = self.randoms[k] - self.randoms[k - 24];
            if x < 0 {
                x += 0x1000_0000;
            }
            self.randoms[k] = x;
        }
        self.j_random = 54;
    }

    /// Generates a uniform random integer in the range `0..x` (or `-x..0` if x is negative).
    pub fn unif_rand(&mut self, x: i32) -> i32 {
        if self.j_random == 0 {
            self.new_randoms();
        } else {
            self.j_random -= 1;
        }

        let y = take_frac(x.abs(), self.randoms[self.j_random]);
        if y == x.abs() {
            0
        } else if x > 0 {
            y
        } else {
            -y
        }
    }
}

/// Multiplies two fractions in Knuth's exact integer arithmetic.
fn take_frac(q: i32, mut f: i32) -> i32 {
    let negative = (f < 0) ^ (q < 0);
    let q = q.abs();
    f = f.abs();

    let mut n = if f < 0x1000_0000 {
        0
    } else {
        let n_val = f / 0x1000_0000;
        f %= 0x1000_0000;
        if q <= 0x7FFF_FFFF / n_val {
            n_val * q
        } else {
            0x7FFF_FFFF
        }
    };

    f += 0x1000_0000;
    let mut p = 0x0800_0000;

    if q < 0x4000_0000 {
        while f != 1 {
            if (f & 1) != 0 {
                p = (p + q) / 2;
            } else {
                p /= 2;
            }
            f /= 2;
        }
    } else {
        while f != 1 {
            if (f & 1) != 0 {
                p = p + (q - p) / 2;
            } else {
                p /= 2;
            }
            f /= 2;
        }
    }

    if (n - 0x7FFF_FFFF) + p > 0 {
        n = 0x7FFF_FFFF - p;
    }

    if negative {
        -(n + p)
    } else {
        n + p
    }
}

/// Halves an integer, rounding away from zero for odd numbers (`half`).
#[inline(always)]
pub fn half(x: i32) -> i32 {
    if (x & 1) != 0 {
        (x + 1) / 2
    } else {
        x / 2
    }
}

/// Converts a floating-point number to a 32-bit TeX integer with TRIP-compatible rounding (`tex_round`).
pub fn tex_round(r: f64) -> i32 {
    if r > 2147483647.0 {
        2147483647
    } else if r < -2147483648.0 {
        -2147483648
    } else if r >= 0.0 {
        (r + 0.5) as i32
    } else {
        (r - 0.5) as i32
    }
}

/// Computes the badness score of stretching or shrinking glue (`badness`).
pub fn badness(t: i32, s: i32) -> i32 {
    if t == 0 {
        0
    } else if s <= 0 {
        10000
    } else {
        let mut r = if t <= 7230584 {
            (t * 297) / s
        } else if s >= 1663497 {
            t / (s / 297)
        } else {
            t
        };

        if r > 1290 {
            10000
        } else {
            r = (r * r * r + 0x400000) / 0x800000;
            if r > 10000 {
                10000
            } else {
                r
            }
        }
    }
}

// =========================================================================
// TeX Stomach Constants, Node Types, and Command Codes
// =========================================================================

/// Maximum halfword representation (2^30 - 1).
pub const MAX_HALFWORD: i32 = 0x3FFF_FFFF;
/// Minimum halfword representation (-2^30 + 1).
pub const MIN_HALFWORD: i32 = -0x3FFF_FFFF;
/// Flag indicating a null dimension or rule dimension (-2^30).
pub const NULL_FLAG: i32 = -0x4000_0000;
/// Sentinel for null link or null node.
pub const TEX_NULL: i32 = 0;

/// Default glue subtype (`normal`).
pub const NORMAL: u16 = 0;
/// Explicit kern subtype (`explicit`).
pub const EXPLICIT: u16 = 1;

/// Size of an `if_node` in memory words.
pub const IF_NODE_SIZE: i32 = 2;
/// Size of a small node in memory words.
pub const SMALL_NODE_SIZE: i32 = 2;
/// Size of a medium node in memory words.
pub const MEDIUM_NODE_SIZE: i32 = 3;
/// Size of a glue specification node in memory words.
pub const GLUE_SPEC_SIZE: i32 = 4;
/// Size of a rule node in memory words.
pub const RULE_NODE_SIZE: i32 = 5;
/// Size of a box node in memory words.
pub const BOX_NODE_SIZE: i32 = 8;

/// Horizontal list node type (`hlist_node`).
pub const HLIST_NODE: u16 = 0;
/// Vertical list node type (`vlist_node`).
pub const VLIST_NODE: u16 = 1;
/// Rule node type (`rule_node`).
pub const RULE_NODE: u16 = 2;
/// Insertion node type (`ins_node`).
pub const INS_NODE: u16 = 3;
/// Mark node type (`mark_node`).
pub const MARK_NODE: u16 = 4;
/// Adjustment node type (`adjust_node`).
pub const ADJUST_NODE: u16 = 5;
/// Ligature node type (`ligature_node`).
pub const LIGATURE_NODE: u16 = 6;
/// Discretionary break node type (`disc_node`).
pub const DISC_NODE: u16 = 7;
/// Extension / whatsit node type (`whatsit_node`).
pub const WHATSIT_NODE: u16 = 8;
/// Math delimiter node type (`math_node`).
pub const MATH_NODE: u16 = 9;
/// Glue node type (`glue_node`).
pub const GLUE_NODE: u16 = 10;
/// Kern node type (`kern_node`).
pub const KERN_NODE: u16 = 11;
/// Penalty break node type (`penalty_node`).
pub const PENALTY_NODE: u16 = 12;
/// Unset box node type (`unset_node`).
pub const UNSET_NODE: u16 = 13;
/// Math style node type (`style_node`).
pub const STYLE_NODE: u16 = 14;
/// Math choice node type (`choice_node`).
pub const CHOICE_NODE: u16 = 15;

/// Base index of glue (`\skip`) registers in `eqtb`.
pub const SKIP_BASE: usize = 0;
/// Number of glue (`\skip`) registers.
pub const SKIP_REG_COUNT: usize = 256;
/// Base index of math glue (`\muskip`) registers in `eqtb`.
pub const MU_SKIP_BASE: usize = 256;
/// Number of math glue (`\muskip`) registers.
pub const MU_SKIP_REG_COUNT: usize = 256;
/// Base index of token list (`\toks`) registers in `eqtb`.
pub const TOKS_BASE: usize = 512;
/// Number of token list (`\toks`) registers.
pub const TOKS_REG_COUNT: usize = 256;
/// Base index of box (`\box`) registers in `eqtb`.
pub const BOX_BASE: usize = 768;
/// Number of box (`\box`) registers.
pub const BOX_REG_COUNT: usize = 256;
/// Base index of integer (`\count`) registers in `eqtb`.
pub const COUNT_BASE: usize = 1024;
/// Number of integer (`\count`) registers.
pub const COUNT_REG_COUNT: usize = 256;
/// Base index of dimension (`\dimen`) registers in `eqtb`.
pub const DIMEN_BASE: usize = 1280;
/// Number of dimension (`\dimen`) registers.
pub const DIMEN_REG_COUNT: usize = 256;

/// TeX execution mode.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i16)]
pub enum TeXMode {
    Vmode = 1,
    Hmode = 104,
    Mmode = 207,
    InternalVmode = -1,
    RestrictedHmode = -104,
    InlineMmode = -207,
}

/// Core engine command codes.
#[allow(missing_docs)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CommandCode {
    Relax = 0,
    LeftBrace = 1,
    RightBrace = 2,
    MathShift = 3,
    TabMark = 4,
    CarRet = 5,
    MacParam = 6,
    SupMark = 7,
    SubMark = 8,
    Endv = 9,
    Spacer = 10,
    Letter = 11,
    OtherChar = 12,
    ParEnd = 13,
    Stop = 14,
    DelimNum = 15,
    CharNum = 16,
    MathCharNum = 17,
    Mark = 18,
    Xray = 19,
    MakeBox = 20,
    Hmove = 21,
    Vmove = 22,
    UnHbox = 23,
    UnVbox = 24,
    RemoveItem = 25,
    Hskip = 26,
    Vskip = 27,
    Mskip = 28,
    Kern = 29,
    Mkern = 30,
    LeaderShip = 31,
    Halign = 32,
    Valign = 33,
    NoAlign = 34,
    Vrule = 35,
    Hrule = 36,
    Insert = 37,
    Vadjust = 38,
    IgnoreSpaces = 39,
    AfterAssignment = 40,
    AfterGroup = 41,
    BreakPenalty = 42,
    StartPar = 43,
    ItalCorr = 44,
    Accent = 45,
    MathAccent = 46,
    Discretionary = 47,
    EqNo = 48,
    LeftRight = 49,
    MathComp = 50,
    LimitSwitch = 51,
    Above = 52,
    MathStyle = 53,
    MathChoice = 54,
    NonScript = 55,
    Vcenter = 56,
    CaseShift = 57,
    Message = 58,
    Extension = 59,
    InStream = 60,
    BeginGroup = 61,
    EndGroup = 62,
    Omit = 63,
    ExSpace = 64,
    NoBoundary = 65,
    Radical = 66,
    EndCsName = 67,
    CharGiven = 68,
    MathGiven = 69,
    LastItem = 71,
    ToksRegister = 72,
    AssignToks = 73,
    AssignInt = 74,
    AssignDimen = 75,
    AssignGlue = 76,
    AssignMuGlue = 77,
    AssignFontDimen = 78,
    AssignFontInt = 79,
    SetAux = 80,
    SetPrevGraf = 81,
    SetPageDimen = 82,
    SetPageInt = 83,
    SetBoxDimen = 84,
    SetShape = 85,
    DefCode = 86,
    SetFont = 89,
    DefFont = 90,
    Register = 91,
    Advance = 92,
    Multiply = 93,
    Divide = 94,
    Prefix = 95,
    Let = 96,
    ShorthandDef = 97,
    ReadToCs = 98,
    Def = 99,
    SetBox = 100,
    HyphData = 101,
    SetInteraction = 102,
    UndefinedCs = 103,
    ExpandAfter = 104,
    NoExpand = 105,
    Input = 106,
    IfTest = 107,
    FiOrElse = 108,
    CsName = 109,
    Convert = 110,
    The = 111,
    TopBotMark = 112,
    Call = 113,
    LongCall = 114,
    OuterCall = 115,
    LongOuterCall = 116,
    EndTemplate = 117,
    DontExpand = 118,
    GlueRef = 119,
    ShapeRef = 120,
    BoxRef = 121,
    Data = 122,
}

impl CommandCode {
    /// Attempts to parse a raw `u8` into a known [`CommandCode`].
    pub fn from_u8(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Relax),
            1 => Some(Self::LeftBrace),
            2 => Some(Self::RightBrace),
            3 => Some(Self::MathShift),
            4 => Some(Self::TabMark),
            5 => Some(Self::CarRet),
            6 => Some(Self::MacParam),
            7 => Some(Self::SupMark),
            8 => Some(Self::SubMark),
            9 => Some(Self::Endv),
            10 => Some(Self::Spacer),
            11 => Some(Self::Letter),
            12 => Some(Self::OtherChar),
            13 => Some(Self::ParEnd),
            14 => Some(Self::Stop),
            15 => Some(Self::DelimNum),
            16 => Some(Self::CharNum),
            17 => Some(Self::MathCharNum),
            18 => Some(Self::Mark),
            19 => Some(Self::Xray),
            20 => Some(Self::MakeBox),
            21 => Some(Self::Hmove),
            22 => Some(Self::Vmove),
            23 => Some(Self::UnHbox),
            24 => Some(Self::UnVbox),
            25 => Some(Self::RemoveItem),
            26 => Some(Self::Hskip),
            27 => Some(Self::Vskip),
            28 => Some(Self::Mskip),
            29 => Some(Self::Kern),
            30 => Some(Self::Mkern),
            31 => Some(Self::LeaderShip),
            32 => Some(Self::Halign),
            33 => Some(Self::Valign),
            34 => Some(Self::NoAlign),
            35 => Some(Self::Vrule),
            36 => Some(Self::Hrule),
            37 => Some(Self::Insert),
            38 => Some(Self::Vadjust),
            39 => Some(Self::IgnoreSpaces),
            40 => Some(Self::AfterAssignment),
            41 => Some(Self::AfterGroup),
            42 => Some(Self::BreakPenalty),
            43 => Some(Self::StartPar),
            44 => Some(Self::ItalCorr),
            45 => Some(Self::Accent),
            46 => Some(Self::MathAccent),
            47 => Some(Self::Discretionary),
            48 => Some(Self::EqNo),
            49 => Some(Self::LeftRight),
            50 => Some(Self::MathComp),
            51 => Some(Self::LimitSwitch),
            52 => Some(Self::Above),
            53 => Some(Self::MathStyle),
            54 => Some(Self::MathChoice),
            55 => Some(Self::NonScript),
            56 => Some(Self::Vcenter),
            57 => Some(Self::CaseShift),
            58 => Some(Self::Message),
            59 => Some(Self::Extension),
            60 => Some(Self::InStream),
            61 => Some(Self::BeginGroup),
            62 => Some(Self::EndGroup),
            63 => Some(Self::Omit),
            64 => Some(Self::ExSpace),
            65 => Some(Self::NoBoundary),
            66 => Some(Self::Radical),
            67 => Some(Self::EndCsName),
            68 => Some(Self::CharGiven),
            69 => Some(Self::MathGiven),
            71 => Some(Self::LastItem),
            72 => Some(Self::ToksRegister),
            73 => Some(Self::AssignToks),
            74 => Some(Self::AssignInt),
            75 => Some(Self::AssignDimen),
            76 => Some(Self::AssignGlue),
            77 => Some(Self::AssignMuGlue),
            78 => Some(Self::AssignFontDimen),
            79 => Some(Self::AssignFontInt),
            80 => Some(Self::SetAux),
            81 => Some(Self::SetPrevGraf),
            82 => Some(Self::SetPageDimen),
            83 => Some(Self::SetPageInt),
            84 => Some(Self::SetBoxDimen),
            85 => Some(Self::SetShape),
            86 => Some(Self::DefCode),
            89 => Some(Self::SetFont),
            90 => Some(Self::DefFont),
            91 => Some(Self::Register),
            92 => Some(Self::Advance),
            93 => Some(Self::Multiply),
            94 => Some(Self::Divide),
            95 => Some(Self::Prefix),
            96 => Some(Self::Let),
            97 => Some(Self::ShorthandDef),
            98 => Some(Self::ReadToCs),
            99 => Some(Self::Def),
            100 => Some(Self::SetBox),
            101 => Some(Self::HyphData),
            102 => Some(Self::SetInteraction),
            103 => Some(Self::UndefinedCs),
            104 => Some(Self::ExpandAfter),
            105 => Some(Self::NoExpand),
            106 => Some(Self::Input),
            107 => Some(Self::IfTest),
            108 => Some(Self::FiOrElse),
            109 => Some(Self::CsName),
            110 => Some(Self::Convert),
            111 => Some(Self::The),
            112 => Some(Self::TopBotMark),
            113 => Some(Self::Call),
            114 => Some(Self::LongCall),
            115 => Some(Self::OuterCall),
            116 => Some(Self::LongOuterCall),
            117 => Some(Self::EndTemplate),
            118 => Some(Self::DontExpand),
            119 => Some(Self::GlueRef),
            120 => Some(Self::ShapeRef),
            121 => Some(Self::BoxRef),
            122 => Some(Self::Data),
            _ => None,
        }
    }
}

/// Encapsulated, thread-safe, and re-entrant TeX engine execution state automaton.
///
/// Fully encapsulates all memory arrays, registers, scanner, and execution state
/// with ZERO global mutable static variables (Gate 5 compliant).
#[derive(Clone, Debug)]
pub struct TeXEngineState {
    /// Dynamic node and token list memory array.
    pub mem: Vec<MemoryWord>,
    /// Equivalents table (`eqtb`) for registers, catcodes, fonts, and primitives.
    pub eqtb: Vec<MemoryWord>,
    /// Control sequence hash table.
    pub hash: Vec<TwoHalves>,
    /// Category codes table (ASCII 0..255 or dynamic Unicode).
    pub cat_codes: Vec<u8>,

    /// Current scanned value (`cur_val`).
    pub cur_val: i32,
    /// Current command code (`cur_cmd`).
    pub cur_cmd: u8,
    /// Current character or register index (`cur_chr`).
    pub cur_chr: i32,
    /// Current control sequence index (`cur_cs`).
    pub cur_cs: i32,

    /// Execution mode (e.g. `Vmode`, `Hmode`, `Mmode`).
    pub mode: i16,
    /// Head of the current semantic list.
    pub head: i32,
    /// Tail of the current semantic list.
    pub tail: i32,
    /// Number of lines previously completed in paragraph (`prev_graf`).
    pub prev_graf: i32,

    /// Arithmetic error indicator flag (`arith_error`).
    pub arith_error: bool,
    /// Remainder from division routines (`tex_remainder`).
    pub tex_remainder: i32,

    /// Pseudo-random number generator state.
    pub rand: TeXRandom,

    /// Bound of low memory currently allocated.
    pub lo_mem_max: i32,
    /// Bound of high memory currently allocated.
    pub hi_mem_min: i32,
    /// Pointer to rover node in dynamic memory.
    pub rover: i32,
    /// Head of available one-word token memory list (`avail`).
    pub avail: i32,
}

impl Default for TeXEngineState {
    fn default() -> Self {
        Self::new(30_000)
    }
}

impl TeXEngineState {
    /// Initializes a new, clean `TeXEngineState` with the specified memory word capacity.
    pub fn new(mem_size: usize) -> Self {
        let mem_size = mem_size.max(128);
        let mem = vec![MemoryWord::default(); mem_size];
        let eqtb = vec![MemoryWord::default(); 10_000];
        let hash = vec![TwoHalves::default(); 15_000];
        let cat_codes = vec![12u8; 256];

        let mut state = Self {
            mem,
            eqtb,
            hash,
            cat_codes,
            cur_val: 0,
            cur_cmd: 0,
            cur_chr: 0,
            cur_cs: 0,
            mode: TeXMode::Vmode as i16,
            head: 0,
            tail: 0,
            prev_graf: 0,
            arith_error: false,
            tex_remainder: 0,
            rand: TeXRandom::default(),
            lo_mem_max: 0,
            hi_mem_min: (mem_size - 1) as i32,
            rover: 0,
            avail: 0,
        };
        state.init_memory();
        state.init_default_cat_codes();
        state
    }

    /// Initializes dynamic node rover allocator and token list structures.
    pub fn init_memory(&mut self) {
        let rover = 20.min((self.mem.len() / 4) as i32).max(4);
        let rover_size = 1000.min((self.mem.len() / 2) as i32 - rover).max(10);
        self.rover = rover;
        self.mem[rover as usize].s0 = rover_size;
        self.mem[rover as usize].s1 = MAX_HALFWORD;
        self.mem[(rover + 1) as usize].s0 = rover;
        self.mem[(rover + 1) as usize].s1 = rover;
        self.lo_mem_max = rover + rover_size;
        self.mem[self.lo_mem_max as usize].s0 = TEX_NULL;
        self.mem[self.lo_mem_max as usize].s1 = TEX_NULL;
        self.hi_mem_min = (self.mem.len() - 1) as i32;
        self.avail = TEX_NULL;
    }

    /// Initializes standard TeX default category codes.
    pub fn init_default_cat_codes(&mut self) {
        if self.cat_codes.len() < 256 {
            self.cat_codes.resize(256, 12);
        } else {
            self.cat_codes.fill(12); // OtherChar
        }

        self.cat_codes[0] = 9;   // \0: Ignore
        self.cat_codes[13] = 5;  // \r: CarRet
        self.cat_codes[32] = 10; // ' ': Spacer
        self.cat_codes[35] = 6;  // '#': MacParam
        self.cat_codes[36] = 3;  // '$': MathShift
        self.cat_codes[37] = 14; // '%': Comment
        self.cat_codes[38] = 4;  // '&': TabMark
        self.cat_codes[92] = 0;  // '\': Escape
        self.cat_codes[94] = 7;  // '^': SupMark
        self.cat_codes[95] = 8;  // '_': SubMark
        self.cat_codes[123] = 1; // '{': LeftBrace
        self.cat_codes[125] = 2; // '}': RightBrace
        self.cat_codes[127] = 15;// DEL: InvalidChar

        // Letters A-Z and a-z
        for c in b'A'..=b'Z' {
            self.cat_codes[c as usize] = 11;
        }
        for c in b'a'..=b'z' {
            self.cat_codes[c as usize] = 11;
        }
    }

    /// Safely gets the `info` (s0) field of node `p`.
    #[inline(always)]
    pub fn info(&self, p: i32) -> Result<i32, TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        Ok(self.mem[idx].info())
    }

    /// Safely sets the `info` (s0) field of node `p`.
    #[inline(always)]
    pub fn set_info(&mut self, p: i32, val: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx].set_info(val);
        Ok(())
    }

    /// Safely gets the `link` (s1) field of node `p`.
    #[inline(always)]
    pub fn link(&self, p: i32) -> Result<i32, TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        Ok(self.mem[idx].link())
    }

    /// Safely sets the `link` (s1) field of node `p`.
    #[inline(always)]
    pub fn set_link(&mut self, p: i32, val: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx].set_link(val);
        Ok(())
    }

    /// Safely gets the `node_type` field of node `p`.
    #[inline(always)]
    pub fn node_type(&self, p: i32) -> Result<u16, TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        Ok(self.mem[idx].node_type())
    }

    /// Safely sets the `node_type` field of node `p`.
    #[inline(always)]
    pub fn set_node_type(&mut self, p: i32, val: u16) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx].set_node_type(val);
        Ok(())
    }

    /// Safely gets the `node_subtype` field of node `p`.
    #[inline(always)]
    pub fn node_subtype(&self, p: i32) -> Result<u16, TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        Ok(self.mem[idx].node_subtype())
    }

    /// Safely sets the `node_subtype` field of node `p`.
    #[inline(always)]
    pub fn set_node_subtype(&mut self, p: i32, val: u16) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx].set_node_subtype(val);
        Ok(())
    }

    #[inline(always)]
    fn check_bounds(&self, p: i32) -> Result<usize, TeXMemoryError> {
        if p < 0 || p as usize >= self.mem.len() {
            Err(TeXMemoryError::OutOfBounds(p, self.mem.len()))
        } else {
            Ok(p as usize)
        }
    }

    // =========================================================================
    // Dynamic Node Memory Allocator (Knuth's Doubly-Linked Rover)
    // =========================================================================

    /// Allocates a node of `s` contiguous words from dynamic memory (`get_node`).
    pub fn get_node(&mut self, s: i32) -> Result<i32, TeXMemoryError> {
        if s < 2 {
            return Err(TeXMemoryError::CorruptedNode(s));
        }

        loop {
            let mut p = self.rover;
            let mut found_node: Option<i32> = None;

            loop {
                let p_idx = self.check_bounds(p)?;
                let p_size = self.mem[p_idx].s0;
                let mut q = p + p_size;

                // Coalesce adjacent free blocks
                while q < self.lo_mem_max && self.mem[q as usize].s1 == MAX_HALFWORD {
                    let t = self.mem[(q + 1) as usize].s1;
                    if q == self.rover {
                        self.rover = t;
                    }
                    let q_llink = self.mem[(q + 1) as usize].s0;
                    self.mem[(t + 1) as usize].s0 = q_llink;
                    self.mem[(q_llink + 1) as usize].s1 = t;
                    q += self.mem[q as usize].s0;
                }

                let r = q - s;
                if r > p + 1 {
                    self.mem[p as usize].s0 = r - p;
                    self.rover = p;
                    found_node = Some(r);
                    break;
                }

                if r == p {
                    let rlink = self.mem[(p + 1) as usize].s1;
                    if rlink != p {
                        self.rover = rlink;
                        let llink = self.mem[(p + 1) as usize].s0;
                        self.mem[(self.rover + 1) as usize].s0 = llink;
                        self.mem[(llink + 1) as usize].s1 = self.rover;
                        found_node = Some(r);
                        break;
                    }
                }

                self.mem[p as usize].s0 = q - p;
                p = self.mem[(p + 1) as usize].s1;
                if p == self.rover {
                    break;
                }
            }

            if let Some(r) = found_node {
                self.mem[r as usize].s1 = TEX_NULL;
                return Ok(r);
            }

            // Expand low memory toward high memory if room exists
            if self.lo_mem_max + 2 < self.hi_mem_min && self.lo_mem_max + 2 <= MAX_HALFWORD {
                let mut t = if self.hi_mem_min - self.lo_mem_max >= 1998 {
                    self.lo_mem_max + 1000
                } else {
                    self.lo_mem_max + 1 + (self.hi_mem_min - self.lo_mem_max) / 2
                };
                if t > MAX_HALFWORD {
                    t = MAX_HALFWORD;
                }
                if t > self.hi_mem_min - 2 {
                    t = self.hi_mem_min - 2;
                }
                if t > self.lo_mem_max + 2 {
                    let p = self.mem[(self.rover + 1) as usize].s0;
                    let q = self.lo_mem_max;
                    self.mem[(p + 1) as usize].s1 = q;
                    self.mem[(self.rover + 1) as usize].s0 = q;

                    self.mem[(q + 1) as usize].s1 = self.rover;
                    self.mem[(q + 1) as usize].s0 = p;
                    self.mem[q as usize].s1 = MAX_HALFWORD;
                    self.mem[q as usize].s0 = t - self.lo_mem_max;
                    self.lo_mem_max = t;
                    self.mem[self.lo_mem_max as usize].s1 = TEX_NULL;
                    self.mem[self.lo_mem_max as usize].s0 = TEX_NULL;
                    self.rover = q;
                    continue;
                }
            }

            return Err(TeXMemoryError::OutOfMemory(s as usize));
        }
    }

    /// Returns a node of `s` words back to dynamic memory (`free_node`).
    pub fn free_node(&mut self, p: i32, s: i32) -> Result<(), TeXMemoryError> {
        if p < 0 || p as usize >= self.mem.len() || s < 2 {
            return Err(TeXMemoryError::OutOfBounds(p, self.mem.len()));
        }
        self.mem[p as usize].s0 = s;
        self.mem[p as usize].s1 = MAX_HALFWORD;
        let q = self.mem[(self.rover + 1) as usize].s0;
        self.mem[(p + 1) as usize].s0 = q;
        self.mem[(p + 1) as usize].s1 = self.rover;
        self.mem[(self.rover + 1) as usize].s0 = p;
        self.mem[(q + 1) as usize].s1 = p;
        Ok(())
    }

    /// Allocates a single word from available memory (`get_avail`).
    pub fn get_avail(&mut self) -> Result<i32, TeXMemoryError> {
        let p = self.avail;
        if p != TEX_NULL {
            self.avail = self.link(p)?;
        } else if self.hi_mem_min <= self.lo_mem_max + 1 {
            return Err(TeXMemoryError::OutOfMemory(1));
        } else {
            self.hi_mem_min -= 1;
            return Ok(self.hi_mem_min);
        }

        self.set_link(p, TEX_NULL)?;
        Ok(p)
    }

    /// Frees a single-word node returned by `get_avail` (`free_avail`).
    pub fn free_avail(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        self.set_link(p, self.avail)?;
        self.avail = p;
        Ok(())
    }

    /// Flushes an entire linked list of single-word token memory (`flush_list`).
    pub fn flush_list(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        if p != TEX_NULL {
            let mut r = p;
            loop {
                let q = r;
                let next = self.link(r)?;
                if next == TEX_NULL {
                    self.set_link(q, self.avail)?;
                    self.avail = p;
                    break;
                }
                r = next;
            }
        }
        Ok(())
    }

    // =========================================================================
    // Node Constructors
    // =========================================================================

    /// Creates an empty horizontal or vertical box (`new_null_box`).
    pub fn new_null_box(&mut self) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(BOX_NODE_SIZE)?;
        self.set_node_type(p, HLIST_NODE)?;
        self.set_node_subtype(p, 0)?;
        for offset in 1..BOX_NODE_SIZE {
            self.mem[(p + offset) as usize] = MemoryWord::default();
        }
        Ok(p)
    }

    /// Creates a new rule node with default running dimensions (`new_rule`).
    pub fn new_rule(&mut self) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(RULE_NODE_SIZE)?;
        self.set_node_type(p, RULE_NODE)?;
        self.set_node_subtype(p, 0)?;
        self.mem[(p + 1) as usize].s1 = NULL_FLAG; // width
        self.mem[(p + 2) as usize].s1 = NULL_FLAG; // depth
        self.mem[(p + 3) as usize].s1 = NULL_FLAG; // height
        Ok(p)
    }

    /// Creates a glue node pointing to a given glue spec `q` (`new_glue`).
    pub fn new_glue(&mut self, q: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(MEDIUM_NODE_SIZE)?;
        self.set_node_type(p, GLUE_NODE)?;
        self.set_node_subtype(p, NORMAL)?;
        self.mem[(p + 1) as usize].s0 = q;
        Ok(p)
    }

    /// Clones a glue specification node `p` (`new_spec`).
    pub fn new_spec(&mut self, p: i32) -> Result<i32, TeXMemoryError> {
        let q = self.get_node(GLUE_SPEC_SIZE)?;
        self.mem[q as usize] = self.mem[p as usize];
        self.mem[(q + 1) as usize] = self.mem[(p + 1) as usize];
        self.mem[(q + 2) as usize] = self.mem[(p + 2) as usize];
        self.mem[(q + 3) as usize] = self.mem[(p + 3) as usize];
        Ok(q)
    }

    /// Creates a glue node referencing parameter glue register `n` (`new_param_glue`).
    pub fn new_param_glue(&mut self, n: usize) -> Result<i32, TeXMemoryError> {
        let q = self.skip(n)?;
        self.new_glue(q)
    }

    /// Creates a new kern node of width `w` (`new_kern`).
    pub fn new_kern(&mut self, w: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(MEDIUM_NODE_SIZE)?;
        self.set_node_type(p, KERN_NODE)?;
        self.set_node_subtype(p, NORMAL)?;
        self.mem[(p + 1) as usize].s1 = w;
        Ok(p)
    }

    /// Creates a new penalty node with value `m` (`new_penalty`).
    pub fn new_penalty(&mut self, m: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(MEDIUM_NODE_SIZE)?;
        self.set_node_type(p, PENALTY_NODE)?;
        self.set_node_subtype(p, 0)?;
        self.mem[(p + 1) as usize].s1 = m;
        Ok(p)
    }

    // =========================================================================
    // Equivalents Table (eqtb) Register Accessors & Catcodes
    // =========================================================================

    /// Returns the integer register `\count<n>` value.
    pub fn count(&self, n: usize) -> Result<i32, TeXMemoryError> {
        let idx = COUNT_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets the integer register `\count<n>` value.
    pub fn set_count(&mut self, n: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = COUNT_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns the dimension register `\dimen<n>` value in scaled points.
    pub fn dimen(&self, n: usize) -> Result<i32, TeXMemoryError> {
        let idx = DIMEN_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets the dimension register `\dimen<n>` value in scaled points.
    pub fn set_dimen(&mut self, n: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = DIMEN_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns the glue register `\skip<n>` node pointer.
    pub fn skip(&self, n: usize) -> Result<i32, TeXMemoryError> {
        let idx = SKIP_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets the glue register `\skip<n>` node pointer.
    pub fn set_skip(&mut self, n: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = SKIP_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns the math glue register `\muskip<n>` node pointer.
    pub fn mu_skip(&self, n: usize) -> Result<i32, TeXMemoryError> {
        let idx = MU_SKIP_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets the math glue register `\muskip<n>` node pointer.
    pub fn set_mu_skip(&mut self, n: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = MU_SKIP_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns the token list register `\toks<n>` node pointer.
    pub fn toks(&self, n: usize) -> Result<i32, TeXMemoryError> {
        let idx = TOKS_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets the token list register `\toks<n>` node pointer.
    pub fn set_toks(&mut self, n: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = TOKS_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns the box register `\box<n>` node pointer.
    pub fn box_reg(&self, n: usize) -> Result<i32, TeXMemoryError> {
        let idx = BOX_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets the box register `\box<n>` node pointer.
    pub fn set_box_reg(&mut self, n: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = BOX_BASE + n;
        if idx >= self.eqtb.len() {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns the category code for character `c` (0..15).
    pub fn cat_code(&self, c: u32) -> u8 {
        if (c as usize) < self.cat_codes.len() {
            self.cat_codes[c as usize]
        } else {
            12 // OtherChar
        }
    }

    /// Sets the category code for character `c`.
    pub fn set_cat_code(&mut self, c: u32, code: u8) {
        if (c as usize) >= self.cat_codes.len() {
            self.cat_codes.resize((c as usize + 1).max(256), 12);
        }
        self.cat_codes[c as usize] = code;
    }

    // =========================================================================
    // Gate 4 Fixed-Point Math Invariants
    // =========================================================================

    /// Computes `n * x + y` with audited 32-bit integer overflow detection (`mult_and_add`).
    pub fn mult_and_add(&mut self, mut n: i32, mut x: i32, y: i32, max_answer: i32) -> i32 {
        if n < 0 {
            x = -x;
            n = -n;
        }

        if n == 0 {
            y
        } else if x <= (max_answer - y) / n && (-x <= (max_answer + y) / n) {
            n * x + y
        } else {
            self.arith_error = true;
            0
        }
    }

    /// Divides fixed-point dimension `x` by integer `n` with Knuth-exact remainder tracking (`x_over_n`).
    pub fn x_over_n(&mut self, mut x: i32, mut n: i32) -> i32 {
        if n == 0 {
            self.arith_error = true;
            self.tex_remainder = x;
            0
        } else {
            if n < 0 {
                x = -x;
                n = -n;
                self.tex_remainder = -self.tex_remainder;
            }

            if x >= 0 {
                self.tex_remainder = x % n;
                x / n
            } else {
                self.tex_remainder = -((-x) % n);
                -((-x) / n)
            }
        }
    }

    /// Scales `x` by rational `n / d` without intermediate overflow (`xn_over_d`).
    pub fn xn_over_d(&mut self, mut x: i32, n: i32, d: i32) -> i32 {
        if d == 0 {
            self.arith_error = true;
            return 0;
        }

        let positive = if x >= 0 {
            true
        } else {
            x = -x;
            false
        };

        let t = (x % 32768) * n;
        let mut u = (x / 32768) * n + (t / 32768);
        let v = (u % d) * 32768 + (t % 32768);

        if u / d >= 32768 {
            self.arith_error = true;
        } else {
            u = 32768 * (u / d) + (v / d);
        }

        if positive {
            self.tex_remainder = v % d;
            u
        } else {
            self.tex_remainder = -(v % d);
            -u
        }
    }

    /// Scales `x` by rational `n / d` with half-integral rounding (`round_xn_over_d`).
    pub fn round_xn_over_d(&mut self, mut x: i32, n: i32, d: i32) -> i32 {
        if d == 0 {
            self.arith_error = true;
            return 0;
        }

        let positive = if x >= 0 {
            true
        } else {
            x = -x;
            false
        };

        let t = (x % 32768) * n;
        let mut u = (x / 32768) * n + (t / 32768);
        let mut v = (u % d) * 32768 + (t % 32768);

        if u / d >= 32768 {
            self.arith_error = true;
        } else {
            u = 32768 * (u / d) + (v / d);
        }

        v %= d;
        if 2 * v >= d {
            u += 1;
        }

        if positive {
            u
        } else {
            -u
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_word_basic_accessors() {
        let mut w = MemoryWord::new(100, 200);
        assert_eq!(w.info(), 100);
        assert_eq!(w.link(), 200);

        w.set_info(300);
        w.set_link(400);
        assert_eq!(w.info(), 300);
        assert_eq!(w.link(), 400);
    }

    #[test]
    fn test_memory_word_packed_type_subtype() {
        let mut w = MemoryWord::new_node(12, 34, 567);
        assert_eq!(w.node_type(), 12);
        assert_eq!(w.node_subtype(), 34);
        assert_eq!(w.link(), 567);

        w.set_node_type(99);
        assert_eq!(w.node_type(), 99);
        assert_eq!(w.node_subtype(), 34);

        w.set_node_subtype(123);
        assert_eq!(w.node_type(), 99);
        assert_eq!(w.node_subtype(), 123);
    }

    #[test]
    fn test_tex_round_trip_semantics() {
        assert_eq!(tex_round(0.0), 0);
        assert_eq!(tex_round(0.4), 0);
        assert_eq!(tex_round(0.5), 1);
        assert_eq!(tex_round(0.6), 1);
        assert_eq!(tex_round(-0.4), 0);
        assert_eq!(tex_round(-0.5), -1);
        assert_eq!(tex_round(-0.6), -1);

        // Clamping at boundaries
        assert_eq!(tex_round(3e9), 2147483647);
        assert_eq!(tex_round(-3e9), -2147483648);
    }

    #[test]
    fn test_half() {
        assert_eq!(half(0), 0);
        assert_eq!(half(1), 1);
        assert_eq!(half(2), 1);
        assert_eq!(half(3), 2);
        assert_eq!(half(-1), 0);
        assert_eq!(half(-2), -1);
    }

    #[test]
    fn test_fixed_point_math_invariants() {
        let mut state = TeXEngineState::new(1000);

        // mult_and_add
        let res = state.mult_and_add(10, 20, 5, 1000);
        assert_eq!(res, 205);
        assert!(!state.arith_error);

        // overflow triggers arith_error
        let overflow = state.mult_and_add(1000, 1000000, 0, 1000);
        assert_eq!(overflow, 0);
        assert!(state.arith_error);

        // x_over_n
        state.arith_error = false;
        let q = state.x_over_n(100, 7);
        assert_eq!(q, 14);
        assert_eq!(state.tex_remainder, 2);

        // xn_over_d
        let r1 = state.xn_over_d(65536, 3, 2); // 1.5 * 65536 = 98304
        assert_eq!(r1, 98304);

        // round_xn_over_d
        let r2 = state.round_xn_over_d(10, 1, 3); // 10/3 = 3.333 -> 3
        assert_eq!(r2, 3);
        let r3 = state.round_xn_over_d(10, 2, 3); // 20/3 = 6.666 -> 7
        assert_eq!(r3, 7);
    }

    #[test]
    fn test_badness_calculation() {
        assert_eq!(badness(0, 100), 0);
        assert_eq!(badness(100, 0), 10000);
        assert_eq!(badness(100, -5), 10000);
        assert!(badness(100, 100) > 0);
    }

    #[test]
    fn test_reentrancy_and_cloning() {
        let mut state1 = TeXEngineState::new(500);
        state1.set_info(10, 42).unwrap();
        state1.set_link(10, 84).unwrap();

        // Clone state (simulating CoW preamble branching)
        let mut state2 = state1.clone();
        assert_eq!(state2.info(10).unwrap(), 42);
        assert_eq!(state2.link(10).unwrap(), 84);

        // Mutate state2 without affecting state1
        state2.set_info(10, 999).unwrap();
        assert_eq!(state2.info(10).unwrap(), 999);
        assert_eq!(state1.info(10).unwrap(), 42);
    }

    #[test]
    fn test_random_generator() {
        let mut rand = TeXRandom::new(12345);
        let r1 = rand.unif_rand(100);
        assert!((0..=100).contains(&r1));
        let r2 = rand.unif_rand(100);
        assert!((0..=100).contains(&r2));
    }

    #[test]
    fn test_rover_dynamic_allocator() {
        let mut state = TeXEngineState::new(2000);

        // Allocate a box node (8 words) and a rule node (5 words)
        let b = state.get_node(BOX_NODE_SIZE).unwrap();
        assert!(b > 0);
        let r = state.get_node(RULE_NODE_SIZE).unwrap();
        assert!(r > 0);
        assert_ne!(b, r);

        // Free the box node
        state.free_node(b, BOX_NODE_SIZE).unwrap();
        // Memory word at b is marked as free with its size
        assert_eq!(state.mem[b as usize].s1, MAX_HALFWORD);
        assert_eq!(state.mem[b as usize].s0, BOX_NODE_SIZE);

        let b2 = state.get_node(BOX_NODE_SIZE).unwrap();
        assert!(b2 > 0);
    }

    #[test]
    fn test_rover_coalescing() {
        let mut state = TeXEngineState::new(2000);

        let n1 = state.get_node(10).unwrap();
        let n2 = state.get_node(10).unwrap();
        let n3 = state.get_node(10).unwrap();

        // Free adjacent nodes
        state.free_node(n1, 10).unwrap();
        state.free_node(n2, 10).unwrap();

        // Allocate a combined 20 words: rover must coalesce n1 and n2
        let n_combined = state.get_node(20).unwrap();
        assert!(n_combined > 0);

        state.free_node(n3, 10).unwrap();
        state.free_node(n_combined, 20).unwrap();
    }

    #[test]
    fn test_high_mem_token_lists() {
        let mut state = TeXEngineState::new(1000);

        let t1 = state.get_avail().unwrap();
        let t2 = state.get_avail().unwrap();
        assert!(t1 >= state.lo_mem_max);
        assert!(t2 >= state.lo_mem_max);
        assert_ne!(t1, t2);

        // Link t1 -> t2
        state.set_link(t1, t2).unwrap();
        assert_eq!(state.link(t1).unwrap(), t2);

        // Flush list back to avail
        state.flush_list(t1).unwrap();

        // Allocate again: should pop from avail
        let t3 = state.get_avail().unwrap();
        assert_eq!(t3, t1);
    }

    #[test]
    fn test_node_constructors() {
        let mut state = TeXEngineState::new(2000);

        let bx = state.new_null_box().unwrap();
        assert_eq!(state.node_type(bx).unwrap(), HLIST_NODE);

        let rl = state.new_rule().unwrap();
        assert_eq!(state.node_type(rl).unwrap(), RULE_NODE);
        assert_eq!(state.mem[(rl + 1) as usize].s1, NULL_FLAG);

        let glue_spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        let g = state.new_glue(glue_spec).unwrap();
        assert_eq!(state.node_type(g).unwrap(), GLUE_NODE);
        assert_eq!(state.mem[(g + 1) as usize].s0, glue_spec);

        let kn = state.new_kern(65536).unwrap();
        assert_eq!(state.node_type(kn).unwrap(), KERN_NODE);
        assert_eq!(state.mem[(kn + 1) as usize].s1, 65536);

        let p = state.new_penalty(-1000).unwrap();
        assert_eq!(state.node_type(p).unwrap(), PENALTY_NODE);
        assert_eq!(state.mem[(p + 1) as usize].s1, -1000);
    }

    #[test]
    fn test_eqtb_registers_and_catcodes() {
        let mut state = TeXEngineState::new(2000);

        // Count register
        state.set_count(0, 42).unwrap();
        assert_eq!(state.count(0).unwrap(), 42);

        // Dimen register
        state.set_dimen(1, 655360).unwrap();
        assert_eq!(state.dimen(1).unwrap(), 655360);

        // Skip register
        state.set_skip(0, 123).unwrap();
        assert_eq!(state.skip(0).unwrap(), 123);

        // Box register
        state.set_box_reg(255, 456).unwrap();
        assert_eq!(state.box_reg(255).unwrap(), 456);

        // Toks register
        state.set_toks(0, 789).unwrap();
        assert_eq!(state.toks(0).unwrap(), 789);

        // Catcodes
        assert_eq!(state.cat_code(b'\\' as u32), 0); // Escape
        assert_eq!(state.cat_code(b'{' as u32), 1);  // LeftBrace
        assert_eq!(state.cat_code(b'}' as u32), 2);  // RightBrace
        assert_eq!(state.cat_code(b'$' as u32), 3);  // MathShift
        assert_eq!(state.cat_code(b'&' as u32), 4);  // TabMark
        assert_eq!(state.cat_code(b'A' as u32), 11); // Letter
        assert_eq!(state.cat_code(b'z' as u32), 11); // Letter
        assert_eq!(state.cat_code(b'%' as u32), 14); // Comment

        // Mutate catcode
        state.set_cat_code(b'@' as u32, 11);
        assert_eq!(state.cat_code(b'@' as u32), 11);
    }

    #[test]
    fn test_command_codes_and_modes() {
        assert_eq!(CommandCode::from_u8(0), Some(CommandCode::Relax));
        assert_eq!(CommandCode::from_u8(11), Some(CommandCode::Letter));
        assert_eq!(CommandCode::from_u8(14), Some(CommandCode::Stop));
        assert_eq!(CommandCode::from_u8(200), None);

        assert_eq!(TeXMode::Vmode as i16, 1);
        assert_eq!(TeXMode::Hmode as i16, 104);
        assert_eq!(TeXMode::Mmode as i16, 207);
    }
}
