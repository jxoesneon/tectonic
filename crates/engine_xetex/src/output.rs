//! Pure-Rust port of `xetex-output.c` and `xetex-errors.c` — TeX's
//! diagnostic print machinery and fatal-error reporting.
//!
//! Characters flow through `print_raw_char`/`print_char` to a sink selected
//! by `selector`: the terminal (`term_out`, C's `rust_stdout`), the log
//! (`log_out`, C's `log_file`), a `\write` stream, the string pool
//! (`SELECTOR_NEW_STRING`), or the show-context trick buffer
//! (`SELECTOR_PSEUDO`). `warn_char` additionally mirrors terminal/log
//! output into the active diagnostic capture (`diag_capture`, C's
//! `current_diagnostic`).

use crate::scanner::{
    TeXMouthError, ABSORBING, ALIGNING, BACKED_UP, BACKED_UP_CHAR, CS_TOKEN_FLAG, DEFINING,
    END_MATCH_TOKEN, EVERY_CR_TEXT, EVERY_DISPLAY_TEXT, EVERY_EOF_TEXT, EVERY_HBOX_TEXT,
    EVERY_JOB_TEXT, EVERY_MATH_TEXT, EVERY_PAR_TEXT, EVERY_VBOX_TEXT, INSERTED, INTER_CHAR_TEXT,
    MACRO, MARK_TEXT, MATCHING, MATCH_TOKEN, MAX_CHAR_VAL, NULL_CS_ID, OUTPUT_TEXT,
    OUT_PARAM_TOKEN, PARAMETER, SKIPPING, TECTONIC_CODA_TEXT, TOKEN_LIST, U_TEMPLATE, V_TEMPLATE,
    WRITE_TEXT,
};
use crate::state::{
    CommandCode, TeXEngineState, BATCH_MODE, BIGGEST_CHAR, BIGGEST_USV, DIMEN_VAL_LIMIT,
    ERROR_STOP_MODE, HISTORY_ERROR_ISSUED, HISTORY_FATAL_ERROR, HISTORY_SPOTLESS,
    HISTORY_WARNING_ISSUED, INT_PAR_END_LINE_CHAR, INT_PAR_ERROR_CONTEXT_LINES,
    INT_PAR_ESCAPE_CHAR, INT_PAR_NEW_LINE_CHAR, INT_PAR_TRACING_ONLINE, LOCAL_ERR_HELP,
    SCRIPT_SIZE, SCROLL_MODE, SELECTOR_LOG_ONLY, SELECTOR_NEW_STRING, SELECTOR_NO_PRINT,
    SELECTOR_PSEUDO, SELECTOR_TERM_AND_LOG, SELECTOR_TERM_ONLY, TEXT_SIZE, TEX_NULL, TOO_BIG_CHAR,
};

impl TeXEngineState {
    /// Mirrors one terminal/log character into the active diagnostic
    /// (`warn_char`).
    fn warn_char(&mut self, c: i32) {
        if let Some(diag) = &mut self.diag_capture {
            push_byte(diag, c);
        }
    }

    /// Starts or replaces the diagnostic capture (`capture_to_diagnostic`).
    pub fn capture_to_diagnostic(&mut self, capture: bool) {
        self.diag_capture = if capture { Some(Vec::new()) } else { None };
    }

    /// Pushes `file:line: ` (or `!`) into the diagnostic capture,
    /// duplicating `print_file_line`'s logic
    /// (`diagnostic_print_file_line`).
    pub(crate) fn diagnostic_print_file_line(&mut self) {
        let mut level = self.in_open;
        while level > 0
            && self
                .file_sources
                .get(level)
                .is_none_or(|src| src.name.is_empty())
        {
            level -= 1;
        }
        if let Some(diag) = &mut self.diag_capture {
            if level == 0 {
                diag.push(b'!');
            } else {
                let line = if level != self.in_open {
                    self.line_stack.get(level + 1).copied().unwrap_or(0)
                } else {
                    self.line
                };
                let text = format!("{}:{}: ", self.file_sources[level].name, line);
                diag.extend_from_slice(text.as_bytes());
            }
        }
    }

    /// Opens an error diagnostic at the current position, prints the file
    /// and message through the selector machinery, and starts capturing
    /// (`error_here_with_diagnostic`).
    pub fn error_here_with_diagnostic(&mut self, message: &str) {
        self.capture_to_diagnostic(true);
        self.diagnostic_print_file_line();
        if let Some(diag) = &mut self.diag_capture {
            diag.extend_from_slice(message.as_bytes());
        }
        if self.file_line_error_style_p {
            let _ = self.print_file_line();
        } else {
            let _ = self.print_nl_cstr("! ");
        }
        let _ = self.print_cstr(message);
    }

    /// Emits a newline on the active sinks (`print_ln`).
    pub fn print_ln(&mut self) {
        match self.selector {
            SELECTOR_TERM_AND_LOG => {
                self.warn_char(i32::from(b'\n'));
                self.term_out.push(b'\n');
                self.log_out.push(b'\n');
                self.term_offset = 0;
                self.file_offset = 0;
            }
            SELECTOR_LOG_ONLY => {
                self.warn_char(i32::from(b'\n'));
                self.log_out.push(b'\n');
                self.file_offset = 0;
            }
            SELECTOR_TERM_ONLY => {
                self.warn_char(i32::from(b'\n'));
                self.term_out.push(b'\n');
                self.term_offset = 0;
            }
            SELECTOR_NO_PRINT | SELECTOR_PSEUDO | SELECTOR_NEW_STRING => {}
            sel => {
                let sel = sel as usize;
                if let Some(Some(stream)) = self.write_streams.get_mut(sel) {
                    stream.text.push('\n');
                }
            }
        }
    }

    /// Emits one raw UTF-16 code unit, tracking line offsets and wrapping at
    /// `max_print_line` (`print_raw_char`). `tally` always advances.
    pub fn print_raw_char(&mut self, s: i32, incr_offset: bool) {
        match self.selector {
            SELECTOR_TERM_AND_LOG => {
                self.warn_char(s);
                push_byte(&mut self.term_out, s);
                push_byte(&mut self.log_out, s);
                if incr_offset {
                    self.term_offset += 1;
                    self.file_offset += 1;
                }
                if self.term_offset == self.max_print_line {
                    self.term_out.push(b'\n');
                    self.term_offset = 0;
                }
                if self.file_offset == self.max_print_line {
                    self.log_out.push(b'\n');
                    self.file_offset = 0;
                }
            }
            SELECTOR_LOG_ONLY => {
                self.warn_char(s);
                push_byte(&mut self.log_out, s);
                if incr_offset {
                    self.file_offset += 1;
                }
                if self.file_offset == self.max_print_line {
                    self.log_out.push(b'\n');
                    self.file_offset = 0;
                }
            }
            SELECTOR_TERM_ONLY => {
                self.warn_char(s);
                push_byte(&mut self.term_out, s);
                if incr_offset {
                    self.term_offset += 1;
                }
                if self.term_offset == self.max_print_line {
                    self.term_out.push(b'\n');
                    self.term_offset = 0;
                }
            }
            SELECTOR_NO_PRINT => {}
            SELECTOR_PSEUDO => {
                if self.tally < self.trick_count && self.error_line > 0 {
                    let idx = (self.tally % self.error_line) as usize;
                    if self.trick_buf.len() <= idx {
                        self.trick_buf.resize(idx + 1, 0);
                    }
                    self.trick_buf[idx] = s;
                }
            }
            SELECTOR_NEW_STRING => {
                if self.pool_ptr < self.pool_size {
                    self.pool_push(s);
                }
            }
            sel => {
                let sel = sel as usize;
                let ch = char::from_u32(s as u32).unwrap_or('\u{FFFD}');
                if let Some(Some(stream)) = self.write_streams.get_mut(sel) {
                    stream.text.push(ch);
                }
            }
        }
        self.tally += 1;
    }

