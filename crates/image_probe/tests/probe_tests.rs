// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Comprehensive test suite for tectonic_image_probe achieving ~100% test coverage.
#![allow(missing_docs)]

use std::io::Write;
use tectonic_image_probe::*;
use tempfile::NamedTempFile;

#[test]
fn test_image_info_default_and_error_display() {
    let default_info = ImageInfo::default();
    assert_eq!(default_info.width, 0);
    assert_eq!(default_info.height, 0);
    assert_eq!(default_info.x_dpi, 72.0);
    assert_eq!(default_info.y_dpi, 72.0);

    let err_unrec = ImageProbeError::UnrecognizedFormat;
    assert_eq!(
        format!("{err_unrec}"),
        "Unrecognized or unsupported image format"
    );

    let err_corrupt = ImageProbeError::CorruptHeader;
    assert_eq!(format!("{err_corrupt}"), "Corrupt image header");

    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let err_io = ImageProbeError::Io(io_err);
    assert!(format!("{err_io}").contains("file not found"));
}

#[test]
fn test_unrecognized_format() {
    // Fewer than 8 bytes
    assert!(matches!(
        probe_image_bytes(b"123"),
        Err(ImageProbeError::UnrecognizedFormat)
    ));
    // 8 bytes of garbage
    assert!(matches!(
        probe_image_bytes(b"abcdefgh"),
        Err(ImageProbeError::UnrecognizedFormat)
    ));
}

#[test]
fn test_probe_png_exhaustive() {
    // 1. Standard PNG with pHYs in meters (unit 1)
    let mut png_data = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr_data = Vec::new();
    ihdr_data.extend_from_slice(&320u32.to_be_bytes());
    ihdr_data.extend_from_slice(&240u32.to_be_bytes());
    ihdr_data.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_data.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
    png_data.extend_from_slice(b"IHDR");
    png_data.extend_from_slice(&ihdr_data);
    png_data.extend_from_slice(&[0; 4]); // CRC

    // Dummy tEXt chunk in-between
    png_data.extend_from_slice(&4u32.to_be_bytes());
    png_data.extend_from_slice(b"tEXt");
    png_data.extend_from_slice(b"test");
    png_data.extend_from_slice(&[0; 4]);

    // pHYs chunk: 2835 ppm (~72 DPI)
    let mut phys_data = Vec::new();
    phys_data.extend_from_slice(&2835u32.to_be_bytes());
    phys_data.extend_from_slice(&2835u32.to_be_bytes());
    phys_data.push(1); // meters
    png_data.extend_from_slice(&(phys_data.len() as u32).to_be_bytes());
    png_data.extend_from_slice(b"pHYs");
    png_data.extend_from_slice(&phys_data);
    png_data.extend_from_slice(&[0; 4]);

    // IEND chunk
    png_data.extend_from_slice(&0u32.to_be_bytes());
    png_data.extend_from_slice(b"IEND");
    png_data.extend_from_slice(&[0; 4]);

    let info = probe_image_bytes(&png_data).expect("Standard PNG failed");
    assert_eq!(info.width, 320);
    assert_eq!(info.height, 240);
    assert!((info.x_dpi - 72.0).abs() < 0.1);

    // 2. PNG with pHYs unit 0 (unknown unit)
    let mut png_unit0 = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png_unit0.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
    png_unit0.extend_from_slice(b"IHDR");
    png_unit0.extend_from_slice(&ihdr_data);
    png_unit0.extend_from_slice(&[0; 4]);

    let mut phys0 = Vec::new();
    phys0.extend_from_slice(&5000u32.to_be_bytes());
    phys0.extend_from_slice(&5000u32.to_be_bytes());
    phys0.push(0); // unknown unit
    png_unit0.extend_from_slice(&(phys0.len() as u32).to_be_bytes());
    png_unit0.extend_from_slice(b"pHYs");
    png_unit0.extend_from_slice(&phys0);
    png_unit0.extend_from_slice(&[0; 4]);

    let info0 = probe_image_bytes(&png_unit0).expect("PNG unit 0 failed");
    assert_eq!(info0.x_dpi, 72.0); // defaults to 72

    // 3. PNG without pHYs (default 72 DPI)
    let mut png_no_phys = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png_no_phys.extend_from_slice(&(ihdr_data.len() as u32).to_be_bytes());
    png_no_phys.extend_from_slice(b"IHDR");
    png_no_phys.extend_from_slice(&ihdr_data);
    png_no_phys.extend_from_slice(&[0; 4]);
    let info_no_phys = probe_image_bytes(&png_no_phys).expect("PNG no phys failed");
    assert_eq!(info_no_phys.x_dpi, 72.0);

    // 4. Corrupt PNGs
    // Corrupt length before IHDR
    let corrupt_len = vec![
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0xFF, 0xFF, 0xFF, 0xFF, b'I', b'H', b'D',
        b'R',
    ];
    assert!(matches!(
        probe_image_bytes(&corrupt_len),
        Err(ImageProbeError::CorruptHeader)
    ));

    // Valid signature but immediate IEND (missing IHDR)
    let mut missing_ihdr = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    missing_ihdr.extend_from_slice(&0u32.to_be_bytes());
    missing_ihdr.extend_from_slice(b"IEND");
    missing_ihdr.extend_from_slice(&[0; 4]);
    assert!(matches!(
        probe_image_bytes(&missing_ihdr),
        Err(ImageProbeError::CorruptHeader)
    ));
}

