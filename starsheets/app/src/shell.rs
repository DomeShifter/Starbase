//! StarSheets' own chrome: the title row, the menu bar and the toolbar, drawn above GridCraft's
//! formula bar and grid. Menus and toolbar are data tables (`MENUS`, `TOOLBAR`): adding an item is
//! one line naming its label and the GridCraft command it runs (`Session::execute` ids).
//! Target layout: docs/sheets-reference.md §1-3.

use egui::{Align, Button, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, icons, theme};

pub mod filterviews;

/// One menu entry.
pub enum Item {
    /// Label, command id, shortcut hint shown on the right.
    Cmd(&'static str, &'static str, &'static str),
    /// A command that takes fixed parameters (JSON text).
    CmdWith(&'static str, &'static str, &'static str, &'static str),
    /// A submenu.
    Sub(&'static str, &'static [Item]),
    Sep,
}
use Item::{Cmd, CmdWith, Sep, Sub};

/// The menu bar: Sheets' order and wording, items limited to commands GridCraft already has.
pub const MENUS: &[(&str, &[Item])] = &[
    (
        "File",
        &[
            Cmd("New spreadsheet", "panel:new", ""),
            Cmd("New from a template…", "panel:templates", ""),
            Cmd("Open", "panel:open", "Ctrl+O"),
            Cmd("Import", "file.open", ""),
            Cmd("Make a copy", "panel:copy", ""),
            Sep,
            Sub(
                "Download",
                &[
                    Cmd("Microsoft Excel (.xlsx)", "file.saveAs", ""),
                    Cmd("PDF (.pdf)", "file.exportPdf", ""),
                    Cmd("Web page (.html)", "file.exportHtml", ""),
                    Cmd("Comma-separated values (.csv)", "file.exportCsv", ""),
                ],
            ),
            Sep,
            Cmd("Move to bin", "panel:bin", ""),
            Sep,
            Cmd("Version history", "panel:versions", ""),
            Sep,
            Cmd("Details", "panel:details", ""),
            Cmd("Settings", "panel:settings", ""),
            Sep,
            Cmd("Print", "file.print", "Ctrl+P"),
        ],
    ),
    (
        "Edit",
        &[
            Cmd("Undo", "edit.undo", "Ctrl+Z"),
            Cmd("Redo", "edit.redo", "Ctrl+Y"),
            Sep,
            Cmd("Cut", "edit.cut", "Ctrl+X"),
            Cmd("Copy", "edit.copy", "Ctrl+C"),
            Cmd("Paste", "edit.paste", "Ctrl+V"),
            Sub(
                "Paste special",
                &[
                    CmdWith("Values only", "edit.pasteSpecial", "Ctrl+Shift+V", r#"{"what":"values"}"#),
                    CmdWith("Format only", "edit.pasteSpecial", "", r#"{"what":"formats"}"#),
                    CmdWith("Formula only", "edit.pasteSpecial", "", r#"{"what":"formulas"}"#),
                    CmdWith("Data validation only", "edit.pasteSpecial", "", r#"{"what":"validation"}"#),
                    Sep,
                    CmdWith("Transposed", "edit.pasteSpecial", "", r#"{"what":"all","transpose":true}"#),
                    CmdWith("Column widths only", "edit.pasteSpecial", "", r#"{"what":"columnWidths"}"#),
                    CmdWith("All except borders", "edit.pasteSpecial", "", r#"{"what":"allExceptBorders"}"#),
                ],
            ),
            Sep,
            Sub(
                "Move",
                &[
                    Cmd("Row up", "panel:moveRowUp", ""),
                    Cmd("Row down", "panel:moveRowDown", ""),
                    Cmd("Column left", "panel:moveColumnLeft", ""),
                    Cmd("Column right", "panel:moveColumnRight", ""),
                ],
            ),
            Sub(
                "Delete",
                &[
                    Cmd("Values", "edit.clearContents", "Delete"),
                    Cmd("Row", "home.deleteRows", ""),
                    Cmd("Column", "home.deleteColumns", ""),
                    Cmd("Cells and shift", "home.deleteCells", ""),
                ],
            ),
            Sep,
            Cmd("Find and replace", "ui:find", "Ctrl+H"),
        ],
    ),
    (
        "View",
        &[
            Sub(
                "Show",
                &[
                    Cmd("Gridlines", "view.gridlines", ""),
                    Cmd("Formula bar", "view.formulaBar", ""),
                    Cmd("Formulas", "formulas.showFormulas", "Ctrl+~"),
                ],
            ),
            Sub(
                "Freeze",
                &[
                    CmdWith("No rows", "panel:freeze", "", r#"{"rows":0}"#),
                    CmdWith("1 row", "panel:freeze", "", r#"{"rows":1}"#),
                    CmdWith("2 rows", "panel:freeze", "", r#"{"rows":2}"#),
                    CmdWith("Up to row {row}", "panel:freeze", "", r#"{"rows":"current"}"#),
                    Sep,
                    CmdWith("No columns", "panel:freeze", "", r#"{"cols":0}"#),
                    CmdWith("1 column", "panel:freeze", "", r#"{"cols":1}"#),
                    CmdWith("2 columns", "panel:freeze", "", r#"{"cols":2}"#),
                    CmdWith("Up to column {col}", "panel:freeze", "", r#"{"cols":"current"}"#),
                ],
            ),
            Sub("Group", &[Cmd("Group", "data.group", "Alt+Shift+→"), Cmd("Ungroup", "data.ungroup", "Alt+Shift+←")]),
            Cmd("Protected ranges", "panel:protect", ""),
            Sep,
            Sub("Hidden sheets", &[Cmd("Show all hidden sheets", "panel:unhideSheets", "")]),
            Sep,
            Sub(
                "Zoom",
                &[
                    CmdWith("50%", "view.zoom", "", r#"{"percent":50}"#),
                    CmdWith("75%", "view.zoom", "", r#"{"percent":75}"#),
                    CmdWith("90%", "view.zoom", "", r#"{"percent":90}"#),
                    CmdWith("100%", "view.zoom", "", r#"{"percent":100}"#),
                    CmdWith("125%", "view.zoom", "", r#"{"percent":125}"#),
                    CmdWith("150%", "view.zoom", "", r#"{"percent":150}"#),
                    CmdWith("200%", "view.zoom", "", r#"{"percent":200}"#),
                ],
            ),
            Cmd("Full screen", "panel:fullscreen", ""),
        ],
    ),
    (
        "Insert",
        &[
            Sub("Cells", &[Cmd("Insert cells", "home.insertCells", "")]),
            Sub("Rows", &[Cmd("Insert row above", "home.insertRows", "Ctrl+Alt+="), Cmd("Insert row below", "panel:insertBelow", "")]),
            Sub("Columns", &[Cmd("Insert column left", "home.insertColumns", "Ctrl+Alt+="), Cmd("Insert column right", "panel:insertRight", "")]),
            Cmd("Sheet", "home.insertSheet", "Shift+F11"),
            Sep,
            Cmd("Chart", "panel:chart", ""),
            Cmd("Pivot table", "insert.pivotTable", ""),
            Cmd("Sparkline", "panel:sparkline", ""),
            Sub("Image", &[Cmd("Insert image in cell", "panel:imageInCell", ""), Cmd("Insert image over cells", "panel:imageOverCells", "")]),
            Sep,
            Sub(
                "Function",
                &[
                    CmdWith("SUM", "formulas.autoSum", "", r#"{"function":"SUM"}"#),
                    CmdWith("AVERAGE", "formulas.autoSum", "", r#"{"function":"AVERAGE"}"#),
                    CmdWith("COUNT", "formulas.autoSum", "", r#"{"function":"COUNT"}"#),
                    CmdWith("MAX", "formulas.autoSum", "", r#"{"function":"MAX"}"#),
                    CmdWith("MIN", "formulas.autoSum", "", r#"{"function":"MIN"}"#),
                    Sep,
                    Cmd("All functions…", "formulas.insertFunction", ""),
                ],
            ),
            Cmd("Link", "insert.link", "Ctrl+K"),
            Sep,
            Cmd("Tick box", "insert.checkbox", ""),
            Cmd("Drop-down", "panel:dropdown", ""),
            Sep,
            Cmd("Comment", "panel:comment", "Ctrl+Alt+M"),
            Cmd("Note", "panel:note", "Shift+F2"),
        ],
    ),
    (
        "Format",
        &[
            Sub(
                "Number",
                &[
                    CmdWith("Automatic", "home.numberFormat", "", r#"{"format":"General"}"#),
                    CmdWith("Plain text", "home.numberFormat", "", r#"{"format":"Text"}"#),
                    Sep,
                    CmdWith("Number", "home.numberFormat", "1,000.12", r##"{"code":"#,##0.00"}"##),
                    CmdWith("Percent", "home.numberFormat", "10.12%", r#"{"code":"0.00%"}"#),
                    CmdWith("Scientific", "home.numberFormat", "1.01E+03", r#"{"code":"0.00E+00"}"#),
                    Sep,
                    CmdWith("Accounting", "home.numberFormat", "£ (1,000.12)", r#"{"format":"Accounting"}"#),
                    CmdWith("Financial", "home.numberFormat", "(1,000.12)", r##"{"code":"#,##0.00;(#,##0.00)"}"##),
                    CmdWith("Currency", "home.numberFormat", "£1,000.12", r##"{"code":"£#,##0.00"}"##),
                    CmdWith("Currency rounded", "home.numberFormat", "£1,000", r##"{"code":"£#,##0"}"##),
                    Sep,
                    CmdWith("Date", "home.numberFormat", "26/09/2008", r#"{"code":"@date"}"#),
                    CmdWith("Time", "home.numberFormat", "15:59:00", r#"{"code":"@time"}"#),
                    CmdWith("Date time", "home.numberFormat", "26/09/2008 15:59:00", r#"{"code":"@datetime"}"#),
                    CmdWith("Duration", "home.numberFormat", "24:01:00", r#"{"code":"[h]:mm:ss"}"#),
                    Sep,
                    Cmd("Custom currency", "panel:customcurrency", ""),
                    Cmd("Custom date and time", "panel:customdate", ""),
                    Cmd("Custom number format", "panel:customnumber", ""),
                ],
            ),
            Sub(
                "Text",
                &[
                    Cmd("Bold", "home.bold", "Ctrl+B"),
                    Cmd("Italic", "home.italic", "Ctrl+I"),
                    Cmd("Underline", "home.underline", "Ctrl+U"),
                    Cmd("Strikethrough", "home.strikethrough", "Alt+Shift+5"),
                ],
            ),
            Sub(
                "Alignment",
                &[
                    CmdWith("Left", "home.alignLeft", "Ctrl+Shift+L", r#"{"on":true}"#),
                    CmdWith("Centre", "home.alignCenter", "Ctrl+Shift+E", r#"{"on":true}"#),
                    CmdWith("Right", "home.alignRight", "Ctrl+Shift+R", r#"{"on":true}"#),
                    Sep,
                    Cmd("Top", "home.alignTop", ""),
                    Cmd("Middle", "home.alignMiddle", ""),
                    Cmd("Bottom", "home.alignBottom", ""),
                ],
            ),
            Sub(
                "Wrapping",
                &[
                    CmdWith("Overflow", "home.wrapText", "", r#"{"on":false}"#),
                    CmdWith("Wrap", "home.wrapText", "", r#"{"on":true}"#),
                    CmdWith("Clip", "home.wrapText", "", r#"{"on":false,"clip":true}"#),
                ],
            ),
            Sub(
                "Rotation",
                &[
                    CmdWith("None", "home.orientation", "", r#"{"angle":0}"#),
                    CmdWith("Tilt up", "home.orientation", "", r#"{"angle":45}"#),
                    CmdWith("Tilt down", "home.orientation", "", r#"{"angle":-45}"#),
                    CmdWith("Stack vertically", "home.orientation", "", r#"{"angle":"vertical"}"#),
                    CmdWith("Rotate up", "home.orientation", "", r#"{"angle":90}"#),
                    CmdWith("Rotate down", "home.orientation", "", r#"{"angle":-90}"#),
                ],
            ),
            Sep,
            Sub(
                "Font size",
                &[
                    CmdWith("6", "home.fontSize", "", r#"{"size":6}"#),
                    CmdWith("8", "home.fontSize", "", r#"{"size":8}"#),
                    CmdWith("9", "home.fontSize", "", r#"{"size":9}"#),
                    CmdWith("10", "home.fontSize", "", r#"{"size":10}"#),
                    CmdWith("11", "home.fontSize", "", r#"{"size":11}"#),
                    CmdWith("12", "home.fontSize", "", r#"{"size":12}"#),
                    CmdWith("14", "home.fontSize", "", r#"{"size":14}"#),
                    CmdWith("18", "home.fontSize", "", r#"{"size":18}"#),
                    CmdWith("24", "home.fontSize", "", r#"{"size":24}"#),
                    CmdWith("36", "home.fontSize", "", r#"{"size":36}"#),
                ],
            ),
            Sub(
                "Merge cells",
                &[
                    Cmd("Merge all", "home.mergeCells", ""),
                    Cmd("Merge vertically", "home.mergeDown", ""),
                    Cmd("Merge horizontally", "home.mergeAcross", ""),
                    Cmd("Unmerge", "home.unmergeCells", ""),
                ],
            ),
            Cmd("Convert to table", "insert.table", "Ctrl+Alt+T"),
            Cmd("Conditional formatting", "ui:manageRules", ""),
            Cmd("Alternating colours", "panel:alternatingColours", ""),
            Sep,
            Cmd("Clear formatting", "edit.clearFormats", "Ctrl+\\"),
        ],
    ),
    (
        "Data",
        &[
            Sub(
                "Sort sheet",
                &[
                    CmdWith("Sort sheet by column {col} (A to Z)", "panel:sortSheet", "", r#"{"desc":false}"#),
                    CmdWith("Sort sheet by column {col} (Z to A)", "panel:sortSheet", "", r#"{"desc":true}"#),
                ],
            ),
            Sub(
                "Sort range",
                &[
                    Cmd("Sort range by column {col} (A to Z)", "data.sortAscending", ""),
                    Cmd("Sort range by column {col} (Z to A)", "data.sortDescending", ""),
                    Sep,
                    Cmd("Advanced range sorting options", "data.sort", ""),
                ],
            ),
            Sep,
            Cmd("Create a filter", "data.filter", ""),
            Sub("Filter views", &[Cmd("Create new filter view", "panel:filterViewNew", ""), Cmd("Save as filter view", "panel:filterViewSave", "")]),
            Cmd("Add a slicer", "insert.slicer", ""),
            Cmd("Refresh pivot tables", "data.refreshAll", ""),
            Sep,
            Cmd("Protect sheets and ranges", "panel:protect", ""),
            Cmd("Named ranges", "ui:nameManager", ""),
            Cmd("Randomise range", "panel:randomise", ""),
            Sep,
            Cmd("Data validation", "ui:dataValidation", ""),
            Sub("Data clean-up", &[Cmd("Remove duplicates", "data.removeDuplicates", ""), Cmd("Trim whitespace", "panel:trimWhitespace", "")]),
            Cmd("Split text to columns", "data.textToColumns", ""),
        ],
    ),
    (
        "Tools",
        &[
            Sub("Spelling", &[Cmd("Spell check", "review.spelling", "")]),
            Sub(
                "Suggestion controls",
                &[Cmd("Smart Fill", "panel:smartFill", "Ctrl+Shift+Y"), Cmd("Turn off Smart Fill suggestions", "panel:smartFillToggle", "")],
            ),
            Sub("Calculation controls", &[CmdWith("Calculate now", "formulas.calculateNow", "", "{}")]),
            Sep,
            Cmd("Accessibility", "review.checkAccessibility", ""),
        ],
    ),
    (
        "Help",
        &[
            Cmd("Search the menus", "panel:searchMenus", "Alt+/"),
            Sep,
            Cmd("Function list", "formulas.insertFunction", ""),
            Cmd("Keyboard shortcuts", "panel:shortcuts", "Ctrl+/"),
        ],
    ),
];

/// One toolbar control.
pub enum Tool {
    /// Icon name, tooltip, command id, parameters (JSON text).
    Btn(&'static str, &'static str, &'static str, &'static str),
    /// Text instead of an icon (e.g. "£", "123"), tooltip, command id, parameters.
    Txt(&'static str, &'static str, &'static str, &'static str),
    /// Icon (or text when no icon of that name), tooltip, items: (label or icon:<name>, command id, parameters).
    Menu(&'static str, &'static str, &'static [(&'static str, &'static str, &'static str)]),
    /// Icon, tooltip, command id taking `color`, the "reset" label and value.
    Colour(&'static str, &'static str, &'static str, &'static str, &'static str),
    /// Sheets' borders menu: the presets, plus border colour and line style for the next one.
    Borders,
    Font,
    FontSize,
    Zoom,
    Gap,
}
use Tool::{Borders, Btn, Colour, Font, FontSize, Gap, Menu, Txt, Zoom};

/// The toolbar, left to right (docs/sheets-reference.md §3).
pub const TOOLBAR: &[Tool] = &[
    Btn("undo", "Undo (Ctrl+Z)", "edit.undo", "{}"),
    Btn("redo", "Redo (Ctrl+Y)", "edit.redo", "{}"),
    Btn("print", "Print (Ctrl+P)", "file.print", "{}"),
    Btn("format_paint", "Paint format", "edit.formatPainter", "{}"),
    Zoom,
    Gap,
    Txt("£", "Format as currency", "home.numberFormat", r#"{"code":"£#,##0.00"}"#),
    Btn("percent", "Format as percent", "home.numberFormat", r#"{"code":"0.00%"}"#),
    Btn("decimal_decrease", "Decrease decimal places", "home.decreaseDecimal", "{}"),
    Btn("decimal_increase", "Increase decimal places", "home.increaseDecimal", "{}"),
    Menu("123", "More formats", NUMBER_FORMATS),
    Gap,
    Font,
    Gap,
    FontSize,
    Gap,
    Btn("format_bold", "Bold (Ctrl+B)", "home.bold", "{}"),
    Btn("format_italic", "Italic (Ctrl+I)", "home.italic", "{}"),
    Btn("strikethrough_s", "Strikethrough (Alt+Shift+5)", "home.strikethrough", "{}"),
    Colour("format_color_text", "Text colour", "home.fontColor", "Reset", "auto"),
    Gap,
    Colour("format_color_fill", "Fill colour", "home.fillColor", "Reset", "none"),
    Borders,
    Menu("cell_merge", "Merge cells", MERGE),
    Gap,
    Menu("format_align_left", "Horizontal align", H_ALIGN),
    Menu("vertical_align_bottom", "Vertical align", V_ALIGN),
    Menu("wrap_text", "Text wrapping", WRAP),
    Menu("text_rotation_angleup", "Text rotation", ROTATION),
    Gap,
    Btn("link", "Insert link (Ctrl+K)", "insert.link", "{}"),
    Btn("insert_chart", "Insert chart", "panel:chart", "{}"),
    Btn("filter_alt", "Create a filter", "data.filter", "{}"),
    Menu("functions", "Functions", FUNCTIONS),
];

const NUMBER_FORMATS: &[(&str, &str, &str)] = &[
    ("Automatic", "home.numberFormat", r#"{"format":"General"}"#),
    ("Plain text", "home.numberFormat", r#"{"format":"Text"}"#),
    ("-", "", ""),
    ("Number|1,000.12", "home.numberFormat", r##"{"code":"#,##0.00"}"##),
    ("Percent|10.12%", "home.numberFormat", r#"{"code":"0.00%"}"#),
    ("Scientific|1.01E+03", "home.numberFormat", r#"{"code":"0.00E+00"}"#),
    ("-", "", ""),
    ("Accounting|£ (1,000.12)", "home.numberFormat", r#"{"format":"Accounting"}"#),
    ("Financial|(1,000.12)", "home.numberFormat", r##"{"code":"#,##0.00;(#,##0.00)"}"##),
    ("Currency|£1,000.12", "home.numberFormat", r##"{"code":"£#,##0.00"}"##),
    ("Currency rounded|£1,000", "home.numberFormat", r##"{"code":"£#,##0"}"##),
    ("-", "", ""),
    ("Date|26/09/2008", "home.numberFormat", r#"{"code":"@date"}"#),
    ("Time|15:59:00", "home.numberFormat", r#"{"code":"@time"}"#),
    ("Date time|26/09/2008 15:59:00", "home.numberFormat", r#"{"code":"@datetime"}"#),
    ("Duration|24:01:00", "home.numberFormat", r#"{"code":"[h]:mm:ss"}"#),
    ("-", "", ""),
    ("Custom currency", "panel:customcurrency", "{}"),
    ("Custom date and time", "panel:customdate", "{}"),
    ("Custom number format", "panel:customnumber", "{}"),
];

/// Border line styles: menu name and GridCraft's style id.
const BORDER_STYLES: &[(&str, &str)] =
    &[("Thin", "thin"), ("Medium", "medium"), ("Thick", "thick"), ("Dashed", "dashed"), ("Dotted", "dotted"), ("Double", "double")];

/// Draws one border style as a line across `r`, with a ✓ on the left when it's the current one.
fn line_sample(ui: &Ui, r: egui::Rect, k: usize, on: bool) {
    let p = ui.painter();
    let (x0, x1, y) = (r.left(), r.right(), r.center().y);
    let c = theme::text();
    match BORDER_STYLES[k].1 {
        "medium" => {
            p.hline(x0..=x1, y, Stroke::new(2.0, c));
        }
        "thick" => {
            p.hline(x0..=x1, y, Stroke::new(3.0, c));
        }
        "dashed" | "dotted" => {
            let (dash, gap) = if BORDER_STYLES[k].1 == "dashed" { (6.0, 3.0) } else { (1.5, 2.5) };
            let mut x = x0;
            while x < x1 {
                p.hline(x..=(x + dash).min(x1), y, Stroke::new(1.0, c));
                x += dash + gap;
            }
        }
        "double" => {
            p.hline(x0..=x1, y - 1.5, Stroke::new(1.0, c));
            p.hline(x0..=x1, y + 1.5, Stroke::new(1.0, c));
        }
        _ => {
            p.hline(x0..=x1, y, Stroke::new(1.0, c));
        }
    }
    if on {
        p.text(egui::pos2(r.left() - 20.0, y), egui::Align2::LEFT_CENTER, "✓", egui::FontId::proportional(16.0), c);
    }
}

const BORDERS: &[(&str, &str, &str)] = &[
    ("icon:border_all|All borders", "home.borders", r#"{"preset":"all"}"#),
    ("icon:border_inner|Inner borders", "home.borders", r#"{"preset":"inside"}"#),
    ("icon:border_horizontal|Horizontal borders", "home.borders", r#"{"preset":"insideHorizontal"}"#),
    ("icon:border_vertical|Vertical borders", "home.borders", r#"{"preset":"insideVertical"}"#),
    ("icon:border_outer|Outer borders", "home.borders", r#"{"preset":"outside"}"#),
    ("icon:border_left|Left border", "home.borders", r#"{"preset":"left"}"#),
    ("icon:border_top|Top border", "home.borders", r#"{"preset":"top"}"#),
    ("icon:border_right|Right border", "home.borders", r#"{"preset":"right"}"#),
    ("icon:border_bottom|Bottom border", "home.borders", r#"{"preset":"bottom"}"#),
    ("icon:border_clear|Clear borders", "home.borders", r#"{"preset":"none"}"#),
];

const MERGE: &[(&str, &str, &str)] = &[
    ("Merge all", "home.mergeCells", "{}"),
    ("Merge vertically", "home.mergeDown", "{}"),
    ("Merge horizontally", "home.mergeAcross", "{}"),
    ("Unmerge", "home.unmergeCells", "{}"),
];

// `on`: picking the alignment a cell already has keeps it (Sheets), not back to General.
const H_ALIGN: &[(&str, &str, &str)] = &[
    ("icon:format_align_left|Left", "home.alignLeft", r#"{"on":true}"#),
    ("icon:format_align_center|Centre", "home.alignCenter", r#"{"on":true}"#),
    ("icon:format_align_right|Right", "home.alignRight", r#"{"on":true}"#),
];

const V_ALIGN: &[(&str, &str, &str)] = &[
    ("icon:vertical_align_top|Top", "home.alignTop", "{}"),
    ("icon:vertical_align_center|Middle", "home.alignMiddle", "{}"),
    ("icon:vertical_align_bottom|Bottom", "home.alignBottom", "{}"),
];

// Sheets also has Clip; GridCraft has no clip mode yet.
const WRAP: &[(&str, &str, &str)] = &[
    ("icon:keyboard_tab|Overflow", "home.wrapText", r#"{"on":false}"#),
    ("icon:wrap_text|Wrap", "home.wrapText", r#"{"on":true}"#),
    ("icon:format_text_clip|Clip", "home.wrapText", r#"{"on":false,"clip":true}"#),
];

const ROTATION: &[(&str, &str, &str)] = &[
    ("icon:text_rotation_none|None", "home.orientation", r#"{"angle":0}"#),
    ("icon:text_rotation_angleup|Tilt up", "home.orientation", r#"{"angle":45}"#),
    ("icon:text_rotation_angledown|Tilt down", "home.orientation", r#"{"angle":-45}"#),
    ("icon:text_rotate_vertical|Stack vertically", "home.orientation", r#"{"angle":"vertical"}"#),
    ("icon:text_rotate_up|Rotate up", "home.orientation", r#"{"angle":90}"#),
    ("icon:text_rotation_down|Rotate down", "home.orientation", r#"{"angle":-90}"#),
];

const FUNCTIONS: &[(&str, &str, &str)] = &[
    ("SUM", "formulas.autoSum", r#"{"function":"SUM"}"#),
    ("AVERAGE", "formulas.autoSum", r#"{"function":"AVERAGE"}"#),
    ("COUNT", "formulas.autoSum", r#"{"function":"COUNT"}"#),
    ("MAX", "formulas.autoSum", r#"{"function":"MAX"}"#),
    ("MIN", "formulas.autoSum", r#"{"function":"MIN"}"#),
    ("-", "", ""),
    ("More functions…", "formulas.insertFunction", "{}"),
];

/// The font list, Sheets-like with Roboto first (Roboto is bundled; sans-serif names render in it
/// until more faces are bundled, serif and monospace names use built-in faces).
/// The font menu: the bundled Roboto, then the web fonts (app/webfonts.txt) A–Z, as in Sheets.
fn fonts() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = crate::fonts::WEB_FONTS.iter().map(|(menu, _)| *menu).collect();
    v.push("Roboto");
    v.sort_by_key(|f| f.to_lowercase());
    v
}

const FONT_SIZES: &[u32] = &[6, 7, 8, 9, 10, 11, 12, 14, 18, 24, 36];
const ZOOMS: &[u32] = &[50, 75, 90, 100, 125, 150, 200];

/// Per-session state of the chrome.
#[derive(Default)]
pub struct Shell {
    /// "Hide the menus" (Ctrl+Shift+F in Sheets): only the toolbar shows.
    pub compact: bool,
    /// The font size box while it's being typed in.
    size_draft: Option<String>,
    /// The spreadsheet's name while it's being renamed (click the title).
    title_draft: Option<String>,
    /// The custom colour picker (⊕ in the colour menus).
    custom: crate::colourpicker::CustomPicker,
    /// Format › Number › Custom currency.
    currency: crate::currency::CustomCurrency,
    /// Help › Keyboard shortcuts and Help › Search the menus.
    help: crate::shortcuts::Help,
    /// Smart Fill's suggestion card under the next empty cell of a column being typed.
    smart: crate::smartfill::SmartFill,
    /// The bar of the filter view open in this tab (Data › Filter views).
    views: filterviews::Bar,
    /// Borders menu: the colour and line style the next border is drawn with (to-do row 66).
    border_colour: Option<u32>,
    border_style: usize,
}

/// What the toolbar shows for the active cell.
struct Current {
    font: String,
    size: f32,
    zoom: u16,
    bold: bool,
    italic: bool,
    strike: bool,
}

impl Current {
    /// Whether the toolbar button that runs `id` is on for the active cell.
    fn on(&self, id: &str) -> bool {
        match id {
            "home.bold" => self.bold,
            "home.italic" => self.italic,
            "home.strikethrough" => self.strike,
            _ => false,
        }
    }
}

fn current(app: &SheetApp) -> Current {
    let d = app.session.active();
    let style = d.and_then(|d| {
        let sh = d.wb.active()?;
        Some(d.wb.styles.get(sh.style_id(d.selection.active)).clone())
    });
    let zoom = d.and_then(|d| d.wb.active().map(|sh| sh.zoom)).unwrap_or(100);
    match style {
        Some(st) => {
            Current { font: st.font.name.clone(), size: st.font.size, zoom, bold: st.font.bold, italic: st.font.italic, strike: st.font.strike }
        }
        None => Current { font: "Arial".into(), size: 10.0, zoom, bold: false, italic: false, strike: false },
    }
}

/// A menu entry's parameters; `@date`, `@time` and `@datetime` become the default formats from
/// File › Settings.
fn params(text: &str) -> serde_json::Value {
    let (date, time, datetime) = crate::settings::default_codes();
    let text =
        text.replace("\"@datetime\"", &format!("{datetime:?}")).replace("\"@date\"", &format!("{date:?}")).replace("\"@time\"", &format!("{time:?}"));
    serde_json::from_str(&text).unwrap_or_else(|_| json!({}))
}

/// Sheets-style flat buttons for the menu bar and toolbar: no border or fill at rest, a soft
/// rounded grey on hover (#DDE3EA), darker while pressed or while its menu is open (#C9D3DF),
/// the same as Starbase Docs. (A button drawn with `.frame(false)` would get no fill at all.)
fn flat(ui: &mut Ui) {
    use theme::{HOVER, PRESSED};
    let w = &mut ui.style_mut().visuals.widgets;
    for (state, fill) in [(&mut w.inactive, egui::Color32::TRANSPARENT), (&mut w.hovered, HOVER), (&mut w.active, PRESSED), (&mut w.open, PRESSED)] {
        state.bg_stroke = Stroke::NONE;
        state.weak_bg_fill = fill;
        state.corner_radius = egui::CornerRadius::same(4);
        state.expansion = 0.0;
    }
}

fn icon_or_text(ui: &mut Ui, name: &str, tip: &str) -> egui::Response {
    tool_button(ui, name, tip, false)
}

/// A toolbar button (inside [`flat`]): grey on hover and press; light blue (#D3E3FD) when `on`
/// (Bold on a bold cell), as in Google Sheets and Starbase Docs.
fn tool_button(ui: &mut Ui, name: &str, tip: &str, on: bool) -> egui::Response {
    let b = match icons::icon(name) {
        Some(img) => Button::image(img),
        None => Button::new(RichText::new(name).size(14.0).color(theme::text_dim())),
    };
    // A set fill replaces the hover and pressed fills too: only a button that is on has one.
    let b = if on { b.fill(if theme::photo_dark() { theme::drive_selection() } else { theme::ON }) } else { b };
    ui.add(b.min_size(vec2(28.0, 28.0))).on_hover_text(tip)
}

impl Shell {
    pub fn ui(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.custom.ui(app, &ctx);
        self.currency.ui(app, &ctx);
        if let Some(found) = self.help.ui(app, &ctx) {
            run_found(app, &ctx, &found);
        }
        sheets_keys(app, &ctx);
        self.smart.ui(app, &ctx);
        crate::formulaimages::fetch_wanted(&ctx);
        // Ctrl+Shift+F: hide/show the menus (taken before the grid sees it).
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::F)) {
            self.compact = !self.compact;
        }
        if !self.compact {
            // No separator lines: Sheets' title, menus and toolbar sit on one plain background.
            egui::Panel::top("starsheets_title")
                .show_separator_line(false)
                .exact_size(64.0)
                .frame(Frame::NONE.fill(theme::page()).inner_margin(Margin { left: 12, right: 16, top: 6, bottom: 0 }))
                .show(ui, |ui| title_and_menus(app, ui, &mut self.title_draft));
        }
        egui::Panel::top("starsheets_toolbar")
            .show_separator_line(false)
            .exact_size(48.0)
            .frame(Frame::NONE.fill(theme::page()).inner_margin(Margin { left: 12, right: 12, top: 4, bottom: 6 }))
            .show(ui, |ui| self.toolbar(app, ui));
        crate::external::bar(ui);
        self.views.ui(app, ui);
    }

    fn toolbar(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let cur = current(app);
        Frame::NONE
            .fill(if theme::photo_dark() { theme::drive_surface() } else { theme::toolbar() })
            .corner_radius(24.0)
            .inner_margin(Margin::symmetric(12, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_centered(|ui| {
                    flat(ui);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for tool in TOOLBAR {
                        match tool {
                            Btn(icon, tip, id, p) => {
                                if tool_button(ui, icon, tip, cur.on(id)).clicked() {
                                    if *id == "panel:chart" {
                                        ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(crate::chartpanel::REQUEST), true));
                                    } else {
                                        app.run_or_alert(id, params(p));
                                    }
                                }
                            }
                            Txt(text, tip, id, p) => {
                                let b = ui.add(Button::new(RichText::new(*text).size(15.0).color(theme::text_dim())).min_size(vec2(28.0, 28.0)));
                                if b.on_hover_text(*tip).clicked() {
                                    app.run_or_alert(id, params(p));
                                }
                            }
                            Menu(icon, tip, items) => {
                                let r = match icons::icon(icon) {
                                    Some(img) => ui.menu_button(img, |ui| tool_menu(app, ui, items)).response,
                                    None => {
                                        ui.menu_button(RichText::new(*icon).size(14.0).color(theme::text_dim()), |ui| tool_menu(app, ui, items))
                                            .response
                                    }
                                };
                                r.on_hover_text(*tip);
                            }
                            Colour(icon, tip, id, reset_label, reset_value) => {
                                let img = icons::icon(icon);
                                let _ = reset_label;
                                let body = |ui: &mut Ui| crate::colourpicker::menu(app, ui, id, reset_value);
                                let r = match img {
                                    Some(img) => ui.menu_button(img, body).response,
                                    None => ui.menu_button(*icon, body).response,
                                };
                                r.on_hover_text(*tip);
                            }
                            Borders => self.borders(app, ui),
                            Font => {
                                let label = RichText::new(format!("{}  ▾", cur.font)).size(13.0).color(theme::text());
                                ui.menu_button(label, |ui| {
                                    // Sheets' font menu: each name in its own font, a ✓ by the current one.
                                    menu_style_width(ui, 220.0);
                                    ui.spacing_mut().button_padding.x = 44.0;
                                    egui::ScrollArea::vertical().max_height(crate::house::menu_max_height(ui.ctx()) - 80.0).show(ui, |ui| {
                                        for f in fonts() {
                                            let base = egui::FontId::proportional(crate::house::BODY + 1.0);
                                            let font = match crate::fonts::preview_font(ui.ctx(), f, base.size) {
                                                // Capitals already match Roboto's height; also meet Roboto's width halfway,
                                                // so narrow fonts (Impact) and wide ones (Verdana) look about as big.
                                                Some(p) => {
                                                    let width = |id: egui::FontId| {
                                                        ui.painter().layout_no_wrap(f.to_string(), id, theme::text()).size().x.max(1.0)
                                                    };
                                                    let k = (width(base.clone()) / width(p.clone())).sqrt().clamp(0.85, 1.25);
                                                    egui::FontId::new(p.size * k, p.family)
                                                }
                                                None => base,
                                            };
                                            let r = ui.add(
                                                Button::new(RichText::new(f).font(font).color(theme::text()))
                                                    .shortcut_text("")
                                                    .frame_when_inactive(false)
                                                    .min_size(vec2(220.0, crate::house::ROW_HEIGHT)),
                                            );
                                            if f == cur.font {
                                                ui.painter().text(
                                                    r.rect.left_center() + vec2(16.0, 0.0),
                                                    egui::Align2::LEFT_CENTER,
                                                    "✓",
                                                    egui::FontId::proportional(16.0),
                                                    theme::text(),
                                                );
                                            }
                                            if r.clicked() {
                                                app.run_or_alert("home.fontName", json!({ "name": f }));
                                                ui.close();
                                            }
                                        }
                                    });
                                })
                                .response
                                .on_hover_text("Font");
                            }
                            FontSize => self.font_size(app, ui, cur.size),
                            Zoom => {
                                ui.menu_button(RichText::new(format!("{}%  ▾", cur.zoom)).size(13.0).color(theme::text()), |ui| {
                                    let labels: Vec<String> = ZOOMS.iter().map(|z| format!("{z}%")).collect();
                                    let now = ZOOMS.iter().position(|z| *z == u32::from(cur.zoom));
                                    if let Some(i) = pick_rows(ui, 120.0, &labels, now) {
                                        app.run_or_alert("view.zoom", json!({ "percent": ZOOMS[i] }));
                                    }
                                })
                                .response
                                .on_hover_text("Zoom");
                            }
                            Gap => {
                                ui.add_space(4.0);
                                let (r, _) = ui.allocate_exact_size(vec2(1.0, 20.0), Sense::hover());
                                ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, theme::hover()));
                                ui.add_space(4.0);
                            }
                        }
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (icon, tip) = if self.compact {
                            ("keyboard_arrow_down", "Show the menus (Ctrl+Shift+F)")
                        } else {
                            ("keyboard_arrow_up", "Hide the menus (Ctrl+Shift+F)")
                        };
                        if icon_or_text(ui, icon, tip).clicked() {
                            self.compact = !self.compact;
                        }
                    });
                });
            });
    }

    /// Sheets' borders menu: the border presets on the left; on the right the colour and the
    /// line style (thin, medium, thick, dashed, dotted, double) that the next preset uses.
    fn borders(&mut self, app: &mut SheetApp, ui: &mut Ui) {
        let body = |ui: &mut Ui| {
            flat(ui);
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.horizontal_top(|ui| {
                egui::Grid::new("starsheets_border_presets").spacing(vec2(4.0, 4.0)).show(ui, |ui| {
                    for (i, (label, id, p)) in BORDERS.iter().enumerate() {
                        let (name, tip) = label.trim_start_matches("icon:").split_once('|').unwrap_or((label, ""));
                        if icon_or_text(ui, name, tip).clicked() {
                            let mut params = params(p);
                            params["style"] = json!(BORDER_STYLES[self.border_style].1);
                            if let Some(rgb) = self.border_colour {
                                params["color"] = json!(format!("#{rgb:06X}"));
                            }
                            app.run_or_alert(id, params);
                        }
                        if (i + 1) % 5 == 0 {
                            ui.end_row();
                        }
                    }
                });
                let (r, _) = ui.allocate_exact_size(vec2(1.0, 64.0), Sense::hover());
                ui.painter().vline(r.center().x, r.y_range(), Stroke::new(1.0, theme::divider()));
                ui.vertical(|ui| {
                    // The colour button shows the colour in a bar under the pencil.
                    let colour =
                        self.border_colour.map_or(theme::TEXT, |rgb| egui::Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8));
                    let cr = match icons::icon("border_color_pen") {
                        Some(img) => ui.menu_button(img, |ui| match crate::colourpicker::palette_open(ui, self.border_colour, false, false) {
                            Some(crate::colourpicker::Choice::Colour(rgb)) => self.border_colour = Some(rgb),
                            Some(crate::colourpicker::Choice::Reset) => self.border_colour = None,
                            _ => {}
                        }),
                        None => ui.menu_button("Colour", |_| {}),
                    }
                    .response;
                    // Where border_color's own bar sits in the 18 px icon (the icon starts one button
                    // padding in from the left, centred top to bottom): 1.5-16.5 px across, the
                    // bottom 3 px (to-do row 77).
                    let (x, y) = (cr.rect.left() + ui.spacing().button_padding.x, cr.rect.center().y - 9.0);
                    ui.painter().rect_filled(egui::Rect::from_min_max(egui::pos2(x + 1.5, y + 15.0), egui::pos2(x + 16.5, y + 18.0)), 0.0, colour);
                    cr.on_hover_text("Border colour");
                    let sr = match icons::icon("line_style") {
                        Some(img) => ui.menu_button(img, |ui| {
                            menu_style_width(ui, 120.0);
                            for (k, (name, _)) in BORDER_STYLES.iter().enumerate() {
                                let r =
                                    ui.add_sized([120.0, crate::house::ROW_HEIGHT], Button::new("").frame_when_inactive(false)).on_hover_text(*name);
                                line_sample(ui, r.rect.shrink2(vec2(28.0, 0.0)), k, self.border_style == k);
                                if r.clicked() {
                                    self.border_style = k;
                                }
                            }
                        }),
                        None => ui.menu_button("Style", |_| {}),
                    }
                    .response;
                    sr.on_hover_text("Border style");
                });
            });
        };
        // Picking a preset, colour or line style keeps the menus open (to-do 2 row 26): they close
        // only on a click outside. The submenus inherit this from the outer menu.
        let stay = egui::containers::menu::MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let r = match icons::icon("border_all") {
            Some(img) => egui::containers::menu::MenuButton::new(img).config(stay).ui(ui, body).0,
            None => egui::containers::menu::MenuButton::new("Borders").config(stay).ui(ui, body).0,
        };
        r.on_hover_text("Borders");
    }

    /// Sheets' size box: − [10] +, type a size and press Enter; the arrow lists common sizes.
    fn font_size(&mut self, app: &mut SheetApp, ui: &mut Ui, size: f32) {
        let shown = if size.fract() == 0.0 { format!("{}", size as i32) } else { format!("{size}") };
        if icon_or_text(ui, "remove", "Decrease font size").clicked() {
            app.run_or_alert("home.decreaseFontSize", json!({}));
        }
        let mut text = self.size_draft.clone().unwrap_or_else(|| shown.clone());
        // Sheets' size box: the number and its ▾ inside one thin grey box (to-do row 68).
        let (r, picked) = Frame::NONE
            .stroke(Stroke::new(1.0, theme::BOX))
            .corner_radius(f32::from(crate::house::FIELD_RADIUS))
            .inner_margin(Margin { left: 2, right: 0, top: 0, bottom: 0 })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut text)
                            .frame(Frame::NONE)
                            .desired_width(26.0)
                            .min_size(vec2(26.0, 24.0))
                            .align(egui::Align2::CENTER_CENTER),
                    );
                    let mut picked = None;
                    ui.spacing_mut().button_padding.x = 3.0;
                    ui.menu_button(RichText::new("▾").size(11.0).color(theme::text_dim()), |ui| {
                        let labels: Vec<String> = FONT_SIZES.iter().map(u32::to_string).collect();
                        let now = FONT_SIZES.iter().position(|s| f64::from(*s) == f64::from(size));
                        picked = pick_rows(ui, 96.0, &labels, now).map(|i| FONT_SIZES[i]);
                    })
                    .response
                    .on_hover_text("Font sizes");
                    (r, picked)
                })
                .inner
            })
            .inner;
        if let Some(s) = picked {
            app.run_or_alert("home.fontSize", json!({ "size": s }));
        }
        if r.changed() {
            self.size_draft = Some(text.clone());
        }
        if r.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Ok(v) = text.trim().parse::<f64>()
            {
                app.run_or_alert("home.fontSize", json!({ "size": v }));
            }
            self.size_draft = None;
        }
        r.on_hover_text("Font size");
        if icon_or_text(ui, "add", "Increase font size").clicked() {
            app.run_or_alert("home.increaseFontSize", json!({}));
        }
    }
}

/// A toolbar dropdown, Sheets style: a strip of icons (borders, alignment, wrapping, rotation)
/// or a list whose entries may show a grey example on the right ("Currency|£1,000.12").
fn tool_menu(app: &mut SheetApp, ui: &mut Ui, items: &[(&str, &str, &str)]) {
    let icon_row = items.iter().all(|(l, _, _)| l.starts_with("icon:"));
    let run = |app: &mut SheetApp, ui: &mut Ui, id: &str, p: &str| {
        if id.starts_with("panel:") {
            run_entry_with(app, ui.ctx(), id, params(p));
        } else {
            app.run_or_alert(id, params(p));
        }
        ui.close();
    };
    if icon_row {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        flat(ui);
        let per_row = if items.len() > 6 { 5 } else { items.len() };
        egui::Grid::new(ui.scope_id().with("strip")).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
            for (i, (label, id, p)) in items.iter().enumerate() {
                let (name, tip) = label.trim_start_matches("icon:").split_once('|').unwrap_or((label.trim_start_matches("icon:"), ""));
                let tip = if tip.is_empty() { name } else { tip };
                if icon_or_text(ui, name, tip).clicked() {
                    run(app, ui, id, p);
                }
                if (i + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
        });
        return;
    }
    let width = menu_style(ui, false).max(if items.iter().any(|(l, _, _)| l.contains('|')) { 280.0 } else { 0.0 });
    ui.set_min_width(width);
    ui.set_max_width(width);
    for (label, id, p) in items {
        if *label == "-" {
            menu_sep_width(ui, width);
            continue;
        }
        let (name, sample) = label.split_once('|').unwrap_or((label, ""));
        if is_merge(id) {
            let on = crate::merge::enabled(app, id);
            if ui.add_enabled_ui(on, |ui| ui.add_sized([width, 32.0], menu_row(name, sample, false))).inner.clicked() {
                ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(crate::merge::REQUEST), id.to_string()));
                ui.close();
            }
        } else if ui.add_sized([width, 32.0], menu_row(name, sample, false)).clicked() {
            run(app, ui, id, p);
        }
    }
}

/// The merge commands, which go through the "Heads up!" warning (merge.rs).
fn is_merge(id: &str) -> bool {
    matches!(id, "home.mergeCells" | "home.mergeDown" | "home.mergeAcross" | "home.unmergeCells")
}

/// The name in the title row: the file's name without its extension ("Budget", not
/// "Budget.xlsx"), as Sheets shows it; Drive's details are where the extension shows.
pub(crate) fn header_name(app: &SheetApp) -> String {
    let Some(d) = app.session.active() else { return "Untitled spreadsheet".into() };
    if !d.title.trim().is_empty() {
        return d.title.clone();
    }
    let shown = d.display_title();
    std::path::Path::new(&shown).file_stem().map_or(shown.clone(), |s| s.to_string_lossy().into_owned())
}

fn title_and_menus(app: &mut SheetApp, ui: &mut Ui, title_draft: &mut Option<String>) {
    let name = header_name(app);
    let dirty = app.session.active().is_some_and(|d| d.is_dirty());
    ui.horizontal(|ui| {
        // Our own mark (a sheet with a star on a green tile; the favicon is a simplified copy).
        // A click goes home (files.rs), as Sheets' logo does.
        let logo = match icons::logo(36.0) {
            Some(img) => ui.add(img.sense(egui::Sense::click())),
            None => ui.add(egui::Label::new(RichText::new("★").size(26.0).color(theme::BLUE)).sense(egui::Sense::click())),
        };
        if logo.on_hover_text("Starbase Drive").clicked() {
            ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_files_request"), "home".to_string()));
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.horizontal(|ui| {
                rename_box(app, ui, &name, title_draft);
            });
            ui.horizontal(|ui| {
                flat(ui);
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.spacing_mut().button_padding = vec2(7.0, 2.0);
                for (title, items) in MENUS {
                    let r = ui.menu_button(RichText::new(*title).size(14.0).color(theme::text()), |ui| menu_items(app, ui, items)).response;
                    house::target(ui.ctx(), &format!("menu:{title}"), r.rect);
                }
            });
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // From the right: your avatar (with its menu), who else is here (to-do 3 row 9), Share,
            // Files, then the save symbol (to-do 3 row 8).
            #[cfg(target_arch = "wasm32")]
            {
                crate::account::me_ui(ui);
                ui.add_space(8.0);
                if !crate::live::others().is_empty() {
                    crate::account::others_ui(ui);
                    ui.add_space(8.0);
                }
            }
            // Share with a lock, as in Sheets.
            #[cfg(target_arch = "wasm32")]
            let share = {
                let label = RichText::new("Share").size(14.0).color(theme::text());
                match icons::icon("lock") {
                    Some(img) => Button::image_and_text(img.fit_to_exact_size(vec2(18.0, 18.0)), label),
                    None => Button::new(label),
                }
                .fill(theme::drive_selection())
                .corner_radius(18.0)
                .min_size(vec2(96.0, 36.0))
            };
            // Files: back to the home screen (the file organiser), in this file's folder. The
            // logo at the left does the same.
            #[cfg(target_arch = "wasm32")]
            let files_button = {
                let label = RichText::new("Files").size(14.0).color(theme::text());
                match icons::icon("folder") {
                    Some(img) => Button::image_and_text(img.fit_to_exact_size(vec2(20.0, 20.0)), label),
                    None => Button::new(label),
                }
                .fill(theme::card())
                .stroke(Stroke::new(1.0, theme::BOX))
                .corner_radius(18.0)
                .min_size(vec2(92.0, 36.0))
            };
            // The desktop app has nothing to share with (its files are on this computer), so it
            // shows no Share button, as desktop Docs doesn't.
            #[cfg(target_arch = "wasm32")]
            {
                let can = crate::share::can_share();
                if ui.add_enabled(can, share).on_disabled_hover_text("Sharing needs sign-in on the server").clicked() {
                    ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(crate::share::REQUEST), ()));
                }
                ui.add_space(8.0);
                if ui.add(files_button).on_hover_text("Your files and folders, in a new tab").clicked() {
                    ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_files_request"), "home_tab".to_string()));
                }
                ui.add_space(8.0);
            }
            save_symbol(ui, dirty);
        });
    });
}

/// Where the open file's saving stands, for the save symbol and the test hook. The words differ
/// by place ("Saved", "Saved to this computer") but the state doesn't, so nothing matches on the
/// words (looks review 2 item 4: desktop showed a red dot on a saved file).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// The live editing states are only made in the web app (live.rs); the desktop app has none.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code, reason = "live editing states are web-only"))]
pub enum SaveState {
    Saved,
    Saving,
    /// Live editing: this person may only read the file.
    ViewOnly,
    /// Live editing: not connected yet, nothing waiting to be sent.
    Connecting,
    /// Live editing: not connected, with changes waiting to be sent.
    Offline,
    /// Not saved, or no access any more.
    Problem,
}

/// The state and the words the save symbol shows (files.rs, live.rs and native.rs set it).
#[derive(Clone, Debug, PartialEq)]
pub struct SaveStatus {
    pub state: SaveState,
    pub text: String,
}

impl SaveStatus {
    pub fn new(state: SaveState, text: impl Into<String>) -> SaveStatus {
        SaveStatus { state, text: text.into() }
    }
}

/// The egui memory key holding a [`SaveStatus`].
pub const SAVE_STATUS: &str = "starsheets_save_status";

/// Sets the save symbol's status; `None` leaves it to the workbook's own changed flag.
pub fn set_save_status(ctx: &egui::Context, status: Option<SaveStatus>) {
    let id = egui::Id::unique(SAVE_STATUS);
    ctx.data_mut(|m| {
        if let Some(s) = status {
            m.insert_temp(id, s);
        } else {
            m.remove::<SaveStatus>(id);
        }
    });
}

/// The save symbol's status: what was set, else saved or not by the workbook's changed flag.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code, reason = "the web test hook's only"))]
pub fn save_status(ctx: &egui::Context, dirty: bool) -> SaveStatus {
    let set = ctx.data(|m| m.get_temp::<SaveStatus>(egui::Id::unique(SAVE_STATUS)));
    set.unwrap_or_else(|| if dirty { SaveStatus::new(SaveState::Problem, "Not saved") } else { SaveStatus::new(SaveState::Saved, "Saved") })
}

/// The save-state icon left of Files (to-do 3 row 38, shared with Docs and Slides): an ochre
/// floppy while unsaved or saving, a green clock once saved; hover shows a short card (when,
/// who, "Click for version history"). A click opens version history.
fn save_symbol(ui: &mut Ui, dirty: bool) {
    let info = save_info(ui.ctx(), dirty);
    let pal = starbase_savestate::SavePalette::house(theme::text_dim(), theme::page());
    let out = starbase_savestate::save_icon(ui, &info, &pal);
    crate::house::target(ui.ctx(), "save-icon", out.response.rect);
    if let Some(card) = out.card {
        crate::house::target(ui.ctx(), "save-icon-card", card);
    }
    if out.response.clicked() && info.history {
        ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_files_request"), "versions".to_string()));
    }
}

/// What the save-state icon shows: the [`SaveStatus`] with when and who (files.rs, native.rs).
pub fn save_info(ctx: &egui::Context, dirty: bool) -> starbase_savestate::SaveInfo {
    use starbase_savestate::{SaveInfo, SaveKind};
    let get = |key: &str| ctx.data(|m| m.get_temp::<String>(egui::Id::unique(key))).unwrap_or_default();
    let set = ctx.data(|m| m.get_temp::<SaveStatus>(egui::Id::unique(SAVE_STATUS)));
    let local = set.as_ref().is_some_and(|s| s.text.contains("this computer"));
    let (kind, detail) = match set {
        None if dirty => (SaveKind::Unsaved, String::new()),
        None => (SaveKind::Saved, String::new()),
        Some(s) => match s.state {
            SaveState::Saved => (SaveKind::Saved, get("starsheets_last_edit")),
            SaveState::Saving => (SaveKind::Saving, String::new()),
            SaveState::ViewOnly => (SaveKind::ViewOnly, get("starsheets_last_edit")),
            SaveState::Connecting => (SaveKind::Offline, "Connecting…".to_string()),
            SaveState::Offline => (SaveKind::Offline, String::new()),
            SaveState::Problem => (SaveKind::Problem, s.text.trim_start_matches("Not saved").trim_start_matches(':').trim().to_string()),
        },
    };
    let when = Some(get(SAVE_WHEN)).filter(|w| !w.is_empty());
    SaveInfo { kind, when, version: None, detail, local, history: !local }
}

/// The egui memory key holding when the file was last saved, in words.
pub const SAVE_WHEN: &str = "starsheets_save_when";

/// The Material Symbols icon Sheets shows beside a menu item (docs/sheets-menus.md).
pub const MENU_ICONS: &[(&str, &str)] = &[
    ("New spreadsheet", "note_add"),
    ("New from a template…", "note_add"),
    ("Open", "folder_open"),
    ("Import", "upload"),
    ("Make a copy", "content_copy"),
    ("Move to bin", "delete"),
    ("Version history", "history"),
    ("Download", "download"),
    ("Details", "info"),
    ("Settings", "settings"),
    ("Print", "print"),
    ("Undo", "undo"),
    ("Redo", "redo"),
    ("Cut", "content_cut"),
    ("Copy", "content_copy"),
    ("Paste", "content_paste"),
    ("Paste special", "content_paste"),
    ("Delete", "delete"),
    ("Find and replace", "find_replace"),
    ("Show", "visibility"),
    ("Freeze", "grid_on"),
    ("Group", "add_box"),
    ("Zoom", "zoom_in"),
    ("Cells", "check_box_outline_blank"),
    ("Rows", "table_rows"),
    ("Columns", "view_column"),
    ("Sheet", "tab"),
    ("Chart", "insert_chart"),
    ("Pivot table", "pivot_table_chart"),
    ("Image", "image"),
    ("Function", "functions"),
    ("Link", "link"),
    ("Comment", "add_comment"),
    ("Note", "sticky_note_2"),
    ("Table", "table"),
    ("Number", "numbers_123"),
    ("Text", "format_bold"),
    ("Alignment", "format_align_left"),
    ("Wrapping", "wrap_text"),
    ("Merge cells", "cell_merge"),
    ("Convert to table", "table"),
    ("Conditional formatting", "format_paint"),
    ("Alternating colours", "format_color_fill"),
    ("Clear formatting", "format_clear"),
    ("Sort range A → Z", "sort_by_alpha"),
    ("Sort range Z → A", "sort_by_alpha"),
    ("Advanced range sorting options", "sort"),
    ("Create a filter", "filter_alt"),
    ("Add a slicer", "filter_list"),
    ("Protect sheets and ranges", "lock"),
    ("Named ranges", "label"),
    ("Data validation", "rule"),
    ("Split text to columns", "call_split"),
    ("Spelling", "spellcheck"),
    ("Accessibility", "accessibility_new"),
    ("Calculate now", "calculate"),
    ("Function list", "functions"),
    ("Rotation", "text_rotation_angleup"),
    ("Font size", "format_size"),
    ("Sort range", "sort"),
    ("Sort sheet", "sort_by_alpha"),
    ("Remove filter", "filter_alt"),
    ("Filter views", "filter_list"),
    ("Create new filter view", "add"),
    ("Save as filter view", "filter_alt"),
    ("Data clean-up", "cleaning_services"),
    ("Calculation controls", "functions"),
    ("Tick box", "check_box"),
    // Right-click menus (cells, row and column headers, sheet tabs).
    ("Insert cells", "add"),
    ("Delete cells", "delete"),
    ("Delete row", "delete"),
    ("Delete column", "delete"),
    ("Clear row", "close"),
    ("Clear column", "close"),
    ("Hide row", "visibility_off"),
    ("Hide column", "visibility_off"),
    ("Unhide rows", "visibility"),
    ("Unhide columns", "visibility"),
    ("Resize the row", "aspect_ratio"),
    ("Resize the column", "aspect_ratio"),
    ("Sort sheet A to Z", "sort_by_alpha"),
    ("Sort sheet Z to A", "sort_by_alpha"),
    ("Edit chart", "edit"),
    ("Delete chart", "delete"),
    ("Delete", "delete"),
    ("Insert link", "link"),
    ("Insert note", "sticky_note_2"),
    ("Drop-down", "arrow_drop_down_circle"),
    ("Filter by cell value", "filter_list"),
];

fn menu_icon(label: &str) -> Option<egui::Image<'static>> {
    let exact = MENU_ICONS.iter().find(|(l, _)| *l == label).map(|(_, n)| *n);
    // GridCraft's counted labels ("Insert 3 rows above", "Delete rows 3 - 5", patch 0015).
    let name = exact.or_else(|| {
        if label.starts_with("Insert ") && (label.contains(" row") || label.contains(" column")) {
            Some("add")
        } else if label.starts_with("Delete rows") || label.starts_with("Delete columns") {
            Some("delete")
        } else {
            None
        }
    })?;
    icons::icon(name)
}

/// Grey for shortcuts and the submenu arrow, as in Sheets.
const MENU_GREY: egui::Color32 = egui::Color32::from_rgb(0x80, 0x86, 0x8B);

/// One menu row: icon (or the same space blank when the menu has icons), 14 px label, and the
/// shortcut in grey on the right. Rows are 32 px tall and fill the menu's width.
pub fn menu_row(label: &str, keys: &str, with_icons: bool) -> Button<'static> {
    let text = RichText::new(label.to_string()).size(14.0).color(theme::text());
    let b = match (menu_icon(label).filter(|_| with_icons), with_icons) {
        (Some(img), _) => Button::image_and_text(img, text),
        // Keeps the label lined up with its iconed neighbours.
        (None, true) => match icons::icon("add") {
            Some(blank) => Button::image_and_text(blank.tint(egui::Color32::TRANSPARENT), text),
            None => Button::new(text),
        },
        (None, false) => Button::new(text),
    };
    // Always add the (maybe empty) shortcut: its stretch keeps the label at the left.
    let b = b.shortcut_text(RichText::new(keys.to_string()).size(13.0).color(MENU_GREY));
    b.frame_when_inactive(false).min_size(vec2(0.0, 32.0))
}

/// Sheets-style menu: flat full-width rows with icons, grey shortcuts, ▸ for submenus, and
/// dividers with breathing room (docs/sheets-menus.md).
/// Sets a menu `Ui` up the Sheets way (flat full-width rows, grey hover) and returns its width:
/// ~320 px with icons, 220 px for plain submenus.
pub fn menu_style(ui: &mut Ui, with_icons: bool) -> f32 {
    let width = if with_icons { 320.0 } else { 220.0 };
    menu_style_width(ui, width);
    width
}

/// The Sheets menu look for a menu of any width (house.rs: 16 px side padding, 32 px rows).
pub fn menu_style_width(ui: &mut Ui, width: f32) {
    crate::house::menu(ui, width);
}

/// A toolbar drop-down list in the house style (docs/house-style.md, "Drop-down lists"): 32 px
/// rows, 16 px side padding, a ✓ by the current entry. Closes the menu on a pick.
pub fn pick_rows(ui: &mut Ui, width: f32, items: &[String], current: Option<usize>) -> Option<usize> {
    pick_rows_from(ui, width, items, current, 40.0)
}

/// [`pick_rows`] with the labels starting `text_x` from the left: 48 lines them up with the
/// labels of a menu with icons (Data › Filter views lists its views under two such rows).
pub fn pick_rows_from(ui: &mut Ui, width: f32, items: &[String], current: Option<usize>, text_x: f32) -> Option<usize> {
    menu_style_width(ui, width);
    ui.spacing_mut().button_padding.x = text_x;
    let mut hit = None;
    for (i, label) in items.iter().enumerate() {
        let r = ui.add(
            Button::new(RichText::new(label).size(crate::house::BODY).color(theme::text()))
                .frame_when_inactive(false)
                .min_size(vec2(width, crate::house::ROW_HEIGHT)),
        );
        if Some(i) == current {
            ui.painter().text(
                r.rect.left_center() + vec2(14.0, 0.0),
                egui::Align2::LEFT_CENTER,
                "✓",
                egui::FontId::proportional(16.0),
                theme::text(),
            );
        }
        if r.clicked() {
            hit = Some(i);
            ui.close();
        }
    }
    hit
}

/// A Sheets menu divider: a light line with 8 px above and below.
pub fn menu_sep_width(ui: &mut Ui, width: f32) {
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, theme::divider()));
    ui.add_space(8.0);
}

/// GridCraft's cell and header menus drawn with our rows (patch 0015).
pub const MENU_LOOK: gridcraft_ui_egui::MenuLook = gridcraft_ui_egui::MenuLook {
    style: |ui| menu_style(ui, true),
    row: |label, keys| menu_row(label, keys, true),
    // As wide as the menu itself (320 for cell menus; a drop-down list is only as wide as its
    // choices, and a fixed 320 made it wider than its rows' highlight).
    sep: |ui| menu_sep_width(ui, ui.min_rect().width().max(1.0)),
    list_style,
    // The empty shortcut's stretch keeps the label at the left (as in menu_row).
    list_row: |label| Button::new(RichText::new(label.to_string()).size(14.0).color(theme::text())).shortcut_text(""),
    list_item: Some(list_item),
};

/// A data-validation drop-down list: only as wide as its longest choice, plus the 16 px row
/// padding each side and room for the scroll bar (James, to-do row 75).
fn list_style(ui: &mut Ui, items: &[String]) -> f32 {
    // "Edit rule" sits under the choices (patch 0074): the list is at least as wide as it. Room
    // too for a tick box and a chip's padding (list_item).
    let width = crate::house::fit_width(ui, items.iter().map(String::as_str).chain(["Edit rule"]), false) + LIST_EXTRA;
    menu_style_width(ui, width);
    width
}

/// What a drop-down row needs beyond its text: a tick box and the gap after it, and a chip's
/// padding.
const LIST_EXTRA: f32 = LIST_TICK + house::M + 2.0 * LIST_CHIP_PAD;
const LIST_TICK: f32 = 20.0;
const LIST_CHIP_PAD: f32 = 10.0;
const LIST_CHIP_HEIGHT: f32 = 24.0;
/// Google's grey for a choice without a colour of its own (GridCraft's chips use the same).
const CHIP_GREY: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xEA, 0xED);

/// One drop-down choice, as in Google Sheets: for multiple selections a tick box (blue when
/// ticked), then the choice as its coloured chip (grey without a colour). A choice of a rule
/// without chips and without a colour is plain text. Grey behind the row on hover or keyboard.
fn list_item(ui: &mut Ui, it: &gridcraft_ui_egui::ListItem<'_>) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(it.size, egui::Sense::click());
    house::target(ui.ctx(), &format!("list:{}", it.text), rect);
    if r.hovered() || it.selected {
        ui.painter().rect_filled(rect, 0.0, theme::menu_hover());
    }
    let mut x = rect.left() + f32::from(house::MENU_INSET);
    if let Some(ticked) = it.ticked {
        let tick = egui::Rect::from_min_size(egui::pos2(x, rect.center().y - LIST_TICK / 2.0), vec2(LIST_TICK, LIST_TICK));
        let icon = if ticked { icons::icon_in("check_box", house::BLUE) } else { icons::icon("check_box_outline_blank") };
        if let Some(img) = icon {
            img.paint_at(ui, tick);
        }
        x += LIST_TICK + house::M;
    }
    let p = ui.painter().with_clip_rect(rect);
    let font = egui::FontId::proportional(house::BODY);
    if it.chips || it.color.is_some() {
        let fill = it.color.unwrap_or(CHIP_GREY);
        // Dark text on a light chip, white on a dark one.
        let light = 0.299 * f32::from(fill.r()) + 0.587 * f32::from(fill.g()) + 0.114 * f32::from(fill.b()) >= 150.0;
        let ink = if light { theme::TEXT } else { egui::Color32::WHITE };
        let galley = p.layout_no_wrap(it.text.to_string(), font, ink);
        let pill = egui::Rect::from_min_size(
            egui::pos2(x, rect.center().y - LIST_CHIP_HEIGHT / 2.0),
            vec2(galley.size().x + 2.0 * LIST_CHIP_PAD, LIST_CHIP_HEIGHT),
        );
        p.rect_filled(pill, LIST_CHIP_HEIGHT / 2.0, fill);
        p.galley(egui::pos2(pill.left() + LIST_CHIP_PAD, pill.center().y - galley.size().y / 2.0), galley, ink);
    } else {
        p.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, it.text, font, theme::text());
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, it.ticked.unwrap_or(it.selected), it.text));
    r
}

/// The spreadsheet's name: click it to rename, like Sheets (Enter keeps, Esc cancels).
fn rename_box(app: &mut SheetApp, ui: &mut Ui, name: &str, draft: &mut Option<String>) {
    let Some(text) = draft.as_mut() else {
        let r = ui.add(Button::new(RichText::new(name).size(18.0).color(theme::text())).frame_when_inactive(false)).on_hover_text("Rename");
        if r.clicked() {
            *draft = Some(name.to_string());
        }
        return;
    };
    let r = ui.add(egui::TextEdit::singleline(text).font(egui::FontId::proportional(18.0)).desired_width(260.0));
    if !r.has_focus() && !r.lost_focus() {
        r.request_focus();
    }
    // Like Sheets: the whole name starts selected, so typing replaces it.
    if r.gained_focus()
        && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id)
    {
        let all = egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
        state.cursor.set_char_range(Some(all));
        state.store(ui.ctx(), r.id);
    }
    if r.lost_focus() {
        // The Enter that confirms the name is used up here, or the grid would read it as its
        // own Enter and start editing the active cell.
        ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { key: egui::Key::Enter, .. })));
        let new = text.trim().to_string();
        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && !new.is_empty() && new != name {
            rename_doc(app, &new);
        }
        *draft = None;
    }
}

/// Renames the active spreadsheet: the title, and a file keeps its folder and extension (the
/// download uses the new name; the desktop app renames the file on disk, native.rs).
fn rename_doc(app: &mut SheetApp, new: &str) {
    let Some(d) = app.session.active_mut() else { return };
    d.title = new.to_string();
    if let Some(p) = d.path.as_deref() {
        let p = std::path::Path::new(p);
        let has_ext = std::path::Path::new(new).extension().is_some();
        let file = match p.extension() {
            Some(e) if !has_ext => format!("{new}.{}", e.to_string_lossy()),
            _ => new.to_string(),
        };
        d.path = Some(p.with_file_name(file).to_string_lossy().into_owned());
    }
}

/// Does what choosing a menu entry with this id does: `panel:` and `ui:` ids open our panels and
/// GridCraft's dialogs, Copy/Cut/Paste go through the clipboard, anything else runs as a command.
/// The test hook's `run` takes this path too (testhook.rs), so tests do what the menus do.
pub fn run_entry(app: &mut SheetApp, ctx: &egui::Context, id: &str) {
    run_entry_with(app, ctx, id, json!({}));
}

/// `run_entry` with the parameters a menu entry carries (Freeze 2 rows, Sort sheet Z to A…).
pub fn run_entry_with(app: &mut SheetApp, ctx: &egui::Context, id: &str, params: serde_json::Value) {
    if crate::menuactions::handles(id) {
        return run_steps(app, id, &params);
    }
    if filterviews::handles(id) {
        return filterviews::run(app, id, &params);
    }
    match id {
        "panel:smartFill" => return crate::smartfill::accept(app),
        "panel:smartFillToggle" => return crate::smartfill::set_off(!crate::smartfill::off()),
        // With parameters (the test hook) it applies them; from the menu it opens the panel.
        "panel:alternatingColours" if params.as_object().is_some_and(|o| !o.is_empty()) => {
            if let Err(e) = crate::altcolours::run(app, &params) {
                app.message = Some(("Alternating colours".into(), e));
            }
            return;
        }
        "panel:alternatingColours" => {
            ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::altcolours::REQUEST), true));
            return;
        }
        _ => {}
    }
    if !id.starts_with("panel:") && !id.starts_with("ui:") && !matches!(id, "edit.copy" | "edit.cut" | "edit.paste") {
        return app.run_or_alert(id, params);
    }
    let request = match id {
        "panel:customcurrency" => Some(crate::currency::REQUEST),
        "panel:shortcuts" => Some(crate::shortcuts::SHORTCUTS_REQUEST),
        "panel:searchMenus" => Some(crate::shortcuts::SEARCH_REQUEST),
        _ => None,
    };
    if let Some(key) = request {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(key), true));
        return;
    }
    // "ui:<dialog>" opens one of GridCraft's dialogs (some, like conditional
    // formatting, are then shown as our own side panel).
    // "panel:dropdown" opens the data-validation panel on a new drop-down.
    if id == "panel:customdate" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::customdate::REQUEST), true));
    } else if id == "panel:settings" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::settings::REQUEST), true));
    } else if id == "panel:comment" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::notes::COMMENT_REQUEST), true));
    } else if id == "panel:note" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::notes::REQUEST), true));
    } else if id == "panel:protect" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::protectpanel::REQUEST), -1i64));
    } else if id == "panel:customnumber" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::customnumber::REQUEST), true));
    } else if id == "panel:fullscreen" {
        toggle_fullscreen();
    } else if id == "panel:chart" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::chartpanel::REQUEST), true));
    } else if id == "panel:dropdown" {
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique(crate::dvpanel::DROPDOWN_REQUEST), true));
    } else if id == "panel:imageInCell" || id == "panel:imageOverCells" {
        #[cfg(target_arch = "wasm32")]
        crate::images::pick(if id == "panel:imageInCell" { "cell" } else { "overCells" });
    } else if id == "panel:insertBelow" || id == "panel:insertRight" {
        gridcraft_ui_egui::grid::insert_after(app, id == "panel:insertBelow");
    } else if let Some(what) = id.strip_prefix("panel:") {
        // File actions (files.rs reads this from egui memory).
        ctx.data_mut(|m| m.insert_temp(egui::Id::unique("starsheets_files_request"), what.to_string()));
    } else {
        match id.strip_prefix("ui:") {
            Some(dialog) => app.open_dialog(dialog, json!({})),
            // Through the clipboard, as the cell menu does: Copy/Cut also fill the
            // system clipboard, Paste takes the last copy (cells or text).
            None if matches!(id, "edit.copy" | "edit.cut") => app.copy_to_clipboard(ctx, id),
            None if id == "edit.paste" => app.paste_from_clipboard(),
            None => app.run_or_alert(id, json!({})),
        }
    }
}

/// Sheets' keys that GridCraft doesn't have, taken before the grid sees them: Ctrl+/ (keyboard
/// shortcuts), Alt+/ (search the menus), Ctrl+Alt+M (comment), Ctrl+Shift+Y (Smart Fill) and
/// Ctrl+Shift+V (paste values only).
fn sheets_keys(app: &mut SheetApp, ctx: &egui::Context) {
    use egui::{Key, KeyboardShortcut, Modifiers};
    let (help, search, comment, smart) = ctx.input_mut(|i| {
        (
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Slash)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::ALT, Key::Slash)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::ALT, Key::M)),
            i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::Y)),
        )
    });
    if smart && app.editor.is_none() {
        run_entry(app, ctx, "panel:smartFill");
    }
    if comment {
        run_entry(app, ctx, "panel:comment");
    }
    if help {
        run_entry(app, ctx, "panel:shortcuts");
    }
    if search {
        run_entry(app, ctx, "panel:searchMenus");
    }
    // The browser turns Ctrl+Shift+V into a paste event like Ctrl+V's; with Shift held it pastes
    // values only. Not while a cell or a text box is being typed in (the text goes there).
    if app.editor.is_some() || ctx.memory(|m| m.focused()).is_some_and(|id| egui::TextEdit::load_state(ctx, id).is_some()) {
        return;
    }
    let text = ctx.input_mut(|i| {
        if !(i.modifiers.command && i.modifiers.shift) {
            return None;
        }
        let at = i.events.iter().position(|e| matches!(e, egui::Event::Paste(_)))?;
        match i.events.remove(at) {
            egui::Event::Paste(t) => Some(t),
            _ => None,
        }
    });
    if let Some(text) = text {
        paste_values(app, &text);
    }
}

/// Ctrl+Shift+V, Google's "paste values only": the copied cells' results, without their
/// formulas, formatting or validation; the destination keeps its own formatting and rules. A copy
/// made here, or in another Starbase Sheets tab (its data rides in the clipboard's HTML), pastes
/// its values; anything else pastes as plain text. The engine's `edit.paste` decides which.
pub(crate) fn paste_values(app: &mut SheetApp, text: &str) {
    let html = app.services.paste_html.as_mut().and_then(|read| read());
    let mut params = json!({"text": text, "what": "values"});
    if let Some(html) = html {
        params["html"] = json!(html);
    }
    app.run_or_alert("edit.paste", params);
}

/// Runs one of menuactions' entries: its commands in order, stopping at the first that fails.
fn run_steps(app: &mut SheetApp, id: &str, params: &serde_json::Value) {
    let steps = match crate::menuactions::steps(&app.session, id, params) {
        Ok(steps) => steps,
        Err(e) => {
            app.message = Some(("Starbase Sheets".into(), e));
            return;
        }
    };
    let n = steps.len();
    for (cmd, p) in steps {
        if let Err(e) = app.run(cmd, p) {
            app.message = Some(("Starbase Sheets".into(), e));
            return;
        }
    }
    if id == "panel:trimWhitespace" {
        let msg = match n {
            0 => "No cells needed trimming.".to_string(),
            1 => "1 cell trimmed of whitespace.".to_string(),
            n => format!("{n} cells trimmed of whitespace."),
        };
        app.message = Some(("Trim whitespace".into(), msg));
    }
}

/// One menu entry with where it sits: "Edit › Paste special › Values only".
#[derive(Clone, Debug)]
pub struct Entry {
    pub path: String,
    pub id: &'static str,
    pub keys: &'static str,
    pub params: &'static str,
}

/// Every runnable menu entry, in menu order (Help › Search the menus and the shortcuts list).
pub fn entries() -> Vec<Entry> {
    fn walk(items: &[Item], path: &str, out: &mut Vec<Entry>) {
        for item in items {
            match item {
                Cmd(label, id, keys) => out.push(Entry { path: format!("{path} › {label}"), id, keys, params: "{}" }),
                CmdWith(label, id, keys, params) => out.push(Entry { path: format!("{path} › {label}"), id, keys, params }),
                Sub(label, sub) => walk(sub, &format!("{path} › {label}"), out),
                Sep => {}
            }
        }
    }
    let mut out = Vec::new();
    for (menu, items) in MENUS {
        walk(items, menu, &mut out);
    }
    out
}

/// Runs an entry found by Help › Search the menus.
pub fn run_found(app: &mut SheetApp, ctx: &egui::Context, e: &Entry) {
    run_entry_with(app, ctx, e.id, params(e.params));
}

/// Whether a menu entry can run here (greyed out when not): Paste special needs something
/// copied, Move column left can't move column A, and so on. Undo/redo and the clipboard trio are
/// always on (the live server keeps undo; Paste reads the system clipboard).
pub fn entry_enabled(app: &SheetApp, id: &str) -> bool {
    if let Some(on) = crate::menuactions::enabled(&app.session, id) {
        return on;
    }
    if id.starts_with("panel:") || id.starts_with("ui:") || matches!(id, "edit.copy" | "edit.cut" | "edit.paste" | "edit.undo" | "edit.redo") {
        return true;
    }
    gridcraft_engine::find_command(id).is_none_or(|c| (c.enabled)(&app.session).is_ok())
}

/// Every id the menus and toolbar use (`panel:` and `ui:` ones too), for the test hook.
#[cfg(any(target_arch = "wasm32", test))]
pub fn entry_ids() -> Vec<&'static str> {
    fn walk(items: &[Item], out: &mut Vec<&'static str>) {
        for item in items {
            match item {
                Cmd(_, id, _) | CmdWith(_, id, _, _) => out.push(id),
                Sub(_, sub) => walk(sub, out),
                Sep => {}
            }
        }
    }
    let mut out = Vec::new();
    for (_, items) in MENUS {
        walk(items, &mut out);
    }
    for tool in TOOLBAR {
        match tool {
            Btn(_, _, id, _) | Txt(_, _, id, _) | Colour(_, _, id, _, _) => out.push(id),
            Menu(_, _, items) => out.extend(items.iter().map(|(_, id, _)| *id).filter(|id| !id.is_empty())),
            Borders => out.extend(BORDERS.iter().map(|(_, id, _)| *id)),
            Font | FontSize | Zoom | Gap => {}
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Draws a menu's entries; returns its width.
fn menu_items(app: &mut SheetApp, ui: &mut Ui, items: &[Item]) -> f32 {
    let with_icons = items.iter().any(|i| match i {
        Cmd(l, ..) | CmdWith(l, ..) | Sub(l, _) => menu_icon(l).is_some(),
        Sep => false,
    });
    let width = menu_style(ui, with_icons);
    for item in items {
        match item {
            Cmd(label, id, keys) if is_merge(id) => {
                let on = crate::merge::enabled(app, id);
                if ui.add_enabled_ui(on, |ui| ui.add_sized([width, 32.0], menu_row(label, keys, with_icons))).inner.clicked() {
                    ui.ctx().data_mut(|m| m.insert_temp(egui::Id::unique(crate::merge::REQUEST), id.to_string()));
                    ui.close();
                }
            }
            Cmd(label, id, keys) | CmdWith(label, id, keys, _) => {
                let on = entry_enabled(app, id);
                let text = crate::menuactions::label(&app.session, label);
                let r = ui.add_enabled_ui(on, |ui| ui.add_sized([width, 32.0], menu_row(&text, keys, with_icons))).inner;
                house::target(ui.ctx(), &text, r.rect);
                if r.clicked() {
                    let p = match item {
                        CmdWith(.., p) => self::params(p),
                        _ => json!({}),
                    };
                    run_entry_with(app, ui.ctx(), id, p);
                    ui.close();
                }
            }
            Sub(label, sub) => {
                // Google's submenu arrow: `arrow_right` the size of the icons (a picture, so the
                // entry is still named just by its wording), as in Starbase Docs.
                let b = menu_row(label, "", with_icons).min_size(vec2(width, 32.0));
                let b = match icons::icon("arrow_right") {
                    Some(arrow) => b.right_text(arrow),
                    None => b.right_text(egui::containers::menu::SubMenuButton::arrow_atom(Some(MENU_GREY))),
                };
                egui::containers::menu::SubMenuButton::from_button(b).ui(ui, |ui| {
                    let width = menu_items(app, ui, sub);
                    // Data › Filter views also lists the sheet's views.
                    if *label == "Filter views" {
                        filterviews::menu_list(app, ui, width);
                    }
                });
            }
            Sep => menu_sep_width(ui, width),
        }
    }
    width
}

/// View › Full screen: the browser's own full screen for the whole page; the same entry leaves it.
fn toggle_fullscreen() {
    #[cfg(target_arch = "wasm32")]
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if doc.fullscreen_element().is_some() {
            doc.exit_fullscreen();
        } else if let Some(el) = doc.document_element() {
            let _ = el.request_fullscreen();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[Item], out: &mut Vec<&'static str>) {
        for item in items {
            match item {
                // "ui:" and "panel:" entries open a dialog or panel, not a command.
                Cmd(_, id, _) | CmdWith(_, id, _, _) if !id.starts_with("ui:") && !id.starts_with("panel:") => out.push(id),
                Cmd(..) | CmdWith(..) => {}
                Sub(_, sub) => ids(sub, out),
                Sep => {}
            }
        }
    }

    /// Every menu and toolbar entry names a command GridCraft's engine knows.
    #[test]
    fn every_menu_icon_is_embedded() {
        for (label, name) in MENU_ICONS {
            assert!(icons::has(name), "menu icon {name} for {label} isn't in app/icons");
        }
    }

    #[test]
    fn every_entry_names_a_real_command() {
        let mut all = Vec::new();
        for (_, items) in MENUS {
            ids(items, &mut all);
        }
        for tool in TOOLBAR {
            match tool {
                Btn(icon, _, id, _) => {
                    assert!(icons::has(icon), "missing icon {icon}");
                    all.push(id);
                }
                Txt(_, _, id, _) | Colour(_, _, id, _, _) => all.push(id),
                Menu(_, _, items) => {
                    for (label, id, _) in *items {
                        if let Some(icon) = label.strip_prefix("icon:") {
                            let icon = icon.split('|').next().unwrap_or(icon);
                            assert!(icons::has(icon), "missing icon {icon}");
                        }
                        if !id.is_empty() {
                            all.push(id);
                        }
                    }
                }
                Borders => {
                    for icon in ["border_all", "border_color", "line_style"] {
                        assert!(icons::has(icon), "missing icon {icon}");
                    }
                    for (label, id, _) in BORDERS {
                        let icon = label.trim_start_matches("icon:").split('|').next().unwrap_or(label);
                        assert!(icons::has(icon), "missing icon {icon}");
                        all.push(id);
                    }
                }
                Font | FontSize | Zoom | Gap => {}
            }
        }
        let missing: Vec<_> = all.into_iter().filter(|id| !id.starts_with("panel:") && gridcraft_engine::find_command(id).is_none()).collect();
        assert!(missing.is_empty(), "unknown command ids: {missing:?}");
    }

    /// The Sheets menu batch 1 entries are where Sheets has them, and every entry's parameters
    /// are valid JSON.
    #[test]
    fn sheets_menu_entries_are_in_place() {
        let paths: Vec<String> = entries().iter().map(|e| e.path.clone()).collect();
        for want in [
            "Edit › Paste special › Values only",
            "Edit › Paste special › Format only",
            "Edit › Paste special › Formula only",
            "Edit › Paste special › Data validation only",
            "Edit › Paste special › Transposed",
            "Edit › Paste special › Column widths only",
            "Edit › Paste special › All except borders",
            "Edit › Move › Column left",
            "Edit › Move › Column right",
            "View › Freeze › 2 rows",
            "View › Freeze › Up to row {row}",
            "View › Freeze › 2 columns",
            "View › Freeze › Up to column {col}",
            "View › Freeze › No columns",
            "Format › Number › Custom currency",
            "Data › Sort sheet › Sort sheet by column {col} (A to Z)",
            "Data › Sort range › Sort range by column {col} (Z to A)",
            "Data › Randomise range",
            "Data › Data clean-up › Trim whitespace",
            "Data › Data clean-up › Remove duplicates",
            "Help › Search the menus",
            "Help › Keyboard shortcuts",
            "Insert › Comment",
        ] {
            assert!(paths.iter().any(|p| p == want), "missing {want}");
        }
        for e in entries() {
            assert!(serde_json::from_str::<serde_json::Value>(e.params).is_ok(), "{}: bad params {}", e.path, e.params);
        }
        // Every id that menuactions runs is on a menu, and every panel: id a menu uses is one
        // run_entry knows (menuactions, a dialog request, or a file action in files.rs).
        let ids = entry_ids();
        for id in crate::menuactions::IDS {
            assert!(ids.contains(id), "{id} is not on any menu");
        }
        let known = [
            "panel:new",
            "panel:templates",
            "panel:open",
            "panel:copy",
            "panel:bin",
            "panel:versions",
            "panel:details",
            "panel:settings",
            "panel:insertBelow",
            "panel:insertRight",
            "panel:chart",
            "panel:imageInCell",
            "panel:imageOverCells",
            "panel:dropdown",
            "panel:note",
            "panel:comment",
            "panel:customdate",
            "panel:customnumber",
            "panel:customcurrency",
            "panel:protect",
            "panel:fullscreen",
            "panel:shortcuts",
            "panel:searchMenus",
            "panel:alternatingColours",
            "panel:smartFill",
            "panel:smartFillToggle",
        ];
        for id in ids.iter().filter(|id| id.starts_with("panel:")) {
            assert!(crate::menuactions::handles(id) || filterviews::handles(id) || known.contains(id), "{id}: nothing opens it");
        }
    }

    /// Greyed by context: Paste special needs a copy first; Move column left not from column A.
    #[test]
    fn entries_grey_out_by_context() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        let on = |s: &gridcraft_engine::Session, id: &str| {
            crate::menuactions::enabled(s, id).unwrap_or_else(|| gridcraft_engine::find_command(id).is_none_or(|c| (c.enabled)(s).is_ok()))
        };
        assert!(!on(&s, "edit.pasteSpecial"), "nothing copied yet");
        assert!(!on(&s, "panel:moveColumnLeft"));
        assert!(!on(&s, "panel:sortSheet"), "an empty sheet");
        s.execute("cell.set", json!({"cell": "B2", "input": "x"})).unwrap();
        s.execute("edit.copy", json!({"range": "B2"})).unwrap();
        s.execute("selection.set", json!({"range": "B2"})).unwrap();
        assert!(on(&s, "edit.pasteSpecial") && on(&s, "panel:moveColumnLeft") && on(&s, "panel:sortSheet"));
        s.execute("edit.pasteSpecial", json!({"what": "values", "at": "D4"})).unwrap();
        assert_eq!(s.active().unwrap().wb.active().unwrap().input_text(gridcraft_engine::core::CellRef::new(3, 3)), "x");
    }

    /// To-do row 21: Ctrl+V pastes the cells whole (formula, bold, drop-down rule); Ctrl+Shift+V
    /// pastes only their results and leaves the destination's formatting and rules alone, for a
    /// copy made here and one from another tab (its cells come in the clipboard's HTML).
    #[test]
    fn ctrl_v_pastes_whole_and_ctrl_shift_v_pastes_values() {
        let new_app = |html: Option<String>| {
            let services = gridcraft_ui_egui::Services { paste_html: Some(Box::new(move || html.clone())), ..Default::default() };
            let mut app = SheetApp::new(gridcraft_engine::Session::new(), services);
            app.session.execute("file.new", json!({})).unwrap();
            app
        };
        let get = |app: &mut SheetApp, c: &str| app.session.execute("cell.get", json!({"cell": c})).unwrap();
        let rules = |app: &SheetApp| app.session.active().unwrap().wb.active().unwrap().validations.len();
        let covered = |app: &SheetApp, row: u32| {
            let at = gridcraft_engine::core::CellRef::new(row, 0);
            app.session.active().unwrap().wb.active().unwrap().validations.iter().any(|dv| dv.ranges.iter().any(|r| r.contains(at)))
        };
        let mut a = new_app(None);
        a.session.execute("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
        a.session.execute("cell.set", json!({"cell": "B1", "input": "=A1+1"})).unwrap();
        a.session.execute("home.bold", json!({"range": "A1:B1"})).unwrap();
        a.session.execute("data.validation", json!({"range": "A1", "type": "list", "formula1": "\"5,6\""})).unwrap();
        a.session.execute("selection.set", json!({"range": "A1:B1"})).unwrap();
        let copied = a.session.execute("edit.copy", json!({"html": true})).unwrap();
        let (text, html) = (copied["text"].as_str().unwrap().to_string(), copied["html"].as_str().unwrap().to_string());
        // Same tab, Ctrl+V: everything.
        a.session.execute("selection.set", json!({"range": "A3"})).unwrap();
        a.paste_external(text.clone());
        assert_eq!(get(&mut a, "B3")["formula"], json!("=A3+1"));
        assert_eq!(get(&mut a, "A3")["style"]["font"]["bold"], json!(true));
        assert!(covered(&a, 2), "A3 gets the drop-down rule");
        // Same tab, Ctrl+Shift+V: results only.
        a.session.execute("selection.set", json!({"range": "A5"})).unwrap();
        paste_values(&mut a, &text);
        let (a5, b5) = (get(&mut a, "A5"), get(&mut a, "B5"));
        assert_eq!(
            (b5["formula"].clone(), b5["value"].clone(), a5["style"]["font"]["bold"].clone()),
            (serde_json::Value::Null, json!(6.0), json!(false))
        );
        assert!(!covered(&a, 4), "values only brings no rule");
        // Another tab, Ctrl+V: the HTML brings the cells.
        let mut b = new_app(Some(html.clone()));
        b.paste_external(text.clone());
        assert_eq!(get(&mut b, "B1")["formula"], json!("=A1+1"));
        assert_eq!(get(&mut b, "A1")["style"]["font"]["bold"], json!(true));
        assert!(covered(&b, 0));
        // Another tab, Ctrl+Shift+V.
        let mut c = new_app(Some(html));
        paste_values(&mut c, &text);
        let (a1, b1) = (get(&mut c, "A1"), get(&mut c, "B1"));
        assert_eq!(
            (b1["formula"].clone(), b1["value"].clone(), a1["style"]["font"]["bold"].clone()),
            (serde_json::Value::Null, json!(6.0), json!(false))
        );
        assert_eq!(rules(&c), 0);
    }
}
