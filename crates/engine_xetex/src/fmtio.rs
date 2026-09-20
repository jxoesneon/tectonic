//! Pure-Rust port of the format-file machinery in `xetex-ini.c`:
//! `do_dump`/`do_undump`/`swap_items`, `store_fmt_file`, and
//! `load_fmt_file`.
//!
//! Format files are always big-endian on disk (`swap_items` byte-reverses
//! every item on little-endian hosts). Each `memory_word` is written as its
//! eight raw bytes reversed, so on disk a word reads as `b32.s1` then
//! `b32.s0` (or `b16.s3, s2, s1, s0`), each big-endian.
//!
//! The live engine keeps compact tables (`cs_table`, compact `eqtb`,
//! `HashMap` hyphenation dictionaries, `FontRecord`s). The serialization
//! path materializes the C-layout image through the shadow structures
//! maintained by `ini.rs` (`fmt_hash`, `fmt_eqtb`, `prim`, the packed trie,
//! `hyph_*` arrays) plus per-region translation from the compact tables.

use std::collections::HashMap;

use crate::scanner::{TeXMouthError, CS_TOKEN_FLAG};
use crate::state::*;

/// `FORMAT_HEADER_MAGIC` — the "TTNC" format-file signature.
pub(crate) const FORMAT_HEADER_MAGIC: i32 = 0x5454_4E43;
/// `FORMAT_FOOTER_MAGIC` — the trailing format-file signature.
pub(crate) const FORMAT_FOOTER_MAGIC: i32 = 0x0000_029A;
/// `FORMAT_SERIAL` — the vendored engine's format version (33).
pub(crate) const FORMAT_SERIAL: i32 = 33;
/// `\patterns`/`\hyphenation` prime modulus used in the format header.
const HYPH_PRIME_VALUE: i32 = HYPH_PRIME;

// === Big-endian reader/writer (`do_dump`/`do_undump`/`swap_items`) ===

/// Big-endian format writer (`do_dump`).
#[derive(Default)]
pub(crate) struct FmtWriter {
    /// Accumulated on-disk image.
    pub bytes: Vec<u8>,
}

impl FmtWriter {
    /// `dump_int` — one big-endian i32.
    pub(crate) fn int(&mut self, v: i32) {
        self.bytes.extend_from_slice(&v.to_be_bytes());
    }

    /// `dump_things` of `packed_UTF16_code`/`nine_bits` — one BE u16.
    pub(crate) fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_be_bytes());
    }

    /// `dump_things` of `small_number` — one BE i16.
    pub(crate) fn i16(&mut self, v: i16) {
        self.bytes.extend_from_slice(&v.to_be_bytes());
    }

    /// `dump_things` of `memory_word` — `s1` then `s0`, each BE.
    pub(crate) fn word(&mut self, w: MemoryWord) {
        self.int(w.s1);
        self.int(w.s0);
    }

    /// `dump_b32` of `two_halves` — `rh` then `lh`, each BE.
    pub(crate) fn b32(&mut self, t: TwoHalves) {
        self.int(t.rh);
        self.int(t.lh);
    }

    /// `dump_things` of a `str_start`-style i32 array.
    pub(crate) fn ints(&mut self, vs: &[i32]) {
        for &v in vs {
            self.int(v);
        }
    }

    /// `dump_things` of a `memory_word` array.
    pub(crate) fn words(&mut self, ws: &[MemoryWord]) {
        for &w in ws {
            self.word(w);
        }
    }

    /// `dump_things` of a UTF-16/`nine_bits` array.
    pub(crate) fn u16s(&mut self, vs: &[u16]) {
        for &v in vs {
            self.u16(v);
        }
    }

    /// `dump_things` of a `small_number` array.
    pub(crate) fn i16s(&mut self, vs: &[i16]) {
        for &v in vs {
            self.i16(v);
        }
    }
}

