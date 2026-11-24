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
/// [`is_on`]), or Docs' grid for picking a table's size.
pub enum Item {
    Cmd(&'static str, &'static str, &'static str, &'static str),
    Sub(&'static str, &'static [Item]),
    Sep,
    Toggle(&'static str, &'static str, &'static str, &'static str),
    TableGrid,
}
use Item::{Cmd, Sep, Sub, TableGrid, Toggle};

const NO: &str = "{}";

/// The menus, in Google Docs' order and wording. An id starting `ui.dialog:` opens that
/// WordCraft dialog.
pub const MENUS: &[(&str, &[Item])] = &[
    (
        "File",
        &[
            Cmd("New document", "file.new", NO, ""),
            Cmd("Open", "file.open", NO, "Ctrl+O"),
            Sep,
            Sub("Download", &[Cmd("Microsoft Word (.docx)", "stardocs.download", NO, ""), Cmd("PDF document (.pdf)", "file.exportPdf", NO, "")]),
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
                ],
            ),
            Sep,
            Cmd("Print layout", "view.printLayout", NO, ""),
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
            Cmd("Page break", "insert.pageBreak", NO, "Ctrl+Enter"),
            Sub("Headers and footers", &[Cmd("Header", "insert.editHeader", NO, ""), Cmd("Footer", "insert.editFooter", NO, "")]),
            Cmd("Page numbers", "insert.pageNumber", NO, ""),
            Cmd("Footnote", "references.footnote", NO, "Ctrl+Alt+F"),
            Sep,
            Cmd("Special characters", "insert.symbol", NO, ""),
            Cmd("Equation", "insert.equation", NO, ""),
            Cmd("Bookmark", "ui.dialog:bookmark", NO, ""),
            Cmd("Table of contents", "references.toc", NO, ""),
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
                ],
            ),
            Sub(
                "Paragraph styles",
                &[
                    Cmd("Normal text", "para.style", r#"{"style":"Normal"}"#, "Ctrl+Alt+0"),
                    Cmd("Title", "para.style", r#"{"style":"Title"}"#, ""),
                    Cmd("Subtitle", "para.style", r#"{"style":"Subtitle"}"#, ""),
                    Cmd("Heading 1", "para.style", r#"{"style":"Heading 1"}"#, "Ctrl+Alt+1"),
                    Cmd("Heading 2", "para.style", r#"{"style":"Heading 2"}"#, "Ctrl+Alt+2"),
                    Cmd("Heading 3", "para.style", r#"{"style":"Heading 3"}"#, "Ctrl+Alt+3"),
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
                    Cmd("Custom spacing", "ui.dialog:paragraph", NO, ""),
                ],
            ),
            Cmd("Columns", "layout.columns", NO, ""),
            Sub(
                "Bullets and numbering",
                &[Cmd("Bulleted list", "para.bullets", NO, "Ctrl+Shift+8"), Cmd("Numbered list", "para.numbering", NO, "Ctrl+Shift+7")],
            ),
            Sep,
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
            Cmd("Accept all suggestions", "review.acceptAll", NO, ""),
            Cmd("Reject all suggestions", "review.rejectAll", NO, ""),
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
            Cmd("About StarDocs", "ui.dialog:about", NO, ""),
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
    /// The colour being mixed in the text colour menu's Custom picker.
    custom: Option<Color32>,
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
    /// Read from storage yet.
    loaded: bool,
}

const FLAGS_KEY: &str = "stardocs.flags";

impl Flags {
    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    /// Fills in the stored settings the first time; afterwards writes them back when they change.
    fn sync(&mut self, before: Flags) {
        if !self.loaded {
            self.loaded = true;
            if let Some(v) = Self::storage().and_then(|s| s.get_item(FLAGS_KEY).ok().flatten()) {
                self.words = v.contains("words");
                self.compact = v.contains("compact");
            }
        } else if (self.words, self.compact) != (before.words, before.compact)
            && let Some(s) = Self::storage()
        {
            let v = [("words", self.words), ("compact", self.compact)].iter().filter(|(_, on)| *on).map(|(k, _)| *k).collect::<Vec<_>>().join(",");
            let _ = s.set_item(FLAGS_KEY, &v);
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
        "trackChanges" => app.session.doc.settings.track_changes,
        _ => app.run("view.state", json!({})).ok().and_then(|v| v.get(key).and_then(Value::as_bool)).unwrap_or(false),
    }
}

/// Editing or suggesting: Docs' mode switch, which is WordCraft's track changes.
fn set_suggesting(app: &mut WordApp, on: bool) {
    if app.session.doc.settings.track_changes != on {
        run(app, "review.trackChanges", json!({ "value": on }));
    }
}

/// Menu commands: a few mean something else when the Starbase server keeps the document.
fn run_menu(app: &mut WordApp, ui: &Ui, flags: &mut Flags, link: Option<&Link>, id: &str, p: Value) {
    match (id, link) {
        ("stardocs.words", _) => flags.words = !flags.words,
        ("stardocs.compact", _) => flags.compact = !flags.compact,
        ("stardocs.shortcuts", _) => flags.shortcuts = true,
        ("stardocs.editing", _) => set_suggesting(app, false),
        ("stardocs.suggesting", _) => set_suggesting(app, true),
        ("stardocs.download", Some(link)) => {
            link.download_next();
            run(app, "file.save", json!({}));
        }
        ("stardocs.download", None) => run(app, "file.save", json!({})),
        ("stardocs.rename", Some(_)) => {
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique(RENAME), true));
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
    /// The caret paragraph's style name, as the style box shows it.
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
        style: style_label(f.get("styleName").or_else(|| f.get("style")).and_then(Value::as_str).unwrap_or("Normal")),
        bold: on("bold"),
        italic: on("italic"),
        underline: f.get("underline").is_some_and(|u| u.as_bool().unwrap_or(!u.is_null() && u != "none")),
        zoom: v.get("zoom").and_then(Value::as_f64).unwrap_or(1.0),
    }
}

impl Shell {
    /// Draws the title row, menus and toolbar above the page. Call before `WordApp::ui`.
    pub fn ui(&mut self, app: &mut WordApp, ui: &mut Ui, link: Option<&Link>) {
        let before = self.flags;
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
                // A view-only document: the toolbar is shown greyed, like Google Docs' viewer.
                let editable = !link.is_some_and(Link::view_only);
                self.toolbar(app, ui, editable);
            });
        self.share.ui(ui.ctx());
        if std::mem::take(&mut self.flags.shortcuts) {
            self.shortcuts.open = true;
        }
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
        let (menus, outline, help) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&KeyboardShortcut::new(ctrl_shift, Key::F)),
                i.consume_shortcut(&KeyboardShortcut::new(ctrl_alt, Key::H)),
                i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Slash)),
            )
        });
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

    fn toolbar(&mut self, app: &mut WordApp, ui: &mut Ui, editable: bool) {
        let cur = current(app);
        let flags = &mut self.flags;
        let custom = &mut self.custom;
        egui::Frame::NONE.fill(TOOLBAR).corner_radius(24.0).inner_margin(egui::Margin::symmetric(10, 4)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                flat(ui);
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_enabled_ui(editable, |ui| {
                    tool(app, ui, "undo", "Undo (Ctrl+Z)", "edit.undo", json!({}), false);
                    tool(app, ui, "redo", "Redo (Ctrl+Y)", "edit.redo", json!({}), false);
                    tool(app, ui, "print", "Print (Ctrl+P)", "file.print", json!({}), false);
                    tool(app, ui, "spellcheck", "Spelling and grammar", "review.spelling", json!({}), false);
                    let painting = app.session.painter.is_some();
                    tool(
                        app,
                        ui,
                        "format_paint",
                        "Paint format: copy this text's look, then select where it goes",
                        "edit.formatPainter",
                        json!({}),
                        painting,
                    );
                    bar(ui);
                    let zoom = format!("{}%", (cur.zoom * 100.0).round());
                    drop_menu(ui, &zoom, 64.0, false, |ui| {
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
                    bar(ui);
                    drop_menu(ui, &cur.style, 116.0, false, |ui| {
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
                    bar(ui);
                    let fonts = self.fonts.get_or_insert_with(|| font_names(app)).clone();
                    let font = drop_menu(ui, &cur.font, 116.0, false, |ui| {
                        egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                            for f in &fonts {
                                if ui.button(f).clicked() {
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
                    bar(ui);
                    tool(app, ui, "remove", "Decrease font size", "format.shrinkFont1", json!({}), false);
                    let size = if cur.size > 0.0 { format!("{}", cur.size) } else { String::new() };
                    drop_menu(ui, &size, 40.0, true, |ui| {
                        for s in SIZES {
                            if ui.button(format!("{s}")).clicked() {
                                run(app, "format.size", json!({ "size": s }));
                                ui.close();
                            }
                        }
                    });
                    tool(app, ui, "add", "Increase font size", "format.growFont1", json!({}), false);
                    bar(ui);
                    tool(app, ui, "format_bold", "Bold (Ctrl+B)", "format.bold", json!({}), cur.bold);
                    tool(app, ui, "format_italic", "Italic (Ctrl+I)", "format.italic", json!({}), cur.italic);
                    tool(app, ui, "format_underlined", "Underline (Ctrl+U)", "format.underline", json!({}), cur.underline);
                    icon_menu(ui, "format_color_text", "Text colour", |ui| {
                        palette(ui, COLOURS.iter().map(|c| (*c, *c)), |c| run(app, "format.color", json!({ "color": c })));
                        ui.horizontal(|ui| {
                            if ui.button("Automatic").clicked() {
                                run(app, "format.color", json!({ "color": "auto" }));
                                ui.close();
                            }
                            if custom.is_none() && ui.button("Custom…").clicked() {
                                *custom = Some(Color32::from_rgb(0x0B, 0x57, 0xD0));
                            }
                        });
                        // Docs' custom colour: pick any colour, then OK.
                        if let Some(mut c) = *custom {
                            ui.separator();
                            egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque);
                            *custom = Some(c);
                            ui.horizontal(|ui| {
                                if ui.button("OK").clicked() {
                                    let hex = format!("{:02X}{:02X}{:02X}", c.r(), c.g(), c.b());
                                    run(app, "format.color", json!({ "color": hex }));
                                    *custom = None;
                                    ui.close();
                                }
                                if ui.button("Cancel").clicked() {
                                    *custom = None;
                                }
                            });
                        }
                    });
                    icon_menu(ui, "ink_highlighter", "Highlight colour", |ui| {
                        palette(ui, HIGHLIGHTS.iter().copied(), |c| run(app, "format.highlight", json!({ "color": c })));
                        if ui.button("None").clicked() {
                            run(app, "format.highlight", json!({ "color": "none" }));
                            ui.close();
                        }
                    });
                    bar(ui);
                    tool(app, ui, "link", "Insert link (Ctrl+K)", "ui.dialog:link", json!({}), false);
                    tool(app, ui, "add_comment", "Add comment (Ctrl+Alt+M)", "review.newComment", json!({}), false);
                    tool(app, ui, "image", "Insert image", "insert.picture", json!({}), false);
                    bar(ui);
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
                    bar(ui);
                    tool(app, ui, "format_list_bulleted", "Bulleted list (Ctrl+Shift+8)", "para.bullets", json!({}), false);
                    tool(app, ui, "format_list_numbered", "Numbered list (Ctrl+Shift+7)", "para.numbering", json!({}), false);
                    tool(app, ui, "format_indent_decrease", "Decrease indent (Ctrl+[)", "para.outdent", json!({}), false);
                    tool(app, ui, "format_indent_increase", "Increase indent (Ctrl+])", "para.indent", json!({}), false);
                    tool(app, ui, "format_clear", "Clear formatting (Ctrl+\\)", "format.clear", json!({}), false);
                });
                // The right end: the mode switch and the compact-mode arrow (Docs' "Hide the menus").
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (icon, tip) = if flags.compact {
                        ("keyboard_arrow_down", "Show the menus (Ctrl+Shift+F)")
                    } else {
                        ("keyboard_arrow_up", "Hide the menus (Ctrl+Shift+F)")
                    };
                    if icon_button(ui, icon, false).on_hover_text(tip).clicked() {
                        flags.compact = !flags.compact;
                    }
                    mode_switch(app, ui, editable);
                });
            });
        });
    }
}

/// Docs' mode box at the right of the toolbar: Editing or Suggesting (WordCraft's track
/// changes), or a fixed "Viewing" for a view-only document.
fn mode_switch(app: &mut WordApp, ui: &mut Ui, editable: bool) {
    let suggesting = app.session.doc.settings.track_changes;
    let (icon, label) = match (editable, suggesting) {
        (false, _) => ("visibility", "Viewing"),
        (true, true) => ("rate_review", "Suggesting"),
        (true, false) => ("edit", "Editing"),
    };
    let text = RichText::new(label).size(14.0).color(TEXT_DIM);
    let button = match icons::icon(icon) {
        Some(img) => Button::image_and_text(img, text),
        None => Button::new(text),
    };
    if !editable {
        ui.add_enabled(false, button).on_disabled_hover_text("You can view this document but not change it");
        return;
    }
    let r = ui.add(button.corner_radius(4.0).min_size(vec2(116.0, 28.0)));
    let tip = if suggesting { "Suggesting: your edits show as suggestions others can accept" } else { "Editing: you change the document directly" };
    let r = r.on_hover_text(tip);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(300.0);
        for (on, icon, title, about) in
            [(false, "edit", "Editing", "Edit the document directly"), (true, "rate_review", "Suggesting", "Edits become suggestions")]
        {
            let text = RichText::new(format!("{title}\n{about}")).size(14.0).color(TEXT);
            let item = match icons::icon(icon) {
                Some(img) => Button::image_and_text(img, text),
                None => Button::new(text),
            };
            if ui.add(item.selected(on == suggesting).min_size(vec2(300.0, 44.0))).clicked() {
                set_suggesting(app, on);
                ui.close();
            }
        }
    });
}

