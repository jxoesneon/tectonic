// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust, zero-allocation header probing for PNG, JPEG, BMP, GIF, and TIFF.

use byteorder::{BigEndian, ByteOrder, LittleEndian};
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use thiserror::Error;

/// Error type encountered while probing image metadata.
#[derive(Debug, Error)]
pub enum ImageProbeError {
    /// Standard I/O error occurred while reading the image file.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The file header does not match any recognized image format.
    #[error("Unrecognized or unsupported image format")]
    UnrecognizedFormat,
    /// The image header or chunk structure is corrupted or truncated.
    #[error("Corrupt image header")]
    CorruptHeader,
}

/// Extracted image dimensions and resolution metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInfo {
    /// Width of the image in pixels.
    pub width: u32,
    /// Height of the image in pixels.
    pub height: u32,
    /// Horizontal resolution in dots per inch (DPI).
    pub x_dpi: f64,
    /// Vertical resolution in dots per inch (DPI).
    pub y_dpi: f64,
}

impl Default for ImageInfo {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            x_dpi: 72.0,
            y_dpi: 72.0,
        }
    }
}

/// Probe image dimensions and resolution given a filesystem path.
pub fn probe_image_path<P: AsRef<Path>>(path: P) -> Result<ImageInfo, ImageProbeError> {
    let mut file = File::open(path)?;
    let mut header = Vec::with_capacity(65536);
    let mut chunk = [0u8; 8192];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        header.extend_from_slice(&chunk[..n]);
        if let Ok(info) = probe_image_bytes(&header) {
            return Ok(info);
        }
        if header.len() > 1024 * 1024 {
            break;
        }
    }
    probe_image_bytes(&header)
}

/// Probe image dimensions and resolution given an in-memory byte slice.
pub fn probe_image_bytes(bytes: &[u8]) -> Result<ImageInfo, ImageProbeError> {
    if bytes.len() < 8 {
        return Err(ImageProbeError::UnrecognizedFormat);
    }

    // 1. PNG Signature: \x89PNG\r\n\x1a\n
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return probe_png(bytes);
    }

    // 2. JPEG Signature: \xFF\xD8
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return probe_jpeg(bytes);
    }

    // 3. BMP Signature: 'BM'
    if bytes.starts_with(b"BM") {
        return probe_bmp(bytes);
    }

    // 4. GIF Signature: 'GIF87a' or 'GIF89a'
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return probe_gif(bytes);
    }

    // 5. TIFF Signature: 'II*\0' or 'MM\0*'
    if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        return probe_tiff(bytes);
    }

    Err(ImageProbeError::UnrecognizedFormat)
}

fn probe_png(bytes: &[u8]) -> Result<ImageInfo, ImageProbeError> {
    let mut offset = 8;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut x_dpi = 72.0;
    let mut y_dpi = 72.0;
    let mut found_ihdr = false;

    while offset + 8 <= bytes.len() {
        let length = BigEndian::read_u32(&bytes[offset..offset + 4]) as usize;
        let chunk_type = &bytes[offset + 4..offset + 8];
        let data_start = offset + 8;
        let data_end = data_start + length;

        if data_end > bytes.len() {
            if found_ihdr {
                break;
            }
            return Err(ImageProbeError::CorruptHeader);
        }

        if chunk_type == b"IHDR" && length >= 8 {
            width = BigEndian::read_u32(&bytes[data_start..data_start + 4]);
            height = BigEndian::read_u32(&bytes[data_start + 4..data_start + 8]);
            found_ihdr = true;
        } else if chunk_type == b"pHYs" && length >= 9 {
            let x_ppm = BigEndian::read_u32(&bytes[data_start..data_start + 4]);
            let y_ppm = BigEndian::read_u32(&bytes[data_start + 4..data_start + 8]);
            let unit = bytes[data_start + 8];
            if unit == 1 {
                if x_ppm > 0 {
                    x_dpi = (x_ppm as f64) * 0.0254;
                }
                if y_ppm > 0 {
                    y_dpi = (y_ppm as f64) * 0.0254;
                }
            }
        } else if chunk_type == b"IEND" {
            break;
        }

        offset = data_end + 4; // Skip 4-byte CRC
    }

    if found_ihdr && width > 0 && height > 0 {
        Ok(ImageInfo {
            width,
            height,
            x_dpi,
            y_dpi,
        })
    } else {
        Err(ImageProbeError::CorruptHeader)
    }
}

fn probe_jpeg(bytes: &[u8]) -> Result<ImageInfo, ImageProbeError> {
    let mut offset = 2;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut x_dpi = 72.0;
    let mut y_dpi = 72.0;

    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xFF {
            offset += 1;
            continue;
        }
        let marker = bytes[offset + 1];
        offset += 2;

        if marker == 0xD9 || marker == 0xDA {
            // EOI or SOS
            break;
        }

        if marker == 0x00 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        if offset + 2 > bytes.len() {
            break;
        }
        let length = BigEndian::read_u16(&bytes[offset..offset + 2]) as usize;
        if length < 2 || offset + length > bytes.len() {
            break;
        }
        let payload = &bytes[offset + 2..offset + length];

        // APP0 / JFIF
        if marker == 0xE0 && payload.starts_with(b"JFIF\0") && payload.len() >= 12 {
            let units = payload[7];
            let x_density = BigEndian::read_u16(&payload[8..10]) as f64;
            let y_density = BigEndian::read_u16(&payload[10..12]) as f64;
            if units == 1 && x_density > 0.0 && y_density > 0.0 {
                x_dpi = x_density;
                y_dpi = y_density;
            } else if units == 2 && x_density > 0.0 && y_density > 0.0 {
                x_dpi = x_density * 2.54;
                y_dpi = y_density * 2.54;
            }
        }

        // SOF0, SOF1, SOF2 (Baseline, Extended Sequential, Progressive)
        if (marker == 0xC0 || marker == 0xC1 || marker == 0xC2) && payload.len() >= 5 {
            height = BigEndian::read_u16(&payload[1..3]) as u32;
            width = BigEndian::read_u16(&payload[3..5]) as u32;
            return Ok(ImageInfo {
                width,
                height,
                x_dpi,
                y_dpi,
            });
        }

        offset += length;
    }

    if width > 0 && height > 0 {
        Ok(ImageInfo {
            width,
            height,
            x_dpi,
            y_dpi,
        })
    } else {
        Err(ImageProbeError::CorruptHeader)
    }
}

