//! XDV/DVI shipout: the pure-Rust port of `xetex-shipout.c`.
//!
//! Emits the XDV byte stream for shipped pages: preamble/postamble,
//! `bop`/`eop` page structure, movement compression, font definitions,
//! native positioned-glyph arrays, rules, leaders, specials, and
//! TeX--XeT direction handling. All output state lives on
//! [`TeXEngineState`]; bytes accumulate in `dvi_bytes` until a driver
//! flushes them to a real sink (the per-engine replacement for C's
//! `dvi_file` handle).

use crate::scanner::{
    TeXMouthError, CS_TOKEN_FLAG, END_WRITE_ID, INSERTED, RIGHT_BRACE_TOKEN, WRITE_TEXT,
};
use crate::state::{
    tex_round, TeXEngineState, ADJUST_NODE, A_LEADERS, BEFORE, BOP, C_LEADERS, DEFINE_NATIVE_FONT,
    DIMEN_PAR_H_OFFSET, DIMEN_PAR_PDF_PAGE_HEIGHT, DIMEN_PAR_PDF_PAGE_WIDTH, DIMEN_PAR_V_OFFSET,
    DLIST, DOWN1, DVI_BUF_SIZE, EDGE_NODE, END_M_CODE, EOP, FILLL, FNT1, FNT_DEF1, FNT_NUM_0,
    FONT_FLAGS_COLORED, FONT_FLAGS_VERTICAL, GLUE_NODE, GLUE_SPEC_SIZE, GLYPH_NODE, HALF_BUF,
    HLIST_NODE, INS_NODE, INT_PAR_MAG, INT_PAR_TRACING_OUTPUT,
    INT_PAR_XETEX_INTERWORD_SPACE_SHAPING, KERN_NODE, LANGUAGE_NODE, LEFT_TO_RIGHT, LIGATURE_NODE,
    L_CODE, MARGIN_KERN_NODE, MARK_NODE, MATH_NODE, MAX_HALFWORD, MEDIUM_NODE_SIZE, MIN_HALFWORD,
    MOVEMENT_NODE_SIZE, MOV_D_FIXED, MOV_NONE_SEEN, MOV_YZ_OK, MOV_Y_HERE, MOV_Y_OK, MOV_Y_SEEN,
    MOV_Z_HERE, MOV_Z_OK, MOV_Z_SEEN, NATIVE_WORD_NODE, NATIVE_WORD_NODE_AT, NORMAL, NULL_FLAG,
    PDF_NODE, PDF_SAVE_POS_NODE, PENALTY_NODE, PIC_NODE, PIC_NODE_SIZE, POP, POST, POST_POST, PRE,
    PUSH, PUT_RULE, REVERSED, RIGHT1, RIGHT_TO_LEFT, RULE_NODE, R_CODE, SET1, SET_GLYPHS, SET_RULE,
    SET_TEXT_AND_GLYPHS, SHRINKING, SMALL_NODE_SIZE, SPACE_ADJUSTMENT, SPX_ID_BYTE, STRETCHING,
    TEX_NULL, VLIST_NODE, WHATSIT_NODE, XDV_FLAG_COLORED, XDV_FLAG_EMBOLDEN, XDV_FLAG_EXTEND,
    XDV_FLAG_SLANT, XDV_FLAG_VERTICAL, XDV_ID_BYTE, XXX1, XXX4,
};

/// One scaled point expressed as a fixed-point inch offset; C's literal
/// `4736287` = `round(0xFFFF * 72.27)`.
const ONE_INCH: i32 = 4_736_287;
/// Largest XDV byte count (`TEX_INFINITY` = `i32::MAX`).
const TEX_INFINITY: i64 = 0x7FFF_FFFF;
/// Glue-ratio saturation bound used by the movement/glue arithmetic.
const GLUE_LIMIT: f64 = 1_000_000_000.0;

/// TeX's `print_scaled`: scaled points as decimal text, no `pt` suffix.
fn format_scaled(value: i32) -> String {
    let mut s = value;
    let mut text = String::new();
    if s < 0 {
        text.push('-');
        s = -s;
    }
    text.push_str(&(s / 65536).to_string());
    text.push('.');

    s = 10 * (s % 65536) + 5;
    let mut delta: i32 = 10;
    loop {
        if delta > 65536 {
            s += 0x8000 - 50000;
        }
        text.push(char::from(b'0' + (s / 65536).clamp(0, 9) as u8));
        s = 10 * (s % 65536);
        delta *= 10;
        if s <= delta {
            break;
        }
    }
    text
}

impl TeXEngineState {
    /// Whether `q` is invisible to the interword-space merge pass
    /// (penalty/ins/mark/adjust nodes and `\openout`-family whatsits,
    /// `NODE_subtype <= LANGUAGE_NODE`).
    fn invisible_to_interword_space(&self, q: i32) -> Result<bool, TeXMouthError> {
        if q == TEX_NULL || self.is_char_node(q) {
            return Ok(false);
        }
        Ok(match self.node_type(q)? {
            PENALTY_NODE | INS_NODE | MARK_NODE | ADJUST_NODE => true,
            WHATSIT_NODE => self.node_subtype(q)? <= LANGUAGE_NODE,
            _ => false,
        })
    }

    /// Whether `q` is a native-word whatsit (`NATIVE_WORD_NODE` or
    /// `NATIVE_WORD_NODE_AT`).
    fn is_native_word_whatsit(&self, q: i32) -> Result<bool, TeXMouthError> {
        if q == TEX_NULL || self.is_char_node(q) || self.node_type(q)? != WHATSIT_NODE {
            return Ok(false);
        }
        let subtype = self.node_subtype(q)?;
        Ok(subtype == NATIVE_WORD_NODE || subtype == NATIVE_WORD_NODE_AT)
    }

    // ---- low-level byte emission ------------------------------------

    /// Appends `dvi_buf[a..=b]` to the accumulated output (`write_to_dvi`).
    fn write_to_dvi(&mut self, a: i32, b: i32) {
        let a = a as usize;
        let b = b as usize;
        self.dvi_bytes.extend_from_slice(&self.dvi_buf[a..=b]);
    }

    /// Flushes the completed half of the staging buffer (`dvi_swap`).
    fn dvi_swap(&mut self) -> Result<(), TeXMouthError> {
        if i64::from(self.dvi_ptr) > TEX_INFINITY - i64::from(self.dvi_offset) {
            self.cur_s = -2;
            return Err(self.confusion("dvi length exceeds 0x7FFFFFFF"));
        }

        if self.dvi_limit == DVI_BUF_SIZE as i32 {
            self.write_to_dvi(0, HALF_BUF as i32 - 1);
            self.dvi_limit = HALF_BUF as i32;
            self.dvi_offset += DVI_BUF_SIZE as i32;
            self.dvi_ptr = 0;
        } else {
            self.write_to_dvi(HALF_BUF as i32, DVI_BUF_SIZE as i32 - 1);
            self.dvi_limit = DVI_BUF_SIZE as i32;
        }
        self.dvi_gone += HALF_BUF as i32;
        Ok(())
    }

    /// Emits one byte into the staging buffer (`dvi_out`).
    fn dvi_out(&mut self, c: u8) -> Result<(), TeXMouthError> {
        self.dvi_buf[self.dvi_ptr as usize] = c;
        self.dvi_ptr += 1;
        if self.dvi_ptr == self.dvi_limit {
            self.dvi_swap()?;
        }
        Ok(())
    }

    /// Emits a two-byte big-endian value (`dvi_two`).
    fn dvi_two(&mut self, s: u16) -> Result<(), TeXMouthError> {
        self.dvi_out((s / 0x100) as u8)?;
        self.dvi_out((s % 0x100) as u8)
    }

    /// Emits a four-byte big-endian value with C's sign folding
    /// (`dvi_four`).
    fn dvi_four(&mut self, x: i32) -> Result<(), TeXMouthError> {
        let mut x = x;
        if x >= 0 {
            self.dvi_out((x / 0x100_0000) as u8)?;
        } else {
            x += 0x4000_0000;
            x += 0x4000_0000;
            self.dvi_out((x / 0x100_0000 + 128) as u8)?;
        }
        x %= 0x100_0000;
        self.dvi_out((x / 0x10000) as u8)?;
        x %= 0x10000;
        self.dvi_out((x / 0x100) as u8)?;
        self.dvi_out((x % 0x100) as u8)
    }

    /// Emits `pop`, reusing the byte at `l` when it is the byte just
    /// written (`dvi_pop`).
    fn dvi_pop(&mut self, l: i32) -> Result<(), TeXMouthError> {
        if l == self.dvi_offset + self.dvi_ptr && self.dvi_ptr > 0 {
            self.dvi_ptr -= 1;
            Ok(())
        } else {
            self.dvi_out(POP)
        }
    }

    // ---- font definitions --------------------------------------------

    /// Serializes a native-word node's positioned glyphs into the XDV
    /// `set_glyphs`/`set_text_and_glyphs` payload
    /// (`make_xdv_glyph_array_data`): width, glyph count, `count`
    /// `(x, y)` fixed-point pairs, then `count` glyph ids.
    fn make_xdv_glyph_array_data(&self, p: i32) -> Result<Vec<u8>, TeXMouthError> {
        let width = self.mem[(p + 1) as usize].s1;
        let info = self.native_glyph_info(p);
        let count = info.map_or(0usize, |info| info.glyphs.len());
        let mut out = Vec::with_capacity(6 + count * 10);
        out.extend_from_slice(&width.to_be_bytes());
        out.extend_from_slice(&(count as u16).to_be_bytes());
        if let Some(info) = info {
            for loc in &info.glyphs {
                out.extend_from_slice(&loc.x.to_be_bytes());
                out.extend_from_slice(&loc.y.to_be_bytes());
            }
            for loc in &info.glyphs {
                out.extend_from_slice(&loc.glyph.to_be_bytes());
            }
        }
        Ok(out)
    }

    /// Serializes a native-font definition payload (`make_font_def`):
    /// size, flags, file name, face index, then color/extend/slant/
    /// embolden parameters as flagged.
    fn make_font_def(&self, f: i32) -> Result<Vec<u8>, TeXMouthError> {
        let record = self
            .font_record(f)
            .ok_or(TeXMouthError::UnsupportedPrimitive("bad native font def"))?;
        let mut flags = 0u16;
        if record.font_flags & FONT_FLAGS_VERTICAL != 0 {
            flags |= XDV_FLAG_VERTICAL;
        }
        if record.font_flags & FONT_FLAGS_COLORED != 0 {
            flags |= XDV_FLAG_COLORED;
        }
        if record.layout_extend != 65_536 {
            flags |= XDV_FLAG_EXTEND;
        }
        if record.layout_slant != 0 {
            flags |= XDV_FLAG_SLANT;
        }
        if record.layout_embolden != 0 {
            flags |= XDV_FLAG_EMBOLDEN;
        }

        let mut out = Vec::new();
        out.extend_from_slice(&record.layout_point_size.to_be_bytes());
        out.extend_from_slice(&flags.to_be_bytes());
        let name = record.layout_filename.as_bytes();
        out.push(name.len().min(255) as u8);
        out.extend_from_slice(&name[..name.len().min(255)]);
        out.extend_from_slice(&record.layout_face_index.to_be_bytes());
        if flags & XDV_FLAG_COLORED != 0 {
            out.extend_from_slice(&record.layout_rgba.to_be_bytes());
        }
        if flags & XDV_FLAG_EXTEND != 0 {
            out.extend_from_slice(&record.layout_extend.to_be_bytes());
        }
        if flags & XDV_FLAG_SLANT != 0 {
            out.extend_from_slice(&record.layout_slant.to_be_bytes());
        }
        if flags & XDV_FLAG_EMBOLDEN != 0 {
            out.extend_from_slice(&record.layout_embolden.to_be_bytes());
        }
        Ok(out)
    }