#[test]
fn test_probe_jpeg_exhaustive() {
    // 1. APP0 with unit 1 (DPI)
    let mut jpeg_dpi = Vec::new();
    jpeg_dpi.extend_from_slice(&[0xFF, 0xD8]); // SOI
                                               // JFIF APP0 (300 DPI)
    let mut app0 = Vec::new();
    app0.extend_from_slice(b"JFIF\0");
    app0.extend_from_slice(&[1, 2]); // v1.02
    app0.push(1); // DPI
    app0.extend_from_slice(&300u16.to_be_bytes());
    app0.extend_from_slice(&300u16.to_be_bytes());
    app0.extend_from_slice(&[0, 0]);
    jpeg_dpi.extend_from_slice(&[0xFF, 0xE0]);
    jpeg_dpi.extend_from_slice(&((app0.len() + 2) as u16).to_be_bytes());
    jpeg_dpi.extend_from_slice(&app0);

    // Dummy RST marker
    jpeg_dpi.extend_from_slice(&[0xFF, 0xD0]);

    // SOF0 (640x480)
    let mut sof0 = Vec::new();
    sof0.push(8);
    sof0.extend_from_slice(&480u16.to_be_bytes());
    sof0.extend_from_slice(&640u16.to_be_bytes());
    sof0.push(3);
    jpeg_dpi.extend_from_slice(&[0xFF, 0xC0]);
    jpeg_dpi.extend_from_slice(&((sof0.len() + 2) as u16).to_be_bytes());
    jpeg_dpi.extend_from_slice(&sof0);

    let info = probe_image_bytes(&jpeg_dpi).expect("JPEG DPI failed");
    assert_eq!(info.width, 640);
    assert_eq!(info.height, 480);
    assert_eq!(info.x_dpi, 300.0);

    // 2. APP0 with unit 2 (dots/cm) -> 100 dots/cm = 254 DPI
    let mut jpeg_cm = Vec::new();
    jpeg_cm.extend_from_slice(&[0xFF, 0xD8]);
    let mut app0_cm = Vec::new();
    app0_cm.extend_from_slice(b"JFIF\0");
    app0_cm.extend_from_slice(&[1, 2]);
    app0_cm.push(2); // dots/cm
    app0_cm.extend_from_slice(&100u16.to_be_bytes());
    app0_cm.extend_from_slice(&100u16.to_be_bytes());
    app0_cm.extend_from_slice(&[0, 0]);
    jpeg_cm.extend_from_slice(&[0xFF, 0xE0]);
    jpeg_cm.extend_from_slice(&((app0_cm.len() + 2) as u16).to_be_bytes());
    jpeg_cm.extend_from_slice(&app0_cm);

    // SOF1 (Extended Sequential)
    jpeg_cm.extend_from_slice(&[0xFF, 0xC1]);
    jpeg_cm.extend_from_slice(&((sof0.len() + 2) as u16).to_be_bytes());
    jpeg_cm.extend_from_slice(&sof0);

    let info_cm = probe_image_bytes(&jpeg_cm).expect("JPEG dots/cm failed");
    assert_eq!(info_cm.x_dpi, 254.0);
    assert_eq!(info_cm.y_dpi, 254.0);

    // 3. SOF2 (Progressive) without APP0 (defaults to 72 DPI)
    let mut jpeg_sof2 = Vec::new();
    jpeg_sof2.extend_from_slice(&[0xFF, 0xD8]);
    jpeg_sof2.extend_from_slice(&[0xFF, 0xC2]);
    jpeg_sof2.extend_from_slice(&((sof0.len() + 2) as u16).to_be_bytes());
    jpeg_sof2.extend_from_slice(&sof0);

    let info_sof2 = probe_image_bytes(&jpeg_sof2).expect("JPEG SOF2 failed");
    assert_eq!(info_sof2.x_dpi, 72.0);

    // 4. JPEG terminated with SOS before SOF (Corrupt)
    let jpeg_sos = vec![0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x00, 0x00, 0x00];
    assert!(matches!(
        probe_image_bytes(&jpeg_sos),
        Err(ImageProbeError::CorruptHeader)
    ));

    // 5. Truncated JPEG
    let jpeg_trunc = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x02, 0x08, 0x00];
    assert!(matches!(
        probe_image_bytes(&jpeg_trunc),
        Err(ImageProbeError::CorruptHeader)
    ));
}

