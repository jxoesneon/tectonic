---
description: how to perform a stellar tectonic release
---

# Stellar Tectonic Release Workflow

Follow these steps to ensure a high-quality, automated release of the Tectonic fork (`jxoesneon/tectonic`).

## 1. Preparation & Hardening

Before releasing, ensure the codebase is resilient:

- [ ] **Audit for Panics**: Search for `unwrap()` or `panic!()` in critical paths (especially `xetex_format` and `bundles`).
- [ ] **Propagate Errors**: Replace panics with proper `Result` propagation using `anyhow` or `thiserror`.
- [ ] **Security Audit**: Run `cargo audit`. Suppress low-risk/unfixable advisories in `.cargo/audit.toml`.

## 2. Verification

Run the full verification suite locally:
// turbo

```bash
cargo test --workspace && cargo clippy --workspace --all-targets && cargo fmt --all --check
```

## 3. Versioning

Tectonic uses **Cranko** for version management.

- [ ] Update the workspace version in `Cargo.toml`.
- [ ] Ensure sub-crates are set to `0.0.0-dev.0` (Cranko will handle them).

## 4. CI/CD Orchestration

The release is automated via [.github/workflows/main_workflow.yml](file:///Users/meilynlopezcubero/tectonic/.github/workflows/main_workflow.yml).

- [ ] Ensure `main_workflow.yml` triggers on tags (`v*`).
- [ ] Verify `dist/rename_for_fork.py` is up to date for the `jxoesneon-` prefixing.

## 5. Execution

Trigger the release by pushing a version tag:
// turbo

```bash
git add .
git commit -m "chore: release vX.Y.Z"
git tag vX.Y.Z
git push origin master --tags
```

## 6. Verification & Cleanup

- [ ] **Monitor CI**: Check the "Main Workflow" run in GitHub Actions.
- [ ] **Registry Check**: Verify the new versions appear on crates.io with the `jxoesneon-` prefix.
- [ ] **Cleanup**: Ensure no transient files (`__pycache__`, `test_output.txt`) are tracked.
      // turbo

```bash
git status
```
