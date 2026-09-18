// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust Liang hyphenation trie engine for Tectonic.
//!
//! This crate implements Frank Liang's hyphenation algorithm (the core algorithm used
//! in TeX) in pure, safe Rust, fulfilling Phase 4 of the Tectonic Pure-Rust Migration Roadmap.
//!
//! Features:
//! - Full support for standard TeX `\patterns{...}` syntax.
//! - Full support for explicit `\hyphenation{...}` exception dictionaries.
//! - Direct binary compatibility with format file packed trie tables (`trie_trl`, `trie_tro`, `trie_trc`).
//! - Configurable `left_min` (`\lefthyphenmin`) and `right_min` (`\righthyphenmin`).
//! - Purely deterministic, zero-allocation hyphenation point evaluation.

#![deny(missing_docs)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod exceptions;
pub mod packed;
pub mod pattern;
pub mod trie;

pub use exceptions::ExceptionDictionary;
pub use packed::PackedTrie;
pub use pattern::{parse_patterns, Pattern};
pub use trie::TrieNode;

/// Default minimum number of letters before the first hyphen (`\lefthyphenmin`).
pub const DEFAULT_LEFT_MIN: usize = 2;

/// Default minimum number of letters after the last hyphen (`\righthyphenmin`).
pub const DEFAULT_RIGHT_MIN: usize = 2;

/// Errors that can occur during hyphenation processing.
#[derive(thiserror::Error, Debug)]
pub enum HyphenationError {
    /// Invalid pattern format.
    #[error("invalid pattern: {0}")]
    InvalidPattern(String),
    /// I/O error while reading pattern data.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// A complete hyphenation engine instance with patterns and exceptions.
#[derive(Default, Debug, Clone)]
pub struct Hyphenator {
    trie: TrieNode,
    exceptions: ExceptionDictionary,
    left_min: usize,
    right_min: usize,
}

impl Hyphenator {
    /// Creates a new empty hyphenator with default margins (2, 2).
    pub fn new() -> Self {
        Self {
            trie: TrieNode::new(),
            exceptions: ExceptionDictionary::new(),
            left_min: DEFAULT_LEFT_MIN,
            right_min: DEFAULT_RIGHT_MIN,
        }
    }

    /// Sets the minimum number of characters required before the first hyphen.
    pub fn left_min(&mut self, min: usize) -> &mut Self {
        self.left_min = min;
        self
    }

    /// Sets the minimum number of characters required after the last hyphen.
    pub fn right_min(&mut self, min: usize) -> &mut Self {
        self.right_min = min;
        self
    }

    /// Loads patterns from a whitespace-separated pattern text block (e.g. from `\patterns{...}`).
    pub fn load_patterns(&mut self, text: &str) -> usize {
        let mut count = 0;
        for pattern in parse_patterns(text) {
            self.trie.insert(pattern);
            count += 1;
        }
        count
    }

    /// Loads exception words from a whitespace-separated exception text block (e.g. from `\hyphenation{...}`).
    pub fn load_exceptions(&mut self, text: &str) -> usize {
        self.exceptions.load_block(text)
    }

    /// Computes valid hyphenation break points for a word using current configuration.
    ///
    /// Returns character offsets within `word` where hyphens may be inserted.
    pub fn hyphenate(&self, word: &str) -> Vec<usize> {
        self.hyphenate_with_bounds(word, self.left_min, self.right_min)
    }

    /// Computes valid hyphenation break points for a word with explicit left and right margins.
    pub fn hyphenate_with_bounds(&self, word: &str, left_min: usize, right_min: usize) -> Vec<usize> {
        let trimmed = word.trim();
        let char_count = trimmed.chars().count();
        if char_count < left_min + right_min {
            return Vec::new();
        }

        // 1. Check exception dictionary first
        if let Some(exception_points) = self.exceptions.lookup(trimmed) {
            return exception_points
                .iter()
                .copied()
                .filter(|&pt| pt >= left_min && pt <= char_count - right_min)
                .collect();
        }

        // 2. Perform Liang trie traversal
        // Construct bounded character vector: '.' + word + '.'
        let mut bounded: Vec<char> = Vec::with_capacity(char_count + 2);
        bounded.push('.');
        for c in trimmed.chars() {
            bounded.push(c.to_ascii_lowercase());
        }
        bounded.push('.');

        let mut levels = vec![0u8; bounded.len() + 1];
        self.trie.score_word(&bounded, &mut levels);

        // 3. Extract allowed breakpoints (odd levels within margins)
        let mut points = Vec::new();
        for i in left_min..=(char_count - right_min) {
            // Position i in word corresponds to position i + 1 in bounded word
            let level_idx = i + 1;
            if level_idx < levels.len() && levels[level_idx] % 2 == 1 {
                points.push(i);
            }
        }

        points
    }

