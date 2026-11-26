#!/usr/bin/env python3
# Last updated: 2026-11-26T12:05:00Z
"""Real-input proof of Photos > People (faces). Launch from anywhere:
  photos_people_input.py http://<host>:<port> --browser chromium|firefox --pics DIR_WITH_JPGS
      [--user NAME] [--name NAME_TO_GIVE] [--not-see NAME_ANOTHER_ACCOUNT_GAVE]
Pictures are named <person><n>.jpg (aldrin1.jpg ...): the person prefix is the ground truth for grouping quality.
Server must run with --features ml and ada must be the owner. Shots land in ./shots."""
import json, os, shlex, subprocess, sys, time, glob
sys.path.insert(0, os.path.expanduser("~/.claude/tools")); sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import starbase_ui as ui
RD = "/tmp/photos-people"

def launcher():
    a = sys.argv[1:]; pics = a[a.index("--pics") + 1]; rest = [x for x in a if x not in ("--pics", pics)]
    extra = {os.path.basename(f): open(f, "rb").read() for f in sorted(glob.glob(pics + "/*.jpg"))}
    d = os.path.join(os.getcwd(), "shots")
    sys.exit(ui.ship_and_run(os.path.abspath(__file__), rest, RD, pyname="p.py", extra=extra, shots_to=d))

def main():
    a = sys.argv[1:]; a.remove("--remote"); base = a[0]; br = a[a.index("--browser") + 1] if "--browser" in a else "chromium"
    res = []
    def rec(n, ok, d=""): res.append(ok); print(("PASS " if ok else "FAIL ") + n, d, flush=True)
    S = ui.Session(br, base=base); pg = S.pg; S.sign_in(a[a.index("--user") + 1] if "--user" in a else "ada")
    files = sorted(glob.glob("*.jpg"))
    folder = pg.request.post(f"{S.base}/api/folders?path={S.user}&name=People%20{br}{int(time.time())}").json()["path"]
    import urllib.parse
    q = lambda x: urllib.parse.quote(x, safe="")
    for f in files: pg.request.post(f"{S.base}/api/files?name={q(f)}&as_is=1&folder={q(folder)}", data=open(f, "rb").read())
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:enable"); time.sleep(8)
    S.hook("home:photos:tab", tab="people"); time.sleep(2)
    S.shot(f"{br}-0-start")
    pe = lambda: S.snapshot("home.photos.people") or {}
    print("people state", json.dumps(pe())[:300], flush=True)
    if pe().get("can_download"):
        S.click_text("Download face models")
        rec("models downloaded", S.wait_for(lambda: not pe().get("missing") and not (pe().get("download") or {}).get("busy"), secs=400), json.dumps(pe().get("download")))
    tg = lambda: S.snapshot("targets") or {}
    back = lambda: S.click_text("people:back")
    NAME = a[a.index("--name") + 1] if "--name" in a else "Sally Ride"; first = len(pe().get("named", []))
    if "Turn on" in tg(): S.click_text("Turn on")
    S.wait_for(lambda: pe().get("on") is True, secs=30)
    ok = S.wait_for(lambda: len(pe().get("named", [])) + len(pe().get("groups", [])) >= 4 and sum(g["count"] for g in pe()["groups"]) >= 8, secs=420, every=5)
    st = pe(); S.shot(f"{br}-1-people")
    print("groups", [g["count"] for g in st.get("groups", [])], flush=True)
    rec("faces indexed into groups", bool(ok), f"{len(st['groups'])} groups counts={[g['count'] for g in st['groups']]} for {len(set(f.split('1')[0].split('2')[0].split('3')[0] for f in files))} real people")
    if "--not-see" in a:  # privacy: another account's name for these faces must never show
        other = a[a.index("--not-see") + 1]
        rec("other account's name hidden", other not in json.dumps(st), json.dumps(st.get("named")))
    t = tg()
    rec("face circles drawn", "person:0" in t, str(sorted(k for k in t if k.startswith('person'))))
    # name person 0
    S.click_text(f"person:{first}"); time.sleep(2)
    S.click_text("people:title"); time.sleep(1)  # the name is the title: click it to type in place
    subprocess.run(["xdotool", "type", "--delay", "40", NAME]); subprocess.run(["xdotool", "key", "Return"]); time.sleep(2)
    back()
    rec("naming works", S.wait_for(lambda: any(n["name"] == NAME for n in pe().get("named", [])), secs=20), json.dumps(pe().get("named")))
    S.shot(f"{br}-2-named")
    # reload the page: the name must stick
    S.open("drive", path=f"/?test&dir={q(folder)}"); time.sleep(2)
    S.hook("home:photos", on=True); time.sleep(2); S.hook("home:photos:tab", tab="people"); time.sleep(3)
    rec("name sticks after reload", S.wait_for(lambda: any(n["name"] == NAME for n in pe().get("named", [])), secs=30), json.dumps(pe().get("named")))
    print("GROUPING", json.dumps({"named": pe().get("named"), "groups": pe().get("groups")})[:1500], flush=True)
    # open the named person: only their photos
    nm = pe()["named"]; k = [x["name"] for x in nm].index(NAME); named = nm[k]; S.click_text(f"person:{k}"); time.sleep(3)
    its = S.snapshot("home.photos.items") or []; n = len(its); print("PERSON0 ITEMS", json.dumps(its)[:600], flush=True)
    rec("person page shows only their photos", n == named["count"] and 1 <= n <= 4, f"items={n} count={named['count']}")
    S.shot(f"{br}-3-person")
    # remove one face
    f0 = S.snapshot("home.photos.people.faces")
    if f0 and f0 > 1:
        S.click_text("face:0"); time.sleep(3)
        rec("remove a face", (S.snapshot("home.photos.people.faces") or 99) == f0 - 1, f"{f0}->{S.snapshot('home.photos.people.faces')}")
    # merge: go back, open an unnamed group, merge into Ada Test
    back(); time.sleep(2)
    before = len(pe()["groups"]) + len(pe()["named"])
    if pe()["groups"]:
        S.click_text(f"person:{len(pe()['named'])}"); time.sleep(3)
        S.click_text("people:more"); time.sleep(1)  # Merge is in the More menu
        S.click_text("Merge with…"); time.sleep(1)
        S.click_text(NAME); time.sleep(3)
        back(); time.sleep(3)
        after = len(pe()["groups"]) + len(pe()["named"])
        rec("merge works", after == before - 1, f"{before}->{after}")
    S.shot(f"{br}-4-merged")
    sys.exit(0 if all(res) else 1)

if __name__ == "__main__":
    main() if "--remote" in sys.argv else launcher()
