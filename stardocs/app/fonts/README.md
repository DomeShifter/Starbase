# Fonts

Arimo (Regular, Bold, Italic, Bold Italic) by Steve Matteson, from the ChromeOS core fonts release
`croscorefonts-1.31.0` (https://commondatastorage.googleapis.com/chromeos-localmirror/distfiles/croscorefonts-1.31.0.tar.bz2),
licensed under the Apache License 2.0 (`LICENSE-APACHE`). Arimo is drawn to Arial's widths, so
Starbase Docs draws Arial text (a new document's default font) in it and lines break where they do in
Word and Google Docs. WordCraft already treats Arimo as Arial's twin; the app only has to load it
(`stardocs/app/src/bundled_fonts.rs`).

Changed only by removing the hinting tables, which keeps every glyph and every width (1.3 MB instead
of 2.3 MB), with fontTools 4.55.3:
`pyftsubset Arimo-X.ttf --glyphs='*' --unicodes='*' --no-hinting --layout-features='*' --name-IDs='*' --name-languages='*' --notdef-outline --glyph-names --legacy-kern --drop-tables+=DSIG`

## office/: Carlito and Caladea

NOTICE: Carlito (Regular, Bold, Italic, Bold Italic), Copyright 2013 The Carlito Project Authors
(https://github.com/googlefonts/carlito), with Reserved Font Name "Carlito", licensed under the SIL
Open Font License 1.1 (`office/LICENSE-OFL-Carlito.txt`). Unmodified files from Google Fonts
(https://github.com/google/fonts, `ofl/carlito`): the Reserved Font Name allows no changed copy
under the name "Carlito", and "Carlito" is the name WordCraft looks for.

NOTICE: Caladea (Regular, Bold, Italic, Bold Italic), Copyright 2012 The Caladea Project Authors
(https://github.com/huertatipografica/Caladea), licensed under the SIL Open Font License 1.1
(`office/LICENSE-OFL-Caladea.txt`). From Google Fonts (`ofl/caladea`), with the hinting tables
removed by the same `pyftsubset` command as Arimo (it has no Reserved Font Name; 216 KB instead of
333 KB).

Carlito is drawn to Calibri's widths and Caladea to Cambria's (Word's default fonts since 2007).
WordCraft already treats them as twins, so documents written in Calibri or Cambria keep their line
breaks. The desktop program carries them inside it. The web app fetches them from beside the page
(`office/`, copied there by Trunk) after it starts, so the program download stays the same size
(together they are 3.0 MB, 0.83 MB brotli).
