"""Make the generated part of the Slides test corpus with python-pptx: one deck per feature area.
All content is made up here (text, numbers, pictures drawn with Pillow), so the decks carry no
third-party material. usage: python make_decks.py OUT_DIR"""
import copy, io, sys
from lxml import etree
from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.chart.data import BubbleChartData, CategoryChartData, XyChartData
from pptx.enum.chart import XL_CHART_TYPE, XL_LEGEND_POSITION, XL_LABEL_POSITION
from pptx.enum.shapes import MSO_SHAPE, MSO_CONNECTOR
from pptx.enum.text import MSO_ANCHOR, MSO_AUTO_SIZE, PP_ALIGN
from pptx.enum.action import PP_ACTION
from pptx.enum.dml import MSO_LINE_DASH_STYLE as DASH, MSO_PATTERN
from pptx.oxml.ns import qn
from PIL import Image, ImageDraw

out = sys.argv[1]


def png(w, h, seed):
    im = Image.new("RGB", (w, h), (30 + seed * 40, 90, 160))
    d = ImageDraw.Draw(im)
    for i in range(0, w, 40):
        d.line([(i, 0), (w - i, h)], fill=(255, 200 - seed * 30, 60), width=4)
    d.ellipse([w // 4, h // 4, 3 * w // 4, 3 * h // 4], outline=(255, 255, 255), width=8)
    b = io.BytesIO(); im.save(b, "PNG"); b.seek(0); return b


def notes(s, text):
    s.notes_slide.notes_text_frame.text = text


# 1. Text-heavy quarterly review: all layouts, nested bullets, run formatting, notes, hyperlink
p = Presentation(); p.slide_width, p.slide_height = Inches(13.333), Inches(7.5)
s = p.slides.add_slide(p.slide_layouts[0])
s.shapes.title.text = "Q3 Business Review"; s.placeholders[1].text = "Operations team · October 2026"
notes(s, "Welcome everyone. Keep this to 2 minutes.")
s = p.slides.add_slide(p.slide_layouts[1]); s.shapes.title.text = "Highlights"
tf = s.placeholders[1].text_frame; tf.text = "Revenue up 12% year on year"
for lvl, t in [(1, "Driven by EMEA renewals"), (2, "Three new enterprise logos"), (0, "Churn down to 3.1%"), (1, "Support response under 2 h")]:
    para = tf.add_paragraph(); para.text = t; para.level = lvl
r = tf.paragraphs[0].runs[0]; r.font.bold = True; r.font.color.rgb = RGBColor(0xC0, 0x30, 0x30)
para = tf.add_paragraph(); run = para.add_run(); run.text = "Details on the intranet"; run.hyperlink.address = "https://example.com/q3"
run = para.add_run(); run.text = " (italic, 14 pt)"; run.font.italic = True; run.font.size = Pt(14)
notes(s, "Mention EMEA specifically.\nSecond line of notes.")
for i, lay in enumerate(p.slide_layouts):
    if i < 2: continue
    s = p.slides.add_slide(lay)
    if s.shapes.title is not None: s.shapes.title.text = f"Layout {i}: {lay.name}"
    for ph in s.placeholders:
        if ph.placeholder_format.idx != 0 and ph.has_text_frame:
            ph.text_frame.text = f"Body text in placeholder {ph.placeholder_format.idx}"
    notes(s, f"Notes for layout {lay.name}")
tb = p.slides[1].shapes.add_textbox(Inches(8), Inches(6), Inches(4), Inches(1))
tb.text_frame.text = "Right-aligned footnote"; tb.text_frame.paragraphs[0].alignment = PP_ALIGN.RIGHT
p.save(f"{out}/gen-01-text-review.pptx")

# 2. Photo deck: full-bleed, cropped, scaled pictures, captions
p = Presentation()
s = p.slides.add_slide(p.slide_layouts[6])
s.shapes.add_picture(png(1600, 900, 0), 0, 0, p.slide_width, p.slide_height)
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Cropped and scaled"
pic = s.shapes.add_picture(png(800, 800, 1), Inches(0.5), Inches(1.8), Inches(4))
pic.crop_left = 0.2; pic.crop_right = 0.1; pic.crop_top = 0.15
pic2 = s.shapes.add_picture(png(400, 300, 2), Inches(5), Inches(1.8), Inches(4), Inches(3)); pic2.rotation = 8
tb = s.shapes.add_textbox(Inches(5), Inches(5.2), Inches(4), Inches(0.6)); tb.text_frame.text = "Rotated photo, caption below"
s = p.slides.add_slide(p.slide_layouts[8]) if len(p.slide_layouts) > 8 else p.slides.add_slide(p.slide_layouts[1])
s.shapes.title.text = "Picture placeholder layout"
for ph in s.placeholders:
    if ph.placeholder_format.type is not None and "PICTURE" in str(ph.placeholder_format.type):
        ph.insert_picture(png(1200, 800, 3))
    elif ph.placeholder_format.idx != 0 and ph.has_text_frame:
        ph.text_frame.text = "Caption text"
notes(s, "Photo credits: generated test images")
p.save(f"{out}/gen-02-photos.pptx")

# 3. Tables: styled, merged cells, fills, many rows
p = Presentation()
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Budget by quarter"
rows, cols = 6, 5
t = s.shapes.add_table(rows, cols, Inches(0.5), Inches(1.5), Inches(9), Inches(3.5)).table
hdr = ["Item", "Q1", "Q2", "Q3", "Q4"]
for c, h in enumerate(hdr): t.cell(0, c).text = h
for r in range(1, rows):
    t.cell(r, 0).text = f"Line {r}"
    for c in range(1, cols): t.cell(r, c).text = f"{r * c * 1000:,}"
t.cell(5, 0).text = "Total"; t.cell(5, 0).merge(t.cell(5, 1))
c = t.cell(2, 2); c.fill.solid(); c.fill.fore_color.rgb = RGBColor(0xFF, 0xE0, 0x80)
c.text_frame.paragraphs[0].runs[0].font.bold = True
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Long table (20 rows)"
t = s.shapes.add_table(20, 3, Inches(0.5), Inches(1.4), Inches(9), Inches(5.5)).table
for r in range(20):
    for cc in range(3): t.cell(r, cc).text = f"r{r}c{cc}"
t.cell(0, 0).merge(t.cell(1, 0))
notes(s, "Table notes")
p.save(f"{out}/gen-03-tables.pptx")

# 4. Charts: clustered bar, line with markers, pie with labels, XY scatter, stacked area
p = Presentation()
cd = CategoryChartData(); cd.categories = ["Q1", "Q2", "Q3", "Q4"]
cd.add_series("North", (19.2, 21.4, 16.7, 25.0)); cd.add_series("South", (22.3, 28.6, 15.2, 18.1))
for kind, title in [(XL_CHART_TYPE.COLUMN_CLUSTERED, "Column"), (XL_CHART_TYPE.LINE_MARKERS, "Line"),
                    (XL_CHART_TYPE.AREA_STACKED, "Stacked area"), (XL_CHART_TYPE.BAR_CLUSTERED, "Bar")]:
    s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = f"{title} chart"
    ch = s.shapes.add_chart(kind, Inches(0.5), Inches(1.5), Inches(9), Inches(5), cd).chart
    ch.has_legend = True; ch.legend.position = XL_LEGEND_POSITION.BOTTOM; ch.legend.include_in_layout = False
pc = CategoryChartData(); pc.categories = ["Rent", "Staff", "Cloud", "Other"]; pc.add_series("Spend", (40, 35, 15, 10))
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Pie chart"
ch = s.shapes.add_chart(XL_CHART_TYPE.PIE, Inches(1.5), Inches(1.5), Inches(7), Inches(5), pc).chart
ch.plots[0].has_data_labels = True; ch.plots[0].data_labels.number_format = '0"%"'
xy = XyChartData(); se = xy.add_series("Samples")
for i in range(10): se.add_data_point(i, i * i / 3)
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Scatter"
s.shapes.add_chart(XL_CHART_TYPE.XY_SCATTER, Inches(0.5), Inches(1.5), Inches(9), Inches(5), xy)
notes(s, "Chart notes")
p.save(f"{out}/gen-04-charts.pptx")

# 5. Diagram / shapes deck: autoshapes, fills, lines, connectors, groups, rotation, z-order
p = Presentation()
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Process flow"
boxes = []
for i, name in enumerate(["Plan", "Build", "Test", "Ship"]):
    sh = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(0.4 + i * 2.4), Inches(3), Inches(1.8), Inches(1))
    sh.text = name; sh.fill.solid(); sh.fill.fore_color.rgb = RGBColor(0x20 + i * 40, 0x80, 0xC0)
    sh.line.color.rgb = RGBColor(0, 0, 0); sh.line.width = Pt(2); boxes.append(sh)
for a, b in zip(boxes, boxes[1:]):
    cx = s.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, 0, 0, 0, 0); cx.begin_connect(a, 3); cx.end_connect(b, 1)
