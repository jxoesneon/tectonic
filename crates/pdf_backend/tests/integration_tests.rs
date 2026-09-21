// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Integration tests running real TeX/XeTeX XDV files through tectonic_pdf_backend.

use std::fs;
use std::path::PathBuf;
use tectonic_pdf_backend::PdfBackendEngine;

fn get_workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("crates")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn test_process_a4paper_xdv() {
    let root = get_workspace_root();
    let xdv_path = root.join("tests/tex-outputs/a4paper.xdv");
    assert!(xdv_path.exists(), "xdv file must exist: {:?}", xdv_path);

    let xdv_bytes = fs::read(&xdv_path).expect("read xdv");

    let mut engine = PdfBackendEngine::new();
    engine.paper_spec("a4");

    let pdf_bytes = engine
        .process_bytes(&xdv_bytes)
        .expect("process a4paper.xdv");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    let pdf_str = String::from_utf8_lossy(&pdf_bytes);
    assert!(pdf_str.contains("/Type /Catalog"));
    assert!(pdf_str.contains("/Type /Pages"));
    assert!(pdf_str.contains("/Type /Page"));
}

#[test]
fn test_process_md5_of_hello_xdv() {
    let root = get_workspace_root();
    let xdv_path = root.join("tests/tex-outputs/md5_of_hello.xdv");
    assert!(xdv_path.exists(), "xdv file must exist: {:?}", xdv_path);

    let xdv_bytes = fs::read(&xdv_path).expect("read xdv");

    let engine = PdfBackendEngine::new();
    let pdf_bytes = engine
        .process_bytes(&xdv_bytes)
        .expect("process md5_of_hello.xdv");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
    let pdf_str = String::from_utf8_lossy(&pdf_bytes);
    assert!(pdf_str.contains("/Type /Page"));
}

#[test]
fn test_process_negative_roman_numeral_xdv() {
    let root = get_workspace_root();
    let xdv_path = root.join("tests/tex-outputs/negative_roman_numeral.xdv");
    assert!(xdv_path.exists(), "xdv file must exist: {:?}", xdv_path);

    let xdv_bytes = fs::read(&xdv_path).expect("read xdv");

    let engine = PdfBackendEngine::new();
    let pdf_bytes = engine
        .process_bytes(&xdv_bytes)
        .expect("process negative_roman_numeral.xdv");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
}

#[test]
fn test_process_issue393_ungetc_xdv() {
    let root = get_workspace_root();
    let xdv_path = root.join("tests/tex-outputs/issue393_ungetc.xdv");
    assert!(xdv_path.exists(), "xdv file must exist: {:?}", xdv_path);

    let xdv_bytes = fs::read(&xdv_path).expect("read xdv");

    let engine = PdfBackendEngine::new();
    let pdf_bytes = engine
        .process_bytes(&xdv_bytes)
        .expect("process issue393_ungetc.xdv");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
}

#[test]
fn test_process_via_launcher_vfs() {
    use std::collections::HashSet;
    use tectonic_bridge_core::{CoreBridgeLauncher, MinimalDriver};
    use tectonic_io_base::filesystem::FilesystemIo;
    use tectonic_status_base::NoopStatusBackend;

    let root = get_workspace_root();
    let xdv_path = root.join("tests/tex-outputs/a4paper.xdv");
    let xdv_bytes = fs::read(&xdv_path).expect("read xdv");

    let temp_dir = tempfile::tempdir().expect("tempdir");
    let in_name = "test_doc.xdv";
    let out_name = "test_doc.pdf";

    fs::write(temp_dir.path().join(in_name), &xdv_bytes).expect("write xdv to temp");

    let fs_io = FilesystemIo::new(temp_dir.path(), true, true, HashSet::new());
    let mut driver = MinimalDriver::new(fs_io);
    let mut status = NoopStatusBackend::default();
    let mut launcher = CoreBridgeLauncher::new(&mut driver, &mut status);

    let mut engine = PdfBackendEngine::new();
    engine.paper_spec("a4");
    engine
        .process(&mut launcher, in_name, out_name)
        .expect("engine process via launcher");

    let out_path = temp_dir.path().join(out_name);
    assert!(out_path.exists(), "generated PDF should exist");
    let pdf_bytes = fs::read(out_path).expect("read generated pdf");
    assert!(pdf_bytes.starts_with(b"%PDF-"));
}
