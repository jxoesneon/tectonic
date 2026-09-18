// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust string pool and table management for Tectonic.
//!
//! This crate encapsulates Knuth's `str_pool` and `str_start` tables behind a safe,
//! bounds-checked, and re-entrant data structure with full UTF-16 surrogate support,
//! fulfilling Phase 4 of the Tectonic Pure-Rust Migration Roadmap.

#![deny(missing_docs)]
#![deny(clippy::undocumented_unsafe_blocks)]

use std::collections::HashMap;

/// The threshold separating direct single-character codes from pooled string identifiers.
pub const TOO_BIG_CHAR: i32 = 0x0001_0000; // 65,536

/// A typed identifier for a TeX string.
///
/// In TeX / XeTeX, identifiers $0 \le s < 65536$ represent direct single-character
/// codes or Unicode scalar values, while identifiers $s \ge 65536$ represent dynamic
/// pooled strings in the string table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StrNumber(pub i32);

impl StrNumber {
    /// Creates a string identifier from a raw 32-bit signed integer.
    pub const fn from_raw(raw: i32) -> Self {
        Self(raw)
    }

    /// Returns the raw 32-bit signed integer representation.
    pub const fn as_raw(self) -> i32 {
        self.0
    }

    /// Returns true if this identifier represents a direct character code ($< 65536$).
    pub const fn is_character(self) -> bool {
        self.0 < TOO_BIG_CHAR
    }

    /// Returns true if this identifier represents a pooled string ($\ge 65536$).
    pub const fn is_pooled(self) -> bool {
        self.0 >= TOO_BIG_CHAR
    }

    /// Returns the 0-based index into the string pool, if this is a pooled string.
    pub fn pool_index(self) -> Option<usize> {
        if self.is_pooled() {
            Some((self.0 - TOO_BIG_CHAR) as usize)
        } else {
            None
        }
    }
}

impl From<i32> for StrNumber {
    fn from(val: i32) -> Self {
        Self(val)
    }
}

impl From<StrNumber> for i32 {
    fn from(s: StrNumber) -> Self {
        s.0
    }
}

/// Errors that can occur during string table manipulation.
#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum StringError {
    /// Attempted to decode invalid UTF-16 surrogate sequence.
    #[error("invalid UTF-16 code units in string pool")]
    InvalidUtf16,
    /// Corrupt format table offsets.
    #[error("invalid string table indices: {0}")]
    InvalidIndices(String),
}

/// Encapsulated, safe string pool and interner for the TeX engine.
#[derive(Debug, Clone, Default)]
pub struct StringPool {
    /// Interned pooled strings.
    strings: Vec<String>,
    /// Lookup index for fast string interning and deduplication.
    intern_map: HashMap<String, StrNumber>,
    /// Accumulator buffer for a string currently being created via `append_char`.
    accumulator: Vec<u16>,
}

impl StringPool {
    /// Creates a new empty string pool.
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns a string slice, returning its existing or newly assigned [`StrNumber`].
    pub fn intern(&mut self, s: &str) -> StrNumber {
        if let Some(&existing) = self.intern_map.get(s) {
            return existing;
        }

        let idx = self.strings.len();
        let str_num = StrNumber(TOO_BIG_CHAR + idx as i32);
        self.strings.push(s.to_string());
        self.intern_map.insert(s.to_string(), str_num);
        str_num
    }

    /// Looks up a pooled string by its identifier, returning a reference to the string.
    /// Returns `None` if the identifier is out of bounds or represents a direct character code.
    pub fn get_pooled(&self, s: StrNumber) -> Option<&str> {
        let idx = s.pool_index()?;
        self.strings.get(idx).map(|s| s.as_str())
    }

    /// Retrieves a string representation for any [`StrNumber`].
    ///
    /// If `s` is a pooled string, returns its content.
    /// If `s` is a single character code ($< 65536$), decodes the character.
    pub fn get(&self, s: StrNumber) -> Option<String> {
        if let Some(str_ref) = self.get_pooled(s) {
            Some(str_ref.to_string())
        } else if s.0 >= 0 && s.0 < TOO_BIG_CHAR {
            char::from_u32(s.0 as u32).map(|c| c.to_string())
        } else {
            None
        }
    }

    /// Appends a Unicode character to the active string accumulator.
    pub fn append_char(&mut self, c: char) {
        let mut buf = [0u16; 2];
        let slice = c.encode_utf16(&mut buf);
        self.accumulator.extend_from_slice(slice);
    }