s = p.slides.add_slide(p.slide_layouts[6])
grp = s.shapes.add_group_shape()
for i, kind in enumerate([MSO_SHAPE.OVAL, MSO_SHAPE.STAR_5_POINT, MSO_SHAPE.RIGHT_ARROW, MSO_SHAPE.HEXAGON, MSO_SHAPE.CLOUD]):
    sh = grp.shapes.add_shape(kind, Inches(0.5 + i * 1.8), Inches(1), Inches(1.5), Inches(1.5)); sh.text = str(i + 1)
tri = s.shapes.add_shape(MSO_SHAPE.ISOSCELES_TRIANGLE, Inches(3), Inches(4), Inches(3), Inches(2)); tri.rotation = 30
tri.fill.background(); tri.line.dash_style = DASH.DASH
ln = s.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(0.5), Inches(6.8), Inches(9.5), Inches(6.8)); ln.line.width = Pt(4)
notes(s, "Group of five shapes + rotated dashed triangle")
p.save(f"{out}/gen-05-shapes.pptx")

# (6, lo-resaved, is deck 1 saved again by LibreOffice: run.py makes it, as it needs soffice.)

P_NS = "http://schemas.openxmlformats.org/presentationml/2006/main"
A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"
P14 = "http://schemas.microsoft.com/office/powerpoint/2010/main"


