// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.
//
// Transpiled from Tectonic's XeTeX engine (xetex/xetex-io.c),
// which is itself derived from XeTeX and TeX82.

//! Low-level encoded file input: `UFILE` byte streams, encoding modes,
//! `get_uni_c`, `input_line`, normalization, and UTF-16 name encoding.
//!
//! This is the pure-Rust port of `xetex-io.c`. The C `rust_input_handle_t`
//! byte stream is modeled by [`UFile`], which owns the opened file's bytes
//! and a read cursor; registered in-memory byte sources take the place of
//! the `ttstub_input_*` filesystem/bundle calls.

use encoding_rs::Encoding;
use unicode_normalization::UnicodeNormalization;

use crate::scanner::{ReadOpen, TeXMouthError};
use crate::state::{
    TeXEngineState, INT_PAR_XETEX_DEFAULT_INPUT_ENCODING, INT_PAR_XETEX_DEFAULT_INPUT_MODE,
    INT_PAR_XETEX_INPUT_NORMALIZATION,
};

/// `AUTO`: encoding decided by `u_open_in`'s byte sniffing.
pub const INPUT_MODE_AUTO: i32 = 0;
/// `UTF8` input encoding mode.
pub const INPUT_MODE_UTF8: i32 = 1;
/// `UTF16BE` input encoding mode.
pub const INPUT_MODE_UTF16BE: i32 = 2;
/// `UTF16LE` input encoding mode.
pub const INPUT_MODE_UTF16LE: i32 = 3;
/// `RAW` (byte-at-a-time) input encoding mode.
pub const INPUT_MODE_RAW: i32 = 4;
/// `ICUMAPPING` input encoding mode (named byte->Unicode converter).
pub const INPUT_MODE_ICUMAPPING: i32 = 5;

/// `TTBC_FILE_FORMAT_TEX` from `ttbc_file_format`.
pub const TTBC_FILE_FORMAT_TEX: i32 = 26;
/// `TTBC_FILE_FORMAT_TECTONIC_PRIMARY` from `ttbc_file_format`.
pub const TTBC_FILE_FORMAT_TECTONIC_PRIMARY: i32 = 59;

/// End-of-stream marker matching C `EOF` in the `getc`/`get_uni_c` paths.
pub const INPUT_EOF: i32 = -1;

/// `offsetsFromUTF8`: subtracted from an assembled UTF-8 sequence to
/// recover the encoded scalar value.
pub const OFFSETS_FROM_UTF8: [u32; 6] = [
    0x0000_0000,
    0x0000_3080,
    0x000E_2080,
    0x03C8_2080,
    0xFA08_2080,
    0x8208_2080,
];

/// `bytesFromUTF8`: number of trailing bytes implied by a UTF-8 lead byte.
#[rustfmt::skip]
pub const BYTES_FROM_UTF8: [u8; 256] = [
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0, 0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
    1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1, 1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
    2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2, 3,3,3,3,3,3,3,3,4,4,4,4,5,5,5,5
];

/// `firstByteMark`: lead-byte mask for emitting `i` UTF-8 bytes.
pub const FIRST_BYTE_MARK: [u8; 7] = [0x00, 0x00, 0xC0, 0xE0, 0xF0, 0xF8, 0xFC];

/// An open encoded input byte stream (`UFILE`).
#[derive(Debug, Default, Clone)]
pub struct UFile {
    /// The opened file's bytes (`rust_input_handle_t` backing store).
    pub bytes: Vec<u8>,
    /// Byte read cursor into `bytes`.
    pub pos: usize,
    /// Active decoding mode (`encodingMode`).
    pub encoding_mode: i32,
    /// Pushed-back decoded scalar (`savedChar`).
    pub saved_char: i32,
    /// Whether the next LF must be swallowed after a CR (`skipNextLF`).
    pub skip_next_lf: bool,
    /// The `ICUMAPPING` byte->Unicode converter (`conversionData`).
    pub converter: Option<&'static Encoding>,
    /// Synthetic terminal stream; reads must abort (`INVALID_HANDLE`).
    pub is_terminal: bool,
}

impl UFile {
    /// Reads one byte, or `INPUT_EOF` at end of stream (`ttstub_input_getc`).
    pub fn getc(&mut self) -> i32 {
        if self.is_terminal {
            return INPUT_EOF;
        }
        match self.bytes.get(self.pos) {
            Some(&b) => {
                self.pos += 1;
                i32::from(b)
            }
            None => INPUT_EOF,
        }
    }

    /// Pushes back the most recently consumed byte (`ttstub_input_ungetc`).
    pub fn ungetc(&mut self) {
        self.pos = self.pos.saturating_sub(1);
    }

    /// Closes the stream (`u_close`); dropping releases the converter.
    pub fn close(self) {}
}

impl TeXEngineState {
    /// Registers raw file bytes for `u_open_in` lookups.
    pub fn register_input_byte_source(&mut self, name: impl Into<String>, bytes: Vec<u8>) {
        self.input_byte_sources.insert(name.into(), bytes);
    }

