// crates/crater/src/lib.rs -- Phase 2 Differential Testing and Parity Harness
// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! `tectonic_crater` is the Phase 2 Differential Testing and Parity Harness for
//! the Tectonic pure-Rust migration, implementing the governance requirements of
//! Gate 1 and Gate 2 in `MIGRATION_ROADMAP.md`.
//!
//! # Purpose
//! Crater provides automated regression and differential verification between the
//! legacy C/C++ TeX/XeTeX reference engine and new pure-Rust components. It guarantees:
//!
//! 1. **Tier A Parity**: Bitwise exact opcode stream equivalence of intermediate `.xdv` outputs.
//! 2. **Tier B Parity**: Normalized PDF stream and visual equivalence (stripping non-deterministic metadata).
//! 3. **Corpus Management**: Structured test suites covering Plain TeX, LaTeX2e, complex macros (`expl3`, `tikz`, `hyperref`), and multi-page stress layouts.

pub mod pdf_diff;
pub mod xdv_diff;

pub use pdf_diff::{PdfComparisonResult, PdfDiffError, PdfNormalizer, VisualComparator};
pub use xdv_diff::{
    BitwiseDivergence, Coordinate, DiffStats, DivergenceKind, XdvBuilder, XdvDiffEngine,
    XdvDivergence, XdvEventCollector, XdvOpcodeEvent,
};

use std::{
    fmt::{self, Display, Formatter},
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tectonic::{
    config::PersistentConfig,
    driver::{OutputFormat, ProcessingSessionBuilder},
    status::ChatterLevel,
};
use tectonic_bundles::Bundle;
use tectonic_status_base::{MessageKind, StatusBackend};
use walkdir::WalkDir;

/// Pluggable factory for instantiating TeX resource bundles for differential runs.
pub type BundleProvider = Arc<dyn Fn() -> Result<Box<dyn Bundle>> + Send + Sync>;

// ============================================================================
// 1. Corpus Management
// ============================================================================

/// Categorical classification of a test case in the Crater corpus.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CorpusCategory {
    /// Primitive Plain TeX documents testing low-level box, glue, and macro mechanics.
    PlainTex,
    /// Standard LaTeX2e documents testing core preamble, packages, and document layout.
    Latex2e,
    /// Advanced macro-heavy documents (expl3, TikZ/PGF, hyperref, amsmath).
    ComplexMacros,
    /// Multi-page documents testing page breaking, shipout, headers, and counters.
    MultiPage,
    /// Custom user-defined or benchmark category.
    Custom(String),
}

impl Display for CorpusCategory {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            CorpusCategory::PlainTex => write!(f, "plain_tex"),
            CorpusCategory::Latex2e => write!(f, "latex2e"),
            CorpusCategory::ComplexMacros => write!(f, "complex_macros"),
            CorpusCategory::MultiPage => write!(f, "multipage"),
            CorpusCategory::Custom(name) => write!(f, "custom({name})"),
        }
    }
}

/// A single TeX/LaTeX test case within the Crater differential corpus.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Unique identifier for this test case.
    pub name: String,
    /// Category classification.
    pub category: CorpusCategory,
    /// Raw TeX/LaTeX source code string.
    pub source: String,
    /// Format name required for compilation (typically "plain" or "latex").
    pub format: String,
    /// Optional human-readable description of test intent.
    pub description: Option<String>,
    /// Searchable tags for selective test suite filtering.
    pub tags: Vec<String>,
}

impl TestCase {
    /// Construct a new test case with default format inference.
    pub fn new(
        name: impl Into<String>,
        category: CorpusCategory,
        source: impl Into<String>,
        format: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            category,
            source: source.into(),
            format: format.into(),
            description: None,
            tags: Vec::new(),
        }
    }

    /// Attach a human-readable description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    /// Add a tag to the test case.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }
}

/// Corpus manager holding a collection of test cases with loading and querying utilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Corpus {
    cases: Vec<TestCase>,
}

impl Corpus {
    /// Create an empty test corpus.
    pub fn new() -> Self {
        Self { cases: Vec::new() }
    }

