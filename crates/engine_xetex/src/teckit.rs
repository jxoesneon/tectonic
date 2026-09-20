//! Pure-Rust port of the TECkit compiled-mapping engine (`teckit-Engine.cpp`)
//! for the complete-input case XeTeX uses: `TECkit_ConvertBuffer` is always
//! invoked with the entire buffer and `inputIsComplete`, so the streaming
//! ring-buffer and saved-byte machinery collapse into flat slice indexing
//! (lookbehind never exceeds `maxPre`, which the compiled buffer sizes
//! guarantee).
//!
//! Tables are big-endian (`READ` swaps on little-endian hosts). The NFC/NFD
//! normalizer stages are provided by `unicode_normalization`, which implements
//! the same Unicode normalization the vendored `Normalizer` stage performs.

use unicode_normalization::{is_nfc, is_nfd, UnicodeNormalization};

const K_MAGIC_NUMBER: u32 = 0x714d_6170; // 'qMap'
const K_MAGIC_NUMBER_CMP: u32 = 0x7a51_6d70; // 'zQmp'
const K_CURRENT_FILE_VERSION: u32 = 0x0003_0000;

const K_TABLE_TYPE_BB: u32 = 0x422d_3e42; // 'B->B'
const K_TABLE_TYPE_BU: u32 = 0x422d_3e55; // 'B->U'
const K_TABLE_TYPE_UB: u32 = 0x552d_3e42; // 'U->B'
const K_TABLE_TYPE_UU: u32 = 0x552d_3e55; // 'U->U'
const K_TABLE_TYPE_NFC: u32 = 0x4e46_4320; // 'NFC '
const K_TABLE_TYPE_NFD: u32 = 0x4e46_4420; // 'NFD '

const K_TABLE_FLAGS_SUPPLEMENTARY: u32 = 0x0001;

const K_LOOKUP_TYPE_STRING_RULES: u8 = 0xff;
const K_LOOKUP_TYPE_ILLEGAL_DBCS: u8 = 0xfe;
const K_LOOKUP_TYPE_UNMAPPED: u8 = 0xfd;
const K_LOOKUP_TYPE_RULE_TYPE_MASK: u8 = 0xc0;
const K_LOOKUP_TYPE_EXT_STRING_RULES: u8 = 0x80;
const K_LOOKUP_TYPE_EXT_RULE_COUNT_MASK: u8 = 0x3f;

const K_MATCH_ELEM_NEGATE: u8 = 0x80;
const K_MATCH_ELEM_NON_LIT: u8 = 0x40;
const K_MATCH_ELEM_TYPE_MASK: u8 = 0x3f;
const K_MATCH_ELEM_TYPE_CLASS: u8 = 0x01;
const K_MATCH_ELEM_TYPE_BGROUP: u8 = 0x02;
const K_MATCH_ELEM_TYPE_EGROUP: u8 = 0x03;
const K_MATCH_ELEM_TYPE_OR: u8 = 0x04;
const K_MATCH_ELEM_TYPE_ANY: u8 = 0x05;
const K_MATCH_ELEM_TYPE_EOS: u8 = 0x06;
const K_MATCH_ELEM_TYPE_COPY: u8 = 0x07;

const K_REP_ELEM_LITERAL: u8 = 0x00;
const K_REP_ELEM_CLASS: u8 = K_MATCH_ELEM_TYPE_CLASS;
const K_REP_ELEM_COPY: u8 = K_MATCH_ELEM_TYPE_COPY;
const K_REP_ELEM_UNMAPPED: u8 = 0x0f;

const K_USV_MASK: u32 = 0x001f_ffff;

const K_FLAGS_EXPECTS_NFC: u32 = 0x0000_0001;
const K_FLAGS_EXPECTS_NFD: u32 = 0x0000_0002;
const K_FLAGS_UNICODE: u32 = 0x0001_0000;

const K_END_OF_TEXT: u32 = 0xffff_fffe;
const K_UNMAPPED_CHAR: u32 = 0xffff_fffd;

const MATCH_YES: u32 = 1;
const MATCH_NO: u32 = 0;

/// A decoded `.tec` compiled mapping (`TECkit_Converter`): a chain of
/// normalization and mapping-table passes applied over the complete input.
#[derive(Clone, Debug)]
pub struct Converter {
    /// Whether the converter emits bytes (`kForm_Bytes` output — TFM
    /// `byteMapping` mode).
    byte_output: bool,
    /// Processing pipeline in application order.
    stages: Vec<Stage>,
}

#[derive(Clone, Debug)]
enum Stage {
    /// A compiled match/replace table (`Pass`).
    Pass(Pass),
    /// Unicode normalization stage (`Normalizer`): `true` = NFC (compose),
    /// `false` = NFD (decompose).
    Normalize(bool),
}

/// A compiled mapping table (`Pass`): the lookup pages, classes, and string
/// rules of one `TableHeader` block, kept as raw big-endian bytes plus the
/// decoded header offsets like C.
#[derive(Clone, Debug)]
struct Pass {
    table: Vec<u8>,
    input_is_unicode: bool,
    output_is_unicode: bool,
    supplementary: bool,
    has_dbcs_page: bool,
    /// `numPageMaps` for supplementary-plane lookups (1 otherwise).
    num_page_maps: usize,
    /// Byte offset of the page table (after the 20-byte plane map for
    /// supplementary tables).
    page_base: usize,
    lookup_base: usize,
    match_class_base: usize,
    rep_class_base: usize,
    string_list_base: usize,
    string_rule_data: usize,
    replacement_char: u32,
}

/// Per-element match bookkeeping (`MatchInfo`): the input span an element
/// consumed, the class member index it matched, and group repeat count.
#[derive(Clone, Copy, Default, Debug)]
struct MatchInfo {
    start: i32,
    limit: i32,
    class_index: i32,
    group_repeats: i32,
}

fn be_u16(t: &[u8], off: usize) -> u16 {
    u16::from_be_bytes([t[off], t[off + 1]])
}

