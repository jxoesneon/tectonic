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

use std::collections::{HashMap, VecDeque};
use std::fmt;

use crate::scanner::{
    ControlSequenceTable, FileInputSource, InputState, ReadOpen, CS_TOKEN_FLAG, MID_LINE, PAR_CS_ID,
};

/// Error type for TeX dynamic memory allocations and operations.
#[derive(Debug, PartialEq, Eq)]
pub enum TeXMemoryError {
    /// Available dynamic node memory exhausted.
    OutOfMemory(usize),

    /// Attempted to access memory outside valid bounds.
    OutOfBounds(i32, usize),

    /// Corrupted free list or node header detected.
    CorruptedNode(i32),

    /// TeX save-stack capacity exceeded.
    SaveStackOverflow,

    /// TeX semantic-list nest capacity exceeded.
    NestStackOverflow,

    /// `pop_nest` was requested while the semantic nest was empty.
    NestStackUnderflow,

    /// TeX grouping depth exceeded the 16-bit level range.
    GroupingLevelOverflow,

    /// `unsave` was requested at the outermost level.
    SaveStackUnderflow,
}

impl fmt::Display for TeXMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfMemory(words) => {
                write!(
                    f,
                    "TeX capacity exceeded (out of memory: {words} words requested)"
                )
            }
            Self::OutOfBounds(addr, len) => {
                write!(
                    f,
                    "TeX memory address {addr} out of bounds (valid range: 0..{len})"
                )
            }
            Self::CorruptedNode(addr) => {
                write!(f, "Corrupted memory node structure at address {addr}")
            }
            Self::SaveStackOverflow => write!(f, "TeX capacity exceeded (save size)"),
            Self::NestStackOverflow => {
                write!(f, "TeX capacity exceeded (semantic nest size)")
            }
            Self::NestStackUnderflow => write!(f, "TeX semantic nest underflow"),
            Self::GroupingLevelOverflow => {
                write!(f, "TeX capacity exceeded (grouping levels)")
            }
            Self::SaveStackUnderflow => write!(f, "TeX save-stack underflow"),
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

    /// TeX symbolic accessor for a glue-set floating-point word (`gr`).
    #[inline(always)]
    pub fn gr(&self) -> f64 {
        f64::from_bits((self.s0 as u32 as u64) | ((self.s1 as u32 as u64) << 32))
    }

    /// TeX symbolic mutator for a glue-set floating-point word (`gr`).
    #[inline(always)]
    pub fn set_gr(&mut self, val: f64) {
        let bits = val.to_bits();
        self.s0 = bits as u32 as i32;
        self.s1 = (bits >> 32) as u32 as i32;
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

/// Fixed-point character metrics used by horizontal packaging.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FontCharMetric {
    /// Character advance width.
    pub width: i32,
    /// Character height above the baseline.
    pub height: i32,
    /// Character depth below the baseline.
    pub depth: i32,
    /// Character italic correction.
    pub italic: i32,
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

    fn next_random(&mut self) -> i32 {
        if self.j_random == 0 {
            self.new_randoms();
        } else {
            self.j_random -= 1;
        }
        self.randoms[self.j_random]
    }

    /// Generates a uniform random integer in the range `0..x` (or `-x..0` if x is negative).
    pub fn unif_rand(&mut self, x: i32) -> i32 {
        let y = take_frac(x.abs(), self.next_random());
        if y == x.abs() {
            0
        } else if x > 0 {
            y
        } else {
            -y
        }
    }

    /// Generates a random value from TeX's fixed-point normal distribution (`norm_rand`).
    pub fn norm_rand(&mut self) -> i32 {
        loop {
            let mut x;
            let mut u;
            loop {
                x = take_frac(112_429, self.next_random() - 0x0800_0000);
                u = self.next_random();
                if x.abs() < u {
                    break;
                }
            }

            x = make_frac(x, u);
            let l = 139_548_960 - m_log(u);
            if ab_vs_cd(1024, l, x, x) >= 0 {
                return x;
            }
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

/// Computes `floor(p / q * 2^28 + 1/2)` with TeX's exact fraction arithmetic.
fn make_frac(mut p: i32, mut q: i32) -> i32 {
    let mut negative = false;
    if p < 0 {
        p = -p;
        negative = true;
    }
    if q <= 0 {
        q = -q;
        negative = !negative;
    }

    let mut n = p / q;
    p %= q;
    if n >= 8 {
        return if negative { i32::MIN + 1 } else { i32::MAX };
    }

    n = (n - 1) * 0x1000_0000;
    let mut f = 1;
    loop {
        let be_careful = p - q;
        p += be_careful;
        if p >= 0 {
            f = f * 2 + 1;
        } else {
            f *= 2;
            p += q;
        }
        if f >= 0x1000_0000 {
            break;
        }
    }

    if p - q + p >= 0 {
        f += 1;
    }
    if negative {
        -(f + n)
    } else {
        f + n
    }
}

/// TeX's fixed-point approximation of `2^28 ln(x / 2^28)` (`m_log`).
fn m_log(mut x: i32) -> i32 {
    if x <= 0 {
        return 0;
    }

    const SPEC_LOG: [i32; 29] = [
        0, 93_032_640, 38_612_034, 17_922_280, 8_662_214, 4_261_238, 2_113_709, 1_052_693, 525_315,
        262_400, 131_136, 65_552, 32_772, 16_385, 8_192, 4_096, 2_048, 1_024, 512, 256, 128, 64,
        32, 16, 8, 4, 2, 1, 1,
    ];

    let mut y = 1_302_456_860;
    let mut z = 6_581_195;
    while x < 0x4000_0000 {
        x *= 2;
        y -= 93_032_639;
        z -= 48_782;
    }
    y += z / 65_536;

    let mut k = 2usize;
    while x > 0x4000_0004 {
        let mut zz = ((x - 1) / (1i32 << k)) + 1;
        while x < 0x4000_0000 + zz {
            zz = (zz + 1) / 2;
            k += 1;
        }
        y += SPEC_LOG[k];
        x -= zz;
    }
    y / 8
}

/// Compares `a/b` with `c/d` without floating point (`ab_vs_cd`).
fn ab_vs_cd(mut a: i32, mut b: i32, mut c: i32, mut d: i32) -> i32 {
    if a < 0 {
        a = -a;
        b = -b;
    }
    if c < 0 {
        c = -c;
        d = -d;
    }

    if d <= 0 {
        if b >= 0 {
            return if (a == 0 || b == 0) && (c == 0 || d == 0) {
                0
            } else {
                1
            };
        }
        if d == 0 {
            return if a == 0 { 0 } else { -1 };
        }
        std::mem::swap(&mut a, &mut c);
        std::mem::swap(&mut b, &mut d);
        b = -b;
        d = -d;
    } else if b <= 0 {
        if b < 0 && a > 0 {
            return -1;
        }
        return if c == 0 { 0 } else { -1 };
    }

    loop {
        let q = a / d;
        let r = c / b;
        if q != r {
            return if q > r { 1 } else { -1 };
        }

        let q = a % d;
        let r = c % b;
        if r == 0 {
            return if q == 0 { 0 } else { 1 };
        }
        if q == 0 {
            return -1;
        }
        a = b;
        b = q;
        c = d;
        d = r;
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
/// Default rule dimension used when `width`, `height`, or `depth` is omitted.
pub const DEFAULT_RULE: i32 = 26_214;
/// Sentinel for null link or null node.
pub const TEX_NULL: i32 = 0;

/// Default glue subtype and normal glue stretch/shrink order (`normal`).
pub const NORMAL: u16 = 0;
/// Glue stretch/shrink order for `fil` (`fil`).
pub const FIL: u16 = 1;
/// Glue stretch/shrink order for `fill` (`fill`).
pub const FILL: u16 = 2;
/// Glue stretch/shrink order for `filll` (`filll`).
pub const FILLL: u16 = 3;
/// Explicit kern subtype (`explicit`).
pub const EXPLICIT: u16 = 1;
/// Kern subtype: italic-correction kern after an `\accent` (`ACC_KERN`).
pub const ACC_KERN: u16 = 2;
/// Kern subtype: space-adjustment kern inserted by
/// `\XeTeXinterwordspaceshaping` (`SPACE_ADJUSTMENT`).
pub const SPACE_ADJUSTMENT: u16 = 3;
/// TeX--XeT math-node direction encoding step for left-to-right boundaries.
pub const L_CODE: i32 = 4;
/// TeX--XeT math-node direction encoding step for right-to-left boundaries.
pub const R_CODE: i32 = 8;
/// Math-node subtype before math (`BEFORE`).
pub const BEFORE: i32 = 0;
/// Math-node subtype after math (`AFTER`).
pub const AFTER: i32 = 1;
/// Math-node subtype before a TeX--XeT begin direction (`BEGIN_M_CODE`).
pub const BEGIN_M_CODE: i32 = 2;
/// Math-node subtype ending math (`END_M_CODE`).
pub const END_M_CODE: i32 = 3;
/// Penalty that forces a page or split break (`eject_penalty`).
pub const EJECT_PENALTY: i32 = -10_000;
/// Infinitely bad ordinary break (`inf_bad`).
pub const INF_BAD: i32 = 10_000;
/// Infinite penalty suppressing a break (`inf_penalty`).
pub const INF_PENALTY: i32 = 10_000;

/// XDV output staging buffer capacity (`DVI_BUF_SIZE`).
pub const DVI_BUF_SIZE: usize = 16_384;
/// Half of the staging buffer; the `dvi_swap` granularity (`HALF_BUF`).
pub const HALF_BUF: usize = 8_192;

/// DVI opcode: set a character with a one-byte code (`set1`).
pub const SET1: u8 = 128;
/// DVI opcode: typeset a rule and advance (`set_rule`).
pub const SET_RULE: u8 = 132;
/// DVI opcode: typeset a rule without advancing (`put_rule`).
pub const PUT_RULE: u8 = 137;
/// DVI opcode: beginning of page (`bop`).
pub const BOP: u8 = 139;
/// DVI opcode: end of page (`eop`).
pub const EOP: u8 = 140;
/// DVI opcode: push position (`push`).
pub const PUSH: u8 = 141;
/// DVI opcode: pop position (`pop`).
pub const POP: u8 = 142;
/// DVI opcode: move right one byte (`right1`); `right2..right4` follow.
pub const RIGHT1: u8 = 143;
/// DVI opcode: move down one byte (`down1`); `down2..down4` follow.
pub const DOWN1: u8 = 157;
/// DVI opcode: select font 0 (`fnt_num_0`); fonts 1..=3 follow.
pub const FNT_NUM_0: u8 = 171;
/// DVI opcode: select font with a one-byte number (`fnt1`); up to `fnt4`.
pub const FNT1: u8 = 235;
/// DVI opcode: one-byte-length special (`xxx1`); `xxx2..xxx4` follow.
pub const XXX1: u8 = 239;
/// DVI opcode: four-byte-length special (`xxx4`).
pub const XXX4: u8 = 242;
/// DVI opcode: font definition with a one-byte number (`fnt_def1`).
pub const FNT_DEF1: u8 = 243;
/// DVI opcode: preamble (`pre`).
pub const PRE: u8 = 247;
/// DVI opcode: postamble (`post`).
pub const POST: u8 = 248;
/// DVI opcode: postamble tail (`post_post`).
pub const POST_POST: u8 = 249;
/// XDV opcode: native-font definition (`define_native_font`).
pub const DEFINE_NATIVE_FONT: u8 = 252;
/// XDV opcode: positioned glyph array (`set_glyphs`).
pub const SET_GLYPHS: u8 = 253;
/// XDV opcode: text plus positioned glyphs (`set_text_and_glyphs`).
pub const SET_TEXT_AND_GLYPHS: u8 = 254;
/// XDV format identification byte (`XDV_ID_BYTE`).
pub const XDV_ID_BYTE: u8 = 7;
/// Semantic-pagination format identification byte (`SPX_ID_BYTE`).
pub const SPX_ID_BYTE: u8 = 100;
/// Native-font definition flag: vertical forms (`XDV_FLAG_VERTICAL`).
pub const XDV_FLAG_VERTICAL: u16 = 0x0100;
/// Native-font definition flag: explicit color (`XDV_FLAG_COLORED`).
pub const XDV_FLAG_COLORED: u16 = 0x0200;
/// Native-font definition flag: horizontal extend (`XDV_FLAG_EXTEND`).
pub const XDV_FLAG_EXTEND: u16 = 0x1000;
/// Native-font definition flag: slant (`XDV_FLAG_SLANT`).
pub const XDV_FLAG_SLANT: u16 = 0x2000;
/// Native-font definition flag: embolden (`XDV_FLAG_EMBOLDEN`).
pub const XDV_FLAG_EMBOLDEN: u16 = 0x4000;
/// Native font flag: colored (`FONT_FLAGS_COLORED`).
pub const FONT_FLAGS_COLORED: u16 = 0x01;
/// Native font flag: vertical (`FONT_FLAGS_VERTICAL`).
pub const FONT_FLAGS_VERTICAL: u16 = 0x02;

/// Size of a movement-record node in memory words (`MOVEMENT_NODE_SIZE`).
pub const MOVEMENT_NODE_SIZE: i32 = 3;
/// Movement-record state: nothing seen yet (`MOV_NONE_SEEN`).
pub const MOV_NONE_SEEN: i32 = 0;
/// Movement-record state: this distance is the current `y` (`MOV_Y_HERE`).
pub const MOV_Y_HERE: i32 = 1;
/// Movement-record state: this distance is the current `z` (`MOV_Z_HERE`).
pub const MOV_Z_HERE: i32 = 2;
/// Movement-record state: distance usable as `y` or `z` (`MOV_YZ_OK`).
pub const MOV_YZ_OK: i32 = 3;
/// Movement-record state: distance usable as `y` only (`MOV_Y_OK`).
pub const MOV_Y_OK: i32 = 4;
/// Movement-record state: distance usable as `z` only (`MOV_Z_OK`).
pub const MOV_Z_OK: i32 = 5;
/// Movement-record state: a `y`-candidate was seen (`MOV_Y_SEEN`); C also
/// spells this value `MOV_D_FIXED` in the "w0/z0 fixed" sense.
pub const MOV_Y_SEEN: i32 = 6;
/// Alias of [`MOV_Y_SEEN`] used when patching records to `d`-fixed
/// (`MOV_D_FIXED`).
pub const MOV_D_FIXED: i32 = MOV_Y_SEEN;
/// Movement-record state: a `z`-candidate was seen (`MOV_Z_SEEN`).
pub const MOV_Z_SEEN: i32 = 12;

/// Box `lr_mode` marking a direction-reversed list (`reversed`).
pub const REVERSED: i32 = 1;
/// Box `lr_mode` marking a list reversed in a `dlist` context
/// (`DLIST`).
pub const DLIST: i32 = 2;

/// `\hoffset` dimension parameter (`DIMEN_PAR__h_offset`).
pub const DIMEN_PAR_H_OFFSET: usize = 18;
/// `\voffset` dimension parameter (`DIMEN_PAR__v_offset`).
pub const DIMEN_PAR_V_OFFSET: usize = 19;
/// `\pdfpagewidth` dimension parameter (`DIMEN_PAR__pdf_page_width`).
pub const DIMEN_PAR_PDF_PAGE_WIDTH: usize = 21;
/// `\pdfpageheight` dimension parameter (`DIMEN_PAR__pdf_page_height`).
pub const DIMEN_PAR_PDF_PAGE_HEIGHT: usize = 22;
/// `\tracingoutput` integer parameter (`INT_PAR__tracing_output`).
pub const INT_PAR_TRACING_OUTPUT: usize = 34;

/// Size of an `if_node` in memory words.
pub const IF_NODE_SIZE: i32 = 2;
/// Size of a small node in memory words.
pub const SMALL_NODE_SIZE: i32 = 2;
/// Size of a medium node in memory words.
pub const MEDIUM_NODE_SIZE: i32 = 3;
/// Size of an open whatsit node in memory words.
pub const OPEN_NODE_SIZE: i32 = 3;
/// Size of a math-style node in memory words.
pub const STYLE_NODE_SIZE: i32 = 3;
/// Size of a glue specification node in memory words.
pub const GLUE_SPEC_SIZE: i32 = 4;
/// Size of an ordinary math noad in memory words.
pub const NOAD_SIZE: i32 = 4;
/// Size of a glyph whatsit node in memory words.
pub const GLYPH_NODE_SIZE: i32 = 5;
/// Size of an insertion node in memory words.
pub const INS_NODE_SIZE: i32 = 5;
/// Size of a rule node in memory words.
pub const RULE_NODE_SIZE: i32 = 5;
/// Size of an accent noad in memory words.
pub const ACCENT_NOAD_SIZE: i32 = 5;
/// Size of a radical noad in memory words.
pub const RADICAL_NOAD_SIZE: i32 = 5;
/// Size of a native-word whatsit node in memory words.
pub const NATIVE_NODE_SIZE: i32 = 6;
/// Size of a fraction noad in memory words.
pub const FRACTION_NOAD_SIZE: i32 = 6;
/// Size of a box node in memory words.
pub const BOX_NODE_SIZE: i32 = 8;
/// Fixed portion of a picture/pdf literal whatsit node.
pub const PIC_NODE_SIZE: i32 = 9;
/// Size of a sparse e-TeX integer/dimension element (`WORD_NODE_SIZE`).
pub const WORD_NODE_SIZE: i32 = 3;
/// Size of a sparse e-TeX pointer-valued element (`POINTER_NODE_SIZE`).
pub const POINTER_NODE_SIZE: i32 = 2;
/// Size of a sparse mark-class element (`MARK_CLASS_NODE_SIZE`).
pub const MARK_CLASS_NODE_SIZE: i32 = 4;
/// Size of one sparse-array index node (`INDEX_NODE_SIZE`).
pub const INDEX_NODE_SIZE: i32 = 33;
/// Number of sparse-array roots (`sa_root[0..8]`).
pub const SA_ROOT_COUNT: usize = 8;
/// Sparse register class for boxes (`BOX_VAL`).
pub const SA_BOX_VAL: i32 = 4;
/// Sparse register class for token lists (`TOK_VAL`).
pub const SA_TOK_VAL: i32 = 5;
/// Sparse register class for XeTeX inter-character token lists (`INTER_CHAR_VAL`).
pub const SA_INTER_CHAR_VAL: i32 = 6;
/// Sparse register class for e-TeX mark classes (`MARK_VAL`).
pub const SA_MARK_VAL: i32 = 7;
/// Encoded type limit below which sparse elements are word-valued (`DIMEN_VAL_LIMIT`).
pub const DIMEN_VAL_LIMIT: i32 = 128;
/// Encoded type limit below which sparse elements hold glue pointers (`MU_VAL_LIMIT`).
pub const MU_VAL_LIMIT: i32 = 256;
/// Encoded type limit below which sparse elements hold box lists (`BOX_VAL_LIMIT`).
pub const BOX_VAL_LIMIT: i32 = 320;
/// Encoded type limit below which sparse elements hold token lists (`TOK_VAL_LIMIT`).
pub const TOK_VAL_LIMIT: i32 = 384;
/// Largest user-visible register number (`max_reg_num`).
pub const MAX_SA_REGISTER_NUM: i32 = 32_767;
/// Largest sparse-array key supported by the four-level index.
pub const MAX_SA_ELEMENT_NUM: i32 = 0xFF_FFFF;
/// `do_marks` action initializing marks before `\vsplit`.
pub const VSPLIT_INIT: u8 = 0;
/// `do_marks` action initializing marks before `fire_up`.
pub const FIRE_UP_INIT: u8 = 1;
/// `do_marks` action finishing marks after `fire_up`.
pub const FIRE_UP_DONE: u8 = 2;
/// `do_marks` action destroying all mark-class references.
pub const DESTROY_MARKS: u8 = 3;
/// Top-mark slot selector (`TOP_MARK_CODE`).
pub const TOP_MARK_CODE: i32 = 0;
/// First-mark slot selector (`FIRST_MARK_CODE`).
pub const FIRST_MARK_CODE: i32 = 1;
/// Bottom-mark slot selector (`BOT_MARK_CODE`).
pub const BOT_MARK_CODE: i32 = 2;
/// First split-first mark slot selector (`SPLIT_FIRST_MARK_CODE`).
pub const SPLIT_FIRST_MARK_CODE: i32 = 3;
/// Last split-bottom mark slot selector (`SPLIT_BOT_MARK_CODE`).
pub const SPLIT_BOT_MARK_CODE: i32 = 4;

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
/// Span-record node size (`SPAN_NODE_SIZE`).
pub const SPAN_NODE_SIZE: i32 = 2;
/// Alignment save-stack node size (`ALIGN_STACK_NODE_SIZE`).
pub const ALIGN_STACK_NODE_SIZE: i32 = 6;
/// Math style node type (`style_node`).
pub const STYLE_NODE: u16 = 14;
/// Math choice node type (`choice_node`).
pub const CHOICE_NODE: u16 = 15;
/// Ordinary math noad type (`ord_noad`).
pub const ORD_NOAD: u16 = 16;
/// Operator math noad type (`op_noad`).
pub const OP_NOAD: u16 = 17;
/// Binary math noad type (`bin_noad`).
pub const BIN_NOAD: u16 = 18;
/// Relation math noad type (`rel_noad`).
pub const REL_NOAD: u16 = 19;
/// Open math noad type (`open_noad`).
pub const OPEN_NOAD: u16 = 20;
/// Close math noad type (`close_noad`).
pub const CLOSE_NOAD: u16 = 21;
/// Punctuation math noad type (`punct_noad`).
pub const PUNCT_NOAD: u16 = 22;
/// Inner math noad type (`inner_noad`).
pub const INNER_NOAD: u16 = 23;
/// Radical math noad type (`radical_noad`).
pub const RADICAL_NOAD: u16 = 24;
/// Fraction math noad type (`fraction_noad`).
pub const FRACTION_NOAD: u16 = 25;
/// Under math noad type (`under_noad`).
pub const UNDER_NOAD: u16 = 26;
/// Over math noad type (`over_noad`).
pub const OVER_NOAD: u16 = 27;
/// Accent math noad type (`accent_noad`).
pub const ACCENT_NOAD: u16 = 28;
/// V-center math noad type (`vcenter_noad`).
pub const VCENTER_NOAD: u16 = 29;
/// Left math noad type (`left_noad`).
pub const LEFT_NOAD: u16 = 30;
/// Right math noad type (`right_noad`).
pub const RIGHT_NOAD: u16 = 31;
/// Margin-kern node type (`margin_kern_node`).
pub const MARGIN_KERN_NODE: u16 = 40;

/// Open-file whatsit subtype (`open_node`).
pub const OPEN_NODE: u16 = 0;
/// Write whatsit subtype (`write_node`).
pub const WRITE_NODE: u16 = 1;
/// Close-file whatsit subtype (`close_node`).
pub const CLOSE_NODE: u16 = 2;
/// Special whatsit subtype (`special_node`).
pub const SPECIAL_NODE: u16 = 3;
/// Language whatsit subtype (`language_node`).
pub const LANGUAGE_NODE: u16 = 4;
/// PDF save-position whatsit subtype (`pdf_save_pos_node`).
pub const PDF_SAVE_POS_NODE: u16 = 21;
/// Native-word whatsit subtype (`native_word_node`).
pub const NATIVE_WORD_NODE: u16 = 40;
/// Native-word whatsit subtype with explicit advances (`native_word_node_AT`).
pub const NATIVE_WORD_NODE_AT: u16 = 41;
/// Glyph whatsit subtype (`glyph_node`).
pub const GLYPH_NODE: u16 = 42;
/// Picture whatsit subtype (`pic_node`).
pub const PIC_NODE: u16 = 43;
/// TeX--XeT begin-L math subtype (`BEGIN_L_CODE`).
pub const BEGIN_L_CODE: i32 = 6;
/// TeX--XeT end-L math subtype (`END_L_CODE`).
pub const END_L_CODE: i32 = 7;
/// Write whatsit node size (`WRITE_NODE_SIZE`).
pub const WRITE_NODE_SIZE: i32 = 2;
/// Math conditional glue subtype (`\nonscript`, `COND_MATH_GLUE`).
pub const COND_MATH_GLUE: u16 = 98;
/// PDF literal whatsit subtype (`pdf_node`).
pub const PDF_NODE: u16 = 44;
/// Math field subtype holding a packed math character (`math_char`).
pub const MATH_CHAR: i32 = 1;
/// Math field subtype holding a text-mode character (`MATH_TEXT_CHAR`).
pub const MATH_TEXT_CHAR: i32 = 4;
/// `\limits` subtype marker on an `OP_NOAD` (`LIMITS`).
pub const LIMITS: u16 = 1;
/// Math field subtype indicating a boxed sublist (`sub_box`).
pub const SUB_BOX: i32 = 2;
/// Math field subtype holding an mlist pointer (`sub_mlist`).
pub const SUB_MLIST: i32 = 3;
/// Empty math-field subtype (`empty`).
pub const MATH_FIELD_EMPTY: i32 = 0;

/// Display math style (`display_style`).
pub const DISPLAY_STYLE: i32 = 0;
/// Text math style (`text_style`).
pub const TEXT_STYLE: i32 = 2;
/// Script math style (`script_style`).
pub const SCRIPT_STYLE: i32 = 4;
/// Scriptscript math style (`script_script_style`).
pub const SCRIPT_SCRIPT_STYLE: i32 = 6;

/// `\above*` chr-code range marking delimited fractions (`delimited_code`).
pub const DELIMITED_CODE: i32 = 3;
/// `\above` fraction code (`above_code`).
pub const ABOVE_CODE: i32 = 0;
/// `\over` fraction code (`over_code`).
pub const OVER_CODE: i32 = 1;
/// `\atop` fraction code (`atop_code`).
pub const ATOP_CODE: i32 = 2;
/// Fixed-position math accent flag (`fixed_acc`).
pub const FIXED_ACC: i32 = 1;
/// Bottom math accent flag (`bottom_acc`).
pub const BOTTOM_ACC: i32 = 2;
/// Marker requesting `default_rule_thickness` (`default_code`).
pub const DEFAULT_CODE: i32 = 0x4000_0000;
/// Math symbol font parameter count (`total_mathsy_params`).
pub const TOTAL_MATHSY_PARAMS: usize = 22;
/// Math extension font parameter count (`total_mathex_params`).
pub const TOTAL_MATHEX_PARAMS: usize = 13;
/// Text-size math-family offset (`text_size`).
pub const TEXT_SIZE: i32 = 0;
/// Script-size math-family offset (`script_size`).
pub const SCRIPT_SIZE: i32 = 256;
/// Scriptscript-size math-family offset (`script_script_size`).
pub const SCRIPT_SCRIPT_SIZE: i32 = 512;
/// Edge node type, aliasing `style_node` (`edge_node`).
pub const EDGE_NODE: u16 = STYLE_NODE;
/// Edge node size (`edge_node_size`).
pub const EDGE_NODE_SIZE: i32 = 3;
/// Left-to-right direction code (`left_to_right`).
pub const LEFT_TO_RIGHT: i32 = 0;
/// Right-to-left direction code (`right_to_left`).
pub const RIGHT_TO_LEFT: i32 = 1;

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
/// Base index of integer parameters in the compact `eqtb`.
pub const INT_PAR_BASE: usize = 1536;
/// Number of integer parameters reserved in the compact `eqtb`.
pub const INT_PAR_COUNT: usize = 256;
/// Base index of local token-list parameters in the compact `eqtb`.
pub const LOCAL_BASE: usize = 1792;
/// Number of local parameters reserved in the compact `eqtb`.
pub const LOCAL_COUNT: usize = 256;
/// Base index of e-TeX penalty arrays in the compact `eqtb`.
pub const ETEX_PEN_BASE: usize = LOCAL_BASE + LOCAL_COUNT;
/// Number of e-TeX penalty arrays.
pub const ETEX_PEN_COUNT: usize = 4;
/// Base index of dimension parameters in the compact `eqtb`.
pub const DIMEN_PAR_BASE: usize = ETEX_PEN_BASE + ETEX_PEN_COUNT;
/// Number of dimension parameters (`DIMEN_PARS`).
pub const DIMEN_PAR_COUNT: usize = 23;
/// Base index of glue parameters in the compact `eqtb`.
pub const GLUE_PAR_BASE: usize = DIMEN_PAR_BASE + DIMEN_PAR_COUNT;
/// Number of glue parameters, including the three mu-glue parameters (`GLUE_PARS`).
pub const GLUE_PAR_COUNT: usize = 19;

/// `\thinmuskip` (`GLUE_PAR__thin_mu_skip`).
pub const GLUE_PAR_THIN_MU_SKIP: usize = 16;
/// `\medmuskip` (`GLUE_PAR__med_mu_skip`).
pub const GLUE_PAR_MED_MU_SKIP: usize = 17;
/// `\thickmuskip` (`GLUE_PAR__thick_mu_skip`).
pub const GLUE_PAR_THICK_MU_SKIP: usize = 18;
/// `\binoppenalty` (`INT_PAR__bin_op_penalty`).
pub const INT_PAR_BIN_OP_PENALTY: usize = 9;
/// `\relpenalty` (`INT_PAR__rel_penalty`).
pub const INT_PAR_REL_PENALTY: usize = 10;
/// `\delimiterfactor` (`INT_PAR__delimiter_factor`).
pub const INT_PAR_DELIMITER_FACTOR: usize = 18;
/// `\delimitershortfall` (`DIMEN_PAR__delimiter_shortfall`).
pub const DIMEN_PAR_DELIMITER_SHORTFALL: usize = 10;
/// `\nulldelimiterspace` (`DIMEN_PAR__null_delimiter_space`).
pub const DIMEN_PAR_NULL_DELIMITER_SPACE: usize = 11;
/// `\scriptspace` (`DIMEN_PAR__script_space`).
pub const DIMEN_PAR_SCRIPT_SPACE: usize = 12;
/// Superscript math-kern command selector (`SUP_CMD`).
pub const SUP_CMD: u8 = 0;
/// Subscript math-kern command selector (`SUB_CMD`).
pub const SUB_CMD: u8 = 1;

/// Compact `eqtb` slot holding the current font number (`CUR_FONT_LOC`).
pub const CUR_FONT_LOC: usize = GLUE_PAR_BASE + GLUE_PAR_COUNT;
/// Base index of the math-family font slots in the compact `eqtb`
/// (`MATH_FONT_BASE`).
pub const FAMILY_BASE: usize = CUR_FONT_LOC + 1;
/// Number of math-family font slots (`NUMBER_MATH_FAMILIES` = 256 per size,
/// three sizes: text, script, scriptscript).
pub const FAMILY_COUNT: usize = 256 * 3;
/// C `eqtb` base address used by the `\textfont` family of primitives; the
/// compact layout maps it to [`FAMILY_BASE`].
pub const MATH_FONT_BASE_C: i32 = 2_255_401;

/// Font-area flag marking an AAT native font (`AAT_FONT_FLAG`).
pub const AAT_FONT_FLAG: u16 = 0xFFFF;
/// Font-area flag marking an OpenType/Graphite native font (`OTGR_FONT_FLAG`).
pub const OTGR_FONT_FLAG: u16 = 0xFFFE;

/// `\fontdimen` code for slant (`SLANT_CODE`).
pub const SLANT_CODE: i32 = 1;
/// `\fontdimen` code for interword space (`SPACE_CODE`).
pub const SPACE_CODE: i32 = 2;
/// `\fontdimen` code for interword stretch (`SPACE_STRETCH_CODE`).
pub const SPACE_STRETCH_CODE: i32 = 3;
/// `\fontdimen` code for interword shrink (`SPACE_SHRINK_CODE`).
pub const SPACE_SHRINK_CODE: i32 = 4;
/// `\fontdimen` code for x-height (`X_HEIGHT_CODE`).
pub const X_HEIGHT_CODE: i32 = 5;
/// `\fontdimen` code for quad width (`QUAD_CODE`).
pub const QUAD_CODE: i32 = 6;
/// `\fontdimen` code for cap height (`CAP_HEIGHT_CODE`).
pub const CAP_HEIGHT_CODE: i32 = 8;
/// `\fontdimen` code for extra space (`EXTRA_SPACE_CODE`).
pub const EXTRA_SPACE_CODE: i32 = 7;
/// `ASSIGN_FONT_INT` code for `\hyphenchar`.
pub const HYPHEN_CHAR_CODE: i32 = 0;
/// `ASSIGN_FONT_INT` code for `\skewchar`.
pub const SKEW_CHAR_CODE: i32 = 1;
/// `ASSIGN_FONT_INT` code for `\lpcode` (`LP_CODE_BASE`).
pub const LP_CODE_BASE: i32 = 2;
/// `ASSIGN_FONT_INT` code for `\rpcode` (`RP_CODE_BASE`).
pub const RP_CODE_BASE: i32 = 3;

/// Local parameter offset for `\parshape`.
pub const LOCAL_PAR_SHAPE: usize = 0;
/// Local parameter offset for `\output`.
pub const LOCAL_OUTPUT_ROUTINE: usize = 1;
/// Local parameter offset for `\everypar`.
pub const LOCAL_EVERY_PAR: usize = 2;
/// Local parameter offset for `\everymath`.
pub const LOCAL_EVERY_MATH: usize = 3;
/// Local parameter offset for `\everydisplay`.
pub const LOCAL_EVERY_DISPLAY: usize = 4;
/// Local parameter offset for `\everyhbox`.
pub const LOCAL_EVERY_HBOX: usize = 5;
/// Local parameter offset for `\everyvbox`.
pub const LOCAL_EVERY_VBOX: usize = 6;
/// Local parameter offset for `\everyjob`.
pub const LOCAL_EVERY_JOB: usize = 7;
/// Local parameter offset for `\everycr`.
pub const LOCAL_EVERY_CR: usize = 8;
/// Local parameter offset for `\errhelp`.
pub const LOCAL_ERR_HELP: usize = 9;
/// Local parameter offset for `\everyeof`.
pub const LOCAL_EVERY_EOF: usize = 10;
/// Local parameter offset for `\XeTeXinterchartoks`.
pub const LOCAL_XETEX_INTER_CHAR_TOKS: usize = 11;
/// Local parameter offset for `\TectonicCodaTokens`.
pub const LOCAL_TECTONIC_CODA_TOKENS: usize = 12;

/// e-TeX penalty-array offset for `\interlinepenalties`.
pub const ETEX_PEN_INTER_LINE: usize = 0;
/// e-TeX penalty-array offset for `\clubpenalties`.
pub const ETEX_PEN_CLUB: usize = 1;
/// e-TeX penalty-array offset for `\widowpenalties`.
pub const ETEX_PEN_WIDOW: usize = 2;
/// e-TeX penalty-array offset for `\displaywidowpenalties`.
pub const ETEX_PEN_DISPLAY_WIDOW: usize = 3;

/// Integer parameter offset for `\escapechar`.
pub const INT_PAR_ESCAPE_CHAR: usize = 45;
/// `INT_PAR__default_hyphen_char`.
pub const INT_PAR_DEFAULT_HYPHEN_CHAR: usize = 46;
/// `INT_PAR__default_skew_char`.
pub const INT_PAR_DEFAULT_SKEW_CHAR: usize = 47;
/// Integer parameter offset for `\globaldefs`.
pub const INT_PAR_GLOBAL_DEFS: usize = 43;
/// Integer parameter offset for `\endlinechar`.
pub const INT_PAR_END_LINE_CHAR: usize = 48;
/// Integer parameter offset for `\newlinechar`.
pub const INT_PAR_NEW_LINE_CHAR: usize = 49;
/// Integer parameter offset for `\tracingscantokens`.
pub const INT_PAR_TRACING_SCAN_TOKENS: usize = 59;
/// Integer parameter offset for `\savingvdiscards`.
pub const INT_PAR_SAVING_VDISCARDS: usize = 63;
/// `\savinghyphcodes` integer parameter (`INT_PAR__saving_hyph_codes`).
pub const INT_PAR_SAVING_HYPH_CODES: usize = 64;
/// Integer parameter offset for `\looseness`.
pub const INT_PAR_LOOSENESS: usize = 19;
/// Integer parameter offset for `\pretolerance`.
pub const INT_PAR_PRETOLERANCE: usize = 0;
/// Integer parameter offset for `\tolerance`.
pub const INT_PAR_TOLERANCE: usize = 1;
/// Integer parameter offset for `\linepenalty`.
pub const INT_PAR_LINE_PENALTY: usize = 2;
/// Integer parameter offset for `\hyphenpenalty`.
pub const INT_PAR_HYPHEN_PENALTY: usize = 3;
/// Integer parameter offset for `\exhyphenpenalty`.
pub const INT_PAR_EX_HYPHEN_PENALTY: usize = 4;
/// Integer parameter offset for `\clubpenalty`.
pub const INT_PAR_CLUB_PENALTY: usize = 5;
/// Integer parameter offset for `\widowpenalty`.
pub const INT_PAR_WIDOW_PENALTY: usize = 6;
/// Integer parameter offset for `\displaywidowpenalty`.
pub const INT_PAR_DISPLAY_WIDOW_PENALTY: usize = 7;
/// Integer parameter offset for `\brokenpenalty`.
pub const INT_PAR_BROKEN_PENALTY: usize = 8;
/// Integer parameter offset for `\predisplaypenalty`.
pub const INT_PAR_PRE_DISPLAY_PENALTY: usize = 11;
/// Integer parameter offset for `\postdisplaypenalty`.
pub const INT_PAR_POST_DISPLAY_PENALTY: usize = 12;
/// `\fam` current math family (`INT_PAR__cur_fam`).
pub const INT_PAR_CUR_FAM: usize = 44;
/// Pre-display direction (`INT_PAR__pre_display_direction`).
pub const INT_PAR_PRE_DISPLAY_DIRECTION: usize = 61;
/// Integer parameter offset for `\interlinepenalty`.
pub const INT_PAR_INTER_LINE_PENALTY: usize = 13;
/// Integer parameter offset for `\doublehyphendemerits`.
pub const INT_PAR_DOUBLE_HYPHEN_DEMERITS: usize = 14;
/// Integer parameter offset for `\finalhyphendemerits`.
pub const INT_PAR_FINAL_HYPHEN_DEMERITS: usize = 15;
/// Integer parameter offset for `\adjdemerits`.
pub const INT_PAR_ADJ_DEMERITS: usize = 16;
/// Integer parameter offset for `\mag`.
pub const INT_PAR_MAG: usize = 17;
/// Integer parameter offset for `\uchyph`.
pub const INT_PAR_UC_HYPH: usize = 38;
/// Integer parameter offset for `\lastlinefit`.
pub const INT_PAR_LAST_LINE_FIT: usize = 62;
/// Integer parameter offset for `\XeTeXprotrudechars`.
pub const INT_PAR_XETEX_PROTRUDE_CHARS: usize = 68;
/// Integer parameter offset for `\outputpenalty`.
pub const INT_PAR_OUTPUT_PENALTY: usize = 39;
/// Integer parameter offset for `\maxdeadcycles`.
pub const INT_PAR_MAX_DEAD_CYCLES: usize = 40;
/// Integer parameter offset for `\hangafter`.
pub const INT_PAR_HANG_AFTER: usize = 41;
/// Integer parameter offset for `\floatingpenalty`.
pub const INT_PAR_FLOATING_PENALTY: usize = 42;
/// Integer parameter offset for `\holdinginserts`.
pub const INT_PAR_HOLDING_INSERTS: usize = 53;
/// Integer parameter offset for `\language`.
pub const INT_PAR_LANGUAGE: usize = 50;
/// Integer parameter offset for `\lefthyphenmin`.
pub const INT_PAR_LEFT_HYPHEN_MIN: usize = 51;
/// Integer parameter offset for `\righthyphenmin`.
pub const INT_PAR_RIGHT_HYPHEN_MIN: usize = 52;
/// String slot used by `\XeTeXlinebreaklocale`.
pub const INT_PAR_XETEX_LINEBREAK_LOCALE: usize = 66;
/// Integer parameter offset for `\XeTeXdefaultencoding`'s input mode.
/// `INT_PAR__xetex_input_normalization`.
pub const INT_PAR_XETEX_INPUT_NORMALIZATION: usize = 74;
/// `INT_PAR__xetex_default_input_mode`.
pub const INT_PAR_XETEX_DEFAULT_INPUT_MODE: usize = 75;
/// String slot used by `\XeTeXdefaultencoding`.
pub const INT_PAR_XETEX_DEFAULT_INPUT_ENCODING: usize = 76;
/// Integer parameter offset for `\showboxbreadth`.
pub const INT_PAR_SHOW_BOX_BREADTH: usize = 24;
/// Integer parameter offset for `\showboxdepth`.
pub const INT_PAR_SHOW_BOX_DEPTH: usize = 25;
/// Integer parameter offset for `\tracingonline`.
pub const INT_PAR_TRACING_ONLINE: usize = 29;
/// Integer parameter offset for `\hbadness`.
pub const INT_PAR_HBADNESS: usize = 26;
/// Integer parameter offset for `\vbadness`.
pub const INT_PAR_VBADNESS: usize = 27;
/// Integer parameter offset for `\TeXXeTstate`.
pub const INT_PAR_TEXXET: usize = 69;
/// Integer parameter offset for `\XeTeXupwardsmode`.
pub const INT_PAR_XETEX_UPWARDS: usize = 71;
/// Integer parameter offset for `\XeTeXuseglyphmetrics`.
pub const INT_PAR_XETEX_USE_GLYPH_METRICS: usize = 72;
/// Integer parameter offset for `\XeTeXgenerateactualtext`.
pub const INT_PAR_XETEX_GENERATE_ACTUAL_TEXT: usize = 79;
/// Integer parameter offset for `\XeTeXlinebreakpenalty`.
pub const INT_PAR_XETEX_LINEBREAK_PENALTY: usize = 67;
/// Integer parameter offset for `\XeTeXdashbreakstate`.
pub const INT_PAR_XETEX_DASH_BREAK: usize = 70;
/// Integer parameter offset for `\XeTeXinterchartokenstate`.
pub const INT_PAR_XETEX_INTER_CHAR_TOKENS: usize = 73;
/// Integer parameter offset for `\XeTeXinterwordspaceshaping`.
pub const INT_PAR_XETEX_INTERWORD_SPACE_SHAPING: usize = 78;
/// Integer parameter offset for `\XeTeXhyphenatablelength`.
pub const INT_PAR_XETEX_HYPHENATABLE_LENGTH: usize = 80;
/// Integer parameter offset for `\XeTeXtracingfonts`
/// (`INT_PAR__xetex_tracing_fonts`).
pub const INT_PAR_XETEX_TRACING_FONTS: usize = 77;
/// `INT_PAR__time` — minutes since midnight.
pub const INT_PAR_TIME: usize = 20;
/// `INT_PAR__day` — day of month.
pub const INT_PAR_DAY: usize = 21;
/// `INT_PAR__month` — month of year.
pub const INT_PAR_MONTH: usize = 22;
/// `INT_PAR__year` — year.
pub const INT_PAR_YEAR: usize = 23;
/// `INT_PAR__tracing_stats` — statistics printing level.
pub const INT_PAR_TRACING_STATS: usize = 31;
/// First font number (`FONT_BASE`); `nullfont` is `FONT_BASE`.
pub const FONT_BASE: i32 = 0;
/// Integer parameter offset for `\tracinglostchars`.
pub const INT_PAR_TRACING_LOST_CHARS: usize = 35;
/// Integer parameter offset for `\suppressfontnotfounderror`.
pub const INT_PAR_SUPPRESS_FONTNOTFOUND_ERROR: usize = 65;

/// Dimension parameter offset for `\lineskiplimit`.
pub const DIMEN_PAR_LINE_SKIP_LIMIT: usize = 2;
/// Dimension parameter offset for `\splitmaxdepth`.
pub const DIMEN_PAR_SPLIT_MAX_DEPTH: usize = 6;
/// Dimension parameter offset for `\boxmaxdepth`.
pub const DIMEN_PAR_BOX_MAX_DEPTH: usize = 7;
/// Dimension parameter offset for `\hfuzz`.
pub const DIMEN_PAR_HFUZZ: usize = 8;
/// Dimension parameter offset for `\vfuzz`.
pub const DIMEN_PAR_VFUZZ: usize = 9;
/// Dimension parameter offset for `\displayindent`.
pub const DIMEN_PAR_DISPLAY_INDENT: usize = 15;
/// Dimension parameter offset for `\overfullrule`.
pub const DIMEN_PAR_OVERFULL_RULE: usize = 16;
/// Dimension parameter offset for `\hsize`.
pub const DIMEN_PAR_HSIZE: usize = 3;
/// Dimension parameter offset for `\emergencystretch`.
pub const DIMEN_PAR_EMERGENCY_STRETCH: usize = 20;
/// Dimension parameter offset for `\parindent`.
pub const DIMEN_PAR_PAR_INDENT: usize = 0;
/// `\mathsurround` (`DIMEN_PAR__math_surround`).
pub const DIMEN_PAR_MATH_SURROUND: usize = 1;
/// `\predisplaysize` (`DIMEN_PAR__pre_display_size`).
pub const DIMEN_PAR_PRE_DISPLAY_SIZE: usize = 13;
/// `\displaywidth` (`DIMEN_PAR__display_width`).
pub const DIMEN_PAR_DISPLAY_WIDTH: usize = 14;
/// Dimension parameter offset for `\vsize`.
pub const DIMEN_PAR_VSIZE: usize = 4;
/// Dimension parameter offset for `\maxdepth`.
pub const DIMEN_PAR_MAX_DEPTH: usize = 5;
/// Dimension parameter offset for `\hangindent`.
pub const DIMEN_PAR_HANG_INDENT: usize = 17;

/// Glue parameter offset for `\lineskip`.
pub const GLUE_PAR_LINE_SKIP: usize = 0;
/// Glue parameter offset for `\baselineskip`.
pub const GLUE_PAR_BASELINE_SKIP: usize = 1;
/// Glue parameter offset for `\parskip`.
pub const GLUE_PAR_PAR_SKIP: usize = 2;
/// Glue parameter offset for `\abovedisplayskip`.
pub const GLUE_PAR_ABOVE_DISPLAY_SKIP: usize = 3;
/// Glue parameter offset for `\belowdisplayskip`.
pub const GLUE_PAR_BELOW_DISPLAY_SKIP: usize = 4;
/// `\abovedisplayshortskip` (`GLUE_PAR__above_display_short_skip`).
pub const GLUE_PAR_ABOVE_DISPLAY_SHORT_SKIP: usize = 5;
/// `\belowdisplayshortskip` (`GLUE_PAR__below_display_short_skip`).
pub const GLUE_PAR_BELOW_DISPLAY_SHORT_SKIP: usize = 6;
/// Glue parameter offset for `\leftskip`.
pub const GLUE_PAR_LEFT_SKIP: usize = 7;
/// Glue parameter offset for `\rightskip`.
pub const GLUE_PAR_RIGHT_SKIP: usize = 8;
/// Glue parameter offset for `\topskip`.
pub const GLUE_PAR_TOP_SKIP: usize = 9;
/// Glue parameter offset for `\splittopskip`.
pub const GLUE_PAR_SPLIT_TOP_SKIP: usize = 10;
/// Glue parameter offset for `\tabskip`.
pub const GLUE_PAR_TAB_SKIP: usize = 11;
/// Glue parameter offset for `\spaceskip`.
pub const GLUE_PAR_SPACE_SKIP: usize = 12;
/// Glue parameter offset for `\xspaceskip`.
pub const GLUE_PAR_XSPACE_SKIP: usize = 13;
/// Glue parameter offset for `\parfillskip`.
pub const GLUE_PAR_PAR_FILL_SKIP: usize = 14;
/// Glue parameter offset for `\XeTeXlinebreakskip`.
pub const GLUE_PAR_XETEX_LINEBREAK_SKIP: usize = 15;

/// Character code-table selector for `\catcode`.
pub const CODE_TABLE_CAT: u8 = 0;
/// Character code-table selector for `\lccode`.
pub const CODE_TABLE_LC: u8 = 1;
/// Character code-table selector for `\uccode`.
pub const CODE_TABLE_UC: u8 = 2;
/// Character code-table selector for `\sfcode` and `\XeTeXcharclass`.
pub const CODE_TABLE_SF: u8 = 3;
/// Character code-table selector for `\mathcode` and XeTeX math-code forms.
pub const CODE_TABLE_MATH: u8 = 4;
/// Character code-table selector for `\delcode` and XeTeX delimiter-code forms.
pub const CODE_TABLE_DEL: u8 = 5;
/// Number of compact character code tables.
pub const CODE_TABLE_COUNT: usize = 6;
/// Maximum XeTeX character class (`CHAR_CLASS_LIMIT`).
pub const CHAR_CLASS_LIMIT: i32 = 4096;

/// C eqtb base address for `\catcode` entries.
pub const CAT_CODE_BASE: i32 = 0x226D29;
/// C eqtb base address for `\lccode` entries.
pub const LC_CODE_BASE: i32 = 0x336D29;
/// C eqtb base address for `\uccode` entries.
pub const UC_CODE_BASE: i32 = 0x446D29;
/// C eqtb base address for `\sfcode` entries.
pub const SF_CODE_BASE: i32 = 0x556D29;
/// C eqtb base address for `\mathcode` entries.
pub const MATH_CODE_BASE: i32 = 0x666D29;
/// C eqtb base address for `\delcode` entries.
pub const DEL_CODE_BASE: i32 = 0x776E7C;

/// Bottom-level group code (`BOTTOM_LEVEL`).
pub const BOTTOM_LEVEL: u8 = 0;
/// Simple `{...}` group (`SIMPLE_GROUP`).
pub const SIMPLE_GROUP: u8 = 1;
/// `\hbox` group (`HBOX_GROUP`).
pub const HBOX_GROUP: u8 = 2;
/// Adjusted `\hbox` group (`ADJUSTED_HBOX_GROUP`).
pub const ADJUSTED_HBOX_GROUP: u8 = 3;
/// `\vbox` group (`VBOX_GROUP`).
pub const VBOX_GROUP: u8 = 4;
/// `\vtop` group (`VTOP_GROUP`).
pub const VTOP_GROUP: u8 = 5;
/// Alignment group (`ALIGN_GROUP`).
pub const ALIGN_GROUP: u8 = 6;
/// `\noalign` group (`NO_ALIGN_GROUP`).
pub const NO_ALIGN_GROUP: u8 = 7;
/// Output routine group (`OUTPUT_GROUP`).
pub const OUTPUT_GROUP: u8 = 8;
/// Inline/display math group (`MATH_GROUP`).
pub const MATH_GROUP: u8 = 9;
/// Discretionary group (`DISC_GROUP`).
pub const DISC_GROUP: u8 = 10;
/// Insertion group (`INSERT_GROUP`).
pub const INSERT_GROUP: u8 = 11;
/// `\vcenter` group (`VCENTER_GROUP`).
pub const VCENTER_GROUP: u8 = 12;
/// `\mathchoice` group (`MATH_CHOICE_GROUP`).
pub const MATH_CHOICE_GROUP: u8 = 13;
/// `\begingroup` group (`SEMI_SIMPLE_GROUP`).
pub const SEMI_SIMPLE_GROUP: u8 = 14;
/// Math-shift group (`MATH_SHIFT_GROUP`).
pub const MATH_SHIFT_GROUP: u8 = 15;
/// `\left`/`math_left` group (`MATH_LEFT_GROUP`).
pub const MATH_LEFT_GROUP: u8 = 16;
/// Maximum language code accepted by XeTeX's `\language` parameter.
pub const BIGGEST_LANG: i32 = 255;
/// One past the largest language code (`TOO_BIG_LANG`).
pub const TOO_BIG_LANG: i32 = 256;
/// Hash modulus of the hyphenation exception table (`HYPH_PRIME`).
pub const HYPH_PRIME: i32 = 607;
/// Size of the hyphenation exception table (`hyph_size`).
pub const HYPH_SIZE: usize = 8191;
/// Maximum number of hyphenation ops (`TRIE_OP_SIZE`).
pub const TRIE_OP_SIZE: i32 = 35111;
/// Negative op-hash bound (`NEG_TRIE_OP_SIZE`).
pub const NEG_TRIE_OP_SIZE: i32 = -TRIE_OP_SIZE;
/// Empty op code (`MIN_TRIE_OP`).
pub const MIN_TRIE_OP: i32 = 0;
/// Largest op code (`MAX_TRIE_OP`).
pub const MAX_TRIE_OP: i32 = 65535;
/// Packed-trie capacity (`trie_size`).
pub const FMT_TRIE_SIZE: i32 = 1_000_000;

// === Format-file (version 33) C-layout constants ===
// These mirror the `xetex_format` v33 symbol table: the serialization
// image materialized by `store_fmt_file`/`load_fmt_file` uses C's absolute
// eqtb/hash layout even though the live engine uses compact tables.
/// Number of Unicode scalar values (`NUMBER_USVS`).
pub const C_NUMBER_USVS: i32 = 0x11_0000;
/// Number of registers per class (`NUMBER_REGS`).
pub const C_NUMBER_REGS: i32 = 256;
/// Number of math-family slots (`NUMBER_MATH_FONTS` = 3 * 256).
pub const C_NUMBER_MATH_FONTS: i32 = 768;
/// Control-sequence hash modulus (`HASH_PRIME`).
pub const C_HASH_PRIME: i32 = 8501;
/// Control-sequence hash table size (`HASH_SIZE`).
pub const C_HASH_SIZE: i32 = 15000;
/// Offset between `yhash` storage and `hash` indices (`hash_offset`).
pub const C_HASH_OFFSET: i32 = 514;
/// Extra hash slots above `EQTB_SIZE` (`hash_extra`).
pub const C_HASH_EXTRA: i32 = 600_000;
/// Primitive hash table size (`PRIM_SIZE`).
pub const C_PRIM_SIZE: i32 = 2100;
/// Primitive hash modulus (`PRIM_PRIME`).
pub const C_PRIM_PRIME: i32 = 1777;
/// First primitive hash slot (`PRIM_BASE`).
pub const C_PRIM_BASE: i32 = 1;
/// Undefined-primitive sentinel (`UNDEFINED_PRIMITIVE`).
pub const C_UNDEFINED_PRIMITIVE: i32 = 0;
/// Maximum font number (`MAX_FONT_MAX`).
pub const C_MAX_FONT_MAX: i32 = 9000;
/// Highest dynamic-memory index (`MEM_TOP`).
pub const C_MEM_TOP: i32 = 4_999_999;
/// First eqtb slot: active characters (`ACTIVE_BASE`).
pub const C_ACTIVE_BASE: i32 = 1;
/// One-letter control sequences (`SINGLE_BASE`).
pub const C_SINGLE_BASE: i32 = C_ACTIVE_BASE + C_NUMBER_USVS;
/// The null control sequence (`NULL_CS`).
pub const C_NULL_CS: i32 = C_SINGLE_BASE + C_NUMBER_USVS;
/// Multiletter hash-table base (`HASH_BASE`).
pub const C_HASH_BASE: i32 = C_NULL_CS + 1;
/// First frozen control sequence (`FROZEN_CONTROL_SEQUENCE`).
pub const C_FROZEN_CONTROL_SEQUENCE: i32 = C_HASH_BASE + C_HASH_SIZE;
/// Frozen `\primitive`-protected cs (`FROZEN_PROTECTION`).
pub const C_FROZEN_PROTECTION: i32 = C_FROZEN_CONTROL_SEQUENCE;
/// Frozen `\cr` (`FROZEN_CR`).
pub const C_FROZEN_CR: i32 = C_FROZEN_CONTROL_SEQUENCE + 1;
/// Frozen `\endgroup` (`FROZEN_END_GROUP`).
pub const C_FROZEN_END_GROUP: i32 = C_FROZEN_CONTROL_SEQUENCE + 2;
/// Frozen `\right` (`FROZEN_RIGHT`).
pub const C_FROZEN_RIGHT: i32 = C_FROZEN_CONTROL_SEQUENCE + 3;
/// Frozen `\fi` (`FROZEN_FI`).
pub const C_FROZEN_FI: i32 = C_FROZEN_CONTROL_SEQUENCE + 4;
/// Frozen `\endtemplate` (`FROZEN_END_TEMPLATE`).
pub const C_FROZEN_END_TEMPLATE: i32 = C_FROZEN_CONTROL_SEQUENCE + 5;
/// Frozen `\endv` (`FROZEN_ENDV`).
pub const C_FROZEN_ENDV: i32 = C_FROZEN_CONTROL_SEQUENCE + 6;
/// Frozen `\relax` (`FROZEN_RELAX`).
pub const C_FROZEN_RELAX: i32 = C_FROZEN_CONTROL_SEQUENCE + 7;
/// Frozen `\endwrite` token (`END_WRITE`).
pub const C_END_WRITE: i32 = C_FROZEN_CONTROL_SEQUENCE + 8;
/// Frozen `\notexpanded:` (`FROZEN_DONT_EXPAND`).
pub const C_FROZEN_DONT_EXPAND: i32 = C_FROZEN_CONTROL_SEQUENCE + 9;
/// Frozen `\special` (`FROZEN_SPECIAL`).
pub const C_FROZEN_SPECIAL: i32 = C_FROZEN_CONTROL_SEQUENCE + 10;
/// Frozen `\primitive` (`FROZEN_PRIMITIVE`).
pub const C_FROZEN_PRIMITIVE: i32 = C_FROZEN_CONTROL_SEQUENCE + 11;
/// Primitive eqtb region (`PRIM_EQTB_BASE`).
pub const C_PRIM_EQTB_BASE: i32 = C_FROZEN_CONTROL_SEQUENCE + 12;
/// Frozen `\nullfont` (`FROZEN_NULL_FONT` = `FONT_ID_BASE`).
pub const C_FROZEN_NULL_FONT: i32 = C_PRIM_EQTB_BASE + C_PRIM_SIZE;
/// Font-id eqtb region (`FONT_ID_BASE`).
pub const C_FONT_ID_BASE: i32 = C_FROZEN_NULL_FONT;
/// Dummy eqtb slot for undefined control sequences
/// (`UNDEFINED_CONTROL_SEQUENCE`).
pub const C_UNDEFINED_CONTROL_SEQUENCE: i32 = C_FROZEN_NULL_FONT + C_MAX_FONT_MAX + 1;
/// Glue parameters (`GLUE_BASE`).
pub const C_GLUE_BASE: i32 = C_UNDEFINED_CONTROL_SEQUENCE + 1;
/// Number of glue parameters (v33 `GLUE_PARS`).
pub const C_GLUE_PARS: i32 = 19;
/// Skip registers (`SKIP_BASE`).
pub const C_SKIP_BASE: i32 = C_GLUE_BASE + C_GLUE_PARS;
/// Mu-skip registers (`MU_SKIP_BASE`).
pub const C_MU_SKIP_BASE: i32 = C_SKIP_BASE + C_NUMBER_REGS;
/// Local parameters (`LOCAL_BASE`).
pub const C_LOCAL_BASE: i32 = C_MU_SKIP_BASE + C_NUMBER_REGS;
/// Number of local parameters (v33 `NUM_LOCALS`).
pub const C_NUM_LOCALS: i32 = 13;
/// Token registers (`TOKS_BASE`).
pub const C_TOKS_BASE: i32 = C_LOCAL_BASE + C_NUM_LOCALS;
/// e-TeX penalty parameters (`ETEX_PEN_BASE`).
pub const C_ETEX_PEN_BASE: i32 = C_TOKS_BASE + C_NUMBER_REGS;
/// Number of e-TeX penalty parameters (`NUM_ETEX_PENALTIES`).
pub const C_NUM_ETEX_PENALTIES: i32 = 4;
/// Box registers (`BOX_BASE`).
pub const C_BOX_BASE: i32 = C_ETEX_PEN_BASE + C_NUM_ETEX_PENALTIES;
/// Current-font eqtb slot (`CUR_FONT_LOC`).
pub const C_CUR_FONT_LOC: i32 = C_BOX_BASE + C_NUMBER_REGS;
/// Math-family font eqtb region (`MATH_FONT_BASE`).
pub const C_MATH_FONT_BASE: i32 = C_CUR_FONT_LOC + 1;
/// `\catcode` table (`CAT_CODE_BASE`).
pub const C_CAT_CODE_BASE: i32 = C_MATH_FONT_BASE + C_NUMBER_MATH_FONTS;
/// `\lccode` table (`LC_CODE_BASE`).
pub const C_LC_CODE_BASE: i32 = C_CAT_CODE_BASE + C_NUMBER_USVS;
/// `\uccode` table (`UC_CODE_BASE`).
pub const C_UC_CODE_BASE: i32 = C_LC_CODE_BASE + C_NUMBER_USVS;
/// `\sfcode` table (`SF_CODE_BASE`).
pub const C_SF_CODE_BASE: i32 = C_UC_CODE_BASE + C_NUMBER_USVS;
/// `\mathcode` table (`MATH_CODE_BASE`).
pub const C_MATH_CODE_BASE: i32 = C_SF_CODE_BASE + C_NUMBER_USVS;
/// Integer parameters (`INT_BASE`; v33 omits `CHAR_SUB_CODE_BASE`).
pub const C_INT_BASE: i32 = C_MATH_CODE_BASE + C_NUMBER_USVS;
/// Number of integer parameters (v33 `INT_PARS`).
pub const C_INT_PARS: i32 = 83;
/// Count registers (`COUNT_BASE`).
pub const C_COUNT_BASE: i32 = C_INT_BASE + C_INT_PARS;
/// `\delcode` table (`DEL_CODE_BASE`).
pub const C_DEL_CODE_BASE: i32 = C_COUNT_BASE + C_NUMBER_REGS;
/// Dimension parameters (`DIMEN_BASE`).
pub const C_DIMEN_BASE: i32 = C_DEL_CODE_BASE + C_NUMBER_USVS;
/// Number of dimension parameters (v33 `DIMEN_PARS`).
pub const C_DIMEN_PARS: i32 = 23;
/// Scaled registers (`SCALED_BASE`).
pub const C_SCALED_BASE: i32 = C_DIMEN_BASE + C_DIMEN_PARS;
/// Highest eqtb index (`EQTB_SIZE`).
pub const C_EQTB_SIZE: i32 = C_SCALED_BASE + C_NUMBER_REGS - 1;
/// Highest eqtb index including extra hash (`EQTB_TOP`).
pub const C_EQTB_TOP: i32 = C_EQTB_SIZE + C_HASH_EXTRA;
/// Format-file magic (`FORMAT_HEADER_MAGIC` = `"TTNC"`).
pub const FORMAT_HEADER_MAGIC: i32 = 0x5454_4E43;
/// Format-file trailer magic (`FORMAT_FOOTER_MAGIC`).
pub const FORMAT_FOOTER_MAGIC: i32 = 0x0000_029A;
/// Format serial this engine reads and writes (`FORMAT_SERIAL` = v33).
pub const FORMAT_SERIAL: i32 = 33;
/// Upper bound on loaded string count (`sup_max_strings`).
pub const SUP_MAX_STRINGS: i32 = 2_097_151;
/// Upper bound on `fmem_ptr` (`sup_font_mem_size`).
pub const SUP_FONT_MEM_SIZE: i32 = 147_483_647;
/// Upper bound on `pool_ptr` (`sup_pool_size`).
pub const SUP_POOL_SIZE: i32 = 40_000_000;

/// Uninitialized equivalents-table level (`LEVEL_ZERO`).
pub const LEVEL_ZERO: u16 = 0;
/// Outer/global definition level (`LEVEL_ONE`).
pub const LEVEL_ONE: u16 = 1;

/// Restore the old saved value (`RESTORE_OLD_VALUE`).
pub const RESTORE_OLD_VALUE: u16 = 0;
/// Restore the undefined/zero value (`RESTORE_ZERO`).
pub const RESTORE_ZERO: u16 = 1;
/// Reinsert a token after a group (`INSERT_TOKEN`).
pub const INSERT_TOKEN: u16 = 2;
/// Save-stack group boundary (`LEVEL_BOUNDARY`).
pub const LEVEL_BOUNDARY: u16 = 3;
/// Sparse-array restoration marker (`RESTORE_SA`).
pub const RESTORE_SA: u16 = 4;

/// Target restored by a save-stack entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveTarget {
    /// A compact `eqtb` slot.
    Eqtb(usize),
    /// A compact control-sequence id.
    ControlSequence(i32),
    /// A character code-table entry.
    CharacterCode {
        /// Compact code-table selector.
        table: u8,
        /// Unicode scalar value being restored.
        character: u32,
    },
}

/// One save-stack slot, preserving the C stack's two-word shape where needed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStackEntry {
    /// Source-line slot preceding a level boundary.
    Line(i32),
    /// Group boundary carrying the enclosing group and boundary pointer.
    Boundary {
        /// Enclosing group code.
        group: u8,
        /// Boundary pointer below this save level.
        boundary: i32,
    },
    /// Saved value slot preceding its restore descriptor.
    SavedWord(MemoryWord),
    /// Raw integer slot used by packaging context (`scan_spec`).
    RawValue(i32),
    /// Restore descriptor for an `eqtb` slot or control sequence.
    Restore {
        /// Save-stack restore code (`RESTORE_OLD_VALUE` or `RESTORE_ZERO`).
        kind: u16,
        /// Level at which the saved value was defined.
        level: u16,
        /// Saved-value target.
        target: SaveTarget,
    },
    /// Token to reinsert after the group closes.
    InsertToken(i32),
    /// e-TeX sparse-array chain boundary.
    RestoreSa {
        /// Sparse-array definition level.
        level: u16,
        /// Previous sparse-array chain head.
        chain: i32,
    },
}

/// Current semantic list (`cur_list`) saved by `push_nest`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ListStateRecord {
    /// Execution mode.
    pub mode: i16,
    /// Head of the current list.
    pub head: i32,
    /// Tail of the current list.
    pub tail: i32,
    /// e-TeX auxiliary pointer (`eTeX_aux`).
    pub etex_aux: i32,
    /// Paragraph line count (`prev_graf`).
    pub prev_graf: i32,
    /// Source line where the list began (`mode_line`).
    pub mode_line: i32,
    /// Mode-dependent auxiliary word (`aux`).
    pub aux: MemoryWord,
}

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

/// Box width offset used by `\wd` (`WIDTH_OFFSET`).
pub const WIDTH_OFFSET: i32 = 1;
/// Box depth offset used by `\dp` (`DEPTH_OFFSET`).
pub const DEPTH_OFFSET: i32 = 2;
/// Box height offset used by `\ht` (`HEIGHT_OFFSET`).
pub const HEIGHT_OFFSET: i32 = 3;
/// Box shift amount offset (`SHIFT_AMOUNT`).
pub const SHIFT_AMOUNT: i32 = 4;
/// Box list pointer offset (`LIST_OFFSET`).
pub const LIST_OFFSET: i32 = 5;
/// Box glue-set floating-point word offset (`GLUE_SET`).
pub const GLUE_SET: i32 = 6;
/// Box glue-sign halfword offset (`GLUE_SIGN`).
pub const GLUE_SIGN: i32 = 5;
/// Box glue-order halfword offset (`GLUE_ORDER`).
pub const GLUE_ORDER: i32 = 5;
/// Box packaging mode for an exact requested size (`EXACTLY`).
pub const EXACTLY: i32 = 0;
/// Box packaging mode for an additional spread (`ADDITIONAL`).
pub const ADDITIONAL: i32 = 1;
/// Box glue sign for natural dimensions (`NORMAL`).
pub const GLUE_NORMAL: u16 = 0;
/// Box glue sign for stretching (`STRETCHING`).
pub const STRETCHING: u16 = 1;
/// Box glue sign for shrinking (`SHRINKING`).
pub const SHRINKING: u16 = 2;
/// Unhyphenated active-list node marker (`UNHYPHENATED`).
pub const UNHYPHENATED: u16 = 0;
/// Hyphenated active-list node marker (`HYPHENATED`).
pub const HYPHENATED: u16 = 1;
/// Line-breaking fitness class: very loose (`VERY_LOOSE_FIT`).
pub const VERY_LOOSE_FIT: u16 = 0;
/// Line-breaking fitness class: loose (`LOOSE_FIT`).
pub const LOOSE_FIT: u16 = 1;
/// Line-breaking fitness class: decent (`DECENT_FIT`).
pub const DECENT_FIT: u16 = 2;
/// Line-breaking fitness class: tight (`TIGHT_FIT`).
pub const TIGHT_FIT: u16 = 3;
/// Demerits at which every break is acceptable (`AWFUL_BAD`).
pub const AWFUL_BAD: i32 = MAX_HALFWORD;
/// Active-list delta nodes share the `RULE_NODE` type code (`DELTA_NODE`).
pub const DELTA_NODE: u16 = RULE_NODE;
/// Passive breakpoint node size (`PASSIVE_NODE_SIZE`).
pub const PASSIVE_NODE_SIZE: i32 = 2;
/// Ordinary active node size (`ACTIVE_NODE_SIZE_NORMAL`).
pub const ACTIVE_NODE_SIZE_NORMAL: i32 = 3;
/// Extended active node size used by `\lastlinefit`
/// (`ACTIVE_NODE_SIZE_EXTENDED`).
pub const ACTIVE_NODE_SIZE_EXTENDED: i32 = 5;
/// Active-list delta node size (`DELTA_NODE_SIZE`).
pub const DELTA_NODE_SIZE: i32 = 7;
/// Maximum hlist nesting tracked by character protrusion
/// (`MAX_HLIST_STACK`).
pub const MAX_HLIST_STACK: usize = 512;
/// Longest hyphenatable word (`HYPHENATABLE_LENGTH_LIMIT`).
pub const HYPHENATABLE_LENGTH_LIMIT: i32 = 4095;
/// Sentinel hyphenation code used at word edges (`max_hyph_char`).
pub const MAX_HYPH_CHAR: i32 = 0x11_0000;
/// Page-insertion tracking marker (`INSERTING`).
pub const INSERTING: u16 = 0;
/// Split insertion marker (`SPLIT_UP`).
pub const SPLIT_UP: u16 = 1;
/// Words allocated for a page-insertion tracking node.
pub const PAGE_INS_NODE_SIZE: i32 = 4;
/// Leader glue subtype base (`A_LEADERS`).
pub const A_LEADERS: i32 = 100;
/// Leader glue subtype for centered leaders (`C_LEADERS`).
pub const C_LEADERS: u16 = 101;
/// Leader glue subtype for expanded leaders (`X_LEADERS`).
pub const X_LEADERS: u16 = 102;
/// Box context flag for `\setbox` (`BOX_FLAG`).
pub const BOX_FLAG: i32 = 0x4000_0000;
/// Box context flag for `\global\setbox` (`GLOBAL_BOX_FLAG`).
pub const GLOBAL_BOX_FLAG: i32 = 0x4000_8000;
/// Box context flag for `\shipout` (`SHIP_OUT_FLAG`).
pub const SHIP_OUT_FLAG: i32 = 0x4001_0000;
/// Box context flag for leaders (`LEADER_FLAG`).
pub const LEADER_FLAG: i32 = 0x4001_0001;
/// `\MakeBox` subtype for `\box` (`BOX_CODE`).
pub const BOX_CODE: i32 = 0;
/// `\MakeBox` subtype for `\copy` (`COPY_CODE`).
pub const COPY_CODE: i32 = 1;
/// `\MakeBox` subtype for `\lastbox` (`LAST_BOX_CODE`).
pub const LAST_BOX_CODE: i32 = 2;
/// `\MakeBox` subtype for `\vsplit` (`VSPLIT_CODE`).
pub const VSPLIT_CODE: i32 = 3;
/// `\MakeBox` subtype boundary for `\vtop` (`VTOP_CODE`).
pub const VTOP_CODE: i32 = 4;
/// Glue-command subtype for explicit `\hskip`/`\vskip` (`SKIP_CODE`).
pub const SKIP_CODE: i32 = 4;
/// Glue-command subtype for explicit `\mskip` (`MSKIP_CODE`).
pub const MSKIP_CODE: i32 = 5;
/// Glue-node subtype for math glue (`MU_GLUE`).
pub const MU_GLUE: u16 = 99;
/// Sentinel previous depth suppressing interline glue (`IGNORE_DEPTH`).
pub const IGNORE_DEPTH: i32 = -65_536_000;

/// Batch interaction mode (`BATCH_MODE`).
pub const BATCH_MODE: u8 = 0;
/// Nonstop interaction mode (`NONSTOP_MODE`).
pub const NONSTOP_MODE: u8 = 1;
/// Scroll interaction mode (`SCROLL_MODE`).
pub const SCROLL_MODE: u8 = 2;
/// Error-stop interaction mode (`ERROR_STOP_MODE`).
pub const ERROR_STOP_MODE: u8 = 3;

/// Selector that suppresses diagnostic output (`SELECTOR_NO_PRINT`).
pub const SELECTOR_NO_PRINT: i32 = 16;
/// Selector that writes diagnostics only to the terminal (`SELECTOR_TERM_ONLY`).
pub const SELECTOR_TERM_ONLY: i32 = 17;
/// Selector that writes diagnostics only to the log (`SELECTOR_LOG_ONLY`).
pub const SELECTOR_LOG_ONLY: i32 = 18;
/// Selector that writes diagnostics to terminal and log (`SELECTOR_TERM_AND_LOG`).
pub const SELECTOR_TERM_AND_LOG: i32 = 19;
/// Selector that accumulates text into the string pool (`SELECTOR_NEW_STRING`).
pub const SELECTOR_NEW_STRING: i32 = 21;

/// Selector that records text into the show-context trick buffer (`SELECTOR_PSEUDO`).
pub const SELECTOR_PSEUDO: i32 = 20;

/// Largest code point representable as a bare `str_number` (`BIGGEST_CHAR`).
pub const BIGGEST_CHAR: i32 = 0xFFFF;
/// Largest Unicode scalar value (`BIGGEST_USV`).
pub const BIGGEST_USV: i32 = 0x10FFFF;
/// `str_number` of the empty string constant (`EMPTY_STRING`).
pub const EMPTY_STRING: i32 = TOO_BIG_CHAR + 1;

/// No errors seen yet (`HISTORY_SPOTLESS`).
pub const HISTORY_SPOTLESS: u8 = 0;
/// A warning has been issued (`HISTORY_WARNING_ISSUED`).
pub const HISTORY_WARNING_ISSUED: u8 = 1;
/// A recoverable error has been issued (`HISTORY_ERROR_ISSUED`).
pub const HISTORY_ERROR_ISSUED: u8 = 2;
/// A fatal error has been issued (`HISTORY_FATAL_ERROR`).
pub const HISTORY_FATAL_ERROR: u8 = 3;

/// `\errorcontextlines` integer parameter (`INT_PAR__error_context_lines`).
pub const INT_PAR_ERROR_CONTEXT_LINES: usize = 54;

/// Empty page-builder contents (`EMPTY`).
pub const PAGE_EMPTY: u8 = 0;
/// Page contains insertions only (`INSERTS_ONLY`).
pub const PAGE_INSERTS_ONLY: u8 = 1;
/// Page contains a boxed contribution (`BOX_THERE`).
pub const PAGE_BOX_THERE: u8 = 2;
/// Number of page-builder dimension slots (`page_so_far`).
pub const PAGE_SO_FAR_COUNT: usize = 8;

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
    XetexMathGiven = 70,
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
    XetexDefCode = 87,
    DefFamily = 88,
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
            70 => Some(Self::XetexMathGiven),
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
            87 => Some(Self::XetexDefCode),
            88 => Some(Self::DefFamily),
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

/// State-owned output stream used by `\openout`, `\write`, and `\closeout`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteStream {
    /// Packed TeX file name associated with the stream.
    pub name: String,
    /// Text written to the stream by `\write`.
    pub text: String,
}

/// Positioned-glyph record attached to a native-word or glyph node; the
/// memory-safe equivalent of C's `native_glyph_info_ptr` buffer: one
/// `FixedPoint` location plus a glyph id per glyph
/// (`native_glyph_info_size = 10` bytes serialized).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeGlyphLoc {
    /// Horizontal offset of the glyph origin in scaled points.
    pub x: i32,
    /// Vertical offset of the glyph origin in scaled points.
    pub y: i32,
    /// Glyph index in the node's font.
    pub glyph: u16,
}

