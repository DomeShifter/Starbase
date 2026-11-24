#!/usr/bin/env bash
# Fetches WordCraft at the commit in upstream.pin (our fork's "stardocs" branch) into
# upstream/wordcraft, then regenerates Cargo.toml from upstream's (scripts/gen-cargo.py), so
# our dependency versions and [patch.crates-io] block always match the engine we build.
# Safe to re-run. Edit WordCraft in the fork, never in upstream/.
set -euo pipefail
cd "$(dirname "$0")/.."
commit=$(sed -n 's/^commit = "\(.*\)"/\1/p' upstream.pin)
repo=$(sed -n 's/^repo = "\(.*\)"/\1/p' upstream.pin)
dir=upstream/wordcraft
if [ ! -d "$dir/.git" ]; then
  mkdir -p "$dir"
  git -C "$dir" init --quiet
  git -C "$dir" remote add origin "$repo"
fi
git -C "$dir" remote set-url origin "$repo"
git -C "$dir" fetch --quiet --depth 1 origin "$commit"
git -C "$dir" checkout --quiet --force FETCH_HEAD
git -C "$dir" clean --quiet -fdx
actual=$(git -C "$dir" rev-parse HEAD)
[ "$actual" = "$commit" ] || { echo "upstream is $actual, expected $commit" >&2; exit 1; }
python3 scripts/gen-cargo.py
[ -f Cargo.lock ] || cp "$dir/Cargo.lock" Cargo.lock
echo "upstream ready: WordCraft ${commit:0:12} (fork branch stardocs)"
