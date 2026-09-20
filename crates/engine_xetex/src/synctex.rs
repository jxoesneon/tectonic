//! SyncTeX input/output synchronization recorder (`xetex-synctex.c`).
//!
//! Ported per-engine: the C `synctex_ctxt` file-static struct becomes
//! [`SyncTeXContext`] on [`TeXEngineState`], and the `.synctex` text stream
//! accumulates in [`SyncTeXContext::bytes`] instead of a `rust_output_handle_t`.
//! The driver is responsible for compressing and storing the finished stream
//! under [`SyncTeXContext::output_name`] (`<jobname>.synctex.gz`).

use crate::scanner::{
    TeXMouthError, CS_TOKEN_FLAG, INSERTED, LEFT_BRACE_TOKEN, MAX_CHAR_VAL, RIGHT_BRACE_TOKEN,
    SPACE_TOKEN,
};
use crate::state::{
    TeXEngineState, BOX_NODE_SIZE, C_FROZEN_SPECIAL, GLUE_NODE, INT_PAR_SYNCTEX, KERN_NODE,
    MEDIUM_NODE_SIZE, RULE_NODE, RULE_NODE_SIZE, SPECIAL_NODE, TEX_NULL, WRITE_NODE_SIZE,
};

/// `compare_paths` (`xetex-texmfmp.c`): byte-wise path comparison where any
/// `IS_DIR_SEP` byte (`/`) on both sides compares equal. C reads signed
/// `char`s; `i8` preserves that ordering quirk for non-ASCII bytes.
fn compare_paths(p1: &[u8], p2: &[u8]) -> i32 {
    let (mut i, mut j) = (0usize, 0usize);
    let ret = loop {
        let a = p1.get(i).copied().unwrap_or(0);
        let b = p2.get(j).copied().unwrap_or(0);
        let ret = i32::from(a as i8) - i32::from(b as i8);
        if !((ret == 0 && b != 0) || (a == b'/' && b == b'/')) {
            break ret;
        }
        i += 1;
        j += 1;
    };
    ret.signum()
}

/// `.synctex` file-format version (`SYNCTEX_VERSION`).
pub const SYNCTEX_VERSION: i32 = 1;

/// Page-origin bias added to `cur_h`/`cur_v` (`SYNCTEX_CURH`/`SYNCTEX_CURV`):
/// one inch in scaled points so the recorded origin is the top-left corner.
const SYNCTEX_OFFSET: i32 = 4_736_287;

/// The pending deferred recorder (`synctex_ctxt.recorder`). C stores a
/// function pointer, but only ever installs `synctex_record_node_kern`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRecorder {
    Kern,
}

/// Per-engine SyncTeX context (C's `synctex_ctxt`).
#[derive(Debug, Clone)]
pub struct SyncTeXContext {
    /// Buffered `.synctex` text stream (C's `synctex_ctxt.file` writes).
    pub bytes: Vec<u8>,
    /// Resolved output name `<jobname>.synctex.gz`, set when the stream opens.
    pub output_name: Option<String>,
    /// Whether the stream is open (C's `file != INVALID_HANDLE`).
    pub file_open: bool,
    /// First input file's name, deferred until the stream opens (`root_name`).
    pub root_name: Option<String>,
    /// Number of records written since the stream opened (`count`).
    pub count: i32,
    /// Node pending deferred recording (`node`).
    pub node: i32,
    /// Deferred recorder, if any (`recorder`; only kern is ever installed).
    recorder: Option<PendingRecorder>,
    /// Current sync tag context (`tag`).
    pub tag: i32,
    /// Current sync line context (`line`).
    pub line: i32,
    /// Captured horizontal page coordinate (`curh`).
    pub curh: i32,
    /// Captured vertical page coordinate (`curv`).
    pub curv: i32,
    /// `\mag` captured at first shipout (`magnification`).
    pub magnification: i32,
    /// Output unit divisor (`unit`; C defaults to 1).
    pub unit: i32,
    /// Byte offset anchor bookkeeping (`total_length`).
    pub total_length: i32,
    /// Last recorded vertical coordinate (`lastv`).
    pub lastv: i32,
    /// Nested PDF xform depth (`form_depth`).
    pub form_depth: i32,
    /// Global input-tag counter (`synctex_tag_counter`).
    pub tag_counter: u32,
    /// `flags.content_ready` — settings+content banner emitted.
    pub content_ready: bool,
    /// `flags.off` — SyncTeX permanently disabled.
    pub off: bool,
    /// `flags.not_void` — sheet contained synchronizable material.
    pub not_void: bool,
    /// `flags.warn` — one-shot disabled-warning latch.
    pub warn: bool,
    /// Resolved rule width captured from the shipout loop (C's `rule_wd`
    /// global, read inside `synctex_record_node_rule`).
    pub rule_wd: i32,
    /// Resolved rule height (C's `rule_ht` global).
    pub rule_ht: i32,
    /// Resolved rule depth (C's `rule_dp` global).
    pub rule_dp: i32,
}

impl Default for SyncTeXContext {
    fn default() -> Self {
        Self {
            bytes: Vec::new(),
            output_name: None,
            file_open: false,
            root_name: None,
            count: 0,
            node: 0,
            recorder: None,
            tag: 0,
            line: 0,
            curh: 0,
            curv: 0,
            magnification: 0,
            unit: 0,
            total_length: 0,
            lastv: -1,
            form_depth: 0,
            tag_counter: 0,
            content_ready: false,
            off: false,
            not_void: false,
            warn: false,
            rule_wd: 0,
            rule_ht: 0,
            rule_dp: 0,
        }
    }
}