/// Big-endian format reader (`do_undump`).
pub(crate) struct FmtReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> FmtReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], TeXMouthError> {
        if self.pos + n > self.data.len() {
            return Err(TeXMouthError::BadFormatFile);
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// `undump_int` — one BE i32.
    pub(crate) fn int(&mut self) -> Result<i32, TeXMouthError> {
        let b = self.take(4)?;
        Ok(i32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// One BE u16.
    pub(crate) fn u16(&mut self) -> Result<u16, TeXMouthError> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    /// One BE i16.
    pub(crate) fn i16(&mut self) -> Result<i16, TeXMouthError> {
        let b = self.take(2)?;
        Ok(i16::from_be_bytes([b[0], b[1]]))
    }

    /// `undump_things` of `memory_word` — `s1` then `s0`.
    pub(crate) fn word(&mut self) -> Result<MemoryWord, TeXMouthError> {
        let s1 = self.int()?;
        let s0 = self.int()?;
        Ok(MemoryWord { s0, s1 })
    }

    /// `undump_b32` of `two_halves` — `rh` then `lh`.
    pub(crate) fn b32(&mut self) -> Result<TwoHalves, TeXMouthError> {
        let rh = self.int()?;
        let lh = self.int()?;
        Ok(TwoHalves { lh, rh })
    }

    pub(crate) fn ints(&mut self, n: usize) -> Result<Vec<i32>, TeXMouthError> {
        (0..n).map(|_| self.int()).collect()
    }

    pub(crate) fn words(&mut self, n: usize) -> Result<Vec<MemoryWord>, TeXMouthError> {
        (0..n).map(|_| self.word()).collect()
    }

    pub(crate) fn u16s(&mut self, n: usize) -> Result<Vec<u16>, TeXMouthError> {
        (0..n).map(|_| self.u16()).collect()
    }

    pub(crate) fn i16s(&mut self, n: usize) -> Result<Vec<i16>, TeXMouthError> {
        (0..n).map(|_| self.i16()).collect()
    }
}

// === Region translation between compact and C-layout eqtb indices ===

impl TeXEngineState {
    /// Maps a compact eqtb region index to its C-layout format position
    /// (`compact_equiv_to_c`). Non-region values pass through unchanged
    /// (mem pointers, code-table bases, noad kinds, font ids are already
    /// shared between the models).
    pub(crate) fn compact_eqtb_pos_to_c(pos: i32) -> i32 {
        let p = pos as usize;
        if p < SKIP_BASE + 256 && pos >= SKIP_BASE as i32 {
            // \skip registers
            C_SKIP_BASE + pos - SKIP_BASE as i32
        } else if (MU_SKIP_BASE..MU_SKIP_BASE + 256).contains(&p) {
            C_MU_SKIP_BASE + pos - MU_SKIP_BASE as i32
        } else if (TOKS_BASE..TOKS_BASE + 256).contains(&p) {
            C_TOKS_BASE + pos - TOKS_BASE as i32
        } else if (BOX_BASE..BOX_BASE + 256).contains(&p) {
            C_BOX_BASE + pos - BOX_BASE as i32
        } else if (COUNT_BASE..COUNT_BASE + 256).contains(&p) {
            C_COUNT_BASE + pos - COUNT_BASE as i32
        } else if (DIMEN_BASE..DIMEN_BASE + 256).contains(&p) {
            C_DIMEN_BASE + C_DIMEN_PARS + pos - DIMEN_BASE as i32
        } else if (INT_PAR_BASE..INT_PAR_BASE + C_INT_PARS as usize).contains(&p) {
            C_INT_BASE + pos - INT_PAR_BASE as i32
        } else if (LOCAL_BASE..LOCAL_BASE + C_NUM_LOCALS as usize).contains(&p) {
            C_LOCAL_BASE + pos - LOCAL_BASE as i32
        } else if (ETEX_PEN_BASE..ETEX_PEN_BASE + C_NUM_ETEX_PENALTIES as usize).contains(&p) {
            C_ETEX_PEN_BASE + pos - ETEX_PEN_BASE as i32
        } else if (DIMEN_PAR_BASE..DIMEN_PAR_BASE + C_DIMEN_PARS as usize).contains(&p) {
            C_DIMEN_BASE + pos - DIMEN_PAR_BASE as i32
        } else if (GLUE_PAR_BASE..GLUE_PAR_BASE + C_GLUE_PARS as usize).contains(&p) {
            C_GLUE_BASE + pos - GLUE_PAR_BASE as i32
        } else if pos == CUR_FONT_LOC as i32 {
            C_CUR_FONT_LOC
        } else if (FAMILY_BASE..FAMILY_BASE + FAMILY_COUNT).contains(&p) {
            C_MATH_FONT_BASE + pos - FAMILY_BASE as i32
        } else {
            pos
        }
    }

    /// Inverse of [`compact_eqtb_pos_to_c`] (`c_equiv_to_compact`).
    pub(crate) fn c_eqtb_pos_to_compact(pos: i32) -> i32 {
        if (C_GLUE_BASE..C_SKIP_BASE).contains(&pos) {
            GLUE_PAR_BASE as i32 + pos - C_GLUE_BASE
        } else if (C_SKIP_BASE..C_MU_SKIP_BASE).contains(&pos) {
            SKIP_BASE as i32 + pos - C_SKIP_BASE
        } else if (C_MU_SKIP_BASE..C_LOCAL_BASE).contains(&pos) {
            MU_SKIP_BASE as i32 + pos - C_MU_SKIP_BASE
        } else if (C_LOCAL_BASE..C_TOKS_BASE).contains(&pos) {
            LOCAL_BASE as i32 + pos - C_LOCAL_BASE
        } else if (C_TOKS_BASE..C_ETEX_PEN_BASE).contains(&pos) {
            TOKS_BASE as i32 + pos - C_TOKS_BASE
        } else if (C_ETEX_PEN_BASE..C_CUR_FONT_LOC).contains(&pos) {
            ETEX_PEN_BASE as i32 + pos - C_ETEX_PEN_BASE
        } else if pos == C_CUR_FONT_LOC {
            CUR_FONT_LOC as i32
        } else if (C_MATH_FONT_BASE..C_CAT_CODE_BASE).contains(&pos) {
            FAMILY_BASE as i32 + pos - C_MATH_FONT_BASE
        } else if (C_INT_BASE..C_COUNT_BASE).contains(&pos) {
            INT_PAR_BASE as i32 + pos - C_INT_BASE
        } else if (C_COUNT_BASE..C_DEL_CODE_BASE).contains(&pos) {
            COUNT_BASE as i32 + pos - C_COUNT_BASE
        } else if pos >= C_DIMEN_BASE {
            let off = pos - C_DIMEN_BASE;
            if off < C_DIMEN_PARS {
                DIMEN_PAR_BASE as i32 + off
            } else {
                DIMEN_BASE as i32 + off - C_DIMEN_PARS
            }
        } else {
            pos
        }
    }

    // === C-layout eqtb image ===

    /// Materializes the `initialize`/`initialize_more_initex_variables`
    /// eqtb defaults in C layout (`eqtb[ACTIVE_BASE..=eqtb_top]`).
    fn fmt_eqtb_defaults(&self) -> Vec<MemoryWord> {
        let size = (C_EQTB_SIZE + 1 + self.hash_high.max(0)) as usize;
        let undefined = Self::fmt_eqtb_word(LEVEL_ZERO, CommandCode::UndefinedCs as u8, TEX_NULL);
        let mut e = vec![undefined; size];
        // glue/par-shape/local regions
        let glue = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::GlueRef as u8, 0);
        for k in C_GLUE_BASE..C_LOCAL_BASE {
            e[k as usize] = glue;
        }
        let shape = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::ShapeRef as u8, TEX_NULL);
        for k in C_LOCAL_BASE..C_TOKS_BASE {
            e[k as usize] = shape;
        }
        for k in C_ETEX_PEN_BASE..C_ETEX_PEN_BASE + C_NUM_ETEX_PENALTIES {
            e[k as usize] = shape;
        }
        for k in C_TOKS_BASE..C_TOKS_BASE + C_NUMBER_REGS {
            e[k as usize] = undefined;
        }
        let bx = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::BoxRef as u8, TEX_NULL);
        for k in C_BOX_BASE..C_BOX_BASE + C_NUMBER_REGS {
            e[k as usize] = bx;
        }
        let font = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::Data as u8, FONT_BASE);
        e[C_CUR_FONT_LOC as usize] = font;
        for k in C_MATH_FONT_BASE..C_MATH_FONT_BASE + C_NUMBER_MATH_FONTS {
            e[k as usize] = font;
        }
        let data = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::Data as u8, 0);
        for k in C_CAT_CODE_BASE..C_INT_BASE {
            e[k as usize] = data;
        }
        // Character-code defaults.
        for k in 0..C_NUMBER_USVS {
            e[(C_CAT_CODE_BASE + k) as usize].s1 = i32::from(crate::scanner::CAT_OTHER_CHAR);
            e[(C_MATH_CODE_BASE + k) as usize].s1 = k;
            e[(C_SF_CODE_BASE + k) as usize].s1 = 1000;
        }
        e[(C_CAT_CODE_BASE + 13) as usize].s1 = i32::from(crate::scanner::CAT_CAR_RET);
        e[(C_CAT_CODE_BASE + 32) as usize].s1 = i32::from(crate::scanner::CAT_SPACER);
        e[(C_CAT_CODE_BASE + 92) as usize].s1 = i32::from(crate::scanner::CAT_ESCAPE);
        e[(C_CAT_CODE_BASE + 37) as usize].s1 = i32::from(crate::scanner::CAT_COMMENT);
        e[(C_CAT_CODE_BASE + 127) as usize].s1 = i32::from(crate::scanner::CAT_INVALID_CHAR);
        e[(C_CAT_CODE_BASE) as usize].s1 = i32::from(crate::scanner::CAT_IGNORE);
        for k in i32::from(b'0')..=i32::from(b'9') {
            e[(C_MATH_CODE_BASE + k) as usize].s1 = k + (7 << 21);
        }
        for k in i32::from(b'A')..=i32::from(b'Z') {
            e[(C_CAT_CODE_BASE + k) as usize].s1 = i32::from(crate::scanner::CAT_LETTER);
            e[(C_CAT_CODE_BASE + k + 32) as usize].s1 = i32::from(crate::scanner::CAT_LETTER);
            e[(C_MATH_CODE_BASE + k) as usize].s1 = k + (1 << 24) + (7 << 21);
            e[(C_MATH_CODE_BASE + k + 32) as usize].s1 = k + 32 + (1 << 24) + (7 << 21);
            e[(C_LC_CODE_BASE + k) as usize].s1 = k + 32;
            e[(C_LC_CODE_BASE + k + 32) as usize].s1 = k + 32;
            e[(C_UC_CODE_BASE + k) as usize].s1 = k;
            e[(C_UC_CODE_BASE + k + 32) as usize].s1 = k;
            e[(C_SF_CODE_BASE + k) as usize].s1 = 999;
        }
        for k in C_INT_BASE..C_DEL_CODE_BASE {
            e[k as usize] = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::UndefinedCs as u8, 0);
        }
        e[(C_INT_BASE + INT_PAR_MAG as i32) as usize].s1 = 1000;
        e[(C_INT_BASE + INT_PAR_TOLERANCE as i32) as usize].s1 = 10000;
        e[(C_INT_BASE + INT_PAR_HANG_AFTER as i32) as usize].s1 = 1;
        e[(C_INT_BASE + INT_PAR_MAX_DEAD_CYCLES as i32) as usize].s1 = 25;
        e[(C_INT_BASE + INT_PAR_ESCAPE_CHAR as i32) as usize].s1 = i32::from(b'\\');
        e[(C_INT_BASE + INT_PAR_END_LINE_CHAR as i32) as usize].s1 = 13;
        for k in 0..C_NUMBER_USVS {
            e[(C_DEL_CODE_BASE + k) as usize].s1 = -1;
        }
        e[(C_DEL_CODE_BASE + i32::from(b'.')) as usize].s1 = 0;
        for k in C_DIMEN_BASE..=C_EQTB_SIZE {
            e[k as usize] = Self::fmt_eqtb_word(LEVEL_ONE, CommandCode::UndefinedCs as u8, 0);
        }
        e
    }

    /// Replays every named `cs_table` entry through `fmt_id_lookup` so the
    /// C-layout image gains a hash position and eqtb word for each defined
    /// control sequence.
    pub(crate) fn sync_fmt_cs_image(&mut self) -> Result<(), TeXMouthError> {
        let saved = self.no_new_control_sequence;
        self.no_new_control_sequence = false;
        let n = self.cs_table.len();
        for cs in 0..n as i32 {
            let name = self.cs_table.name(cs).to_vec();
            // Sentinel/empty names have no C representation.
            if name.iter().any(|&c| c < 0) {
                continue;
            }
            let command = self.cs_table.command(cs);
            let chr = self.cs_table.character(cs);
            let level = self.cs_table.level(cs).unwrap_or(LEVEL_ONE);
            let pos = if name.len() == 1 {
                name[0] + C_SINGLE_BASE
            } else if name.is_empty() {
                continue;
            } else {
                if self.buffer.len() < name.len() {
                    self.buffer.resize(name.len(), 0);
                }
                self.buffer[..name.len()].copy_from_slice(&name);
                self.fmt_id_lookup(0, name.len())?
            };
            self.fmt_eqtb.insert(
                pos,
                Self::fmt_eqtb_word(level, command, Self::compact_eqtb_pos_to_c(chr)),
            );
        }
        self.no_new_control_sequence = saved;
        Ok(())
    }

    /// Materializes the complete C-layout eqtb image: defaults, compact
    /// region copies, and the primitive/registered `fmt_eqtb` overlay.
    fn materialize_fmt_eqtb(&mut self) -> Result<Vec<MemoryWord>, TeXMouthError> {
        self.sync_fmt_cs_image()?;
        let mut e = self.fmt_eqtb_defaults();

        // Compact register/parameter regions → C positions. Copy the full
        // word (level + type + value); type/level differences from C
        // defaults are preserved because they reflect real assignments.
        let copy_region = |state: &Self, lo_c: usize, hi_c: usize, e: &mut [MemoryWord]| {
            for cp in lo_c..=hi_c {
                let c_pos = Self::compact_eqtb_pos_to_c(cp as i32) as usize;
                if c_pos < e.len() && cp < state.eqtb.len() {
                    e[c_pos] = state.eqtb[cp];
                }
            }
        };
        copy_region(self, SKIP_BASE, SKIP_BASE + 255, &mut e);
        copy_region(self, MU_SKIP_BASE, MU_SKIP_BASE + 255, &mut e);
        copy_region(self, TOKS_BASE, TOKS_BASE + 255, &mut e);
        copy_region(self, BOX_BASE, BOX_BASE + 255, &mut e);
        copy_region(self, COUNT_BASE, COUNT_BASE + 255, &mut e);
        copy_region(self, DIMEN_BASE, DIMEN_BASE + 255, &mut e);
        copy_region(
            self,
            INT_PAR_BASE,
            INT_PAR_BASE + (C_INT_PARS - 1) as usize,
            &mut e,
        );
        copy_region(
            self,
            LOCAL_BASE,
            LOCAL_BASE + (C_NUM_LOCALS - 1) as usize,
            &mut e,
        );
        copy_region(
            self,
            ETEX_PEN_BASE,
            ETEX_PEN_BASE + (C_NUM_ETEX_PENALTIES - 1) as usize,
            &mut e,
        );
        copy_region(
            self,
            DIMEN_PAR_BASE,
            DIMEN_PAR_BASE + (C_DIMEN_PARS - 1) as usize,
            &mut e,
        );
        copy_region(
            self,
            GLUE_PAR_BASE,
            GLUE_PAR_BASE + (C_GLUE_PARS - 1) as usize,
            &mut e,
        );
        copy_region(self, CUR_FONT_LOC, CUR_FONT_LOC, &mut e);
        copy_region(self, FAMILY_BASE, FAMILY_BASE + FAMILY_COUNT - 1, &mut e);

        // Character code tables.
        for c in 0..C_NUMBER_USVS {
            let cu = c as u32;
            e[(C_CAT_CODE_BASE + c) as usize] = Self::fmt_eqtb_word(
                self.cat_code_levels
                    .get(cu as usize)
                    .copied()
                    .unwrap_or(LEVEL_ONE),
                CommandCode::Data as u8,
                i32::from(self.cat_code(cu)),
            );
            for (table, base) in [
                (CODE_TABLE_LC, C_LC_CODE_BASE),
                (CODE_TABLE_UC, C_UC_CODE_BASE),
                (CODE_TABLE_SF, C_SF_CODE_BASE),
                (CODE_TABLE_MATH, C_MATH_CODE_BASE),
                (CODE_TABLE_DEL, C_DEL_CODE_BASE),
            ] {
                let w = self.code_table_word(table, cu);
                e[(base + c) as usize] = w;
            }
        }

        // fmt_eqtb overlay: primitives, frozen entries, registered
        // control-sequence words.
        for (&pos, &w) in &self.fmt_eqtb {
            if pos >= 0 && (pos as usize) < e.len() {
                e[pos as usize] = w;
            }
        }
        // Token-list pointers live in the single-word region at the top of
        // the compact arena; shift them onto the C `MEM_TOP` boundary (see
        // `store_fmt_file`'s hi-image relocation). Only the token-list
        // regions are remapped — register values elsewhere must never move.
        let compact_mem_top = self.mem.len() as i32 - 1;
        let hi_shift = C_MEM_TOP - compact_mem_top;
        for pos in (C_LOCAL_BASE..C_LOCAL_BASE + C_NUM_LOCALS)
            .chain(C_TOKS_BASE..C_TOKS_BASE + C_NUMBER_REGS)
        {
            let cell = &mut e[pos as usize];
            if (self.hi_mem_min..=compact_mem_top).contains(&cell.s1) {
                cell.s1 += hi_shift;
            }
        }
        Ok(e)
    }

    // === `store_fmt_file` ===

    /// C `store_fmt_file` — serializes the engine state into
    /// [`Self::fmt_bytes`] in the exact upstream big-endian layout.
    pub(crate) fn store_fmt_file(&mut self) -> Result<(), TeXMouthError> {
        if self.save_ptr != 0 {
            self.error_here_with_diagnostic("You can't dump inside a group");
            self.capture_to_diagnostic(false);
            self.help_ptr = 1;
            self.help_line[0] = "`{...\\dump}' is a no-no.".to_string();
            if self.interaction == ERROR_STOP_MODE {
                self.interaction = SCROLL_MODE;
            }
            if self.log_opened {
                self.error()?;
            }
            self.history = HISTORY_FATAL_ERROR;
            return Err(TeXMouthError::BadFormatFile);
        }

        self.selector = SELECTOR_NEW_STRING;
        self.print_cstr(" (preloaded format=")?;
        if let Some(job) = self.job_name.clone() {
            self.print_cstr(&job)?;
        }
        self.print_char(i32::from(b' '))?;
        self.print_int(self.int_par(INT_PAR_YEAR)?)?;
        self.print_char(i32::from(b'.'))?;
        self.print_int(self.int_par(INT_PAR_MONTH)?)?;
        self.print_char(i32::from(b'.'))?;
        self.print_int(self.int_par(INT_PAR_DAY)?)?;
        self.print_char(i32::from(b')'))?;

        if self.interaction == BATCH_MODE {
            self.selector = SELECTOR_LOG_ONLY;
        } else {
            self.selector = SELECTOR_TERM_AND_LOG;
        }

        if self.pool_ptr + 1 > self.pool_size {
            self.overflow("pool size", self.pool_size - self.init_pool_ptr)?;
        }
        self.format_ident = self.make_string()?;

        self.print_nl_cstr("Beginning to dump on file ")?;
        self.print(self.format_ident)?;
        self.str_ptr -= 1;
        self.pool_ptr = self
            .str_start
            .get((self.str_ptr - TOO_BIG_CHAR) as usize)
            .copied()
            .unwrap_or(0);
        self.print_nl_cstr("")?;
        self.print(self.format_ident)?;

        while self.pseudo_files != TEX_NULL {
            self.pseudo_close()?;
        }

        // Intern every `cs_table` name into the string pool before it is
        // serialized — the C-layout hash image is also synced as a side
        // effect (idempotent; `materialize_fmt_eqtb` re-runs it later).
        self.sync_fmt_cs_image()?;

        let mut w = FmtWriter::default();
        w.int(FORMAT_HEADER_MAGIC);
        w.int(FORMAT_SERIAL);
        w.int(self.hash_high);
        w.int(C_MEM_TOP);
        w.int(C_EQTB_SIZE);
        w.int(C_HASH_PRIME);
        w.int(HYPH_PRIME_VALUE);

        // string pool
        w.int(self.pool_ptr);
        w.int(self.str_ptr);
        w.ints(&self.str_start[..(self.str_ptr - TOO_BIG_CHAR + 1) as usize]);
        let pool: Vec<u16> = self.str_pool[..self.pool_ptr as usize]
            .iter()
            .map(|&u| u as u16)
            .collect();
        w.u16s(&pool);

        self.print_ln();
        self.print_int(self.str_ptr)?;
        self.print_cstr(" strings of total length ")?;
        self.print_int(self.pool_ptr)?;

        // "memory locations"
        self.sort_avail()?;
        self.var_used = 0;
        w.int(self.lo_mem_max);
        w.int(self.rover);
        for k in 0..=SA_INTER_CHAR_VAL {
            w.int(self.sa_root[k as usize]);
        }

        let mut p = 0i32;
        let mut q = self.rover;
        let mut x = 0i32;
        loop {
            let count = (q + 2 - p) as usize;
            w.words(&self.mem[p as usize..(q + 2) as usize]);
            debug_assert_eq!(count, (q + 2 - p) as usize);
            x += q + 2 - p;
            self.var_used += q - p;
            p = q + self.mem[q as usize].s0;
            q = self.mem[(q + 1) as usize].s1;
            if q == self.rover {
                break;
            }
        }
        self.var_used += self.lo_mem_max - p;
        // The compact arena's high end maps onto C `MEM_TOP`: single-word
        // nodes (token lists, `avail`, the stat heads) are dumped at the C
        // boundary and their intra-region links are shifted accordingly.
        let compact_mem_top = self.mem.len() as i32 - 1;
        let hi_shift = C_MEM_TOP - compact_mem_top;
        let c_hi_mem_min = self.hi_mem_min + hi_shift;
        let mut hi_image: Vec<MemoryWord> =
            self.mem[self.hi_mem_min as usize..=compact_mem_top as usize].to_vec();
        for wd in &mut hi_image {
            if (self.hi_mem_min..=compact_mem_top).contains(&wd.s1) {
                wd.s1 += hi_shift;
            }
        }
        self.dyn_used = C_MEM_TOP + 1 - c_hi_mem_min;
        w.words(&self.mem[p as usize..(self.lo_mem_max + 1) as usize]);
        x += self.lo_mem_max + 1 - p;
        w.int(c_hi_mem_min);
        let avail = if (self.hi_mem_min..=compact_mem_top).contains(&self.avail) {
            self.avail + hi_shift
        } else {
            self.avail
        };
        w.int(avail);
        w.words(&hi_image);
        x += C_MEM_TOP + 1 - c_hi_mem_min;
        let mut p = self.avail;
        while p != TEX_NULL {
            self.dyn_used -= 1;
            p = self.mem[p as usize].s1;
        }
        w.int(self.var_used);
        w.int(self.dyn_used);

        self.print_ln();
        self.print_int(x)?;
        self.print_cstr(" memory locations dumped; current usage is ")?;
        self.print_int(self.var_used)?;
        self.print_char(i32::from(b'&'))?;
        self.print_int(self.dyn_used)?;

        // equivalents table / primitives
        let e = self.materialize_fmt_eqtb()?;
        let eq = |w1: &MemoryWord, w2: &MemoryWord| w1.s0 == w2.s0 && w1.s1 == w2.s1;

        let mut k = C_ACTIVE_BASE;
        while k != C_INT_BASE {
            let mut j = k;
            let l;
            let mut found = false;
            while j < C_INT_BASE - 1 {
                let a = &e[j as usize];
                let b = &e[(j + 1) as usize];
                if a.s1 == b.s1 && a.b16_s1() == b.b16_s1() && a.b16_s0() == b.b16_s0() {
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                l = C_INT_BASE;
            } else {
                j += 1;
                l = j;
                while j < C_INT_BASE - 1 {
                    let a = &e[j as usize];
                    let b = &e[(j + 1) as usize];
                    if a.s1 != b.s1 || a.b16_s1() != b.b16_s1() || a.b16_s0() != b.b16_s0() {
                        break;
                    }
                    j += 1;
                }
            }
            w.int(l - k);
            w.words(&e[k as usize..l as usize]);
            k = j + 1;
            w.int(k - l);
        }

        while k <= C_EQTB_SIZE {
            let mut j = k;
            let l;
            let mut found = false;
            while j < C_EQTB_SIZE {
                if e[j as usize].s1 == e[(j + 1) as usize].s1 {
                    found = true;
                    break;
                }
                j += 1;
            }
            if !found {
                l = C_EQTB_SIZE + 1;
            } else {
                j += 1;
                l = j;
                while j < C_EQTB_SIZE {
                    if e[j as usize].s1 != e[(j + 1) as usize].s1 {
                        break;
                    }
                    j += 1;
                }
            }
            w.int(l - k);
            w.words(&e[k as usize..l as usize]);
            k = j + 1;
            w.int(k - l);
        }
        let _ = eq;

        if self.hash_high > 0 {
            w.words(&e[(C_EQTB_SIZE + 1) as usize..(C_EQTB_SIZE + 1 + self.hash_high) as usize]);
        }
        w.int(self.fmt_par_loc);
        w.int(self.fmt_write_loc);
        for p in 0..=C_PRIM_SIZE {
            w.b32(self.prim[p as usize]);
        }

        // control sequences
        w.int(self.hash_used);
        self.cs_count = (C_FROZEN_CONTROL_SEQUENCE - 1) - self.hash_used + self.hash_high;
        for p in C_HASH_BASE..=self.hash_used {
            if let Some(t) = self.fmt_hash.get(&p) {
                if t.rh != 0 {
                    w.int(p);
                    w.b32(*t);
                    self.cs_count += 1;
                }
            }
        }
        for p in (self.hash_used + 1)..C_UNDEFINED_CONTROL_SEQUENCE {
            w.b32(self.fmt_hash.get(&p).copied().unwrap_or_default());
        }
        if self.hash_high > 0 {
            for p in (C_EQTB_SIZE + 1)..=(C_EQTB_SIZE + self.hash_high) {
                w.b32(self.fmt_hash.get(&p).copied().unwrap_or_default());
            }
        }
        w.int(self.cs_count);

        self.print_ln();
        self.print_int(self.cs_count)?;
        self.print_cstr(" multiletter control sequences")?;

        // fonts
        let font_ptr = self.font_table.len().saturating_sub(1) as i32;
        let (font_info, arrays) = self.build_fmt_font_arrays()?;
        self.fmem_ptr = font_info.len() as i32;
        w.int(self.fmem_ptr);
        w.words(&font_info);
        w.int(font_ptr);
        w.words(&arrays.font_check);
        for arr in &arrays.i32_arrays {
            w.ints(arr);
        }
        w.u16s(&arrays.font_bc);
        w.u16s(&arrays.font_ec);
        w.ints(&arrays.char_base);
        w.ints(&arrays.width_base);
        w.ints(&arrays.height_base);
        w.ints(&arrays.depth_base);
        w.ints(&arrays.italic_base);
        w.ints(&arrays.lig_kern_base);
        w.ints(&arrays.kern_base);
        w.ints(&arrays.exten_base);
        w.ints(&arrays.param_base);
        w.ints(&arrays.font_glue);
        w.ints(&arrays.bchar_label);
        w.u16s(&arrays.font_bchar);
        w.u16s(&arrays.font_false_bchar);

        for k in FONT_BASE as usize..=font_ptr as usize {
            self.print_nl_cstr("\\font")?;
            let name = self.font_table[k].cs_name.clone();
            let name_id = self.intern_string(&name);
            self.print_esc(name_id)?;
            self.flush_str(name_id);
            self.print_char(i32::from(b'='))?;
            let f = &self.font_table[k];
            if f.font_type != 0 || !f.text_mapping.is_empty() {
                self.error_here_with_diagnostic(
                    "Can't \\dump a format with native fonts or font-mappings",
                );
                self.capture_to_diagnostic(false);
                self.help_ptr = 3;
                self.help_line[2] = "You really, really don't want to do this.".to_string();
                self.help_line[1] = "It won't work, and only confuses me.".to_string();
                self.help_line[0] =
                    "(Load them at runtime, not as part of the format file.)".to_string();
                self.error()?;
            }
            let size = self.font_table[k].size;
            if size != self.font_table[k].dsize {
                self.print_cstr(" at ")?;
                self.print_scaled(size)?;
                self.print_cstr("pt")?;
            }
        }

        self.print_ln();
        self.print_int(self.fmem_ptr - 7)?;
        self.print_cstr(" words of font info for ")?;
        self.print_int(font_ptr)?;
        if font_ptr != FONT_BASE + 1 {
            self.print_cstr(" preloaded fonts")?;
        } else {
            self.print_cstr(" preloaded font")?;
        }

        // hyphenation exceptions
        w.int(self.hyph_count);
        if self.hyph_next <= HYPH_PRIME {
            self.hyph_next = HYPH_SIZE as i32;
        }
        w.int(self.hyph_next);
        let compact_mem_top = self.mem.len() as i32 - 1;
        for k in 0..=HYPH_SIZE {
            if self.hyph_word[k] != 0 {
                w.int(k as i32 + 65536 * i32::from(self.hyph_link[k]));
                w.int(self.hyph_word[k]);
                let mut list = self.hyph_list[k];
                if (self.hi_mem_min..=compact_mem_top).contains(&list) {
                    list += C_MEM_TOP - compact_mem_top;
                }
                w.int(list);
            }
        }

        self.print_ln();
        self.print_int(self.hyph_count)?;
        if self.hyph_count != 1 {
            self.print_cstr(" hyphenation exceptions")?;
        } else {
            self.print_cstr(" hyphenation exception")?;
        }

        if self.trie_not_ready {
            self.init_trie()?;
        }
        w.int(self.fmt_trie_max);
        w.int(self.hyph_start);
        w.ints(&self.trie_trl[..(self.fmt_trie_max + 1) as usize]);
        w.ints(&self.trie_tro[..(self.fmt_trie_max + 1) as usize]);
        w.u16s(&self.trie_trc[..(self.fmt_trie_max + 1) as usize]);
        w.int(self.fmt_max_hyph_char);
        w.int(self.trie_op_ptr);
        w.i16s(&self.hyf_distance[1..(self.trie_op_ptr + 1) as usize]);
        w.i16s(&self.hyf_num[1..(self.trie_op_ptr + 1) as usize]);
        w.u16s(&self.hyf_next[1..(self.trie_op_ptr + 1) as usize]);

        self.print_nl_cstr("Hyphenation trie of length ")?;
        self.print_int(self.fmt_trie_max)?;
        self.print_cstr(" has ")?;
        self.print_int(self.trie_op_ptr)?;
        if self.trie_op_ptr != 1 {
            self.print_cstr(" ops")?;
        } else {
            self.print_cstr(" op")?;
        }
        self.print_cstr(" out of ")?;
        self.print_int(TRIE_OP_SIZE)?;

        for k in (0..=BIGGEST_LANG).rev() {
            if self.trie_used[k as usize] > 0 {
                self.print_nl_cstr("  ")?;
                self.print_int(self.trie_used[k as usize])?;
                self.print_cstr(" for language ")?;
                self.print_int(k)?;
                w.int(k);
                w.int(self.trie_used[k as usize]);
            }
        }

        w.int(FORMAT_FOOTER_MAGIC);
        self.set_int_par(INT_PAR_TRACING_STATS, 0)?;
        self.fmt_bytes = w.bytes;
        Ok(())
    }

    /// C `pseudo_close` — pops one pseudo-input-file node off
    /// `pseudo_files`, freeing its line buffer.
    pub(crate) fn pseudo_close(&mut self) -> Result<(), TeXMouthError> {
        let p = self.mem[(self.pseudo_files + 1) as usize].s1;
        self.pseudo_files = self.mem[self.pseudo_files as usize].s1;
        self.free_node(p, self.mem[p as usize].s0)?;
        self.free_node(p + 1, self.mem[(p + 1) as usize].s0)?;
        Ok(())
    }

    // === Font-array bridge ===

    /// Rebuilds the C `font_info` flat array plus the 22 parallel font
    /// arrays from `font_table` (`read_font_info` in reverse).
    fn build_fmt_font_arrays(&self) -> Result<(Vec<MemoryWord>, FmtFontArrays), TeXMouthError> {
        let font_ptr = self.font_table.len().saturating_sub(1) as i32;
        let n = (font_ptr + 1) as usize;
        let mut arrays = FmtFontArrays {
            i32_arrays: Vec::with_capacity(10),
            font_check: Vec::with_capacity(n),
            font_bc: vec![0; n],
            font_ec: vec![0; n],
            char_base: vec![0; n],
            width_base: vec![0; n],
            height_base: vec![0; n],
            depth_base: vec![0; n],
            italic_base: vec![0; n],
            lig_kern_base: vec![0; n],
            kern_base: vec![0; n],
            exten_base: vec![0; n],
            param_base: vec![0; n],
            font_glue: vec![0; n],
            bchar_label: vec![0; n],
            font_bchar: vec![0; n],
            font_false_bchar: vec![0; n],
        };
        let mut font_size = vec![0; n];
        let mut font_dsize = vec![0; n];
        let mut font_params = vec![0; n];
        let mut hyphen_char = vec![0; n];
        let mut skew_char = vec![0; n];
        let mut font_name = vec![0; n];
        let mut font_area = vec![0; n];
        let mut font_info: Vec<MemoryWord> = vec![MemoryWord::default(); 7];

        // C's font_info[0..7] holds header defaults; index 7 is where the
        // first char_info can begin (non_address sentinel is word 0 of the
        // first lig/kern table when present).
        for f in 0..n {
            let rec = &self.font_table[f];
            let mut check = MemoryWord::default();
            check.set_b16_s3(((rec.checksum >> 24) & 0xFF) as u16);
            check.set_b16_s2(((rec.checksum >> 16) & 0xFF) as u16);
            check.set_b16_s1(((rec.checksum >> 8) & 0xFF) as u16);
            check.set_b16_s0((rec.checksum & 0xFF) as u16);
            arrays.font_check.push(check);
            font_size[f] = rec.size;
            font_dsize[f] = rec.dsize;
            font_params[f] = rec.params.len() as i32;
            hyphen_char[f] = rec.hyphen_char;
            skew_char[f] = rec.skew_char;
            font_name[f] = if rec.name.is_empty() {
                0
            } else {
                // The name str_number must already exist in the pool; look
                // it up without interning (fonts were interned at load).
                self.find_pool_string(&rec.name)
            };
            font_area[f] = if rec.area.is_empty() {
                0
            } else {
                self.find_pool_string(&rec.area)
            };
            arrays.font_bc[f] = rec.bc.min(0xFFFF) as u16;
            arrays.font_ec[f] = rec.ec.min(0xFFFF) as u16;
            arrays.font_glue[f] = rec.glue;
            arrays.bchar_label[f] = rec.bchar_label;
            arrays.font_bchar[f] = rec.bchar.clamp(0, TOO_BIG_CHAR) as u16;
            arrays.font_false_bchar[f] = rec.false_bchar.clamp(0, TOO_BIG_CHAR) as u16;

            // char_info block: dedupe metric values into index tables in
            // first-seen order over bc..=ec.
            let mut widths: Vec<i32> = Vec::new();
            let mut heights: Vec<i32> = Vec::new();
            let mut depths: Vec<i32> = Vec::new();
            let mut italics: Vec<i32> = Vec::new();
            arrays.char_base[f] = font_info.len() as i32 - rec.bc as i32;
            for c in rec.bc..=rec.ec {
                let m = self
                    .font_char_metrics
                    .get(&(f as i32, c))
                    .copied()
                    .unwrap_or_default();
                let wi = index_of(&mut widths, m.width);
                let hi = index_of(&mut heights, m.height);
                let di = index_of(&mut depths, m.depth);
                let ii = index_of(&mut italics, m.italic);
                let tag = rec.char_tags.get(&c).copied().unwrap_or(0);
                let rem = rec.char_remainders.get(&c).copied().unwrap_or(0);
                let mut w = MemoryWord::default();
                w.set_b16_s3(wi as u16);
                w.set_b16_s2(((hi << 4) | di) as u16);
                w.set_b16_s1(((ii << 2) | i32::from(tag)) as u16);
                w.set_b16_s0(rem as u16);
                font_info.push(w);
            }
            arrays.width_base[f] = font_info.len() as i32;
            for &v in &widths {
                font_info.push(MemoryWord { s0: 0, s1: v });
            }
            arrays.height_base[f] = font_info.len() as i32;
            for &v in &heights {
                font_info.push(MemoryWord { s0: 0, s1: v });
            }
            arrays.depth_base[f] = font_info.len() as i32;
            for &v in &depths {
                font_info.push(MemoryWord { s0: 0, s1: v });
            }
            arrays.italic_base[f] = font_info.len() as i32;
            for &v in &italics {
                font_info.push(MemoryWord { s0: 0, s1: v });
            }
            arrays.lig_kern_base[f] = font_info.len() as i32;
            for insn in &rec.lig_kern {
                let mut w = MemoryWord::default();
                w.set_b16_s3(u16::from(insn.skip));
                w.set_b16_s2(insn.next_char as u16);
                w.set_b16_s1(u16::from(insn.op));
                w.set_b16_s0(insn.remainder as u16);
                font_info.push(w);
            }
            arrays.kern_base[f] = font_info.len() as i32;
            for &v in &rec.kerns {
                font_info.push(MemoryWord { s0: 0, s1: v });
            }
            arrays.exten_base[f] = font_info.len() as i32;
            let mut recipes: Vec<(&i32, &[i32; 4])> = rec.exten_recipes.iter().collect();
            recipes.sort_by_key(|(k, _)| **k);
            for (_, r) in recipes {
                let mut w = MemoryWord::default();
                w.set_b16_s3(r[0] as u16);
                w.set_b16_s2(r[1] as u16);
                w.set_b16_s1(r[2] as u16);
                w.set_b16_s0(r[3] as u16);
                font_info.push(w);
            }
            arrays.param_base[f] = font_info.len() as i32 - 1;
            for &v in &rec.params {
                font_info.push(MemoryWord { s0: 0, s1: v });
            }
        }
        arrays.i32_arrays = vec![
            font_size,
            font_dsize,
            font_params,
            hyphen_char,
            skew_char,
            font_name,
            font_area,
        ];
        Ok((font_info, arrays))
    }

    /// Finds an existing pool string matching `units` (`search_string`).
    fn find_pool_string(&self, units: &[i32]) -> i32 {
        for s in TOO_BIG_CHAR..self.str_ptr {
            let start = self
                .str_start
                .get((s - TOO_BIG_CHAR) as usize)
                .copied()
                .unwrap_or(0) as usize;
            let end = self
                .str_start
                .get((s - TOO_BIG_CHAR + 1) as usize)
                .copied()
                .unwrap_or(self.pool_ptr) as usize;
            let len = end.saturating_sub(start);
            if len == units.len()
                && self.str_pool[start..end]
                    .iter()
                    .copied()
                    .eq(units.iter().copied())
            {
                return s;
            }
        }
        0
    }

    // === `load_fmt_file` ===

    /// C `load_fmt_file` — restores engine state from an upstream big-endian
    /// format image, populating the compact runtime tables and the
    /// C-layout shadow structures alike.
    pub(crate) fn load_fmt_file(&mut self, data: &[u8]) -> Result<(), TeXMouthError> {
        let mut r = FmtReader::new(data);
        let bad = || TeXMouthError::BadFormatFile;

        let x = r.int()?;
        if x != FORMAT_HEADER_MAGIC {
            return Err(bad());
        }
        let x = r.int()?;
        if x != FORMAT_SERIAL {
            return Err(bad());
        }
        self.hash_high = r.int()?;
        if self.hash_high < 0 || self.hash_high > C_HASH_EXTRA {
            return Err(bad());
        }
        if self.hash_extra < self.hash_high {
            self.hash_extra = self.hash_high;
        }
        self.eqtb_top = C_EQTB_SIZE + self.hash_extra;
        self.hash_top = if self.hash_extra == 0 {
            C_UNDEFINED_CONTROL_SEQUENCE
        } else {
            self.eqtb_top
        };
        self.max_reg_num = 32767;
        self.max_reg_help_line = "A register number must be between 0 and 32767.".to_string();

        let x = r.int()?;
        if x != C_MEM_TOP {
            return Err(bad());
        }
        let x = r.int()?;
        if x != C_EQTB_SIZE {
            return Err(bad());
        }
        let x = r.int()?;
        if x != C_HASH_PRIME {
            return Err(bad());
        }
        let x = r.int()?;
        if x != HYPH_PRIME {
            return Err(bad());
        }

        // string pool
        let pool_ptr = r.int()?;
        if pool_ptr < 0 {
            return Err(bad());
        }
        self.pool_ptr = pool_ptr;
        let str_ptr = r.int()?;
        if str_ptr < 0 {
            return Err(bad());
        }
        self.str_ptr = str_ptr;
        let n_starts = (str_ptr - TOO_BIG_CHAR + 1).max(0) as usize;
        self.str_start = r.ints(n_starts)?;
        for &v in &self.str_start {
            if v < 0 || v > pool_ptr {
                return Err(bad());
            }
        }
        self.str_pool = r
            .u16s(pool_ptr as usize)?
            .iter()
            .map(|&u| i32::from(u))
            .collect();
        self.init_str_ptr = str_ptr;
        self.init_pool_ptr = pool_ptr;
        self.pool_size = pool_ptr.max(self.pool_size);

        // mem
        let x = r.int()?;
        if !(1019..=C_MEM_TOP - HI_MEM_STAT_USAGE_CHECK).contains(&x) {
            return Err(bad());
        }
        self.lo_mem_max = x;
        let x = r.int()?;
        if !(20..=self.lo_mem_max).contains(&x) {
            return Err(bad());
        }
        self.rover = x;
        self.mem = vec![MemoryWord::default(); (C_MEM_TOP + 1) as usize];
        for k in 0..=SA_INTER_CHAR_VAL {
            let x = r.int()?;
            if !(MIN_HALFWORD..=self.lo_mem_max).contains(&x) {
                return Err(bad());
            }
            self.sa_root[k as usize] = x;
        }
        let mut p = 0i32;
        let mut q = self.rover;
        loop {
            let words = r.words((q + 2 - p) as usize)?;
            self.mem[p as usize..(q + 2) as usize].copy_from_slice(&words);
            p = q + self.mem[q as usize].s0;
            if p > self.lo_mem_max
                || (q >= self.mem[(q + 1) as usize].s1
                    && self.mem[(q + 1) as usize].s1 != self.rover)
            {
                return Err(bad());
            }
            q = self.mem[(q + 1) as usize].s1;
            if q == self.rover {
                break;
            }
        }
        let words = r.words((self.lo_mem_max + 1 - p) as usize)?;
        self.mem[p as usize..(self.lo_mem_max + 1) as usize].copy_from_slice(&words);
        let x = r.int()?;
        if !(self.lo_mem_max + 1..=self.pre_adjust_head()).contains(&x) {
            return Err(bad());
        }
        self.hi_mem_min = x;
        let x = r.int()?;
        if !(MIN_HALFWORD..=C_MEM_TOP).contains(&x) {
            return Err(bad());
        }
        self.avail = x;
        self.mem_end = C_MEM_TOP;
        let words = r.words((self.mem_end + 1 - self.hi_mem_min) as usize)?;
        self.mem[self.hi_mem_min as usize..(self.mem_end + 1) as usize].copy_from_slice(&words);
        self.var_used = r.int()?;
        self.dyn_used = r.int()?;

        // eqtb (compressed)
        let mut e = vec![
            Self::fmt_eqtb_word(LEVEL_ZERO, CommandCode::UndefinedCs as u8, TEX_NULL);
            (C_EQTB_SIZE + 1 + self.hash_high) as usize
        ];
        let mut k = C_ACTIVE_BASE;
        loop {
            let x = r.int()?;
            if x < 1 || k + x > C_EQTB_SIZE + 1 {
                return Err(bad());
            }
            let words = r.words(x as usize)?;
            e[k as usize..(k + x) as usize].copy_from_slice(&words);
            k += x;
            let x = r.int()?;
            if x < 0 || k + x > C_EQTB_SIZE + 1 {
                return Err(bad());
            }
            for j in k..(k + x) {
                e[j as usize] = e[(k - 1) as usize];
            }
            k += x;
            if k > C_EQTB_SIZE {
                break;
            }
        }
        if self.hash_high > 0 {
            let words = r.words(self.hash_high as usize)?;
            e[(C_EQTB_SIZE + 1) as usize..(C_EQTB_SIZE + 1 + self.hash_high) as usize]
                .copy_from_slice(&words);
        }
        let x = r.int()?;
        if !(C_HASH_BASE..=self.hash_top).contains(&x) {
            return Err(bad());
        }
        self.fmt_par_loc = x;
        let x = r.int()?;
        if !(C_HASH_BASE..=self.hash_top).contains(&x) {
            return Err(bad());
        }
        self.fmt_write_loc = x;

        // prim table
        for p in 0..=C_PRIM_SIZE {
            self.prim[p as usize] = r.b32()?;
        }

        // control sequences
        let x = r.int()?;
        if !(C_HASH_BASE..=C_FROZEN_CONTROL_SEQUENCE).contains(&x) {
            return Err(bad());
        }
        self.hash_used = x;
        let mut p = C_HASH_BASE - 1;
        loop {
            let x = r.int()?;
            if !(p + 1..=self.hash_used).contains(&x) {
                return Err(bad());
            }
            p = x;
            self.fmt_hash.insert(p, r.b32()?);
            if p == self.hash_used {
                break;
            }
        }
        for pp in (self.hash_used + 1)..C_UNDEFINED_CONTROL_SEQUENCE {
            let t = r.b32()?;
            if t.rh != 0 || t.lh != 0 {
                self.fmt_hash.insert(pp, t);
            }
        }
        if self.hash_high > 0 {
            for pp in (C_EQTB_SIZE + 1)..=(C_EQTB_SIZE + self.hash_high) {
                let t = r.b32()?;
                if t.rh != 0 || t.lh != 0 {
                    self.fmt_hash.insert(pp, t);
                }
            }
        }
        self.cs_count = r.int()?;

        // fonts
        let x = r.int()?;
        if x < 7 {
            return Err(bad());
        }
        self.fmem_ptr = x;
        let font_info = r.words(x as usize)?;
        let x = r.int()?;
        if x < FONT_BASE {
            return Err(bad());
        }
        if x > FONT_BASE + C_MAX_FONT_MAX {
            return Err(bad());
        }
        let font_ptr = x;
        let nf = (font_ptr + 1) as usize;
        let font_check = r.words(nf)?;
        let font_size = r.ints(nf)?;
        let font_dsize = r.ints(nf)?;
        let font_params = r.ints(nf)?;
        for &v in &font_params {
            if !(MIN_HALFWORD..=MAX_HALFWORD).contains(&v) {
                return Err(bad());
            }
        }
        let hyphen_char = r.ints(nf)?;
        let skew_char = r.ints(nf)?;
        let font_name = r.ints(nf)?;
        let font_area = r.ints(nf)?;
        for &v in font_name.iter().chain(font_area.iter()) {
            if v > str_ptr {
                return Err(bad());
            }
        }
        let font_bc = r.u16s(nf)?;
        let font_ec = r.u16s(nf)?;
        let char_base = r.ints(nf)?;
        let width_base = r.ints(nf)?;
        let height_base = r.ints(nf)?;
        let depth_base = r.ints(nf)?;
        let italic_base = r.ints(nf)?;
        let lig_kern_base = r.ints(nf)?;
        let kern_base = r.ints(nf)?;
        let exten_base = r.ints(nf)?;
        let param_base = r.ints(nf)?;
        let font_glue = r.ints(nf)?;
        for &v in &font_glue {
            if !(MIN_HALFWORD..=self.lo_mem_max).contains(&v) {
                return Err(bad());
            }
        }
        let bchar_label = r.ints(nf)?;
        for &v in &bchar_label {
            if !(0..self.fmem_ptr).contains(&v) {
                return Err(bad());
            }
        }
        let font_bchar = r.u16s(nf)?;
        let font_false_bchar = r.u16s(nf)?;

        self.unpack_fmt_fonts(
            &font_info,
            font_ptr,
            &font_check,
            &font_size,
            &font_dsize,
            &font_params,
            &hyphen_char,
            &skew_char,
            &font_name,
            &font_area,
            &font_bc,
            &font_ec,
            &char_base,
            &width_base,
            &height_base,
            &depth_base,
            &italic_base,
            &lig_kern_base,
            &kern_base,
            &exten_base,
            &param_base,
            &font_glue,
            &bchar_label,
            &font_bchar,
            &font_false_bchar,
        )?;

        // hyphenation exceptions
        let x = r.int()?;
        if x < 0 {
            return Err(bad());
        }
        if x > HYPH_SIZE as i32 {
            return Err(bad());
        }
        self.hyph_count = x;
        let x = r.int()?;
        if x < HYPH_PRIME {
            return Err(bad());
        }
        if x > HYPH_SIZE as i32 {
            return Err(bad());
        }
        self.hyph_next = x;
        self.hyph_word = vec![0; HYPH_SIZE + 1];
        self.hyph_list = vec![TEX_NULL; HYPH_SIZE + 1];
        self.hyph_link = vec![0; HYPH_SIZE + 1];
        self.hyph_exceptions.clear();
        let mut j = 0i32;
        for _ in 1..=self.hyph_count {
            j = r.int()?;
            if j < 0 {
                return Err(bad());
            }
            let next;
            if j > 65535 {
                next = j / 65536;
                j -= next * 65536;
            } else {
                next = 0;
            }
            if j as usize >= HYPH_SIZE || next as usize > HYPH_SIZE {
                return Err(bad());
            }
            self.hyph_next = next;
            self.hyph_link[j as usize] = next as u16;
            let x = r.int()?;
            if !(0..=str_ptr).contains(&x) {
                return Err(bad());
            }
            self.hyph_word[j as usize] = x;
            let x = r.int()?;
            if !(MIN_HALFWORD..=MAX_HALFWORD).contains(&x) {
                return Err(bad());
            }
            self.hyph_list[j as usize] = x;
            // Mirror into the runtime exception map.
            let key = self
                .string_units(self.hyph_word[j as usize])
                .unwrap_or(&[])
                .to_vec();
            let mut positions = Vec::new();
            let mut node = x;
            while node != TEX_NULL {
                positions.push(self.mem[node as usize].s0);
                node = self.mem[node as usize].s1;
            }
            self.hyph_exceptions.insert(key, positions);
        }
        j += 1;
        if j < HYPH_PRIME {
            j = HYPH_PRIME;
        }
        self.hyph_next = j;
        if self.hyph_next >= HYPH_SIZE as i32 {
            self.hyph_next = HYPH_PRIME;
        } else if self.hyph_next >= HYPH_PRIME {
            self.hyph_next += 1;
        }

        // packed trie
        let x = r.int()?;
        if x < 0 {
            return Err(bad());
        }
        self.fmt_trie_max = x;
        let j = x;
        let x = r.int()?;
        if !(0..=j).contains(&x) {
            return Err(bad());
        }
        self.hyph_start = x;
        self.trie_trl = r.ints((j + 1) as usize)?;
        self.trie_tro = r.ints((j + 1) as usize)?;
        self.trie_trc = r.u16s((j + 1) as usize)?;
        self.fmt_max_hyph_char = r.int()?;
        let x = r.int()?;
        if !(0..=TRIE_OP_SIZE).contains(&x) {
            return Err(bad());
        }
        self.trie_op_ptr = x;
        self.hyf_distance = vec![0; (TRIE_OP_SIZE + 1) as usize];
        self.hyf_num = vec![0; (TRIE_OP_SIZE + 1) as usize];
        self.hyf_next = vec![0; (TRIE_OP_SIZE + 1) as usize];
        let dist = r.i16s(x as usize)?;
        self.hyf_distance[1..(x + 1) as usize].copy_from_slice(&dist);
        let num = r.i16s(x as usize)?;
        self.hyf_num[1..(x + 1) as usize].copy_from_slice(&num);
        let nxt = r.u16s(x as usize)?;
        for (i, &v) in nxt.iter().enumerate() {
            if v > MAX_TRIE_OP as u16 {
                return Err(bad());
            }
            self.hyf_next[1 + i] = v;
        }
        for k in 0..=BIGGEST_LANG {
            self.trie_used[k as usize] = 0;
        }
        let mut k = BIGGEST_LANG + 1;
        let mut j = x;
        while j > 0 {
            let x = r.int()?;
            if !(0..k).contains(&x) {
                return Err(bad());
            }
            k = x;
            let x = r.int()?;
            if !(1..=j).contains(&x) {
                return Err(bad());
            }
            self.trie_used[k as usize] = x;
            j -= x;
            self.op_start[k as usize] = j;
        }
        self.trie_not_ready = false;
        self.rebuild_hyph_patterns_from_trie();

        let x = r.int()?;
        if x != FORMAT_FOOTER_MAGIC {
            return Err(bad());
        }

        // eqtb image → compact runtime tables + cs_table.
        self.unpack_fmt_eqtb(&e)?;
        // `par_loc`/`write_loc` are compact `cs_table` ids at runtime; the
        // dumped values were C eqtb positions resolved through the rebuilt
        // table by name.
        if let Some(cs) = self.cs_table.lookup_str("par") {
            self.par_loc = cs;
            self.par_token = CS_TOKEN_FLAG + cs;
        }
        if let Some(cs) = self.cs_table.lookup_str("write") {
            self.write_loc = cs;
        }

        // List-state fields initialized by `new()`/`initialize` to the
        // stat heads must be re-pointed: `mem` was resized to the C layout
        // above, moving `contrib_head`/`page_head` to format coordinates.
        // (In C these never change because `MEM_TOP` is fixed.)
        self.head = self.contrib_head();
        self.tail = self.contrib_head();
        self.page_tail = self.page_head();
        Ok(())
    }

    /// Rebuilds `hyph_patterns` from the packed trie after `load_fmt_file`
    /// (inverse of `init_trie`'s packing for the runtime map mirror).
    ///
    /// In the packed layout each occupied position `p` is an edge of the
    /// node based at `p - trie_trc[p]`, labelled `trie_trc[p]`; language
    /// `l`'s pattern family is rooted at `trie_trl[l + 1]` (the `trie_l[0]`
    /// family packs at position 1).
    fn rebuild_hyph_patterns_from_trie(&mut self) {
        self.hyph_patterns.clear();
        // One pass: parent base → sorted (char, position) edges.
        let mut edges: HashMap<i32, Vec<(i32, i32)>> = HashMap::new();
        for p in 1..=self.fmt_trie_max.min(self.trie_trc.len() as i32 - 1) {
            let pu = p as usize;
            let occupied = self.trie_trl[pu] > 0 || self.trie_tro[pu] != MIN_TRIE_OP;
            if !occupied {
                continue;
            }
            let parent = p - i32::from(self.trie_trc[pu]);
            edges
                .entry(parent)
                .or_default()
                .push((i32::from(self.trie_trc[pu]), p));
        }
        for children in edges.values_mut() {
            children.sort_unstable();
        }
        for lang in 0..=BIGGEST_LANG {
            // A language `l`'s pattern family is rooted at position `l + 1`
            // only when that position is actually the language node — its
            // edge label must be `l` (C relies on edge-char matching during
            // the hyphenation walk; positions `l + 1` that are ordinary
            // pattern nodes must not be mistaken for language roots).
            if self.trie_trc.get((lang + 1) as usize).copied().unwrap_or(0) != lang as u16 {
                continue;
            }
            let base = self.trie_trl.get((lang + 1) as usize).copied().unwrap_or(0);
            if base <= 0 {
                continue;
            }
            let mut path: Vec<i32> = Vec::new();
            self.unpack_trie_dfs(&edges, base, lang, &mut path);
        }
    }

    /// DFS over the packed trie from family base `z`, reconstructing each
    /// terminating pattern's letters and digit placements.
    fn unpack_trie_dfs(
        &mut self,
        edges: &HashMap<i32, Vec<(i32, i32)>>,
        z: i32,
        lang: i32,
        path: &mut Vec<i32>,
    ) {
        let Some(children) = edges.get(&z) else {
            return;
        };
        for &(c, p) in children {
            path.push(c);
            let op = self.trie_tro[p as usize];
            if op != MIN_TRIE_OP {
                // Decode the op chain into gap→digit placements. `d` is the
                // distance back from the pattern end.
                let mut digits = vec![0i32; path.len() + 1];
                let mut op = op as usize;
                loop {
                    let d = i32::from(self.hyf_distance[op]);
                    let n = i32::from(self.hyf_num[op]);
                    let gap = path.len() as i32 - d;
                    if (0..digits.len() as i32).contains(&gap) {
                        digits[gap as usize] = n;
                    }
                    let next = self.hyf_next[op];
                    if next == MIN_TRIE_OP as u16 || next == 0 {
                        break;
                    }
                    op = usize::from(next);
                }
                // Re-interleave digit characters at gap positions to match
                // the map mirror's (letters+digits, aligned digits) form:
                // a nonzero gap `g` emits a digit char before letter `g`.
                let mut pattern: Vec<i32> = Vec::new();
                let mut dvec: Vec<i32> = Vec::new();
                for g in 0..=path.len() {
                    if digits[g] != 0 {
                        pattern.push(i32::from(b'0') + digits[g]);
                        dvec.push(digits[g]);
                    }
                    if g < path.len() {
                        pattern.push(path[g]);
                        dvec.push(0);
                    }
                }
                let mut key = pattern;
                key.push(lang);
                self.hyph_patterns.insert(key, dvec);
            }
            let child = self.trie_trl[p as usize];
            if child > 0 {
                self.unpack_trie_dfs(edges, child, lang, path);
            }
            path.pop();
        }
    }

    /// Translates a loaded C-layout eqtb image into the compact runtime
    /// tables and `cs_table` (inverse of `materialize_fmt_eqtb`).
    fn unpack_fmt_eqtb(&mut self, e: &[MemoryWord]) -> Result<(), TeXMouthError> {
        // Register/parameter regions.
        let copy_region = |state: &mut Self, lo_c: i32, hi_c: i32, e: &[MemoryWord]| {
            for cp in lo_c..=hi_c {
                let target = Self::c_eqtb_pos_to_compact(cp) as usize;
                if target < state.eqtb.len() && (cp as usize) < e.len() {
                    state.eqtb[target] = e[cp as usize];
                }
            }
        };
        copy_region(self, C_GLUE_BASE, C_SKIP_BASE + 255, e);
        copy_region(self, C_MU_SKIP_BASE, C_LOCAL_BASE - 1, e);
        copy_region(self, C_LOCAL_BASE, C_LOCAL_BASE + C_NUM_LOCALS - 1, e);
        copy_region(self, C_TOKS_BASE, C_TOKS_BASE + 255, e);
        copy_region(
            self,
            C_ETEX_PEN_BASE,
            C_ETEX_PEN_BASE + C_NUM_ETEX_PENALTIES - 1,
            e,
        );
        copy_region(self, C_CUR_FONT_LOC, C_CUR_FONT_LOC, e);
        copy_region(
            self,
            C_MATH_FONT_BASE,
            C_MATH_FONT_BASE + C_NUMBER_MATH_FONTS - 1,
            e,
        );
        copy_region(self, C_INT_BASE, C_INT_BASE + C_INT_PARS - 1, e);
        copy_region(self, C_COUNT_BASE, C_COUNT_BASE + 255, e);
        copy_region(self, C_DIMEN_BASE, C_EQTB_SIZE, e);

        // Character code tables.
        for c in 0..C_NUMBER_USVS {
            let cu = c as u32;
            let w = e[(C_CAT_CODE_BASE + c) as usize];
            self.set_cat_code(cu, w.s1.clamp(0, 15) as u8);
            if (cu as usize) < self.cat_code_levels.len() {
                self.cat_code_levels[cu as usize] = w.b16_s0();
            }
            for (table, base) in [
                (CODE_TABLE_LC, C_LC_CODE_BASE),
                (CODE_TABLE_UC, C_UC_CODE_BASE),
                (CODE_TABLE_SF, C_SF_CODE_BASE),
                (CODE_TABLE_MATH, C_MATH_CODE_BASE),
                (CODE_TABLE_DEL, C_DEL_CODE_BASE),
            ] {
                let w = e[(base + c) as usize];
                let default = self.default_code_table_value_pub(table, cu);
                if w.s1 != default || w.b16_s0() != LEVEL_ONE {
                    self.code_tables[table as usize].insert(cu, w);
                }
            }
        }

        // Control sequences: rebuild cs_table from the hash image.
        let mut positions: Vec<i32> = self.fmt_hash.keys().copied().collect();
        positions.sort_unstable();
        for pos in positions {
            let t = self.fmt_hash[&pos];
            if t.rh == 0 {
                continue;
            }
            if (C_SINGLE_BASE..C_HASH_BASE).contains(&pos) {
                continue; // single-char cs's are implicit
            }
            let name = self.string_units(t.rh).unwrap_or(&[]).to_vec();
            let w = e.get(pos as usize).copied().unwrap_or_default();
            let command = w.b16_s1() as u8;
            let chr = Self::c_eqtb_pos_to_compact(w.s1);
            let level = w.b16_s0();
            let cs = self.cs_table.define_units(&name, command, chr);
            let _ = self.cs_table.restore(cs, command, chr, level);
        }
        self.unpack_fmt_eqtb_shadow(e);
        Ok(())
    }

    /// Stores the loaded eqtb words back into `fmt_eqtb` for re-dumps.
    fn unpack_fmt_eqtb_shadow(&mut self, e: &[MemoryWord]) {
        self.fmt_eqtb.clear();
        for (pos, w) in e.iter().enumerate() {
            if w.s0 != 0 || w.s1 != 0 {
                self.fmt_eqtb.insert(pos as i32, *w);
            }
        }
    }

    /// Public wrapper for default code-table values.
    pub(crate) fn default_code_table_value_pub(&self, table: u8, c: u32) -> i32 {
        self.default_code_table_value(table, c)
    }

    /// Rebuilds `font_table`/`font_char_metrics` from the loaded parallel
    /// font arrays (inverse of `build_fmt_font_arrays`).
    #[allow(clippy::too_many_arguments)]
    fn unpack_fmt_fonts(
        &mut self,
        font_info: &[MemoryWord],
        font_ptr: i32,
        font_check: &[MemoryWord],
        font_size: &[i32],
        font_dsize: &[i32],
        font_params: &[i32],
        hyphen_char: &[i32],
        skew_char: &[i32],
        font_name: &[i32],
        font_area: &[i32],
        font_bc: &[u16],
        font_ec: &[u16],
        char_base: &[i32],
        width_base: &[i32],
        height_base: &[i32],
        depth_base: &[i32],
        italic_base: &[i32],
        lig_kern_base: &[i32],
        kern_base: &[i32],
        exten_base: &[i32],
        param_base: &[i32],
        font_glue: &[i32],
        bchar_label: &[i32],
        font_bchar: &[u16],
        font_false_bchar: &[u16],
    ) -> Result<(), TeXMouthError> {
        self.font_table.clear();
        self.font_char_metrics.clear();
        for f in 0..=font_ptr as usize {
            let mut rec = FontRecord {
                checksum: (u32::from(font_check[f].b16_s3()) << 24)
                    | (u32::from(font_check[f].b16_s2()) << 16)
                    | (u32::from(font_check[f].b16_s1()) << 8)
                    | u32::from(font_check[f].b16_s0()),
                size: font_size[f],
                dsize: font_dsize[f],
                hyphen_char: hyphen_char[f],
                skew_char: skew_char[f],
                name: if font_name[f] > 0 {
                    self.string_units(font_name[f]).unwrap_or(&[]).to_vec()
                } else {
                    Vec::new()
                },
                area: if font_area[f] > 0 {
                    self.string_units(font_area[f]).unwrap_or(&[]).to_vec()
                } else {
                    Vec::new()
                },
                bc: u32::from(font_bc[f]),
                ec: u32::from(font_ec[f]),
                glue: font_glue[f],
                bchar_label: bchar_label[f],
                bchar: i32::from(font_bchar[f]),
                false_bchar: i32::from(font_false_bchar[f]),
                ..Default::default()
            };

            // char_info words → metrics + tags.
            for c in rec.bc..=rec.ec {
                let idx = (c as i32 + char_base[f]) as usize;
                let w = font_info.get(idx).copied().unwrap_or_default();
                let wi = i32::from(w.b16_s3());
                let hi = i32::from(w.b16_s2()) >> 4;
                let di = i32::from(w.b16_s2()) & 0xF;
                let ii = i32::from(w.b16_s1()) >> 2;
                let tag = (i32::from(w.b16_s1()) & 3) as u8;
                let rem = i32::from(w.b16_s0());
                let metric = FontCharMetric {
                    width: font_info
                        .get((wi + width_base[f]) as usize)
                        .map(|w| w.s1)
                        .unwrap_or(0),
                    height: font_info
                        .get((hi + height_base[f]) as usize)
                        .map(|w| w.s1)
                        .unwrap_or(0),
                    depth: font_info
                        .get((di + depth_base[f]) as usize)
                        .map(|w| w.s1)
                        .unwrap_or(0),
                    italic: font_info
                        .get((ii + italic_base[f]) as usize)
                        .map(|w| w.s1)
                        .unwrap_or(0),
                };
                if metric.width != 0 || metric.height != 0 {
                    self.font_char_metrics.insert((f as i32, c), metric);
                }
                if tag != 0 {
                    rec.char_tags.insert(c, tag);
                }
                if rem != 0 {
                    rec.char_remainders.insert(c, rem);
                }
            }

            // lig/kern program — block ends at the next font's base or the
            // kern base.
            let lk_start = lig_kern_base[f] as usize;
            let lk_end = kern_base[f] as usize;
            for i in lk_start..lk_end {
                let w = font_info.get(i).copied().unwrap_or_default();
                rec.lig_kern.push(LigKernInsn {
                    skip: w.b16_s3() as u8,
                    next_char: i32::from(w.b16_s2()),
                    op: w.b16_s1() as u8,
                    remainder: i32::from(w.b16_s0()),
                });
            }
            let k_end = exten_base[f] as usize;
            for i in kern_base[f] as usize..k_end {
                rec.kerns.push(font_info.get(i).map(|w| w.s1).unwrap_or(0));
            }
            let x_end = param_base[f] as usize + 1;
            for i in exten_base[f] as usize..x_end {
                let w = font_info.get(i).copied().unwrap_or_default();
                rec.exten_recipes.insert(
                    (i - exten_base[f] as usize) as i32,
                    [
                        i32::from(w.b16_s3()),
                        i32::from(w.b16_s2()),
                        i32::from(w.b16_s1()),
                        i32::from(w.b16_s0()),
                    ],
                );
            }
            let n_params = font_params[f].max(0) as usize;
            for i in 0..n_params {
                rec.params.push(
                    font_info
                        .get(param_base[f] as usize + 1 + i)
                        .map(|w| w.s1)
                        .unwrap_or(0),
                );
            }
            self.font_table.push(rec);
        }
        Ok(())
    }
}

