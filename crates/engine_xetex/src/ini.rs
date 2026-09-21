//! Pure-Rust port of `xetex-ini.c` — INITEX initialization, control-sequence
//! and primitive hash tables, hyphenation-trie packing, and the format-file
//! support machinery.
//!
//! The live engine keeps compact, Rust-shaped tables (`cs_table`, compact
//! `eqtb`, `HashMap` hyphenation dictionaries). This module additionally
//! maintains the C-layout *image* (`fmt_hash`, `fmt_eqtb`, `prim`, the packed
//! hyphenation trie) that `store_fmt_file`/`load_fmt_file` serialize so the
//! emitted format files match the upstream binary layout exactly.

use crate::primitives::{primitive_defs, PrimitiveExtraInit};
use crate::scan::INT_VAL;
use crate::scanner::{TeXMouthError, EVERY_JOB_TEXT, NEW_LINE, TOKEN_LIST};
use crate::state::*;

/// Highest `mem` index available to single-word token nodes; the
/// `HI_MEM_STAT_USAGE` slots below `MEM_TOP` are reserved for statistics.
pub(crate) const HI_MEM_STAT_USAGE: i32 = 14;
/// `if_limit` value for a fully-processed conditional (`NORMAL`).
pub(crate) const NORMAL_IF_LIMIT: u8 = 0;
/// `do_marks` action that destroys a mark list (`DESTROY_MARKS`).
pub(crate) const DO_MARKS_DESTROY: u8 = 0;

impl TeXEngineState {
    // === String-pool helpers ===

    /// Pending-string length (`cur_length`).
    pub(crate) fn cur_length(&self) -> i32 {
        self.pool_ptr
            - self
                .str_start
                .get((self.str_ptr - TOO_BIG_CHAR) as usize)
                .copied()
                .unwrap_or(0)
    }

    /// Builds a C-layout eqtb image word (`level`, `cmd`, `equiv`).
    pub(crate) fn fmt_eqtb_word(level: u16, cmd: u8, equiv: i32) -> MemoryWord {
        let mut w = MemoryWord::default();
        w.set_b16_s0(level);
        w.set_b16_s1(u16::from(cmd));
        w.s1 = equiv;
        w
    }

    /// Interns `units` as a pool string and returns its `str_number`
    /// (`maketexstring`).
    pub(crate) fn make_tex_string_units(&mut self, units: &[i32]) -> Result<i32, TeXMemoryError> {
        for &u in units {
            self.pool_push(u);
        }
        self.make_string()
    }

    // === Control-sequence hash image (`id_lookup`) ===

    /// C `id_lookup` — locates or inserts the name held in
    /// `buffer[j..j + l]` in the C-layout `fmt_hash` image, maintaining the
    /// `hash_used`/`hash_high` allocation chains exactly.
    pub(crate) fn fmt_id_lookup(&mut self, j: usize, l: usize) -> Result<i32, TeXMouthError> {
        let mut h: i32 = 0;
        for k in j..(j + l) {
            h = h + h + self.buffer[k];
            while h >= C_HASH_PRIME {
                h -= C_HASH_PRIME;
            }
        }
        let mut p = h + C_HASH_BASE;
        let mut ll = l as i32;
        for d in 0..l {
            if self.buffer[j + d] >= 65536 {
                ll += 1;
            }
        }
        loop {
            let (link, text) = {
                let e = self.fmt_hash.get(&p).copied().unwrap_or_default();
                (e.lh, e.rh)
            };
            if text > 0 && self.length(text) == ll && self.str_eq_buf(text, j) {
                return Ok(p);
            }
            if link == 0 {
                if self.no_new_control_sequence {
                    return Ok(C_UNDEFINED_CONTROL_SEQUENCE);
                }
                if text > 0 {
                    if self.hash_high < self.hash_extra {
                        self.hash_high += 1;
                        let np = self.hash_high + C_EQTB_SIZE;
                        self.fmt_hash.entry(p).or_default().lh = np;
                        p = np;
                    } else {
                        loop {
                            if self.hash_used == C_HASH_BASE {
                                self.overflow("hash size", C_HASH_SIZE + self.hash_extra)?;
                            }
                            self.hash_used -= 1;
                            if self
                                .fmt_hash
                                .get(&self.hash_used)
                                .map(|e| e.rh)
                                .unwrap_or(0)
                                == 0
                            {
                                break;
                            }
                        }
                        self.fmt_hash.entry(p).or_default().lh = self.hash_used;
                        p = self.hash_used;
                    }
                }
                if self.pool_ptr + ll > self.pool_size {
                    self.overflow("pool size", self.pool_size - self.init_pool_ptr)?;
                }
                let d = self.cur_length();
                while self.pool_ptr
                    > self
                        .str_start
                        .get((self.str_ptr - TOO_BIG_CHAR) as usize)
                        .copied()
                        .unwrap_or(0)
                {
                    self.pool_ptr -= 1;
                    let src = self.str_pool[self.pool_ptr as usize];
                    let idx = (self.pool_ptr + l as i32) as usize;
                    if idx >= self.str_pool.len() {
                        self.str_pool.resize(idx + 1, 0);
                    }
                    self.str_pool[idx] = src;
                }
                for k in j..(j + l) {
                    let b = self.buffer[k];
                    if b < 65536 {
                        self.pool_push(b);
                    } else {
                        self.pool_push(0xD800 + (b - 65536) / 1024);
                        self.pool_push(0xDC00 + (b - 65536) % 1024);
                    }
                }
                let s = self.make_string()?;
                self.fmt_hash.entry(p).or_default().rh = s;
                self.pool_ptr += d;
                return Ok(p);
            }
            p = link;
        }
    }

    // === Primitive hash table (`prim_lookup`, `primitive`) ===

    /// C `prim_lookup` — locates or inserts `s` (a bare char or a
    /// `str_number`) in the `prim` table.
    pub(crate) fn fmt_prim_lookup(&mut self, s: i32) -> Result<i32, TeXMouthError> {
        let mut l: i32 = 0;
        let mut p: i32;
        if s <= BIGGEST_CHAR {
            if s < 0 {
                return Ok(C_UNDEFINED_PRIMITIVE);
            }
            p = (s % C_PRIM_PRIME) + 1;
        } else {
            let j = self
                .str_start
                .get((s - TOO_BIG_CHAR) as usize)
                .copied()
                .unwrap_or(0) as usize;
            l = if s == self.str_ptr {
                self.cur_length()
            } else {
                self.length(s)
            };
            let mut h = self.str_pool.get(j).copied().unwrap_or(0);
            for k in (j + 1)..(j + l.max(0) as usize) {
                h = h + h + self.str_pool.get(k).copied().unwrap_or(0);
                while h >= C_PRIM_PRIME {
                    h -= C_PRIM_PRIME;
                }
            }
            p = h + 1;
        }
        loop {
            let (link, text) = {
                let e = self.prim[p as usize];
                (e.lh, e.rh)
            };
            if text > TOO_BIG_CHAR {
                if self.length(text - 1) == l && self.str_eq_str(text - 1, s) {
                    return Ok(p);
                }
            } else if text == 1 + s {
                return Ok(p);
            }
            if link == 0 {
                if self.no_new_control_sequence {
                    return Ok(C_UNDEFINED_PRIMITIVE);
                }
                if text > 0 {
                    loop {
                        if self.prim_used == C_PRIM_BASE {
                            self.overflow("primitive size", C_PRIM_SIZE)?;
                        }
                        self.prim_used -= 1;
                        if self.prim[self.prim_used as usize].rh == 0 {
                            break;
                        }
                    }
                    self.prim[p as usize].lh = self.prim_used;
                    p = self.prim_used;
                }
                self.prim[p as usize].rh = s + 1;
                return Ok(p);
            }
            p = link;
        }
    }

