// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust TeX expansion loop and macro-call machinery.
//!
//! Fulfills the second half of Phase 5 Milestone 2 in `MIGRATION_ROADMAP.md`:
//! - Ports `expand`, `get_x_token`, `x_token`, and `macro_call` from
//!   `xetex-xetex0.c`.
//! - Provides a compact primitive dispatch table through the control-sequence
//!   table in [`crate::scanner`].
//! - Keeps macro arguments and replacement text in TeX's linked token-list
//!   representation so `OUT_PARAM` substitution remains lazy and re-entrant.

use std::collections::VecDeque;

use crate::scan::{DIMEN_VAL, INT_VAL, TOK_VAL};
use crate::scanner::{
    TeXMouthError, BIGGEST_USV, CAT_ACTIVE_CHAR, CS_TOKEN_FLAG, END_MATCH_TOKEN, FROZEN_RELAX_ID,
    INSERTED, LEFT_BRACE_LIMIT, MACRO, MARK_TEXT, MATCH_TOKEN, MAX_CHAR_VAL, MAX_COMMAND, NORMAL,
    NO_EXPAND_FLAG, NULL_CS_ID, PROTECTED_TOKEN, RIGHT_BRACE_LIMIT, SPACE_TOKEN, TOKEN_LIST,
    TOO_BIG_USV, V_TEMPLATE,
};
use crate::state::{
    CommandCode, TeXEngineState, TeXMode, ADJUST_NODE, DISC_NODE, GLUE_NODE, HLIST_NODE, INS_NODE,
    KERN_NODE, MARGIN_KERN_NODE, MARK_NODE, MATH_NODE, NORMAL as NODE_NORMAL, PENALTY_NODE,
    SA_BOX_VAL, SA_MARK_VAL, TEX_NULL,
};

pub(crate) const IF_CODE: u8 = 1;
pub(crate) const FI_CODE: i32 = 2;
pub(crate) const ELSE_CODE: i32 = 3;
pub(crate) const OR_CODE: i32 = 4;
pub(crate) const UNLESS_CODE: i32 = 32;
const IF_CASE_CODE: i32 = 16;
const IF_NODE_SIZE: i32 = 2;

#[derive(Clone, Debug)]
enum ParameterSpec {
    DelimitersOnly(Vec<i32>),
    Argument(Vec<i32>),
}

impl TeXEngineState {
    /// Defines a macro control sequence from TeX token lists.
    ///
    /// `parameter_text` should contain `MATCH_TOKEN` markers, delimiter tokens,
    /// and the terminating `END_MATCH_TOKEN`, exactly as produced by `scan_toks`.
    /// `replacement` is the macro replacement text and may contain
    /// `OUT_PARAM_TOKEN + n` parameter references.
    pub fn define_macro(
        &mut self,
        name: &str,
        parameter_text: &[i32],
        replacement: &[i32],
        command: CommandCode,
    ) -> Result<i32, TeXMouthError> {
        let mut definition = Vec::with_capacity(parameter_text.len() + replacement.len() + 1);
        definition.extend_from_slice(parameter_text);
        if !parameter_text.contains(&END_MATCH_TOKEN) {
            definition.push(END_MATCH_TOKEN);
        }
        definition.extend_from_slice(replacement);

        let definition_head = self.store_token_list(&definition)?;
        let reference = match self.get_avail() {
            Ok(reference) => reference,
            Err(err) => {
                self.flush_list(definition_head)?;
                return Err(err.into());
            }
        };
        self.mem[reference as usize].s0 = TEX_NULL;
        self.mem[reference as usize].s1 = definition_head;

        Ok(self.cs_table.define_str(name, command, reference))
    }

    /// Defines a zero-argument macro replacement.
    pub fn define_simple_macro(
        &mut self,
        name: &str,
        replacement: &[i32],
    ) -> Result<i32, TeXMouthError> {
        self.define_macro(name, &[END_MATCH_TOKEN], replacement, CommandCode::Call)
    }

    /// Performs one expansion step (`expand`).
    ///
    /// The five scanner/result globals and the backed-up token-list chain are
    /// saved and restored around the expansion, matching the C implementation's
    /// re-entrant behavior.
    pub fn expand(&mut self) -> Result<(), TeXMouthError> {
        if self.expand_depth_count >= self.expand_depth {
            return Err(TeXMouthError::ExpansionDepthExceeded);
        }

        self.expand_depth_count += 1;
        let cv_backup = self.cur_val;
        let cvl_backup = self.cur_val_level;
        let radix_backup = self.radix;
        let co_backup = self.cur_order;
        let backup_head = self.backup_head();
        let backup_index = self.checked_engine_mem_index(backup_head)?;
        let backup_link = self.mem[backup_index].s1;

        let result = self.expand_once();

        self.cur_val = cv_backup;
        self.cur_val_level = cvl_backup;
        self.radix = radix_backup;
        self.cur_order = co_backup;
        self.mem[backup_index].s1 = backup_link;
        self.expand_depth_count -= 1;
        result
    }

    /// Reads one fully expanded token (`get_x_token`).
    pub fn get_x_token(&mut self) -> Result<(), TeXMouthError> {
        loop {
            self.get_next()?;
            if self.cur_cmd <= MAX_COMMAND {
                break;
            }
            if self.cur_cmd >= CommandCode::Call as u8 {
                if self.cur_cmd < CommandCode::EndTemplate as u8 {
                    self.macro_call()?;
                    continue;
                }
                self.cur_cs = crate::scanner::FROZEN_ENDV_ID;
                self.cur_cmd = CommandCode::Endv as u8;
                break;
            }
            self.expand()?;
        }
        self.set_cur_tok_from_current();
        Ok(())
    }

    /// Expands until the current token is non-expandable (`x_token`).
    pub fn x_token(&mut self) -> Result<(), TeXMouthError> {
        while self.cur_cmd > MAX_COMMAND {
            self.expand()?;
            self.get_next()?;
        }
        self.set_cur_tok_from_current();
        Ok(())
    }

    /// Expands one macro call (`macro_call`).
    ///
    /// The parameter template stored after the macro reference-count node is
    /// interpreted as alternating `MATCH_TOKEN` boundaries and delimiter runs,
    /// preserving TeX's delimited/undelimited argument behavior.
    pub fn macro_call(&mut self) -> Result<(), TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        let save_warning_index = self.warning_index;

        self.warning_index = self.cur_cs;
        let result = self.macro_call_inner();

