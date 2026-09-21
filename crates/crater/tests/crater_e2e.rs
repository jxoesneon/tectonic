// Copyright 2026 the Tectonic Project
// Licensed under the MIT License.

//! End-to-End Integration Test Suite for the Tectonic Crater Differential Testing Harness.
//!
//! Validates the two-tier verification workflow mandated by the Pure-Rust Migration Roadmap:
//! - **Gate 1 (Tier A)**: Bitwise and opcode-level intermediate `.xdv` parity.
//! - **Gate 2 (Tier B)**: Normalized AST content stream and visual raster `.pdf` parity.

use std::path::PathBuf;
use std::time::Duration;
use tectonic_crater::pdf_diff::compare_pdfs;
use tectonic_crater::xdv_diff::{XdvBuilder, XdvDiffEngine};
use tectonic_crater::{
    compare_runs, compare_tier_a_xdv, compare_tier_b_pdf, Corpus, CorpusCategory,
    DifferentialReport, ParityStatus, RunResult, RunStatus, TestCase,
};

#[test]
fn test_e2e_tier_a_xdv_parity_and_coordinate_divergence() {
    // 1. Construct baseline XDV document with font definition, pages, rules, and glyphs
    let base_xdv = XdvBuilder::new()
        .native_font(0, "cmr10", 655360)
        .begin_page(1)
        .push_stack()
        .move_right(655360) // 10 pt
        .move_down(1310720) // 20 pt
        .glyph(65, 0, 0, 655360) // 'A'
        .rule(65536, 655360) // 1pt rule
        .special(b"color push rgb 0 0 1")
        .pop_stack()
        .end_page()
        .build();

    // 2. Candidate 1: Identical compilation output -> Gate 1 MUST PASS
    let (cand1_status, cand1_diff) = compare_tier_a_xdv(&base_xdv, &base_xdv);
    assert_eq!(cand1_status, ParityStatus::ExactMatch);
    assert!(cand1_diff.is_none());

    // Verify bitwise comparison via diff engine directly
    let stats =
        XdvDiffEngine::compare_bytes(&base_xdv, &base_xdv).expect("should match identically");
    assert!(stats.bitwise_identical);
    assert_eq!(stats.pages_checked, 1);
    assert!(stats.total_opcodes > 0);

    // 3. Candidate 2: Injected 1pt coordinate drift (dx = 65,536 sp) -> Gate 1 MUST FAIL
    let perturbed_xdv = XdvBuilder::new()
        .native_font(0, "cmr10", 655360)
        .begin_page(1)
        .push_stack()
        .move_right(655360 + 65536) // Shifted by 1 pt = 65,536 sp
        .move_down(1310720)
        .glyph(65, 0, 0, 655360)
        .rule(65536, 655360)
        .special(b"color push rgb 0 0 1")
        .pop_stack()
        .end_page()
        .build();

    let (cand2_status, cand2_diff) = compare_tier_a_xdv(&base_xdv, &perturbed_xdv);
    assert!(matches!(cand2_status, ParityStatus::Diverged(_)));
    let diff_msg = cand2_diff.expect("divergence details must be reported");
    assert!(
        diff_msg.contains("Divergence")
            || diff_msg.contains("Movement")
            || diff_msg.contains("Bitwise"),
        "Divergence report should detail coordinate or bitwise deviation: {diff_msg}"
    );

    // Verify exact opcode trace diff diagnostics
    let trace_err = XdvDiffEngine::compare_bytes(&base_xdv, &perturbed_xdv).unwrap_err();
    assert_eq!(trace_err.page_number, 1);
}

#[test]
fn test_e2e_tier_b_pdf_normalization_and_ast_divergence() {
    // 1. Reference PDF with creation timestamps and font subset tag ABCDEF
    let pdf_ref = b"%PDF-1.4\n\
1 0 obj\n\
<< /Type /Catalog /Pages 2 0 R >>\n\
endobj\n\
2 0 obj\n\
<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n\
endobj\n\
3 0 obj\n\
<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\n\
endobj\n\
4 0 obj\n\
<< /Length 75 >>\n\
stream\n\
BT\n\
/ABCDEF+CMR10 12 Tf\n\
100.000000 200.000000 Td\n\
(Tectonic Pure Rust Parity) Tj\n\
ET\n\
endstream\n\
endobj\n\
trailer\n\
<< /Root 1 0 R /Info << /CreationDate (D:20260918080000Z) /Producer (Tectonic 0.17.3) >> >>\n\
%%EOF";

    // 2. Candidate PDF with differing timestamp, producer string, font subset tag XYZWVU,
    // and float precision variations that normalize identically
    let pdf_cand_normalized = b"%PDF-1.4\n\
1 0 obj\n\
<< /Type /Catalog /Pages 2 0 R >>\n\
endobj\n\
2 0 obj\n\
<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n\
endobj\n\
3 0 obj\n\
<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\n\
endobj\n\
4 0 obj\n\
<< /Length 75 >>\n\
stream\n\
BT\n\
/XYZWVU+CMR10 12 Tf\n\
100.000049 199.999951 Td\n\
(Tectonic Pure Rust Parity) Tj\n\
ET\n\
endstream\n\
endobj\n\
trailer\n\
<< /Root 1 0 R /Info << /CreationDate (D:20261025120000Z) /Producer (PureRustTectonic 1.0.0) >> >>\n\
%%EOF";

    // Compare PDFs directly -> Gate 2 MUST PASS under normalization
    let res =
        compare_pdfs(pdf_ref, pdf_cand_normalized, 300).expect("PDF comparison should succeed");
    assert!(
        res.passes_gate2(0.001),
        "Gate 2 must pass for normalized equivalent PDF outputs"
    );
    assert!(
        res.stream_matches,
        "Normalized content streams must match identically"
    );
    assert_eq!(res.divergent_pages.len(), 0);

    // Verify via crater public API
    let (cand_status, cand_diff) = compare_tier_b_pdf(pdf_ref, pdf_cand_normalized);
    assert_eq!(cand_status, ParityStatus::NormalizedMatch);
    assert!(cand_diff.is_none());

    // 3. Candidate with divergent content stream operator -> Gate 2 MUST FAIL
    let pdf_cand_diverged = b"%PDF-1.4\n\
1 0 obj\n\
<< /Type /Catalog /Pages 2 0 R >>\n\
endobj\n\
2 0 obj\n\
<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n\
endobj\n\
3 0 obj\n\
<< /Type /Page /Parent 2 0 R /Contents 4 0 R >>\n\
endobj\n\
4 0 obj\n\
<< /Length 85 >>\n\
stream\n\
BT\n\
/ABCDEF+CMR10 12 Tf\n\
150.0000 350.0000 Td\n\
(Divergent Text Payload) Tj\n\
ET\n\
endstream\n\
endobj\n\
trailer\n\
<< /Root 1 0 R >>\n\
%%EOF";

    let (div_status, div_diff) = compare_tier_b_pdf(pdf_ref, pdf_cand_diverged);
    assert!(matches!(div_status, ParityStatus::Diverged(_)));
    let diff_msg = div_diff.expect("divergence details must be provided");
    assert!(diff_msg.contains("divergent_pages=[0]"));
}

