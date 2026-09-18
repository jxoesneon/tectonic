// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust image and PDF dimension and metadata probing crate.
//!
//! This crate decouples `engine_xetex` from legacy `dpx-*` PDF and image
//! headers, enabling pure-Rust extraction of image dimensions and PDF bounding boxes.

pub mod c_api;
pub mod image;
pub mod pdf;

pub use crate::c_api::*;
pub use crate::image::{probe_image_bytes, probe_image_path, ImageInfo, ImageProbeError};
pub use crate::pdf::{
    probe_pdf_box, probe_pdf_box_bytes, probe_pdf_page_count, probe_pdf_page_count_bytes,
    PdfBoxType, PdfProbeError,
};
