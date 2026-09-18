// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Integration tests for tectonic_image_probe crate.
#![allow(missing_docs)]

use std::io::Write;
use tectonic_image_probe::*;
use tempfile::NamedTempFile;

#[test]
fn test_probe_png_synthetic() {
    let mut png_data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    // IHDR chunk: length=13, IHDR, width=320, height=240, bit_depth=8, color_type=2...
    let mut ihdr_data = Vec::new();
    ihdr_data.extend_from_slice(&320u32.to_be_bytes());
    ihdr_data.extend_from_slice(&240u32.to_be_bytes());
    ihdr_data.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_data.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
    png_data.extend_from_slice(b"IHDR");
    png_data.extend_from_slice(&ihdr_data);
    png_data.extend_from_slice(&[0, 0, 0, 0]); // CRC

    // pHYs chunk: 2835 ppm (~72 DPI)
    let mut phys_data = Vec::new();
    phys_data.extend_from_slice(&2835u32.to_be_bytes());
    phys_data.extend_from_slice(&2835u32.to_be_bytes());
    phys_data.push(1); // meters
    png_data.extend_from_slice(&(phys_data.len() as u32).to_be_bytes());
    png_data.extend_from_slice(b"pHYs");
    png_data.extend_from_slice(&phys_data);
    png_data.extend_from_slice(&[0, 0, 0, 0]); // CRC

    let info = probe_image_bytes(&png_data).expect("Failed to probe synthetic PNG");
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!((info.x_dpi - 72.0).abs() < 0.1);
    assert!((info.y_dpi - 72.0).abs() < 0.1);
}

#[test]
fn test_probe_jpeg_synthetic() {
    let mut jpeg = Vec::new();
    jpeg.extend_from_slice(&[0xFF, 0xD8]); // SOI
    // APP0 (JFIF 300 DPI)
    let mut app0 = Vec::new();
    app0.extend_from_slice(b"JFIF\0");
    app0.extend_from_slice(&[1, 2]); // v1.02
    app0.push(1); // DPI
    app0.extend_from_slice(&300u16.to_be_bytes());
    app0.extend_from_slice(&300u16.to_be_bytes());
    app0.extend_from_slice(&[0, 0]);
    jpeg.extend_from_slice(&[0xFF, 0xE0]);
    jpeg.extend_from_slice(&((app0.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(&app0);

    // SOF0 (640x480)
    let mut sof0 = Vec::new();
    sof0.push(8); // precision
    sof0.extend_from_slice(&480u16.to_be_bytes());
    sof0.extend_from_slice(&640u16.to_be_bytes());
    sof0.push(3); // components
    jpeg.extend_from_slice(&[0xFF, 0xC0]);
    jpeg.extend_from_slice(&((sof0.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(&sof0);

    let info = probe_image_bytes(&jpeg).expect("Failed to probe synthetic JPEG");
    assert_eq!(info.width, 640);
    assert_eq!(info.height, 480);
    assert_eq!(info.x_dpi, 300.0);
    assert_eq!(info.y_dpi, 300.0);
}

#[test]
fn test_probe_bmp_synthetic() {
    let mut bmp = Vec::new();
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&54u32.to_le_bytes()); // file size
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes()); // pixel offset

    // BITMAPINFOHEADER (40 bytes)
    bmp.extend_from_slice(&40u32.to_le_bytes()); // header size
    bmp.extend_from_slice(&1920i32.to_le_bytes()); // width
    bmp.extend_from_slice(&1080i32.to_le_bytes()); // height
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&24u16.to_le_bytes()); // bpp
    bmp.extend_from_slice(&0u32.to_le_bytes()); // compression
    bmp.extend_from_slice(&0u32.to_le_bytes()); // img size
    bmp.extend_from_slice(&3780u32.to_le_bytes()); // 3780 ppm = ~96 DPI
    bmp.extend_from_slice(&3780u32.to_le_bytes());
    bmp.extend_from_slice(&[0; 8]);

    let info = probe_image_bytes(&bmp).expect("Failed to probe synthetic BMP");
    assert_eq!(info.width, 1920);
    assert_eq!(info.height, 1080);
    assert!((info.x_dpi - 96.0).abs() < 0.1);
    assert!((info.y_dpi - 96.0).abs() < 0.1);
}

#[test]
fn test_probe_gif_synthetic() {
    let mut gif = Vec::new();
    gif.extend_from_slice(b"GIF89a");
    gif.extend_from_slice(&800u16.to_le_bytes()); // width
    gif.extend_from_slice(&600u16.to_le_bytes()); // height

    let info = probe_image_bytes(&gif).expect("Failed to probe synthetic GIF");
    assert_eq!(info.width, 800);
    assert_eq!(info.height, 600);
    assert_eq!(info.x_dpi, 72.0);
    assert_eq!(info.y_dpi, 72.0);
}

#[test]
fn test_probe_pdf_synthetic_multipage() {
    let pdf_content = r#"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 /MediaBox [0 0 612 792] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /CropBox [10 10 600 780] >>
endobj
4 0 obj
<< /Type /Page /Parent 2 0 R /Rotate 90 >>
endobj
xref
0 5
0000000000 65535 f 
0000000009 00000 n 
0000000058 00000 n 
0000000140 00000 n 
0000000215 00000 n 
trailer
<< /Size 5 /Root 1 0 R >>
startxref
280
%%EOF"#;

    let count = probe_pdf_page_count_bytes(pdf_content.as_bytes()).expect("Page count probe failed");
    assert_eq!(count, 2);

    // Page 1: CropBox [10 10 600 780]
    let (x0, y0, x1, y1) =
        probe_pdf_box_bytes(pdf_content.as_bytes(), 1, PdfBoxType::Crop).expect("Page 1 crop probe failed");
    assert_eq!((x0, y0, x1, y1), (10.0, 10.0, 600.0, 780.0));

    // Page 1: MediaBox [0 0 612 792] inherited from parent
    let (x0, y0, x1, y1) =
        probe_pdf_box_bytes(pdf_content.as_bytes(), 1, PdfBoxType::Media).expect("Page 1 media probe failed");
    assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 612.0, 792.0));

    // Page 2: Rotated 90 degrees -> width (612) and height (792) swapped
    let (x0, y0, x1, y1) =
        probe_pdf_box_bytes(pdf_content.as_bytes(), 2, PdfBoxType::Media).expect("Page 2 media probe failed");
    assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 792.0, 612.0));
}

#[test]
fn test_c_api_via_file() {
    let mut file = NamedTempFile::new().unwrap();
    let pdf_content = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 500 800] >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
    file.write_all(pdf_content).unwrap();
    let path = file.path().to_str().unwrap();
    let c_path = std::ffi::CString::new(path).unwrap();

    // SAFETY: pointers point to valid data on the stack.
    unsafe {
        let pages = tt_probe_pdf_page_count(c_path.as_ptr());
        assert_eq!(pages, 1);

        let mut x0 = 0.0;
        let mut y0 = 0.0;
        let mut x1 = 0.0;
        let mut y1 = 0.0;
        let res = tt_probe_pdf_box(c_path.as_ptr(), 1, 2, &mut x0, &mut y0, &mut x1, &mut y1);
        assert_eq!(res, 0);
        assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 500.0, 800.0));
    }
}
