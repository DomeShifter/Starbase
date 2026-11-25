# Data validation, as Google Sheets does it

Studied in live Google Sheets on 2026-11-21. Data › Data validation (and Insert › Drop-down) opens a
side panel on the right.

## The panel
| Part | Sheets | Starbase Sheets |
|---|---|---|
| List | "Data validation rules": one card per rule (range, criterion), "+ Add rule" | same; bin on hover removes a rule |
| Apply to range | text box | same |
| Criteria | 25 choices in groups (below) | all 25 |
| Drop-down items | one row per item: colour chip, text, bin; "Add another item" | same (Sheets' light chip colours) |
| Allow multiple selections | tick box under the items | later |
| Help text | "Show help text for a selected cell" + text | same (GridCraft's input message, shown under the cell) |
| If invalid | Show a warning / Reject the input | same |
| Display style | Chip / Arrow / Plain text | Arrow / Plain text (chips later) |
| Buttons | Remove rule, Done; the rule applies as you build it | same |

## Criteria and how Starbase Sheets stores them
GridCraft only knows Excel's rule types, so some criteria become a custom formula written for the
range's top-left cell (TL). Item colours become ordinary conditional-formatting rules ("is equal
to", fill), so they show in that panel too and survive in .xlsx files.

| Criteria | Stored as |
|---|---|
| Drop-down | list `a,b,c` (items can't contain commas) |
| Drop-down (from a range) | list `=Sheet1!A1:A9` |
| Text contains / does not contain | custom `ISNUMBER(SEARCH(v,TL))` / `ISERROR(SEARCH(v,TL))` |
| Text is exactly | custom `EXACT(TL,v)` |
| Text is valid email / URL | custom `COUNTIF` with wildcards (`?*@?*.?*`, `http(s)://?*.?*`) |
| Is valid date | date between 1 and 2958465 |
| Date is (today, tomorrow, yesterday, past week/month/year, exact date) | date equal / between, with `TODAY()` formulas |
| Date is before / on or before / after / on or after | date with operator; today, tomorrow, yesterday, one week/month/year ago, or exact date |
| Date is (not) between | date between / notBetween |
| Greater than … Is not between | decimal with operator |
| Tick box | tick boxes in the cells + custom `ISLOGICAL(TL)` |
| Custom formula is | custom |

## How invalid data looks
Reject: a message box, the edit stays open. Warning: the value is kept and the cell gets a red
top-right corner (patch 0018). A drop-down cell shows a grey ▾ inside its right edge.
