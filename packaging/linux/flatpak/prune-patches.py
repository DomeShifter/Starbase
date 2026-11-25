#!/usr/bin/env python3
"""Removes [patch.*] entries for crates that are not in Cargo.lock, for the offline Flatpak build.

Our [patch.crates-io] blocks are copied from the engines and name every crate of the egui and
winit forks, also ones we never build (egui_demo_lib, egui_kittest, ...). Online, Cargo only warns
about an unused patch. In the Flatpak build each git source is replaced by a vendored copy that
holds just the crates in Cargo.lock (flatpak-cargo-generator vendors what the lockfile lists), so
Cargo stops with "patch location ... does not contain packages matching `egui_demo_lib`".
Dropping those entries changes nothing that gets built: they were unused, and --locked proves the
resolution is the same.

Usage (in the workspace folder, during the Flatpak build only):
    python3 prune-patches.py Cargo.toml Cargo.lock
"""
import re
import sys
import tomllib

manifest, lockfile = sys.argv[1], sys.argv[2]
with open(lockfile, "rb") as f:
    locked = {p["name"] for p in tomllib.load(f).get("package", [])}

out, in_patch, dropped = [], False, []
for line in open(manifest, encoding="utf-8").read().splitlines(keepends=True):
    header = re.match(r"\s*\[([^\]]+)\]", line)
    if header:
        in_patch = header.group(1).strip().startswith("patch.")
    elif in_patch:
        entry = re.match(r"\s*([A-Za-z0-9_-]+)\s*=", line)
        if entry and entry.group(1) not in locked:
            dropped.append(entry.group(1))
            continue
    out.append(line)

open(manifest, "w", encoding="utf-8").write("".join(out))
print(f"{manifest}: dropped unused patches: {', '.join(dropped) or 'none'}")