    /// Create the canonical Crater standard corpus covering all mandatory roadmap tiers:
    /// - Plain TeX primitives and box model
    /// - LaTeX2e minimal document structure
    /// - Complex macros (`expl3`, `tikz`, `hyperref`)
    /// - Multi-page page-breaking and cross-page state
    pub fn standard() -> Self {
        let mut corpus = Self::new();

        // 1. Plain TeX Minimal
        corpus.add_case(
            TestCase::new(
                "plain_minimal",
                CorpusCategory::PlainTex,
                "\\relax Hello, Crater Plain TeX!\\bye\n",
                "plain",
            )
            .with_description("Minimal Plain TeX document asserting baseline TeX stomach initialization")
            .with_tag("smoke")
            .with_tag("plain"),
        );

        // 2. Plain TeX Boxes and Glue
        corpus.add_case(
            TestCase::new(
                "plain_boxes_glue",
                CorpusCategory::PlainTex,
                "\\setbox0=\\hbox{A}\\copy0 \\hskip 20pt \\vbox{\\hrule width 50pt height 1pt}\\bye\n",
                "plain",
            )
            .with_description("Exercises horizontal/vertical boxes and fixed-point glue scaling")
            .with_tag("box")
            .with_tag("plain"),
        );

        // 3. LaTeX2e Minimal Article
        corpus.add_case(
            TestCase::new(
                "latex_minimal",
                CorpusCategory::Latex2e,
                "\\documentclass{article}\n\\begin{document}\nHello from Crater LaTeX2e!\n\\end{document}\n",
                "latex",
            )
            .with_description("Baseline LaTeX2e article compilation checking format file and fonts")
            .with_tag("smoke")
            .with_tag("latex"),
        );

        // 4. Multi-page document
        corpus.add_case(
            TestCase::new(
                "multipage_report",
                CorpusCategory::MultiPage,
                "\\documentclass{article}\n\\begin{document}\nFirst Page Content\n\\newpage\nSecond Page Content\n\\newpage\nThird Page Content\n\\end{document}\n",
                "latex",
            )
            .with_description("Multi-page shipout testing page sequence counters and state preservation")
            .with_tag("pagination")
            .with_tag("multipage"),
        );

        // 5. Complex Macro: expl3
        corpus.add_case(
            TestCase::new(
                "macro_expl3_token_list",
                CorpusCategory::ComplexMacros,
                "\\documentclass{article}\n\\usepackage{expl3}\n\\begin{document}\n\\ExplSyntaxOn\n\\tl_new:N \\l_crater_test_tl\n\\tl_set:Nn \\l_crater_test_tl { Expl3~Parity~Verified }\n\\tl_use:N \\l_crater_test_tl\n\\ExplSyntaxOff\n\\end{document}\n",
                "latex",
            )
            .with_description("Tests expl3 L3 programming layer, catcode transitions, and token lists")
            .with_tag("expl3")
            .with_tag("macro"),
        );

        // 6. Complex Macro: hyperref
        corpus.add_case(
            TestCase::new(
                "macro_hyperref_outlines",
                CorpusCategory::ComplexMacros,
                "\\documentclass{article}\n\\usepackage{hyperref}\n\\begin{document}\n\\section{Crater Section}\\label{sec:first}\nLink to section~\\ref{sec:first}.\n\\end{document}\n",
                "latex",
            )
            .with_description("Tests PDF outlines, bookmarks, and hyperref cross-referencing specials")
            .with_tag("hyperref")
            .with_tag("specials"),
        );

        // 7. Complex Macro: TikZ / PGF
        corpus.add_case(
            TestCase::new(
                "macro_tikz_vector",
                CorpusCategory::ComplexMacros,
                "\\documentclass{article}\n\\usepackage{tikz}\n\\begin{document}\n\\begin{tikzpicture}\n\\draw[thick,->] (0,0) -- (2,2);\n\\end{tikzpicture}\n\\end{document}\n",
                "latex",
            )
            .with_description("Tests TikZ macro parser, coordinate computation, and vector PDF specials")
            .with_tag("tikz")
            .with_tag("vector"),
        );

        corpus
    }

    /// Load a corpus by recursively walking a directory and discovering `.tex` files.
    pub fn from_directory<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let mut corpus = Self::new();
        let root = dir.as_ref();

        for entry in WalkDir::new(root).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "tex") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let source = fs::read_to_string(path)
                    .with_context(|| format!("failed to read test case from {}", path.display()))?;

                let category = if source.contains("\\usepackage{expl3}")
                    || source.contains("\\usepackage{tikz}")
                    || source.contains("\\usepackage{hyperref}")
                {
                    CorpusCategory::ComplexMacros
                } else if source.contains("\\newpage") || source.contains("\\clearpage") {
                    CorpusCategory::MultiPage
                } else if source.contains("\\documentclass") {
                    CorpusCategory::Latex2e
                } else {
                    CorpusCategory::PlainTex
                };

                let format = if source.contains("\\documentclass") {
                    "latex".to_string()
                } else {
                    "plain".to_string()
                };

                corpus.add_case(
                    TestCase::new(name, category, source, format)
                        .with_description(format!("Discovered from {}", path.display())),
                );
            }
        }

        Ok(corpus)
    }

    /// Add a test case to the corpus.
    pub fn add_case(&mut self, case: TestCase) {
        self.cases.push(case);
    }

    /// Immutable slice view of all test cases in the corpus.
    pub fn cases(&self) -> &[TestCase] {
        &self.cases
    }

    /// Total number of test cases in the corpus.
    pub fn len(&self) -> usize {
        self.cases.len()
    }

    /// Check if the corpus contains no test cases.
    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    /// Find a test case by exact name match.
    pub fn get_case(&self, name: &str) -> Option<&TestCase> {
        self.cases.iter().find(|c| c.name == name)
    }

    /// Filter test cases belonging to a specific category.
    pub fn filter_by_category(&self, category: &CorpusCategory) -> Vec<&TestCase> {
        self.cases.iter().filter(|c| &c.category == category).collect()
    }

    /// Filter test cases containing a specific tag.
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&TestCase> {
        self.cases.iter().filter(|c| c.tags.iter().any(|t| t == tag)).collect()
    }
}

// ============================================================================
// 2. Execution Run Results
// ============================================================================

/// Outcome status for an engine execution run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    /// Successful compilation with clean output artifacts.
    Success,
    /// TeX engine reported a compilation error.
    EngineError(String),
    /// Tectonic driver or pipeline failure.
    DriverError(String),
    /// File system or I/O failure.
    IoError(String),
}

impl Display for RunStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            RunStatus::Success => write!(f, "Success"),
            RunStatus::EngineError(err) => write!(f, "EngineError: {err}"),
            RunStatus::DriverError(err) => write!(f, "DriverError: {err}"),
            RunStatus::IoError(err) => write!(f, "IoError: {err}"),
        }
    }
}

/// Detailed execution record capturing timing, logs, and generated artifacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunResult {
    /// Final execution outcome.
    pub status: RunStatus,
    /// Total wall-clock execution duration.
    pub duration: Duration,
    /// Standard output messages emitted during compilation.
    pub stdout: String,
    /// Standard error messages or warnings emitted during compilation.
    pub stderr: String,
    /// Path to intermediate `.xdv` artifact on disk, if preserved.
    pub xdv_path: Option<PathBuf>,
    /// Path to final `.pdf` artifact on disk, if preserved.
    pub pdf_path: Option<PathBuf>,
    /// Raw byte buffer of the generated `.xdv` stream.
    #[serde(skip)]
    pub xdv_bytes: Option<Vec<u8>>,
    /// Raw byte buffer of the generated `.pdf` document.
    #[serde(skip)]
    pub pdf_bytes: Option<Vec<u8>>,
    /// Number of pages rendered in the document.
    pub page_count: usize,
}

impl RunResult {
    /// Returns `true` if the compilation completed successfully.
    pub fn is_success(&self) -> bool {
        matches!(self.status, RunStatus::Success)
    }

