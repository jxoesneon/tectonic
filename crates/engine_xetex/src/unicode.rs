// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust Unicode line breaking, BiDi analysis, and text encoding conversions.
//!
//! Replaces external ICU C dependencies with pure-Rust `icu_segmenter`, `unicode-bidi`,
//! and `encoding_rs`.

use encoding_rs::Encoding;
use icu_segmenter::options::LineBreakOptions;
use icu_segmenter::LineSegmenter;
use std::ffi::CStr;
use std::sync::Mutex;
use unicode_bidi::{BidiInfo, Level};

// ---------------------------------------------------------------------------
// 1. Unicode Line Breaking (replacing ubrk_*)
// ---------------------------------------------------------------------------

struct LineBreakState {
    breaks: Vec<i32>,
    cursor: usize,
}

static CURRENT_LINEBREAK: Mutex<Option<LineBreakState>> = Mutex::new(None);

/// Initializes line break opportunities for a UTF-16 text slice.
///
/// Returns 0 on success, or negative on error.
///
/// # Safety
///
/// `text` must be null or point to `text_len` contiguous UTF-16 code units.
/// `_locale` must be null or point to a valid NUL-terminated C string. Only one
/// line-break iterator is active at a time; callers must initialize it before
/// requesting boundaries and close it before starting another iterator.
#[no_mangle]
pub unsafe extern "C" fn tt_linebreak_start(
    text: *const u16,
    text_len: i32,
    _locale: *const libc::c_char,
) -> i32 {
    if text.is_null() || text_len <= 0 {
        let mut state = CURRENT_LINEBREAK.lock().unwrap();
        *state = None;
        return 0;
    }

    let slice = std::slice::from_raw_parts(text, text_len as usize);
    let segmenter = LineSegmenter::new_auto(LineBreakOptions::default());
    let raw_breaks: Vec<usize> = segmenter.segment_utf16(slice).collect();

    // In ICU ubrk_next(), the start (offset 0) is never returned;
    // only break boundaries strictly greater than 0 are emitted.
    let breaks: Vec<i32> = raw_breaks
        .into_iter()
        .filter(|&b| b > 0)
        .map(|b| b as i32)
        .collect();

    let mut state = CURRENT_LINEBREAK.lock().unwrap();
    *state = Some(LineBreakState { breaks, cursor: 0 });
    0
}

/// Returns the next UTF-16 code unit offset where a line break is permitted,
/// or -1 (`UBRK_DONE`) when there are no further line break opportunities.
///
/// # Safety
///
/// The caller must have initialized the shared line-break iterator with
/// [`tt_linebreak_start`]. Only one iterator may be active at a time.
#[no_mangle]
pub unsafe extern "C" fn tt_linebreak_next() -> i32 {
    let mut state = CURRENT_LINEBREAK.lock().unwrap();
    if let Some(ref mut lb) = *state {
        if lb.cursor < lb.breaks.len() {
            let val = lb.breaks[lb.cursor];
            lb.cursor += 1;
            val
        } else {
            -1
        }
    } else {
        -1
    }
}

/// Closes and cleans up the current line break iterator.
///
/// # Safety
///
/// The caller must not use the shared iterator after this call except by
/// initializing it again with [`tt_linebreak_start`].
#[no_mangle]
pub unsafe extern "C" fn tt_linebreak_close() {
    let mut state = CURRENT_LINEBREAK.lock().unwrap();
    *state = None;
}

/// Returns every UTF-16 code-unit offset `> 0` at which a line break is
/// permitted in `text`, folded into a single state-local call
/// (`tt_linebreak_start` + repeated `tt_linebreak_next` without the
/// shared iterator).
pub fn linebreak_offsets(text: &[u16]) -> Vec<i32> {
    if text.is_empty() {
        return Vec::new();
    }
    let segmenter = LineSegmenter::new_auto(LineBreakOptions::default());
    segmenter
        .segment_utf16(text)
        .filter(|&b| b > 0)
        .map(|b| b as i32)
        .collect()
}

// ---------------------------------------------------------------------------
// 2. Unicode Bidirectional Algorithm (replacing ubidi_*)
// ---------------------------------------------------------------------------

/// Visual run representation for bidirectional text layout.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TtBiDiRun {
    /// Starting offset in UTF-16 code units.
    pub start: i32,
    /// Length in UTF-16 code units.
    pub length: i32,
    /// Direction: true if RTL, false if LTR.
    pub is_rtl: bool,
}

/// Opaque context holding resolved visual runs for a text paragraph.
pub struct TtBiDiContext {
    runs: Vec<TtBiDiRun>,
    direction: i32, // 0: LTR, 1: RTL, 2: MIXED
}

