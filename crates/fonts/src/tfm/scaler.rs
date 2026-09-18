// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Fixed-point arithmetic and scaling algorithms matching Knuth's TeX implementation.

/// Multiplies `x` by `n` and divides by `d` using integer arithmetic with truncation towards zero.
///
/// Bitwise equivalent to Knuth's `xn_over_d` routine in TeX.
#[inline]
pub fn xn_over_d(x: i32, n: i32, d: i32) -> i32 {
    if d == 0 {
        return 0;
    }
    let p = (x as i64) * (n as i64);
    (p / (d as i64)) as i32
}

/// Multiplies `x` by `n` and divides by `d`, rounding to the nearest integer.
///
/// Bitwise equivalent to Knuth's `round_xn_over_d` routine in TeX.
#[inline]
pub fn round_xn_over_d(x: i32, n: i32, d: i32) -> i32 {
    if d == 0 {
        return 0;
    }
    let p = (x as i64) * (n as i64);
    let rem = p % (d as i64);
    let mut quot = p / (d as i64);
    if rem.abs() * 2 >= (d as i64).abs() {
        if (p > 0 && d > 0) || (p < 0 && d < 0) {
            quot += 1;
        } else {
            quot -= 1;
        }
    }
    quot as i32
}

/// Knuth's exact integer TFM scaler for fixed-point fraction dimensions (`FixWord`).
///
/// In TeX, a `FixWord` is a 32-bit signed number with 12 bits of integer and 20 bits of fraction.
/// When scaling dimensions by a font size $z$ (in scaled points), Knuth's algorithm
/// decomposes the 4 bytes into nested divisions to prevent 32-bit overflow while preserving
/// bitwise identical TeX layout results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TfmScaler {
    /// Scaled font size in scaled points (`sp`).
    pub z: i32,
    /// Intermediate scaling factor `alpha`.
    pub alpha: i32,
    /// Intermediate divisor `beta`.
    pub beta: i32,
}

impl TfmScaler {
    /// Initializes a `TfmScaler` for a given font size `z` (in `sp`).
    pub fn new(mut z: i32) -> Self {
        let mut alpha = 16;
        while z >= 0x80_0000 {
            z /= 2;
            alpha += alpha;
        }
        let beta = if alpha == 0 { 1 } else { 256 / alpha };
        alpha *= z;
        Self { z, alpha, beta }
    }

    /// Scales a 4-byte `FixWord` into scaled points (`sp`).
    pub fn scale_bytes(&self, bytes: [u8; 4]) -> i32 {
        let a = bytes[0] as i32;
        let b = bytes[1] as i32;
        let c = bytes[2] as i32;
        let d = bytes[3] as i32;

        let sw = (((((d * self.z) / 256) + c * self.z) / 256) + b * self.z) / self.beta;

        if a == 0 {
            sw
        } else if a == 255 {
            sw - self.alpha
        } else {
            // Signed byte calculation for rare non-standard fixwords
            let signed_a = if a > 127 { a - 256 } else { a };
            sw + signed_a * self.alpha
        }
    }

    /// Scales an integer `FixWord` value into scaled points (`sp`).
    pub fn scale_fix_word(&self, fix: i32) -> i32 {
        self.scale_bytes(fix.to_be_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xn_over_d() {
        assert_eq!(xn_over_d(100, 2, 5), 40);
        assert_eq!(xn_over_d(-100, 2, 5), -40);
        assert_eq!(xn_over_d(100, -2, 5), -40);
        assert_eq!(xn_over_d(100, 2, 0), 0);
        assert_eq!(xn_over_d(7, 10, 3), 23); // 70 / 3 = 23
    }

    #[test]
    fn test_round_xn_over_d() {
        assert_eq!(round_xn_over_d(100, 2, 5), 40);
        assert_eq!(round_xn_over_d(7, 10, 3), 23); // 70 / 3 = 23.33 -> 23
        assert_eq!(round_xn_over_d(8, 10, 3), 27); // 80 / 3 = 26.66 -> 27
        assert_eq!(round_xn_over_d(-8, 10, 3), -27);
        assert_eq!(round_xn_over_d(10, 2, 0), 0);
    }

    #[test]
    fn test_tfm_scaler() {
        // Standard 10pt font: 655360 sp
        let scaler = TfmScaler::new(655360);
        assert_eq!(scaler.z, 655360);
        assert_eq!(scaler.alpha, 16 * 655360);
        assert_eq!(scaler.beta, 16);

        // FixWord for 1.0 (2^20 = 1048576 = 0x00100000)
        // At 10pt, 1.0 should scale to exactly 10pt = 655360 sp
        let fix_one = 1048576_i32;
        assert_eq!(scaler.scale_fix_word(fix_one), 655360);

        // FixWord for 0.5 (2^19 = 524288 = 0x00080000)
        let fix_half = 524288_i32;
        assert_eq!(scaler.scale_fix_word(fix_half), 327680);

        // FixWord for 0.0
        assert_eq!(scaler.scale_fix_word(0), 0);

        // Negative FixWord: -1.0 (-1048576 = 0xFFF00000)
        let fix_neg_one = -1048576_i32;
        assert_eq!(scaler.scale_fix_word(fix_neg_one), -655360);

        // Byte a not 0 or 255 (e.g. positive signed byte and negative signed byte)
        let res_pos_a = scaler.scale_bytes([2, 0, 0, 0]);
        assert_eq!(res_pos_a, 2 * scaler.alpha);

        let res_neg_a = scaler.scale_bytes([254, 0, 0, 0]); // 254 - 256 = -2
        assert_eq!(res_neg_a, -2 * scaler.alpha);

        // Large font size >= 0x800000
        let large_scaler = TfmScaler::new(0x100_0000); // 16777216 sp
        assert_eq!(large_scaler.scale_fix_word(fix_one), 0x100_0000);
    }
}
