#!/usr/bin/env python3
"""Fixes the Cargo source-replacement config in a flatpak-cargo-generator output file.

flatpak-cargo-generator keys each git dependency's replacement by repository URL only. StarDocs
uses one repository (storytold/craft-libs) at five different revisions (ours plus WordCraft's),
so all but one revision were left out and the offline build failed with "can't checkout from
'https://github.com/storytold/craft-libs': you are in the offline mode". This rewrites the
generated `cargo/config` with one entry per (repository, revision) found in Cargo.lock. The
vendored crates themselves are fine: no crate name occurs twice (the script checks).

Usage: vendor-config.py Cargo.lock cargo-sources.json   (rewrites the JSON in place)
"""
import json
import sys
import tomllib
from urllib.parse import parse_qs, urlparse

lockfile, sources_file = sys.argv[1], sys.argv[2]
with open(lockfile, "rb") as f:
    packages = tomllib.load(f).get("package", [])

seen_names = set()
entries = {}
for p in packages:
    src = p.get("source", "")
    if not src.startswith("git+"):
        continue
    if p["name"] in seen_names:
        sys.exit(f"{p['name']} comes from git twice; the vendored folder cannot hold both")
    seen_names.add(p["name"])
    url = urlparse(src[len("git+"):])
    repo = url._replace(query="", fragment="").geturl()
    query = parse_qs(url.query)
    kind, value = next(((k, query[k][0]) for k in ("rev", "tag", "branch") if k in query), (None, None))
    key = f"{repo}#{kind}={value}" if kind else repo
    entries[key] = {"git": repo, **({kind: value} if kind else {})}


def q(s):
    return json.dumps(s)  # a TOML basic string is a JSON string for these values


lines = ['[source.vendored-sources]', 'directory = "cargo/vendor"', "",
         '[source.crates-io]', 'replace-with = "vendored-sources"', ""]
for key, e in sorted(entries.items()):
    lines.append(f"[source.{q(key)}]")
    lines += [f"{k} = {q(v)}" for k, v in e.items()]
    lines += ['replace-with = "vendored-sources"', ""]
config = "\n".join(lines)

sources = json.load(open(sources_file))
inline = [s for s in sources if s.get("type") == "inline" and s.get("dest-filename") == "config"]
if len(inline) != 1:
    sys.exit(f"{sources_file}: expected one generated cargo config, found {len(inline)}")
inline[0]["contents"] = config
inline[0]["dest-filename"] = "config.toml"  # cargo warns that plain "config" is deprecated
json.dump(sources, open(sources_file, "w"), indent=4)
print(f"{sources_file}: {len(entries)} git sources replaced by vendored copies")
