// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust shared font metrics and ingestion engine for Tectonic and FerroTeX.
//!
//! This crate provides:
//! - Complete, safe TeX Font Metric (TFM) parsing and Knuth-accurate integer scaling.
//! - OpenType and TrueType font metric extraction and scaling via `ttf-parser`.
//! - Unified, fixed-point font metric queries (`FontMetricsProvider`, `GlyphMetrics`, `FontDimensions`).
//! - A re-entrant, thread-safe `FontRegistry` shared between the TeX layout engine and PDF backend.

pub mod error;
pub mod metrics;
pub mod opentype;
pub mod registry;
pub mod tfm;

pub use error::{FontError, TfmError};
pub use metrics::{bp_to_sp, sp_to_bp, FontDimensions, FontMetricsProvider, GlyphMetrics};
pub use opentype::{OpenTypeFont, ScaledOpenType};
pub use registry::{sanitize_ps_name, FontRegistry, LoadedFont};
pub use tfm::char_info::{CharInfo, CharTag};
pub use tfm::header::TfmHeader;
pub use tfm::scaler::{round_xn_over_d, xn_over_d, TfmScaler};
pub use tfm::tables::{ExtensibleRecipe, FixWord, FontParams, LigKernStep};
pub use tfm::{ScaledTfm, TfmFont};
