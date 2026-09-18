// Copyright 2026 the Tectonic Project & FerroTeX Authors
// Licensed under the MIT License.

//! Build script for tectonic_image_probe exporting C include path to dependent crates.

use std::{env, path::PathBuf};

fn main() {
    let manifest_dir: PathBuf = env::var("CARGO_MANIFEST_DIR").unwrap().into();
    let mut include_dir = manifest_dir;
    include_dir.push("include");
    println!("cargo:include-path={}", include_dir.display());
}