/// Creates and resolves bidirectional runs for a UTF-16 text slice.
///
/// # Safety
///
/// `text` must be null or point to `text_len` contiguous UTF-16 code units.
/// The returned pointer is owned by the caller and must be released with
/// [`tt_bidi_destroy`].
#[no_mangle]
pub unsafe extern "C" fn tt_bidi_create(
    text: *const u16,
    text_len: i32,
    default_dir_rtl: bool,
) -> *mut TtBiDiContext {
    if text.is_null() || text_len <= 0 {
        let ctx = Box::new(TtBiDiContext {
            runs: Vec::new(),
            direction: if default_dir_rtl { 1 } else { 0 },
        });
        return Box::into_raw(ctx);
    }

    let slice = std::slice::from_raw_parts(text, text_len as usize);
    let s = String::from_utf16_lossy(slice);

    let default_level = if default_dir_rtl {
        Level::rtl()
    } else {
        Level::ltr()
    };

    let bidi_info = BidiInfo::new(&s, Some(default_level));
    let mut runs = Vec::new();

    let mut has_ltr = false;
    let mut has_rtl = false;

    for para in &bidi_info.paragraphs {
        let (levels, visual_runs) = bidi_info.visual_runs(para, para.range.clone());
        for range in visual_runs {
            let is_rtl = levels.get(range.start).map(|l| l.is_rtl()).unwrap_or(false);
            if is_rtl {
                has_rtl = true;
            } else {
                has_ltr = true;
            }

            let utf16_start = s[..range.start].encode_utf16().count() as i32;
            let utf16_len = s[range].encode_utf16().count() as i32;

            runs.push(TtBiDiRun {
                start: utf16_start,
                length: utf16_len,
                is_rtl,
            });
        }
    }

    let direction = if has_ltr && has_rtl {
        2 // MIXED
    } else if has_rtl {
        1 // RTL
    } else {
        0 // LTR
    };

    Box::into_raw(Box::new(TtBiDiContext { runs, direction }))
}

/// Returns the overall direction of the paragraph (0: LTR, 1: RTL, 2: MIXED).
///
/// # Safety
///
/// `ctx` must be null or a live pointer returned by [`tt_bidi_create`].
#[no_mangle]
pub unsafe extern "C" fn tt_bidi_get_direction(ctx: *mut TtBiDiContext) -> i32 {
    if ctx.is_null() {
        return 0;
    }
    (*ctx).direction
}

/// Returns the number of visual runs.
///
/// # Safety
///
/// `ctx` must be null or a live pointer returned by [`tt_bidi_create`].
#[no_mangle]
pub unsafe extern "C" fn tt_bidi_count_runs(ctx: *mut TtBiDiContext) -> i32 {
    if ctx.is_null() {
        return 0;
    }
    (*ctx).runs.len() as i32
}

/// Retrieves the visual run at `run_index`.
///
/// Returns 1 for RTL, 0 for LTR, or -1 if index is invalid.
///
/// # Safety
///
/// `ctx` must be null or a live pointer returned by [`tt_bidi_create`].
/// `logical_start` and `length` must be null or point to writable `i32`
/// storage.
#[no_mangle]
pub unsafe extern "C" fn tt_bidi_get_visual_run(
    ctx: *mut TtBiDiContext,
    run_index: i32,
    logical_start: *mut i32,
    length: *mut i32,
) -> i32 {
    if ctx.is_null() || run_index < 0 {
        return -1;
    }
    let runs = &(*ctx).runs;
    if (run_index as usize) < runs.len() {
        let r = &runs[run_index as usize];
        if !logical_start.is_null() {
            *logical_start = r.start;
        }
        if !length.is_null() {
            *length = r.length;
        }
        if r.is_rtl {
            1 // RTL
        } else {
            0 // LTR
        }
    } else {
        -1
    }
}

/// Destroys a bidirectional context.
///
/// # Safety
///
/// `ctx` must be null or a pointer previously returned by [`tt_bidi_create`].
/// The pointer must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn tt_bidi_destroy(ctx: *mut TtBiDiContext) {
    if !ctx.is_null() {
        drop(Box::from_raw(ctx));
    }
}

// ---------------------------------------------------------------------------
// 3. Text Encodings (replacing ucnv_*)
// ---------------------------------------------------------------------------

/// Opaque converter handle wrapping an `encoding_rs::Encoding`.
pub struct TtEncodingHandle {
    encoding: &'static Encoding,
}