#[test]
fn test_probe_bmp_exhaustive() {
    // 1. Standard BITMAPINFOHEADER (40 bytes) with resolution
    let mut bmp = Vec::new();
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes()); // header size
    bmp.extend_from_slice(&1920i32.to_le_bytes());
    bmp.extend_from_slice(&(-1080i32).to_le_bytes()); // negative height (top-down)
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&3780u32.to_le_bytes()); // 3780 ppm = ~96 DPI
    bmp.extend_from_slice(&3780u32.to_le_bytes());
    bmp.extend_from_slice(&[0; 8]);

    let info = probe_image_bytes(&bmp).expect("BMP failed");
    assert_eq!(info.width, 1920);
    assert_eq!(info.height, 1080);
    assert!((info.x_dpi - 96.0).abs() < 0.1);

    // 2. OS/2 BITMAPCOREHEADER (12 bytes)
    let mut bmp_core = Vec::new();
    bmp_core.extend_from_slice(b"BM");
    bmp_core.extend_from_slice(&26u32.to_le_bytes());
    bmp_core.extend_from_slice(&[0; 4]);
    bmp_core.extend_from_slice(&26u32.to_le_bytes());
    bmp_core.extend_from_slice(&12u32.to_le_bytes()); // 12-byte header
    bmp_core.extend_from_slice(&640u16.to_le_bytes());
    bmp_core.extend_from_slice(&480u16.to_le_bytes());
    bmp_core.extend_from_slice(&1u16.to_le_bytes());
    bmp_core.extend_from_slice(&24u16.to_le_bytes());

    let info_core = probe_image_bytes(&bmp_core).expect("BMP core failed");
    assert_eq!(info_core.width, 640);
    assert_eq!(info_core.height, 480);
    assert_eq!(info_core.x_dpi, 72.0);

    // 3. BITMAPINFOHEADER without resolution (ppm = 0)
    let mut bmp_no_res = bmp.clone();
    bmp_no_res[38..42].copy_from_slice(&0u32.to_le_bytes());
    bmp_no_res[42..46].copy_from_slice(&0u32.to_le_bytes());
    let info_no_res = probe_image_bytes(&bmp_no_res).expect("BMP no res failed");
    assert_eq!(info_no_res.x_dpi, 72.0);

    // 4. Truncated BMPs
    assert!(matches!(
        probe_image_bytes(b"BM12345678"),
        Err(ImageProbeError::CorruptHeader)
    ));
    let mut bmp_bad_size = bmp.clone();
    bmp_bad_size[14..18].copy_from_slice(&8u32.to_le_bytes()); // < 12
    assert!(matches!(
        probe_image_bytes(&bmp_bad_size),
        Err(ImageProbeError::CorruptHeader)
    ));
}

#[test]
fn test_probe_gif_exhaustive() {
    // 1. GIF87a
    let mut gif87 = Vec::new();
    gif87.extend_from_slice(b"GIF87a");
    gif87.extend_from_slice(&100u16.to_le_bytes());
    gif87.extend_from_slice(&200u16.to_le_bytes());
    let info87 = probe_image_bytes(&gif87).expect("GIF87a failed");
    assert_eq!(info87.width, 100);
    assert_eq!(info87.height, 200);

    // 2. GIF89a
    let mut gif89 = Vec::new();
    gif89.extend_from_slice(b"GIF89a");
    gif89.extend_from_slice(&300u16.to_le_bytes());
    gif89.extend_from_slice(&400u16.to_le_bytes());
    let info89 = probe_image_bytes(&gif89).expect("GIF89a failed");
    assert_eq!(info89.width, 300);
    assert_eq!(info89.height, 400);

    // 3. Truncated GIF
    assert!(matches!(
        probe_image_bytes(b"GIF89a12"),
        Err(ImageProbeError::CorruptHeader)
    ));
}