impl SyncTeXContext {
    fn write(&mut self, text: &str) -> usize {
        self.bytes.extend_from_slice(text.as_bytes());
        text.len()
    }
}

impl TeXEngineState {
    /// `cur_h` biased to the top-left page origin (`SYNCTEX_CURH`).
    fn synctex_curh(&self) -> i32 {
        self.cur_h + SYNCTEX_OFFSET
    }

    /// `cur_v` biased to the top-left page origin (`SYNCTEX_CURV`).
    fn synctex_curv(&self) -> i32 {
        self.cur_v + SYNCTEX_OFFSET
    }

    /// `SYNCTEX_TAG_MODEL`: the tag half of a node's trailing sync word.
    fn sync_tag(&self, p: i32, node_size: i32) -> i32 {
        self.mem[(p + node_size - 1) as usize].s0
    }

    /// `SYNCTEX_LINE_MODEL`: the line half of a node's trailing sync word.
    fn sync_line(&self, p: i32, node_size: i32) -> i32 {
        self.mem[(p + node_size - 1) as usize].s1
    }

    /// `INTPAR(synctex) != 0` without the bounds-check error path.
    fn synctex_int_par(&self) -> i32 {
        self.int_par(INT_PAR_SYNCTEX).unwrap_or(0)
    }

    /// `SYNCTEX_IGNORE(NODE)` for the file-gated recorders.
    fn synctex_ignore(&self) -> bool {
        self.synctex.off || self.synctex_int_par() == 0 || !self.synctex.file_open
    }

    /// `SYNCTEX_IGNORE(NODE,TYPE)` for the rule/glue/kern recorders: ignores
    /// nodes whose tag or line was never synchronized.
    fn synctex_ignore_typed(&self, p: i32, node_size: i32) -> bool {
        self.synctex.off
            || self.synctex_int_par() == 0
            || self.sync_tag(p, node_size) <= 0
            || self.sync_line(p, node_size) <= 0
    }

    /// `SYNCTEX_CONTEXT_DID_CHANGE(NODE,TYPE)`.
    fn synctex_context_did_change(&self, p: i32, node_size: i32) -> bool {
        self.synctex.node == 0
            || self.sync_tag(p, node_size) != self.synctex.tag
            || self.sync_line(p, node_size) != self.synctex.line
    }

    /// `get_current_name`: the current input file's name as opened
    /// (`abspath_of_input_file`; the resolved byte-source name under
    /// virtualized I/O).
    fn current_input_name(&self) -> String {
        self.abspath_of_input_file.clone()
    }

    /// `synctex_init_command`: resets the context and maps the driver
    /// `synctex_enabled` flag onto `\synctex` (C `main_init`).
    pub fn synctex_init_command(&mut self) {
        self.synctex = SyncTeXContext::default();
        let enabled = i32::from(self.synctex_enabled);
        let _ = self.set_int_par(INT_PAR_SYNCTEX, enabled);
    }

    /// `synctexabort`: closes the stream and permanently disables recording.
    fn synctex_abort(&mut self) {
        self.synctex.file_open = false;
        self.synctex.root_name = None;
        self.synctex.off = true;
    }

    /// `ttstub_issue_warning` for the SyncTeX disabled notice: written
    /// straight to the terminal stream like C's stderr write. C's two call
    /// sites use slightly different dashes; the caller picks the variant.
    fn synctex_warn_disabled(&mut self, dash: &str) {
        self.synctex.warn = true;
        self.term_out.extend_from_slice(
            format!("SyncTeX was disabled {dash} changing the value of \\synctex has no effect\n")
                .as_bytes(),
        );
    }

    /// `synctex_dot_open`: opens `<jobname>.synctex` and writes the preamble.
    /// Returns whether the stream is open (C's handle check). The record
    /// itself lands in the `<jobname>.synctex.gz` output stream.
    fn synctex_dot_open(&mut self) -> bool {
        if self.synctex.off || self.synctex_int_par() == 0 {
            return false;
        }
        if self.synctex.file_open {
            return true;
        }

        let job_name = self.job_name.clone().unwrap_or_default();
        if job_name.is_empty() {
            self.synctex_abort();
            return false;
        }

        self.synctex.output_name = Some(format!("{job_name}.synctex.gz"));
        self.synctex.file_open = true;
        self.synctex_record_preamble();

        if self.synctex.magnification == 0 {
            self.synctex.magnification = 1000;
        }
        self.synctex.unit = 1;

        if let Some(root_name) = self.synctex.root_name.take() {
            self.synctex_record_input(1, &root_name);
        }
        self.synctex.count = 0;
        true
    }

    /// `synctex_prepare_content`: emits the settings and `Content:` banner
    /// once, as late as possible.
    fn synctex_prepare_content(&mut self) -> bool {
        if self.synctex.content_ready {
            return self.synctex.file_open;
        }
        if self.synctex_dot_open() {
            self.synctex_record_settings();
            self.synctex_record_content();
            self.synctex.content_ready = true;
            return true;
        }
        self.synctex_abort();
        false
    }

