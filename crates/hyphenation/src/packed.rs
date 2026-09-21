// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Packed trie representation matching Knuth's exact TeX/XeTeX format serialization tables.
//!
//! In Web2C / XeTeX, the hyphenation trie is flattened and packed into parallel arrays:
//! - `trie_trl`: Base pointers and links
//! - `trie_tro`: Op indices
//! - `trie_trc`: Character codes
//! - `hyf_distance`, `hyf_num`, `hyf_next`: Linked lists of hyphenation operations

/// Sentinel value indicating empty trie operation.
pub const MIN_TRIE_OP: i32 = 0;

/// Packed trie tables matching TeX format structure.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackedTrie {
    /// Link/base pointers.
    pub trl: Vec<i32>,
    /// Operation pointers.
    pub tro: Vec<i32>,
    /// Character values.
    pub trc: Vec<u16>,
    /// Base offsets for each language's operations.
    pub op_start: Vec<i32>,
    /// Operation distance back from current position.
    pub hyf_distance: Vec<i16>,
    /// Operation hyphenation number/weight.
    pub hyf_num: Vec<i16>,
    /// Pointer to next operation in linked list.
    pub hyf_next: Vec<u16>,
}

impl PackedTrie {
    /// Evaluates hyphenation breakpoints for a sequence of character codes `chars`
    /// in language `cur_lang` with specified minimum left and right margins.
    pub fn hyphenate(
        &self,
        cur_lang: usize,
        chars: &[u16],
        left_min: usize,
        right_min: usize,
    ) -> Vec<usize> {
        let hn = chars.len();
        if hn < left_min + right_min {
            return Vec::new();
        }

        // Check if language is supported in trie
        if cur_lang + 1 >= self.trc.len() || self.trc[cur_lang + 1] as usize != cur_lang {
            return Vec::new();
        }

        // Construct 1-based hc array with boundary zeros: hc[0]=0, hc[1..=hn]=chars, hc[hn+1]=0
        let mut hc = Vec::with_capacity(hn + 3);
        hc.push(0u16);
        hc.extend_from_slice(chars);
        hc.push(0u16);
        hc.push(u16::MAX); // max_hyph_char

        let mut hyf = vec![0i16; hn + 1];

        // Traverse trie for each starting offset j
        let r_bound = if hn >= right_min {
            hn - right_min + 1
        } else {
            0
        };
        for j in 0..=r_bound {
            let base = cur_lang + 1;
            if base >= self.trl.len() {
                continue;
            }
            let mut z = (self.trl[base] as isize + hc[j] as isize) as usize;
            let mut l = j;

            while z < self.trc.len() && hc[l] == self.trc[z] {
                if z < self.tro.len() && self.tro[z] != MIN_TRIE_OP {
                    let mut v = self.tro[z];
                    while v != MIN_TRIE_OP {
                        let op_base = if cur_lang < self.op_start.len() {
                            self.op_start[cur_lang]
                        } else {
                            0
                        };
                        let op_idx = (v + op_base) as usize;
                        if op_idx < self.hyf_distance.len() && op_idx < self.hyf_num.len() {
                            let dist = self.hyf_distance[op_idx] as usize;
                            if l >= dist {
                                let i = l - dist;
                                if i < hyf.len() && self.hyf_num[op_idx] > hyf[i] {
                                    hyf[i] = self.hyf_num[op_idx];
                                }
                            }
                            v = if op_idx < self.hyf_next.len() {
                                self.hyf_next[op_idx] as i32
                            } else {
                                MIN_TRIE_OP
                            };
                        } else {
                            break;
                        }
                    }
                }

                l += 1;
                if l >= hc.len() || z >= self.trl.len() {
                    break;
                }
                z = (self.trl[z] as isize + hc[l] as isize) as usize;
            }
        }

        // Apply left and right hyphen margins
        for val in hyf.iter_mut().take(left_min) {
            *val = 0;
        }
        for j in 0..right_min {
            if j < hyf.len() {
                hyf[hn - j] = 0;
            }
        }

        // Extract odd values as allowed hyphenation points (1-based offsets in word)
        let mut points = Vec::new();
        for (i, &val) in hyf.iter().enumerate().skip(1) {
            if val % 2 == 1 {
                points.push(i);
            }
        }

        points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packed_trie_empty() {
        let trie = PackedTrie::default();
        let points = trie.hyphenate(0, &[104, 121, 112, 104, 101, 110], 2, 2);
        assert!(points.is_empty());

        // Test short word margin rejection
        let points_short = trie.hyphenate(0, &[104, 105], 2, 2);
        assert!(points_short.is_empty());
    }

    #[test]
    fn test_packed_trie_execution() {
        // Build a minimal packed trie for language 0
        let mut trie = PackedTrie::default();
        let max_idx = 300;
        trie.trl = vec![0; max_idx];
        trie.tro = vec![0; max_idx];
        trie.trc = vec![u16::MAX; max_idx];
        trie.op_start = vec![0; 2];
        trie.hyf_distance = vec![0, 1]; // op 1: distance 1
        trie.hyf_num = vec![0, 3]; // op 1: weight 3 (odd)
        trie.hyf_next = vec![0, 0];

        // Language 0 entry at index 1 (cur_lang + 1)
        trie.trc[1] = 0; // matches cur_lang = 0
        trie.trl[1] = 10;

        // At starting offset j = 1, hc[1] = 'h' (104)
        // z = trl[1] + hc[1] = 10 + 104 = 114
        trie.trc[114] = 104; // matches 'h'
        trie.trl[114] = 50; // next base is 50

        // hc[2] = 'y' (121)
        // z = trl[114] + hc[2] = 50 + 121 = 171
        trie.trc[171] = 121; // matches 'y'
        trie.tro[171] = 1; // triggers op 1 at l = 2 (i = 2 - 1 = 1)
        trie.trl[171] = 0;

        // Word: "hyphen" -> [104, 121, 112, 104, 101, 110]
        let word = [104u16, 121, 112, 104, 101, 110];
        // With left_min = 1, right_min = 1: offset 1 is allowed
        let points = trie.hyphenate(0, &word, 1, 1);
        assert_eq!(points, vec![1]);

        // With left_min = 2: offset 1 is masked out
        let points_masked = trie.hyphenate(0, &word, 2, 1);
        assert!(points_masked.is_empty());
    }
}