#[test]
fn test_probe_tiff_exhaustive() {
    // 1. Little Endian TIFF ("II") with ResolutionUnit in inches (2)
    let mut tiff_le = Vec::new();
    tiff_le.extend_from_slice(b"II*\0");
    tiff_le.extend_from_slice(&8u32.to_le_bytes()); // IFD offset at byte 8

    // IFD: 3 entries
    tiff_le.extend_from_slice(&3u16.to_le_bytes());

    // Entry 1: ImageWidth (0x0100)
    tiff_le.extend_from_slice(&0x0100u16.to_le_bytes());
    tiff_le.extend_from_slice(&4u16.to_le_bytes()); // LONG
    tiff_le.extend_from_slice(&1u32.to_le_bytes()); // count
    tiff_le.extend_from_slice(&1024u32.to_le_bytes()); // value

    // Entry 2: ImageLength (0x0101)
    tiff_le.extend_from_slice(&0x0101u16.to_le_bytes());
    tiff_le.extend_from_slice(&4u16.to_le_bytes());
    tiff_le.extend_from_slice(&1u32.to_le_bytes());
    tiff_le.extend_from_slice(&768u32.to_le_bytes());

    // Entry 3: ResolutionUnit (0x0128) = 2 (inches)
    tiff_le.extend_from_slice(&0x0128u16.to_le_bytes());
    tiff_le.extend_from_slice(&3u16.to_le_bytes()); // SHORT
    tiff_le.extend_from_slice(&1u32.to_le_bytes());
    tiff_le.extend_from_slice(&2u16.to_le_bytes());
    tiff_le.extend_from_slice(&[0; 2]); // pad

    let info_le = probe_image_bytes(&tiff_le).expect("Little-endian TIFF failed");
    assert_eq!(info_le.width, 1024);
    assert_eq!(info_le.height, 768);
    assert_eq!(info_le.x_dpi, 72.0);

    // 2. Big Endian TIFF ("MM") with ResolutionUnit in cm (3)
    let mut tiff_be = Vec::new();
    tiff_be.extend_from_slice(b"MM\0*");
    tiff_be.extend_from_slice(&8u32.to_be_bytes()); // IFD offset

    tiff_be.extend_from_slice(&3u16.to_be_bytes());

    // ImageWidth (0x0100)
    tiff_be.extend_from_slice(&0x0100u16.to_be_bytes());
    tiff_be.extend_from_slice(&4u16.to_be_bytes());
    tiff_be.extend_from_slice(&1u32.to_be_bytes());
    tiff_be.extend_from_slice(&800u32.to_be_bytes());

    // ImageLength (0x0101)
    tiff_be.extend_from_slice(&0x0101u16.to_be_bytes());
    tiff_be.extend_from_slice(&4u16.to_be_bytes());
    tiff_be.extend_from_slice(&1u32.to_be_bytes());
    tiff_be.extend_from_slice(&600u32.to_be_bytes());

    // ResolutionUnit (0x0128) = 3 (cm -> * 2.54)
    tiff_be.extend_from_slice(&0x0128u16.to_be_bytes());
    tiff_be.extend_from_slice(&3u16.to_be_bytes());
    tiff_be.extend_from_slice(&1u32.to_be_bytes());
    tiff_be.extend_from_slice(&3u16.to_be_bytes());
    tiff_be.extend_from_slice(&[0; 2]);

    let info_be = probe_image_bytes(&tiff_be).expect("Big-endian TIFF failed");
    assert_eq!(info_be.width, 800);
    assert_eq!(info_be.height, 600);
    assert!((info_be.x_dpi - (72.0 * 2.54)).abs() < 0.1);

    // 3. Truncated TIFF
    assert!(matches!(
        probe_image_bytes(b"II*\0\xFF\xFF\xFF\xFF"),
        Err(ImageProbeError::CorruptHeader)
    ));
}

#[test]
fn test_probe_image_path_file() {
    let mut file = NamedTempFile::new().unwrap();
    let mut gif = Vec::new();
    gif.extend_from_slice(b"GIF89a");
    gif.extend_from_slice(&120u16.to_le_bytes());
    gif.extend_from_slice(&240u16.to_le_bytes());
    file.write_all(&gif).unwrap();

    let info = probe_image_path(file.path()).expect("probe_image_path failed");
    assert_eq!(info.width, 120);
    assert_eq!(info.height, 240);

    // Non-existent path
    assert!(matches!(
        probe_image_path("/non/existent/path/image.gif"),
        Err(ImageProbeError::Io(_))
    ));
}

