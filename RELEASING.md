# Releasing

Releases publish versioned source archives and release notes on GitHub. Registry
publication is not part of this process and no registry token is required.

## Prepare the version

1. Start from `main` and choose the next Semantic Versioning release number.
2. Update changed workspace package versions and matching internal dependency
   constraints. Update the README installation tag, Forma package metadata and
   standalone WASM lockfile when applicable.
3. Move the entries under `Unreleased` in `CHANGELOG.md` into a dated version
   section, retain an empty `Unreleased` section and update comparison links.
4. Never reuse or move an existing published version tag.

## Validate and merge

Run the local gates:

```console
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --no-deps --all-features
```

Build and test affected browser/WASM packages as well. Submit the release changes
through a pull request, confirm CI is green, and merge into protected `main`.
Keep unrelated active work and unfinished branches separate from the release.

## Create the GitHub release

1. Run the **Release** workflow against the validated `main` commit with the
   exact version `X.Y.Z`.
2. The workflow validates the package version and tag state, then creates
   `vX.Y.Z` and the GitHub release.
3. Verify the release is public, its tag points to the intended commit and its
   notes describe the compatibility changes and distribution method.
4. Verify the long-lived `release` branch points to that tag's commit. If tag
   events are suppressed by the workflow token, advance it explicitly with a
   fast-forward push after checking its ancestry. Do not overwrite unique work.

The workflow may be rerun only when an existing tag points to the same commit.
For corrections, release a new version instead of moving a published tag.

## Consume the source release

```toml
[dependencies]
themoretheless-tokenizer = { git = "https://github.com/themoretheless/tokenizer", tag = "v0.8.0" }
```

Install the Rush CLI from the same source tag:

```console
cargo install --git https://github.com/themoretheless/tokenizer --tag v0.8.0 themoretheless-tokenizer-rush --bin rush
```

The `release` branch is available for consumers that intentionally track the
latest release; use an immutable tag for reproducible builds.
