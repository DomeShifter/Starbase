#!/usr/bin/env python3
# RETIRED (replaced by crates/server, the Rust server). Kept only as the reference that
# crates/server/tests/parity.py checks the Rust server against. Not deployed.
"""StarSheets file store: spreadsheets as ordinary .xlsx files in a folder tree (STORE).

A small JSON API for the web app, served behind the same nginx as the app (under /api/).
Folders are paths relative to STORE with "/" ("" = the top). Files:

  GET    /api/files                         {files: [{id, name, folder, modified, size, version}], folders: [path]}
  POST   /api/files?name=N&folder=F         new file (body = xlsx bytes) -> {id, name, version}
  GET    /api/files/ID                      the file's bytes (header ETag = version, X-File-Name)
  PUT    /api/files/ID                      save (header If-Match = the version it was based on)
                                            -> {version}; 409 if the file changed since
  POST   /api/files/ID/rename?name=N        -> {name}
  POST   /api/files/ID/move?folder=F        -> {name}
  POST   /api/files/ID/copy?folder=F        -> {id, name}  (same folder: "Copy of N")
  DELETE /api/files/ID                      move to the bin
  GET    /api/files/ID/versions             older copies: [{v, modified, size}]
  GET    /api/files/ID/versions/V           an older copy's bytes
Folders:
  POST   /api/folders?path=P&name=N         new folder in P -> {path}
  POST   /api/folders/rename?path=P&name=N  -> {path}
  POST   /api/folders/move?path=P&to=F      -> {path}
  POST   /api/folders/copy?path=P&to=F      -> {path}
  DELETE /api/folders?path=P                move to the bin (with everything in it)
Bin (STORE/.bin):
  GET    /api/bin                           [{name, original, deleted, folder}]
  POST   /api/bin/restore?name=B            back where it was (a free name if that's taken)
  DELETE /api/bin?name=B                    delete forever; without name: empty the bin

Names stay readable ("Budget.xlsx"). A name that is taken gets " (2)", " (3)"…; the free name is
claimed with an exclusive create, so two saves at once can't take the same one. Each file has a
hidden id (STORE/.starsheets/index.json) so the app follows a file across renames and moves; a
file added by hand gets an id when it is first listed. The last KEEP versions of each file are
kept in STORE/.starsheets/versions/ID/. Folders and files starting with "." are not shown.

Also GET /api/info -> {location, auth, signed_in}; POST /api/login {password}; POST /api/logout.

Login: on unless STARSHEETS_AUTH=off. The password is STARSHEETS_PASSWORD; without one, a random
password is made on first start, kept in STORE/.starsheets/password and printed in the log. A
login gives an HttpOnly cookie for 30 days. Everything under /api/ except info and login needs it.

The app itself: with SITE set (a built StarSheets site), every other path serves its files
(index.html never cached, hashed files cached for good, a .gz copy sent when the browser takes it).

Standard library only. Settings: STORE (folder), KEEP (versions, default 6), PORT (8080),
LOCATION (how people are told where files are kept, e.g. "the office server"; default "the server"),
STARSHEETS_AUTH, STARSHEETS_PASSWORD, SITE.
"""

import hashlib
import hmac
import json
import mimetypes
import os
import re
import secrets
import shutil
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

STORE = os.environ.get("STORE", "/data")
KEEP = int(os.environ.get("KEEP", "6"))
PORT = int(os.environ.get("PORT", "8080"))
LOCATION = os.environ.get("LOCATION", "the server")
AUTH = os.environ.get("STARSHEETS_AUTH", "on").strip().lower() not in ("off", "false", "0", "no")
SITE = os.environ.get("SITE", "")
SESSION_DAYS = 30
COOKIE = "starsheets_session"
META = os.path.join(STORE, ".starsheets")
INDEX = os.path.join(META, "index.json")
BIN_INDEX = os.path.join(META, "bin.json")
VERSIONS = os.path.join(META, "versions")
BIN = os.path.join(STORE, ".bin")
MAX_BYTES = 200 * 1024 * 1024
EXTS = (".xlsx", ".xlsm", ".xls", ".csv", ".ods")