#[test]
fn test_pdf_probing_exhaustive() {
    // 1. Multi-page PDF with various boxes and rotations
    let pdf_content = r#"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 /MediaBox [0 0 612 792] >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /CropBox [10 10 600 780] /BleedBox [5 5 605 785] /TrimBox [15 15 595 775] /ArtBox [20 20 590 770] >>
endobj
4 0 obj
<< /Type /Page /Parent 2 0 R /Rotate 180 >>
endobj
5 0 obj
<< /Type /Page /Parent 2 0 R /Rotate 270 >>
endobj
xref
0 6
trailer
<< /Size 6 /Root 1 0 R >>
%%EOF"#;

    let bytes = pdf_content.as_bytes();

    let count = probe_pdf_page_count_bytes(bytes).expect("Page count probe failed");
    assert_eq!(count, 3);

    // Page 1: CropBox
    let crop = probe_pdf_box_bytes(bytes, 1, PdfBoxType::Crop).unwrap();
    assert_eq!(crop, (10.0, 10.0, 600.0, 780.0));

    // Page 1: BleedBox
    let bleed = probe_pdf_box_bytes(bytes, 1, PdfBoxType::Bleed).unwrap();
    assert_eq!(bleed, (5.0, 5.0, 605.0, 785.0));

    // Page 1: TrimBox
    let trim = probe_pdf_box_bytes(bytes, 1, PdfBoxType::Trim).unwrap();
    assert_eq!(trim, (15.0, 15.0, 595.0, 775.0));

    // Page 1: ArtBox
    let art = probe_pdf_box_bytes(bytes, 1, PdfBoxType::Art).unwrap();
    assert_eq!(art, (20.0, 20.0, 590.0, 770.0));

    // Page 2: Rotated 180 (no width/height swap)
    let rot180 = probe_pdf_box_bytes(bytes, 2, PdfBoxType::Media).unwrap();
    assert_eq!(rot180, (0.0, 0.0, 612.0, 792.0));

    // Page 3: Rotated 270 (width/height swapped)
    let rot270 = probe_pdf_box_bytes(bytes, 3, PdfBoxType::Media).unwrap();
    assert_eq!(rot270, (0.0, 0.0, 792.0, 612.0));

    // Box type casting from integer
    assert_eq!(PdfBoxType::from(1), PdfBoxType::Crop);
    assert_eq!(PdfBoxType::from(2), PdfBoxType::Media);
    assert_eq!(PdfBoxType::from(3), PdfBoxType::Bleed);
    assert_eq!(PdfBoxType::from(4), PdfBoxType::Trim);
    assert_eq!(PdfBoxType::from(5), PdfBoxType::Art);
    assert_eq!(PdfBoxType::from(99), PdfBoxType::Crop);

    // File path testing for PDF
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(bytes).unwrap();
    let count_file = probe_pdf_page_count(file.path()).unwrap();
    assert_eq!(count_file, 3);
    let box_file = probe_pdf_box(file.path(), 1, 2).unwrap();
    assert_eq!(box_file, (0.0, 0.0, 612.0, 792.0));

    // Errors
    assert!(matches!(
        probe_pdf_page_count_bytes(b"NOT_A_PDF"),
        Err(PdfProbeError::InvalidPdf)
    ));
    assert!(matches!(
        probe_pdf_box_bytes(b"NOT_A_PDF", 1, PdfBoxType::Media),
        Err(PdfProbeError::InvalidPdf)
    ));
    assert!(matches!(
        probe_pdf_box_bytes(bytes, 0, PdfBoxType::Media),
        Err(PdfProbeError::PageNotFound(0))
    ));
    assert!(matches!(
        probe_pdf_box_bytes(bytes, 99, PdfBoxType::Media),
        Err(PdfProbeError::PageNotFound(99))
    ));

    // Error messages display
    assert_eq!(
        format!("{}", PdfProbeError::InvalidPdf),
        "Not a valid PDF file"
    );
    assert_eq!(
        format!("{}", PdfProbeError::PageNotFound(5)),
        "Page 5 not found in PDF"
    );
    assert_eq!(
        format!("{}", PdfProbeError::BoxNotFound),
        "Bounding box not found or invalid"
    );
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    assert!(format!("{}", PdfProbeError::Io(io_err)).contains("file not found"));
}

