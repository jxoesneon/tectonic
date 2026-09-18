// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! C-ABI entry points for tectonic_image_probe.

use std::ffi::CStr;

/// Probe image bounding box and resolution.
///
/// # Safety
/// The caller must ensure `path` is a valid null-terminated C string and
/// `width`, `height`, `x_res`, and `y_res` point to writable `f64` storage.
#[no_mangle]
pub unsafe extern "C" fn tt_probe_image_bbox(
    path: *const libc::c_char,
    width: *mut f64,
    height: *mut f64,
    x_res: *mut f64,
    y_res: *mut f64,
) -> i32 {
    if path.is_null() || width.is_null() || height.is_null() || x_res.is_null() || y_res.is_null() {
        return -1;
    }
    // SAFETY: Caller guarantees path is a valid null-terminated C string.
    let c_str = unsafe { CStr::from_ptr(path) };
    let path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    match crate::image::probe_image_path(path_str) {
        Ok(info) => {
            // SAFETY: Caller guarantees pointer arguments are valid for writes.
            unsafe {
                *width = info.width as f64;
                *height = info.height as f64;
                *x_res = info.x_dpi;
                *y_res = info.y_dpi;
            }
            0
        }
        Err(_) => -1,
    }
}

/// Probe total page count in a PDF file.
///
/// # Safety
/// The caller must ensure `path` is a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tt_probe_pdf_page_count(path: *const libc::c_char) -> i32 {
    if path.is_null() {
        return 0;
    }
    // SAFETY: Caller guarantees path is a valid null-terminated C string.
    let c_str = unsafe { CStr::from_ptr(path) };
    let path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return 0,
    };

    match crate::pdf::probe_pdf_page_count(path_str) {
        Ok(count) => count as i32,
        Err(_) => 0,
    }
}

/// Probe specific page boundary box in a PDF file with rotation applied.
///
/// # Safety
/// The caller must ensure `path` is a valid null-terminated C string and
/// `x0`, `y0`, `x1`, and `y1` point to writable `f64` storage.
#[no_mangle]
pub unsafe extern "C" fn tt_probe_pdf_box(
    path: *const libc::c_char,
    page_num: i32,
    box_type: i32,
    x0: *mut f64,
    y0: *mut f64,
    x1: *mut f64,
    y1: *mut f64,
) -> i32 {
    if path.is_null() || x0.is_null() || y0.is_null() || x1.is_null() || y1.is_null() {
        return -1;
    }
    // SAFETY: Caller guarantees path is a valid null-terminated C string.
    let c_str = unsafe { CStr::from_ptr(path) };
    let path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return -1,
    };

    if page_num <= 0 {
        return -1;
    }

    match crate::pdf::probe_pdf_box(path_str, page_num as usize, box_type) {
        Ok((bx0, by0, bx1, by1)) => {
            // SAFETY: Caller guarantees pointer arguments are valid for writes.
            unsafe {
                *x0 = bx0;
                *y0 = by0;
                *x1 = bx1;
                *y1 = by1;
            }
            0
        }
        Err(_) => -1,
    }
}

/// Probe image bounding box and resolution from in-memory byte buffer.
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
/// `width`, `height`, `x_res`, and `y_res` must point to writable `f64` storage.
#[no_mangle]
pub unsafe extern "C" fn tt_probe_image_bbox_data(
    data: *const u8,
    len: usize,
    width: *mut f64,
    height: *mut f64,
    x_res: *mut f64,
    y_res: *mut f64,
) -> i32 {
    if data.is_null() || width.is_null() || height.is_null() || x_res.is_null() || y_res.is_null() {
        return -1;
    }
    // SAFETY: Caller guarantees data is valid for reads up to len.
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    match crate::image::probe_image_bytes(slice) {
        Ok(info) => {
            // SAFETY: Caller guarantees pointers are valid for writes.
            unsafe {
                *width = info.width as f64;
                *height = info.height as f64;
                *x_res = info.x_dpi;
                *y_res = info.y_dpi;
            }
            0
        }
        Err(_) => -1,
    }
}

/// Probe total page count in a PDF from in-memory byte buffer.
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn tt_probe_pdf_page_count_data(data: *const u8, len: usize) -> i32 {
    if data.is_null() {
        return 0;
    }
    // SAFETY: Caller guarantees data is valid for reads up to len.
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    match crate::pdf::probe_pdf_page_count_bytes(slice) {
        Ok(count) => count as i32,
        Err(_) => 0,
    }
}

/// Probe specific page boundary box in a PDF from in-memory byte buffer with rotation applied.
///
/// # Safety
/// `data` must point to at least `len` readable bytes.
/// `x0`, `y0`, `x1`, and `y1` must point to writable `f64` storage.
#[no_mangle]
pub unsafe extern "C" fn tt_probe_pdf_box_data(
    data: *const u8,
    len: usize,
    page_num: i32,
    box_type: i32,
    x0: *mut f64,
    y0: *mut f64,
    x1: *mut f64,
    y1: *mut f64,
) -> i32 {
    if data.is_null() || x0.is_null() || y0.is_null() || x1.is_null() || y1.is_null() || page_num <= 0 {
        return -1;
    }
    // SAFETY: Caller guarantees data is valid for reads up to len.
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    match crate::pdf::probe_pdf_box_bytes(slice, page_num as usize, box_type.into()) {
        Ok((bx0, by0, bx1, by1)) => {
            // SAFETY: Caller guarantees pointers are valid for writes.
            unsafe {
                *x0 = bx0;
                *y0 = by0;
                *x1 = bx1;
                *y1 = by1;
            }
            0
        }
        Err(_) => -1,
    }
}
