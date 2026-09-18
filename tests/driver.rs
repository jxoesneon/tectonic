// Copyright 2018 the Tectonic Project
// Licensed under the MIT License.

//! This test rig is a total hack to quickly exercise `src/driver.rs`.
//!
//! I should make it real, but I just want Codecov to stop complaining about
//! my test coverage. To re-use the `tex-outputs` test artifacts, the
//! ProcessingSessionBuilder will need to learn how to tell `xdvipdfmx` to
//! enable the reproducibility options used in the `tex-outputs` test rig.

use tectonic::{
    config::PersistentConfig, driver::ProcessingSessionBuilder,
    status::termcolor::TermcolorStatusBackend, status::ChatterLevel, test_util::TestBundle,
};

mod util;

// Keep these alphabetized.

#[test]
fn the_letter_a() {
    util::set_test_root();

    let _config = PersistentConfig::default();

    // The "Normal" chatter escapes the test rig's attempts to eat stdout ...
    let mut status = TermcolorStatusBackend::new(ChatterLevel::Minimal);

    let bundle = TestBundle::default();

    let tempdir = tempfile::Builder::new()
        .prefix("tectonic_driver_test")
        .tempdir()
        .unwrap();

    let mut pbuilder = ProcessingSessionBuilder::default();
    pbuilder
        .primary_input_path(util::test_path(&["tex-outputs", "the_letter_a.tex"]))
        .tex_input_name("the_letter_a.tex")
        .format_name("plain")
        .format_cache_path(util::test_path(&[]))
        .output_dir(tempdir.path())
        .bundle(Box::new(bundle));

    let mut session = pbuilder
        .create(&mut status)
        .expect("couldn't create processing session");

    session
        .run(&mut status)
        .expect("failed to execute processing session");
}

#[test]
fn macro_expansion_hook_test() {
    util::set_test_root();

    let _config = PersistentConfig::default();
    let mut status = TermcolorStatusBackend::new(ChatterLevel::Minimal);
    let bundle = TestBundle::default();

    let tempdir = tempfile::Builder::new()
        .prefix("tectonic_macro_expand_test")
        .tempdir()
        .unwrap();

    let test_tex = tempdir.path().join("expand_test.tex");
    std::fs::write(&test_tex, b"\\def\\mymacro{Hello}\\mymacro\\bye\n").unwrap();

    let expansion_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let last_macro_name = std::sync::Arc::new(std::sync::Mutex::new(String::new()));

    let count_clone = expansion_count.clone();
    let name_clone = last_macro_name.clone();

    let mut pbuilder = ProcessingSessionBuilder::default();
    pbuilder
        .primary_input_path(&test_tex)
        .tex_input_name("expand_test.tex")
        .format_name("plain")
        .format_cache_path(util::test_path(&[]))
        .output_dir(tempdir.path())
        .bundle(Box::new(bundle))
        .expansion_hook(move |ctx| {
            count_clone.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if ctx.name.contains("mymacro") {
                *name_clone.lock().unwrap() = ctx.name.to_string();
            }
        });

    let mut session = pbuilder
        .create(&mut status)
        .expect("couldn't create processing session");

    session
        .run(&mut status)
        .expect("failed to execute processing session");

    assert!(expansion_count.load(std::sync::atomic::Ordering::SeqCst) > 0);
    assert_eq!(*last_macro_name.lock().unwrap(), "\\mymacro");
}