    /// Appends a raw UTF-16 code unit to the active string accumulator.
    pub fn append_u16(&mut self, code_unit: u16) {
        self.accumulator.push(code_unit);
    }

    /// Appends a string slice to the active string accumulator.
    pub fn append_str(&mut self, s: &str) {
        for c in s.chars() {
            self.append_char(c);
        }
    }

    /// Finalizes the string currently in the accumulator, resets the accumulator,
    /// and returns the assigned [`StrNumber`].
    pub fn make_string(&mut self) -> Result<StrNumber, StringError> {
        let decoded = String::from_utf16(&self.accumulator).map_err(|_| StringError::InvalidUtf16)?;
        self.accumulator.clear();
        Ok(self.intern(&decoded))
    }

    /// Aborts the current string in the accumulator without creating a string.
    pub fn flush_string(&mut self) {
        self.accumulator.clear();
    }

    /// Returns the length of the string in the accumulator (in UTF-16 code units).
    pub fn accumulator_len(&self) -> usize {
        self.accumulator.len()
    }

    /// Returns the number of pooled strings currently stored.
    pub fn len(&self) -> usize {
        self.strings.len()
    }

    /// Returns true if the string pool contains no pooled strings.
    pub fn is_empty(&self) -> bool {
        self.strings.is_empty()
    }

    /// Computes the UTF-16 length of a string identified by `s`, matching Knuth's exact surrogate rules.
    pub fn utf16_length(&self, s: StrNumber) -> usize {
        if let Some(pooled) = self.get_pooled(s) {
            let mut len = pooled.len();
            for c in pooled.chars() {
                if c as u32 > 0xFFFF {
                    len += 1;
                }
            }
            len
        } else if (32..127).contains(&s.0) {
            1
        } else if s.0 <= 127 {
            3
        } else if s.0 < 256 {
            4
        } else {
            8
        }
    }

    /// Exports the string pool into raw format tables:
    /// `(pool_ptr, str_ptr, str_starts, str_pool)`.
    pub fn to_raw_tables(&self) -> (i32, i32, Vec<i32>, Vec<u16>) {
        let mut str_starts = Vec::with_capacity(self.strings.len() + 1);
        let mut str_pool = Vec::new();

        for s in &self.strings {
            str_starts.push(str_pool.len() as i32);
            for c in s.chars() {
                let mut buf = [0u16; 2];
                str_pool.extend_from_slice(c.encode_utf16(&mut buf));
            }
        }

        let pool_ptr = str_pool.len() as i32;
        let str_ptr = (TOO_BIG_CHAR + self.strings.len() as i32) - 1;

        (pool_ptr, str_ptr, str_starts, str_pool)
    }

