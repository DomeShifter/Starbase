//! The Google Docs-style window around WordCraft's page: a title row, the menu bar and a one-row
//! toolbar. WordCraft's own title bar, ribbon and status bar are switched off
//! (`HostChrome`, fork change StarDocs 0001); every item here runs a WordCraft command by id
//! (`WordApp::run`), or opens one of its dialogs (`ui.dialog`).

use egui::{Align, Button, Color32, Layout, RichText, Stroke, Ui, vec2};
use serde_json::{Value, json};
use wordcraft_ui_egui::WordApp;

use crate::icons;

pub const BLUE: Color32 = Color32::from_rgb(0x0B, 0x57, 0xD0);
pub const PAGE: Color32 = Color32::from_rgb(0xF9, 0xFB, 0xFD);
pub const TOOLBAR: Color32 = Color32::from_rgb(0xED, 0xF2, 0xFA);
pub const TEXT: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x1F);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x44, 0x47, 0x46);
pub const DIVIDER: Color32 = Color32::from_rgb(0xC7, 0xC7, 0xC7);
pub const ON: Color32 = Color32::from_rgb(0xD3, 0xE3, 0xFD);
pub const SHARE: Color32 = Color32::from_rgb(0xC2, 0xE7, 0xFF);

/// A menu entry: a command (label, id, params as JSON text, shortcut shown), a sub-menu, a line,
/// an on/off setting with a tick (label, command id, what it reads, shortcut shown; see
/// [`is_on`]), a command whose wording follows a setting (label when off, label when on, command
/// id, what it reads: Docs' "Add/Remove space before paragraph"), or Docs' grid for picking a
/// table's size. Commands are greyed out where WordCraft would refuse them ([`greyed`]).
pub enum Item {
    Cmd(&'static str, &'static str, &'static str, &'static str),
    Sub(&'static str, &'static [Item]),
    Sep,
    Toggle(&'static str, &'static str, &'static str, &'static str),
    Flip(&'static str, &'static str, &'static str, &'static str),
    TableGrid,
}
use Item::{Cmd, Flip, Sep, Sub, TableGrid, Toggle};

const NO: &str = "{}";