        self.scanner_status = save_scanner_status;
        self.warning_index = save_warning_index;
        result
    }

    /// Returns the next fully expanded TeX token value.
    pub fn next_expanded_token(&mut self) -> Result<i32, TeXMouthError> {
        self.get_x_token()?;
        Ok(self.cur_tok)
    }

    fn expand_once(&mut self) -> Result<(), TeXMouthError> {
        loop {
            if self.cur_cmd < CommandCode::Call as u8 {
                // C `expand` reswitch: `\tracingcommands > 1` reports every
                // expandable command the expander processes.
                if self.int_par(crate::state::INT_PAR_TRACING_COMMANDS)? > 1 {
                    self.show_cur_cmd_chr()?;
                }
                match self.cur_cmd {
                    x if x == CommandCode::TopBotMark as u8 => {
                        self.expand_top_bot_mark()?;
                        return Ok(());
                    }
                    x if x == CommandCode::ExpandAfter as u8 => {
                        if self.cur_chr == 0 {
                            self.expand_after()?;
                            return Ok(());
                        }
                        if self.expand_unless()? {
                            continue;
                        }
                        return Ok(());
                    }
                    x if x == CommandCode::NoExpand as u8 => {
                        if self.cur_chr == 0 {
                            self.expand_noexpand()?;
                            return Ok(());
                        }
                        if self.expand_primitive()? {
                            continue;
                        }
                        return Ok(());
                    }
                    x if x == CommandCode::Input as u8 => {
                        match self.cur_chr {
                            0 => {
                                if self.name_in_progress {
                                    self.insert_relax()?;
                                } else {
                                    self.start_input()?;
                                }
                            }
                            1 => self.force_eof = true,
                            2 => self.pseudo_start()?,
                            _ => {
                                return Err(TeXMouthError::UnsupportedPrimitive("input primitive"))
                            }
                        }
                        return Ok(());
                    }
                    x if x == CommandCode::IfTest as u8 => {
                        self.conditional()?;
                        return Ok(());
                    }
                    x if x == CommandCode::FiOrElse as u8 => {
                        self.expand_fi_or_else()?;
                        return Ok(());
                    }
                    x if x == CommandCode::CsName as u8 => {
                        self.expand_cs_name()?;
                        return Ok(());
                    }
                    x if x == CommandCode::Convert as u8 => {
                        self.expand_convert()?;
                        return Ok(());
                    }
                    x if x == CommandCode::The as u8 => {
                        self.expand_the()?;
                        return Ok(());
                    }
                    x if x == CommandCode::UndefinedCs as u8 => {
                        return Err(TeXMouthError::UndefinedControlSequence(self.cur_cs));
                    }
                    _ => return Err(TeXMouthError::UnsupportedPrimitive("expandable command")),
                }
            } else if self.cur_cmd < CommandCode::EndTemplate as u8 {
                return self.macro_call();
            }

            self.cur_tok = CS_TOKEN_FLAG + crate::scanner::FROZEN_ENDV_ID;
            self.back_input()?;
            return Ok(());
        }
    }

    fn macro_call_inner(&mut self) -> Result<(), TeXMouthError> {
        let reference = self.cur_chr;
        let (specifications, end_match) = self.parse_parameter_template(reference)?;
        self.long_state = if self.cur_cmd >= CommandCode::OuterCall as u8 {
            self.cur_cmd - 2
        } else {
            self.cur_cmd
        };

        self.scanner_status = crate::scanner::MATCHING;
        self.pstack = [TEX_NULL; 9];
        let mut arg_count = 0usize;

        let matching_result = (|| {
            for specification in &specifications {
                match specification {
                    ParameterSpec::DelimitersOnly(delimiters) => {
                        self.consume_delimiters(delimiters)?;
                    }
                    ParameterSpec::Argument(delimiters) => {
                        if arg_count >= self.pstack.len() {
                            return Err(TeXMouthError::ParameterStackOverflow);
                        }
                        let argument = self.scan_macro_argument(delimiters)?;
                        self.pstack[arg_count] = self.store_token_list(&argument)?;
                        arg_count += 1;
                    }
                }
            }
            Ok(())
        })();

        if let Err(err) = matching_result {
            let argument_heads = self.pstack;
            for head in &argument_heads[..arg_count] {
                if *head != TEX_NULL {
                    self.flush_list(*head)?;
                }
            }
            return Err(err);
        }

        while self.cur_input.state == TOKEN_LIST
            && self.cur_input.loc == TEX_NULL
            && self.cur_input.index != V_TEMPLATE
        {
            self.end_token_list()?;
        }

        if self.param_ptr + arg_count > self.param_stack_capacity {
            return Err(TeXMouthError::ParameterStackOverflow);
        }

        self.begin_token_list(reference, MACRO)?;
        self.cur_input.name = self.warning_index;
        self.cur_input.loc = self.mem[end_match as usize].s1;

        if self.param_ptr + arg_count > self.param_stack.len() {
            self.param_stack
                .resize(self.param_ptr + arg_count, TEX_NULL);
        }
        for (offset, &head) in self.pstack.iter().take(arg_count).enumerate() {
            self.param_stack[self.param_ptr + offset] = head;
        }
        self.param_ptr += arg_count;
        self.max_param_stack = self.max_param_stack.max(self.param_ptr);
        Ok(())
    }

    fn parse_parameter_template(
        &self,
        reference: i32,
    ) -> Result<(Vec<ParameterSpec>, i32), TeXMouthError> {
        let mut specifications = Vec::new();
        let mut r = self.mem[self.checked_engine_mem_index(reference)?].s1;
        if r == TEX_NULL {
            return Err(TeXMouthError::MalformedMacroParameters);
        }

        if self.mem[self.checked_engine_mem_index(r)?].s0 == PROTECTED_TOKEN {
            r = self.mem[self.checked_engine_mem_index(r)?].s1;
            if r == TEX_NULL {
                return Err(TeXMouthError::MalformedMacroParameters);
            }
        }

        while self.mem[self.checked_engine_mem_index(r)?].s0 != END_MATCH_TOKEN {
            if self.is_match_marker(r)? {
                r = self.mem[self.checked_engine_mem_index(r)?].s1;
                let mut delimiters = Vec::new();
                while r != TEX_NULL && !self.is_match_boundary(r)? {
                    delimiters.push(self.mem[self.checked_engine_mem_index(r)?].s0);
                    r = self.mem[self.checked_engine_mem_index(r)?].s1;
                }
                if r == TEX_NULL {
                    return Err(TeXMouthError::MalformedMacroParameters);
                }
                specifications.push(ParameterSpec::Argument(delimiters));
            } else {
                let mut delimiters = Vec::new();
                while r != TEX_NULL && !self.is_match_boundary(r)? {
                    delimiters.push(self.mem[self.checked_engine_mem_index(r)?].s0);
                    r = self.mem[self.checked_engine_mem_index(r)?].s1;
                }
                if r == TEX_NULL {
                    return Err(TeXMouthError::MalformedMacroParameters);
                }
                specifications.push(ParameterSpec::DelimitersOnly(delimiters));
            }
        }

        Ok((specifications, r))
    }

    fn is_match_marker(&self, node: i32) -> Result<bool, TeXMouthError> {
        let token = self.mem[self.checked_engine_mem_index(node)?].s0;
        Ok((MATCH_TOKEN..END_MATCH_TOKEN).contains(&token))
    }

    fn is_match_boundary(&self, node: i32) -> Result<bool, TeXMouthError> {
        let token = self.mem[self.checked_engine_mem_index(node)?].s0;
        Ok((MATCH_TOKEN..=END_MATCH_TOKEN).contains(&token))
    }

    fn consume_delimiters(&mut self, delimiters: &[i32]) -> Result<(), TeXMouthError> {
        for (index, &expected) in delimiters.iter().enumerate() {
            self.get_token()?;
            if self.cur_tok != expected {
                return Err(TeXMouthError::MacroArgumentMismatch {
                    expected,
                    found: self.cur_tok,
                });
            }
            if index + 1 == delimiters.len() && self.cur_tok < LEFT_BRACE_LIMIT {
                self.align_state -= 1;
            }
        }
        Ok(())
    }

    fn scan_macro_argument(&mut self, delimiters: &[i32]) -> Result<Vec<i32>, TeXMouthError> {
        if delimiters.is_empty() {
            return self.scan_undelimited_argument();
        }
        self.scan_delimited_argument(delimiters)
    }

    fn scan_undelimited_argument(&mut self) -> Result<Vec<i32>, TeXMouthError> {
        loop {
            self.get_token()?;
            if self.cur_tok == SPACE_TOKEN {
                continue;
            }
            self.check_argument_par_token()?;
            if self.cur_tok < LEFT_BRACE_LIMIT {
                let (inner, _) = self.read_balanced_group()?;
                return Ok(inner);
            }
            if (LEFT_BRACE_LIMIT..RIGHT_BRACE_LIMIT).contains(&self.cur_tok) {
                return Err(TeXMouthError::ExtraRightBrace);
            }
            return Ok(vec![self.cur_tok]);
        }
    }

    fn scan_delimited_argument(&mut self, delimiters: &[i32]) -> Result<Vec<i32>, TeXMouthError> {
        let mut argument = Vec::new();
        let mut argument_units = 0usize;

        loop {
            self.get_token()?;
            if self.cur_tok == delimiters[0] {
                let mut lookahead = vec![self.cur_tok];
                let mut matched = true;
                for &expected in &delimiters[1..] {
                    self.get_token()?;
                    lookahead.push(self.cur_tok);
                    if self.cur_tok != expected {
                        matched = false;
                        break;
                    }
                }

                if matched {
                    if self.cur_tok < LEFT_BRACE_LIMIT {
                        self.align_state -= 1;
                    }
                    if argument_units == 1 && argument.len() >= 2 {
                        let first = argument[0];
                        let last = argument[argument.len() - 1];
                        if first < LEFT_BRACE_LIMIT
                            && (LEFT_BRACE_LIMIT..RIGHT_BRACE_LIMIT).contains(&last)
                        {
                            argument.remove(0);
                            argument.pop();
                        }
                    }
                    return Ok(argument);
                }

                self.unget_tokens(&lookahead)?;
                self.get_token()?;
            }

            self.check_argument_par_token()?;
            self.append_argument_token(&mut argument)?;
            argument_units += 1;
        }
    }

    fn append_argument_token(&mut self, argument: &mut Vec<i32>) -> Result<(), TeXMouthError> {
        if self.cur_tok < LEFT_BRACE_LIMIT {
            let opening = self.cur_tok;
            let (mut inner, closing) = self.read_balanced_group()?;
            argument.push(opening);
            argument.append(&mut inner);
            argument.push(closing);
            return Ok(());
        }
        if (LEFT_BRACE_LIMIT..RIGHT_BRACE_LIMIT).contains(&self.cur_tok) {
            return Err(TeXMouthError::ExtraRightBrace);
        }
        argument.push(self.cur_tok);
        Ok(())
    }

    fn read_balanced_group(&mut self) -> Result<(Vec<i32>, i32), TeXMouthError> {
        let mut unbalance = 1i32;
        let mut tokens = Vec::new();

        loop {
            self.get_token()?;
            self.check_argument_par_token()?;
            if self.cur_tok < LEFT_BRACE_LIMIT {
                unbalance += 1;
            } else if (LEFT_BRACE_LIMIT..RIGHT_BRACE_LIMIT).contains(&self.cur_tok) {
                unbalance -= 1;
                if unbalance == 0 {
                    return Ok((tokens, self.cur_tok));
                }
            }
            tokens.push(self.cur_tok);
        }
    }

    fn check_argument_par_token(&self) -> Result<(), TeXMouthError> {
        if self.cur_tok == self.par_token && self.long_state != CommandCode::LongCall as u8 {
            return Err(TeXMouthError::ParagraphInShortArgument);
        }
        Ok(())
    }

    fn expand_after(&mut self) -> Result<(), TeXMouthError> {
        self.get_token()?;
        let saved = self.cur_tok;
        self.get_token()?;
        if self.cur_cmd > MAX_COMMAND {
            self.expand()?;
        } else {
            self.back_input()?;
        }
        self.cur_tok = saved;
        self.back_input()
    }

    fn expand_unless(&mut self) -> Result<bool, TeXMouthError> {
        self.get_token()?;
        if self.cur_cmd == CommandCode::IfTest as u8 && self.cur_chr != IF_CASE_CODE {
            self.cur_chr += UNLESS_CODE;
            return Ok(true);
        }
        // C §1553: recoverable `back_error` — report, push the token back,
        // and continue.
        let name = CommandCode::from_u8(self.cur_cmd)
            .and_then(|cmd| crate::commands::primitive_name_for(cmd, self.cur_chr))
            .unwrap_or("a non-conditional token");
        self.report_error(
            &format!("You can't use `\\unless' before `{name}'"),
            &["Continue, and I'll forget that it ever happened."],
        );
        self.back_input()?;
        Ok(false)
    }

    fn expand_noexpand(&mut self) -> Result<(), TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        self.scanner_status = NORMAL;
        let token_result = self.get_token();
        self.scanner_status = save_scanner_status;
        token_result?;

        let token = self.cur_tok;
        self.back_input()?;
        if token >= CS_TOKEN_FLAG {
            let p = self.get_avail()?;
            self.mem[p as usize].s0 = CS_TOKEN_FLAG + crate::scanner::FROZEN_DONT_EXPAND_ID;
            self.mem[p as usize].s1 = self.cur_input.loc;
            self.cur_input.loc = p;
            self.cur_input.start = p;
        }
        Ok(())
    }

    /// Implements `\primitive` (C `NO_EXPAND` chr 1). Returns `true` when the
    /// looked-up primitive is expandable and must be reswitched immediately.
    fn expand_primitive(&mut self) -> Result<bool, TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        self.scanner_status = NORMAL;
        let token_result = self.get_token();
        self.scanner_status = save_scanner_status;
        token_result?;

        let Some((command, character)) = self.prim_lookup_cs(self.cur_cs) else {
            return Ok(false);
        };

        if command > MAX_COMMAND {
            self.cur_cmd = command;
            self.cur_chr = character;
            self.cur_tok = command as i32 * MAX_CHAR_VAL + character;
            self.cur_cs = 0;
            return Ok(true);
        }

        self.back_input()?;
        let p = self.get_avail()?;
        self.mem[p as usize].s0 = CS_TOKEN_FLAG + crate::scanner::FROZEN_PRIMITIVE_ID;
        self.mem[p as usize].s1 = self.cur_input.loc;
        self.cur_input.loc = p;
        self.cur_input.start = p;
        Ok(false)
    }

    fn expand_cs_name(&mut self) -> Result<(), TeXMouthError> {
        let name = self.scan_cs_name_tokens()?;
        let cs = if name.is_empty() {
            crate::scanner::NULL_CS_ID
        } else {
            self.lookup_cs_units(&name, true)
        };

        if self.cs_command(cs) == CommandCode::UndefinedCs as u8 {
            self.eq_define_cs(cs, CommandCode::Relax, TOO_BIG_USV)?;
        }
        self.cur_tok = CS_TOKEN_FLAG + cs;
        self.back_input()
    }

    fn scan_cs_name_tokens(&mut self) -> Result<Vec<i32>, TeXMouthError> {
        let was_in_csname = self.is_in_csname;
        self.is_in_csname = true;
        let mut name = Vec::new();

        let scan_result = (|| loop {
            self.get_x_token()?;
            if self.cur_cs == 0 {
                name.push(self.cur_tok % MAX_CHAR_VAL);
            } else if self.cur_cmd != CommandCode::EndCsName as u8 {
                self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
                self.back_input()?;
                return Err(TeXMouthError::MissingEndcsname);
            } else {
                return Ok(());
            }
        })();

        self.is_in_csname = was_in_csname;
        scan_result?;
        Ok(name)
    }

    fn conditional(&mut self) -> Result<(), TeXMouthError> {
        // C `conditional` entry: `\tracingifs` reports the `\if` when
        // `\tracingcommands` hasn't already.
        if self.int_par(crate::state::INT_PAR_TRACING_IFS)? > 0
            && self.int_par(crate::state::INT_PAR_TRACING_COMMANDS)? <= 1
        {
            self.show_cur_cmd_chr()?;
        }
        self.push_if_node()?;
        let save_cond_ptr = self.cond_ptr;
        let is_unless = self.cur_chr >= UNLESS_CODE;
        let this_if = self.cur_chr % UNLESS_CODE;
        let mut condition = match this_if {
            0 | 1 => self.evaluate_if_char_or_cat(this_if)?,
            2 | 3 => self.evaluate_if_num_or_dim(this_if)?,
            4 => self.scan_int()? % 2 != 0,
            5 => self.mode.abs() == TeXMode::Vmode as i16,
            6 => self.mode.abs() == TeXMode::Hmode as i16,
            7 => self.mode.abs() == TeXMode::Mmode as i16,
            8 => self.mode < 0,
            9..=11 => self.evaluate_if_box(this_if)?,
            12 => self.evaluate_ifx()?,
            13 => {
                self.scan_four_bit_int_or_18()?;
                self.read_stream_eof(self.cur_val)
            }
            14 => true,
            15 => false,
            16 => return self.evaluate_if_case(save_cond_ptr),
            17 => {
                let save_scanner_status = self.scanner_status;
                self.scanner_status = NORMAL;
                let read_result = self.get_next();
                self.scanner_status = save_scanner_status;
                read_result?;
                self.cur_cmd != CommandCode::UndefinedCs as u8
            }
            18 => {
                let name = self.scan_cs_name_tokens()?;
                let cs = if name.is_empty() {
                    crate::scanner::NULL_CS_ID
                } else {
                    self.lookup_cs_units(&name, false)
                };
                self.cs_command(cs) != CommandCode::UndefinedCs as u8
            }
            19 => {
                // C ~10497 `IF_FONT_CHAR_CODE`: native fonts test
                // `map_char_to_glyph`, TFM fonts test bc..=ec plus
                // `char_exists` on the effective character.
                self.scan_font_ident()?;
                let n = self.cur_val;
                self.scan_usv_num()?;
                let c = self.cur_val;
                if self.is_native_font(n) {
                    self.map_char_to_glyph(n, c.max(0) as u32) > 0
                } else {
                    let in_range = self
                        .font_record(n)
                        .is_some_and(|r| c >= r.bc as i32 && c <= r.ec as i32);
                    in_range && {
                        let eff = self.effective_char(n, c);
                        self.char_exists(n, eff.max(0) as u32)
                    }
                }
            }
            20 => self.is_in_csname,
            21 => {
                let save_scanner_status = self.scanner_status;
                self.scanner_status = NORMAL;
                let read_result = self.get_next();
                self.scanner_status = save_scanner_status;
                read_result?;
                self.cur_cmd != CommandCode::UndefinedCs as u8
                    && self.prim_lookup_cs(self.cur_cs) == Some((self.cur_cmd, self.cur_chr))
            }
            _ => return Err(TeXMouthError::UnsupportedPrimitive("\\if")),
        };

        if is_unless {
            condition = !condition;
        }

        // C emits `{true}`/`{false}` when `\tracingcommands > 1`.
        self.trace_conditional_result(condition)?;

        if condition {
            self.change_if_limit(ELSE_CODE as u8, save_cond_ptr)?;
            return Ok(());
        }

        // Skip to the `\else` or `\fi` matching this conditional (C "Skip to
        // \else or \fi, then goto common_ending"). A delimiter found while
        // `cond_ptr != save_cond_ptr` belongs to a conditional expanded
        // inside this one's operands (e.g. `\if\iftrue abc\else d\fi`), so
        // only `\fi` — which pops that inner node — ends a pass there.
        loop {
            self.pass_text()?;
            if self.cond_ptr == save_cond_ptr {
                if self.cur_chr != OR_CODE {
                    return self.conditional_common_ending();
                }
                // C reports "Extra \or" and ignores it; keep skipping.
            } else if self.cur_chr == FI_CODE {
                self.pop_if_node()?;
            }
        }
    }

    /// Shared conditional epilogue (C `common_ending`): a `\fi` delimiter
    /// pops the condition stack; any other delimiter leaves `if_limit`
    /// waiting for the matching `\fi`.
    fn conditional_common_ending(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_chr == FI_CODE {
            self.pop_if_node()
        } else {
            self.if_limit = FI_CODE as u8;
            Ok(())
        }
    }

    /// Sets `if_limit` for the conditional whose node is `p`
    /// (C `change_if_limit`).
    ///
    /// When `p` is no longer on top of the condition stack — possible when a
    /// nested conditional was expanded while evaluating this one's operands —
    /// the limit is stored into the enclosing node that links to `p`.
    fn change_if_limit(&mut self, l: u8, p: i32) -> Result<(), TeXMouthError> {
        if p == self.cond_ptr {
            self.if_limit = l;
            return Ok(());
        }

        let mut q = self.cond_ptr;
        loop {
            if q == TEX_NULL {
                // C `confusion("if")`.
                return Err(TeXMouthError::MalformedMacroParameters);
            }
            let idx = self.checked_engine_mem_index(q)?;
            if self.mem[idx].s1 == p {
                self.mem[idx].set_b16_s1(u16::from(l));
                return Ok(());
            }
            q = self.mem[idx].s1;
        }
    }

    /// `get_x_token_or_active_char` (C ~10250): after `\noexpand`, a
    /// deferred control sequence reappears as `Relax` + `NO_EXPAND_FLAG`;
    /// decode it back into an active character so `\if` and `\ifcat`
    /// compare it correctly. Non-active deferred sequences decode to
    /// `TOO_BIG_USV`, matching C's `cur_tok - cs_token_flag - active_base`
    /// overflowing `BIGGEST_USV` for non-active control sequences.
    fn get_x_token_or_active_char(&mut self) -> Result<(), TeXMouthError> {
        self.get_x_token()?;
        if self.cur_cmd == CommandCode::Relax as u8 && self.cur_chr == NO_EXPAND_FLAG {
            self.cur_cmd = CAT_ACTIVE_CHAR;
            let cs = self.cur_tok - CS_TOKEN_FLAG;
            let name = self.cs_table.name(cs);
            self.cur_chr = if name.len() == 1 && self.cs_table.lookup_active(name[0]) == Some(cs) {
                name[0]
            } else {
                TOO_BIG_USV
            };
        }
        Ok(())
    }

    fn evaluate_if_char_or_cat(&mut self, this_if: i32) -> Result<bool, TeXMouthError> {
        self.get_x_token_or_active_char()?;
        let (m, n) = if self.cur_cmd > CAT_ACTIVE_CHAR || self.cur_chr > BIGGEST_USV {
            (CommandCode::Relax as u8, TOO_BIG_USV)
        } else {
            (self.cur_cmd, self.cur_chr)
        };

        self.get_x_token_or_active_char()?;
        if self.cur_cmd > CAT_ACTIVE_CHAR || self.cur_chr > BIGGEST_USV {
            self.cur_cmd = CommandCode::Relax as u8;
            self.cur_chr = TOO_BIG_USV;
        }

        Ok(if this_if == 0 {
            n == self.cur_chr
        } else {
            m == self.cur_cmd
        })
    }

    fn evaluate_if_num_or_dim(&mut self, this_if: i32) -> Result<bool, TeXMouthError> {
        let left = if this_if == 2 {
            self.scan_int()?
        } else {
            self.scan_dimen(false, false, false)?
        };
        loop {
            self.get_x_token()?;
            if self.cur_tok != SPACE_TOKEN {
                break;
            }
        }

        let relation_token = self.cur_tok - 12 * MAX_CHAR_VAL;
        let relation = if (60..=62).contains(&relation_token) {
            relation_token
        } else {
            // C reports "Missing = inserted for \ifnum/\ifdim" and recovers
            // by pushing the offending token back and treating the relation
            // as `=`; the same post-error recovery is applied silently here.
            self.back_input()?;
            i32::from(b'=')
        };

        let right = if this_if == 2 {
            self.scan_int()?
        } else {
            self.scan_dimen(false, false, false)?
        };
        Ok(match relation {
            60 => left < right,
            61 => left == right,
            _ => left > right,
        })
    }

    fn evaluate_if_box(&mut self, this_if: i32) -> Result<bool, TeXMouthError> {
        let register = self.scan_int()?;
        if !(0..256).contains(&register) {
            return Err(TeXMouthError::InputOutOfBounds(register));
        }
        let node = self.box_reg(register as usize)?;
        Ok(match this_if {
            9 => node == TEX_NULL,
            _ if node == TEX_NULL => false,
            10 => self.mem[node as usize].node_type() == crate::state::HLIST_NODE,
            _ => self.mem[node as usize].node_type() == crate::state::VLIST_NODE,
        })
    }

    fn evaluate_ifx(&mut self) -> Result<bool, TeXMouthError> {
        // C resets `scanner_status` to `NORMAL` because `\outer` control
        // sequences are legitimate operands even while scanning a macro
        // definition or alignment preamble.
        let save_scanner_status = self.scanner_status;
        self.scanner_status = NORMAL;
        let result = self.evaluate_ifx_tokens();
        self.scanner_status = save_scanner_status;
        result
    }

    fn evaluate_ifx_tokens(&mut self) -> Result<bool, TeXMouthError> {
        self.get_next()?;
        let first_cs = self.cur_cs;
        let first_cmd = self.cur_cmd;
        let first_chr = self.cur_chr;
        self.get_next()?;

        if self.cur_cmd != first_cmd {
            return Ok(false);
        }
        if self.cur_cmd < CommandCode::Call as u8 {
            return Ok(self.cur_chr == first_chr);
        }

        // Compare macro replacement texts, omitting the reference-count
        // node at the head of each definition (C ~527).
        let first_equiv = self.checked_engine_mem_index(self.cs_character(first_cs))?;
        let second_equiv = self.checked_engine_mem_index(self.cur_chr)?;
        let mut p = self.mem[first_equiv].s1;
        let mut q = self.mem[second_equiv].s1;
        while p != TEX_NULL && q != TEX_NULL {
            let p_idx = self.checked_engine_mem_index(p)?;
            let q_idx = self.checked_engine_mem_index(q)?;
            if self.mem[p_idx].s0 != self.mem[q_idx].s0 {
                return Ok(false);
            }
            p = self.mem[p_idx].s1;
            q = self.mem[q_idx].s1;
        }
        Ok(p == TEX_NULL && q == TEX_NULL)
    }

    fn evaluate_if_case(&mut self, save_cond_ptr: i32) -> Result<(), TeXMouthError> {
        let mut n = self.scan_int()?;
        // C loops while `n != 0`, so an out-of-range or negative case number
        // consumes `\or` delimiters until the `\else` or `\fi` at this
        // conditional's level sends control to `common_ending`.
        while n != 0 {
            self.pass_text()?;
            if self.cond_ptr == save_cond_ptr {
                if self.cur_chr == OR_CODE {
                    n -= 1;
                } else {
                    return self.conditional_common_ending();
                }
            } else if self.cur_chr == FI_CODE {
                self.pop_if_node()?;
            }
        }
        self.change_if_limit(OR_CODE as u8, save_cond_ptr)
    }

    fn pass_text(&mut self) -> Result<(), TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        self.scanner_status = crate::scanner::SKIPPING;
        let mut level = 0i32;
        self.skip_line = self.line;

        let result = loop {
            if let Err(err) = self.get_next() {
                break Err(err);
            }
            if self.cur_cmd == CommandCode::Relax as u8
                && self.cur_chr == 0
                && self.cur_cs == 0
                && self.cur_input.state != TOKEN_LIST
                && self.cur_input.loc > self.cur_input.limit
            {
                break Err(TeXMouthError::EndOfInput);
            }
            if self.cur_cmd == CommandCode::FiOrElse as u8 {
                if level == 0 {
                    break Ok(());
                }
                if self.cur_chr == FI_CODE {
                    level -= 1;
                }
            } else if self.cur_cmd == CommandCode::IfTest as u8 {
                level += 1;
            }
        };

        self.scanner_status = save_scanner_status;
        // C `pass_text`'s `done:` label traces the terminating delimiter when
        // `\tracingifs` is enabled.
        if result.is_ok() && self.int_par(crate::state::INT_PAR_TRACING_IFS)? > 0 {
            self.show_cur_cmd_chr()?;
        }
        result
    }

    fn expand_fi_or_else(&mut self) -> Result<(), TeXMouthError> {
        // C's `FI_OR_ELSE` expander arm: `\tracingifs` reports the delimiter
        // when `\tracingcommands` hasn't already.
        if self.int_par(crate::state::INT_PAR_TRACING_IFS)? > 0
            && self.int_par(crate::state::INT_PAR_TRACING_COMMANDS)? <= 1
        {
            self.show_cur_cmd_chr()?;
        }
        if self.cur_chr > i32::from(self.if_limit) {
            if self.if_limit == IF_CODE {
                self.insert_relax()?;
            } else {
                return Err(TeXMouthError::MacroArgumentMismatch {
                    expected: FI_CODE,
                    found: self.cur_chr,
                });
            }
            return Ok(());
        }

        while self.cur_chr != FI_CODE {
            self.pass_text()?;
        }
        self.pop_if_node()
    }

    fn push_if_node(&mut self) -> Result<(), TeXMouthError> {
        let p = self.get_node(IF_NODE_SIZE)?;
        let p_index = self.checked_engine_mem_index(p)?;
        self.mem[p_index].s1 = self.cond_ptr;
        self.mem[p_index].set_b16_s1(u16::from(self.if_limit));
        self.mem[p_index].set_b16_s0(self.cur_if as u16);
        self.mem[p_index + 1].s1 = self.if_line;
        self.cond_ptr = p;
        self.cur_if = self.cur_chr as i16;
        self.if_limit = IF_CODE;
        self.if_line = self.line;
        Ok(())
    }

    fn pop_if_node(&mut self) -> Result<(), TeXMouthError> {
        if self.cond_ptr == TEX_NULL {
            return Err(TeXMouthError::MalformedMacroParameters);
        }
        // C checks `if_stack[in_open] == cond_ptr` at every conditional pop
        // (`common_ending`, the expander's `FI_OR_ELSE` arm, and the
        // conditional's own `\fi` handling) and warns about a conditional
        // that began in a different file.
        if self.if_stack.get(self.in_open).copied() == Some(self.cond_ptr) {
            self.if_warning()?;
        }
        let p = self.cond_ptr;
        let p_index = self.checked_engine_mem_index(p)?;
        self.if_line = self.mem[p_index + 1].s1;
        self.cur_if = self.mem[p_index].b16_s0() as i16;
        self.if_limit = self.mem[p_index].b16_s1() as u8;
        self.cond_ptr = self.mem[p_index].s1;
        self.free_node(p, IF_NODE_SIZE)?;
        Ok(())
    }

    /// C `insert_relax`: when a `\fi` (or `\else`/`\or`) is seen before its
    /// condition was evaluated, the triggering token is preserved and a
    /// frozen `\relax` is inserted in front of it so the conditional's
    /// pending scans read a harmless token and the delimiter can be
    /// re-processed afterwards.
    fn insert_relax(&mut self) -> Result<(), TeXMouthError> {
        self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        self.back_input()?;
        self.cur_tok = CS_TOKEN_FLAG + FROZEN_RELAX_ID;
        self.back_input()?;
        self.cur_input.index = INSERTED;
        Ok(())
    }

    fn expand_convert(&mut self) -> Result<(), TeXMouthError> {
        match self.cur_chr {
            0 => {
                let value = self.scan_int()?;
                self.insert_character_tokens(value.to_string().chars())
            }
            1 => {
                let value = self.scan_int()?;
                self.insert_character_tokens(to_roman_numeral(value).chars())
            }
            2 => self.expand_string(),
            3 => {
                self.get_token()?;
                let meaning = self.meaning_of_current()?;
                self.insert_character_tokens(meaning.chars())
            }
            4 => self.expand_font_name(),
            5 => self.insert_character_tokens(".6".chars()),
            6 => self.expand_expanded(),
            16 | 17 => self.expand_margin_kern(self.cur_chr == 17),
            18 => self.expand_strcmp(),
            22 => {
                let text = self.creation_date_text();
                self.insert_character_tokens(text.chars())
            }
            23..=26 => self.expand_file_convert(self.cur_chr),
            29 => {
                let bound = self.scan_int()?;
                let value = self.rand.unif_rand(bound);
                self.insert_character_tokens(value.to_string().chars())
            }
            30 => {
                let value = self.rand.norm_rand();
                self.insert_character_tokens(value.to_string().chars())
            }
            33 => self.insert_character_tokens(".999994".chars()),
            34..=36 => self.expand_font_engine_name(self.cur_chr),
            37 => self.expand_glyph_name(),
            38 => self.expand_uchar(CommandCode::OtherChar as u8),
            39 => self.expand_ucharcat(),
            40 => self.insert_character_tokens("FerroTeX".chars()),
            _ => Err(TeXMouthError::UnsupportedPrimitive("conversion primitive")),
        }
    }

    /// Expands `\fontname` (C `conv_toks` `FONT_NAME_CODE` ~9723): the font
    /// name is quoted when the font is native, and ` at <size>pt` is
    /// appended when the loaded size differs from the design size.
    fn expand_font_name(&mut self) -> Result<(), TeXMouthError> {
        self.scan_font_ident()?;
        let f = self.cur_val;
        let Some(record) = self.font_record(f) else {
            return Ok(());
        };
        let name: String = record
            .name
            .iter()
            .filter_map(|&unit| char::from_u32(unit as u32))
            .collect();
        let mut text = String::new();
        if self.is_native_font(f) {
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
        }
        self.insert_character_tokens(text.chars())
    }

    /// Expands `\XeTeXvariationname`, `\XeTeXfeaturename`, and
    /// `\XeTeXselectorname` (C `conv_toks` ~9630-9675). Non-AAT/Graphite
    /// fonts record the C `not_*_font_error` diagnostic and emit nothing.
    fn expand_font_engine_name(&mut self, code: i32) -> Result<(), TeXMouthError> {
        const XETEX_VARIATION_NAME_CODE: i32 = 34;
        const XETEX_SELECTOR_NAME_CODE: i32 = 36;
        self.scan_font_ident()?;
        let f = self.cur_val;
        let aat_only = code == XETEX_VARIATION_NAME_CODE;
        if self.is_aat_font(f) || (!aat_only && self.using_graphite(f)) {
            self.scan_int()?;
            let arg1 = self.cur_val;
            let arg2 = if code == XETEX_SELECTOR_NAME_CODE {
                self.scan_int()?;
                self.cur_val
            } else {
                0
            };
            match self
                .font_engine_name(f, code, arg1, arg2)
                .map(str::to_string)
            {
                Some(name) => self.insert_character_tokens(name.chars()),
                None => Ok(()),
            }
        } else {
            self.convert_font_error(
                code,
                f,
                if aat_only {
                    "not an AAT font"
                } else {
                    "not an AAT or Graphite font"
                },
            );
            Ok(())
        }
    }

    /// Expands `\XeTeXglyphname` (C `conv_toks` `XETEX_GLYPH_NAME_CODE`
    /// ~9676): a native font scans a glyph id and prints its name; other
    /// fonts record the `not_native_font_error` diagnostic.
    fn expand_glyph_name(&mut self) -> Result<(), TeXMouthError> {
        self.scan_font_ident()?;
        let f = self.cur_val;
        if self.is_native_font(f) {
            self.scan_int()?;
            let gid = self.cur_val.max(0) as u32;
            let name = self.glyph_name(f, gid).unwrap_or_default().to_string();
            self.insert_character_tokens(name.chars())
        } else {
            self.convert_font_error(37, f, "not a native platform font");
            Ok(())
        }
    }

    /// Records a C `not_*_font_error` diagnostic for a `CONVERT` font query.
    fn convert_font_error(&mut self, code: i32, font: i32, suffix: &str) {
        let primitive =
            crate::commands::primitive_name_for(CommandCode::Convert, code).unwrap_or("convert");
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

    fn expand_expanded(&mut self) -> Result<(), TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        let save_warning_index = self.warning_index;
        let save_def_ref = self.def_ref;
        let scan_result = self.scan_toks(false, true);
        let reference = self.def_ref;
        self.scanner_status = save_scanner_status;
        self.warning_index = save_warning_index;
        self.def_ref = save_def_ref;
        scan_result?;

        let reference_index = self.checked_engine_mem_index(reference)?;
        let tokens = self.mem[reference_index].s1;
        self.free_avail(reference)?;
        self.begin_token_list(tokens, INSERTED)
    }

    fn pseudo_start(&mut self) -> Result<(), TeXMouthError> {
        let reference = self.scan_general_text_reference()?;
        let text = self.token_list_display_string(reference)?;
        self.delete_token_ref(reference)?;

        let new_line = self
            .int_par(crate::state::INT_PAR_NEW_LINE_CHAR)
            .unwrap_or(-1);
        let mut units: Vec<i32> = text.encode_utf16().map(i32::from).collect();
        units.push(i32::from(b' '));

        let mut lines = VecDeque::new();
        let mut start = 0usize;
        for (index, &unit) in units.iter().enumerate() {
            if unit == new_line {
                let mut line = units[start..index].to_vec();
                while line.last() == Some(&i32::from(b' ')) {
                    line.pop();
                }
                line.push(i32::from(b'\r'));
                lines.push_back(line);
                start = index + 1;
            }
        }
        let mut line = units[start..].to_vec();
        while line.last() == Some(&i32::from(b' ')) {
            line.pop();
        }
        line.push(i32::from(b'\r'));
        lines.push_back(line);

        self.open_pseudo_source(lines)
    }

    fn expand_strcmp(&mut self) -> Result<(), TeXMouthError> {
        let save_cur_cs = self.cur_cs;
        self.scan_toks(false, true)?;
        let first = self.def_ref;
        let first_string = self.token_list_display_string(first)?;
        self.delete_token_ref(first)?;
        self.cur_cs = save_cur_cs;

        self.scan_toks(false, true)?;
        let second = self.def_ref;
        let second_string = self.token_list_display_string(second)?;
        self.delete_token_ref(second)?;
        self.cur_cs = save_cur_cs;

        let value = match first_string.cmp(&second_string) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
        self.insert_character_tokens(value.to_string().chars())
    }

    /// Expands `\filemoddate`, `\filesize`, `\mdfivesum`, and `\filedump`
    /// (C `conv_toks` `PDF_FILE_*` codes ~9406-9578): the keyword options
    /// are scanned first, then the file name is materialized from
    /// `scan_pdf_ext_toks` + `tokens_to_string` and the texmfmp utility
    /// produces the inserted text. Negative offsets/lengths record C's
    /// error and clamp to zero.
    fn expand_file_convert(&mut self, code: i32) -> Result<(), TeXMouthError> {
        const PDF_FILE_MOD_DATE_CODE: i32 = 23;
        const PDF_FILE_SIZE_CODE: i32 = 24;
        const PDF_MDFIVE_SUM_CODE: i32 = 25;
        const PDF_FILE_DUMP_CODE: i32 = 26;

        let mut offset = 0;
        let mut length = 0;
        if code == PDF_FILE_DUMP_CODE {
            if self.scan_keyword("offset")? {
                self.scan_int()?;
                if self.cur_val < 0 {
                    self.report_error(
                        &format!("Bad file offset ({})", self.cur_val),
                        &[
                            "A file offset must be between 0 and 2^_31_-1,",
                            "I changed this one to zero.",
                        ],
                    );
                    self.cur_val = 0;
                }
            }
            offset = self.cur_val;
            self.cur_val = 0;
            if self.scan_keyword("length")? {
                self.scan_int()?;
                if self.cur_val < 0 {
                    self.report_error(
                        &format!("Bad dump length ({})", self.cur_val),
                        &[
                            "A dump length must be between 0 and 2^_31_-1,",
                            "I changed this one to zero.",
                        ],
                    );
                    self.cur_val = 0;
                }
            }
            length = self.cur_val;
        }

        let is_file = code == PDF_MDFIVE_SUM_CODE && self.scan_keyword("file")?;

        // `scan_pdf_ext_toks` + `tokens_to_string(def_ref)`: the scanned
        // token list is rendered to the file-name text (C's `gettexstring`
        // yields UTF-8; `token_list_display_string` yields the same text).
        self.scan_toks(false, true)?;
        let name_ref = self.def_ref;
        let name = self.token_list_display_string(name_ref)?;
        self.delete_token_ref(name_ref)?;

        let text = match code {
            PDF_FILE_MOD_DATE_CODE => self.file_mod_date_text(&name),
            PDF_FILE_SIZE_CODE => self.file_size_text(&name),
            PDF_MDFIVE_SUM_CODE => self.md5_sum_text(&name, is_file),
            _ => self.file_dump_text(&name, offset, length),
        };
        self.insert_character_tokens(text.chars())
    }

    fn token_list_display_string(&self, reference: i32) -> Result<String, TeXMouthError> {
        let mut output = String::new();
        let mut p = self.mem[self.checked_engine_mem_index(reference)?].s1;
        while p != TEX_NULL {
            let index = self.checked_engine_mem_index(p)?;
            let token = self.mem[index].s0;
            if token >= CS_TOKEN_FLAG {
                let cs = token - CS_TOKEN_FLAG;
                let name = self.cs_table.name(cs);
                if cs == NULL_CS_ID {
                    output.push_str("\\csname\\endcsname ");
                } else if name.len() == 1 && self.cs_table.lookup_active(name[0]) == Some(cs) {
                    if let Some(ch) = char::from_u32(name[0].max(0) as u32) {
                        output.push(ch);
                        if self.cat_code(name[0].max(0) as u32) == crate::scanner::CAT_LETTER {
                            output.push(' ');
                        }
                    }
                } else {
                    output.push('\\');
                    for &unit in name {
                        if let Some(ch) = char::from_u32(unit.max(0) as u32) {
                            output.push(ch);
                        }
                    }
                    output.push(' ');
                }
            } else {
                let command = token / MAX_CHAR_VAL;
                let character = token % MAX_CHAR_VAL;
                match command {
                    x if x == CommandCode::LeftBrace as i32 => output.push('{'),
                    x if x == CommandCode::RightBrace as i32 => output.push('}'),
                    x if x == CommandCode::MacParam as i32 => output.push('#'),
                    _ => {
                        if let Some(ch) = char::from_u32(character.max(0) as u32) {
                            output.push(ch);
                        }
                    }
                }
            }
            p = self.mem[index].s1;
        }
        Ok(output)
    }

    fn expand_margin_kern(&mut self, right: bool) -> Result<(), TeXMouthError> {
        self.scan_register_num()?;
        let box_node = if self.cur_val > 255 {
            let element = self.find_sa_element(SA_BOX_VAL, self.cur_val, false)?;
            if element == TEX_NULL {
                TEX_NULL
            } else {
                self.sparse_ptr_value(element)?
            }
        } else {
            self.box_reg(
                usize::try_from(self.cur_val)
                    .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?,
            )?
        };

        if box_node == TEX_NULL
            || self.mem[self.checked_engine_mem_index(box_node)?].b16_s1() != HLIST_NODE
        {
            return Err(TeXMouthError::UnsupportedPrimitive(
                "marginkern of a non-hbox",
            ));
        }

        let list = self.mem[self.checked_engine_mem_index(box_node + 5)?].s1;
        let p = if right {
            let mut p = self.prev_rightmost(list, TEX_NULL)?;
            while p != TEX_NULL && self.margin_kern_ignorable(p, right)? {
                p = self.prev_rightmost(list, p)?;
            }
            p
        } else {
            let mut p = list;
            while p != TEX_NULL && self.margin_kern_ignorable(p, right)? {
                p = self.mem[self.checked_engine_mem_index(p)?].s1;
            }
            p
        };

        let width = if p != TEX_NULL
            && p < self.hi_mem_min
            && self.mem[self.checked_engine_mem_index(p)?].b16_s1() == MARGIN_KERN_NODE
            && self.mem[self.checked_engine_mem_index(p)?].b16_s0() == u16::from(right)
        {
            self.mem[self.checked_engine_mem_index(p + 1)?].s1
        } else {
            0
        };
        self.insert_character_tokens(format_scaled_pt(width).chars())
    }

    fn margin_kern_ignorable(&self, p: i32, right: bool) -> Result<bool, TeXMouthError> {
        if p >= self.hi_mem_min {
            return Ok(false);
        }
        let index = self.checked_engine_mem_index(p)?;
        let node_type = self.mem[index].b16_s1();
        let subtype = self.mem[index].b16_s0();
        let first = self.mem[index + 1].s0;
        let second = self.mem[index + 1].s1;

        Ok(
            matches!(node_type, INS_NODE | MARK_NODE | ADJUST_NODE | PENALTY_NODE)
                || (node_type == DISC_NODE
                    && first == TEX_NULL
                    && second == TEX_NULL
                    && subtype == 0)
                || (node_type == MATH_NODE && second == 0)
                || (node_type == KERN_NODE && (second == 0 || subtype == NODE_NORMAL))
                || (node_type == GLUE_NODE && (first == 0 || subtype == if right { 9 } else { 8 }))
                || (node_type == HLIST_NODE
                    && second == 0
                    && self.mem[index + 2].s1 == 0
                    && self.mem[index + 3].s1 == 0
                    && self.mem[index + 5].s1 == TEX_NULL),
        )
    }

    pub(crate) fn prev_rightmost(&self, start: i32, end: i32) -> Result<i32, TeXMouthError> {
        let mut p = start;
        if p == TEX_NULL {
            return Ok(TEX_NULL);
        }
        while self.mem[self.checked_engine_mem_index(p)?].s1 != end {
            p = self.mem[self.checked_engine_mem_index(p)?].s1;
            if p == TEX_NULL {
                return Ok(TEX_NULL);
            }
        }
        Ok(p)
    }

    fn expand_uchar(&mut self, category: u8) -> Result<(), TeXMouthError> {
        self.scan_usv_num()?;
        let token = i32::from(category) * MAX_CHAR_VAL + self.cur_val;
        self.insert_token_vec(&[token])
    }

    fn expand_ucharcat(&mut self) -> Result<(), TeXMouthError> {
        self.scan_usv_num()?;
        let character = self.cur_val;
        self.scan_int()?;
        let category = if (1..=4).contains(&self.cur_val)
            || (6..=8).contains(&self.cur_val)
            || (10..=13).contains(&self.cur_val)
        {
            self.cur_val
        } else {
            12
        };
        let token = category * MAX_CHAR_VAL + character;
        self.insert_token_vec(&[token])
    }

    fn scan_general_text_reference(&mut self) -> Result<i32, TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        let save_warning_index = self.warning_index;
        let save_def_ref = self.def_ref;
        let result = self.scan_toks(false, false);
        let reference = self.def_ref;
        self.scanner_status = save_scanner_status;
        self.warning_index = save_warning_index;
        self.def_ref = save_def_ref;
        result?;
        Ok(reference)
    }

    fn expand_string(&mut self) -> Result<(), TeXMouthError> {
        let save_scanner_status = self.scanner_status;
        self.scanner_status = NORMAL;
        let token_result = self.get_token();
        self.scanner_status = save_scanner_status;
        token_result?;

        let mut characters = Vec::new();
        if self.cur_cs != 0 {
            let cs = self.cur_cs;
            let name = self.cs_table.name(cs);
            if cs == NULL_CS_ID {
                // C `sprint_cs(NULL_CS)` prints `\csname\endcsname`.
                characters.extend("\\csname\\endcsname".chars());
            } else if name.len() == 1 && self.cs_table.lookup_active(name[0]) == Some(cs) {
                // C `sprint_cs`: an active-character control sequence prints
                // as just its character.
                if let Some(ch) = char::from_u32(name[0].max(0) as u32) {
                    characters.push(ch);
                }
            } else {
                characters.push('\\');
                for &unit in name {
                    if let Some(ch) = char::from_u32(unit.max(0) as u32) {
                        characters.push(ch);
                    }
                }
            }
        } else if let Some(ch) = char::from_u32(self.cur_chr.max(0) as u32) {
            // C `print_char(cur_chr)`; this also covers space tokens, whose
            // `cur_chr` is a space.
            characters.push(ch);
        }
        self.insert_character_tokens(characters)
    }

    fn expand_the(&mut self) -> Result<(), TeXMouthError> {
        let tokens = self.the_token_vec()?;
        self.insert_token_vec(&tokens)
    }

    pub(crate) fn the_token_vec(&mut self) -> Result<Vec<i32>, TeXMouthError> {
        if self.cur_chr & 1 != 0 {
            let general_text_kind = self.cur_chr;
            let reference = self.scan_general_text_reference()?;
            let start = self.mem[self.checked_engine_mem_index(reference)?].s1;
            let tokens = self.token_list_to_vec(start)?;
            if general_text_kind == 1 {
                self.delete_token_ref(reference)?;
                return Ok(tokens);
            }
            let text = self.token_list_display_string(reference)?;
            self.delete_token_ref(reference)?;
            return Ok(self.character_tokens(text.chars()));
        }
        // C `the_toks` reads the operand with `get_x_token`, so macros that
        // expand to an internal quantity (for example `\the\foo` where
        // `\foo` expands to `\count0`) are accepted.
        self.get_x_token()?;
        self.scan_something_internal(TOK_VAL, false)?;

        match self.cur_val_level {
            INT_VAL => Ok(self.character_tokens(self.cur_val.to_string().chars())),
            DIMEN_VAL => Ok(self.character_tokens(format_scaled_pt(self.cur_val).chars())),
            TOK_VAL => {
                // Compact token-list values may either point directly at the
                // first token or at a `def_ref`-style reference head.
                let start = self.stored_token_list_start(self.cur_val)?;
                if start == TEX_NULL {
                    Ok(Vec::new())
                } else {
                    self.token_list_to_vec(start)
                }
            }
            _ => Err(TeXMouthError::UnsupportedPrimitive(
                "\\the of a glue-valued quantity",
            )),
        }
    }

    fn character_tokens<I>(&self, characters: I) -> Vec<i32>
    where
        I: IntoIterator<Item = char>,
    {
        characters
            .into_iter()
            .map(|ch| {
                if ch == ' ' {
                    SPACE_TOKEN
                } else {
                    12 * MAX_CHAR_VAL + ch as i32
                }
            })
            .collect()
    }

    fn insert_character_tokens<I>(&mut self, characters: I) -> Result<(), TeXMouthError>
    where
        I: IntoIterator<Item = char>,
    {
        let tokens = self.character_tokens(characters);
        self.insert_token_vec(&tokens)
    }

    fn insert_token_vec(&mut self, tokens: &[i32]) -> Result<(), TeXMouthError> {
        if tokens.is_empty() {
            return Ok(());
        }
        let head = self.store_token_list(tokens)?;
        self.begin_token_list(head, INSERTED)
    }

    fn expand_top_bot_mark(&mut self) -> Result<(), TeXMouthError> {
        if self.cur_chr >= 5 {
            let mark_kind = self.cur_chr % 5;
            self.scan_register_num()?;
            let element = self.find_sa_element(SA_MARK_VAL, self.cur_val, false)?;
            if element == TEX_NULL {
                return Ok(());
            }
            let index = self.checked_engine_mem_index(element + mark_kind / 2 + 1)?;
            let p = if mark_kind & 1 == 0 {
                self.mem[index].s0
            } else {
                self.mem[index].s1
            };
            if p != TEX_NULL {
                self.begin_token_list(p, MARK_TEXT)?;
            }
            return Ok(());
        }
        let p = self.cur_mark[self.cur_chr as usize];
        if p != TEX_NULL {
            self.begin_token_list(p, MARK_TEXT)?;
        }
        Ok(())
    }

    fn set_cur_tok_from_current(&mut self) {
        if self.cur_cs == 0 {
            self.cur_tok = self.cur_cmd as i32 * MAX_CHAR_VAL + self.cur_chr;
        } else {
            self.cur_tok = CS_TOKEN_FLAG + self.cur_cs;
        }
    }

    pub(crate) fn checked_engine_mem_index(&self, p: i32) -> Result<usize, TeXMouthError> {
        if p < 0 || p as usize >= self.mem.len() {
            Err(TeXMouthError::InputOutOfBounds(p))
        } else {
            Ok(p as usize)
        }
    }
}