#[test]
fn test_c_api_exhaustive() {
    let mut file = NamedTempFile::new().unwrap();
    let pdf_content = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox [0 0 500 800] >>\nendobj\n3 0 obj\n<< /Type /Page /Parent 2 0 R >>\nendobj\ntrailer\n<< /Root 1 0 R >>\n%%EOF";
    file.write_all(pdf_content).unwrap();
    let path = file.path().to_str().unwrap();
    let c_path = std::ffi::CString::new(path).unwrap();

    let mut img_file = NamedTempFile::new().unwrap();
    let mut gif = Vec::new();
    gif.extend_from_slice(b"GIF89a");
    gif.extend_from_slice(&320u16.to_le_bytes());
    gif.extend_from_slice(&240u16.to_le_bytes());
    img_file.write_all(&gif).unwrap();
    let img_path = img_file.path().to_str().unwrap();
    let c_img_path = std::ffi::CString::new(img_path).unwrap();

    unsafe {
        // SAFETY: `c_img_path` is a live NUL-terminated string; the four
        // out-pointers reference live `f32` locals for the call duration.
        let mut w = 0.0;
        let mut h = 0.0;
        let mut x_res = 0.0;
        let mut y_res = 0.0;

        // Success
        let res = tt_probe_image_bbox(c_img_path.as_ptr(), &mut w, &mut h, &mut x_res, &mut y_res);
        assert_eq!(res, 0);
        assert_eq!(w, 320.0);
        assert_eq!(h, 240.0);
        assert_eq!(x_res, 72.0);

        // Null pointer guards
        assert_eq!(
            tt_probe_image_bbox(std::ptr::null(), &mut w, &mut h, &mut x_res, &mut y_res),
            -1
        );
        assert_eq!(
            tt_probe_image_bbox(
                c_img_path.as_ptr(),
                std::ptr::null_mut(),
                &mut h,
                &mut x_res,
                &mut y_res
            ),
            -1
        );
        assert_eq!(
            tt_probe_image_bbox(
                c_img_path.as_ptr(),
                &mut w,
                std::ptr::null_mut(),
                &mut x_res,
                &mut y_res
            ),
            -1
        );
        assert_eq!(
            tt_probe_image_bbox(
                c_img_path.as_ptr(),
                &mut w,
                &mut h,
                std::ptr::null_mut(),
                &mut y_res
            ),
            -1
        );
        assert_eq!(
            tt_probe_image_bbox(
                c_img_path.as_ptr(),
                &mut w,
                &mut h,
                &mut x_res,
                std::ptr::null_mut()
            ),
            -1
        );

        // Non-existent path
        let non_exist = std::ffi::CString::new("/no/such/file").unwrap();
        assert_eq!(
            tt_probe_image_bbox(non_exist.as_ptr(), &mut w, &mut h, &mut x_res, &mut y_res),
            -1
        );

        // tt_probe_pdf_page_count
        assert_eq!(tt_probe_pdf_page_count(c_path.as_ptr()), 1);
        assert_eq!(tt_probe_pdf_page_count(std::ptr::null()), 0);
        assert_eq!(tt_probe_pdf_page_count(non_exist.as_ptr()), 0);

        // tt_probe_pdf_box
        let mut x0 = 0.0;
        let mut y0 = 0.0;
        let mut x1 = 0.0;
        let mut y1 = 0.0;

        let res = tt_probe_pdf_box(c_path.as_ptr(), 1, 2, &mut x0, &mut y0, &mut x1, &mut y1);
        assert_eq!(res, 0);
        assert_eq!((x0, y0, x1, y1), (0.0, 0.0, 500.0, 800.0));

        // Null pointer guards
        assert_eq!(
            tt_probe_pdf_box(std::ptr::null(), 1, 2, &mut x0, &mut y0, &mut x1, &mut y1),
            -1
        );
        assert_eq!(
            tt_probe_pdf_box(
                c_path.as_ptr(),
                1,
                2,
                std::ptr::null_mut(),
                &mut y0,
                &mut x1,
                &mut y1
            ),
            -1
        );
        assert_eq!(
            tt_probe_pdf_box(
                c_path.as_ptr(),
                1,
                2,
                &mut x0,
                std::ptr::null_mut(),
                &mut x1,
                &mut y1
            ),
            -1
        );
        assert_eq!(
            tt_probe_pdf_box(
                c_path.as_ptr(),
                1,
                2,
                &mut x0,
                &mut y0,
                std::ptr::null_mut(),
                &mut y1
            ),
            -1
        );
        assert_eq!(
            tt_probe_pdf_box(
                c_path.as_ptr(),
                1,
                2,
                &mut x0,
                &mut y0,
                &mut x1,
                std::ptr::null_mut()
            ),
            -1
        );

        // Invalid UTF-8 path
        let invalid_utf8 = b"\xFF\xFE\0";
        assert_eq!(
            tt_probe_image_bbox(
                invalid_utf8.as_ptr() as *const libc::c_char,
                &mut w,
                &mut h,
                &mut x_res,
                &mut y_res
            ),
            -1
        );
        assert_eq!(
            tt_probe_pdf_page_count(invalid_utf8.as_ptr() as *const libc::c_char),
            0
        );
        assert_eq!(
            tt_probe_pdf_box(
                invalid_utf8.as_ptr() as *const libc::c_char,
                1,
                2,
                &mut x0,
                &mut y0,
                &mut x1,
                &mut y1
            ),
            -1
        );

        // Data buffer C-API tests
        let mut w_d = 0.0;
        let mut h_d = 0.0;
        let mut x_d = 0.0;
        let mut y_d = 0.0;
        let res_d = tt_probe_image_bbox_data(
            gif.as_ptr(),
            gif.len(),
            &mut w_d,
            &mut h_d,
            &mut x_d,
            &mut y_d,
        );
        assert_eq!(res_d, 0);
        assert_eq!(w_d, 320.0);
        assert_eq!(h_d, 240.0);
        assert_eq!(x_d, 72.0);

        // Null pointer and corrupt data guards
        assert_eq!(
            tt_probe_image_bbox_data(
                std::ptr::null(),
                gif.len(),
                &mut w_d,
                &mut h_d,
                &mut x_d,
                &mut y_d
            ),
            -1
        );
        assert_eq!(
            tt_probe_image_bbox_data(gif.as_ptr(), 2, &mut w_d, &mut h_d, &mut x_d, &mut y_d),
            -1
        );

        // PDF data buffer tests
        let pdf_slice = pdf_content;
        assert_eq!(
            tt_probe_pdf_page_count_data(pdf_slice.as_ptr(), pdf_slice.len()),
            1
        );
        assert_eq!(tt_probe_pdf_page_count_data(std::ptr::null(), 0), 0);

        let mut bx0 = 0.0;
        let mut by0 = 0.0;
        let mut bx1 = 0.0;
        let mut by1 = 0.0;
        let res_box_d = tt_probe_pdf_box_data(
            pdf_slice.as_ptr(),
            pdf_slice.len(),
            1,
            2,
            &mut bx0,
            &mut by0,
            &mut bx1,
            &mut by1,
        );
        assert_eq!(res_box_d, 0);
        assert_eq!((bx0, by0, bx1, by1), (0.0, 0.0, 500.0, 800.0));

        assert_eq!(
            tt_probe_pdf_box_data(
                std::ptr::null(),
                0,
                1,
                2,
                &mut bx0,
                &mut by0,
                &mut bx1,
                &mut by1
            ),
            -1
        );
        assert_eq!(
            tt_probe_pdf_box_data(
                pdf_slice.as_ptr(),
                pdf_slice.len(),
                0,
                2,
                &mut bx0,
                &mut by0,
                &mut bx1,
                &mut by1
            ),
            -1
        );
    }
}