/// Per-glyph positioning data attached to a native-word node
/// (`native_glyph_info_ptr` + `native_glyph_count`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NativeGlyphInfo {
    /// Locations and glyph ids, parallel to C's `locations`/`glyphIDs`
    /// arrays.
    pub glyphs: Vec<NativeGlyphLoc>,
}

/// Per-glyph metrics reported by a native layout engine
/// (`getGlyphWidthFromEngine`, `getGlyphHeightDepth`, and
/// `getGlyphSidebearings` data).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativeGlyphMetric {
    /// Advance width in scaled points.
    pub width: i32,
    /// Height above the baseline in scaled points (pre snap-zone).
    pub height: i32,
    /// Depth below the baseline in scaled points (pre snap-zone).
    pub depth: i32,
    /// Left sidebearing in scaled points.
    pub lsb: i32,
    /// Right sidebearing in scaled points.
    pub rsb: i32,
}

/// One loaded font's state (`font_name`, `font_size`, `font_info`,
/// TFM lig/kern program instruction (one `font_info` LIG/KERN word).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LigKernInsn {
    /// `skip_byte` — program steps to skip when `next_char` does not
    /// match; `>= 128` also marks the program's end.
    pub skip: u8,
    /// `next_char` — the lookahead character this instruction matches.
    pub next_char: i32,
    /// `op_byte` — ligature operation (`< 128`) or kern flag (`>= 128`).
    pub op: u8,
    /// `remainder` — replacement character (`op < 128`) or kern index
    /// component (`op >= 128`).
    pub remainder: i32,
}

/// char_info tag: plain character (`0`).
pub const CHAR_TAG_NONE: u8 = 0;
/// char_info tag: lig/kern program (`LIG_TAG`).
pub const LIG_TAG: u8 = 1;
/// char_info tag: next-larger character (`LIST_TAG`).
pub const LIST_TAG: u8 = 2;
/// char_info tag: extensible recipe (`EXT_TAG`).
pub const EXT_TAG: u8 = 3;