lock = threading.Lock()


class Bad(Exception):
    """A request that can't be done: (http status, message)."""

    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


# ---------------------------------------------------------------- paths and names

def rel(path):
    """A safe folder path relative to STORE ("" = the top), or Bad."""
    parts = [p for p in (path or "").replace("\\", "/").split("/") if p]
    for p in parts:
        if p in (".", "..") or p.startswith("."):
            raise Bad(400, "bad folder name")
    return "/".join(parts)


def full(relpath):
    return os.path.join(STORE, *relpath.split("/")) if relpath else STORE


def join(folder, name):
    return f"{folder}/{name}" if folder else name


def parent_of(relpath):
    return relpath.rsplit("/", 1)[0] if "/" in relpath else ""


def base_of(relpath):
    return relpath.rsplit("/", 1)[-1]


def clean_name(name):
    """A safe display name: no folders, no characters Windows/SMB refuse, not empty."""
    name = re.sub(r'[\\/:*?"<>|\x00-\x1f]', "-", (name or "").strip()).strip(". ")
    return name[:150] or "Untitled spreadsheet"


def split_name(name):
    stem, ext = os.path.splitext(name)
    if ext.lower() not in EXTS:
        stem, ext = name, ".xlsx"
    return stem, ext


def claim_name(folder, stem, ext, own=None):
    """Reserves a free file name in `folder` (stem.ext, stem (2).ext, …) by creating it exclusively."""
    for n in range(1, 10000):
        name = f"{stem}{'' if n == 1 else f' ({n})'}{ext}"
        if own is not None and join(folder, name) == own:
            return name
        try:
            fd = os.open(os.path.join(full(folder), name), os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o664)
            os.close(fd)
            return name
        except FileExistsError:
            continue
    raise Bad(409, "no free name")


def claim_folder(parent, name):
    """Creates a free folder name in `parent` ("Name", "Name (2)", …)."""
    name = clean_name(name) if name else "New folder"
    for n in range(1, 10000):
        candidate = f"{name}{'' if n == 1 else f' ({n})'}"
        try:
            os.mkdir(os.path.join(full(parent), candidate))
            return join(parent, candidate)
        except FileExistsError:
            continue
    raise Bad(409, "no free name")


def free_folder_name(parent, name):
    for n in range(1, 10000):
        candidate = f"{name}{'' if n == 1 else f' ({n})'}"
        if not os.path.exists(os.path.join(full(parent), candidate)):
            return candidate
    raise Bad(409, "no free name")


def need_folder(folder):
    if not os.path.isdir(full(folder)):
        raise Bad(404, "no such folder")


# ---------------------------------------------------------------- index

def load_json(path):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def save_json(path, data):
    os.makedirs(META, exist_ok=True)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8") as f:
        json.dump(data, f, ensure_ascii=False, indent=1)
    os.replace(tmp, path)


def walk():
    """(spreadsheet files, folders) as paths relative to STORE, skipping hidden ones."""
    files, folders = [], []
    for root, dirs, names in os.walk(STORE):
        dirs[:] = sorted(d for d in dirs if not d.startswith("."))
        here = os.path.relpath(root, STORE).replace(os.sep, "/")
        here = "" if here == "." else here
        for d in dirs:
            folders.append(join(here, d))
        for n in names:
            # Since the any-file Drive (2026-11) every file is listed, not only spreadsheets.
            if not n.startswith(".") and os.path.isfile(os.path.join(root, n)):
                files.append(join(here, n))
    return files, folders


