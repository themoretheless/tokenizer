# Releasing

Releases use two explicit GitHub Actions workflows. The publish workflow first
uploads the workspace crates to crates.io; the release workflow creates the
Git tag and GitHub release after version and tag checks. Do not create the tag
before publishing.

## Prerequisites

- Work from the `main` branch with all intended changes committed and pushed.
- Confirm the CI and conformance checks are green.
- Configure the `crates-io` GitHub environment, preferably with required
  reviewers, and provide `CARGO_REGISTRY_TOKEN` as an environment or repository
  secret.
- Ensure the intended version is unused on crates.io and has no Git tag.

## Prepare the version

1. Choose the version according to Semantic Versioning.
2. Set `package.version` in `Cargo.toml`, update the README installation
   constraint and any other displayed version references; do not lower
   `rust-version` without an explicit compatibility decision.
3. Move the entries under `Unreleased` in `CHANGELOG.md` into a heading such as
   `## [X.Y.Z] - YYYY-MM-DD`, add a fresh empty `Unreleased` section, and update
   the comparison links. Existing versions and tags, including `v0.2.0`, must
   never be reused or moved.
4. Run the local gates:

   ```console
   cargo fmt --check
   cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
   cargo test --locked --workspace --all-targets --all-features
   RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
   cargo +beta publish --locked --workspace --exclude tokenizer-wasm --dry-run
   ```

5. Commit the version, changelog, and any regenerated `Cargo.lock`, push to
   `main`, and wait for CI to pass on that exact commit.

## Publish to crates.io

The workflow selects every publishable workspace crate with Cargo's workspace
publish support, excluding `tokenizer-wasm` (`publish = false`). Cargo orders
packages by their dependencies: core, language/format plugins, then the facade.
New plugin crates are included automatically; optional dependencies are included.
Use the beta toolchain configured by CI for workspace publishing.

```console
cargo +beta publish --locked --workspace --exclude tokenizer-wasm --dry-run
```

This dry run packages and verifies the workspace together without uploading it,
including dependencies that have not been published yet. Before a new release,
update every changed crate's version and the matching path dependency versions;
never reuse a published version for changed contents. Existing registry versions
and publication permissions remain Cargo's responsibility to validate.

1. In GitHub Actions, open **Publish to crates.io** and run it against `main`.
2. Enter the exact Cargo version, leave **publish** disabled, and run the
   workflow. This validates the version and dry-runs every package without
   exposing the registry token.
3. Run the same workflow again on the same `main` commit with **publish**
   enabled and confirmation equal to the Cargo version (for example `0.4.0`).
4. Approve the protected `crates-io` environment when prompted, then wait until
   all selected packages are visible on crates.io. Never publish from an uncommitted
   local checkout.

## Create the GitHub release

1. In GitHub Actions, run **Release** against the same `main` commit with
   version `X.Y.Z`.
2. After confirming publication, run the workflow to validate the version and
   existing tag state, then create the `vX.Y.Z` tag and GitHub release.
3. Confirm the tag, generated release notes, crates.io page, and docs.rs build.

The release workflow is safe to rerun when its existing tag points to the same
commit. Never move or overwrite a published version tag. If the published crate
is defective, yank it when appropriate, fix the problem, and release a new
patch version; crates.io versions cannot be replaced.