    /// Installs the lazy input resolver (`ttstub_input_open`'s live I/O
    /// path). Consulted on `input_byte_sources` misses before any `.tex`
    /// extension fallback.
    pub fn set_input_resolver(&mut self, resolver: crate::state::InputResolverFn) {
        self.input_resolver = Some(crate::state::InputResolver(resolver));
    }

    /// Resolves a byte source name, adding `.tex` when the stem has no
    /// extension (the search-path fallback used for `TTBC_FILE_FORMAT_TEX`).
    /// On a registered-map miss the injected `input_resolver` is consulted
    /// and a hit is cached under the resolved name.
    pub fn input_byte_source_name(&mut self, name: &str) -> Option<String> {
        if self.input_byte_sources.contains_key(name) {
            return Some(name.to_string());
        }
        if name
            .rsplit('/')
            .next()
            .is_some_and(|stem| !stem.contains('.'))
        {
            let with_ext = format!("{name}.tex");
            if self.input_byte_sources.contains_key(&with_ext) {
                return Some(with_ext);
            }
        }
        if let Some(resolver) = &self.input_resolver {
            if let Some(bytes) = resolver.resolve(name) {
                self.input_byte_sources.insert(name.to_string(), bytes);
                return Some(name.to_string());
            }
            if name
                .rsplit('/')
                .next()
                .is_some_and(|stem| !stem.contains('.'))
            {
                let with_ext = format!("{name}.tex");
                if let Some(bytes) = resolver.resolve(&with_ext) {
                    self.input_byte_sources.insert(with_ext.clone(), bytes);
                    return Some(with_ext);
                }
            }
        }
        None
    }

    /// Fetches raw bytes for `name` from the registered map, consulting
    /// the injected `input_resolver` on a miss and caching a hit (the
    /// byte-level half of `input_byte_source_name`, used by TFM and
    /// picture loaders that carry their own extension).
    pub fn input_byte_source_bytes(&mut self, name: &str) -> Option<Vec<u8>> {
        if let Some(bytes) = self.input_byte_sources.get(name) {
            return Some(bytes.clone());
        }
        let resolver = self.input_resolver.clone()?;
        let bytes = resolver.resolve(name)?;
        self.input_byte_sources
            .insert(name.to_string(), bytes.clone());
        Some(bytes)
    }

    /// Packs the scanned `cur_name`/`cur_area`/`cur_ext` into
    /// `name_of_file` bytes (`pack_file_name`'s `name_of_file` product).
    pub fn pack_name_of_file(&mut self) {
        self.name_of_file = self.file_name_string().into_bytes();
        self.name_length = self.name_of_file.len() as i32;
    }

    /// Opens the file named by `name_of_file` (`tt_xetex_open_input`).
    ///
    /// Returns the file's bytes, or `None` when the name is unresolvable or
    /// names a `|` pipe command (unsupported in Tectonic).
    fn tt_xetex_open_input(&mut self, filefmt: i32) -> Option<Vec<u8>> {
        let name = String::from_utf8_lossy(&self.name_of_file).into_owned();
        if filefmt != TTBC_FILE_FORMAT_TECTONIC_PRIMARY && name.starts_with('|') {
            let _ = self.print_nl_cstr("Warning: ");
            self.capture_to_diagnostic(true);
            self.diagnostic_print_file_line();
            let _ = self
                .print_cstr("piped inputs from external commands are not implemented in Tectonic");
            self.capture_to_diagnostic(false);
            return None;
        }

        let resolved = self.input_byte_source_name(&name)?;
        let bytes = self.input_byte_sources.get(&resolved)?.clone();
        self.name_length = self.name_of_file.len() as i32;
        self.name_of_input_file = Some(name);
        self.abspath_of_input_file.clear();
        Some(bytes)
    }

    /// Selects a stream's decoding mode (`set_input_file_encoding`).
    ///
    /// `encoding_data` is the interned converter name for `ICUMAPPING`, as
    /// produced by `\XeTeXinputencoding`/`\XeTeXdefaultencoding`.
    pub fn set_input_file_encoding(&mut self, f: &mut UFile, mode: i32, encoding_data: i32) {
        match mode {
            INPUT_MODE_UTF8 | INPUT_MODE_UTF16BE | INPUT_MODE_UTF16LE | INPUT_MODE_RAW => {
                f.encoding_mode = mode;
            }
            INPUT_MODE_ICUMAPPING => {
                let name = self.string_text(encoding_data).unwrap_or_default();
                match Encoding::for_label(name.as_bytes()) {
                    None => {
                        let _ = self.begin_diagnostic();
                        let _ = self.print_nl(i32::from(b'E'));
                        let _ = self.print_cstr("rror creating Unicode converter for `");
                        let _ = self.print_cstr(&name);
                        let _ = self.print_cstr("'; reading as raw bytes");
                        let _ = self.end_diagnostic(true);
                        f.encoding_mode = INPUT_MODE_RAW;
                        f.converter = None;
                    }
                    Some(encoding) => {
                        f.encoding_mode = INPUT_MODE_ICUMAPPING;
                        f.converter = Some(encoding);
                    }
                }
            }
            _ => {}
        }
    }

