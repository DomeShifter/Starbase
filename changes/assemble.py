#!/usr/bin/env python3
"""Changelog fragments -> CHANGELOG.md (see CONTRIBUTING.md, "Changelog").

Each pull request adds one file here, changes/<branch-or-topic>.md, with entries under
`### Added`, `### Changed`, `### Fixed` (or `### Removed`, `### Security`) headings, instead of
editing CHANGELOG.md: two pull requests then never conflict over the same lines.

  changes/assemble.py --version 0.2.0 [--date 2026-12-01] [--write] [--keep]
      Builds the section `## [0.2.0] - <date>` from the [Unreleased] entries (if any) and every
      fragment, grouped by heading. If CHANGELOG.md already has a 0.2.0 section (written by hand),
      the fragments are merged into it. Prints the section. --write saves CHANGELOG.md and, unless
      --keep, deletes the fragments (do that in the release-preparation pull request).
  changes/assemble.py --unreleased
      Prints what the next release would say (the [Unreleased] entries plus the fragments).

release.yml runs it (--write --keep, on the runner's copy) so the release notes always include the
fragments, even when nobody ran it before tagging.
"""
import argparse, datetime, pathlib, re, sys

ORDER = ["Added", "Changed", "Deprecated", "Removed", "Fixed", "Security"]
HERE = pathlib.Path(__file__).resolve().parent
CHANGELOG = HERE.parent / "CHANGELOG.md"


def fragments():
    return sorted(p for p in HERE.glob("*.md") if p.name != "README.md")


def parse(text, where):
    """Entries of a section body, by heading: {heading: [entry, ...]}, plus the text before any heading."""
    groups, intro, cur = {}, [], None
    for line in text.splitlines():
        m = re.match(r"^###\s+(\w+)\s*$", line)
        if m:
            cur = m.group(1).capitalize()
            if cur not in ORDER:
                sys.exit(f"{where}: unknown heading '### {m.group(1)}' (use one of {', '.join(ORDER)})")
            groups.setdefault(cur, [])
        elif cur is None:
            intro.append(line)
        elif line.startswith("- "):
            groups[cur].append(line)
        elif line.strip() and groups[cur]:
            groups[cur][-1] += "\n" + line  # continuation of the entry above
        elif line.strip():
            sys.exit(f"{where}: text under '### {cur}' must be '- ' entries")
    return groups, "\n".join(intro).strip()


def render(groups):
    out = []
    for h in ORDER:
        if groups.get(h):
            out.append(f"### {h}\n" + "\n".join(groups[h]))
    return "\n\n".join(out)


def sections(text):
    """[(title line, body)] for each '## [' section; the head before the first one as title None."""
    parts = re.split(r"(?m)^(## \[.*)$", text)
    res = [(None, parts[0])]
    for i in range(1, len(parts), 2):
        res.append((parts[i], parts[i + 1]))
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version")
    ap.add_argument("--date", default=datetime.date.today().isoformat())
    ap.add_argument("--unreleased", action="store_true")
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--keep", action="store_true")
    a = ap.parse_args()
    if not a.version and not a.unreleased:
        ap.error("--version or --unreleased")

    merged = {}
    for p in fragments():
        g, intro = parse(p.read_text(), p.name)
        if intro:
            sys.exit(f"{p.name}: start with a '### Added', '### Changed' or '### Fixed' heading")
        if not any(g.values()):
            sys.exit(f"{p.name}: no entries")
        for h, es in g.items():
            merged.setdefault(h, []).extend(es)

    secs = sections(CHANGELOG.read_text())
    unrel = next((i for i, (t, _) in enumerate(secs) if t and t.startswith("## [Unreleased]")), None)
    if unrel is None:
        sys.exit("CHANGELOG.md has no '## [Unreleased]' section")
    ug, uintro = parse(secs[unrel][1], "CHANGELOG.md [Unreleased]")
    for h, es in ug.items():
        merged[h] = es + merged.get(h, [])

    if a.unreleased:
        print(render(merged))
        return

    ver = next((i for i, (t, _) in enumerate(secs) if t and re.match(rf"^## \[{re.escape(a.version)}\]", t)), None)
    if ver is None:
        intro = uintro
        title = f"## [{a.version}] - {a.date}"
    else:
        vg, intro = parse(secs[ver][1], f"CHANGELOG.md [{a.version}]")
        for h, es in merged.items():
            vg.setdefault(h, []).extend(es)
        merged, title = vg, secs[ver][0]
    body = "\n\n".join(x for x in [intro, render(merged)] if x)
    if not body:
        sys.exit(f"nothing to release: no fragments in changes/ and no section for {a.version}")
    print(body)
    if not a.write:
        return
    new = [secs[0][1].rstrip("\n") + "\n\n", "## [Unreleased]\n\n", f"{title}\n\n{body}\n\n"]
    for i, (t, b) in enumerate(secs[1:], 1):
        if i in (unrel, ver):
            continue
        new.append(f"{t}{b.rstrip()}\n\n" if b.strip() else f"{t}\n\n")
    CHANGELOG.write_text("".join(new).rstrip("\n") + "\n")
    if not a.keep:
        for p in fragments():
            p.unlink()


if __name__ == "__main__":
    main()
