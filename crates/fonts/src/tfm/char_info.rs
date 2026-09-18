// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! TFM character info word representation and unpacking.

/// Character tag specifying special handling for ligatures, math lists, or delimiters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharTag {
    /// Normal character without ligature, list, or extensible recipe.
    Vanilla = 0,
    /// Remainder indicates the starting index in the ligature/kerning table.
    LigKern = 1,
    /// Remainder indicates the character code of the next larger variant in a list.
    List = 2,
    /// Remainder indicates the index in the extensible character table.
    Extensible = 3,
}

impl CharTag {
    /// Creates a `CharTag` from the 2-bit tag field.
    pub fn from_u8(val: u8) -> Self {
        match val & 0x03 {
            1 => Self::LigKern,
            2 => Self::List,
            3 => Self::Extensible,
            _ => Self::Vanilla,
        }
    }
}

/// Unpacked character metric and program pointers from a TFM `char_info_word`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharInfo {
    /// Index into the font's width table.
    pub width_index: u8,
    /// Index into the font's height table (0..15).
    pub height_index: u8,
    /// Index into the font's depth table (0..15).
    pub depth_index: u8,
    /// Index into the font's italic correction table (0..63).
    pub italic_index: u8,
    /// Tag specifying the interpretation of `remainder`.
    pub tag: CharTag,
    /// Remainder byte (meaning depends on `tag`).
    pub remainder: u8,
}

impl CharInfo {
    /// Parses a 4-byte `char_info_word` from TFM data.
    pub fn from_bytes(bytes: [u8; 4]) -> Self {
        let width_index = bytes[0];
        let height_index = (bytes[1] >> 4) & 0x0F;
        let depth_index = bytes[1] & 0x0F;
        let italic_index = (bytes[2] >> 2) & 0x3F;
        let tag = CharTag::from_u8(bytes[2]);
        let remainder = bytes[3];

        Self {
            width_index,
            height_index,
            depth_index,
            italic_index,
            tag,
            remainder,
        }
    }

    /// Returns `true` if this character actually exists in the font (non-zero width index).
    pub fn exists(&self) -> bool {
        self.width_index > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_info_unpacking() {
        // width=5, height=2, depth=3, italic=7, tag=1 (LigKern), remainder=42
        // byte 0: 5
        // byte 1: (2 << 4) | 3 = 0x23
        // byte 2: (7 << 2) | 1 = 28 | 1 = 29 (0x1D)
        // byte 3: 42 (0x2A)
        let bytes = [5, 0x23, 0x1D, 42];
        let info = CharInfo::from_bytes(bytes);

        assert_eq!(info.width_index, 5);
        assert_eq!(info.height_index, 2);
        assert_eq!(info.depth_index, 3);
        assert_eq!(info.italic_index, 7);
        assert_eq!(info.tag, CharTag::LigKern);
        assert_eq!(info.remainder, 42);
        assert!(info.exists());
    }

    #[test]
    fn test_char_info_empty() {
        let info = CharInfo::from_bytes([0, 0, 0, 0]);
        assert_eq!(info.width_index, 0);
        assert_eq!(info.tag, CharTag::Vanilla);
        assert!(!info.exists());
    }

    #[test]
    fn test_char_tags() {
        assert_eq!(CharTag::from_u8(0), CharTag::Vanilla);
        assert_eq!(CharTag::from_u8(1), CharTag::LigKern);
        assert_eq!(CharTag::from_u8(2), CharTag::List);
        assert_eq!(CharTag::from_u8(3), CharTag::Extensible);
    }
}
