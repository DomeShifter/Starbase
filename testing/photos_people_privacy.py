#!/usr/bin/env python3
# Last updated: 2026-11-26T19:10:00Z
"""Privacy proof for Photos > People, through the HTTP API (stdlib only).
  photos_people_privacy.py URL --pics DIR     (server built with --features ml, face models downloaded; ada = owner)
Case A: makes two NEW users (names carry the run time, so re-runs never meet old accounts), each uploading a DIFFERENT
half of the portraits in DIR (<person><n>.jpg), turns faces on, waits for the grouping, then asserts that every photo id
listed by api/photos/people/ID for one user is one of that user's own photos (the WHOLE library, api/photos?all=1, not
one folder) and that each card's count matches its photo list. Fails loudly when a user has 0 person cards.
Case B: both users upload one IDENTICAL picture (same bytes). Each must see only their own id for it, and fetching the
other user's id (thumb, raw) as themselves must be 403/404."""
import glob, http.cookiejar, json, os, sys, time, urllib.error, urllib.parse, urllib.request


class C:
    def __init__(s, base):
        s.b = base.rstrip("/") + "/"
        s.o = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))

    def call(s, m, p, body=None, raw=None):
        d = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        r = urllib.request.Request(s.b + p, data=d, method=m, headers={"content-type": "application/json"} if body is not None else {})
        try:
            with s.o.open(r, timeout=120) as x:
                t = x.read()
        except urllib.error.HTTPError as e:
            return e.code, {}
        try:
            return 200, json.loads(t)
        except Exception:
            return 200, t


q = lambda x: urllib.parse.quote(x, safe="")


def main():
    base = sys.argv[1]
    pics = sys.argv[sys.argv.index("--pics") + 1]
    names = sorted({os.path.basename(f).rstrip("0123456789.jpg") for f in glob.glob(pics + "/*.jpg")})
    assert len(names) >= 2, "need at least two people in --pics"
    run = time.strftime("%H%M%S")
    half = {"una" + run: names[: len(names) // 2], "vic" + run: names[len(names) // 2:]}
    ada = C(base)
    assert ada.call("POST", "api/login", {"username": "ada", "password": "adapassword"})[0] == 200
    users = {}
    for u, ns in half.items():
        c = C(base)
        assert c.call("POST", "api/signup", {"username": u, "name": u.title(), "password": u + "password"})[0] == 200, "signup"
        fo = c.call("POST", f"api/folders?path={u}&name=Faces")[1]["path"]
        for n in ns:
            for f in sorted(glob.glob(f"{pics}/{n}*.jpg")):
                st, _ = c.call("POST", f"api/files?name={q(os.path.basename(f))}&as_is=1&folder={q(fo)}", raw=open(f, "rb").read())
                assert st == 200, st
        # Case B: the identical picture (same bytes) for both users
        same = sorted(glob.glob(pics + "/*.jpg"))[0]
        st, _ = c.call("POST", f"api/files?name=same.jpg&as_is=1&folder={q(fo)}", raw=open(same, "rb").read())
        assert st == 200, st
        assert c.call("POST", f"api/photos/enable?folder={q(fo)}")[0] == 200
        users[u] = (c, fo, ns)
    print("faces on:", ada.call("POST", "api/photos/faces", {"on": True})[0], flush=True)

    def cards(c):
        p = c.call("GET", "api/photos/people")[1]
        return p.get("people", []) + p.get("groups", [])

    def library(c):
        lst = c.call("GET", "api/photos?all=1")[1]
        return lst.get("items") or lst.get("photos") or []

    t = time.time()
    while time.time() - t < 600:
        time.sleep(5)
        if all(sum(g["count"] for g in cards(c)) >= 3 * len(ns) for c, fo, ns in users.values()):
            break
    bad = 0
    mine_all = {}
    for u, (c, fo, ns) in users.items():
        mine = {i["id"]: i.get("name", "?") for i in library(c)}
        mine_all[u] = mine
        cs = cards(c)
        compared = 0
        for card in cs:
            one = c.call("GET", f"api/photos/people/{card['id']}")[1]
            ids = [p["id"] for p in one["photos"]]
            foreign = [i for i in ids if i not in mine]
            compared += len(ids)
            print(f"{u}: {card['id']} count={card['count']} listed={len(ids)} FOREIGN={foreign}")
            bad += len(foreign) + (card["count"] != len(ids))
        print(f"{u}: owns {len(mine)} photos (all=1), {len(cs)} person cards, {compared} photo ids compared", flush=True)
        if not cs or compared == 0:
            print(f"{u}: FAIL 0 person cards / 0 photos compared: the test proves nothing (are the face models downloaded?)")
            bad += 1
    # Case B: identical bytes, two owners
    sid = {}
    for u in users:
        ids = [i for i, n in mine_all[u].items() if n == "same.jpg"]
        print(f"caseB {u}: ids named same.jpg in own library = {len(ids)}")
        if len(ids) != 1:
            bad += 1
        sid[u] = ids[0] if ids else None
    a, b = list(users)
    if sid[a] and sid[b]:
        if sid[a] == sid[b]:
            print("caseB FAIL: both users got the same id")
            bad += 1
        for me, other in ((a, b), (b, a)):
            c = users[me][0]
            for path in (f"api/photos/thumb/{sid[other]}/256", f"api/files/{sid[other]}/raw"):
                st = c.call("GET", path)[0]
                print(f"caseB {me} fetching {other}'s {path.split('/')[1]}/{path.split('/')[2]}: HTTP {st}")
                bad += st not in (403, 404)
            st = c.call("GET", f"api/files/{sid[me]}/raw")[0]
            print(f"caseB {me} fetching own raw: HTTP {st}")
            bad += st != 200
            for card in cards(c):
                ids = [p["id"] for p in c.call("GET", f"api/photos/people/{card['id']}")[1]["photos"]]
                if sid[other] in ids:
                    print(f"caseB FAIL: {me}'s card lists {other}'s same.jpg")
                    bad += 1
    print("PRIVACY", "FAIL" if bad else "PASS")
    sys.exit(1 if bad else 0)


main()