def xml(s):
    return etree.fromstring(f'<r xmlns:p="{P_NS}" xmlns:a="{A_NS}" xmlns:p14="{P14}">{s}</r>')[0]


def rpr_attr(run, **kv):
    rpr = run._r.get_or_add_rPr()
    for k, v in kv.items():
        rpr.set(k, str(v))
    return rpr


# 7. Text formatting: fonts, sizes, styles, spacing, alignment, autofit, wrapping, scripts
p = Presentation(); p.slide_width, p.slide_height = Inches(13.333), Inches(7.5)
s = p.slides.add_slide(p.slide_layouts[6])
tb = s.shapes.add_textbox(Inches(0.5), Inches(0.4), Inches(12), Inches(3.2)); tf = tb.text_frame; tf.word_wrap = True
para = tf.paragraphs[0]
for text, kw in [("Bold ", {"bold": True}), ("italic ", {"italic": True}), ("underline ", {"underline": True}),
                 ("serif 28 ", {"name": "Liberation Serif", "size": Pt(28)}), ("mono 14 ", {"name": "DejaVu Sans Mono", "size": Pt(14)}),
                 ("coloured", {"color": RGBColor(0x1F, 0x77, 0xB4)})]:
    r = para.add_run(); r.text = text
    for k, v in kw.items():
        if k == "color": r.font.color.rgb = v
        else: setattr(r.font, k, v)