    /// C `primitive` — registers `ident` in the C-layout image (hash entry,
    /// eqtb entry, prim table, prim-eqtb entry). Returns the C eqtb position.
    pub(crate) fn fmt_primitive(
        &mut self,
        ident: &[i32],
        c: u8,
        o: i32,
    ) -> Result<i32, TeXMouthError> {
        let cur_val;
        let prim_val;
        if ident.len() > 1 {
            let s = self.make_tex_string_units(ident)?;
            let first = self.first as usize;
            if first + ident.len() > self.buf_size + 1 {
                self.overflow("buffer size", self.buf_size as i32)?;
            }
            if self.buffer.len() < first + ident.len() {
                self.buffer.resize(first + ident.len(), 0);
            }
            for (i, &u) in ident.iter().enumerate() {
                self.buffer[first + i] = u;
            }
            cur_val = self.fmt_id_lookup(first, ident.len())?;
            self.str_ptr -= 1;
            self.pool_ptr = self
                .str_start
                .get((self.str_ptr - TOO_BIG_CHAR) as usize)
                .copied()
                .unwrap_or(0);
            self.fmt_hash.entry(cur_val).or_default().rh = s;
            prim_val = self.fmt_prim_lookup(s)?;
        } else {
            cur_val = ident[0] + C_SINGLE_BASE;
            prim_val = self.fmt_prim_lookup(ident[0])?;
        }
        let word = Self::fmt_eqtb_word(LEVEL_ONE, c, o);
        self.fmt_eqtb.insert(cur_val, word);
        self.fmt_eqtb.insert(C_PRIM_EQTB_BASE + prim_val, word);
        Ok(cur_val)
    }

    /// C `initialize_primitives` — installs the vendored primitive table into
    /// the C-layout image, resolving the `xf_prim_init_*` extra actions and
    /// the fixed frozen entries.
    pub(crate) fn initialize_primitives_fmt(&mut self) -> Result<(), TeXMouthError> {
        let null_list = self.null_list();
        self.no_new_control_sequence = false;
        self.first = 0;
        for def in primitive_defs().expect("vendored XeTeX primitive table is valid") {
            let units: Vec<i32> = def.name.chars().map(|c| c as i32).collect();
            let cur_val = self.fmt_primitive(&units, def.command as u8, def.c_chr)?;
            match def.extra_init {
                PrimitiveExtraInit::None => {}
                PrimitiveExtraInit::Par => {
                    self.fmt_par_loc = cur_val;
                }
                PrimitiveExtraInit::Write => {
                    self.fmt_write_loc = cur_val;
                }
                PrimitiveExtraInit::Frozen(symbol) => {
                    let pos = c_frozen_position(symbol);
                    let s = self.make_tex_string_units(&units)?;
                    self.fmt_hash.entry(pos).or_default().rh = s;
                    if let Some(w) = self.fmt_eqtb.get(&cur_val).copied() {
                        self.fmt_eqtb.insert(pos, w);
                    }
                }
            }
        }

        // Fixed frozen entries installed after the primitive loop.
        let frozen: &[(&str, i32, u8, i32)] = &[
            (
                "endtemplate",
                C_FROZEN_END_TEMPLATE,
                CommandCode::EndTemplate as u8,
                null_list,
            ),
            (
                "endtemplate",
                C_FROZEN_ENDV,
                CommandCode::Endv as u8,
                null_list,
            ),
            (
                "notexpanded:",
                C_FROZEN_DONT_EXPAND,
                CommandCode::DontExpand as u8,
                0,
            ),
            (
                "primitive",
                C_FROZEN_PRIMITIVE,
                CommandCode::IgnoreSpaces as u8,
                1,
            ),
            (
                "inaccessible",
                C_FROZEN_PROTECTION,
                CommandCode::UndefinedCs as u8,
                0,
            ),
            (
                "endwrite",
                C_END_WRITE,
                CommandCode::OuterCall as u8,
                TEX_NULL,
            ),
        ];
        for &(name, pos, cmd, equiv) in frozen {
            let units: Vec<i32> = name.chars().map(|c| c as i32).collect();
            let s = self.make_tex_string_units(&units)?;
            self.fmt_hash.entry(pos).or_default().rh = s;
            self.fmt_eqtb
                .insert(pos, Self::fmt_eqtb_word(LEVEL_ONE, cmd, equiv));
        }
        self.no_new_control_sequence = true;
        Ok(())
    }

    // === Initialization ===

    /// C `initialize_more_variables` — re-asserts the scalar/list defaults
    /// shared by INITEX and production runs. Table-sized defaults live in
    /// `TeXEngineState::new`; this covers the deltas the constructor cannot
    /// know (pool state, interaction, error machinery).
    pub(crate) fn initialize_more_variables(&mut self) {
        self.interaction = ERROR_STOP_MODE;
        self.deletions_allowed = true;
        self.set_box_allowed = true;
        self.error_count = 0;
        self.help_ptr = 0;
        self.use_err_help = false;
        self.shown_mode = 0;
        self.page_contents = PAGE_EMPTY;
        self.last_glue = MAX_HALFWORD;
        self.last_penalty = 0;
        self.last_kern = 0;
        self.last_node_type = -1;
        self.no_new_control_sequence = true;
        self.prim = vec![TwoHalves::default(); (C_PRIM_SIZE + 1) as usize];
        self.save_ptr = 0;
        self.cur_level = LEVEL_ONE;
        self.cur_group = BOTTOM_LEVEL;
        self.cur_boundary = 0;
        self.max_save_stack = 0;
        self.mag_set = 0;
        self.expand_depth_count = 0;
        self.is_in_csname = false;
        self.cur_mark = [TEX_NULL; 5];
        self.cur_val = 0;
        self.cur_val_level = INT_VAL;
        self.radix = 0;
        self.cond_ptr = TEX_NULL;
        self.if_limit = NORMAL_IF_LIMIT;
        self.cur_if = 0;
        self.if_line = 0;
        self.max_v = 0;
        self.max_h = 0;
        self.max_push = 0;
        self.last_bop = -1;
        self.doing_leaders = false;
        self.dead_cycles = 0;
        self.adjust_tail = TEX_NULL;
        self.last_badness = 0;
        self.pre_adjust_tail = TEX_NULL;
        self.pack_begin_line = 0;
        self.align_ptr = TEX_NULL;
        self.cur_align = TEX_NULL;
        self.cur_span = TEX_NULL;
        self.cur_loop = TEX_NULL;
        self.cur_head = TEX_NULL;
        self.cur_tail = TEX_NULL;
        self.cur_pre_head = TEX_NULL;
        self.cur_pre_tail = TEX_NULL;
        self.cur_f = 0;
        self.fmt_max_hyph_char = TOO_BIG_LANG;
        self.hyph_word.fill(0);
        self.hyph_list.fill(TEX_NULL);
        self.hyph_link.fill(0);
        self.hyph_count = 0;
        self.hyph_next = HYPH_PRIME + 1;
        if self.hyph_next > HYPH_SIZE as i32 {
            self.hyph_next = HYPH_PRIME;
        }
        self.output_active = false;
        self.insert_penalties = 0;
        self.ligature_present = false;
        self.cancel_boundary = false;
        self.lft_hit = false;
        self.rt_hit = false;
        self.ins_disc = false;
        self.after_token = 0;
        self.long_help_seen = false;
        self.format_ident = 0;
        self.lr_ptr = TEX_NULL;
        self.lr_problems = 0;
        self.cur_dir = LEFT_TO_RIGHT;
        self.pseudo_files = TEX_NULL;
        self.sa_root[SA_MARK_VAL as usize] = TEX_NULL;
        self.sa_null = MemoryWord::default();
        self.sa_chain = TEX_NULL;
        self.sa_level = LEVEL_ZERO;
        self.disc_ptr[LAST_BOX_CODE as usize] = TEX_NULL;
        self.disc_ptr[VSPLIT_CODE as usize] = TEX_NULL;
        self.edit_name_start = 0;
        self.stop_at_space = true;
    }

