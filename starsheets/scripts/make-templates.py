#!/usr/bin/env python3
"""Writes the starter templates (crates/templates/files/*) as small, plain Office files.

Original content only. Run from anywhere: python3 starsheets/scripts/make-templates.py
The output is deterministic (fixed zip dates), so re-running changes nothing unless this does.
"""
import os
import zipfile
from xml.sax.saxutils import escape

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "templates", "files")
XML = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
REL = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PKG = "http://schemas.openxmlformats.org/package/2006/relationships"
CT = "http://schemas.openxmlformats.org/package/2006/content-types"
RELS_CT = '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'


def write(name, parts):
    with zipfile.ZipFile(os.path.join(OUT, name), "w", zipfile.ZIP_DEFLATED) as z:
        for path, text in parts:
            info = zipfile.ZipInfo(path, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, XML + text)


def rels(items):
    body = "".join(f'<Relationship Id="rId{i + 1}" Type="{t}" Target="{tg}"/>' for i, (t, tg) in enumerate(items))
    return f'<Relationships xmlns="{PKG}">{body}</Relationships>'


# ------------------------------------------------------------------ documents

W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"


def para(text, size=22, bold=False, before=0, after=120, colour=None):
    rpr = ("<w:b/>" if bold else "") + (f'<w:color w:val="{colour}"/>' if colour else "") + f'<w:sz w:val="{size}"/>'
    run = f'<w:r><w:rPr>{rpr}</w:rPr><w:t xml:space="preserve">{escape(text)}</w:t></w:r>' if text else ""
    return f'<w:p><w:pPr><w:spacing w:before="{before}" w:after="{after}"/></w:pPr>{run}</w:p>'


def title(t):
    return para(t, size=40, bold=True, after=80)


def heading(t):
    return para(t, size=28, bold=True, before=240, after=80, colour="1F4E79")


def docx(name, paras):
    body = "".join(paras)
    sect = ('<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" '
            'w:left="1134" w:header="567" w:footer="567" w:gutter="0"/></w:sectPr>')
    write(name, [
        ("[Content_Types].xml", f'<Types xmlns="{CT}">{RELS_CT}<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>'),
        ("_rels/.rels", rels([(f"{REL}/officeDocument", "word/document.xml")])),
        ("word/document.xml", f'<w:document xmlns:w="{W}"><w:body>{body}{sect}</w:body></w:document>'),
    ])


docx("cv.docx", [
    title("Your Name"),
    para("Town, Country  ·  you@example.com  ·  01234 567890", colour="595959"),
    heading("Profile"),
    para("Two or three sentences about who you are, what you are good at and the kind of work you are looking for."),
    heading("Experience"),
    para("Job title, Organisation", bold=True, after=40),
    para("Month Year – Present", colour="595959", after=60),
    para("What you were responsible for, and one result you are proud of, with a number if you can."),
    para("Job title, Organisation", bold=True, after=40),
    para("Month Year – Month Year", colour="595959", after=60),
    para("What you did there and what changed because of it."),
    heading("Education"),
    para("Qualification, Place of study", bold=True, after=40),
    para("Year – Year", colour="595959"),
    heading("Skills"),
    para("Skill one  ·  Skill two  ·  Skill three  ·  Languages"),
])

docx("letter.docx", [
    para("Your Name"),
    para("Your street, Town, Postcode", after=360),
    para("Their Name"),
    para("Their organisation"),
    para("Their street, Town, Postcode", after=360),
    para("1 January 2026", after=360),
    para("Dear Their Name,"),
    para("Subject of the letter", bold=True),
    para("Say why you are writing in the first sentence. Keep each paragraph to one point."),
    para("Give the details they need: dates, amounts, reference numbers."),
    para("End with what you would like them to do next, and by when.", after=360),
    para("Yours sincerely,", after=720),
    para("Your Name"),
])

docx("meeting-notes.docx", [
    title("Meeting notes"),
    para("Date: 1 January 2026   ·   Time: 10:00   ·   Place: room or video link", colour="595959"),
    para("Attendees: Name, Name, Name", colour="595959"),
    heading("Agenda"),
    para("1. First topic"),
    para("2. Second topic"),
    para("3. Any other business"),
    heading("Notes"),
    para("What was discussed and agreed, one short paragraph per topic."),
    heading("Action items"),
    para("☐ Who – what – by when"),
    para("☐ Who – what – by when"),
    heading("Next meeting"),
    para("Date and time"),
])

