#!/usr/bin/env python3
"""Checks the Rust server answers like server/files.py did: runs both on two copies of the same
folder, sends the same requests to each, and compares status codes, replies and the files left
on disk. Random ids and file versions are mapped to placeholders before comparing.

  python3 crates/server/tests/parity.py path/to/starsheets-server [path/to/files.py]
"""

import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
RUST = sys.argv[1]
PY = sys.argv[2] if len(sys.argv) > 2 else os.path.join(HERE, "reference", "files.py")


def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def seed(store):
    os.makedirs(os.path.join(store, "Work", "Old"))
    for rel_path, data in [("Budget.xlsx", b"budget"), ("Work/Plan.xlsx", b"plan"), ("Work/Old/Ancient.csv", b"a,b"), ("notes.txt", b"not a sheet"), (".hidden.xlsx", b"x")]:
        with open(os.path.join(store, rel_path), "wb") as f:
            f.write(data)
    site = os.path.join(store, "..", "site")
    os.makedirs(os.path.join(site, "assets"), exist_ok=True)
    for name, data in [("index.html", b"<html>app</html>"), ("app-0123456789abcdef.wasm", b"\0asm"), ("build.json", b"{}")]:
        with open(os.path.join(site, name), "wb") as f:
            f.write(data)
    return os.path.abspath(site)


class Server:
    def __init__(self, cmd, store, site, auth):
        self.port = free_port()
        env = dict(os.environ, STORE=store, PORT=str(self.port), KEEP="2", LOCATION="the test", SITE=site,
                   STARSHEETS_AUTH="on" if auth else "off", STARSHEETS_PASSWORD="parity-setup-code")
        self.proc = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.cookie = ""
        for _ in range(100):
            try:
                urllib.request.urlopen(f"http://127.0.0.1:{self.port}/api/info", timeout=1)
                return
            except (urllib.error.URLError, ConnectionError):
                time.sleep(0.05)
        raise SystemExit(f"{cmd} did not start")

    def call(self, method, path, body=None, headers=None):
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=body, method=method, headers=headers or {})
        if self.cookie:
            req.add_header("Cookie", self.cookie)
        try:
            r = urllib.request.urlopen(req, timeout=10)
        except urllib.error.HTTPError as e:
            r = e
        data = r.read()
        h = {k.lower(): v for k, v in r.headers.items()}
        if "set-cookie" in h and h["set-cookie"].split("=", 1)[1].split(";")[0]:
            self.cookie = h["set-cookie"].split(";")[0]
        return r.status, h, data

    def stop(self):
        self.proc.terminate()
        self.proc.wait(5)


class Norm:
    """Maps random ids / versions / times to stable placeholders, per server."""

    def __init__(self):
        self.ids, self.versions = {}, {}

    def id(self, v):
        return self.ids.setdefault(v, f"ID{len(self.ids)}") if v else v

    def value(self, v, key=""):
        if isinstance(v, dict):
            return {k: self.value(x, k) for k, x in v.items()}
        if isinstance(v, list):
            return [self.value(x, key) for x in v]
        if key == "id":
            return self.id(v)
        if key in ("version", "v"):
            return self.versions.setdefault(v, f"V{len(self.versions)}")
        if key in ("modified", "deleted"):
            return "T"
        return v


def run(server, norm, steps):
    out = []
    ctx = {}
    for step in steps:
        method, path, body, headers, keep = step
        path = path.format(**{k: urllib.parse.quote(str(v), safe='') for k, v in ctx.items()})
        headers = {k: v.format(**ctx) for k, v in (headers or {}).items()}
        status, h, data = server.call(method, path, body, headers)
        try:
            reply = json.loads(data) if data else None
        except ValueError:
            reply = data.decode("latin-1")
        if keep and isinstance(reply, dict):
            for name, key in keep.items():
                ctx[name] = reply[key]
        if method == "GET" and keep and "etag" in h:
            ctx[keep.get("_etag", "etag")] = h["etag"]
        if isinstance(reply, list) and keep and "_first_v" in keep and reply:
            ctx[keep["_first_v"]] = reply[0]["v"]
        if isinstance(reply, list) and keep and "_first_bin" in keep and reply:
            ctx[keep["_first_bin"]] = reply[0]["name"]
        shown = {k: h[k] for k in ("x-file-name", "x-folder", "content-type", "cache-control", "content-encoding") if k in h}
        if "etag" in h:
            shown["etag"] = norm.value(h["etag"], "version")
        reply = norm.value(reply)
        if isinstance(reply, dict) and isinstance(reply.get("files"), list):
            reply["files"].sort(key=lambda f: (f["folder"], f["name"]))
        out.append((method, re.sub(r"[0-9a-f]{16}", "ID", path), status, reply, shown))
    return out