para = tf.add_paragraph()
r = para.add_run(); r.text = "x"; r = para.add_run(); r.text = "2"; rpr_attr(r, baseline=30000)
r = para.add_run(); r.text = " H"; r = para.add_run(); r.text = "2"; rpr_attr(r, baseline=-25000)
r = para.add_run(); r.text = "O  strike  "; rpr_attr(r, strike="sngStrike")
r = para.add_run(); r.text = "wide spacing  "; rpr_attr(r, spc=600)
r = para.add_run(); r.text = "SMALL CAPS  "; rpr_attr(r, cap="small")
r = para.add_run(); r.text = "highlighted"; r.font.color.rgb = RGBColor(0, 0, 0)
r._r.get_or_add_rPr().append(xml('<a:highlight><a:srgbClr val="FFFF00"/></a:highlight>'))
for align, name in [(PP_ALIGN.LEFT, "left"), (PP_ALIGN.CENTER, "centred"), (PP_ALIGN.RIGHT, "right"), (PP_ALIGN.JUSTIFY, "justified " * 12)]:
    para = tf.add_paragraph(); para.text = f"Paragraph aligned {name}"; para.alignment = align
    para.space_before = Pt(6); para.line_spacing = 1.2
for i, (auto, wrap, anchor) in enumerate([(MSO_AUTO_SIZE.TEXT_TO_FIT_SHAPE, True, MSO_ANCHOR.TOP),
                                         (MSO_AUTO_SIZE.SHAPE_TO_FIT_TEXT, True, MSO_ANCHOR.MIDDLE),
                                         (MSO_AUTO_SIZE.NONE, False, MSO_ANCHOR.BOTTOM)]):
    sh = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0.5 + i * 4.2), Inches(4), Inches(3.8), Inches(2.6))
    sh.fill.background(); sh.line.color.rgb = RGBColor(0x80, 0x80, 0x80)
    t = sh.text_frame; t.text = f"Box {i + 1}: " + "a fairly long line of text that needs wrapping or fitting " * 3
    t.auto_size = auto; t.word_wrap = wrap; t.vertical_anchor = anchor
    t.margin_left = t.margin_right = Inches(0.05 + i * 0.15)
    for pa in t.paragraphs:
        for rr in pa.runs: rr.font.size = Pt(18); rr.font.color.rgb = RGBColor(0x20, 0x20, 0x20)
notes(s, "Run formatting, paragraph alignment, autofit, wrap and anchoring")
s = p.slides.add_slide(p.slide_layouts[1]); s.shapes.title.text = "Scripts and languages"
tf = s.placeholders[1].text_frame
lines = ["English: The quick brown fox", "Deutsch: Größenänderung über Übermaß", "Ελληνικά: Γρήγορη καφέ αλεπού",
         "日本語：素早い茶色の狐", "中文：敏捷的棕色狐狸", "한국어: 빠른 갈색 여우", "العربية: الثعلب البني السريع",
         "עברית: השועל החום המהיר", "Emoji: 🎉 ✅ 🚀", "Combining: é ä ñ"]
tf.text = lines[0]
for ln_ in lines[1:]:
    para = tf.add_paragraph(); para.text = ln_
    if ln_.startswith(("العربية", "עברית")):
        para._p.get_or_add_pPr().set("rtl", "1"); para.alignment = PP_ALIGN.RIGHT
notes(s, "Mixed scripts, right-to-left paragraphs, emoji, combining marks")
p.save(f"{out}/gen-07-text-formatting.pptx")

# 8. A long deck: 60 slides of bullets with notes (size and speed)
p = Presentation()
for i in range(60):
    s = p.slides.add_slide(p.slide_layouts[1]); s.shapes.title.text = f"Section {i // 10 + 1}, slide {i + 1}"
    tf = s.placeholders[1].text_frame; tf.text = f"Point one of slide {i + 1}"
    for j in range(2, 6):
        para = tf.add_paragraph(); para.text = f"Point {j}: detail {i * j}"; para.level = j % 3
    notes(s, f"Speaker notes for slide {i + 1}. " * 3)