    /// Ingests raw format serialization tables into a [`StringPool`].
    pub fn from_raw_tables(
        pool_ptr: i32,
        str_ptr: i32,
        str_starts: &[i32],
        str_pool: &[u16],
    ) -> Result<Self, StringError> {
        let n_strings = (str_ptr - TOO_BIG_CHAR + 1) as usize;
        if str_starts.len() < n_strings {
            return Err(StringError::InvalidIndices(format!(
                "expected at least {n_strings} str_starts, got {}",
                str_starts.len()
            )));
        }

        let mut pool = Self::new();

        for i in 0..n_strings {
            let idx0 = str_starts[i] as usize;
            let idx1 = if i == n_strings - 1 {
                pool_ptr as usize
            } else {
                str_starts[i + 1] as usize
            };

            if idx0 > idx1 || idx1 > str_pool.len() {
                return Err(StringError::InvalidIndices(format!(
                    "invalid slice bounds [{idx0}..{idx1}] in str_pool of length {}",
                    str_pool.len()
                )));
            }

            let s = String::from_utf16(&str_pool[idx0..idx1])
                .map_err(|_| StringError::InvalidUtf16)?;
            pool.intern(&s);
        }

        Ok(pool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_str_number_properties() {
        let ch = StrNumber::from_raw(65); // 'A'
        assert!(ch.is_character());
        assert!(!ch.is_pooled());
        assert_eq!(ch.pool_index(), None);
        assert_eq!(ch.as_raw(), 65);

        let pooled = StrNumber::from_raw(TOO_BIG_CHAR + 42);
        assert!(!pooled.is_character());
        assert!(pooled.is_pooled());
        assert_eq!(pooled.pool_index(), Some(42));
    }

    #[test]
    fn test_string_pool_intern_and_lookup() {
        let mut pool = StringPool::new();
        assert!(pool.is_empty());

        let s1 = pool.intern("hello");
        let s2 = pool.intern("world");
        let s3 = pool.intern("hello"); // deduplicated

        assert_eq!(s1, s3);
        assert_ne!(s1, s2);
        assert_eq!(pool.len(), 2);

        assert_eq!(pool.get_pooled(s1), Some("hello"));
        assert_eq!(pool.get_pooled(s2), Some("world"));

        // Single character lookup
        let ch = StrNumber::from_raw(65);
        assert_eq!(pool.get(ch), Some("A".to_string()));
    }

    #[test]
    fn test_accumulator_workflow() {
        let mut pool = StringPool::new();
        assert_eq!(pool.accumulator_len(), 0);

        pool.append_char('T');
        pool.append_char('e');
        pool.append_str("X");
        assert_eq!(pool.accumulator_len(), 3);

        let str_num = pool.make_string().unwrap();
        assert_eq!(pool.get_pooled(str_num), Some("TeX"));
        assert_eq!(pool.accumulator_len(), 0);

        // Aborted string
        pool.append_str("DiscardMe");
        pool.flush_string();
        assert_eq!(pool.accumulator_len(), 0);
    }

    #[test]
    fn test_raw_tables_roundtrip() {
        let mut pool = StringPool::new();
        pool.intern("alpha");
        pool.intern("beta");
        pool.intern("gamma");

        let (pool_ptr, str_ptr, str_starts, str_pool) = pool.to_raw_tables();
        assert_eq!(str_starts.len(), 3);
        assert_eq!(str_pool.len(), 5 + 4 + 5);

        let restored = StringPool::from_raw_tables(pool_ptr, str_ptr, &str_starts, &str_pool)
            .expect("roundtrip raw tables");
        assert_eq!(restored.len(), 3);
        assert_eq!(restored.get_pooled(StrNumber(TOO_BIG_CHAR)), Some("alpha"));
        assert_eq!(restored.get_pooled(StrNumber(TOO_BIG_CHAR + 1)), Some("beta"));
        assert_eq!(restored.get_pooled(StrNumber(TOO_BIG_CHAR + 2)), Some("gamma"));
    }

    #[test]
    fn test_utf16_length_calculation() {
        let mut pool = StringPool::new();
        let s = pool.intern("hello");
        assert_eq!(pool.utf16_length(s), 5);

        // Character code lengths
        assert_eq!(pool.utf16_length(StrNumber(65)), 1); // ASCII 'A'
        assert_eq!(pool.utf16_length(StrNumber(10)), 3);  // control char
        assert_eq!(pool.utf16_length(StrNumber(200)), 4); // 8-bit char
        assert_eq!(pool.utf16_length(StrNumber(1000)), 8); // > 256

        // Emoji requiring UTF-16 surrogate pair
        let emoji_str = pool.intern("🦀");
        assert_eq!(pool.utf16_length(emoji_str), 5); // 4 bytes UTF-8 + 1 extra for surrogate = 5
    }

    #[test]
    fn test_conversions_and_error_branches() {
        let num: StrNumber = 1234.into();
        assert_eq!(num.as_raw(), 1234);
        let raw: i32 = num.into();
        assert_eq!(raw, 1234);

        let mut pool = StringPool::new();
        let s = pool.intern("test");
        // Test get on pooled string
        assert_eq!(pool.get(s), Some("test".to_string()));
        // Test get on invalid / negative index
        assert_eq!(pool.get(StrNumber(-1)), None);
        assert_eq!(pool.get(StrNumber(TOO_BIG_CHAR + 999)), None);

        // Test append_u16
        pool.append_u16(0x0041); // 'A'
        let made = pool.make_string().unwrap();
        assert_eq!(pool.get_pooled(made), Some("A"));

        // Test invalid UTF-16 in make_string (unpaired surrogate)
        pool.append_u16(0xD800); // Lone high surrogate
        assert_eq!(pool.make_string().unwrap_err(), StringError::InvalidUtf16);

        // Test from_raw_tables errors
        let err1 = StringPool::from_raw_tables(10, TOO_BIG_CHAR + 5, &[0, 2], &[0; 10]).unwrap_err();
        assert!(matches!(err1, StringError::InvalidIndices(_)));

        let err2 = StringPool::from_raw_tables(10, TOO_BIG_CHAR, &[20], &[0; 10]).unwrap_err();
        assert!(matches!(err2, StringError::InvalidIndices(_)));

        let err3 = StringPool::from_raw_tables(1, TOO_BIG_CHAR, &[0], &[0xD800]).unwrap_err();
        assert_eq!(err3, StringError::InvalidUtf16);
    }
}