def tree(store):
    """Visible files with contents, plus how many versions/bin entries exist."""
    found = []
    for root, dirs, names in os.walk(store):
        dirs.sort()
        for n in sorted(names):
            full = os.path.join(root, n)
            rel_path = os.path.relpath(full, store)
            if rel_path.startswith(".starsheets"):
                if "versions" in rel_path:
                    found.append(("version of", rel_path.split(os.sep)[2] and "X", open(full, "rb").read()))
                continue
            found.append((rel_path, open(full, "rb").read()))
    return sorted(found, key=repr)


STEPS = [
    ("GET", "/api/info", None, None, None),
    ("GET", "/api/files", None, None, None),
    ("POST", "/api/files?name=Budget&folder=", b"new budget", None, {"new": "id"}),
    ("POST", "/api/files?name=a%2Fb%3Ac.XLSX&folder=Work", b"slashes", None, None),
    ("POST", "/api/files?folder=Nope", b"x", None, None),
    ("POST", "/api/files?folder=..%2Fx", b"x", None, None),
    ("GET", "/api/files/{new}", None, None, {"_etag": "etag"}),
    ("PUT", "/api/files/{new}", b"v2", {"If-Match": "{etag}"}, None),
    ("PUT", "/api/files/{new}", b"stale", {"If-Match": "{etag}"}, None),
    ("PUT", "/api/files/{new}", b"v3", None, None),
    ("PUT", "/api/files/{new}", b"v4", None, None),
    ("GET", "/api/files/{new}/versions", None, None, {"_first_v": "v0"}),
    ("GET", "/api/files/{new}/versions/{v0}", None, None, None),
    ("GET", "/api/files/{new}/versions/nope", None, None, None),
    ("POST", "/api/files/{new}/rename?name=Budget", None, None, None),
    ("POST", "/api/files/{new}/rename?name=Caf%C3%A9%20%E2%9C%93", None, None, None),
    ("GET", "/api/files/{new}", None, None, None),
    ("POST", "/api/files/{new}/copy", None, None, {"copy": "id"}),
    ("POST", "/api/files/{copy}/move?folder=Work/Old", None, None, None),
    ("POST", "/api/files/{copy}/copy?folder=Work", None, None, None),
    ("GET", "/api/files/nope", None, None, None),
    ("POST", "/api/folders?path=Work&name=Sub", None, None, None),
    ("POST", "/api/folders?path=Work&name=Sub", None, None, None),
    ("POST", "/api/folders?path=", None, None, None),
    ("POST", "/api/folders/rename?path=Work/Sub&name=Renamed", None, None, None),
    ("POST", "/api/folders/move?path=Work/Old&to=Work/Renamed", None, None, None),
    ("POST", "/api/folders/move?path=Work&to=Work/Renamed", None, None, None),
    ("POST", "/api/folders/copy?path=Work/Renamed&to=", None, None, None),
    ("POST", "/api/folders/rename?path=&name=x", None, None, None),
    ("GET", "/api/files", None, None, None),
    ("DELETE", "/api/files/{copy}", None, None, None),
    ("DELETE", "/api/folders?path=Renamed", None, None, None),
    ("GET", "/api/bin", None, None, {"_first_bin": "b0"}),
    ("POST", "/api/bin/restore?name={b0}", None, None, None),
    ("POST", "/api/bin/restore?name=nope", None, None, None),
    ("DELETE", "/api/bin?name=nope", None, None, None),
    ("DELETE", "/api/bin", None, None, None),
    ("GET", "/api/bin", None, None, None),
    ("GET", "/api/files", None, None, None),
    ("GET", "/api/whatever", None, None, None),
    ("GET", "/", None, None, None),
    ("GET", "/some/route", None, None, None),
    ("GET", "/app-0123456789abcdef.wasm", None, None, None),
    ("GET", "/missing.js", None, None, None),
    ("GET", "/build.json", None, None, None),
]