    /// C `initialize_more_initex_variables` — primes the INITEX-only counters
    /// and the format-ident marker. The fixed `mem`/`eqtb` layout defaults
    /// are materialized by `new()` and the serialization image.
    pub(crate) fn initialize_more_initex_variables(&mut self) -> Result<(), TeXMouthError> {
        self.mem_end = self.mem_top();
        self.var_used = 20;
        self.dyn_used = HI_MEM_STAT_USAGE;
        self.prim_used = C_PRIM_SIZE;
        self.hash_used = C_FROZEN_CONTROL_SEQUENCE;
        self.hash_high = 0;
        self.cs_count = 0;
        self.trie_op_hash.fill(0);
        self.trie_used.fill(MIN_TRIE_OP);
        self.max_op_used = MIN_TRIE_OP;
        self.trie_op_ptr = 0;
        self.trie_not_ready = true;
        let units: Vec<i32> = " (INITEX)".chars().map(|c| c as i32).collect();
        self.format_ident = self.make_tex_string_units(&units)?;
        self.max_reg_num = 32767;
        self.max_reg_help_line = "A register number must be between 0 and 32767.".to_string();
        for i in 0..=SA_INTER_CHAR_VAL {
            self.sa_root[i as usize] = TEX_NULL;
        }
        self.set_int_par(INT_PAR_XETEX_HYPHENATABLE_LENGTH, 63)?;
        Ok(())
    }

    /// C `init_io` — resets the terminal input stream and the bottom input
    /// level's buffer state.
    pub(crate) fn init_io(&mut self) {
        if self.buffer.len() <= self.first as usize {
            self.buffer.resize(self.first as usize + 1, 0);
        }
        self.buffer[self.first as usize] = 0;
        self.last = self.first;
        self.cur_input.loc = self.first;
        self.cur_input.limit = self.last;
        self.first = self.last + 1;
    }

    // === `sort_avail` ===

    /// C `sort_avail` — sorts the variable-size free list into address order
    /// before dumping so the hole between used regions is maximal.
    pub(crate) fn sort_avail(&mut self) -> Result<(), TeXMouthError> {
        let _ = self.get_node(0x4000_0000);
        let mut p = self.mem[(self.rover + 1) as usize].s1;
        self.mem[(self.rover + 1) as usize].s1 = MAX_HALFWORD;
        let old_rover = self.rover;
        while p != old_rover {
            if p < self.rover {
                let q = p;
                p = self.mem[(q + 1) as usize].s1;
                self.mem[(q + 1) as usize].s1 = self.rover;
                self.rover = q;
            } else {
                let mut q = self.rover;
                while self.mem[(q + 1) as usize].s1 < p {
                    q = self.mem[(q + 1) as usize].s1;
                }
                let r = self.mem[(p + 1) as usize].s1;
                self.mem[(p + 1) as usize].s1 = self.mem[(q + 1) as usize].s1;
                self.mem[(q + 1) as usize].s1 = p;
                p = r;
            }
        }
        p = self.rover;
        while self.mem[(p + 1) as usize].s1 != MAX_HALFWORD {
            let nxt = self.mem[(p + 1) as usize].s1;
            self.mem[(nxt + 1) as usize].s0 = p;
            p = nxt;
        }
        self.mem[(p + 1) as usize].s1 = self.rover;
        self.mem[(self.rover + 1) as usize].s0 = p;
        Ok(())
    }

    /// C `pack_buffered_name` — reloads `name_of_file` from
    /// `TEX_format_default`.
    pub(crate) fn pack_buffered_name(&mut self) {
        self.name_of_file = self.tex_format_default.clone().into_bytes();
        self.name_length = self.name_of_file.len() as i32;
    }

    // === Hyphenation trie (`new_trie_op` … `init_trie`) ===

    /// Ensures the unpacked-trie build arrays are allocated.
    fn ensure_trie_arrays(&mut self) {
        let n = FMT_TRIE_SIZE as usize + 1;
        if self.fmt_trie_c.len() < n {
            self.fmt_trie_c.resize(n, 0);
            self.fmt_trie_o.resize(n, 0);
            self.fmt_trie_l.resize(n, 0);
            self.fmt_trie_r.resize(n, 0);
            self.fmt_trie_hash.resize(n, 0);
            self.fmt_trie_taken.resize(n, false);
        }
        if self.fmt_trie_min.len() < (BIGGEST_CHAR + 1) as usize {
            self.fmt_trie_min.resize((BIGGEST_CHAR + 1) as usize, 0);
        }
        if self.trie_trl.len() < n {
            self.trie_trl.resize(n, 0);
            self.trie_tro.resize(n, 0);
            self.trie_trc.resize(n, 0);
        }
    }

    /// C `new_trie_op` — interns the op `(d, n, v)` for `cur_lang`, returning
    /// its per-language serial.
    pub(crate) fn new_trie_op(&mut self, d: i16, n: i16, v: u16) -> Result<i32, TeXMouthError> {
        let lang = self.cur_lang;
        let mut h = (i32::from(n) + 313 * i32::from(d) + 361 * i32::from(v) + 1009 * lang).abs()
            % (TRIE_OP_SIZE - NEG_TRIE_OP_SIZE)
            + NEG_TRIE_OP_SIZE;
        loop {
            let l = self.trie_op_hash[(h + TRIE_OP_SIZE) as usize];
            if l == 0 {
                if self.trie_op_ptr == TRIE_OP_SIZE {
                    self.overflow("pattern memory ops", TRIE_OP_SIZE)?;
                }
                let mut u = self.trie_used[lang as usize];
                if u == MAX_TRIE_OP {
                    self.overflow("pattern memory ops per language", MAX_TRIE_OP - MIN_TRIE_OP)?;
                }
                self.trie_op_ptr += 1;
                u += 1;
                self.trie_used[lang as usize] = u;
                if u > self.max_op_used {
                    self.max_op_used = u;
                }
                let p = self.trie_op_ptr as usize;
                self.hyf_distance[p] = d;
                self.hyf_num[p] = n;
                self.hyf_next[p] = v;
                self.trie_op_lang[p] = lang as u8;
                self.trie_op_hash[(h + TRIE_OP_SIZE) as usize] = self.trie_op_ptr;
                self.trie_op_val[p] = u as u16;
                return Ok(u);
            }
            let lu = l as usize;
            if self.hyf_distance[lu] == d
                && self.hyf_num[lu] == n
                && self.hyf_next[lu] == v
                && i32::from(self.trie_op_lang[lu]) == lang
            {
                return Ok(i32::from(self.trie_op_val[lu]));
            }
            if h > -TRIE_OP_SIZE {
                h -= 1;
            } else {
                h = TRIE_OP_SIZE;
            }
        }
    }

    /// C `trie_node` — hash-conses the unpacked trie node `p`.
    pub(crate) fn trie_node(&mut self, p: i32) -> i32 {
        let c = i32::from(self.fmt_trie_c[p as usize]);
        let o = self.fmt_trie_o[p as usize];
        let l = self.fmt_trie_l[p as usize];
        let r = self.fmt_trie_r[p as usize];
        let mut h = (c + 1009 * o + 2718 * l + 3142 * r).abs() % FMT_TRIE_SIZE;
        loop {
            let q = self.fmt_trie_hash[h as usize];
            if q == 0 {
                self.fmt_trie_hash[h as usize] = p;
                return p;
            }
            let qu = q as usize;
            if i32::from(self.fmt_trie_c[qu]) == c
                && self.fmt_trie_o[qu] == o
                && self.fmt_trie_l[qu] == l
                && self.fmt_trie_r[qu] == r
            {
                return q;
            }
            if h > 0 {
                h -= 1;
            } else {
                h = FMT_TRIE_SIZE;
            }
        }
    }

