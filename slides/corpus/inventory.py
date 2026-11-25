"""What a .pptx contains, as JSON (python-pptx plus a few raw-XML counts), and the difference
between two of them. Used by run.py; also runs on its own:

  python inventory.py A.pptx            print A's inventory
  python inventory.py A.pptx B.pptx     print what changed from A to B
"""

import difflib
import json
import re
import sys
import zipfile

from lxml import etree
from pptx import Presentation
from pptx.enum.shapes import MSO_SHAPE_TYPE

NS = {
    "p": "http://schemas.openxmlformats.org/presentationml/2006/main",
    "a": "http://schemas.openxmlformats.org/drawingml/2006/main",
    "mc": "http://schemas.openxmlformats.org/markup-compatibility/2006",
    "p14": "http://schemas.microsoft.com/office/powerpoint/2010/main",
    "dgm": "http://schemas.openxmlformats.org/drawingml/2006/diagram",
    "r": "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
}


def walk(shapes):
    for s in shapes:
        yield s
        if s.shape_type == MSO_SHAPE_TYPE.GROUP:
            yield from walk(s.shapes)


def run_format(r):
    f = r.font
    try:
        color = str(f.color.rgb) if f.color and f.color.type is not None and f.color.rgb is not None else ""
    except Exception:  # theme colours have no rgb
        color = "theme"
    return "".join(["b" if f.bold else "", "i" if f.italic else "", "u" if f.underline else ""]) + \
        f"/{f.size.pt if f.size else ''}/{f.name or ''}/{color}"


def xml_counts(root):
    """Counts read straight from a slide's XML. Transitions and effects are counted with the fallback
    branches of mc:AlternateContent left out (a feature written twice, new form + fallback, counts once).
    OLE objects and controls are counted in the whole XML, because their preview pictures often sit in
    the fallback branch."""
    full = lambda q: len(root.xpath(q, namespaces=NS))  # noqa: E731
    counts = {
        "controls": full("//p:control"),
        "control_previews": full("//p:control//a:blip"),
        "ole": full("//p:oleObj"),
        "ole_previews": full("//p:oleObj//a:blip"),
    }
    for fb in root.xpath("//mc:Fallback", namespaces=NS):
        fb.getparent().remove(fb)
    x = lambda q: len(root.xpath(q, namespaces=NS))  # noqa: E731
    tr = root.xpath("//p:transition", namespaces=NS)
    kinds = [etree.QName(c).localname for t in tr for c in t if etree.QName(c).localname not in ("sndAc", "extLst")]
    counts.update({
        "transition": kinds[0] if kinds else "",
        "effects": x("//p:timing//p:cTn[@presetClass]"),
        "media": x("//p14:media") + x("//a:videoFile") + x("//a:audioFile"),
        "smartart": x("//dgm:relIds"),
        "click_links": x("//a:hlinkClick"),
    })
    return counts