#[test]
fn test_image_edge_cases_and_zero_dimensions() {
    // 1. PNG with zero width or zero height in IHDR
    let mut png_zero = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr_zero = Vec::new();
    ihdr_zero.extend_from_slice(&0u32.to_be_bytes()); // width = 0
    ihdr_zero.extend_from_slice(&100u32.to_be_bytes());
    ihdr_zero.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_zero.extend_from_slice(&(ihdr_zero.len() as u32).to_be_bytes());
    png_zero.extend_from_slice(b"IHDR");
    png_zero.extend_from_slice(&ihdr_zero);
    png_zero.extend_from_slice(&[0; 4]);
    assert!(matches!(
        probe_image_bytes(&png_zero),
        Err(ImageProbeError::CorruptHeader)
    ));

    // 2. JPEG with short length marker or truncated SOF
    let jpeg_short_len = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01, 0x00, 0x00];
    assert!(matches!(
        probe_image_bytes(&jpeg_short_len),
        Err(ImageProbeError::CorruptHeader)
    ));

    let jpeg_sof_short = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x04, 0x01, 0x02];
    assert!(matches!(
        probe_image_bytes(&jpeg_sof_short),
        Err(ImageProbeError::CorruptHeader)
    ));

    // 3. BMP with 0 width or height
    let mut bmp_zero = Vec::new();
    bmp_zero.extend_from_slice(b"BM");
    bmp_zero.extend_from_slice(&54u32.to_le_bytes());
    bmp_zero.extend_from_slice(&[0; 4]);
    bmp_zero.extend_from_slice(&54u32.to_le_bytes());
    bmp_zero.extend_from_slice(&40u32.to_le_bytes());
    bmp_zero.extend_from_slice(&0i32.to_le_bytes()); // width = 0
    bmp_zero.extend_from_slice(&100i32.to_le_bytes());
    bmp_zero.extend_from_slice(&1u16.to_le_bytes());
    bmp_zero.extend_from_slice(&24u16.to_le_bytes());
    bmp_zero.extend_from_slice(&[0; 16]);
    assert!(matches!(
        probe_image_bytes(&bmp_zero),
        Err(ImageProbeError::CorruptHeader)
    ));

    // 4. GIF with 0 width or height
    let mut gif_zero = Vec::new();
    gif_zero.extend_from_slice(b"GIF89a");
    gif_zero.extend_from_slice(&0u16.to_le_bytes()); // width = 0
    gif_zero.extend_from_slice(&100u16.to_le_bytes());
    assert!(matches!(
        probe_image_bytes(&gif_zero),
        Err(ImageProbeError::CorruptHeader)
    ));

    // 5. TIFF with truncated IFD entries or zero width
    let mut tiff_short = Vec::new();
    tiff_short.extend_from_slice(b"II*\0");
    tiff_short.extend_from_slice(&8u32.to_le_bytes());
    tiff_short.extend_from_slice(&1u16.to_le_bytes()); // 1 entry, but only 4 bytes follow
    tiff_short.extend_from_slice(&[0; 4]);
    assert!(matches!(
        probe_image_bytes(&tiff_short),
        Err(ImageProbeError::CorruptHeader)
    ));

    // 6. Multi-chunk large file reading in probe_image_path
    let mut large_file = NamedTempFile::new().unwrap();
    let mut large_data = Vec::with_capacity(32768);
    // Write a valid PNG at the beginning
    let mut valid_png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&64u32.to_be_bytes());
    ihdr.extend_from_slice(&64u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    valid_png.extend_from_slice(&(ihdr.len() as u32).to_be_bytes());
    valid_png.extend_from_slice(b"IHDR");
    valid_png.extend_from_slice(&ihdr);
    valid_png.extend_from_slice(&[0; 4]);
    large_data.extend_from_slice(&valid_png);
    large_data.resize(32768, 0xAA);
    large_file.write_all(&large_data).unwrap();

    let info_large = probe_image_path(large_file.path()).unwrap();
    assert_eq!(info_large.width, 64);
    assert_eq!(info_large.height, 64);
}