fn be_u32(t: &[u8], off: usize) -> u32 {
    u32::from_be_bytes([t[off], t[off + 1], t[off + 2], t[off + 3]])
}

impl Converter {
    /// `TECkit_CreateConverter` for the two XeTeX call sites: `byte_mapping`
    /// selects the reverse pipeline with `kForm_Bytes` output (TFM font
    /// mappings); otherwise the forward `UTF16 -> UTF16` pipeline is used.
    /// Returns `None` when the table is invalid (`kStatus_InvalidMapping`
    /// and friends all collapse to "not loadable").
    pub fn parse(data: &[u8], byte_mapping: bool) -> Option<Self> {
        let table = decompress_table(data)?;
        if table.len() < 32 || be_u32(&table, 0) != K_MAGIC_NUMBER {
            return None;
        }
        if be_u32(&table, 4) & 0xFFFF_0000 > K_CURRENT_FILE_VERSION & 0xFFFF_0000 {
            return None;
        }
        let form_flags_lhs = be_u32(&table, 12);
        let form_flags_rhs = be_u32(&table, 16);
        let num_names = be_u32(&table, 20) as usize;
        let num_fwd = be_u32(&table, 24) as usize;
        let num_rev = be_u32(&table, 28) as usize;

        let forward = !byte_mapping;
        let target_flags = if forward {
            form_flags_rhs
        } else {
            form_flags_lhs
        };
        let source_flags = if forward {
            form_flags_lhs
        } else {
            form_flags_rhs
        };

        // C validates the requested input/output forms against the mapping's
        // flags; our call sites always use Unicode input and Unicode/byte
        // output, which the flag checks below encode.
        if target_flags & K_FLAGS_UNICODE == 0 && !byte_mapping {
            return None;
        }
        if source_flags & K_FLAGS_UNICODE == 0 {
            return None; // callers always feed Unicode input
        }

        let table_base = 32 + 4 * num_names + if forward { 0 } else { 4 * num_fwd };
        let num_tables = if forward { num_fwd } else { num_rev };

        let mut stages: Vec<Stage> = Vec::new();
        let mut norm_form: Option<bool> = None;
        if source_flags & K_FLAGS_EXPECTS_NFD != 0 {
            stages.push(Stage::Normalize(false));
            norm_form = Some(false);
        } else if source_flags & K_FLAGS_EXPECTS_NFC != 0 {
            stages.push(Stage::Normalize(true));
            norm_form = Some(true);
        }

        for i in 0..num_tables {
            let off = be_u32(&table, table_base + 4 * i) as usize;
            // NFC/NFD entries are only the 4-byte type tag; mapping tables
            // need the full 48-byte header (checked in `Pass::parse`).
            let t = table.get(off..).filter(|s| s.len() >= 4)?;
            match be_u32(t, 0) {
                K_TABLE_TYPE_BB | K_TABLE_TYPE_BU | K_TABLE_TYPE_UU | K_TABLE_TYPE_UB => {
                    stages.push(Stage::Pass(Pass::parse(t)?));
                    norm_form = None;
                }
                K_TABLE_TYPE_NFC => {
                    stages.push(Stage::Normalize(true));
                    norm_form = Some(true);
                }
                K_TABLE_TYPE_NFD => {
                    stages.push(Stage::Normalize(false));
                    norm_form = Some(false);
                }
                _ => return None,
            }
        }
        let _ = norm_form; // outForm normalization is never requested by our callers
        Some(Self {
            byte_output: byte_mapping,
            stages,
        })
    }

    /// `TECkit_ConvertBuffer` for UTF-16 text input (`apply_mapping`):
    /// decodes the units to scalars, runs the pipeline, and re-encodes to
    /// UTF-16. A conversion failure yields an empty vector, matching C's
    /// `apply_mapping` returning 0 on non-`NoError` status.
    pub fn convert_text(&self, input: &[u16]) -> Vec<u16> {
        let Some(scalars) = decode_utf16(input) else {
            return Vec::new();
        };
        let out = self.convert_scalars(&scalars);
        encode_utf16(&out)
    }

    /// `TECkit_ConvertBuffer` for a single byte-mapped character
    /// (`apply_tfm_font_mapping`): returns the first output byte, or 0 when
    /// the mapping produces nothing.
    pub fn convert_byte(&self, c: u16) -> u16 {
        let out = self.convert_scalars(&[u32::from(c)]);
        out.first().map_or(0, |&v| v as u16 & 0xff)
    }

    /// Runs the stage pipeline over scalar input, returning the final
    /// stage's output scalars (byte-output stages produce values ≤ 0xFF).
    fn convert_scalars(&self, input: &[u32]) -> Vec<u32> {
        let mut stream = input.to_vec();
        for stage in &self.stages {
            stream = match stage {
                Stage::Pass(pass) => pass.convert(&stream),
                Stage::Normalize(true) => normalize(&stream, true),
                Stage::Normalize(false) => normalize(&stream, false),
            };
        }
        stream
    }

    /// Whether the converter was built for byte output (`byteMapping`).
    pub fn byte_mapping(&self) -> bool {
        self.byte_output
    }
}

/// Decompresses a `zQmp` table via the flate bridge; uncompressed tables
/// pass through cloned.
fn decompress_table(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() >= 8 && be_u32(data, 0) == K_MAGIC_NUMBER_CMP {
        let len = be_u32(data, 4) as usize;
        let mut out = vec![0u8; len];
        let mut decomp = tectonic_bridge_flate::flate2::Decompress::new(true);
        let status = decomp
            .decompress(
                &data[8..],
                &mut out,
                tectonic_bridge_flate::flate2::FlushDecompress::Finish,
            )
            .ok()?;
        if status != tectonic_bridge_flate::flate2::Status::StreamEnd
            || decomp.total_out() as usize != len
        {
            return None;
        }
        Some(out)
    } else {
        Some(data.to_vec())
    }
}