def synced_index():
    """The index ({id: path}), in line with the folder tree: new files get ids, gone ones drop."""
    index = load_json(INDEX)
    files, _ = walk()
    present = set(files)
    changed = False
    for fid, path in list(index.items()):
        if path not in present:
            del index[fid]
            changed = True
    known = set(index.values())
    for path in sorted(present - known):
        index[secrets.token_hex(8)] = path
        changed = True
    if changed:
        save_json(INDEX, index)
    return index


def move_prefix(index, old, new):
    """Rewrites index paths under folder `old` to `new` (a folder renamed or moved)."""
    for fid, path in index.items():
        if path.startswith(old + "/"):
            index[fid] = new + path[len(old):]


def version_of(path):
    st = os.stat(path)
    return f"{st.st_mtime_ns:x}-{st.st_size:x}"


def write_file(path, data):
    os.makedirs(META, exist_ok=True)
    tmp = os.path.join(META, f".{secrets.token_hex(6)}.tmp")
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def keep_version(fid, path):
    """Copies the file as it is now into its versions folder, keeping the newest KEEP."""
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return
    folder = os.path.join(VERSIONS, fid)
    os.makedirs(folder, exist_ok=True)
    shutil.copyfile(path, os.path.join(folder, f"{time.strftime('%Y%m%d-%H%M%S')}-{secrets.token_hex(2)}.xlsx"))
    old = sorted(os.listdir(folder))
    for v in old[:-KEEP] if KEEP > 0 else old:
        os.remove(os.path.join(folder, v))


def to_bin(relpath):
    """Moves a file or folder into the bin, remembering where it was."""
    os.makedirs(BIN, exist_ok=True)
    name = base_of(relpath)
    stem, ext = os.path.splitext(name) if os.path.isfile(full(relpath)) else (name, "")
    # Its own name; only a clash with something already in the bin adds " (2)", " (3)"…
    target = name
    n = 1
    while os.path.exists(os.path.join(BIN, target)):
        n += 1
        target = f"{stem} ({n}){ext}"
    shutil.move(full(relpath), os.path.join(BIN, target))
    bins = load_json(BIN_INDEX)
    bins[target] = {"original": relpath, "deleted": int(time.time())}
    save_json(BIN_INDEX, bins)


# ---------------------------------------------------------------- HTTP

# ---------------------------------------------------------------- login