    /// `synctex_start_input`: assigns a fresh sync tag to the input frame
    /// being opened and records the tag↔name mapping. The first file's name
    /// is stashed until the stream opens.
    pub fn synctex_start_input(&mut self) {
        if self.synctex.off {
            return;
        }
        if self.synctex.tag_counter != u32::MAX {
            self.synctex.tag_counter += 1;
        } else {
            // Tag space exhausted: softly ignore subsequent files.
            self.synctex.tag_counter = 0;
            return;
        }
        self.cur_input.synctex_tag = self.synctex.tag_counter as i32;
        if self.synctex.tag_counter == 1 {
            let mut name = self.current_input_name();
            if name.is_empty() {
                name = "texput".to_string();
            }
            self.synctex.root_name = Some(name);
            return;
        }
        // Always record the input, even if INTPAR(synctex) is 0 — but only
        // when the stream is or can be opened.
        if self.synctex.file_open || self.synctex_dot_open() {
            let tag = self.cur_input.synctex_tag;
            let name = self.current_input_name();
            self.synctex_record_input(tag, &name);
        }
    }

    /// `synctex_terminate`: writes the postamble and closes the stream
    /// (`close_files_and_terminate`).
    pub fn synctex_terminate(&mut self, _log_opened: bool) {
        if self.synctex.file_open {
            self.synctex_record_postamble();
            self.synctex.file_open = false;
        }
        self.synctex_abort();
    }

    /// `synctex_sheet`: `{` sheet record at the head of `ship_out`.
    pub fn synctex_sheet(&mut self, mag: i32) {
        if self.synctex.off {
            if self.synctex_int_par() != 0 && !self.synctex.warn {
                self.synctex_warn_disabled("--");
            }
            return;
        }
        if self.total_pages == 0 && mag > 0 {
            self.synctex.magnification = mag;
        }
        if self.synctex_prepare_content() {
            self.synctex_record_sheet(self.total_pages + 1);
        }
    }

    /// `synctex_teehs`: `}` sheet record at the tail of `ship_out`.
    pub fn synctex_teehs(&mut self) {
        if self.synctex.off || !self.synctex.file_open {
            return;
        }
        self.synctex_record_teehs(self.total_pages);
    }