/// Formats a scaled-point value like C `print_scaled` (`1.0pt`,
/// `1.5pt`, `-2.25pt`): the decimal point is always emitted, the fraction
/// is rounded to at most five digits, and `s` may temporarily go negative
/// inside the digit loop exactly as the C `do/while` allows.
pub(crate) fn format_scaled_pt(value: i32) -> String {
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
    text.push_str("pt");
    text
}

fn to_roman_numeral(mut value: i32) -> String {
    if value <= 0 {
        return String::new();
    }

    const TABLE: [(i32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut result = String::new();
    for (amount, numeral) in TABLE {
        while value >= amount {
            result.push_str(numeral);
            value -= amount;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{CS_TOKEN_FLAG, INSERTED, MATCH_TOKEN, NULL_CS_ID, OUT_PARAM_TOKEN};

    const LETTER_TOKEN: i32 = 11 * MAX_CHAR_VAL;
    const OTHER_TOKEN: i32 = 12 * MAX_CHAR_VAL;
    const MATCH_MARK: i32 = MATCH_TOKEN + b'#' as i32;

    fn state() -> TeXEngineState {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state
    }

    fn token(state: &mut TeXEngineState) -> i32 {
        state.next_expanded_token().unwrap()
    }

    fn char_token(character: u8) -> i32 {
        LETTER_TOKEN + i32::from(character)
    }

    #[test]
    fn expands_a_zero_argument_macro() {
        let mut state = state();
        state
            .define_simple_macro("foo", &[char_token(b'x')])
            .unwrap();
        state.set_source_text("\\foo");

        assert_eq!(token(&mut state), char_token(b'x'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.param_ptr, 0);
        assert_eq!(state.input_ptr, 0);
    }

    #[test]
    fn substitutes_an_undelimited_single_token_argument() {
        let mut state = state();
        state
            .define_macro(
                "dup",
                &[MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1, OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\dup A");

        assert_eq!(token(&mut state), char_token(b'A'));
        assert_eq!(token(&mut state), char_token(b'A'));
        assert_eq!(token(&mut state), crate::scanner::SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.param_ptr, 0);
    }

    #[test]
    fn strips_outer_braces_from_a_single_group_argument() {
        let mut state = state();
        state
            .define_macro(
                "dup",
                &[MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1, OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\dup {AB}");

        assert_eq!(token(&mut state), char_token(b'A'));
        assert_eq!(token(&mut state), char_token(b'B'));
        assert_eq!(token(&mut state), char_token(b'A'));
        assert_eq!(token(&mut state), char_token(b'B'));
    }

    #[test]
    fn empty_braced_argument_produces_an_empty_parameter_list() {
        let mut state = state();
        state
            .define_macro(
                "id",
                &[MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\id {}");

        assert_eq!(token(&mut state), crate::scanner::SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.param_ptr, 0);
    }

    #[test]
    fn matches_a_delimited_argument_without_consuming_delimiter_as_argument() {
        let mut state = state();
        state
            .define_macro(
                "take",
                &[MATCH_MARK, OTHER_TOKEN + i32::from(b','), END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\take ab,");

        assert_eq!(token(&mut state), char_token(b'a'));
        assert_eq!(token(&mut state), char_token(b'b'));
        assert_eq!(token(&mut state), crate::scanner::SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn delimiters_inside_braces_do_not_terminate_the_argument() {
        let mut state = state();
        state
            .define_macro(
                "take",
                &[MATCH_MARK, OTHER_TOKEN + i32::from(b','), END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\take {a,b},");

        assert_eq!(token(&mut state), char_token(b'a'));
        assert_eq!(token(&mut state), OTHER_TOKEN + i32::from(b','));
        assert_eq!(token(&mut state), char_token(b'b'));
        assert_eq!(token(&mut state), crate::scanner::SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn matches_leading_parameter_text_before_an_argument() {
        let mut state = state();
        state
            .define_macro(
                "prefix",
                &[LETTER_TOKEN + i32::from(b'x'), MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\prefix xY");

        assert_eq!(token(&mut state), char_token(b'Y'));
    }

    #[test]
    fn reports_a_parameter_text_mismatch() {
        let mut state = state();
        state
            .define_macro(
                "prefix",
                &[LETTER_TOKEN + i32::from(b'x'), MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\prefix z");

        let err = state.next_expanded_token().unwrap_err();
        assert!(matches!(err, TeXMouthError::MacroArgumentMismatch { .. }));
    }

    #[test]
    fn short_macro_arguments_reject_par_tokens() {
        let mut state = state();
        state
            .define_macro(
                "arg",
                &[MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\arg \\par");

        let err = state.next_expanded_token().unwrap_err();
        assert!(matches!(err, TeXMouthError::ParagraphInShortArgument));
    }

    #[test]
    fn long_macro_arguments_accept_par_tokens() {
        let mut state = state();
        state
            .define_macro(
                "arg",
                &[MATCH_MARK, END_MATCH_TOKEN],
                &[OUT_PARAM_TOKEN + 1],
                CommandCode::LongCall,
            )
            .unwrap();
        state.set_source_text("\\arg \\par");

        assert_eq!(token(&mut state), state.par_token);
    }

    #[test]
    fn csname_creates_and_inserts_a_named_control_sequence() {
        let mut state = state();
        state.set_source_text("\\csname foo\\endcsname");

        let observed = token(&mut state);
        let expected = CS_TOKEN_FLAG + state.cs_table.lookup_str("foo").unwrap();
        assert_eq!(observed, expected);
        assert_eq!(state.cur_cmd, CommandCode::Relax as u8);
    }

    #[test]
    fn csname_without_a_name_yields_null_control_sequence() {
        let mut state = state();
        state.set_source_text("\\csname\\endcsname");

        assert_eq!(token(&mut state), CS_TOKEN_FLAG + NULL_CS_ID);
    }

    #[test]
    fn missing_endcsname_is_a_structured_error() {
        let mut state = state();
        state.define_cs("oops", CommandCode::Relax, 0);
        state.set_source_text("\\csname a\\oops");

        let err = state.next_expanded_token().unwrap_err();
        assert!(matches!(err, TeXMouthError::MissingEndcsname));
    }

    #[test]
    fn noexpand_defers_a_macro_control_sequence_once() {
        let mut state = state();
        state
            .define_simple_macro("foo", &[char_token(b'x')])
            .unwrap();
        let foo = state.cs_table.lookup_str("foo").unwrap();
        state.set_source_text("\\noexpand\\foo");

        assert_eq!(token(&mut state), CS_TOKEN_FLAG + foo);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn expandafter_expands_the_second_token_before_reinserting_the_first() {
        let mut state = state();
        state.define_simple_macro("a", &[char_token(b'x')]).unwrap();
        state.define_cs("b", CommandCode::Relax, 0);
        let b = state.cs_table.lookup_str("b").unwrap();
        state.set_source_text("\\expandafter\\b\\a");

        assert_eq!(token(&mut state), CS_TOKEN_FLAG + b);
        assert_eq!(token(&mut state), char_token(b'x'));
    }

    #[test]
    fn topmark_inserts_the_current_mark_token_list() {
        let mut state = state();
        let mark_tokens = state.store_token_list(&[char_token(b'm')]).unwrap();
        let mark = state.get_avail().unwrap();
        state.mem[mark as usize].s0 = TEX_NULL;
        state.mem[mark as usize].s1 = mark_tokens;
        state.cur_mark[0] = mark;
        state.set_source_text("\\topmark");

        assert_eq!(token(&mut state), char_token(b'm'));
    }

    #[test]
    fn endinput_sets_the_forced_eof_flag() {
        let mut state = state();
        state.set_source_text("\\endinput");

        assert_eq!(token(&mut state), 0);
        assert!(state.force_eof);
    }

    #[test]
    fn undefined_control_sequences_report_an_error() {
        let mut state = state();
        state.set_source_text("\\missing");

        let err = state.next_expanded_token().unwrap_err();
        assert!(matches!(err, TeXMouthError::UndefinedControlSequence(_)));
    }

    #[test]
    fn expansion_restores_the_backup_head_chain() {
        let mut state = state();
        let backup_head = state.backup_head();
        assert_eq!(backup_head, state.mem.len() as i32 - 14);
        let saved = state.store_token_list(&[char_token(b'z')]).unwrap();
        state.mem[backup_head as usize].s1 = saved;

        state.define_simple_macro("a", &[char_token(b'x')]).unwrap();
        state.set_source_text("\\a");

        assert_eq!(token(&mut state), char_token(b'x'));
        assert_eq!(state.mem[backup_head as usize].s1, saved);
        state.flush_list(saved).unwrap();
    }

    #[test]
    fn expansion_depth_limit_is_enforced() {
        let mut state = state();
        state.expand_depth = 0;
        state.define_simple_macro("a", &[]).unwrap();
        let a = state.cs_table.lookup_str("a").unwrap();
        state.cur_cmd = CommandCode::Call as u8;
        state.cur_chr = state.cs_character(a);
        state.cur_cs = a;

        let err = state.expand().unwrap_err();
        assert!(matches!(err, TeXMouthError::ExpansionDepthExceeded));
    }

    #[test]
    fn primitive_marks_a_nonexpandable_primitive_for_rescanning() {
        let mut state = state();
        let relax = state.cs_table.lookup_str("relax").unwrap();
        state.set_source_text("\\primitive\\relax");

        assert_eq!(
            token(&mut state),
            CS_TOKEN_FLAG + crate::scanner::FROZEN_PRIMITIVE_ID
        );
        assert_eq!(token(&mut state), CS_TOKEN_FLAG + relax);
    }

    #[test]
    fn primitive_reswitches_an_expandable_primitive() {
        let mut state = state();
        state.set_source_text("\\primitive\\number 42");

        assert_eq!(token(&mut state), OTHER_TOKEN + i32::from(b'4'));
        assert_eq!(token(&mut state), OTHER_TOKEN + i32::from(b'2'));
    }

    #[test]
    fn protected_token_is_skipped_in_parameter_text() {
        let mut state = state();
        state
            .define_macro(
                "protected",
                &[PROTECTED_TOKEN, END_MATCH_TOKEN],
                &[char_token(b'p')],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\protected");

        assert_eq!(token(&mut state), char_token(b'p'));
    }

    #[test]
    fn token_list_parser_reads_inserted_lists_and_parameter_lists() {
        let mut state = state();
        let param = state.store_token_list(&[char_token(b'q')]).unwrap();
        let replacement = state.store_token_list(&[OUT_PARAM_TOKEN + 1]).unwrap();
        state.param_stack[0] = param;
        state.param_ptr = 1;
        state.begin_token_list(replacement, INSERTED).unwrap();
        state.cur_input.limit = 0;

        assert_eq!(token(&mut state), char_token(b'q'));
        assert_eq!(token(&mut state), 0);
    }
}

#[cfg(test)]
mod conditional_and_conversion_tests {
    use super::*;
    use crate::scanner::{
        CAT_ACTIVE_CHAR, CS_TOKEN_FLAG, FROZEN_RELAX_ID, MATCH_TOKEN, OUT_PARAM_TOKEN,
    };

    const LETTER_TOKEN: i32 = 11 * MAX_CHAR_VAL;
    const OTHER_TOKEN: i32 = 12 * MAX_CHAR_VAL;
    const MATCH_MARK: i32 = MATCH_TOKEN + b'#' as i32;

    fn state() -> TeXEngineState {
        let mut state = TeXEngineState::new(4_096);
        state.install_core_primitives();
        state
    }

    fn token(state: &mut TeXEngineState) -> i32 {
        state.next_expanded_token().unwrap()
    }

    fn letter(character: u8) -> i32 {
        LETTER_TOKEN + i32::from(character)
    }

    fn other(character: u8) -> i32 {
        OTHER_TOKEN + i32::from(character)
    }

    /// Token produced by `str_toks`: spaces become `SPACE_TOKEN`, every
    /// other character keeps catcode `OTHER`.
    fn str_tok(character: u8) -> i32 {
        if character == b' ' {
            crate::scanner::SPACE_TOKEN
        } else {
            other(character)
        }
    }

    #[test]
    fn iftrue_selects_the_true_branch() {
        let mut state = state();
        state.set_source_text("\\iftrue T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn iffalse_selects_the_else_branch() {
        let mut state = state();
        state.set_source_text("\\iffalse T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'F'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn false_conditionals_skip_nested_conditionals() {
        let mut state = state();
        state.set_source_text("\\iffalse \\iftrue A\\fi B\\else C\\fi");

        assert_eq!(token(&mut state), letter(b'C'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifnum_compares_scanned_decimal_integers() {
        let mut state = state();
        state.set_source_text("\\ifnum 2>1 T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn ifodd_tests_integer_parity() {
        let mut state = state();
        state.set_source_text("\\ifodd 3 T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
    }

    #[test]
    fn ifx_compares_macro_replacement_token_lists() {
        let mut state = state();
        state.define_simple_macro("a", &[letter(b'x')]).unwrap();
        state.define_simple_macro("b", &[letter(b'x')]).unwrap();
        state.set_source_text("\\ifx\\a\\b T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
    }

    #[test]
    fn ifdefined_does_not_define_missing_names() {
        let mut state = state();
        state.set_source_text("\\ifdefined\\missing F\\else T\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert!(state.cs_table.lookup_str("missing").is_none());
    }

    #[test]
    fn ifcsname_queries_without_creating_the_name() {
        let mut state = state();
        state.set_source_text("\\ifcsname relax\\endcsname T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
    }

    #[test]
    fn ifprimitive_matches_the_original_primitive_meaning() {
        let mut state = state();
        state.set_source_text("\\ifprimitive\\relax T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
    }

    #[test]
    fn ifprimitive_rejects_a_redefined_primitive_name() {
        let mut state = state();
        state.define_cs("relax", CommandCode::ParEnd, 0);
        state.set_source_text("\\ifprimitive\\relax T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'F'));
    }

    #[test]
    fn unless_negates_an_if_test() {
        let mut state = state();
        state.set_source_text("\\unless\\iffalse T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
    }

    #[test]
    fn iffontchar_reports_tfm_char_existence() {
        let mut state = state();
        state.register_font_source("cmr10", crate::state::FontRecord::default());
        state.set_source_text("\\font\\foo=cmr10 \\foo");
        state.dispatch_x_command().unwrap();
        state.dispatch_x_command().unwrap();
        state.font_table[1].bc = 32;
        state.font_table[1].ec = 126;
        state.set_font_char_metric(
            1,
            u32::from(b'x'),
            crate::state::FontCharMetric {
                width: 10,
                height: 7,
                depth: 0,
                italic: 0,
            },
        );

        state.set_source_text("\\iffontchar\\foo`x T\\else F\\fi");
        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);

        state.set_source_text("\\iffontchar\\foo`y T\\else F\\fi");
        assert_eq!(token(&mut state), letter(b'F'));
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn unless_before_a_non_conditional_recovers() {
        let mut state = state();
        state.set_source_text("\\unless\\relax X");
        // C uses `back_error`: the token is pushed back and expansion
        // continues, so `\relax` and `X` are still delivered.
        assert_ne!(token(&mut state), 0);
        assert!(
            state.transcript.iter().any(|l| l.contains("unless")),
            "transcript: {:?}",
            state.transcript
        );
    }

    #[test]
    fn ifcase_selects_the_requested_or_branch() {
        let mut state = state();
        state.set_source_text("\\ifcase1 A\\or B\\else C\\fi");

        assert_eq!(token(&mut state), letter(b'B'));
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn number_conversion_inserts_decimal_character_tokens() {
        let mut state = state();
        state.set_source_text("\\number-42");

        assert_eq!(token(&mut state), other(b'-'));
        assert_eq!(token(&mut state), other(b'4'));
        assert_eq!(token(&mut state), other(b'2'));
    }

    #[test]
    fn romannumeral_conversion_inserts_lowercase_numerals() {
        let mut state = state();
        state.set_source_text("\\romannumeral2024");

        for expected in *b"mmxxiv" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn string_conversion_spell_out_a_control_sequence_name() {
        let mut state = state();
        state.set_source_text("\\string\\relax");

        for expected in *b"\\relax" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn the_count_register_expands_to_decimal_text() {
        let mut state = state();
        state.set_count(0, 123).unwrap();
        state.set_source_text("\\the\\count0");

        for expected in *b"123" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn the_toks_register_inserts_a_copy_of_its_token_list() {
        let mut state = state();
        let head = state.store_token_list(&[letter(b'z')]).unwrap();
        state.set_toks(0, head).unwrap();
        state.set_source_text("\\the\\toks0");

        assert_eq!(token(&mut state), letter(b'z'));
        assert_eq!(state.toks(0).unwrap(), head);
    }

    #[test]
    fn unmatched_else_reports_an_error() {
        let mut state = state();
        state.set_source_text("\\else");

        let err = state.next_expanded_token().unwrap_err();
        assert!(matches!(err, TeXMouthError::MacroArgumentMismatch { .. }));
    }

    #[test]
    fn nine_parameter_macros_round_trip_all_arguments() {
        let mut state = state();
        let params = [
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            MATCH_MARK,
            END_MATCH_TOKEN,
        ];
        let replacement = [
            OUT_PARAM_TOKEN + 9,
            OUT_PARAM_TOKEN + 8,
            OUT_PARAM_TOKEN + 7,
            OUT_PARAM_TOKEN + 6,
            OUT_PARAM_TOKEN + 5,
            OUT_PARAM_TOKEN + 4,
            OUT_PARAM_TOKEN + 3,
            OUT_PARAM_TOKEN + 2,
            OUT_PARAM_TOKEN + 1,
        ];
        state
            .define_macro("nine", &params, &replacement, CommandCode::Call)
            .unwrap();
        state.set_source_text("\\nine ABCDEFGHI");

        for expected in *b"IHGFEDCBA" {
            assert_eq!(token(&mut state), letter(expected));
        }
    }

    #[test]
    fn macro_with_two_delimited_arguments() {
        let mut state = state();
        state
            .define_macro(
                "two",
                &[
                    MATCH_MARK,
                    OTHER_TOKEN + i32::from(b','),
                    MATCH_MARK,
                    OTHER_TOKEN + i32::from(b';'),
                    END_MATCH_TOKEN,
                ],
                &[OUT_PARAM_TOKEN + 2, OUT_PARAM_TOKEN + 1],
                CommandCode::Call,
            )
            .unwrap();
        state.set_source_text("\\two a,bb;");

        assert_eq!(token(&mut state), letter(b'b'));
        assert_eq!(token(&mut state), letter(b'b'));
        assert_eq!(token(&mut state), letter(b'a'));
    }

    #[test]
    fn ifnum_without_operands_inserts_relax_and_preserves_fi() {
        // `\ifnum\fi` recovers through `insert_relax`: the triggering `\fi`
        // is pushed back, a frozen `\relax` is inserted ahead of it, and the
        // preserved `\fi` still closes the conditional afterwards.
        let mut state = state();
        state.set_source_text("\\ifnum\\fi");

        assert_eq!(token(&mut state), CS_TOKEN_FLAG + FROZEN_RELAX_ID);
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifcase_beyond_the_last_or_selects_the_else_branch() {
        let mut state = state();
        state.set_source_text("\\ifcase2 A\\or B\\else C\\fi");

        assert_eq!(token(&mut state), letter(b'C'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifcase_negative_selects_the_else_branch() {
        let mut state = state();
        state.set_source_text("\\ifcase-1 A\\or B\\else C\\fi");

        assert_eq!(token(&mut state), letter(b'C'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn if_compares_a_control_sequence_with_relax() {
        let mut state = state();
        state.define_cs("foo", CommandCode::Relax, TOO_BIG_USV);
        state.set_source_text("\\if\\relax\\foo T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifcat_normalizes_control_sequences() {
        let mut state = state();
        state.define_cs("foo", CommandCode::CharGiven, 0);
        state.define_cs("bar", CommandCode::CharGiven, 0);
        state.set_source_text("\\ifcat\\foo\\bar T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    fn define_active_macro(state: &mut TeXEngineState, character: char, replacement: &[i32]) {
        let mut definition = vec![END_MATCH_TOKEN];
        definition.extend_from_slice(replacement);
        let definition_head = state.store_token_list(&definition).unwrap();
        let reference = state.get_avail().unwrap();
        state.mem[reference as usize].s0 = TEX_NULL;
        state.mem[reference as usize].s1 = definition_head;
        state.set_cat_code(character as u32, CAT_ACTIVE_CHAR);
        state.define_active_cs(character, CommandCode::Call, reference);
    }

    #[test]
    fn if_decodes_a_noexpanded_active_character() {
        let mut state = state();
        define_active_macro(&mut state, '~', &[letter(b'x')]);
        state.set_source_text("\\if\\noexpand~\\noexpand~ T\\else F\\fi");

        // The space after the second operand is an ordinary input token.
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifcat_compares_a_noexpanded_active_character_category() {
        let mut state = state();
        define_active_macro(&mut state, '~', &[letter(b'x')]);
        state.set_source_text("\\ifcat\\noexpand~a F\\else T\\fi");

        // `~` keeps category 13 while `a` is a letter, so the test is false.
        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifdim_compares_scanned_dimensions() {
        let mut positive_state = state();
        positive_state.set_source_text("\\ifdim 1pt>0pt T\\else F\\fi");

        assert_eq!(token(&mut positive_state), letter(b'T'));
        assert_eq!(token(&mut positive_state), 0);
        assert_eq!(positive_state.cond_ptr, TEX_NULL);

        let mut negative_state = state();
        negative_state.set_source_text("\\ifdim -2pt<0pt T\\else F\\fi");

        assert_eq!(token(&mut negative_state), letter(b'T'));
        assert_eq!(token(&mut negative_state), 0);
        assert_eq!(negative_state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifeof_reports_end_of_file_without_read_streams() {
        // Closed streams follow C's `read_open[cur_val] == closed` test.
        let mut state = state();
        state.set_source_text("\\ifeof0 T\\else F\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn ifeof_observes_an_open_read_stream() {
        let mut state = state();
        state
            .open_read_stream(
                0,
                crate::scanner::FileInputSource::new("stream", 18, Default::default()),
            )
            .unwrap();
        state.set_source_text("\\ifeof0 F\\else T\\fi");

        assert_eq!(token(&mut state), letter(b'T'));
        assert_eq!(token(&mut state), 0);
        assert_eq!(state.cond_ptr, TEX_NULL);
    }

    #[test]
    fn the_expands_a_macro_reaching_an_internal_quantity() {
        let mut state = state();
        state.set_count(0, 456).unwrap();
        let count_cs = state.cs_table.lookup_str("count").unwrap();
        state
            .define_simple_macro("foo", &[CS_TOKEN_FLAG + count_cs, other(b'0')])
            .unwrap();
        state.set_source_text("\\the\\foo");

        for expected in *b"456" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn the_dimen_register_prints_scaled_points() {
        let mut state = state();
        state.set_dimen(0, 3 * 65536 / 2).unwrap();
        state.set_source_text("\\the\\dimen0");

        for expected in *b"1.5pt" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn the_dimen_register_prints_a_zero_fraction() {
        let mut state = state();
        state.set_dimen(0, 65536).unwrap();
        state.set_source_text("\\the\\dimen0");

        for expected in *b"1.0pt" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn the_dimen_register_prints_negative_scaled_points() {
        let mut state = state();
        state.set_dimen(0, -(3 * 65536 / 2)).unwrap();
        state.set_source_text("\\the\\dimen0");

        for expected in *b"-1.5pt" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn string_of_an_active_character_prints_just_the_character() {
        let mut state = state();
        state.set_cat_code(u32::from(b'~'), CAT_ACTIVE_CHAR);
        state.define_active_cs('~', CommandCode::Relax, 0);
        state.set_source_text("\\string~");

        assert_eq!(token(&mut state), other(b'~'));
        assert_eq!(token(&mut state), crate::scanner::SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn string_of_the_null_control_sequence_prints_csname_endcsname() {
        let mut state = state();
        // A trailing `\` at end of line produces the null control sequence.
        let units: Vec<i32> = "\\string\\".chars().map(|c| c as i32).collect();
        state.set_source_units(&units);

        for &expected in b"\\csname\\endcsname" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    #[test]
    fn string_of_a_character_token_prints_the_character() {
        let mut state = state();
        state.set_source_text("\\string a");

        assert_eq!(token(&mut state), other(b'a'));
        assert_eq!(token(&mut state), crate::scanner::SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn expanded_inserts_the_fully_expanded_general_text() {
        let mut state = state();
        state.set_source_text("\\expanded{a\\number12b}");

        assert_eq!(token(&mut state), letter(b'a'));
        assert_eq!(token(&mut state), other(b'1'));
        assert_eq!(token(&mut state), other(b'2'));
        assert_eq!(token(&mut state), letter(b'b'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn uchar_and_ucharcharcat_insert_character_tokens() {
        let mut state = state();
        state.set_source_text("\\Uchar65\\Ucharcat66 11");

        assert_eq!(token(&mut state), other(b'A'));
        assert_eq!(token(&mut state), letter(b'B'));
        assert_eq!(token(&mut state), 0);
    }

    #[test]
    fn strcmp_compares_expanded_general_text() {
        let mut state = state();
        state.set_source_text("\\strcmp{a\\number12}{a12}");

        assert_eq!(token(&mut state), other(b'0'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), 0);
    }

    /// Collects inserted convert text until end of input.
    fn collect_text(state: &mut TeXEngineState) -> String {
        let mut text = String::new();
        loop {
            let t = token(state);
            if t == 0 {
                break;
            }
            if t == crate::scanner::SPACE_TOKEN {
                text.push(' ');
            } else if let Some(ch) = char::from_u32((t % MAX_CHAR_VAL).max(0) as u32) {
                text.push(ch);
            }
        }
        text
    }

    #[test]
    fn filesize_reports_registered_byte_count() {
        let mut state = state();
        state.register_input_byte_source("data.bin", b"hello world!".to_vec());
        state.set_source_text("\\filesize{data.bin}");
        assert_eq!(collect_text(&mut state).trim(), "12");

        // Unresolvable names evaluate to the empty string, as in C.
        state.set_source_text("\\filesize{missing.bin}");
        assert_eq!(collect_text(&mut state).trim(), "");
    }

    #[test]
    fn filedump_hex_dumps_bytes_from_offset() {
        let mut state = state();
        state.register_input_byte_source("data.bin", b"\x01\xAB\xFF\x00xyz".to_vec());
        state.set_source_text("\\filedump offset 1 length 3{data.bin}");
        assert_eq!(collect_text(&mut state).trim(), "ABFF00");

        // A missing `length` keyword leaves C's zero default → empty dump.
        state.set_source_text("\\filedump{data.bin}");
        assert_eq!(collect_text(&mut state).trim(), "");
    }

    #[test]
    fn filedump_negative_offset_reports_error_and_clamps() {
        let mut state = state();
        state.register_input_byte_source("data.bin", b"abc".to_vec());
        state.set_source_text("\\filedump offset -5{data.bin}");

        let text = collect_text(&mut state);
        assert_eq!(state.error_count, 1);
        assert!(state
            .transcript
            .iter()
            .any(|l| l.starts_with("! Bad file offset")));
        // The clamped offset is zero, but no `length` keyword was given.
        assert_eq!(text.trim(), "");
    }

    #[test]
    fn filemoddate_formats_registered_mtime() {
        let mut state = state();
        state.register_input_byte_source("f.tex", b"x".to_vec());
        state.register_input_mtime("f.tex", 1_700_000_000);
        state.set_source_text("\\filemoddate{f.tex}");
        // 1700000000 = 2023-11-14 22:13:20 UTC.
        assert_eq!(collect_text(&mut state).trim(), "D:20231114221320Z");
    }

    #[test]
    fn mdfivesum_hashes_text_and_file_bytes() {
        let mut state = state();
        state.set_source_text("\\mdfivesum{abc}");
        assert_eq!(
            collect_text(&mut state).trim(),
            "900150983CD24FB0D6963F7D28E17F72"
        );

        state.register_input_byte_source("f.bin", b"abc".to_vec());
        state.set_source_text("\\mdfivesum file{f.bin}");
        assert_eq!(
            collect_text(&mut state).trim(),
            "900150983CD24FB0D6963F7D28E17F72"
        );
    }

    #[test]
    fn creationdate_inserts_the_job_start_time() {
        let mut state = state();
        state.init_start_time(1_700_000_000);
        state.set_source_text("\\creationdate");
        assert_eq!(collect_text(&mut state).trim(), "D:20231114221320Z");
    }

    #[test]
    fn uniform_and_normal_deviates_use_the_deterministic_tex_generator() {
        let mut state = state();
        state.rand.init(123_456);
        state.set_source_text("\\uniformdeviate10");
        let uniform = token(&mut state);
        assert!((other(b'0')..=other(b'9')).contains(&uniform));

        state.rand.init(123_456);
        state.set_source_text("\\normaldeviate");
        let first = token(&mut state);
        state.rand.init(123_456);
        state.set_source_text("\\normaldeviate");
        assert_eq!(token(&mut state), first);
    }

    #[test]
    fn unexpanded_and_detokenize_insert_general_text() {
        let mut state = state();
        state.set_source_text("\\unexpanded{x\\count0} \\detokenize{x\\count0}");
        let count = state.cs_table.lookup_str("count").unwrap();

        assert_eq!(token(&mut state), letter(b'x'));
        assert_eq!(token(&mut state), CS_TOKEN_FLAG + count);
        assert_eq!(token(&mut state), other(b'0'));

        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), other(b'x'));
        assert_eq!(token(&mut state), other(b'\\'));
        assert_eq!(token(&mut state), other(b'c'));
    }

    #[test]
    fn scantokens_rescans_general_text_as_a_pseudo_file() {
        let mut state = state();
        state.set_source_text("\\scantokens{a\\count0}");
        let count = state.cs_table.lookup_str("count").unwrap();

        assert_eq!(token(&mut state), letter(b'a'));
        assert_eq!(token(&mut state), CS_TOKEN_FLAG + count);
        assert_eq!(token(&mut state), other(b'0'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(state.cur_input.name, 1);
        assert_eq!(state.in_open, 0);
    }

    #[test]
    fn scantokens_splits_lines_on_newlinechar() {
        let mut state = state();
        state
            .set_int_par(crate::state::INT_PAR_NEW_LINE_CHAR, i32::from(b'|'))
            .unwrap();
        state.set_source_text("\\scantokens{a|b}");

        assert_eq!(token(&mut state), letter(b'a'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), letter(b'b'));
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(token(&mut state), SPACE_TOKEN);
        assert_eq!(state.in_open, 0);
    }

    #[test]
    fn meaning_conversion_inserts_the_current_meaning_as_character_text() {
        let mut state = state();
        state.set_source_text("\\meaning\\relax");

        for expected in "\\relax".chars() {
            assert_eq!(token(&mut state), other(expected as u8));
        }
    }

    #[test]
    fn margin_kern_conversions_read_sparse_and_dense_hboxes() {
        let mut state = state();
        let boxed = state.new_null_box().unwrap();
        let left = state.get_node(2).unwrap();
        state.set_node_type(left, MARGIN_KERN_NODE).unwrap();
        state.set_node_subtype(left, 0).unwrap();
        state.mem[(left + 1) as usize].s1 = 2 * 65536;
        state.mem[(boxed + 5) as usize].s1 = left;
        state.set_box_reg(0, boxed).unwrap();

        state.set_source_text("\\leftmarginkern0");
        for expected in *b"2.0pt" {
            assert_eq!(token(&mut state), other(expected));
        }

        let sparse_box = state.new_null_box().unwrap();
        let right = state.get_node(2).unwrap();
        state.set_node_type(right, MARGIN_KERN_NODE).unwrap();
        state.set_node_subtype(right, 1).unwrap();
        state.mem[(right + 1) as usize].s1 = 3 * 65536;
        state.mem[(sparse_box + 5) as usize].s1 = right;
        let element = state.find_sa_element(SA_BOX_VAL, 300, true).unwrap();
        let idx = state.checked_engine_mem_index(element).unwrap();
        state.mem[idx + 1].s1 = sparse_box;

        state.set_source_text("\\rightmarginkern300");
        for expected in *b"3.0pt" {
            assert_eq!(token(&mut state), other(expected));
        }
    }

    fn font_state(record: crate::state::FontRecord) -> TeXEngineState {
        let mut state = state();
        state.font_table.push(record);
        state.define_cs("foo", CommandCode::SetFont, 1);
        state
    }

    fn packed_name(name: &str) -> Vec<i32> {
        name.chars().map(|c| c as i32).collect()
    }

    #[test]
    fn fontname_prints_tfm_name_and_requested_size() {
        let record = crate::state::FontRecord {
            name: packed_name("cmr10"),
            ..Default::default()
        };
        let mut state = font_state(record);
        state.set_source_text("\\fontname\\foo");
        for expected in "cmr10".chars() {
            assert_eq!(token(&mut state), other(expected as u8));
        }

        let sized = crate::state::FontRecord {
            name: packed_name("cmr10"),
            size: 12 * 65_536,
            ..Default::default()
        };
        let mut state = font_state(sized);
        state.set_source_text("\\fontname\\foo");
        for expected in "cmr10 at 12.0pt".chars() {
            assert_eq!(token(&mut state), str_tok(expected as u8));
        }
    }

    #[test]
    fn fontname_quotes_native_font_names() {
        let record = crate::state::FontRecord {
            name: packed_name("Native Font"),
            ..Default::default()
        };
        let mut state = font_state(record);
        state.font_area.insert(1, crate::state::OTGR_FONT_FLAG);
        state.set_source_text("\\fontname\\foo");
        for expected in "\"Native Font\"".chars() {
            assert_eq!(token(&mut state), str_tok(expected as u8));
        }

        let quoted = crate::state::FontRecord {
            name: packed_name("has\"quote"),
            ..Default::default()
        };
        let mut state = font_state(quoted);
        state.font_area.insert(1, crate::state::OTGR_FONT_FLAG);
        state.set_source_text("\\fontname\\foo");
        for expected in "'has\"quote'".chars() {
            assert_eq!(token(&mut state), str_tok(expected as u8));
        }
    }

    #[test]
    fn xetex_glyphname_prints_native_glyph_names() {
        let mut record = crate::state::FontRecord::default();
        record.glyph_id_names.insert(7, "A.gid".to_string());
        let mut state = font_state(record);
        state.font_area.insert(1, crate::state::OTGR_FONT_FLAG);
        state.set_source_text("\\XeTeXglyphname\\foo7");
        for expected in *b"A.gid" {
            assert_eq!(token(&mut state), other(expected));
        }

        let mut tfm = font_state(crate::state::FontRecord::default());
        tfm.set_source_text("\\XeTeXglyphname\\foo7");
        assert_eq!(token(&mut tfm), other(b'7'));
        assert!(tfm
            .transcript
            .iter()
            .any(|line| line.contains("not a native platform font")));
    }

    #[test]
    fn xetex_font_name_queries_use_registered_engine_strings() {
        let record = crate::state::FontRecord {
            font_type: 3,
            ..Default::default()
        };
        let mut state = font_state(record);
        state.font_area.insert(1, crate::state::OTGR_FONT_FLAG);
        state.register_font_engine_name(1, 35, 4, 0, "Small Caps");
        state.register_font_engine_name(1, 36, 4, 2, "All Small Caps");

        state.set_source_text("\\XeTeXfeaturename\\foo4");
        for expected in "Small Caps".chars() {
            assert_eq!(token(&mut state), str_tok(expected as u8));
        }

        state.set_source_text("\\XeTeXselectorname\\foo4 2");
        for expected in "All Small Caps".chars() {
            assert_eq!(token(&mut state), str_tok(expected as u8));
        }

        // \XeTeXvariationname requires an AAT font: the Graphite font
        // records the C diagnostic and emits nothing.
        state.set_source_text("\\XeTeXvariationname\\foo x");
        assert_eq!(token(&mut state), letter(b'x'));
        assert!(state
            .transcript
            .iter()
            .any(|line| line.contains("not an AAT font")));
    }
}