/// Sentinel program index meaning "no boundary-char program"
/// (`NON_ADDRESS`).
pub const NON_ADDRESS: i32 = 0;
/// Sentinel character meaning "no boundary character" (`TOO_BIG_CHAR`).
pub const TOO_BIG_CHAR: i32 = 65536;

/// One glyph part of an OpenType MATH glyph assembly
/// (`GlyphAssembly` part record).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OtMathPart {
    /// Glyph id stacked for this part (`ot_part_glyph`).
    pub glyph: u32,
    /// Whether the part repeats to fill the target (`ot_part_is_extender`).
    pub is_extender: bool,
    /// Scaled start connector length (`ot_part_start_connector`).
    pub start_connector: i32,
    /// Scaled end connector length (`ot_part_end_connector`).
    pub end_connector: i32,
    /// Scaled full advance (`ot_part_full_advance`).
    pub full_advance: i32,
}

/// An OpenType MATH glyph assembly (`GlyphAssembly`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OtMathAssembly {
    /// Ordered glyph parts (`ot_part_count` is `parts.len()`).
    pub parts: Vec<OtMathPart>,
}

/// Per-font metric and auxiliary data (`font_name`, `font_area`,
/// `font_size`, `font_dsize`, `font_check`, `font_params`,
/// `hyphen_char`, `skew_char`, and the character-protrusion tables in C).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontRecord {
    /// Packed file-name stem (`font_name`).
    pub name: Vec<i32>,
    /// Packed file-name area (`font_area` for TFM fonts; native fonts use the
    /// [`AAT_FONT_FLAG`]/[`OTGR_FONT_FLAG`] sentinels in
    /// [`TeXEngineState::font_area`]).
    pub area: Vec<i32>,
    /// Packed file-name extension (`cur_ext` at load time).
    pub ext: Vec<i32>,
    /// Requested size in scaled points (`font_size`).
    pub size: i32,
    /// Design size in scaled points (`font_dsize`).
    pub dsize: i32,
    /// TFM checksum (`font_check`).
    pub checksum: u32,
    /// `\fontdimen` parameters; `params[0]` is unused so code `n` maps to
    /// `params[n]` (`font_params` is `params.len() - 1`).
    pub params: Vec<i32>,
    /// Default hyphenation character (`hyphen_char`).
    pub hyphen_char: i32,
    /// Math skew character (`skew_char`).
    pub skew_char: i32,
    /// Left character-protrusion codes keyed by character or glyph id.
    pub lp_codes: HashMap<u32, i32>,
    /// Right character-protrusion codes keyed by character or glyph id.
    pub rp_codes: HashMap<u32, i32>,
    /// Whether the font exposes an OpenType MATH table
    /// (`isOpenTypeMathFont` input).
    pub has_math_table: bool,
    /// OpenType MATH symbol-parameter values (`get_native_mathsy_param`),
    /// keyed by parameter index.
    pub mathsy_params: HashMap<i32, i32>,
    /// OpenType MATH extension-parameter values (`get_native_mathex_param`),
    /// keyed by parameter index.
    pub mathex_params: HashMap<i32, i32>,
    /// OpenType MATH table constants (`get_ot_math_constant`), keyed by
    /// constant id.
    pub math_constants: HashMap<i32, i32>,
    /// Math glyph variants keyed by `(character, horizontal flag, step)`:
    /// the layout engine's `get_ot_math_variant` results `(glyph, advance)`.
    pub math_variants: HashMap<(u32, u8, i32), (u32, i32)>,
    /// Per-glyph math italic corrections (`get_ot_math_ital_corr`).
    pub math_ital_corr: HashMap<u32, i32>,
    /// Per-glyph top-accent attachment positions
    /// (`get_ot_math_accent_pos`); missing glyphs yield `TEX_INFINITY`.
    pub accent_positions: HashMap<u32, i32>,
    /// Post-blend math kern tables keyed by `(base glyph, script glyph,
    /// SUP_CMD|SUB_CMD)`: ascending `(correction height, kern)` pairs.
    pub math_kerns: HashMap<(u32, u32, u8), Vec<(i32, i32)>>,
    /// Extensible glyph assemblies keyed by `(glyph, horizontal flag)`
    /// (`get_ot_assembly_ptr` data).
    pub math_assemblies: HashMap<(u32, u8), OtMathAssembly>,
    /// Minimum connector overlap for glyph assemblies
    /// (`ot_min_connector_overlap`).
    pub min_connector_overlap: i32,
    /// TFM extensible recipes (`font_info[exten_base]`): recipe index maps
    /// to `[rep, top, bot, mid]` character codes.
    pub exten_recipes: HashMap<i32, [i32; 4]>,
    /// Fallback glyph height when glyph metrics are not requested
    /// (`height_base`).
    pub height_base: i32,
    /// Fallback glyph depth when glyph metrics are not requested
    /// (`depth_base`).
    pub depth_base: i32,
    /// First valid character code (`font_bc`).
    pub bc: u32,
    /// Last valid character code (`font_ec`).
    pub ec: u32,
    /// Interword glue specification node (`font_glue`), `TEX_NULL` when absent.
    pub glue: i32,
    /// Whether the font has contributed characters to a list (`font_used`).
    pub used: bool,
    /// Name of the control sequence that defined the font
    /// (`hash[FONT_ID_BASE + f]`).
    pub cs_name: Vec<i32>,
    /// Native glyph-name to glyph-index map (`map_glyph_to_index` data).
    pub glyph_names: HashMap<String, u32>,
    /// Native glyph-index to name map (`getGlyphName`/`print_glyph_name`
    /// data).
    pub glyph_id_names: HashMap<u32, String>,
    /// Unicode scalar to glyph-index map (`map_char_to_glyph` data).
    pub char_to_glyph: HashMap<u32, u32>,
    /// Per-glyph metrics for native fonts keyed by glyph id.
    pub glyph_metrics: HashMap<u32, NativeGlyphMetric>,
    /// Native glyph count (`\XeTeXcountglyphs` result).
    pub glyph_count: i32,
    /// Letter-space adjustment in scaled points (`font_letter_space`).
    pub letter_space: i32,
    /// Native engine class reported by `\XeTeXfonttype` for `OTGR` fonts:
    /// `2` OpenType, `3` Graphite, `0` unknown.
    pub font_type: i32,
    /// Font flag bits from the layout engine (`font_flags`:
    /// [`FONT_FLAGS_COLORED`]/[`FONT_FLAGS_VERTICAL`]).
    pub font_flags: u16,
    /// Source file name reported by `getFontFilename` for XDV font
    /// definitions.
    pub layout_filename: String,
    /// Face index within the font file (`getFontFilename` index output).
    pub layout_face_index: u32,
    /// Foreground color as packed RGBA (`getRgbValue`).
    pub layout_rgba: u32,
    /// Horizontal scale factor in `D2Fix` form (`getExtendFactor`;
    /// `65536` = 1.0 = none).
    pub layout_extend: i32,
    /// Slant factor in `D2Fix` form (`getSlantFactor`; `0` = none).
    pub layout_slant: i32,
    /// Embolden factor in `D2Fix` form (`getEmboldenFactor`; `0` = none).
    pub layout_embolden: i32,
    /// Point size in scaled points (`getPointSize` as `D2Fix`).
    pub layout_point_size: i32,
    /// TFM lig/kern program (`font_info` LIG/KERN words); index 0 is the
    /// `NON_ADDRESS` sentinel slot.
    pub lig_kern: Vec<LigKernInsn>,
    /// Resolved kern values addressed by `256 * op + remainder` (the TFM
    /// `kern_base` index folded into the seam).
    pub kerns: Vec<i32>,
    /// Per-character `char_info` tag nibble (`CHAR_TAG_NONE`, `LIG_TAG`,
    /// `LIST_TAG`, `EXT_TAG`), keyed by effective character.
    pub char_tags: HashMap<u32, u8>,
    /// Per-character `char_info` remainder: lig/kern program start,
    /// next-larger character, or extensible recipe index, keyed by
    /// effective character.
    pub char_remainders: HashMap<u32, i32>,
    /// `bchar_label[f]` — boundary-char program start index, or
    /// `NON_ADDRESS`.
    pub bchar_label: i32,
    /// `font_bchar` — boundary character, `TOO_BIG_CHAR` when absent.
    pub bchar: i32,
    /// `font_false_bchar`.
    pub false_bchar: i32,
    /// `font_mapping` text substitution pairs (the `apply_mapping` seam):
    /// greedy longest-match UTF-16 rewrites applied to collected words
    /// before metrics are computed.
    pub text_mapping: Vec<(Vec<u16>, Vec<u16>)>,
    /// Name of the `font_mapping` converter attached at load time
    /// (`mapping=` feature or `:mapping=` TFM suffix); resolves through
    /// [`TeXEngineState::font_mappings`].
    pub mapping_name: Option<String>,
    /// `tfm_font_mapping` character substitution (the `effective_char`
    /// seam).
    pub tfm_font_mapping: HashMap<u32, u32>,
}