#[test]
fn test_e2e_differential_report_lifecycle_and_json_schema() {
    let xdv_data = XdvBuilder::new().begin_page(1).end_page().build();
    let pdf_data =
        b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\ntrailer\n<< >>\n%%EOF".to_vec();

    let ref_run = RunResult {
        status: RunStatus::Success,
        duration: Duration::from_millis(125),
        stdout: "Reference engine pass 1 & 2 completed".into(),
        stderr: String::new(),
        xdv_path: Some(PathBuf::from("target/ref.xdv")),
        pdf_path: Some(PathBuf::from("target/ref.pdf")),
        xdv_bytes: Some(xdv_data.clone()),
        pdf_bytes: Some(pdf_data.clone()),
        page_count: 1,
    };

    let cand_run = RunResult {
        status: RunStatus::Success,
        duration: Duration::from_millis(98),
        stdout: "Candidate engine pass 1 & 2 completed".into(),
        stderr: String::new(),
        xdv_path: Some(PathBuf::from("target/cand.xdv")),
        pdf_path: Some(PathBuf::from("target/cand.pdf")),
        xdv_bytes: Some(xdv_data),
        pdf_bytes: Some(pdf_data),
        page_count: 1,
    };

    let report = compare_runs("e2e_parity_doc", &ref_run, &cand_run);

    assert_eq!(report.case_name, "e2e_parity_doc");
    assert!(report.passed_all());
    assert!(!report.has_divergences());
    assert_eq!(report.tier_a_status, ParityStatus::ExactMatch);
    assert_eq!(report.tier_b_status, ParityStatus::NormalizedMatch);
    assert!(report.summary().contains("PASSED"));

    // Verify JSON serialization and round-trip parsing
    let serialized_json =
        serde_json::to_string_pretty(&report).expect("report must serialize to valid JSON");
    assert!(serialized_json.contains("\"case_name\": \"e2e_parity_doc\""));
    assert!(serialized_json.contains("\"tier_a_status\": \"ExactMatch\""));

    let deserialized_report: DifferentialReport =
        serde_json::from_str(&serialized_json).expect("report must deserialize from JSON");
    assert_eq!(deserialized_report.case_name, report.case_name);
    assert_eq!(deserialized_report.tier_a_status, report.tier_a_status);
    assert_eq!(deserialized_report.tier_b_status, report.tier_b_status);
}

#[test]
fn test_e2e_corpus_management_and_filtering() {
    let mut corpus = Corpus::new();
    corpus.add_case(
        TestCase::new(
            "plain_math",
            CorpusCategory::PlainTex,
            "$$E=mc^2$$\\bye",
            "plain",
        )
        .with_tag("math")
        .with_tag("smoke"),
    );
    corpus.add_case(
        TestCase::new(
            "latex_article",
            CorpusCategory::Latex2e,
            "\\documentclass{article}\\begin{document}Hi\\end{document}",
            "latex",
        )
        .with_tag("latex")
        .with_tag("smoke"),
    );

    assert_eq!(corpus.len(), 2);
    assert!(!corpus.is_empty());

    let smoke_cases = corpus.filter_by_tag("smoke");
    assert_eq!(smoke_cases.len(), 2);

    let plain_cases = corpus.filter_by_category(&CorpusCategory::PlainTex);
    assert_eq!(plain_cases.len(), 1);
    assert_eq!(plain_cases[0].name, "plain_math");

    let latex_cases = corpus.filter_by_category(&CorpusCategory::Latex2e);
    assert_eq!(latex_cases.len(), 1);
    assert_eq!(latex_cases[0].name, "latex_article");
}