    /// Generate a short human-readable diagnostic summary.
    pub fn summary(&self) -> String {
        format!(
            "status={:?}, duration={:?}, pages={}, xdv_len={}, pdf_len={}",
            self.status,
            self.duration,
            self.page_count,
            self.xdv_bytes.as_ref().map_or(0, |b| b.len()),
            self.pdf_bytes.as_ref().map_or(0, |b| b.len()),
        )
    }
}

// ============================================================================
// 3. Status Collector Backend
// ============================================================================

/// Memory-buffered status backend that captures messages for diagnostic reporting.
#[derive(Debug, Clone, Default)]
pub struct MemoryStatusBackend {
    logs: Arc<Mutex<Vec<String>>>,
    warnings: Arc<Mutex<Vec<String>>>,
    errors: Arc<Mutex<Vec<String>>>,
}

impl MemoryStatusBackend {
    /// Create a new status collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any errors were reported.
    pub fn has_errors(&self) -> bool {
        !self.errors.lock().unwrap().is_empty()
    }

    /// Check if any warnings were reported.
    pub fn has_warnings(&self) -> bool {
        !self.warnings.lock().unwrap().is_empty()
    }

    /// Formatted stdout log string.
    pub fn stdout_text(&self) -> String {
        self.logs.lock().unwrap().join("\n")
    }

    /// Formatted stderr string combining warnings and errors.
    pub fn stderr_text(&self) -> String {
        let mut errs = self.warnings.lock().unwrap().clone();
        errs.extend(self.errors.lock().unwrap().clone());
        errs.join("\n")
    }

    /// Dump all collected error logs.
    pub fn dump_error_logs_text(&self) -> String {
        self.errors.lock().unwrap().join("\n")
    }
}

impl StatusBackend for MemoryStatusBackend {
    fn report(
        &mut self,
        kind: MessageKind,
        args: fmt::Arguments,
        _err: Option<&tectonic_errors::Error>,
    ) {
        let msg = format!("{args}");
        match kind {
            MessageKind::Note => self.logs.lock().unwrap().push(msg),
            MessageKind::Warning => self.warnings.lock().unwrap().push(format!("warning: {msg}")),
            MessageKind::Error => self.errors.lock().unwrap().push(format!("error: {msg}")),
        }
    }

    fn dump_error_logs(&mut self, output: &[u8]) {
        if let Ok(text) = std::str::from_utf8(output) {
            self.errors.lock().unwrap().push(text.to_string());
        }
    }
}

// ============================================================================
// 4. Differential Session
// ============================================================================

/// Configuration builder for a [`DifferentialSession`].
#[derive(Clone, Default)]
pub struct DifferentialSessionBuilder {
    output_dir: Option<PathBuf>,
    format_cache_path: Option<PathBuf>,
    keep_intermediates: bool,
    print_stdout: bool,
    chatter_level: Option<ChatterLevel>,
    bundle_provider: Option<BundleProvider>,
}

impl DifferentialSessionBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure a persistent output directory for generated artifacts.
    pub fn output_dir<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.output_dir = Some(path.as_ref().to_owned());
        self
    }

    /// Configure the path to format cache files (`.fmt`).
    pub fn format_cache_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.format_cache_path = Some(path.as_ref().to_owned());
        self
    }

    /// Choose whether to keep intermediate compilation files on disk.
    pub fn keep_intermediates(mut self, keep: bool) -> Self {
        self.keep_intermediates = keep;
        self
    }

    /// Choose whether to echo engine stdout to console.
    pub fn print_stdout(mut self, print: bool) -> Self {
        self.print_stdout = print;
        self
    }

    /// Set chatter level for engine reporting.
    pub fn chatter_level(mut self, level: ChatterLevel) -> Self {
        self.chatter_level = Some(level);
        self
    }

    /// Supply a pluggable bundle provider closure for headless / in-memory runs.
    pub fn bundle_provider<F>(mut self, provider: F) -> Self
    where
        F: Fn() -> Result<Box<dyn Bundle>> + Send + Sync + 'static,
    {
        self.bundle_provider = Some(Arc::new(provider));
        self
    }

    /// Build the configured [`DifferentialSession`].
    pub fn build(self) -> DifferentialSession {
        DifferentialSession {
            output_dir: self.output_dir,
            format_cache_path: self.format_cache_path,
            keep_intermediates: self.keep_intermediates,
            print_stdout: self.print_stdout,
            chatter_level: self.chatter_level.unwrap_or(ChatterLevel::Minimal),
            bundle_provider: self.bundle_provider,
        }
    }
}

/// Execution session that drives compilation and extracts both intermediate `.xdv`
/// and final `.pdf` artifacts for parity comparison.
#[derive(Clone)]
pub struct DifferentialSession {
    output_dir: Option<PathBuf>,
    format_cache_path: Option<PathBuf>,
    keep_intermediates: bool,
    print_stdout: bool,
    chatter_level: ChatterLevel,
    bundle_provider: Option<BundleProvider>,
}

impl Default for DifferentialSession {
    fn default() -> Self {
        Self::new()
    }
}

impl DifferentialSession {
    /// Create a new differential session with default configuration.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Create a configuration builder.
    pub fn builder() -> DifferentialSessionBuilder {
        DifferentialSessionBuilder::default()
    }

    /// Return the chatter level configured for this session.
    pub fn chatter_level(&self) -> ChatterLevel {
        self.chatter_level
    }

