// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Exception dictionary handling for explicit `\hyphenation{...}` rules.

use std::collections::HashMap;

/// A dictionary of explicit hyphenation exceptions.
#[derive(Default, Debug, Clone)]
pub struct ExceptionDictionary {
    exceptions: HashMap<String, Vec<usize>>,
}

impl ExceptionDictionary {
    /// Creates a new empty exception dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a hyphenated word exception (e.g., `man-u-script` or `as-so-ciate`).
    ///
    /// The word is normalized to lowercase with hyphens stripped as the key,
    /// and the character indices immediately preceding each hyphen are stored.
    pub fn insert(&mut self, hyphenated_word: &str) {
        let trimmed = hyphenated_word.trim();
        if trimmed.is_empty() {
            return;
        }

        let mut key = String::with_capacity(trimmed.len());
        let mut breakpoints = Vec::new();
        let mut char_count = 0;

        for c in trimmed.chars() {
            if c == '-' {
                if char_count > 0 {
                    breakpoints.push(char_count);
                }
            } else {
                key.push(c.to_ascii_lowercase());
                char_count += 1;
            }
        }

        if !key.is_empty() {
            self.exceptions.insert(key, breakpoints);
        }
    }

    /// Looks up a word in the exception dictionary.
    /// Returns the allowed hyphenation character offsets if present.
    pub fn lookup(&self, word: &str) -> Option<&[usize]> {
        let lower = word.to_ascii_lowercase();
        self.exceptions.get(&lower).map(|v| v.as_slice())
    }

    /// Loads exceptions from a whitespace-separated block of text (e.g. from `\hyphenation{...}`).
    pub fn load_block(&mut self, text: &str) -> usize {
        let mut count = 0;
        for line in text.lines() {
            let line_clean = if let Some(pos) = line.find('%') {
                &line[..pos]
            } else {
                line
            };
            for token in line_clean.split_whitespace() {
                self.insert(token);
                count += 1;
            }
        }
        count
    }

    /// Returns the number of exceptions in the dictionary.
    pub fn len(&self) -> usize {
        self.exceptions.len()
    }

    /// Returns true if the dictionary contains no exceptions.
    pub fn is_empty(&self) -> bool {
        self.exceptions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exception_insert_and_lookup() {
        let mut dict = ExceptionDictionary::new();
        assert!(dict.is_empty());

        dict.insert("man-u-script");
        assert_eq!(dict.len(), 1);

        let points = dict.lookup("manuscript").unwrap();
        // m-a-n (3) - u (4) - script
        assert_eq!(points, &[3, 4]);

        assert_eq!(dict.lookup("other"), None);
    }

    #[test]
    fn test_exception_case_insensitivity() {
        let mut dict = ExceptionDictionary::new();
        dict.insert("As-So-Ciate");
        assert_eq!(dict.lookup("associate").unwrap(), &[2, 4]);
        assert_eq!(dict.lookup("ASSOCIATE").unwrap(), &[2, 4]);
    }

    #[test]
    fn test_exception_load_block() {
        let mut dict = ExceptionDictionary::new();
        let block = "
            % Comments
            pro-ject
            com-pu-ter
        ";
        let count = dict.load_block(block);
        assert_eq!(count, 2);
        assert_eq!(dict.lookup("project").unwrap(), &[3]);
        assert_eq!(dict.lookup("computer").unwrap(), &[3, 5]);
    }

    #[test]
    fn test_exception_empty_strings() {
        let mut dict = ExceptionDictionary::new();
        dict.insert("");
        dict.insert("   ");
        dict.insert("-");
        assert!(dict.is_empty());
    }
}
