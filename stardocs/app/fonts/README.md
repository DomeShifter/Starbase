# Fonts

Arimo (Regular, Bold, Italic, Bold Italic) by Steve Matteson, from the ChromeOS core fonts release
`croscorefonts-1.31.0` (https://commondatastorage.googleapis.com/chromeos-localmirror/distfiles/croscorefonts-1.31.0.tar.bz2),
licensed under the Apache License 2.0 (`LICENSE-APACHE`). Arimo is drawn to Arial's widths, so
StarDocs draws Arial text (a new document's default font) in it and lines break where they do in
Word and Google Docs. WordCraft already treats Arimo as Arial's twin; the app only has to load it
(`add_bundled_fonts` in `stardocs/app/src/web.rs`).

Changed only by removing the hinting tables, which keeps every glyph and every width (1.3 MB instead
of 2.3 MB), with fontTools 4.55.3:
`pyftsubset Arimo-X.ttf --glyphs='*' --unicodes='*' --no-hinting --layout-features='*' --name-IDs='*' --name-languages='*' --notdef-outline --glyph-names --legacy-kern --drop-tables+=DSIG`