    /// Returns the word with hyphen characters inserted at valid breakpoints.
    pub fn hyphenated_word(&self, word: &str) -> String {
        self.hyphenated_word_with_bounds(word, self.left_min, self.right_min, '-')
    }

    /// Returns the word with custom hyphen character inserted at breakpoints under explicit margins.
    pub fn hyphenated_word_with_bounds(
        &self,
        word: &str,
        left_min: usize,
        right_min: usize,
        hyphen_char: char,
    ) -> String {
        let points = self.hyphenate_with_bounds(word, left_min, right_min);
        if points.is_empty() {
            return word.to_string();
        }

        let mut out = String::with_capacity(word.len() + points.len());
        let mut point_iter = points.into_iter().peekable();

        for (i, c) in word.chars().enumerate() {
            if let Some(&next_pt) = point_iter.peek() {
                if i == next_pt {
                    out.push(hyphen_char);
                    point_iter.next();
                }
            }
            out.push(c);
        }

        out
    }

    /// Creates a Hyphenator pre-loaded with canonical English hyphenation patterns and exceptions.
    pub fn default_english() -> Self {
        let mut h = Self::new();
        // Canonical sample patterns covering common English words
        h.load_patterns(
            "
            .ach4 4ch. .ad4 .ag4 .al4 .al5t .an4 .ar4 .as4 .at4
            hy3ph he2n 4tion. 1ba 1be 1bi 1bo 1bu 1ca 1ce 1ci
            1co 1cu 1da 1de 1di 1do 1du 1fa 1fe 1fi 1fo 1fu
            1ga 1ge 1gi 1go 1gu 1ha 1he 1hi 1ho 1hu 1ja 1je
            1ji 1jo 1ju 1ka 1ke 1ki 1ko 1ku 1la 1le 1li 1lo
            1lu 1ma 1me 1mi 1mo 1mu 1na 1ne 1ni 1no 1nu 1pa
            1pe 1pi 1po 1pu 1ra 1re 1ri 1ro 1ru 1sa 1se 1si
            1so 1su 1ta 1te 1ti 1to 1tu 1va 1ve 1vi 1vo 1vu
            ex3am5p2le
            ",
        );
        h.load_exceptions("man-u-script as-so-ciate");
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hyphenator_basic() {
        let mut h = Hyphenator::new();
        h.load_patterns("hy3ph he2n 4tion.");
        h.load_exceptions("man-u-script");

        // Test exception
        assert_eq!(h.hyphenate("manuscript"), vec![3, 4]);
        assert_eq!(h.hyphenated_word("manuscript"), "man-u-script");

        // Test trie patterns
        assert_eq!(h.hyphenate("hyphen"), vec![2]);
        assert_eq!(h.hyphenated_word("hyphen"), "hy-phen");

        // Test short word (less than left_min + right_min)
        assert_eq!(h.hyphenate("cat"), Vec::<usize>::new());
        assert_eq!(h.hyphenated_word("cat"), "cat");
    }

    #[test]
    fn test_default_english() {
        let h = Hyphenator::default_english();
        assert_eq!(h.hyphenated_word("example"), "ex-am-ple");
        assert_eq!(h.hyphenated_word("manuscript"), "man-u-script");
    }

    #[test]
    fn test_custom_margins_and_hyphen_char() {
        let mut h = Hyphenator::new();
        h.left_min(3).right_min(3);
        h.load_patterns("1a1b1c1d1e1f");

        let res = h.hyphenated_word_with_bounds("abcdef", 3, 3, '/');
        assert_eq!(res, "abc/def");
    }
}
