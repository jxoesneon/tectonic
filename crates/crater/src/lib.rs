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
use tectonic_status_base::{MessageKind, StatusBackend};
use walkdir::WalkDir;

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
#[derive(Debug, Clone, Default)]
pub struct DifferentialSessionBuilder {
    output_dir: Option<PathBuf>,
    format_cache_path: Option<PathBuf>,
    keep_intermediates: bool,
    print_stdout: bool,
    chatter_level: Option<ChatterLevel>,
}

impl DifferentialSessionBuilder {
    /// Create a new builder with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Configure a persistent output directory for generated artifacts.
    pub fn output_dir<P: AsRef<Path>>(&mut self, path: P) -> &mut Self {
        self.output_dir = Some(path.as_ref().to_owned());
        self
    }

    /// Configure the path to format cache files (`.fmt`).
    pub fn format_cache_path<P: AsRef<Path>>(&mut self, path: P) -> &mut Self {
        self.format_cache_path = Some(path.as_ref().to_owned());
        self
    }

    /// Choose whether to keep intermediate compilation files on disk.
    pub fn keep_intermediates(&mut self, keep: bool) -> &mut Self {
        self.keep_intermediates = keep;
        self
    }

    /// Choose whether to echo engine stdout to console.
    pub fn print_stdout(&mut self, print: bool) -> &mut Self {
        self.print_stdout = print;
        self
    }

    /// Set chatter level for engine reporting.
    pub fn chatter_level(&mut self, level: ChatterLevel) -> &mut Self {
        self.chatter_level = Some(level);
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
        }
    }
}

/// Execution session that drives compilation and extracts both intermediate `.xdv`
/// and final `.pdf` artifacts for parity comparison.
#[derive(Debug)]
pub struct DifferentialSession {
    output_dir: Option<PathBuf>,
    format_cache_path: Option<PathBuf>,
    keep_intermediates: bool,
    print_stdout: bool,
    chatter_level: ChatterLevel,
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
        let config = PersistentConfig::open(false)
            .context("failed to open Tectonic persistent configuration")?;
        let bundle = config
            .default_bundle(false)
            .context("failed to load resource bundle")?;
        let format_cache = match &self.format_cache_path {
            Some(p) => p.clone(),
            None => config
                .format_cache_path()
                .context("failed to resolve format cache path")?,
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

        let mut xdv_session = xdv_builder
            .create(&mut status)
            .context("failed to create XDV processing session")?;

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
        let config2 = PersistentConfig::open(false)?;
        let bundle2 = config2.default_bundle(false)?;

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

        let mut pdf_session = pdf_builder
            .create(&mut status)
            .context("failed to create PDF processing session")?;

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
}

/// Compare intermediate XDV streams (Tier A Parity).
pub fn compare_tier_a_xdv(reference: &[u8], candidate: &[u8]) -> (ParityStatus, Option<String>) {
    match xdv_diff::XdvDiffEngine::compare_bytes(reference, candidate) {
        Ok(stats) => {
            if stats.bitwise_identical {
                (ParityStatus::ExactMatch, None)
            } else {
                (
                    ParityStatus::NormalizedMatch,
                    Some("Opcodes match identically, non-semantic byte variations".to_string()),
                )
            }
        }
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
}