    /// Compile a test case using the reference Tectonic pipeline, capturing
    /// intermediate `.xdv` and final `.pdf` artifacts.
    pub fn run(&self, case: &TestCase) -> Result<RunResult> {
        let start_time = Instant::now();

        // 1. Establish working and output directories
        let temp_dir = tempfile::Builder::new()
            .prefix("crater_run_")
            .tempdir()
            .context("failed to create temporary working directory")?;
        let work_dir = match &self.output_dir {
            Some(dir) => {
                fs::create_dir_all(dir).context("failed to create output directory")?;
                dir.clone()
            }
            None => temp_dir.path().to_path_buf(),
        };

        let mut status = MemoryStatusBackend::new();

        // 2. Resolve Tectonic configuration and bundles
        let (bundle, format_cache) = if let Some(ref provider) = self.bundle_provider {
            let b = match provider() {
                Ok(b) => b,
                Err(e) => {
                    return Ok(RunResult {
                        status: RunStatus::DriverError(format!("Failed to load bundle: {e}")),
                        duration: start_time.elapsed(),
                        stdout: status.stdout_text(),
                        stderr: format!("Bundle error: {e}"),
                        xdv_path: None,
                        pdf_path: None,
                        xdv_bytes: None,
                        pdf_bytes: None,
                        page_count: 0,
                    });
                }
            };
            let fc = match &self.format_cache_path {
                Some(p) => p.clone(),
                None => temp_dir.path().to_path_buf(),
            };
            (b, fc)
        } else {
            let config = match PersistentConfig::open(false) {
                Ok(c) => c,
                Err(e) => {
                    return Ok(RunResult {
                        status: RunStatus::DriverError(format!("Failed to open config: {e}")),
                        duration: start_time.elapsed(),
                        stdout: status.stdout_text(),
                        stderr: format!("Config error: {e}"),
                        xdv_path: None,
                        pdf_path: None,
                        xdv_bytes: None,
                        pdf_bytes: None,
                        page_count: 0,
                    });
                }
            };
            let b = match config.default_bundle(false) {
                Ok(b) => b,
                Err(e) => {
                    return Ok(RunResult {
                        status: RunStatus::DriverError(format!("Failed to load default bundle: {e}")),
                        duration: start_time.elapsed(),
                        stdout: status.stdout_text(),
                        stderr: format!("Bundle error: {e}"),
                        xdv_path: None,
                        pdf_path: None,
                        xdv_bytes: None,
                        pdf_bytes: None,
                        page_count: 0,
                    });
                }
            };
            let fc = match &self.format_cache_path {
                Some(p) => p.clone(),
                None => config
                    .format_cache_path()
                    .unwrap_or_else(|_| temp_dir.path().to_path_buf()),
            };
            (b, fc)
        };

        let input_name = format!("{}.tex", case.name);

        // 3. Pass 1: TeX Stomach -> Emit Intermediate XDV
        let mut xdv_builder = ProcessingSessionBuilder::default();
        xdv_builder
            .bundle(bundle)
            .primary_input_buffer(case.source.as_bytes())
            .tex_input_name(&input_name)
            .format_name(&case.format)
            .format_cache_path(format_cache.clone())
            .keep_intermediates(true)
            .output_format(OutputFormat::Xdv)
            .output_dir(&work_dir)
            .print_stdout(self.print_stdout);

        let mut xdv_session = match xdv_builder.create(&mut status) {
            Ok(s) => s,
            Err(e) => {
                return Ok(RunResult {
                    status: RunStatus::DriverError(format!("{e}")),
                    duration: start_time.elapsed(),
                    stdout: status.stdout_text(),
                    stderr: status.stderr_text(),
                    xdv_path: None,
                    pdf_path: None,
                    xdv_bytes: None,
                    pdf_bytes: None,
                    page_count: 0,
                });
            }
        };

        let xdv_res = xdv_session.run(&mut status);
        if let Err(e) = xdv_res {
            return Ok(RunResult {
                status: RunStatus::EngineError(format!("{e}")),
                duration: start_time.elapsed(),
                stdout: status.stdout_text(),
                stderr: status.stderr_text(),
                xdv_path: None,
                pdf_path: None,
                xdv_bytes: None,
                pdf_bytes: None,
                page_count: 0,
            });
        }

        // Locate and read generated XDV
        let expected_xdv_path = work_dir.join(format!("{}.xdv", case.name));
        let xdv_bytes = if expected_xdv_path.exists() {
            Some(fs::read(&expected_xdv_path).context("failed to read generated XDV file")?)
        } else {
            None
        };

        // 4. Pass 2: Pipeline -> Emit Final PDF
        let bundle2 = if let Some(ref provider) = self.bundle_provider {
            match provider() {
                Ok(b) => b,
                Err(e) => {
                    return Ok(RunResult {
                        status: RunStatus::DriverError(format!("Failed to reload bundle for Pass 2: {e}")),
                        duration: start_time.elapsed(),
                        stdout: status.stdout_text(),
                        stderr: format!("Bundle error: {e}"),
                        xdv_path: if expected_xdv_path.exists() {
                            Some(expected_xdv_path)
                        } else {
                            None
                        },
                        pdf_path: None,
                        xdv_bytes,
                        pdf_bytes: None,
                        page_count: 0,
                    });
                }
            }
        } else {
            let config2 = match PersistentConfig::open(false) {
                Ok(c) => c,
                Err(e) => {
                    return Ok(RunResult {
                        status: RunStatus::DriverError(format!("{e}")),
                        duration: start_time.elapsed(),
                        stdout: status.stdout_text(),
                        stderr: format!("Config error: {e}"),
                        xdv_path: if expected_xdv_path.exists() {
                            Some(expected_xdv_path)
                        } else {
                            None
                        },
                        pdf_path: None,
                        xdv_bytes,
                        pdf_bytes: None,
                        page_count: 0,
                    });
                }
            };
            match config2.default_bundle(false) {
                Ok(b) => b,
                Err(e) => {
                    return Ok(RunResult {
                        status: RunStatus::DriverError(format!("{e}")),
                        duration: start_time.elapsed(),
                        stdout: status.stdout_text(),
                        stderr: format!("Bundle error: {e}"),
                        xdv_path: if expected_xdv_path.exists() {
                            Some(expected_xdv_path)
                        } else {
                            None
                        },
                        pdf_path: None,
                        xdv_bytes,
                        pdf_bytes: None,
                        page_count: 0,
                    });
                }
            }
        };

        let mut pdf_builder = ProcessingSessionBuilder::default();
        pdf_builder
            .bundle(bundle2)
            .primary_input_buffer(case.source.as_bytes())
            .tex_input_name(&input_name)
            .format_name(&case.format)
            .format_cache_path(format_cache)
            .keep_intermediates(self.keep_intermediates)
            .output_format(OutputFormat::Pdf)
            .output_dir(&work_dir)
            .print_stdout(self.print_stdout);

        let mut pdf_session = match pdf_builder.create(&mut status) {
            Ok(s) => s,
            Err(e) => {
                return Ok(RunResult {
                    status: RunStatus::DriverError(format!("{e}")),
                    duration: start_time.elapsed(),
                    stdout: status.stdout_text(),
                    stderr: status.stderr_text(),
                    xdv_path: if expected_xdv_path.exists() {
                        Some(expected_xdv_path)
                    } else {
                        None
                    },
                    pdf_path: None,
                    xdv_bytes,
                    pdf_bytes: None,
                    page_count: 0,
                });
            }
        };

        let pdf_res = pdf_session.run(&mut status);
        if let Err(e) = pdf_res {
            return Ok(RunResult {
                status: RunStatus::DriverError(format!("{e}")),
                duration: start_time.elapsed(),
                stdout: status.stdout_text(),
                stderr: status.stderr_text(),
                xdv_path: if expected_xdv_path.exists() {
                    Some(expected_xdv_path)
                } else {
                    None
                },
                pdf_path: None,
                xdv_bytes,
                pdf_bytes: None,
                page_count: 0,
            });
        }

        let expected_pdf_path = work_dir.join(format!("{}.pdf", case.name));
        let pdf_bytes = if expected_pdf_path.exists() {
            Some(fs::read(&expected_pdf_path).context("failed to read generated PDF file")?)
        } else {
            None
        };

        // 5. Count pages from XDV stream
        let page_count = if let Some(ref bytes) = xdv_bytes {
            let collector = xdv_diff::XdvDiffEngine::parse_with_tectonic_xdv(Cursor::new(bytes));
            collector.map(|c| c.pages.len()).unwrap_or(1)
        } else {
            1
        };

        Ok(RunResult {
            status: RunStatus::Success,
            duration: start_time.elapsed(),
            stdout: status.stdout_text(),
            stderr: status.stderr_text(),
            xdv_path: if expected_xdv_path.exists() {
                Some(expected_xdv_path)
            } else {
                None
            },
            pdf_path: if expected_pdf_path.exists() {
                Some(expected_pdf_path)
            } else {
                None
            },
            xdv_bytes,
            pdf_bytes,
            page_count,
        })
    }
}

