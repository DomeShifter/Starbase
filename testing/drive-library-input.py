#!/usr/bin/env python3
"""Check row 54 with real browser input on a disposable CT105 test server.

Run: python3 testing/drive-library-input.py --testn 756 [--browser chromium|firefox]

The ada test account starts with the account folder as My Files and the separate
"library" grant as Photos. A second grant, "work", supplies My Files. The work
mount contains Child and the library mount contains PhotosOnly. Both grants
must point to real mounts. The script creates uniquely named text files, restores
the initial library choices, and leaves only those temporary files in the bin.
"""

import argparse
from pathlib import Path
import sys
from tempfile import TemporaryDirectory
from urllib.parse import urlencode
from uuid import uuid4

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def request(session, method, path, status, **kwargs):
    response = session.pg.request.fetch(session.base + "/api/" + path, method=method, **kwargs)
    assert response.status == status, f"{method} {path}: expected {status}, got {response.status}"
    return response


def roots(session):
    return request(session, "GET", "library-roots", 200).json()


def expect_view(session, path):
    assert session.wait_for(
        lambda: session.snapshot("home.view.mine") == path
        and session.snapshot("home.loading") is False
    ), f"Drive did not load {path}"


def file_names(session):
    return {file["name"] for file in (session.snapshot("home.files") or [])}


def select_my_files(session, mount):
    session.click_text("account:avatar", button=3)
    session.click_text("tab:Photos")
    session.click_text("settings:my-files-location")
    choice = f"settings:my-files-choice:{mount}" if mount else "settings:my-files-account"
    session.click_text(choice)
    session.click_text("settings:save-library")
    assert session.wait_for(lambda: roots(session)["selected"]["my_files"] == mount)
    session.key("Escape")


def upload(session, temp, name):
    path = temp / name
    path.write_text("Drive library browser test\n")
    session.click_text("New")
    session.upload(str(path), target="Upload files…")


def check(browser, testn):
    with Session(browser, testn=testn) as session, TemporaryDirectory() as directory:
        session.sign_in("ada")
        starting = roots(session)
        account = starting["roots"]["my_files"]
        work = starting["mounts"]["work"]
        library = starting["mounts"]["library"]
        child = f"{work}/Child"
        assert account == "ada", starting["roots"]
        assert starting["selected"] == {"my_files": None, "photos": {"mount": "library"}}
        assert starting["roots"]["photos"] == library and work != library
        prefix = f"row54-{uuid4().hex[:8]}"
        names = {label: f"{prefix}-{label}.txt" for label in ("outside", "child", "photos", "at-root", "at-child")}
        temp = Path(directory)

        try:
            for label, folder in (("outside", account), ("child", child), ("photos", f"{library}/PhotosOnly")):
                query = urlencode({"folder": folder, "name": names[label], "as_is": 1})
                request(session, "POST", f"files?{query}", 201, data=f"Fixture for {label}\n")

            session.open("drive")
            expect_view(session, account)
            assert session.wait_for(lambda: names["outside"] in file_names(session))

            select_my_files(session, "work")
            expect_view(session, work)
            assert roots(session)["roots"] == {"my_files": work, "photos": library}
            assert names["outside"] not in file_names(session)
            assert session.wait_text("Child"), "Child missing from the My files sidebar"
            assert "PhotosOnly" not in (session.snapshot("targets") or {})
            session.click_text("Child")
            expect_view(session, child)
            assert session.wait_for(lambda: names["child"] in file_names(session))

            upload(session, temp, names["at-child"])
            assert session.wait_for(lambda: names["at-child"] in file_names(session)), "Child upload missing without refresh"
            session.click_text("My files")
            expect_view(session, work)

            session.click_text("drive:search")
            session.pg.keyboard.type(prefix)
            assert session.wait_for(lambda: session.snapshot("home.search") == prefix)
            assert session.wait_for(lambda: file_names(session) == {names["child"], names["at-child"]}), file_names(session)

            session.key("ctrl+a", settle=0)
            session.key("BackSpace")
            assert session.wait_for(lambda: session.snapshot("home.search") == "")
            session.click_text("Recent")
            assert session.wait_for(lambda: session.snapshot("home.view") == "recent")
            upload(session, temp, names["at-root"])
            session.click_text("My files")
            expect_view(session, work)
            assert session.wait_for(lambda: names["at-root"] in file_names(session)), "Recent upload missing from My Files root"

            current = request(session, "GET", "files", 200).json()["files"]
            destinations = {file["name"]: file["folder"] for file in current}
            assert destinations[names["at-child"]] == child
            assert destinations[names["at-root"]] == work

            session.open("drive")
            expect_view(session, work)
            session.click_text("drive:photo_mode")
            assert session.wait_for(lambda: session.snapshot("home.photos.folder") == library), "Photos did not open its separate mount"

            session.open("drive")
            select_my_files(session, None)
            expect_view(session, account)
            assert session.wait_for(lambda: names["outside"] in file_names(session))
            assert names["at-root"] not in file_names(session)
            assert roots(session)["roots"]["photos"] == library
        finally:
            request(session, "PUT", "library-roots", 200, data=starting["selected"])
            files = request(session, "GET", "files", 200).json()["files"]
            for file in files:
                if file["name"] in names.values():
                    response = session.pg.request.delete(session.base + "/api/files/" + file["id"])
                    assert response.ok, f"Failed to clean up {file['name']}: HTTP {response.status}"

    print(f"PASS {browser}: My Files setting, default, sidebar, search and uploads. Photos kept its separate library.", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--browser", choices=("chromium", "firefox"))
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/drive-library-input", env="set -o pipefail &&"))
    for browser in (args.browser,) if args.browser else ("chromium", "firefox"):
        check(browser, args.testn)