p.save(f"{out}/gen-08-many-slides.pptx")

# 9. Fills and lines: slide backgrounds, gradients, patterns, transparency, line ends, dashes
p = Presentation()
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Solid background"
s.background.fill.solid(); s.background.fill.fore_color.rgb = RGBColor(0xF4, 0xEC, 0xD8)
s = p.slides.add_slide(p.slide_layouts[6])
s.background.fill.gradient(); s.background.fill.gradient_angle = 45
s.background.fill.gradient_stops[0].color.rgb = RGBColor(0x10, 0x30, 0x60)
s.background.fill.gradient_stops[1].color.rgb = RGBColor(0x90, 0xC0, 0xF0)
sh = s.shapes.add_shape(MSO_SHAPE.RECTANGLE, Inches(0.5), Inches(0.5), Inches(4), Inches(3))
sh.fill.gradient(); sh.fill.gradient_angle = 90
sh.fill.gradient_stops[0].color.rgb = RGBColor(0xFF, 0x80, 0x00); sh.fill.gradient_stops[1].color.rgb = RGBColor(0x80, 0x00, 0x80)
sh = s.shapes.add_shape(MSO_SHAPE.OVAL, Inches(5), Inches(0.5), Inches(4), Inches(3))
sh.fill.patterned(); sh.fill.pattern = MSO_PATTERN.DARK_DOWNWARD_DIAGONAL; sh.fill.fore_color.rgb = RGBColor(0, 0x60, 0); sh.fill.back_color.rgb = RGBColor(0xE0, 0xFF, 0xE0)
sh = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(2.5), Inches(2), Inches(5), Inches(2.5)); sh.text = "50% transparent"
sh.fill.solid(); sh.fill.fore_color.rgb = RGBColor(0xFF, 0xFF, 0xFF)
sh.fill._xPr.find(qn("a:solidFill"))[0].append(xml('<a:alpha val="50000"/>'))
sh.line.fill.background()
for i, (dash, w) in enumerate([(DASH.SOLID, 1), (DASH.DASH, 2.5), (DASH.LONG_DASH, 4), (DASH.ROUND_DOT, 6)]):
    ln = s.shapes.add_connector(MSO_CONNECTOR.STRAIGHT, Inches(0.5), Inches(5 + i * 0.6), Inches(9), Inches(5 + i * 0.6))
    ln.line.width = Pt(w); ln.line.dash_style = dash; ln.line.color.rgb = RGBColor(0x30, 0x30, 0x30)
    lnx = ln.line._get_or_add_ln()
    lnx.append(xml('<a:headEnd type="oval"/>')); lnx.append(xml('<a:tailEnd type="triangle" w="lg" len="lg"/>'))
notes(s, "Gradient background, gradient/pattern/transparent fills, dashed lines with arrow heads")
s = p.slides.add_slide(p.slide_layouts[6])
s.background.fill.patterned(); s.background.fill.pattern = MSO_PATTERN.SMALL_GRID
s.background.fill.fore_color.rgb = RGBColor(0xDD, 0xDD, 0xDD); s.background.fill.back_color.rgb = RGBColor(0xFF, 0xFF, 0xFF)
sh = s.shapes.add_shape(MSO_SHAPE.HEART, Inches(3), Inches(1.5), Inches(4), Inches(4)); sh.rotation = -15
sh.shadow.inherit = False
p.save(f"{out}/gen-09-fills-lines.pptx")


