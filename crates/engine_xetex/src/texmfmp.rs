//! texmfmp utility ports (`xetex-texmfmp.c`): PDF-style time strings, file
//! metadata `\pdf*date`/`\pdf*size` converts, MD5 sums, and file dumps over
//! the registered byte-source VFS.
//!
//! C pushes results into `str_pool` and returns via `str_toks`; the port
//! materializes the same text directly at the convert-expansion boundary.
//! `maketexstring`/`gettexstring` map to the existing UTF-16 pool/string
//! machinery; the src-special trio (`is_new_source`/`remember_source_info`/
//! `make_src_special`) ships with SyncTeX.

use md5::{Digest, Md5};

use crate::ini::civil_from_unix;
use crate::state::TeXEngineState;

/// `makepdftime` with `utc=true`: `D:YYYYmmddHHMMSS` + `Z` for the zero
/// UTC offset. Civil time is derived deterministically from Unix seconds
/// (`localtime`/`gmtime` are process-dependent and not WASM-portable); C's
/// leap-second `60|61 -> 59` clamp cannot arise from Unix seconds.
fn pdf_time_str(secs: i64) -> String {
    let secs_of_day = secs.rem_euclid(86_400);
    let (minutes, day, month, year) = civil_from_unix(secs);
    format!(
        "D:{year:04}{month:02}{day:02}{:02}{:02}{:02}Z",
        minutes / 60,
        minutes % 60,
        secs_of_day % 60
    )
}

impl TeXEngineState {
    /// Registers a file modification time for `name`, backing
    /// `ttstub_input_get_mtime` (`\pdffilemoddate`).
    pub fn register_input_mtime(&mut self, name: impl Into<String>, secs: i64) {
        self.input_file_mtimes.insert(name.into(), secs);
    }

    /// Captures the job creation-date string (`init_start_time`).
    pub fn init_start_time(&mut self, secs: i64) {
        self.start_time_str = pdf_time_str(secs);
    }

    /// The `\pdfcreationdate` text (`getcreationdate`).
    pub(crate) fn creation_date_text(&self) -> String {
        self.start_time_str.clone()
    }

    /// The `\pdffilemoddate` text for `name` (`getfilemoddate`): the
    /// registered mtime in PDF format, or empty when the file cannot be
    /// opened.
    pub(crate) fn file_mod_date_text(&mut self, name: &str) -> String {
        let Some(resolved) = self.input_byte_source_name(name) else {
            return String::new();
        };
        if !self.input_byte_sources.contains_key(&resolved) {
            return String::new();
        }
        // `ttstub_input_get_mtime`: consult the provider metadata resolver
        // on a cache miss so `\pdffilemoddate` reports real mtimes.
        if let std::collections::hash_map::Entry::Vacant(e) =
            self.input_file_mtimes.entry(resolved.clone())
        {
            if let Some(mtime) = self
                .input_metadata_resolver
                .clone()
                .and_then(|m| m.resolve(&resolved))
                .and_then(|meta| meta.mtime)
            {
                e.insert(mtime);
            }
        }
        pdf_time_str(self.input_file_mtimes.get(&resolved).copied().unwrap_or(0))
    }

    /// The `\pdffilesize` text for `name` (`getfilesize`): the decimal byte
    /// count, or empty when the file cannot be opened.
    pub(crate) fn file_size_text(&mut self, name: &str) -> String {
        self.input_byte_source_name(name)
            .and_then(|resolved| self.input_byte_sources.get(&resolved))
            .map(|bytes| bytes.len().to_string())
            .unwrap_or_default()
    }

    /// The `\pdffiledump` text for `name` (`getfiledump`): `length` bytes
    /// from `offset` as uppercase hex, or empty when the file cannot be
    /// opened. A zero `length` (the keyword's default) yields nothing, as
    /// in C.
    pub(crate) fn file_dump_text(&mut self, name: &str, offset: i32, length: i32) -> String {
        if length == 0 {
            return String::new();
        }
        let Some(bytes) = self
            .input_byte_source_name(name)
            .and_then(|resolved| self.input_byte_sources.get(&resolved))
        else {
            return String::new();
        };
        let start = usize::try_from(offset).unwrap_or(0).min(bytes.len());
        let take = usize::try_from(length)
            .unwrap_or(usize::MAX)
            .min(bytes.len() - start);
        let mut text = String::with_capacity(take * 2);
        for byte in &bytes[start..start + take] {
            text.push_str(&format!("{byte:02X}"));
        }
        text
    }

    /// The `\pdfmdfivesum` text (`getmd5sum`): uppercase hex MD5 of the
    /// file's bytes (with the `file` keyword) or of the argument text;
    /// empty when the file cannot be opened.
    pub(crate) fn md5_sum_text(&mut self, name: &str, file: bool) -> String {
        let data: Option<Vec<u8>> = if file {
            self.input_byte_source_name(name)
                .and_then(|resolved| self.input_byte_sources.get(&resolved).cloned())
        } else {
            Some(name.as_bytes().to_vec())
        };
        let Some(data) = data else {
            return String::new();
        };
        let mut hash = Md5::default();
        hash.update(&data);
        let digest = hash.finalize();
        let mut text = String::with_capacity(32);
        for byte in digest {
            text.push_str(&format!("{byte:02X}"));
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_helpers_resolve_through_the_injected_resolver() {
        let mut state = TeXEngineState::new(4_096);
        state.set_input_resolver(std::sync::Arc::new(|name: &str| {
            (name == "doc.bin").then(|| vec![0xDE, 0xAD, 0xBE, 0xEF])
        }));
        // `getfilesize` resolves lazily and caches the hit.
        assert_eq!(state.file_size_text("doc.bin"), "4");
        assert!(state.input_byte_sources.contains_key("doc.bin"));
        // `getfiledump` reads the cached bytes.
        assert_eq!(state.file_dump_text("doc.bin", 0, 4), "DEADBEEF");
        // `getmd5sum` hashes the resolved file content.
        assert_eq!(state.md5_sum_text("doc.bin", true).len(), 32);
        // A miss yields the empty string for all helpers.
        assert_eq!(state.file_size_text("absent.bin"), "");
        assert_eq!(state.md5_sum_text("absent.bin", true), "");
    }
}