    /// Emits a native-font definition (`dvi_native_font_def`).
    fn dvi_native_font_def(&mut self, f: i32) -> Result<(), TeXMouthError> {
        self.dvi_out(DEFINE_NATIVE_FONT)?;
        self.dvi_four(f - 1)?;
        let payload = self.make_font_def(f)?;
        for byte in payload {
            self.dvi_out(byte)?;
        }
        Ok(())
    }

    /// Emits a font definition (`dvi_font_def`): native fonts use
    /// `define_native_font`; TFM fonts use `fnt_def` with checksum,
    /// sizes, and the area/name strings (name truncated at `:`).
    fn dvi_font_def(&mut self, f: i32) -> Result<(), TeXMouthError> {
        if self.is_native_font(f) {
            return self.dvi_native_font_def(f);
        }

        if f <= 256 {
            self.dvi_out(FNT_DEF1)?;
            self.dvi_out((f - 1) as u8)?;
        } else {
            self.dvi_out(FNT_DEF1 + 1)?;
            self.dvi_out(((f - 1) / 256) as u8)?;
            self.dvi_out(((f - 1) % 256) as u8)?;
        }

        let (checksum, size, dsize, area, name) = match self.font_record(f) {
            Some(record) => (
                record.checksum,
                record.size,
                record.dsize,
                record.area.clone(),
                record.name.clone(),
            ),
            None => (0, 0, 0, Vec::new(), Vec::new()),
        };
        self.dvi_out((checksum >> 24) as u8)?;
        self.dvi_out((checksum >> 16) as u8)?;
        self.dvi_out((checksum >> 8) as u8)?;
        self.dvi_out(checksum as u8)?;
        self.dvi_four(size)?;
        self.dvi_four(dsize)?;
        self.dvi_out(area.len() as u8)?;
        let l = name
            .iter()
            .position(|&unit| unit == i32::from(b':'))
            .unwrap_or(name.len());
        self.dvi_out(l as u8)?;
        for unit in &area {
            self.dvi_out(*unit as u8)?;
        }
        for unit in &name[..l] {
            self.dvi_out(*unit as u8)?;
        }
        Ok(())
    }

    /// Emits the `fnt`-selection opcode sequence for `f` when it differs
    /// from `dvi_f` (C's shared "Change font dvi_f to f" block).
    fn dvi_set_font(&mut self, f: i32) -> Result<(), TeXMouthError> {
        if f == self.dvi_f {
            return Ok(());
        }
        if let Some(record) = self.font_record(f) {
            if !record.used {
                self.dvi_font_def(f)?;
                if let Some(record) = self.font_record_mut(f) {
                    record.used = true;
                }
            }
        } else {
            self.dvi_font_def(f)?;
        }

        if f <= 64 {
            self.dvi_out(f as u8 + FNT_NUM_0 - 1)?;
        } else if f <= 256 {
            self.dvi_out(FNT1)?;
            self.dvi_out((f - 1) as u8)?;
        } else {
            self.dvi_out(FNT1 + 1)?;
            self.dvi_out(((f - 1) / 256) as u8)?;
            self.dvi_out(((f - 1) % 256) as u8)?;
        }
        self.dvi_f = f;
        Ok(())
    }