# 10. Show settings: transitions, entrance animations, a hidden slide, sections
def anim_timing(spids):
    pars, cid = [], 3
    for spid in spids:
        pars.append(f'''<p:par><p:cTn id="{cid}" fill="hold"><p:stCondLst><p:cond delay="indefinite"/></p:stCondLst><p:childTnLst>
<p:par><p:cTn id="{cid + 1}" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
<p:par><p:cTn id="{cid + 2}" presetID="10" presetClass="entr" presetSubtype="0" fill="hold" nodeType="clickEffect">
<p:stCondLst><p:cond delay="0"/></p:stCondLst><p:childTnLst>
<p:set><p:cBhvr><p:cTn id="{cid + 3}" dur="1" fill="hold"><p:stCondLst><p:cond delay="0"/></p:stCondLst></p:cTn>
<p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl><p:attrNameLst><p:attrName>style.visibility</p:attrName></p:attrNameLst></p:cBhvr>
<p:to><p:strVal val="visible"/></p:to></p:set>
<p:animEffect transition="in" filter="fade"><p:cBhvr><p:cTn id="{cid + 4}" dur="500"/><p:tgtEl><p:spTgt spid="{spid}"/></p:tgtEl></p:cBhvr></p:animEffect>
</p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par></p:childTnLst></p:cTn></p:par>''')
        cid += 5
    return xml(f'''<p:timing><p:tnLst><p:par><p:cTn id="1" dur="indefinite" restart="never" nodeType="tmRoot"><p:childTnLst>
<p:seq concurrent="1" nextAc="seek"><p:cTn id="2" dur="indefinite" nodeType="mainSeq"><p:childTnLst>{"".join(pars)}</p:childTnLst></p:cTn>
<p:prevCondLst><p:cond evt="onPrev" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:prevCondLst>
<p:nextCondLst><p:cond evt="onNext" delay="0"><p:tgtEl><p:sldTgt/></p:tgtEl></p:cond></p:nextCondLst></p:seq>
</p:childTnLst></p:cTn></p:par></p:tnLst></p:timing>''')


p = Presentation()
transitions = ['<p:fade/>', '<p:push dir="u"/>', '<p:wipe dir="r"/>', '<p:split orient="vert" dir="in"/>', '<p:cover dir="l"/>', '<p:randomBar/>']
for i, tr in enumerate(transitions):
    s = p.slides.add_slide(p.slide_layouts[1]); s.shapes.title.text = f"Transition {i + 1}"
    tf = s.placeholders[1].text_frame; tf.text = "First bullet"; tf.add_paragraph().text = "Second bullet"
    box = s.shapes.add_shape(MSO_SHAPE.OVAL, Inches(7), Inches(5), Inches(2), Inches(1.5)); box.text = "appears"
    clr = s._element.find(qn("p:clrMapOvr"))
    pos = list(s._element).index(clr) + 1 if clr is not None else 1
    s._element.insert(pos, xml(f'<p:transition spd="med" advClick="1">{tr}</p:transition>'))
    if i % 2 == 0:
        s._element.insert(pos + 1, anim_timing([box.shape_id, s.placeholders[1].shape_id]))
    notes(s, f"Slide {i + 1}: {tr}")
p.slides[3]._element.set("show", "0")
p.slides[3].shapes.title.text = "Hidden slide"
ids = [sid.get("id") for sid in p.part._element.find(qn("p:sldIdLst"))]
secs = "".join(f'<p14:section name="{n}" id="{{6A3D1F00-0000-4000-8000-00000000000{k}}}"><p14:sldIdLst>'
               + "".join(f'<p14:sldId id="{x}"/>' for x in grp) + "</p14:sldIdLst></p14:section>"
               for k, (n, grp) in enumerate([("Opening", ids[:2]), ("Middle", ids[2:4]), ("Close", ids[4:])]))
ext_lst = p.part._element.find(qn("p:extLst"))
if ext_lst is None:
    ext_lst = xml("<p:extLst/>"); p.part._element.append(ext_lst)
ext_lst.append(xml(f'<p:ext uri="{{521415D9-36F7-43E2-AB2F-B90AF26B5E84}}"><p14:sectionLst>{secs}</p14:sectionLst></p:ext>'))
p.save(f"{out}/gen-10-show-settings.pptx")