// ============================================================================
// 5. Two-Tier Parity Harness Verification
// ============================================================================

/// Status result of a differential parity check between two engines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParityStatus {
    /// 1:1 Bitwise byte-for-byte exact match.
    ExactMatch,
    /// Normalized match (non-deterministic metadata stripped, AST identical).
    NormalizedMatch,
    /// Engine output divergence detected.
    Diverged(String),
}

impl Display for ParityStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            ParityStatus::ExactMatch => write!(f, "Exact Bitwise Match"),
            ParityStatus::NormalizedMatch => write!(f, "Normalized Match (Gate 2 Passed)"),
            ParityStatus::Diverged(msg) => write!(f, "Diverged: {msg}"),
        }
    }
}

/// Full differential report comparing reference and candidate execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferentialReport {
    /// Target test case name.
    pub case_name: String,
    /// Tier A (Intermediate XDV) verification status.
    pub tier_a_status: ParityStatus,
    /// Tier A opcode divergence details if mismatched.
    pub tier_a_diff: Option<String>,
    /// Tier B (Normalized PDF) verification status.
    pub tier_b_status: ParityStatus,
    /// Tier B stream divergence details if mismatched.
    pub tier_b_diff: Option<String>,
    /// Reference engine wall-clock execution time.
    pub reference_duration: Duration,
    /// Candidate engine wall-clock execution time.
    pub candidate_duration: Option<Duration>,
}

impl DifferentialReport {
    /// Returns `true` if both Tier A and Tier B parity checks passed.
    pub fn is_parity_passed(&self) -> bool {
        matches!(
            self.tier_a_status,
            ParityStatus::ExactMatch | ParityStatus::NormalizedMatch
        ) && matches!(
            self.tier_b_status,
            ParityStatus::ExactMatch | ParityStatus::NormalizedMatch
        )
    }

    /// Returns `true` if any divergence was detected in Tier A or Tier B.
    pub fn has_divergences(&self) -> bool {
        !self.is_parity_passed()
    }

    /// Alias for `is_parity_passed` for test clarity.
    pub fn passed_all(&self) -> bool {
        self.is_parity_passed()
    }

    /// Return a human-readable summary of the differential report.
    pub fn summary(&self) -> String {
        if self.is_parity_passed() {
            format!("[{}] PASSED (Parity verified)", self.case_name)
        } else {
            format!(
                "[{}] DIVERGED - Tier A: {}, Tier B: {}",
                self.case_name, self.tier_a_status, self.tier_b_status
            )
        }
    }
}

/// Compare intermediate XDV streams (Tier A Parity).
pub fn compare_tier_a_xdv(reference: &[u8], candidate: &[u8]) -> (ParityStatus, Option<String>) {
    match xdv_diff::XdvDiffEngine::compare_bytes(reference, candidate) {
        Ok(_) => (ParityStatus::ExactMatch, None),
        Err(div) => (
            ParityStatus::Diverged(div.to_string()),
            Some(div.to_string()),
        ),
    }
}