    /// `synctex_vlist`: `[` record at the head of `vlist_out`.
    pub fn synctex_vlist(&mut self, this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        self.synctex.node = this_box;
        self.synctex.recorder = None;
        self.synctex.tag = self.sync_tag(this_box, BOX_NODE_SIZE);
        self.synctex.line = self.sync_line(this_box, BOX_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex_record_node_vlist(this_box);
    }

    /// `synctex_tsilv`: `]` record balancing `synctex_vlist`.
    pub fn synctex_tsilv(&mut self, this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        self.synctex.node = this_box;
        self.synctex.tag = self.sync_tag(this_box, BOX_NODE_SIZE);
        self.synctex.line = self.sync_line(this_box, BOX_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        self.synctex_record_node_tsilv();
    }

    /// `synctex_void_vlist`: `v` record for an empty vlist.
    pub fn synctex_void_vlist(&mut self, p: i32, _this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        self.synctex.node = p;
        self.synctex.tag = self.sync_tag(p, BOX_NODE_SIZE);
        self.synctex.line = self.sync_line(p, BOX_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        self.synctex_record_node_void_vlist(p);
    }

    /// `synctex_hlist`: `(` record at the head of `hlist_out`.
    pub fn synctex_hlist(&mut self, this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        self.synctex.node = this_box;
        self.synctex.tag = self.sync_tag(this_box, BOX_NODE_SIZE);
        self.synctex.line = self.sync_line(this_box, BOX_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        self.synctex_record_node_hlist(this_box);
    }

    /// `synctex_tsilh`: `)` record balancing `synctex_hlist`.
    pub fn synctex_tsilh(&mut self, this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        self.synctex.node = this_box;
        self.synctex.tag = self.sync_tag(this_box, BOX_NODE_SIZE);
        self.synctex.line = self.sync_line(this_box, BOX_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        self.synctex_record_node_tsilh();
    }

    /// `synctex_void_hlist`: `h` record for an empty hlist; flushes any
    /// deferred kern first since the context is being replaced.
    pub fn synctex_void_hlist(&mut self, p: i32, _this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        if self.synctex.recorder.is_some() {
            self.synctex_flush_recorder();
        }
        self.synctex.node = p;
        self.synctex.tag = self.sync_tag(p, BOX_NODE_SIZE);
        self.synctex.line = self.sync_line(p, BOX_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        self.synctex_record_node_void_hlist(p);
    }

    /// `synctex_math`: `$` record for an inline math node.
    pub fn synctex_math(&mut self, p: i32, _this_box: i32) {
        if self.synctex_ignore() {
            return;
        }
        if self.synctex.recorder.is_some() && self.synctex_context_did_change(p, MEDIUM_NODE_SIZE) {
            self.synctex_flush_recorder();
        }
        self.synctex.node = p;
        self.synctex.tag = self.sync_tag(p, MEDIUM_NODE_SIZE);
        self.synctex.line = self.sync_line(p, MEDIUM_NODE_SIZE);
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        self.synctex_record_node_math(p);
    }

    /// `synctex_horizontal_rule_or_glue`: synchronous `r`/`g`/`k` records for
    /// rule, glue, and leader-kern nodes in `hlist_out`'s `move_past`.
    pub fn synctex_horizontal_rule_or_glue(&mut self, p: i32, _this_box: i32) {
        let node_type = self.node_type(p).unwrap_or(0);
        match node_type {
            RULE_NODE => {
                if self.synctex_ignore_typed(p, RULE_NODE_SIZE) {
                    return;
                }
            }
            GLUE_NODE | KERN_NODE => {
                if self.synctex_ignore_typed(p, MEDIUM_NODE_SIZE) {
                    return;
                }
            }
            _ => {
                self.term_out.extend_from_slice(
                    format!("unknown node type {node_type} in SyncTeX\n").as_bytes(),
                );
                return;
            }
        }
        self.synctex.node = p;
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        self.synctex.recorder = None;
        if node_type == RULE_NODE {
            self.synctex.tag = self.sync_tag(p, RULE_NODE_SIZE);
            self.synctex.line = self.sync_line(p, RULE_NODE_SIZE);
            self.synctex_record_node_rule();
        } else {
            self.synctex.tag = self.sync_tag(p, MEDIUM_NODE_SIZE);
            self.synctex.line = self.sync_line(p, MEDIUM_NODE_SIZE);
            if node_type == GLUE_NODE {
                self.synctex_record_node_glue(p);
            } else {
                self.synctex_record_node_kern(p);
            }
        }
    }

    /// `synctex_kern`: deferred `k` recording — consecutive kerns sharing the
    /// sync context collapse to a single record emitted on context change.
    pub fn synctex_kern(&mut self, p: i32, this_box: i32) {
        if self.synctex_ignore_typed(p, MEDIUM_NODE_SIZE) {
            return;
        }
        if self.synctex_context_did_change(p, MEDIUM_NODE_SIZE) {
            if self.synctex.recorder.is_some() {
                self.synctex_flush_recorder();
            }
            if self.synctex.node == this_box {
                // First node in the list: defer.
                self.synctex.node = p;
                self.synctex.tag = self.sync_tag(p, MEDIUM_NODE_SIZE);
                self.synctex.line = self.sync_line(p, MEDIUM_NODE_SIZE);
                self.synctex.recorder = Some(PendingRecorder::Kern);
            } else {
                self.synctex.node = p;
                self.synctex.tag = self.sync_tag(p, MEDIUM_NODE_SIZE);
                self.synctex.line = self.sync_line(p, MEDIUM_NODE_SIZE);
                self.synctex.recorder = None;
                self.synctex_record_node_kern(p);
            }
        } else {
            // Same context: just update geometry, keep deferring.
            self.synctex.node = p;
            self.synctex.tag = self.sync_tag(p, MEDIUM_NODE_SIZE);
            self.synctex.line = self.sync_line(p, MEDIUM_NODE_SIZE);
            self.synctex.recorder = Some(PendingRecorder::Kern);
        }
    }

    /// `synctex_current`: `x` record for the current source location.
    pub fn synctex_current(&mut self) {
        if self.synctex_ignore() {
            return;
        }
        let curh = self.synctex_curh();
        let curv = self.synctex_curv();
        let len = self.synctex.write(&format!(
            "x{},{}:{},{}\n",
            self.synctex.tag,
            self.synctex.line,
            curh / self.synctex.unit,
            curv / self.synctex.unit
        ));
        self.synctex.lastv = curv;
        if len > 0 {
            self.synctex.total_length += len as i32;
        } else {
            self.synctex_abort();
        }
    }

    // === src-specials (`insert_src_special`/`append_src_special`) ===

    /// `source_filename_stack[in_open]`: the current input frame's file
    /// name; `None`/empty mirrors C's `str_number == 0`.
    fn current_source_filename(&self) -> Option<&str> {
        let name = self.file_sources.get(self.in_open)?.name.as_str();
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    /// `is_new_source`: path or line differs from the remembered pair.
    fn is_new_source(&self, name: &str, lineno: i32) -> bool {
        compare_paths(name.as_bytes(), self.src_last_name.as_bytes()) != 0
            || lineno != self.src_last_lineno
    }

    /// `remember_source_info`.
    fn remember_source_info(&mut self, name: &str, lineno: i32) {
        self.src_last_name = name.to_string();
        self.src_last_lineno = lineno;
    }

    /// `make_src_special` text: `src:<lineno> <filename>` (the space after
    /// the number is required by parsers).
    fn src_special_text(&self, name: &str, lineno: i32) -> String {
        format!("src:{lineno} {name}")
    }

    /// `str_toks_cat(b, 0)`: each character becomes an `OTHER_TOKEN`, except
    /// ASCII space which becomes a `SPACE_TOKEN`.
    fn src_special_chars(text: &str) -> Vec<i32> {
        text.chars()
            .map(|ch| {
                if ch == ' ' {
                    SPACE_TOKEN
                } else {
                    12 * MAX_CHAR_VAL + ch as i32
                }
            })
            .collect()
    }

    /// `insert_src_special`: splices `\special{src:<line> <file>}` into the
    /// input stream as an inserted token list when the source position is new.
    pub fn insert_src_special(&mut self) -> Result<(), TeXMouthError> {
        let Some(name) = self.current_source_filename().map(str::to_owned) else {
            return Ok(());
        };
        let line = self.line;
        if !self.is_new_source(&name, line) {
            return Ok(());
        }
        let mut tokens = Vec::new();
        tokens.push(CS_TOKEN_FLAG + C_FROZEN_SPECIAL);
        tokens.push(LEFT_BRACE_TOKEN + i32::from(b'{'));
        tokens.extend(Self::src_special_chars(&self.src_special_text(&name, line)));
        tokens.push(RIGHT_BRACE_TOKEN + i32::from(b'}'));
        let toklist = self.store_token_list(&tokens)?;
        self.begin_token_list(toklist, INSERTED)?;
        self.remember_source_info(&name, line);
        Ok(())
    }

    /// `append_src_special`: appends a `src:` special whatsit to the current
    /// list (`insert_src_special_auto` in `main_loop`).
    pub fn append_src_special(&mut self) -> Result<(), TeXMouthError> {
        let Some(name) = self.current_source_filename().map(str::to_owned) else {
            return Ok(());
        };
        let line = self.line;
        if !self.is_new_source(&name, line) {
            return Ok(());
        }
        let node = self.new_whatsit(SPECIAL_NODE, WRITE_NODE_SIZE)?;
        self.mem[(node + 1) as usize].s0 = 0;
        self.def_ref = self.get_avail()?;
        let def_ref = self.def_ref;
        self.mem[def_ref as usize].s0 = TEX_NULL;
        let toks = Self::src_special_chars(&self.src_special_text(&name, line));
        let head = self.store_token_list(&toks)?;
        self.mem[def_ref as usize].s1 = head;
        self.mem[(node + 1) as usize].s1 = def_ref;
        self.remember_source_info(&name, line);
        Ok(())
    }

    /// `synctex_pdfxform`: `<` record for a nested PDF form.
    pub fn synctex_pdfxform(&mut self, _p: i32) {
        if self.synctex.off {
            if self.synctex_int_par() != 0 && !self.synctex.warn {
                self.synctex_warn_disabled("-");
            }
            return;
        }
        if self.synctex_prepare_content() {
            self.synctex_record_pdfxform();
        }
    }

    /// `synctex_mrofxfdp`: `>` record closing a PDF form.
    pub fn synctex_mrofxfdp(&mut self) {
        if self.synctex.file_open {
            self.synctex_record_mrofxfdp();
        }
    }

    /// `synctex_pdfrefxform`: `f` record for a form reference.
    pub fn synctex_pdfrefxform(&mut self, objnum: i32) {
        if self.synctex.file_open {
            self.synctex_record_node_pdfrefxform(objnum);
        }
    }

    /// Runs the deferred recorder on `synctex.node` (C's
    /// `(*synctex_ctxt.recorder)(synctex_ctxt.node)`).
    fn synctex_flush_recorder(&mut self) {
        if self.synctex.recorder.take() == Some(PendingRecorder::Kern) {
            let node = self.synctex.node;
            self.synctex_record_node_kern(node);
        }
    }

    // === low-level record writers ===

    fn synctex_record_preamble(&mut self) {
        let len = self
            .synctex
            .write(&format!("SyncTeX Version:{SYNCTEX_VERSION}\n"));
        if len > 0 {
            self.synctex.total_length = len as i32;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_input(&mut self, tag: i32, name: &str) {
        let len = self.synctex.write(&format!("Input:{tag}:{name}\n"));
        if len > 0 {
            self.synctex.total_length += len as i32;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_anchor(&mut self) {
        let len = self
            .synctex
            .write(&format!("!{}\n", self.synctex.total_length));
        if len > 0 {
            self.synctex.total_length = len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_settings(&mut self) {
        let len = self.synctex.write(&format!(
            "Output:pdf\nMagnification:{}\nUnit:{}\nX Offset:0\nY Offset:0\n",
            self.synctex.magnification, self.synctex.unit
        ));
        if len > 0 {
            self.synctex.total_length += len as i32;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_content(&mut self) {
        let len = self.synctex.write("Content:\n");
        if len > 0 {
            self.synctex.total_length += len as i32;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_sheet(&mut self, sheet: i32) {
        self.synctex_record_anchor();
        let len = self.synctex.write(&format!("{{{sheet}\n"));
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_teehs(&mut self, sheet: i32) {
        self.synctex_record_anchor();
        let len = self.synctex.write(&format!("}}{sheet}\n"));
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_pdfxform(&mut self) {
        if self.synctex_ignore() {
            return;
        }
        self.synctex.form_depth += 1;
        let len = self
            .synctex
            .write(&format!("<{}\n", self.synctex.form_depth));
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_mrofxfdp(&mut self) {
        self.synctex_record_anchor();
        self.synctex.form_depth -= 1;
        let len = self.synctex.write(">\n");
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_pdfrefxform(&mut self, objnum: i32) {
        self.synctex.curh = self.synctex_curh();
        self.synctex.curv = self.synctex_curv();
        if self.synctex_ignore() {
            return;
        }
        let curh = self.synctex.curh;
        let curv = self.synctex.curv;
        let len = self.synctex.write(&format!(
            "f{objnum}:{},{}\n",
            curh / self.synctex.unit,
            curv / self.synctex.unit
        ));
        self.synctex.lastv = curv;
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_void_vlist(&mut self, p: i32) {
        let (tag, line) = (
            self.sync_tag(p, BOX_NODE_SIZE),
            self.sync_line(p, BOX_NODE_SIZE),
        );
        let (h, v, w, ht, dp) = self.synctex_box_metrics(p);
        let len = self
            .synctex
            .write(&format!("v{tag},{line}:{h},{v}:{w},{ht},{dp}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_vlist(&mut self, p: i32) {
        self.synctex.not_void = true;
        let (tag, line) = (
            self.sync_tag(p, BOX_NODE_SIZE),
            self.sync_line(p, BOX_NODE_SIZE),
        );
        let (h, v, w, ht, dp) = self.synctex_box_metrics(p);
        let len = self
            .synctex
            .write(&format!("[{tag},{line}:{h},{v}:{w},{ht},{dp}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_tsilv(&mut self) {
        let len = self.synctex.write("]\n");
        if len > 0 {
            self.synctex.total_length += len as i32;
            // C deliberately does not bump `count` here.
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_void_hlist(&mut self, p: i32) {
        let (tag, line) = (
            self.sync_tag(p, BOX_NODE_SIZE),
            self.sync_line(p, BOX_NODE_SIZE),
        );
        let (h, v, w, ht, dp) = self.synctex_box_metrics(p);
        let len = self
            .synctex
            .write(&format!("h{tag},{line}:{h},{v}:{w},{ht},{dp}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_hlist(&mut self, p: i32) {
        self.synctex.not_void = true;
        let (tag, line) = (
            self.sync_tag(p, BOX_NODE_SIZE),
            self.sync_line(p, BOX_NODE_SIZE),
        );
        let (h, v, w, ht, dp) = self.synctex_box_metrics(p);
        let len = self
            .synctex
            .write(&format!("({tag},{line}:{h},{v}:{w},{ht},{dp}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_tsilh(&mut self) {
        let len = self.synctex.write(")\n");
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_glue(&mut self, p: i32) {
        let (tag, line) = (
            self.sync_tag(p, MEDIUM_NODE_SIZE),
            self.sync_line(p, MEDIUM_NODE_SIZE),
        );
        let (h, v) = (
            self.synctex.curh / self.synctex.unit,
            self.synctex.curv / self.synctex.unit,
        );
        let len = self.synctex.write(&format!("g{tag},{line}:{h},{v}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_kern(&mut self, p: i32) {
        // C reads `SYNCTEX_TAG_MODEL(p,GLUE)` here; the glue/kern models share
        // `MEDIUM_NODE_SIZE`, so the same word is read.
        let (tag, line) = (
            self.sync_tag(p, MEDIUM_NODE_SIZE),
            self.sync_line(p, MEDIUM_NODE_SIZE),
        );
        let (h, v) = (
            self.synctex.curh / self.synctex.unit,
            self.synctex.curv / self.synctex.unit,
        );
        let width = self.mem[(p + 1) as usize].s1 / self.synctex.unit;
        let len = self
            .synctex
            .write(&format!("k{tag},{line}:{h},{v}:{width}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_rule(&mut self) {
        let p = self.synctex.node;
        let (tag, line) = (
            self.sync_tag(p, RULE_NODE_SIZE),
            self.sync_line(p, RULE_NODE_SIZE),
        );
        let (h, v) = (
            self.synctex.curh / self.synctex.unit,
            self.synctex.curv / self.synctex.unit,
        );
        let (w, ht, dp) = (
            self.synctex.rule_wd / self.synctex.unit,
            self.synctex.rule_ht / self.synctex.unit,
            self.synctex.rule_dp / self.synctex.unit,
        );
        let len = self
            .synctex
            .write(&format!("r{tag},{line}:{h},{v}:{w},{ht},{dp}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_node_math(&mut self, p: i32) {
        let (tag, line) = (
            self.sync_tag(p, MEDIUM_NODE_SIZE),
            self.sync_line(p, MEDIUM_NODE_SIZE),
        );
        let (h, v) = (
            self.synctex.curh / self.synctex.unit,
            self.synctex.curv / self.synctex.unit,
        );
        let len = self.synctex.write(&format!("${tag},{line}:{h},{v}\n"));
        self.synctex.lastv = self.synctex_curv();
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex.count += 1;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_count(&mut self) {
        let len = self
            .synctex
            .write(&format!("Count:{}\n", self.synctex.count));
        if len > 0 {
            self.synctex.total_length += len as i32;
        } else {
            self.synctex_abort();
        }
    }

    fn synctex_record_postamble(&mut self) {
        self.synctex_record_anchor();
        let len = self.synctex.write("Postamble:\n");
        if len > 0 {
            self.synctex.total_length += len as i32;
            self.synctex_record_count();
            self.synctex_record_anchor();
            let len = self.synctex.write("Post scriptum:\n");
            if len > 0 {
                self.synctex.total_length += len as i32;
            } else {
                self.synctex_abort();
            }
        } else {
            self.synctex_abort();
        }
    }

    /// Shared box-geometry readout for the list recorders: scaled
    /// `curh,curv` plus width/height/depth divided by `unit`.
    fn synctex_box_metrics(&self, p: i32) -> (i32, i32, i32, i32, i32) {
        let unit = self.synctex.unit;
        (
            self.synctex.curh / unit,
            self.synctex.curv / unit,
            self.mem[(p + 1) as usize].s1 / unit,
            self.mem[(p + 3) as usize].s1 / unit,
            self.mem[(p + 2) as usize].s1 / unit,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::{FileInputSource, TOKEN_LIST};
    use crate::state::{TeXMode, GLUE_NODE, VLIST_NODE, WHATSIT_NODE};
    use std::collections::VecDeque;

    fn enabled_state() -> TeXEngineState {
        let mut state = TeXEngineState::new(30_000);
        state.synctex_enabled = true;
        state.job_name = Some("job".to_string());
        state.synctex_init_command();
        state
    }

    fn boxed(state: &mut TeXEngineState, tag: i32, line: i32) -> i32 {
        state.cur_input.synctex_tag = tag;
        state.line = line;
        state.new_null_box().unwrap()
    }

    #[test]
    fn get_node_stamps_sync_tag_and_line() {
        let mut state = TeXEngineState::new(30_000);
        state.cur_input.synctex_tag = 7;
        state.line = 42;
        let p = state.get_node(MEDIUM_NODE_SIZE).unwrap();
        assert_eq!(state.mem[(p + 2) as usize].s0, 7);
        assert_eq!(state.mem[(p + 2) as usize].s1, 42);
        let small = state.get_node(2).unwrap();
        assert_eq!(state.mem[(small + 1) as usize].s0, 0);
    }

    #[test]
    fn new_null_box_preserves_stamp() {
        let mut state = TeXEngineState::new(30_000);
        state.cur_input.synctex_tag = 9;
        state.line = 11;
        let p = state.new_null_box().unwrap();
        assert_eq!(state.mem[(p + 7) as usize].s0, 9);
        assert_eq!(state.mem[(p + 7) as usize].s1, 11);
    }

    #[test]
    fn start_input_assigns_tags_and_stashes_root() {
        let mut state = enabled_state();
        state.abspath_of_input_file = "main.tex".to_string();
        state.synctex_start_input();
        assert_eq!(state.cur_input.synctex_tag, 1);
        assert_eq!(state.synctex.root_name.as_deref(), Some("main.tex"));
        assert!(state.synctex.bytes.is_empty());

        state.abspath_of_input_file = "sub.tex".to_string();
        state.synctex_start_input();
        assert_eq!(state.cur_input.synctex_tag, 2);
        // Opening the stream emits the preamble + root + this input.
        let text = String::from_utf8(state.synctex.bytes.clone()).unwrap();
        assert!(text.starts_with("SyncTeX Version:1\n"));
        assert!(text.contains("Input:1:main.tex\n"));
        assert!(text.contains("Input:2:sub.tex\n"));
        assert!(state.synctex.file_open);
    }

    #[test]
    fn root_name_falls_back_to_texput() {
        let mut state = enabled_state();
        state.synctex_start_input();
        assert_eq!(state.synctex.root_name.as_deref(), Some("texput"));
    }

    #[test]
    fn disabled_int_par_skips_open() {
        let mut state = TeXEngineState::new(30_000);
        state.synctex_init_command(); // synctex_enabled = false
        state.synctex_start_input();
        state.synctex_sheet(1000);
        assert!(!state.synctex.file_open);
        assert!(state.synctex.bytes.is_empty());
    }

    #[test]
    fn sheet_and_terminate_emit_full_stream() {
        let mut state = enabled_state();
        state.abspath_of_input_file = "doc.tex".to_string();
        state.synctex_start_input();

        state.synctex_sheet(1200);
        let p = boxed(&mut state, 1, 5);
        state.mem[(p + 1) as usize].s1 = 100_000;
        state.mem[(p + 3) as usize].s1 = 20_000;
        state.mem[(p + 2) as usize].s1 = 5_000;
        state.cur_h = 0;
        state.cur_v = 0;
        state.synctex_vlist(p);
        state.synctex_tsilv(p);
        state.total_pages = 1;
        state.synctex_teehs();
        state.synctex_terminate(true);

        let text = String::from_utf8(state.synctex.bytes.clone()).unwrap();
        assert!(text.starts_with("SyncTeX Version:1\nInput:1:doc.tex\n"));
        assert!(text.contains("Output:pdf\nMagnification:1200\nUnit:1\n"));
        assert!(text.contains("Content:\n"));
        assert!(text.contains("{1\n"));
        assert!(text.contains("[1,5:4736287,4736287:100000,20000,5000\n"));
        assert!(text.contains("]\n"));
        assert!(text.contains("}1\n"));
        assert!(text.contains("Postamble:\n"));
        assert!(text.contains("Count:"));
        assert!(text.ends_with("Post scriptum:\n"));
        assert!(!state.synctex.file_open);
        assert!(state.synctex.off);
    }

    #[test]
    fn kern_defers_until_context_change() {
        let mut state = enabled_state();
        state.synctex_sheet(1000);
        let outer = boxed(&mut state, 1, 1);
        state.synctex_hlist(outer);

        // Two kerns with the same tag/line: only the first is deferred;
        // a tag change flushes it.
        state.cur_input.synctex_tag = 3;
        state.line = 10;
        let k1 = state.new_kern(500).unwrap();
        state.synctex.node = outer;
        state.synctex_kern(k1, outer);
        assert!(
            state.synctex.bytes.ends_with(b"(\n") || {
                let t = String::from_utf8_lossy(&state.synctex.bytes);
                !t.contains("k3,10")
            }
        );
        let k2 = state.new_kern(600).unwrap();
        state.synctex_kern(k2, outer);
        let k3 = {
            state.cur_input.synctex_tag = 4;
            state.new_kern(700).unwrap()
        };
        state.synctex_kern(k3, outer);

        let text = String::from_utf8_lossy(&state.synctex.bytes);
        // The deferred kern flushed when the context changed to tag 4.
        assert!(text.contains("k3,10:"), "{text}");
    }

    #[test]
    fn horizontal_rule_glue_and_math_records() {
        let mut state = enabled_state();
        state.synctex_sheet(1000);

        state.cur_input.synctex_tag = 2;
        state.line = 8;
        let rule = state.new_rule().unwrap();
        state.synctex.rule_wd = 10_000;
        state.synctex.rule_ht = 40_000;
        state.synctex.rule_dp = 3_000;
        state.synctex_horizontal_rule_or_glue(rule, 0);

        let glue = state.new_null_box().unwrap(); // reuse stamp machinery
        state.set_node_type(glue, GLUE_NODE).unwrap();
        // glue node sync word is at +2
        let spec = state.new_spec(0).unwrap_or(0);
        let _ = spec;
        let g = {
            state.cur_input.synctex_tag = 2;
            state.line = 9;
            state.new_glue(0).unwrap()
        };
        state.synctex_horizontal_rule_or_glue(g, 0);

        let m = {
            state.cur_input.synctex_tag = 2;
            state.line = 10;
            state.new_math(0, 0).unwrap()
        };
        state.synctex_math(m, 0);
        state.synctex_current();

        let text = String::from_utf8_lossy(&state.synctex.bytes);
        assert!(text.contains("r2,8:"), "{text}");
        assert!(text.contains(":10000,40000,3000\n"), "{text}");
        assert!(text.contains("g2,9:"), "{text}");
        assert!(text.contains("$2,10:"), "{text}");
        assert!(text.contains("x2,10:"), "{text}");
        let _ = glue;
    }

    #[test]
    fn copy_node_list_carries_sync_fields() {
        let mut state = TeXEngineState::new(30_000);
        state.cur_input.synctex_tag = 4;
        state.line = 20;
        let b = state.new_null_box().unwrap();
        state.set_node_type(b, VLIST_NODE).unwrap();
        // Stamp a different context after creation, then copy.
        state.cur_input.synctex_tag = 8;
        state.line = 30;
        let copy = state.copy_node_list(b).unwrap();
        assert_eq!(state.mem[(copy + 7) as usize].s0, 4);
        assert_eq!(state.mem[(copy + 7) as usize].s1, 20);
    }

    fn with_open_source(name: &str) -> TeXEngineState {
        let mut state = TeXEngineState::new(30_000);
        state
            .file_sources
            .push(FileInputSource::new(name, 1, VecDeque::new()));
        state.in_open = 1;
        state
    }

    #[test]
    fn compare_paths_matches_dir_seps() {
        assert_eq!(compare_paths(b"a/b/c.tex", b"a/b/c.tex"), 0);
        assert_ne!(compare_paths(b"a/b/c.tex", b"a/b/d.tex"), 0);
        assert_eq!(compare_paths(b"/x", b"/x"), 0);
        assert_ne!(compare_paths(b"a", b"ab"), 0);
    }

    #[test]
    fn insert_src_special_pushes_special_token_list() {
        let mut state = with_open_source("doc.tex");
        state.line = 5;
        state.insert_src_special().unwrap();
        // An INSERTED token-list frame was pushed; its list starts with
        // `\special` (frozen CS) then `{`.
        assert_eq!(state.cur_input.state, TOKEN_LIST);
        let head = state.cur_input.loc;
        assert_eq!(
            state.mem[head as usize].s0,
            CS_TOKEN_FLAG + C_FROZEN_SPECIAL
        );
        let brace = state.mem[head as usize].s1;
        assert_eq!(
            state.mem[brace as usize].s0,
            LEFT_BRACE_TOKEN + i32::from(b'{')
        );

        // Same source position: a second call inserts nothing.
        let ptr = state.input_ptr;
        state.insert_src_special().unwrap();
        assert_eq!(state.input_ptr, ptr);
    }

    #[test]
    fn append_src_special_appends_whatsit() {
        let mut state = with_open_source("dir/doc.tex");
        state.push_nest().unwrap();
        state.mode = TeXMode::Hmode as i16;
        state.line = 12;
        state.append_src_special().unwrap();
        let tail = state.tail;
        assert_eq!(state.node_type(tail).unwrap(), WHATSIT_NODE);
        assert_eq!(state.node_subtype(tail).unwrap(), SPECIAL_NODE);
        let def_ref = state.mem[(tail + 1) as usize].s1;
        assert!(def_ref > 0);
        // The def_ref chain holds "src:12 dir/doc.tex" as OTHER/SPACE tokens.
        let mut text = String::new();
        let mut p = state.mem[def_ref as usize].s1;
        while p != TEX_NULL {
            let tok = state.mem[p as usize].s0;
            text.push(char::from_u32((tok % MAX_CHAR_VAL) as u32).unwrap());
            p = state.mem[p as usize].s1;
        }
        assert_eq!(text, "src:12 dir/doc.tex");
        assert_eq!(state.src_last_name, "dir/doc.tex");
        assert_eq!(state.src_last_lineno, 12);
    }
}