fn probe_bmp(bytes: &[u8]) -> Result<ImageInfo, ImageProbeError> {
    if bytes.len() < 26 {
        return Err(ImageProbeError::CorruptHeader);
    }
    let dib_header_size = LittleEndian::read_u32(&bytes[14..18]) as usize;
    if dib_header_size < 12 || bytes.len() < 14 + dib_header_size {
        return Err(ImageProbeError::CorruptHeader);
    }

    let (width, height, x_dpi, y_dpi) = if dib_header_size == 12 {
        // OS/2 BITMAPCOREHEADER
        let w = LittleEndian::read_u16(&bytes[18..20]) as u32;
        let h = LittleEndian::read_u16(&bytes[20..22]) as u32;
        (w, h, 72.0, 72.0)
    } else {
        // BITMAPINFOHEADER or later
        let w = (LittleEndian::read_i32(&bytes[18..22])).unsigned_abs();
        let h = (LittleEndian::read_i32(&bytes[22..26])).unsigned_abs();
        let mut x = 72.0;
        let mut y = 72.0;
        if dib_header_size >= 40 && bytes.len() >= 46 {
            let x_ppm = LittleEndian::read_u32(&bytes[38..42]);
            let y_ppm = LittleEndian::read_u32(&bytes[42..46]);
            if x_ppm > 0 {
                x = (x_ppm as f64) * 0.0254;
            }
            if y_ppm > 0 {
                y = (y_ppm as f64) * 0.0254;
            }
        }
        (w, h, x, y)
    };

    if width > 0 && height > 0 {
        Ok(ImageInfo {
            width,
            height,
            x_dpi,
            y_dpi,
        })
    } else {
        Err(ImageProbeError::CorruptHeader)
    }
}

fn probe_gif(bytes: &[u8]) -> Result<ImageInfo, ImageProbeError> {
    if bytes.len() < 10 {
        return Err(ImageProbeError::CorruptHeader);
    }
    let width = LittleEndian::read_u16(&bytes[6..8]) as u32;
    let height = LittleEndian::read_u16(&bytes[8..10]) as u32;
    if width > 0 && height > 0 {
        Ok(ImageInfo {
            width,
            height,
            x_dpi: 72.0,
            y_dpi: 72.0,
        })
    } else {
        Err(ImageProbeError::CorruptHeader)
    }
}

fn probe_tiff(bytes: &[u8]) -> Result<ImageInfo, ImageProbeError> {
    if bytes.len() < 8 {
        return Err(ImageProbeError::CorruptHeader);
    }
    let is_le = bytes.starts_with(b"II");
    let ifd_offset = if is_le {
        LittleEndian::read_u32(&bytes[4..8]) as usize
    } else {
        BigEndian::read_u32(&bytes[4..8]) as usize
    };

    if ifd_offset + 2 > bytes.len() {
        return Err(ImageProbeError::CorruptHeader);
    }

    let num_entries = if is_le {
        LittleEndian::read_u16(&bytes[ifd_offset..ifd_offset + 2]) as usize
    } else {
        BigEndian::read_u16(&bytes[ifd_offset..ifd_offset + 2]) as usize
    };

    let mut width = 0u32;
    let mut height = 0u32;
    let mut x_res = 72.0;
    let mut y_res = 72.0;
    let mut res_unit = 2; // 2 = inch

    let mut entry_ptr = ifd_offset + 2;
    for _ in 0..num_entries {
        if entry_ptr + 12 > bytes.len() {
            break;
        }
        let tag = if is_le {
            LittleEndian::read_u16(&bytes[entry_ptr..entry_ptr + 2])
        } else {
            BigEndian::read_u16(&bytes[entry_ptr..entry_ptr + 2])
        };
        let val_offset = entry_ptr + 8;

        match tag {
            0x0100 => {
                // ImageWidth
                width = if is_le {
                    LittleEndian::read_u32(&bytes[val_offset..val_offset + 4])
                } else {
                    BigEndian::read_u32(&bytes[val_offset..val_offset + 4])
                };
            }
            0x0101 => {
                // ImageLength
                height = if is_le {
                    LittleEndian::read_u32(&bytes[val_offset..val_offset + 4])
                } else {
                    BigEndian::read_u32(&bytes[val_offset..val_offset + 4])
                };
            }
            0x0128 => {
                // ResolutionUnit
                res_unit = if is_le {
                    LittleEndian::read_u16(&bytes[val_offset..val_offset + 2])
                } else {
                    BigEndian::read_u16(&bytes[val_offset..val_offset + 2])
                };
            }
            _ => {}
        }
        entry_ptr += 12;
    }

    if res_unit == 3 {
        // cm to inch
        x_res *= 2.54;
        y_res *= 2.54;
    }

    if width > 0 && height > 0 {
        Ok(ImageInfo {
            width,
            height,
            x_dpi: x_res,
            y_dpi: y_res,
        })
    } else {
        Err(ImageProbeError::CorruptHeader)
    }
}