# 11. More charts: doughnut, radar, bubble, 100% stacked column with labels, line with axis titles
p = Presentation()
cd = CategoryChartData(); cd.categories = ["Mon", "Tue", "Wed", "Thu", "Fri"]
cd.add_series("Visits", (120, 135, 128, 160, 190)); cd.add_series("Orders", (12, 18, 11, 25, 30))
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Doughnut"
pc = CategoryChartData(); pc.categories = ["A", "B", "C"]; pc.add_series("Share", (50, 30, 20))
ch = s.shapes.add_chart(XL_CHART_TYPE.DOUGHNUT, Inches(1.5), Inches(1.5), Inches(7), Inches(5), pc).chart
ch.plots[0].has_data_labels = True; ch.plots[0].data_labels.show_percentage = True
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Radar"
s.shapes.add_chart(XL_CHART_TYPE.RADAR_MARKERS, Inches(1), Inches(1.5), Inches(8), Inches(5), cd)
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Bubble"
bd = BubbleChartData(); se = bd.add_series("Projects")
for x, y, z in [(1, 4, 10), (2, 2, 25), (3, 5, 5), (4, 3, 15)]: se.add_data_point(x, y, z)
s.shapes.add_chart(XL_CHART_TYPE.BUBBLE, Inches(1), Inches(1.5), Inches(8), Inches(5), bd)
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "100% stacked, labelled"
ch = s.shapes.add_chart(XL_CHART_TYPE.COLUMN_STACKED_100, Inches(1), Inches(1.5), Inches(8), Inches(5), cd).chart
ch.plots[0].has_data_labels = True; ch.plots[0].data_labels.position = XL_LABEL_POSITION.CENTER
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Line with axis titles"
ch = s.shapes.add_chart(XL_CHART_TYPE.LINE, Inches(1), Inches(1.5), Inches(8), Inches(5), cd).chart
ch.has_title = True; ch.chart_title.text_frame.text = "Week overview"
ch.category_axis.has_title = True; ch.category_axis.axis_title.text_frame.text = "Day"
ch.value_axis.has_title = True; ch.value_axis.axis_title.text_frame.text = "Count"
ch.value_axis.has_major_gridlines = True; ch.value_axis.maximum_scale = 200
notes(s, "Chart title, axis titles, fixed axis maximum")
p.save(f"{out}/gen-11-charts-more.pptx")

# 12. Links and actions: web links on runs and shapes, mail link, jump to slide, next/previous
p = Presentation()
slides = [p.slides.add_slide(p.slide_layouts[5]) for _ in range(4)]
for i, s in enumerate(slides): s.shapes.title.text = f"Navigation slide {i + 1}"
s = slides[0]
tb = s.shapes.add_textbox(Inches(0.5), Inches(1.6), Inches(9), Inches(1)); para = tb.text_frame.paragraphs[0]
r = para.add_run(); r.text = "A web link"; r.hyperlink.address = "https://example.org/page?x=1&y=2"
r = para.add_run(); r.text = " and a mail link"; r.hyperlink.address = "mailto:someone@example.org?subject=Hello"
for i, (label, how) in enumerate([("Go to slide 3", slides[2]), ("Next", PP_ACTION.NEXT_SLIDE), ("Last", PP_ACTION.LAST_SLIDE), ("Website", "https://example.com/")]):
    b = s.shapes.add_shape(MSO_SHAPE.ROUNDED_RECTANGLE, Inches(0.5 + i * 2.3), Inches(3.5), Inches(2), Inches(0.8)); b.text = label
    if isinstance(how, str): b.click_action.hyperlink.address = how
    elif how in (PP_ACTION.NEXT_SLIDE, PP_ACTION.LAST_SLIDE):
        b.click_action.target_slide = slides[1] if how == PP_ACTION.NEXT_SLIDE else slides[-1]
    else: b.click_action.target_slide = how