/// Decodes UTF-16 units to scalars; a lone or truncated surrogate fails the
/// whole input (`kInvalidChar` under complete input).
fn decode_utf16(input: &[u16]) -> Option<Vec<u32>> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let u = u32::from(input[i]);
        if (0xD800..=0xDBFF).contains(&u) {
            if i + 1 >= input.len() {
                return None;
            }
            let low = u32::from(input[i + 1]);
            if !(0xDC00..=0xDFFF).contains(&low) {
                return None;
            }
            out.push(((u - 0xD800) << 10) + (low - 0xDC00) + 0x10000);
            i += 2;
        } else if (0xDC00..=0xDFFF).contains(&u) {
            return None;
        } else {
            out.push(u);
            i += 1;
        }
    }
    Some(out)
}

/// Encodes scalars to UTF-16 units; out-of-range values are dropped like
/// C's `usv <= 0x0010ffff` guard.
fn encode_utf16(input: &[u32]) -> Vec<u16> {
    let mut out = Vec::with_capacity(input.len());
    for &c in input {
        if c <= 0xFFFF {
            out.push(c as u16);
        } else if c <= 0x10FFFF {
            let v = c - 0x10000;
            out.push((0xD800 + (v >> 10)) as u16);
            out.push((0xDC00 + (v & 0x3FF)) as u16);
        }
    }
    out
}

/// Unicode normalization via the standard-conformant implementation
/// (`Normalizer` stage).
fn normalize(input: &[u32], compose: bool) -> Vec<u32> {
    let text: String = input.iter().filter_map(|&c| char::from_u32(c)).collect();
    if compose && is_nfc(&text) || !compose && is_nfd(&text) {
        return input.to_vec();
    }
    let normalized: String = if compose {
        text.chars().nfc().collect()
    } else {
        text.chars().nfd().collect()
    };
    normalized.chars().map(|c| c as u32).collect()
}

impl Pass {
    /// `Pass::Pass`: decodes the `TableHeader` and locates the sub-tables.
    fn parse(t: &[u8]) -> Option<Self> {
        if t.len() < 48 {
            return None;
        }
        let ty = be_u32(t, 0);
        let flags = be_u32(t, 12);
        let mut page_base = be_u32(t, 16) as usize;
        let lookup_base = be_u32(t, 20) as usize;
        let match_class_base = be_u32(t, 24) as usize;
        let rep_class_base = be_u32(t, 28) as usize;
        let string_list_base = be_u32(t, 32) as usize;
        let string_rule_data = be_u32(t, 36) as usize;
        // Bytes 40-43 are maxMatch/maxPre/maxPost/maxOutput; the
        // replacement character follows at 44.
        let replacement_char = be_u32(t, 44);

        let input_is_unicode = (ty >> 24) == u32::from(b'U');
        let output_is_unicode = (ty & 0xFF) == u32::from(b'U');
        let supplementary = flags & K_TABLE_FLAGS_SUPPLEMENTARY != 0;

        let mut num_page_maps = 1usize;
        if input_is_unicode && supplementary {
            // The 20-byte plane map precedes the page table.
            if t.len() < page_base + 20 {
                return None;
            }
            num_page_maps = t[page_base + 17] as usize;
            page_base += 20;
        }
        // A byte pass has a DBCS lead-byte page when pageBase is nonzero.
        let has_dbcs_page = !input_is_unicode && page_base != 0;

        Some(Self {
            table: t.to_vec(),
            input_is_unicode,
            output_is_unicode,
            supplementary,
            has_dbcs_page,
            num_page_maps,
            page_base,
            lookup_base,
            match_class_base,
            rep_class_base,
            string_list_base,
            string_rule_data,
            replacement_char,
        })
    }

    /// `Pass::getChar`/`DoMapping` driven to end of input for complete
    /// input: repeatedly maps the character at `pos` until end of text.
    fn convert(&self, input: &[u32]) -> Vec<u32> {
        let mut out = Vec::with_capacity(input.len());
        let mut pos = 0usize;
        while pos < input.len() {
            match self.do_mapping(input, pos, &mut out) {
                Some(consumed) => pos += consumed.max(1),
                // A failed rule path consumes one char to avoid wedging,
                // matching C's advanceInput(matchedLength) where the default
                // is 1.
                None => pos += 1,
            }
        }
        out
    }

    /// `Pass::inputChar`: the character at `pos + rel`, or `K_END_OF_TEXT`
    /// outside the stream.
    #[inline]
    fn input_char(&self, input: &[u32], pos: usize, rel: i32) -> u32 {
        let idx = pos as i64 + i64::from(rel);
        if idx < 0 || idx >= input.len() as i64 {
            K_END_OF_TEXT
        } else {
            input[idx as usize]
        }
    }

    /// `Pass::classMatch`: binary-search member lookup in a match class;
    /// element width follows the input form (8/16/32-bit).
    fn class_match(&self, class_number: u32, in_char: u32) -> i32 {
        let base = self.match_class_base
            + be_u32(
                &self.table,
                self.match_class_base + 4 * class_number as usize,
            ) as usize;
        let Some(t) = self.table.get(base..) else {
            return -1;
        };
        if t.len() < 4 {
            return -1;
        }
        let count = be_u32(t, 0) as usize;
        let members = &t[4..];
        let idx = if self.input_is_unicode {
            if self.supplementary {
                binary_search_u32(members, count, in_char)
            } else {
                binary_search_u16(members, count, in_char)
            }
        } else {
            binary_search_u8(members, count, in_char)
        };
        idx.map_or(-1, |i| i as i32)
    }