    /// Emits one character with TeX's `^^`-escape and UTF-8 conventions
    /// (`print_char`).
    pub fn print_char(&mut self, s: i32) -> Result<(), TeXMouthError> {
        if self.selector > SELECTOR_PSEUDO && !self.doing_special {
            if s >= 0x10000 {
                self.print_raw_char(0xD800 + (s - 0x10000) / 1024, true);
                self.print_raw_char(0xDC00 + (s - 0x10000) % 1024, true);
            } else {
                self.print_raw_char(s, true);
            }
            return Ok(());
        }

        if s == self.int_par(INT_PAR_NEW_LINE_CHAR)? && self.selector < SELECTOR_PSEUDO {
            self.print_ln();
            return Ok(());
        }

        if s < 32 && !self.doing_special {
            self.print_raw_char(i32::from(b'^'), true);
            self.print_raw_char(i32::from(b'^'), true);
            self.print_raw_char(s + 64, true);
        } else if s < 127 {
            self.print_raw_char(s, true);
        } else if s == 127 {
            if !self.doing_special {
                self.print_raw_char(i32::from(b'^'), true);
                self.print_raw_char(i32::from(b'^'), true);
                self.print_raw_char(i32::from(b'?'), true);
            } else {
                self.print_raw_char(s, true);
            }
        } else if s < 160 && !self.doing_special {
            self.print_raw_char(i32::from(b'^'), true);
            self.print_raw_char(i32::from(b'^'), true);
            let l = (s % 256) / 16;
            if l < 10 {
                self.print_raw_char(i32::from(b'0') + l, true);
            } else {
                self.print_raw_char(i32::from(b'a') + l - 10, true);
            }
            let l = s % 16;
            if l < 10 {
                self.print_raw_char(i32::from(b'0') + l, true);
            } else {
                self.print_raw_char(i32::from(b'a') + l - 10, true);
            }
        } else if self.selector == SELECTOR_PSEUDO {
            self.print_raw_char(s, true);
        } else if s < 2048 {
            self.print_raw_char(192 + s / 64, false);
            self.print_raw_char(128 + s % 64, true);
        } else if s < 0x10000 {
            self.print_raw_char(224 + s / 4096, false);
            self.print_raw_char(128 + (s % 4096) / 64, false);
            self.print_raw_char(128 + s % 64, true);
        } else {
            self.print_raw_char(240 + s / 0x40000, false);
            self.print_raw_char(128 + (s % 0x40000) / 4096, false);
            self.print_raw_char(128 + (s % 4096) / 64, false);
            self.print_raw_char(128 + s % 64, true);
        }
        Ok(())
    }