    /// Emits a right/down movement to `w`, then updates `dvi_h`/`dvi_v`
    /// bookkeeping; the C callers assign `dvi_h = cur_h` after each
    /// `movement` call — this helper only emits (`movement`).
    fn movement(&mut self, w: i32, o: u8) -> Result<(), TeXMouthError> {
        let mut w = w;
        let q = self.get_node(MOVEMENT_NODE_SIZE)?;
        self.mem[(q + 1) as usize].s1 = w;
        self.mem[(q + 2) as usize].s1 = self.dvi_offset + self.dvi_ptr;

        if o == DOWN1 {
            self.mem[q as usize].s1 = self.down_ptr;
            self.down_ptr = q;
        } else {
            self.mem[q as usize].s1 = self.right_ptr;
            self.right_ptr = q;
        }

        let mut p = self.mem[q as usize].s1;
        let mut mstate = MOV_NONE_SEEN;

        // Search the record list for a reusable distance (C's §630-634).
        enum Search {
            Found(i32),
            NotFound,
        }
        let mut result = Search::NotFound;

        while p != TEX_NULL {
            if self.mem[(p + 1) as usize].s1 == w {
                match mstate + self.mem[p as usize].s0 {
                    x if x == MOV_NONE_SEEN + MOV_YZ_OK
                        || x == MOV_NONE_SEEN + MOV_Y_OK
                        || x == MOV_Z_SEEN + MOV_YZ_OK
                        || x == MOV_Z_SEEN + MOV_Y_OK =>
                    {
                        if self.mem[(p + 2) as usize].s1 < self.dvi_gone {
                            break;
                        }
                        let mut k = self.mem[(p + 2) as usize].s1 - self.dvi_offset;
                        if k < 0 {
                            k += DVI_BUF_SIZE as i32;
                        }
                        self.dvi_buf[k as usize] += 5;
                        self.mem[p as usize].s0 = MOV_Y_HERE;
                        result = Search::Found(p);
                        break;
                    }
                    x if x == MOV_NONE_SEEN + MOV_Z_OK
                        || x == MOV_Y_SEEN + MOV_YZ_OK
                        || x == MOV_Y_SEEN + MOV_Z_OK =>
                    {
                        if self.mem[(p + 2) as usize].s1 < self.dvi_gone {
                            break;
                        }
                        let mut k = self.mem[(p + 2) as usize].s1 - self.dvi_offset;
                        if k < 0 {
                            k += DVI_BUF_SIZE as i32;
                        }
                        self.dvi_buf[k as usize] += 10;
                        self.mem[p as usize].s0 = MOV_Z_HERE;
                        result = Search::Found(p);
                        break;
                    }
                    x if x == MOV_NONE_SEEN + MOV_Y_HERE
                        || x == MOV_NONE_SEEN + MOV_Z_HERE
                        || x == MOV_Y_SEEN + MOV_Z_HERE
                        || x == MOV_Z_SEEN + MOV_Y_HERE =>
                    {
                        result = Search::Found(p);
                        break;
                    }
                    _ => {}
                }
            } else {
                match mstate + self.mem[p as usize].s0 {
                    x if x == MOV_NONE_SEEN + MOV_Y_HERE => mstate = MOV_Y_SEEN,
                    x if x == MOV_NONE_SEEN + MOV_Z_HERE => mstate = MOV_Z_SEEN,
                    x if x == MOV_Y_SEEN + MOV_Z_HERE || x == MOV_Z_SEEN + MOV_Y_HERE => {
                        break;
                    }
                    _ => {}
                }
            }
            p = self.mem[p as usize].s1;
        }

        match result {
            Search::NotFound => {
                self.mem[q as usize].s0 = MOV_YZ_OK;
                if w.abs() >= 0x80_0000 {
                    self.dvi_out(o + 3)?;
                    self.dvi_four(w)?;
                } else if w.abs() >= 0x8000 {
                    self.dvi_out(o + 2)?;
                    if w < 0 {
                        w += 0x100_0000;
                    }
                    self.dvi_out((w / 0x10000) as u8)?;
                    self.dvi_out((w % 0x10000 / 256) as u8)?;
                    self.dvi_out((w % 256) as u8)?;
                } else if w.abs() >= 128 {
                    self.dvi_out(o + 1)?;
                    if w < 0 {
                        w += 0x10000;
                    }
                    self.dvi_out((w / 256) as u8)?;
                    self.dvi_out((w % 256) as u8)?;
                } else {
                    self.dvi_out(o)?;
                    if w < 0 {
                        w += 256;
                    }
                    self.dvi_out((w % 256) as u8)?;
                }
            }
            Search::Found(p) => {
                self.mem[q as usize].s0 = self.mem[p as usize].s0;
                let mut q = q;
                if self.mem[q as usize].s0 == MOV_Y_HERE {
                    self.dvi_out(o + 4)?;
                    while self.mem[q as usize].s1 != p {
                        q = self.mem[q as usize].s1;
                        match self.mem[q as usize].s0 {
                            MOV_YZ_OK => self.mem[q as usize].s0 = MOV_Z_OK,
                            MOV_Y_OK => self.mem[q as usize].s0 = MOV_D_FIXED,
                            _ => {}
                        }
                    }
                } else {
                    self.dvi_out(o + 9)?;
                    while self.mem[q as usize].s1 != p {
                        q = self.mem[q as usize].s1;
                        match self.mem[q as usize].s0 {
                            MOV_YZ_OK => self.mem[q as usize].s0 = MOV_Y_OK,
                            MOV_Z_OK => self.mem[q as usize].s0 = MOV_D_FIXED,
                            _ => {}
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Drops movement records emitted after position `l`
    /// (`prune_movements`), called when a box ends.
    fn prune_movements(&mut self, l: i32) -> Result<(), TeXMouthError> {
        while self.down_ptr != TEX_NULL {
            if self.mem[(self.down_ptr + 2) as usize].s1 < l {
                break;
            }
            let p = self.down_ptr;
            self.down_ptr = self.mem[p as usize].s1;
            self.free_node(p, MOVEMENT_NODE_SIZE)?;
        }
        while self.right_ptr != TEX_NULL {
            if self.mem[(self.right_ptr + 2) as usize].s1 < l {
                break;
            }
            let p = self.right_ptr;
            self.right_ptr = self.mem[p as usize].s1;
            self.free_node(p, MOVEMENT_NODE_SIZE)?;
        }
        Ok(())
    }

    /// Advances `q` past penalty/insertion/mark/adjust and small-whatsit
    /// nodes (C's §641 `check_next` scan).
    fn skip_invisible_nodes(&self, mut q: i32) -> Result<i32, TeXMouthError> {
        while self.invisible_to_interword_space(q)? {
            q = self.mem[q as usize].s1;
        }
        Ok(q)
    }

    /// Merges adjacent native-word nodes separated by interword glue or
    /// space-adjustment kerns into single native words, then justifies
    /// their glyphs (C's §640-641 "Extra stuff for justifiable AAT text").
    /// Only runs when `\XeTeXinterwordspaceshaping` > 1.
    fn merge_native_word_runs(&mut self, this_box: i32) -> Result<(), TeXMouthError> {
        let g_sign = self.mem[(this_box + 5) as usize].b16_s1();
        let g_order = self.mem[(this_box + 5) as usize].b16_s0();
        let glue_set = self.mem[(this_box + 6) as usize].gr();

        let mut p = self.mem[(this_box + 5) as usize].s1;
        let mut prev_p = this_box + 5;

        while p != TEX_NULL {
            if self.mem[p as usize].s1 != TEX_NULL && self.is_native_word_whatsit(p)? {
                let font = i32::from(self.mem[(p + 4) as usize].b16_s2());
                let letter_space = self
                    .font_record(font)
                    .map_or(0, |record| record.letter_space);
                if letter_space == 0 {
                    // Start of a potential run.
                    let r = p;
                    let mut q = self.mem[p as usize].s1;
                    let font_glue = self.font_record(font).map_or(TEX_NULL, |r| r.glue);

                    // Extend the run while words in the same font follow
                    // (C's `check_next`/`end_node_run` loop).
                    loop {
                        q = self.skip_invisible_nodes(q)?;
                        if q != TEX_NULL && !self.is_char_node(q) {
                            if self.node_type(q)? == GLUE_NODE
                                && self.mem[q as usize].b16_s0() == NORMAL
                            {
                                let spec = self.mem[(q + 1) as usize].s0;
                                if spec == font_glue {
                                    q = self.mem[q as usize].s1;
                                    q = self.skip_invisible_nodes(q)?;
                                    if self.is_native_word_whatsit(q)?
                                        && self.mem[(q + 4) as usize].b16_s2()
                                            == self.mem[(r + 4) as usize].b16_s2()
                                    {
                                        p = q;
                                        q = self.mem[q as usize].s1;
                                        continue;
                                    }
                                } else {
                                    q = self.mem[q as usize].s1;
                                }

                                if q != TEX_NULL
                                    && !self.is_char_node(q)
                                    && self.node_type(q)? == KERN_NODE
                                    && self.node_subtype(q)? == SPACE_ADJUSTMENT
                                {
                                    q = self.mem[q as usize].s1;
                                    q = self.skip_invisible_nodes(q)?;
                                    if self.is_native_word_whatsit(q)?
                                        && i32::from(self.mem[(q + 4) as usize].b16_s2()) == font
                                    {
                                        p = q;
                                        q = self.mem[q as usize].s1;
                                        continue;
                                    }
                                }
                                break;
                            }
                            if self.is_native_word_whatsit(q)?
                                && i32::from(self.mem[(q + 4) as usize].b16_s2()) == font
                            {
                                p = q;
                                q = self.mem[q as usize].s1;
                                continue;
                            }
                        }
                        break;
                    }

                    // `r` is the first word of the run, `p` the last.
                    if p != r {
                        let mut text: Vec<u16> = Vec::new();
                        let mut width = 0i32;
                        let mut q = r;
                        loop {
                            match self.node_type(q)? {
                                WHATSIT_NODE => {
                                    if self.is_native_word_whatsit(q)? {
                                        let len = i32::from(self.mem[(q + 4) as usize].b16_s1());
                                        for j in 0..len {
                                            text.push(self.native_word_text_at(q, j)?);
                                        }
                                        width = width.saturating_add(self.mem[(q + 1) as usize].s1);
                                    }
                                }
                                GLUE_NODE => {
                                    text.push(u16::from(b' '));
                                    let spec = self.mem[(q + 1) as usize].s0;
                                    width = width.saturating_add(self.mem[(spec + 1) as usize].s1);
                                    if g_sign != NORMAL {
                                        if g_sign == STRETCHING {
                                            if self.mem[spec as usize].b16_s1() == g_order {
                                                width = width.saturating_add(tex_round(
                                                    glue_set
                                                        * f64::from(
                                                            self.mem[(spec + 2) as usize].s1,
                                                        ),
                                                ));
                                            }
                                        } else if self.mem[spec as usize].b16_s0() == g_order {
                                            width = width.saturating_sub(tex_round(
                                                glue_set
                                                    * f64::from(self.mem[(spec + 3) as usize].s1),
                                            ));
                                        }
                                    }
                                }
                                KERN_NODE => {
                                    width = width.saturating_add(self.mem[(q + 1) as usize].s1);
                                }
                                _ => {}
                            }
                            if q == p {
                                break;
                            }
                            q = self.mem[q as usize].s1;
                        }

                        let merged = self.new_native_word_node(font, text.len() as i32)?;
                        let subtype = self.node_subtype(r)?;
                        self.set_node_subtype(merged, subtype)?;
                        for (j, unit) in text.iter().enumerate() {
                            self.set_native_word_text_at(merged, j as i32, *unit)?;
                        }

                        self.mem[(merged + 1) as usize].s1 = width;
                        self.set_justified_native_glyphs(merged)?;
                        self.mem[prev_p as usize].s1 = merged;
                        self.mem[merged as usize].s1 = self.mem[p as usize].s1;
                        self.mem[p as usize].s1 = TEX_NULL;
                        prev_p = r;
                        p = self.mem[r as usize].s1;

                        // Re-insert invisible nodes after the merged word.
                        // `q_insert` is the advancing insertion point
                        // (C's `q`); `prev_p` tracks the detached-run
                        // predecessor and updates unconditionally.
                        let mut q_insert = merged;
                        while p != TEX_NULL {
                            if self.invisible_to_interword_space(p)? {
                                self.mem[prev_p as usize].s1 = self.mem[p as usize].s1;
                                self.mem[p as usize].s1 = self.mem[q_insert as usize].s1;
                                self.mem[q_insert as usize].s1 = p;
                                q_insert = p;
                            }
                            prev_p = p;
                            p = self.mem[p as usize].s1;
                        }

                        self.flush_node_list(r)?;
                        p = q_insert;
                    }
                }
            }
            if p != TEX_NULL {
                prev_p = p;
                p = self.mem[p as usize].s1;
            }
        }
        Ok(())
    }

    /// Reverses the node list at `temp_ptr`, appending to `t`, updating
    /// the running glue state; returns the new head (`reverse`, C §1510).
    fn reverse(
        &mut self,
        this_box: i32,
        t: i32,
        cur_g: &mut i32,
        cur_glue: &mut f64,
    ) -> Result<i32, TeXMouthError> {
        let g_order = self.mem[(this_box + 5) as usize].b16_s0();
        let g_sign = self.mem[(this_box + 5) as usize].b16_s1();
        let mut l = t;
        let mut p = self.temp_ptr;
        let mut m = MIN_HALFWORD;
        let mut n = MIN_HALFWORD;

        'outer: loop {
            while p != TEX_NULL {
                // §1511: move node p to the new list.
                'reswitch: loop {
                    if self.is_char_node(p) {
                        while self.is_char_node(p) {
                            let f = i32::from(self.mem[p as usize].b16_s1());
                            let c = i32::from(self.mem[p as usize].b16_s0());
                            // `effective_char(true, f, c)`: the TFM mapping
                            // applies unconditionally here.
                            let e = self
                                .font_record(f)
                                .and_then(|record| {
                                    record.tfm_font_mapping.get(&(c.max(0) as u32)).copied()
                                })
                                .map_or(c, |m| m as i32);
                            self.xtx_ligature_present = false;
                            self.cur_h += self.font_char_metric(f, e.max(0) as u32).width;
                            let q = self.mem[p as usize].s1;
                            self.mem[p as usize].s1 = l;
                            l = p;
                            p = q;
                        }
                        break 'reswitch;
                    }
                    let q = self.mem[p as usize].s1;
                    let mut rule_wd = 0i32;
                    let mut skip_link = false;
                    match self.node_type(p)? {
                        HLIST_NODE | VLIST_NODE | RULE_NODE | KERN_NODE => {
                            rule_wd = self.mem[(p + 1) as usize].s1;
                        }
                        WHATSIT_NODE => {
                            let subtype = self.node_subtype(p)?;
                            if matches!(
                                subtype,
                                NATIVE_WORD_NODE
                                    | NATIVE_WORD_NODE_AT
                                    | GLYPH_NODE
                                    | PIC_NODE
                                    | PDF_NODE
                            ) {
                                rule_wd = self.mem[(p + 1) as usize].s1;
                            } else {
                                skip_link = true;
                            }
                        }
                        GLUE_NODE => {
                            let mut spec = self.mem[(p + 1) as usize].s0;
                            rule_wd = self.mem[(spec + 1) as usize].s1 - *cur_g;
                            if g_sign != NORMAL {
                                if g_sign == STRETCHING {
                                    if self.mem[spec as usize].b16_s1() == g_order {
                                        *cur_glue += f64::from(self.mem[(spec + 2) as usize].s1);
                                        let glue_temp = (self.mem[(this_box + 6) as usize].gr()
                                            * *cur_glue)
                                            .clamp(-GLUE_LIMIT, GLUE_LIMIT);
                                        *cur_g = tex_round(glue_temp);
                                    }
                                } else if self.mem[spec as usize].b16_s0() == g_order {
                                    *cur_glue -= f64::from(self.mem[(spec + 3) as usize].s1);
                                    let glue_temp = (self.mem[(this_box + 6) as usize].gr()
                                        * *cur_glue)
                                        .clamp(-GLUE_LIMIT, GLUE_LIMIT);
                                    *cur_g = tex_round(glue_temp);
                                }
                            }
                            rule_wd += *cur_g;

                            if (g_sign == STRETCHING && self.mem[spec as usize].b16_s1() == g_order)
                                || (g_sign == SHRINKING
                                    && self.mem[spec as usize].b16_s0() == g_order)
                            {
                                if self.mem[spec as usize].s1 == TEX_NULL {
                                    self.free_node(spec, GLUE_SPEC_SIZE)?;
                                } else {
                                    self.mem[spec as usize].s1 -= 1;
                                }
                                if i32::from(self.node_subtype(p)?) < A_LEADERS {
                                    self.set_node_type(p, KERN_NODE)?;
                                    self.mem[(p + 1) as usize].s1 = rule_wd;
                                } else {
                                    spec = self.get_node(GLUE_SPEC_SIZE)?;
                                    self.mem[spec as usize].set_b16_s1(FILLL + 1);
                                    self.mem[spec as usize].set_b16_s0(FILLL + 1);
                                    self.mem[(spec + 1) as usize].s1 = rule_wd;
                                    self.mem[(spec + 2) as usize].s1 = 0;
                                    self.mem[(spec + 3) as usize].s1 = 0;
                                    self.mem[(p + 1) as usize].s0 = spec;
                                }
                            }
                        }
                        LIGATURE_NODE => {
                            self.flush_node_list(self.mem[(p + 1) as usize].s1)?;
                            self.temp_ptr = p;
                            p = self.get_avail()?;
                            self.mem[p as usize] = self.mem[(self.temp_ptr + 1) as usize];
                            self.mem[p as usize].s1 = q;
                            self.free_node(self.temp_ptr, SMALL_NODE_SIZE)?;
                            continue 'reswitch;
                        }
                        MATH_NODE => {
                            // §1516: math nodes in an inner reflected
                            // segment are modified; outer-level ones become
                            // kern nodes.
                            rule_wd = self.mem[(p + 1) as usize].s1;
                            let subtype = i32::from(self.node_subtype(p)?);
                            if subtype % 2 != 0 {
                                let end_lr_type = L_CODE * (subtype / L_CODE) + END_M_CODE;
                                if self.mem[self.lr_ptr as usize].s0 != end_lr_type {
                                    self.set_node_type(p, KERN_NODE)?;
                                    self.lr_problems += 1;
                                } else {
                                    let temp = self.lr_ptr;
                                    self.lr_ptr = self.mem[temp as usize].s1;
                                    self.mem[temp as usize].s1 = self.avail;
                                    self.avail = temp;

                                    if n > MIN_HALFWORD {
                                        n -= 1;
                                        self.set_node_subtype(p, (subtype - 1) as u16)?;
                                    } else {
                                        self.set_node_type(p, KERN_NODE)?;
                                        if m > MIN_HALFWORD {
                                            m -= 1;
                                        } else {
                                            // §1517: finish the reversed
                                            // segment.
                                            self.free_node(p, MEDIUM_NODE_SIZE)?;
                                            self.mem[t as usize].s1 = q;
                                            self.mem[(t + 1) as usize].s1 = rule_wd;
                                            self.mem[(t + 2) as usize].s1 = -self.cur_h - rule_wd;
                                            break 'outer;
                                        }
                                    }
                                }
                            } else {
                                let end_lr_type = L_CODE * (subtype / L_CODE) + END_M_CODE;
                                let temp = self.get_avail()?;
                                self.mem[temp as usize].s0 = end_lr_type;
                                self.mem[temp as usize].s1 = self.lr_ptr;
                                self.lr_ptr = temp;
                                let lr_dir = subtype / R_CODE;
                                if n > MIN_HALFWORD || lr_dir != self.cur_dir {
                                    n += 1;
                                    self.set_node_subtype(p, (subtype + 1) as u16)?;
                                } else {
                                    self.set_node_type(p, KERN_NODE)?;
                                    m += 1;
                                }
                            }
                        }
                        EDGE_NODE => {
                            return Err(self.confusion("LR2"));
                        }
                        _ => {
                            skip_link = true;
                        }
                    }

                    if skip_link {
                        self.mem[p as usize].s1 = l;
                        l = p;
                        p = q;
                        break 'reswitch;
                    }

                    self.cur_h += rule_wd;
                    self.mem[p as usize].s1 = l;

                    if self.node_type(p)? == KERN_NODE && (rule_wd == 0 || l == TEX_NULL) {
                        // C: `p = l` then the unconditional `l = p; p = q`
                        // leaves `l` unchanged.
                        self.free_node(p, MEDIUM_NODE_SIZE)?;
                    } else {
                        l = p;
                    }
                    p = q;
                    break 'reswitch;
                }
            }

            if t == TEX_NULL && m == MIN_HALFWORD && n == MIN_HALFWORD {
                break 'outer;
            }

            let subtype = self.mem[self.lr_ptr as usize].s0;
            p = self.new_math(0, subtype)?;
            self.lr_problems += 10_000;
        }

        Ok(l)
    }

    /// Emits the hlist at `temp_ptr` (`hlist_out`, C §639): characters,
    /// native words/glyphs, nested boxes, rules, glue, leaders, kerns,
    /// math-direction boundaries, ligatures, and whatsits.
    fn hlist_out(&mut self) -> Result<(), TeXMouthError> {
        let this_box = self.temp_ptr;
        let g_order = self.mem[(this_box + 5) as usize].b16_s0();
        let g_sign = self.mem[(this_box + 5) as usize].b16_s1();
        let mut cur_g = 0i32;
        let mut cur_glue = 0.0f64;

        if self.int_par(INT_PAR_XETEX_INTERWORD_SPACE_SHAPING)? > 1 {
            self.merge_native_word_runs(this_box)?;
        }

        let mut p = self.mem[(this_box + 5) as usize].s1;
        self.cur_s += 1;
        if self.cur_s > 0 {
            self.dvi_out(PUSH)?;
        }
        if self.cur_s > self.max_push {
            self.max_push = self.cur_s;
        }

        let save_loc = self.dvi_offset + self.dvi_ptr;
        let base_line = self.cur_v;
        let mut prev_p = this_box + 5;

        // §1501: initialize for mixed direction typesetting.
        let temp = self.get_avail()?;
        self.mem[temp as usize].s0 = BEFORE;
        self.mem[temp as usize].s1 = self.lr_ptr;
        self.lr_ptr = temp;

        if i32::from(self.mem[this_box as usize].b16_s0()) == DLIST {
            if self.cur_dir == RIGHT_TO_LEFT {
                self.cur_dir = LEFT_TO_RIGHT;
                self.cur_h -= self.mem[(this_box + 1) as usize].s1;
            } else {
                self.mem[this_box as usize].set_b16_s0(0);
            }
        }

        if self.cur_dir == RIGHT_TO_LEFT
            && i32::from(self.mem[this_box as usize].b16_s0()) != REVERSED
        {
            // §1508: reverse the complete hlist and mark it reversed.
            let save_h = self.cur_h;
            self.temp_ptr = p;
            p = self.new_kern(0)?;
            // SyncTeX: too late to record this kern (`SYNCTEX_tag = 0`).
            self.mem[(p + 2) as usize].s0 = 0;
            self.mem[p as usize].s1 = TEX_NULL;
            self.mem[prev_p as usize].s1 = p;
            self.cur_h = 0;
            let reversed = self.reverse(this_box, TEX_NULL, &mut cur_g, &mut cur_glue)?;
            self.mem[p as usize].s1 = reversed;
            self.mem[(p + 1) as usize].s1 = -self.cur_h;
            self.cur_h = save_h;
            self.mem[this_box as usize].set_b16_s0(REVERSED as u16);
        }

        let mut left_edge = self.cur_h;
        self.synctex_hlist(this_box);

        while p != TEX_NULL {
            // §642: emit node p and advance, keeping cur_v = base_line.
            'reswitch: loop {
                if self.is_char_node(p) {
                    if self.cur_h != self.dvi_h {
                        self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                        self.dvi_h = self.cur_h;
                    }
                    if self.cur_v != self.dvi_v {
                        self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                        self.dvi_v = self.cur_v;
                    }

                    loop {
                        let f = i32::from(self.mem[p as usize].b16_s1());
                        let mut c = i32::from(self.mem[p as usize].b16_s0());
                        if p != self.lig_trick() {
                            if let Some(&mapped) = self
                                .font_record(f)
                                .and_then(|record| record.tfm_font_mapping.get(&(c.max(0) as u32)))
                            {
                                c = mapped as i32;
                            }
                        }

                        self.dvi_set_font(f)?;

                        let (bc, ec) = self
                            .font_record(f)
                            .map_or((0, 0), |record| (record.bc, record.ec));
                        if (ec as i32) >= c
                            && (bc as i32) <= c
                            && self.char_exists(f, c.max(0) as u32)
                        {
                            if c >= 128 {
                                self.dvi_out(SET1)?;
                            }
                            self.dvi_out(c as u8)?;
                            self.cur_h += self.font_char_metric(f, c.max(0) as u32).width;
                        }

                        prev_p = self.mem[prev_p as usize].s1;
                        p = self.mem[p as usize].s1;
                        if !self.is_char_node(p) {
                            break;
                        }
                    }

                    self.synctex_current();
                    self.dvi_h = self.cur_h;
                    break 'reswitch;
                }

                // §644: emit the non-char node p and move to the next.
                // `rule_ht`/`rule_dp`/`rule_wd` are C globals shared with the
                // SyncTeX recorder; seeding from the context reproduces the
                // cross-node persistence of the C statics.
                let mut rule_ht: i32 = self.synctex.rule_ht;
                let mut rule_dp: i32 = self.synctex.rule_dp;
                let mut rule_wd: i32;
                let mut reswitch = false;

                'dispatch: {
                    match self.node_type(p)? {
                        HLIST_NODE | VLIST_NODE => {
                            if self.mem[(p + 5) as usize].s1 == TEX_NULL {
                                if self.node_type(p)? == VLIST_NODE {
                                    self.synctex_void_vlist(p, this_box);
                                } else {
                                    self.synctex_void_hlist(p, this_box);
                                }
                                self.cur_h += self.mem[(p + 1) as usize].s1;
                            } else {
                                let save_h = self.dvi_h;
                                let save_v = self.dvi_v;
                                self.cur_v = base_line + self.mem[(p + 4) as usize].s1;
                                self.temp_ptr = p;
                                let edge = self.cur_h + self.mem[(p + 1) as usize].s1;
                                if self.cur_dir == RIGHT_TO_LEFT {
                                    self.cur_h = edge;
                                }
                                if self.node_type(p)? == VLIST_NODE {
                                    self.vlist_out()?;
                                } else {
                                    self.hlist_out()?;
                                }
                                self.dvi_h = save_h;
                                self.dvi_v = save_v;
                                self.cur_h = edge;
                                self.cur_v = base_line;
                            }
                        }
                        RULE_NODE => {
                            rule_ht = self.mem[(p + 3) as usize].s1;
                            rule_dp = self.mem[(p + 2) as usize].s1;
                            rule_wd = self.mem[(p + 1) as usize].s1;
                            // fin_rule
                            if rule_ht == NULL_FLAG {
                                rule_ht = self.mem[(this_box + 3) as usize].s1;
                            }
                            if rule_dp == NULL_FLAG {
                                rule_dp = self.mem[(this_box + 2) as usize].s1;
                            }
                            rule_ht += rule_dp;
                            if rule_ht > 0 && rule_wd > 0 {
                                if self.cur_h != self.dvi_h {
                                    self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                    self.dvi_h = self.cur_h;
                                }
                                self.cur_v = base_line + rule_dp;
                                if self.cur_v != self.dvi_v {
                                    self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                    self.dvi_v = self.cur_v;
                                }
                                self.dvi_out(SET_RULE)?;
                                self.dvi_four(rule_ht)?;
                                self.dvi_four(rule_wd)?;
                                self.cur_v = base_line;
                                self.dvi_h += rule_wd;
                            }
                            // move_past
                            self.cur_h += rule_wd;
                            // SyncTeX reads the resolved rule dims (C globals).
                            self.synctex.rule_wd = rule_wd;
                            self.synctex.rule_ht = rule_ht;
                            self.synctex.rule_dp = rule_dp;
                            self.synctex_horizontal_rule_or_glue(p, this_box);
                        }
                        WHATSIT_NODE => {
                            self.hlist_out_whatsit(p, base_line)?;
                        }
                        GLUE_NODE => {
                            // §647: move right or output leaders.
                            let mut spec = self.mem[(p + 1) as usize].s0;
                            rule_wd = self.mem[(spec + 1) as usize].s1 - cur_g;

                            if g_sign != NORMAL {
                                if g_sign == STRETCHING {
                                    if self.mem[spec as usize].b16_s1() == g_order {
                                        cur_glue += f64::from(self.mem[(spec + 2) as usize].s1);
                                        let glue_temp = (self.mem[(this_box + 6) as usize].gr()
                                            * cur_glue)
                                            .clamp(-GLUE_LIMIT, GLUE_LIMIT);
                                        cur_g = tex_round(glue_temp);
                                    }
                                } else if self.mem[spec as usize].b16_s0() == g_order {
                                    cur_glue -= f64::from(self.mem[(spec + 3) as usize].s1);
                                    let glue_temp = (self.mem[(this_box + 6) as usize].gr()
                                        * cur_glue)
                                        .clamp(-GLUE_LIMIT, GLUE_LIMIT);
                                    cur_g = tex_round(glue_temp);
                                }
                            }
                            rule_wd += cur_g;

                            // §1486: handle glue for mixed directions.
                            if (g_sign == STRETCHING && self.mem[spec as usize].b16_s1() == g_order)
                                || (g_sign == SHRINKING
                                    && self.mem[spec as usize].b16_s0() == g_order)
                            {
                                if self.mem[spec as usize].s1 == TEX_NULL {
                                    self.free_node(spec, GLUE_SPEC_SIZE)?;
                                } else {
                                    self.mem[spec as usize].s1 -= 1;
                                }
                                if i32::from(self.node_subtype(p)?) < A_LEADERS {
                                    self.set_node_type(p, KERN_NODE)?;
                                    self.mem[(p + 1) as usize].s1 = rule_wd;
                                } else {
                                    spec = self.get_node(GLUE_SPEC_SIZE)?;
                                    self.mem[spec as usize].set_b16_s1(FILLL + 1);
                                    self.mem[spec as usize].set_b16_s0(FILLL + 1);
                                    self.mem[(spec + 1) as usize].s1 = rule_wd;
                                    self.mem[(spec + 2) as usize].s1 = 0;
                                    self.mem[(spec + 3) as usize].s1 = 0;
                                    self.mem[(p + 1) as usize].s0 = spec;
                                }
                            }

                            if i32::from(self.node_subtype(p)?) >= A_LEADERS {
                                // §648: output leaders into an hlist.
                                let leader_box = self.mem[(p + 1) as usize].s1;
                                if self.node_type(leader_box)? == RULE_NODE {
                                    rule_ht = self.mem[(leader_box + 3) as usize].s1;
                                    rule_dp = self.mem[(leader_box + 2) as usize].s1;
                                    // fin_rule with rule_wd as computed
                                    if rule_ht == NULL_FLAG {
                                        rule_ht = self.mem[(this_box + 3) as usize].s1;
                                    }
                                    if rule_dp == NULL_FLAG {
                                        rule_dp = self.mem[(this_box + 2) as usize].s1;
                                    }
                                    rule_ht += rule_dp;
                                    if rule_ht > 0 && rule_wd > 0 {
                                        if self.cur_h != self.dvi_h {
                                            self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                            self.dvi_h = self.cur_h;
                                        }
                                        self.cur_v = base_line + rule_dp;
                                        if self.cur_v != self.dvi_v {
                                            self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                            self.dvi_v = self.cur_v;
                                        }
                                        self.dvi_out(SET_RULE)?;
                                        self.dvi_four(rule_ht)?;
                                        self.dvi_four(rule_wd)?;
                                        self.cur_v = base_line;
                                        self.dvi_h += rule_wd;
                                    }
                                    self.cur_h += rule_wd;
                                    // SyncTeX reads the resolved rule dims.
                                    self.synctex.rule_wd = rule_wd;
                                    self.synctex.rule_ht = rule_ht;
                                    self.synctex.rule_dp = rule_dp;
                                    self.synctex_horizontal_rule_or_glue(p, this_box);
                                    break 'dispatch;
                                }

                                let leader_wd = self.mem[(leader_box + 1) as usize].s1;
                                if leader_wd > 0 && rule_wd > 0 {
                                    rule_wd += 10;
                                    if self.cur_dir == RIGHT_TO_LEFT {
                                        self.cur_h -= 10;
                                    }
                                    let edge = self.cur_h + rule_wd;
                                    let mut lx = 0i32;

                                    // §649: position of the first leader box.
                                    if i32::from(self.node_subtype(p)?) == A_LEADERS {
                                        let save_h = self.cur_h;
                                        self.cur_h = left_edge
                                            + leader_wd * ((self.cur_h - left_edge) / leader_wd);
                                        if self.cur_h < save_h {
                                            self.cur_h += leader_wd;
                                        }
                                    } else {
                                        let lq = rule_wd / leader_wd;
                                        let lr = rule_wd % leader_wd;
                                        if self.node_subtype(p)? == C_LEADERS {
                                            self.cur_h += lr / 2;
                                        } else {
                                            lx = lr / (lq + 1);
                                            self.cur_h += (lr - (lq - 1) * lx) / 2;
                                        }
                                    }

                                    while self.cur_h + leader_wd <= edge {
                                        // §650: output a leader box.
                                        self.cur_v =
                                            base_line + self.mem[(leader_box + 4) as usize].s1;
                                        if self.cur_v != self.dvi_v {
                                            self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                            self.dvi_v = self.cur_v;
                                        }
                                        let save_v = self.dvi_v;
                                        if self.cur_h != self.dvi_h {
                                            self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                            self.dvi_h = self.cur_h;
                                        }
                                        let save_h = self.dvi_h;
                                        self.temp_ptr = leader_box;
                                        if self.cur_dir == RIGHT_TO_LEFT {
                                            self.cur_h += leader_wd;
                                        }
                                        let outer_doing_leaders = self.doing_leaders;
                                        self.doing_leaders = true;
                                        if self.node_type(leader_box)? == VLIST_NODE {
                                            self.vlist_out()?;
                                        } else {
                                            self.hlist_out()?;
                                        }
                                        self.doing_leaders = outer_doing_leaders;
                                        self.dvi_v = save_v;
                                        self.dvi_h = save_h;
                                        self.cur_v = base_line;
                                        self.cur_h = save_h + leader_wd + lx;
                                    }

                                    if self.cur_dir == RIGHT_TO_LEFT {
                                        self.cur_h = edge;
                                    } else {
                                        self.cur_h = edge - 10;
                                    }
                                    break 'dispatch;
                                }
                            }
                            // move_past
                            self.cur_h += rule_wd;
                            // SyncTeX reads the resolved rule dims (C globals).
                            self.synctex.rule_wd = rule_wd;
                            self.synctex.rule_ht = rule_ht;
                            self.synctex.rule_dp = rule_dp;
                            self.synctex_horizontal_rule_or_glue(p, this_box);
                        }
                        MARGIN_KERN_NODE => {
                            self.cur_h += self.mem[(p + 1) as usize].s1;
                        }
                        KERN_NODE => {
                            self.synctex_kern(p, this_box);
                            self.cur_h += self.mem[(p + 1) as usize].s1;
                        }
                        MATH_NODE => {
                            self.synctex_math(p, this_box);
                            let subtype = i32::from(self.node_subtype(p)?);
                            if subtype % 2 != 0 {
                                // end_LR
                                let end_lr_type = L_CODE * (subtype / L_CODE) + END_M_CODE;
                                if self.mem[self.lr_ptr as usize].s0 == end_lr_type {
                                    let temp = self.lr_ptr;
                                    self.lr_ptr = self.mem[temp as usize].s1;
                                    self.mem[temp as usize].s1 = self.avail;
                                    self.avail = temp;
                                } else if subtype > L_CODE {
                                    self.lr_problems += 1;
                                }
                            } else {
                                let end_lr_type = L_CODE * (subtype / L_CODE) + END_M_CODE;
                                let temp = self.get_avail()?;
                                self.mem[temp as usize].s0 = end_lr_type;
                                self.mem[temp as usize].s1 = self.lr_ptr;
                                self.lr_ptr = temp;

                                if subtype / R_CODE != self.cur_dir {
                                    // §1509: reverse an hlist segment.
                                    let save_h = self.cur_h;
                                    self.temp_ptr = self.mem[p as usize].s1;
                                    let rule_wd = self.mem[(p + 1) as usize].s1;
                                    self.free_node(p, MEDIUM_NODE_SIZE)?;
                                    self.cur_dir = 1 - self.cur_dir;
                                    p = self.new_edge(self.cur_dir, rule_wd)?;
                                    self.mem[prev_p as usize].s1 = p;
                                    self.cur_h = self.cur_h - left_edge + rule_wd;
                                    let tail = self.new_edge(1 - self.cur_dir, 0)?;
                                    let reversed =
                                        self.reverse(this_box, tail, &mut cur_g, &mut cur_glue)?;
                                    self.mem[p as usize].s1 = reversed;
                                    self.mem[(p + 2) as usize].s1 = self.cur_h;
                                    self.cur_dir = 1 - self.cur_dir;
                                    self.cur_h = save_h;
                                    reswitch = true;
                                    break 'dispatch;
                                }
                            }
                            self.set_node_type(p, KERN_NODE)?;
                            self.cur_h += self.mem[(p + 1) as usize].s1;
                        }
                        LIGATURE_NODE => {
                            // §675: make p look like a char node.
                            let lig_trick = self.lig_trick();
                            self.mem[lig_trick as usize] = self.mem[(p + 1) as usize];
                            self.mem[lig_trick as usize].s1 = self.mem[p as usize].s1;
                            p = lig_trick;
                            self.xtx_ligature_present = true;
                            reswitch = true;
                            break 'dispatch;
                        }
                        EDGE_NODE => {
                            self.cur_h += self.mem[(p + 1) as usize].s1;
                            left_edge = self.cur_h + self.mem[(p + 2) as usize].s1;
                            self.cur_dir = i32::from(self.node_subtype(p)?);
                        }
                        _ => {}
                    }
                }

                if reswitch {
                    continue 'reswitch;
                }
                prev_p = p;
                p = self.mem[p as usize].s1;
                break 'reswitch;
            }
        }

        self.synctex_tsilh(this_box);

        // §1502/1505: finish mixed-direction handling; check LR anomalies.
        while self.mem[self.lr_ptr as usize].s0 != BEFORE {
            if self.mem[self.lr_ptr as usize].s0 > L_CODE {
                self.lr_problems += 10_000;
            }
            let temp = self.lr_ptr;
            self.lr_ptr = self.mem[temp as usize].s1;
            self.mem[temp as usize].s1 = self.avail;
            self.avail = temp;
        }
        let temp = self.lr_ptr;
        self.lr_ptr = self.mem[temp as usize].s1;
        self.mem[temp as usize].s1 = self.avail;
        self.avail = temp;

        if i32::from(self.mem[this_box as usize].b16_s0()) == DLIST {
            self.cur_dir = RIGHT_TO_LEFT;
        }

        self.prune_movements(save_loc)?;
        if self.cur_s > 0 {
            self.dvi_pop(save_loc)?;
        }
        self.cur_s -= 1;
        Ok(())
    }

    /// Emits a whatsit node in an hlist (C's §1407 hlist whatsit cases).
    fn hlist_out_whatsit(&mut self, p: i32, base_line: i32) -> Result<(), TeXMouthError> {
        match self.node_subtype(p)? {
            NATIVE_WORD_NODE | NATIVE_WORD_NODE_AT | GLYPH_NODE => {
                if self.cur_h != self.dvi_h {
                    self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                    self.dvi_h = self.cur_h;
                }
                if self.cur_v != self.dvi_v {
                    self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                    self.dvi_v = self.cur_v;
                }
                let f = i32::from(self.mem[(p + 4) as usize].b16_s2());
                self.dvi_set_font(f)?;

                if self.node_subtype(p)? == GLYPH_NODE {
                    self.dvi_out(SET_GLYPHS)?;
                    self.dvi_four(self.mem[(p + 1) as usize].s1)?;
                    self.dvi_two(1)?;
                    self.dvi_four(0)?;
                    self.dvi_four(0)?;
                    self.dvi_two(self.mem[(p + 4) as usize].b16_s1())?;
                    self.cur_h += self.mem[(p + 1) as usize].s1;
                } else {
                    if self.node_subtype(p)? == NATIVE_WORD_NODE_AT {
                        if i32::from(self.mem[(p + 4) as usize].b16_s1()) > 0
                            || self.has_native_glyph_info(p)
                        {
                            self.dvi_out(SET_TEXT_AND_GLYPHS)?;
                            let len = i32::from(self.mem[(p + 4) as usize].b16_s1());
                            self.dvi_two(len as u16)?;
                            for k in 0..len {
                                self.dvi_two(self.native_word_text_at(p, k)?)?;
                            }
                            for byte in self.make_xdv_glyph_array_data(p)? {
                                self.dvi_out(byte)?;
                            }
                        }
                    } else if self.has_native_glyph_info(p) {
                        self.dvi_out(SET_GLYPHS)?;
                        for byte in self.make_xdv_glyph_array_data(p)? {
                            self.dvi_out(byte)?;
                        }
                    }
                    self.cur_h += self.mem[(p + 1) as usize].s1;
                }
                self.dvi_h = self.cur_h;
            }
            PIC_NODE | PDF_NODE => {
                let save_h = self.dvi_h;
                let save_v = self.dvi_v;
                self.cur_v = base_line;
                let edge = self.cur_h + self.mem[(p + 1) as usize].s1;
                self.pic_out(p)?;
                self.dvi_h = save_h;
                self.dvi_v = save_v;
                self.cur_h = edge;
                self.cur_v = base_line;
            }
            PDF_SAVE_POS_NODE => {
                self.pdf_last_x_pos = self.cur_h + 4_736_286;
                self.pdf_last_y_pos = self.cur_page_height - self.cur_v - 4_736_286;
            }
            _ => {
                self.out_what(p)?;
            }
        }
        Ok(())
    }

    /// Emits the vlist at `temp_ptr` (`vlist_out`, C §651): nested boxes,
    /// rules, native glyphs, glue, leaders, and vertical movement with
    /// `upwards` support.
    fn vlist_out(&mut self) -> Result<(), TeXMouthError> {
        let this_box = self.temp_ptr;
        let g_order = self.mem[(this_box + 5) as usize].b16_s0();
        let g_sign = self.mem[(this_box + 5) as usize].b16_s1();
        let mut cur_g = 0i32;
        let mut cur_glue = 0.0f64;
        let mut p = self.mem[(this_box + 5) as usize].s1;
        let upwards = i32::from(self.node_subtype(this_box)?) == 1;

        self.cur_s += 1;
        if self.cur_s > 0 {
            self.dvi_out(PUSH)?;
        }
        if self.cur_s > self.max_push {
            self.max_push = self.cur_s;
        }

        let save_loc = self.dvi_offset + self.dvi_ptr;
        let left_edge = self.cur_h;
        self.synctex_vlist(this_box);

        if upwards {
            self.cur_v += self.mem[(this_box + 2) as usize].s1;
        } else {
            self.cur_v -= self.mem[(this_box + 3) as usize].s1;
        }
        let top_edge = self.cur_v;

        while p != TEX_NULL {
            // §652: emit node p keeping cur_h = left_edge.
            if self.is_char_node(p) {
                return Err(self.confusion("vlistout"));
            }

            // §653: emit the non-char node p.

            'dispatch: {
                match self.node_type(p)? {
                    HLIST_NODE | VLIST_NODE => {
                        // §654: output a box in a vlist.
                        if self.mem[(p + 5) as usize].s1 == TEX_NULL {
                            if upwards {
                                self.cur_v -= self.mem[(p + 2) as usize].s1;
                            } else {
                                self.cur_v += self.mem[(p + 3) as usize].s1;
                            }
                            if self.node_type(p)? == VLIST_NODE {
                                self.synctex_void_vlist(p, this_box);
                            } else {
                                self.synctex_void_hlist(p, this_box);
                            }
                            if upwards {
                                self.cur_v -= self.mem[(p + 3) as usize].s1;
                            } else {
                                self.cur_v += self.mem[(p + 2) as usize].s1;
                            }
                        } else {
                            if upwards {
                                self.cur_v -= self.mem[(p + 2) as usize].s1;
                            } else {
                                self.cur_v += self.mem[(p + 3) as usize].s1;
                            }
                            if self.cur_v != self.dvi_v {
                                self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                self.dvi_v = self.cur_v;
                            }
                            let save_h = self.dvi_h;
                            let save_v = self.dvi_v;
                            if self.cur_dir == RIGHT_TO_LEFT {
                                self.cur_h = left_edge - self.mem[(p + 4) as usize].s1;
                            } else {
                                self.cur_h = left_edge + self.mem[(p + 4) as usize].s1;
                            }
                            self.temp_ptr = p;
                            if self.node_type(p)? == VLIST_NODE {
                                self.vlist_out()?;
                            } else {
                                self.hlist_out()?;
                            }
                            self.dvi_h = save_h;
                            self.dvi_v = save_v;
                            if upwards {
                                self.cur_v = save_v - self.mem[(p + 3) as usize].s1;
                            } else {
                                self.cur_v = save_v + self.mem[(p + 2) as usize].s1;
                            }
                            self.cur_h = left_edge;
                        }
                    }
                    RULE_NODE => {
                        let mut rule_ht = self.mem[(p + 3) as usize].s1;
                        let rule_dp = self.mem[(p + 2) as usize].s1;
                        let mut rule_wd = self.mem[(p + 1) as usize].s1;
                        // fin_rule
                        if rule_wd == NULL_FLAG {
                            rule_wd = self.mem[(this_box + 1) as usize].s1;
                        }
                        rule_ht += rule_dp;
                        if upwards {
                            self.cur_v -= rule_ht;
                        } else {
                            self.cur_v += rule_ht;
                        }
                        if rule_ht > 0 && rule_wd > 0 {
                            if self.cur_dir == RIGHT_TO_LEFT {
                                self.cur_h -= rule_wd;
                            }
                            if self.cur_h != self.dvi_h {
                                self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                self.dvi_h = self.cur_h;
                            }
                            if self.cur_v != self.dvi_v {
                                self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                self.dvi_v = self.cur_v;
                            }
                            self.dvi_out(PUT_RULE)?;
                            self.dvi_four(rule_ht)?;
                            self.dvi_four(rule_wd)?;
                            self.cur_h = left_edge;
                        }
                    }
                    WHATSIT_NODE => {
                        // §1403: whatsit in a vlist.
                        match self.node_subtype(p)? {
                            GLYPH_NODE => {
                                self.cur_v += self.mem[(p + 3) as usize].s1;
                                self.cur_h = left_edge;
                                if self.cur_h != self.dvi_h {
                                    self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                    self.dvi_h = self.cur_h;
                                }
                                if self.cur_v != self.dvi_v {
                                    self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                    self.dvi_v = self.cur_v;
                                }
                                let f = i32::from(self.mem[(p + 4) as usize].b16_s2());
                                self.dvi_set_font(f)?;
                                self.dvi_out(SET_GLYPHS)?;
                                self.dvi_four(0)?;
                                self.dvi_two(1)?;
                                self.dvi_four(0)?;
                                self.dvi_four(0)?;
                                self.dvi_two(self.mem[(p + 4) as usize].b16_s1())?;
                                self.cur_v += self.mem[(p + 2) as usize].s1;
                                self.cur_h = left_edge;
                            }
                            PIC_NODE | PDF_NODE => {
                                let save_h = self.dvi_h;
                                let save_v = self.dvi_v;
                                self.cur_v += self.mem[(p + 3) as usize].s1;
                                self.pic_out(p)?;
                                self.dvi_h = save_h;
                                self.dvi_v = save_v;
                                self.cur_v = save_v + self.mem[(p + 2) as usize].s1;
                                self.cur_h = left_edge;
                            }
                            PDF_SAVE_POS_NODE => {
                                self.pdf_last_x_pos = self.cur_h + 4_736_286;
                                self.pdf_last_y_pos = self.cur_page_height - self.cur_v - 4_736_286;
                            }
                            _ => {
                                self.out_what(p)?;
                            }
                        }
                    }
                    GLUE_NODE => {
                        // §656: move down or output leaders.
                        let spec = self.mem[(p + 1) as usize].s0;
                        let mut rule_ht = self.mem[(spec + 1) as usize].s1 - cur_g;

                        if g_sign != NORMAL {
                            if g_sign == STRETCHING {
                                if self.mem[spec as usize].b16_s1() == g_order {
                                    cur_glue += f64::from(self.mem[(spec + 2) as usize].s1);
                                    let glue_temp = (self.mem[(this_box + 6) as usize].gr()
                                        * cur_glue)
                                        .clamp(-GLUE_LIMIT, GLUE_LIMIT);
                                    cur_g = tex_round(glue_temp);
                                }
                            } else if self.mem[spec as usize].b16_s0() == g_order {
                                cur_glue -= f64::from(self.mem[(spec + 3) as usize].s1);
                                let glue_temp = (self.mem[(this_box + 6) as usize].gr() * cur_glue)
                                    .clamp(-GLUE_LIMIT, GLUE_LIMIT);
                                cur_g = tex_round(glue_temp);
                            }
                        }
                        rule_ht += cur_g;

                        if i32::from(self.node_subtype(p)?) >= A_LEADERS {
                            // §657: output leaders in a vlist.
                            let leader_box = self.mem[(p + 1) as usize].s1;
                            if self.node_type(leader_box)? == RULE_NODE {
                                let mut rule_wd = self.mem[(leader_box + 1) as usize].s1;
                                // fin_rule with rule_dp = 0: `rh` stays
                                // the accumulated glue distance.
                                let rh = rule_ht;
                                if rule_wd == NULL_FLAG {
                                    rule_wd = self.mem[(this_box + 1) as usize].s1;
                                }
                                if upwards {
                                    self.cur_v -= rh;
                                } else {
                                    self.cur_v += rh;
                                }
                                if rh > 0 && rule_wd > 0 {
                                    if self.cur_dir == RIGHT_TO_LEFT {
                                        self.cur_h -= rule_wd;
                                    }
                                    if self.cur_h != self.dvi_h {
                                        self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                        self.dvi_h = self.cur_h;
                                    }
                                    if self.cur_v != self.dvi_v {
                                        self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                        self.dvi_v = self.cur_v;
                                    }
                                    self.dvi_out(PUT_RULE)?;
                                    self.dvi_four(rh)?;
                                    self.dvi_four(rule_wd)?;
                                    self.cur_h = left_edge;
                                }
                                break 'dispatch;
                            }

                            let leader_ht = self.mem[(leader_box + 3) as usize].s1
                                + self.mem[(leader_box + 2) as usize].s1;
                            if leader_ht > 0 && rule_ht > 0 {
                                rule_ht += 10;
                                let edge = self.cur_v + rule_ht;
                                let mut lx = 0i32;

                                // §658: position of the first leader box.
                                if i32::from(self.node_subtype(p)?) == A_LEADERS {
                                    let save_v = self.cur_v;
                                    self.cur_v = top_edge
                                        + leader_ht * ((self.cur_v - top_edge) / leader_ht);
                                    if self.cur_v < save_v {
                                        self.cur_v += leader_ht;
                                    }
                                } else {
                                    let lq = rule_ht / leader_ht;
                                    let lr = rule_ht % leader_ht;
                                    if self.node_subtype(p)? == C_LEADERS {
                                        self.cur_v += lr / 2;
                                    } else {
                                        lx = lr / (lq + 1);
                                        self.cur_v += (lr - (lq - 1) * lx) / 2;
                                    }
                                }

                                while self.cur_v + leader_ht <= edge {
                                    // §659: output a leader box at cur_v.
                                    if self.cur_dir == RIGHT_TO_LEFT {
                                        self.cur_h =
                                            left_edge - self.mem[(leader_box + 4) as usize].s1;
                                    } else {
                                        self.cur_h =
                                            left_edge + self.mem[(leader_box + 4) as usize].s1;
                                    }
                                    if self.cur_h != self.dvi_h {
                                        self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
                                        self.dvi_h = self.cur_h;
                                    }
                                    let save_h = self.dvi_h;
                                    self.cur_v += self.mem[(leader_box + 3) as usize].s1;
                                    if self.cur_v != self.dvi_v {
                                        self.movement(self.cur_v - self.dvi_v, DOWN1)?;
                                        self.dvi_v = self.cur_v;
                                    }
                                    let save_v = self.dvi_v;
                                    self.temp_ptr = leader_box;
                                    let outer_doing_leaders = self.doing_leaders;
                                    self.doing_leaders = true;
                                    if self.node_type(leader_box)? == VLIST_NODE {
                                        self.vlist_out()?;
                                    } else {
                                        self.hlist_out()?;
                                    }
                                    self.doing_leaders = outer_doing_leaders;
                                    self.dvi_v = save_v;
                                    self.dvi_h = save_h;
                                    self.cur_h = left_edge;
                                    self.cur_v = save_v - self.mem[(leader_box + 3) as usize].s1
                                        + leader_ht
                                        + lx;
                                }
                                self.cur_v = edge - 10;
                                break 'dispatch;
                            }
                        }
                        // move_past
                        if upwards {
                            self.cur_v -= rule_ht;
                        } else {
                            self.cur_v += rule_ht;
                        }
                    }
                    KERN_NODE => {
                        if upwards {
                            self.cur_v -= self.mem[(p + 1) as usize].s1;
                        } else {
                            self.cur_v += self.mem[(p + 1) as usize].s1;
                        }
                    }
                    _ => {}
                }
            }

            p = self.mem[p as usize].s1;
        }

        self.synctex_tsilv(this_box);
        self.prune_movements(save_loc)?;
        if self.cur_s > 0 {
            self.dvi_pop(save_loc)?;
        }
        self.cur_s -= 1;
        Ok(())
    }

    // ---- top-level page emission ------------------------------------

    /// Byte `i` of a PIC node's stored path (`PIC_NODE_path`).
    fn pic_path_byte(&self, p: i32, i: i32) -> Result<u8, TeXMouthError> {
        let word = self.mem[(p + PIC_NODE_SIZE + i / 8) as usize];
        let half = if i % 8 < 4 { word.s0 } else { word.s1 };
        Ok((half >> ((i % 4) * 8)) as u8)
    }

    /// Emits a `pdf:image ...` special for a PIC/PDF node (`pic_out`).
    pub(crate) fn pic_out(&mut self, p: i32) -> Result<(), TeXMouthError> {
        if self.cur_h != self.dvi_h {
            self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
            self.dvi_h = self.cur_h;
        }
        if self.cur_v != self.dvi_v {
            self.movement(self.cur_v - self.dvi_v, DOWN1)?;
            self.dvi_v = self.cur_v;
        }

        let mut text = String::from("pdf:image matrix ");
        for word in 0..3 {
            text.push_str(&format_scaled(self.mem[(p + 5 + word) as usize].s0));
            text.push(' ');
            text.push_str(&format_scaled(self.mem[(p + 5 + word) as usize].s1));
            text.push(' ');
        }
        text.push_str("page ");
        text.push_str(&i32::from(self.mem[(p + 4) as usize].b16_s0()).to_string());
        text.push(' ');
        text.push_str(match i32::from(self.mem[(p + 8) as usize].b16_s1()) {
            1 => "pagebox cropbox ",
            2 => "pagebox mediabox ",
            3 => "pagebox bleedbox ",
            5 => "pagebox artbox ",
            4 => "pagebox trimbox ",
            _ => "",
        });
        text.push('(');
        let path_len = i32::from(self.mem[(p + 4) as usize].b16_s1());
        for i in 0..path_len {
            text.push(char::from(self.pic_path_byte(p, i)?));
        }
        text.push(')');

        let bytes = text.into_bytes();
        if bytes.len() < 256 {
            self.dvi_out(XXX1)?;
            self.dvi_out(bytes.len() as u8)?;
        } else {
            self.dvi_out(XXX4)?;
            self.dvi_four(bytes.len() as i32)?;
        }
        for byte in bytes {
            self.dvi_out(byte)?;
        }
        Ok(())
    }

    /// Emits a `\special` node's text as an `xxx` opcode (`special_out`).
    pub(crate) fn special_out(&mut self, p: i32) -> Result<(), TeXMouthError> {
        if self.cur_h != self.dvi_h {
            self.movement(self.cur_h - self.dvi_h, RIGHT1)?;
            self.dvi_h = self.cur_h;
        }
        if self.cur_v != self.dvi_v {
            self.movement(self.cur_v - self.dvi_v, DOWN1)?;
            self.dvi_v = self.cur_v;
        }
        self.doing_special = true;
        let start = self.stored_token_list_start(self.word_at(p, 1)?.s1)?;
        let text = self.token_list_to_string(start)?;
        let bytes = text.into_bytes();
        if bytes.len() < 256 {
            self.dvi_out(XXX1)?;
            self.dvi_out(bytes.len() as u8)?;
        } else {
            self.dvi_out(XXX4)?;
            self.dvi_four(bytes.len() as i32)?;
        }
        for byte in bytes {
            self.dvi_out(byte)?;
        }
        self.doing_special = false;
        Ok(())
    }

    /// Executes a deferred `\write` at shipout time (`write_out`):
    /// re-scans the stored token list with expansion, then routes the
    /// resulting text to the named stream, the log, or a recorded
    /// shell-escape request.
    pub(crate) fn write_out(&mut self, p: i32) -> Result<(), TeXMouthError> {
        // Sandwich the stored token list in braces and \endwrite so
        // `scan_toks` stops exactly at its end (C §1411).
        let q = self.get_avail()?;
        self.mem[q as usize].s0 = RIGHT_BRACE_TOKEN + i32::from(b'}');
        let r = self.get_avail()?;
        self.mem[q as usize].s1 = r;
        self.mem[r as usize].s0 = CS_TOKEN_FLAG + END_WRITE_ID;
        self.begin_token_list(q, INSERTED)?;
        self.begin_token_list(self.mem[(p + 1) as usize].s1, WRITE_TEXT)?;
        let q = self.get_avail()?;
        self.mem[q as usize].s0 = crate::scanner::LEFT_BRACE_TOKEN + i32::from(b'{');
        self.begin_token_list(q, INSERTED)?;

        let old_mode = self.mode;
        self.mode = 0;
        self.cur_cs = self.write_loc;
        self.scan_toks(false, true)?;
        self.get_token()?;

        if self.cur_tok != CS_TOKEN_FLAG + END_WRITE_ID {
            self.report_error(
                "Unbalanced write command",
                &[
                    "I can't handle that very well; good luck.",
                    "On this page there's a \\write with fewer real {'s than }'s.",
                ],
            );
            while self.cur_tok != CS_TOKEN_FLAG + END_WRITE_ID {
                self.get_token()?;
            }
        }

        self.mode = old_mode;
        self.end_token_list()?;

        let j = self.mem[(p + 1) as usize].s0;
        let list_start = self.stored_token_list_start(self.def_ref)?;
        let text = self.token_list_to_string(list_start)?;
        self.flush_list(self.def_ref)?;

        if j == 18 {
            if self.shell_escape_enabled {
                self.shell_escape_requests.push(text);
            } else {
                self.transcript
                    .push(format!("runsystem({text})...disabled."));
            }
        } else if (0..16).contains(&j) && self.write_stream_open(j) {
            if let Some(Some(stream)) = self.write_streams.get_mut(j as usize) {
                stream.text.push_str(&text);
                stream.text.push('\n');
            }
        } else {
            self.transcript.push(text);
        }
        Ok(())
    }

    /// Ships out box `p` as a page of XDV output (`ship_out`, C §660):
    /// page-size validation, preamble on first page, `bop`/counters/
    /// `last_bop`, the `pdf:pagesize` special, recursive list emission,
    /// `eop`, and the LR anomaly check.
    pub fn ship_out(&mut self, p: i32) -> Result<(), TeXMouthError> {
        let mag = self.int_par(INT_PAR_MAG)?;
        self.synctex_sheet(mag);

        if self.job_name.is_none() {
            self.job_name = Some("texput".to_string());
        }

        if self.int_par(INT_PAR_TRACING_OUTPUT)? > 0 {
            self.transcript
                .push("Completed box being shipped out".to_string());
        }

        // "[1" page-counter progress line.
        let mut j = 9usize;
        while j > 0 && self.count(j)? == 0 {
            j -= 1;
        }
        let mut page_numbers = String::from("[");
        for k in 0..=j {
            page_numbers.push_str(&self.count(k)?.to_string());
            if k < j {
                page_numbers.push('.');
            }
        }
        self.transcript.push(page_numbers);

        // §663: huge-page check.
        let (height, depth, width) = (
            self.mem[(p + 3) as usize].s1,
            self.mem[(p + 2) as usize].s1,
            self.mem[(p + 1) as usize].s1,
        );
        let v_offset = self.dimen_par(DIMEN_PAR_V_OFFSET)?;
        let h_offset = self.dimen_par(DIMEN_PAR_H_OFFSET)?;
        if height > MAX_HALFWORD
            || depth > MAX_HALFWORD
            || height + depth + v_offset > MAX_HALFWORD
            || width + h_offset > MAX_HALFWORD
        {
            self.report_error(
                "Huge page cannot be shipped out",
                &[
                    "more than 18 feet wide, so I suspect something went wrong.",
                    "The page just created is more than 18 feet tall or",
                ],
            );
        } else {
            if height + depth + v_offset > self.max_v {
                self.max_v = height + depth + v_offset;
            }
            if width + h_offset > self.max_h {
                self.max_h = width + h_offset;
            }

            // §637/1405: initialize and calculate page dimensions.
            self.dvi_h = 0;
            self.dvi_v = 0;
            self.cur_h = h_offset;
            self.dvi_f = 0;

            self.cur_h_offset = h_offset + ONE_INCH;
            self.cur_v_offset = v_offset + ONE_INCH;

            let pdf_w = self.dimen_par(DIMEN_PAR_PDF_PAGE_WIDTH)?;
            let pdf_h = self.dimen_par(DIMEN_PAR_PDF_PAGE_HEIGHT)?;
            self.cur_page_width = if pdf_w != 0 {
                pdf_w
            } else {
                width + 2 * self.cur_h_offset
            };
            self.cur_page_height = if pdf_h != 0 {
                pdf_h
            } else {
                height + depth + 2 * self.cur_v_offset
            };

            self.ensure_output_open();

            if self.total_pages == 0 {
                self.dvi_out(PRE)?;
                self.dvi_out(if self.semantic_pagination_enabled {
                    SPX_ID_BYTE
                } else {
                    XDV_ID_BYTE
                })?;
                self.dvi_four(25_400_000)?;
                self.dvi_four(473_628_672)?;
                self.prepare_mag()?;
                self.dvi_four(self.int_par(INT_PAR_MAG)?)?;
                let comment = b"tectonic";
                self.dvi_out(comment.len() as u8)?;
                for &byte in comment {
                    self.dvi_out(byte)?;
                }
            }

            let page_loc = self.dvi_offset + self.dvi_ptr;
            self.dvi_out(BOP)?;
            for k in 0..10 {
                let value = self.count(k)?;
                self.dvi_four(value)?;
            }
            self.dvi_four(self.last_bop)?;
            self.last_bop = page_loc;

            // Unilateral `pdf:pagesize` special.
            let mut special = String::from("pdf:pagesize ");
            if pdf_w <= 0 || pdf_h <= 0 {
                special.push_str("default");
            } else {
                special.push_str("width ");
                special.push_str(&format_scaled(pdf_w));
                special.push_str("pt height ");
                special.push_str(&format_scaled(pdf_h));
                special.push_str("pt");
            }
            let bytes = special.into_bytes();
            self.dvi_out(XXX1)?;
            self.dvi_out(bytes.len() as u8)?;
            for byte in bytes {
                self.dvi_out(byte)?;
            }

            self.cur_v = height + v_offset;
            self.temp_ptr = p;
            if self.node_type(p)? == VLIST_NODE {
                self.vlist_out()?;
            } else {
                self.hlist_out()?;
            }

            self.dvi_out(EOP)?;
            self.total_pages += 1;
            self.cur_s = -1;
        }

        // §1518: check for LR anomalies at the end of ship_out.
        if self.lr_problems > 0 {
            self.transcript.push(format!(
                "\\endL or \\endR problem ({} missing, {} extra)",
                self.lr_problems / 10_000,
                self.lr_problems % 10_000
            ));
            self.lr_problems = 0;
        }
        if self.lr_ptr != TEX_NULL || self.cur_dir != LEFT_TO_RIGHT {
            return Err(self.confusion("LR3"));
        }
        if self.int_par(INT_PAR_TRACING_OUTPUT)? <= 0 {
            self.transcript.push("]".to_string());
        }
        self.dead_cycles = 0;
        self.flush_node_list(p)?;
        self.shipped_pages.push(p);
        self.synctex_teehs();
        Ok(())
    }

    /// Writes the postamble and finishes the XDV stream
    /// (`finalize_dvi_file`): drains pending `pop`/`eop`, emits `post`,
    /// the font definitions, `post_post`, the id byte, padding, and the
    /// final flush into [`TeXEngineState::dvi_bytes`].
    pub fn finalize_dvi_file(&mut self) -> Result<(), TeXMouthError> {
        while self.cur_s > -1 {
            if self.cur_s > 0 {
                self.dvi_out(POP)?;
            } else {
                self.dvi_out(EOP)?;
                self.total_pages += 1;
            }
            self.cur_s -= 1;
        }

        if self.total_pages == 0 {
            self.transcript.push("No pages of output.".to_string());
            return Ok(());
        }
        if self.cur_s == -2 {
            return Ok(());
        }

        self.dvi_out(POST)?;
        self.dvi_four(self.last_bop)?;
        self.last_bop = self.dvi_offset + self.dvi_ptr - 5;
        self.dvi_four(25_400_000)?;
        self.dvi_four(473_628_672)?;
        self.prepare_mag()?;
        self.dvi_four(self.int_par(INT_PAR_MAG)?)?;
        self.dvi_four(self.max_v)?;
        self.dvi_four(self.max_h)?;
        self.dvi_out((self.max_push / 256) as u8)?;
        self.dvi_out((self.max_push % 256) as u8)?;
        self.dvi_out(((self.total_pages / 256) % 256) as u8)?;
        self.dvi_out((self.total_pages % 256) as u8)?;

        for f in (1..self.font_table.len()).rev() {
            let used = self.font_record(f as i32).is_some_and(|r| r.used);
            if used {
                self.dvi_font_def(f as i32)?;
            }
        }
        self.font_table.truncate(1);

        self.dvi_out(POST_POST)?;
        self.dvi_four(self.last_bop)?;
        self.dvi_out(if self.semantic_pagination_enabled {
            SPX_ID_BYTE
        } else {
            XDV_ID_BYTE
        })?;

        let mut k = 4 + (DVI_BUF_SIZE as i32 - self.dvi_ptr) % 4;
        while k > 0 {
            self.dvi_out(223)?;
            k -= 1;
        }

        if self.dvi_limit == HALF_BUF as i32 {
            self.write_to_dvi(HALF_BUF as i32, DVI_BUF_SIZE as i32 - 1);
        }
        if i64::from(self.dvi_ptr) > TEX_INFINITY - i64::from(self.dvi_offset) {
            self.cur_s = -2;
            return Err(self.confusion("dvi length exceeds 0x7FFFFFFF"));
        }
        if self.dvi_ptr > 0 {
            let end = self.dvi_ptr - 1;
            self.write_to_dvi(0, end);
        }

        self.transcript.push(format!(
            "Output written on {} ({} {}, {} bytes).",
            self.output_file_name,
            self.total_pages,
            if self.total_pages == 1 {
                "page"
            } else {
                "pages"
            },
            self.dvi_offset + self.dvi_ptr
        ));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FontCharMetric, FontRecord, DOWN1, EOP, FNT_NUM_0, RIGHT1, XDV_ID_BYTE};

    fn engine() -> TeXEngineState {
        TeXEngineState::new(4_096)
    }

    fn buf(state: &TeXEngineState) -> &[u8] {
        &state.dvi_buf[..state.dvi_ptr as usize]
    }

    #[test]
    fn dvi_out_and_swap_flush_completed_halves() {
        let mut state = engine();
        state.dvi_out(0xAB).unwrap();
        assert_eq!(state.dvi_buf[0], 0xAB);
        assert_eq!(state.dvi_ptr, 1);

        // Fill to the end of the buffer; the swap flushes the low half
        // only — the byte written at index DVI_BUF_SIZE-1 stays staged in
        // the high half until the next swap (C's alternating halves).
        state.dvi_ptr = state.dvi_limit - 1;
        state.dvi_out(0xCD).unwrap();
        assert_eq!(state.dvi_bytes.len(), HALF_BUF);
        assert_eq!(state.dvi_bytes[0], 0xAB);
        assert_eq!(state.dvi_buf[DVI_BUF_SIZE - 1], 0xCD);
        assert_eq!(state.dvi_ptr, 0);
        assert_eq!(state.dvi_limit, HALF_BUF as i32);
        assert_eq!(state.dvi_offset, DVI_BUF_SIZE as i32);
        assert_eq!(state.dvi_gone, HALF_BUF as i32);

        // Refill the low half; the next swap flushes the high half
        // (including the staged 0xCD) and restores the full limit.
        state.dvi_ptr = state.dvi_limit - 1;
        state.dvi_out(0xEF).unwrap();
        assert_eq!(state.dvi_bytes.len(), DVI_BUF_SIZE);
        assert_eq!(state.dvi_bytes[DVI_BUF_SIZE - 1], 0xCD);
        assert_eq!(state.dvi_limit, DVI_BUF_SIZE as i32);
        assert_eq!(state.dvi_ptr, HALF_BUF as i32);
        assert_eq!(state.dvi_gone, DVI_BUF_SIZE as i32);
    }

    #[test]
    fn dvi_two_and_four_emit_big_endian_with_sign_folding() {
        let mut state = engine();
        state.dvi_two(0x1234).unwrap();
        state.dvi_four(0x0102_0304).unwrap();
        state.dvi_four(-1).unwrap();
        assert_eq!(
            buf(&state),
            &[0x12, 0x34, 0x01, 0x02, 0x03, 0x04, 0xFF, 0xFF, 0xFF, 0xFF]
        );
    }

    #[test]
    fn dvi_pop_reclaims_the_push_byte_when_nothing_was_emitted() {
        let mut state = engine();
        state.dvi_out(PUSH).unwrap();
        let save_loc = state.dvi_offset + state.dvi_ptr;
        state.dvi_pop(save_loc).unwrap();
        assert_eq!(state.dvi_ptr, 0);

        state.dvi_out(PUSH).unwrap();
        state.dvi_out(0x00).unwrap();
        let before = state.dvi_ptr;
        state.dvi_pop(save_loc).unwrap();
        assert_eq!(state.dvi_ptr, before + 1);
        assert_eq!(state.dvi_buf[(state.dvi_ptr - 1) as usize], POP);
    }

    #[test]
    fn movement_emits_short_forms_and_reuses_distances() {
        let mut state = engine();
        state.movement(100, RIGHT1).unwrap();
        assert_eq!(buf(&state), &[RIGHT1, 100]);

        // The same distance reuses the record: the earlier right1 byte is
        // patched to w1 in place (`dvi_buf[k] += 5`) and w0 (o + 4) is
        // emitted.
        state.movement(100, RIGHT1).unwrap();
        assert_eq!(buf(&state), &[RIGHT1 + 5, 100, RIGHT1 + 4]);

        // A new small distance is a fresh right1.
        state.movement(-50, RIGHT1).unwrap();
        assert_eq!(buf(&state)[3..], [RIGHT1, 206]);

        // Two-, three-, and four-byte encodings.
        state.movement(300, RIGHT1).unwrap();
        assert_eq!(&buf(&state)[5..], &[RIGHT1 + 1, 1, 44]);
        state.movement(70_000, RIGHT1).unwrap();
        assert_eq!(&buf(&state)[8..], &[RIGHT1 + 2, 1, 17, 112]);
        state.movement(0x80_0000, RIGHT1).unwrap();
        assert_eq!(&buf(&state)[12..], &[RIGHT1 + 3, 0, 128, 0, 0]);
    }

    #[test]
    fn movement_reuses_down_distances_as_y_opcodes() {
        let mut state = engine();
        state.movement(512, DOWN1).unwrap();
        assert_eq!(buf(&state), &[DOWN1 + 1, 2, 0]);
        // Same down movement again becomes y0 (DOWN1 + 4).
        state.movement(512, DOWN1).unwrap();
        assert_eq!(buf(&state)[3], DOWN1 + 4);
    }

    #[test]
    fn ship_out_empty_hbox_writes_preamble_bop_and_eop() {
        let mut state = engine();
        let boxed = state.new_null_box().unwrap();
        state.mem[(boxed + 1) as usize].s1 = 10 * 65_536;
        state.mem[(boxed + 2) as usize].s1 = 2 * 65_536;
        state.mem[(boxed + 3) as usize].s1 = 8 * 65_536;
        state.set_count(0, 1).unwrap();

        state.ship_out(boxed).unwrap();

        assert_eq!(state.total_pages, 1);
        assert_eq!(state.shipped_pages.len(), 1);
        assert_eq!(state.output_file_name, "texput.xdv");
        assert!(state.output_file_opened);

        let out = buf(&state);
        // PRE, id byte, numerator, denominator, mag, comment length, comment.
        assert_eq!(out[0], PRE);
        assert_eq!(out[1], XDV_ID_BYTE);
        assert_eq!(&out[2..6], &25_400_000i32.to_be_bytes());
        assert_eq!(&out[6..10], &473_628_672i32.to_be_bytes());
        assert_eq!(&out[10..14], &1000i32.to_be_bytes());
        assert_eq!(out[14], 8);
        assert_eq!(&out[15..23], b"tectonic");

        // BOP + ten counters + backward pointer (-1 first page).
        assert_eq!(out[23], BOP);
        assert_eq!(&out[24..28], &1i32.to_be_bytes()); // \count0
        assert_eq!(&out[64..68], &(-1i32).to_be_bytes());

        // Unilateral pdf:pagesize special, then EOP (empty hlist).
        assert_eq!(out[68], XXX1);
        let len = out[69] as usize;
        assert_eq!(&out[70..70 + len], b"pdf:pagesize default");
        assert_eq!(out[70 + len], EOP);
        assert_eq!(out.len(), 71 + len);
    }

    #[test]
    fn ship_out_emits_font_selection_and_character_bytes() {
        let mut state = engine();
        state.font_table.push(FontRecord::default());
        {
            let record = state.font_table.get_mut(1).unwrap();
            record.bc = 0;
            record.ec = 0x10FFFF;
            record.name = b"test".iter().map(|&c| i32::from(c)).collect();
        }
        state.set_font_char_metric(
            1,
            u32::from(b'a'),
            FontCharMetric {
                width: 10,
                height: 7,
                depth: 0,
                italic: 0,
            },
        );
        state.set_font_char_metric(
            1,
            u32::from(b'b'),
            FontCharMetric {
                width: 12,
                height: 7,
                depth: 0,
                italic: 0,
            },
        );

        let boxed = state.new_null_box().unwrap();
        let c1 = state.new_character(1, i32::from(b'a')).unwrap();
        let c2 = state.new_character(1, i32::from(b'b')).unwrap();
        state.mem[c1 as usize].s1 = c2;
        state.mem[(boxed + 5) as usize].s1 = c1;
        state.mem[(boxed + 1) as usize].s1 = 22;
        state.mem[(boxed + 3) as usize].s1 = 7;

        state.ship_out(boxed).unwrap();

        let out = buf(&state);
        // After the pagesize special: a down movement for the baseline,
        // fnt_num_0 (internal font 1 → DVI font 0), then 'a' and 'b'.
        let special_end = 70 + out[69] as usize;
        let tail = &out[special_end..];
        let fnt_pos = tail
            .iter()
            .position(|&b| b == FNT_NUM_0)
            .expect("font selection opcode");
        assert_eq!(tail[fnt_pos + 1], b'a');
        assert_eq!(tail[fnt_pos + 2], b'b');
        assert_eq!(*tail.last().unwrap(), EOP);
        assert!(state.font_record(1).unwrap().used);
    }

    #[test]
    fn finalize_writes_postamble_and_four_byte_padding() {
        let mut state = engine();
        let boxed = state.new_null_box().unwrap();
        state.ship_out(boxed).unwrap();
        state.finalize_dvi_file().unwrap();

        let bytes = &state.dvi_bytes;
        assert_eq!(bytes.len() % 4, 0);
        // Trailer ends in 223 fill bytes; the id byte precedes them.
        assert_eq!(*bytes.last().unwrap(), 223);
        let post_post = bytes
            .iter()
            .rposition(|&b| b == POST_POST)
            .expect("post_post opcode");
        assert_eq!(bytes[post_post + 5], XDV_ID_BYTE);
        assert!(bytes[post_post + 6..].iter().all(|&b| b == 223));
        // POST appears once, before post_post.
        assert!(bytes.contains(&POST));
        assert!(state
            .transcript
            .iter()
            .any(|l| l.contains("Output written")));
    }

    #[test]
    fn finalize_without_pages_reports_no_output() {
        let mut state = engine();
        state.finalize_dvi_file().unwrap();
        assert!(state
            .transcript
            .iter()
            .any(|l| l.contains("No pages of output")));
        assert!(state.dvi_bytes.is_empty());
    }

    #[test]
    fn prepare_mag_clamps_and_locks_magnification() {
        let mut state = engine();
        state.set_int_par(INT_PAR_MAG, 5000).unwrap();
        state.prepare_mag().unwrap();
        assert_eq!(state.mag_set, 5000);

        // Changing \mag after it is locked reverts the change.
        state.set_int_par(INT_PAR_MAG, 2000).unwrap();
        state.prepare_mag().unwrap();
        assert_eq!(state.int_par(INT_PAR_MAG).unwrap(), 5000);
        assert_eq!(state.error_count, 1);

        // Illegal values clamp to 1000.
        let mut state = engine();
        state.set_int_par(INT_PAR_MAG, 0).unwrap();
        state.prepare_mag().unwrap();
        assert_eq!(state.int_par(INT_PAR_MAG).unwrap(), 1000);
        assert_eq!(state.mag_set, 1000);
    }
}