    /// `Pass::repClassMember`: member `index` of a replacement class.
    fn rep_class_member(&self, class_number: u32, index: i32) -> u32 {
        if index < 0 {
            return 0;
        }
        let base = self.rep_class_base
            + be_u32(&self.table, self.rep_class_base + 4 * class_number as usize) as usize;
        let Some(t) = self.table.get(base..) else {
            return 0;
        };
        if t.len() < 4 {
            return 0;
        }
        let count = be_u32(t, 0);
        let members = &t[4..];
        let index = index as usize;
        if index >= count as usize {
            return 0;
        }
        if self.output_is_unicode {
            if self.supplementary {
                members
                    .get(4 * index..4 * index + 4)
                    .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
                    .unwrap_or(0)
            } else {
                members
                    .get(2 * index..2 * index + 2)
                    .map(|b| u32::from(u16::from_be_bytes([b[0], b[1]])))
                    .unwrap_or(0)
            }
        } else {
            members.get(index).map(|&b| u32::from(b)).unwrap_or(0)
        }
    }

    /// `Pass::match`: recursive backtracking matcher. `elem_base` is the
    /// element offset of the current pattern within the rule data,
    /// `index` the element number inside it, `repeats` the current
    /// repetition count, `text_loc` the input offset relative to `pos`,
    /// `direction` `1` for match/post-context or `-1` for pre-context, and
    /// `matched_length` records where the match portion ended
    /// (`index == match_elems`).
    #[allow(clippy::too_many_arguments)]
    fn match_rule(
        &self,
        input: &[u32],
        pos: usize,
        elem_base: usize,
        index: i32,
        mut repeats: i32,
        mut text_loc: i32,
        direction: i32,
        info: &mut [MatchInfo; 256],
        info_limit: i32,
        match_elems: i32,
        pattern_len: i32,
        matched_length: &mut i32,
    ) -> u32 {
        let mut index = index;
        'restart: loop {
            if repeats == 0 {
                if index == match_elems {
                    *matched_length = text_loc;
                }
                if (0..info_limit).contains(&index) {
                    info[index as usize].start = text_loc;
                }
            }
            if index >= pattern_len {
                return MATCH_YES;
            }

            let elem = elem_base + index as usize * 8;
            let Some(m) = self.table.get(elem..elem + 8) else {
                return MATCH_NO;
            };
            let repeat_min = i32::from(m[0] >> 4);
            let repeat_max = i32::from(m[0] & 0x0f);
            let raw_type = m[1];
            let negate = raw_type & K_MATCH_ELEM_NEGATE != 0;
            let ty = if raw_type & K_MATCH_ELEM_NON_LIT != 0 {
                raw_type & K_MATCH_ELEM_TYPE_MASK
            } else {
                0
            };

            if ty == K_MATCH_ELEM_TYPE_BGROUP {
                if let Some(inf) = info.get_mut(index as usize) {
                    inf.group_repeats = repeats;
                }
                if repeats < repeat_max {
                    let mut alt_index = index;
                    loop {
                        let mr = self.match_rule(
                            input,
                            pos,
                            elem_base,
                            alt_index + 1,
                            0,
                            text_loc,
                            direction,
                            info,
                            info_limit,
                            match_elems,
                            pattern_len,
                            matched_length,
                        );
                        if mr != MATCH_NO {
                            return mr;
                        }
                        alt_index += i32::from(self.table[elem_base + alt_index as usize * 8 + 6]);
                        let next_type = self.table[elem_base + alt_index as usize * 8 + 1];
                        if next_type & K_MATCH_ELEM_TYPE_MASK != K_MATCH_ELEM_TYPE_OR {
                            break;
                        }
                    }
                }
                if repeats >= repeat_min {
                    let d_after = i32::from(m[7]);
                    let mr = self.match_rule(
                        input,
                        pos,
                        elem_base,
                        index + d_after,
                        0,
                        text_loc,
                        direction,
                        info,
                        info_limit,
                        match_elems,
                        pattern_len,
                        matched_length,
                    );
                    if mr == MATCH_YES && (0..info_limit).contains(&index) {
                        info[index as usize].limit = text_loc;
                        for i in (index + 1..index + d_after).rev() {
                            if i < info_limit {
                                let inf = &mut info[i as usize];
                                if inf.start > text_loc {
                                    inf.start = text_loc;
                                }
                                if inf.limit > text_loc {
                                    inf.limit = text_loc;
                                }
                            }
                        }
                    }
                    if mr == MATCH_NO && (0..info_limit).contains(&index) {
                        info[index as usize].limit = text_loc;
                    }
                    return mr;
                }
                if (0..info_limit).contains(&index) {
                    info[index as usize].limit = text_loc;
                }
                return MATCH_NO;
            }

            if ty == K_MATCH_ELEM_TYPE_OR || ty == K_MATCH_ELEM_TYPE_EGROUP {
                let d_start = i32::from(m[7]);
                let start_index = index - d_start;
                let repeats_next = info
                    .get(start_index as usize)
                    .map_or(0, |i| i.group_repeats)
                    + 1;
                let mr = self.match_rule(
                    input,
                    pos,
                    elem_base,
                    start_index,
                    repeats_next,
                    text_loc,
                    direction,
                    info,
                    info_limit,
                    match_elems,
                    pattern_len,
                    matched_length,
                );
                if mr == MATCH_NO && (0..info_limit).contains(&index) {
                    info[index as usize].limit = text_loc;
                }
                return mr;
            }

            // Non-group element: match at least repeat_min times.
            // C does not early-out on kEndOfText here: EOS elements must be
            // able to match it (kInvalidChar/kUnmappedChar/kNeedMoreInput
            // cannot occur for complete, pre-decoded input).
            while repeats < repeat_min {
                let in_char = self.input_char(input, pos, text_loc);
                let mut matches = match ty {
                    0 => (be_u32(&self.table, elem + 4) & K_USV_MASK) == in_char,
                    K_MATCH_ELEM_TYPE_CLASS => {
                        let idx =
                            self.class_match(u32::from(be_u16(&self.table, elem + 6)), in_char);
                        if idx != -1 && repeats == 0 && (0..info_limit).contains(&index) {
                            info[index as usize].class_index = idx;
                        }
                        idx != -1
                    }
                    K_MATCH_ELEM_TYPE_ANY => in_char != K_END_OF_TEXT,
                    K_MATCH_ELEM_TYPE_EOS => in_char == K_END_OF_TEXT,
                    _ => false,
                };
                matches = matches != negate;
                if !matches {
                    if (0..info_limit).contains(&index) {
                        info[index as usize].limit = text_loc;
                    }
                    return MATCH_NO;
                }
                repeats += 1;
                text_loc += direction;
            }

            if (0..info_limit).contains(&index) {
                info[index as usize].limit = text_loc;
            }

            if repeat_min == repeat_max {
                index += 1;
                repeats = 0;
                continue 'restart;
            }

            // Optional further repeats: recurse on success.
            if repeats < repeat_max {
                let in_char = self.input_char(input, pos, text_loc);
                let mut matches = match ty {
                    0 => (be_u32(&self.table, elem + 4) & K_USV_MASK) == in_char,
                    K_MATCH_ELEM_TYPE_CLASS => {
                        let idx =
                            self.class_match(u32::from(be_u16(&self.table, elem + 6)), in_char);
                        if idx != -1 && repeats == 0 && (0..info_limit).contains(&index) {
                            info[index as usize].class_index = idx;
                        }
                        idx != -1
                    }
                    K_MATCH_ELEM_TYPE_ANY => in_char != K_END_OF_TEXT,
                    K_MATCH_ELEM_TYPE_EOS => in_char == K_END_OF_TEXT,
                    _ => false,
                };
                matches = matches != negate;
                if matches {
                    let mr = self.match_rule(
                        input,
                        pos,
                        elem_base,
                        index,
                        repeats + 1,
                        text_loc + direction,
                        direction,
                        info,
                        info_limit,
                        match_elems,
                        pattern_len,
                        matched_length,
                    );
                    if mr != MATCH_NO {
                        return mr;
                    }
                }
            }

            let mr = self.match_rule(
                input,
                pos,
                elem_base,
                index + 1,
                0,
                text_loc,
                direction,
                info,
                info_limit,
                match_elems,
                pattern_len,
                matched_length,
            );
            if mr == MATCH_NO && (0..info_limit).contains(&index) {
                info[index as usize].limit = text_loc;
            }
            return mr;
        }
    }

    /// `Pass::DoMapping`: maps the character at `pos`, appending outputs to
    /// `out`. Returns the number of input scalars consumed, or `None` when
    /// the pass declares the input unmapped and the replacement behavior
    /// fails (`kUnmappedChar` — only reachable for cross-form tables under
    /// `DontUseReplacementChar`, which our callers never request).
    fn do_mapping(&self, input: &[u32], pos: usize, out: &mut Vec<u32>) -> Option<usize> {
        let in_char = input[pos];
        let mut matched_length = 1usize;

        let char_index = if self.input_is_unicode {
            let mut char_index = 0usize;
            if self.lookup_base != self.page_base {
                let plane = (in_char >> 16) as usize;
                let page_map = if self.supplementary {
                    if plane < 17
                        && self
                            .table
                            .get(self.page_base - 20 + plane)
                            .copied()
                            .unwrap_or(0xff)
                            != 0xff
                    {
                        Some(
                            self.page_base
                                + 256 * usize::from(self.table[self.page_base - 20 + plane]),
                        )
                    } else {
                        None
                    }
                } else if plane == 0 {
                    Some(self.page_base)
                } else {
                    None
                };
                if let Some(page_map) = page_map {
                    let page = ((in_char >> 8) & 0xff) as usize;
                    if let Some(&pm) = self.table.get(page_map + page) {
                        if pm != 0xff {
                            let char_map_base = self.page_base + 256 * self.num_page_maps;
                            let char_map = char_map_base + 256 * usize::from(pm);
                            char_index = usize::from(be_u16(
                                &self.table,
                                char_map + 2 * (in_char as usize & 0xff),
                            ));
                        }
                    }
                }
            }
            char_index
        } else {
            // Byte-oriented input with optional DBCS lead-byte page.
            let in_char = in_char as usize;
            if self.has_dbcs_page {
                let page_number = usize::from(*self.table.get(self.page_base + in_char)?);
                if page_number == 0 {
                    in_char
                } else {
                    let next_char = self.input_char(input, pos, 1);
                    if next_char == K_END_OF_TEXT || next_char == K_UNMAPPED_CHAR {
                        in_char
                    } else {
                        let lookup = page_number * 256 + next_char as usize;
                        if self.table.get(self.lookup_base + 4 * lookup).copied()
                            == Some(K_LOOKUP_TYPE_ILLEGAL_DBCS)
                        {
                            in_char
                        } else {
                            matched_length = 2;
                            lookup
                        }
                    }
                }
            } else {
                in_char
            }
        };

        let lookup = self.lookup_base + 4 * char_index;
        let rule_type = *self.table.get(lookup)?;
        let output_unmapped = |out: &mut Vec<u32>| {
            if self.output_is_unicode == self.input_is_unicode {
                out.push(in_char);
            } else {
                // Callers use silent replacement (`UseReplacementCharSilently`).
                out.push(self.replacement_char);
            }
            Some(1usize)
        };

        if rule_type == K_LOOKUP_TYPE_STRING_RULES
            || rule_type & K_LOOKUP_TYPE_RULE_TYPE_MASK == K_LOOKUP_TYPE_EXT_STRING_RULES
        {
            let mut rule_count = i32::from(*self.table.get(lookup + 1)?);
            if rule_type & K_LOOKUP_TYPE_RULE_TYPE_MASK == K_LOOKUP_TYPE_EXT_STRING_RULES {
                rule_count += 256 * i32::from(rule_type & K_LOOKUP_TYPE_EXT_RULE_COUNT_MASK);
            }
            let rule_index = usize::from(be_u16(&self.table, lookup + 2));
            let mut matched = false;
            let mut allow_insertion = true;
            let mut ml = matched_length as i32;
            for r in 0..rule_count {
                let rule_off = be_u32(
                    &self.table,
                    self.string_list_base + 4 * (rule_index + r as usize),
                ) as usize;
                let rule = self.string_rule_data + rule_off;
                let match_len = i32::from(*self.table.get(rule)?);
                if match_len == 0 && !allow_insertion {
                    continue;
                }
                let post_len = i32::from(self.table[rule + 1]);
                let pre_len = i32::from(self.table[rule + 2]);
                let rep_len = i32::from(self.table[rule + 3]);
                let pattern_len = match_len + post_len;

                let mut info = [MatchInfo::default(); 256];
                let mut mr = self.match_rule(
                    input,
                    pos,
                    rule + 4,
                    0,
                    0,
                    0,
                    1,
                    &mut info,
                    match_len,
                    match_len,
                    pattern_len,
                    &mut ml,
                );
                if mr == MATCH_YES {
                    if ml == 0 && !allow_insertion {
                        continue;
                    }
                    if pre_len > 0 {
                        // Pre-context: pattern pointer advances past
                        // match+post elements; patternLength resets to
                        // preLength and the match runs backwards.
                        mr = self.match_rule(
                            input,
                            pos,
                            rule + 4 + pattern_len as usize * 8,
                            0,
                            0,
                            -1,
                            -1,
                            &mut info,
                            0,
                            -1,
                            pre_len,
                            &mut ml,
                        );
                    }
                    if mr == MATCH_YES {
                        // Execute the replacement elements.
                        let rep_base = rule + 4 + (pattern_len + pre_len) as usize * 8;
                        for i in 0..rep_len {
                            let re = rep_base + i as usize * 4;
                            let ty = *self.table.get(re)?;
                            match ty {
                                K_REP_ELEM_LITERAL => {
                                    out.push(be_u32(&self.table, re) & K_USV_MASK);
                                }
                                K_REP_ELEM_CLASS => {
                                    let match_index = usize::from(self.table[re + 1]);
                                    let rep_class = be_u16(&self.table, re + 2);
                                    let inf = info.get(match_index).copied().unwrap_or_default();
                                    if inf.start < inf.limit {
                                        out.push(self.rep_class_member(
                                            u32::from(rep_class),
                                            inf.class_index,
                                        ));
                                    }
                                }
                                K_REP_ELEM_COPY => {
                                    let match_index = usize::from(self.table[re + 1]);
                                    let inf = info.get(match_index).copied().unwrap_or_default();
                                    for j in inf.start..inf.limit {
                                        let c = self.input_char(input, pos, j);
                                        if c != K_END_OF_TEXT {
                                            out.push(c);
                                        }
                                    }
                                }
                                K_REP_ELEM_UNMAPPED => {
                                    if self.output_is_unicode == self.input_is_unicode {
                                        out.push(in_char);
                                    } else {
                                        out.push(self.replacement_char);
                                    }
                                }
                                _ => {}
                            }
                        }
                        if ml > 0 {
                            matched = true;
                            matched_length = ml as usize;
                            break;
                        } else {
                            allow_insertion = false;
                        }
                    } else if mr != MATCH_NO {
                        return None;
                    }
                } else if mr != MATCH_NO {
                    return None;
                }
            }
            if !matched {
                // `!matched`: C resets `matchedLength` to 1 even for DBCS.
                return output_unmapped(out);
            }
        } else if rule_type == K_LOOKUP_TYPE_UNMAPPED {
            // `kLookupType_Unmapped`: C keeps `matchedLength` (2 for a DBCS
            // pair), so push the unmapped output but return the real span.
            if self.output_is_unicode == self.input_is_unicode {
                out.push(in_char);
            } else {
                out.push(self.replacement_char);
            }
            return Some(matched_length);
        } else {
            // Direct lookup: Unicode scalar or byte payload.
            if self.output_is_unicode {
                let usv = be_u32(&self.table, lookup);
                if usv <= 0x0010_FFFF {
                    out.push(usv);
                }
            } else {
                let count = usize::from(*self.table.get(lookup)?);
                for i in 0..count.min(3) {
                    out.push(u32::from(self.table[lookup + 1 + i]));
                }
            }
        }
        Some(matched_length)
    }
}