    /// Opens an encoded input stream (`u_open_in`).
    ///
    /// `mode == INPUT_MODE_AUTO` sniffs the first bytes for a UTF-16 or
    /// UTF-8 byte-order mark, matching C's `AUTO` resolution.
    pub fn u_open_in(&mut self, filefmt: i32, mode: i32, encoding_data: i32) -> Option<UFile> {
        let bytes = self.tt_xetex_open_input(filefmt)?;
        let mut f = UFile {
            bytes,
            pos: 0,
            encoding_mode: 0,
            saved_char: -1,
            skip_next_lf: false,
            converter: None,
            is_terminal: false,
        };

        let mut mode = mode;
        if mode == INPUT_MODE_AUTO {
            let b1 = f.getc();
            let b2 = f.getc();

            if b1 == 0xFE && b2 == 0xFF {
                mode = INPUT_MODE_UTF16BE;
            } else if b2 == 0xFE && b1 == 0xFF {
                mode = INPUT_MODE_UTF16LE;
            } else if b1 == 0 && b2 != 0 {
                mode = INPUT_MODE_UTF16BE;
                f.pos = 0;
            } else if b2 == 0 && b1 != 0 {
                mode = INPUT_MODE_UTF16LE;
                f.pos = 0;
            } else if b1 == 0xEF && b2 == 0xBB {
                let b3 = f.getc();
                if b3 == 0xBF {
                    mode = INPUT_MODE_UTF8;
                }
            }

            if mode == INPUT_MODE_AUTO {
                f.pos = 0;
                mode = INPUT_MODE_UTF8;
            }
        }

        self.set_input_file_encoding(&mut f, mode, encoding_data);
        Some(f)
    }

    /// Aborts when a single input line exceeds `buf_size` (`buffer_overflow`).
    fn buffer_overflow<T>(&self) -> Result<T, TeXMouthError> {
        Err(TeXMouthError::BufferOverflow(self.buf_size))
    }

    /// Reports a failed byte->Unicode conversion (`conversion_error`).
    fn conversion_error(&mut self, errcode: i32) {
        let _ = self.begin_diagnostic();
        let _ = self.print_nl(i32::from(b'U'));
        let _ = self.print_cstr("nicode conversion failed (ICU error code = ");
        let _ = self.print_int(errcode);
        let _ = self.print_cstr(") discarding any remaining text");
        let _ = self.end_diagnostic(true);
    }

    /// `\XeTeXinputnormalization` state (`get_input_normalization_state`).
    fn get_input_normalization_state(&self) -> i32 {
        self.int_par(INT_PAR_XETEX_INPUT_NORMALIZATION).unwrap_or(0)
    }

    /// Writes one buffer unit at `idx`, growing the line buffer as needed.
    fn buffer_put(&mut self, idx: usize, value: i32) {
        if self.buffer.len() <= idx {
            self.buffer.resize(idx + 1, 0);
        }
        self.buffer[idx] = value;
    }

    /// Applies NFC or NFD normalization to `buf` into `buffer[first..]`
    /// (`apply_normalization`). Sets `last` to the normalized end.
    fn apply_normalization(&mut self, buf: &[u32], norm: i32) -> Result<(), TeXMouthError> {
        let text: String = buf
            .iter()
            .map(|&u| char::from_u32(u).unwrap_or('\u{FFFD}'))
            .collect();
        let normalized: Vec<u32> = if norm == 1 {
            text.nfc().map(|c| c as u32).collect()
        } else {
            text.nfd().map(|c| c as u32).collect()
        };
        let first = usize::try_from(self.first).unwrap_or(0);
        if normalized.len() > self.buf_size.saturating_sub(first) {
            return self.buffer_overflow();
        }
        for (i, &u) in normalized.iter().enumerate() {
            self.buffer_put(first + i, i32::try_from(u).unwrap_or(0xFFFD));
        }
        self.last = self.first + normalized.len() as i32;
        Ok(())
    }

    /// Reads one logical input line into `buffer[first..last]`
    /// (`input_line`). Returns `Ok(false)` at end of stream with no
    /// characters consumed.
    ///
    /// On return `buffer[last]` holds the ` ` sentinel and trailing
    /// space/EOL characters have been trimmed, exactly like C; callers set
    /// `cur_input.limit = last` and place `\endlinechar` at `buffer[limit]`.
    pub fn input_line(&mut self, f: &mut UFile) -> Result<bool, TeXMouthError> {
        if f.is_terminal {
            return Err(TeXMouthError::UnsupportedPrimitive(
                "reads from synthetic \"terminal\" file #0 should never happen",
            ));
        }

        let norm = self.get_input_normalization_state();
        self.last = self.first;
        let first = usize::try_from(self.first).unwrap_or(0);
        self.buffer.truncate(first);

        let mut i: i32;
        if f.encoding_mode == INPUT_MODE_ICUMAPPING {
            // Read raw bytes up to the line terminator.
            i = f.getc();
            if f.skip_next_lf {
                f.skip_next_lf = false;
                if i == i32::from(b'\n') {
                    i = f.getc();
                }
            }

            let mut bytes_read = 0usize;
            if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                if self.io_byte_buf.len() <= bytes_read {
                    self.io_byte_buf.resize(self.buf_size + 1, 0);
                }
                self.io_byte_buf[bytes_read] = i as u8;
                bytes_read += 1;
            }
            if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                while bytes_read < self.buf_size {
                    i = f.getc();
                    if i == INPUT_EOF || i == i32::from(b'\n') || i == i32::from(b'\r') {
                        break;
                    }
                    self.io_byte_buf[bytes_read] = i as u8;
                    bytes_read += 1;
                }
            }

