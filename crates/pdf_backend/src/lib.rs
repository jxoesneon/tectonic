// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Pure-Rust declarative XDV-to-PDF generation engine for Tectonic.
//!
//! This crate provides [`PdfBackendEngine`], a pure-Rust replacement for the legacy
//! C `xdvipdfmx` engine, fulfilling Phase 3b of the Tectonic Pure-Rust Migration Roadmap.
//! It parses `.xdv` files using [`tectonic_xdv::XdvParser`] and produces optimized,
//! standards-compliant PDF documents using [`pdf_writer`].

#![deny(missing_docs)]
#![deny(clippy::undocumented_unsafe_blocks)]

pub mod fonts;
pub mod pdf;
pub mod pipeline;
pub mod specials;
pub mod units;

use std::io::{Cursor, Write};
use std::time::SystemTime;
use tectonic_bridge_core::{CoreBridgeLauncher, FileFormat};
use tectonic_errors::prelude::*;
use tectonic_xdv::XdvParser;

pub use crate::pipeline::XdvToPdfPipeline;
pub use crate::units::PdfRect;

/// Configuration options for the pure-Rust PDF backend engine.
#[derive(Clone, Debug)]
pub struct PdfBackendConfig {
    /// Initial paper specification name (e.g. "letter", "a4").
    pub paper_spec: String,
    /// Build timestamp for reproducible PDF metadata.
    pub build_date: SystemTime,
    /// Enable PDF stream compression.
    pub enable_compression: bool,
    /// Enforce deterministic font and object tags.
    pub deterministic_tags: bool,
}

impl Default for PdfBackendConfig {
    fn default() -> Self {
        Self {
            paper_spec: "letter".to_string(),
            build_date: SystemTime::UNIX_EPOCH,
            enable_compression: true,
            deterministic_tags: true,
        }
    }
}

/// The pure-Rust PDF backend engine for converting XDV streams to PDF.
#[derive(Default, Debug)]
pub struct PdfBackendEngine {
    config: PdfBackendConfig,
}

impl PdfBackendEngine {
    /// Creates a new pure-Rust PDF backend engine with default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the paper specification name (e.g. "a4", "letter").
    pub fn paper_spec(&mut self, spec: impl Into<String>) -> &mut Self {
        self.config.paper_spec = spec.into();
        self
    }

    /// Sets the build date for metadata.
    pub fn build_date(&mut self, date: SystemTime) -> &mut Self {
        self.config.build_date = date;
        self
    }

    /// Sets whether stream compression is enabled.
    pub fn enable_compression(&mut self, enabled: bool) -> &mut Self {
        self.config.enable_compression = enabled;
        self
    }

    /// Sets whether deterministic tags are enabled.
    pub fn enable_deterministic_tags(&mut self, enabled: bool) -> &mut Self {
        self.config.deterministic_tags = enabled;
        self
    }

    /// Processes an in-memory XDV byte slice and returns the generated PDF bytes.
    pub fn process_bytes(&self, xdv_bytes: &[u8]) -> Result<Vec<u8>> {
        let mut pipeline = XdvToPdfPipeline::new();

        // Apply paper spec default
        if self.config.paper_spec.to_lowercase() == "a4" {
            // A4: 210mm x 297mm in bp
            pipeline.set_page_dimensions(210.0 * 72.0 / 25.4, 297.0 * 72.0 / 25.4);
        } else {
            // Default Letter: 8.5in x 11in
            pipeline.set_page_dimensions(8.5 * 72.0, 11.0 * 72.0);
        }

        let reader = Cursor::new(xdv_bytes);
        let (finished_pipeline, _bytes_read) = XdvParser::process(reader, pipeline)
            .map_err(|e| Error::msg(format!("PDF pipeline failed: {e:?}")))?;

        finished_pipeline
            .finish()
            .map_err(|e| Error::msg(format!("Failed to serialize PDF: {e}")))
    }

    /// Processes an XDV file using Tectonic's VFS bridge and writes the output PDF.
    pub fn process(
        &mut self,
        launcher: &mut CoreBridgeLauncher,
        dvi_name: &str,
        pdf_name: &str,
    ) -> Result<()> {
        let pdf_bytes = launcher.with_global_lock(|state| {
            let in_id = state
                .input_open(dvi_name, FileFormat::ProgramData, false)
                .ok_or_else(|| Error::msg(format!("unable to open input XDV: {dvi_name}")))?;
            let size = state.input_get_size(in_id);
            let mut xdv_bytes = vec![0u8; size];
            state.input_read(in_id, &mut xdv_bytes)?;
            state.input_close(in_id);

            self.process_bytes(&xdv_bytes)
        })?;

        launcher.with_global_lock(|state| {
            let out_id = state
                .output_open(pdf_name, false)
                .ok_or_else(|| Error::msg(format!("unable to open output PDF: {pdf_name}")))?;
            state.get_output(out_id).write_all(&pdf_bytes)?;
            state.output_close(out_id);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_engine_builder_configuration() {
        let mut engine = PdfBackendEngine::new();
        engine
            .paper_spec("a4")
            .enable_compression(false)
            .enable_deterministic_tags(true)
            .build_date(SystemTime::UNIX_EPOCH);

        assert_eq!(engine.config.paper_spec, "a4");
        assert!(!engine.config.enable_compression);
        assert!(engine.config.deterministic_tags);
    }

    #[test]
    fn test_process_bytes_invalid_input() {
        let engine = PdfBackendEngine::new();
        let err = engine.process_bytes(b"not a valid xdv stream").unwrap_err();
        assert!(err.to_string().contains("PDF pipeline failed"));
    }

    #[test]
    fn test_process_missing_file() {
        use std::collections::HashSet;
        use tectonic_bridge_core::MinimalDriver;
        use tectonic_io_base::filesystem::FilesystemIo;
        use tectonic_status_base::NoopStatusBackend;

        let temp_dir = tempfile::tempdir().unwrap();
        let fs_io = FilesystemIo::new(temp_dir.path(), true, true, HashSet::new());
        let mut driver = MinimalDriver::new(fs_io);
        let mut status = NoopStatusBackend::default();
        let mut launcher = CoreBridgeLauncher::new(&mut driver, &mut status);

        let mut engine = PdfBackendEngine::default();
        let err = engine.process(&mut launcher, "nonexistent.xdv", "output.pdf").unwrap_err();
        assert!(err.to_string().contains("unable to open input XDV"));
    }
}