def password():
    """The password: STARSHEETS_PASSWORD, else one made (and kept) on first start."""
    given = os.environ.get("STARSHEETS_PASSWORD", "")
    if given:
        return given
    path = os.path.join(META, "password")
    try:
        return open(path, encoding="utf-8").read().strip()
    except OSError:
        made = secrets.token_urlsafe(12)
        os.makedirs(META, exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(made + "\n")
        os.chmod(path, 0o600)
        return made


SESSIONS = os.path.join(META, "sessions.json")


def token_hash(token):
    return hashlib.sha256(token.encode()).hexdigest()


def new_session():
    token = secrets.token_urlsafe(32)
    sessions = {k: v for k, v in load_json(SESSIONS).items() if v > time.time()}
    sessions[token_hash(token)] = time.time() + SESSION_DAYS * 86400
    save_json(SESSIONS, sessions)
    return token


def signed_in(h):
    if not AUTH:
        return True
    for part in (h.headers.get("Cookie") or "").split(";"):
        name, _, value = part.strip().partition("=")
        if name == COOKIE and value:
            return load_json(SESSIONS).get(token_hash(value), 0) > time.time()
    return False


class Handler(BaseHTTPRequestHandler):
    server_version = "StarSheetsFiles/2"

    def log_message(self, fmt, *args):
        pass

    def send_json(self, obj, code=200):
        body = json.dumps(obj, ensure_ascii=False).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def send_bytes(self, data, name, version="", folder="", modified=0):
        self.send_response(200)
        self.send_header("Content-Type", "application/octet-stream")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-File-Name", json.dumps(name))
        self.send_header("X-Folder", json.dumps(folder))
        self.send_header("X-Modified", str(int(modified)))
        if version:
            self.send_header("ETag", version)
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        if n > MAX_BYTES:
            raise Bad(413, "file too large")
        return self.rfile.read(n)

    def parts(self):
        u = urlparse(self.path)
        p = [unquote(x) for x in u.path.split("/") if x]
        if p[:1] != ["api"]:
            raise Bad(404, "not found")
        return p[1:], {k: v[0] for k, v in parse_qs(u.query, keep_blank_values=True).items()}

    def serve_site(self):
        """The app's own files (SITE), for a server that is the whole of StarSheets."""
        path = unquote(urlparse(self.path).path)
        if path == "/healthz":
            body = b"ok"
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        rel_path = os.path.normpath(path.lstrip("/")) if path not in ("", "/") else "index.html"
        full_path = os.path.join(SITE, rel_path)
        if rel_path.startswith("..") or not os.path.isfile(full_path):
            full_path = os.path.join(SITE, "index.html") if "." not in os.path.basename(rel_path) else ""
        if not full_path or not os.path.isfile(full_path):
            return self.send_json({"error": "not found"}, 404)
        kind = "application/wasm" if full_path.endswith(".wasm") else (mimetypes.guess_type(full_path)[0] or "application/octet-stream")
        send = full_path
        gz = full_path + ".gz"
        zipped = "gzip" in (self.headers.get("Accept-Encoding") or "") and os.path.isfile(gz)
        if zipped:
            send = gz
        data = open(send, "rb").read()
        self.send_response(200)
        self.send_header("Content-Type", kind)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("X-Content-Type-Options", "nosniff")
        if zipped:
            self.send_header("Content-Encoding", "gzip")
            self.send_header("Vary", "Accept-Encoding")
        name = os.path.basename(full_path)
        if name in ("index.html", "build.json"):
            self.send_header("Cache-Control", "no-cache")
        elif re.search(r"-[0-9a-f]{16}[._]", name):
            self.send_header("Cache-Control", "public, max-age=31536000, immutable")
        self.end_headers()
        self.wfile.write(data)

    def handle_any(self, method):
        try:
            if SITE and not urlparse(self.path).path.startswith("/api/") and method == "GET":
                return self.serve_site()
            p, q = self.parts()
            if p == ["info"] and method == "GET":
                return self.send_json({"location": LOCATION, "auth": AUTH, "signed_in": signed_in(self)})
            if p == ["login"] and method == "POST":
                try:
                    given = json.loads(self.body() or b"{}").get("password", "")
                except ValueError:
                    given = ""
                if not hmac.compare_digest(str(given).encode(), password().encode()):
                    time.sleep(1.5)  # slows down guessing
                    return self.send_json({"error": "wrong password"}, 401)
                with lock:
                    token = new_session()
                body = json.dumps({"ok": True}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Set-Cookie", f"{COOKIE}={token}; Path=/; Max-Age={SESSION_DAYS * 86400}; HttpOnly; SameSite=Strict")
                self.end_headers()
                self.wfile.write(body)
                return
            if p == ["logout"] and method == "POST":
                self.send_response(200)
                self.send_header("Set-Cookie", f"{COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Strict")
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            if not signed_in(self):
                return self.send_json({"error": "please sign in"}, 401)
            with lock:
                os.makedirs(META, exist_ok=True)
                out = route(self, method, p, q)
            if out is not None:
                self.send_json(*out) if isinstance(out, tuple) else self.send_json(out)
        except Bad as e:
            self.send_json({"error": str(e)}, e.code)
        except FileNotFoundError:
            self.send_json({"error": "not found"}, 404)
        except OSError as e:
            self.send_json({"error": f"storage error: {e.strerror or e}"}, 500)

    def do_GET(self):
        self.handle_any("GET")

    def do_POST(self):
        self.handle_any("POST")

    def do_PUT(self):
        self.handle_any("PUT")

    def do_DELETE(self):
        self.handle_any("DELETE")


def file_of(index, fid):
    path = index.get(fid)
    if not path:
        raise Bad(404, "no such file")
    return path


def route(h, method, p, q):
    """Returns a JSON reply (or (reply, status)); None when the handler already answered."""
    index = synced_index()
    # ---- files
    if p[:1] == ["files"]:
        if len(p) == 1 and method == "GET":
            files, folders = walk()
            by_path = {v: k for k, v in index.items()}
            out = []
            for path in files:
                st = os.stat(full(path))
                out.append({"id": by_path.get(path, ""), "name": base_of(path), "folder": parent_of(path), "modified": int(st.st_mtime), "size": st.st_size, "version": f"{st.st_mtime_ns:x}-{st.st_size:x}"})
            return {"files": out, "folders": folders}
        if len(p) == 1 and method == "POST":
            folder = rel(q.get("folder"))
            need_folder(folder)
            data = h.body()
            stem, ext = split_name(clean_name(q.get("name")))
            name = claim_name(folder, stem, ext)
            path = join(folder, name)
            write_file(full(path), data)
            fid = secrets.token_hex(8)
            index[fid] = path
            save_json(INDEX, index)
            return {"id": fid, "name": name, "folder": folder, "version": version_of(full(path))}, 201
        fid = p[1]
        path = file_of(index, fid)
        rest = p[2:]
        if not rest and method == "GET":
            data = open(full(path), "rb").read()
            h.send_bytes(data, base_of(path), version_of(full(path)), parent_of(path), os.path.getmtime(full(path)))
            return None
        if not rest and method == "PUT":
            data = h.body()
            want = h.headers.get("If-Match")
            if want and want != version_of(full(path)):
                return {"error": "changed elsewhere", "name": base_of(path)}, 409
            keep_version(fid, full(path))
            write_file(full(path), data)
            return {"version": version_of(full(path)), "name": base_of(path), "folder": parent_of(path)}
        if not rest and method == "DELETE":
            to_bin(path)
            del index[fid]
            save_json(INDEX, index)
            return {"ok": True}
        if rest == ["rename"] and method == "POST":
            folder = parent_of(path)
            stem, ext = split_name(clean_name(q.get("name")))
            _, old_ext = split_name(base_of(path))
            name = claim_name(folder, stem, old_ext or ext, own=path)
            new = join(folder, name)
            if new != path:
                os.replace(full(path), full(new))
                index[fid] = new
                save_json(INDEX, index)
            return {"name": name}
        if rest == ["move"] and method == "POST":
            folder = rel(q.get("folder"))
            need_folder(folder)
            if folder == parent_of(path):
                return {"name": base_of(path)}
            stem, ext = split_name(base_of(path))
            name = claim_name(folder, stem, ext)
            os.replace(full(path), os.path.join(full(folder), name))
            index[fid] = join(folder, name)
            save_json(INDEX, index)
            return {"name": name}
        if rest == ["copy"] and method == "POST":
            folder = rel(q.get("folder", parent_of(path)))
            need_folder(folder)
            stem, ext = split_name(base_of(path))
            if folder == parent_of(path):
                stem = f"Copy of {stem}"
            name = claim_name(folder, stem, ext)
            shutil.copyfile(full(path), os.path.join(full(folder), name))
            new_id = secrets.token_hex(8)
            index[new_id] = join(folder, name)
            save_json(INDEX, index)
            return {"id": new_id, "name": name}
        if rest[:1] == ["versions"] and method == "GET":
            vdir = os.path.join(VERSIONS, fid)
            vs = sorted(os.listdir(vdir), reverse=True) if os.path.isdir(vdir) else []
            if len(rest) == 1:
                return [{"v": v, "modified": int(os.path.getmtime(os.path.join(vdir, v))), "size": os.path.getsize(os.path.join(vdir, v))} for v in vs]
            if rest[1] not in vs:
                raise Bad(404, "no such version")
            h.send_bytes(open(os.path.join(vdir, rest[1]), "rb").read(), base_of(path))
            return None
        raise Bad(404, "not found")
    # ---- folders
    if p[:1] == ["folders"]:
        action = p[1] if len(p) > 1 else ""
        if not action and method == "POST":
            parent = rel(q.get("path"))
            need_folder(parent)
            return {"path": claim_folder(parent, q.get("name"))}, 201
        path = rel(q.get("path"))
        if not path:
            raise Bad(400, "the top folder can't be changed")
        need_folder(path)
        if not action and method == "DELETE":
            to_bin(path)
            return {"ok": True}
        if action == "rename" and method == "POST":
            want = clean_name(q.get("name"))
            if want == base_of(path):
                return {"path": path}
            new = join(parent_of(path), free_folder_name(parent_of(path), want))
            os.rename(full(path), full(new))
            move_prefix(index, path, new)
            save_json(INDEX, index)
            return {"path": new}
        if action in ("move", "copy") and method == "POST":
            to = rel(q.get("to"))
            need_folder(to)
            if to == path or to.startswith(path + "/"):
                raise Bad(400, "a folder can't go inside itself")
            if action == "move" and to == parent_of(path):
                return {"path": path}
            name = free_folder_name(to, base_of(path))
            new = join(to, name)
            if action == "move":
                shutil.move(full(path), full(new))
                move_prefix(index, path, new)
                save_json(INDEX, index)
            else:
                shutil.copytree(full(path), full(new))
            return {"path": new}
        raise Bad(404, "not found")
    # ---- bin
    if p[:1] == ["bin"]:
        bins = load_json(BIN_INDEX)
        present = set(os.listdir(BIN)) if os.path.isdir(BIN) else set()
        if len(p) == 1 and method == "GET":
            out = []
            for name in sorted(present):
                info = bins.get(name, {})
                st = os.stat(os.path.join(BIN, name))
                out.append({"name": name, "original": info.get("original", name), "deleted": info.get("deleted", int(st.st_mtime)), "folder": os.path.isdir(os.path.join(BIN, name))})
            out.sort(key=lambda x: -x["deleted"])
            return out
        if p[1:] == ["restore"] and method == "POST":
            name = q.get("name", "")
            if name not in present or "/" in name:
                raise Bad(404, "not in the bin")
            original = bins.get(name, {}).get("original", name)
            folder = parent_of(original)
            if not os.path.isdir(full(folder)):
                folder = ""
            src = os.path.join(BIN, name)
            if os.path.isdir(src):
                target = join(folder, free_folder_name(folder, base_of(original)))
                shutil.move(src, full(target))
            else:
                stem, ext = split_name(base_of(original))
                target = join(folder, claim_name(folder, stem, ext))
                os.replace(src, full(target))
            bins.pop(name, None)
            save_json(BIN_INDEX, bins)
            return {"path": target}
        if len(p) == 1 and method == "DELETE":
            names = [q["name"]] if "name" in q else sorted(present)
            for name in names:
                if name not in present or "/" in name:
                    continue
                src = os.path.join(BIN, name)
                shutil.rmtree(src) if os.path.isdir(src) else os.remove(src)
                bins.pop(name, None)
            save_json(BIN_INDEX, bins)
            return {"ok": True}
    raise Bad(404, "not found")


if __name__ == "__main__":
    os.makedirs(META, exist_ok=True)
    print(f"StarSheets files: {STORE} on :{PORT}, keeping {KEEP} versions", flush=True)
    if not AUTH:
        print("Login is OFF (STARSHEETS_AUTH=off): anyone who can reach this port can read and change every file.", flush=True)
    elif os.environ.get("STARSHEETS_PASSWORD"):
        print("Login is on, with the password from STARSHEETS_PASSWORD.", flush=True)
    else:
        print(f"Login is on. Password (set STARSHEETS_PASSWORD to choose your own): {password()}", flush=True)
    ThreadingHTTPServer(("0.0.0.0", PORT), Handler).serve_forever()