fn font_names(app: &mut WordApp) -> Vec<String> {
    let v = app.run("font.list", json!({})).unwrap_or_default();
    let list = v.get("fonts").or_else(|| v.get("families")).unwrap_or(&v);
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
                logo.on_hover_text("StarDocs");
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| self.title(app, ui, link));
                ui.horizontal(|ui| {
                    flat(ui);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.spacing_mut().button_padding = vec2(7.0, 2.0);
                    for (title, items) in MENUS {
                        ui.menu_button(RichText::new(*title).size(14.0).color(TEXT), |ui| menu_items(app, ui, &mut self.flags, items, link));
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
            let edit = ui.add(egui::TextEdit::singleline(text).font(egui::FontId::proportional(18.0)).desired_width(320.0));
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

fn menu_items(app: &mut WordApp, ui: &mut Ui, flags: &mut Flags, items: &[Item], link: Option<&Link>) {
    ui.set_min_width(260.0);
    for item in items {
        match item {
            Cmd(label, id, p, keys) => {
                let b = Button::new(RichText::new(*label).size(14.0).color(TEXT))
                    .right_text(RichText::new(*keys).size(13.0).color(TEXT_DIM))
                    .min_size(vec2(260.0, 30.0));
                if ui.add(b).clicked() {
                    run_menu(app, ui, flags, link, id, params(p));
                    ui.close();
                }
            }
            Toggle(label, id, key, keys) => {
                // A tick in the margin when it is on, as Docs' View menu shows.
                let on = is_on(app, flags, key);
                let tick = if on { icons::icon("check") } else { None };
                let text = RichText::new(*label).size(14.0).color(TEXT);
                let b = match tick {
                    Some(img) => Button::image_and_text(img, text),
                    None => Button::new(RichText::new(format!("      {label}")).size(14.0).color(TEXT)),
                };
                if ui.add(b.right_text(RichText::new(*keys).size(13.0).color(TEXT_DIM)).min_size(vec2(260.0, 30.0))).clicked() {
                    run_menu(app, ui, flags, link, id, json!({}));
                    ui.close();
                }
            }
            TableGrid => table_grid(app, ui),
            Sub(label, sub) => {
                ui.menu_button(RichText::new(*label).size(14.0).color(TEXT), |ui| menu_items(app, ui, flags, sub, link));
            }
            Sep => {
                ui.separator();
            }
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

/// Toolbar buttons are flat until hovered, as in Docs.
fn flat(ui: &mut Ui) {
    let w = &mut ui.visuals_mut().widgets;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::NONE;
    w.hovered.bg_stroke = Stroke::NONE;
    w.active.bg_stroke = Stroke::NONE;
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
    ui.add(b.fill(if on { ON } else { Color32::TRANSPARENT }).corner_radius(4.0).min_size(vec2(28.0, 28.0)))
}

fn tool(app: &mut WordApp, ui: &mut Ui, icon: &str, tip: &str, id: &str, p: Value, on: bool) {
    if icon_button(ui, icon, on).on_hover_text(tip).clicked() {
        run(app, id, p);
    }
}

fn icon_menu(ui: &mut Ui, icon: &str, tip: &str, add: impl FnOnce(&mut Ui)) {
    let img = icons::icon(icon);
    let r = match img {
        Some(img) => ui.menu_image_button(img, add).response,
        None => ui.menu_button(icon, add).response,
    };
    r.on_hover_text(tip);
}

/// A toolbar box of a fixed `width` showing `text` (cut short with "…" when too long) that
/// opens a menu, as Docs' zoom, style, font and size boxes do. `boxed` draws the size box's
/// border; the others have a ▾ instead.
fn drop_menu(ui: &mut Ui, text: &str, width: f32, boxed: bool, add: impl FnOnce(&mut Ui)) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
        let p = ui.painter();
        if resp.hovered() || open {
            p.rect_filled(rect, 4.0, ui.visuals().widgets.hovered.weak_bg_fill);
        }
        if boxed {
            p.rect_stroke(rect.shrink(1.0), 4.0, Stroke::new(1.0, TEXT_DIM), egui::StrokeKind::Inside);
        }
        let arrow = if boxed { 0.0 } else { 14.0 };
        let room = rect.width() - 12.0 - arrow;
        let galley = egui::WidgetText::from(RichText::new(text).size(14.0).color(TEXT_DIM)).into_galley(
            ui,
            Some(egui::TextWrapMode::Truncate),
            room,
            egui::TextStyle::Button,
        );
        let x = if boxed { rect.center().x - galley.size().x / 2.0 } else { rect.left() + 6.0 };
        p.galley(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, TEXT_DIM);
        if !boxed {
            p.text(egui::pos2(rect.right() - 6.0, rect.center().y), egui::Align2::RIGHT_CENTER, "▾", egui::FontId::proportional(13.0), TEXT_DIM);
        }
    }
    egui::Popup::menu(&resp).show(add);
    resp
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