/// Compare final PDF outputs (Tier B Parity).
pub fn compare_tier_b_pdf(reference: &[u8], candidate: &[u8]) -> (ParityStatus, Option<String>) {
    match pdf_diff::compare_pdfs(reference, candidate, 300) {
        Ok(res) => {
            if res.passes_gate2(0.001) {
                (ParityStatus::NormalizedMatch, None)
            } else {
                let msg = format!(
                    "PDF Tier B Divergence: stream_matches={}, visual_diff_score={:.4}%, divergent_pages={:?}",
                    res.stream_matches, res.visual_diff_score, res.divergent_pages
                );
                (ParityStatus::Diverged(msg.clone()), Some(msg))
            }
        }
        Err(e) => (
            ParityStatus::Diverged(format!("PDF compare error: {e}")),
            Some(e.to_string()),
        ),
    }
}

/// Compare complete reference and candidate run results, producing a [`DifferentialReport`].
pub fn compare_runs(
    case_name: &str,
    reference: &RunResult,
    candidate: &RunResult,
) -> DifferentialReport {
    if !reference.is_success() {
        let err_msg = format!("Reference run failed: {:?}", reference.status);
        return DifferentialReport {
            case_name: case_name.to_string(),
            tier_a_status: ParityStatus::Diverged(err_msg.clone()),
            tier_a_diff: Some(err_msg.clone()),
            tier_b_status: ParityStatus::Diverged(err_msg.clone()),
            tier_b_diff: Some(err_msg),
            reference_duration: reference.duration,
            candidate_duration: Some(candidate.duration),
        };
    }
    if !candidate.is_success() {
        let err_msg = format!("Candidate run failed: {:?}", candidate.status);
        return DifferentialReport {
            case_name: case_name.to_string(),
            tier_a_status: ParityStatus::Diverged(err_msg.clone()),
            tier_a_diff: Some(err_msg.clone()),
            tier_b_status: ParityStatus::Diverged(err_msg.clone()),
            tier_b_diff: Some(err_msg),
            reference_duration: reference.duration,
            candidate_duration: Some(candidate.duration),
        };
    }

    let (tier_a_status, tier_a_diff) = match (&reference.xdv_bytes, &candidate.xdv_bytes) {
        (Some(r), Some(c)) => compare_tier_a_xdv(r, c),
        _ => (
            ParityStatus::Diverged("Missing intermediate XDV output".to_string()),
            Some("One or both runs did not generate XDV".to_string()),
        ),
    };

    let (tier_b_status, tier_b_diff) = match (&reference.pdf_bytes, &candidate.pdf_bytes) {
        (Some(r), Some(c)) => compare_tier_b_pdf(r, c),
        _ => (
            ParityStatus::Diverged("Missing final PDF output".to_string()),
            Some("One or both runs did not generate PDF".to_string()),
        ),
    };

    DifferentialReport {
        case_name: case_name.to_string(),
        tier_a_status,
        tier_a_diff,
        tier_b_status,
        tier_b_diff,
        reference_duration: reference.duration,
        candidate_duration: Some(candidate.duration),
    }
}