    /// Prints a `str_number`: a bare character code or a pool string with
    /// surrogate decoding (`print`).
    pub fn print(&mut self, s: i32) -> Result<(), TeXMouthError> {
        if s >= self.str_ptr {
            return self.print_cstr("???");
        } else if s <= BIGGEST_CHAR {
            if s < 0 {
                return self.print_cstr("???");
            }
            if self.selector > SELECTOR_PSEUDO {
                return self.print_char(s);
            }
            if s == self.int_par(INT_PAR_NEW_LINE_CHAR)? && self.selector < SELECTOR_PSEUDO {
                self.print_ln();
                return Ok(());
            }
            let nl = self.int_par(INT_PAR_NEW_LINE_CHAR)?;
            self.set_int_par(INT_PAR_NEW_LINE_CHAR, -1)?;
            self.print_char(s)?;
            self.set_int_par(INT_PAR_NEW_LINE_CHAR, nl)?;
            return Ok(());
        }

        let idx = (s - TOO_BIG_CHAR) as usize;
        let start = self.str_start.get(idx).copied().unwrap_or(0) as usize;
        let end = self.str_start.get(idx + 1).copied().unwrap_or(0) as usize;
        let mut i = start;
        while i < end {
            let unit = self.str_pool[i];
            if (0xD800..0xDC00).contains(&unit)
                && i + 1 < end
                && (0xDC00..0xE000).contains(&self.str_pool[i + 1])
            {
                self.print_char(0x10000 + (unit - 0xD800) * 1024 + self.str_pool[i + 1] - 0xDC00)?;
                i += 1;
            } else {
                self.print_char(unit)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// Prints ASCII text (`print_cstr`).
    pub fn print_cstr(&mut self, text: &str) -> Result<(), TeXMouthError> {
        for &b in text.as_bytes() {
            self.print_char(i32::from(b))?;
        }
        Ok(())
    }

    /// Starts a fresh line when mid-line, then prints a `str_number`
    /// (`print_nl`).
    pub fn print_nl(&mut self, s: i32) -> Result<(), TeXMouthError> {
        if (self.term_offset > 0 && self.selector % 2 == 1)
            || (self.file_offset > 0 && self.selector >= SELECTOR_LOG_ONLY)
        {
            self.print_ln();
        }
        self.print(s)
    }

    /// Starts a fresh line when mid-line, then prints ASCII text
    /// (`print_nl_cstr`).
    pub fn print_nl_cstr(&mut self, text: &str) -> Result<(), TeXMouthError> {
        if (self.term_offset > 0 && self.selector % 2 == 1)
            || (self.file_offset > 0 && self.selector >= SELECTOR_LOG_ONLY)
        {
            self.print_ln();
        }
        self.print_cstr(text)
    }

    /// Prints the escape character then a `str_number` (`print_esc`).
    pub fn print_esc(&mut self, s: i32) -> Result<(), TeXMouthError> {
        let c = self.int_par(INT_PAR_ESCAPE_CHAR)?;
        if (0..=BIGGEST_USV).contains(&c) {
            self.print_char(c)?;
        }
        self.print(s)
    }

    /// Prints the escape character then ASCII text (`print_esc_cstr`).
    pub fn print_esc_cstr(&mut self, text: &str) -> Result<(), TeXMouthError> {
        let c = self.int_par(INT_PAR_ESCAPE_CHAR)?;
        if (0..=BIGGEST_USV).contains(&c) {
            self.print_char(c)?;
        }
        self.print_cstr(text)
    }

    /// Prints the digits recorded in `dig` (`print_the_digs`).
    fn print_the_digs(&mut self, mut k: usize) -> Result<(), TeXMouthError> {
        while k > 0 {
            k -= 1;
            if self.dig[k] < 10 {
                self.print_char(i32::from(b'0') + i32::from(self.dig[k]))?;
            } else {
                self.print_char(55 + i32::from(self.dig[k]))?;
            }
        }
        Ok(())
    }

    /// Prints a signed decimal integer (`print_int`), preserving the C
    /// algorithm for `i32::MIN`-adjacent values.
    pub fn print_int(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let mut n = n;
        let mut k = 0usize;
        if n < 0 {
            self.print_char(i32::from(b'-'))?;
            if n > -100_000_000 {
                n = -n;
            } else {
                let mut m = -1 - n;
                n = m / 10;
                m = (m % 10) + 1;
                k = 1;
                if m < 10 {
                    self.dig[0] = m as u8;
                } else {
                    self.dig[0] = 0;
                    n += 1;
                }
            }
        }
        loop {
            self.dig[k] = (n % 10) as u8;
            n /= 10;
            k += 1;
            if n == 0 {
                break;
            }
        }
        self.print_the_digs(k)
    }

    /// Prints a control sequence's TeX form (`print_cs`): active characters
    /// print bare, `\csname\endcsname` gets the special form, letter
    /// catcodes get a trailing space.
    pub fn print_cs(&mut self, cs: i32) -> Result<(), TeXMouthError> {
        if self.cs_table.is_active(cs) {
            if let Some(&ch) = self.cs_table.name(cs).first() {
                self.print_char(ch)?;
            }
            return Ok(());
        }
        if cs == NULL_CS_ID {
            self.print_esc_cstr("csname")?;
            self.print_esc_cstr("endcsname")?;
            self.print_char(i32::from(b' '))?;
            return Ok(());
        }
        let name = self.cs_table.name(cs).to_vec();
        if name.is_empty() || name.iter().any(|&c| c < 0) {
            self.print_esc_cstr("NONEXISTENT.")?;
            self.print_char(i32::from(b' '))?;
            return Ok(());
        }
        if name.len() == 1 {
            let c = name[0];
            self.print_esc(c)?;
            if u32::try_from(c).is_ok_and(|u| self.cat_code(u) == CommandCode::Letter as u8) {
                self.print_char(i32::from(b' '))?;
            }
        } else {
            let s = self.intern_string(&name);
            self.print_esc(s)?;
            self.print_char(i32::from(b' '))?;
        }
        Ok(())
    }

    /// Prints a control sequence without the trailing space (`sprint_cs`).
    pub fn sprint_cs(&mut self, cs: i32) -> Result<(), TeXMouthError> {
        if self.cs_table.is_active(cs) {
            if let Some(&ch) = self.cs_table.name(cs).first() {
                self.print_char(ch)?;
            }
            return Ok(());
        }
        if cs == NULL_CS_ID {
            self.print_esc_cstr("csname")?;
            self.print_esc_cstr("endcsname")?;
            return Ok(());
        }
        let name = self.cs_table.name(cs).to_vec();
        if name.len() == 1 {
            self.print_esc(name[0])?;
        } else {
            let s = self.intern_string(&name);
            self.print_esc(s)?;
        }
        Ok(())
    }

    /// Renders a control sequence's name as UTF-8 text (`tt_get_cs_name`).
    pub fn get_cs_name(&self, cs: i32) -> String {
        let mut out = String::new();
        if let Ok(esc) = self.int_par(INT_PAR_ESCAPE_CHAR) {
            if (0..=BIGGEST_USV).contains(&esc) {
                if let Some(c) = char::from_u32(esc as u32) {
                    out.push(c);
                }
            }
        }
        if self.cs_table.is_active(cs) {
            if let Some(&ch) = self.cs_table.name(cs).first() {
                if let Some(c) = char::from_u32(ch as u32) {
                    out.push(c);
                }
            }
            return out;
        }
        if cs == NULL_CS_ID {
            out.push_str("csname\\endcsname");
            return out;
        }
        for &u in self.cs_table.name(cs) {
            if let Some(c) = char::from_u32(u as u32) {
                out.push(c);
            }
        }
        out
    }

    /// Prints a quoted file-name triple (`print_file_name`); `n`, `a`, `e`
    /// are pool `str_number`s or 0.
    pub fn print_file_name(&mut self, n: i32, a: i32, e: i32) -> Result<(), TeXMouthError> {
        let mut must_quote = false;
        let mut quote_char = 0i32;
        for s in [a, n, e] {
            if s == 0 {
                continue;
            }
            let idx = (s - TOO_BIG_CHAR) as usize;
            let mut j = self.str_start.get(idx).copied().unwrap_or(0) as usize;
            let end = self.str_start.get(idx + 1).copied().unwrap_or(0) as usize;
            while (!must_quote || quote_char == 0) && j < end {
                if self.str_pool[j] == i32::from(b' ') {
                    must_quote = true;
                } else if self.str_pool[j] == i32::from(b'"')
                    || self.str_pool[j] == i32::from(b'\'')
                {
                    must_quote = true;
                    quote_char = 73 - self.str_pool[j];
                }
                j += 1;
            }
        }

        if must_quote {
            if quote_char == 0 {
                quote_char = i32::from(b'"');
            }
            self.print_char(quote_char)?;
        }

        for s in [a, n, e] {
            if s == 0 {
                continue;
            }
            let idx = (s - TOO_BIG_CHAR) as usize;
            let mut j = self.str_start.get(idx).copied().unwrap_or(0) as usize;
            let end = self.str_start.get(idx + 1).copied().unwrap_or(0) as usize;
            while j < end {
                if self.str_pool[j] == quote_char {
                    self.print(quote_char)?;
                    quote_char = 73 - quote_char;
                    self.print(quote_char)?;
                }
                self.print(self.str_pool[j])?;
                j += 1;
            }
        }

        if quote_char != 0 {
            self.print_char(quote_char)?;
        }
        Ok(())
    }

    /// Prints `\textfont`, `\scriptfont`, or `\scriptscriptfont`
    /// (`print_size`).
    pub fn print_size(&mut self, s: i32) -> Result<(), TeXMouthError> {
        if s == TEXT_SIZE {
            self.print_esc_cstr("textfont")
        } else if s == SCRIPT_SIZE {
            self.print_esc_cstr("scriptfont")
        } else {
            self.print_esc_cstr("scriptscriptfont")
        }
    }

    /// Prints `\openout`/`\write`/`\closeout` plus the stream decoration
    /// (`print_write_whatsit`).
    pub fn print_write_whatsit(&mut self, s: &str, p: i32) -> Result<(), TeXMouthError> {
        self.print_esc_cstr(s)?;
        let stream = self.mem[(p + 1) as usize].s0;
        if stream < 16 {
            self.print_int(stream)
        } else if stream == 16 {
            self.print_char(i32::from(b'*'))
        } else {
            self.print_char(i32::from(b'-'))
        }
    }

    /// Prints a native-word node's text with surrogate handling
    /// (`print_native_word`).
    pub fn print_native_word(&mut self, p: i32) -> Result<(), TeXMouthError> {
        let len = i32::from(self.mem[(p + 4) as usize].b16_s1());
        let mut i = 0;
        while i < len {
            let mut c = i32::from(self.native_word_text_at(p, i)?);
            if (0xD800..0xDC00).contains(&c) {
                if i < len - 1 {
                    let cc = i32::from(self.native_word_text_at(p, i + 1)?);
                    if (0xDC00..0xE000).contains(&cc) {
                        c = 0x10000 + (c - 0xD800) * 1024 + (cc - 0xDC00);
                        self.print_char(c)?;
                        i += 1;
                    } else {
                        self.print(i32::from(b'.'))?;
                    }
                } else {
                    self.print(i32::from(b'.'))?;
                }
            } else {
                self.print_char(c)?;
            }
            i += 1;
        }
        Ok(())
    }

    /// Prints a sparse-register number (`print_sa_num`).
    pub fn print_sa_num(&mut self, q: i32) -> Result<(), TeXMouthError> {
        let mut q = q;
        let n = if self.mem[q as usize].b16_s1() < DIMEN_VAL_LIMIT as u16 {
            self.mem[(q + 1) as usize].s1
        } else {
            let mut n = i32::from(self.mem[q as usize].b16_s1() % 64);
            q = self.mem[q as usize].b32_s1();
            n += 64 * i32::from(self.mem[q as usize].b16_s1());
            q = self.mem[q as usize].b32_s1();
            n + 64
                * 64
                * (i32::from(self.mem[q as usize].b16_s1())
                    + 64 * i32::from(self.mem[self.mem[q as usize].s1 as usize].b16_s1()))
        };
        self.print_int(n)
    }

    /// Prints the `file:line:` error prefix (`print_file_line`).
    pub fn print_file_line(&mut self) -> Result<(), TeXMouthError> {
        let mut level = self.in_open;
        while level > 0
            && self
                .file_sources
                .get(level)
                .is_none_or(|src| src.name.is_empty())
        {
            level -= 1;
        }
        if level == 0 {
            self.print_nl_cstr("! ")
        } else {
            self.print_nl_cstr("")?;
            let name_id = self.file_sources[level].name_id;
            if name_id >= TOO_BIG_CHAR && name_id < self.str_ptr {
                self.print(name_id)?;
            } else {
                let text = self.file_sources[level].name.clone();
                self.print_cstr(&text)?;
            }
            self.print_char(i32::from(b':'))?;
            if level == self.in_open {
                self.print_int(self.line)?;
            } else {
                self.print_int(self.line_stack.get(level + 1).copied().unwrap_or(0))?;
            }
            self.print_cstr(": ")
        }
    }

    /// Prints a two-digit number (`print_two`).
    pub fn print_two(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let n = n.abs() % 100;
        self.print_char(i32::from(b'0') + n / 10)?;
        self.print_char(i32::from(b'0') + n % 10)
    }

    /// Prints a hex number with the `"` prefix (`print_hex`).
    pub fn print_hex(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let mut n = n;
        let mut k = 0usize;
        self.print_char(i32::from(b'"'))?;
        loop {
            self.dig[k] = (n % 16) as u8;
            n /= 16;
            k += 1;
            if n == 0 {
                break;
            }
        }
        self.print_the_digs(k)
    }

    /// Prints a lowercase Roman numeral (`print_roman_int`).
    pub fn print_roman_int(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let mut n = n;
        let roman_data = b"m2d5c2l5x2v5i";
        let mut j = 0usize;
        let mut k;
        let mut v = 1000i32;
        loop {
            while n >= v {
                self.print_char(i32::from(roman_data[j]))?;
                n -= v;
            }
            if n <= 0 {
                return Ok(());
            }
            k = j + 2;
            let mut u = v / i32::from(roman_data[k - 1] - b'0');
            if roman_data[k - 1] == b'2' {
                k += 2;
                u /= i32::from(roman_data[k - 1] - b'0');
            }
            if n + u >= v {
                self.print_char(i32::from(roman_data[k]))?;
                n += u;
            } else {
                j += 2;
                v /= i32::from(roman_data[j - 1] - b'0');
            }
        }
    }

    /// Prints the string under construction (`print_current_string`).
    pub fn print_current_string(&mut self) -> Result<(), TeXMouthError> {
        let mut j = self.str_start[(self.str_ptr - TOO_BIG_CHAR) as usize] as usize;
        while (j as i32) < self.pool_ptr {
            let unit = self.str_pool[j];
            self.print_char(unit)?;
            j += 1;
        }
        Ok(())
    }

    /// Prints a scaled point value in TeX's decimal notation
    /// (`print_scaled`).
    pub fn print_scaled(&mut self, s: i32) -> Result<(), TeXMouthError> {
        let mut s = s;
        if s < 0 {
            self.print_char(i32::from(b'-'))?;
            s = -s;
        }
        self.print_int(s / 0x10000)?;
        self.print_char(i32::from(b'.'))?;
        s = 10 * (s % 0x10000) + 5;
        let mut delta = 10i32;
        loop {
            if delta > 0x10000 {
                s += 0x8000 - 50000;
            }
            self.print_char(i32::from(b'0') + s / 0x10000)?;
            s = 10 * (s % 0x10000);
            delta *= 10;
            if s <= delta {
                break;
            }
        }
        Ok(())
    }

    /// Prints a `U+xxxx` code (`print_ucs_code`).
    pub fn print_ucs_code(&mut self, n: i32) -> Result<(), TeXMouthError> {
        let mut n = n;
        let mut k = 0usize;
        self.print_cstr("U+")?;
        loop {
            self.dig[k] = (n % 16) as u8;
            n /= 16;
            k += 1;
            if n == 0 {
                break;
            }
        }
        while k < 4 {
            self.dig[k] = 0;
            k += 1;
        }
        self.print_the_digs(k)
    }

    /// Enters diagnostic mode, lowering the selector when tracing is
    /// terminal-only (`begin_diagnostic`).
    pub fn begin_diagnostic(&mut self) -> Result<(), TeXMouthError> {
        self.old_setting = self.selector;
        if self.int_par(INT_PAR_TRACING_ONLINE)? <= 0 && self.selector == SELECTOR_TERM_AND_LOG {
            self.selector -= 1;
            if self.history == HISTORY_SPOTLESS {
                self.history = HISTORY_WARNING_ISSUED;
            }
        }
        Ok(())
    }

    /// Exits diagnostic mode, restoring the selector (`end_diagnostic`).
    pub fn end_diagnostic(&mut self, blank_line: bool) -> Result<(), TeXMouthError> {
        self.print_nl_cstr("")?;
        if blank_line {
            self.print_ln();
        }
        self.selector = self.old_setting;
        Ok(())
    }

    /// Prints a token list up to `l` characters with parameter-matching
    /// decorations (`show_token_list`); `q` is the current-context node.
    pub fn show_token_list(&mut self, p: i32, q: i32, l: i32) -> Result<(), TeXMouthError> {
        let mut p = p;
        let mut match_chr = i32::from(b'#');
        let mut n = i32::from(b'0');
        self.tally = 0;
        while p != TEX_NULL && self.tally < l {
            if p == q {
                self.first_count = self.tally;
                self.trick_count = self.tally + 1 + self.error_line - self.half_error_line;
                if self.trick_count < self.error_line {
                    self.trick_count = self.error_line;
                }
            }
            if p < self.hi_mem_min || p > self.mem.len() as i32 - 1 {
                self.print_esc_cstr("CLOBBERED.")?;
                return Ok(());
            }
            let info = self.mem[p as usize].s0;
            if info >= CS_TOKEN_FLAG {
                self.print_cs(info - CS_TOKEN_FLAG)?;
            } else {
                let m = info / MAX_CHAR_VAL;
                let c = info % MAX_CHAR_VAL;
                if info < 0 {
                    self.print_esc_cstr("BAD.")?;
                } else {
                    match m {
                        x if x == CommandCode::LeftBrace as i32
                            || x == CommandCode::RightBrace as i32
                            || x == CommandCode::MathShift as i32
                            || x == CommandCode::TabMark as i32
                            || x == CommandCode::SupMark as i32
                            || x == CommandCode::SubMark as i32
                            || x == CommandCode::Spacer as i32
                            || x == CommandCode::Letter as i32
                            || x == CommandCode::OtherChar as i32 =>
                        {
                            self.print_char(c)?;
                        }
                        x if x == CommandCode::MacParam as i32 => {
                            self.print_char(c)?;
                            self.print_char(c)?;
                        }
                        x if x == OUT_PARAM_TOKEN / MAX_CHAR_VAL => {
                            self.print_char(match_chr)?;
                            if c <= 9 {
                                self.print_char(c + 48)?;
                            } else {
                                self.print_char(i32::from(b'!'))?;
                            }
                        }
                        x if x == MATCH_TOKEN / MAX_CHAR_VAL => {
                            match_chr = c;
                            self.print_char(c)?;
                            n += 1;
                            self.print_char(n)?;
                            if n > i32::from(b'9') {
                                return Ok(());
                            }
                        }
                        x if x == END_MATCH_TOKEN / MAX_CHAR_VAL => {
                            if c == 0 {
                                self.print_cstr("->")?;
                            }
                        }
                        _ => {
                            self.print_esc_cstr("BAD.")?;
                        }
                    }
                }
            }
            p = self.mem[p as usize].s1;
        }
        if p != TEX_NULL {
            self.print_esc_cstr("ETC.")?;
        }
        Ok(())
    }

    /// Prints a token list for `\show`/`\meaning` (`token_show`).
    pub fn token_show(&mut self, p: i32) -> Result<(), TeXMouthError> {
        if p != TEX_NULL {
            self.show_token_list(self.mem[p as usize].s1, TEX_NULL, 10_000_000)?;
        }
        Ok(())
    }

    /// Reports a runaway scan context (`runaway`).
    pub fn runaway(&mut self) -> Result<(), TeXMouthError> {
        let mut p = TEX_NULL;
        if self.scanner_status > SKIPPING {
            match self.scanner_status {
                DEFINING | ABSORBING => {
                    self.print_nl_cstr(if self.scanner_status == DEFINING {
                        "Runaway definition"
                    } else {
                        "Runaway text"
                    })?;
                    p = self.def_ref;
                }
                MATCHING => {
                    self.print_nl_cstr("Runaway argument")?;
                    p = self.temp_head();
                }
                ALIGNING => {
                    self.print_nl_cstr("Runaway preamble")?;
                    p = self.hold_head();
                }
                _ => {}
            }
            if p != TEX_NULL {
                self.print_char(i32::from(b'?'))?;
                self.print_ln();
                self.show_token_list(self.mem[p as usize].s1, TEX_NULL, self.error_line - 10)?;
            }
        }
        Ok(())
    }

    /// Prints the input-stack context lines around the error
    /// (`show_context`), using the trick buffer to mark the error point.
    pub fn show_context(&mut self) -> Result<(), TeXMouthError> {
        let mut base_ptr = self.input_ptr;
        if self.input_stack.len() <= base_ptr {
            self.input_stack.push(self.cur_input);
        } else {
            self.input_stack[base_ptr] = self.cur_input;
        }
        let mut nn = -1i32;
        let mut bottom_line = false;

        loop {
            self.cur_input = self.input_stack[base_ptr];
            if self.cur_input.state != TOKEN_LIST && (self.cur_input.name > 19 || base_ptr == 0) {
                bottom_line = true;
            }
            if base_ptr == self.input_ptr
                || bottom_line
                || nn < self.int_par(INT_PAR_ERROR_CONTEXT_LINES)?
            {
                if base_ptr == self.input_ptr
                    || self.cur_input.state != TOKEN_LIST
                    || self.cur_input.index != BACKED_UP
                    || self.cur_input.loc != TEX_NULL
                {
                    self.tally = 0;
                    self.old_setting = self.selector;
                    let is_token_list = self.cur_input.state == TOKEN_LIST;
                    if !is_token_list {
                        if self.cur_input.name <= 17 {
                            if self.cur_input.name == 0 {
                                if base_ptr == 0 {
                                    self.print_nl_cstr("<*>")?;
                                } else {
                                    self.print_nl_cstr("<insert> ")?;
                                }
                            } else {
                                self.print_nl_cstr("<read ")?;
                                if self.cur_input.name == 17 {
                                    self.print_char(i32::from(b'*'))?;
                                } else {
                                    self.print_int(self.cur_input.name - 1)?;
                                }
                                self.print_char(i32::from(b'>'))?;
                            }
                        } else {
                            self.print_nl_cstr("l.")?;
                            if usize::from(self.cur_input.index) == self.in_open {
                                self.print_int(self.line)?;
                            } else {
                                self.print_int(
                                    self.line_stack
                                        .get(self.cur_input.index as usize + 1)
                                        .copied()
                                        .unwrap_or(0),
                                )?;
                            }
                        }
                        self.print_char(i32::from(b' '))?;
                    } else {
                        match self.cur_input.index {
                            PARAMETER => {
                                self.print_nl_cstr("<argument> ")?;
                            }
                            U_TEMPLATE | V_TEMPLATE => {
                                self.print_nl_cstr("<template> ")?;
                            }
                            BACKED_UP | BACKED_UP_CHAR => {
                                if self.cur_input.loc == TEX_NULL {
                                    self.print_nl_cstr("<recently read> ")?;
                                } else {
                                    self.print_nl_cstr("<to be read again> ")?;
                                }
                            }
                            INSERTED => {
                                self.print_nl_cstr("<inserted text> ")?;
                            }
                            MACRO => {
                                self.print_ln();
                                self.print_cs(self.cur_input.name)?;
                            }
                            OUTPUT_TEXT => {
                                self.print_nl_cstr("<output> ")?;
                            }
                            EVERY_PAR_TEXT => {
                                self.print_nl_cstr("<everypar> ")?;
                            }
                            EVERY_MATH_TEXT => {
                                self.print_nl_cstr("<everymath> ")?;
                            }
                            EVERY_DISPLAY_TEXT => {
                                self.print_nl_cstr("<everydisplay> ")?;
                            }
                            EVERY_HBOX_TEXT => {
                                self.print_nl_cstr("<everyhbox> ")?;
                            }
                            EVERY_VBOX_TEXT => {
                                self.print_nl_cstr("<everyvbox> ")?;
                            }
                            EVERY_JOB_TEXT => {
                                self.print_nl_cstr("<everyjob> ")?;
                            }
                            EVERY_CR_TEXT => {
                                self.print_nl_cstr("<everycr> ")?;
                            }
                            MARK_TEXT => {
                                self.print_nl_cstr("<mark> ")?;
                            }
                            EVERY_EOF_TEXT => {
                                self.print_nl_cstr("<everyeof> ")?;
                            }
                            INTER_CHAR_TEXT => {
                                self.print_nl_cstr("<XeTeXinterchartoks> ")?;
                            }
                            WRITE_TEXT => {
                                self.print_nl_cstr("<write> ")?;
                            }
                            TECTONIC_CODA_TEXT => {
                                self.print_nl_cstr("<TectonicCodaTokens> ")?;
                            }
                            _ => {
                                self.print_nl(i32::from(b'?'))?;
                            }
                        }
                    }
                    let l = self.tally;
                    self.tally = 0;
                    self.selector = SELECTOR_PSEUDO;
                    self.trick_count = 1_000_000;
                    if !is_token_list {
                        let j = if self
                            .buffer
                            .get(self.cur_input.limit as usize)
                            .copied()
                            .unwrap_or(0)
                            == self.int_par(INT_PAR_END_LINE_CHAR)?
                        {
                            self.cur_input.limit
                        } else {
                            self.cur_input.limit + 1
                        };
                        if j > 0 {
                            let mut i = self.cur_input.start;
                            while i < j {
                                if i == self.cur_input.loc {
                                    self.first_count = self.tally;
                                    self.trick_count =
                                        self.tally + 1 + self.error_line - self.half_error_line;
                                    if self.trick_count < self.error_line {
                                        self.trick_count = self.error_line;
                                    }
                                }
                                let b = self.buffer.get(i as usize).copied().unwrap_or(0);
                                self.print_char(b)?;
                                i += 1;
                            }
                        }
                    } else if self.cur_input.index < MACRO {
                        self.show_token_list(self.cur_input.start, self.cur_input.loc, 100_000)?;
                    } else {
                        let start = self.mem[self.cur_input.start as usize].s1;
                        self.show_token_list(start, self.cur_input.loc, 100_000)?;
                    }
                    self.selector = self.old_setting;
                    self.finish_context_line(l)?;
                    nn += 1;
                }
            } else if nn == self.int_par(INT_PAR_ERROR_CONTEXT_LINES)? {
                self.print_nl_cstr("...")?;
                nn += 1;
            }
            if bottom_line {
                break;
            }
            base_ptr = base_ptr.saturating_sub(1);
            if self.input_stack.len() <= base_ptr {
                break;
            }
        }
        self.cur_input = self.input_stack[self.input_ptr];
        Ok(())
    }

    /// Renders the recorded trick-buffer line with the error-point marker
    /// (the shared tail of `show_context`'s context loop).
    fn finish_context_line(&mut self, l: i32) -> Result<(), TeXMouthError> {
        if self.trick_count == 1_000_000 {
            self.first_count = self.tally;
            self.trick_count = self.tally + 1 + self.error_line - self.half_error_line;
            if self.trick_count < self.error_line {
                self.trick_count = self.error_line;
            }
        }
        let m = if self.tally < self.trick_count {
            self.tally - self.first_count
        } else {
            self.trick_count - self.first_count
        };
        let (mut p, n) = if l + self.first_count <= self.half_error_line {
            (0, l + self.first_count)
        } else {
            self.print_cstr("...")?;
            (
                l + self.first_count - self.half_error_line + 3,
                self.half_error_line,
            )
        };
        let mut q = p;
        while q < self.first_count {
            let ch = self
                .trick_buf
                .get((q % self.error_line.max(1)) as usize)
                .copied()
                .unwrap_or(0);
            self.print_char(ch)?;
            q += 1;
        }
        self.print_ln();
        let mut q = 1;
        while q <= n {
            self.print_raw_char(i32::from(b' '), true);
            q += 1;
        }
        if m + n <= self.error_line {
            p = self.first_count + m;
        } else {
            p = self.first_count + (self.error_line - n - 3);
        }
        let mut q = self.first_count;
        while q < p {
            let ch = self
                .trick_buf
                .get((q % self.error_line.max(1)) as usize)
                .copied()
                .unwrap_or(0);
            self.print_char(ch)?;
            q += 1;
        }
        if m + n > self.error_line {
            self.print_cstr("...")?;
        }
        Ok(())
    }

    /// Selector normalization before an error diagnostic
    /// (`pre_error_message`): route to terminal+log when the log is open,
    /// open the log if needed, degrade in batch mode, and start the
    /// diagnostic capture.
    pub fn pre_error_message(&mut self) {
        self.selector = if self.log_opened {
            SELECTOR_TERM_AND_LOG
        } else {
            SELECTOR_TERM_ONLY
        };
        if self.job_name.is_none() {
            self.log_opened = true;
        }
        if self.interaction == BATCH_MODE {
            self.selector -= 1;
        }
        self.error_here_with_diagnostic("");
    }

    /// Finishes a fatal diagnostic: end capture, downgrade interaction,
    /// report the error when requested, mark history fatal, and terminate
    /// (`post_error_message`).
    pub fn post_error_message(&mut self, need_to_print_it: bool) -> Result<(), TeXMouthError> {
        self.capture_to_diagnostic(false);
        if self.interaction == ERROR_STOP_MODE {
            self.interaction = SCROLL_MODE;
        }
        if need_to_print_it && self.log_opened {
            self.error()?;
        }
        self.history = HISTORY_FATAL_ERROR;
        self.job_stopped = true;
        Ok(())
    }

    /// The recoverable-error printer (`error`): mark history, print the
    /// context, count errors, and display help lines.
    pub fn error(&mut self) -> Result<(), TeXMouthError> {
        if self.history < HISTORY_ERROR_ISSUED {
            self.history = HISTORY_ERROR_ISSUED;
        }
        self.print_char(i32::from(b'.'))?;
        self.show_context()?;
        if self.halt_on_error_p {
            self.history = HISTORY_FATAL_ERROR;
            self.post_error_message(false)?;
            return Err(TeXMouthError::UnsupportedPrimitive(
                "halted on potentially-recoverable error as specified",
            ));
        }
        self.error_count += 1;
        if self.error_count == 100 {
            self.print_nl_cstr("(That makes 100 errors; please try again.)")?;
            self.history = HISTORY_FATAL_ERROR;
            self.post_error_message(false)?;
            return Err(TeXMouthError::UnsupportedPrimitive(
                "halted after 100 potentially-recoverable errors",
            ));
        }
        if self.interaction > BATCH_MODE {
            self.selector -= 1;
        }
        if self.use_err_help {
            self.print_ln();
            self.give_err_help()?;
        } else {
            while self.help_ptr > 0 {
                self.help_ptr -= 1;
                if let Some(line) = self.help_line.get(self.help_ptr).cloned() {
                    self.print_nl_cstr(&line)?;
                }
            }
        }
        self.print_ln();
        if self.interaction > BATCH_MODE {
            self.selector += 1;
        }
        self.print_ln();
        Ok(())
    }

    /// Prints `\errhelp` token text (`give_err_help`).
    pub fn give_err_help(&mut self) -> Result<(), TeXMouthError> {
        let err_help = self.local(LOCAL_ERR_HELP)?;
        if err_help != TEX_NULL {
            self.token_show(err_help)?;
        }
        Ok(())
    }

    /// Reports an unrecoverable TeX capacity problem (`overflow`): prints
    /// the message and aborts through the error channel.
    pub fn overflow(&mut self, what: &str, n: i32) -> Result<(), TeXMouthError> {
        self.pre_error_message();
        self.print_cstr("TeX capacity exceeded, sorry [")?;
        self.print_cstr(what)?;
        self.print_char(i32::from(b'='))?;
        self.print_int(n)?;
        self.print_char(i32::from(b']'))?;
        self.help_ptr = 2;
        self.help_line = vec![
            "you can ask a wizard to enlarge me.".to_string(),
            "If you really absolutely need more capacity,".to_string(),
        ];
        self.post_error_message(true)?;
        Err(TeXMouthError::UnsupportedPrimitive("halted on overflow()"))
    }

    /// Prints an emergency-stop diagnostic and aborts (`fatal_error`).
    pub fn fatal_error(&mut self, s: &str) -> Result<(), TeXMouthError> {
        self.pre_error_message();
        self.print_cstr("Emergency stop")?;
        self.print_nl_cstr(s)?;
        self.capture_to_diagnostic(false);
        self.job_stopped = true;
        Err(TeXMouthError::UnsupportedPrimitive(
            "halted on fatal_error()",
        ))
    }

    /// Prints a `confusion` internal-error diagnostic (`confusion`), then
    /// returns the aborting error.
    pub(crate) fn print_confusion(&mut self, s: &str) -> TeXMouthError {
        self.pre_error_message();
        let help: Vec<String>;
        if self.history < HISTORY_ERROR_ISSUED {
            let _ = self.print_cstr("This can't happen (");
            let _ = self.print_cstr(s);
            let _ = self.print_char(i32::from(b')'));
            help = vec!["I'm broken. Please show this to someone who can fix can fix".to_string()];
            self.help_ptr = 1;
        } else {
            let _ = self.print_cstr("I can't go on meeting you like this");
            help = vec![
                "in fact, I'm barely conscious. Please fix it and try again.".to_string(),
                "One of your faux pas seems to have wounded me deeply...".to_string(),
            ];
            self.help_ptr = 2;
        }
        self.help_line = help;
        let _ = self.post_error_message(true);
        TeXMouthError::UnsupportedPrimitive("halted on confusion()")
    }

    /// Prints a PDF-backend-style error diagnostic (`pdf_error`).
    pub fn pdf_error(&mut self, t: Option<&str>, p: &str) -> Result<(), TeXMouthError> {
        self.pre_error_message();
        self.print_cstr("Error")?;
        if let Some(t) = t {
            self.print_cstr(" (")?;
            self.print_cstr(t)?;
            self.print(i32::from(b')'))?;
        }
        self.print_cstr(": ")?;
        self.print_cstr(p)?;
        self.post_error_message(true)?;
        Err(TeXMouthError::UnsupportedPrimitive("halted on pdf_error()"))
    }
}

/// Appends one byte to `sink`, matching the C bridge's `ttstub_output_putc`
/// which writes a single output byte; values above 255 are UTF-8 encoded as
/// a safety net (the print paths only feed pre-encoded bytes and ASCII).
fn push_byte(sink: &mut Vec<u8>, c: i32) {
    if (0..=255).contains(&c) {
        sink.push(c as u8);
    } else if let Some(ch) = char::from_u32(c as u32) {
        let mut buf = [0u8; 4];
        sink.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    } else {
        sink.extend_from_slice("�".as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        EMPTY_STRING, SELECTOR_LOG_ONLY, SELECTOR_NEW_STRING, SELECTOR_NO_PRINT, SELECTOR_PSEUDO,
        SELECTOR_TERM_AND_LOG, SELECTOR_TERM_ONLY, TOO_BIG_CHAR,
    };

    fn engine() -> TeXEngineState {
        TeXEngineState::new(4_096)
    }

    fn term(state: &TeXEngineState) -> String {
        String::from_utf8_lossy(&state.term_out).into_owned()
    }

    fn log(state: &TeXEngineState) -> String {
        String::from_utf8_lossy(&state.log_out).into_owned()
    }

    #[test]
    fn get_strings_started_loads_constants() {
        let state = engine();
        assert_eq!(state.str_ptr, TOO_BIG_CHAR + 2);
        assert_eq!(state.length(EMPTY_STRING), 0);
        assert_eq!(state.string_units(EMPTY_STRING), Some(&[][..]));
        let first = state.string_units(TOO_BIG_CHAR).unwrap();
        assert_eq!(first.len(), "this marks the start of the stringpool".len());
    }

    #[test]
    fn intern_string_deduplicates_via_slow_make_string() {
        let mut state = engine();
        let units: Vec<i32> = "hello".chars().map(|c| c as i32).collect();
        let a = state.intern_string(&units);
        let b = state.intern_string(&units);
        assert_eq!(a, b);
        let other: Vec<i32> = "world".chars().map(|c| c as i32).collect();
        let c = state.intern_string(&other);
        assert_ne!(a, c);
        assert_eq!(state.string_units(a), Some(&units[..]));
        let empty = state.intern_string(&[]);
        assert_eq!(empty, EMPTY_STRING);
    }

    #[test]
    fn str_eq_str_and_search_string() {
        let mut state = engine();
        let units: Vec<i32> = "abc".chars().map(|c| c as i32).collect();
        let a = state.intern_string(&units);
        // Manually construct the same string a second time via raw pool ops.
        for &u in &units {
            state.pool_push(u);
        }
        let t = state.make_string().unwrap();
        assert_ne!(a, t);
        assert!(state.str_eq_str(a, t));
        assert_eq!(state.search_string(t), a);
        // slow_make_string frees the duplicate.
        state.str_ptr -= 1;
        state.pool_ptr = state.str_start[(state.str_ptr - TOO_BIG_CHAR) as usize];
        state.str_pool.truncate(state.pool_ptr as usize);
        for &u in &units {
            state.pool_push(u);
        }
        assert_eq!(state.slow_make_string().unwrap(), a);
    }

    #[test]
    fn append_str_copies_pool_text() {
        let mut state = engine();
        let units: Vec<i32> = "xy".chars().map(|c| c as i32).collect();
        let a = state.intern_string(&units);
        let before = state.pool_ptr;
        state.append_str(a).unwrap();
        assert_eq!(state.pool_ptr, before + 2);
        let s = state.make_string().unwrap();
        assert_eq!(state.string_units(s), Some(&units[..]));
    }

    #[test]
    fn print_int_variants() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        state.print_int(0).unwrap();
        state.print_int(-42).unwrap();
        state.print_int(1_234_567).unwrap();
        assert_eq!(term(&state), "0-421234567");
    }

    #[test]
    fn print_scaled_and_hex_and_roman_and_two_and_ucs() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        state.print_scaled(0x10000).unwrap(); // 1.0pt
        state.print_char(i32::from(b' ')).unwrap();
        state.print_scaled(-3 * 0x8000).unwrap(); // -1.5
        state.print_char(i32::from(b' ')).unwrap();
        state.print_hex(255).unwrap();
        state.print_char(i32::from(b' ')).unwrap();
        state.print_roman_int(1987).unwrap();
        state.print_char(i32::from(b' ')).unwrap();
        state.print_two(7).unwrap();
        state.print_char(i32::from(b' ')).unwrap();
        state.print_ucs_code(0x4E2D).unwrap();
        assert_eq!(term(&state), "1.0 -1.5 \"FF mcmlxxxvii 07 U+4E2D");
    }

    #[test]
    fn print_char_escapes_controls_and_utf8() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        state.print_char(1).unwrap(); // ^^A
        state.print_char(127).unwrap(); // ^^?
        state.print_char(0x80).unwrap(); // ^^80
        state.print_char(i32::from(b'x')).unwrap();
        state.print_char(0x20AC).unwrap(); // euro -> UTF-8
        let text = term(&state);
        assert!(text.starts_with("^^A^^?^^80x"));
        assert!(text.contains('\u{20AC}'));
    }

    #[test]
    fn selector_routes_to_sinks() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        state.print_cstr("term").unwrap();
        state.selector = SELECTOR_LOG_ONLY;
        state.print_cstr("log").unwrap();
        state.selector = SELECTOR_TERM_AND_LOG;
        state.print_cstr("both").unwrap();
        state.selector = SELECTOR_NO_PRINT;
        state.print_cstr("hidden").unwrap();
        assert_eq!(term(&state), "termboth");
        assert_eq!(log(&state), "logboth");
    }