fn binary_search_u8(members: &[u8], count: usize, value: u32) -> Option<usize> {
    let mut lo = 0usize;
    let mut hi = count;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let v = u32::from(*members.get(mid)?);
        if v < value {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    members
        .get(lo)
        .filter(|&&v| u32::from(v) == value)
        .map(|_| lo)
}

fn binary_search_u16(members: &[u8], count: usize, value: u32) -> Option<usize> {
    let mut lo = 0usize;
    let mut hi = count;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let v = u32::from(u16::from_be_bytes([
            *members.get(2 * mid)?,
            *members.get(2 * mid + 1)?,
        ]));
        if v < value {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    members
        .get(2 * lo..2 * lo + 2)
        .filter(|b| u32::from(u16::from_be_bytes([b[0], b[1]])) == value)
        .map(|_| lo)
}

fn binary_search_u32(members: &[u8], count: usize, value: u32) -> Option<usize> {
    let mut lo = 0usize;
    let mut hi = count;
    while lo < hi {
        let mid = (lo + hi) / 2;
        let v = u32::from_be_bytes([
            *members.get(4 * mid)?,
            *members.get(4 * mid + 1)?,
            *members.get(4 * mid + 2)?,
            *members.get(4 * mid + 3)?,
        ]);
        if v < value {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    members
        .get(4 * lo..4 * lo + 4)
        .filter(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) == value)
        .map(|_| lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_be_bytes());
    }
    fn w16(v: &mut Vec<u8>, x: u16) {
        v.extend_from_slice(&x.to_be_bytes());
    }

    /// Literal match element: repeat min/max packed, `usv` scalar.
    fn m_lit(rep: u8, usv: u32) -> [u8; 8] {
        let mut e = [0u8; 8];
        e[0] = rep;
        e[4..8].copy_from_slice(&usv.to_be_bytes());
        e
    }
    /// Non-literal match element (EOS/ANY/class/group codes).
    fn m_typed(rep: u8, ty: u8, value: [u8; 4]) -> [u8; 8] {
        let mut e = [0u8; 8];
        e[0] = rep;
        e[1] = K_MATCH_ELEM_NON_LIT | ty;
        e[4..8].copy_from_slice(&value);
        e
    }
    /// Literal replacement element (the whole word is the value).
    fn r_lit(usv: u32) -> [u8; 4] {
        usv.to_be_bytes()
    }

    /// Builds a U->U table exercising every rule feature:
    /// `a->A`, `b->B` direct; `x` -> `X` at EOS else `Y` (string rules);
    /// `c` -> `K` after `a` (pre-context); `d`/`e` -> `D`/`E` (class +
    /// replacement class).
    fn uu_table() -> Vec<u8> {
        let mut t = Vec::new();
        // TableHeader (48 bytes).
        w32(&mut t, K_TABLE_TYPE_UU); // 0
        w32(&mut t, 0x0001_0000); // 4 version
        w32(&mut t, 960); // 8 length
        w32(&mut t, 0); // 12 flags
        w32(&mut t, 48); // 16 pageBase
        w32(&mut t, 816); // 20 lookupBase
        w32(&mut t, 840); // 24 matchClassBase
        w32(&mut t, 852); // 28 repClassBase
        w32(&mut t, 864); // 32 stringListBase
        w32(&mut t, 880); // 36 stringRuleData
        t.extend_from_slice(&[1, 1, 1, 1]); // 40 maxMatch/maxPre/maxPost/maxOutput
        w32(&mut t, 0x003f); // 44 replacementChar '?'
        assert_eq!(t.len(), 48);

        // Page map (256 bytes): page 0 -> charMap 0.
        t.push(0);
        t.extend_from_slice(&[0xff; 255]);
        // Char map 0 (256 u16): char -> lookup index.
        let mut cm = vec![0u16; 256];
        cm[0x61] = 1; // a
        cm[0x62] = 2; // b
        cm[0x78] = 3; // x
        cm[0x63] = 4; // c
        cm[0x64] = 5; // d
        cm[0x65] = 5; // e shares d's rule
        for x in cm {
            w16(&mut t, x);
        }
        assert_eq!(t.len(), 816);

        // Lookup table (6 entries).
        t.extend_from_slice(&[0xfd, 0, 0, 0]); // L0 unmapped
        w32(&mut t, 0x41); // L1 a -> 'A'
        w32(&mut t, 0x42); // L2 b -> 'B'
        t.extend_from_slice(&[0xff, 2, 0, 0]); // L3 x: 2 rules at list 0
        t.extend_from_slice(&[0xff, 1, 0, 2]); // L4 c: 1 rule at list 2
        t.extend_from_slice(&[0xff, 1, 0, 3]); // L5 d/e: 1 rule at list 3
        assert_eq!(t.len(), 840);

        // Match class 0 = {'d','e'}.
        w32(&mut t, 4); // class offset from matchClassBase
        w32(&mut t, 2); // count
        w16(&mut t, 0x64);
        w16(&mut t, 0x65);
        assert_eq!(t.len(), 852);

        // Replacement class 0 = {'D','E'}.
        w32(&mut t, 4);
        w32(&mut t, 2);
        w16(&mut t, 0x44);
        w16(&mut t, 0x45);
        assert_eq!(t.len(), 864);

        // String rule list: offsets into stringRuleData.
        for off in [0u32, 24, 40, 64] {
            w32(&mut t, off);
        }
        assert_eq!(t.len(), 880);

        // Rule 0: x / _EOS -> 'X'  (match=1, post=1, pre=0, rep=1)
        t.extend_from_slice(&[1, 1, 0, 1]);
        t.extend_from_slice(&m_lit(0x11, 0x78));
        t.extend_from_slice(&m_typed(0x11, K_MATCH_ELEM_TYPE_EOS, [0; 4]));
        t.extend_from_slice(&r_lit(0x58));
        // Rule 1: x -> 'Y' (match=1, post=0, pre=0, rep=1)
        t.extend_from_slice(&[1, 0, 0, 1]);
        t.extend_from_slice(&m_lit(0x11, 0x78));
        t.extend_from_slice(&r_lit(0x59));
        // Rule 2: c / a_ -> 'K' (match=1, post=0, pre=1, rep=1)
        t.extend_from_slice(&[1, 0, 1, 1]);
        t.extend_from_slice(&m_lit(0x11, 0x63));
        t.extend_from_slice(&m_lit(0x11, 0x61));
        t.extend_from_slice(&r_lit(0x4b));
        // Rule 3: [class0] -> repClass0 (match=1, rep=1)
        t.extend_from_slice(&[1, 0, 0, 1]);
        t.extend_from_slice(&m_typed(0x11, K_MATCH_ELEM_TYPE_CLASS, [0, 0, 0, 0]));
        t.extend_from_slice(&[K_REP_ELEM_CLASS, 0, 0, 0]);
        assert_eq!(t.len(), 960);
        t
    }

    /// Builds a U->B table for byte_mapping (reverse pipeline): 'A' -> 0x80,
    /// everything else -> replacement char '?'.
    fn ub_table() -> Vec<u8> {
        let mut t = Vec::new();
        w32(&mut t, K_TABLE_TYPE_UB);
        w32(&mut t, 0x0001_0000);
        w32(&mut t, 824);
        w32(&mut t, 0);
        w32(&mut t, 48);
        w32(&mut t, 816);
        w32(&mut t, 824);
        w32(&mut t, 824);
        w32(&mut t, 824);
        w32(&mut t, 824);
        t.extend_from_slice(&[0, 0, 0, 1]);
        w32(&mut t, 0x003f);
        assert_eq!(t.len(), 48);
        t.push(0);
        t.extend_from_slice(&[0xff; 255]);
        let mut cm = vec![0u16; 256];
        cm[0x41] = 1;
        for x in cm {
            w16(&mut t, x);
        }
        assert_eq!(t.len(), 816);
        t.extend_from_slice(&[0xfd, 0, 0, 0]); // unmapped
        t.extend_from_slice(&[0x01, 0x80, 0, 0]); // 'A' -> byte 0x80
        assert_eq!(t.len(), 824);
        t
    }

    fn uu_file() -> Vec<u8> {
        let mut f = Vec::new();
        w32(&mut f, K_MAGIC_NUMBER);
        w32(&mut f, K_CURRENT_FILE_VERSION);
        w32(&mut f, 36); // headerLength
        w32(&mut f, K_FLAGS_UNICODE); // formFlagsLHS
        w32(&mut f, K_FLAGS_UNICODE); // formFlagsRHS
        w32(&mut f, 0); // numNames
        w32(&mut f, 1); // numFwdTables
        w32(&mut f, 0); // numRevTables
        w32(&mut f, 36); // fwd table offset
        f.extend_from_slice(&uu_table());
        f
    }

    fn ub_file() -> Vec<u8> {
        let mut f = Vec::new();
        w32(&mut f, K_MAGIC_NUMBER);
        w32(&mut f, K_CURRENT_FILE_VERSION);
        w32(&mut f, 36);
        w32(&mut f, 1); // LHS = bytes
        w32(&mut f, K_FLAGS_UNICODE); // RHS = Unicode
        w32(&mut f, 0);
        w32(&mut f, 0); // numFwdTables
        w32(&mut f, 1); // numRevTables
        w32(&mut f, 36); // rev table offset (fwd array is empty)
        f.extend_from_slice(&ub_table());
        f
    }

    fn u16s(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }
    fn from_u16s(v: &[u16]) -> String {
        String::from_utf16_lossy(v)
    }

    #[test]
    fn direct_lookups() {
        let c = Converter::parse(&uu_file(), false).unwrap();
        assert_eq!(from_u16s(&c.convert_text(&u16s("ab"))), "AB");
        // Unmapped chars in a U->U pass copy through.
        assert_eq!(from_u16s(&c.convert_text(&u16s("z"))), "z");
    }

    #[test]
    fn string_rules_and_context() {
        let c = Converter::parse(&uu_file(), false).unwrap();
        // Rule 0: x at end of input -> 'X' (EOS post-context).
        assert_eq!(from_u16s(&c.convert_text(&u16s("ax"))), "AX");
        // Rule 1: x mid-input -> 'Y'.
        assert_eq!(from_u16s(&c.convert_text(&u16s("axb"))), "AYB");
        // Rule 2: 'c' with 'a' pre-context -> 'K'.
        assert_eq!(from_u16s(&c.convert_text(&u16s("ac"))), "AK");
        // Without the 'a' pre-context 'c' has no matching rule -> copied.
        assert_eq!(from_u16s(&c.convert_text(&u16s("bc"))), "Bc");
    }

    #[test]
    fn match_and_rep_classes() {
        let c = Converter::parse(&uu_file(), false).unwrap();
        assert_eq!(from_u16s(&c.convert_text(&u16s("de"))), "DE");
        assert_eq!(from_u16s(&c.convert_text(&u16s("ade"))), "ADE");
    }

    #[test]
    fn byte_mapping_reverse_pipeline() {
        let c = Converter::parse(&ub_file(), true).unwrap();
        assert!(c.byte_mapping());
        assert_eq!(c.convert_byte(u16::from(b'A')), 0x80);
        // Unmapped -> replacement char '?'.
        assert_eq!(c.convert_byte(u16::from(b'z')), 0x3f);
    }

    #[test]
    fn utf16_surrogates() {
        // U+10348 round-trips through the scalar pipeline.
        let units = u16s("\u{10348}");
        assert_eq!(units, vec![0xD800, 0xDF48]);
        assert_eq!(decode_utf16(&units).unwrap(), vec![0x10348]);
        assert_eq!(encode_utf16(&[0x10348]), units);
        // Lone surrogates fail the input like C's kInvalidChar.
        assert!(decode_utf16(&[0xD800]).is_none());
        assert!(decode_utf16(&[0xDC00]).is_none());
        assert!(decode_utf16(&[0xD800, 0x41]).is_none());
        // A converter fed a lone surrogate produces no output.
        let c = Converter::parse(&uu_file(), false).unwrap();
        assert!(c.convert_text(&[0xD800]).is_empty());
    }

    #[test]
    fn normalization_stages() {
        // NFC stage first in the pipeline: e + combining acute -> é then
        // passes through the U->U table unmapped (copied).
        let mut f = Vec::new();
        w32(&mut f, K_MAGIC_NUMBER);
        w32(&mut f, K_CURRENT_FILE_VERSION);
        w32(&mut f, 40);
        w32(&mut f, K_FLAGS_UNICODE);
        w32(&mut f, K_FLAGS_UNICODE);
        w32(&mut f, 0);
        w32(&mut f, 2);
        w32(&mut f, 0);
        w32(&mut f, 40); // table 0 at offset 40
        w32(&mut f, 44); // table 1 at offset 44
        w32(&mut f, K_TABLE_TYPE_NFC);
        f.extend_from_slice(&uu_table());
        let c = Converter::parse(&f, false).unwrap();
        let decomposed = u16s("e\u{301}");
        assert_eq!(from_u16s(&c.convert_text(&decomposed)), "\u{e9}");
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(Converter::parse(&[], false).is_none());
        assert!(Converter::parse(&[0u8; 40], false).is_none());
        // Bad magic.
        let mut f = uu_file();
        f[0] = b'X';
        assert!(Converter::parse(&f, false).is_none());
        // Non-Unicode source form rejected for text conversion.
        let mut f2 = uu_file();
        f2[12] = 0; // formFlagsLHS &= ~kFlags_Unicode
        f2[13] = 0;
        f2[14] = 0;
        f2[15] = 1;
        assert!(Converter::parse(&f2, false).is_none());
    }
}