def compare(label, steps, auth):
    results, trees = [], []
    for cmd in ([sys.executable, "-u", PY], [RUST]):
        base = tempfile.mkdtemp()
        store = os.path.join(base, "store")
        os.makedirs(store)
        site = seed(store)
        s = Server(cmd, store, site, auth)
        try:
            results.append(run(s, Norm(), steps))
        finally:
            s.stop()
        trees.append(tree(store))
        shutil.rmtree(base)
    bad = 0
    for a, b in zip(*results):
        # files.py sorted versions saved within one second at random (fixed in the Rust server).
        if "/versions" in a[1]:
            continue
        # /api/info gained fields with accounts (setup, user): the old ones must still match.
        if a[1] == "/api/info" and isinstance(b[3], dict):
            b = (*b[:3], {k: v for k, v in b[3].items() if k in a[3]}, b[4])
        # The Drive's Undo (2026-11-25): a move or rename also answers with the file's id, a delete
        # with its name in the bin and whose bin it is. New fields only: the old ones must match.
        if a[0] in ("POST", "DELETE") and isinstance(a[3], dict) and isinstance(b[3], dict):
            b = (*b[:3], {k: v for k, v in b[3].items() if k in a[3] or k not in ("id", "bin", "space")}, b[4])
        a_cmp, b_cmp = list(a), list(b)
        # A file restored from the bin keeps its id in the Rust server (its versions and sharing
        # come back); files.py gave it a new one. Ids are compared as names, so drop them here.
        if a[1] == "/api/files" and a[0] == "GET":
            for x in (a_cmp, b_cmp):
                x[3] = {**x[3], "files": [{k: v for k, v in f.items() if k != "id"} for f in x[3]["files"]]}
        # Content types may be spelt differently (text/javascript vs application/javascript).
        for x in (a_cmp, b_cmp):
            x[4] = {k: v for k, v in x[4].items() if k != "content-type" or "json" in v or "octet" in v or "wasm" in v}
        if a_cmp != b_cmp:
            bad += 1
            print(f"DIFF {label}: {a[0]} {a[1]}\n  python: {a[2:]}\n  rust:   {b[2:]}")
    trees = [[t for t in tr if t[0] != "version of"] for tr in trees]
    if trees[0] != trees[1]:
        bad += 1
        print(f"DIFF {label}: files on disk\n  python: {trees[0]}\n  rust:   {trees[1]}")
    print(f"{label}: {len(steps)} requests, {'all the same' if not bad else f'{bad} differences'}")
    return bad


def version_order():
    """Rust only: a burst of saves (within a minute) keeps the newest two, oldest dropped."""
    base = tempfile.mkdtemp()
    store = os.path.join(base, "store")
    os.makedirs(store)
    s = Server([RUST], store, seed(store), auth=False)
    try:
        _, _, data = s.call("POST", "/api/files?name=V", b"v1")
        fid = json.loads(data)["id"]
        for body in (b"v2", b"v3", b"v4", b"v5", b"v6", b"v7", b"v8"):
            s.call("PUT", f"/api/files/{fid}", body)
        vs = json.loads(s.call("GET", f"/api/files/{fid}/versions")[2])
        got = [s.call("GET", f"/api/files/{fid}/versions/{v['v']}")[2] for v in vs]
    finally:
        s.stop()
        shutil.rmtree(base)
    ok = got == [b"v7", b"v6"]
    print(f"version order: {'right' if ok else f'WRONG {got}'}")
    return 0 if ok else 1