def inv(path):
    z = zipfile.ZipFile(path)
    names = z.namelist()
    d = {"parts": {
        "media": sorted(n.split("/")[-1].rsplit(".", 1)[-1].lower().replace("jpeg", "jpg") for n in names if n.startswith("ppt/media/")),
        "comments": sum(1 for n in names if re.match(r"ppt/comments/.*\.xml$", n)),
        "embeddings": sum(1 for n in names if n.startswith("ppt/embeddings/")),
        "charts": sum(1 for n in names if re.match(r"ppt/charts/chart\d+\.xml$", n)),
    }}
    pres = etree.fromstring(z.read("ppt/presentation.xml"))
    d["sections"] = [s.get("name") for s in pres.xpath("//p14:section", namespaces=NS)]
    try:
        p = Presentation(path)
    except Exception as e:
        d["error"] = f"python-pptx cannot open it: {type(e).__name__}: {str(e)[:120]}"
        return d
    d.update({"size": [p.slide_width, p.slide_height], "masters": len(p.slide_masters),
              "layouts": sum(len(m.slide_layouts) for m in p.slide_masters), "slides": []})
    for s in p.slides:
        sd = {"layout": s.slide_layout.name, "hidden": s._element.get("show") == "0", "text": [], "fmt": [], "pics": 0,
              "tables": [], "charts": [], "groups": 0, "connectors": 0, "autoshapes": [], "links": [], "crops": [], "rot": [],
              "notes": s.notes_slide.notes_text_frame.text if s.has_notes_slide and s.notes_slide.notes_text_frame else ""}
        for sh in walk(s.shapes):
            t = sh.shape_type
            if sh.has_text_frame and sh.text_frame.text.strip():
                sd["text"].append(sh.text_frame.text)
                for para in sh.text_frame.paragraphs:
                    for r in para.runs:
                        sd["fmt"].append(run_format(r))
                        try:
                            addr = r.hyperlink.address
                        except KeyError:  # the link points at a relationship that does not exist
                            addr = "(broken link)"
                        if addr:
                            sd["links"].append(addr)
            if t == MSO_SHAPE_TYPE.PICTURE or (t == MSO_SHAPE_TYPE.PLACEHOLDER and hasattr(sh, "image")):
                sd["pics"] += 1
                try:
                    sd["crops"].append([round(sh.crop_left, 2), round(sh.crop_top, 2)])
                except Exception:
                    pass
            if getattr(sh, "has_table", False) and sh.has_table:
                tb = sh.table
                sd["tables"].append({"rc": [len(tb.rows), len(tb.columns)], "cells": [c.text for r in tb.rows for c in r.cells],
                                     "merged": sum(1 for r in tb.rows for c in r.cells if c.is_merge_origin)})
            if getattr(sh, "has_chart", False) and sh.has_chart:
                try:
                    ch = sh.chart
                    sd["charts"].append({"type": str(ch.chart_type).split(" ")[0],
                                         "series": [[round(v, 6) if isinstance(v, float) else v for v in se.values] for pl in ch.plots for se in pl.series]})
                except Exception as e:
                    sd["charts"].append({"error": type(e).__name__})
            if t == MSO_SHAPE_TYPE.GROUP:
                sd["groups"] += 1
            if t == MSO_SHAPE_TYPE.LINE or sh.__class__.__name__ == "Connector":
                sd["connectors"] += 1
            if t == MSO_SHAPE_TYPE.AUTO_SHAPE:
                try:
                    sd["autoshapes"].append(str(sh.auto_shape_type).split(" ")[0])
                except Exception:
                    sd["autoshapes"].append("?")
            if getattr(sh, "rotation", 0):
                sd["rot"].append(round(sh.rotation))
        sd.update(xml_counts(etree.fromstring(etree.tostring(s._element))))
        d["slides"].append(sd)
    return d


def short(v):
    s = json.dumps(v, ensure_ascii=False)
    return s if len(s) <= 140 else s[:137] + "..."


def diff(a, b):
    """Differences from inventory a to inventory b, as stable one-line strings ("where key: old -> new")."""
    out = []
    for k in ("error", "size", "masters", "layouts", "sections"):
        if a.get(k) != b.get(k):
            out.append(f"{k}: {short(a.get(k))} -> {short(b.get(k))}")
    for k in a["parts"]:
        if a["parts"][k] != b["parts"].get(k):
            out.append(f"parts.{k}: {short(a['parts'][k])} -> {short(b['parts'].get(k))}")
    sa, sb = a.get("slides", []), b.get("slides", [])
    if len(sa) != len(sb):
        out.append(f"slides: {len(sa)} -> {len(sb)}")
    for i, (x, y) in enumerate(zip(sa, sb)):
        for k in x:
            xv, yv = x[k], y.get(k)
            if xv == yv:
                continue
            if isinstance(xv, list) and isinstance(yv, list) and len(xv) == len(yv) and len(short(xv)) > 60:
                # long lists of the same length: name only the items that changed
                for j, (p, q) in enumerate(zip(xv, yv)):
                    if p != q:
                        out.append(f"slide {i + 1} {k}[{j}]: {short(p)} -> {short(q)}")
            elif isinstance(xv, list) and isinstance(yv, list) and len(short(xv)) > 60:
                # long lists that grew or shrank: name the stretches that changed
                sm = difflib.SequenceMatcher(None, [short(v) for v in xv], [short(v) for v in yv], autojunk=False)
                for op, a1, a2, b1, b2 in sm.get_opcodes():
                    if op != "equal":
                        out.append(f"slide {i + 1} {k}[{a1}:{a2}]: {short(xv[a1:a2])} -> {short(yv[b1:b2])}")
            else:
                out.append(f"slide {i + 1} {k}: {short(xv)} -> {short(yv)}")
    return out


if __name__ == "__main__":
    if len(sys.argv) == 2:
        print(json.dumps(inv(sys.argv[1]), indent=1, ensure_ascii=False))
    else:
        print("\n".join(diff(inv(sys.argv[1]), inv(sys.argv[2]))) or "no differences")
