#!/bin/sh
# Static checks for standalone Cargo workspaces not covered by root CI.
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
for engine in lua luau rhai koto; do
  manifest="$script_dir/$engine-runtime/Cargo.toml"
  cargo fmt --manifest-path "$manifest" -- --check
  cargo clippy --locked --offline --manifest-path "$manifest" --all-targets -- -D warnings
done