/// Format › Image, which the page's right-click menu also shows for a selected picture
/// ([`picture_menu`]).
pub const IMAGE: &[Item] = &[
    Cmd("Crop image", "ui.crop", NO, ""),
    Cmd("Size and rotation", "picture.size", NO, ""),
    Sub(
        "Rotate",
        &[
            Cmd("Rotate clockwise 90°", "arrange.rotate", r#"{"direction":"right"}"#, ""),
            Cmd("Rotate anticlockwise 90°", "arrange.rotate", r#"{"direction":"left"}"#, ""),
            Cmd("Flip horizontally", "arrange.rotate", r#"{"direction":"flipHorizontal"}"#, ""),
            Cmd("Flip vertically", "arrange.rotate", r#"{"direction":"flipVertical"}"#, ""),
        ],
    ),
    Sub(
        "Text wrapping",
        &[
            Cmd("In line with text", "arrange.wrap", r#"{"wrap":"inline"}"#, ""),
            Cmd("Wrap text", "arrange.wrap", r#"{"wrap":"square"}"#, ""),
            Cmd("Break text", "arrange.wrap", r#"{"wrap":"topAndBottom"}"#, ""),
            Cmd("Behind text", "arrange.wrap", r#"{"wrap":"behindText"}"#, ""),
            Cmd("In front of text", "arrange.wrap", r#"{"wrap":"inFrontOfText"}"#, ""),
        ],
    ),
    Sep,
    Cmd("Alt text", "view.altTextPane", r#"{"value":true}"#, ""),
    Cmd("Replace image", "ui.changePicture", NO, ""),
    Cmd("Reset image", "picture.reset", NO, ""),
];

/// The page's right-click menu for a selected picture: Format › Image's items in place of
/// WordCraft's own, whose Size and Crop would open a ribbon tab StarDocs hides (fork change
/// StarDocs 0007, `WordApp::set_picture_menu`). They are all WordCraft commands.
pub fn picture_menu(app: &mut WordApp, ui: &mut Ui) {
    menu_items(app, ui, &mut Flags::default(), IMAGE, None);
}

/// The menus, in Google Docs' order and wording. An id starting `ui.dialog:` opens that
/// WordCraft dialog.
pub const MENUS: &[(&str, &[Item])] = &[
    (
        "File",
        &[
            Cmd("New document", "file.new", NO, ""),
            Cmd("Open", "file.open", NO, "Ctrl+O"),
            Sep,
            Sub(
                "Download",
                &[
                    Cmd("Microsoft Word (.docx)", "stardocs.download", NO, ""),
                    Cmd("OpenDocument format (.odt)", "stardocs.downloadAs", r#"{"ext":"odt"}"#, ""),
                    Cmd("Rich Text Format (.rtf)", "stardocs.downloadAs", r#"{"ext":"rtf"}"#, ""),
                    Cmd("PDF document (.pdf)", "file.exportPdf", NO, ""),
                    Cmd("Plain text (.txt)", "stardocs.downloadAs", r#"{"ext":"txt"}"#, ""),
                    Cmd("Web page (.html)", "stardocs.downloadAs", r#"{"ext":"html"}"#, ""),
                    Cmd("Markdown (.md)", "stardocs.downloadAs", r#"{"ext":"md"}"#, ""),
                ],
            ),
            Cmd("Rename", "stardocs.rename", NO, ""),
            Sep,
            Cmd("Page setup", "ui.dialog:pageSetup", NO, ""),
            Cmd("Print", "file.print", NO, "Ctrl+P"),
        ],
    ),
    (
        "Edit",
        &[
            Cmd("Undo", "edit.undo", NO, "Ctrl+Z"),
            Cmd("Redo", "edit.redo", NO, "Ctrl+Y"),
            Sep,
            Cmd("Cut", "edit.cut", NO, "Ctrl+X"),
            Cmd("Copy", "edit.copy", NO, "Ctrl+C"),
            Cmd("Paste", "edit.paste", NO, "Ctrl+V"),
            Cmd("Paste without formatting", "edit.pasteText", NO, "Ctrl+Shift+V"),
            Sep,
            Cmd("Select all", "select.all", NO, "Ctrl+A"),
            Sep,
            Cmd("Find and replace", "ui.dialog:replace", NO, "Ctrl+H"),
        ],
    ),
    (
        "View",
        &[
            Sub(
                "Mode",
                &[
                    Cmd("Editing: edit the document directly", "stardocs.editing", NO, ""),
                    Cmd("Suggesting: edits become suggestions", "stardocs.suggesting", NO, ""),
                    Cmd("Viewing: read or print the final document", "stardocs.viewing", NO, ""),
                ],
            ),
            Sep,
            Toggle("Print layout", "view.printLayout", "stardocs.print", ""),
            Toggle("Show ruler", "view.ruler", "ruler", ""),
            Toggle("Show outline", "view.navigationPane", "navPane", "Ctrl+Alt+H"),
            Toggle("Show comments", "view.commentsPane", "commentsPane", ""),
            Toggle("Show word count", "stardocs.words", "stardocs.words", ""),
            Toggle("Show non-printing characters", "view.marks", "marks", "Ctrl+Shift+P"),
            Toggle("Show the menus", "stardocs.compact", "stardocs.menus", "Ctrl+Shift+F"),
            Sep,
            Cmd("Zoom in", "view.zoomIn", NO, ""),
            Cmd("Zoom out", "view.zoomOut", NO, ""),
            Cmd("Fit page width", "view.zoom", r#"{"value":"pageWidth"}"#, ""),
            Sep,
            Cmd("Full screen", "view.fullScreen", NO, ""),
        ],
    ),
    (
        "Insert",
        &[
            Cmd("Image", "insert.picture", NO, ""),
            Sub("Table", &[TableGrid, Sep, Cmd("Table options", "ui.dialog:insertTable", NO, "")]),
            Cmd("Date", "insert.dateTime", r#"{"format":"d MMMM yyyy"}"#, ""),
            Sep,
            Cmd("Link", "ui.dialog:link", NO, "Ctrl+K"),
            Cmd("Comment", "review.newComment", NO, "Ctrl+Alt+M"),
            Sep,
            Cmd("Horizontal line", "insert.horizontalLine", NO, ""),
            Sub(
                "Break",
                &[
                    Cmd("Page break", "insert.pageBreak", NO, "Ctrl+Enter"),
                    Cmd("Column break", "text.columnBreak", NO, "Ctrl+Shift+Enter"),
                    Sep,
                    Cmd("Section break (next page)", "layout.break", r#"{"kind":"nextPage"}"#, ""),
                    Cmd("Section break (continuous)", "layout.break", r#"{"kind":"continuous"}"#, ""),
                ],
            ),
            Sub(
                "Headers and footers",
                &[
                    Cmd("Header", "insert.editHeader", NO, ""),
                    Cmd("Footer", "insert.editFooter", NO, ""),
                    Sep,
                    Toggle("Different first page", "layout.differentFirstPage", "layout.firstPage", ""),
                    Toggle("Different odd and even", "layout.differentOddEven", "layout.oddEven", ""),
                    Sep,
                    Cmd("Remove header", "insert.removeHeader", NO, ""),
                    Cmd("Remove footer", "insert.removeFooter", NO, ""),
                ],
            ),
            Sub(
                "Page numbers",
                &[
                    Cmd("Top of page", "insert.pageNumber", r#"{"position":"top","align":"right"}"#, ""),
                    Cmd("Bottom of page", "insert.pageNumber", r#"{"position":"bottom","align":"right"}"#, ""),
                    Cmd("Page count", "insert.field", r#"{"instr":"NUMPAGES"}"#, ""),
                    Sep,
                    Sub(
                        "Number format",
                        &[
                            Cmd("1, 2, 3", "layout.pageNumberFormat", r#"{"format":"decimal"}"#, ""),
                            Cmd("i, ii, iii", "layout.pageNumberFormat", r#"{"format":"lowerRoman"}"#, ""),
                            Cmd("I, II, III", "layout.pageNumberFormat", r#"{"format":"upperRoman"}"#, ""),
                            Cmd("a, b, c", "layout.pageNumberFormat", r#"{"format":"lowerLetter"}"#, ""),
                            Cmd("A, B, C", "layout.pageNumberFormat", r#"{"format":"upperLetter"}"#, ""),
                        ],
                    ),
                    Cmd("More options", "ui.dialog:pageNumbers", NO, ""),
                ],
            ),
            Cmd("Footnote", "references.footnote", NO, "Ctrl+Alt+F"),
            Sep,
            Cmd("Special characters", "insert.symbol", NO, ""),
            Cmd("Equation", "insert.equation", NO, ""),
            Cmd("Bookmark", "ui.dialog:bookmark", NO, ""),
            Sub(
                "Table of contents",
                &[
                    Cmd("With page numbers", "references.toc", r#"{"pageNumbers":true,"hyperlinks":true}"#, ""),
                    Cmd("With links", "references.toc", r#"{"pageNumbers":false,"hyperlinks":true}"#, ""),
                    Sep,
                    Cmd("Update table of contents", "references.updateToc", NO, ""),
                    Cmd("Delete table of contents", "references.removeToc", NO, ""),
                ],
            ),
        ],
    ),
    (
        "Format",
        &[
            Sub(
                "Text",
                &[
                    Cmd("Bold", "format.bold", NO, "Ctrl+B"),
                    Cmd("Italic", "format.italic", NO, "Ctrl+I"),
                    Cmd("Underline", "format.underline", NO, "Ctrl+U"),
                    Cmd("Strikethrough", "format.strikethrough", NO, "Alt+Shift+5"),
                    Cmd("Small caps", "format.smallCaps", NO, ""),
                    Cmd("Superscript", "format.superscript", NO, "Ctrl+."),
                    Cmd("Subscript", "format.subscript", NO, "Ctrl+,"),
                    Sep,
                    Sub(
                        "Size",
                        &[
                            Cmd("Increase font size", "format.growFont1", NO, "Ctrl+Shift+."),
                            Cmd("Decrease font size", "format.shrinkFont1", NO, "Ctrl+Shift+,"),
                        ],
                    ),
                    Sub(
                        "Capitalisation",
                        &[
                            Cmd("lowercase", "format.changeCase", r#"{"mode":"lower"}"#, ""),
                            Cmd("UPPERCASE", "format.changeCase", r#"{"mode":"upper"}"#, ""),
                            Cmd("Title Case", "format.changeCase", r#"{"mode":"title"}"#, ""),
                        ],
                    ),
                ],
            ),
            Sub(
                "Paragraph styles",
                &[
                    Sub(
                        "Normal text",
                        &[
                            Cmd("Apply 'Normal text'", "para.style", r#"{"style":"Normal"}"#, "Ctrl+Alt+0"),
                            Cmd("Update 'Normal text' to match", "styles.updateToMatch", r#"{"style":"Normal"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Title",
                        &[
                            Cmd("Apply 'Title'", "para.style", r#"{"style":"Title"}"#, ""),
                            Cmd("Update 'Title' to match", "styles.updateToMatch", r#"{"style":"Title"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Subtitle",
                        &[
                            Cmd("Apply 'Subtitle'", "para.style", r#"{"style":"Subtitle"}"#, ""),
                            Cmd("Update 'Subtitle' to match", "styles.updateToMatch", r#"{"style":"Subtitle"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Heading 1",
                        &[
                            Cmd("Apply 'Heading 1'", "para.style", r#"{"style":"Heading 1"}"#, "Ctrl+Alt+1"),
                            Cmd("Update 'Heading 1' to match", "styles.updateToMatch", r#"{"style":"Heading 1"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Heading 2",
                        &[
                            Cmd("Apply 'Heading 2'", "para.style", r#"{"style":"Heading 2"}"#, "Ctrl+Alt+2"),
                            Cmd("Update 'Heading 2' to match", "styles.updateToMatch", r#"{"style":"Heading 2"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Heading 3",
                        &[
                            Cmd("Apply 'Heading 3'", "para.style", r#"{"style":"Heading 3"}"#, "Ctrl+Alt+3"),
                            Cmd("Update 'Heading 3' to match", "styles.updateToMatch", r#"{"style":"Heading 3"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Heading 4",
                        &[
                            Cmd("Apply 'Heading 4'", "para.style", r#"{"style":"Heading 4"}"#, "Ctrl+Alt+4"),
                            Cmd("Update 'Heading 4' to match", "styles.updateToMatch", r#"{"style":"Heading 4"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Heading 5",
                        &[
                            Cmd("Apply 'Heading 5'", "para.style", r#"{"style":"Heading 5"}"#, "Ctrl+Alt+5"),
                            Cmd("Update 'Heading 5' to match", "styles.updateToMatch", r#"{"style":"Heading 5"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Heading 6",
                        &[
                            Cmd("Apply 'Heading 6'", "para.style", r#"{"style":"Heading 6"}"#, "Ctrl+Alt+6"),
                            Cmd("Update 'Heading 6' to match", "styles.updateToMatch", r#"{"style":"Heading 6"}"#, ""),
                        ],
                    ),
                ],
            ),
            Sub(
                "Align and indent",
                &[
                    Cmd("Left", "para.alignLeft", NO, "Ctrl+Shift+L"),
                    Cmd("Centre", "para.alignCenter", NO, "Ctrl+Shift+E"),
                    Cmd("Right", "para.alignRight", NO, "Ctrl+Shift+R"),
                    Cmd("Justified", "para.justify", NO, "Ctrl+Shift+J"),
                    Sep,
                    Cmd("Increase indent", "para.indent", NO, "Ctrl+]"),
                    Cmd("Decrease indent", "para.outdent", NO, "Ctrl+["),
                    Cmd("Indentation options", "ui.dialog:paragraph", NO, ""),
                ],
            ),
            Sub(
                "Line and paragraph spacing",
                &[
                    Cmd("Single", "para.lineSpacing", r#"{"value":1.0}"#, ""),
                    Cmd("1.15", "para.lineSpacing", r#"{"value":1.15}"#, ""),
                    Cmd("1.5", "para.lineSpacing", r#"{"value":1.5}"#, ""),
                    Cmd("Double", "para.lineSpacing", r#"{"value":2.0}"#, ""),
                    Sep,
                    Flip("Add space before paragraph", "Remove space before paragraph", "para.addSpaceBefore", "para.spaceBefore"),
                    Flip("Add space after paragraph", "Remove space after paragraph", "para.removeSpaceAfter", "para.spaceAfter"),
                    Sep,
                    Cmd("Custom spacing", "ui.dialog:paragraph", NO, ""),
                    Sep,
                    Toggle("Keep with next", "para.keepNext", "para.keepNext", ""),
                    Toggle("Keep lines together", "para.keepLines", "para.keepLines", ""),
                ],
            ),
            Cmd("Columns", "layout.columns", NO, ""),
            Sub(
                "Bullets and numbering",
                &[
                    Cmd("Bulleted list", "para.bullets", NO, "Ctrl+Shift+8"),
                    Cmd("Numbered list", "para.numbering", NO, "Ctrl+Shift+7"),
                    Cmd("Checklist", "list.checklist", NO, "Ctrl+Shift+9"),
                    Sep,
                    Sub(
                        "Bullet style",
                        &[
                            Cmd("Disc, circle, square", "para.bullets", r#"{"kind":"bullet"}"#, ""),
                            Cmd("Diamond", "para.bullets", r#"{"kind":"❖"}"#, ""),
                            Cmd("Arrowhead", "para.bullets", r#"{"kind":"➢"}"#, ""),
                            Cmd("Arrow", "para.bullets", r#"{"kind":"➔"}"#, ""),
                            Cmd("Star", "para.bullets", r#"{"kind":"★"}"#, ""),
                            Cmd("Square", "para.bullets", r#"{"kind":"■"}"#, ""),
                            Cmd("Dash", "para.bullets", r#"{"kind":"–"}"#, ""),
                        ],
                    ),
                    Sub(
                        "Numbering style",
                        &[
                            Cmd("1.  2.  3.", "para.numbering", r#"{"kind":"numbered"}"#, ""),
                            Cmd("1)  2)  3)", "para.numbering", r#"{"kind":"numberedParen"}"#, ""),
                            Cmd("A.  B.  C.", "para.numbering", r#"{"kind":"upperLetter"}"#, ""),
                            Cmd("a.  b.  c.", "para.numbering", r#"{"kind":"lowerLetter"}"#, ""),
                            Cmd("i.  ii.  iii.", "para.numbering", r#"{"kind":"lowerRoman"}"#, ""),
                            Cmd("1.  a.  i.", "para.numbering", r#"{"kind":"outline"}"#, ""),
                        ],
                    ),
                    Sub("List options", &[Cmd("Restart numbering", "stardocs.restartNumbering", NO, "")]),
                ],
            ),
            Sep,
            Sub(
                "Table",
                &[
                    Cmd("Insert row above", "table.insertRowAbove", NO, ""),
                    Cmd("Insert row below", "table.insertRowBelow", NO, ""),
                    Cmd("Insert column left", "table.insertColumnLeft", NO, ""),
                    Cmd("Insert column right", "table.insertColumnRight", NO, ""),
                    Sep,
                    Cmd("Delete row", "table.deleteRow", NO, ""),
                    Cmd("Delete column", "table.deleteColumn", NO, ""),
                    Cmd("Delete table", "table.deleteTable", NO, ""),
                    Sep,
                    Cmd("Merge cells", "table.merge", NO, ""),
                    Cmd("Split cells", "table.split", NO, ""),
                    Sep,
                    Cmd("Distribute rows", "table.distributeRows", NO, ""),
                    Cmd("Distribute columns", "table.distributeColumns", NO, ""),
                    Sep,
                    Toggle("Pin header row", "table.repeatHeader", "table.header", ""),
                    Cmd("Table properties", "table.properties", NO, ""),
                ],
            ),
            Sub("Image", IMAGE),
            Cmd("Borders and shading", "ui.dialog:borders", NO, ""),
            Cmd("Clear formatting", "format.clear", NO, "Ctrl+\\"),
        ],
    ),
    (
        "Tools",
        &[
            Cmd("Spelling and grammar", "review.spelling", NO, ""),
            Cmd("Word count", "ui.dialog:wordCount", NO, "Ctrl+Shift+C"),
            Sep,
            Toggle("Suggesting (track changes)", "review.trackChanges", "trackChanges", ""),
            Sub(
                "Review suggested edits",
                &[
                    Cmd("Accept suggestion", "review.accept", NO, ""),
                    Cmd("Reject suggestion", "review.reject", NO, ""),
                    Sep,
                    Cmd("Next suggestion", "review.nextChange", NO, ""),
                    Cmd("Previous suggestion", "review.previousChange", NO, ""),
                    Sep,
                    Cmd("Accept all", "review.acceptAll", NO, ""),
                    Cmd("Reject all", "review.rejectAll", NO, ""),
                ],
            ),
            Sub(
                "Citations",
                &[
                    Sub(
                        "Citation style",
                        &[
                            Cmd("MLA", "references.citationStyle", r#"{"style":"MLA"}"#, ""),
                            Cmd("APA", "references.citationStyle", r#"{"style":"APA"}"#, ""),
                            Cmd("Chicago", "references.citationStyle", r#"{"style":"Chicago"}"#, ""),
                            Cmd("Harvard", "references.citationStyle", r#"{"style":"Harvard (Anglia)"}"#, ""),
                        ],
                    ),
                    Cmd("Add citation source", "references.citation", NO, ""),
                    Cmd("Manage sources", "references.sources", NO, ""),
                    Sep,
                    Cmd("Insert bibliography", "references.bibliography", NO, ""),
                ],
            ),
            Sep,
            Cmd("Read aloud", "review.readAloud", NO, ""),
        ],
    ),
    (
        "Help",
        &[
            Cmd("Search the menus", "tools.search", NO, "Alt+/"),
            Cmd("Keyboard shortcuts", "stardocs.shortcuts", NO, "Ctrl+/"),
            Sep,
            Cmd("About Starbase Docs", "ui.dialog:about", NO, ""),
        ],
    ),
];

const STYLES: &[(&str, &str)] = &[
    ("Normal text", "Normal"),
    ("Title", "Title"),
    ("Subtitle", "Subtitle"),
    ("Heading 1", "Heading 1"),
    ("Heading 2", "Heading 2"),
    ("Heading 3", "Heading 3"),
];
const SIZES: &[f64] = &[8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 18.0, 24.0, 30.0, 36.0, 48.0, 60.0, 72.0, 96.0];
const ZOOMS: &[u32] = &[50, 75, 90, 100, 125, 150, 200];
/// Google Docs' first palette row and its brighter row, enough for a prototype.
const COLOURS: &[&str] = &[
    "000000", "434343", "666666", "999999", "B7B7B7", "CCCCCC", "D9D9D9", "EFEFEF", "F3F3F3", "FFFFFF", "980000", "FF0000", "FF9900", "FFFF00",
    "00FF00", "00FFFF", "4A86E8", "0000FF", "9900FF", "FF00FF",
];
const HIGHLIGHTS: &[(&str, &str)] = &[
    ("yellow", "FFFF00"),
    ("brightGreen", "00FF00"),
    ("turquoise", "00FFFF"),
    ("pink", "FF00FF"),
    ("blue", "0000FF"),
    ("red", "FF0000"),
    ("green", "008000"),
    ("lightGray", "C0C0C0"),
];

/// Our part of the window. Fonts are listed once from WordCraft (`font.list`).
#[derive(Default)]
pub struct Shell {
    fonts: Option<Vec<String>>,
    /// The title being typed over (server documents: the file's name).
    renaming: Option<String>,
    share: crate::share::Share,
    flags: Flags,
    shortcuts: crate::shortcuts::Shortcuts,
    word_count: crate::wordcount::WordCount,
    /// The colour being mixed in the Custom colour dialog, while it is open.
    custom: Option<Color32>,
    /// The number typed in the Restart numbering dialog, while it is open.
    restart: Option<String>,
    /// The dialog has just opened: its box takes the keyboard.
    restart_focus: bool,
    /// Each toolbar group's width last frame, to decide which fit.
    widths: Widths,
    /// How many toolbar groups didn't fit and are in the ⋮ menu.
    in_more: usize,
    /// The mode box and arrow's width last frame, and the room the groups had.
    right_end: f32,
    room: f32,
    /// The window's egui context, kept from the last frame for the test hook.
    ctx: Option<egui::Context>,
}

/// The toolbar groups' widths, starting from a guess.
struct Widths([f32; 9]);
impl Default for Widths {
    fn default() -> Self {
        Self(GUESSED_WIDTHS)
    }
}
impl std::ops::Deref for Widths {
    type Target = [f32; 9];
    fn deref(&self) -> &[f32; 9] {
        &self.0
    }
}
impl std::ops::DerefMut for Widths {
    fn deref_mut(&mut self) -> &mut [f32; 9] {
        &mut self.0
    }
}

/// StarDocs' own on/off settings, kept in the browser between visits ([`FLAGS_KEY`]).
#[derive(Clone, Copy, PartialEq, Default)]
struct Flags {
    /// The live word count box at the bottom left.
    words: bool,
    /// Compact mode: the title row and menus hidden, only the toolbar shown (Docs' Ctrl+Shift+F).
    compact: bool,
    /// The keyboard shortcuts list is open.
    shortcuts: bool,
    /// Asks for the Restart numbering dialog (Format › Bullets and numbering › List options).
    restart: bool,
    /// Read from storage yet.
    loaded: bool,
}

const FLAGS_KEY: &str = "stardocs.flags";

/// Where the flags are kept: the browser's `localStorage`, or a per-thread map in native tests.
#[cfg(target_arch = "wasm32")]
mod store {
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }
    pub fn get(key: &str) -> Option<String> {
        storage()?.get_item(key).ok().flatten()
    }
    pub fn set(key: &str, value: &str) {
        if let Some(s) = storage() {
            let _ = s.set_item(key, value);
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
mod store {
    use std::{cell::RefCell, collections::HashMap};
    thread_local!(static MAP: RefCell<HashMap<String, String>> = RefCell::default());
    pub fn get(key: &str) -> Option<String> {
        MAP.with(|m| m.borrow().get(key).cloned())
    }
    pub fn set(key: &str, value: &str) {
        MAP.with(|m| m.borrow_mut().insert(key.into(), value.into()));
    }
}

impl Flags {
    /// Fills in the stored settings the first time; afterwards writes them back when they change.
    fn sync(&mut self, before: Flags) {
        if !self.loaded {
            self.loaded = true;
            if let Some(v) = store::get(FLAGS_KEY) {
                self.words = v.contains("words");
                self.compact = v.contains("compact");
            }
        } else if (self.words, self.compact) != (before.words, before.compact) {
            let v = [("words", self.words), ("compact", self.compact)].iter().filter(|(_, on)| *on).map(|(k, _)| *k).collect::<Vec<_>>().join(",");
            store::set(FLAGS_KEY, &v);
        }
    }
}

use crate::server::{Link, Saved};

/// Asks the title row to start renaming (File › Rename), for the next frame.
const RENAME: &str = "stardocs_rename";

/// Whether a ticked menu item is on: StarDocs' own settings (`stardocs.*`), suggesting
/// (`trackChanges`, kept in the document), or a field of WordCraft's `view.state`.
fn is_on(app: &mut WordApp, flags: &Flags, key: &str) -> bool {
    match key {
        "stardocs.words" => flags.words,
        "stardocs.menus" => !flags.compact,
        "stardocs.print" => app.run("view.state", json!({})).ok().is_some_and(|v| v["mode"] == "print"),
        "trackChanges" => app.session.doc.settings.track_changes,
        "para.spaceBefore" => selection_para(app).space_before > 0.0,
        "para.spaceAfter" => selection_para(app).space_after > 0.0,
        "para.keepNext" => selection_para(app).keep_next,
        "para.keepLines" => selection_para(app).keep_lines,
        "table.header" => header_row(app),
        "layout.firstPage" => wordcraft_engine::cmd::page::sect(&app.session).title_page,
        "layout.oddEven" => app.session.doc.settings.even_odd_headers,
        _ => app.run("view.state", json!({})).ok().and_then(|v| v.get(key).and_then(Value::as_bool)).unwrap_or(false),
    }
}

/// The caret paragraph's formatting, its style's included (what WordCraft's own buttons read).
fn selection_para(app: &WordApp) -> wordcraft_doc::resolve::ResolvedPara {
    wordcraft_engine::cmd::para::selection_para(&app.session)
}

/// Whether the caret's table row is a header row repeated on each page (Docs' pinned row).
fn header_row(app: &WordApp) -> bool {
    let focus = &app.session.sel.focus;
    focus.path.cell().and_then(|(t, r, _)| app.session.doc.table(focus.story, &t)?.rows.get(r).map(|row| row.props.header)).unwrap_or(false)
}

/// Whether the caret is in a numbered (not bulleted) list.
fn in_numbered_list(app: &WordApp) -> bool {
    selection_para(app)
        .numbering
        .filter(|n| n.num != 0)
        .and_then(|n| app.session.doc.numbering.level(n.num, n.level))
        .is_some_and(|l| l.format != wordcraft_doc::section::NumFormat::Bullet)
}

/// Whether a menu command is greyed out, as Docs greys what doesn't apply where the caret is:
/// WordCraft's own check (`why_disabled`: table commands outside a table, …), plus Docs' rules
/// that WordCraft only checks when the command runs.
pub fn greyed(app: &WordApp, id: &str) -> bool {
    match id {
        "table.merge" => wordcraft_engine::cmd::table::selected_cells(&app.session).len() < 2,
        "stardocs.restartNumbering" => !in_numbered_list(app),
        // Format › Image: WordCraft's crop mode, and the items it keys to a selected picture.
        "ui.crop" => !wordcraft_ui_egui::widgets::enabled(app, id),
        "ui.changePicture" | "view.altTextPane" => app.session.why_disabled("picture.reset").is_some(),
        _ if id.starts_with("stardocs.") || id.starts_with("ui.") => false,
        _ => app.session.why_disabled(id).is_some(),
    }
}

/// Docs' three modes (the mode box and View › Mode).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Editing,
    Suggesting,
    Viewing,
}

impl Mode {
    pub fn of(app: &WordApp) -> Mode {
        if app.session.view.viewing {
            Mode::Viewing
        } else if app.session.doc.settings.track_changes {
            Mode::Suggesting
        } else {
            Mode::Editing
        }
    }
}

/// Switches mode. Editing or suggesting is WordCraft's track changes; Viewing is its read-only
/// state for this session (`view.viewing`, fork change StarDocs 0005, never saved in the
/// document), showing the document as it would print: no suggestions marked up.
pub fn set_mode(app: &mut WordApp, mode: Mode) {
    let viewing = mode == Mode::Viewing;
    if app.session.view.viewing != viewing {
        run(app, "view.viewing", json!({ "value": viewing }));
        run(app, "review.markup", json!({ "value": if viewing { "noMarkup" } else { "all" } }));
    }
    let suggesting = mode == Mode::Suggesting;
    if !viewing && app.session.doc.settings.track_changes != suggesting {
        run(app, "review.trackChanges", json!({ "value": suggesting }));
    }
}

/// StarDocs' own menu commands, which [`run_menu`] handles itself.
fn is_own(id: &str) -> bool {
    matches!(
        id,
        "stardocs.words"
            | "stardocs.compact"
            | "stardocs.shortcuts"
            | "stardocs.editing"
            | "stardocs.suggesting"
            | "stardocs.viewing"
            | "stardocs.download"
            | "stardocs.downloadAs"
            | "stardocs.rename"
            | "stardocs.restartNumbering"
    )
}

/// Menu commands: a few mean something else when the Starbase server keeps the document.
fn run_menu(app: &mut WordApp, ctx: &egui::Context, flags: &mut Flags, link: Option<&Link>, id: &str, p: Value) {
    match (id, link) {
        ("stardocs.words", _) => flags.words = !flags.words,
        ("stardocs.compact", _) => flags.compact = !flags.compact,
        ("stardocs.shortcuts", _) => flags.shortcuts = true,
        ("stardocs.restartNumbering", _) => flags.restart = true,
        // Someone who may only view the document stays in Viewing.
        ("stardocs.editing" | "stardocs.suggesting", Some(link)) if link.view_only() => app.status("You can view this document but not change it"),
        ("stardocs.editing", _) => set_mode(app, Mode::Editing),
        ("stardocs.suggesting", _) => set_mode(app, Mode::Suggesting),
        ("stardocs.viewing", _) => set_mode(app, Mode::Viewing),
        ("stardocs.download", Some(link)) => {
            link.download_next();
            run(app, "file.save", json!({}));
        }
        ("stardocs.download", None) => run(app, "file.save", json!({})),
        ("stardocs.downloadAs", _) => {
            let ext = p.get("ext").and_then(Value::as_str).unwrap_or("odt");
            if let Err(e) = download_as(app, link, ext) {
                log::warn!("download .{ext}: {e}");
                app.status(e);
            }
        }
        ("stardocs.rename", Some(_)) => {
            ctx.data_mut(|d| d.insert_temp(egui::Id::unique(RENAME), true));
        }
        ("stardocs.rename", None) => run(app, "ui.dialog:renameDocument", json!({})),
        ("file.new", Some(link)) => link.new_document(),
        ("file.new", None) => {
            run(app, "file.new", p);
            let mut doc = app.session.doc.clone();
            docs_defaults(&mut doc);
            app.session.set_document(doc);
        }
        ("file.open", Some(link)) => link.home(),
        _ => run(app, id, p),
    }
}

/// File › Download in a format other than Word's: a copy written by WordCraft's `formats` crate,
/// handed straight to the browser's download. It is never the document's own save (`file.saveAs`
/// would make an .odt the server's copy, or mark unsaved changes as saved).
fn download_as(app: &mut WordApp, link: Option<&Link>, ext: &str) -> Result<(), String> {
    let stem = match link.map(Link::name).filter(|n| !n.is_empty()) {
        Some(n) => n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s).to_string(),
        None => app.title_stem(),
    };
    let name = format!("{stem}.{ext}");
    let opts = wordcraft_engine::cmd::file::export_options(&app.session, &json!({}));
    let bytes = wordcraft_engine::io::save_bytes_with(&name, &app.session.doc, None, &opts)?;
    match &app.services.download {
        Some(download) => download(&name, &bytes),
        None => Err("downloads are unavailable here".into()),
    }
}

/// Google Docs' look for a new blank document: Arial, 11 pt body text, and Arial for the
/// headings too (WordCraft's Normal template uses Aptos 12 pt). The margins are already 1 inch
/// (2.54 cm) all round in WordCraft's blank document; they are set again here so they stay so.
pub fn docs_defaults(doc: &mut wordcraft_doc::Document) {
    use wordcraft_doc::styles::{BODY_FONT, HEADING_FONT};
    const FONT: &str = "Arial";
    let styles = std::sync::Arc::make_mut(&mut doc.styles);
    styles.default_chr.font = Some(FONT.into());
    styles.default_chr.size = Some(11.0);
    for st in &mut styles.styles {
        if matches!(st.chr.font.as_deref(), Some(BODY_FONT | HEADING_FONT)) {
            st.chr.font = Some(FONT.into());
        }
    }
    let s = &mut doc.last_section;
    (s.margin_top, s.margin_bottom, s.margin_left, s.margin_right) = (72.0, 72.0, 72.0, 72.0);
}

fn params(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| json!({}))
}

/// Runs a command; a refusal goes to the app's status line instead of vanishing.
pub fn run(app: &mut WordApp, id: &str, p: Value) {
    let r = match id.strip_prefix("ui.dialog:") {
        Some(name) => app.run("ui.dialog", json!({ "name": name })),
        None => app.run(id, p),
    };
    if let Err(e) = r {
        log::warn!("{id}: {e}");
        app.status(e);
    }
    // A click on our toolbar or menus leaves the page without the keyboard (WordCraft gives it
    // back only after clicks on its own chrome): hand it back, unless a dialog took over.
    if app.dialog.is_none() && !id.starts_with("ui.dialog") {
        app.canvas.want_focus = true;
    }
}

/// What the toolbar shows about the selection.
#[derive(Default)]
struct Current {
    /// The selection's font as the document names it; blank when it mixes fonts.
    font: String,
    /// The face it is drawn in when that font isn't available.
    shown_as: Option<String>,
    /// Points; 0 when the selection mixes sizes.
    size: f64,
    /// The caret paragraph's style name, as the style box shows it; blank when the selection mixes styles.
    style: String,
    bold: bool,
    italic: bool,
    underline: bool,
    zoom: f64,
}

/// A style's name as the style box shows it: Google Docs' wording for the styles it lists
/// ("Normal" is "Normal text"; Word files name headings "heading 1"), others as they are.
fn style_label(name: &str) -> String {
    STYLES.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).map_or(name, |(label, _)| label).to_string()
}

fn current(app: &mut WordApp) -> Current {
    let f = app.run("format.state", json!({})).unwrap_or_default();
    let v = app.run("view.state", json!({})).unwrap_or_default();
    let on = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(false);
    Current {
        // `format.state` gives null for a font or size the selection mixes.
        font: f.get("font").and_then(Value::as_str).unwrap_or("").to_string(),
        shown_as: f.get("fontShownAs").and_then(Value::as_str).map(str::to_string),
        size: f.get("size").and_then(Value::as_f64).unwrap_or(0.0),
        // Paragraphs of different styles selected (null): an empty box, as Google Docs shows.
        style: match f.get("styleName").or_else(|| f.get("style")) {
            Some(Value::Null) => String::new(),
            v => style_label(v.and_then(Value::as_str).unwrap_or("Normal")),
        },
        bold: on("bold"),
        italic: on("italic"),
        underline: f.get("underline").is_some_and(|u| u.as_bool().unwrap_or(!u.is_null() && u != "none")),
        zoom: v.get("zoom").and_then(Value::as_f64).unwrap_or(1.0),
    }
}

impl Shell {
    /// What the window shows, as JSON, for tests and AI agents (the test hook, `testhook.rs`):
    /// filled in by the lead; keys are stable names, values plain JSON.
    /// Runs one of StarDocs' own commands (`stardocs.*`, as the menus do) for the test hook.
    /// `stardocs.rename` takes `{"name": …}` and renames at once; `stardocs.share` opens the
    /// Share dialog; `stardocs.customColour` opens the Custom colour dialog.
    pub fn test_command(&mut self, app: &mut WordApp, link: Option<&Link>, id: &str, params: Value) -> Result<Value, String> {
        let ctx = self.ctx.clone().ok_or("the window hasn't been drawn yet")?;
        match (id, link) {
            ("stardocs.rename", Some(link)) if params.get("name").is_some() => {
                let name = params["name"].as_str().ok_or("name must be text")?;
                link.rename(name);
            }
            ("stardocs.share", Some(link)) => self.share.ask(app, link, &ctx),
            ("stardocs.share", None) => return Err("sharing needs a Starbase server document".into()),
            ("stardocs.customColour", _) => self.custom = Some(BLUE),
            _ if is_own(id) => run_menu(app, &ctx, &mut self.flags, link, id, params),
            _ => return Err(format!("{id}: not a Starbase Docs command")),
        }
        ctx.request_repaint();
        Ok(json!(true))
    }

    pub fn test_state(&mut self, app: &mut WordApp, link: Option<&Link>) -> Value {
        let cur = current(app);
        let saved = link.map(|l| match l.saved() {
            Saved::Nothing => "nothing".to_string(),
            Saved::Saving => "saving".to_string(),
            Saved::Done => "saved".to_string(),
            Saved::Failed(why) => format!("failed: {why}"),
        });
        json!({
            "title": link.map(Link::name),
            "saved": saved,
            "notice": link.and_then(Link::notice),
            "viewOnly": link.is_some_and(Link::view_only),
            "mode": match Mode::of(app) {
                _ if link.is_some_and(Link::view_only) => "viewing",
                Mode::Viewing => "viewing",
                Mode::Suggesting => "suggesting",
                Mode::Editing => "editing",
            },
            "compact": self.flags.compact,
            "wordCountShown": self.flags.words,
            "renaming": self.renaming,
            "dialogs": {
                "share": self.share.is_open(),
                "shortcuts": self.shortcuts.open,
                "customColour": self.custom.is_some(),
                "restartNumbering": self.restart.is_some(),
                "wordcraft": app.dialog.is_some(),
            },
            "toolbar": {
                "font": cur.font, "fontShownAs": cur.shown_as, "size": cur.size, "style": cur.style,
                "bold": cur.bold, "italic": cur.italic, "underline": cur.underline, "zoom": cur.zoom,
                "painting": app.session.painter.is_some(),
                "groupWidths": self.widths.to_vec(),
                "groupsInMoreMenu": self.in_more,
                "groupRoom": self.room,
                "rightEndWidth": self.right_end,
            },
            "fontsListed": self.fonts.as_ref().map_or(0, Vec::len),
        })
    }

    /// Draws the title row, menus and toolbar above the page. Call before `WordApp::ui`.
    pub fn ui(&mut self, app: &mut WordApp, ui: &mut Ui, link: Option<&Link>) {
        self.ctx = Some(ui.ctx().clone());
        crate::theme::keep_light(app);
        crate::theme::install(ui.ctx());
        let before = self.flags;
        if !self.flags.loaded {
            // Docs has no floating mini toolbar over selected text.
            app.ui.mini_toolbar = false;
            // A selected picture's right-click menu: Format › Image's items.
            app.set_picture_menu(Some(picture_menu));
        }
        self.flags.sync(before);
        self.keys(app, ui.ctx());
        egui::Panel::top("stardocs_head")
            .frame(egui::Frame::NONE.fill(PAGE).inner_margin(egui::Margin { left: 12, right: 16, top: 8, bottom: 6 }))
            .show(ui, |ui| {
                self.share.logic(link, ui.ctx());
                if !self.flags.compact {
                    self.title_and_menus(app, ui, link);
                    ui.add_space(6.0);
                }
                // A view-only document, or Viewing chosen: the toolbar is shown greyed, like
                // Google Docs' viewer. Only someone who may edit can leave Viewing.
                let may_edit = !link.is_some_and(Link::view_only);
                self.toolbar(app, ui, may_edit);
            });
        self.share.ui(ui.ctx());
        self.custom_colour(app, ui.ctx());
        if std::mem::take(&mut self.flags.shortcuts) {
            self.shortcuts.open = true;
        }
        if std::mem::take(&mut self.flags.restart) {
            self.restart = Some("1".into());
            self.restart_focus = true;
        }
        self.restart_numbering(app, ui.ctx());
        self.shortcuts.ui(ui.ctx(), MENUS);
        if self.flags.words {
            self.word_count.ui(app, ui.ctx());
        }
        self.flags.sync(before);
    }

    /// StarDocs' own shortcuts, taken before WordCraft sees the keys.
    fn keys(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut, Modifiers};
        let ctrl_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let ctrl_alt = Modifiers::COMMAND | Modifiers::ALT;
        let (menus, outline, help, count) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::F)),
                i.consume_shortcut(&KeyboardShortcut::new(ctrl_alt, Key::H)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Slash)),
                // Docs' word count; WordCraft would copy formatting (Word's Ctrl+Shift+C).
                i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::C)),
            )
        });
        // Docs' text size up and down by one point (WordCraft's keys step through Word's size
        // list), and Headings 4 to 6, which WordCraft has no keys for.
        let more: [(Modifiers, Key, &str, Value); 6] = [
            (ctrl_shift, Key::Period, "format.growFont1", json!({})),
            (ctrl_shift, Key::Comma, "format.shrinkFont1", json!({})),
            (ctrl_alt, Key::Num4, "para.style", json!({"style": "Heading 4"})),
            (ctrl_alt, Key::Num5, "para.style", json!({"style": "Heading 5"})),
            (ctrl_alt, Key::Num6, "para.style", json!({"style": "Heading 6"})),
            // Docs' checklist (fork change StarDocs 0006).
            (ctrl_shift, Key::Num9, "list.checklist", json!({})),
        ];
        for (m, k, id, p) in more {
            if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(m, k))) {
                run(app, id, p);
            }
        }
        if count {
            run(app, "ui.dialog:wordCount", json!({}));
        }
        if menus {
            self.flags.compact = !self.flags.compact;
        }
        if outline {
            run(app, "view.navigationPane", json!({}));
        }
        if help {
            self.flags.shortcuts = true;
        }
    }

    fn toolbar(&mut self, app: &mut WordApp, ui: &mut Ui, may_edit: bool) {
        let editable = may_edit && Mode::of(app) != Mode::Viewing;
        let cur = current(app);
        let fonts = self.fonts.get_or_insert_with(|| font_names(app)).clone();
        let flags = &mut self.flags;
        let widths = &mut self.widths;
        let in_more = &mut self.in_more;
        let right_end = &mut self.right_end;
        let room_seen = &mut self.room;
        let mut tools = Tools { app, cur: &cur, fonts: &fonts, custom: &mut self.custom };
        egui::Frame::NONE.fill(TOOLBAR).corner_radius(24.0).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                flat(ui);
                tight(ui);
                // As many groups as fit, in order; the rest go into the ⋮ menu (as Docs does),
                // leaving room for the mode box and arrow at the right end.
                // The right end's width as drawn last frame (a guess before the first).
                let mut room = ui.available_width() - if *right_end > 0.0 { *right_end + 8.0 } else { RIGHT_END };
                *room_seen = room;
                let mut shown = GROUPS.len();
                let mut used = 0.0;
                for (i, w) in widths.iter().enumerate() {
                    if used + w > room {
                        shown = i;
                        break;
                    }
                    used += w;
                }
                if shown < GROUPS.len() {
                    room -= MORE;
                    used = 0.0;
                    shown = widths
                        .iter()
                        .take_while(|w| {
                            used += *w;
                            used <= room
                        })
                        .count();
                }
                *in_more = GROUPS.len() - shown;
                ui.add_enabled_ui(editable, |ui| {
                    for (i, g) in GROUPS.iter().take(shown).enumerate() {
                        let x = ui.cursor().left();
                        tools.draw(ui, *g);
                        if i + 1 < GROUPS.len() {
                            bar(ui);
                        }
                        widths[i] = ui.cursor().left() - x;
                    }
                    if shown < GROUPS.len() {
                        icon_menu(ui, "more_vert", "More", |ui| {
                            for g in &GROUPS[shown..] {
                                ui.horizontal(|ui| {
                                    flat(ui);
                                    tight(ui);
                                    tools.draw(ui, *g);
                                });
                            }
                        });
                    }
                });
                // The right end: the mode switch and the compact-mode arrow (Docs' "Hide the menus").
                let end = ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let start = ui.cursor().right();
                    let (icon, tip) = if flags.compact {
                        ("keyboard_arrow_down", "Show the menus (Ctrl+Shift+F)")
                    } else {
                        ("keyboard_arrow_up", "Hide the menus (Ctrl+Shift+F)")
                    };
                    if icon_button(ui, icon, false).on_hover_text(tip).clicked() {
                        flags.compact = !flags.compact;
                    }
                    mode_switch(tools.app, ui, may_edit);
                    start - ui.cursor().right()
                });
                *right_end = end.inner;
            });
        });
    }

    /// Docs' custom colour: a small dialog with a colour picker, opened from the text colour menu.
    /// Docs' Restart numbering dialog: the caret's numbered list starts again here, at the number
    /// typed (`para.setNumberingValue`).
    fn restart_numbering(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        let Some(mut text) = self.restart.take() else { return };
        let mut done = None;
        let modal = egui::Modal::new(egui::Id::unique("stardocs_restart_numbering"))
            .frame(egui::Frame::popup(&ctx.global_style()).fill(Color32::WHITE).corner_radius(8.0).inner_margin(20.0));
        let shown = modal.show(ctx, |ui| {
            ui.label(RichText::new("Restart numbering").size(18.0).color(TEXT));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Start at").size(14.0).color(TEXT));
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(80.0));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Start at"));
                if std::mem::take(&mut self.restart_focus) {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    done = Some(true);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
                if ui.add(Button::new(RichText::new("OK").color(Color32::WHITE)).fill(BLUE)).clicked() {
                    done = Some(true);
                }
            });
        });
        if shown.should_close() {
            done.get_or_insert(false);
        }
        match done {
            Some(true) => match text.trim().parse::<u32>() {
                Ok(n) => run(app, "para.setNumberingValue", json!({ "value": n })),
                Err(_) => {
                    app.status("Type a whole number to start the list at".to_string());
                    self.restart = Some(text);
                }
            },
            Some(false) => {}
            None => self.restart = Some(text),
        }
    }

    fn custom_colour(&mut self, app: &mut WordApp, ctx: &egui::Context) {
        let Some(mut c) = self.custom else { return };
        let mut done = None;
        let modal = egui::Modal::new(egui::Id::unique("stardocs_custom_colour"))
            .frame(egui::Frame::popup(&ctx.global_style()).fill(Color32::WHITE).corner_radius(8.0).inner_margin(20.0));
        let shown = modal.show(ctx, |ui| {
            ui.label(RichText::new("Custom colour").size(18.0).color(TEXT));
            ui.add_space(8.0);
            egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
                if ui.add(Button::new(RichText::new("OK").color(Color32::WHITE)).fill(BLUE)).clicked() {
                    done = Some(true);
                }
            });
        });
        if shown.should_close() {
            done.get_or_insert(false);
        }
        match done {
            Some(true) => {
                run(app, "format.color", json!({ "color": format!("{:02X}{:02X}{:02X}", c.r(), c.g(), c.b()) }));
                self.custom = None;
            }
            Some(false) => self.custom = None,
            None => self.custom = Some(c),
        }
    }
}

/// The toolbar's groups, left to right, with a first guess at their widths (measured after
/// the first frame).
#[derive(Clone, Copy)]
enum Group {
    History,
    Zoom,
    Style,
    Font,
    Size,
    Text,
    Insert,
    Align,
    Lists,
}
const GROUPS: [Group; 9] =
    [Group::History, Group::Zoom, Group::Style, Group::Font, Group::Size, Group::Text, Group::Insert, Group::Align, Group::Lists];
pub const GUESSED_WIDTHS: [f32; 9] = [151.0, 79.0, 129.0, 109.0, 99.0, 151.0, 95.0, 151.0, 140.0];
/// Room kept for the mode box and the compact-mode arrow, and for the ⋮ button.
const RIGHT_END: f32 = 170.0;
const MORE: f32 = 34.0;

/// What the toolbar's groups draw with.
struct Tools<'a> {
    app: &'a mut WordApp,
    cur: &'a Current,
    fonts: &'a [String],
    custom: &'a mut Option<Color32>,
}

impl Tools<'_> {
    fn draw(&mut self, ui: &mut Ui, g: Group) {
        let (app, cur) = (&mut *self.app, self.cur);
        match g {
            Group::History => {
                tool(app, ui, "undo", "Undo (Ctrl+Z)", "edit.undo", json!({}), false);
                tool(app, ui, "redo", "Redo (Ctrl+Y)", "edit.redo", json!({}), false);
                tool(app, ui, "print", "Print (Ctrl+P)", "file.print", json!({}), false);
                tool(app, ui, "spellcheck", "Spelling and grammar", "review.spelling", json!({}), false);
                let painting = app.session.painter.is_some();
                let tip = "Paint format: copy this text's look, then select where it goes";
                tool(app, ui, "format_paint", tip, "edit.formatPainter", json!({}), painting);
            }
            Group::Zoom => {
                let zoom = format!("{}%", (cur.zoom * 100.0).round());
                drop_menu(ui, &zoom, 66.0, |ui| {
                    for z in ZOOMS {
                        if ui.button(format!("{z}%")).clicked() {
                            run(app, "view.zoom", json!({ "value": z }));
                            ui.close();
                        }
                    }
                    if ui.button("Fit").clicked() {
                        run(app, "view.zoom", json!({ "value": "pageWidth" }));
                        ui.close();
                    }
                });
            }
            Group::Style => {
                drop_menu(ui, &cur.style, 116.0, |ui| {
                    ui.set_min_width(180.0);
                    for (label, name) in STYLES {
                        let size = match *name {
                            "Title" => 22.0,
                            "Heading 1" => 18.0,
                            "Heading 2" | "Subtitle" => 16.0,
                            "Heading 3" => 14.0,
                            _ => 13.0,
                        };
                        if ui.button(RichText::new(*label).size(size)).clicked() {
                            run(app, "para.style", json!({ "style": name }));
                            ui.close();
                        }
                    }
                });
            }
            Group::Font => {
                let fonts = self.fonts;
                let font = drop_menu(ui, &cur.font, 96.0, |ui| {
                    ui.set_min_width(200.0);
                    egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                        for f in fonts {
                            if ui.button(RichText::new(f).size(14.0)).clicked() {
                                run(app, "format.font", json!({ "name": f }));
                                ui.close();
                            }
                        }
                    });
                });
                match &cur.shown_as {
                    Some(face) => font.on_hover_text(format!("Font: {} (not installed, shown as {face})", cur.font)),
                    None => font.on_hover_text("Font"),
                };
            }
            Group::Size => {
                tool(app, ui, "remove", "Decrease font size", "format.shrinkFont1", json!({}), false);
                size_box(app, ui, cur.size);
                tool(app, ui, "add", "Increase font size", "format.growFont1", json!({}), false);
            }
            Group::Text => {
                tool(app, ui, "format_bold", "Bold (Ctrl+B)", "format.bold", json!({}), cur.bold);
                tool(app, ui, "format_italic", "Italic (Ctrl+I)", "format.italic", json!({}), cur.italic);
                tool(app, ui, "format_underlined", "Underline (Ctrl+U)", "format.underline", json!({}), cur.underline);
                let custom = &mut *self.custom;
                icon_menu(ui, "format_color_text", "Text colour", |ui| {
                    palette(ui, COLOURS.iter().map(|c| (*c, *c)), |c| run(app, "format.color", json!({ "color": c })));
                    ui.horizontal(|ui| {
                        if ui.button("Automatic").clicked() {
                            run(app, "format.color", json!({ "color": "auto" }));
                            ui.close();
                        }
                        // Opens its own dialog (Shell::custom_colour): a click in a menu closes it.
                        if ui.button("Custom…").clicked() {
                            *custom = Some(BLUE);
                            ui.close();
                        }
                    });
                });
                icon_menu(ui, "ink_highlighter", "Highlight colour", |ui| {
                    palette(ui, HIGHLIGHTS.iter().copied(), |c| run(app, "format.highlight", json!({ "color": c })));
                    if ui.button("None").clicked() {
                        run(app, "format.highlight", json!({ "color": "none" }));
                        ui.close();
                    }
                });
            }
            Group::Insert => {
                tool(app, ui, "link", "Insert link (Ctrl+K)", "ui.dialog:link", json!({}), false);
                tool(app, ui, "add_comment", "Add comment (Ctrl+Alt+M)", "review.newComment", json!({}), false);
                tool(app, ui, "image", "Insert image", "insert.picture", json!({}), false);
            }
            Group::Align => {
                tool(app, ui, "format_align_left", "Left align (Ctrl+Shift+L)", "para.alignLeft", json!({}), false);
                tool(app, ui, "format_align_center", "Centre align (Ctrl+Shift+E)", "para.alignCenter", json!({}), false);
                tool(app, ui, "format_align_right", "Right align (Ctrl+Shift+R)", "para.alignRight", json!({}), false);
                tool(app, ui, "format_align_justify", "Justify (Ctrl+Shift+J)", "para.justify", json!({}), false);
                icon_menu(ui, "format_line_spacing", "Line and paragraph spacing", |ui| {
                    for (label, v) in [("Single", 1.0), ("1.15", 1.15), ("1.5", 1.5), ("Double", 2.0)] {
                        if ui.button(label).clicked() {
                            run(app, "para.lineSpacing", json!({ "value": v }));
                            ui.close();
                        }
                    }
                });
            }
            Group::Lists => {
                tool(app, ui, "format_list_bulleted", "Bulleted list (Ctrl+Shift+8)", "para.bullets", json!({}), false);
                tool(app, ui, "format_list_numbered", "Numbered list (Ctrl+Shift+7)", "para.numbering", json!({}), false);
                tool(app, ui, "format_indent_decrease", "Decrease indent (Ctrl+[)", "para.outdent", json!({}), false);
                tool(app, ui, "format_indent_increase", "Increase indent (Ctrl+])", "para.indent", json!({}), false);
                tool(app, ui, "format_clear", "Clear formatting (Ctrl+\\)", "format.clear", json!({}), false);
            }
        }
    }
}

/// Docs' mode box at the right of the toolbar: Editing, Suggesting or Viewing ([`set_mode`]),
/// or a fixed "Viewing" for someone who may only view the document.
fn mode_switch(app: &mut WordApp, ui: &mut Ui, may_edit: bool) {
    let mode = if may_edit { Mode::of(app) } else { Mode::Viewing };
    let (icon, label) = match mode {
        Mode::Viewing => ("visibility", "Viewing"),
        Mode::Suggesting => ("rate_review", "Suggesting"),
        Mode::Editing => ("edit", "Editing"),
    };
    let text = RichText::new(label).size(14.0).color(TEXT_DIM);
    let button = match icons::icon(icon) {
        Some(img) => Button::image_and_text(img, text),
        None => Button::new(text),
    };
    if !may_edit {
        ui.add_enabled(false, button).on_disabled_hover_text("You can view this document but not change it");
        return;
    }
    let r = ui.add(button.corner_radius(4.0).min_size(vec2(116.0, 28.0)));
    let tip = match mode {
        Mode::Viewing => "Viewing: read or print the final document",
        Mode::Suggesting => "Suggesting: your edits show as suggestions others can accept",
        Mode::Editing => "Editing: you change the document directly",
    };
    let r = r.on_hover_text(tip);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(300.0);
        for (to, icon, title, about) in [
            (Mode::Editing, "edit", "Editing", "Edit the document directly"),
            (Mode::Suggesting, "rate_review", "Suggesting", "Edits become suggestions"),
            (Mode::Viewing, "visibility", "Viewing", "Read or print the final document"),
        ] {
            let text = RichText::new(format!("{title}\n{about}")).size(14.0).color(TEXT);
            let item = match icons::icon(icon) {
                Some(img) => Button::image_and_text(img, text),
                None => Button::new(text),
            };
            if ui.add(item.selected(to == mode).min_size(vec2(300.0, 44.0))).clicked() {
                set_mode(app, to);
                ui.close();
            }
        }
    });
}

fn font_names(app: &mut WordApp) -> Vec<String> {
    let v = app.run("font.list", json!({ "all": true })).unwrap_or_default();
    // "families" is every font it can draw; "fonts" only the ones the user added.
    let list = v.get("families").or_else(|| v.get("fonts")).unwrap_or(&v);
    let mut out: Vec<String> = list
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|f| {
                    f.as_str().map(str::to_string).or_else(|| f.get("family").or_else(|| f.get("name")).and_then(Value::as_str).map(str::to_string))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|s| s.to_lowercase());
    out.dedup();
    out
}

impl Shell {
    fn title_and_menus(&mut self, app: &mut WordApp, ui: &mut Ui, link: Option<&Link>) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let logo = ui.add(icons::logo(36.0).sense(egui::Sense::click()));
            ui.add_space(6.0);
            if let Some(link) = link {
                if logo.on_hover_text("Starbase home").clicked() {
                    link.home();
                }
            } else {
                logo.on_hover_text("Starbase Docs");
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| self.title(app, ui, link));
                ui.horizontal(|ui| {
                    flat(ui);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.spacing_mut().button_padding = vec2(7.0, 2.0);
                    for (title, items) in MENUS {
                        menu_bar_menu(ui, title, |ui| menu_items(app, ui, &mut self.flags, items, link));
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if self.share.waiting() { "Saving…" } else { "Share" };
                let share = Button::new(RichText::new(label).size(14.0).color(TEXT)).fill(SHARE).corner_radius(18.0).min_size(vec2(84.0, 36.0));
                match link {
                    // Opening failed (or hasn't finished): nothing to share yet.
                    Some(link) if link.id().is_some() && link.name().is_empty() && !self.share.waiting() => {
                        ui.add_enabled(false, share).on_disabled_hover_text("The document isn't open");
                    }
                    Some(link) => {
                        if ui.add(share).on_hover_text("Share this document with people on this server").clicked() {
                            self.share.ask(app, link, ui.ctx());
                        }
                    }
                    None => {
                        ui.add_enabled(false, share)
                            .on_disabled_hover_text("Sharing needs a Starbase server: open the document from the home screen");
                    }
                }
                // Docs' comment history button: opens WordCraft's comments panel.
                ui.add_space(8.0);
                flat(ui);
                let open = is_on(app, &self.flags, "commentsPane");
                if icon_button(ui, "comment", open).on_hover_text("Show all comments").clicked() {
                    run(app, "view.commentsPane", json!({}));
                }
            });
        });
    }

    /// The document's name (click to rename a server document) and, beside it, how saving is going.
    fn title(&mut self, app: &mut WordApp, ui: &mut Ui, link: Option<&Link>) {
        let server_name = link.map(Link::name).filter(|n| !n.is_empty());
        let stem = match &server_name {
            Some(n) => n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s).to_string(),
            None if link.is_some() => "Untitled document".to_string(),
            None => app.title_stem(),
        };
        let asked = ui.ctx().data_mut(|d| d.remove_temp::<bool>(egui::Id::unique(RENAME))).unwrap_or(false);
        if asked && server_name.is_some() {
            self.renaming = Some(stem.clone());
        }
        if let (Some(text), Some(link)) = (&mut self.renaming, link) {
            // Docs' rename box hugs the name (it grows as you type), with a 2 px blue outline and
            // 4 px corners.
            let font = egui::FontId::proportional(18.0);
            let width = ui.painter().layout_no_wrap(text.clone(), font.clone(), TEXT).size().x + 18.0; // the 6 px margins, and room for the caret
            let edit = ui
                .scope(|ui| {
                    let v = ui.visuals_mut();
                    v.selection.stroke = Stroke::new(2.0, BLUE);
                    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
                        w.bg_stroke = Stroke::new(2.0, BLUE);
                        w.corner_radius = egui::CornerRadius::same(4);
                    }
                    ui.add(egui::TextEdit::singleline(text).font(font).margin(vec2(6.0, 0.0)).desired_width(width.clamp(40.0, 600.0)))
                })
                .inner;
            if asked {
                edit.request_focus();
            }
            let done = edit.lost_focus();
            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if done || cancel {
                let new = text.trim().to_string();
                if !cancel && !new.is_empty() && new != stem {
                    link.rename(&new);
                }
                self.renaming = None;
            }
        } else {
            let label = ui.add(egui::Label::new(RichText::new(&stem).size(18.0).color(TEXT)).sense(egui::Sense::click()));
            if server_name.is_some() && label.on_hover_text("Rename").clicked() {
                self.renaming = Some(stem.clone());
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(RENAME), true));
            }
        }
        if let Some(link) = link {
            let (text, colour) = match link.saved() {
                _ if link.view_only() => ("View only".to_string(), TEXT_DIM),
                Saved::Nothing => (String::new(), TEXT_DIM),
                Saved::Saving => ("Saving…".to_string(), TEXT_DIM),
                Saved::Done => ("Saved to Starbase".to_string(), TEXT_DIM),
                Saved::Failed(why) => (why, Color32::from_rgb(0xb3, 0x26, 0x1e)),
            };
            if !text.is_empty() {
                ui.add_space(10.0);
                ui.label(RichText::new(text).size(13.0).color(colour));
            }
            if let Some(notice) = link.notice() {
                ui.add_space(10.0);
                ui.label(RichText::new(notice).size(13.0).color(Color32::from_rgb(0x8a, 0x5a, 0x00)));
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
    }
}

/// Docs' menu rows: 32 px tall whatever they hold, 16 px from the sides.
const ROW: f32 = 32.0;
const ROW_INSET: f32 = 16.0;
/// The icon column: 20 px icons (or ticks), 12 px before the label.
const ICON: f32 = 20.0;
const ICON_GAP: f32 = 12.0;
/// Shortcuts and submenu arrows: Docs' grey.
const KEYS_GREY: Color32 = Color32::from_rgb(0x5F, 0x63, 0x68);
/// Menu dividers.
const MENU_LINE: Color32 = Color32::from_rgb(0xDA, 0xDC, 0xE0);

/// Sets a menu's `Ui` up the Docs way: flat full-width one-line rows, 32 px tall, grey on
/// hover (the frame round it is [`docs_menu_style`]).
fn menu_look(ui: &mut Ui) {
    let st = ui.style_mut();
    // One line per row, always (a wrapped label made a taller row).
    st.wrap_mode = Some(egui::TextWrapMode::Extend);
    st.spacing.button_padding = vec2(ROW_INSET, 6.0);
    st.spacing.item_spacing = vec2(ICON_GAP, 0.0);
    st.spacing.icon_spacing = ICON_GAP;
    st.spacing.interact_size.y = ROW;
    let w = &mut st.visuals.widgets;
    for (state, fill) in [
        (&mut w.inactive, Color32::TRANSPARENT),
        (&mut w.hovered, crate::theme::MENU_HOVER),
        (&mut w.active, crate::theme::HOVER),
        (&mut w.open, crate::theme::MENU_HOVER),
    ] {
        state.bg_stroke = Stroke::NONE;
        state.weak_bg_fill = fill;
        state.corner_radius = egui::CornerRadius::ZERO;
        state.expansion = 0.0;
    }
}

/// The frame of our menus (and of their submenus, which inherit it): 8 px top and bottom, none at
/// the sides so rows and dividers reach the edges, 8 px corners. A pop-up's frame takes the
/// window's style (WordCraft's 6 px all round) plus this.
fn docs_menu_style() -> egui::style::StyleModifier {
    egui::style::StyleModifier::new(|s: &mut egui::Style| {
        egui::containers::menu::menu_style(s);
        s.spacing.menu_margin = egui::Margin::symmetric(0, 8);
        s.visuals.menu_corner_radius = egui::CornerRadius::same(8);
    })
}

/// One of the menu bar's menus with [`docs_menu_style`].
fn menu_bar_menu(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    let config = egui::containers::menu::MenuConfig::new().style(docs_menu_style());
    let _ = egui::containers::menu::MenuButton::new(RichText::new(title).size(14.0).color(TEXT)).config(config).ui(ui, add);
}

/// The wording a menu entry shows, and its shortcut.
fn wording<'a>(app: &mut WordApp, flags: &Flags, item: &'a Item) -> (&'a str, &'a str) {
    match item {
        Cmd(label, _, _, keys) | Toggle(label, _, _, keys) => (*label, *keys),
        Flip(off, on, _, key) => (if is_on(app, flags, key) { *on } else { *off }, ""),
        Sub(label, _) => (*label, ""),
        Sep | TableGrid => ("", ""),
    }
}

/// Whether a menu has the icon column (any icon or tick in it): then every label lines up after it.
fn has_icons(items: &[Item]) -> bool {
    items.iter().any(|i| match i {
        Toggle(..) => true,
        Cmd(label, ..) | Sub(label, _) => icons::menu_icon_name(label).is_some(),
        _ => false,
    })
}

/// A menu row: the icon column (an icon, a tick, or the same space blank), the label, and the
/// shortcut in grey at the right.
fn menu_row(icon: Option<&str>, column: bool, label: &str, keys: &str, width: f32) -> Button<'static> {
    let text = RichText::new(label.to_string()).size(14.0).color(TEXT);
    let b = match icon.and_then(|n| icons::icon_px(n, ICON)) {
        Some(img) if column => Button::image_and_text(img, text),
        // Keeps the label lined up with its iconed neighbours.
        _ if column => match icons::icon_px("check", ICON) {
            Some(blank) => Button::image_and_text(blank.tint(Color32::TRANSPARENT), text),
            None => Button::new(text),
        },
        _ => Button::new(text),
    };
    // The shortcut's stretch keeps the label at the left; without one an empty growing space
    // does (an empty shortcut text would add a space to the entry's name).
    let b =
        if keys.is_empty() { b.right_text(egui::Atom::grow()) } else { b.shortcut_text(RichText::new(keys.to_string()).size(13.0).color(KEYS_GREY)) };
    b.min_size(vec2(width, ROW))
}

/// How wide a menu is: its longest label plus shortcut, the icon column and the padding (at
/// least 240 px, as Docs' shortest menus).
fn menu_width(app: &mut WordApp, ui: &Ui, flags: &Flags, items: &[Item], column: bool) -> f32 {
    let measure = |t: &str, size: f32| ui.painter().layout_no_wrap(t.to_string(), egui::FontId::proportional(size), TEXT).size().x;
    let widest = items
        .iter()
        .map(|i| {
            let (label, keys) = wording(app, flags, i);
            let keys = if keys.is_empty() { 0.0 } else { 32.0 + measure(keys, 13.0) };
            measure(label, 14.0) + keys + if matches!(i, Sub(..)) { ICON + ICON_GAP + 8.0 } else { 0.0 }
        })
        .fold(0.0_f32, f32::max);
    (widest + 2.0 * ROW_INSET + if column { ICON + ICON_GAP } else { 0.0 } + 8.0).max(240.0).ceil()
}

/// A menu divider: a light line with 8 px above and below, the menu's full width.
fn menu_sep(ui: &mut Ui, width: f32) {
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, MENU_LINE));
    ui.add_space(8.0);
}

fn menu_items(app: &mut WordApp, ui: &mut Ui, flags: &mut Flags, items: &[Item], link: Option<&Link>) {
    menu_look(ui);
    let column = has_icons(items);
    let width = menu_width(app, ui, flags, items, column);
    ui.set_min_width(width);
    ui.set_max_width(width);
    for item in items {
        let (label, keys) = wording(app, flags, item);
        match item {
            Cmd(_, id, p, _) => {
                let b = menu_row(icons::menu_icon_name(label), column, label, keys, width);
                if ui.add_enabled(!greyed(app, id), b).clicked() {
                    run_menu(app, ui.ctx(), flags, link, id, params(p));
                    ui.close();
                }
            }
            Toggle(_, id, key, _) => {
                // A tick in the icon column when it is on, as Docs' View menu shows.
                let tick = is_on(app, flags, key).then_some("check");
                if ui.add_enabled(!greyed(app, id), menu_row(tick, column, label, keys, width)).clicked() {
                    run_menu(app, ui.ctx(), flags, link, id, json!({}));
                    ui.close();
                }
            }
            Flip(_, _, id, _) => {
                if ui.add_enabled(!greyed(app, id), menu_row(None, column, label, "", width)).clicked() {
                    run_menu(app, ui.ctx(), flags, link, id, json!({}));
                    ui.close();
                }
            }
            TableGrid => {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(ROW_INSET);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        table_grid(app, ui);
                    });
                });
                ui.add_space(4.0);
            }
            Sub(_, sub) => {
                // Docs' submenu arrow: `arrow_right` the size of the icons (a picture, so the
                // entry is still named just by its wording).
                let b = menu_row(icons::menu_icon_name(label), column, label, "", width);
                let b = match icons::icon_px("arrow_right", ICON) {
                    Some(arrow) => b.right_text(arrow),
                    None => b.right_text(egui::containers::menu::SubMenuButton::arrow_atom(Some(KEYS_GREY))),
                };
                egui::containers::menu::SubMenuButton::from_button(b).ui(ui, |ui| menu_items(app, ui, flags, sub, link));
            }
            Sep => menu_sep(ui, width),
        }
    }
}

/// Docs' Insert › Table grid: hover to choose rows × columns (up to 10 × 10), click to insert.
fn table_grid(app: &mut WordApp, ui: &mut Ui) {
    const N: usize = 10;
    const CELL: f32 = 18.0;
    const GAP: f32 = 3.0;
    let id = ui.scope_id().with("stardocs_table_grid");
    let side = N as f32 * (CELL + GAP);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), egui::Sense::click());
    named(ui, &resp, "Table size");
    // The size under the pointer, kept while the pointer is outside so the label doesn't flicker.
    let mut pick: (usize, usize) = ui.data(|d| d.get_temp(id)).unwrap_or((1, 1));
    if let Some(p) = resp.hover_pos() {
        let col = (((p.x - rect.left()) / (CELL + GAP)) as usize).min(N - 1) + 1;
        let row = (((p.y - rect.top()) / (CELL + GAP)) as usize).min(N - 1) + 1;
        pick = (row, col);
        ui.data_mut(|d| d.insert_temp(id, pick));
    }
    let painter = ui.painter();
    for r in 0..N {
        for c in 0..N {
            let min = rect.min + vec2(c as f32 * (CELL + GAP), r as f32 * (CELL + GAP));
            let cell = egui::Rect::from_min_size(min, vec2(CELL, CELL));
            let lit = r < pick.0 && c < pick.1;
            painter.rect_filled(cell, 2.0, if lit { ON } else { Color32::WHITE });
            painter.rect_stroke(cell, 2.0, Stroke::new(1.0, if lit { BLUE } else { DIVIDER }), egui::StrokeKind::Inside);
        }
    }
    ui.label(RichText::new(format!("{} × {}", pick.1, pick.0)).size(13.0).color(TEXT_DIM));
    if resp.clicked() {
        run(app, "insert.table", json!({ "rows": pick.0, "cols": pick.1 }));
        ui.close();
    }
}

/// Docs' toolbar spacing: icon buttons 26 px wide (an 18 px icon and 4 px each side) with 2 px
/// between them, so the whole toolbar fits a 1400 px window.
fn tight(ui: &mut Ui) {
    let s = ui.spacing_mut();
    s.item_spacing.x = 2.0;
    s.button_padding = vec2(4.0, 2.0);
}

/// Toolbar and menu bar buttons are flat until hovered, as in Docs: a rounded 4 px grey on hover
/// ([`HOVER`](crate::theme::HOVER)), darker while pressed or while its menu is open. (StarSheets'
/// toolbar uses the same colours.) A button that is on (Bold in bold text) is light blue ([`ON`]).
fn flat(ui: &mut Ui) {
    use crate::theme::{HOVER, PRESSED};
    let w = &mut ui.visuals_mut().widgets;
    for (state, fill) in [(&mut w.inactive, Color32::TRANSPARENT), (&mut w.hovered, HOVER), (&mut w.active, PRESSED), (&mut w.open, PRESSED)] {
        state.bg_stroke = Stroke::NONE;
        state.weak_bg_fill = fill;
        state.corner_radius = egui::CornerRadius::same(4);
        state.expansion = 0.0;
    }
}

fn bar(ui: &mut Ui) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 20.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, DIVIDER);
    ui.add_space(4.0);
}

fn icon_button(ui: &mut Ui, name: &str, on: bool) -> egui::Response {
    let b = match icons::icon(name) {
        Some(img) => Button::image(img),
        None => Button::new(name),
    };
    // A set fill would also replace the hover and pressed fills: only a button that is on has one.
    let b = if on { b.fill(ON) } else { b };
    ui.add(b.corner_radius(4.0).min_size(vec2(26.0, 28.0)))
}

fn tool(app: &mut WordApp, ui: &mut Ui, icon: &str, tip: &str, id: &str, p: Value, on: bool) {
    let r = icon_button(ui, icon, on);
    named(ui, &r, tip);
    if r.on_hover_text(tip).clicked() {
        run(app, id, p);
    }
}

fn icon_menu(ui: &mut Ui, icon: &str, tip: &str, add: impl FnOnce(&mut Ui)) {
    let img = icons::icon(icon);
    let r = match img {
        Some(img) => ui.menu_image_button(img, add).response,
        None => ui.menu_button(icon, add).response,
    };
    named(ui, &r, tip);
    r.on_hover_text(tip);
}

/// Names a picture-only control for screen readers (and the window's tests) by its tooltip.
fn named(ui: &Ui, r: &egui::Response, name: &str) {
    let enabled = ui.is_enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
}

/// The drop-down arrow after Docs' zoom, style and font boxes: Material's `arrow_drop_down` at
/// 20 px (a 10 × 5 triangle) in #444746.
const ARROW: f32 = 20.0;

/// A toolbar box of a fixed `width` showing `text` (cut short with "…" when too long) and a
/// drop-down arrow that opens a menu, as Docs' zoom, style and font boxes do. The menu's rows
/// are flat and full width.
fn drop_menu(ui: &mut Ui, text: &str, width: f32, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        let p = ui.painter();
        if open || resp.is_pointer_button_down_on() {
            p.rect_filled(rect, 4.0, crate::theme::PRESSED);
        } else if resp.hovered() {
            p.rect_filled(rect, 4.0, crate::theme::HOVER);
        }
        let room = rect.width() - 6.0 - ARROW - 2.0;
        let galley = egui::WidgetText::from(RichText::new(text).size(14.0).color(TEXT_DIM)).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            room,
            egui::TextStyle::Button,
        );
        p.galley(egui::pos2(rect.left() + 6.0, rect.center().y - galley.size().y / 2.0), galley, TEXT_DIM);
        if let Some(arrow) = icons::icon_px("arrow_drop_down", ARROW) {
            let at = egui::Rect::from_center_size(egui::pos2(rect.right() - 2.0 - ARROW / 2.0, rect.center().y), vec2(ARROW, ARROW));
            arrow.paint_at(ui, at);
        }
    }
    egui::Popup::menu(&resp).style(docs_menu_style()).show(|ui| {
        menu_look(ui);
        ui.with_layout(Layout::top_down_justified(Align::LEFT), add);
    });
    resp
}

/// Docs' font size box: type a size (1 to 400) and press Enter, or pick one from the list that
/// opens under it. Shows the selection's size; blank when it mixes sizes. Escape leaves it as
/// it was. After Enter the keyboard goes back to the page.
fn size_box(app: &mut WordApp, ui: &mut Ui, size: f64) {
    let id = ui.scope_id().with("stardocs_size_box");
    let list = id.with("list");
    let shown = if size > 0.0 { format!("{size}") } else { String::new() };
    let focused = ui.memory(|m| m.has_focus(id));
    let mut text = if focused { ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.clone()) } else { shown.clone() };
    let edit = egui::TextEdit::singleline(&mut text)
        .id(id)
        .desired_width(30.0)
        .char_limit(5)
        .align(egui::Align2::CENTER_CENTER)
        .font(egui::FontId::proportional(14.0))
        .text_color(TEXT_DIM)
        .margin(vec2(4.0, 5.0))
        .background_color(Color32::TRANSPARENT);
    let r = ui
        .scope(|ui| {
            let w = &mut ui.visuals_mut().widgets;
            for s in [&mut w.inactive, &mut w.hovered] {
                s.bg_stroke = Stroke::new(1.0, TEXT_DIM);
            }
            ui.add(edit)
        })
        .inner;
    let enabled = ui.is_enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, enabled, "Font size"));
    if r.gained_focus() {
        // A click selects the number, so typing replaces it, and opens the list.
        if let Some(mut st) = egui::TextEdit::load_state(ui.ctx(), id) {
            st.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&text)));
            st.store(ui.ctx(), id);
        }
        egui::Popup::open_id(ui.ctx(), list);
    }
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    }
    if r.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        let (enter, escape) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        if enter || escape {
            egui::Popup::close_id(ui.ctx(), list);
        }
        let typed = text.trim().parse::<f64>().ok().filter(|v| (1.0..=400.0).contains(v));
        match typed {
            Some(v) if !escape && v != size => run(app, "format.size", json!({ "size": v })),
            _ if enter || escape => app.canvas.want_focus = true,
            _ => {}
        }
    }
    egui::Popup::from_response(&r)
        .id(list)
        .open_memory(None)
        .style(docs_menu_style())
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            menu_look(ui);
            ui.set_min_width(60.0);
            ui.with_layout(Layout::top_down_justified(Align::LEFT), |ui| {
                for s in SIZES {
                    if ui.button(format!("{s}")).clicked() {
                        run(app, "format.size", json!({ "size": s }));
                        ui.close();
                    }
                }
            });
        });
}

fn palette<'a>(ui: &mut Ui, colours: impl Iterator<Item = (&'a str, &'a str)>, mut pick: impl FnMut(&str)) {
    let all: Vec<_> = colours.collect();
    egui::Grid::new(ui.next_auto_id()).spacing(vec2(4.0, 4.0)).show(ui, |ui| {
        for (i, (value, hex)) in all.iter().enumerate() {
            let c = u32::from_str_radix(hex, 16).unwrap_or(0);
            let colour = Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8);
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), egui::Sense::click());
            ui.painter().circle_filled(r.center(), 9.0, colour);
            ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(1.0, DIVIDER));
            if resp.on_hover_text(*value).clicked() {
                pick(value);
                ui.close();
            }
            if i % 10 == 9 {
                ui.end_row();
            }
        }
    });
}