    /// C `compress_trie` — hash-conses the whole subtree rooted at `p`.
    pub(crate) fn compress_trie(&mut self, p: i32) -> i32 {
        if p == 0 {
            return 0;
        }
        let l = self.compress_trie(self.fmt_trie_l[p as usize]);
        self.fmt_trie_l[p as usize] = l;
        let r = self.compress_trie(self.fmt_trie_r[p as usize]);
        self.fmt_trie_r[p as usize] = r;
        self.trie_node(p)
    }

    /// C `first_fit` — finds the lowest packed position where `p`'s family
    /// fits and marks the slots taken.
    pub(crate) fn first_fit(&mut self, p: i32) -> Result<(), TeXMouthError> {
        let c = i32::from(self.fmt_trie_c[p as usize]);
        let mut z = self.fmt_trie_min[c as usize];
        let mut h;
        'outer: loop {
            h = z - c;
            if self.fmt_trie_max < h + self.fmt_max_hyph_char {
                if FMT_TRIE_SIZE <= h + self.fmt_max_hyph_char {
                    self.overflow("pattern memory", FMT_TRIE_SIZE)?;
                }
                loop {
                    self.fmt_trie_max += 1;
                    let m = self.fmt_trie_max as usize;
                    self.fmt_trie_taken[m] = false;
                    self.trie_trl[m] = self.fmt_trie_max + 1;
                    self.trie_tro[m] = self.fmt_trie_max - 1;
                    if self.fmt_trie_max == h + self.fmt_max_hyph_char {
                        break;
                    }
                }
            }
            if self.fmt_trie_taken[h as usize] {
                z = self.trie_trl[z as usize];
                continue 'outer;
            }
            let mut q = self.fmt_trie_r[p as usize];
            while q > 0 {
                if self.trie_trl[(h + i32::from(self.fmt_trie_c[q as usize])) as usize] == 0 {
                    z = self.trie_trl[z as usize];
                    continue 'outer;
                }
                q = self.fmt_trie_r[q as usize];
            }
            break;
        }
        self.fmt_trie_taken[h as usize] = true;
        self.fmt_trie_hash[p as usize] = h;
        let mut q = p;
        loop {
            let z2 = h + i32::from(self.fmt_trie_c[q as usize]);
            let l = self.trie_tro[z2 as usize];
            let r = self.trie_trl[z2 as usize];
            self.trie_tro[r as usize] = l;
            self.trie_trl[l as usize] = r;
            self.trie_trl[z2 as usize] = 0;
            if l < self.fmt_max_hyph_char {
                let ll = if z2 < self.fmt_max_hyph_char {
                    z2
                } else {
                    self.fmt_max_hyph_char
                };
                let mut l = l;
                while l != ll {
                    self.fmt_trie_min[l as usize] = r;
                    l += 1;
                }
            }
            q = self.fmt_trie_r[q as usize];
            if q == 0 {
                return Ok(());
            }
        }
    }

    /// C `trie_pack` — recursively packs every unpacked sibling family.
    pub(crate) fn trie_pack(&mut self, p: i32) -> Result<(), TeXMouthError> {
        let mut p = p;
        loop {
            let q = self.fmt_trie_l[p as usize];
            if q > 0 && self.fmt_trie_hash[q as usize] == 0 {
                self.first_fit(q)?;
                self.trie_pack(q)?;
            }
            p = self.fmt_trie_r[p as usize];
            if p == 0 {
                return Ok(());
            }
        }
    }

    /// C `trie_fix` — writes the packed node's `(trl, tro, trc)` triples.
    pub(crate) fn trie_fix(&mut self, p: i32) {
        let z = self.fmt_trie_hash[p as usize];
        let mut p = p;
        loop {
            let q = self.fmt_trie_l[p as usize];
            let c = i32::from(self.fmt_trie_c[p as usize]);
            self.trie_trl[(z + c) as usize] = self.fmt_trie_hash[q as usize];
            self.trie_trc[(z + c) as usize] = c as u16;
            self.trie_tro[(z + c) as usize] = self.fmt_trie_o[p as usize];
            if q > 0 {
                self.trie_fix(q);
            }
            p = self.fmt_trie_r[p as usize];
            if p == 0 {
                return;
            }
        }
    }

    /// C `init_trie` — packs the unpacked pattern trie and sorts the op
    /// tables into per-language order.
    pub(crate) fn init_trie(&mut self) -> Result<(), TeXMouthError> {
        self.ensure_trie_arrays();
        self.fmt_max_hyph_char += 1;
        self.op_start[0] = -MIN_TRIE_OP;
        for j in 1..=BIGGEST_LANG as usize {
            self.op_start[j] = self.op_start[j - 1] + self.trie_used[j - 1];
        }
        for j in 1..=self.trie_op_ptr as usize {
            self.trie_op_hash[j] =
                self.op_start[self.trie_op_lang[j] as usize] + i32::from(self.trie_op_val[j]);
        }
        for j in 1..=self.trie_op_ptr as usize {
            while self.trie_op_hash[j] > j as i32 {
                let k = self.trie_op_hash[j] as usize;
                self.hyf_distance.swap(k, j);
                self.hyf_num.swap(k, j);
                self.hyf_next.swap(k, j);
                self.trie_op_hash[j] = self.trie_op_hash[k];
                self.trie_op_hash[k] = k as i32;
            }
        }
        for p in 0..=FMT_TRIE_SIZE as usize {
            self.fmt_trie_hash[p] = 0;
        }
        let r0 = self.fmt_trie_r[0];
        let r0c = self.compress_trie(r0);
        self.fmt_trie_r[0] = r0c;
        let l0 = self.fmt_trie_l[0];
        let l0c = self.compress_trie(l0);
        self.fmt_trie_l[0] = l0c;
        for p in 0..=self.fmt_trie_ptr as usize {
            self.fmt_trie_hash[p] = 0;
        }
        for p in 0..=BIGGEST_CHAR as usize {
            self.fmt_trie_min[p] = p as i32 + 1;
        }
        self.trie_trl[0] = 1;
        self.fmt_trie_max = 0;
        if self.fmt_trie_l[0] != 0 {
            let l0 = self.fmt_trie_l[0];
            self.first_fit(l0)?;
            self.trie_pack(l0)?;
        }
        if self.fmt_trie_r[0] != 0 {
            if self.fmt_trie_l[0] == 0 {
                for p in 0..=255usize {
                    self.fmt_trie_min[p] = p as i32 + 2;
                }
            }
            let r0 = self.fmt_trie_r[0];
            self.first_fit(r0)?;
            self.trie_pack(r0)?;
            self.hyph_start = self.fmt_trie_hash[r0 as usize];
        }
        if self.fmt_trie_max == 0 {
            for r in 0..=self.fmt_max_hyph_char as usize {
                self.trie_trl[r] = 0;
                self.trie_tro[r] = MIN_TRIE_OP;
                self.trie_trc[r] = 0;
            }
            self.fmt_trie_max = self.fmt_max_hyph_char;
        } else {
            if self.fmt_trie_r[0] > 0 {
                let r0 = self.fmt_trie_r[0];
                self.trie_fix(r0);
            }
            if self.fmt_trie_l[0] > 0 {
                let l0 = self.fmt_trie_l[0];
                self.trie_fix(l0);
            }
            let mut r = 0i32;
            loop {
                let s = self.trie_trl[r as usize];
                self.trie_trl[r as usize] = 0;
                self.trie_tro[r as usize] = MIN_TRIE_OP;
                self.trie_trc[r as usize] = 0;
                r = s;
                if r > self.fmt_trie_max {
                    break;
                }
            }
        }
        self.trie_trc[0] = u16::from(b'?');
        self.trie_not_ready = false;
        Ok(())
    }

    // === `\patterns` (`new_patterns`) ===

    /// C `new_patterns` — scans `\patterns{...}` in INITEX mode, inserting
    /// each pattern into the unpacked trie (and mirroring into
    /// `hyph_patterns` for the runtime hyphenator). Once the trie is packed
    /// this becomes the "too late" error path.
    pub(crate) fn new_patterns(&mut self) -> Result<(), TeXMouthError> {
        if !self.trie_not_ready {
            self.error_here_with_diagnostic("Too late for ");
            self.print_esc_cstr("patterns")?;
            self.capture_to_diagnostic(false);
            self.help_ptr = 1;
            self.help_line[0] = "All patterns must be given before typesetting begins.".to_string();
            self.error()?;
            let garbage = self.lig_trick();
            let toks = self.scan_toks(false, false)?;
            self.mem[garbage as usize].s1 = toks;
            self.flush_list(toks)?;
            return Ok(());
        }
        self.ensure_trie_arrays();
        let language = self.int_par(INT_PAR_LANGUAGE)?;
        self.cur_lang = if (1..=BIGGEST_LANG).contains(&language) {
            language
        } else {
            0
        };
        self.scan_left_brace()?;
        let mut k: i32 = 0;
        self.lb_hyf[0] = 0;
        let mut digit_sensed = false;
        let mut pattern: Vec<i32> = Vec::new();
        let mut digits: Vec<i32> = Vec::new();
        'scan: loop {
            self.get_x_token()?;
            match CommandCode::from_u8(self.cur_cmd) {
                Some(CommandCode::Letter | CommandCode::OtherChar) => {
                    if digit_sensed || !(i32::from(b'0')..=i32::from(b'9')).contains(&self.cur_chr)
                    {
                        if self.cur_chr == i32::from(b'.') {
                            self.cur_chr = 0;
                        } else {
                            self.cur_chr = self.lc_code(self.cur_chr.max(0) as u32);
                            if self.cur_chr == 0 {
                                self.error_here_with_diagnostic("Nonletter");
                                self.capture_to_diagnostic(false);
                                self.help_ptr = 1;
                                self.help_line[0] = "(See Appendix H.)".to_string();
                                self.error()?;
                            }
                        }
                        if self.cur_chr > self.fmt_max_hyph_char {
                            self.fmt_max_hyph_char = self.cur_chr;
                        }
                        if k < self.max_hyphenatable_length()? {
                            k += 1;
                            self.lb_hc[k as usize] = self.cur_chr;
                            self.lb_hyf[k as usize] = 0;
                            digit_sensed = false;
                            pattern.push(self.cur_chr);
                            digits.push(0);
                        }
                    } else if k < self.max_hyphenatable_length()? {
                        self.lb_hyf[k as usize] = self.cur_chr - i32::from(b'0');
                        digit_sensed = true;
                        pattern.push(self.cur_chr);
                        digits.push(self.cur_chr - i32::from(b'0'));
                    }
                }
                Some(CommandCode::Spacer | CommandCode::RightBrace) => {
                    if k > 0 {
                        if self.lb_hc[1] == 0 {
                            self.lb_hyf[0] = 0;
                        }
                        if self.lb_hc[k as usize] == 0 {
                            self.lb_hyf[k as usize] = 0;
                        }
                        let mut l = k;
                        let mut v = MIN_TRIE_OP as u16;
                        loop {
                            if self.lb_hyf[l as usize] != 0 {
                                v = self.new_trie_op(
                                    (k - l) as i16,
                                    self.lb_hyf[l as usize] as i16,
                                    v,
                                )? as u16;
                            }
                            if l > 0 {
                                l -= 1;
                            } else {
                                break;
                            }
                        }
                        let mut q = 0i32;
                        self.lb_hc[0] = self.cur_lang;
                        let mut l = 0;
                        while l <= k {
                            let c = self.lb_hc[l as usize];
                            l += 1;
                            let mut p = self.fmt_trie_l[q as usize];
                            let mut first_child = true;
                            while p > 0 && c > i32::from(self.fmt_trie_c[p as usize]) {
                                q = p;
                                p = self.fmt_trie_r[q as usize];
                                first_child = false;
                            }
                            if p == 0 || c < i32::from(self.fmt_trie_c[p as usize]) {
                                if self.fmt_trie_ptr == FMT_TRIE_SIZE {
                                    self.overflow("pattern memory", FMT_TRIE_SIZE)?;
                                }
                                self.fmt_trie_ptr += 1;
                                self.fmt_trie_r[self.fmt_trie_ptr as usize] = p;
                                p = self.fmt_trie_ptr;
                                self.fmt_trie_l[p as usize] = 0;
                                if first_child {
                                    self.fmt_trie_l[q as usize] = p;
                                } else {
                                    self.fmt_trie_r[q as usize] = p;
                                }
                                self.fmt_trie_c[p as usize] = c as u16;
                                self.fmt_trie_o[p as usize] = MIN_TRIE_OP;
                            }
                            q = p;
                        }
                        if self.fmt_trie_o[q as usize] != MIN_TRIE_OP {
                            self.error_here_with_diagnostic("Duplicate pattern");
                            self.capture_to_diagnostic(false);
                            self.help_ptr = 1;
                            self.help_line[0] = "(See Appendix H.)".to_string();
                            self.error()?;
                        }
                        self.fmt_trie_o[q as usize] = i32::from(v);
                        // Mirror into the runtime hyphenation map.
                        let mut key = pattern.clone();
                        key.push(self.cur_lang);
                        self.hyph_patterns.insert(key, digits.clone());
                    }
                    if self.cur_cmd == CommandCode::RightBrace as u8 {
                        break 'scan;
                    }
                    k = 0;
                    self.lb_hyf[0] = 0;
                    digit_sensed = false;
                    pattern.clear();
                    digits.clear();
                }
                _ => {
                    self.error_here_with_diagnostic("Bad ");
                    self.print_esc_cstr("patterns")?;
                    self.capture_to_diagnostic(false);
                    self.help_ptr = 1;
                    self.help_line[0] = "(See Appendix H.)".to_string();
                    self.error()?;
                }
            }
        }
        if self.int_par(INT_PAR_SAVING_HYPH_CODES)? > 0 {
            let mut c = self.cur_lang;
            let mut first_child = false;
            let mut p = 0i32;
            let mut q;
            loop {
                q = p;
                p = self.fmt_trie_r[q as usize];
                if p == 0 || c <= i32::from(self.fmt_trie_c[p as usize]) {
                    break;
                }
            }
            if p == 0 || c < i32::from(self.fmt_trie_c[p as usize]) {
                if self.fmt_trie_ptr == FMT_TRIE_SIZE {
                    self.overflow("pattern memory", FMT_TRIE_SIZE)?;
                }
                self.fmt_trie_ptr += 1;
                self.fmt_trie_r[self.fmt_trie_ptr as usize] = p;
                p = self.fmt_trie_ptr;
                self.fmt_trie_l[p as usize] = 0;
                if first_child {
                    self.fmt_trie_l[q as usize] = p;
                } else {
                    self.fmt_trie_r[q as usize] = p;
                }
                self.fmt_trie_c[p as usize] = c as u16;
                self.fmt_trie_o[p as usize] = MIN_TRIE_OP;
            }
            q = p;
            p = self.fmt_trie_l[q as usize];
            first_child = true;
            for c in 0..=255i32 {
                if self.lc_code(c as u32) > 0 || (c == 255 && first_child) {
                    if p == 0 {
                        if self.fmt_trie_ptr == FMT_TRIE_SIZE {
                            self.overflow("pattern memory", FMT_TRIE_SIZE)?;
                        }
                        self.fmt_trie_ptr += 1;
                        self.fmt_trie_r[self.fmt_trie_ptr as usize] = p;
                        p = self.fmt_trie_ptr;
                        self.fmt_trie_l[p as usize] = 0;
                        if first_child {
                            self.fmt_trie_l[q as usize] = p;
                        } else {
                            self.fmt_trie_r[q as usize] = p;
                        }
                        self.fmt_trie_c[p as usize] = c as u16;
                        self.fmt_trie_o[p as usize] = MIN_TRIE_OP;
                    } else {
                        self.fmt_trie_c[p as usize] = c as u16;
                    }
                    self.fmt_trie_o[p as usize] = self.lc_code(c as u32);
                    q = p;
                    p = self.fmt_trie_r[q as usize];
                    first_child = false;
                }
            }
            if first_child {
                self.fmt_trie_l[q as usize] = 0;
            } else {
                self.fmt_trie_r[q as usize] = 0;
            }
            c = self.cur_lang; // keep `c` live for clarity; C reuses it below
            let _ = c;
        }
        Ok(())
    }

    /// C `new_hyph_exceptions` — scans `\hyphenation{...}` exception words.
    /// Hyphen points become a `get_avail` node chain (`s1`=prev, `s0`=n);
    /// words are interned (with the language code appended) into the
    /// `hyph_word`/`hyph_list`/`hyph_link` hash table. The runtime map
    /// mirror (`hyph_exceptions`) is updated alongside for the hyphenator.
    pub(crate) fn new_hyph_exceptions(&mut self) -> Result<(), TeXMouthError> {
        self.scan_left_brace()?;
        let language = self.int_par(INT_PAR_LANGUAGE)?;
        self.cur_lang = if (1..=BIGGEST_LANG).contains(&language) {
            language
        } else {
            0
        };

        self.hyph_index = if self.trie_not_ready
            || self
                .trie_trc
                .get((self.hyph_start + self.cur_lang) as usize)
                .copied()
                .unwrap_or(0)
                != self.cur_lang as u16
        {
            0
        } else {
            self.trie_trl[(self.hyph_start + self.cur_lang) as usize]
        };

        let mut n: i32 = 0;
        let mut p = TEX_NULL;
        loop {
            self.get_x_token()?;
            'reswitch: loop {
                match CommandCode::from_u8(self.cur_cmd) {
                    Some(CommandCode::Letter | CommandCode::OtherChar | CommandCode::CharGiven) => {
                        if self.cur_chr == i32::from(b'-') {
                            if n < self.max_hyphenatable_length()? {
                                let q = self.get_avail()?;
                                self.mem[q as usize].s1 = p;
                                self.mem[q as usize].s0 = n;
                                p = q;
                            }
                        } else {
                            let c = self.cur_chr;
                            let hc0 = if self.hyph_index == 0 || c > 255 {
                                self.lc_code(c.max(0) as u32)
                            } else if self
                                .trie_trc
                                .get((self.hyph_index + c) as usize)
                                .copied()
                                .unwrap_or(0)
                                != c as u16
                            {
                                0
                            } else {
                                self.trie_tro[(self.hyph_index + c) as usize]
                            };
                            if hc0 == 0 {
                                self.error_here_with_diagnostic("Not a letter");
                                self.capture_to_diagnostic(false);
                                self.help_ptr = 2;
                                self.help_line[1] =
                                    "Letters in \\hyphenation words must have \\lccode>0."
                                        .to_string();
                                self.help_line[0] =
                                    "Proceed; I'll ignore the character I just read.".to_string();
                                self.error()?;
                            } else if n < self.max_hyphenatable_length()? {
                                n += 1;
                                if hc0 < 65_536 {
                                    self.lb_hc[n as usize] = hc0;
                                } else {
                                    self.lb_hc[n as usize] = (hc0 - 65_536) / 1024 + 55_296;
                                    n += 1;
                                    self.lb_hc[n as usize] = hc0 % 1024 + 56_320;
                                }
                            }
                        }
                        break 'reswitch;
                    }
                    Some(CommandCode::CharNum) => {
                        self.scan_char_num()?;
                        self.cur_chr = self.cur_val;
                        self.cur_cmd = CommandCode::CharGiven as u8;
                        continue 'reswitch;
                    }
                    Some(CommandCode::Spacer | CommandCode::RightBrace) => {
                        if n > 1 {
                            n += 1;
                            self.lb_hc[n as usize] = self.cur_lang;
                            if self.pool_ptr + n > self.pool_size {
                                self.overflow("pool size", self.pool_size - self.init_pool_ptr)?;
                            }
                            let mut h = 0i32;
                            let mut word: Vec<i32> = Vec::new();
                            for j in 1..=n as usize {
                                h = (h + h + self.lb_hc[j]) % HYPH_PRIME;
                                word.push(self.lb_hc[j]);
                            }
                            for &u in &word {
                                self.pool_push(u);
                            }
                            let s = self.make_string()?;
                            if self.hyph_next <= HYPH_PRIME {
                                while self.hyph_next > 0
                                    && self.hyph_word[(self.hyph_next - 1) as usize] > 0
                                {
                                    self.hyph_next -= 1;
                                }
                            }
                            if self.hyph_count == HYPH_SIZE as i32 || self.hyph_next == 0 {
                                self.overflow("exception dictionary", HYPH_SIZE as i32)?;
                            }
                            self.hyph_count += 1;
                            let mut s = s;
                            while self.hyph_word[h as usize] != 0 {
                                let k = self.hyph_word[h as usize];
                                if self.length(k) == self.length(s) && self.str_eq_str(k, s) {
                                    self.str_ptr -= 1;
                                    self.pool_ptr = self
                                        .str_start
                                        .get((self.str_ptr - TOO_BIG_CHAR) as usize)
                                        .copied()
                                        .unwrap_or(0);
                                    s = k;
                                    self.hyph_count -= 1;
                                    break;
                                }
                                if self.hyph_link[h as usize] == 0 {
                                    self.hyph_link[h as usize] = self.hyph_next as u16;
                                    if self.hyph_next >= HYPH_SIZE as i32 {
                                        self.hyph_next = HYPH_PRIME;
                                    }
                                    if self.hyph_next > HYPH_PRIME {
                                        self.hyph_next += 1;
                                    }
                                }
                                h = i32::from(self.hyph_link[h as usize]) - 1;
                            }
                            self.hyph_word[h as usize] = s;
                            self.hyph_list[h as usize] = p;
                            // Runtime map mirror: word+lang key, scan-order
                            // hyphen positions.
                            let mut positions: Vec<i32> = Vec::new();
                            let mut node = p;
                            while node != TEX_NULL {
                                positions.push(self.mem[node as usize].s0);
                                node = self.mem[node as usize].s1;
                            }
                            positions.reverse();
                            self.hyph_exceptions.insert(word, positions);
                        }
                        if self.cur_cmd == CommandCode::RightBrace as u8 {
                            return Ok(());
                        }
                        n = 0;
                        p = TEX_NULL;
                        break 'reswitch;
                    }
                    _ => {
                        self.error_here_with_diagnostic("Improper ");
                        self.print_esc_cstr("hyphenation")?;
                        self.print_cstr(" will be flushed")?;
                        self.capture_to_diagnostic(false);
                        self.help_ptr = 2;
                        self.help_line[1] =
                            "Hyphenation exceptions must contain only letters".to_string();
                        self.help_line[0] =
                            "and hyphens. But continue; I'll forgive and forget.".to_string();
                        self.error()?;
                        break 'reswitch;
                    }
                }
            }
        }
    }

    // === `final_cleanup` ===

    /// C `open_log_file` — echoes `**` plus the loaded input line to the log
    /// only, then sets `selector = old + 2` (`TERM_AND_LOG` from
    /// `TERM_ONLY`, `LOG_ONLY` from `NO_PRINT`). The byte sink is `log_out`;
    /// the driver's job is to flush it to `<jobname>.log`.
    pub(crate) fn open_log_file(&mut self) -> Result<(), TeXMouthError> {
        let old_setting = self.selector;
        if self.job_name.is_none() {
            self.job_name = Some("texput".to_string());
        }
        let base = self.job_name.clone().unwrap_or_default();
        // `pack_job_name(".log")` appends the extension to `job_name`
        // verbatim — the job name is already a stem.
        self.name_of_file = format!("{base}.log").into_bytes();
        self.name_length = self.name_of_file.len() as i32;
        self.log_name = Some(format!("{base}.log"));
        self.selector = SELECTOR_LOG_ONLY;
        self.log_opened = true;

        // Catch the log up with anything already printed: `**` plus the
        // bottom input frame's loaded line. The limit refers to the frame
        // that was active when the engine started, so clamp it to the
        // bytes the buffer actually holds.
        if let Some(top) = self.input_stack.first().copied() {
            self.print_nl_cstr("**")?;
            let mut l = top.limit.min(self.buffer.len() as i32 - 1);
            if l >= 0
                && self.buffer[l as usize] == self.int_par(INT_PAR_END_LINE_CHAR).unwrap_or(-1)
            {
                l -= 1;
            }
            for k in 1..=l {
                self.print(self.buffer[k as usize])?;
            }
            self.print_ln();
        }
        self.selector = old_setting + 2;
        Ok(())
    }

    /// C `final_cleanup` — closes dangling input/conditional/group state and
    /// performs `\dump` when invoked in INITEX mode.
    pub(crate) fn final_cleanup(&mut self) -> Result<(), TeXMouthError> {
        let c = self.cur_chr;
        if c != 1 {
            self.set_int_par(INT_PAR_NEW_LINE_CHAR, -1)?;
        }
        if self.job_name.is_none() {
            self.open_log_file()?;
        }
        while self.input_ptr > 0 {
            if self.cur_input.state == TOKEN_LIST {
                self.end_token_list()?;
            } else {
                self.end_file_reading()?;
            }
        }
        while self.open_parens > 0 {
            self.print_cstr(" )")?;
            self.open_parens -= 1;
        }
        if self.cur_level > LEVEL_ONE {
            self.print_nl_cstr("(")?;
            self.print_esc_cstr("end occurred ")?;
            self.print_cstr("inside a group at level ")?;
            self.print_int(i32::from(self.cur_level) - 1)?;
            self.print_char(i32::from(b')'))?;
            self.show_save_groups()?;
        }
        while self.cond_ptr != TEX_NULL {
            self.print_nl_cstr("(")?;
            self.print_esc_cstr("end occurred ")?;
            self.print_cstr("when ")?;
            self.print_if_test_name(i32::from(self.cur_if))?;
            if self.if_line != 0 {
                self.print_cstr(" on line ")?;
                self.print_int(self.if_line)?;
            }
            self.print_cstr(" was incomplete)")?;
            self.if_line = self.mem[(self.cond_ptr + 1) as usize].s1;
            self.cur_if = self.mem[self.cond_ptr as usize].b16_s0() as i16;
            let temp_ptr = self.cond_ptr;
            self.cond_ptr = self.mem[self.cond_ptr as usize].s1;
            self.free_node(temp_ptr, IF_NODE_SIZE)?;
        }
        if self.history != HISTORY_SPOTLESS
            && (self.history == HISTORY_WARNING_ISSUED || self.interaction < ERROR_STOP_MODE)
            && self.selector == SELECTOR_TERM_AND_LOG
        {
            self.selector = SELECTOR_TERM_ONLY;
            self.print_nl_cstr("(see the transcript file for additional information)")?;
            self.selector = SELECTOR_TERM_AND_LOG;
        }
        if c == 1 {
            if self.in_initex_mode {
                for m in TOP_MARK_CODE..=SPLIT_BOT_MARK_CODE {
                    if self.cur_mark[m as usize] != TEX_NULL {
                        self.delete_token_ref(self.cur_mark[m as usize])?;
                    }
                }
                if self.sa_root[SA_MARK_VAL as usize] != TEX_NULL
                    && self.do_marks(DO_MARKS_DESTROY, 0, self.sa_root[SA_MARK_VAL as usize])?
                {
                    self.sa_root[SA_MARK_VAL as usize] = TEX_NULL;
                }
                for dp in LAST_BOX_CODE..=VSPLIT_CODE {
                    let p = self.disc_ptr[dp as usize];
                    self.flush_node_list(p)?;
                    self.disc_ptr[dp as usize] = TEX_NULL;
                }
                if self.last_glue != MAX_HALFWORD {
                    self.delete_glue_ref(self.last_glue)?;
                }
                self.store_fmt_file()?;
                return Ok(());
            }
            self.print_nl_cstr("(\\dump is performed only by INITEX)")?;
        }
        Ok(())
    }

    /// C `close_files_and_terminate` — runs after `final_cleanup` at the
    /// `tt_run_engine` tail: closes `\write` streams, finalizes the XDV
    /// file (`dvi_close`, which prints `Output written on …`), terminates
    /// SyncTeX, writes the trailing newline into the log and reports
    /// `Transcript written on <job>.log.` on the terminal.
    pub(crate) fn close_files_and_terminate(&mut self) -> Result<(), TeXMouthError> {
        for k in 0..16 {
            if self.write_stream_open(k) {
                self.write_open[k as usize] = false;
            }
        }
        self.set_int_par(INT_PAR_NEW_LINE_CHAR, -1)?;

        if !self.dvi_finalized {
            self.finalize_dvi_file()?;
            self.dvi_finalized = true;
        }
        self.synctex_terminate(self.log_opened);

        if self.log_opened {
            // C writes `\n` directly to the log file, then closes it —
            // the byte lands in `log_out` regardless of `selector`.
            // `log_opened` stays set: it gates the driver's `log_out`
            // flush (our buffered equivalent of C's open file handle).
            self.log_out.push(b'\n');
            self.selector -= 2;
            if self.selector == SELECTOR_TERM_ONLY {
                let name = self.log_name.clone().unwrap_or_default();
                self.print_nl_cstr("Transcript written on ")?;
                self.print_str_text(&name)?;
                self.print_char(i32::from(b'.'))?;
            }
        }
        self.print_ln();
        Ok(())
    }

    /// C `tt_run_engine` — drives a complete engine invocation on this
    /// state: INITEX primitive setup or format load, input start, the main
    /// control loop, and `final_cleanup`. Returns `history`.
    ///
    /// `dump_name` is the format-file name (`TEX_format_default`); in
    /// non-INITEX mode `fmt_bytes` supplies the format image (the caller
    /// resolves the file — C opens it through the I/O stack). `input_name`
    /// is the primary input file name opened through `start_input_source`.
    ///
    /// `build_date` seeds the `\time`/`\day`/`\month`/`\year` parameters and
    /// the random generator. C uses `localtime`; this port derives the
    /// broken-down date in UTC so engine behavior is platform-independent.
    /// Driver-facing entry: resolves `<dump_name>.fmt` through the
    /// byte-source registry / injected `input_resolver` and runs the job
    /// (`tt_engine_xetex_main`'s Rust equivalent). XDV accumulates in
    /// `dvi_bytes`, the transcript in `transcript`/`log_out`.
    pub fn run(
        &mut self,
        dump_name: &str,
        input_name: &str,
        build_date: std::time::SystemTime,
    ) -> Result<u8, TeXMouthError> {
        let fmt_bytes = if self.in_initex_mode {
            None
        } else {
            // C `open_fmt_file`: `name_of_file` is `dump_name` verbatim,
            // opened with `TTBC_FILE_FORMAT_FORMAT` so the provider's
            // format search path (not the extension-derived file kind)
            // resolves it.
            let bytes = match self.fmt_input_resolver.clone() {
                Some(resolver) => resolver.resolve(dump_name),
                None => self.input_byte_source_bytes(dump_name),
            };
            Some(bytes.ok_or(TeXMouthError::BadFormatFile)?)
        };
        self.run_engine(dump_name, fmt_bytes.as_deref(), input_name, build_date)
    }

    /// C `tt_run_engine` — drives a complete engine invocation on this
    /// state: INITEX primitive setup or format load, input start, the main
    /// control loop, and `final_cleanup`. Returns `history`.
    ///
    /// `dump_name` is the format-file name (`TEX_format_default`); in
    /// non-INITEX mode `fmt_bytes` supplies the format image (the caller
    /// resolves the file — C opens it through the I/O stack). `input_name`
    /// is the primary input file name opened through `start_input_source`.
    ///
    /// `build_date` seeds the `\time`/`\day`/`\month`/`\year` parameters and
    /// the random generator. C uses `localtime`; this port derives the
    /// broken-down date in UTC so engine behavior is platform-independent.
    pub fn run_engine(
        &mut self,
        dump_name: &str,
        fmt_bytes: Option<&[u8]>,
        input_name: &str,
        build_date: std::time::SystemTime,
    ) -> Result<u8, TeXMouthError> {
        self.tex_format_default = dump_name.to_string();

        self.initialize_more_variables();

        if self.in_initex_mode {
            self.get_strings_started();
            self.install_core_primitives();
            self.initialize_more_initex_variables()?;
            self.initialize_primitives_fmt()?;
            self.init_str_ptr = self.str_ptr;
            self.init_pool_ptr = self.pool_ptr;
        }

        self.selector = SELECTOR_TERM_ONLY;
        self.tally = 0;
        self.term_offset = 0;
        self.file_offset = 0;
        self.job_name = None;
        self.name_in_progress = false;
        self.log_opened = false;
        self.log_name = None;
        self.diag_events.clear();

        self.input_ptr = 0;
        self.max_in_stack = 0;
        self.in_open = 0;
        self.open_parens = 0;
        self.param_ptr = 0;
        self.scanner_status = crate::scanner::NORMAL;
        self.warning_index = TEX_NULL;
        self.first = 1;
        self.cur_input.state = NEW_LINE;
        self.cur_input.start = 1;
        self.cur_input.index = 0;
        self.line = 0;
        self.cur_input.name = 0;
        self.force_eof = false;
        self.align_state = 1_000_000;

        self.init_io();

        if self.in_initex_mode {
            self.no_new_control_sequence = false;
            self.max_reg_num = 32767;
            self.max_reg_help_line = "A register number must be between 0 and 32767.".to_string();
        }
        self.no_new_control_sequence = true;

        if !self.in_initex_mode {
            // C `open_fmt_file`: `pack_buffered_name(0, ".fmt")` resolves the
            // format name into `name_of_file` before `load_fmt_file` reads.
            self.pack_buffered_name();
            match fmt_bytes {
                Some(bytes) => self.load_fmt_file(bytes)?,
                None => return Err(TeXMouthError::BadFormatFile),
            }
        }

        let end_char = self.int_par(INT_PAR_END_LINE_CHAR)?;
        if !(0..=BIGGEST_CHAR).contains(&end_char) {
            self.cur_input.limit -= 1;
        } else {
            let limit = self.cur_input.limit as usize;
            if self.buffer.len() <= limit {
                self.buffer.resize(limit + 1, 0);
            }
            self.buffer[limit] = end_char;
        }

        let (secs, micros) = epoch_secs_micros(build_date);
        self.init_start_time(secs);
        if self.in_initex_mode {
            // INITEX zeroes the date for format-file reproducibility.
            self.set_int_par(INT_PAR_TIME, 0)?;
            self.set_int_par(INT_PAR_DAY, 0)?;
            self.set_int_par(INT_PAR_MONTH, 0)?;
            self.set_int_par(INT_PAR_YEAR, 0)?;
        } else {
            let (minutes, day, month, year) = civil_from_unix(secs);
            self.set_int_par(INT_PAR_TIME, minutes)?;
            self.set_int_par(INT_PAR_DAY, day)?;
            self.set_int_par(INT_PAR_MONTH, month)?;
            self.set_int_par(INT_PAR_YEAR, year)?;
        }

        if self.trie_not_ready {
            self.ensure_trie_arrays();
            self.fmem_ptr = 7;
        }

        self.random_seed = micros * 1000 + (secs % 1_000_000) as i32;
        self.rand.init(self.random_seed);

        if self.interaction == BATCH_MODE {
            self.selector = SELECTOR_NO_PRINT;
        } else {
            self.selector = SELECTOR_TERM_ONLY;
        }

        if self.semantic_pagination_enabled {
            self.set_int_par(INT_PAR_XETEX_GENERATE_ACTUAL_TEXT, 1)?;
        }

        // C `tt_run_engine` tail: map the driver SyncTeX flag onto
        // `\synctex` *after* the format load — the dumped value must not
        // override the driver's request.
        self.synctex_init_command();
        self.start_input_source(input_name)?;
        self.history = HISTORY_SPOTLESS;

        // C `main_control` prologue: `\everyjob` runs before the first
        // dispatch of the job.
        let every_job = self.local(LOCAL_EVERY_JOB)?;
        if every_job != TEX_NULL {
            self.begin_token_list(every_job, EVERY_JOB_TEXT)?;
        }

        // C `main_control` — dispatch until `\end`/`its_all_over` stops the
        // job (end-of-input without `\end` surfaces as `EndOfInput`).
        while !self.job_stopped {
            self.dispatch_x_command()?;
        }

        self.final_cleanup()?;
        self.close_files_and_terminate()?;
        Ok(self.history)
    }

    /// Prints the `\if…` primitive name for an incomplete conditional
    /// (`print_cmd_chr(IF_TEST, cur_if)`).
    fn print_if_test_name(&mut self, if_code: i32) -> Result<(), TeXMouthError> {
        let name = self.if_test_text(if_code)?;
        self.print_cstr(&name)
    }
}

