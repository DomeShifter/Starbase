# StarDocs wasm: what can be trimmed at the edges (measured 2026-11-24)

Starting point: CI artifact `stardocs-web` from run 45477196092 (StarDocs workflow; the web build job
passed, other jobs failed). `stardocs-…_bg.wasm` = **53.47 MB raw, 11.52 MB brotli (q11)**.

## How it was measured

- Builds on a build machine, from a copy of the same StarDocs tree.
  Each variant is a full `cargo build --release --target wasm32-unknown-unknown`, then
  `wasm-opt -Oz` with the flags in `app/index.html` (binaryen 133), then `brotli -q 11`.
  No wasm-bindgen step, so local numbers are ~1.2 MB raw / ~0.13 MB br above CI; compare variants
  with each other, not with CI.
- Per-crate code sizes: a name-keeping relink (`-C strip=none`) and `twiggy top` (sizes before
  wasm-opt; wasm-opt removes roughly a quarter).
- Embedded data: data segments matched against the font files byte-for-byte; brotli effect measured
  on the data section alone.

## Measured builds (after wasm-opt -Oz)

| Build | Raw MB | Brotli MB | vs baseline (raw / br) |
|---|---|---|---|
| Baseline (today's profile: opt-level 3, thin LTO, 16 CGUs) | 54.65 | 11.65 | – |
| opt-level = "s" | 38.31 | 9.60 | −16.3 / −2.05 |
| opt-level = "z" | 34.66 | 8.94 | −20.0 / −2.71 |
| "s" + codegen-units = 1 | 37.40 | 9.41 | −17.3 / −2.24 (−0.19 br over "s") |
| "s" + eframe `glow` instead of `wgpu` | 35.93 | 8.97 | −18.7 / −2.68 (−0.63 br over "s") |
| "s" + image without `tiff`, zip without zopfli | 37.75 | 9.52 | −16.9 / −2.13 (−0.08 br over "s") |
| **"z" + glow + no tiff/zopfli (combined)** | **32.00** | **8.27** | **−22.7 / −3.38 (−29 % br)** |

## Candidates

| Candidate | Saves (raw MB / est. brotli MB) | Docs features lost | How | Risk |
|---|---|---|---|---|
| Release opt-level "z" (or "s") | 20.0 / 2.71 measured ("s": 16.3 / 2.05) | none | `opt-level` in gen-cargo.py's `[profile.release]` (StarDocs is the only release build of this workspace) | Medium: slower code. Time a big document's layout/typing before choosing; "s" is the safer middle |
| eframe `glow` instead of `wgpu` | 2.4 / 0.63 measured (drops naga + wgpu-core/hal) | none: WebGL2 renders everything; only the WebGPU path goes | app/Cargo.toml eframe features `wgpu` → `glow`; drop the `?webgl` + `wgpu_render_state` lines in app/src/web.rs (WordCraft engine never uses wgpu) | Low–medium: test text crispness, big pictures (texture limit), Safari/Firefox |
| codegen-units = 1 | 0.9 / 0.19 measured | none | gen-cargo.py profile | Low; longer CI builds. (Fat LTO not tested: likely a bit more, needs lots of RAM) |
| image: drop `tiff`; zip: `deflate` → `deflate-flate2-zlib-rs` (no zopfli) | 0.55 / 0.08 measured | TIFF pictures inside an imported .docx stop showing (Google Docs can't insert TIFF either). Zopfli only runs above level 9; WordCraft writes level 6 | gen-cargo.py workspace deps | Low |
| Duplicate UI fonts (Inter ×3, Source Sans 3 Regular, JetBrains Mono embedded twice: `ui-egui/theme.rs` and `fonts/fontdb/face.rs`) | 1.84 / ~0 (brotli already spots the copy: 3.407 vs 3.409 MB) | none | fork patch: theme.rs takes the bytes from wordcraft-fonts instead of its own `include_bytes!` | Very low. Helps memory and start-up only, not download |
| egui's default fonts Hack + Ubuntu-Light (shadowed by JetBrains Mono / Inter) | 0.67 / ≤0.27 | none expected (keep NotoEmoji + emoji-icon-font for symbols) | fork patch: ui-egui stops enabling `default_fonts`, adds the emoji fonts itself; StarDocs drops `default_fonts` from eframe | Medium: some UI symbols may fall back to boxes; check menus/ribbon |
| Bundled document fonts (Source Serif 4 ×5, Source Sans 3 Bold/It/Semibold) loaded on demand as woff2 | ~2.9 / ~1.2 off the first download (still fetched when a document uses them) | none, if the fetch works | fork patch in wordcraft-fonts (`wuff` already decodes woff2) + serve the files | Medium: layout must wait or re-flow when a face arrives |
| Zotero integration (`wordcraft-zotero`, ~0.2 MB code before wasm-opt) | ~0.15 / ~0.03 | Zotero citations (not a Google Docs feature) | fork patch: make it an optional feature of ui-egui | Low, but small gain |
| WebP decoder (`image-webp`, ~0.66 MB code before wasm-opt) | ~0.5 / ~0.12 | WebP pictures (Docs accepts WebP), unless the browser decodes them instead | drop `webp`, decode via `createImageBitmap` in StarDocs | Medium; not worth it now |
| PDF export (krilla, subsetter, write-fonts, pdf-writer: ~1.55 MB code before wasm-opt) | ~1.2 / ~0.3 | Download as PDF (a Docs feature) | lazy-loaded second wasm, or browser print | High; not recommended |
| Spell check (craft-proof ~0.77 MB code + 0.34 MB SCOWL list) | – | spelling (a Docs feature) | keep | – |
| Arimo (1.29 MB raw / 0.60 br, StarDocs' own) | – | Arial metrics for every new document | keep (needed on every load) | – |
| Carlito + Caladea (3.0 MB raw / 0.83 br, added 2026-11-25) | – | Calibri and Cambria metrics | not in the wasm: fetched from `office/` beside the page after start (Carlito cannot be subset: Reserved Font Name) | – |

Already fine: no UI translations are embedded (English only); no CJK fonts; `rfd` and accesskit are tiny
(<0.1 MB each).

## Top 5 safe cuts (recommendation)

1. **opt-level "s" now, "z" if a timing check shows no visible slowdown**: −2.05 / −2.71 MB brotli. By far the biggest win, one line in gen-cargo.py.
2. **eframe `glow` instead of `wgpu`**: −0.63 MB brotli, no Docs feature lost; test rendering in the three main browsers.
3. **codegen-units = 1**: −0.19 MB brotli, costs only CI time.
4. **No `tiff`, no zopfli**: −0.08 MB brotli, no Docs feature lost.
5. **De-duplicate the UI fonts** (small fork patch): −1.84 MB raw (memory, start-up), ~0 brotli.

1–4 together (measured as "z" + glow + codecs): **32.0 MB raw / 8.27 MB brotli, down from 54.7 / 11.65 (−29 % download)**. 1, 3 and 4 touch only gen-cargo.py; 2 touches only StarDocs' own app; 5 is the only fork patch.

Build artefacts (variants, twiggy CSV, logs) were kept on the build machine.
