"""Download the third-party test decks listed in fetched.json, at pinned commits, and check each
file's sha256. They are fetched at test time and never stored in this repository.

usage: python fetch.py OUT_DIR            download + verify (exit 1 on any mismatch)
       python fetch.py OUT_DIR --pin      download and write the sha256 values into fetched.json
"""

import hashlib
import json
import pathlib
import sys
import time
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
MANIFEST = HERE / "fetched.json"


def url(repo, deck):
    return f"https://raw.githubusercontent.com/{repo['github']}/{repo['commit']}/{repo['dir']}/{deck['file']}"


def download(u):
    for attempt in range(4):
        try:
            with urllib.request.urlopen(u, timeout=60) as r:
                return r.read()
        except Exception as e:  # network hiccup: retry a few times
            if attempt == 3:
                raise RuntimeError(f"{u}: {e}") from e
            time.sleep(2 + attempt * 3)


def main():
    out = pathlib.Path(sys.argv[1])
    pin = "--pin" in sys.argv
    out.mkdir(parents=True, exist_ok=True)
    m = json.loads(MANIFEST.read_text())
    bad = 0
    for d in m["decks"]:
        dest = out / f"{d['id']}.pptx"
        data = download(url(m["repos"][d["repo"]], d))
        h = hashlib.sha256(data).hexdigest()
        if pin:
            d["sha256"] = h
        elif h != d["sha256"]:
            print(f"SHA256 MISMATCH {d['id']}: got {h}, want {d['sha256']}")
            bad += 1
            continue
        dest.write_bytes(data)
    if pin:
        MANIFEST.write_text(json.dumps(m, indent=1, ensure_ascii=False) + "\n")
    print(f"fetched {len(m['decks']) - bad} of {len(m['decks'])} decks into {out}")
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
