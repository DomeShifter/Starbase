# Conditional formatting, the Google Sheets way (checked against live Sheets, 2026-11-21)

| Part | Google Sheets | StarSheets |
|---|---|---|
| Opens from | Format › Conditional formatting, right-click › Conditional formatting | same; a side panel on the right (GridCraft's dialogs are taken over) |
| List | One card per rule: swatch ("123" in the style, or the gradient), what it does, its range; "+ Add another rule" | same; bin icon on hover removes a rule |
| Tabs | Single colour · Colour scale | same |
| Single colour | Apply to range → "Format cells if…" (19 conditions below) → value(s) → Formatting style: preview, B I U S, text colour, fill, presets → Cancel / Done | same (style presets as chips under the bar) |
| Colour scale | Preview (click for preset gradients) → Minpoint / Midpoint / Maxpoint: type (Min value, Max value, None for mid, Number, Percent, Percentile), value, colour | same (patch 0017 lets GridCraft take the three points) |

**Conditions (Sheets' order):** Is empty · Is not empty · Text contains · Text does not contain · Text starts with · Text ends with · Text is exactly · Date is · Date is before · Date is after · Greater than · Greater than or equal to · Less than · Less than or equal to · Is equal to · Is not equal to · Is between · Is not between · Custom formula is

Not yet: Sheets' "Date is" choices (today, tomorrow, past week…; StarSheets takes an exact date), live preview while editing, dragging rules to reorder.