/// Checks if an encoding label is supported by `encoding_rs`.
///
/// # Safety
///
/// `name` must be null or point to a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn tt_encoding_is_supported(name: *const libc::c_char) -> bool {
    if name.is_null() {
        return false;
    }
    if let Ok(label) = CStr::from_ptr(name).to_str() {
        Encoding::for_label(label.as_bytes()).is_some()
    } else {
        false
    }
}

/// Opens an encoding converter for the specified encoding label.
///
/// # Safety
///
/// `name` must be null or point to a valid NUL-terminated C string. The
/// returned pointer is owned by the caller and must be released with
/// [`tt_encoding_close`].
#[no_mangle]
pub unsafe extern "C" fn tt_encoding_open(name: *const libc::c_char) -> *mut TtEncodingHandle {
    if name.is_null() {
        return std::ptr::null_mut();
    }
    if let Ok(label) = CStr::from_ptr(name).to_str() {
        if let Some(encoding) = Encoding::for_label(label.as_bytes()) {
            return Box::into_raw(Box::new(TtEncodingHandle { encoding }));
        }
    }
    std::ptr::null_mut()
}

/// Converts an input byte buffer into UTF-32 native endian codepoints.
///
/// Returns 0 on success, or -1 on error.
///
/// # Safety
///
/// `handle` must be null or a live pointer returned by [`tt_encoding_open`].
/// `in_bytes` must point to `in_len` readable bytes, `out_utf32` must point to
/// `out_max_len` writable `u32` values, and `out_written` must be null or point
/// to writable `usize` storage.
#[no_mangle]
pub unsafe extern "C" fn tt_encoding_to_utf32(
    handle: *mut TtEncodingHandle,
    in_bytes: *const u8,
    in_len: usize,
    out_utf32: *mut u32,
    out_max_len: usize,
    out_written: *mut usize,
) -> i32 {
    if handle.is_null() || in_bytes.is_null() || out_utf32.is_null() {
        return -1;
    }

    let input_slice = std::slice::from_raw_parts(in_bytes, in_len);
    let (decoded_str, _, _) = (*handle).encoding.decode(input_slice);

    let mut written = 0;
    let out_slice = std::slice::from_raw_parts_mut(out_utf32, out_max_len);

    for ch in decoded_str.chars() {
        if written >= out_max_len {
            break;
        }
        out_slice[written] = ch as u32;
        written += 1;
    }

    if !out_written.is_null() {
        *out_written = written;
    }

    0
}

