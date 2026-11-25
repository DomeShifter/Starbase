#!/usr/bin/env python3
"""Downloads the British English word list Starbase Sheets spell-checks cell text with: SCOWL's word
lists (sizes 10 to 70, the common English words plus the British spellings, with their plurals and
other forms) from the wordlist-english package, into app/dictionary/en-GB.txt (one word per line),
with SCOWL's copyright and permission notice beside it as LICENSE.txt. Run before `trunk build`."""
import json, os, urllib.request

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "app")
OUT = os.path.join(ROOT, "dictionary")
BASE = "https://cdn.jsdelivr.net/npm/wordlist-english@1.2.1/"
SIZES = (10, 20, 35, 40, 50, 55, 60, 70)


def get(name):
    with urllib.request.urlopen(BASE + name, timeout=60) as r:
        return r.read()


def main():
    os.makedirs(OUT, exist_ok=True)
    words = set()
    for kind in ("english", "british"):
        for n in SIZES:
            words.update(json.loads(get(f"{kind}-words-{n}.json")))
    with open(os.path.join(OUT, "en-GB.txt"), "w", encoding="utf-8") as f:
        f.write("\n".join(sorted(words)) + "\n")
    with open(os.path.join(OUT, "LICENSE.txt"), "wb") as f:
        f.write(get("Copyright"))
    print(f"{len(words)} words")


if __name__ == "__main__":
    main()