# ------------------------------------------------------------------ spreadsheets

S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main"


def xlsx(name, sheet, rows, widths):
    strings, index = [], {}

    def sid(t):
        if t not in index:
            index[t] = len(strings)
            strings.append(t)
        return index[t]

    xml_rows = []
    for r, row in enumerate(rows, start=1):
        cells = []
        for c, v in enumerate(row):
            ref = f"{chr(ord('A') + c)}{r}"
            bold = r == 1 or (isinstance(v, tuple) and v[0] == "b")
            if isinstance(v, tuple):
                v = v[1]
            s = ' s="1"' if bold else ""
            if v is None:
                continue
            if isinstance(v, str) and v.startswith("="):
                cells.append(f'<c r="{ref}"{s}><f>{escape(v[1:])}</f></c>')
            elif isinstance(v, (int, float)):
                cells.append(f'<c r="{ref}"{s}><v>{v}</v></c>')
            else:
                cells.append(f'<c r="{ref}"{s} t="s"><v>{sid(v)}</v></c>')
        xml_rows.append(f'<row r="{r}">{"".join(cells)}</row>')
    cols = "".join(f'<col min="{i + 1}" max="{i + 1}" width="{w}" customWidth="1"/>' for i, w in enumerate(widths))
    sst = "".join(f"<si><t>{escape(t)}</t></si>" for t in strings)
    sml = "application/vnd.openxmlformats-officedocument.spreadsheetml"
    styles = (f'<styleSheet xmlns="{S}"><fonts count="2"><font><sz val="11"/><name val="Calibri"/></font>'
              '<font><b/><sz val="11"/><name val="Calibri"/></font></fonts>'
              '<fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills>'
              '<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>'
              '<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>'
              '<cellXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>'
              '<xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/></cellXfs></styleSheet>')
    write(name, [
        ("[Content_Types].xml", f'<Types xmlns="{CT}">{RELS_CT}'
                                f'<Override PartName="/xl/workbook.xml" ContentType="{sml}.sheet.main+xml"/>'
                                f'<Override PartName="/xl/worksheets/sheet1.xml" ContentType="{sml}.worksheet+xml"/>'
                                f'<Override PartName="/xl/styles.xml" ContentType="{sml}.styles+xml"/>'
                                f'<Override PartName="/xl/sharedStrings.xml" ContentType="{sml}.sharedStrings+xml"/></Types>'),
        ("_rels/.rels", rels([(f"{REL}/officeDocument", "xl/workbook.xml")])),
        ("xl/workbook.xml", f'<workbook xmlns="{S}" xmlns:r="{REL}"><sheets><sheet name="{escape(sheet)}" sheetId="1" r:id="rId1"/></sheets></workbook>'),
        ("xl/_rels/workbook.xml.rels", rels([(f"{REL}/worksheet", "worksheets/sheet1.xml"), (f"{REL}/styles", "styles.xml"), (f"{REL}/sharedStrings", "sharedStrings.xml")])),
        ("xl/worksheets/sheet1.xml", f'<worksheet xmlns="{S}"><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>'
                                     f'<cols>{cols}</cols><sheetData>{"".join(xml_rows)}</sheetData></worksheet>'),
        ("xl/styles.xml", styles),
        ("xl/sharedStrings.xml", f'<sst xmlns="{S}" count="{len(strings)}" uniqueCount="{len(strings)}">{sst}</sst>'),
    ])


budget = [["Item", "Planned", "Actual", "Difference"], [("b", "Income")]]
for item, plan in [("Pay", 2000), ("Other income", 0)]:
    r = len(budget) + 1
    budget.append([item, plan, plan, f"=C{r}-B{r}"])
inc_end = len(budget)
budget.append([("b", "Total income"), f"=SUM(B3:B{inc_end})", f"=SUM(C3:C{inc_end})", f"=C{inc_end + 1}-B{inc_end + 1}"])
inc_total = len(budget)
budget.append([("b", "Spending")])
first = len(budget) + 1
for item, plan in [("Rent or mortgage", 800), ("Bills", 200), ("Food", 300), ("Travel", 120),
                   ("Phone and internet", 50), ("Savings", 200), ("Fun", 100), ("Other", 50)]:
    r = len(budget) + 1
    budget.append([item, plan, 0, f"=B{r}-C{r}"])