/// Closes an encoding converter handle.
///
/// # Safety
///
/// `handle` must be null or a pointer previously returned by
/// [`tt_encoding_open`]. The pointer must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn tt_encoding_close(handle: *mut TtEncodingHandle) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linebreak_simple_english() {
        // "Hello world test" in UTF-16
        let text: Vec<u16> = "Hello world test".encode_utf16().collect();
        // SAFETY: `text` is a live UTF-16 slice for the duration of the calls.
        unsafe {
            assert_eq!(
                tt_linebreak_start(text.as_ptr(), text.len() as i32, std::ptr::null()),
                0
            );
            let mut breaks = Vec::new();
            loop {
                let b = tt_linebreak_next();
                if b < 0 {
                    break;
                }
                breaks.push(b);
            }
            tt_linebreak_close();

            // Should have breaks after "Hello ", "world ", "test"
            assert!(!breaks.is_empty());
            assert_eq!(*breaks.last().unwrap(), text.len() as i32);
        }
    }

    #[test]
    fn test_linebreak_null_or_empty() {
        // SAFETY: all pointers are either null or point to valid test storage.
        unsafe {
            assert_eq!(tt_linebreak_start(std::ptr::null(), 0, std::ptr::null()), 0);
            assert_eq!(tt_linebreak_next(), -1);
            tt_linebreak_close();

            let dummy = [0u16; 2];
            assert_eq!(tt_linebreak_start(dummy.as_ptr(), 0, std::ptr::null()), 0);
            assert_eq!(tt_linebreak_next(), -1);
            tt_linebreak_close();
        }
    }

    #[test]
    fn test_bidi_simple() {
        let text: Vec<u16> = "Hello World".encode_utf16().collect();
        // SAFETY: `text` remains live while `ctx` is used and `ctx` is destroyed once.
        unsafe {
            let ctx = tt_bidi_create(text.as_ptr(), text.len() as i32, false);
            assert!(!ctx.is_null());
            assert_eq!(tt_bidi_get_direction(ctx), 0); // LTR
            assert_eq!(tt_bidi_count_runs(ctx), 1);

            let mut start = 0;
            let mut len = 0;
            let dir = tt_bidi_get_visual_run(ctx, 0, &mut start, &mut len);
            assert_eq!(dir, 0); // LTR
            assert_eq!(start, 0);
            assert_eq!(len, text.len() as i32);

            // Out-of-bounds run index
            assert_eq!(tt_bidi_get_visual_run(ctx, 999, &mut start, &mut len), -1);

            tt_bidi_destroy(ctx);
        }
    }

    #[test]
    fn test_bidi_rtl_and_mixed() {
        // Mixed text: "Hello שלום" (English + Hebrew)
        let text: Vec<u16> = "Hello שלום".encode_utf16().collect();
        // SAFETY: all input slices and output pointers are valid for the calls.
        unsafe {
            let ctx = tt_bidi_create(text.as_ptr(), text.len() as i32, false);
            assert!(!ctx.is_null());
            assert_eq!(tt_bidi_get_direction(ctx), 2); // MIXED
            assert!(tt_bidi_count_runs(ctx) >= 2);

            let mut start = 0;
            let mut len = 0;
            let _ = tt_bidi_get_visual_run(ctx, 0, &mut start, &mut len);
            tt_bidi_destroy(ctx);

            // Pure Hebrew: "שלום" with default_dir_rtl = true
            let hebrew: Vec<u16> = "שלום".encode_utf16().collect();
            let ctx_rtl = tt_bidi_create(hebrew.as_ptr(), hebrew.len() as i32, true);
            assert!(!ctx_rtl.is_null());
            assert_eq!(tt_bidi_get_direction(ctx_rtl), 1); // RTL
            assert_eq!(tt_bidi_get_visual_run(ctx_rtl, 0, &mut start, &mut len), 1); // RTL run
            tt_bidi_destroy(ctx_rtl);
        }
    }

    #[test]
    fn test_bidi_null_and_empty() {
        // SAFETY: null pointers exercise documented error paths; `ctx` is destroyed once.
        unsafe {
            assert_eq!(tt_bidi_get_direction(std::ptr::null_mut()), 0);
            assert_eq!(tt_bidi_count_runs(std::ptr::null_mut()), 0);
            assert_eq!(
                tt_bidi_get_visual_run(
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut()
                ),
                -1
            );
            tt_bidi_destroy(std::ptr::null_mut());

            let ctx = tt_bidi_create(std::ptr::null(), 0, false);
            assert!(!ctx.is_null());
            assert_eq!(tt_bidi_count_runs(ctx), 0);
            tt_bidi_destroy(ctx);
        }
    }

    #[test]
    fn test_encoding_latin1() {
        use std::ffi::CString;
        let cname = CString::new("iso-8859-1").unwrap();
        // SAFETY: `cname`, `input`, `output`, and `handle` are valid for these calls.
        unsafe {
            assert!(tt_encoding_is_supported(cname.as_ptr()));
            let handle = tt_encoding_open(cname.as_ptr());
            assert!(!handle.is_null());

            // "Café" in ISO-8859-1: 'C', 'a', 'f', 0xE9
            let input = [b'C', b'a', b'f', 0xE9];
            let mut output = [0u32; 10];
            let mut written = 0;

            assert_eq!(
                tt_encoding_to_utf32(
                    handle,
                    input.as_ptr(),
                    input.len(),
                    output.as_mut_ptr(),
                    output.len(),
                    &mut written
                ),
                0
            );

            assert_eq!(written, 4);
            assert_eq!(output[0], 'C' as u32);
            assert_eq!(output[1], 'a' as u32);
            assert_eq!(output[2], 'f' as u32);
            assert_eq!(output[3], 0x00E9); // é

            tt_encoding_close(handle);
        }
    }

    #[test]
    fn test_encoding_null_and_unknown() {
        use std::ffi::CString;
        // SAFETY: null pointers exercise documented error paths; valid handles are closed once.
        unsafe {
            assert!(!tt_encoding_is_supported(std::ptr::null()));
            assert!(tt_encoding_open(std::ptr::null()).is_null());
            tt_encoding_close(std::ptr::null_mut());

            let bad_name = CString::new("completely-unknown-encoding-xyz").unwrap();
            assert!(!tt_encoding_is_supported(bad_name.as_ptr()));
            assert!(tt_encoding_open(bad_name.as_ptr()).is_null());

            let cname = CString::new("utf-8").unwrap();
            let handle = tt_encoding_open(cname.as_ptr());
            assert!(!handle.is_null());

            let mut written = 0;
            assert_eq!(
                tt_encoding_to_utf32(
                    std::ptr::null_mut(),
                    b"test".as_ptr(),
                    4,
                    std::ptr::null_mut(),
                    0,
                    &mut written
                ),
                -1
            );

            tt_encoding_close(handle);
        }
    }
}
