# Slides test corpus: where every deck comes from

The corpus is ~56 .pptx decks that the **Slides corpus** workflow (`.github/workflows/slides-corpus.yml`)
opens and saves with `deckcraft-cli`, then compares before and after. **No deck is stored in this
repository**: generated decks are made at test time by `make_decks.py`, and third-party test files are
downloaded at test time from pinned commits and checked against a sha256 (`fetch.py`, `fetched.json`).
Only decks with a clear open-source licence are used: no decks of unknown origin, nothing personal
(real-world decks collected from the web, which some test suites contain, were left out on purpose).

| Part | Count | Licence | Where |
|---|---|---|---|
| Generated with python-pptx | 14 | this repository's licence (MIT or Apache-2.0) | `make_decks.py` (all text, numbers and pictures are made up there) |
| Apache POI test files | 17 | Apache-2.0 | `apache/poi` at `4793c3e741c2858bf29e728f13f617ee784243f4`, folder `test-data/slideshow` |
| LibreOffice test files | 25 | MPL-2.0 | `LibreOffice/core` at `88289f1ea16f267c3f0438ff3bb72b33318da285`, folder `sd/qa/unit/data/pptx` |

## Attribution

- **Apache POI**: Copyright The Apache Software Foundation. Licensed under the Apache License, Version
  2.0 (https://www.apache.org/licenses/LICENSE-2.0). The files are used unchanged, as test input only.
- **LibreOffice**: test documents from the LibreOffice project, under the Mozilla Public License 2.0
  (https://mozilla.org/MPL/2.0/). The files are used unchanged, as test input only; the source of each
  is the link below.

## Generated decks (`make_decks.py`)

| Deck | What it covers |
|---|---|
| `gen-01-text-review` | all 11 layouts, nested bullets, run formatting, notes, a hyperlink, 16:9 |
| `gen-02-photos` | full-bleed picture, crop, rotation, picture placeholder |
| `gen-03-tables` | tables with merged cells, cell fill, a 20-row table |
| `gen-04-charts` | column, line, stacked area, bar, pie with labels, scatter |
| `gen-05-shapes` | autoshapes, connectors glued to shapes, a group, rotation, dashed line |
| `gen-06-lo-resaved` | deck 1 saved again by LibreOffice (made in the workflow: a deck written by another program) |
| `gen-07-text-formatting` | fonts, sizes, super/subscript, strike, spacing, small caps, highlight, alignment, autofit, wrap, anchors; 10 scripts incl. right-to-left, emoji |
| `gen-08-many-slides` | 60 slides of bullets with notes |
| `gen-09-fills-lines` | solid/gradient/pattern backgrounds, gradient/pattern/transparent fills, dashes, arrow heads |
| `gen-10-show-settings` | 6 transition types, entrance animations, a hidden slide, 3 sections |
| `gen-11-charts-more` | doughnut, radar, bubble, 100% stacked with labels, line with axis titles |
| `gen-12-links-actions` | web and mail links, jump-to-slide / next / last actions, action button |
| `gen-13-picture-formats` | JPEG, GIF, BMP, TIFF, transparent PNG, picture borders |
| `gen-14-tables-styled` | table style flags, 2×2 merge, cell anchors and margins, a dashed border |

## Third-party decks (`fetched.json`)

| Deck | What it covers | Source file |
|---|---|---|
| `poi-SampleShow` | sample deck: text, layouts | [SampleShow.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/SampleShow.pptx) |
| `poi-sample_pptx_grouping_issues` | nested groups | [sample_pptx_grouping_issues.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/sample_pptx_grouping_issues.pptx) |
| `poi-SmartArt` | SmartArt diagram | [SmartArt.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/SmartArt.pptx) |
| `poi-smartart-rotated-text` | SmartArt with rotated text | [smartart-rotated-text.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/smartart-rotated-text.pptx) |
| `poi-bug58144-headers-footers-2007` | headers, footers, slide numbers | [bug58144-headers-footers-2007.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/bug58144-headers-footers-2007.pptx) |
| `poi-radar-chart` | radar chart | [radar-chart.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/radar-chart.pptx) |
| `poi-scatter-chart` | scatter chart | [scatter-chart.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/scatter-chart.pptx) |
| `poi-chart-texture-bg` | chart with a texture background | [chart-texture-bg.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/chart-texture-bg.pptx) |
| `poi-chart-picture-bg` | chart with a picture background | [chart-picture-bg.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/chart-picture-bg.pptx) |
| `poi-layouts` | many slide layouts | [layouts.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/layouts.pptx) |
| `poi-backgrounds` | slide backgrounds: gradient, picture, pattern | [backgrounds.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/backgrounds.pptx) |
| `poi-shapes` | preset shapes and fills | [shapes.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/shapes.pptx) |
| `poi-with_japanese` | Japanese text | [with_japanese.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/with_japanese.pptx) |
| `poi-EmbeddedAudio` | embedded audio | [EmbeddedAudio.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/EmbeddedAudio.pptx) |
| `poi-EmbeddedVideo` | embedded video (p14:media) | [EmbeddedVideo.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/EmbeddedVideo.pptx) |
| `poi-themes` | several themes and masters | [themes.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/themes.pptx) |
| `poi-ArtisticEffectSample` | picture artistic effects | [ArtisticEffectSample.pptx](https://github.com/apache/poi/blob/4793c3e741c2858bf29e728f13f617ee784243f4/test-data/slideshow/ArtisticEffectSample.pptx) |
| `lo-ole-emf_min` | OLE object with an EMF preview | [ole-emf_min.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/ole-emf_min.pptx) |
| `lo-ole` | embedded OLE object | [ole.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/ole.pptx) |
| `lo-activex_checkbox` | ActiveX check box | [activex_checkbox.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/activex_checkbox.pptx) |
| `lo-activex_picture` | ActiveX control with a picture | [activex_picture.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/activex_picture.pptx) |
| `lo-smartart-org-chart` | SmartArt organisation chart | [smartart-org-chart.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/smartart-org-chart.pptx) |
| `lo-modern-comments` | modern (threaded) comments | [modern-comments.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/modern-comments.pptx) |
| `lo-MorphTransition` | Morph transition | [MorphTransition.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/MorphTransition.pptx) |
| `lo-airplane-transition` | airplane transition (p15) | [airplane-transition.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/airplane-transition.pptx) |
| `lo-rightToLeftParagraph` | right-to-left paragraphs | [rightToLeftParagraph.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/rightToLeftParagraph.pptx) |
| `lo-multicol` | text in columns | [multicol.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/multicol.pptx) |
| `lo-predefined-table-style` | built-in table style | [predefined-table-style.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/predefined-table-style.pptx) |
| `lo-tableBorderLineStyle` | table border line styles | [tableBorderLineStyle.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/tableBorderLineStyle.pptx) |
| `lo-shape-glow-effect` | glow effect | [shape-glow-effect.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/shape-glow-effect.pptx) |
| `lo-group-rot` | rotated group | [group-rot.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/group-rot.pptx) |
| `lo-crop-to-shape` | picture cropped to a shape | [crop-to-shape.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/crop-to-shape.pptx) |
| `lo-slidenum_field` | slide number field | [slidenum_field.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/slidenum_field.pptx) |
| `lo-hyperlinktest` | hyperlinks and click actions | [hyperlinktest.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/hyperlinktest.pptx) |
| `lo-video-loop-mute` | video set to loop and mute | [video-loop-mute.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/video-loop-mute.pptx) |
| `lo-effectOrder` | glow and outer shadow on one shape | [effectOrder.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/effectOrder.pptx) |
| `lo-NumberedList-12ab-ab-34` | numbered lists, mixed styles | [NumberedList-12ab-ab-34.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/NumberedList-12ab-ab-34.pptx) |
| `lo-custgeom-overlapping-contours` | custom geometry, overlapping paths | [custgeom-overlapping-contours.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/custgeom-overlapping-contours.pptx) |
| `lo-connector-shape-animations` | animations on connectors and shapes | [connector-shape-animations.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/connector-shape-animations.pptx) |
| `lo-anim-color-by-rgb` | colour animation | [anim-color-by-rgb.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/anim-color-by-rgb.pptx) |
| `lo-activex_commandbutton` | ActiveX command button | [activex_commandbutton.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/activex_commandbutton.pptx) |
| `lo-smartart-cycle` | SmartArt cycle | [smartart-cycle.pptx](https://github.com/LibreOffice/core/blob/88289f1ea16f267c3f0438ff3bb72b33318da285/sd/qa/unit/data/pptx/smartart-cycle.pptx) |

## Baseline and known losses

`baseline.json` is the `results.json` of a run of the workflow (see the comment at the top of
`.github/workflows/slides-corpus.yml` for how to refresh it). The check fails only when a deck does
worse than this baseline: a new inventory change, a second save that changes something, a page count
that no longer matches, or a page whose pixel difference grew by more than 0.3 points. Better results
are listed in the summary as a reason to refresh the baseline.

Known changes with DeckCraft v0.18.1 (all 56 decks open, save and reopen; a second save never
changes anything; LibreOffice opens every saved file with the same page count):

| Deck | What changes on save | How bad |
|---|---|---|
| `lo-activex_checkbox`, `lo-activex_commandbutton`, `lo-activex_picture` | ActiveX controls (`p:control`) are dropped; their preview pictures are kept as plain pictures | the controls are gone (PowerPoint blocks them today anyway); LibreOffice draws 0.4–4.5% differently |
| `lo-ole-emf_min` | the OLE object becomes a plain picture of its preview | looks the same; the embedded object can no longer be opened |
| `lo-ole` | the embedded file moves from `ppt/embeddings/` to `ppt/media/` (as .docx); the fallback copy of the object is dropped | looks the same; Office not tested |
| `lo-smartart-cycle`, `lo-smartart-org-chart` | SmartArt parts are written to `ppt/media/mediaN.xml` instead of `ppt/diagrams/` | valid package (right content types and relationships), unusual location; Office not tested |
| `lo-modern-comments` | a comment authors part is added | harmless |
| `poi-with_japanese` | 3 empty text runs are dropped; 4 hyperlinks whose relationship is missing in the original are written with the target `ppt/slides` | the input is broken; the written link is odd |
| `gen-06-lo-resaved` | no inventory change, but DeckCraft's own render of the saved file differs by up to 1.5% per page (LibreOffice: 0.4% on slide 1) | small; cause not looked into |
| `poi-sample_pptx_grouping_issues`, `poi-smartart-rotated-text` | no inventory change; renders differ by ≤0.5% | small |
