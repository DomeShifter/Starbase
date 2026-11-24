#!/usr/bin/env bash
# Runs GridCraft's own tests (our fork, fetched into upstream/gridcraft) for every crate: the
# engine, xlsx reader/writer, grid, and the Google functions (sheets-functions). CI runs this; so
# can you after changing the fork. Extra arguments go to cargo test (e.g. a test name filter).
set -euo pipefail
cd "$(dirname "$0")/../upstream/gridcraft"
packages=()
for c in crates/*/Cargo.toml; do
  packages+=(-p "$(sed -n 's/^name = "\(.*\)"/\1/p' "$c" | head -1)")
done
cargo test --no-fail-fast "${packages[@]}" "$@"