/// One font's worth of serializable parallel-array data.
struct FmtFontArrays {
    /// `font_check` words (serialized as 8-byte memory words).
    font_check: Vec<MemoryWord>,
    /// The i32-valued parallel arrays in C dump order after `font_check`.
    i32_arrays: Vec<Vec<i32>>,
    font_bc: Vec<u16>,
    font_ec: Vec<u16>,
    char_base: Vec<i32>,
    width_base: Vec<i32>,
    height_base: Vec<i32>,
    depth_base: Vec<i32>,
    italic_base: Vec<i32>,
    lig_kern_base: Vec<i32>,
    kern_base: Vec<i32>,
    exten_base: Vec<i32>,
    param_base: Vec<i32>,
    font_glue: Vec<i32>,
    bchar_label: Vec<i32>,
    font_bchar: Vec<u16>,
    font_false_bchar: Vec<u16>,
}

/// First-seen dedupe into `table`; returns the 1-based index (TFM indices
/// start at 0, so return len-1).
fn index_of(table: &mut Vec<i32>, v: i32) -> i32 {
    if let Some(i) = table.iter().position(|&x| x == v) {
        i as i32
    } else {
        table.push(v);
        (table.len() - 1) as i32
    }
}

const HI_MEM_STAT_USAGE_CHECK: i32 = 14;

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal INITEX-configured engine: runtime primitives plus the
    /// C-layout image (prim table, frozen entries, INITEX counters).
    fn initex_state() -> TeXEngineState {
        let mut state = TeXEngineState::default();
        state.install_core_primitives();
        state.in_initex_mode = true;
        state.initialize_more_variables();
        state.initialize_more_initex_variables().unwrap();
        state.initialize_primitives_fmt().unwrap();
        state
    }

    #[test]
    fn fmt_writer_emits_big_endian_words() {
        let mut w = FmtWriter::default();
        w.int(0x0102_0304);
        w.u16(0xA0B1);
        w.word(MemoryWord {
            s0: 0x1122_3344,
            s1: 0x5566_7788,
        });
        w.b32(TwoHalves {
            lh: 0x0BAD_CAFE,
            rh: -559_038_737,
        });
        assert_eq!(
            w.bytes,
            vec![
                0x01, 0x02, 0x03, 0x04, // int
                0xA0, 0xB1, // u16
                0x55, 0x66, 0x77, 0x88, // word s1 first
                0x11, 0x22, 0x33, 0x44, // word s0
                0xDE, 0xAD, 0xBE, 0xEF, // b32 rh first
                0x0B, 0xAD, 0xCA, 0xFE, // b32 lh
            ]
        );
    }

    #[test]
    fn fmt_reader_round_trips_writer_items() {
        let mut w = FmtWriter::default();
        w.int(-7);
        w.u16(0xFFFF);
        w.i16(-1234);
        w.word(MemoryWord { s0: -1, s1: 42 });
        w.b32(TwoHalves { lh: 9, rh: -9 });
        let mut r = FmtReader::new(&w.bytes);
        assert_eq!(r.int().unwrap(), -7);
        assert_eq!(r.u16().unwrap(), 0xFFFF);
        assert_eq!(r.i16().unwrap(), -1234);
        let mw = r.word().unwrap();
        assert_eq!((mw.s0, mw.s1), (-1, 42));
        let t = r.b32().unwrap();
        assert_eq!((t.lh, t.rh), (9, -9));
        assert!(r.int().is_err()); // exhausted
    }

    #[test]
    fn load_rejects_bad_magic_and_short_input() {
        let mut state = TeXEngineState::default();
        assert!(matches!(
            state.load_fmt_file(&[0, 0, 0, 0]),
            Err(TeXMouthError::BadFormatFile)
        ));
        assert!(matches!(
            state.load_fmt_file(&[]),
            Err(TeXMouthError::BadFormatFile)
        ));
    }

    #[test]
    fn store_dump_round_trips_pool_eqtb_and_control_sequences() {
        let mut a = initex_state();
        a.set_source_text("\\hyphenation{as-sert}");
        a.dispatch_x_command().unwrap();
        a.store_fmt_file().unwrap();
        assert!(!a.fmt_bytes.is_empty());

        let mut b = TeXEngineState::default();
        b.initialize_more_variables();
        b.load_fmt_file(&a.fmt_bytes).unwrap();

        assert_eq!(b.str_ptr, a.str_ptr);
        assert_eq!(b.pool_ptr, a.pool_ptr);
        assert_eq!(
            &b.str_pool[..a.pool_ptr as usize],
            &a.str_pool[..a.pool_ptr as usize]
        );
        // `format_ident` is not serialized — C's load path resets it to 0.
        assert_eq!(b.format_ident, 0);
        assert_eq!(b.prim, a.prim);
        assert_eq!(b.fmt_hash, a.fmt_hash);
        assert_eq!(b.hyph_word, a.hyph_word);
        assert_eq!(b.hyph_link, a.hyph_link);
        assert_eq!(b.hyph_count, a.hyph_count);
        assert_eq!(b.hyph_exceptions, a.hyph_exceptions);
        // Compact-table bridge: integer parameters copied through.
        assert_eq!(b.int_par(INT_PAR_MAG).unwrap(), 1000);
        // `par` resolves to a real compact cs id and token.
        let par = b.cs_table.lookup_str("par").unwrap();
        assert_eq!(b.par_loc, par);
        assert_eq!(b.par_token, CS_TOKEN_FLAG + par);
        assert!(b.mem_end == C_MEM_TOP);
    }

    #[test]
    fn store_dump_round_trips_packed_trie_and_patterns() {
        let mut a = initex_state();
        a.set_source_text("\\patterns{a1b .c2d}");
        a.dispatch_x_command().unwrap();
        a.store_fmt_file().unwrap();
        assert!(!a.trie_not_ready);
        assert!(a.fmt_trie_max > 0);

        let mut b = TeXEngineState::default();
        b.initialize_more_variables();
        b.load_fmt_file(&a.fmt_bytes).unwrap();

        assert_eq!(b.fmt_trie_max, a.fmt_trie_max);
        assert_eq!(b.hyph_start, a.hyph_start);
        // Only `0..=fmt_trie_max` is serialized; the rest of `a`'s build arrays
        // is unwritten capacity.
        let n = a.fmt_trie_max as usize + 1;
        assert_eq!(b.trie_trl.as_slice(), &a.trie_trl[..n]);
        assert_eq!(b.trie_tro.as_slice(), &a.trie_tro[..n]);
        assert_eq!(b.trie_trc.as_slice(), &a.trie_trc[..n]);
        assert_eq!(b.fmt_max_hyph_char, a.fmt_max_hyph_char);
        assert_eq!(b.hyf_distance, a.hyf_distance);
        assert_eq!(b.hyf_num, a.hyf_num);
        assert_eq!(b.hyf_next, a.hyf_next);
        assert!(!b.trie_not_ready);
        // The runtime mirror is rebuilt from the packed trie.
        assert_eq!(b.hyph_patterns, a.hyph_patterns);
        assert_eq!(b.hyph_patterns.len(), 2);
    }

    #[test]
    fn run_engine_initex_dumps_then_load_drives_a_job() {
        // INITEX pass: `\dump` produces a format image.
        let mut ini = TeXEngineState::default();
        ini.in_initex_mode = true;
        ini.register_input_source("job.tex", "\\dump");
        let history = ini
            .run_engine(
                "test.fmt",
                None,
                "job.tex",
                std::time::SystemTime::UNIX_EPOCH,
            )
            .unwrap();
        assert_eq!(history, HISTORY_SPOTLESS);
        assert!(ini.job_stopped);
        assert!(!ini.fmt_bytes.is_empty());

        // Production pass: the dumped image loads and `\end` stops cleanly.
        let mut b = TeXEngineState::default();
        b.register_input_source("job.tex", "\\end");
        let history = b
            .run_engine(
                "test.fmt",
                Some(&ini.fmt_bytes),
                "job.tex",
                std::time::SystemTime::UNIX_EPOCH,
            )
            .unwrap();
        assert_eq!(history, HISTORY_SPOTLESS);
        assert!(b.job_stopped);
        assert!(!b.trie_not_ready);
        assert_eq!(b.int_par(INT_PAR_YEAR).unwrap(), 1970);
        assert_eq!(b.int_par(INT_PAR_MONTH).unwrap(), 1);
        assert_eq!(b.int_par(INT_PAR_DAY).unwrap(), 1);
        assert_eq!(b.int_par(INT_PAR_TIME).unwrap(), 0);
    }

    #[test]
    fn civil_from_unix_matches_known_dates() {
        assert_eq!(crate::ini::civil_from_unix(0), (0, 1, 1, 1970));
        // 2000-02-29 12:34 UTC (leap day).
        assert_eq!(
            crate::ini::civil_from_unix(951_827_640),
            (12 * 60 + 34, 29, 2, 2000)
        );
        // 2024-12-31 23:59 UTC.
        assert_eq!(
            crate::ini::civil_from_unix(1_735_689_540),
            (23 * 60 + 59, 31, 12, 2024)
        );
    }

    #[test]
    fn dump_via_dispatch_emits_format_bytes() {
        let mut a = initex_state();
        a.set_source_text("\\dump");
        a.dispatch_x_command().unwrap();
        assert!(a.job_stopped);
        assert!(!a.fmt_bytes.is_empty());
        assert_eq!(&a.fmt_bytes[..4], &FORMAT_HEADER_MAGIC.to_be_bytes());
    }
}