def accounts():
    """Rust only (files.py had one shared password): setup code, owner, people, sign in and out."""
    base = tempfile.mkdtemp()
    store = os.path.join(base, "store")
    os.makedirs(store)
    s = Server([RUST], store, seed(store), auth=True)
    post = lambda path, body: s.call("POST", path, json.dumps(body).encode(), {"Content-Type": "application/json"})
    got = []
    try:
        info = json.loads(s.call("GET", "/api/info")[2])
        got.append(("new server asks for setup", info["setup"] and not info["signed_in"]))
        got.append(("files need sign-in", s.call("GET", "/api/files")[0] == 401))
        got.append(("wrong setup code", post("/api/setup", {"code": "nope", "username": "owner", "name": "O", "password": "longpassword"})[0] == 401))
        got.append(("setup", post("/api/setup", {"code": "parity-setup-code", "username": "Owner", "name": "The Owner", "password": "longpassword"})[0] == 200))
        info = json.loads(s.call("GET", "/api/info")[2])
        got.append(("signed in as owner", info["user"]["username"] == "owner" and info["user"]["owner"] and not info["setup"]))
        got.append(("second setup refused", post("/api/setup", {"code": "parity-setup-code", "username": "x2", "name": "X", "password": "longpassword"})[0] == 400))
        got.append(("owner adds a person", post("/api/users", {"username": "bob", "name": "Bob", "password": "bobspassword"})[0] == 200))
        got.append(("files open", s.call("GET", "/api/files")[0] == 200))
        s.call("POST", "/api/logout")
        s.cookie = ""
        got.append(("signed out", s.call("GET", "/api/files")[0] == 401))
        got.append(("wrong password", post("/api/login", {"username": "bob", "password": "nope"})[0] == 401))
        got.append(("bob signs in", post("/api/login", {"username": "BOB", "password": "bobspassword"})[0] == 200))
        got.append(("bob can't add people", post("/api/users", {"username": "eve", "name": "Eve", "password": "evespassword"})[0] == 403))
        got.append(("bob renames himself", json.loads(post("/api/me", {"name": "Robert"})[2])["user"]["name"] == "Robert"))
        got.append(("people list", [u["username"] for u in json.loads(s.call("GET", "/api/users")[2])] == ["owner", "bob"]))
        got.append(("no password in the list", "hash" not in s.call("GET", "/api/users")[2].decode()))
        # Sharing: bob (signed in) can't see the owner's files until they're shared with him.
        listed = lambda: [f["name"] for f in json.loads(s.call("GET", "/api/files")[2])["files"]]
        got.append(("bob sees no one else's files", listed() == []))
        budget_id = None
        s.call("POST", "/api/logout"); s.cookie = ""
        post("/api/login", {"username": "owner", "password": "longpassword"})
        files = json.loads(s.call("GET", "/api/files")[2])["files"]
        budget_id = [f["id"] for f in files if f["name"] == "Budget.xlsx"][0]
        got.append(("the server's owner sees every file", len(files) == 4))  # notes.txt too (every file type is listed)
        got.append(("owner shares with bob to view", post(f"/api/files/{budget_id}/sharing", {"people": {"bob": "view"}})[0] == 200))
        s.call("POST", "/api/logout"); s.cookie = ""
        post("/api/login", {"username": "bob", "password": "bobspassword"})
        got.append(("bob sees the shared file", listed() == ["Budget.xlsx"]))
        got.append(("bob can open it", s.call("GET", f"/api/files/{budget_id}")[0] == 200))
        got.append(("bob can't save it", s.call("PUT", f"/api/files/{budget_id}", b"x")[0] == 403))
        got.append(("bob can't share it", post(f"/api/files/{budget_id}/sharing", {"link": "edit"})[0] == 403))
        got.append(("bob can't touch folders", post("/api/folders/rename?path=Work&name=X", {})[0] == 403))
        sharing = json.loads(s.call("GET", f"/api/files/{budget_id}/sharing")[2])
        got.append(("bob sees who has access", sharing["you"] == "view" and [p["role"] for p in sharing["people"]] == ["owner", "view"]))
        made = json.loads(s.call("POST", "/api/files?name=Bobs", b"bob's")[2])
        got.append(("bob's own file is his", json.loads(s.call("GET", f"/api/files/{made['id']}/sharing")[2])["you"] == "owner"))
        # Sign-up: off until the owner turns it on; the code when one is set; only the owner sets it.
        s.call("POST", "/api/logout"); s.cookie = ""
        info = json.loads(s.call("GET", "/api/info")[2])
        got.append(("sign-up off by default", not info["signup"] and post("/api/signup", {"username": "cat", "name": "Cat", "password": "catspassword"})[0] == 403))
        post("/api/login", {"username": "bob", "password": "bobspassword"})
        got.append(("bob can't turn sign-up on", post("/api/signup/settings", {"on": True})[0] == 403))
        s.call("POST", "/api/logout"); s.cookie = ""
        post("/api/login", {"username": "owner", "password": "longpassword"})
        got.append(("owner turns sign-up on with a code", json.loads(post("/api/signup/settings", {"on": True, "code": "joinus"})[2]) == {"on": True, "code": "joinus"}))
        s.call("POST", "/api/logout"); s.cookie = ""
        info = json.loads(s.call("GET", "/api/info")[2])
        got.append(("sign-in box offers sign-up", info["signup"] and info["signup_code"] and "joinus" not in json.dumps(info)))
        got.append(("wrong sign-up code", post("/api/signup", {"username": "cat", "name": "Cat", "password": "catspassword", "code": "nope"})[0] == 401))
        r = post("/api/signup", {"username": "Cat", "name": "Cat", "password": "catspassword", "code": "joinus"})
        got.append(("cat signs up and is signed in", r[0] == 200 and not json.loads(r[2])["user"]["owner"] and json.loads(s.call("GET", "/api/info")[2])["user"]["username"] == "cat"))
        got.append(("cat sees no one else's files", listed() == []))
        # Everyone has their own folder; folder sharing covers what's inside, also added later.
        lst = lambda: json.loads(s.call("GET", "/api/files")[2])
        got.append(("cat's files go in cat's folder", json.loads(s.call("POST", "/api/files?name=Mine", b"m")[2])["folder"] == "cat"))
        got.append(("no folders at the very top", post("/api/folders?path=&name=X", {})[0] == 201 and lst()["folders"] == ["cat", "cat/X"]))
        got.append(("not in someone else's folder", post("/api/folders?path=owner&name=Y", {})[0] == 403))
        team = json.loads(post("/api/folders?path=cat/X&name=Team", {})[2])["path"]
        s.call("POST", f"/api/files?name=Inside&folder={team}", b"i")
        got.append(("cat shares a folder with bob", post(f"/api/folders/sharing?path={team}", {"people": {"bob": "edit"}})[0] == 200))
        got.append(("cat colours a folder (only for cat)", post(f"/api/folders/colour?path={team}", {"colour": "#2a9d8f"})[0] == 200 and json.loads(s.call("GET", "/api/files")[2])["folder_info"][team]["colour"] == "#2A9D8F"))
        got.append(("a person's own folder can't be shared", post("/api/folders/sharing?path=cat", {"link": "view"})[0] == 400))
        s.call("POST", "/api/logout"); s.cookie = ""
        post("/api/login", {"username": "bob", "password": "bobspassword"})
        mine = lst()
        got.append(("bob sees the shared folder", mine["shared"]["folders"] == [team] and "cat/X" not in mine["folders"] and any(f["name"] == "Inside.xlsx" for f in mine["files"])))
        got.append(("bob adds a file in it", json.loads(s.call("POST", f"/api/files?name=FromBob&folder={team}", b"f")[2]).get("folder") == team))
        got.append(("bob can't take cat's folder", post(f"/api/folders/move?path={team}&to=bob", {})[0] == 403))
        fb = [f for f in lst()["files"] if f["name"] == "FromBob.xlsx"][0]
        got.append(("what bob adds there is cat's", fb["owner"] == "cat" and fb["role"] == "edit"))
        got.append(("bob stars it", post(f"/api/files/{fb['id']}/star", {"on": True})[0] == 200 and [f for f in lst()["files"] if f["id"] == fb["id"]][0]["starred"]))
        got.append(("bob deletes it: it goes to cat's bin", s.call("DELETE", f"/api/files/{fb['id']}")[0] == 200 and json.loads(s.call("GET", "/api/bin")[2]) == []))
        s.call("POST", "/api/logout"); s.cookie = ""
        post("/api/login", {"username": "cat", "password": "catspassword"})
        bin_ = json.loads(s.call("GET", "/api/bin")[2])
        got.append(("cat's bin has it", [b["name"] for b in bin_] == ["FromBob.xlsx"]))
        post("/api/bin/restore?name=FromBob.xlsx", {})
        got.append(("restored with its id", any(f["id"] == fb["id"] for f in lst()["files"])))
    finally:
        s.stop()
        shutil.rmtree(base)
    bad = [name for name, ok in got if not ok]
    print(f"accounts: {len(got)} checks, {'all right' if not bad else 'WRONG: ' + ', '.join(bad)}")
    return 1 if bad else 0


if __name__ == "__main__":
    fails = compare("files", STEPS, auth=False) + version_order() + accounts()
    sys.exit(1 if fails else 0)