    #[test]
    fn new_string_selector_builds_pool_string() {
        let mut state = engine();
        state.selector = SELECTOR_NEW_STRING;
        state.print_cstr("pooled").unwrap();
        let s = state.make_string().unwrap();
        assert_eq!(state.string_units(s).unwrap().len(), 6);
        state.selector = SELECTOR_TERM_ONLY;
        state.print(s).unwrap();
        assert_eq!(term(&state), "pooled");
    }

    #[test]
    fn pseudo_selector_records_trick_buf() {
        let mut state = engine();
        state.selector = SELECTOR_PSEUDO;
        state.trick_count = 10;
        state.tally = 0;
        state.print_cstr("ab").unwrap();
        assert_eq!(state.trick_buf[0], i32::from(b'a'));
        assert_eq!(state.trick_buf[1], i32::from(b'b'));
        assert_eq!(state.tally, 2);
    }

    #[test]
    fn print_cs_letter_and_csname() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        // Single-letter cs with catcode LETTER prints a trailing space.
        state.print_cs(NULL_CS_ID).unwrap();
        assert_eq!(term(&state), "\\csname\\endcsname ");
    }

    #[test]
    fn print_file_name_quotes_spaces() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        let name: Vec<i32> = "my file.tex".chars().map(|c| c as i32).collect();
        let n = state.intern_string(&name);
        state.print_file_name(n, 0, 0).unwrap();
        assert_eq!(term(&state), "\"my file.tex\"");
    }

    #[test]
    fn error_reports_context_and_help() {
        let mut state = engine();
        state.interaction = ERROR_STOP_MODE;
        state.help_ptr = 1;
        state.help_line = vec!["try harder".to_string()];
        // TERM_AND_LOG decrements to LOG_ONLY for the help block per C's
        // `selector--` in error().
        state.selector = SELECTOR_TERM_AND_LOG;
        state.error().unwrap();
        let text = term(&state);
        let log_text = log(&state);
        assert!(text.contains('.'));
        assert!(log_text.contains("try harder"));
        assert_eq!(state.selector, SELECTOR_TERM_AND_LOG);
        assert_eq!(state.history, HISTORY_ERROR_ISSUED);
        assert_eq!(state.error_count, 1);
        assert_eq!(state.help_ptr, 0);
    }

    #[test]
    fn pre_and_post_error_message_flow() {
        let mut state = engine();
        state.interaction = ERROR_STOP_MODE;
        state.pre_error_message();
        assert_eq!(state.selector, SELECTOR_TERM_ONLY);
        assert!(state.diag_capture.is_some());
        state.post_error_message(false).unwrap();
        assert_eq!(state.interaction, SCROLL_MODE);
        assert_eq!(state.history, HISTORY_FATAL_ERROR);
        assert!(state.diag_capture.is_none());
        assert!(state.job_stopped);
    }

    #[test]
    fn show_token_list_renders_tokens() {
        let mut state = engine();
        state.selector = SELECTOR_TERM_ONLY;
        // Build a one-token list: letter 'x' (info = 11*MAX_CHAR_VAL + 'x').
        let node = state.get_avail().unwrap();
        state.mem[node as usize].s0 = 11 * MAX_CHAR_VAL + i32::from(b'x');
        state.mem[node as usize].s1 = TEX_NULL;
        state.show_token_list(node, TEX_NULL, 100).unwrap();
        assert_eq!(term(&state), "x");
    }
}
