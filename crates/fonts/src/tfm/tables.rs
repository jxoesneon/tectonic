// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Auxiliary tables for TFM files: FixWords, ligatures, kerning, extensible recipes, and font parameters.

use crate::tfm::scaler::TfmScaler;

/// A 32-bit signed fixed-point number used in TFM files with 20 fractional bits ($1.0 = 2^{20} = 1,048,576$).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct FixWord(pub i32);

impl FixWord {
    /// Zero value.
    pub const ZERO: Self = Self(0);

    /// Fixed-point unity (1.0 = $2^{20}$).
    pub const ONE: Self = Self(1 << 20);

    /// Scales this `FixWord` into scaled points (`sp`) using Knuth's exact integer arithmetic.
    pub fn scale(&self, scaler: &TfmScaler) -> i32 {
        scaler.scale_fix_word(self.0)
    }

    /// Converts this `FixWord` to a floating point value for non-layout inspection.
    pub fn to_f64(&self) -> f64 {
        (self.0 as f64) / ((1 << 20) as f64)
    }
}

/// A step in the ligature and kerning program table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LigKernStep {
    /// Skip byte. If `> 128`, indicates stop or jump.
    pub skip_byte: u8,
    /// Next character code to match.
    pub next_char: u8,
    /// Operation byte. If `>= 128`, this is a kerning step; otherwise ligature.
    pub op_byte: u8,
    /// Remainder byte pointing to kern table or ligature result character.
    pub remainder: u8,
}

impl LigKernStep {
    /// Parses a `LigKernStep` from 4 bytes.
    pub fn from_bytes(bytes: [u8; 4]) -> Self {
        Self {
            skip_byte: bytes[0],
            next_char: bytes[1],
            op_byte: bytes[2],
            remainder: bytes[3],
        }
    }

    /// Returns `true` if this step represents a kerning operation rather than a ligature.
    pub fn is_kern(&self) -> bool {
        self.op_byte >= 128
    }

    /// Returns the kern table index if this is a kerning step.
    pub fn kern_index(&self) -> Option<usize> {
        if self.is_kern() {
            Some((((self.op_byte - 128) as usize) << 8) | (self.remainder as usize))
        } else {
            None
        }
    }

    /// Returns the replacement ligature character if this is a standard ligature.
    pub fn ligature_char(&self) -> Option<u8> {
        if !self.is_kern() {
            Some(self.remainder)
        } else {
            None
        }
    }

    /// Returns `true` if this is the final step in the lig/kern program for the current character.
    pub fn is_stop(&self) -> bool {
        self.skip_byte >= 128
    }
}

/// Recipe for building large extensible delimiters or math operators from pieces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtensibleRecipe {
    /// Top character code (0 if absent).
    pub top: u8,
    /// Middle character code (0 if absent).
    pub mid: u8,
    /// Bottom character code (0 if absent).
    pub bot: u8,
    /// Repeated character code used to extend the delimiter.
    pub rep: u8,
}

impl ExtensibleRecipe {
    /// Parses an `ExtensibleRecipe` from 4 bytes.
    pub fn from_bytes(bytes: [u8; 4]) -> Self {
        Self {
            top: bytes[0],
            mid: bytes[1],
            bot: bytes[2],
            rep: bytes[3],
        }
    }
}

/// Standard TeX font parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontParams {
    /// Parameter 1: Slant per point (FixWord ratio).
    pub slant: FixWord,
    /// Parameter 2: Interword space.
    pub space: FixWord,
    /// Parameter 3: Interword space stretchability.
    pub space_stretch: FixWord,
    /// Parameter 4: Interword space shrinkability.
    pub space_shrink: FixWord,
    /// Parameter 5: x-height (height of lowercase 'x').
    pub x_height: FixWord,
    /// Parameter 6: quad width (width of 1 em).
    pub quad: FixWord,
    /// Parameter 7: Extra space added after punctuation.
    pub extra_space: FixWord,
    /// Additional parameters for math symbol and math extension fonts (parameters 8..=np).
    pub additional: Vec<FixWord>,
}

impl FontParams {
    /// Parses font parameters from a slice of `FixWord` values.
    pub fn from_words(words: &[FixWord]) -> Self {
        let get = |idx: usize| -> FixWord {
            words.get(idx).copied().unwrap_or(FixWord::ZERO)
        };

        Self {
            slant: get(0),
            space: get(1),
            space_stretch: get(2),
            space_shrink: get(3),
            x_height: get(4),
            quad: get(5),
            extra_space: get(6),
            additional: if words.len() > 7 {
                words[7..].to_vec()
            } else {
                Vec::new()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fix_word() {
        assert_eq!(FixWord::ZERO.0, 0);
        assert_eq!(FixWord::ONE.0, 1048576);
        assert_eq!(FixWord::ONE.to_f64(), 1.0);

        let scaler = TfmScaler::new(655360);
        assert_eq!(FixWord::ONE.scale(&scaler), 655360);
    }

    #[test]
    fn test_lig_kern_step() {
        // Kern step: op_byte = 129, remainder = 5 -> kern index = (1 * 256) + 5 = 261
        let kern_step = LigKernStep::from_bytes([0, b'e', 129, 5]);
        assert!(kern_step.is_kern());
        assert_eq!(kern_step.kern_index(), Some(261));
        assert_eq!(kern_step.ligature_char(), None);
        assert!(!kern_step.is_stop());

        // Ligature step: op_byte = 0, remainder = b'f' (ff ligature)
        let lig_step = LigKernStep::from_bytes([130, b'i', 0, 0x0B]); // fi ligature, stop
        assert!(!lig_step.is_kern());
        assert_eq!(lig_step.kern_index(), None);
        assert_eq!(lig_step.ligature_char(), Some(0x0B));
        assert!(lig_step.is_stop());
    }

    #[test]
    fn test_extensible_recipe() {
        let recipe = ExtensibleRecipe::from_bytes([0x20, 0x21, 0x22, 0x23]);
        assert_eq!(recipe.top, 0x20);
        assert_eq!(recipe.mid, 0x21);
        assert_eq!(recipe.bot, 0x22);
        assert_eq!(recipe.rep, 0x23);
    }

    #[test]
    fn test_font_params() {
        let words = vec![
            FixWord(0),
            FixWord(100),
            FixWord(50),
            FixWord(25),
            FixWord(400),
            FixWord(1000),
            FixWord(120),
            FixWord(888), // Additional math param
        ];
        let params = FontParams::from_words(&words);
        assert_eq!(params.space, FixWord(100));
        assert_eq!(params.quad, FixWord(1000));
        assert_eq!(params.additional.len(), 1);
        assert_eq!(params.additional[0], FixWord(888));
    }
}