            if i == INPUT_EOF && bytes_read == 0 {
                return Ok(false);
            }
            if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                return self.buffer_overflow();
            }

            // Apply the mapping to turn external bytes into Unicode scalars.
            let Some(converter) = f.converter else {
                self.conversion_error(1);
                return Ok(false);
            };
            let line_bytes = self.io_byte_buf[..bytes_read].to_vec();
            let (decoded, _, _) = converter.decode(&line_bytes);
            match norm {
                1 | 2 => {
                    let utf32: Vec<u32> = decoded.chars().map(|c| c as u32).collect();
                    if utf32.len() > self.buf_size {
                        self.conversion_error(1);
                        return Ok(false);
                    }
                    self.apply_normalization(&utf32, norm)?;
                }
                _ => {
                    let mut written = 0usize;
                    for c in decoded.chars() {
                        if written >= self.buf_size - first {
                            break;
                        }
                        self.buffer_put(first + written, c as i32);
                        written += 1;
                    }
                    self.last = self.first + written as i32;
                }
            }
        } else {
            i = self.get_uni_c(f);
            if f.skip_next_lf {
                f.skip_next_lf = false;
                if i == i32::from(b'\n') {
                    i = self.get_uni_c(f);
                }
            }

            match norm {
                1 | 2 => {
                    // Read decoded scalars into the UTF-32 scratch buffer.
                    let mut tmp_len = 0usize;
                    if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                        self.io_utf32_buf.clear();
                        self.io_utf32_buf.push(i as u32);
                        tmp_len += 1;
                    }
                    if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                        while tmp_len < self.buf_size {
                            i = self.get_uni_c(f);
                            if i == INPUT_EOF || i == i32::from(b'\n') || i == i32::from(b'\r') {
                                break;
                            }
                            self.io_utf32_buf.push(i as u32);
                            tmp_len += 1;
                        }
                    }

                    if i == INPUT_EOF && tmp_len == 0 {
                        return Ok(false);
                    }
                    if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                        return self.buffer_overflow();
                    }
                    let utf32 = self.io_utf32_buf[..tmp_len].to_vec();
                    self.apply_normalization(&utf32, norm)?;
                }
                _ => {
                    if self.last < self.buf_size as i32
                        && i != INPUT_EOF
                        && i != i32::from(b'\n')
                        && i != i32::from(b'\r')
                    {
                        self.buffer_put(self.last as usize, i);
                        self.last += 1;
                    }
                    if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                        while self.last < self.buf_size as i32 {
                            i = self.get_uni_c(f);
                            if i == INPUT_EOF || i == i32::from(b'\n') || i == i32::from(b'\r') {
                                break;
                            }
                            self.buffer_put(self.last as usize, i);
                            self.last += 1;
                        }
                    }

                    if i == INPUT_EOF && self.last == self.first {
                        return Ok(false);
                    }
                    if i != INPUT_EOF && i != i32::from(b'\n') && i != i32::from(b'\r') {
                        return self.buffer_overflow();
                    }
                }
            }
        }

        // If the line ended with CR, remember to skip a following LF.
        if i == i32::from(b'\r') {
            f.skip_next_lf = true;
        }

        self.buffer_put(self.last as usize, i32::from(b' '));
        if self.last >= self.max_buf_stack {
            self.max_buf_stack = self.last;
        }

        // Trim trailing space or EOL characters.
        while self.last > self.first {
            let c = self.buffer[(self.last - 1) as usize];
            if c != i32::from(b' ') && c != i32::from(b'\r') && c != i32::from(b'\n') {
                break;
            }
            self.last -= 1;
        }
        self.buffer.truncate((self.last + 1) as usize);

        Ok(true)
    }

    /// Decodes one character from the stream (`get_uni_c`), returning
    /// `INPUT_EOF` at end of stream. Honors `saved_char` and the
    /// encoding mode's multi-byte forms.
    pub fn get_uni_c(&mut self, f: &mut UFile) -> i32 {
        if f.saved_char != -1 {
            let rval = f.saved_char;
            f.saved_char = -1;
            return rval;
        }

        match f.encoding_mode {
            INPUT_MODE_UTF8 => {
                let mut rval = f.getc();
                if rval == INPUT_EOF {
                    return rval;
                }
                let extra_bytes = BYTES_FROM_UTF8[rval as usize] as usize;
                if extra_bytes >= 4 {
                    self.bad_utf8_warning();
                    return 0xFFFD;
                }
                for _ in 0..extra_bytes {
                    let c = f.getc();
                    if !(0x80..0xC0).contains(&c) {
                        if c != INPUT_EOF {
                            f.ungetc();
                        }
                        self.bad_utf8_warning();
                        return 0xFFFD;
                    }
                    rval = (rval << 6) + c;
                }
                rval -= OFFSETS_FROM_UTF8[extra_bytes] as i32;
                if !(0..=0x10FFFF).contains(&rval) {
                    self.bad_utf8_warning();
                    return 0xFFFD;
                }
                rval
            }
            INPUT_MODE_UTF16BE => {
                let mut rval = f.getc();
                if rval == INPUT_EOF {
                    return rval;
                }
                rval = (rval << 8) + f.getc();
                if (0xD800..=0xDBFF).contains(&rval) {
                    let mut lo = f.getc();
                    lo = (lo << 8) + f.getc();
                    if (0xDC00..=0xDFFF).contains(&lo) {
                        rval = 0x10000 + (rval - 0xD800) * 0x400 + (lo - 0xDC00);
                    } else {
                        rval = 0xFFFD;
                        f.saved_char = lo;
                    }
                } else if (0xDC00..=0xDFFF).contains(&rval) {
                    rval = 0xFFFD;
                }
                rval
            }
            INPUT_MODE_UTF16LE => {
                let mut rval = f.getc();
                if rval == INPUT_EOF {
                    return rval;
                }
                rval += f.getc() << 8;
                if (0xD800..=0xDBFF).contains(&rval) {
                    let mut lo = f.getc();
                    lo += f.getc() << 8;
                    if (0xDC00..=0xDFFF).contains(&lo) {
                        rval = 0x10000 + (rval - 0xD800) * 0x400 + (lo - 0xDC00);
                    } else {
                        rval = 0xFFFD;
                        f.saved_char = lo;
                    }
                } else if (0xDC00..=0xDFFF).contains(&rval) {
                    rval = 0xFFFD;
                }
                rval
            }
            INPUT_MODE_RAW => f.getc(),
            _ => INPUT_EOF,
        }
    }

    /// Warns that invalid UTF-8 input was replaced by U+FFFD
    /// (`bad_utf8_warning`).
    pub fn bad_utf8_warning(&mut self) {
        let _ = self.begin_diagnostic();
        self.capture_to_diagnostic(true);
        self.diagnostic_print_file_line();
        let _ = self.print_nl_cstr("Invalid UTF-8 byte or sequence");
        if self.cur_input.name == 0 {
            let _ = self.print_cstr(" in terminal input");
        } else {
            let _ = self.print_cstr(" at line ");
            let _ = self.print_int(self.line);
        }
        let _ = self.print_cstr(" replaced by U+FFFD.");
        self.capture_to_diagnostic(false);
        let _ = self.end_diagnostic(false);
    }

    /// Re-encodes `name_of_file` as UTF-16 code units in `name_of_file16`
    /// (`make_utf16_name`), including the historical `if (*s)` guard that
    /// stops consuming a continuation byte at NUL or end-of-name.
    pub fn make_utf16_name(&mut self) {
        let s = std::mem::take(&mut self.name_of_file);
        let mut t: Vec<u16> = Vec::with_capacity(s.len() + 10);
        let mut i = 0usize;
        while i < self.name_length.max(0) as usize && i < s.len() {
            let mut rval = u32::from(s[i]);
            i += 1;
            let extra_bytes = BYTES_FROM_UTF8[rval as usize] as usize;
            for _ in 0..extra_bytes {
                rval <<= 6;
                if i < s.len() && s[i] != 0 {
                    rval += u32::from(s[i]);
                    i += 1;
                }
            }
            rval = rval.wrapping_sub(OFFSETS_FROM_UTF8[extra_bytes]);
            if rval > 0xFFFF {
                let rval = rval - 0x10000;
                t.push(0xD800 + (rval / 0x0400) as u16);
                t.push(0xDC00 + (rval % 0x0400) as u16);
            } else {
                t.push(rval as u16);
            }
        }
        self.name_of_file = s;
        self.name_of_file16 = t;
        self.name_length16 = self.name_of_file16.len() as i32;
    }

    /// Opens or closes a numbered `\read` stream (`open_or_close_in`).
    pub fn open_or_close_in(&mut self) -> Result<(), TeXMouthError> {
        let opening = self.cur_chr != 0;
        self.scan_four_bit_int()?;
        let n = usize::try_from(self.cur_val)
            .map_err(|_| TeXMouthError::InputOutOfBounds(self.cur_val))?;
        if n >= self.read_open.len() || n >= self.read_file.len() {
            return Err(TeXMouthError::InputOutOfBounds(self.cur_val));
        }

        if self.read_open[n] != ReadOpen::Closed {
            self.read_file[n] = None;
            self.read_open[n] = ReadOpen::Closed;
        }

        if !opening {
            return Ok(());
        }

        self.scan_optional_equals()?;
        self.scan_file_name()?;
        self.pack_name_of_file();

        let mode = self.int_par(INT_PAR_XETEX_DEFAULT_INPUT_MODE).unwrap_or(0);
        let encoding = self
            .int_par(INT_PAR_XETEX_DEFAULT_INPUT_ENCODING)
            .unwrap_or(0);
        if let Some(f) = self.u_open_in(TTBC_FILE_FORMAT_TEX, mode, encoding) {
            self.read_file[n] = Some(f);
            self.read_sources[n] = None;
            self.make_utf16_name();
            self.name_in_progress = true;
            self.begin_name();
            self.stop_at_space = false;
            for k in 0..self.name_length16.max(0) as usize {
                let c16 = self.name_of_file16[k];
                if !self.more_name(i32::from(c16)) {
                    break;
                }
            }
            self.stop_at_space = true;
            self.end_name();
            self.name_in_progress = false;
            self.read_open[n] = ReadOpen::JustOpen;
        } else {
            // Fall back to a registered unit-line source (the
            // `register_input_source_units` compatibility path).
            let name = self.file_name_string();
            if let Some(lines) = self.input_source_lines(&name) {
                let name_id = self.next_source_name;
                self.next_source_name += 1;
                self.read_sources[n] =
                    Some(crate::scanner::FileInputSource::new(name, name_id, lines));
                self.read_open[n] = ReadOpen::JustOpen;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::INT_PAR_XETEX_INPUT_NORMALIZATION;

    fn ufile(mode: i32, bytes: &[u8]) -> UFile {
        UFile {
            bytes: bytes.to_vec(),
            pos: 0,
            encoding_mode: mode,
            saved_char: -1,
            skip_next_lf: false,
            converter: None,
            is_terminal: false,
        }
    }

    fn open(state: &mut TeXEngineState, name: &str, bytes: &[u8], mode: i32) -> Option<UFile> {
        state.register_input_byte_source(name, bytes.to_vec());
        state.name_of_file = name.as_bytes().to_vec();
        state.name_length = name.len() as i32;
        state.u_open_in(TTBC_FILE_FORMAT_TEX, mode, 0)
    }

    #[test]
    fn u_open_in_sniffs_utf16_boms() {
        let mut state = TeXEngineState::new(4_096);
        let f = open(&mut state, "be", &[0xFE, 0xFF, 0x00, 0x61], INPUT_MODE_AUTO).unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF16BE);
        assert_eq!(f.pos, 2);

        let f = open(&mut state, "le", &[0xFF, 0xFE, 0x61, 0x00], INPUT_MODE_AUTO).unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF16LE);
        assert_eq!(f.pos, 2);
    }

    #[test]
    fn u_open_in_sniffs_utf8_bom_and_null_heuristic() {
        let mut state = TeXEngineState::new(4_096);
        let f = open(&mut state, "u8", &[0xEF, 0xBB, 0xBF, b'a'], INPUT_MODE_AUTO).unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF8);
        assert_eq!(f.pos, 3);

        // Leading NUL implies UTF-16BE without a BOM; the cursor rewinds.
        let f = open(&mut state, "nb", &[0x00, 0x61, 0x00, 0x62], INPUT_MODE_AUTO).unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF16BE);
        assert_eq!(f.pos, 0);

        let f = open(&mut state, "nl", &[0x61, 0x00, 0x62, 0x00], INPUT_MODE_AUTO).unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF16LE);
        assert_eq!(f.pos, 0);

        // Plain ASCII rewinds and decodes as UTF-8.
        let f = open(&mut state, "plain", b"abc", INPUT_MODE_AUTO).unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF8);
        assert_eq!(f.pos, 0);
    }

    #[test]
    fn get_uni_c_decodes_utf8_sequences() {
        let mut state = TeXEngineState::new(4_096);
        // "aé€𐍈" as UTF-8.
        let mut f = ufile(INPUT_MODE_UTF8, "a\u{e9}\u{20ac}\u{10348}".as_bytes());
        assert_eq!(state.get_uni_c(&mut f), i32::from(b'a'));
        assert_eq!(state.get_uni_c(&mut f), 0xE9);
        assert_eq!(state.get_uni_c(&mut f), 0x20AC);
        assert_eq!(state.get_uni_c(&mut f), 0x10348);
        assert_eq!(state.get_uni_c(&mut f), INPUT_EOF);
    }

    #[test]
    fn get_uni_c_recovers_from_bad_utf8() {
        let mut state = TeXEngineState::new(4_096);
        // Lead byte 0xC2 followed by ASCII 'x': invalid continuation.
        let mut f = ufile(INPUT_MODE_UTF8, &[0xC2, b'x']);
        assert_eq!(state.get_uni_c(&mut f), 0xFFFD);
        // 'x' was pushed back, not consumed.
        assert_eq!(state.get_uni_c(&mut f), i32::from(b'x'));

        // Lead bytes outside Unicode range warn immediately.
        let mut f = ufile(INPUT_MODE_UTF8, &[0xF8, 0x88, 0x80, 0x80, 0x80]);
        assert_eq!(state.get_uni_c(&mut f), 0xFFFD);
    }

    #[test]
    fn get_uni_c_decodes_utf16_surrogate_pairs() {
        let mut state = TeXEngineState::new(4_096);
        // U+10348 in UTF-16BE: D800 DF48.
        let mut f = ufile(INPUT_MODE_UTF16BE, &[0xD8, 0x00, 0xDF, 0x48]);
        assert_eq!(state.get_uni_c(&mut f), 0x10348);

        // UTF-16LE bytes.
        let mut f = ufile(INPUT_MODE_UTF16LE, &[0x00, 0xD8, 0x48, 0xDF]);
        assert_eq!(state.get_uni_c(&mut f), 0x10348);

        // Lone low surrogate.
        let mut f = ufile(INPUT_MODE_UTF16BE, &[0xDC, 0x00]);
        assert_eq!(state.get_uni_c(&mut f), 0xFFFD);

        // High surrogate followed by a non-low unit: FFFD plus saved char.
        let mut f = ufile(INPUT_MODE_UTF16BE, &[0xD8, 0x00, 0x00, 0x41]);
        assert_eq!(state.get_uni_c(&mut f), 0xFFFD);
        assert_eq!(state.get_uni_c(&mut f), 0x41);
    }

    #[test]
    fn get_uni_c_honors_saved_char_and_raw_mode() {
        let mut state = TeXEngineState::new(4_096);
        let mut f = ufile(INPUT_MODE_RAW, &[0x80, b'a']);
        f.saved_char = 0x4242;
        assert_eq!(state.get_uni_c(&mut f), 0x4242);
        assert_eq!(state.get_uni_c(&mut f), 0x80);
        assert_eq!(state.get_uni_c(&mut f), i32::from(b'a'));
        assert_eq!(state.get_uni_c(&mut f), INPUT_EOF);
    }

    #[test]
    fn input_line_reads_lines_and_trims() {
        let mut state = TeXEngineState::new(4_096);
        let mut f = ufile(INPUT_MODE_UTF8, b"abc  \ndef");
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.last, 3);
        assert_eq!(
            state.buffer[..4],
            [b'a' as i32, b'b' as i32, b'c' as i32, b' ' as i32]
        );

        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.buffer[0], b'd' as i32);
        assert_eq!(state.last, 3);

        state.first = 0;
        assert!(!state.input_line(&mut f).unwrap());
    }

    #[test]
    fn input_line_handles_crlf_and_lone_cr() {
        let mut state = TeXEngineState::new(4_096);
        let mut f = ufile(INPUT_MODE_UTF8, b"a\r\nb\rc");
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.buffer[0], b'a' as i32);
        assert!(f.skip_next_lf);

        // The LF after CR is swallowed.
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.buffer[0], b'b' as i32);

        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.buffer[0], b'c' as i32);
    }

    #[test]
    fn input_line_decodes_utf16_files() {
        let mut state = TeXEngineState::new(4_096);
        // UTF-16BE BOM + "ab\n".
        let bytes = [0xFE, 0xFF, 0x00, b'a', 0x00, b'b', 0x00, b'\n'];
        let mut f = open(&mut state, "f16", &bytes, INPUT_MODE_AUTO).unwrap();
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.last, 2);
        assert_eq!(state.buffer[..2], [b'a' as i32, b'b' as i32]);
    }

    #[test]
    fn input_line_applies_nfc_and_nfd() {
        let mut state = TeXEngineState::new(4_096);
        state
            .set_int_par(INT_PAR_XETEX_INPUT_NORMALIZATION, 1)
            .unwrap();
        // "e" + combining acute -> NFC "é" (one scalar).
        let mut f = ufile(INPUT_MODE_UTF8, "e\u{301}".as_bytes());
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.last, 1);
        assert_eq!(state.buffer[0], 0xE9);

        state
            .set_int_par(INT_PAR_XETEX_INPUT_NORMALIZATION, 2)
            .unwrap();
        // "é" -> NFD "e" + combining acute (two scalars).
        let mut f = ufile(INPUT_MODE_UTF8, "\u{e9}".as_bytes());
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.last, 2);
        assert_eq!(state.buffer[..2], [b'e' as i32, 0x301]);
    }

    #[test]
    fn icu_mapping_converts_bytes_via_encoding_rs() {
        let mut state = TeXEngineState::new(4_096);
        let mut f = ufile(INPUT_MODE_RAW, &[0xE9]);
        // An encoding-data string id for "iso-8859-1".
        let name_units: Vec<i32> = "iso-8859-1".chars().map(|c| c as i32).collect();
        let id = state.intern_string(&name_units);
        state.set_input_file_encoding(&mut f, INPUT_MODE_ICUMAPPING, id);
        assert_eq!(f.encoding_mode, INPUT_MODE_ICUMAPPING);

        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.buffer[0], 0xE9); // Latin-1 0xE9 = é
    }

    #[test]
    fn unknown_icu_mapping_falls_back_to_raw_with_warning() {
        let mut state = TeXEngineState::new(4_096);
        let mut f = ufile(INPUT_MODE_RAW, &[0xE9]);
        let name_units: Vec<i32> = "no-such-encoding-xyz".chars().map(|c| c as i32).collect();
        let id = state.intern_string(&name_units);
        state.set_input_file_encoding(&mut f, INPUT_MODE_ICUMAPPING, id);
        assert_eq!(f.encoding_mode, INPUT_MODE_RAW);
        assert_eq!(state.get_uni_c(&mut f), 0xE9);
    }

    #[test]
    fn input_line_reports_buffer_overflow() {
        let mut state = TeXEngineState::new(4_096);
        state.buf_size = 4;
        let mut f = ufile(INPUT_MODE_UTF8, b"abcde");
        state.first = 0;
        assert!(matches!(
            state.input_line(&mut f),
            Err(TeXMouthError::BufferOverflow(4))
        ));
    }

    #[test]
    fn make_utf16_name_encodes_scalars_and_surrogates() {
        let mut state = TeXEngineState::new(4_096);
        state.name_of_file = "a/\u{10348}.tex".as_bytes().to_vec();
        state.name_length = state.name_of_file.len() as i32;
        state.make_utf16_name();
        let expected: Vec<u16> = "a/\u{10348}.tex".encode_utf16().collect();
        assert_eq!(state.name_of_file16, expected);
        assert_eq!(state.name_length16, expected.len() as i32);
    }

    #[test]
    fn openin_reads_a_utf16_stream() {
        let mut state = TeXEngineState::new(4_096);
        // UTF-16LE bytes for "x" registered under an ASCII name.
        state.register_input_byte_source("f16", vec![0xFF, 0xFE, b'x', 0x00]);
        state.name_of_file = b"f16".to_vec();
        state.name_length = 3;
        let f = state
            .u_open_in(TTBC_FILE_FORMAT_TEX, INPUT_MODE_AUTO, 0)
            .unwrap();
        assert_eq!(f.encoding_mode, INPUT_MODE_UTF16LE);
        let mut f = f;
        state.first = 0;
        assert!(state.input_line(&mut f).unwrap());
        assert_eq!(state.buffer[0], b'x' as i32);
    }

    #[test]
    fn pipe_input_warns_and_fails() {
        let mut state = TeXEngineState::new(4_096);
        state.name_of_file = b"|some-command".to_vec();
        state.name_length = state.name_of_file.len() as i32;
        assert!(state
            .u_open_in(TTBC_FILE_FORMAT_TEX, INPUT_MODE_AUTO, 0)
            .is_none());
    }

    #[test]
    fn resolver_fills_misses_and_caches_by_exact_name() {
        let mut state = TeXEngineState::new(4_096);
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = calls.clone();
        state.set_input_resolver(std::sync::Arc::new(move |name: &str| {
            seen.lock().unwrap().push(name.to_string());
            (name == "chap1.tex").then(|| b"hello".to_vec())
        }));
        // Exact name resolves through the injected callback.
        assert_eq!(
            state.input_byte_source_name("chap1.tex").as_deref(),
            Some("chap1.tex")
        );
        assert_eq!(state.input_byte_sources["chap1.tex"], b"hello");
        // A second lookup hits the cache, not the resolver.
        assert_eq!(
            state.input_byte_source_name("chap1.tex").as_deref(),
            Some("chap1.tex")
        );
        assert_eq!(calls.lock().unwrap().as_slice(), &["chap1.tex"]);
    }

    #[test]
    fn resolver_tex_extension_fallback() {
        let mut state = TeXEngineState::new(4_096);
        state.set_input_resolver(std::sync::Arc::new(|name: &str| {
            (name == "story.tex").then(|| b"Once".to_vec())
        }));
        // Extensionless name falls back to `.tex`.
        assert_eq!(
            state.input_byte_source_name("story").as_deref(),
            Some("story.tex")
        );
        assert_eq!(state.input_byte_sources["story.tex"], b"Once");
        // Total miss still resolves to nothing.
        assert_eq!(state.input_byte_source_name("absent"), None);
    }

    #[test]
    fn resolver_byte_source_caches_tfm_style_names() {
        let mut state = TeXEngineState::new(4_096);
        state.set_input_resolver(std::sync::Arc::new(|name: &str| {
            (name == "cmr10.tfm").then(|| vec![0u8; 64])
        }));
        assert_eq!(
            state.input_byte_source_bytes("cmr10.tfm"),
            Some(vec![0u8; 64])
        );
        assert!(state.input_byte_sources.contains_key("cmr10.tfm"));
        assert_eq!(state.input_byte_source_bytes("nope.tfm"), None);
    }
}