last = len(budget)
budget.append([("b", "Total spending"), f"=SUM(B{first}:B{last})", f"=SUM(C{first}:C{last})", f"=B{last + 1}-C{last + 1}"])
spend_total = len(budget)
budget.append([])
budget.append([("b", "Left over"), f"=B{inc_total}-B{spend_total}", f"=C{inc_total}-C{spend_total}"])
xlsx("monthly-budget.xlsx", "Budget", budget, [24, 12, 12, 12])

todo = [["Task", "Who", "Due", "Status", "Notes"]]
for t, s in [("First task", "Not started"), ("Second task", "In progress"), ("Third task", "Done")]:
    todo.append([t, "Name", "2026-01-31", s])
end = len(todo)
todo.append([])
todo.append([("b", "Done"), f'=COUNTIF(D2:D{end},"Done")&" of "&COUNTA(A2:A{end})'])
xlsx("todo-tracker.xlsx", "To-do", todo, [32, 14, 12, 14, 32])

# ------------------------------------------------------------------ presentation

A = "http://schemas.openxmlformats.org/drawingml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"
NS = f'xmlns:a="{A}" xmlns:r="{REL}" xmlns:p="{P}"'
EMPTY_TREE = '<p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/></p:spTree>'


def box(i, x, y, w, h, lines, size, bold=False, colour="202124"):
    b = ' b="1"' if bold else ""
    paras = "".join(f'<a:p><a:r><a:rPr lang="en-GB" sz="{size}"{b}><a:solidFill><a:srgbClr val="{colour}"/></a:solidFill></a:rPr>'
                    f'<a:t>{escape(t)}</a:t></a:r></a:p>' for t in lines)
    return (f'<p:sp><p:nvSpPr><p:cNvPr id="{i}" name="Text {i}"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr>'
            f'<p:spPr><a:xfrm><a:off x="{x}" y="{y}"/><a:ext cx="{w}" cy="{h}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:noFill/></p:spPr>'
            f'<p:txBody><a:bodyPr wrap="square"/><a:lstStyle/>{paras}</p:txBody></p:sp>')


def slide(shapes):
    tree = EMPTY_TREE.replace("</p:spTree>", "".join(shapes) + "</p:spTree>")
    return f'<p:sld {NS}><p:cSld>{tree}</p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>'


CX, CY = 12192000, 6858000
slides = [
    slide([box(2, 914400, 2286000, 10363200, 1143000, ["Presentation title"], 4400, True),
           box(3, 914400, 3505200, 10363200, 685800, ["Your name  ·  Date"], 2000, colour="5F6368")]),
    slide([box(2, 609600, 457200, 10972800, 914400, ["Why this matters"], 3200, True),
           box(3, 609600, 1524000, 10972800, 4572000, ["• The problem in one line", "• Who it affects", "• What happens if nothing changes"], 2400)]),
    slide([box(2, 609600, 457200, 10972800, 914400, ["What we propose"], 3200, True),
           box(3, 609600, 1524000, 10972800, 4572000, ["• The idea", "• What it costs", "• When it can be ready"], 2400)]),
    slide([box(2, 914400, 2743200, 10363200, 1143000, ["Questions?"], 4400, True)]),
]
solid = '<a:solidFill><a:schemeClr val="phClr"/></a:solidFill>'
theme = (f'<a:theme xmlns:a="{A}" name="Plain"><a:themeElements><a:clrScheme name="Plain">'
         '<a:dk1><a:srgbClr val="000000"/></a:dk1><a:lt1><a:srgbClr val="FFFFFF"/></a:lt1>'
         '<a:dk2><a:srgbClr val="202124"/></a:dk2><a:lt2><a:srgbClr val="F1F3F4"/></a:lt2>'
         '<a:accent1><a:srgbClr val="1A73E8"/></a:accent1><a:accent2><a:srgbClr val="34A853"/></a:accent2>'
         '<a:accent3><a:srgbClr val="FBBC04"/></a:accent3><a:accent4><a:srgbClr val="EA4335"/></a:accent4>'
         '<a:accent5><a:srgbClr val="5F6368"/></a:accent5><a:accent6><a:srgbClr val="9334E6"/></a:accent6>'
         '<a:hlink><a:srgbClr val="1A73E8"/></a:hlink><a:folHlink><a:srgbClr val="9334E6"/></a:folHlink></a:clrScheme>'
         '<a:fontScheme name="Plain"><a:majorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:majorFont>'
         '<a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/></a:minorFont></a:fontScheme>'
         '<a:fmtScheme name="Plain"><a:fillStyleLst>' + solid * 3 + '</a:fillStyleLst>'
         '<a:lnStyleLst>' + f'<a:ln w="9525">{solid}</a:ln>' * 3 + '</a:lnStyleLst>'
         '<a:effectStyleLst>' + '<a:effectStyle><a:effectLst/></a:effectStyle>' * 3 + '</a:effectStyleLst>'
         '<a:bgFillStyleLst>' + solid * 3 + '</a:bgFillStyleLst></a:fmtScheme>'
         '</a:themeElements></a:theme>')
