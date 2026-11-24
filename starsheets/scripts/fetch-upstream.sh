#!/usr/bin/env bash
# Fetches GridCraft at the commit in upstream.pin (our fork's "starsheets" branch: upstream's
# release plus our changes as commits) into upstream/gridcraft. Safe to re-run: it resets to a
# clean checkout of that commit every time. Edit GridCraft in the fork, never in upstream/.
set -euo pipefail
cd "$(dirname "$0")/.."
commit=$(sed -n 's/^commit = "\(.*\)"/\1/p' upstream.pin)
repo=$(sed -n 's/^repo = "\(.*\)"/\1/p' upstream.pin)
dir=upstream/gridcraft
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
ours=$(sed -n '/^# Keep the complete egui family/,$p' Cargo.toml)
theirs=$(sed -n '/^# Keep the complete egui family/,$p' "$dir/Cargo.toml")
[ "$ours" = "$theirs" ] || { echo "Cargo.toml [patch.crates-io] differs from upstream's: copy it over" >&2; exit 1; }
echo "upstream ready: GridCraft ${commit:0:12} (fork branch starsheets)"