// ============================================================================
// 6. Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_corpus_coverage() {
        let corpus = Corpus::standard();

        assert!(!corpus.is_empty());
        assert!(corpus.len() >= 7, "standard corpus must contain >= 7 baseline tests");

        let plain_cases = corpus.filter_by_category(&CorpusCategory::PlainTex);
        assert!(!plain_cases.is_empty(), "must contain plain TeX cases");

        let latex_cases = corpus.filter_by_category(&CorpusCategory::Latex2e);
        assert!(!latex_cases.is_empty(), "must contain LaTeX2e cases");

        let macro_cases = corpus.filter_by_category(&CorpusCategory::ComplexMacros);
        assert!(macro_cases.len() >= 3, "must cover expl3, tikz, and hyperref");

        let multipage_cases = corpus.filter_by_category(&CorpusCategory::MultiPage);
        assert!(!multipage_cases.is_empty(), "must cover multi-page documents");

        let smoke_tags = corpus.filter_by_tag("smoke");
        assert!(smoke_tags.len() >= 2);
    }

    #[test]
    fn test_corpus_directory_loading() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file1 = temp_dir.path().join("case1.tex");
        let file2 = temp_dir.path().join("case2.tex");

        fs::write(&file1, "\\relax Hello from disk\\bye\n").unwrap();
        fs::write(
            &file2,
            "\\documentclass{article}\n\\usepackage{expl3}\n\\begin{document}Hi\\end{document}\n",
        )
        .unwrap();

        let corpus = Corpus::from_directory(temp_dir.path()).unwrap();
        assert_eq!(corpus.len(), 2);

        let case1 = corpus.get_case("case1").unwrap();
        assert_eq!(case1.category, CorpusCategory::PlainTex);
        assert_eq!(case1.format, "plain");

        let case2 = corpus.get_case("case2").unwrap();
        assert_eq!(case2.category, CorpusCategory::ComplexMacros);
        assert_eq!(case2.format, "latex");
    }

    #[test]
    fn test_run_result_and_report_serialization() {
        let ref_res = RunResult {
            status: RunStatus::Success,
            duration: Duration::from_millis(150),
            stdout: "Compilation finished".into(),
            stderr: String::new(),
            xdv_path: Some(PathBuf::from("/tmp/test.xdv")),
            pdf_path: Some(PathBuf::from("/tmp/test.pdf")),
            xdv_bytes: Some(vec![1, 2, 3]),
            pdf_bytes: Some(vec![4, 5, 6]),
            page_count: 1,
        };

        let cand_res = ref_res.clone();
        let report = compare_runs("sample_case", &ref_res, &cand_res);

        assert_eq!(report.case_name, "sample_case");
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("sample_case"));
    }

    #[test]
    fn test_corpus_category_and_test_case_methods() {
        assert_eq!(format!("{}", CorpusCategory::PlainTex), "plain_tex");
        assert_eq!(format!("{}", CorpusCategory::Latex2e), "latex2e");
        assert_eq!(format!("{}", CorpusCategory::ComplexMacros), "complex_macros");
        assert_eq!(format!("{}", CorpusCategory::MultiPage), "multipage");
        assert_eq!(format!("{}", CorpusCategory::Custom("benchmarks".into())), "custom(benchmarks)");

        let case = TestCase::new("test_case", CorpusCategory::PlainTex, "\\relax", "plain")
            .with_tag("fast")
            .with_tag("math")
            .with_description("A quick test case");

        assert_eq!(case.name, "test_case");
        assert_eq!(case.format, "plain");
        assert_eq!(case.tags, vec!["fast", "math"]);
        assert_eq!(case.description.as_deref(), Some("A quick test case"));

        let mut corpus = Corpus::new();
        assert!(corpus.is_empty());
        corpus.add_case(case);
        assert_eq!(corpus.len(), 1);
        assert!(corpus.get_case("non_existent").is_none());
        assert_eq!(corpus.filter_by_tag("non_existent").len(), 0);
        assert_eq!(corpus.filter_by_tag("fast").len(), 1);
        assert_eq!(corpus.cases().len(), 1);
    }

    #[test]
    fn test_run_status_and_memory_status_backend() {
        assert_eq!(format!("{}", RunStatus::Success), "Success");
        assert_eq!(format!("{}", RunStatus::EngineError("fatal error".into())), "EngineError: fatal error");
        assert_eq!(format!("{}", RunStatus::DriverError("driver error".into())), "DriverError: driver error");
        assert_eq!(format!("{}", RunStatus::IoError("disk full".into())), "IoError: disk full");

        let mut status = MemoryStatusBackend::new();
        use tectonic_status_base::{MessageKind, StatusBackend};
        status.report(MessageKind::Note, format_args!("Note message"), None);
        status.report(MessageKind::Warning, format_args!("Warning message"), None);
        status.report(MessageKind::Error, format_args!("Error message"), None);
        status.dump_error_logs(b"Dumped error trace");

        assert!(status.has_errors());
        assert!(status.has_warnings());
        assert!(status.stdout_text().contains("Note message"));
        assert!(status.stderr_text().contains("Warning message"));
        assert!(status.stderr_text().contains("Error message"));
        assert!(status.dump_error_logs_text().contains("Dumped error trace"));
    }

    #[test]
    fn test_differential_session_builder_and_getters() {
        let temp_dir = tempfile::tempdir().unwrap();
        let builder = DifferentialSession::builder()
            .output_dir(temp_dir.path())
            .format_cache_path(temp_dir.path())
            .keep_intermediates(true)
            .print_stdout(false)
            .chatter_level(ChatterLevel::Minimal);

        let session = builder.build();
        assert_eq!(session.chatter_level(), ChatterLevel::Minimal);

        let default_session = DifferentialSession::default();
        assert_eq!(default_session.chatter_level(), ChatterLevel::Minimal);
    }

    #[test]
    fn test_parity_status_and_report_summary() {
        assert_eq!(format!("{}", ParityStatus::ExactMatch), "Exact Bitwise Match");
        assert_eq!(format!("{}", ParityStatus::NormalizedMatch), "Normalized Match (Gate 2 Passed)");
        assert_eq!(format!("{}", ParityStatus::Diverged("glyph shift".into())), "Diverged: glyph shift");

        let clean_report = DifferentialReport {
            case_name: "clean_doc".into(),
            tier_a_status: ParityStatus::ExactMatch,
            tier_a_diff: None,
            tier_b_status: ParityStatus::NormalizedMatch,
            tier_b_diff: None,
            reference_duration: Duration::from_millis(100),
            candidate_duration: Some(Duration::from_millis(90)),
        };
        assert!(clean_report.passed_all());
        assert!(!clean_report.has_divergences());
        assert!(clean_report.summary().contains("PASSED"));

        let diverged_report = DifferentialReport {
            case_name: "diverged_doc".into(),
            tier_a_status: ParityStatus::Diverged("coord mismatch".into()),
            tier_a_diff: Some("coord mismatch".into()),
            tier_b_status: ParityStatus::ExactMatch,
            tier_b_diff: None,
            reference_duration: Duration::from_millis(100),
            candidate_duration: Some(Duration::from_millis(100)),
        };
        assert!(!diverged_report.passed_all());
        assert!(diverged_report.has_divergences());
        assert!(diverged_report.summary().contains("DIVERGED"));
    }

    #[test]
    fn test_compare_runs_all_divergence_branches() {
        // 1. Reference engine error
        let ref_fail = RunResult {
            status: RunStatus::EngineError("Reference boom".into()),
            duration: Duration::from_millis(10),
            stdout: String::new(),
            stderr: "Reference error".into(),
            xdv_path: None,
            pdf_path: None,
            xdv_bytes: None,
            pdf_bytes: None,
            page_count: 0,
        };
        let cand_ok = RunResult {
            status: RunStatus::Success,
            duration: Duration::from_millis(10),
            stdout: String::new(),
            stderr: String::new(),
            xdv_path: None,
            pdf_path: None,
            xdv_bytes: Some(vec![1, 2, 3]),
            pdf_bytes: Some(vec![4, 5, 6]),
            page_count: 1,
        };
        let rep1 = compare_runs("ref_fail_case", &ref_fail, &cand_ok);
        assert!(matches!(rep1.tier_a_status, ParityStatus::Diverged(_)));

        // 2. Candidate engine error
        let rep2 = compare_runs("cand_fail_case", &cand_ok, &ref_fail);
        assert!(matches!(rep2.tier_a_status, ParityStatus::Diverged(_)));

        // 3. Missing XDV in reference
        let ref_missing_xdv = RunResult {
            xdv_bytes: None,
            ..cand_ok.clone()
        };
        let rep3 = compare_runs("missing_xdv_ref", &ref_missing_xdv, &cand_ok);
        assert!(matches!(rep3.tier_a_status, ParityStatus::Diverged(_)));

        // 4. Missing XDV in candidate
        let rep4 = compare_runs("missing_xdv_cand", &cand_ok, &ref_missing_xdv);
        assert!(matches!(rep4.tier_a_status, ParityStatus::Diverged(_)));

        // 5. Missing PDF in reference
        let ref_missing_pdf = RunResult {
            pdf_bytes: None,
            ..cand_ok.clone()
        };
        let rep5 = compare_runs("missing_pdf_ref", &ref_missing_pdf, &cand_ok);
        assert!(matches!(rep5.tier_b_status, ParityStatus::Diverged(_)));

        // 6. Missing PDF in candidate
        let rep6 = compare_runs("missing_pdf_cand", &cand_ok, &ref_missing_pdf);
        assert!(matches!(rep6.tier_b_status, ParityStatus::Diverged(_)));

        // 7. XDV byte divergence
        let cand_diff_xdv = RunResult {
            xdv_bytes: Some(vec![1, 9, 3]),
            ..cand_ok.clone()
        };
        let rep7 = compare_runs("diff_xdv_case", &cand_ok, &cand_diff_xdv);
        assert!(matches!(rep7.tier_a_status, ParityStatus::Diverged(_)));

        // 8. PDF stream divergence
        let cand_diff_pdf = RunResult {
            pdf_bytes: Some(vec![9, 9, 9]),
            ..cand_ok.clone()
        };
        let rep8 = compare_runs("diff_pdf_case", &cand_ok, &cand_diff_pdf);
        assert!(matches!(rep8.tier_b_status, ParityStatus::Diverged(_)));
    }

    #[test]
    fn test_corpus_directory_multipage_and_latex() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file_multi = temp_dir.path().join("multi.tex");
        let file_latex = temp_dir.path().join("latex_simple.tex");

        fs::write(&file_multi, "\\relax Page 1\\newpage Page 2\\bye\n").unwrap();
        fs::write(
            &file_latex,
            "\\documentclass{minimal}\n\\begin{document}Hello Simple LaTeX\\end{document}\n",
        )
        .unwrap();

        let corpus = Corpus::from_directory(temp_dir.path()).unwrap();
        assert_eq!(corpus.len(), 2);

        let case_multi = corpus.get_case("multi").unwrap();
        assert_eq!(case_multi.category, CorpusCategory::MultiPage);

        let case_latex = corpus.get_case("latex_simple").unwrap();
        assert_eq!(case_latex.category, CorpusCategory::Latex2e);
    }

    #[test]
    fn test_run_result_helpers_and_builder_options() {
        let res_success = RunResult {
            status: RunStatus::Success,
            duration: Duration::from_millis(50),
            stdout: "Ok".into(),
            stderr: String::new(),
            xdv_path: None,
            pdf_path: None,
            xdv_bytes: Some(vec![1, 2]),
            pdf_bytes: Some(vec![3, 4]),
            page_count: 1,
        };
        assert!(res_success.is_success());
        let summary_str = res_success.summary();
        assert!(summary_str.contains("Success"));
        assert!(summary_str.contains("xdv_len=2"));
        assert!(summary_str.contains("pdf_len=2"));

        let res_err = RunResult {
            status: RunStatus::EngineError("fatal".into()),
            duration: Duration::from_millis(50),
            stdout: String::new(),
            stderr: "fatal".into(),
            xdv_path: None,
            pdf_path: None,
            xdv_bytes: None,
            pdf_bytes: None,
            page_count: 0,
        };
        assert!(!res_err.is_success());
        assert!(res_err.summary().contains("xdv_len=0"));

        let b = DifferentialSessionBuilder::new()
            .output_dir("/tmp/test_out")
            .format_cache_path("/tmp/test_fmt")
            .keep_intermediates(false)
            .print_stdout(true)
            .chatter_level(ChatterLevel::Minimal);
        let s = b.build();
        assert_eq!(s.chatter_level(), ChatterLevel::Minimal);
    }

    #[test]
    fn test_differential_session_run_mock_bundle_providers() {
        let case = TestCase::new("test_case", CorpusCategory::PlainTex, "\\relax", "plain");

        // 1. Session with bundle provider returning error
        let session_err = DifferentialSession::builder()
            .bundle_provider(|| Err(anyhow::anyhow!("mock bundle failure")))
            .build();
        let res_err = session_err.run(&case).unwrap();
        assert!(matches!(res_err.status, RunStatus::DriverError(_)));

        // 2. Session with DirBundle provider failing format cache lookup
        let temp_dir = tempfile::tempdir().unwrap();
        let dir_path = temp_dir.path().to_path_buf();
        let session_dir = DifferentialSession::builder()
            .bundle_provider(move || Ok(Box::new(tectonic_bundles::dir::DirBundle::new(&dir_path))))
            .build();
        let res_dir = session_dir.run(&case).unwrap();
        assert!(matches!(res_dir.status, RunStatus::DriverError(_)));

        // 3. Session with default configuration fallback
        let session_default = DifferentialSession::default();
        let res_default = session_default.run(&case).unwrap();
        // Returns DriverError when no default bundle is present in offline test harness
        assert!(!res_default.is_success());
    }

    #[test]
    fn test_compare_tier_a_and_b_direct() {
        use crate::xdv_diff::XdvBuilder;

        // 1. Tier A exact match
        let xdv_bytes = XdvBuilder::new().begin_page(1).end_page().build();
        let (status_a, diff_a) = compare_tier_a_xdv(&xdv_bytes, &xdv_bytes);
        assert_eq!(status_a, ParityStatus::ExactMatch);
        assert!(diff_a.is_none());

        // 2. Tier A divergence
        let (status_a_div, diff_a_div) = compare_tier_a_xdv(&xdv_bytes, b"corrupted");
        assert!(matches!(status_a_div, ParityStatus::Diverged(_)));
        assert!(diff_a_div.is_some());

        // 3. Tier B identical
        let pdf_sample = b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< >>\n%%EOF";
        let (status_b, diff_b) = compare_tier_b_pdf(pdf_sample, pdf_sample);
        assert_eq!(status_b, ParityStatus::NormalizedMatch);
        assert!(diff_b.is_none());

        // 4. Tier B parse error
        let (status_b_err, diff_b_err) = compare_tier_b_pdf(b"not a pdf", b"also not a pdf");
        assert!(matches!(status_b_err, ParityStatus::Diverged(_)));
        assert!(diff_b_err.is_some());
    }
}
