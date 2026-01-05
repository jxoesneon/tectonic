---
description: how to perform a stellar tectonic release
---

# Stellar Tectonic Release Workflow

Follow these steps to ensure a high-quality, automated release of the Tectonic fork (`jxoesneon/tectonic`).

## 1. Preparation & Hardening

Before releasing, ensure the codebase is resilient and clean:

- [ ] **Audit for Panics**: Search for `unwrap()` or `panic!()` in critical paths (especially `xetex_format` and `bundles`).
- [ ] **Propagate Errors**: Replace panics with proper `Result` propagation using `anyhow` or `thiserror`.
- [ ] **Security Audit**: Run `cargo audit`. Update or suppress advisories in `.cargo/audit.toml`.
- [ ] **Dependency Audit**: Ensure no required imports were accidentally removed during refactoring. Run `cargo build` on all workspace members.

## 2. Verification (Pre-release)

Run the full verification suite locally. This is a **MANDATORY** preemptive step to catch lints early.

// turbo

```bash
cargo test --workspace && cargo clippy --workspace --all-targets --all-features && cargo fmt --all --check
```

> [!TIP]
> Always use `--all-targets` to ensure Clippy checks tests and binaries, where many "unused result" or "deprecated" warnings hide.

## 3. Versioning

Tectonic uses **Cranko** for version management.

- [ ] Update the workspace version in the root `Cargo.toml`.
- [ ] Ensure sub-crate `version` fields remain `0.0.0-dev.0`. Cranko will substitute these during the CI/CD pipeline.

## 4. CI/CD Orchestration

The release is automated via [.github/workflows/main_workflow.yml](file:///Users/meilynlopezcubero/tectonic/.github/workflows/main_workflow.yml).

- [ ] **Workflow Health**: Ensure the main workflow and its sub-workflows (`prep.yml`, `build_and_test.yml`, `deploy.yml`) are in sync with the current repository structure.
- [ ] **Renaming Verification**: If new crates were added, ensure they are included in `dist/rename_for_fork.py` to maintain the `jxoesneon-` prefix.

## 5. Execution (Tagging)

Trigger the release by pushing a version tag. Perform a final "clean check" before pushing.

// turbo

```bash
# Ensure no transient noise is staged
git clean -fd && git status

# Commit and Tag
git commit -m "chore: release vX.Y.Z"
git tag vX.Y.Z
git push origin master --tags
```

## 6. Verification & Cleanup (Post-release)

- [ ] **Monitor CI**: Watch the "Main Workflow" execution in GitHub Actions. Any failure here blocks the release.
- [ ] **Crates.io Verification**: Confirm all 26+ crates are live with the `jxoesneon-` prefix.
- [ ] **Doc Update**: Update the [walkthrough.md](file:///Users/meilynlopezcubero/.gemini/antigravity/brain/a1fae71c-9812-4e7d-8f31-4075fc031df1/walkthrough.md) with the new release highlights and commit hash.
