// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! TFM header representation and parsing.

use crate::error::TfmError;
use byteorder::{BigEndian, ByteOrder};

/// Parsed header of a TeX Font Metric (TFM) file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TfmHeader {
    /// Font checksum used to verify consistency across TeX, DVI, and driver files.
    pub checksum: u32,
    /// Design size of the font in scaled points (`sp`), where $1\,\text{pt} = 65536\,\text{sp}$.
    pub design_size_sp: i32,
    /// Character coding scheme (e.g., "TeX text", "TeX math symbols").
    pub coding_scheme: Option<String>,
    /// Font family identifier (e.g., "CMR", "CMBX").
    pub font_family: Option<String>,
    /// Seven-bit safe flag (true if character codes are strictly 0..=127).
    pub seven_bit_safe: bool,
    /// Raw additional header words if `lh > 18`.
    pub extra_words: Vec<u32>,
}

impl TfmHeader {
    /// Parses the TFM header from a byte slice of length `lh * 4`.
    pub fn parse(header_bytes: &[u8], lh: usize) -> Result<Self, TfmError> {
        if lh < 2 {
            return Err(TfmError::InvalidHeaderLength { lh });
        }
        if header_bytes.len() < lh * 4 {
            return Err(TfmError::TruncatedHeader);
        }

        let checksum = BigEndian::read_u32(&header_bytes[0..4]);

        // Word 1: Design size (FixWord with 20-bit fraction, converted to 16-bit sp by / 16)
        let raw_design = BigEndian::read_u32(&header_bytes[4..8]);
        let design_size_sp = (raw_design >> 4) as i32;

        if design_size_sp < 65536 {
            return Err(TfmError::InvalidDesignSize {
                size: design_size_sp,
            });
        }

        let mut coding_scheme = None;
        if lh >= 12 {
            // Words 2..=11: 40 bytes. First byte is BCPL length (max 39)
            let bcpl_slice = &header_bytes[8..48];
            let len = bcpl_slice[0] as usize;
            if len > 0 && len < 40 {
                let s = String::from_utf8_lossy(&bcpl_slice[1..=len])
                    .trim()
                    .to_string();
                if !s.is_empty() {
                    coding_scheme = Some(s);
                }
            }
        }

        let mut font_family = None;
        if lh >= 17 {
            // Words 12..=16: 20 bytes. First byte is BCPL length (max 19)
            let bcpl_slice = &header_bytes[48..68];
            let len = bcpl_slice[0] as usize;
            if len > 0 && len < 20 {
                let s = String::from_utf8_lossy(&bcpl_slice[1..=len])
                    .trim()
                    .to_string();
                if !s.is_empty() {
                    font_family = Some(s);
                }
            }
        }

        let mut seven_bit_safe = false;
        if lh >= 18 {
            let flag_word = BigEndian::read_i32(&header_bytes[68..72]);
            seven_bit_safe = flag_word >= 0;
        }

        let mut extra_words = Vec::new();
        if lh > 18 {
            for i in 18..lh {
                extra_words.push(BigEndian::read_u32(&header_bytes[i * 4..(i + 1) * 4]));
            }
        }

        Ok(Self {
            checksum,
            design_size_sp,
            coding_scheme,
            font_family,
            seven_bit_safe,
            extra_words,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_parse_minimal() {
        let mut buf = vec![0u8; 8];
        BigEndian::write_u32(&mut buf[0..4], 0x12345678);
        // Design size: 10pt = 10 * 2^20 FixWord = 10485760 = 0x00A00000
        // (0x00A00000 >> 4) = 0x000A0000 = 655360 sp
        BigEndian::write_u32(&mut buf[4..8], 0x00A0_0000);

        let header = TfmHeader::parse(&buf, 2).expect("valid header");
        assert_eq!(header.checksum, 0x12345678);
        assert_eq!(header.design_size_sp, 655360);
        assert_eq!(header.coding_scheme, None);
        assert_eq!(header.font_family, None);
        assert!(!header.seven_bit_safe);
    }

    #[test]
    fn test_header_invalid_length() {
        assert_eq!(
            TfmHeader::parse(&[], 1),
            Err(TfmError::InvalidHeaderLength { lh: 1 })
        );
    }

    #[test]
    fn test_header_invalid_design_size() {
        let mut buf = vec![0u8; 8];
        // 0.5 pt = 32768 sp < 65536
        BigEndian::write_u32(&mut buf[4..8], 0x0008_0000);
        assert!(matches!(
            TfmHeader::parse(&buf, 2),
            Err(TfmError::InvalidDesignSize { .. })
        ));
    }

    #[test]
    fn test_header_truncated_slice() {
        let buf = vec![0u8; 8];
        assert_eq!(TfmHeader::parse(&buf, 3), Err(TfmError::TruncatedHeader));
    }

    #[test]
    fn test_header_full_with_extra_words() {
        let mut buf = vec![0u8; 20 * 4];
        BigEndian::write_u32(&mut buf[0..4], 0x11223344);
        BigEndian::write_u32(&mut buf[4..8], 0x00A0_0000); // 10pt

        // Coding scheme at words 2..11 (byte 8..48)
        let scheme = "TeX text";
        buf[8] = scheme.len() as u8;
        buf[9..9 + scheme.len()].copy_from_slice(scheme.as_bytes());

        // Font family at words 12..16 (byte 48..68)
        let family = "CMR";
        buf[48] = family.len() as u8;
        buf[49..49 + family.len()].copy_from_slice(family.as_bytes());

        // Seven bit safe flag at word 17 (byte 68..72)
        BigEndian::write_i32(&mut buf[68..72], 1);

        // Extra words 18 and 19 (byte 72..80)
        BigEndian::write_u32(&mut buf[72..76], 0xAAAA);
        BigEndian::write_u32(&mut buf[76..80], 0xBBBB);

        let header = TfmHeader::parse(&buf, 20).expect("parse full header");
        assert_eq!(header.coding_scheme.as_deref(), Some("TeX text"));
        assert_eq!(header.font_family.as_deref(), Some("CMR"));
        assert!(header.seven_bit_safe);
        assert_eq!(header.extra_words, vec![0xAAAA, 0xBBBB]);
    }

    #[test]
    fn test_header_whitespace_bcpl_strings() {
        let mut buf = vec![0u8; 18 * 4];
        BigEndian::write_u32(&mut buf[0..4], 0x11223344);
        BigEndian::write_u32(&mut buf[4..8], 0x00A0_0000); // 10pt

        // Whitespace only for coding scheme
        buf[8] = 3;
        buf[9..12].copy_from_slice(b"   ");

        // Whitespace only for font family
        buf[48] = 2;
        buf[49..51].copy_from_slice(b"  ");

        let header = TfmHeader::parse(&buf, 18).expect("parse header");
        assert_eq!(header.coding_scheme, None);
        assert_eq!(header.font_family, None);
    }
}
