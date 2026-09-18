// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Parsing and representation of Liang hyphenation patterns.
//!
//! A hyphenation pattern consists of a sequence of letters interspersed with
//! numeric hyphenation priority digits (0-9). For example:
//! - `hy3ph` -> characters "hyph", weights [0, 0, 3, 0, 0]
//! - `.hy1`  -> characters ".hy",  weights [0, 0, 0, 1]
//! - `4tion.` -> characters "tion.", weights [4, 0, 0, 0, 0]

/// A parsed hyphenation pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    /// The normalized characters of the pattern (including optional leading/trailing dot `.`).
    pub text: String,
    /// The hyphenation weights at each position (length is `text.chars().count() + 1`).
    pub weights: Vec<u8>,
}

impl Pattern {
    /// Parses a pattern string (e.g., `hy3ph`, `.a5b`, `4tion.`).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        let mut text = String::with_capacity(s.len());
        let mut weights = Vec::with_capacity(s.len() + 1);
        let mut current_weight: u8 = 0;

        for c in s.chars() {
            if c.is_ascii_digit() {
                current_weight = c.to_digit(10)? as u8;
            } else {
                weights.push(current_weight);
                current_weight = 0;
                text.push(c.to_ascii_lowercase());
            }
        }
        weights.push(current_weight);

        Some(Self { text, weights })
    }
}

/// Parses a block of patterns separated by whitespace (such as inside `\patterns{...}`).
pub fn parse_patterns(input: &str) -> impl Iterator<Item = Pattern> + '_ {
    input
        .lines()
        .map(|line| {
            if let Some(pos) = line.find('%') {
                &line[..pos]
            } else {
                line
            }
        })
        .flat_map(str::split_whitespace)
        .filter_map(Pattern::parse)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_parse_basic() {
        let p = Pattern::parse("hy3ph").unwrap();
        assert_eq!(p.text, "hyph");
        assert_eq!(p.weights, vec![0, 0, 3, 0, 0]);

        let p2 = Pattern::parse(".hy1").unwrap();
        assert_eq!(p2.text, ".hy");
        assert_eq!(p2.weights, vec![0, 0, 0, 1]);

        let p3 = Pattern::parse("4tion.").unwrap();
        assert_eq!(p3.text, "tion.");
        assert_eq!(p3.weights, vec![4, 0, 0, 0, 0, 0]);

        let p_mixed = Pattern::parse("1a2b3").unwrap();
        assert_eq!(p_mixed.text, "ab");
        assert_eq!(p_mixed.weights, vec![1, 2, 3]);
    }

    #[test]
    fn test_pattern_parse_empty() {
        assert_eq!(Pattern::parse(""), None);
        assert_eq!(Pattern::parse("   "), None);
    }

    #[test]
    fn test_parse_patterns_block() {
        let block = "
            % Sample patterns
            .ach4
            4ch.
            hy3ph
            1a2b3
        ";
        let patterns: Vec<Pattern> = parse_patterns(block).collect();
        assert_eq!(patterns.len(), 4);
        assert_eq!(patterns[0].text, ".ach");
        assert_eq!(patterns[0].weights, vec![0, 0, 0, 0, 4]);
        assert_eq!(patterns[2].text, "hyph");
        assert_eq!(patterns[2].weights, vec![0, 0, 3, 0, 0]);
    }
}