back = slides[3].shapes.add_shape(MSO_SHAPE.ACTION_BUTTON_HOME, Inches(0.5), Inches(5.5), Inches(1), Inches(1))
back.click_action.target_slide = slides[0]
notes(slides[0], "Hyperlinks on runs and shapes, slide jumps")
p.save(f"{out}/gen-12-links-actions.pptx")


# 13. Picture formats: JPEG, GIF, BMP, TIFF, transparent PNG; borders, overlap with text
def img(fmt, w, h, seed, mode="RGB"):
    im = Image.new(mode, (w, h), (40 * seed % 255, 120, 200 - 30 * seed) + ((0,) if mode == "RGBA" else ()))
    d = ImageDraw.Draw(im)
    d.rectangle([w // 6, h // 6, 5 * w // 6, 5 * h // 6], fill=(250, 220, 60) + ((200,) if mode == "RGBA" else ()))
    d.text((10, 10), fmt, fill=(0, 0, 0) + ((255,) if mode == "RGBA" else ()))
    b = io.BytesIO(); im.save(b, fmt); b.seek(0); return b


p = Presentation()
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Picture formats"
for i, fmt in enumerate(["JPEG", "GIF", "BMP", "TIFF"]):
    pic = s.shapes.add_picture(img(fmt, 300, 200, i + 1), Inches(0.3 + i * 2.4), Inches(1.8), Inches(2.2))
    if i % 2: pic.line.color.rgb = RGBColor(0xC0, 0, 0); pic.line.width = Pt(3)
s = p.slides.add_slide(p.slide_layouts[6])
s.shapes.add_picture(img("PNG", 800, 600, 2), 0, 0, p.slide_width, p.slide_height)
s.shapes.add_picture(img("PNG", 300, 300, 4, "RGBA"), Inches(3.5), Inches(2), Inches(3))
tb = s.shapes.add_textbox(Inches(1), Inches(6), Inches(8), Inches(1)); tb.text_frame.text = "Text over a transparent picture"
tb.text_frame.paragraphs[0].runs[0].font.size = Pt(32); tb.text_frame.paragraphs[0].runs[0].font.color.rgb = RGBColor(255, 255, 255)
notes(s, "Full-bleed PNG, semi-transparent PNG on top, text on top of both")
p.save(f"{out}/gen-13-picture-formats.pptx")

# 14. Tables, styled: no header row, banded columns, 2x2 merge, cell anchors, margins, borders
p = Presentation()
s = p.slides.add_slide(p.slide_layouts[5]); s.shapes.title.text = "Table styling"
gf = s.shapes.add_table(5, 4, Inches(0.5), Inches(1.5), Inches(9), Inches(4.5)); t = gf.table
t.first_row = False; t.horz_banding = False; t.vert_banding = True; t.last_col = True
t.columns[0].width = Inches(3); t.columns[1].width = Inches(2)
for r in range(5):
    for c in range(4): t.cell(r, c).text = f"R{r + 1}C{c + 1}"
t.cell(1, 1).merge(t.cell(2, 2)); t.cell(1, 1).text = "2 x 2 merged"
for r, anchor in enumerate([MSO_ANCHOR.TOP, MSO_ANCHOR.MIDDLE, MSO_ANCHOR.BOTTOM]):
    cell = t.cell(r + 2, 0); cell.vertical_anchor = anchor; cell.margin_left = Inches(0.3)
cell = t.cell(4, 3)
cell._tc.get_or_add_tcPr().insert(0, xml('<a:lnB w="38100"><a:solidFill><a:srgbClr val="C00000"/></a:solidFill><a:prstDash val="dash"/></a:lnB>'))
t.rows[4].height = Inches(1.2)
notes(s, "Table flags, merged block, anchors, margins, a dashed border")
p.save(f"{out}/gen-14-tables-styled.pptx")
print("ok")
