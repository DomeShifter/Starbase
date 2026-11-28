#!/usr/bin/env python3
"""Check separate Photos and My Files folder trees on a disposable CT105 server.

Run: python3 testing/photos-library-trees-input.py --testn 756 [--browser chromium|firefox]

The ada test account needs two real mount grants: work contains Child/DriveOnly.png,
and library contains PhotosOnly/LibraryOnly.png. Photos indexing must be enabled
on the library root. The script restores the account's initial library settings.
"""

import argparse
from pathlib import Path
import sys
from urllib.parse import parse_qs, urlsplit

sys.path.insert(0, str(Path.home() / ".claude/tools"))
from starbase_ui import Session, ship_and_run


def targets(session):
    return session.snapshot("targets") or {}


def library_settings(session, choices=None):
    url = session.base + "/api/library-roots"
    response = session.pg.request.get(url) if choices is None else session.pg.request.put(url, data=choices)
    assert response.ok, f"Library settings returned HTTP {response.status}"
    return response.json()


def open_photos(session, root):
    session.open("drive")
    entry = "drive:photo_mode"
    exit_button = "photo-exit"
    visible = session.wait_for(lambda: {entry, exit_button} & targets(session).keys())
    assert visible, f"Neither Drive nor Photos rendered its navigation: {sorted(targets(session))}"
    if exit_button not in visible:
        session.click_text(entry)
    expect_folder(session, root)
    assert session.wait_text("photo-exit")


def expect_folder(session, path):
    assert session.wait_for(lambda: session.snapshot("home.photos.folder") == path), f"Photos did not open {path}"


def photo_names(session):
    return {item["name"] for item in (session.snapshot("home.photos.items") or [])}


def expect_picture(session, name, excluded):
    assert session.wait_for(
        lambda: name in photo_names(session)
        and session.snapshot("home.photos.loading") is False,
        secs=60,
    ), f"Photos did not finish loading {name}"
    assert excluded not in photo_names(session), photo_names(session)


def check_tree(session, tab, key, root, child, other_child):
    root_row = f"photos:folder:{root}"
    child_row = f"photos:folder:{child}"
    other_row = f"photos:folder:{other_child}"
    hide = "photos:tree:hide"
    session.click_text(tab)
    assert session.wait_text(child_row)
    opened = targets(session)
    assert root_row in opened and hide in opened and other_row not in opened

    arrow = f"photos:{key}:{root}:toggle"
    session.click_text(arrow)
    assert session.wait_for(lambda: child_row not in targets(session)), "Root did not collapse"
    assert root_row in targets(session)
    session.click_text(arrow)
    assert session.wait_text(child_row), "Root did not expand"
    session.click_text(child_row)
    expect_folder(session, child)

    session.click_text(hide)
    assert session.wait_for(lambda: hide not in targets(session) and root_row not in targets(session))
    expect_folder(session, child)
    session.click_text(tab)
    assert session.wait_text(child_row), "Clicking the tab did not reopen the tree"
    expect_folder(session, child)


def open_info(session, name):
    items = session.snapshot("home.photos.items")
    index = next(index for index, item in enumerate(items) if item["name"] == name)
    session.click_text(f"photos:tile:{index}")
    assert session.wait_for(lambda: session.snapshot("home.photos.viewer.name") == name)
    if not session.snapshot("home.photos.viewer.info"):
        session.click_text("viewer:info")
    assert session.wait_for(lambda: session.snapshot("home.photos.viewer.info") is True)
    assert session.wait_for(
        lambda: dict(session.snapshot("home.photos.viewer.info_rows") or []).get("Name") == name
    ), f"Viewer Info did not show {name}"
    return items[index]["id"]


def check(browser, testn):
    with Session(browser, testn=testn) as session:
        session.sign_in("ada")
        initial = library_settings(session)
        work = initial["mounts"]["work"]
        library = initial["mounts"]["library"]
        work_child = f"{work}/Child"
        library_child = f"{library}/PhotosOnly"
        drive_picture = "DriveOnly.png"
        library_picture = "LibraryOnly.png"
        face_requests = []
        listings = []

        def record_photo_request(request):
            url = urlsplit(request.url)
            if url.path.startswith(("/api/photos/faces/", "/api/photos/people")):
                face_requests.append(url.path)
            elif url.path == "/api/photos":
                listings.append(parse_qs(url.query))

        session.pg.on("request", record_photo_request)
        try:
            selected = library_settings(session, {"my_files": "work", "photos": {"mount": "library"}})
            assert selected["roots"] == {"my_files": work, "photos": library}
            open_photos(session, library)

            check_tree(session, "My Files", "my-files", work, work_child, library_child)
            expect_picture(session, drive_picture, library_picture)
            drive_id = open_info(session, drive_picture)
            assert session.wait_text("viewer:info-panel")
            assert "photos:info-people" not in targets(session)
            assert session.snapshot("home.photos.viewer.people") is None
            session.key("Escape")
            assert session.wait_for(lambda: session.snapshot("home.photos.viewer.info") is False)
            session.key("Escape")
            assert session.wait_for(lambda: session.snapshot("home.photos.viewer") is None)
            assert not face_requests, face_requests

            check_tree(session, "Photos", "library", library, library_child, work_child)
            expect_picture(session, library_picture, drive_picture)
            before_everything = len(listings)
            session.click_text("photos:view-menu")
            session.click_text("Everything")
            assert session.wait_for(lambda: session.snapshot("home.photos.scope") == "Everything")
            assert session.wait_for(
                lambda: any(q.get("all") == ["1"] and "browse" not in q for q in listings[before_everything:])
            ), listings[before_everything:]
            if "Everything" in targets(session):
                session.key("Escape")
            expect_picture(session, library_picture, drive_picture)
            library_id = open_info(session, library_picture)
            assert session.wait_for(lambda: f"/api/photos/faces/{library_id}" in face_requests), face_requests
            assert f"/api/photos/faces/{drive_id}" not in face_requests, face_requests
            assert not any(path.startswith("/api/photos/people") for path in face_requests), face_requests

            for choices, root in (
                ({"my_files": "work", "photos": {"mount": "work"}}, work),
                ({"my_files": None, "photos": None}, "ada"),
            ):
                state = library_settings(session, choices)
                assert state["roots"] == {"my_files": root, "photos": root}
                open_photos(session, root)
                assert "Photos" in targets(session)
                assert "My Files" not in targets(session), "My Files appeared for an identical Photos root"
        finally:
            library_settings(session, initial["selected"])

    print(f"PASS {browser}: separate Photos and My Files trees, scoped Everything, and People requests only in the library.", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--testn", type=int, required=True)
    parser.add_argument("--browser", choices=("chromium", "firefox"))
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    if not args.remote:
        sys.exit(ship_and_run(__file__, sys.argv[1:], "/tmp/photos-library-trees-input", env="set -o pipefail &&"))
    for browser in (args.browser,) if args.browser else ("chromium", "firefox"):
        check(browser, args.testn)