#[test]
fn test_pdf_advanced_edge_cases() {
    // 1. Inverted coordinates swap (llx > urx and lly > ury)
    let pdf_inverted = r#"%PDF-1.4
1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
2 0 obj
<< /Type /Pages /Kids [3 0 R] /Count 1 >>
endobj
3 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [612 792 0 0] >>
endobj
trailer
<< /Root 1 0 R >>
%%EOF"#;
    let box_res = probe_pdf_box_bytes(pdf_inverted.as_bytes(), 1, PdfBoxType::Media).unwrap();
    assert_eq!(box_res, (0.0, 0.0, 612.0, 792.0));

    // 2. Flat PDF with NO /Pages tree, fallback directly to /Type /Page objects
    let pdf_flat = r#"%PDF-1.4
1 0 obj
<< /Type /Page /MediaBox [0 0 400 500] >>
endobj
2 0 obj
<< /Type /Page /MediaBox [0 0 300 400] >>
endobj
trailer
<< >>
%%EOF"#;
    let count_flat = probe_pdf_page_count_bytes(pdf_flat.as_bytes()).unwrap();
    assert_eq!(count_flat, 2);
    let box_flat = probe_pdf_box_bytes(pdf_flat.as_bytes(), 1, PdfBoxType::Media).unwrap();
    assert_eq!(box_flat, (0.0, 0.0, 400.0, 500.0));

    // 3. PDF with malformed box (e.g. 3 numbers instead of 4)
    let pdf_bad_box = r#"%PDF-1.4
1 0 obj
<< /Type /Page /MediaBox [0 0 400] >>
endobj
trailer
<< >>
%%EOF"#;
    assert!(matches!(
        probe_pdf_box_bytes(pdf_bad_box.as_bytes(), 1, PdfBoxType::Media),
        Err(PdfProbeError::BoxNotFound)
    ));
}
