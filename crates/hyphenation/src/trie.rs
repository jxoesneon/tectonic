// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Trie data structure for Liang hyphenation pattern matching.

use crate::pattern::Pattern;
use std::collections::HashMap;

/// A node in the hyphenation pattern trie.
#[derive(Default, Debug, Clone)]
pub struct TrieNode {
    children: HashMap<char, TrieNode>,
    weights: Option<Vec<u8>>,
}

impl TrieNode {
    /// Creates a new empty trie root node.
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a pattern into the trie.
    pub fn insert(&mut self, pattern: Pattern) {
        let mut curr = self;
        for c in pattern.text.chars() {
            curr = curr.children.entry(c).or_default();
        }
        curr.weights = Some(pattern.weights);
    }

    /// Scans a bounded word (e.g. `.word.`) and populates the hyphenation level vector.
    ///
    /// `chars`: characters of the bounded word.
    /// `levels`: output array of length `chars.len() + 1`, initialized to zeros.
    pub fn score_word(&self, chars: &[char], levels: &mut [u8]) {
        let n = chars.len();
        for start in 0..n {
            let mut curr = self;
            for offset in 0..(n - start) {
                let c = chars[start + offset];
                if let Some(next) = curr.children.get(&c) {
                    curr = next;
                    if let Some(ref weights) = curr.weights {
                        for (k, &w) in weights.iter().enumerate() {
                            let idx = start + k;
                            if idx < levels.len() && w > levels[idx] {
                                levels[idx] = w;
                            }
                        }
                    }
                } else {
                    break;
                }
            }
        }
    }

    /// Recursively counts the total number of pattern nodes in the trie.
    pub fn count_patterns(&self) -> usize {
        let here = if self.weights.is_some() { 1 } else { 0 };
        let sub: usize = self.children.values().map(|c| c.count_patterns()).sum();
        here + sub
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trie_insert_and_score() {
        let mut trie = TrieNode::new();
        trie.insert(Pattern::parse("hy3ph").unwrap());
        trie.insert(Pattern::parse("he2n").unwrap());

        assert_eq!(trie.count_patterns(), 2);

        // Word: "hyphen", bounded: ".hyphen."
        let chars: Vec<char> = ".hyphen.".chars().collect();
        let mut levels = vec![0u8; chars.len() + 1];
        trie.score_word(&chars, &mut levels);

        // "hy3ph" starts at index 1: weights at 1..=5: [0, 0, 3, 0, 0] -> levels[3] = 3
        // Position 3 in bounded word corresponds to after 2nd letter ('y') -> "hy-phen"
        assert_eq!(levels[3], 3);
        // "he2n" starts at index 4 ('h'): weights [0, 0, 2, 0] -> levels[6] = 2
        assert_eq!(levels[6], 2);
    }
}
