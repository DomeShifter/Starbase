# House style

Starbase Sheets should feel like Google Sheets. These are the sizes, colours and shapes every part of
the app uses. They live in `app/src/house.rs`: build new UI from its helpers, never with numbers of
its own.

## Spacing

Everything sits on a 4 px scale.

| Name | Size | Used for |
|---|---|---|
| XS | 4 px | Between swatches, inside small bubbles |
| S | 8 px | Between items in a dialog, between buttons |
| M | 12 px | Inside text boxes, inside a note |
| L | 16 px | Between groups of fields, side panel and card padding, menu side padding |
| XL | 24 px | Dialog padding, the space above a dialog's buttons |

## Text

| Use | Size | Colour |
|---|---|---|
| Dialog title | 22 px | #1F1F1F |
| Field label | 14 px, bold | #1F1F1F |
| Body, menu rows, buttons | 14 px | #1F1F1F |
| Help text under a label | 12 px | #444746 |
| Small capitals heading (STANDARD) | 11 px, bold | #444746 |
| Menu shortcuts | 13 px | grey |
| Errors | 12 px | #B3261E |

## Shapes

| Thing | Corners | Padding | Other |
|---|---|---|---|
| Dialog | 8 px | 24 px | White, soft shadow, title at the top left, ✕ at the top right |
| Menu | 8 px | 8 px top and bottom | Rows run edge to edge, 32 px high, 16 px side padding; anything that isn't a row (a palette) is inset 16 px |
| Card (note, link, date picker) | 8 px | 16 px (notes 12 px) | White, soft shadow |
| Bubble (calculation preview) | 4 px | 8 x 4 px | |
| Side panel | none | 16 px | 1 px divider line |
| Text box, drop-down | 4 px | 12 px inside | 36 px high, 1 px grey outline (#747775), blue when open |
| Button | pill (18 px) | 12 px inside | 36 px high, at least 88 px wide |
| Divider | | 8 px above and below | 1 px #DADCE0 |
| Tabs in a dialog (`house::tabs`) | | 16 px below the row | 14 px labels, at least 88 px wide; the chosen one blue and bold with a 3 px blue bar under it; 1 px #DADCE0 line under the row |

## Lists and drop-downs

- A drop-down (`house::select`) opens a menu that shows **every choice at once**, as tall as the
  window allows, with a ✓ by the current one and dividers between groups.
- When it still doesn't fit (a small window or a low-resolution screen), it scrolls with a
  **solid scroll bar** that is always visible, so it's obvious there is more. Every scrolling
  list uses solid bars; only the sheet tab strip keeps a thin bar that shows on hover.
- **Toolbar drop-downs** (zoom, font size, and any new one) use `shell::pick_rows`: the same
  32 px rows, 16 px side padding and ✓ as the menus. A box with a list (font size) keeps its ▾
  **inside** the box.
- **Never** fill a menu with plain `egui` buttons: every list is `house::select` (in panels and
  dialogs) or `shell::pick_rows` (in the toolbar), so a new one can't miss the house style.

## Buttons

- **Main button:** green (#146C2E) pill with white text. Only one per dialog, at the far right.
- **Other buttons:** white pill, grey outline, green text, to the left of the main one, 8 px apart.
- **Buttons inside a list** ("Restore"): green text, no outline.
- **Blue (#0B57D0) instead of green** in the Share dialog and on the Starbase Drive home screen (`house::BLUE` with `primary_in`, `secondary_in`, `actions_in`): Google uses blue there, and Starbase Docs' share dialog does too.

## Text fields and search bars

Every one-line text field (search bars, rename boxes, name fields) is `house::field`: the text is
**centred top to bottom** in its box (never riding high), 14 px (`BODY`), with 8 px at the sides.
Search bars use 16 px text (`SEARCH_TEXT`) in a 48 px rounded box with a search icon. A rename
box inside a row or card is inset from the row's edges (at least 4 px all round) so there is white
space around it.

## Menus

Every menu (menu bar, right-click menus, the New button, the avatar menu) uses `house::menu(ui, width)`
first thing: flat full-width rows, **16 px from the sides**, **32 px tall**, grey on hover, no
rounded rows, 8 px above the first row and below the last. Rows are `house::menu_row` (an icon,
the label, a grey shortcut at the right); a row without an icon in a menu that has icons passes
`Some("")` so the labels line up. Dividers are `house::menu_sep` (a light line with 8 px above and
below). Menus are 220 px plain, 260–320 px with icons or shortcuts. Anything that isn't a row
(a palette, a heading) sits inside `house::menu_inset`.

**List menus fit their content (rule since 2026-11-23).** A menu whose rows are names rather than
commands (the all-sheets list, a drop-down cell's choices, any list of the user's own items) is
only as wide as its longest row: `house::fit_width(ui, labels, icon_column)` = the widest label +
16 px padding each side + the icon column if there is one + 12 px, between 80 and 480 px. The
fixed 220 / 320 px widths are only for command menus.

**Every row of a menu is the same kind of row, and every divider is as wide as its menu (rule
since 2026-11-23, after "Edit rule" in drop-down lists came out on two lines, further right than
the choices, under a divider wider than the rows).** A row added to a list menu (a command under
the choices, such as "Edit rule") is drawn with the list's own row (`MENU_LOOK.list_row`), not a
command row with an icon column, and counts towards `fit_width`. A divider never has a width of
its own: `menu_sep_width` takes the menu's width (`ui.min_rect().width()`). **Check:** before a menu
change is marked Done, take one screenshot of it open (ff-shot) and look at three things: every
label on one line, every label on the same left edge, the highlight as wide as the menu.

## Folder colours

Folders can be given one of 14 colours (`house::FOLDER_COLOURS`), chosen to be muted, easy to tell
apart, readable on white and on the light card fill, and never a plain primary:

| Name | Hex | Name | Hex |
|---|---|---|---|
| Slate | #5B6B7F | Coral | #E07A5F |
| Ink | #34495E | Terracotta | #B5523B |
| Ocean | #2E7DA3 | Sand | #B08D57 |
| Periwinkle | #6C7FD1 | Mustard | #C9A227 |
| Lavender | #8E7CC3 | Olive | #8A9A3B |
| Plum | #8E4A87 | Sage | #6E9E75 |
| Rose | #C2577A | Teal | #2A9D8F |

A coloured folder is drawn as a filled folder in its colour (`folder_fill`, white in the file so
it takes the tint); a folder without one is the grey outline. Swatches are 22 px circles on a
32 px grid, 7 to a row, with a dark ring round the current one. The colour is personal, as in
Drive: it only changes how the folder looks to the person who picked it.

## Selection and hover

| State | Look |
|---|---|
| Selected cells | Blue (#1A73E8) 2 px border, light blue fill |
| Menu row hover | #F1F3F4 |
| Selected item in a list or chip | #E8F0FE |
| Focused field | Blue outline |

## Tooltips and messages

| Thing | Look |
|---|---|
| Tooltip | Dark grey (#3C4043), white text, 4 px corners, 8 px side padding |
| Message ("Heads up") | A house dialog with an OK button, in Starbase Sheets' words (never "GridCraft") |

## Side panels

Right-hand panels (conditional formatting, data validation, chart editor, named ranges, protected
ranges) are 320 px of content with 16 px padding, keep room for their scroll bar, and only one is
open at a time. Changes in them show on the sheet straight away.

## Live preview

Where a change can be shown before it is applied (colours, formats), show it in the sheet straight
away; Cancel puts it back.