master = (f'<p:sldMaster {NS}><p:cSld>{EMPTY_TREE}</p:cSld>'
          '<p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" '
          'accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/>'
          '<p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/></p:sldLayoutIdLst></p:sldMaster>')
layout = (f'<p:sldLayout {NS} type="blank" preserve="1"><p:cSld name="Blank">{EMPTY_TREE}</p:cSld>'
          '<p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>')
n = len(slides)
pres = (f'<p:presentation {NS}><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst>'
        '<p:sldIdLst>' + "".join(f'<p:sldId id="{256 + i}" r:id="rId{i + 3}"/>' for i in range(n)) + '</p:sldIdLst>'
        f'<p:sldSz cx="{CX}" cy="{CY}"/><p:notesSz cx="{CY}" cy="{CX}"/></p:presentation>')
pml = "application/vnd.openxmlformats-officedocument.presentationml"
overrides = (f'<Override PartName="/ppt/presentation.xml" ContentType="{pml}.presentation.main+xml"/>'
             f'<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="{pml}.slideMaster+xml"/>'
             f'<Override PartName="/ppt/slideLayouts/slideLayout1.xml" ContentType="{pml}.slideLayout+xml"/>'
             '<Override PartName="/ppt/theme/theme1.xml" ContentType="application/vnd.openxmlformats-officedocument.theme+xml"/>'
             + "".join(f'<Override PartName="/ppt/slides/slide{i + 1}.xml" ContentType="{pml}.slide+xml"/>' for i in range(n)))
to_layout = rels([(f"{REL}/slideLayout", "../slideLayouts/slideLayout1.xml")])
parts = [
    ("[Content_Types].xml", f'<Types xmlns="{CT}">{RELS_CT}{overrides}</Types>'),
    ("_rels/.rels", rels([(f"{REL}/officeDocument", "ppt/presentation.xml")])),
    ("ppt/presentation.xml", pres),
    ("ppt/_rels/presentation.xml.rels", rels([(f"{REL}/slideMaster", "slideMasters/slideMaster1.xml"), (f"{REL}/theme", "theme/theme1.xml")]
                                             + [(f"{REL}/slide", f"slides/slide{i + 1}.xml") for i in range(n)])),
    ("ppt/slideMasters/slideMaster1.xml", master),
    ("ppt/slideMasters/_rels/slideMaster1.xml.rels", rels([(f"{REL}/slideLayout", "../slideLayouts/slideLayout1.xml"), (f"{REL}/theme", "../theme/theme1.xml")])),
    ("ppt/slideLayouts/slideLayout1.xml", layout),
    ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", rels([(f"{REL}/slideMaster", "../slideMasters/slideMaster1.xml")])),
    ("ppt/theme/theme1.xml", theme),
]
for i, s in enumerate(slides):
    parts.append((f"ppt/slides/slide{i + 1}.xml", s))
    parts.append((f"ppt/slides/_rels/slide{i + 1}.xml.rels", to_layout))
write("plain-deck.pptx", parts)
print("wrote", sorted(os.listdir(OUT)))