/// C eqtb position for a `xf_prim_init_*` frozen alias name.
fn c_frozen_position(symbol: &str) -> i32 {
    match symbol {
        "FROZEN_PROTECTION" => C_FROZEN_PROTECTION,
        "FROZEN_CR" => C_FROZEN_CR,
        "FROZEN_END_GROUP" => C_FROZEN_END_GROUP,
        "FROZEN_RIGHT" => C_FROZEN_RIGHT,
        "FROZEN_FI" => C_FROZEN_FI,
        "FROZEN_END_TEMPLATE" => C_FROZEN_END_TEMPLATE,
        "FROZEN_ENDV" => C_FROZEN_ENDV,
        "FROZEN_RELAX" => C_FROZEN_RELAX,
        "END_WRITE" => C_END_WRITE,
        "FROZEN_DONT_EXPAND" => C_FROZEN_DONT_EXPAND,
        "FROZEN_SPECIAL" => C_FROZEN_SPECIAL,
        "FROZEN_PRIMITIVE" => C_FROZEN_PRIMITIVE,
        "FROZEN_NULL_FONT" => C_FROZEN_NULL_FONT,
        _ => C_FROZEN_PROTECTION,
    }
}

/// Seconds and microseconds since the Unix epoch for `build_date`
/// (`get_seconds_and_micros`).
fn epoch_secs_micros(t: std::time::SystemTime) -> (i64, i32) {
    match t.duration_since(std::time::SystemTime::UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_micros() as i32),
        Err(e) => (-(e.duration().as_secs() as i64), 0),
    }
}

/// Broken-down UTC civil date for `secs` seconds since the Unix epoch:
/// `(minutes_since_midnight, day, month, year)` — the deterministic
/// counterpart of C's `localtime`-based `get_date_and_time`
/// (Hinnant's civil-from-days algorithm).
pub(crate) fn civil_from_unix(secs: i64) -> (i32, i32, i32, i32) {
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let minutes = (secs_of_day / 60) as i32;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (minutes, d as i32, m as i32, year as i32)
}