impl Default for FontRecord {
    fn default() -> Self {
        Self {
            name: Vec::new(),
            area: Vec::new(),
            ext: Vec::new(),
            size: 0,
            dsize: 10 * 65_536,
            checksum: 0,
            params: vec![0; 8],
            hyphen_char: i32::from(b'-'),
            skew_char: -1,
            lp_codes: HashMap::new(),
            rp_codes: HashMap::new(),
            bc: 0,
            ec: 0,
            glue: TEX_NULL,
            used: false,
            cs_name: Vec::new(),
            glyph_names: HashMap::new(),
            glyph_id_names: HashMap::new(),
            char_to_glyph: HashMap::new(),
            glyph_metrics: HashMap::new(),
            glyph_count: 0,
            letter_space: 0,
            font_type: 0,
            font_flags: 0,
            layout_filename: String::new(),
            layout_face_index: 0,
            layout_rgba: 0,
            layout_extend: 65_536,
            layout_slant: 0,
            layout_embolden: 0,
            layout_point_size: 0,
            has_math_table: false,
            mathsy_params: HashMap::new(),
            mathex_params: HashMap::new(),
            math_constants: HashMap::new(),
            math_variants: HashMap::new(),
            math_ital_corr: HashMap::new(),
            accent_positions: HashMap::new(),
            math_kerns: HashMap::new(),
            math_assemblies: HashMap::new(),
            min_connector_overlap: 0,
            exten_recipes: HashMap::new(),
            height_base: 0,
            depth_base: 0,
            lig_kern: Vec::new(),
            kerns: Vec::new(),
            char_tags: HashMap::new(),
            char_remainders: HashMap::new(),
            bchar_label: NON_ADDRESS,
            bchar: TOO_BIG_CHAR,
            false_bchar: TOO_BIG_CHAR,
            text_mapping: Vec::new(),
            mapping_name: None,
            tfm_font_mapping: HashMap::new(),
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
    /// Definition level associated with each category code.
    pub cat_code_levels: Vec<u16>,
    /// Sparse character code tables for lc/uc/sf/math/del codes.
    pub code_tables: [HashMap<u32, MemoryWord>; CODE_TABLE_COUNT],
    /// Character metrics indexed by `(font, Unicode scalar)`.
    pub font_char_metrics: HashMap<(i32, u32), FontCharMetric>,
    /// Font area/native-layout flags indexed by internal font number.
    pub font_area: HashMap<i32, u16>,
    /// Loaded font records indexed by internal font number; slot 0 is
    /// `nullfont` (`font_ptr == font_table.len() - 1`).
    pub font_table: Vec<FontRecord>,
    /// Deterministic loadable font templates registered by embedders, keyed
    /// by `area + name` text (the `read_font_info` backend seam).
    pub font_sources: HashMap<String, FontRecord>,
    /// Hidden control-sequence id used as the `IDENT_VAL` token for each font
    /// (`FONT_ID_BASE + f`), or -1 until created.
    pub font_id_cs: Vec<i32>,
    /// Deterministic picture bounds registered by embedders, as `(x, y, width, height)`.
    pub picture_bounds: HashMap<String, (f64, f64, f64, f64)>,
    /// Parsed feature strings per loaded native font (`font_layout_engine`
    /// payload): script/language/shaper selection plus feature settings.
    pub font_specs: HashMap<i32, crate::fonts::FontSpec>,
    /// Raw `.tec` mapping bytes registered by embedders, keyed by mapping
    /// name (the `load_mapping_file` backend seam).
    pub tec_sources: HashMap<String, Vec<u8>>,
    /// Loaded TECkit/text converters keyed by mapping name
    /// (`font_mapping[f]` resolution seam).
    pub font_mappings: HashMap<String, crate::fonts::FontMapping>,
    /// Font-area flag of the most recently loaded native font
    /// (`native_font_type_flag`).
    pub native_font_type_flag: u16,
    /// Mapping name produced by the last `load_ot_font` `mapping=` feature
    /// (`loaded_font_mapping`); `None` when no converter is attached.
    pub loaded_font_mapping: Option<String>,
    /// Font flags produced by the last `load_ot_font` (`loaded_font_flags`).
    pub loaded_font_flags: u16,
    /// Letter-space produced by the last `load_ot_font`
    /// (`loaded_font_letter_space`), in scaled points.
    pub loaded_font_letter_space: i32,
    /// Design size of the most recently loaded native font
    /// (`loaded_font_design_size`), in scaled points.
    pub loaded_font_design_size: i32,
    /// Requested rendering engine for the current font spec
    /// (`getReqEngine`/`setReqEngine`): `b'A'`, `b'O'`, `b'G'`, or 0.
    pub req_engine: u8,
    /// Saved `:mapping=` name captured by `check_for_tfm_font_mapping`
    /// (`saved_mapping_name`).
    pub saved_mapping_name: Option<String>,
    /// Resolved scaled size from the `find_native_font` dsize probe (C's
    /// `scaled_size` out-parameter update).
    pub find_native_font_scaled: i32,
    /// Font spec produced by `find_native_font`→`load_ot_font`, awaiting
    /// attachment to the loaded font slot in `load_native_font`.
    pub pending_font_spec: Option<crate::fonts::FontSpec>,

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
    /// Mode-dependent auxiliary word (`cur_list.aux`).
    pub aux: MemoryWord,
    /// e-TeX auxiliary pointer (`cur_list.eTeX_aux`).
    pub etex_aux: i32,
    /// Source line at which the current list began (`mode_line`).
    pub mode_line: i32,
    /// Semantic-list nest below `nest_ptr`.
    pub nest: Vec<ListStateRecord>,
    /// Number of active entries in `nest`.
    pub nest_ptr: usize,
    /// Deepest semantic-nest level reached so far.
    pub max_nest_stack: usize,
    /// Maximum number of semantic-list nest entries.
    pub nest_size: usize,
    /// Page-builder dimension registers (`page_so_far`).
    pub page_so_far: [i32; PAGE_SO_FAR_COUNT],
    /// Number of consecutive failed output cycles (`dead_cycles`).
    pub dead_cycles: i32,
    /// Set when `its_all_over` ends the job (C `main_control` return path).
    pub job_stopped: bool,
    /// `\setbox` permission inside `do_assignments` (`set_box_allowed`).
    pub set_box_allowed: bool,
    /// Whether box contents may be deleted during error recovery
    /// (`deletions_allowed`).
    pub deletions_allowed: bool,
    /// Whether an insertion is being split (`ins_disc`).
    pub ins_disc: bool,
    /// Unclosed output-file-name parentheses (`open_parens`).
    pub open_parens: i32,
    /// Mode shown by `\showlists`/`trapnl` diagnostics (`shown_mode`).
    pub shown_mode: i32,
    /// First character of the editable portion of `name_of_file`
    /// (`edit_name_start`).
    pub edit_name_start: i32,
    /// Current typesetting font (`cur_f`).
    pub cur_f: i32,
    /// Whether the current character node is a ligature (`ligature_present`).
    pub ligature_present: bool,
    /// Whether the discretionary's left arm hit (`lft_hit`).
    pub lft_hit: bool,
    /// Whether the discretionary's right arm hit (`rt_hit`).
    pub rt_hit: bool,
    /// Penalty count accumulated by insertions (`insert_penalties`).
    pub insert_penalties: i32,
    /// Current interaction mode.
    pub interaction: u8,
    /// Diagnostic output selector.
    pub selector: i32,
    /// Whether the transcript/log file has been opened.
    pub log_opened: bool,
    /// State-owned terminal/log text emitted by message and diagnostic commands.
    pub transcript: Vec<String>,
    /// Flat UTF-16 string pool (`str_pool`); one entry per code unit.
    pub str_pool: Vec<i32>,
    /// Per-string start offsets (`str_start`), indexed by
    /// `str_number - TOO_BIG_CHAR`.
    pub str_start: Vec<i32>,
    /// String-pool write cursor (`pool_ptr`).
    pub pool_ptr: i32,
    /// One past the highest allocated string number (`str_ptr`).
    pub str_ptr: i32,
    /// Enforced pool capacity (`pool_size`).
    pub pool_size: i32,
    /// Enforced string-table capacity (`max_strings`).
    pub max_strings: i32,
    /// `pool_ptr` after engine initialization (`init_pool_ptr`).
    pub init_pool_ptr: i32,
    /// `str_ptr` after engine initialization (`init_str_ptr`).
    pub init_str_ptr: i32,
    /// Pool space reserved for the runtime (`string_vacancies`).
    pub string_vacancies: i32,
    /// Pool headroom allowed on top of a dumped format (`pool_free`).
    pub pool_free: i32,
    /// String slots allowed on top of a dumped format (`strings_free`).
    pub strings_free: i32,
    /// Terminal line cursor for diagnostics (`term_offset`).
    pub term_offset: i32,
    /// Log-file line cursor for diagnostics (`file_offset`).
    pub file_offset: i32,
    /// Maximum characters per diagnostic output line (`max_print_line`).
    pub max_print_line: i32,
    /// Show-context window width (`error_line`).
    pub error_line: i32,
    /// Show-context buffer (`trick_buf`), indexed by `tally % error_line`.
    pub trick_buf: Vec<i32>,
    /// Number of characters recorded toward the show-context window
    /// (`trick_count`).
    pub trick_count: i32,
    /// Running count of characters printed (`tally`).
    pub tally: i32,
    /// Decimal-digit scratch buffer used by the numeric print routines
    /// (`dig`).
    pub dig: [u8; 23],
    /// First string number allocated at runtime (`first_str`), used when
    /// dumping format files.
    pub first_str: i32,
    /// Terminal text emitted by the print machinery (`rust_stdout`
    /// replacement).
    pub term_out: Vec<u8>,
    /// Log-file text emitted by the print machinery (`log_file`
    /// replacement).
    pub log_out: Vec<u8>,
    /// Active diagnostic capture (`current_diagnostic` replacement).
    pub diag_capture: Option<Vec<u8>>,
    /// Error-context help text lines (`help_line`).
    pub help_line: Vec<String>,
    /// Number of active help lines (`help_ptr`).
    pub help_ptr: usize,
    /// Worst error level seen so far (`history`: `HISTORY_*`).
    pub history: u8,
    /// Whether file:line-style error prefixes are enabled
    /// (`file_line_error_style_p`).
    pub file_line_error_style_p: bool,
    /// Whether `\errhelp` text applies to the next error (`use_err_help`).
    pub use_err_help: bool,
    /// Abort on the first recoverable error (`halt_on_error_p`).
    pub halt_on_error_p: bool,
    /// Selector saved by `begin_diagnostic` (`old_setting`).
    pub old_setting: i32,
    /// Tally recorded at the current-context boundary (`first_count`).
    pub first_count: i32,
    /// Half the context window width (`half_error_line`).
    pub half_error_line: i32,
    /// Whether each TeX output stream is open (`write_open`).
    pub write_open: Vec<bool>,
    /// State-owned output streams indexed by TeX stream number.
    pub write_streams: Vec<Option<WriteStream>>,
    /// Whether the long default `\errmessage` help text has already been shown.
    pub long_help_seen: bool,
    /// Page-builder contents state (`EMPTY`, `INSERTS_ONLY`, or `BOX_THERE`).
    pub page_contents: u8,
    /// Whether the output routine is currently active.
    pub output_active: bool,
    /// Box produced by the latest packaging operation (`cur_box`).
    pub cur_box: i32,
    /// Box retained by `\lastbox`/`package` context (`just_box`).
    pub just_box: i32,
    /// Current box-shipping/package context source line (`pack_begin_line`).
    pub pack_begin_line: i32,
    /// Badness computed by the latest packaging pass (`last_badness`).
    pub last_badness: i32,
    /// Accumulated glue stretch by order for the active package (`total_stretch`).
    pub total_stretch: [i32; 4],
    /// Accumulated glue shrink by order for the active package (`total_shrink`).
    pub total_shrink: [i32; 4],
    /// Tail of the adjust-node holding list (`adjust_tail`).
    pub adjust_tail: i32,
    /// Tail of the pre-adjust holding list (`pre_adjust_tail`).
    pub pre_adjust_tail: i32,
    /// Tail of the contribution list (`page_tail`).
    pub page_tail: i32,
    /// Active vertical dimensions used by `vert_break` and the page builder.
    pub active_width: [i32; 7],
    /// Best legal breakpoint selected by the current vertical scan.
    pub best_place: i32,
    /// Cost associated with `best_place`.
    pub least_page_cost: i32,
    /// Height plus previous depth at `best_place`.
    pub best_height_plus_depth: i32,
    /// Most recent glue spec contributed to the current page (`last_glue`).
    pub last_glue: i32,
    /// Most recent penalty contributed to the current page (`last_penalty`).
    pub last_penalty: i32,
    /// Most recent kern width contributed to the current page (`last_kern`).
    pub last_kern: i32,
    /// Node-type selector for `\lastpenalty`, `\lastkern`, and `\lastskip` (`last_node_type`).
    pub last_node_type: i32,
    /// Best page breakpoint chosen by `build_page` (`best_page_break`).
    pub best_page_break: i32,
    /// Target height at `best_page_break` (`best_size`).
    pub best_size: i32,
    /// Maximum depth allowed on the current page (`page_max_depth`).
    pub page_max_depth: i32,
    /// Whether Tectonic semantic pagination bypasses ordinary page output.
    pub semantic_pagination_enabled: bool,
    /// Page boxes transferred to the output backend by `ship_out`.
    pub shipped_pages: Vec<i32>,
    /// Lists of discarded nodes retained by e-TeX `\savingvdiscards`.
    pub disc_ptr: [i32; 4],
    /// Temporary node pointer shared with C helper routines (`temp_ptr`).
    pub temp_ptr: i32,
    /// TeX--XeT left/right direction stack (`LR_ptr`).
    pub lr_ptr: i32,
    /// TeX--XeT direction-stack mismatch counter (`LR_problems`).
    pub lr_problems: i32,
    /// XDV output staging buffer (`dvi_buf`), always [`DVI_BUF_SIZE`] bytes.
    pub dvi_buf: Vec<u8>,
    /// End of the writable half of [`Self::dvi_buf`] (`dvi_limit`).
    pub dvi_limit: i32,
    /// Next free byte in [`Self::dvi_buf`] (`dvi_ptr`).
    pub dvi_ptr: i32,
    /// Absolute output-stream offset of `dvi_buf[0]` (`dvi_offset`).
    pub dvi_offset: i32,
    /// Output bytes already flushed to the file (`dvi_gone`).
    pub dvi_gone: i32,
    /// Accumulated XDV file contents; the per-engine replacement for C's
    /// `dvi_file` stream (`write_to_dvi` appends staged halves here).
    pub dvi_bytes: Vec<u8>,
    /// Set once `finalize_dvi_file` has written the postamble; guards
    /// against re-entry if dispatch resumes after `job_stopped`.
    pub dvi_finalized: bool,
    /// Whether the XDV output stream has been opened this job
    /// (`output_file_name != 0` in C).
    pub output_file_opened: bool,
    /// Resolved output file name once opened (`output_file_name`).
    pub output_file_name: String,
    /// Job name used to derive output file names (`job_name`; `None`
    /// mirrors C's `job_name == 0`).
    pub job_name: Option<String>,
    /// Vertical movement-record list head (`down_ptr`).
    pub down_ptr: i32,
    /// Horizontal movement-record list head (`right_ptr`).
    pub right_ptr: i32,
    /// Horizontal position in the XDV stream (`dvi_h`).
    pub dvi_h: i32,
    /// Vertical position in the XDV stream (`dvi_v`).
    pub dvi_v: i32,
    /// Currently selected output font (`dvi_f`).
    pub dvi_f: i32,
    /// Reference-point horizontal coordinate (`cur_h`).
    pub cur_h: i32,
    /// Reference-point vertical coordinate (`cur_v`).
    pub cur_v: i32,
    /// Nesting depth of `push`/`pop` in the output (`cur_s`).
    pub cur_s: i32,
    /// Absolute horizontal page origin offset (`cur_h_offset`).
    pub cur_h_offset: i32,
    /// Absolute vertical page origin offset (`cur_v_offset`).
    pub cur_v_offset: i32,
    /// Current page width (`cur_page_width`).
    pub cur_page_width: i32,
    /// Current page height (`cur_page_height`).
    pub cur_page_height: i32,
    /// Tallest page height+depth+offset seen (`max_v`).
    pub max_v: i32,
    /// Widest page width+offset seen (`max_h`).
    pub max_h: i32,
    /// Deepest `push` nesting emitted (`max_push`).
    pub max_push: i32,
    /// Completed page count for the postamble (`total_pages`).
    pub total_pages: i32,
    /// Byte offset of the most recent `bop` (`last_bop`).
    pub last_bop: i32,
    /// Locked magnification for `prepare_mag` (`mag_set`).
    pub mag_set: i32,
    /// Whether leader boxes are being emitted (`doing_leaders`).
    pub doing_leaders: bool,
    /// Whether a `\special` is being emitted (`doing_special`).
    pub doing_special: bool,
    /// Whether SyncTeX recording is enabled; all `synctex_*` recorders
    /// early-return when false, matching the C guards.
    pub synctex_enabled: bool,
    /// Per-glyph positioning data attached to native-word/glyph nodes:
    /// the memory-safe replacement for C's `native_glyph_info_ptr`
    /// malloc'd side buffer, keyed by node address.
    pub native_glyph_info: HashMap<i32, NativeGlyphInfo>,
    /// Whether a XeTeX ligature node was encountered while packaging.
    pub xtx_ligature_present: bool,
    /// Previous `\XeTeXcharclass` seen by `main_loop` (`prev_class`).
    pub prev_class: i32,
    /// `\XeTeXcharclass` of the most recently scanned character
    /// (`space_class`), consulted by the inter-char token hooks in
    /// `main_loop`/`append_normal_space`.
    pub space_class: i32,
    /// `\noboundary` pending flag (`cancel_boundary`).
    pub cancel_boundary: bool,
    /// Registered `\XeTeXlinebreaklocale` results: `(locale id, utf16
    /// text)` → break offsets (the `linebreak_start`/`linebreak_next`
    /// backend seam).
    pub locale_break_offsets: HashMap<(i32, Vec<u16>), Vec<i32>>,
    /// Current grouping level (`cur_level`).
    pub cur_level: u16,
    /// Current group code (`cur_group`).
    pub cur_group: u8,
    /// Save-stack entries below `save_ptr`.
    pub save_stack: Vec<SaveStackEntry>,
    /// Current save-stack pointer.
    pub save_ptr: usize,
    /// Deepest save-stack level reached so far.
    pub max_save_stack: usize,
    /// Maximum number of save-stack entries.
    pub save_size: usize,
    /// Roots of the eight sparse e-TeX arrays (`sa_root`).
    pub sa_root: [i32; SA_ROOT_COUNT],
    /// Current sparse-element pointer (`cur_ptr`).
    pub cur_ptr: i32,
    /// Null word used to initialize sparse index slots (`sa_null`).
    pub sa_null: MemoryWord,
    /// e-TeX sparse-array save chain (`sa_chain`).
    pub sa_chain: i32,
    /// e-TeX sparse-array save level (`sa_level`).
    pub sa_level: u16,

    /// Current hyphenation language (`cur_lang`).
    pub cur_lang: i32,
    /// Whether INITEX-only data commands such as `\patterns` are enabled.
    pub in_initex_mode: bool,
    /// Hyphenation exceptions keyed by lowercased word plus language, with
    /// discretionary positions recorded in reading order.
    pub hyph_exceptions: HashMap<Vec<i32>, Vec<i32>>,
    /// INITEX hyphenation patterns keyed by language and pattern text.
    pub hyph_patterns: HashMap<Vec<i32>, Vec<i32>>,

    // === INITEX / format-file state (`xetex-ini.c`) ===
    /// Unpacked hyphenation trie characters (`trie_c`), build-time only.
    pub fmt_trie_c: Vec<u16>,
    /// Unpacked trie op codes (`trie_o`).
    pub fmt_trie_o: Vec<i32>,
    /// Unpacked trie left (child) links (`trie_l`).
    pub fmt_trie_l: Vec<i32>,
    /// Unpacked trie right (sibling) links (`trie_r`).
    pub fmt_trie_r: Vec<i32>,
    /// Highest allocated unpacked trie node (`trie_ptr`).
    pub fmt_trie_ptr: i32,
    /// Trie packing hash table (`trie_hash`).
    pub fmt_trie_hash: Vec<i32>,
    /// Packed-position occupancy marks (`trie_taken`).
    pub fmt_trie_taken: Vec<bool>,
    /// Lowest free packed position per character (`trie_min`).
    pub fmt_trie_min: Vec<i32>,
    /// Highest occupied packed trie position (`trie_max`).
    pub fmt_trie_max: i32,
    /// Packed trie link table (`trie_trl` — next-position pointer).
    pub trie_trl: Vec<i32>,
    /// Packed trie op table (`trie_tro`).
    pub trie_tro: Vec<i32>,
    /// Packed trie character table (`trie_trc`).
    pub trie_trc: Vec<u16>,
    /// Hyphenation op distances (`hyf_distance`), 1-based.
    pub hyf_distance: Vec<i16>,
    /// Hyphenation op values (`hyf_num`), 1-based.
    pub hyf_num: Vec<i16>,
    /// Hyphenation op chains (`hyf_next`), 1-based.
    pub hyf_next: Vec<u16>,
    /// Language owning each hyphenation op (`trie_op_lang`), 1-based.
    pub trie_op_lang: Vec<u8>,
    /// Per-language serial assigned to each op (`trie_op_val`), 1-based.
    pub trie_op_val: Vec<u16>,
    /// Trie-op hash buckets (`trie_op_hash`), indexed
    /// `-TRIE_OP_SIZE..=TRIE_OP_SIZE` with offset `TRIE_OP_SIZE`.
    pub trie_op_hash: Vec<i32>,
    /// Number of allocated hyphenation ops (`trie_op_ptr`).
    pub trie_op_ptr: i32,
    /// Ops allocated per language (`trie_used`).
    pub trie_used: Vec<i32>,
    /// First op index of each language (`op_start`).
    pub op_start: Vec<i32>,
    /// Largest op serial issued (`max_op_used`).
    pub max_op_used: i32,
    /// Whether the packed hyphenation trie still needs building
    /// (`trie_not_ready`).
    pub trie_not_ready: bool,
    /// Packed position of the `\lccode` trie root (`hyph_start`).
    pub hyph_start: i32,
    /// Largest character seen in hyphenation data (`max_hyph_char`).
    pub fmt_max_hyph_char: i32,
    /// Exception-table word strings (`hyph_word`), `str_number`s.
    pub hyph_word: Vec<i32>,
    /// Exception-table discretionary lists (`hyph_list`).
    pub hyph_list: Vec<i32>,
    /// Exception-table collision links (`hyph_link`).
    pub hyph_link: Vec<u16>,
    /// Number of stored hyphenation exceptions (`hyph_count`).
    pub hyph_count: i32,
    /// Next free exception slot candidate (`hyph_next`).
    pub hyph_next: i32,
    /// Language node inside the packed `\lccode` trie used while scanning
    /// `\hyphenation` (`hyph_index`).
    pub hyph_index: i32,
    /// Lower boundary of the dense hash region (`hash_used`).
    pub hash_used: i32,
    /// High-water mark of the extra hash region (`hash_high`).
    pub hash_high: i32,
    /// Extra hash slots beyond `EQTB_SIZE` (`hash_extra`).
    pub hash_extra: i32,
    /// Last valid control-sequence hash index (`hash_top`).
    pub hash_top: i32,
    /// Last valid eqtb index (`eqtb_top`).
    pub eqtb_top: i32,
    /// C-layout control-sequence hash image (`hash`/`yhash`), sparse: maps
    /// eqtb-region index → `TwoHalves` (`s0` link, `s1` name).
    pub fmt_hash: HashMap<i32, TwoHalves>,
    /// C-layout eqtb image for control-sequence and primitive entries
    /// (`eqtb[cur_val]`, `eqtb[PRIM_EQTB_BASE + p]`, frozen entries).
    pub fmt_eqtb: HashMap<i32, MemoryWord>,
    /// `\par` position in the C-layout image (`par_loc` counterpart).
    pub fmt_par_loc: i32,
    /// `\write` position in the C-layout image (`write_loc` counterpart).
    pub fmt_write_loc: i32,
    /// Primitive hash table (`prim[0..=PRIM_SIZE]`).
    pub prim: Vec<TwoHalves>,
    /// Lowest free primitive slot (`prim_used`).
    pub prim_used: i32,
    /// Number of multiletter control sequences (`cs_count`).
    pub cs_count: i32,
    /// `str_number` of the format identification line (`format_ident`).
    pub format_ident: i32,
    /// Highest allocated `mem` word (`mem_end`).
    pub mem_end: i32,
    /// One-word memory usage reported by `\dump` (`var_used`).
    pub var_used: i32,
    /// Multi-word memory usage reported by `\dump` (`dyn_used`).
    pub dyn_used: i32,
    /// `font_info` words in use (`fmem_ptr`).
    pub fmem_ptr: i32,
    /// Serialized format-file image produced by `store_fmt_file`; the
    /// driver flushes it to `<jobname>.fmt`.
    pub fmt_bytes: Vec<u8>,
    /// Default format-file name stem (`TEX_format_default`).
    pub tex_format_default: String,
    /// Internal-consistency failure code from `tt_run_engine` (`bad`).
    pub bad: i32,
    /// Largest legal register number (`max_reg_num`).
    pub max_reg_num: i32,
    /// Help text for register-number range errors (`max_reg_help_line`).
    pub max_reg_help_line: String,
    /// Pseudo-input file list for the `\input` recorder (`pseudo_files`).
    pub pseudo_files: i32,
    /// Output filename extension (`output_file_extension`).
    pub output_file_extension: String,
    /// Whether the generic char-warning help was already printed
    /// (`gave_char_warning_help`).
    pub gave_char_warning_help: bool,

    /// Head of the passive breakpoint chain built by `try_break`
    /// (`passive`).
    pub lb_passive: i32,
    /// Running totals while scanning the active list (`cur_active_width`).
    pub lb_cur_active_width: [i32; 7],
    /// Paragraph background width/stretch/shrink (`background`).
    pub lb_background: [i32; 7],
    /// Width contributions up to the current breakpoint (`break_width`).
    pub lb_break_width: [i32; 7],
    /// Best feasible active node per fitness class (`best_place`).
    pub lb_best_place: [i32; 4],
    /// Line number recorded for each `lb_best_place` entry
    /// (`best_pl_line`).
    pub lb_best_pl_line: [i32; 4],
    /// Width of a discretionary's pre-break list (`disc_width`).
    pub lb_disc_width: i32,
    /// Whether the "infinite shrinkage" diagnostic is still pending
    /// (`no_shrink_error_yet`).
    pub lb_no_shrink_error_yet: bool,
    /// Node currently under consideration as a breakpoint (`cur_p`).
    pub lb_cur_p: i32,
    /// Whether the tolerance pass is running (`second_pass`).
    pub lb_second_pass: bool,
    /// Whether the emergency pass is running (`final_pass`).
    pub lb_final_pass: bool,
    /// Badness cutoff for candidate breaks (`threshold`).
    pub lb_threshold: i32,
    /// Minimum demerits recorded per fitness class (`minimal_demerits`).
    pub lb_minimal_demerits: [i32; 4],
    /// Minimum demerits seen in the current class (`minimum_demerits`).
    pub lb_minimum_demerits: i32,
    /// Line number beyond which all lines share a width (`easy_line`).
    pub lb_easy_line: i32,
    /// Lines with individually shaped widths (`last_special_line`).
    pub lb_last_special_line: i32,
    /// Width of specially shaped lines (`first_width`).
    pub lb_first_width: i32,
    /// Width of regularly shaped lines (`second_width`).
    pub lb_second_width: i32,
    /// Indentation of specially shaped lines (`first_indent`).
    pub lb_first_indent: i32,
    /// Indentation of regularly shaped lines (`second_indent`).
    pub lb_second_indent: i32,
    /// Active node of the best chosen breakpoint (`best_bet`).
    pub lb_best_bet: i32,
    /// Demerits of `lb_best_bet` (`fewest_demerits`).
    pub lb_fewest_demerits: i32,
    /// Number of lines in the broken paragraph (`best_line`).
    pub lb_best_line: i32,
    /// Line-count difference achieved for `\looseness`
    /// (`actual_looseness`).
    pub lb_actual_looseness: i32,
    /// Per-candidate line-count difference (`line_diff`).
    pub lb_line_diff: i32,
    /// Letters collected for the current word (`hn`).
    pub lb_hn: i32,
    /// First node of the word being hyphenated (`ha`).
    pub lb_ha: i32,
    /// Node after the last letter collected (`hb`).
    pub lb_hb: i32,
    /// Hyphen character of the word's font (`hyf_char`).
    pub lb_hyf_char: i32,
    /// `\language` captured at paragraph start (`init_cur_lang`).
    pub lb_init_cur_lang: i32,
    /// Working left hyphenation minimum (`l_hyf`).
    pub lb_l_hyf: i32,
    /// Working right hyphenation minimum (`r_hyf`).
    pub lb_r_hyf: i32,
    /// `\lefthyphenmin` captured at paragraph start (`init_l_hyf`).
    pub lb_init_l_hyf: i32,
    /// `\righthyphenmin` captured at paragraph start (`init_r_hyf`).
    pub lb_init_r_hyf: i32,
    /// Character preceding the word in ligature reconstruction
    /// (`hyf_bchar`).
    pub lb_hyf_bchar: i32,
    /// `\parfillskip` node appended to the paragraph (`last_line_fill`).
    pub lb_last_line_fill: i32,
    /// Whether `\lastlinefit` adjustment applies (`do_last_line_fit`).
    pub lb_do_last_line_fit: bool,
    /// Active node allocation size for this pass (`active_node_size`).
    pub lb_active_node_size: i32,
    /// Infinite stretch totals per order for `\lastlinefit`
    /// (`fill_width`).
    pub lb_fill_width: [i32; 3],
    /// Shortfall recorded per fitness class (`best_pl_short`).
    pub lb_best_pl_short: [i32; 4],
    /// Glue recorded per fitness class (`best_pl_glue`).
    pub lb_best_pl_glue: [i32; 4],
    /// Font of the word being hyphenated (`hf`).
    pub lb_hf: i32,
    /// Original characters of the word (`hu`).
    pub lb_hu: Vec<i32>,
    /// Lowercased hyphenation codes of the word (`hc`).
    pub lb_hc: Vec<i32>,
    /// Hyphenation weights at each letter gap (`hyf`).
    pub lb_hyf: Vec<i32>,
    /// Last `hyf` position already emitted (`hyphen_passed`).
    pub lb_hyphen_passed: i32,
    /// Leading material replayed by `reconstitute` (`init_list`).
    pub lb_init_list: i32,
    /// Whether `lb_init_list` is a ligature body (`init_lig`).
    pub lb_init_lig: bool,
    /// Whether `lb_init_list` opened with a left boundary (`init_lft`).
    pub lb_init_lft: bool,
    /// Ligature assembly flag inside `reconstitute` (`ligature_present`).
    pub lb_ligature_present: bool,
    /// Left-boundary hit flag inside `reconstitute` (`lft_hit`).
    pub lb_lft_hit: bool,
    /// Right-boundary hit flag inside `reconstitute` (`rt_hit`).
    pub lb_rt_hit: bool,
    /// Current left character inside `reconstitute` (`cur_l`).
    pub lb_cur_l: i32,
    /// Current right character inside `reconstitute` (`cur_r`).
    pub lb_cur_r: i32,
    /// Tail of the `HOLD_HEAD` list inside `reconstitute` (`cur_q`).
    pub lb_cur_q: i32,
    /// Pending ligature items inside `reconstitute` (`lig_stack`).
    pub lb_lig_stack: i32,
    /// First node of the paragraph for protrusion scans (`first_p`).
    pub lb_first_p: i32,
    /// Node preceding `lb_cur_p` for protrusion scans (`global_prev_p`).
    pub lb_global_prev_p: i32,
    /// Font of the last node printed by `short_display`
    /// (`font_in_short_display`).
    pub lb_font_in_short_display: i32,
    /// Leftmost protrudable character found (`last_leftmost_char`).
    pub lb_last_leftmost_char: i32,
    /// Rightmost protrudable character found (`last_rightmost_char`).
    pub lb_last_rightmost_char: i32,
    /// Explicit hlist nesting stack for protrusion walks (`hlist_stack`).
    pub lb_hlist_stack: Vec<i32>,
    /// Number of recoverable errors accumulated by the engine (`error_count`).
    pub error_count: i32,
    /// Arithmetic error indicator flag (`arith_error`).
    pub arith_error: bool,
    /// Remainder from division routines (`tex_remainder`).
    pub tex_remainder: i32,

    /// Pseudo-random number generator state.
    pub rand: TeXRandom,
    /// Seed last assigned by `\setrandomseed`.
    pub random_seed: i32,
    /// Seconds component captured by `\resettimer`.
    pub epochseconds: i64,
    /// Microseconds component captured by `\resettimer`.
    pub microseconds: i32,
    /// Whether unrestricted shell escape is enabled (`shell_escape_enabled`).
    pub shell_escape_enabled: bool,
    /// `\write18` command strings collected for the driver to execute
    /// (replaces C's direct `ttstub_shell_escape` call; the pure engine
    /// never spawns processes).
    pub shell_escape_requests: Vec<String>,
    /// Horizontal position recorded by the most recent `\pdfsavepos`
    /// (`pdf_last_x_pos`).
    pub pdf_last_x_pos: i32,
    /// Vertical position recorded by the most recent `\pdfsavepos`
    /// (`pdf_last_y_pos`).
    pub pdf_last_y_pos: i32,
    /// Registered PDF page counts keyed by file name, backing
    /// `\XeTeXpdfpagecount` without host filesystem access
    /// (`count_pdf_file_pages` returns 0 when the name is absent, matching
    /// C's failed-open path).
    pub pdf_page_counts: HashMap<String, i32>,
    /// Registered layout-engine query results for native fonts, keyed by
    /// `(font, m - XETEX_INT code offset, scanned arguments)`. Backs the C
    /// `aat_font_get*`/`ot_font_get*` calls without a platform engine.
    pub font_engine_values: HashMap<(i32, i32, Vec<i32>), i32>,
    /// Named layout-engine query results keyed by `(font, code offset,
    /// extra selector argument or -1, packed name)`, backing
    /// `aat_font_get_named*`/`gr_font_get_named*`.
    pub font_engine_named_values: HashMap<(i32, i32, i32, String), i32>,
    /// Layout-engine name strings keyed by `(font, convert code, arg1,
    /// arg2)`, backing `aat_print_font_name`/`gr_print_font_name` for
    /// `\XeTeXvariationname`, `\XeTeXfeaturename`, and
    /// `\XeTeXselectorname`.
    pub font_engine_named_strings: HashMap<(i32, i32, i32, i32), String>,

    /// Bound of low memory currently allocated.
    pub lo_mem_max: i32,
    /// Bound of high memory currently allocated.
    pub hi_mem_min: i32,
    /// Pointer to rover node in dynamic memory.
    pub rover: i32,
    /// Head of available one-word token memory list (`avail`).
    pub avail: i32,

    /// TeX input buffer holding UTF-16 units or Unicode scalar values.
    pub buffer: Vec<i32>,
    /// Remaining source lines queued by `set_source_text`.
    pub source_lines: VecDeque<Vec<i32>>,
    /// First untouched buffer position on the current line.
    pub first: i32,
    /// Last occupied buffer position on the current line.
    pub last: i32,
    /// Maximum buffer position reached so far.
    pub max_buf_stack: i32,
    /// Capacity of the TeX input buffer.
    pub buf_size: usize,
    /// The current input frame.
    pub cur_input: InputState,
    /// Saved input frames below `input_ptr`.
    pub input_stack: Vec<InputState>,
    /// Number of active input frames pushed on `input_stack`.
    pub input_ptr: usize,
    /// Deepest input-stack level reached so far.
    pub max_in_stack: usize,
    /// Maximum number of pushed input frames permitted.
    pub input_stack_capacity: usize,
    /// Number of open file-like input levels.
    pub in_open: usize,
    /// Maximum number of open file-like input levels (`max_in_open`).
    pub max_in_open: usize,
    /// Registered in-memory sources available to `\input`.
    pub input_sources: HashMap<String, VecDeque<Vec<i32>>>,
    /// Open file-source contexts indexed by `in_open`.
    pub file_sources: Vec<FileInputSource>,
    /// Whether `\everyeof` has already been inserted for each input level.
    pub eof_seen: Vec<bool>,
    /// Parent line number saved for each open input level.
    pub line_stack: Vec<i32>,
    /// Save-boundary pointer captured when each input level opened.
    pub grp_stack: Vec<i32>,
    /// Conditional pointer captured when each input level opened.
    pub if_stack: Vec<i32>,
    /// TeX `\read` stream open state used by `\ifeof`.
    pub read_open: Vec<ReadOpen>,
    /// Sources attached to TeX `\read` streams.
    pub read_sources: Vec<Option<FileInputSource>>,
    /// Registered raw file bytes available to `\input`/`\openin`
    /// (`ttstub_input_open`'s virtualized backing store).
    pub input_byte_sources: HashMap<String, Vec<u8>>,
    /// Registered modification times (seconds since the Unix epoch) backing
    /// `ttstub_input_get_mtime` for `\pdffilemoddate`; absent entries read
    /// as the epoch.
    pub input_file_mtimes: HashMap<String, i64>,
    /// PDF-style creation-date string captured at job start
    /// (`init_start_time`'s `start_time_str`, UTC `makepdftime`).
    pub start_time_str: String,
    /// C `input_file[in_open]` byte streams for real input files.
    pub input_file: Vec<Option<crate::input::UFile>>,
    /// C `read_file[0..=16]` byte streams for `\read` streams.
    pub read_file: Vec<Option<crate::input::UFile>>,
    /// Packed file name bytes produced by `pack_file_name` (`name_of_file`).
    pub name_of_file: Vec<u8>,
    /// Length of `name_of_file` (`name_length`).
    pub name_length: i32,
    /// `name_of_file` re-encoded as UTF-16 code units (`name_of_file16`).
    pub name_of_file16: Vec<u16>,
    /// Length of `name_of_file16` (`name_length16`).
    pub name_length16: i32,
    /// Most recently opened input file name (`name_of_input_file`).
    pub name_of_input_file: Option<String>,
    /// Absolute path of the last opened input for SyncTeX
    /// (`abspath_of_input_file`); empty under virtualized I/O.
    pub abspath_of_input_file: String,
    /// Byte scratch used by the `ICUMAPPING` `input_line` path.
    pub io_byte_buf: Vec<u8>,
    /// Scalar scratch used by the normalization `input_line` paths.
    pub io_utf32_buf: Vec<u32>,
    /// Current TeX line number.
    pub line: i32,
    /// Whether file-name scanning stops at an unquoted space.
    pub stop_at_space: bool,
    /// Active quote character while scanning a file name.
    pub file_name_quote_char: i32,
    /// Whether the current file name was quoted.
    pub quoted_filename: bool,
    /// Byte/code-unit offset of the last directory delimiter in a scanned name.
    pub area_delimiter: usize,
    /// Byte/code-unit offset of the last extension delimiter in a scanned name.
    pub ext_delimiter: usize,
    /// File-name stem produced by `scan_file_name`/`end_name`.
    pub cur_name: Vec<i32>,
    /// Directory component produced by `scan_file_name`/`end_name`.
    pub cur_area: Vec<i32>,
    /// Extension component produced by `scan_file_name`/`end_name`.
    pub cur_ext: Vec<i32>,
    /// Monotonic TeX name id assigned to opened file sources.
    pub next_source_name: i32,
    /// Current token corresponding to `cur_cmd`/`cur_chr`/`cur_cs`.
    pub cur_tok: i32,
    /// Token saved by `\afterassignment` for reinsertion after the next assignment.
    pub after_token: i32,
    /// Brace-alignment counter used by the scanner and macro matcher.
    pub align_state: i32,
    /// Scanner status (`NORMAL`, `SKIPPING`, `DEFINING`, `MATCHING`, ...).
    pub scanner_status: u8,
    /// Control sequence used in restricted-scan diagnostics.
    pub warning_index: i32,
    /// Token-list head for a definition currently being assembled.
    pub def_ref: i32,
    /// Macro argument token lists referenced by `OUT_PARAM` tokens.
    pub param_stack: Vec<i32>,
    /// Number of live entries in `param_stack`.
    pub param_ptr: usize,
    /// Deepest parameter-stack level reached so far.
    pub max_param_stack: usize,
    /// Maximum number of macro parameters that may be live at once.
    pub param_stack_capacity: usize,
    /// Control-sequence id for `\par`.
    pub par_loc: i32,
    /// Token encoding for `\par`.
    pub par_token: i32,
    /// Control-sequence id used by `\write` (`write_loc`).
    pub write_loc: i32,
    /// Current alignment list head, or `TEX_NULL` outside alignments.
    pub cur_align: i32,
    /// Alignment save-stack pointer (`align_ptr`).
    pub align_ptr: i32,
    /// First preamble entry of the current span (`cur_span`).
    pub cur_span: i32,
    /// Repeat-column entry for preamble `\span` loops (`cur_loop`).
    pub cur_loop: i32,
    /// Holding-list head for `\vadjust` material inside a column (`cur_head`).
    pub cur_head: i32,
    /// Holding-list tail for `\vadjust` material inside a column (`cur_tail`).
    pub cur_tail: i32,
    /// Holding-list head for `\vadjust pre` material (`cur_pre_head`).
    pub cur_pre_head: i32,
    /// Holding-list tail for `\vadjust pre` material (`cur_pre_tail`).
    pub cur_pre_tail: i32,
    /// Current conditional stack pointer, or `TEX_NULL`.
    pub cond_ptr: i32,
    /// Current alignment boundary pointer, or `TEX_NULL`.
    pub cur_boundary: i32,
    /// Second scanned value while packing a math code (`cur_val1`).
    pub cur_val1: i32,
    /// Current math-list head during `mlist_to_hlist` (`cur_mlist`).
    pub cur_mlist: i32,
    /// Current math style during `mlist_to_hlist` (`cur_style`).
    pub cur_style: i32,
    /// Current math size code during `mlist_to_hlist` (`cur_size`).
    pub cur_size: i32,
    /// Current mu-unit scale during `mlist_to_hlist` (`cur_mu`).
    pub cur_mu: i32,
    /// Whether penalties are recorded inside formulas (`mlist_penalties`).
    pub mlist_penalties: bool,
    /// Shared `fil_glue` specification node (`rebox` and friends);
    /// allocated lazily and pinned so `delete_glue_ref` never frees it.
    pub fil_glue_spec: i32,
    /// Current math character during conversion (`cur_c`).
    pub math_cur_c: i32,
    /// Current math font during conversion (`cur_f`).
    pub math_cur_f: i32,
    /// Current `char_info`/lig-kern word during conversion (`cur_i`):
    /// for `char_info`, `skip` doubles as the nonzero-width flag, `op` is
    /// the tag nibble, and `remainder` the remainder byte; for lig/kern
    /// instructions the fields map directly (`s3`/`s2`/`s1`/`s0`).
    pub math_cur_i: LigKernInsn,
    /// TeX--XeT current typesetting direction (`cur_dir`).
    pub cur_dir: i32,
    /// SyncTeX `insert_src_special_every_math` flag (source-info insertion
    /// itself is not yet ported; the flags preserve the C guard shape).
    pub insert_src_special_every_math: bool,
    /// SyncTeX `insert_src_special_every_display` flag.
    pub insert_src_special_every_display: bool,
    /// SyncTeX `insert_src_special_every_vbox` flag.
    pub insert_src_special_every_vbox: bool,
    /// Whether unknown control sequences should resolve to `undefined_cs`.
    pub no_new_control_sequence: bool,
    /// Whether the next end-of-file has been forced by `\endinput`.
    pub force_eof: bool,
    /// Whether Tectonic coda tokens have already been inserted.
    pub used_tectonic_coda_tokens: bool,
    /// Number of nested expansion steps currently active.
    pub expand_depth_count: usize,
    /// Maximum supported nested expansion depth.
    pub expand_depth: usize,
    /// Whether a `\csname...\endcsname` name is being scanned.
    pub is_in_csname: bool,
    /// Current mark-class token lists.
    pub cur_mark: [i32; 5],
    /// Whether the macro currently being expanded accepts `\par` tokens.
    pub long_state: u8,
    /// Argument token-list heads for the macro call being matched.
    pub pstack: [i32; 9],
    /// Level of `cur_val` (integer, dimension, glue, ...).
    pub cur_val_level: u8,
    /// Current integer-scan radix.
    pub radix: i16,
    /// Current order used by scanning routines.
    pub cur_order: i16,
    /// Conditional-limit code for `\else`/`\or` handling.
    pub if_limit: u8,
    /// Current conditional command subtype.
    pub cur_if: i16,
    /// Source line associated with the innermost conditional.
    pub if_line: i32,
    /// Line where conditional skipping began.
    pub skip_line: i32,
    /// Whether a file name is currently being scanned.
    pub name_in_progress: bool,
    /// Compact pure-Rust control-sequence definitions table.
    pub cs_table: ControlSequenceTable,
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
        let cat_code_levels = vec![LEVEL_ONE; 256];
        let cs_table = ControlSequenceTable::new();
        let par_token = CS_TOKEN_FLAG + PAR_CS_ID;
        let cur_input = InputState {
            state: MID_LINE,
            index: 0,
            start: 0,
            loc: 0,
            limit: -1,
            name: 1,
            synctex_tag: 0,
        };

        let mut state = Self {
            mem,
            eqtb,
            hash,
            cat_codes,
            cat_code_levels,
            code_tables: Default::default(),
            font_char_metrics: HashMap::new(),
            font_area: HashMap::new(),
            font_table: vec![FontRecord::default()],
            font_sources: HashMap::new(),
            font_id_cs: vec![-1],
            picture_bounds: HashMap::new(),
            font_specs: HashMap::new(),
            tec_sources: HashMap::new(),
            font_mappings: HashMap::new(),
            native_font_type_flag: 0,
            loaded_font_mapping: None,
            loaded_font_flags: 0,
            loaded_font_letter_space: 0,
            loaded_font_design_size: 0,
            req_engine: 0,
            saved_mapping_name: None,
            find_native_font_scaled: 0,
            pending_font_spec: None,
            cur_val: 0,
            cur_cmd: 0,
            cur_chr: 0,
            cur_cs: 0,
            mode: TeXMode::Vmode as i16,
            head: mem_size.saturating_sub(2) as i32,
            tail: mem_size.saturating_sub(2) as i32,
            prev_graf: 0,
            aux: MemoryWord::default(),
            etex_aux: TEX_NULL,
            mode_line: 0,
            nest: Vec::with_capacity(500),
            nest_ptr: 0,
            max_nest_stack: 0,
            nest_size: 500,
            page_so_far: [0; PAGE_SO_FAR_COUNT],
            dead_cycles: 0,
            job_stopped: false,
            set_box_allowed: true,
            deletions_allowed: true,
            ins_disc: false,
            open_parens: 0,
            shown_mode: 0,
            edit_name_start: 0,
            cur_f: 0,
            ligature_present: false,
            lft_hit: false,
            rt_hit: false,
            insert_penalties: 0,
            interaction: ERROR_STOP_MODE,
            selector: SELECTOR_TERM_ONLY,
            log_opened: false,
            transcript: Vec::new(),
            str_pool: Vec::new(),
            str_start: vec![0],
            pool_ptr: 0,
            str_ptr: TOO_BIG_CHAR,
            pool_size: 6_250_000,
            max_strings: 565_536,
            init_pool_ptr: 0,
            init_str_ptr: TOO_BIG_CHAR,
            string_vacancies: 90_000,
            pool_free: 47_500,
            strings_free: 100,
            term_offset: 0,
            file_offset: 0,
            max_print_line: 79,
            error_line: 79,
            trick_buf: vec![0; 79],
            trick_count: 0,
            tally: 0,
            dig: [0; 23],
            first_str: 0,
            term_out: Vec::new(),
            log_out: Vec::new(),
            diag_capture: None,
            help_line: Vec::new(),
            help_ptr: 0,
            history: HISTORY_SPOTLESS,
            file_line_error_style_p: false,
            use_err_help: false,
            halt_on_error_p: false,
            old_setting: 0,
            first_count: 0,
            half_error_line: 50,
            write_open: vec![false; 16],
            write_streams: vec![None; 16],
            long_help_seen: false,
            page_contents: PAGE_EMPTY,
            output_active: false,
            cur_box: TEX_NULL,
            just_box: TEX_NULL,
            pack_begin_line: 0,
            last_badness: 0,
            total_stretch: [0; 4],
            total_shrink: [0; 4],
            adjust_tail: TEX_NULL,
            pre_adjust_tail: TEX_NULL,
            page_tail: TEX_NULL,
            active_width: [0; 7],
            best_place: TEX_NULL,
            least_page_cost: MAX_HALFWORD,
            best_height_plus_depth: 0,
            last_glue: MAX_HALFWORD,
            last_penalty: 0,
            last_kern: 0,
            last_node_type: -1,
            best_page_break: TEX_NULL,
            best_size: 0,
            page_max_depth: 0,
            semantic_pagination_enabled: false,
            shipped_pages: Vec::new(),
            dvi_finalized: false,
            disc_ptr: [TEX_NULL; 4],
            temp_ptr: TEX_NULL,
            lr_ptr: TEX_NULL,
            lr_problems: 0,
            dvi_buf: vec![0; DVI_BUF_SIZE],
            dvi_limit: DVI_BUF_SIZE as i32,
            dvi_ptr: 0,
            dvi_offset: 0,
            dvi_gone: 0,
            dvi_bytes: Vec::new(),
            output_file_opened: false,
            output_file_name: String::new(),
            job_name: None,
            down_ptr: TEX_NULL,
            right_ptr: TEX_NULL,
            dvi_h: 0,
            dvi_v: 0,
            dvi_f: 0,
            cur_h: 0,
            cur_v: 0,
            cur_s: -1,
            cur_h_offset: 0,
            cur_v_offset: 0,
            cur_page_width: 0,
            cur_page_height: 0,
            max_v: 0,
            max_h: 0,
            max_push: 0,
            total_pages: 0,
            last_bop: -1,
            mag_set: 0,
            doing_leaders: false,
            doing_special: false,
            synctex_enabled: false,
            native_glyph_info: HashMap::new(),
            xtx_ligature_present: false,
            prev_class: 0,
            space_class: 0,
            cancel_boundary: false,
            locale_break_offsets: HashMap::new(),
            cur_level: LEVEL_ONE,
            cur_group: BOTTOM_LEVEL,
            save_stack: Vec::with_capacity(64),
            save_ptr: 0,
            max_save_stack: 0,
            save_size: 80_000,
            sa_root: [TEX_NULL; SA_ROOT_COUNT],
            cur_ptr: TEX_NULL,
            sa_null: MemoryWord::default(),
            sa_chain: TEX_NULL,
            sa_level: LEVEL_ZERO,
            cur_lang: 0,
            in_initex_mode: false,
            hyph_exceptions: HashMap::new(),
            hyph_patterns: HashMap::new(),
            fmt_trie_c: Vec::new(),
            fmt_trie_o: Vec::new(),
            fmt_trie_l: Vec::new(),
            fmt_trie_r: Vec::new(),
            fmt_trie_ptr: 0,
            fmt_trie_hash: Vec::new(),
            fmt_trie_taken: Vec::new(),
            fmt_trie_min: Vec::new(),
            fmt_trie_max: 0,
            trie_trl: Vec::new(),
            trie_tro: Vec::new(),
            trie_trc: Vec::new(),
            hyf_distance: vec![0; (TRIE_OP_SIZE + 1) as usize],
            hyf_num: vec![0; (TRIE_OP_SIZE + 1) as usize],
            hyf_next: vec![0; (TRIE_OP_SIZE + 1) as usize],
            trie_op_lang: vec![0; (TRIE_OP_SIZE + 1) as usize],
            trie_op_val: vec![0; (TRIE_OP_SIZE + 1) as usize],
            trie_op_hash: vec![0; (2 * TRIE_OP_SIZE + 1) as usize],
            trie_op_ptr: 0,
            trie_used: vec![MIN_TRIE_OP; (BIGGEST_LANG + 1) as usize],
            op_start: vec![0; (BIGGEST_LANG + 1) as usize],
            max_op_used: MIN_TRIE_OP,
            trie_not_ready: true,
            hyph_start: 0,
            fmt_max_hyph_char: TOO_BIG_LANG,
            hyph_word: vec![0; HYPH_SIZE + 1],
            hyph_list: vec![TEX_NULL; HYPH_SIZE + 1],
            hyph_link: vec![0; HYPH_SIZE + 1],
            hyph_count: 0,
            hyph_next: HYPH_PRIME + 1,
            hyph_index: 0,
            hash_used: C_FROZEN_CONTROL_SEQUENCE,
            hash_high: 0,
            hash_extra: 0,
            hash_top: C_UNDEFINED_CONTROL_SEQUENCE,
            eqtb_top: C_EQTB_SIZE,
            fmt_bytes: Vec::new(),
            fmt_hash: HashMap::new(),
            fmt_eqtb: HashMap::new(),
            fmt_par_loc: 0,
            fmt_write_loc: 0,
            prim: vec![TwoHalves::default(); (C_PRIM_SIZE + 1) as usize],
            prim_used: C_PRIM_SIZE,
            cs_count: 0,
            format_ident: 0,
            mem_end: 0,
            var_used: 0,
            dyn_used: 0,
            fmem_ptr: 7,
            tex_format_default: String::new(),
            bad: 0,
            max_reg_num: 32767,
            max_reg_help_line: "A register number must be between 0 and 32767.".to_string(),
            pseudo_files: TEX_NULL,
            output_file_extension: ".xdv".to_string(),
            gave_char_warning_help: false,
            lb_passive: TEX_NULL,
            lb_cur_active_width: [0; 7],
            lb_background: [0; 7],
            lb_break_width: [0; 7],
            lb_best_place: [TEX_NULL; 4],
            lb_best_pl_line: [0; 4],
            lb_disc_width: 0,
            lb_no_shrink_error_yet: true,
            lb_cur_p: TEX_NULL,
            lb_second_pass: false,
            lb_final_pass: false,
            lb_threshold: 0,
            lb_minimal_demerits: [MAX_HALFWORD; 4],
            lb_minimum_demerits: MAX_HALFWORD,
            lb_easy_line: 0,
            lb_last_special_line: 0,
            lb_first_width: 0,
            lb_second_width: 0,
            lb_first_indent: 0,
            lb_second_indent: 0,
            lb_best_bet: TEX_NULL,
            lb_fewest_demerits: 0,
            lb_best_line: 0,
            lb_actual_looseness: 0,
            lb_line_diff: 0,
            lb_hn: 0,
            lb_ha: TEX_NULL,
            lb_hb: TEX_NULL,
            lb_hyf_char: 0,
            lb_init_cur_lang: 0,
            lb_l_hyf: 0,
            lb_r_hyf: 0,
            lb_init_l_hyf: 0,
            lb_init_r_hyf: 0,
            lb_hyf_bchar: 0,
            lb_last_line_fill: TEX_NULL,
            lb_do_last_line_fit: false,
            lb_active_node_size: ACTIVE_NODE_SIZE_NORMAL,
            lb_fill_width: [0; 3],
            lb_best_pl_short: [0; 4],
            lb_best_pl_glue: [0; 4],
            lb_hf: 0,
            lb_hu: vec![0; (HYPHENATABLE_LENGTH_LIMIT + 4) as usize],
            lb_hc: vec![0; (HYPHENATABLE_LENGTH_LIMIT + 4) as usize],
            lb_hyf: vec![0; (HYPHENATABLE_LENGTH_LIMIT + 4) as usize],
            lb_hyphen_passed: 0,
            lb_init_list: TEX_NULL,
            lb_init_lig: false,
            lb_init_lft: false,
            lb_ligature_present: false,
            lb_lft_hit: false,
            lb_rt_hit: false,
            lb_cur_l: 0,
            lb_cur_r: 0,
            lb_cur_q: TEX_NULL,
            lb_lig_stack: TEX_NULL,
            lb_first_p: TEX_NULL,
            lb_global_prev_p: TEX_NULL,
            lb_font_in_short_display: 0,
            lb_last_leftmost_char: TEX_NULL,
            lb_last_rightmost_char: TEX_NULL,
            lb_hlist_stack: Vec::new(),
            error_count: 0,
            arith_error: false,
            tex_remainder: 0,
            rand: TeXRandom::default(),
            random_seed: 0,
            epochseconds: 0,
            microseconds: 0,
            shell_escape_enabled: false,
            shell_escape_requests: Vec::new(),
            pdf_last_x_pos: 0,
            pdf_last_y_pos: 0,
            pdf_page_counts: HashMap::new(),
            font_engine_values: HashMap::new(),
            font_engine_named_values: HashMap::new(),
            font_engine_named_strings: HashMap::new(),
            lo_mem_max: 0,
            hi_mem_min: (mem_size - 1) as i32,
            rover: 0,
            avail: 0,
            buffer: Vec::with_capacity(1_024),
            source_lines: VecDeque::new(),
            first: 0,
            last: -1,
            max_buf_stack: 0,
            buf_size: 200_000,
            cur_input,
            input_stack: Vec::with_capacity(64),
            input_ptr: 0,
            max_in_stack: 0,
            input_stack_capacity: 5_000,
            in_open: 0,
            max_in_open: 127,
            input_sources: HashMap::new(),
            file_sources: vec![FileInputSource::default()],
            eof_seen: vec![false],
            line_stack: vec![0],
            grp_stack: vec![TEX_NULL],
            if_stack: vec![TEX_NULL],
            read_open: vec![ReadOpen::Closed; 17],
            read_sources: vec![None; 17],
            input_byte_sources: HashMap::new(),
            input_file_mtimes: HashMap::new(),
            start_time_str: String::new(),
            input_file: Vec::new(),
            read_file: vec![None; 17],
            name_of_file: Vec::new(),
            name_length: 0,
            name_of_file16: Vec::new(),
            name_length16: 0,
            name_of_input_file: None,
            abspath_of_input_file: String::new(),
            io_byte_buf: Vec::new(),
            io_utf32_buf: Vec::new(),
            line: 0,
            stop_at_space: true,
            file_name_quote_char: 0,
            quoted_filename: false,
            area_delimiter: 0,
            ext_delimiter: 0,
            cur_name: Vec::new(),
            cur_area: Vec::new(),
            cur_ext: Vec::new(),
            next_source_name: 20,
            cur_tok: 0,
            after_token: TEX_NULL,
            align_state: 1_000_000,
            scanner_status: crate::scanner::NORMAL,
            warning_index: 0,
            def_ref: TEX_NULL,
            param_stack: vec![TEX_NULL; 256],
            param_ptr: 0,
            max_param_stack: 0,
            param_stack_capacity: 10_000,
            par_loc: PAR_CS_ID,
            par_token,
            write_loc: TEX_NULL,
            cur_align: TEX_NULL,
            align_ptr: TEX_NULL,
            cur_span: TEX_NULL,
            cur_loop: TEX_NULL,
            cur_head: TEX_NULL,
            cur_tail: TEX_NULL,
            cur_pre_head: TEX_NULL,
            cur_pre_tail: TEX_NULL,
            cond_ptr: TEX_NULL,
            cur_boundary: TEX_NULL,
            cur_val1: 0,
            cur_mlist: TEX_NULL,
            cur_style: 0,
            cur_size: 0,
            cur_mu: 0,
            mlist_penalties: false,
            fil_glue_spec: TEX_NULL,
            math_cur_c: 0,
            math_cur_f: 0,
            math_cur_i: LigKernInsn::default(),
            cur_dir: LEFT_TO_RIGHT,
            insert_src_special_every_math: false,
            insert_src_special_every_display: false,
            insert_src_special_every_vbox: false,
            no_new_control_sequence: true,
            force_eof: false,
            used_tectonic_coda_tokens: false,
            expand_depth_count: 0,
            expand_depth: 10_000,
            is_in_csname: false,
            cur_mark: [TEX_NULL; 5],
            long_state: CommandCode::Call as u8,
            pstack: [TEX_NULL; 9],
            cur_val_level: 0,
            radix: 0,
            cur_order: 0,
            if_limit: 0,
            cur_if: 0,
            if_line: 0,
            skip_line: 0,
            name_in_progress: false,
            cs_table,
        };
        state.init_memory();
        state.init_default_cat_codes();
        state.head = state.contrib_head();
        state.tail = state.contrib_head();
        state.page_tail = state.page_head();
        state.aux.s1 = IGNORE_DEPTH;
        for p in (INT_PAR_BASE..INT_PAR_BASE + INT_PAR_COUNT)
            .chain(DIMEN_PAR_BASE..DIMEN_PAR_BASE + DIMEN_PAR_COUNT)
            .chain(GLUE_PAR_BASE..GLUE_PAR_BASE + GLUE_PAR_COUNT)
            .chain(LOCAL_BASE..LOCAL_BASE + LOCAL_COUNT)
        {
            state.eqtb[p].set_b16_s0(LEVEL_ONE);
        }
        state.eqtb[INT_PAR_BASE + INT_PAR_ESCAPE_CHAR].s1 = i32::from(b'\\');
        state.eqtb[INT_PAR_BASE + INT_PAR_END_LINE_CHAR].s1 = i32::from(b'\r');
        state.eqtb[INT_PAR_BASE + INT_PAR_NEW_LINE_CHAR].s1 = i32::from(b'\n');
        state.eqtb[INT_PAR_BASE + INT_PAR_LEFT_HYPHEN_MIN].s1 = 2;
        state.eqtb[INT_PAR_BASE + INT_PAR_RIGHT_HYPHEN_MIN].s1 = 3;
        state.eqtb[INT_PAR_BASE + INT_PAR_MAG].s1 = 1000;
        state.eqtb[INT_PAR_BASE + INT_PAR_TOLERANCE].s1 = 10000;
        state.eqtb[INT_PAR_BASE + INT_PAR_HANG_AFTER].s1 = 1;
        state.eqtb[INT_PAR_BASE + INT_PAR_MAX_DEAD_CYCLES].s1 = 25;
        state.eqtb[INT_PAR_BASE + INT_PAR_XETEX_HYPHENATABLE_LENGTH].s1 = 63;
        for p in [LOCAL_BASE + LOCAL_PAR_SHAPE]
            .into_iter()
            .chain(ETEX_PEN_BASE..ETEX_PEN_BASE + ETEX_PEN_COUNT)
        {
            state.eqtb[p].set_b16_s0(LEVEL_ONE);
            state.eqtb[p].set_b16_s1(CommandCode::ShapeRef as u16);
            state.eqtb[p].s1 = TEX_NULL;
        }
        for p in CUR_FONT_LOC..=FAMILY_BASE + FAMILY_COUNT - 1 {
            state.eqtb[p].set_b16_s0(LEVEL_ONE);
            state.eqtb[p].set_b16_s1(CommandCode::Data as u16);
            state.eqtb[p].s1 = 0;
        }
        state.font_table[0].name = "nullfont".chars().map(|c| c as i32).collect();
        state.get_strings_started();
        state
    }

    /// Highest valid dynamic-memory index (`MEM_TOP`).
    pub fn mem_top(&self) -> i32 {
        self.mem.len().saturating_sub(1) as i32
    }

    /// Page insertion head (`PAGE_INS_HEAD`).
    pub fn page_ins_head(&self) -> i32 {
        self.mem_top()
    }

    /// Contribution-list head (`CONTRIB_HEAD`).
    pub fn contrib_head(&self) -> i32 {
        self.mem_top() - 1
    }

    /// Current-page head (`PAGE_HEAD`).
    pub fn page_head(&self) -> i32 {
        self.mem_top() - 2
    }

    /// Temporary token/node-list head (`TEMP_HEAD`).
    pub fn temp_head(&self) -> i32 {
        self.mem_top() - 3
    }

    /// Alignment holding-list head (`HOLD_HEAD`).
    pub fn hold_head(&self) -> i32 {
        self.mem_top() - 4
    }

    /// Adjustment holding-list head (`ADJUST_HEAD`).
    pub fn adjust_head(&self) -> i32 {
        self.mem_top() - 5
    }

    /// First word of the active line-break list (`ACTIVE_LIST`).
    pub fn active_list(&self) -> i32 {
        self.mem_top() - 7
    }

    /// Alignment-list head (`ALIGN_HEAD`).
    pub fn align_head(&self) -> i32 {
        self.mem_top() - 8
    }

    /// Alignment end-span sentinel (`END_SPAN`).
    pub fn end_span(&self) -> i32 {
        self.mem_top() - 9
    }

    /// `\omit` template token head (`OMIT_TEMPLATE`).
    pub fn omit_template(&self) -> i32 {
        self.mem_top() - 10
    }

    /// Permanent null-list head (`NULL_LIST`).
    pub fn null_list(&self) -> i32 {
        self.mem_top() - 11
    }

    /// Ligature-trick/garbage word (`LIG_TRICK`, also `GARBAGE`).
    pub fn lig_trick(&self) -> i32 {
        self.mem_top() - 12
    }

    /// Expansion backup-list head (`BACKUP_HEAD`).
    pub fn backup_head(&self) -> i32 {
        self.mem_top() - 13
    }

    /// Pre-adjustment holding-list head (`PRE_ADJUST_HEAD`).
    pub fn pre_adjust_head(&self) -> i32 {
        self.mem_top() - 14
    }

    /// Returns whether `p` is a one-word character token node (`is_char_node`).
    pub fn is_char_node(&self, p: i32) -> bool {
        p >= self.hi_mem_min && p != TEX_NULL
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
        // Reserve the fixed heads at the top of TeX memory. With
        // `MEM_TOP = mem.len() - 1`, `pre_adjust_head` is `MEM_TOP - 14`;
        // `get_avail` decrements before allocating, matching C initialization.
        self.hi_mem_min = self.mem.len().saturating_sub(15) as i32;
        for k in self.pre_adjust_head()..=self.mem_top() {
            self.mem[k as usize] = self.mem[self.lo_mem_max as usize];
        }
        let omit_template = self.omit_template();
        let end_span = self.end_span();
        let active_list = self.active_list();
        let page_ins_head = self.page_ins_head();
        let page_head = self.page_head();
        self.mem[omit_template as usize].s0 =
            crate::scanner::CS_TOKEN_FLAG + crate::scanner::FROZEN_END_TEMPLATE_ID;
        self.mem[end_span as usize].s1 = u16::MAX as i32 + 1;
        self.mem[end_span as usize].s0 = TEX_NULL;
        self.mem[active_list as usize].set_node_type(HYPHENATED);
        self.mem[active_list as usize].set_node_subtype(0);
        self.mem[(active_list + 1) as usize].s0 = MAX_HALFWORD;
        self.mem[page_ins_head as usize].set_node_subtype(255);
        self.mem[page_ins_head as usize].set_node_type(SPLIT_UP);
        self.mem[page_ins_head as usize].s1 = page_ins_head;
        self.mem[page_head as usize].set_node_type(GLUE_NODE);
        self.mem[page_head as usize].set_node_subtype(NORMAL);
        self.avail = TEX_NULL;
    }

    /// Initializes standard TeX default category codes.
    pub fn init_default_cat_codes(&mut self) {
        if self.cat_codes.len() < 256 {
            self.cat_codes.resize(256, 12);
        } else {
            self.cat_codes.fill(12); // OtherChar
        }
        self.cat_code_levels.resize(self.cat_codes.len(), LEVEL_ONE);
        self.cat_code_levels.fill(LEVEL_ONE);

        self.cat_codes[0] = 9; // \0: Ignore
        self.cat_codes[13] = 5; // \r: CarRet
        self.cat_codes[32] = 10; // ' ': Spacer
        self.cat_codes[35] = 6; // '#': MacParam
        self.cat_codes[36] = 3; // '$': MathShift
        self.cat_codes[37] = 14; // '%': Comment
        self.cat_codes[38] = 4; // '&': TabMark
        self.cat_codes[92] = 0; // '\': Escape
        self.cat_codes[94] = 7; // '^': SupMark
        self.cat_codes[95] = 8; // '_': SubMark
        self.cat_codes[123] = 1; // '{': LeftBrace
        self.cat_codes[125] = 2; // '}': RightBrace
        self.cat_codes[127] = 15; // DEL: InvalidChar

        // Letters A-Z and a-z
        for c in b'A'..=b'Z' {
            self.cat_codes[c as usize] = 11;
        }
        for c in b'a'..=b'z' {
            self.cat_codes[c as usize] = 11;
        }
    }

    pub(crate) fn check_save_stack(&mut self) -> Result<(), TeXMemoryError> {
        if self.save_ptr > self.max_save_stack {
            self.max_save_stack = self.save_ptr;
            if self.max_save_stack > self.save_size.saturating_sub(7) {
                return Err(TeXMemoryError::SaveStackOverflow);
            }
        }
        Ok(())
    }

    pub(crate) fn push_save_entry(&mut self, entry: SaveStackEntry) -> Result<(), TeXMemoryError> {
        if self.save_stack.len() == self.save_ptr {
            self.save_stack.push(entry);
        } else if self.save_stack.len() > self.save_ptr {
            self.save_stack[self.save_ptr] = entry;
        } else {
            return Err(TeXMemoryError::OutOfBounds(
                self.save_ptr as i32,
                self.save_stack.len(),
            ));
        }
        self.save_ptr += 1;
        Ok(())
    }

    /// Opens a new save level (`new_save_level`).
    pub fn new_save_level(&mut self, group: u8) -> Result<(), TeXMemoryError> {
        self.check_save_stack()?;
        self.push_save_entry(SaveStackEntry::Line(self.line))?;
        let boundary = self.save_ptr as i32;
        self.push_save_entry(SaveStackEntry::Boundary {
            group: self.cur_group,
            boundary: self.cur_boundary,
        })?;
        if self.cur_level == u16::MAX {
            return Err(TeXMemoryError::GroupingLevelOverflow);
        }
        self.cur_boundary = boundary;
        self.cur_group = group;
        self.cur_level += 1;
        Ok(())
    }

    fn check_eqtb_index(&self, p: usize) -> Result<(), TeXMemoryError> {
        if p >= self.eqtb.len() {
            Err(TeXMemoryError::OutOfBounds(p as i32, self.eqtb.len()))
        } else {
            Ok(())
        }
    }

    /// Saves an `eqtb` value when it crosses a group boundary (`eq_save`).
    pub fn eq_save(&mut self, p: usize, level: u16) -> Result<(), TeXMemoryError> {
        self.check_eqtb_index(p)?;
        self.check_save_stack()?;
        if level != LEVEL_ZERO {
            self.push_save_entry(SaveStackEntry::SavedWord(self.eqtb[p]))?;
        }
        self.push_save_entry(SaveStackEntry::Restore {
            kind: if level == LEVEL_ZERO {
                RESTORE_ZERO
            } else {
                RESTORE_OLD_VALUE
            },
            level,
            target: SaveTarget::Eqtb(p),
        })
    }

    /// Saves a compact control-sequence value (`eq_save` for a cs target).
    pub(crate) fn cs_eq_save(
        &mut self,
        cs: i32,
        level: u16,
        saved: MemoryWord,
    ) -> Result<(), TeXMemoryError> {
        self.check_save_stack()?;
        if level != LEVEL_ZERO {
            self.push_save_entry(SaveStackEntry::SavedWord(saved))?;
        }
        self.push_save_entry(SaveStackEntry::Restore {
            kind: if level == LEVEL_ZERO {
                RESTORE_ZERO
            } else {
                RESTORE_OLD_VALUE
            },
            level,
            target: SaveTarget::ControlSequence(cs),
        })
    }

    /// Saves a character code-table value when it crosses a group boundary.
    pub(crate) fn code_eq_save(
        &mut self,
        table: u8,
        character: u32,
        level: u16,
        saved: MemoryWord,
    ) -> Result<(), TeXMemoryError> {
        self.check_save_stack()?;
        if level != LEVEL_ZERO {
            self.push_save_entry(SaveStackEntry::SavedWord(saved))?;
        }
        self.push_save_entry(SaveStackEntry::Restore {
            kind: if level == LEVEL_ZERO {
                RESTORE_ZERO
            } else {
                RESTORE_OLD_VALUE
            },
            level,
            target: SaveTarget::CharacterCode { table, character },
        })
    }

    /// Releases a saved equivalents-table value (`eq_destroy`).
    pub fn eq_destroy(&mut self, word: MemoryWord) -> Result<(), TeXMemoryError> {
        match word.b16_s1() {
            x if x == CommandCode::Call as u16
                || x == CommandCode::LongCall as u16
                || x == CommandCode::OuterCall as u16
                || x == CommandCode::LongOuterCall as u16 =>
            {
                if word.s1 != TEX_NULL {
                    self.delete_token_ref(word.s1)?;
                }
            }
            x if x == CommandCode::GlueRef as u16 => self.delete_glue_ref(word.s1)?,
            x if x == CommandCode::ShapeRef as u16 => {
                let q = word.s1;
                if q != TEX_NULL {
                    let size = self.word_at(q, 0)?.s0;
                    self.free_node(q, size.saturating_mul(2).saturating_add(1))?;
                }
            }
            x if x == CommandCode::BoxRef as u16 => self.flush_node_list(word.s1)?,
            x if (x == CommandCode::Register as u16 || x == CommandCode::ToksRegister as u16)
                && !(0..=19).contains(&word.s1) =>
            {
                self.delete_sa_ref(word.s1)?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Defines an `eqtb` entry at the current level (`eq_define`).
    pub fn eq_define(&mut self, p: usize, t: u16, e: i32) -> Result<(), TeXMemoryError> {
        self.check_eqtb_index(p)?;
        let old = self.eqtb[p];
        if old.b16_s1() == t && old.s1 == e {
            self.eq_destroy(old)?;
            return Ok(());
        }
        if old.b16_s0() == self.cur_level {
            self.eq_destroy(old)?;
        } else if self.cur_level > LEVEL_ONE {
            self.eq_save(p, old.b16_s0())?;
        }
        self.eqtb[p].set_b16_s0(self.cur_level);
        self.eqtb[p].set_b16_s1(t);
        self.eqtb[p].s1 = e;
        Ok(())
    }

    /// Defines an integer-like `eqtb` word at the current level
    /// (`eq_word_define`).
    pub fn eq_word_define(&mut self, p: usize, w: i32) -> Result<(), TeXMemoryError> {
        self.check_eqtb_index(p)?;
        if self.eqtb[p].s1 == w {
            return Ok(());
        }
        let old_level = self.eqtb[p].b16_s0();
        if old_level != self.cur_level {
            self.eq_save(p, old_level)?;
            self.eqtb[p].set_b16_s0(self.cur_level);
        }
        self.eqtb[p].s1 = w;
        Ok(())
    }

    /// Defines an `eqtb` entry globally (`geq_define`).
    pub fn geq_define(&mut self, p: usize, t: u16, e: i32) -> Result<(), TeXMemoryError> {
        self.check_eqtb_index(p)?;
        self.eq_destroy(self.eqtb[p])?;
        self.eqtb[p].set_b16_s0(LEVEL_ONE);
        self.eqtb[p].set_b16_s1(t);
        self.eqtb[p].s1 = e;
        Ok(())
    }

    /// Defines an integer-like `eqtb` word globally (`geq_word_define`).
    pub fn geq_word_define(&mut self, p: usize, w: i32) -> Result<(), TeXMemoryError> {
        self.check_eqtb_index(p)?;
        self.eqtb[p].s1 = w;
        self.eqtb[p].set_b16_s0(LEVEL_ONE);
        Ok(())
    }

    // =========================================================================
    // e-TeX Sparse Arrays (`sa_*`)
    // =========================================================================

    fn sparse_index(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(i32::from(self.mem[self.check_bounds(p)?].b16_s1()))
    }

    fn sparse_child(&self, parent: i32, index: i32) -> Result<i32, TeXMemoryError> {
        let slot = parent
            .checked_add(index / 2)
            .and_then(|p| p.checked_add(1))
            .ok_or(TeXMemoryError::CorruptedNode(parent))?;
        let idx = self.check_bounds(slot)?;
        Ok(if index & 1 == 0 {
            self.mem[idx].s0
        } else {
            self.mem[idx].s1
        })
    }

    fn set_sparse_child(
        &mut self,
        parent: i32,
        index: i32,
        child: i32,
    ) -> Result<(), TeXMemoryError> {
        let slot = parent
            .checked_add(index / 2)
            .and_then(|p| p.checked_add(1))
            .ok_or(TeXMemoryError::CorruptedNode(parent))?;
        let idx = self.check_bounds(slot)?;
        if index & 1 == 0 {
            self.mem[idx].s0 = child;
        } else {
            self.mem[idx].s1 = child;
        }
        let parent_idx = self.check_bounds(parent)?;
        self.mem[parent_idx].s0 = self.mem[parent_idx].s0.wrapping_add(1);
        Ok(())
    }

    fn new_index(&mut self, index: i32, parent: i32) -> Result<i32, TeXMemoryError> {
        if !(0..=63).contains(&index) {
            return Err(TeXMemoryError::OutOfBounds(index, 64));
        }
        let p = self.get_node(INDEX_NODE_SIZE)?;
        let idx = self.check_bounds(p)?;
        self.mem[idx].set_b16_s1(index as u16);
        self.mem[idx].set_b16_s0(0);
        self.mem[idx].s1 = parent;
        for offset in 1..INDEX_NODE_SIZE {
            self.mem[(p + offset) as usize] = self.sa_null;
        }
        self.cur_ptr = p;
        Ok(p)
    }

    /// Finds or lazily creates a sparse e-TeX element (`find_sa_element`).
    ///
    /// `t` is the sparse class (integer, dimension, glue, mu-glue, box,
    /// token-list, inter-character, or mark); `n` is the four-level index. When
    /// `w` is false this performs lookup only and leaves `cur_ptr` at the found
    /// element or `TEX_NULL`.
    pub fn find_sa_element(&mut self, t: i32, n: i32, w: bool) -> Result<i32, TeXMemoryError> {
        if !(0..SA_ROOT_COUNT as i32).contains(&t) || !(0..=MAX_SA_ELEMENT_NUM).contains(&n) {
            return Err(TeXMemoryError::OutOfBounds(
                n,
                MAX_SA_ELEMENT_NUM as usize + 1,
            ));
        }

        let mut q = self.sa_root[t as usize];
        self.cur_ptr = q;
        if q == TEX_NULL {
            if !w {
                return Ok(TEX_NULL);
            }
            q = self.new_index(t, TEX_NULL)?;
            self.sa_root[t as usize] = q;
        }

        for level in 0..4 {
            let index = match level {
                0 => n / 0x40000,
                1 => (n / 4096) % 64,
                2 => (n / 64) % 64,
                _ => n % 64,
            };
            let child = self.sparse_child(q, index)?;
            self.cur_ptr = child;
            if child != TEX_NULL {
                q = child;
                continue;
            }
            if !w {
                return Ok(TEX_NULL);
            }
            if level < 3 {
                q = self.new_index(index, q)?;
                let parent = self.mem[self.check_bounds(q)?].s1;
                self.set_sparse_child(parent, index, q)?;
            } else {
                self.new_sparse_leaf(t, index, q, n)?;
                let leaf = self.cur_ptr;
                self.set_sparse_child(q, index, leaf)?;
                return Ok(leaf);
            }
        }
        Ok(self.cur_ptr)
    }

    fn new_sparse_leaf(
        &mut self,
        t: i32,
        index: i32,
        parent: i32,
        n: i32,
    ) -> Result<i32, TeXMemoryError> {
        let p = if t == SA_MARK_VAL {
            let p = self.get_node(MARK_CLASS_NODE_SIZE)?;
            for offset in 1..MARK_CLASS_NODE_SIZE {
                self.mem[(p + offset) as usize] = self.sa_null;
            }
            p
        } else if t <= 1 {
            let p = self.get_node(WORD_NODE_SIZE)?;
            self.mem[(p + 1) as usize].s1 = n;
            self.mem[(p + 2) as usize].s1 = 0;
            p
        } else {
            let p = self.get_node(POINTER_NODE_SIZE)?;
            self.mem[(p + 1) as usize].s1 = if t <= 3 { 0 } else { TEX_NULL };
            p
        };

        let idx = self.check_bounds(p)?;
        self.mem[idx].set_b16_s1((64 * t + index) as u16);
        self.mem[idx].set_b16_s0(LEVEL_ONE);
        self.mem[idx].s1 = parent;
        if t != SA_MARK_VAL {
            self.mem[(p + 1) as usize].s0 = TEX_NULL;
        }
        self.cur_ptr = p;
        Ok(p)
    }

    /// Releases one reference to a sparse element and prunes empty index nodes
    /// (`delete_sa_ref`).
    pub fn delete_sa_ref(&mut self, q: i32) -> Result<(), TeXMemoryError> {
        let mut q = q;
        let q_idx = self.check_bounds(q)?;
        self.mem[q_idx + 1].s0 = self.mem[q_idx + 1].s0.wrapping_sub(1);
        if self.mem[q_idx + 1].s0 != TEX_NULL {
            return Ok(());
        }

        let element_type = self.sparse_index(q)?;
        let mut size = if element_type < DIMEN_VAL_LIMIT {
            if self.mem[q_idx + 2].s1 != 0 {
                return Ok(());
            }
            WORD_NODE_SIZE
        } else {
            if element_type < MU_VAL_LIMIT {
                if self.mem[q_idx + 1].s1 != 0 {
                    return Ok(());
                }
                self.delete_glue_ref(0)?;
            } else if self.mem[q_idx + 1].s1 != TEX_NULL {
                return Ok(());
            }
            POINTER_NODE_SIZE
        };

        loop {
            let index = self.sparse_index(q)? % 64;
            let p = q;
            q = self.mem[self.check_bounds(p)?].s1;
            self.free_node(p, size)?;
            if q == TEX_NULL {
                self.sa_root[index as usize] = TEX_NULL;
                return Ok(());
            }

            let parent = q;
            let slot = parent
                .checked_add(index / 2)
                .and_then(|p| p.checked_add(1))
                .ok_or(TeXMemoryError::CorruptedNode(parent))?;
            let slot_idx = self.check_bounds(slot)?;
            if index & 1 == 0 {
                self.mem[slot_idx].s0 = TEX_NULL;
            } else {
                self.mem[slot_idx].s1 = TEX_NULL;
            }
            let parent_idx = self.check_bounds(parent)?;
            let live_children = self.mem[parent_idx].b16_s0().saturating_sub(1);
            self.mem[parent_idx].set_b16_s0(live_children);
            if live_children > 0 {
                return Ok(());
            }
            size = INDEX_NODE_SIZE;
        }
    }

    /// Saves the current sparse element value on this level's chain (`sa_save`).
    pub fn sa_save(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        if self.cur_level != self.sa_level {
            self.check_save_stack()?;
            self.push_save_entry(SaveStackEntry::RestoreSa {
                level: self.sa_level,
                chain: self.sa_chain,
            })?;
            self.sa_chain = TEX_NULL;
            self.sa_level = self.cur_level;
        }

        let p_idx = self.check_bounds(p)?;
        let mut element_type = i32::from(self.mem[p_idx].b16_s1());
        let saved = if element_type < DIMEN_VAL_LIMIT {
            if self.mem[p_idx + 2].s1 == 0 {
                let q = self.get_node(POINTER_NODE_SIZE)?;
                element_type = TOK_VAL_LIMIT;
                self.mem[(q + 1) as usize].s1 = TEX_NULL;
                q
            } else {
                let q = self.get_node(WORD_NODE_SIZE)?;
                self.mem[(q + 1) as usize].s1 = TEX_NULL;
                self.mem[(q + 2) as usize].s1 = self.mem[p_idx + 2].s1;
                q
            }
        } else {
            let q = self.get_node(POINTER_NODE_SIZE)?;
            self.mem[(q + 1) as usize].s1 = self.mem[p_idx + 1].s1;
            q
        };

        let saved_idx = self.check_bounds(saved)?;
        let saved_level = self.mem[p_idx].b16_s0();
        self.mem[saved_idx + 1].s0 = p;
        self.mem[saved_idx].set_b16_s1(element_type as u16);
        self.mem[saved_idx].set_b16_s0(saved_level);
        self.mem[saved_idx].s1 = self.sa_chain;
        self.sa_chain = saved;
        self.mem[p_idx + 1].s0 = self.mem[p_idx + 1].s0.wrapping_add(1);
        Ok(())
    }

    /// Destroys the pointer value held by a sparse element (`sa_destroy`).
    pub fn sa_destroy(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        let element_type = i32::from(self.mem[idx].b16_s1());
        if element_type < MU_VAL_LIMIT {
            self.delete_glue_ref(self.mem[idx + 1].s1)?;
        } else if self.mem[idx + 1].s1 != TEX_NULL {
            if element_type < BOX_VAL_LIMIT {
                self.flush_node_list(self.mem[idx + 1].s1)?;
            } else {
                self.delete_token_ref(self.mem[idx + 1].s1)?;
            }
        }
        Ok(())
    }

    /// Assigns a pointer-valued sparse element locally (`sa_def`).
    pub fn sa_def(&mut self, p: i32, e: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
        if self.mem[idx + 1].s1 == e {
            self.sa_destroy(p)?;
        } else {
            if self.mem[idx].b16_s0() == self.cur_level {
                self.sa_destroy(p)?;
            } else {
                self.sa_save(p)?;
            }
            let idx = self.check_bounds(p)?;
            self.mem[idx].set_b16_s0(self.cur_level);
            self.mem[idx + 1].s1 = e;
        }
        self.delete_sa_ref(p)
    }

    /// Assigns a word-valued sparse element locally (`sa_w_def`).
    pub fn sa_w_def(&mut self, p: i32, w: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
        if self.mem[idx + 2].s1 != w {
            if self.mem[idx].b16_s0() != self.cur_level {
                self.sa_save(p)?;
            }
            let idx = self.check_bounds(p)?;
            self.mem[idx].set_b16_s0(self.cur_level);
            self.mem[idx + 2].s1 = w;
        }
        self.delete_sa_ref(p)
    }

    /// Assigns a pointer-valued sparse element globally (`gsa_def`).
    pub fn gsa_def(&mut self, p: i32, e: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
        self.sa_destroy(p)?;
        let idx = self.check_bounds(p)?;
        self.mem[idx].set_b16_s0(LEVEL_ONE);
        self.mem[idx + 1].s1 = e;
        self.delete_sa_ref(p)
    }

    /// Assigns a word-valued sparse element globally (`gsa_w_def`).
    pub fn gsa_w_def(&mut self, p: i32, w: i32) -> Result<(), TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
        self.mem[idx].set_b16_s0(LEVEL_ONE);
        self.mem[idx + 2].s1 = w;
        self.delete_sa_ref(p)
    }

    /// Restores all sparse values saved on the current level (`sa_restore`).
    pub fn sa_restore(&mut self) -> Result<(), TeXMemoryError> {
        while self.sa_chain != TEX_NULL {
            let saved = self.sa_chain;
            let saved_idx = self.check_bounds(saved)?;
            let p = self.mem[saved_idx + 1].s0;
            let p_idx = self.check_bounds(p)?;
            let p_type = i32::from(self.mem[p_idx].b16_s1());
            let saved_type = i32::from(self.mem[saved_idx].b16_s1());

            if self.mem[p_idx].b16_s0() == LEVEL_ONE {
                if p_type >= DIMEN_VAL_LIMIT {
                    self.sa_destroy(saved)?;
                }
            } else {
                if p_type < DIMEN_VAL_LIMIT {
                    self.mem[p_idx + 2].s1 = if saved_type < DIMEN_VAL_LIMIT {
                        self.mem[saved_idx + 2].s1
                    } else {
                        0
                    };
                } else {
                    self.sa_destroy(p)?;
                    let p_idx = self.check_bounds(p)?;
                    self.mem[p_idx + 1].s1 = self.mem[saved_idx + 1].s1;
                }
                let p_idx = self.check_bounds(p)?;
                let saved_level = self.mem[saved_idx].b16_s0();
                self.mem[p_idx].set_b16_s0(saved_level);
            }

            self.delete_sa_ref(p)?;
            self.sa_chain = self.mem[saved_idx].s1;
            if saved_type < DIMEN_VAL_LIMIT {
                self.free_node(saved, WORD_NODE_SIZE)?;
            } else {
                self.free_node(saved, POINTER_NODE_SIZE)?;
            }
        }
        Ok(())
    }

    /// Recursively walks one sparse mark-class trie (`do_marks`).
    ///
    /// Returns `true` when `q` became empty and was freed. Level values below
    /// four traverse index nodes; level four applies the requested mark action
    /// to a mark-class element.
    pub fn do_marks(&mut self, a: u8, l: u8, q: i32) -> Result<bool, TeXMemoryError> {
        if q == TEX_NULL {
            return Ok(true);
        }

        if l < 4 {
            for index in 0..=15 {
                let child = self.sparse_child(q, index)?;
                self.cur_ptr = child;
                if child != TEX_NULL && self.do_marks(a, l + 1, child)? {
                    let slot = q
                        .checked_add(index / 2)
                        .and_then(|p| p.checked_add(1))
                        .ok_or(TeXMemoryError::CorruptedNode(q))?;
                    let slot_idx = self.check_bounds(slot)?;
                    if index & 1 == 0 {
                        self.mem[slot_idx].s0 = TEX_NULL;
                    } else {
                        self.mem[slot_idx].s1 = TEX_NULL;
                    }
                    let q_idx = self.check_bounds(q)?;
                    let live_children = self.mem[q_idx].b16_s0().saturating_sub(1);
                    self.mem[q_idx].set_b16_s0(live_children);
                }
            }

            let q_idx = self.check_bounds(q)?;
            if self.mem[q_idx].b16_s0() == 0 {
                self.free_node(q, INDEX_NODE_SIZE)?;
                return Ok(true);
            }
            return Ok(false);
        }

        let q_idx = self.check_bounds(q)?;
        match a {
            VSPLIT_INIT => {
                if self.mem[q_idx + 2].s1 != TEX_NULL {
                    self.delete_token_ref(self.mem[q_idx + 2].s1)?;
                    let q_idx = self.check_bounds(q)?;
                    self.mem[q_idx + 2].s1 = TEX_NULL;
                    self.delete_token_ref(self.mem[q_idx + 3].s0)?;
                    self.mem[q_idx + 3].s0 = TEX_NULL;
                }
            }
            FIRE_UP_INIT => {
                if self.mem[q_idx + 2].s0 != TEX_NULL {
                    if self.mem[q_idx + 1].s0 != TEX_NULL {
                        self.delete_token_ref(self.mem[q_idx + 1].s0)?;
                    }
                    let q_idx = self.check_bounds(q)?;
                    self.delete_token_ref(self.mem[q_idx + 1].s1)?;
                    self.mem[q_idx + 1].s1 = TEX_NULL;
                    let bot = self.mem[q_idx + 2].s0;
                    if self.mem[self.check_bounds(bot)?].s1 == TEX_NULL {
                        self.delete_token_ref(bot)?;
                        let q_idx = self.check_bounds(q)?;
                        self.mem[q_idx + 2].s0 = TEX_NULL;
                    } else {
                        let bot_idx = self.check_bounds(bot)?;
                        self.mem[bot_idx].s0 = self.mem[bot_idx].s0.wrapping_add(1);
                    }
                    let q_idx = self.check_bounds(q)?;
                    self.mem[q_idx + 1].s0 = self.mem[q_idx + 2].s0;
                }
            }
            FIRE_UP_DONE => {
                if self.mem[q_idx + 1].s0 != TEX_NULL && self.mem[q_idx + 1].s1 == TEX_NULL {
                    self.mem[q_idx + 1].s1 = self.mem[q_idx + 1].s0;
                    let first = self.mem[q_idx + 1].s0;
                    let first_idx = self.check_bounds(first)?;
                    self.mem[first_idx].s0 = self.mem[first_idx].s0.wrapping_add(1);
                }
            }
            DESTROY_MARKS => {
                for mark_kind in TOP_MARK_CODE..=SPLIT_BOT_MARK_CODE {
                    let mark = if mark_kind & 1 == 0 {
                        self.mem[q_idx + (mark_kind / 2 + 1) as usize].s0
                    } else {
                        self.mem[q_idx + (mark_kind / 2 + 1) as usize].s1
                    };
                    if mark != TEX_NULL {
                        self.delete_token_ref(mark)?;
                        let q_idx = self.check_bounds(q)?;
                        if mark_kind & 1 == 0 {
                            self.mem[q_idx + (mark_kind / 2 + 1) as usize].s0 = TEX_NULL;
                        } else {
                            self.mem[q_idx + (mark_kind / 2 + 1) as usize].s1 = TEX_NULL;
                        }
                    }
                }
            }
            _ => return Err(TeXMemoryError::CorruptedNode(i32::from(a))),
        }

        let q_idx = self.check_bounds(q)?;
        if self.mem[q_idx + 2].s0 == TEX_NULL && self.mem[q_idx + 3].s0 == TEX_NULL {
            self.free_node(q, MARK_CLASS_NODE_SIZE)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns the encoded sparse class of element `p` (`mem[p].b16.s1 / 64`).
    pub fn sparse_element_class(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(self.sparse_index(p)? / 64)
    }

    /// Returns the word value of a sparse integer/dimension element.
    pub fn sparse_word_value(&self, p: i32) -> Result<i32, TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        Ok(self.mem[idx + 2].s1)
    }

    /// Returns the pointer value of a sparse pointer-valued element.
    pub fn sparse_ptr_value(&self, p: i32) -> Result<i32, TeXMemoryError> {
        let idx = self.check_bounds(p)?;
        Ok(self.mem[idx + 1].s1)
    }

    /// Queues a token to be inserted after the current group (`save_for_after`).
    pub fn save_for_after(&mut self, token: i32) -> Result<(), TeXMemoryError> {
        if self.cur_level > LEVEL_ONE {
            self.check_save_stack()?;
            self.push_save_entry(SaveStackEntry::InsertToken(token))?;
        }
        Ok(())
    }

    /// Pushes the current semantic list onto the nest stack (`push_nest`).
    pub fn push_nest(&mut self) -> Result<(), TeXMemoryError> {
        if self.nest_ptr > self.max_nest_stack {
            self.max_nest_stack = self.nest_ptr;
            if self.nest_ptr == self.nest_size {
                return Err(TeXMemoryError::NestStackOverflow);
            }
        }

        let current = ListStateRecord {
            mode: self.mode,
            head: self.head,
            tail: self.tail,
            etex_aux: self.etex_aux,
            prev_graf: self.prev_graf,
            mode_line: self.mode_line,
            aux: self.aux,
        };
        if self.nest.len() == self.nest_ptr {
            self.nest.push(current);
        } else if self.nest.len() > self.nest_ptr {
            self.nest[self.nest_ptr] = current;
        } else {
            return Err(TeXMemoryError::OutOfBounds(
                self.nest_ptr as i32,
                self.nest.len(),
            ));
        }
        self.nest_ptr += 1;

        self.head = self.get_avail()?;
        self.tail = self.head;
        self.prev_graf = 0;
        self.mode_line = self.line;
        self.etex_aux = TEX_NULL;
        Ok(())
    }

    /// Pops the current semantic list (`pop_nest`).
    pub fn pop_nest(&mut self) -> Result<(), TeXMemoryError> {
        if self.nest_ptr == 0 {
            return Err(TeXMemoryError::NestStackUnderflow);
        }
        self.free_avail(self.head)?;
        self.nest_ptr -= 1;
        let parent = self.nest[self.nest_ptr];
        self.mode = parent.mode;
        self.head = parent.head;
        self.tail = parent.tail;
        self.etex_aux = parent.etex_aux;
        self.prev_graf = parent.prev_graf;
        self.mode_line = parent.mode_line;
        self.aux = parent.aux;
        Ok(())
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
    pub(crate) fn check_bounds(&self, p: i32) -> Result<usize, TeXMemoryError> {
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

    pub(crate) fn word_at(&self, p: i32, offset: i32) -> Result<MemoryWord, TeXMemoryError> {
        let p = p
            .checked_add(offset)
            .ok_or(TeXMemoryError::CorruptedNode(p))?;
        Ok(self.mem[self.check_bounds(p)?])
    }

    /// Recursively frees a semantic node list (`flush_node_list`).
    pub fn flush_node_list(&mut self, mut p: i32) -> Result<(), TeXMemoryError> {
        while p != TEX_NULL {
            let word = self.word_at(p, 0)?;
            let next = word.s1;
            if p >= self.hi_mem_min {
                self.mem[p as usize].s1 = self.avail;
                self.avail = p;
                p = next;
                continue;
            }

            match word.b16_s1() {
                HLIST_NODE | VLIST_NODE | UNSET_NODE => {
                    let list = self.word_at(p, 5)?.s1;
                    self.flush_node_list(list)?;
                    self.free_node(p, BOX_NODE_SIZE)?;
                }
                RULE_NODE => self.free_node(p, RULE_NODE_SIZE)?,
                INS_NODE => {
                    let contents = self.word_at(p, 4)?.s0;
                    let split_top = self.word_at(p, 4)?.s1;
                    self.flush_node_list(contents)?;
                    self.delete_glue_ref(split_top)?;
                    self.free_node(p, INS_NODE_SIZE)?;
                }
                WHATSIT_NODE => match word.b16_s0() {
                    OPEN_NODE => self.free_node(p, OPEN_NODE_SIZE)?,
                    WRITE_NODE | SPECIAL_NODE => {
                        let tokens = self.word_at(p, 1)?.s1;
                        self.delete_token_ref(tokens)?;
                        self.free_node(p, SMALL_NODE_SIZE)?;
                    }
                    CLOSE_NODE | LANGUAGE_NODE | PDF_SAVE_POS_NODE => {
                        self.free_node(p, SMALL_NODE_SIZE)?;
                    }
                    NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT => {
                        let size = self.word_at(p, 4)?.b16_s3() as i32;
                        self.free_node(p, size)?;
                    }
                    GLYPH_NODE => self.free_node(p, GLYPH_NODE_SIZE)?,
                    PIC_NODE | PDF_NODE => {
                        let path_len = self.word_at(p, 4)?.b16_s1() as i32;
                        self.free_node(p, PIC_NODE_SIZE + (path_len + 7) / 8)?;
                    }
                    _ => return Err(TeXMemoryError::CorruptedNode(p)),
                },
                GLUE_NODE => {
                    let leader = self.word_at(p, 1)?;
                    self.delete_glue_ref(leader.s0)?;
                    if leader.s1 != TEX_NULL {
                        self.flush_node_list(leader.s1)?;
                    }
                    self.free_node(p, MEDIUM_NODE_SIZE)?;
                }
                KERN_NODE | MATH_NODE | PENALTY_NODE | MARGIN_KERN_NODE => {
                    self.free_node(p, MEDIUM_NODE_SIZE)?;
                }
                LIGATURE_NODE => {
                    let ligature = self.word_at(p, 1)?.s1;
                    self.flush_node_list(ligature)?;
                    self.free_node(p, SMALL_NODE_SIZE)?;
                }
                MARK_NODE => {
                    let tokens = self.word_at(p, 1)?.s1;
                    self.delete_token_ref(tokens)?;
                    self.free_node(p, SMALL_NODE_SIZE)?;
                }
                DISC_NODE => {
                    let pre = self.word_at(p, 1)?.s0;
                    let post = self.word_at(p, 1)?.s1;
                    self.flush_node_list(pre)?;
                    self.flush_node_list(post)?;
                    self.free_node(p, SMALL_NODE_SIZE)?;
                }
                ADJUST_NODE => {
                    let adjustment = self.word_at(p, 1)?.s1;
                    self.flush_node_list(adjustment)?;
                    self.free_node(p, SMALL_NODE_SIZE)?;
                }
                STYLE_NODE => self.free_node(p, STYLE_NODE_SIZE)?,
                CHOICE_NODE => {
                    let display = self.word_at(p, 1)?.s0;
                    let text = self.word_at(p, 1)?.s1;
                    let script = self.word_at(p, 2)?.s0;
                    let script_script = self.word_at(p, 2)?.s1;
                    self.flush_node_list(display)?;
                    self.flush_node_list(text)?;
                    self.flush_node_list(script)?;
                    self.flush_node_list(script_script)?;
                    self.free_node(p, STYLE_NODE_SIZE)?;
                }
                ORD_NOAD | OP_NOAD | BIN_NOAD | REL_NOAD | OPEN_NOAD | CLOSE_NOAD | PUNCT_NOAD
                | INNER_NOAD | RADICAL_NOAD | OVER_NOAD | UNDER_NOAD | VCENTER_NOAD
                | ACCENT_NOAD => {
                    for offset in 1..=3 {
                        let field = self.word_at(p, offset)?;
                        if field.s1 >= SUB_BOX {
                            self.flush_node_list(field.s0)?;
                        }
                    }
                    if word.b16_s1() == RADICAL_NOAD {
                        self.free_node(p, RADICAL_NOAD_SIZE)?;
                    } else if word.b16_s1() == ACCENT_NOAD {
                        self.free_node(p, ACCENT_NOAD_SIZE)?;
                    } else {
                        self.free_node(p, NOAD_SIZE)?;
                    }
                }
                LEFT_NOAD | RIGHT_NOAD => self.free_node(p, NOAD_SIZE)?,
                FRACTION_NOAD => {
                    let numerator = self.word_at(p, 2)?.s0;
                    let denominator = self.word_at(p, 3)?.s0;
                    self.flush_node_list(numerator)?;
                    self.flush_node_list(denominator)?;
                    self.free_node(p, FRACTION_NOAD_SIZE)?;
                }
                _ => return Err(TeXMemoryError::CorruptedNode(p)),
            }
            p = next;
        }
        Ok(())
    }

    /// Recursively copies a semantic node list (`copy_node_list`).
    ///
    /// Glue and token-list references are retained exactly as in C; nested box,
    /// leader, ligature, discretionary, insertion, and adjustment lists are
    /// recursively copied.
    pub fn copy_node_list(&mut self, mut p: i32) -> Result<i32, TeXMemoryError> {
        let h = self.get_avail()?;
        let mut q = h;

        while p != TEX_NULL {
            let mut words = 1i32;
            let r = if self.is_char_node(p) {
                self.get_avail()?
            } else {
                let node_type = self.node_type(p)?;
                match node_type {
                    HLIST_NODE | VLIST_NODE | UNSET_NODE => {
                        let r = self.get_node(BOX_NODE_SIZE)?;
                        self.mem[(r + 6) as usize] = self.mem[(p + 6) as usize];
                        self.mem[(r + 5) as usize] = self.mem[(p + 5) as usize];
                        let list = self.mem[(p + 5) as usize].s1;
                        let copied = self.copy_node_list(list)?;
                        self.mem[(r + 5) as usize].s1 = copied;
                        words = 5;
                        r
                    }
                    RULE_NODE => {
                        words = RULE_NODE_SIZE - 1;
                        self.get_node(RULE_NODE_SIZE)?
                    }
                    INS_NODE => {
                        let r = self.get_node(INS_NODE_SIZE)?;
                        self.mem[(r + 4) as usize] = self.mem[(p + 4) as usize];
                        self.add_glue_ref(self.mem[(p + 4) as usize].s1)?;
                        let contents = self.mem[(p + 4) as usize].s0;
                        let copied = self.copy_node_list(contents)?;
                        self.mem[(r + 4) as usize].s0 = copied;
                        words = INS_NODE_SIZE - 1;
                        r
                    }
                    WHATSIT_NODE => match self.node_subtype(p)? {
                        OPEN_NODE => {
                            words = OPEN_NODE_SIZE;
                            self.get_node(OPEN_NODE_SIZE)?
                        }
                        WRITE_NODE | SPECIAL_NODE => {
                            let r = self.get_node(SMALL_NODE_SIZE)?;
                            let tokens = self.word_at(p, 1)?.s1;
                            let idx = self.check_bounds(tokens)?;
                            self.mem[idx].s0 += 1;
                            words = SMALL_NODE_SIZE;
                            r
                        }
                        CLOSE_NODE | LANGUAGE_NODE | PDF_SAVE_POS_NODE => {
                            words = SMALL_NODE_SIZE;
                            self.get_node(SMALL_NODE_SIZE)?
                        }
                        NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT => {
                            words = self.word_at(p, 4)?.b16_s3() as i32;
                            let r = self.get_node(words)?;
                            for offset in 0..words {
                                self.mem[(r + offset) as usize] = self.mem[(p + offset) as usize];
                            }
                            words = 0;
                            r
                        }
                        GLYPH_NODE => {
                            words = GLYPH_NODE_SIZE;
                            self.get_node(GLYPH_NODE_SIZE)?
                        }
                        PIC_NODE | PDF_NODE => {
                            let path_len = self.word_at(p, 4)?.b16_s1() as i32;
                            words = PIC_NODE_SIZE + (path_len + 7) / 8;
                            self.get_node(words)?
                        }
                        _ => return Err(TeXMemoryError::CorruptedNode(p)),
                    },
                    GLUE_NODE => {
                        let r = self.get_node(MEDIUM_NODE_SIZE)?;
                        self.add_glue_ref(self.word_at(p, 1)?.s0)?;
                        self.mem[(r + 2) as usize] = self.mem[(p + 2) as usize];
                        self.mem[(r + 1) as usize].s0 = self.mem[(p + 1) as usize].s0;
                        let leader = self.word_at(p, 1)?.s1;
                        let copied = self.copy_node_list(leader)?;
                        self.mem[(r + 1) as usize].s1 = copied;
                        r
                    }
                    KERN_NODE | MATH_NODE | PENALTY_NODE => {
                        words = MEDIUM_NODE_SIZE;
                        self.get_node(MEDIUM_NODE_SIZE)?
                    }
                    MARGIN_KERN_NODE => {
                        words = MEDIUM_NODE_SIZE;
                        self.get_node(MEDIUM_NODE_SIZE)?
                    }
                    LIGATURE_NODE => {
                        let r = self.get_node(SMALL_NODE_SIZE)?;
                        self.mem[(r + 1) as usize] = self.mem[(p + 1) as usize];
                        let ligature = self.word_at(p, 1)?.s1;
                        let copied = self.copy_node_list(ligature)?;
                        self.mem[(r + 1) as usize].s1 = copied;
                        r
                    }
                    DISC_NODE => {
                        let r = self.get_node(SMALL_NODE_SIZE)?;
                        let pre = self.word_at(p, 1)?.s0;
                        let post = self.word_at(p, 1)?.s1;
                        let copied_pre = self.copy_node_list(pre)?;
                        let copied_post = self.copy_node_list(post)?;
                        self.mem[(r + 1) as usize].s0 = copied_pre;
                        self.mem[(r + 1) as usize].s1 = copied_post;
                        r
                    }
                    MARK_NODE => {
                        let r = self.get_node(SMALL_NODE_SIZE)?;
                        let tokens = self.word_at(p, 1)?.s1;
                        let idx = self.check_bounds(tokens)?;
                        self.mem[idx].s0 += 1;
                        words = SMALL_NODE_SIZE;
                        r
                    }
                    ADJUST_NODE => {
                        let r = self.get_node(SMALL_NODE_SIZE)?;
                        let adjustment = self.word_at(p, 1)?.s1;
                        let copied = self.copy_node_list(adjustment)?;
                        self.mem[(r + 1) as usize].s1 = copied;
                        r
                    }
                    _ => return Err(TeXMemoryError::CorruptedNode(p)),
                }
            };

            while words > 0 {
                words -= 1;
                self.mem[(r + words) as usize] = self.mem[(p + words) as usize];
            }
            self.mem[q as usize].s1 = r;
            q = r;
            p = self.link(p)?;
        }

        self.mem[q as usize].s1 = TEX_NULL;
        let result = self.mem[h as usize].s1;
        self.mem[h as usize].s1 = self.avail;
        self.avail = h;
        Ok(result)
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
    ///
    /// Matches C: `leader_ptr` is nulled and the spec's reference count is
    /// incremented for the new reference.
    pub fn new_glue(&mut self, q: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(MEDIUM_NODE_SIZE)?;
        self.set_node_type(p, GLUE_NODE)?;
        self.set_node_subtype(p, NORMAL)?;
        self.mem[(p + 1) as usize].s0 = q;
        self.mem[(p + 1) as usize].s1 = TEX_NULL;
        self.add_glue_ref(q)?;
        Ok(p)
    }

    /// Clones a glue specification node `p` (`new_spec`).
    ///
    /// Matches C: the copy keeps the width/stretch/shrink words and the
    /// stretch/shrink orders of `p`, but its `glue_ref_count` is reset to
    /// `TEX_NULL`.
    pub fn new_spec(&mut self, p: i32) -> Result<i32, TeXMemoryError> {
        self.check_bounds(p)?;
        self.check_bounds(p + 3)?;
        let q = self.get_node(GLUE_SPEC_SIZE)?;
        self.mem[q as usize] = self.mem[p as usize];
        self.mem[q as usize].s1 = TEX_NULL;
        self.mem[(q + 1) as usize] = self.mem[(p + 1) as usize];
        self.mem[(q + 2) as usize] = self.mem[(p + 2) as usize];
        self.mem[(q + 3) as usize] = self.mem[(p + 3) as usize];
        Ok(q)
    }

    /// Returns the shared `fil_glue` spec, allocating it on first use.
    ///
    /// C pre-populates lo-mem with the shared glue specs (`fil_glue` is
    /// spec 12); the compact arena allocates it lazily and pins its
    /// reference count so `delete_glue_ref` never frees it.
    pub fn fil_glue(&mut self) -> Result<i32, TeXMemoryError> {
        if self.fil_glue_spec == TEX_NULL {
            let q = self.get_node(GLUE_SPEC_SIZE)?;
            self.mem[q as usize].s1 = 1; // pinned refcount
            self.mem[q as usize].set_b16_s1(FIL);
            self.mem[q as usize].set_b16_s0(NORMAL);
            self.mem[(q + 1) as usize].s1 = 0;
            self.mem[(q + 2) as usize].s1 = 65536;
            self.mem[(q + 3) as usize].s1 = 0;
            self.fil_glue_spec = q;
        }
        Ok(self.fil_glue_spec)
    }

    /// Creates a glue node referencing parameter glue register `n` (`new_param_glue`).
    pub fn new_param_glue(&mut self, n: usize) -> Result<i32, TeXMemoryError> {
        let q = self.glue_par(n)?;
        let p = self.new_glue(q)?;
        self.set_node_subtype(p, (n + 1) as u16)?;
        Ok(p)
    }

    /// Adds one reference to glue specification node `p` (`add_glue_ref`).
    ///
    /// `p == TEX_NULL` is a no-op: the compact eqtb stores `TEX_NULL` for
    /// unset glue registers rather than pointing at a shared `zero_glue`
    /// node as the C layout does.
    pub fn add_glue_ref(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        if p == TEX_NULL {
            return Ok(());
        }
        let idx = self.check_bounds(p)?;
        self.mem[idx].s1 += 1;
        Ok(())
    }

    /// Releases one reference to glue specification node `p`
    /// (`delete_glue_ref`).
    ///
    /// A spec whose `glue_ref_count` is `TEX_NULL` is returned to free
    /// memory; otherwise the count is decremented. `p == TEX_NULL` is a
    /// no-op for the same reason as [`TeXEngineState::add_glue_ref`].
    pub fn delete_glue_ref(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        if p == TEX_NULL {
            return Ok(());
        }
        let idx = self.check_bounds(p)?;
        if self.mem[idx].s1 == TEX_NULL {
            self.free_node(p, GLUE_SPEC_SIZE)?;
        } else {
            self.mem[idx].s1 -= 1;
        }
        Ok(())
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

    /// Creates a discretionary node (`new_disc`).
    pub fn new_disc(&mut self) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(SMALL_NODE_SIZE)?;
        self.set_node_type(p, DISC_NODE)?;
        self.set_node_subtype(p, 0)?;
        self.mem[(p + 1) as usize].s0 = TEX_NULL;
        self.mem[(p + 1) as usize].s1 = TEX_NULL;
        Ok(p)
    }

    /// Creates a math boundary node (`new_math`).
    pub fn new_math(&mut self, w: i32, subtype: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(MEDIUM_NODE_SIZE)?;
        self.set_node_type(p, MATH_NODE)?;
        self.set_node_subtype(p, subtype as u16)?;
        self.mem[(p + 1) as usize].s1 = w;
        Ok(p)
    }

    /// Creates a margin-kern node (`new_margin_kern`).
    pub fn new_margin_kern(&mut self, w: i32, side: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(MEDIUM_NODE_SIZE)?;
        self.set_node_type(p, MARGIN_KERN_NODE)?;
        self.set_node_subtype(p, side as u16)?;
        self.mem[(p + 1) as usize].s1 = w;
        Ok(p)
    }

    /// Creates a glue node containing a private copy of glue parameter `n`
    /// (`new_skip_param`). The copied spec is exposed through `temp_ptr`.
    pub fn new_skip_param(&mut self, n: usize) -> Result<i32, TeXMemoryError> {
        self.temp_ptr = self.new_spec(self.glue_par(n)?)?;
        let p = self.new_glue(self.temp_ptr)?;
        self.mem[self.temp_ptr as usize].s1 = TEX_NULL;
        self.set_node_subtype(p, (n + 1) as u16)?;
        Ok(p)
    }

    /// Creates an empty math noad (`new_noad`).
    pub fn new_noad(&mut self) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(NOAD_SIZE)?;
        self.set_node_type(p, ORD_NOAD)?;
        self.set_node_subtype(p, NORMAL)?;
        for offset in 1..=3 {
            self.mem[(p + offset) as usize] = MemoryWord::default();
        }
        Ok(p)
    }

    /// Appends a box to the current vertical list (`append_to_vlist`).
    ///
    /// Interline glue follows C exactly: baseline glue is used when the
    /// calculated separation is at least `\lineskiplimit`; otherwise a
    /// parameter glue node for `\lineskip` is inserted.
    pub fn append_to_vlist(&mut self, b: i32) -> Result<(), TeXMemoryError> {
        let upwards = self.int_par(INT_PAR_XETEX_UPWARDS)? > 0;
        if self.aux.s1 > IGNORE_DEPTH {
            let baseline = self.word_at(self.glue_par(GLUE_PAR_BASELINE_SKIP)?, 1)?.s1;
            let box_dimen = self.word_at(b, if upwards { 2 } else { 3 })?.s1;
            let d = baseline - self.aux.s1 - box_dimen;
            let p = if d < self.dimen_par(DIMEN_PAR_LINE_SKIP_LIMIT)? {
                self.new_param_glue(GLUE_PAR_LINE_SKIP)?
            } else {
                let p = self.new_skip_param(GLUE_PAR_BASELINE_SKIP)?;
                self.mem[(self.temp_ptr + 1) as usize].s1 = d;
                p
            };
            self.mem[self.tail as usize].s1 = p;
            self.tail = p;
        }
        self.mem[self.tail as usize].s1 = b;
        self.tail = b;
        self.aux.s1 = self.word_at(b, if upwards { 3 } else { 2 })?.s1;
        Ok(())
    }

    /// Returns fixed-point metrics for a font character (`FONT_CHARACTER_INFO`).
    ///
    /// Missing metrics intentionally read as TeX's null character metrics;
    /// font loading populates this state-local table.
    pub fn font_char_metric(&self, font: i32, character: u32) -> FontCharMetric {
        self.font_char_metrics
            .get(&(font, character))
            .copied()
            .unwrap_or_default()
    }

    /// Installs one fixed-point character metric entry.
    pub fn set_font_char_metric(&mut self, font: i32, character: u32, metric: FontCharMetric) {
        self.font_char_metrics.insert((font, character), metric);
    }

    /// Returns the current font number (`eqtb[CUR_FONT_LOC]`).
    pub fn current_font(&self) -> i32 {
        self.eqtb[CUR_FONT_LOC].s1
    }

    /// Returns the largest allocated font number (`font_ptr`).
    pub fn font_ptr(&self) -> i32 {
        self.font_table.len().saturating_sub(1) as i32
    }

    /// Returns the record for internal font number `f`.
    pub fn font_record(&self, f: i32) -> Option<&FontRecord> {
        usize::try_from(f)
            .ok()
            .and_then(|index| self.font_table.get(index))
    }

    /// Returns the mutable record for internal font number `f`.
    pub fn font_record_mut(&mut self, f: i32) -> Option<&mut FontRecord> {
        usize::try_from(f)
            .ok()
            .and_then(|index| self.font_table.get_mut(index))
    }

    /// Returns whether `f` is a native platform font (`AAT`/`OTGR` area flag).
    pub fn is_native_font(&self, f: i32) -> bool {
        matches!(self.font_area.get(&f), Some(&flag) if flag == AAT_FONT_FLAG || flag == OTGR_FONT_FLAG)
    }

    /// Returns a character-protrusion code (`get_cp_code`).
    pub fn get_cp_code(&self, f: i32, k: u32, left_side: bool) -> i32 {
        let Some(record) = self.font_record(f) else {
            return 0;
        };
        let table = if left_side {
            &record.lp_codes
        } else {
            &record.rp_codes
        };
        table.get(&k).copied().unwrap_or(0)
    }

    /// Assigns a character-protrusion code (`set_cp_code`).
    pub fn set_cp_code(&mut self, f: i32, k: u32, left_side: bool, value: i32) {
        let Some(record) = self.font_record_mut(f) else {
            return;
        };
        let table = if left_side {
            &mut record.lp_codes
        } else {
            &mut record.rp_codes
        };
        if value == 0 {
            table.remove(&k);
        } else {
            table.insert(k, value);
        }
    }

    /// Maps a glyph name to a glyph index for a native font
    /// (`map_glyph_to_index`).
    pub fn map_glyph_to_index(&self, f: i32, name: &str) -> i32 {
        self.font_record(f)
            .and_then(|record| record.glyph_names.get(name).copied())
            .map_or(0, |index| index as i32)
    }

    /// Maps a Unicode scalar to a glyph index for a native font
    /// (`map_char_to_glyph`).
    pub fn map_char_to_glyph(&self, f: i32, character: u32) -> i32 {
        self.font_record(f)
            .and_then(|record| record.char_to_glyph.get(&character).copied())
            .map_or(0, |index| index as i32)
    }

    /// Returns the registered per-glyph metrics for native font `f`.
    pub fn native_glyph_metric(&self, f: i32, gid: u32) -> NativeGlyphMetric {
        self.font_record(f)
            .and_then(|record| record.glyph_metrics.get(&gid).copied())
            .unwrap_or_default()
    }

    /// Returns a native character's advance width (`getnativecharwd`): the
    /// character maps through `char_to_glyph` to a glyph whose metrics are
    /// then consulted.
    pub fn get_native_char_wd(&self, f: i32, c: u32) -> i32 {
        let gid = self.map_char_to_glyph(f, c);
        self.native_glyph_metric(f, gid.max(0) as u32).width
    }

    /// Returns a native character's height and depth
    /// (`get_native_char_height_depth`), applying C's snap zones: depth snaps
    /// to 0 and height snaps to 0, x-height, or cap-height when within
    /// `quad/25` of the zone.
    pub fn get_native_char_height_depth(&self, f: i32, c: u32) -> (i32, i32) {
        let gid = self.map_char_to_glyph(f, c);
        let metric = self.native_glyph_metric(f, gid.max(0) as u32);
        let (mut height, mut depth) = (metric.height, metric.depth);
        let Some(record) = self.font_record(f) else {
            return (height, depth);
        };
        let param = |code: i32| {
            usize::try_from(code)
                .ok()
                .and_then(|index| record.params.get(index).copied())
                .unwrap_or(0)
        };
        let fuzz = i64::from(param(QUAD_CODE)) / 25;
        let snap = |value: &mut i32, zone: i32| {
            let difference = i64::from(*value) - i64::from(zone);
            if difference <= fuzz && difference >= -fuzz {
                *value = zone;
            }
        };
        snap(&mut depth, 0);
        snap(&mut height, 0);
        snap(&mut height, param(X_HEIGHT_CODE));
        snap(&mut height, param(CAP_HEIGHT_CODE));
        (height, depth)
    }

    /// Returns a native character's italic correction (`getnativecharic`):
    /// `letter_space - rsb` when the right sidebearing is negative, else
    /// `letter_space`.
    pub fn get_native_char_ic(&self, f: i32, c: u32) -> i32 {
        let gid = self.map_char_to_glyph(f, c);
        let metric = self.native_glyph_metric(f, gid.max(0) as u32);
        let letter_space = self.font_record(f).map_or(0, |record| record.letter_space);
        if metric.rsb < 0 {
            letter_space - metric.rsb
        } else {
            letter_space
        }
    }

    /// Returns one edge of a native glyph's bounds (`get_glyph_bounds`):
    /// edges 1 and 3 report the left and right sidebearings, edges 2 and 4
    /// report the height and depth.
    pub fn get_glyph_bounds(&self, f: i32, edge: i32, gid: u32) -> i32 {
        let metric = self.native_glyph_metric(f, gid);
        let (a, b) = if edge & 1 != 0 {
            (metric.lsb, metric.rsb)
        } else {
            (metric.height, metric.depth)
        };
        if edge <= 2 {
            a
        } else {
            b
        }
    }

    /// Returns the first or last character covered by font `f`
    /// (`get_font_char_range`); for both TFM and native records this is the
    /// registered `bc`/`ec` range.
    pub fn get_font_char_range(&self, f: i32, first: bool) -> i32 {
        self.font_record(f).map_or(0, |record| {
            if first {
                record.bc as i32
            } else {
                record.ec as i32
            }
        })
    }

    /// Returns whether `f` carries the AAT font-area flag.
    pub fn is_aat_font(&self, f: i32) -> bool {
        self.font_area.get(&f) == Some(&AAT_FONT_FLAG)
    }

    /// Returns whether `f` is an `OTGR` font whose engine class is Graphite
    /// (C's `usingGraphite(font_layout_engine[f])`).
    pub fn using_graphite(&self, f: i32) -> bool {
        self.font_area.get(&f) == Some(&OTGR_FONT_FLAG)
            && self
                .font_record(f)
                .is_some_and(|record| record.font_type == 3)
    }

    /// Returns whether `f` is an `OTGR` font whose engine class is OpenType
    /// (C's `usingOpenType(font_layout_engine[f])`).
    pub fn using_open_type(&self, f: i32) -> bool {
        self.font_area.get(&f) == Some(&OTGR_FONT_FLAG)
            && self
                .font_record(f)
                .is_some_and(|record| record.font_type == 2)
    }

    /// Returns whether `f` is an OTGR font with an OpenType MATH table
    /// (C's `isOpenTypeMathFont(font_layout_engine[f])`).
    pub fn is_ot_math_font(&self, f: i32) -> bool {
        self.using_open_type(f)
            && self
                .font_record(f)
                .is_some_and(|record| record.has_math_table)
    }

    /// Math symbol-font parameter `n` of `f` (`mathsy` parameter access):
    /// OpenType MATH fonts read the parameter map; TFM fonts read
    /// `font_info[n + param_base[f]]`.
    pub fn mathsy_param(&self, f: i32, n: i32) -> i32 {
        if self.is_ot_math_font(f) {
            return self
                .font_record(f)
                .and_then(|record| record.mathsy_params.get(&n).copied())
                .unwrap_or(0);
        }
        self.font_record(f)
            .and_then(|record| usize::try_from(n).ok().and_then(|i| record.params.get(i)))
            .copied()
            .unwrap_or(0)
    }

    /// Math extension-font parameter `n` of `f` (`mathex` parameter
    /// access): OpenType MATH fonts read the parameter map; TFM fonts
    /// read `font_info[n + param_base[f]]`.
    pub fn mathex_param(&self, f: i32, n: i32) -> i32 {
        if self.is_ot_math_font(f) {
            return self
                .font_record(f)
                .and_then(|record| record.mathex_params.get(&n).copied())
                .unwrap_or(0);
        }
        self.font_record(f)
            .and_then(|record| usize::try_from(n).ok().and_then(|i| record.params.get(i)))
            .copied()
            .unwrap_or(0)
    }

    /// OpenType MATH table constant `n` of `f` (`get_ot_math_constant`),
    /// scaled to points by the layout engine; `0` when absent.
    pub fn ot_math_constant(&self, f: i32, n: i32) -> i32 {
        self.font_record(f)
            .and_then(|record| record.math_constants.get(&n).copied())
            .unwrap_or(0)
    }

    /// Step `n` of the math-variant chain for `c` in direction `horiz`
    /// (`get_ot_math_variant`): `(glyph, advance)`; a negative advance
    /// ends the chain, matching the C loop condition.
    pub fn ot_math_variant(&self, f: i32, c: i32, n: i32, horiz: u8) -> (i32, i32) {
        self.font_record(f)
            .and_then(|record| {
                record
                    .math_variants
                    .get(&(c.max(0) as u32, horiz, n))
                    .copied()
            })
            .map_or((0, -1), |(g, adv)| (g as i32, adv))
    }

    /// Math italic correction of glyph `g` (`get_ot_math_ital_corr`).
    pub fn ot_math_ital_corr(&self, f: i32, g: i32) -> i32 {
        self.font_record(f)
            .and_then(|record| record.math_ital_corr.get(&(g.max(0) as u32)).copied())
            .unwrap_or(0)
    }

    /// Top-accent attachment position of glyph `g`
    /// (`get_ot_math_accent_pos`); `TEX_INFINITY` when the font has no
    /// entry for the glyph.
    pub fn ot_math_accent_pos(&self, f: i32, g: i32) -> i32 {
        self.font_record(f)
            .and_then(|record| record.accent_positions.get(&(g.max(0) as u32)).copied())
            .unwrap_or(0x7FFF_FFFF)
    }

    /// Blended math kern between base glyph `g` and script glyph `sg`
    /// at shift `shift` (`get_ot_math_kern` with `SUP_CMD`/`SUB_CMD`):
    /// the stored table holds the layout engine's `(correction height,
    /// kern)` pairs, and the kern of the largest height below `shift` is
    /// selected.
    pub fn ot_math_kern(&self, f: i32, g: i32, sf: i32, sg: i32, cmd: u8, shift: i32) -> i32 {
        if !(self.using_open_type(f) && self.using_open_type(sf)) {
            return 0;
        }
        let Some(record) = self.font_record(f) else {
            return 0;
        };
        let Some(pairs) = record
            .math_kerns
            .get(&(g.max(0) as u32, sg.max(0) as u32, cmd))
        else {
            return 0;
        };
        let mut kern = 0;
        for &(height, value) in pairs {
            if height < shift {
                kern = value;
            }
        }
        kern
    }

    /// Extensible assembly of glyph `g` in direction `horiz`
    /// (`get_ot_assembly_ptr` data).
    pub fn ot_math_assembly(&self, f: i32, g: i32, horiz: u8) -> Option<OtMathAssembly> {
        self.font_record(f).and_then(|record| {
            record
                .math_assemblies
                .get(&(g.max(0) as u32, horiz))
                .cloned()
        })
    }

    /// Minimum connector overlap of `f` (`ot_min_connector_overlap`).
    pub fn ot_min_connector_overlap(&self, f: i32) -> i32 {
        self.font_record(f)
            .map_or(0, |record| record.min_connector_overlap)
    }

    /// Extensible recipe `i` of `f` (`font_info[exten_base[f] + i]`):
    /// `[rep, top, bot, mid]` character codes.
    pub fn exten_recipe(&self, f: i32, i: i32) -> [i32; 4] {
        self.font_record(f)
            .and_then(|record| record.exten_recipes.get(&i).copied())
            .unwrap_or([0; 4])
    }

    /// Whether `p` is a `GLYPH_NODE` whatsit (`is_glyph_node`).
    pub fn is_glyph_node(&self, p: i32) -> bool {
        p != TEX_NULL
            && !self.is_char_node(p)
            && self.mem[p as usize].b16_s1() == WHATSIT_NODE
            && self.mem[p as usize].b16_s0() == GLYPH_NODE
    }

    /// Whether `p` is a valid dynamic-memory pointer
    /// (`is_valid_pointer`).
    pub fn is_valid_mem_ptr(&self, p: i32) -> bool {
        0 <= p && p <= self.mem_top()
    }

    /// Recomputes the metrics of `GLYPH_NODE` `p`
    /// (`set_native_glyph_metrics`/`measure_native_glyph`): width always
    /// comes from the glyph metric; height/depth fall back to the font's
    /// base metrics when `use_glyph_metrics` is clear.
    pub fn set_native_glyph_metrics(&mut self, p: i32, use_glyph_metrics: bool) {
        let info = self.mem[(p + 4) as usize];
        let font = i32::from(info.b16_s2());
        let glyph = i32::from(info.b16_s1());
        let metric = self.native_glyph_metric(font, glyph.max(0) as u32);
        self.mem[(p + 1) as usize].s1 = metric.width;
        if use_glyph_metrics {
            self.mem[(p + 3) as usize].s1 = metric.height;
            self.mem[(p + 2) as usize].s1 = metric.depth;
        } else {
            let (hb, db) = self
                .font_record(font)
                .map_or((0, 0), |record| (record.height_base, record.depth_base));
            self.mem[(p + 3) as usize].s1 = hb;
            self.mem[(p + 2) as usize].s1 = db;
        }
    }

    /// Glyph id `i` of a native-word node (`get_native_glyph`): resolves
    /// the `i`th character through the font's character map.
    pub fn get_native_glyph(&self, p: i32, i: i32) -> i32 {
        let font = i32::from(self.mem[(p + 4) as usize].b16_s2());
        match self.get_native_usv(p, i) {
            Ok(c) => self.map_char_to_glyph(font, c.max(0) as u32),
            Err(_) => 0,
        }
    }

    /// Packed `char_info` word of `c` in `f` as used by the math
    /// machinery: `skip` doubles as the nonzero-width (`char_exists`)
    /// flag, `op` carries the tag nibble, `remainder` the remainder byte.
    pub fn math_char_info(&mut self, f: i32, c: i32) -> LigKernInsn {
        let e = self.effective_char(f, c).max(0) as u32;
        let exists = self.char_exists(f, e);
        let (tag, remainder) = self
            .font_record(f)
            .map(|record| {
                (
                    record.char_tags.get(&e).copied().unwrap_or(0),
                    record.char_remainders.get(&e).copied().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));
        LigKernInsn {
            skip: u8::from(exists),
            next_char: 0,
            op: tag,
            remainder,
        }
    }

    /// Updates `cur_size`/`cur_mu` from `cur_style`
    /// (C's `set_cur_size_and_mu`).
    pub fn set_cur_size_and_mu(&mut self) {
        if self.cur_style < SCRIPT_STYLE {
            self.cur_size = TEXT_SIZE;
        } else {
            self.cur_size = SCRIPT_SIZE * ((self.cur_style - 2) / 2);
        }
        let quad = self.mathsy_param(self.math_font(2 + self.cur_size), 6);
        self.cur_mu = self.x_over_n(quad, 18);
    }

    /// Number of `\fontdimen` parameters registered for font `f`
    /// (C's `font_params[f]`).
    pub fn font_params_count(&self, f: i32) -> usize {
        self.font_record(f)
            .map_or(0, |record| record.params.len().saturating_sub(1))
    }

    /// Math-family font number `n` (`MATH_FONT(n)`); the compact eqtb maps
    /// `MATH_FONT_BASE` onto `FAMILY_BASE`.
    pub fn math_font(&self, n: i32) -> i32 {
        self.eqtb
            .get(FAMILY_BASE + n.max(0) as usize)
            .map_or(0, |word| word.s1)
    }

    /// Math-family slot of packed math code `c` (`math_fam`).
    #[inline]
    pub fn math_fam(c: i32) -> i32 {
        ((c as u32) >> 24 & 0xFF) as i32
    }

    /// Math-class slot of packed math code `c` (`math_class`).
    #[inline]
    pub fn math_class(c: i32) -> i32 {
        ((c as u32) >> 21 & 0x07) as i32
    }

    /// Character slot of packed math code `c` (`math_char`).
    #[inline]
    pub fn math_char(c: i32) -> i32 {
        (c as u32 & 0x1F_FFFF) as i32
    }

    /// Packs a family slot into a math code (`set_family`).
    #[inline]
    pub fn set_family(x: i32) -> i32 {
        ((x as u32 & 0xFF) << 24) as i32
    }

    /// Packs a class slot into a math code (`set_class`).
    #[inline]
    pub fn set_class(x: i32) -> i32 {
        ((x as u32 & 0x07) << 21) as i32
    }

    /// Creates an edge node of subtype `s` and width `w` (`new_edge`).
    pub fn new_edge(&mut self, s: i32, w: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(EDGE_NODE_SIZE)?;
        self.set_node_type(p, EDGE_NODE)?;
        self.set_node_subtype(p, s as u16)?;
        self.mem[(p + 1) as usize].s1 = w;
        self.mem[(p + 2) as usize].s1 = 0;
        Ok(p)
    }

    /// Creates a style node for style `s` (`new_style`).
    pub fn new_style(&mut self, s: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(STYLE_NODE_SIZE)?;
        self.set_node_type(p, STYLE_NODE)?;
        self.mem[p as usize].set_b16_s0(s as u16);
        self.mem[(p + 1) as usize].s1 = 0;
        self.mem[(p + 2) as usize].s1 = 0;
        Ok(p)
    }

    /// Creates a `\mathchoice` node (`new_choice`).
    pub fn new_choice(&mut self) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(STYLE_NODE_SIZE)?;
        self.set_node_type(p, CHOICE_NODE)?;
        self.mem[p as usize].set_b16_s0(0);
        self.mem[(p + 1) as usize].s0 = TEX_NULL;
        self.mem[(p + 1) as usize].s1 = TEX_NULL;
        self.mem[(p + 2) as usize].s0 = TEX_NULL;
        self.mem[(p + 2) as usize].s1 = TEX_NULL;
        Ok(p)
    }

    /// Current font number (`eqtb[CUR_FONT_LOC]`).
    pub fn cur_font(&self) -> i32 {
        self.eqtb[CUR_FONT_LOC].s1
    }

    /// Returns whether character `c` exists in font `f` (`char_exists`):
    /// presence in the registered metric table stands in for C's nonzero
    /// `char_info` width index.
    pub fn char_exists(&self, f: i32, c: u32) -> bool {
        self.font_char_metrics.contains_key(&(f, c))
    }

    /// `effective_char`: applies the font's TFM character mapping and
    /// clears the pending native-ligature flag, mirroring the C side
    /// effect (`xtx_ligature_present = false`).
    pub fn effective_char(&mut self, f: i32, c: i32) -> i32 {
        let mapped = if self.xtx_ligature_present {
            c
        } else {
            match self.font_record(f).and_then(|r| r.mapping_name.clone()) {
                // A `:mapping=` byte converter (`apply_tfm_font_mapping`).
                Some(name) if self.font_mappings.contains_key(&name) => {
                    i32::from(self.apply_tfm_font_mapping(&name, c.max(0) as u16))
                }
                _ => self
                    .font_record(f)
                    .and_then(|record| record.tfm_font_mapping.get(&(c.max(0) as u32)).copied())
                    .map_or(c, |m| m as i32),
            }
        };
        self.xtx_ligature_present = false;
        mapped
    }

    /// Metrics of `effective_char(f, c)` for TFM fonts
    /// (`char_info(f, effective_char(f, c))`).
    pub fn effective_char_metric(&mut self, f: i32, c: i32) -> FontCharMetric {
        let e = self.effective_char(f, c).max(0) as u32;
        self.font_char_metric(f, e)
    }

    /// `char_tag` nibble of `effective_char(f, c)` (`CHAR_TAG_NONE`,
    /// `LIG_TAG`, `LIST_TAG`, or `EXT_TAG`).
    pub fn char_tag(&mut self, f: i32, c: i32) -> u8 {
        let e = self.effective_char(f, c).max(0) as u32;
        self.font_record(f)
            .and_then(|record| record.char_tags.get(&e).copied())
            .unwrap_or(0)
    }

    /// `char_info` remainder of `effective_char(f, c)`: the lig/kern
    /// program start for `LIG_TAG` characters.
    pub fn char_lig_kern_start(&mut self, f: i32, c: i32) -> i32 {
        let e = self.effective_char(f, c).max(0) as u32;
        self.font_record(f)
            .and_then(|record| record.char_remainders.get(&e).copied())
            .unwrap_or(0)
    }

    /// Lig/kern program instruction at `lig_kern_base + i`
    /// (`font_info[k]`).
    pub fn lig_kern_insn(&self, f: i32, i: i32) -> LigKernInsn {
        self.font_record(f)
            .and_then(|record| {
                usize::try_from(i)
                    .ok()
                    .and_then(|i| record.lig_kern.get(i))
                    .copied()
            })
            .unwrap_or_default()
    }

    /// Resolved kern value `font_info[kern_base + 256 * op + rem]` where
    /// `kern_base` is offset so the kern array starts at `op == 128`;
    /// index `256 * (op - 128) + rem` into the font's kern table.
    pub fn kern_value(&self, f: i32, op: u8, rem: i32) -> i32 {
        self.font_record(f)
            .and_then(|record| {
                usize::try_from(256 * (i32::from(op) - 128) + rem)
                    .ok()
                    .and_then(|i| record.kerns.get(i))
            })
            .copied()
            .unwrap_or(0)
    }

    /// Creates a character node for `c` in font `f` (`new_character`):
    /// native fonts produce a single-character `NATIVE_WORD_NODE`, TFM
    /// fonts a one-word character cell; `TEX_NULL` when the character is
    /// missing.
    pub fn new_character(&mut self, f: i32, c: i32) -> Result<i32, TeXMemoryError> {
        if self.is_native_font(f) {
            return self.new_native_character(f, c);
        }
        if c < 0 || !self.char_exists(f, c as u32) {
            return Ok(TEX_NULL);
        }
        let p = self.get_avail()?;
        self.mem[p as usize].set_b16_s1(f as u16);
        self.mem[p as usize].set_b16_s0(c as u16);
        Ok(p)
    }

    /// Creates a one- or two-code-unit `NATIVE_WORD_NODE` for `c`
    /// (`new_native_character`), applying the font's text mapping first
    /// and warning through `char_warning` when the result maps to no
    /// glyph.
    pub fn new_native_character(&mut self, f: i32, c: i32) -> Result<i32, TeXMemoryError> {
        let mut units: Vec<u16> = Vec::new();
        if c > 65535 {
            units.push(((c - 65536) / 1024 + 0xD800) as u16);
            units.push(((c - 65536) % 1024 + 0xDC00) as u16);
        } else {
            units.push(c.max(0) as u16);
        }
        let units = self.apply_text_mapping(f, &units);
        if self.int_par(INT_PAR_TRACING_LOST_CHARS)? > 0 {
            let mut i = 0;
            while i < units.len() {
                let scalar = if units[i] >= 0xD800 && units[i] < 0xDC00 {
                    let high = units[i];
                    i += 2;
                    65536 + (high as i32 - 0xD800) * 1024 + i32::from(units[i - 1]) - 0xDC00
                } else {
                    i += 1;
                    i32::from(units[i - 1])
                };
                if self.map_char_to_glyph(f, scalar as u32) == 0 {
                    self.char_warning(f, scalar);
                }
            }
        }
        let p = self.new_native_word_node(f, units.len() as i32)?;
        for (i, unit) in units.iter().enumerate() {
            self.set_native_text_at(p, i as i32, *unit)?;
        }
        self.set_native_metrics(p)?;
        Ok(p)
    }

    /// Applies the font's registered text mapping (`apply_mapping`):
    /// the named `font_mappings` converter attached by `mapping=` when
    /// present, else greedy longest-match substitution over the record's
    /// `text_mapping` pairs; identity when neither exists.
    pub fn apply_text_mapping(&self, f: i32, input: &[u16]) -> Vec<u16> {
        let Some(record) = self.font_record(f) else {
            return input.to_vec();
        };
        if let Some(name) = &record.mapping_name {
            if let Some(mapping) = self.font_mappings.get(name) {
                return mapping.converter.convert_text(input);
            }
        }
        if record.text_mapping.is_empty() {
            return input.to_vec();
        }
        let mut output = Vec::with_capacity(input.len());
        let mut i = 0;
        while i < input.len() {
            let mut best: Option<&(Vec<u16>, Vec<u16>)> = None;
            for pair in &record.text_mapping {
                let (from, _) = pair;
                if input[i..].starts_with(from) && best.is_none_or(|b| from.len() > b.0.len()) {
                    best = Some(pair);
                }
            }
            if let Some((from, to)) = best {
                output.extend_from_slice(to);
                i += from.len();
            } else {
                output.push(input[i]);
                i += 1;
            }
        }
        output
    }

    /// Records a `char_warning` for an unmapped character
    /// (`Missing character: There is no <c> in font <name>`).
    pub fn char_warning(&mut self, f: i32, c: i32) {
        let name = self
            .font_record(f)
            .map(|record| {
                record
                    .name
                    .iter()
                    .filter_map(|u| u32::try_from(*u).ok().and_then(char::from_u32))
                    .collect::<String>()
            })
            .unwrap_or_default();
        let shown = char::from_u32(c.max(0) as u32).unwrap_or('\u{FFFD}');
        self.transcript.push(format!(
            "Missing character: There is no {} in font {}!",
            shown, name
        ));
    }

    /// Creates a ligature node (`new_ligature`): `lig_char`/`lig_font`
    /// packed in `mem[p + 1].s0`, `lig_ptr` in `mem[p + 1].s1`.
    pub fn new_ligature(&mut self, f: i32, c: i32, q: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(2)?;
        self.set_node_type(p, LIGATURE_NODE)?;
        self.set_node_subtype(p, 0)?;
        self.mem[(p + 1) as usize].set_b16_s1(f.max(0) as u16);
        self.mem[(p + 1) as usize].set_b16_s0(c.max(0) as u16);
        self.mem[(p + 1) as usize].s1 = q;
        Ok(p)
    }

    /// Creates a two-word ligature-stack item (`new_lig_item`): `b16.s0`
    /// carries the character and `mem[p + 1].s1` the deferred char list.
    pub fn new_lig_item(&mut self, c: i32) -> Result<i32, TeXMemoryError> {
        let p = self.get_node(2)?;
        self.mem[p as usize].set_b16_s0(c.max(0) as u16);
        self.mem[(p + 1) as usize].s1 = TEX_NULL;
        Ok(p)
    }

    /// `lig_char(p)` of a ligature node or lig-stack item.
    pub fn lig_char(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(i32::from(self.word_at(p, 1)?.b16_s0()))
    }

    /// `lig_font(p)` of a ligature node.
    pub fn lig_font(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(i32::from(self.word_at(p, 1)?.b16_s1()))
    }

    /// `lig_ptr(p)` of a ligature node or lig-stack item.
    pub fn lig_ptr(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(self.word_at(p, 1)?.s1)
    }

    /// Font number stored in a `NATIVE_WORD_NODE`/`GLYPH_NODE`
    /// (`NATIVE_NODE_font`).
    pub fn native_node_font(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(i32::from(self.word_at(p, 4)?.b16_s2()))
    }

    /// UTF-16 length stored in a `NATIVE_WORD_NODE`
    /// (`NATIVE_NODE_length`).
    pub fn native_node_length(&self, p: i32) -> Result<i32, TeXMemoryError> {
        Ok(i32::from(self.word_at(p, 4)?.b16_s1()))
    }

    /// UTF-16 code unit at `index` in a native-word node
    /// (`NATIVE_NODE_text[p][index]`).
    pub fn native_word_text_at(&self, p: i32, index: i32) -> Result<u16, TeXMemoryError> {
        self.native_text_at(p, index)
    }

    /// Writes one UTF-16 code unit into a native-word node.
    pub fn set_native_word_text_at(
        &mut self,
        p: i32,
        index: i32,
        value: u16,
    ) -> Result<(), TeXMemoryError> {
        self.set_native_text_at(p, index, value)
    }

    /// Returns whether `p` is a `NATIVE_WORD_NODE`/`NATIVE_WORD_NODE_AT`
    /// whatsit (`is_native_word` shape check).
    pub fn is_native_word_node(&self, p: i32) -> Result<bool, TeXMemoryError> {
        if p == TEX_NULL || self.is_char_node(p) || self.node_type(p)? != WHATSIT_NODE {
            return Ok(false);
        }
        let subtype = self.node_subtype(p)?;
        Ok(subtype == NATIVE_WORD_NODE || subtype == NATIVE_WORD_NODE_AT)
    }

    /// Italic correction of a native-word node
    /// (`get_native_italic_correction`): the registered italic of the last
    /// character in the word.
    pub fn get_native_italic_correction(&mut self, p: i32) -> Result<i32, TeXMemoryError> {
        let font = self.native_node_font(p)?;
        let length = self.native_node_length(p)?;
        let mut last = 0i32;
        let mut i = length - 1;
        while i >= 0 {
            let unit = self.native_text_at(p, i)?;
            if !(0xDC00..0xE000).contains(&unit) {
                last = if unit >= 0xD800 && i + 1 < length {
                    let low = self.native_text_at(p, i + 1)?;
                    65536 + (i32::from(unit) - 0xD800) * 1024 + i32::from(low) - 0xDC00
                } else {
                    i32::from(unit)
                };
                break;
            }
            i -= 1;
        }
        Ok(self.get_native_char_ic(font, last as u32))
    }

    /// Italic correction of a `GLYPH_NODE`
    /// (`get_native_glyph_italic_correction`).
    pub fn get_native_glyph_italic_correction(&mut self, p: i32) -> Result<i32, TeXMemoryError> {
        let font = self.native_node_font(p)?;
        let gid = u32::from(self.word_at(p, 4)?.b16_s1());
        let metric = self.native_glyph_metric(font, gid);
        let letter_space = self
            .font_record(font)
            .map_or(0, |record| record.letter_space);
        Ok(if metric.rsb < 0 {
            letter_space - metric.rsb
        } else {
            letter_space
        })
    }

    /// Returns a registered layout-engine query result
    /// (`aat_font_get`/`ot_font_get` and their argument-taking variants);
    /// missing entries yield 0.
    pub fn font_engine_get(&self, f: i32, code: i32, args: &[i32]) -> i32 {
        self.font_engine_values
            .get(&(f, code, args.to_vec()))
            .copied()
            .unwrap_or(0)
    }

    /// Registers a layout-engine query result for native font `f`.
    pub fn register_font_engine_value(&mut self, f: i32, code: i32, args: &[i32], value: i32) {
        self.font_engine_values
            .insert((f, code, args.to_vec()), value);
    }

    /// Returns a registered named layout-engine query result
    /// (`aat_font_get_named`/`gr_font_get_named` and the selector variant);
    /// `k` carries the extra feature argument, `-1` when absent. Missing
    /// entries yield 0.
    pub fn font_engine_named(&self, f: i32, code: i32, k: i32, name: &str) -> i32 {
        self.font_engine_named_values
            .get(&(f, code, k, name.to_string()))
            .copied()
            .unwrap_or(0)
    }

    /// Registers a named layout-engine query result for native font `f`.
    pub fn register_font_engine_named(
        &mut self,
        f: i32,
        code: i32,
        k: i32,
        name: &str,
        value: i32,
    ) {
        self.font_engine_named_values
            .insert((f, code, k, name.to_string()), value);
    }

    /// Returns a registered layout-engine name string
    /// (`aat_print_font_name`/`gr_print_font_name`), `None` when absent.
    pub fn font_engine_name(&self, f: i32, code: i32, arg1: i32, arg2: i32) -> Option<&str> {
        self.font_engine_named_strings
            .get(&(f, code, arg1, arg2))
            .map(String::as_str)
    }

    /// Registers a layout-engine name string for native font `f`.
    pub fn register_font_engine_name(
        &mut self,
        f: i32,
        code: i32,
        arg1: i32,
        arg2: i32,
        name: &str,
    ) {
        self.font_engine_named_strings
            .insert((f, code, arg1, arg2), name.to_string());
    }

    /// Returns the name of glyph `gid` in native font `f`
    /// (`getGlyphName`/`print_glyph_name` data).
    pub fn glyph_name(&self, f: i32, gid: u32) -> Option<&str> {
        self.font_record(f)
            .and_then(|record| record.glyph_id_names.get(&gid))
            .map(String::as_str)
    }

    /// Returns the `\XeTeXfonttype` class: `1` for AAT, the record's
    /// `font_type` for `OTGR` (2 OpenType / 3 Graphite), else `0`.
    pub fn native_font_type(&self, f: i32) -> i32 {
        match self.font_area.get(&f).copied() {
            Some(flag) if flag == AAT_FONT_FLAG => 1,
            Some(flag) if flag == OTGR_FONT_FLAG => self
                .font_record(f)
                .map_or(0, |record| record.font_type.clamp(0, 3)),
            _ => 0,
        }
    }

    /// Returns the elapsed time in 1/65536 seconds since `\resettimer`
    /// (`get_microinterval`), `-1` when the interval overflows.
    pub fn get_microinterval(&self) -> i32 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let s = now.as_secs() as i64;
        let m = now.subsec_micros() as i64;
        let seconds = s - self.epochseconds;
        if seconds > 0x7FFF {
            return -1;
        }
        let micros = if i64::from(self.microseconds) > m {
            seconds - 1
        } else {
            seconds
        } * 65_536;
        let fraction = if i64::from(self.microseconds) > m {
            m + 1_000_000 - i64::from(self.microseconds)
        } else {
            m - i64::from(self.microseconds)
        };
        (micros + ((fraction / 100) * 65_536) / 10_000) as i32
    }

    /// Registers a PDF page count for `name`, backing
    /// `\XeTeXpdfpagecount`.
    pub fn register_pdf_page_count(&mut self, name: &str, pages: i32) {
        self.pdf_page_counts.insert(name.to_string(), pages.max(0));
    }

    /// Returns the page count for the currently scanned file name
    /// (`count_pdf_file_pages`), 0 when the file is unknown — the same
    /// recovery C uses for an unopenable file. Registered overrides win;
    /// otherwise the file's registered bytes are probed like C's
    /// `tt_probe_pdf_page_count_data` over the VFS handle.
    pub fn count_pdf_file_pages(&self) -> i32 {
        let name = self.file_name_string();
        if let Some(&pages) = self.pdf_page_counts.get(&name) {
            return pages.max(0);
        }
        self.input_byte_sources
            .get(&name)
            .and_then(|bytes| tectonic_image_probe::probe_pdf_page_count_bytes(bytes).ok())
            .map(|pages| pages.min(i32::MAX as usize) as i32)
            .unwrap_or(0)
            .max(0)
    }

    /// Registers a loadable font template for `new_font`'s `read_font_info`
    /// seam. The key is the packed `area + name` text scanned by
    /// `scan_file_name`.
    pub fn register_font_source(&mut self, key: &str, record: FontRecord) {
        self.font_sources.insert(key.to_string(), record);
    }

    /// Returns the hidden control sequence used as the `IDENT_VAL` token for
    /// font `f` (`FONT_ID_BASE + f`), creating it on first use.
    pub fn font_ident_cs(&mut self, f: i32) -> i32 {
        let Ok(index) = usize::try_from(f) else {
            return 0;
        };
        while self.font_id_cs.len() <= index {
            self.font_id_cs.push(-1);
        }
        if self.font_id_cs[index] < 0 {
            // An untypable code unit keeps the sequence out of user namespace.
            let name = [0x11_0000 + f];
            let cs = self
                .cs_table
                .define_units(&name, CommandCode::SetFont as u8, f);
            self.font_id_cs[index] = cs;
        }
        self.font_id_cs[index]
    }

    /// Creates a native-word whatsit with room for `n` UTF-16 code units
    /// (`new_native_word_node`).
    pub fn new_native_word_node(&mut self, font: i32, n: i32) -> Result<i32, TeXMemoryError> {
        if n < 0 {
            return Err(TeXMemoryError::CorruptedNode(n));
        }
        let size = NATIVE_NODE_SIZE + (n + 3) / 4;
        let p = self.get_node(size)?;
        self.set_node_type(p, WHATSIT_NODE)?;
        let subtype = if self.int_par(INT_PAR_XETEX_GENERATE_ACTUAL_TEXT)? > 0 {
            NATIVE_WORD_NODE_AT
        } else {
            NATIVE_WORD_NODE
        };
        self.set_node_subtype(p, subtype)?;
        self.mem[(p + 4) as usize].set_b16_s3(size as u16);
        self.mem[(p + 4) as usize].set_b16_s2(font as u16);
        self.mem[(p + 4) as usize].set_b16_s1(n as u16);
        self.mem[(p + 4) as usize].set_b16_s0(0);
        self.mem[(p + 5) as usize] = MemoryWord::default();
        Ok(p)
    }

    fn native_text_at(&self, p: i32, index: i32) -> Result<u16, TeXMemoryError> {
        if index < 0 {
            return Err(TeXMemoryError::CorruptedNode(index));
        }
        let word = self.word_at(p + NATIVE_NODE_SIZE + index / 4, 0)?;
        Ok(match index % 4 {
            0 => word.b16_s0(),
            1 => word.b16_s1(),
            2 => word.b16_s2(),
            _ => word.b16_s3(),
        })
    }

    fn set_native_text_at(&mut self, p: i32, index: i32, value: u16) -> Result<(), TeXMemoryError> {
        if index < 0 {
            return Err(TeXMemoryError::CorruptedNode(index));
        }
        let idx = self.check_bounds(p + NATIVE_NODE_SIZE + index / 4)?;
        match index % 4 {
            0 => self.mem[idx].set_b16_s0(value),
            1 => self.mem[idx].set_b16_s1(value),
            2 => self.mem[idx].set_b16_s2(value),
            _ => self.mem[idx].set_b16_s3(value),
        }
        Ok(())
    }

    /// Computes the dimensions of a native-word node from the state-local
    /// character metric table (`set_native_metrics`).
    pub fn set_native_metrics(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        let info = self.word_at(p, 4)?;
        let font = i32::from(info.b16_s2());
        let length = i32::from(info.b16_s1());
        let mut text = Vec::with_capacity(length as usize);
        for index in 0..length {
            text.push(self.native_text_at(p, index)?);
        }

        let mut width = 0i32;
        let mut height = 0i32;
        let mut depth = 0i32;
        let text = String::from_utf16(&text).map_err(|_| TeXMemoryError::CorruptedNode(p))?;
        for c in text.chars() {
            let glyph = self.map_char_to_glyph(font, c as u32);
            let (w, h, d) = if glyph > 0 {
                let m = self.native_glyph_metric(font, glyph as u32);
                (m.width, m.height, m.depth)
            } else {
                let m = self.font_char_metric(font, c as u32);
                (m.width, m.height, m.depth)
            };
            width = width.saturating_add(w);
            height = height.max(h);
            depth = depth.max(d);
        }
        self.mem[(p + 1) as usize].s1 = width;
        self.mem[(p + 3) as usize].s1 = height;
        self.mem[(p + 2) as usize].s1 = depth;
        Ok(())
    }

    /// UTF-16 text of a native-word node (`NATIVE_NODE_text` contents).
    pub fn native_word_text(&self, p: i32) -> Result<Vec<u16>, TeXMemoryError> {
        let length = i32::from(self.word_at(p, 4)?.b16_s1());
        let mut text = Vec::with_capacity(length.max(0) as usize);
        for index in 0..length {
            text.push(self.native_text_at(p, index)?);
        }
        Ok(text)
    }

    /// Attached positioned-glyph data; `None` mirrors C's
    /// `native_glyph_info_ptr(p) == NULL`.
    pub fn native_glyph_info(&self, p: i32) -> Option<&NativeGlyphInfo> {
        self.native_glyph_info.get(&p)
    }

    /// Whether the node carries a positioned-glyph buffer
    /// (`native_glyph_info_ptr(p) != NULL`).
    pub fn has_native_glyph_info(&self, p: i32) -> bool {
        self.native_glyph_info.contains_key(&p)
    }

    /// Number of positioned glyphs attached to the node
    /// (`native_glyph_count`).
    pub fn native_glyph_count(&self, p: i32) -> usize {
        self.native_glyph_info
            .get(&p)
            .map_or(0, |info| info.glyphs.len())
    }

    /// Attaches positioned-glyph data to a native node
    /// (`native_glyph_info_ptr(p) = info`).
    pub fn set_native_glyph_info(&mut self, p: i32, info: NativeGlyphInfo) {
        self.native_glyph_info.insert(p, info);
    }

    /// Removes a node's positioned-glyph buffer (used when nodes are
    /// freed, mirroring C's `free(native_glyph_info_ptr)` cleanup).
    pub fn drop_native_glyph_info(&mut self, p: i32) {
        self.native_glyph_info.remove(&p);
    }

    /// Copies one node's positioned-glyph buffer to another
    /// (`copy_native_glyph_info`).
    pub fn copy_native_glyph_info(&mut self, src: i32, dest: i32) {
        if let Some(info) = self.native_glyph_info.get(&src).cloned() {
            self.native_glyph_info.insert(dest, info);
        }
    }

    /// Fills a native-word node's positioned-glyph buffer and dimensions
    /// (`measure_native_node`). Until the OT layout engine is ported,
    /// shaping is one glyph per character via `map_char_to_glyph` with
    /// cumulative advances — equivalent to `layoutChars` output for fonts
    /// with no active GSUB/GPOS features. The `font_letter_space`
    /// adjustment loop mirrors C exactly.
    pub fn measure_native_node(
        &mut self,
        p: i32,
        use_glyph_metrics: bool,
    ) -> Result<(), TeXMemoryError> {
        let info = self.word_at(p, 4)?;
        let font = i32::from(info.b16_s2());
        let length = i32::from(info.b16_s1());
        let letter_space = self
            .font_record(font)
            .map_or(0, |record| record.letter_space);

        let mut glyphs = Vec::new();
        let mut advances = Vec::new();
        let mut x = 0i64;
        let mut index = 0i32;
        while index < length {
            let c = self.get_native_usv(p, index)?.max(0) as u32;
            let glyph = self.map_char_to_glyph(font, c).max(0) as u16;
            let advance = self.native_glyph_metric(font, u32::from(glyph)).width;
            glyphs.push(NativeGlyphLoc {
                x: x as i32,
                y: 0,
                glyph,
            });
            advances.push(advance);
            x += i64::from(advance);
            index += if c >= 0x10000 { 2 } else { 1 };
        }

        let mut width = x as i32;
        if letter_space != 0 && !glyphs.is_empty() {
            let mut ls_delta = 0i32;
            for (i, loc) in glyphs.iter_mut().enumerate() {
                if advances[i] == 0 && ls_delta != 0 {
                    ls_delta -= letter_space;
                }
                loc.x = loc.x.saturating_add(ls_delta);
                ls_delta = ls_delta.saturating_add(letter_space);
            }
            if ls_delta != 0 {
                width = width.saturating_add(ls_delta - letter_space);
            }
        }

        self.set_native_glyph_info(p, NativeGlyphInfo { glyphs });
        self.mem[(p + 1) as usize].s1 = width;

        let (height_base, depth_base) = self
            .font_record(font)
            .map_or((0, 0), |record| (record.height_base, record.depth_base));
        if !use_glyph_metrics || self.native_glyph_count(p) == 0 {
            self.mem[(p + 3) as usize].s1 = height_base;
            self.mem[(p + 2) as usize].s1 = depth_base;
        } else {
            let mut y_max = f64::NEG_INFINITY;
            let mut y_min = f64::INFINITY;
            if let Some(info) = self.native_glyph_info.get(&p) {
                for loc in &info.glyphs {
                    let metric = self.native_glyph_metric(font, u32::from(loc.glyph));
                    let y = -f64::from(loc.y) / 65_536.0;
                    let ht = f64::from(metric.height) / 65_536.0;
                    let dp = f64::from(metric.depth) / 65_536.0;
                    y_max = y_max.max(y + ht);
                    y_min = y_min.min(y - dp);
                }
            }
            let (height, depth) = if y_max == f64::NEG_INFINITY {
                (height_base, depth_base)
            } else {
                (tex_round(y_max * 65_536.0), tex_round(-y_min * 65_536.0))
            };
            self.mem[(p + 3) as usize].s1 = height;
            self.mem[(p + 2) as usize].s1 = depth;
        }
        Ok(())
    }

    /// Re-justifies a merged/space-adjusted native-word node
    /// (`store_justified_native_glyphs`): re-measures, then redistributes
    /// the width difference across space glyphs — or evenly across all
    /// glyphs when there are none — restoring the requested width.
    pub fn set_justified_native_glyphs(&mut self, p: i32) -> Result<(), TeXMemoryError> {
        let saved_width = self.mem[(p + 1) as usize].s1;
        self.measure_native_node(p, false)?;

        if self.mem[(p + 1) as usize].s1 != saved_width {
            let font = i32::from(self.word_at(p, 4)?.b16_s2());
            let just_amount = f64::from(saved_width - self.mem[(p + 1) as usize].s1) / 65_536.0;
            let space_glyph = self.map_char_to_glyph(font, u32::from(b' ')).max(0) as u16;
            if let Some(info) = self.native_glyph_info.get_mut(&p) {
                let space_count = info
                    .glyphs
                    .iter()
                    .filter(|loc| loc.glyph == space_glyph)
                    .count();
                if space_count > 0 {
                    let mut adjustment = 0.0f64;
                    let mut space_index = 0usize;
                    for loc in &mut info.glyphs {
                        loc.x = tex_round((f64::from(loc.x) / 65_536.0 + adjustment) * 65_536.0);
                        if loc.glyph == space_glyph {
                            space_index += 1;
                            adjustment = just_amount * space_index as f64 / space_count as f64;
                        }
                    }
                } else if info.glyphs.len() > 1 {
                    let count = info.glyphs.len() as f64;
                    for (i, loc) in info.glyphs.iter_mut().enumerate().skip(1) {
                        loc.x = tex_round(
                            (f64::from(loc.x) / 65_536.0 + just_amount * i as f64 / (count - 1.0))
                                * 65_536.0,
                        );
                    }
                }
            }
            self.mem[(p + 1) as usize].s1 = saved_width;
        }
        Ok(())
    }

    /// Validates and locks `\mag` (`prepare_mag`): reverts an incompatible
    /// change, clamps an illegal value to 1000, then records `mag_set`.
    pub fn prepare_mag(&mut self) -> Result<(), TeXMemoryError> {
        let mag = self.int_par(INT_PAR_MAG)?;
        if self.mag_set > 0 && mag != self.mag_set {
            self.transcript.push(format!(
                "! Incompatible magnification ({mag}); the previous value will be retained ({})",
                self.mag_set
            ));
            self.error_count += 1;
            let mag_set = self.mag_set;
            self.set_int_par(INT_PAR_MAG, mag_set)?;
        }
        let mag = self.int_par(INT_PAR_MAG)?;
        if !(1..=32768).contains(&mag) {
            self.transcript.push(format!(
                "! Illegal magnification has been changed to 1000 ({mag})"
            ));
            self.error_count += 1;
            self.set_int_par(INT_PAR_MAG, 1000)?;
        }
        self.mag_set = self.int_par(INT_PAR_MAG)?;
        Ok(())
    }

    /// Opens the XDV output stream for this job: the per-engine equivalent
    /// of `pack_job_name(output_file_extension); ttstub_output_open`.
    /// Output accumulates in [`Self::dvi_bytes`] until a driver flushes
    /// it to a real sink.
    pub fn ensure_output_open(&mut self) {
        if self.output_file_opened {
            return;
        }
        let extension = if self.semantic_pagination_enabled {
            ".spx"
        } else {
            ".xdv"
        };
        let base = self
            .job_name
            .clone()
            .unwrap_or_else(|| "texput".to_string());
        let base = base.rsplit('.').next().map_or(base.as_str(), |_| {
            base.rsplit_once('.')
                .map_or(base.as_str(), |(stem, _)| stem)
        });
        self.output_file_name = format!("{base}{extension}");
        self.output_file_opened = true;
    }

    /// SyncTeX recorders: faithful no-ops while `synctex_enabled` is
    /// false, matching the early returns in `xetex-synctex.c`.
    pub fn synctex_sheet(&self, _mag: i32) {}
    /// SyncTeX `synctex_teehs` (end of sheet).
    pub fn synctex_teehs(&self) {}
    /// SyncTeX `synctex_hlist` (entering an hlist).
    pub fn synctex_hlist(&self, _p: i32) {}
    /// SyncTeX `synctex_tsilh` (leaving an hlist).
    pub fn synctex_tsilh(&self, _p: i32) {}
    /// SyncTeX `synctex_vlist` (entering a vlist).
    pub fn synctex_vlist(&self, _p: i32) {}
    /// SyncTeX `synctex_tsilv` (leaving a vlist).
    pub fn synctex_tsilv(&self, _p: i32) {}
    /// SyncTeX `synctex_void_vlist` (empty vlist).
    pub fn synctex_void_vlist(&self, _p: i32, _this_box: i32) {}
    /// SyncTeX `synctex_void_hlist` (empty hlist).
    pub fn synctex_void_hlist(&self, _p: i32, _this_box: i32) {}
    /// SyncTeX `synctex_math` (math node).
    pub fn synctex_math(&self, _p: i32, _this_box: i32) {}
    /// SyncTeX `synctex_kern` (kern node).
    pub fn synctex_kern(&self, _p: i32, _this_box: i32) {}
    /// SyncTeX `synctex_horizontal_rule_or_glue`.
    pub fn synctex_horizontal_rule_or_glue(&self, _p: i32, _this_box: i32) {}
    /// SyncTeX `synctex_current` (end of a character run).
    pub fn synctex_current(&self) {}

    /// Returns the Unicode scalar at UTF-16 index `i` of a native-word
    /// node, combining surrogate pairs (`get_native_usv`).
    pub fn get_native_usv(&self, p: i32, i: i32) -> Result<i32, TeXMemoryError> {
        let c = self.native_word_text_at(p, i)?;
        if (0xD800..0xDC00).contains(&c) {
            let low = self.native_word_text_at(p, i + 1)?;
            return Ok(0x10000 + (i32::from(c) - 0xD800) * 0x400 + i32::from(low) - 0xDC00);
        }
        Ok(i32::from(c))
    }

    /// Returns `\XeTeXhyphenatablelength` clamped to
    /// `HYPHENATABLE_LENGTH_LIMIT` (`max_hyphenatable_length`).
    pub fn max_hyphenatable_length(&self) -> Result<i32, TeXMemoryError> {
        let value = self.int_par(INT_PAR_XETEX_HYPHENATABLE_LENGTH)?;
        Ok(value.min(HYPHENATABLE_LENGTH_LIMIT))
    }

    /// Returns whether `p` is a `NATIVE_WORD_NODE`/`NATIVE_WORD_NODE_AT`
    /// whatsit in font `font`.
    pub fn is_native_word_with_font(&self, p: i32, font: i32) -> Result<bool, TeXMemoryError> {
        if p == TEX_NULL || self.is_char_node(p) || self.node_type(p)? != WHATSIT_NODE {
            return Ok(false);
        }
        let subtype = self.node_subtype(p)?;
        Ok(
            (subtype == NATIVE_WORD_NODE || subtype == NATIVE_WORD_NODE_AT)
                && i32::from(self.word_at(p, 4)?.b16_s2()) == font,
        )
    }

    /// Packages a horizontal list (`hpack`).
    ///
    /// This follows the C list walk and glue-set calculation. Character metrics
    /// come from the state-local font metric table; diagnostics are represented
    /// by `last_badness` and the packaged node fields until the output channel
    /// lands in the diagnostics packet.
    pub fn hpack(&mut self, mut p: i32, mut w: i32, m: i32) -> Result<i32, TeXMemoryError> {
        self.last_badness = 0;
        let r = self.get_node(BOX_NODE_SIZE)?;
        self.set_node_type(r, HLIST_NODE)?;
        self.set_node_subtype(r, 0)?;
        self.mem[(r + 4) as usize].s1 = 0;
        let mut q = r + 5;
        self.mem[q as usize].s1 = p;
        let mut h = 0i32;
        let mut d = 0i32;
        let mut x = 0i32;
        self.total_stretch = [0; 4];
        self.total_shrink = [0; 4];

        let using_lr = self.int_par(INT_PAR_TEXXET)? > 0;
        if using_lr {
            let temp = self.get_avail()?;
            self.mem[temp as usize].s0 = BEFORE;
            self.mem[temp as usize].s1 = self.lr_ptr;
            self.lr_ptr = temp;
        }

        'outer: while p != TEX_NULL {
            while self.is_char_node(p) {
                let word = self.word_at(p, 0)?;
                let metric =
                    self.font_char_metric(i32::from(word.b16_s1()), u32::from(word.b16_s0()));
                x = x.saturating_add(metric.width);
                h = h.max(metric.height);
                d = d.max(metric.depth);
                p = self.link(p)?;
            }
            if p == TEX_NULL {
                break;
            }

            match self.node_type(p)? {
                HLIST_NODE | VLIST_NODE | RULE_NODE | UNSET_NODE => {
                    x = x.saturating_add(self.word_at(p, 1)?.s1);
                    let shift = if self.node_type(p)? >= RULE_NODE {
                        0
                    } else {
                        self.word_at(p, 4)?.s1
                    };
                    h = h.max(self.word_at(p, 3)?.s1 - shift);
                    d = d.max(self.word_at(p, 2)?.s1 + shift);
                }
                INS_NODE | MARK_NODE | ADJUST_NODE => {
                    if self.adjust_tail != TEX_NULL || self.pre_adjust_tail != TEX_NULL {
                        while self.link(q)? != p {
                            q = self.link(q)?;
                        }
                        if self.node_type(p)? == ADJUST_NODE {
                            let adjustment = self.word_at(p, 1)?.s1;
                            let tail = if self.node_subtype(p)? != 0 {
                                if self.pre_adjust_tail == TEX_NULL {
                                    return Err(TeXMemoryError::CorruptedNode(p));
                                }
                                self.pre_adjust_tail
                            } else {
                                if self.adjust_tail == TEX_NULL {
                                    return Err(TeXMemoryError::CorruptedNode(p));
                                }
                                self.adjust_tail
                            };
                            self.mem[tail as usize].s1 = adjustment;
                            let mut tail = tail;
                            while self.link(tail)? != TEX_NULL {
                                tail = self.link(tail)?;
                            }
                            if self.node_subtype(p)? != 0 {
                                self.pre_adjust_tail = tail;
                            } else {
                                self.adjust_tail = tail;
                            }
                            let next = self.link(p)?;
                            self.free_node(p, SMALL_NODE_SIZE)?;
                            self.mem[q as usize].s1 = next;
                            p = q;
                        } else {
                            if self.adjust_tail == TEX_NULL {
                                return Err(TeXMemoryError::CorruptedNode(p));
                            }
                            self.mem[self.adjust_tail as usize].s1 = p;
                            self.adjust_tail = p;
                            let next = self.link(p)?;
                            self.mem[q as usize].s1 = next;
                            p = q;
                        }
                    }
                }
                WHATSIT_NODE => match self.node_subtype(p)? {
                    NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT => {
                        let font = i32::from(self.word_at(p, 4)?.b16_s2());
                        let mut k = if q != r + 5
                            && !self.is_char_node(q)
                            && self.node_type(q)? == DISC_NODE
                        {
                            i32::from(self.node_subtype(q)?)
                        } else {
                            0
                        };
                        while self.link(q)? != p {
                            k -= 1;
                            q = self.link(q)?;
                            if !self.is_char_node(q) && self.node_type(q)? == DISC_NODE {
                                k = i32::from(self.node_subtype(q)?);
                            }
                        }

                        let mut pp = self.link(p)?;
                        loop {
                            if !(k <= 0 && pp != TEX_NULL && !self.is_char_node(pp)) {
                                break;
                            }
                            if self.is_native_word_with_font(pp, font)? {
                                pp = self.link(pp)?;
                                continue;
                            }
                            if self.node_type(pp)? == DISC_NODE {
                                let ppp = self.word_at(pp, 1)?.s1;
                                if self.is_native_word_with_font(ppp, font)? {
                                    pp = self.link(ppp)?;
                                    continue;
                                }
                            }
                            break;
                        }

                        if pp != self.link(p)? {
                            let mut total_chars = 0i32;
                            let mut cursor = p;
                            let mut last = p;
                            while cursor != pp {
                                if self.node_type(cursor)? == WHATSIT_NODE {
                                    total_chars += i32::from(self.word_at(cursor, 4)?.b16_s1());
                                }
                                last = cursor;
                                cursor = self.link(cursor)?;
                            }

                            let merged = self.new_native_word_node(font, total_chars)?;
                            self.set_node_subtype(merged, self.node_subtype(p)?)?;
                            self.mem[q as usize].s1 = merged;
                            self.mem[merged as usize].s1 = self.mem[last as usize].s1;
                            self.mem[last as usize].s1 = TEX_NULL;

                            let mut cursor = p;
                            let mut text_index = 0i32;
                            while cursor != TEX_NULL {
                                if self.node_type(cursor)? == WHATSIT_NODE {
                                    let len = i32::from(self.word_at(cursor, 4)?.b16_s1());
                                    for index in 0..len {
                                        let ch = self.native_text_at(cursor, index)?;
                                        self.set_native_text_at(merged, text_index, ch)?;
                                        text_index += 1;
                                    }
                                }
                                cursor = self.link(cursor)?;
                            }
                            self.flush_node_list(p)?;
                            p = merged;
                            self.set_native_metrics(p)?;
                        }

                        h = h.max(self.word_at(p, 3)?.s1);
                        d = d.max(self.word_at(p, 2)?.s1);
                        x = x.saturating_add(self.word_at(p, 1)?.s1);
                    }
                    GLYPH_NODE | PIC_NODE | PDF_NODE => {
                        h = h.max(self.word_at(p, 3)?.s1);
                        d = d.max(self.word_at(p, 2)?.s1);
                        x = x.saturating_add(self.word_at(p, 1)?.s1);
                    }
                    _ => {}
                },
                GLUE_NODE => {
                    let spec = self.word_at(p, 1)?.s0;
                    x = x.saturating_add(self.word_at(spec, 1)?.s1);
                    let stretch_order = self.word_at(spec, 0)?.b16_s1() as usize;
                    let shrink_order = self.word_at(spec, 0)?.b16_s0() as usize;
                    if stretch_order >= self.total_stretch.len()
                        || shrink_order >= self.total_shrink.len()
                    {
                        return Err(TeXMemoryError::CorruptedNode(spec));
                    }
                    self.total_stretch[stretch_order] =
                        self.total_stretch[stretch_order].saturating_add(self.word_at(spec, 2)?.s1);
                    self.total_shrink[shrink_order] =
                        self.total_shrink[shrink_order].saturating_add(self.word_at(spec, 3)?.s1);
                    if i32::from(self.node_subtype(p)?) >= A_LEADERS {
                        let leader = self.word_at(p, 1)?.s1;
                        h = h.max(self.word_at(leader, 3)?.s1);
                        d = d.max(self.word_at(leader, 2)?.s1);
                    }
                }
                KERN_NODE | MARGIN_KERN_NODE => {
                    x = x.saturating_add(self.word_at(p, 1)?.s1);
                }
                MATH_NODE => {
                    x = x.saturating_add(self.word_at(p, 1)?.s1);
                    if using_lr {
                        let subtype = i32::from(self.node_subtype(p)?);
                        if subtype & 1 != 0 {
                            let expected = L_CODE * (subtype / L_CODE) + END_M_CODE;
                            if self.lr_ptr != TEX_NULL
                                && self.word_at(self.lr_ptr, 0)?.s0 == expected
                            {
                                let temp = self.lr_ptr;
                                self.lr_ptr = self.word_at(temp, 0)?.s1;
                                self.free_avail(temp)?;
                            } else {
                                self.lr_problems += 1;
                                self.set_node_type(p, KERN_NODE)?;
                                self.set_node_subtype(p, EXPLICIT)?;
                            }
                        } else {
                            let temp = self.get_avail()?;
                            self.mem[temp as usize].s0 = L_CODE * (subtype / L_CODE) + END_M_CODE;
                            self.mem[temp as usize].s1 = self.lr_ptr;
                            self.lr_ptr = temp;
                        }
                    }
                }
                LIGATURE_NODE => {
                    let lig_trick = self.lig_trick();
                    self.mem[lig_trick as usize] = self.word_at(p, 1)?;
                    self.mem[lig_trick as usize].s1 = self.link(p)?;
                    p = lig_trick;
                    self.xtx_ligature_present = true;
                    continue 'outer;
                }
                _ => {}
            }
            p = self.link(p)?;
        }

        if self.adjust_tail != TEX_NULL {
            self.mem[self.adjust_tail as usize].s1 = TEX_NULL;
        }
        if self.pre_adjust_tail != TEX_NULL {
            self.mem[self.pre_adjust_tail as usize].s1 = TEX_NULL;
        }
        self.mem[(r + 3) as usize].s1 = h;
        self.mem[(r + 2) as usize].s1 = d;
        if m == ADDITIONAL {
            w = w.saturating_add(x);
        }
        self.mem[(r + 1) as usize].s1 = w;
        let excess = w - x;
        if excess == 0 {
            self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
            self.mem[(r + 5) as usize].set_b16_s0(NORMAL);
            self.mem[(r + 6) as usize].set_gr(0.0);
        } else if excess > 0 {
            let order = if self.total_stretch[FILLL as usize] != 0 {
                FILLL
            } else if self.total_stretch[FILL as usize] != 0 {
                FILL
            } else if self.total_stretch[FIL as usize] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.mem[(r + 5) as usize].set_b16_s0(order);
            self.mem[(r + 5) as usize].set_b16_s1(STRETCHING);
            if self.total_stretch[order as usize] != 0 {
                let glue_set = f64::from(excess) / f64::from(self.total_stretch[order as usize]);
                self.mem[(r + 6) as usize].set_gr(glue_set);
            } else {
                self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
                self.mem[(r + 6) as usize].set_gr(0.0);
            }
            if order == NORMAL && self.word_at(r, 5)?.s1 != TEX_NULL {
                self.last_badness = badness(excess, self.total_stretch[NORMAL as usize]);
            }
        } else {
            let needed = -excess;
            let order = if self.total_shrink[FILLL as usize] != 0 {
                FILLL
            } else if self.total_shrink[FILL as usize] != 0 {
                FILL
            } else if self.total_shrink[FIL as usize] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.mem[(r + 5) as usize].set_b16_s0(order);
            self.mem[(r + 5) as usize].set_b16_s1(SHRINKING);
            if self.total_shrink[order as usize] != 0 {
                let glue_set = f64::from(needed) / f64::from(self.total_shrink[order as usize]);
                self.mem[(r + 6) as usize].set_gr(glue_set);
            } else {
                self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
                self.mem[(r + 6) as usize].set_gr(0.0);
            }
            if self.total_shrink[order as usize] < needed
                && order == NORMAL
                && self.word_at(r, 5)?.s1 != TEX_NULL
            {
                self.last_badness = 1_000_000;
                self.mem[(r + 6) as usize].set_gr(1.0);
                let overflow = needed - self.total_shrink[NORMAL as usize];
                if overflow > self.dimen_par(DIMEN_PAR_HFUZZ)?
                    && self.dimen_par(DIMEN_PAR_OVERFULL_RULE)? > 0
                {
                    while self.link(q)? != TEX_NULL {
                        q = self.link(q)?;
                    }
                    let rule = self.new_rule()?;
                    self.mem[q as usize].s1 = rule;
                    self.mem[(rule + 1) as usize].s1 = self.dimen_par(DIMEN_PAR_OVERFULL_RULE)?;
                }
            } else if order == NORMAL && self.word_at(r, 5)?.s1 != TEX_NULL {
                self.last_badness = badness(needed, self.total_shrink[NORMAL as usize]);
            }
        }

        if using_lr {
            if self.lr_ptr == TEX_NULL {
                return Err(TeXMemoryError::CorruptedNode(r));
            }
            if self.word_at(self.lr_ptr, 0)?.s0 != BEFORE {
                while self.link(q)? != TEX_NULL {
                    q = self.link(q)?;
                }
                loop {
                    let lr_type = self.word_at(self.lr_ptr, 0)?.s0;
                    let temp = q;
                    q = self.new_math(0, lr_type)?;
                    self.mem[temp as usize].s1 = q;
                    self.lr_problems += 10_000;
                    let old = self.lr_ptr;
                    self.lr_ptr = self.word_at(old, 0)?.s1;
                    self.free_avail(old)?;
                    if self.lr_ptr == TEX_NULL || self.word_at(self.lr_ptr, 0)?.s0 == BEFORE {
                        break;
                    }
                }
            }
            if self.lr_problems > 0 {
                self.lr_problems = 0;
            }
            let old = self.lr_ptr;
            self.lr_ptr = self.word_at(old, 0)?.s1;
            self.free_avail(old)?;
            if self.lr_ptr != TEX_NULL {
                return Err(TeXMemoryError::CorruptedNode(r));
            }
        }
        Ok(r)
    }

    /// Packages a vertical list (`vpackage`).
    pub fn vpackage(
        &mut self,
        mut p: i32,
        mut h: i32,
        m: i32,
        l: i32,
    ) -> Result<i32, TeXMemoryError> {
        self.last_badness = 0;
        let r = self.get_node(BOX_NODE_SIZE)?;
        self.set_node_type(r, VLIST_NODE)?;
        let subtype = if self.int_par(INT_PAR_XETEX_UPWARDS)? > 0 {
            1
        } else {
            0
        };
        self.set_node_subtype(r, subtype)?;
        self.mem[(r + 4) as usize].s1 = 0;
        self.mem[(r + 5) as usize].s1 = p;
        let mut w = 0i32;
        let mut d = 0i32;
        let mut x = 0i32;
        self.total_stretch = [0; 4];
        self.total_shrink = [0; 4];

        while p != TEX_NULL {
            if self.is_char_node(p) {
                return Err(TeXMemoryError::CorruptedNode(p));
            }
            match self.node_type(p)? {
                HLIST_NODE | VLIST_NODE | RULE_NODE | UNSET_NODE => {
                    x = x.saturating_add(d).saturating_add(self.word_at(p, 3)?.s1);
                    d = self.word_at(p, 2)?.s1;
                    let shift = if self.node_type(p)? >= RULE_NODE {
                        0
                    } else {
                        self.word_at(p, 4)?.s1
                    };
                    w = w.max(self.word_at(p, 1)?.s1 + shift);
                }
                WHATSIT_NODE => {
                    let subtype = self.node_subtype(p)?;
                    if subtype == PIC_NODE || subtype == PDF_NODE {
                        x = x.saturating_add(d).saturating_add(self.word_at(p, 3)?.s1);
                        d = self.word_at(p, 2)?.s1;
                        w = w.max(self.word_at(p, 1)?.s1);
                    }
                }
                GLUE_NODE => {
                    x = x.saturating_add(d);
                    d = 0;
                    let spec = self.word_at(p, 1)?.s0;
                    x = x.saturating_add(self.word_at(spec, 1)?.s1);
                    let stretch_order = self.word_at(spec, 0)?.b16_s1() as usize;
                    let shrink_order = self.word_at(spec, 0)?.b16_s0() as usize;
                    if stretch_order >= self.total_stretch.len()
                        || shrink_order >= self.total_shrink.len()
                    {
                        return Err(TeXMemoryError::CorruptedNode(spec));
                    }
                    self.total_stretch[stretch_order] =
                        self.total_stretch[stretch_order].saturating_add(self.word_at(spec, 2)?.s1);
                    self.total_shrink[shrink_order] =
                        self.total_shrink[shrink_order].saturating_add(self.word_at(spec, 3)?.s1);
                    if i32::from(self.node_subtype(p)?) >= A_LEADERS {
                        let leader = self.word_at(p, 1)?.s1;
                        w = w.max(self.word_at(leader, 1)?.s1);
                    }
                }
                KERN_NODE => {
                    x = x.saturating_add(d).saturating_add(self.word_at(p, 1)?.s1);
                    d = 0;
                }
                _ => {}
            }
            p = self.link(p)?;
        }

        self.mem[(r + 1) as usize].s1 = w;
        if d > l {
            x = x.saturating_add(d - l);
            self.mem[(r + 2) as usize].s1 = l;
        } else {
            self.mem[(r + 2) as usize].s1 = d;
        }
        if m == ADDITIONAL {
            h = h.saturating_add(x);
        }
        self.mem[(r + 3) as usize].s1 = h;
        let excess = h - x;
        if excess == 0 {
            self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
            self.mem[(r + 5) as usize].set_b16_s0(NORMAL);
            self.mem[(r + 6) as usize].set_gr(0.0);
        } else if excess > 0 {
            let order = if self.total_stretch[FILLL as usize] != 0 {
                FILLL
            } else if self.total_stretch[FILL as usize] != 0 {
                FILL
            } else if self.total_stretch[FIL as usize] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.mem[(r + 5) as usize].set_b16_s0(order);
            self.mem[(r + 5) as usize].set_b16_s1(STRETCHING);
            if self.total_stretch[order as usize] != 0 {
                let glue_set = f64::from(excess) / f64::from(self.total_stretch[order as usize]);
                self.mem[(r + 6) as usize].set_gr(glue_set);
            } else {
                self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
                self.mem[(r + 6) as usize].set_gr(0.0);
            }
            if order == NORMAL && self.word_at(r, 5)?.s1 != TEX_NULL {
                self.last_badness = badness(excess, self.total_stretch[NORMAL as usize]);
            }
        } else {
            let needed = -excess;
            let order = if self.total_shrink[FILLL as usize] != 0 {
                FILLL
            } else if self.total_shrink[FILL as usize] != 0 {
                FILL
            } else if self.total_shrink[FIL as usize] != 0 {
                FIL
            } else {
                NORMAL
            };
            self.mem[(r + 5) as usize].set_b16_s0(order);
            self.mem[(r + 5) as usize].set_b16_s1(SHRINKING);
            if self.total_shrink[order as usize] != 0 {
                let glue_set = f64::from(needed) / f64::from(self.total_shrink[order as usize]);
                self.mem[(r + 6) as usize].set_gr(glue_set);
            } else {
                self.mem[(r + 5) as usize].set_b16_s1(NORMAL);
                self.mem[(r + 6) as usize].set_gr(0.0);
            }
            if self.total_shrink[order as usize] < needed
                && order == NORMAL
                && self.word_at(r, 5)?.s1 != TEX_NULL
            {
                self.last_badness = 1_000_000;
                self.mem[(r + 6) as usize].set_gr(1.0);
            } else if order == NORMAL && self.word_at(r, 5)?.s1 != TEX_NULL {
                self.last_badness = badness(needed, self.total_shrink[NORMAL as usize]);
            }
        }
        Ok(r)
    }

    /// Finds the least-cost legal vertical breakpoint (`vert_break`).
    ///
    /// The seven active-width slots mirror C: natural height at index one,
    /// stretch orders at two through five, and total shrink at six.
    pub fn vert_break(&mut self, mut p: i32, h: i32, d: i32) -> Result<i32, TeXMemoryError> {
        let mut prev_p = p;
        let mut prev_dp = 0i32;
        self.least_page_cost = MAX_HALFWORD;
        self.best_place = TEX_NULL;
        self.best_height_plus_depth = 0;
        self.active_width = [0; 7];

        enum BreakAction {
            Candidate(i32),
            Update,
            Skip,
        }

        loop {
            let action = if p == TEX_NULL {
                BreakAction::Candidate(EJECT_PENALTY)
            } else {
                match self.node_type(p)? {
                    HLIST_NODE | VLIST_NODE | RULE_NODE => {
                        self.active_width[1] = self.active_width[1]
                            .saturating_add(prev_dp)
                            .saturating_add(self.word_at(p, 3)?.s1);
                        prev_dp = self.word_at(p, 2)?.s1;
                        BreakAction::Skip
                    }
                    WHATSIT_NODE => {
                        let subtype = self.node_subtype(p)?;
                        if subtype == PIC_NODE || subtype == PDF_NODE {
                            self.active_width[1] = self.active_width[1]
                                .saturating_add(prev_dp)
                                .saturating_add(self.word_at(p, 3)?.s1);
                            prev_dp = self.word_at(p, 2)?.s1;
                        }
                        BreakAction::Skip
                    }
                    GLUE_NODE => {
                        let discardable = if self.is_char_node(prev_p) {
                            false
                        } else {
                            self.node_type(prev_p)? >= MATH_NODE
                        };
                        if discardable {
                            BreakAction::Update
                        } else {
                            BreakAction::Candidate(0)
                        }
                    }
                    KERN_NODE => {
                        let next = self.link(p)?;
                        let next_type = if next == TEX_NULL {
                            PENALTY_NODE
                        } else {
                            self.node_type(next)?
                        };
                        if next_type == GLUE_NODE {
                            BreakAction::Candidate(0)
                        } else {
                            BreakAction::Update
                        }
                    }
                    PENALTY_NODE => BreakAction::Candidate(self.word_at(p, 1)?.s1),
                    MARK_NODE | INS_NODE => BreakAction::Skip,
                    _ => return Err(TeXMemoryError::CorruptedNode(p)),
                }
            };

            if let BreakAction::Candidate(pi) = action {
                if pi < INF_PENALTY {
                    let mut b = if self.active_width[1] < h {
                        if self.active_width[3] != 0
                            || self.active_width[4] != 0
                            || self.active_width[5] != 0
                        {
                            0
                        } else {
                            badness(h - self.active_width[1], self.active_width[2])
                        }
                    } else if self.active_width[1] - h > self.active_width[6] {
                        MAX_HALFWORD
                    } else {
                        badness(self.active_width[1] - h, self.active_width[6])
                    };

                    if b < MAX_HALFWORD {
                        if pi <= EJECT_PENALTY {
                            b = pi;
                        } else if b < INF_BAD {
                            b = b.saturating_add(pi);
                        } else {
                            b = 100_000;
                        }
                    }
                    if b <= self.least_page_cost {
                        self.best_place = p;
                        self.least_page_cost = b;
                        self.best_height_plus_depth = self.active_width[1].saturating_add(prev_dp);
                    }
                    if b == MAX_HALFWORD || pi <= EJECT_PENALTY {
                        return Ok(self.best_place);
                    }
                }
            }

            if matches!(action, BreakAction::Update) && p != TEX_NULL {
                let q = if self.node_type(p)? == KERN_NODE {
                    p
                } else {
                    self.word_at(p, 1)?.s0
                };
                if self.node_type(p)? == GLUE_NODE {
                    let stretch_order = self.word_at(q, 0)?.b16_s1() as usize;
                    let shrink_order = self.word_at(q, 0)?.b16_s0() as usize;
                    if stretch_order >= 4 || shrink_order >= 4 {
                        return Err(TeXMemoryError::CorruptedNode(q));
                    }
                    self.active_width[2 + stretch_order] =
                        self.active_width[2 + stretch_order].saturating_add(self.word_at(q, 2)?.s1);
                    self.active_width[6] =
                        self.active_width[6].saturating_add(self.word_at(q, 3)?.s1);
                    if shrink_order != NORMAL as usize && self.word_at(q, 3)?.s1 != 0 {
                        let replacement = self.new_spec(q)?;
                        self.mem[replacement as usize].set_b16_s0(NORMAL);
                        self.delete_glue_ref(q)?;
                        self.mem[(p + 1) as usize].s0 = replacement;
                    }
                }
                let width = self.word_at(q, 1)?.s1;
                self.active_width[1] = self.active_width[1]
                    .saturating_add(prev_dp)
                    .saturating_add(width);
                prev_dp = 0;
            }

            if prev_dp > d {
                self.active_width[1] = self.active_width[1].saturating_add(prev_dp - d);
                prev_dp = d;
            }
            prev_p = p;
            p = self.link(prev_p)?;
        }
    }

    /// Removes leading discardable nodes before the first split box
    /// (`prune_page_top`).
    pub fn prune_page_top(
        &mut self,
        mut p: i32,
        save_discards: bool,
    ) -> Result<i32, TeXMemoryError> {
        let mut prev_p = self.temp_head();
        self.mem[prev_p as usize].s1 = p;
        let mut discarded_tail = TEX_NULL;

        while p != TEX_NULL {
            match self.node_type(p)? {
                HLIST_NODE | VLIST_NODE | RULE_NODE => {
                    let q = self.new_skip_param(GLUE_PAR_SPLIT_TOP_SKIP)?;
                    self.mem[prev_p as usize].s1 = q;
                    self.mem[q as usize].s1 = p;
                    let spec = self.temp_ptr;
                    let height = self.word_at(p, 3)?.s1;
                    let width = self.word_at(spec, 1)?.s1;
                    self.mem[(spec + 1) as usize].s1 =
                        if width > height { width - height } else { 0 };
                    p = TEX_NULL;
                }
                WHATSIT_NODE | MARK_NODE | INS_NODE => {
                    prev_p = p;
                    p = self.link(prev_p)?;
                }
                GLUE_NODE | KERN_NODE | PENALTY_NODE => {
                    let q = p;
                    p = self.link(q)?;
                    self.mem[q as usize].s1 = TEX_NULL;
                    self.mem[prev_p as usize].s1 = p;
                    if save_discards {
                        if self.disc_ptr[VSPLIT_CODE as usize] == TEX_NULL {
                            self.disc_ptr[VSPLIT_CODE as usize] = q;
                        } else {
                            self.mem[discarded_tail as usize].s1 = q;
                        }
                        discarded_tail = q;
                    } else {
                        self.flush_node_list(q)?;
                    }
                }
                _ => return Err(TeXMemoryError::CorruptedNode(p)),
            }
        }

        self.link(self.temp_head())
    }

    /// Splits a vertical box at height `h` (`vsplit`).
    pub fn vsplit(&mut self, n: i32, h: i32) -> Result<i32, TeXMemoryError> {
        self.cur_val = n;
        let v = if n < 256 {
            self.box_reg(usize::try_from(n).map_err(|_| TeXMemoryError::OutOfBounds(n, 256))?)?
        } else {
            let element = self.find_sa_element(SA_BOX_VAL, n, false)?;
            if element == TEX_NULL {
                TEX_NULL
            } else {
                self.sparse_ptr_value(element)?
            }
        };

        self.flush_node_list(self.disc_ptr[VSPLIT_CODE as usize])?;
        self.disc_ptr[VSPLIT_CODE as usize] = TEX_NULL;
        let mark_root = self.sa_root[SA_MARK_VAL as usize];
        if mark_root != TEX_NULL && self.do_marks(VSPLIT_INIT, 0, mark_root)? {
            self.sa_root[SA_MARK_VAL as usize] = TEX_NULL;
        }
        if self.cur_mark[SPLIT_FIRST_MARK_CODE as usize] != TEX_NULL {
            self.delete_token_ref(self.cur_mark[SPLIT_FIRST_MARK_CODE as usize])?;
            self.cur_mark[SPLIT_FIRST_MARK_CODE as usize] = TEX_NULL;
            self.delete_token_ref(self.cur_mark[SPLIT_BOT_MARK_CODE as usize])?;
            self.cur_mark[SPLIT_BOT_MARK_CODE as usize] = TEX_NULL;
        }
        if v == TEX_NULL || self.node_type(v)? != VLIST_NODE {
            return Ok(TEX_NULL);
        }

        let mut q = self.vert_break(
            self.word_at(v, 5)?.s1,
            h,
            self.dimen_par(DIMEN_PAR_SPLIT_MAX_DEPTH)?,
        )?;
        let mut p = self.word_at(v, 5)?.s1;
        if p == q {
            self.mem[(v + 5) as usize].s1 = TEX_NULL;
        } else {
            loop {
                if self.node_type(p)? == MARK_NODE {
                    let mark_class = self.word_at(p, 1)?.s0;
                    let mark = self.word_at(p, 1)?.s1;
                    if mark_class != 0 {
                        let element = self.find_sa_element(SA_MARK_VAL, mark_class, true)?;
                        let element_idx = self.check_bounds(element)?;
                        if self.mem[element_idx + 2].s1 == TEX_NULL {
                            self.mem[element_idx + 2].s1 = mark;
                            self.mem[mark as usize].s0 += 1;
                        } else {
                            self.delete_token_ref(self.mem[element_idx + 3].s0)?;
                        }
                        let element_idx = self.check_bounds(element)?;
                        self.mem[element_idx + 3].s0 = mark;
                        self.mem[mark as usize].s0 += 1;
                    } else if self.cur_mark[SPLIT_FIRST_MARK_CODE as usize] == TEX_NULL {
                        self.cur_mark[SPLIT_FIRST_MARK_CODE as usize] = mark;
                        self.cur_mark[SPLIT_BOT_MARK_CODE as usize] = mark;
                        self.mem[mark as usize].s0 += 2;
                    } else {
                        self.delete_token_ref(self.cur_mark[SPLIT_BOT_MARK_CODE as usize])?;
                        self.cur_mark[SPLIT_BOT_MARK_CODE as usize] = mark;
                        self.mem[mark as usize].s0 += 1;
                    }
                }
                if self.link(p)? == q {
                    self.mem[p as usize].s1 = TEX_NULL;
                    break;
                }
                p = self.link(p)?;
            }
        }

        q = self.prune_page_top(q, self.int_par(INT_PAR_SAVING_VDISCARDS)? > 0)?;
        p = self.word_at(v, 5)?.s1;
        self.free_node(v, BOX_NODE_SIZE)?;
        if q != TEX_NULL {
            q = self.vpackage(q, 0, ADDITIONAL, MAX_HALFWORD)?;
        }
        if n < 256 {
            self.set_box_reg(
                usize::try_from(n).map_err(|_| TeXMemoryError::OutOfBounds(n, 256))?,
                q,
            )?;
        } else {
            let element = self.find_sa_element(SA_BOX_VAL, n, false)?;
            if element != TEX_NULL {
                let idx = self.check_bounds(element)?;
                self.mem[idx + 1].s1 = q;
                self.mem[idx + 1].s0 = self.mem[idx + 1].s0.wrapping_add(1);
                self.delete_sa_ref(element)?;
            }
        }
        self.vpackage(p, h, EXACTLY, self.dimen_par(DIMEN_PAR_SPLIT_MAX_DEPTH)?)
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

    /// Returns an integer parameter value (`INTPAR(...)`).
    pub fn int_par(&self, index: usize) -> Result<i32, TeXMemoryError> {
        let idx = INT_PAR_BASE + index;
        if idx >= self.eqtb.len() || index >= INT_PAR_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets an integer parameter value (`INTPAR(...) = ...`).
    pub fn set_int_par(&mut self, index: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = INT_PAR_BASE + index;
        if idx >= self.eqtb.len() || index >= INT_PAR_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Interns a TeX string and returns its `str_number` (`slow_make_string`).
    pub fn intern_string(&mut self, units: &[i32]) -> i32 {
        self.str_pool.extend_from_slice(units);
        self.pool_ptr = self.str_pool.len() as i32;
        self.slow_make_string().unwrap_or(0)
    }

    /// Returns the UTF-16/code-unit data for a pool `str_number`.
    pub fn string_units(&self, id: i32) -> Option<&[i32]> {
        if id < TOO_BIG_CHAR {
            return None;
        }
        let index = (id - TOO_BIG_CHAR) as usize;
        let start = *self.str_start.get(index)?;
        let end = *self.str_start.get(index + 1)?;
        Some(&self.str_pool[start as usize..end as usize])
    }

    /// Converts a pool `str_number` to text for diagnostics.
    pub fn string_text(&self, id: i32) -> Option<String> {
        self.string_units(id).map(|units| {
            units
                .iter()
                .filter_map(|&unit| char::from_u32(unit as u32))
                .collect()
        })
    }

    /// Packs the currently scanned `cur_name`/`cur_area`/`cur_ext` triple.
    pub fn packed_file_name(&self) -> String {
        self.cur_area
            .iter()
            .chain(&self.cur_name)
            .chain(&self.cur_ext)
            .filter_map(|&unit| char::from_u32(unit as u32))
            .collect()
    }

    /// Returns whether a TeX output stream is open (`write_open[n]`).
    pub fn write_stream_open(&self, n: i32) -> bool {
        usize::try_from(n)
            .ok()
            .and_then(|index| self.write_open.get(index))
            .copied()
            .unwrap_or(false)
    }

    /// Returns a dimension parameter value (`DIMENPAR(...)`).
    pub fn dimen_par(&self, index: usize) -> Result<i32, TeXMemoryError> {
        let idx = DIMEN_PAR_BASE + index;
        if idx >= self.eqtb.len() || index >= DIMEN_PAR_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets a dimension parameter value (`DIMENPAR(...) = ...`).
    pub fn set_dimen_par(&mut self, index: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = DIMEN_PAR_BASE + index;
        if idx >= self.eqtb.len() || index >= DIMEN_PAR_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns a glue parameter specification pointer (`GLUEPAR(...)`).
    pub fn glue_par(&self, index: usize) -> Result<i32, TeXMemoryError> {
        let idx = GLUE_PAR_BASE + index;
        if idx >= self.eqtb.len() || index >= GLUE_PAR_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets a glue parameter specification pointer (`GLUEPAR(...) = ...`).
    pub fn set_glue_par(&mut self, index: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = GLUE_PAR_BASE + index;
        if idx >= self.eqtb.len() || index >= GLUE_PAR_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns a local token-list parameter (`LOCAL(...)`).
    pub fn local(&self, index: usize) -> Result<i32, TeXMemoryError> {
        let idx = LOCAL_BASE + index;
        if idx >= self.eqtb.len() || index >= LOCAL_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        Ok(self.eqtb[idx].s1)
    }

    /// Sets a local token-list parameter (`LOCAL(...) = ...`).
    pub fn set_local(&mut self, index: usize, val: i32) -> Result<(), TeXMemoryError> {
        let idx = LOCAL_BASE + index;
        if idx >= self.eqtb.len() || index >= LOCAL_COUNT {
            return Err(TeXMemoryError::OutOfBounds(idx as i32, self.eqtb.len()));
        }
        self.eqtb[idx].s1 = val;
        Ok(())
    }

    /// Returns whether a TeX read stream reports EOF (`read_open[n] == closed`).
    pub fn read_stream_eof(&self, n: i32) -> bool {
        if n == 18 {
            return true;
        }
        if !(0..=15).contains(&n) {
            return true;
        }
        self.read_open
            .get(n as usize)
            .is_none_or(|state| *state == ReadOpen::Closed)
    }

    /// Opens an in-memory source as a TeX read stream (`\openin` substrate).
    pub fn open_read_stream(
        &mut self,
        n: usize,
        source: FileInputSource,
    ) -> Result<(), TeXMemoryError> {
        if n >= self.read_open.len() || n >= self.read_sources.len() {
            return Err(TeXMemoryError::OutOfBounds(n as i32, self.read_open.len()));
        }
        self.read_open[n] = ReadOpen::JustOpen;
        self.read_sources[n] = Some(source);
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
        if (c as usize) >= self.cat_code_levels.len() {
            self.cat_code_levels
                .resize((c as usize + 1).max(256), LEVEL_ONE);
        }
        self.cat_codes[c as usize] = code;
    }

    pub(crate) fn default_code_table_value(&self, table: u8, character: u32) -> i32 {
        let c = character as i32;
        match table {
            CODE_TABLE_CAT => i32::from(self.cat_code(character)),
            CODE_TABLE_LC => match character {
                65..=90 => c + 32,
                97..=122 => c,
                _ => 0,
            },
            CODE_TABLE_UC => match character {
                65..=90 => c,
                97..=122 => c - 32,
                _ => 0,
            },
            CODE_TABLE_SF => match character {
                65..=90 | 97..=122 => 999,
                _ => 1000,
            },
            CODE_TABLE_MATH => match character {
                48..=57 => c + (7 << 21),
                65..=90 | 97..=122 => c + (7 << 21) + (1 << 24),
                _ => c,
            },
            CODE_TABLE_DEL => {
                if character == u32::from(b'.') {
                    0
                } else {
                    -1
                }
            }
            _ => 0,
        }
    }

    pub(crate) fn code_table_word(&self, table: u8, character: u32) -> MemoryWord {
        if table == CODE_TABLE_CAT {
            let mut word = MemoryWord::default();
            word.set_b16_s0(
                self.cat_code_levels
                    .get(character as usize)
                    .copied()
                    .unwrap_or(LEVEL_ONE),
            );
            word.set_b16_s1(CommandCode::Data as u16);
            word.s1 = self.cat_code(character) as i32;
            return word;
        }

        self.code_tables
            .get(table as usize)
            .and_then(|entries| entries.get(&character))
            .copied()
            .unwrap_or_else(|| {
                let mut word = MemoryWord::default();
                word.set_b16_s0(LEVEL_ONE);
                word.set_b16_s1(CommandCode::Data as u16);
                word.s1 = self.default_code_table_value(table, character);
                word
            })
    }

    fn write_code_table_word(
        &mut self,
        table: u8,
        character: u32,
        word: MemoryWord,
    ) -> Result<(), TeXMemoryError> {
        if table as usize >= CODE_TABLE_COUNT {
            return Err(TeXMemoryError::OutOfBounds(
                i32::from(table),
                CODE_TABLE_COUNT,
            ));
        }
        if table == CODE_TABLE_CAT {
            self.set_cat_code(character, word.s1.clamp(0, 15) as u8);
            self.cat_code_levels[character as usize] = word.b16_s0();
        } else {
            self.code_tables[table as usize].insert(character, word);
        }
        Ok(())
    }

    /// Defines a character code-table value with TeX local/global semantics.
    pub fn define_character_code(
        &mut self,
        table: u8,
        character: u32,
        value: i32,
        global: bool,
    ) -> Result<(), TeXMemoryError> {
        if table as usize >= CODE_TABLE_COUNT {
            return Err(TeXMemoryError::OutOfBounds(
                i32::from(table),
                CODE_TABLE_COUNT,
            ));
        }
        let mut word = self.code_table_word(table, character);
        if word.s1 == value && (!global || word.b16_s0() == LEVEL_ONE) {
            return Ok(());
        }
        if !global && word.b16_s0() != self.cur_level {
            self.code_eq_save(table, character, word.b16_s0(), word)?;
        }
        word.set_b16_s0(if global { LEVEL_ONE } else { self.cur_level });
        word.set_b16_s1(CommandCode::Data as u16);
        word.s1 = value;
        self.write_code_table_word(table, character, word)
    }

    /// Restores a saved character code-table word during `unsave`.
    pub(crate) fn restore_character_code(
        &mut self,
        table: u8,
        character: u32,
        saved: MemoryWord,
    ) -> Result<(), TeXMemoryError> {
        let current = self.code_table_word(table, character);
        if current.b16_s0() != LEVEL_ONE {
            self.write_code_table_word(table, character, saved)?;
        }
        Ok(())
    }

    /// Returns a `\lccode` value.
    pub fn lc_code(&self, character: u32) -> i32 {
        self.code_table_word(CODE_TABLE_LC, character).s1
    }

    /// Returns a `\uccode` value.
    pub fn uc_code(&self, character: u32) -> i32 {
        self.code_table_word(CODE_TABLE_UC, character).s1
    }

    /// Returns the packed `\sfcode` / `\XeTeXcharclass` value.
    pub fn sf_code(&self, character: u32) -> i32 {
        self.code_table_word(CODE_TABLE_SF, character).s1
    }

    /// Returns the packed `\mathcode` / XeTeX math-code value.
    pub fn math_code(&self, character: u32) -> i32 {
        self.code_table_word(CODE_TABLE_MATH, character).s1
    }

    /// Returns the packed `\delcode` / XeTeX delimiter-code value.
    pub fn del_code(&self, character: u32) -> i32 {
        self.code_table_word(CODE_TABLE_DEL, character).s1
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
    fn test_new_spec_resets_reference_count() {
        let mut state = TeXEngineState::new(2000);
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[spec as usize].s0 = (FILL as i32) << 16 | FILLL as i32;
        state.mem[spec as usize].s1 = 7; // glue_ref_count
        state.mem[(spec + 1) as usize].s1 = 100;
        state.mem[(spec + 2) as usize].s1 = 200;
        state.mem[(spec + 3) as usize].s1 = 300;

        let copy = state.new_spec(spec).unwrap();
        assert_ne!(copy, spec);
        // Fields and orders are copied; the reference count is nulled.
        assert_eq!(state.mem[copy as usize].s1, TEX_NULL);
        assert_eq!(state.mem[copy as usize].b16_s1(), FILL);
        assert_eq!(state.mem[copy as usize].b16_s0(), FILLL);
        assert_eq!(state.mem[(copy + 1) as usize].s1, 100);
        assert_eq!(state.mem[(copy + 2) as usize].s1, 200);
        assert_eq!(state.mem[(copy + 3) as usize].s1, 300);
        // The original refcount is untouched.
        assert_eq!(state.mem[spec as usize].s1, 7);
    }

    #[test]
    fn test_glue_ref_lifecycle() {
        let mut state = TeXEngineState::new(2000);
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        assert_eq!(state.mem[spec as usize].s1, TEX_NULL);

        state.add_glue_ref(spec).unwrap();
        state.add_glue_ref(spec).unwrap();
        assert_eq!(state.mem[spec as usize].s1, 2);

        state.delete_glue_ref(spec).unwrap();
        assert_eq!(state.mem[spec as usize].s1, 1);
        // Down to TEX_NULL: still allocated (C checks before decrementing).
        state.delete_glue_ref(spec).unwrap();
        assert_eq!(state.mem[spec as usize].s1, TEX_NULL);
        assert_ne!(state.mem[spec as usize].s1, MAX_HALFWORD);
        // A null refcount on delete frees the node.
        state.delete_glue_ref(spec).unwrap();
        assert_eq!(state.mem[spec as usize].s1, MAX_HALFWORD);
        assert_eq!(state.mem[spec as usize].s0, GLUE_SPEC_SIZE);
    }

    #[test]
    fn test_glue_ref_ops_on_null_are_noops() {
        let mut state = TeXEngineState::new(2000);
        state.add_glue_ref(TEX_NULL).unwrap();
        state.delete_glue_ref(TEX_NULL).unwrap();
        assert_eq!(state.mem[0].s0, 0);
        assert_eq!(state.mem[0].s1, 0);
    }

    #[test]
    fn test_new_glue_takes_a_spec_reference() {
        let mut state = TeXEngineState::new(2000);
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        let g = state.new_glue(spec).unwrap();
        assert_eq!(state.node_type(g).unwrap(), GLUE_NODE);
        assert_eq!(state.mem[(g + 1) as usize].s0, spec);
        assert_eq!(state.mem[(g + 1) as usize].s1, TEX_NULL);
        assert_eq!(state.mem[spec as usize].s1, 1);
    }

    #[test]
    fn test_fixed_heads_and_glue_set_word() {
        let state = TeXEngineState::new(2000);
        let top = state.mem_top();
        assert_eq!(state.page_ins_head(), top);
        assert_eq!(state.contrib_head(), top - 1);
        assert_eq!(state.page_head(), top - 2);
        assert_eq!(state.temp_head(), top - 3);
        assert_eq!(state.hold_head(), top - 4);
        assert_eq!(state.adjust_head(), top - 5);
        assert_eq!(state.active_list(), top - 7);
        assert_eq!(state.align_head(), top - 8);
        assert_eq!(state.end_span(), top - 9);
        assert_eq!(state.omit_template(), top - 10);
        assert_eq!(state.null_list(), top - 11);
        assert_eq!(state.lig_trick(), top - 12);
        assert_eq!(state.backup_head(), top - 13);
        assert_eq!(state.pre_adjust_head(), top - 14);
        assert_eq!(state.head, state.contrib_head());
        assert_eq!(state.page_tail, state.page_head());
        assert_eq!(state.aux.s1, IGNORE_DEPTH);
        assert_eq!(state.mem[state.page_ins_head() as usize].s1, top);
        assert_eq!(state.mem[state.active_list() as usize].b16_s1(), HYPHENATED);

        let mut glue_set = MemoryWord::default();
        glue_set.set_gr(0.5);
        assert_eq!(glue_set.gr(), 0.5);
    }

    #[test]
    fn test_copy_node_list_retains_nested_references() {
        let mut state = TeXEngineState::new(4000);
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[(spec + 1) as usize].s1 = 65_536;
        state.mem[(spec + 2) as usize].s1 = 10;
        state.mem[(spec + 3) as usize].s1 = 5;

        let inner = state.new_kern(123).unwrap();
        let boxed = state.new_null_box().unwrap();
        state.mem[(boxed + 5) as usize].s1 = inner;
        state.mem[(boxed + 6) as usize].set_gr(0.25);
        let glue = state.new_glue(spec).unwrap();
        state.mem[glue as usize].s1 = boxed;

        let copied_box = state.copy_node_list(boxed).unwrap();
        assert_ne!(copied_box, boxed);
        assert_eq!(state.node_type(copied_box).unwrap(), HLIST_NODE);
        assert_eq!(state.mem[(copied_box + 6) as usize].gr(), 0.25);
        let copied_inner = state.mem[(copied_box + 5) as usize].s1;
        assert_ne!(copied_inner, inner);
        assert_eq!(state.mem[(copied_inner + 1) as usize].s1, 123);

        let copied_glue = state.copy_node_list(glue).unwrap();
        assert_ne!(copied_glue, glue);
        assert_eq!(state.mem[(copied_glue + 1) as usize].s0, spec);
        assert_eq!(state.mem[spec as usize].s1, 2);
        assert_ne!(state.mem[(copied_glue + 1) as usize].s1, boxed);
    }

    #[test]
    fn test_new_param_and_skip_param_use_glue_parameters() {
        let mut state = TeXEngineState::new(4000);
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[(spec + 1) as usize].s1 = 70;
        state.set_glue_par(GLUE_PAR_BASELINE_SKIP, spec).unwrap();

        let param = state.new_param_glue(GLUE_PAR_BASELINE_SKIP).unwrap();
        assert_eq!(state.mem[(param + 1) as usize].s0, spec);
        assert_eq!(state.mem[param as usize].b16_s0(), 2);
        assert_eq!(state.mem[spec as usize].s1, 1);

        let skipped = state.new_skip_param(GLUE_PAR_BASELINE_SKIP).unwrap();
        let copied_spec = state.mem[(skipped + 1) as usize].s0;
        assert_ne!(copied_spec, spec);
        assert_eq!(state.mem[(copied_spec + 1) as usize].s1, 70);
        assert_eq!(state.mem[copied_spec as usize].s1, TEX_NULL);
    }

    #[test]
    fn test_append_to_vlist_inserts_adjusted_baseline_glue() {
        let mut state = TeXEngineState::new(4000);
        let baseline = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[(baseline + 1) as usize].s1 = 20;
        state
            .set_glue_par(GLUE_PAR_BASELINE_SKIP, baseline)
            .unwrap();
        state.set_dimen_par(DIMEN_PAR_LINE_SKIP_LIMIT, 0).unwrap();

        let boxed = state.new_null_box().unwrap();
        state.mem[(boxed + 2) as usize].s1 = 3;
        state.mem[(boxed + 3) as usize].s1 = 10;
        state.aux.s1 = 0;
        state.append_to_vlist(boxed).unwrap();

        let glue = state.mem[state.head as usize].s1;
        assert_eq!(state.node_type(glue).unwrap(), GLUE_NODE);
        assert_eq!(state.mem[glue as usize].b16_s0(), 2);
        assert_eq!(state.mem[(glue + 1) as usize].s0, state.temp_ptr);
        assert_eq!(state.mem[(state.temp_ptr + 1) as usize].s1, 10);
        assert_eq!(state.tail, boxed);
        assert_eq!(state.aux.s1, 3);
    }

    #[test]
    fn test_hpack_accumulates_character_dimensions() {
        let mut state = TeXEngineState::new(4000);
        state.set_font_char_metric(
            1,
            u32::from(b'A'),
            FontCharMetric {
                width: 10,
                height: 6,
                depth: 2,
                italic: 0,
            },
        );
        state.set_font_char_metric(
            1,
            u32::from(b'B'),
            FontCharMetric {
                width: 5,
                height: 7,
                depth: 1,
                italic: 0,
            },
        );
        let first = state.get_avail().unwrap();
        state.mem[first as usize].set_node_subtype(u16::from(b'A'));
        state.mem[first as usize].set_node_type(1);
        let second = state.get_avail().unwrap();
        state.mem[second as usize].set_node_subtype(u16::from(b'B'));
        state.mem[second as usize].set_node_type(1);
        state.mem[first as usize].s1 = second;

        let boxed = state.hpack(first, 15, EXACTLY).unwrap();
        assert_eq!(state.node_type(boxed).unwrap(), HLIST_NODE);
        assert_eq!(state.mem[(boxed + 1) as usize].s1, 15);
        assert_eq!(state.mem[(boxed + 3) as usize].s1, 7);
        assert_eq!(state.mem[(boxed + 2) as usize].s1, 2);
        assert_eq!(state.mem[(boxed + 5) as usize].s1, first);
        assert_eq!(state.mem[(boxed + 6) as usize].gr(), 0.0);
        assert_eq!(state.last_badness, 0);
    }

    #[test]
    fn test_hpack_computes_stretch_and_shrink_sets() {
        let mut state = TeXEngineState::new(5000);
        let kern = state.new_kern(10).unwrap();
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[(spec + 1) as usize].s1 = 5;
        state.mem[(spec + 2) as usize].s1 = 10;
        state.mem[(spec + 3) as usize].s1 = 8;
        let glue = state.new_glue(spec).unwrap();
        state.mem[kern as usize].s1 = glue;

        let stretched = state.hpack(kern, 30, EXACTLY).unwrap();
        assert_eq!(state.mem[(stretched + 1) as usize].s1, 30);
        assert_eq!(state.mem[(stretched + 5) as usize].b16_s0(), NORMAL);
        assert_eq!(state.mem[(stretched + 5) as usize].b16_s1(), STRETCHING);
        assert_eq!(state.mem[(stretched + 6) as usize].gr(), 1.5);
        assert_eq!(state.last_badness, badness(15, 10));

        let kern = state.new_kern(10).unwrap();
        let glue = state.new_glue(spec).unwrap();
        state.mem[kern as usize].s1 = glue;
        let shrunk = state.hpack(kern, 10, EXACTLY).unwrap();
        assert_eq!(state.mem[(shrunk + 5) as usize].b16_s1(), SHRINKING);
        assert_eq!(state.mem[(shrunk + 6) as usize].gr(), 0.625);
        assert_eq!(state.last_badness, badness(5, 8));
    }

    #[test]
    fn test_hpack_moves_adjust_material_to_holding_lists() {
        let mut state = TeXEngineState::new(5000);
        state.adjust_tail = state.adjust_head();
        state.pre_adjust_tail = state.pre_adjust_head();
        let boxed_item = state.new_null_box().unwrap();
        state.mem[(boxed_item + 1) as usize].s1 = 11;
        state.mem[(boxed_item + 3) as usize].s1 = 4;
        state.mem[(boxed_item + 2) as usize].s1 = 3;
        let adjust = state.get_node(SMALL_NODE_SIZE).unwrap();
        state.set_node_type(adjust, ADJUST_NODE).unwrap();
        let adjustment = state.new_kern(7).unwrap();
        state.mem[(adjust + 1) as usize].s1 = adjustment;
        state.mem[boxed_item as usize].s1 = adjust;

        let packaged = state.hpack(boxed_item, 0, ADDITIONAL).unwrap();
        assert_eq!(state.mem[(packaged + 5) as usize].s1, boxed_item);
        assert_eq!(state.link(boxed_item).unwrap(), TEX_NULL);
        assert_eq!(state.mem[state.adjust_head() as usize].s1, adjustment);
        assert_eq!(state.adjust_tail, adjustment);
        assert_eq!(state.link(adjustment).unwrap(), TEX_NULL);
        assert_eq!(state.mem[(packaged + 1) as usize].s1, 11);
    }

    #[test]
    fn test_hpack_merges_adjacent_native_words() {
        let mut state = TeXEngineState::new(6000);
        state.set_font_char_metric(
            2,
            u32::from(b'a'),
            FontCharMetric {
                width: 5,
                height: 4,
                depth: 0,
                italic: 0,
            },
        );
        state.set_font_char_metric(
            2,
            u32::from(b'b'),
            FontCharMetric {
                width: 6,
                height: 5,
                depth: 1,
                italic: 0,
            },
        );
        let first = state.new_native_word_node(2, 1).unwrap();
        state.set_native_text_at(first, 0, u16::from(b'a')).unwrap();
        let second = state.new_native_word_node(2, 1).unwrap();
        state
            .set_native_text_at(second, 0, u16::from(b'b'))
            .unwrap();
        state.mem[first as usize].s1 = second;

        let packaged = state.hpack(first, 0, ADDITIONAL).unwrap();
        let merged = state.mem[(packaged + 5) as usize].s1;
        assert_eq!(merged, state.mem[(packaged + 5) as usize].s1);
        assert_eq!(state.node_type(merged).unwrap(), WHATSIT_NODE);
        assert_eq!(state.node_subtype(merged).unwrap(), NATIVE_WORD_NODE);
        assert_eq!(state.mem[(merged + 4) as usize].b16_s1(), 2);
        assert_eq!(state.native_text_at(merged, 0).unwrap(), u16::from(b'a'));
        assert_eq!(state.native_text_at(merged, 1).unwrap(), u16::from(b'b'));
        assert_eq!(state.mem[(merged + 1) as usize].s1, 11);
        assert_eq!(state.mem[(merged + 3) as usize].s1, 5);
        assert_eq!(state.mem[(merged + 2) as usize].s1, 1);
        assert_eq!(state.link(merged).unwrap(), TEX_NULL);
    }

    #[test]
    fn test_vpackage_accumulates_vertical_dimensions_and_glue() {
        let mut state = TeXEngineState::new(5000);
        let boxed_item = state.new_null_box().unwrap();
        state.mem[(boxed_item + 1) as usize].s1 = 20;
        state.mem[(boxed_item + 3) as usize].s1 = 3;
        state.mem[(boxed_item + 2) as usize].s1 = 4;
        let spec = state.get_node(GLUE_SPEC_SIZE).unwrap();
        state.mem[(spec + 1) as usize].s1 = 5;
        state.mem[(spec + 2) as usize].s1 = 10;
        let glue = state.new_glue(spec).unwrap();
        state.mem[boxed_item as usize].s1 = glue;

        let packaged = state.vpackage(boxed_item, 20, EXACTLY, 2).unwrap();
        assert_eq!(state.node_type(packaged).unwrap(), VLIST_NODE);
        assert_eq!(state.mem[(packaged + 1) as usize].s1, 20);
        assert_eq!(state.mem[(packaged + 2) as usize].s1, 0);
        assert_eq!(state.mem[(packaged + 3) as usize].s1, 20);
        assert_eq!(state.mem[(packaged + 5) as usize].b16_s1(), STRETCHING);
        assert_eq!(state.mem[(packaged + 6) as usize].gr(), 0.8);
        assert_eq!(state.last_badness, badness(8, 10));
    }

    #[test]
    fn test_vpackage_additional_mode_and_character_confusion() {
        let mut state = TeXEngineState::new(4000);
        let kern = state.new_kern(7).unwrap();
        let packaged = state.vpackage(kern, 5, ADDITIONAL, 100).unwrap();
        assert_eq!(state.mem[(packaged + 3) as usize].s1, 12);
        assert_eq!(state.mem[(packaged + 2) as usize].s1, 0);

        let character = state.get_avail().unwrap();
        assert!(matches!(
            state.vpackage(character, 0, EXACTLY, 0),
            Err(TeXMemoryError::CorruptedNode(p)) if p == character
        ));
    }

    #[test]
    fn test_eq_destroy_releases_reference_counted_values() {
        let mut state = TeXEngineState::new(2000);

        let token_ref = state
            .store_token_ref(&[11 * 0x200000 + i32::from(b'x')])
            .unwrap();
        let mut call = MemoryWord::default();
        call.set_b16_s1(CommandCode::Call as u16);
        call.s1 = token_ref;
        state.eq_destroy(call).unwrap();
        assert_eq!(state.avail, token_ref);

        let shape = state.get_node(5).unwrap();
        state.mem[shape as usize].s0 = 2;
        let mut shape_ref = MemoryWord::default();
        shape_ref.set_b16_s1(CommandCode::ShapeRef as u16);
        shape_ref.s1 = shape;
        state.eq_destroy(shape_ref).unwrap();
        assert_eq!(state.mem[shape as usize].s1, MAX_HALFWORD);

        let boxed = state.new_null_box().unwrap();
        let mut box_ref = MemoryWord::default();
        box_ref.set_b16_s1(CommandCode::BoxRef as u16);
        box_ref.s1 = boxed;
        state.eq_destroy(box_ref).unwrap();
        assert_eq!(state.mem[boxed as usize].s1, MAX_HALFWORD);
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
        assert_eq!(state.cat_code(b'{' as u32), 1); // LeftBrace
        assert_eq!(state.cat_code(b'}' as u32), 2); // RightBrace
        assert_eq!(state.cat_code(b'$' as u32), 3); // MathShift
        assert_eq!(state.cat_code(b'&' as u32), 4); // TabMark
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
