# Icons

Material Symbols (Outlined, 24 px, weight 400) from https://github.com/google/material-design-icons,
licensed under the Apache License 2.0. Unchanged except for `fill="#444746"` added to each `<svg>`.
To add one: download `symbols/web/<name>/materialsymbolsoutlined/<name>_24px.svg`, add the same
fill, and list it in `app/src/icons.rs`.
- `numbers_123.svg` has a tighter `viewBox` (the glyph drawn a third larger), so "123" is as big as the other menu icons.
- `folder_fill.svg` has `fill="#FFFFFF"` instead, so it can be tinted (folder colours).
- `starsheets.svg`, `stardocs.svg` and `stardrive.svg` are our own logos, not Material Symbols. `stardrive.svg`
  (an amber rounded square with a white folder and a star, like the other two, the home screen's logo) is also `app/stardrive.svg`, its tab icon.
