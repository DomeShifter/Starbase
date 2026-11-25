//! The Google Slides-style window around DeckCraft's slides: a title row with the menu bar and
//! the Slideshow button, and a one-row toolbar. DeckCraft's own title bar, ribbon and status bar
//! are switched off (`HostChrome`, fork change "Starbase Slides 0001"); every item here runs a
//! DeckCraft command by id (`SlideApp::run`), opens one of its dialogs (`app.dialog`), or is one of
//! our own `slides.*` commands ([`Shell::own`]). DeckCraft's keyboard shortcuts keep working.

use std::cell::Cell;
use std::rc::Rc;

use deckcraft_ui_egui::{SlideApp, ViewMode};
use egui::{Align, Button, Color32, Layout, RichText, Stroke, Ui, vec2};
use serde_json::{Value, json};

use crate::icons;
use crate::theme::{Mode, Palette};

/// What the title row says about saving.
// Saving and Failed come from the server link, which only the browser build has.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Saved {
    #[default]
    Nothing,
    Saving,
    Done,
    Failed(String),
}

/// Where the presentation lives: the Starbase server (`server::Link`), seen from the window.
pub trait Drive {
    /// The file's name on the server ("Plan.pptx"); empty until it is known.
    fn name(&self) -> String;
    fn saved(&self) -> Saved;
    /// A message from the last little while, for the title row.
    fn notice(&self) -> Option<String>;
    /// The signed-in person may only look at this presentation.
    fn view_only(&self) -> bool;
    /// The Starbase home screen (Drive), at this presentation's folder.
    fn home(&self);
    /// File › New presentation: a new, named .pptx in this presentation's folder, in a new tab.
    fn new_presentation(&self);
    fn rename(&self, stem: &str);
}

/// A menu entry: a command (label, id, params as JSON text, shortcut shown), a sub-menu, a line,
/// an on/off setting with a tick (label, id, params, what it reads: [`is_on`]), or the theme
/// list (filled from DeckCraft's `design.themes`).
pub enum Item {
    Cmd(&'static str, &'static str, &'static str, &'static str),
    Sub(&'static str, &'static [Item]),
    Sep,
    Check(&'static str, &'static str, &'static str, &'static str),
    Themes,
}
use Item::{Check, Cmd, Sep, Sub, Themes};

const NO: &str = "{}";

/// The menu bar. Ids starting `slides.` are ours ([`Shell::own`]); the rest are DeckCraft's.
pub const MENUS: &[(&str, &[Item])] = &[
    (
        "File",
        &[
            Cmd("New presentation", "slides.new", NO, ""),
            Cmd("Open from Starbase", "slides.home", NO, ""),
            Sep,
            Cmd("Rename", "slides.rename", NO, ""),
            Sub(
                "Download",
                &[
                    Cmd("Microsoft PowerPoint (.pptx)", "slides.download", r#"{"format":"pptx"}"#, ""),
                    Cmd("PDF document (.pdf)", "slides.download", r#"{"format":"pdf"}"#, ""),
                    Cmd("Plain text outline (.txt)", "slides.download", r#"{"format":"outline"}"#, ""),
                ],
            ),
            Cmd("Export…", "app.exportDialog", NO, ""),
            Sep,
            Cmd("Page setup", "app.dialog", r#"{"id":"slideSize"}"#, ""),
            Cmd("Slide numbers, date and footer", "app.dialog", r#"{"id":"headerFooter"}"#, ""),
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
            Cmd("Select all", "edit.selectAll", NO, "Ctrl+A"),
            Cmd("Delete", "edit.delete", NO, ""),
            Cmd("Duplicate", "edit.duplicate", NO, "Ctrl+D"),
            Sep,
            Cmd("Find and replace", "app.replace", NO, "Ctrl+H"),
        ],
    ),
    (
        "View",
        &[
            Cmd("Slideshow", "show.fromStart", NO, "Ctrl+Shift+Enter"),
            Cmd("Presenter view", "slides.presenter", NO, ""),
            Sep,
            Check("Normal", "view.normal", NO, "normal"),
            Check("Grid view", "view.sorter", NO, "sorter"),
            Sep,
            Check("Speaker notes", "view.notes", NO, "notes"),
            Check("Comments", "view.pane", r#"{"pane":"comments","toggle":true}"#, "comments"),
            Check("Ruler", "view.ruler", NO, "ruler"),
            Check("Guides", "view.guides", NO, "guides"),
            Sep,
            Cmd("Zoom in", "view.zoomIn", NO, "Ctrl++"),
            Cmd("Zoom out", "view.zoomOut", NO, "Ctrl+-"),
            Cmd("Fit", "view.fit", NO, ""),
            Sep,
            Sub(
                "Theme",
                &[
                    Check("Match the system", "slides.theme", r#"{"mode":"system"}"#, "theme:system"),
                    Check("Light", "slides.theme", r#"{"mode":"light"}"#, "theme:light"),
                    Check("Dark", "slides.theme", r#"{"mode":"dark"}"#, "theme:dark"),
                ],
            ),
        ],
    ),
    (
        "Insert",
        &[
            Cmd("Image", "app.insertPictureDialog", NO, ""),
            Cmd("Text box", "slides.textBox", NO, ""),
            Sub(
                "Shape",
                &[
                    Cmd("Rectangle", "slides.shape", r#"{"preset":"rect"}"#, ""),
                    Cmd("Rounded rectangle", "slides.shape", r#"{"preset":"roundRect"}"#, ""),
                    Cmd("Oval", "slides.shape", r#"{"preset":"ellipse"}"#, ""),
                    Cmd("Triangle", "slides.shape", r#"{"preset":"triangle"}"#, ""),
                    Cmd("Arrow", "slides.shape", r#"{"preset":"rightArrow"}"#, ""),
                    Cmd("Star", "slides.shape", r#"{"preset":"star5"}"#, ""),
                    Cmd("Line", "slides.shape", r#"{"preset":"line"}"#, ""),
                ],
            ),
            Cmd("Table", "app.dialog", r#"{"id":"table"}"#, ""),
            Cmd("Chart", "app.dialog", r#"{"id":"chartData"}"#, ""),
            Cmd("Diagram", "app.dialog", r#"{"id":"smartart"}"#, ""),
            Sep,
            Cmd("Audio", "app.insertAudioDialog", NO, ""),
            Cmd("Video", "app.insertVideoDialog", NO, ""),
            Sep,
            Cmd("Special characters", "app.dialog", r#"{"id":"symbol"}"#, ""),
            Cmd("Link", "app.dialog", r#"{"id":"hyperlink"}"#, "Ctrl+K"),
            Cmd("Comment", "app.dialog", r#"{"id":"comment"}"#, ""),
            Sep,
            Cmd("New slide", "slide.new", NO, "Ctrl+M"),
            Cmd("Slide numbers", "app.dialog", r#"{"id":"headerFooter"}"#, ""),
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
                    Cmd("Strikethrough", "format.strikethrough", NO, ""),
                    Cmd("Superscript", "format.superscript", NO, ""),
                    Cmd("Subscript", "format.subscript", NO, ""),
                    Sep,
                    Cmd("Increase font size", "format.grow", NO, "Ctrl+Shift+."),
                    Cmd("Decrease font size", "format.shrink", NO, "Ctrl+Shift+,"),
                ],
            ),
            Sub(
                "Align and indent",
                &[
                    Cmd("Left", "format.alignLeft", NO, "Ctrl+Shift+L"),
                    Cmd("Centre", "format.alignCenter", NO, "Ctrl+Shift+E"),
                    Cmd("Right", "format.alignRight", NO, "Ctrl+Shift+R"),
                    Cmd("Justified", "format.justify", NO, "Ctrl+Shift+J"),
                    Sep,
                    Cmd("Increase indent", "format.indent", NO, ""),
                    Cmd("Decrease indent", "format.outdent", NO, ""),
                ],
            ),
            Cmd("Line and paragraph spacing", "app.dialog", r#"{"id":"paragraph"}"#, ""),
            Sub(
                "Bullets and numbering",
                &[
                    Cmd("Bulleted list", "format.bullets", NO, ""),
                    Cmd("Numbered list", "format.numbering", NO, ""),
                    Cmd("List options", "app.dialog", r#"{"id":"bullets"}"#, ""),
                ],
            ),
            Sep,
            Cmd("Font…", "app.dialog", r#"{"id":"font"}"#, ""),
            Cmd("Size and position", "app.dialog", r#"{"id":"sizePosition"}"#, ""),
            Cmd("Alt text", "app.dialog", r#"{"id":"altText"}"#, ""),
            Sep,
            Cmd("Clear formatting", "format.clear", NO, "Ctrl+Space"),
        ],
    ),
    (
        "Slide",
        &[
            Cmd("New slide", "slide.new", NO, "Ctrl+M"),
            Cmd("Duplicate slide", "slide.duplicate", NO, ""),
            Cmd("Delete slide", "slide.delete", NO, ""),
            Check("Skip slide", "slide.hide", NO, "hidden"),
            Sep,
            Sub(
                "Apply layout",
                &[
                    Cmd("Title slide", "slide.layout", r#"{"layout":"title"}"#, ""),
                    Cmd("Title and body", "slide.layout", r#"{"layout":"titleAndContent"}"#, ""),
                    Cmd("Section header", "slide.layout", r#"{"layout":"sectionHeader"}"#, ""),
                    Cmd("Title and two columns", "slide.layout", r#"{"layout":"twoContent"}"#, ""),
                    Cmd("Comparison", "slide.layout", r#"{"layout":"comparison"}"#, ""),
                    Cmd("Title only", "slide.layout", r#"{"layout":"titleOnly"}"#, ""),
                    Cmd("Caption", "slide.layout", r#"{"layout":"contentWithCaption"}"#, ""),
                    Cmd("Blank", "slide.layout", r#"{"layout":"blank"}"#, ""),
                ],
            ),
            Sub(
                "Transition",
                &[
                    Cmd("None", "transition.set", r#"{"kind":"none"}"#, ""),
                    Cmd("Fade", "transition.set", r#"{"kind":"fade"}"#, ""),
                    Cmd("Push", "transition.set", r#"{"kind":"push"}"#, ""),
                    Cmd("Wipe", "transition.set", r#"{"kind":"wipe"}"#, ""),
                    Cmd("Cover", "transition.set", r#"{"kind":"cover"}"#, ""),
                    Cmd("Morph", "transition.set", r#"{"kind":"morph"}"#, ""),
                    Sep,
                    Cmd("Apply to all slides", "transition.applyAll", NO, ""),
                ],
            ),
            Sub("Change theme", &[Themes]),
            Cmd("Edit theme", "view.slideMaster", NO, ""),
            Sep,
            Cmd("Next slide", "slide.next", NO, "Page Down"),
            Cmd("Previous slide", "slide.previous", NO, "Page Up"),
        ],
    ),
    (
        "Arrange",
        &[
            Sub(
                "Order",
                &[
                    Cmd("Bring to front", "arrange.bringToFront", NO, "Ctrl+Shift+↑"),
                    Cmd("Bring forward", "arrange.bringForward", NO, "Ctrl+↑"),
                    Cmd("Send backward", "arrange.sendBackward", NO, "Ctrl+↓"),
                    Cmd("Send to back", "arrange.sendToBack", NO, "Ctrl+Shift+↓"),
                ],
            ),
            Sub(
                "Align",
                &[
                    Cmd("Left", "arrange.align", r#"{"edge":"left"}"#, ""),
                    Cmd("Centre", "arrange.align", r#"{"edge":"center"}"#, ""),
                    Cmd("Right", "arrange.align", r#"{"edge":"right"}"#, ""),
                    Sep,
                    Cmd("Top", "arrange.align", r#"{"edge":"top"}"#, ""),
                    Cmd("Middle", "arrange.align", r#"{"edge":"middle"}"#, ""),
                    Cmd("Bottom", "arrange.align", r#"{"edge":"bottom"}"#, ""),
                ],
            ),
            Sub(
                "Distribute",
                &[
                    Cmd("Horizontally", "arrange.distribute", r#"{"dir":"horizontal"}"#, ""),
                    Cmd("Vertically", "arrange.distribute", r#"{"dir":"vertical"}"#, ""),
                ],
            ),
            Sub(
                "Rotate",
                &[
                    Cmd("Rotate clockwise 90°", "arrange.rotateRight", NO, ""),
                    Cmd("Rotate anticlockwise 90°", "arrange.rotateLeft", NO, ""),
                    Cmd("Flip horizontally", "arrange.flipHorizontal", NO, ""),
                    Cmd("Flip vertically", "arrange.flipVertical", NO, ""),
                ],
            ),
            Sep,
            Cmd("Group", "arrange.group", NO, "Ctrl+G"),
            Cmd("Ungroup", "arrange.ungroup", NO, "Ctrl+Shift+G"),
        ],
    ),
    ("Help", &[Cmd("Search the menus", "app.palette", NO, "Alt+/"), Cmd("About the slide engine (DeckCraft)", "app.about", NO, "")]),
];

/// Our window's own state.
#[derive(Default)]
pub struct Shell {
    pub mode: Mode,
    /// The rename box's text while renaming.
    renaming: Option<String>,
    /// DeckCraft's built-in themes, read once (`design.themes`).
    themes: Option<Vec<String>>,
    /// View › Presenter view asked for this show's presenter window: the browser build's
    /// `Services::displays` reads it (without it, DeckCraft opens that window only on a second
    /// screen, as Google Slides' Slideshow shows just the slides).
    presenter: Rc<Cell<bool>>,
    /// A show has been seen since [`Self::presenter`] was set (it is cleared when that show ends).
    presenting: bool,
}

/// Where the Slideshow button was drawn last (egui points, which are CSS pixels), in egui's
/// temporary data: the browser build asks for full screen when a click lands there (`fullscreen`).
pub fn slideshow_rect_id() -> egui::Id {
    egui::Id::unique("starslides_slideshow_rect")
}

/// The parts of DeckCraft's window we draw ourselves.
pub fn chrome() -> deckcraft_ui_egui::HostChrome {
    deckcraft_ui_egui::HostChrome { title_bar: false, ribbon: false, status_bar: false }
}

impl Shell {
    pub fn new(mode: Mode) -> Self {
        Shell { mode, ..Default::default() }
    }

    /// Set while View › Presenter view starts a show (see [`Shell::presenter`]).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn presenter_flag(&self) -> Rc<Cell<bool>> {
        self.presenter.clone()
    }

    /// The palette for this frame.
    pub fn palette(&self, ctx: &egui::Context) -> Palette {
        Palette::of(self.mode.is_dark(ctx))
    }

    /// Draws the title row, menus and toolbar (nothing during a slideshow, which is full screen).
    /// Call before `SlideApp::ui`, which takes the room left.
    pub fn ui(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>) {
        let ctx = ui.ctx().clone();
        let pal = self.palette(&ctx);
        crate::theme::sync(app, pal.dark);
        if app.show.is_some() {
            self.presenting = self.presenter.get();
        } else if self.presenting {
            self.presenting = false;
            self.presenter.set(false);
        }
        if app.show.is_some() {
            return;
        }
        egui::Panel::top("slides_head")
            .frame(egui::Frame::NONE.fill(pal.head).inner_margin(egui::Margin { left: 12, right: 16, top: 8, bottom: 6 }))
            .show(ui, |ui| {
                self.title_and_menus(app, ui, drive, &pal);
                ui.add_space(6.0);
                self.toolbar(app, ui, drive, &pal);
            });
    }

    fn title_and_menus(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            let logo = ui.add(icons::logo(36.0).sense(egui::Sense::click()));
            ui.add_space(6.0);
            match drive {
                Some(d) => {
                    if logo.on_hover_text("Starbase home").clicked() {
                        d.home();
                    }
                }
                None => {
                    let _ = logo.on_hover_text("Starbase Slides");
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.horizontal(|ui| self.title(app, ui, drive, pal));
                ui.horizontal(|ui| {
                    flat(ui, pal);
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.spacing_mut().button_padding = vec2(7.0, 2.0);
                    for (title, items) in MENUS {
                        let config = egui::containers::menu::MenuConfig::new().style(menu_style(*pal));
                        let _ = egui::containers::menu::MenuButton::new(RichText::new(*title).size(14.0).color(pal.text))
                            .config(config)
                            .ui(ui, |ui| self.menu_items(app, ui, drive, items, pal));
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let present = Button::image_and_text(
                    icons::icon("slideshow", pal.present_text).unwrap_or_else(|| icons::logo(20.0)),
                    RichText::new("Slideshow").size(14.0).color(pal.present_text),
                )
                .fill(pal.present)
                .corner_radius(18.0)
                .min_size(vec2(118.0, 36.0));
                let r = ui.add_enabled(deckcraft_ui_egui::ribbon::enabled(app, "show.fromStart"), present);
                named(ui, &r, "Slideshow");
                // For the browser's full screen, which must be asked for in the click itself.
                ui.ctx().data_mut(|d| d.insert_temp(slideshow_rect_id(), r.rect));
                if r.on_hover_text("Present from the current slide (Ctrl+Enter); from the start: Ctrl+Shift+Enter").clicked() {
                    self.run(app, drive, "show.fromCurrent", json!({}));
                }
                ui.add_space(8.0);
                flat(ui, pal);
                let open = app.ui.pane.as_deref() == Some("comments");
                if icon_button(ui, "comment", open, pal).on_hover_text("Show all comments").clicked() {
                    self.run(app, drive, "view.pane", json!({"pane": "comments", "toggle": true}));
                }
            });
        });
    }

    /// The presentation's name (click to rename one on the server) and how saving is going.
    fn title(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        let stem = self.stem(app, drive);
        if let (Some(text), Some(d)) = (&mut self.renaming, drive) {
            let font = egui::FontId::proportional(18.0);
            let width = ui.painter().layout_no_wrap(text.clone(), font.clone(), pal.text).size().x + 18.0;
            let edit = ui.add(egui::TextEdit::singleline(text).font(font).margin(vec2(6.0, 0.0)).desired_width(width.clamp(40.0, 600.0)));
            if !edit.has_focus() && !edit.lost_focus() {
                edit.request_focus();
            }
            let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if edit.lost_focus() || cancel {
                let new = text.trim().to_string();
                if !cancel && !new.is_empty() && new != stem {
                    d.rename(&new);
                }
                self.renaming = None;
            }
        } else {
            let label = ui.add(egui::Label::new(RichText::new(&stem).size(18.0).color(pal.text)).sense(egui::Sense::click()));
            if drive.is_some() && label.on_hover_text("Rename").clicked() {
                self.renaming = Some(stem.clone());
            }
        }
        if let Some(d) = drive {
            let (text, colour) = match d.saved() {
                _ if d.view_only() => ("View only".to_string(), pal.text_dim),
                Saved::Nothing => (String::new(), pal.text_dim),
                Saved::Saving => ("Saving…".to_string(), pal.text_dim),
                Saved::Done => ("Saved to Starbase".to_string(), pal.text_dim),
                Saved::Failed(why) => (why, pal.error),
            };
            if !text.is_empty() {
                ui.add_space(10.0);
                ui.add(egui::Label::new(RichText::new(&text).size(13.0).color(colour)).truncate()).on_hover_text(&text);
            }
            if let Some(notice) = d.notice() {
                ui.add_space(10.0);
                ui.add(egui::Label::new(RichText::new(&notice).size(13.0).color(pal.notice)).truncate()).on_hover_text(&notice);
                ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
    }

    /// The name shown: the server's file name without .pptx, else DeckCraft's title.
    pub fn stem(&self, app: &SlideApp, drive: Option<&dyn Drive>) -> String {
        match drive.map(|d| d.name()).filter(|n| !n.is_empty()) {
            Some(n) => n.rsplit_once('.').map_or(n.as_str(), |(s, _)| s).to_string(),
            None if drive.is_some() => "Untitled presentation".to_string(),
            None => app.session.active().map(|d| d.title()).unwrap_or_else(|| "Untitled presentation".into()),
        }
    }

    fn toolbar(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        let may_edit = !drive.is_some_and(|d| d.view_only());
        egui::Frame::NONE.fill(pal.toolbar).corner_radius(24.0).inner_margin(egui::Margin::symmetric(10, 3)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                flat(ui, pal);
                let s = ui.spacing_mut();
                s.item_spacing.x = 2.0;
                s.button_padding = vec2(4.0, 2.0);
                ui.add_enabled_ui(may_edit, |ui| self.tools_history(app, ui, drive, pal));
                bar(ui, pal);
                self.tools_zoom(app, ui, drive, pal);
                bar(ui, pal);
                ui.add_enabled_ui(may_edit, |ui| {
                    self.tools_insert(app, ui, drive, pal);
                    bar(ui, pal);
                    self.tools_text(app, ui, drive, pal);
                    bar(ui, pal);
                    self.tools_slide(app, ui, drive, pal);
                });
            });
        });
    }

    /// New slide, Undo, Redo.
    fn tools_history(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        for (icon, tip, id) in
            [("add", "New slide (Ctrl+M)", "slide.new"), ("undo", "Undo (Ctrl+Z)", "edit.undo"), ("redo", "Redo (Ctrl+Y)", "edit.redo")]
        {
            if tool(ui, pal, icon, tip, false) {
                self.run(app, drive, id, json!({}));
            }
        }
    }

    /// Zoom out, Fit, Zoom in.
    fn tools_zoom(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        if tool(ui, pal, "zoom_out", "Zoom out", false) {
            self.run(app, drive, "view.zoomOut", json!({}));
        }
        let fit = Button::new(RichText::new("Fit").size(14.0).color(pal.text)).min_size(vec2(36.0, 28.0));
        if ui.add(fit).on_hover_text("Fit the slide to the window").clicked() {
            self.run(app, drive, "view.fit", json!({}));
        }
        if tool(ui, pal, "zoom_in", "Zoom in", false) {
            self.run(app, drive, "view.zoomIn", json!({}));
        }
    }

    /// Select, Text box, Image, Shape (DeckCraft's shape gallery), Table, Comment.
    fn tools_insert(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        use deckcraft_engine::ToolKind;
        let kind = app.session.tool.kind.clone();
        if tool(ui, pal, "arrow_selector_tool", "Select", matches!(kind, ToolKind::Select)) {
            app.session.set_tool(ToolKind::Select);
        }
        if tool(ui, pal, "text_fields", "Text box", matches!(kind, ToolKind::TextBox)) {
            self.run(app, drive, "slides.textBox", json!({}));
        }
        if tool(ui, pal, "image", "Insert image", false) {
            self.run(app, drive, "app.insertPictureDialog", json!({}));
        }
        let r = icon_button(ui, "shapes", matches!(kind, ToolKind::Shape { .. }), pal);
        named(ui, &r, "Shape");
        let r = r.on_hover_text("Shape");
        egui::Popup::menu(&r).show(|ui| {
            if let Some(p) = deckcraft_ui_egui::ribbon::shapes_gallery(ui) {
                app.session.set_tool(ToolKind::Shape { preset: p.to_string() });
            }
        });
        if tool(ui, pal, "table", "Insert table", false) {
            self.run(app, drive, "app.dialog", json!({"id": "table"}));
        }
        if tool(ui, pal, "add_comment", "Add comment", false) {
            self.run(app, drive, "app.dialog", json!({"id": "comment"}));
        }
    }

    /// Bold, Italic, Underline (lit while on), alignment, lists, indents, Clear formatting.
    fn tools_text(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        let f = app.session.execute("format.state", &json!({})).unwrap_or_default();
        let on = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(false);
        let tools = [
            ("format_bold", "Bold (Ctrl+B)", "format.bold", on("bold")),
            ("format_italic", "Italic (Ctrl+I)", "format.italic", on("italic")),
            ("format_underlined", "Underline (Ctrl+U)", "format.underline", on("underline")),
            ("format_align_left", "Left align", "format.alignLeft", false),
            ("format_align_center", "Centre align", "format.alignCenter", false),
            ("format_align_right", "Right align", "format.alignRight", false),
            ("format_list_bulleted", "Bulleted list", "format.bullets", false),
            ("format_list_numbered", "Numbered list", "format.numbering", false),
            ("format_indent_decrease", "Decrease indent", "format.outdent", false),
            ("format_indent_increase", "Increase indent", "format.indent", false),
            ("format_clear", "Clear formatting", "format.clear", false),
        ];
        for (i, (icon, tip, id, lit)) in tools.into_iter().enumerate() {
            if i == 3 {
                bar(ui, pal);
            }
            if tool(ui, pal, icon, tip, lit) {
                self.run(app, drive, id, json!({}));
            }
        }
    }

    /// Google Slides' text buttons for the slide as a whole: Layout, Theme, Transition.
    fn tools_slide(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, pal: &Palette) {
        for (label, sub) in [("Layout", "Apply layout"), ("Theme", "Change theme"), ("Transition", "Transition")] {
            let config = egui::containers::menu::MenuConfig::new().style(menu_style(*pal));
            let items = slide_sub(sub);
            let _ = egui::containers::menu::MenuButton::new(RichText::new(label).size(14.0).color(pal.text))
                .config(config)
                .ui(ui, |ui| self.menu_items(app, ui, drive, items, pal));
        }
    }
    fn menu_items(&mut self, app: &mut SlideApp, ui: &mut Ui, drive: Option<&dyn Drive>, items: &[Item], pal: &Palette) {
        menu_look(ui, pal);
        let width = menu_width(ui, items);
        for item in items {
            match item {
                Cmd(label, id, p, keys) => {
                    let enabled = self.allowed(app, drive, id);
                    if ui.add_enabled(enabled, row(None, label, keys, width, pal)).clicked() {
                        self.run(app, drive, id, params(p));
                        ui.close();
                    }
                }
                Check(label, id, p, key) => {
                    let on = self.is_on(app, key);
                    let tick = on.then_some("check");
                    let enabled = self.allowed(app, drive, id);
                    if ui.add_enabled(enabled, row(tick, label, "", width, pal)).clicked() {
                        self.run(app, drive, id, params(p));
                        ui.close();
                    }
                }
                Sub(label, inner) => {
                    let b = row(None, label, "", width, pal).right_text(match icons::icon_px("arrow_right", 20.0, pal.keys) {
                        Some(img) => egui::Atom::from(img),
                        None => egui::Atom::grow(),
                    });
                    let _ = egui::containers::menu::SubMenuButton::from_button(b).ui(ui, |ui| self.menu_items(app, ui, drive, inner, pal));
                }
                Sep => {
                    ui.add_space(8.0);
                    let (r, _) = ui.allocate_exact_size(vec2(width, 1.0), egui::Sense::hover());
                    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, pal.menu_line));
                    ui.add_space(8.0);
                }
                Themes => {
                    let themes = self.themes.get_or_insert_with(|| theme_names(app)).clone();
                    for name in themes {
                        if ui.add_enabled(drive.is_none_or(|d| !d.view_only()), row(None, &name, "", width, pal)).clicked() {
                            self.run(app, drive, "design.theme", json!({ "name": name }));
                            ui.close();
                        }
                    }
                }
            }
        }
    }

    /// Whether a menu entry can be used now: DeckCraft's own check for its commands; nothing that
    /// changes the presentation for someone who may only view it.
    fn allowed(&self, app: &SlideApp, drive: Option<&dyn Drive>, id: &str) -> bool {
        if drive.is_some_and(|d| d.view_only()) && !READ_ONLY_OK.contains(&id) && !id.starts_with("view.") && !id.starts_with("show.") {
            return false;
        }
        if let Some(own) = id.strip_prefix("slides.") {
            return match own {
                "rename" | "home" | "new" => drive.is_some() || own == "new",
                _ => true,
            };
        }
        deckcraft_ui_egui::ribbon::enabled(app, id)
    }

    /// Whether a ticked entry is on now.
    fn is_on(&self, app: &mut SlideApp, key: &str) -> bool {
        match key {
            "normal" => app.ui.view == ViewMode::Normal,
            "sorter" => app.ui.view == ViewMode::Sorter,
            "notes" => app.ui.notes,
            "comments" => app.ui.pane.as_deref() == Some("comments"),
            "ruler" => app.ui.ruler,
            "guides" => app.ui.guides,
            "hidden" => app.session.active().is_some_and(|d| d.doc.slides.get(d.selection.slide).is_some_and(|s| s.hidden)),
            k => k.strip_prefix("theme:").is_some_and(|m| Mode::parse(m) == self.mode),
        }
    }

    /// Runs a menu or toolbar entry; a refusal goes to DeckCraft's status line, as upstream.
    pub fn run(&mut self, app: &mut SlideApp, drive: Option<&dyn Drive>, id: &str, p: Value) {
        if let Err(e) = self.command(app, drive, id, p) {
            app.set_status(e);
        }
    }

    /// One command: ours (`slides.*`) or DeckCraft's. The answer is what the test hook reports.
    pub fn command(&mut self, app: &mut SlideApp, drive: Option<&dyn Drive>, id: &str, p: Value) -> Result<Value, String> {
        match id.strip_prefix("slides.") {
            Some(own) => self.own(app, drive, own, p),
            None => app.run(id, p),
        }
    }

    /// Our own commands.
    fn own(&mut self, app: &mut SlideApp, drive: Option<&dyn Drive>, id: &str, p: Value) -> Result<Value, String> {
        match id {
            "new" => match drive {
                Some(d) => d.new_presentation(),
                None => {
                    app.run("file.new", json!({}))?;
                }
            },
            "home" => drive.ok_or("not opened from Starbase")?.home(),
            "rename" => {
                drive.ok_or("only a presentation on the server can be renamed")?;
                match p.get("name").and_then(Value::as_str) {
                    Some(name) if !name.trim().is_empty() => drive.ok_or("no server")?.rename(name.trim()),
                    _ => self.renaming = Some(self.stem(app, drive)),
                }
            }
            "download" => {
                let format = p.get("format").and_then(Value::as_str).unwrap_or("pptx");
                let doc = app.session.active().map(|d| d.doc.clone()).ok_or("no presentation open")?;
                let bytes = deckcraft_engine::cmd::file::save_bytes(&doc, format).map_err(|e| e.to_string())?;
                let ext = match format {
                    "outline" => "txt",
                    f => f,
                };
                let name = format!("{}.{ext}", self.stem(app, drive));
                let size = bytes.len();
                let download = app.services.download.as_mut().ok_or("downloads are unavailable here")?;
                download(&name, &bytes);
                return Ok(json!({ "name": name, "bytes": size }));
            }
            "theme" => {
                self.mode = Mode::parse(p.get("mode").and_then(Value::as_str).unwrap_or("system"));
                crate::prefs::set(crate::theme::KEY, self.mode.name());
            }
            "presenter" => {
                // The slides here, the presenter window (notes, next slide, timer) beside them.
                self.presenter.set(true);
                app.ui.presenter_view = true;
                app.run("show.fromCurrent", json!({}))?;
            }
            "textBox" => app.session.set_tool(deckcraft_engine::ToolKind::TextBox),
            "shape" => {
                let preset = p.get("preset").and_then(Value::as_str).ok_or("missing `preset`")?;
                app.session.set_tool(deckcraft_engine::ToolKind::Shape { preset: preset.to_string() });
            }
            _ => return Err(format!("unknown command `slides.{id}`")),
        }
        Ok(Value::Null)
    }

    /// The window's own state, for the test hook (browser only).
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    pub fn state(&self, app: &SlideApp, drive: Option<&dyn Drive>) -> Value {
        json!({
            "title": self.stem(app, drive),
            "renaming": self.renaming.is_some(),
            "theme": self.mode.name(),
            "saved": drive.map(|d| match d.saved() {
                Saved::Nothing => "".to_string(),
                Saved::Saving => "saving".to_string(),
                Saved::Done => "saved".to_string(),
                Saved::Failed(why) => format!("failed: {why}"),
            }),
            "serverName": drive.map(|d| d.name()),
            "viewOnly": drive.is_some_and(|d| d.view_only()),
        })
    }
}

/// Commands that don't change the presentation: a viewer may still use them.
const READ_ONLY_OK: &[&str] = &[
    "slides.download",
    "slides.theme",
    "slides.home",
    "slides.new",
    "app.exportDialog",
    "app.palette",
    "app.about",
    "edit.copy",
    "edit.selectAll",
    "slide.next",
    "slide.previous",
];

/// One of the Slide menu's sub-menus, which the toolbar's text buttons also show.
fn slide_sub(label: &str) -> &'static [Item] {
    for (title, items) in MENUS {
        if *title == "Slide" {
            for i in *items {
                if let Sub(l, inner) = i
                    && *l == label
                {
                    return inner;
                }
            }
        }
    }
    &[]
}

/// DeckCraft's built-in themes, by name.
fn theme_names(app: &mut SlideApp) -> Vec<String> {
    let v = app.session.execute("design.themes", &json!({})).unwrap_or_default();
    v["themes"].as_array().map(|a| a.iter().filter_map(|t| t["name"].as_str().map(str::to_string)).collect()).unwrap_or_default()
}

fn params(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| json!({}))
}

/// Google's menu rows: 32 px tall, 16 px from the sides.
const ROW: f32 = 32.0;
const ROW_INSET: f32 = 16.0;
const ICON: f32 = 20.0;
const ICON_GAP: f32 = 12.0;

fn menu_look(ui: &mut Ui, pal: &Palette) {
    let st = ui.style_mut();
    st.wrap_mode = Some(egui::TextWrapMode::Extend);
    st.spacing.button_padding = vec2(ROW_INSET, 6.0);
    st.spacing.item_spacing = vec2(ICON_GAP, 0.0);
    st.spacing.icon_spacing = ICON_GAP;
    st.spacing.interact_size.y = ROW;
    let w = &mut st.visuals.widgets;
    for (state, fill) in
        [(&mut w.inactive, Color32::TRANSPARENT), (&mut w.hovered, pal.menu_hover), (&mut w.active, pal.hover), (&mut w.open, pal.menu_hover)]
    {
        state.bg_stroke = Stroke::NONE;
        state.weak_bg_fill = fill;
        state.corner_radius = egui::CornerRadius::ZERO;
        state.expansion = 0.0;
        state.fg_stroke.color = pal.text;
    }
}

/// Our menus' frame: 8 px top and bottom, 8 px corners, our menu colour.
fn menu_style(pal: Palette) -> egui::style::StyleModifier {
    egui::style::StyleModifier::new(move |s: &mut egui::Style| {
        egui::containers::menu::menu_style(s);
        s.spacing.menu_margin = egui::Margin::symmetric(0, 8);
        s.visuals.menu_corner_radius = egui::CornerRadius::same(8);
        s.visuals.window_fill = pal.menu;
        s.visuals.window_stroke = Stroke::new(1.0, pal.menu_line);
    })
}

/// How wide a menu is: its longest label plus shortcut, the tick column and the padding.
fn menu_width(ui: &Ui, items: &[Item]) -> f32 {
    let measure = |t: &str, size: f32| ui.painter().layout_no_wrap(t.to_string(), egui::FontId::proportional(size), Color32::WHITE).size().x;
    let widest = items
        .iter()
        .map(|i| match i {
            Cmd(label, _, _, keys) => measure(label, 14.0) + if keys.is_empty() { 0.0 } else { 32.0 + measure(keys, 13.0) },
            Check(label, ..) => measure(label, 14.0),
            Sub(label, _) => measure(label, 14.0) + ICON + ICON_GAP + 8.0,
            Sep | Themes => 120.0,
        })
        .fold(0.0_f32, f32::max);
    (widest + 2.0 * ROW_INSET + ICON + ICON_GAP + 8.0).max(240.0).ceil()
}

/// A menu row: the tick column (a tick or the same space blank), the label, the shortcut in grey.
fn row(tick: Option<&str>, label: &str, keys: &str, width: f32, pal: &Palette) -> Button<'static> {
    let text = RichText::new(label.to_string()).size(14.0).color(pal.text);
    let img = icons::icon_px(tick.unwrap_or("check"), ICON, if tick.is_some() { pal.text } else { Color32::TRANSPARENT });
    let b = match img {
        Some(img) => Button::image_and_text(img, text),
        None => Button::new(text),
    };
    let b =
        if keys.is_empty() { b.right_text(egui::Atom::grow()) } else { b.shortcut_text(RichText::new(keys.to_string()).size(13.0).color(pal.keys)) };
    b.min_size(vec2(width, ROW))
}

/// Toolbar and menu bar buttons are flat until hovered, as in Google's apps.
fn flat(ui: &mut Ui, pal: &Palette) {
    let w = &mut ui.visuals_mut().widgets;
    for (state, fill) in
        [(&mut w.inactive, Color32::TRANSPARENT), (&mut w.hovered, pal.hover), (&mut w.active, pal.pressed), (&mut w.open, pal.pressed)]
    {
        state.bg_stroke = Stroke::NONE;
        state.weak_bg_fill = fill;
        state.corner_radius = egui::CornerRadius::same(4);
        state.expansion = 0.0;
        state.fg_stroke.color = pal.text;
    }
}

fn bar(ui: &mut Ui, pal: &Palette) {
    ui.add_space(4.0);
    let (r, _) = ui.allocate_exact_size(vec2(1.0, 20.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, pal.divider);
    ui.add_space(4.0);
}

fn icon_button(ui: &mut Ui, name: &str, on: bool, pal: &Palette) -> egui::Response {
    let tint = if ui.is_enabled() { pal.icon } else { pal.icon.gamma_multiply(0.4) };
    let b = match icons::icon(name, tint) {
        Some(img) => Button::image(img),
        None => Button::new(name),
    };
    let b = if on { b.fill(pal.on) } else { b };
    ui.add(b.corner_radius(4.0).min_size(vec2(28.0, 28.0)))
}

/// A toolbar icon button named and tipped `tip`; whether it was clicked.
fn tool(ui: &mut Ui, pal: &Palette, icon: &str, tip: &str, on: bool) -> bool {
    let r = icon_button(ui, icon, on, pal);
    named(ui, &r, tip);
    r.on_hover_text(tip).clicked()
}

/// Names a picture-only control for screen readers (and the window's tests) by its tooltip.
fn named(ui: &Ui, r: &egui::Response, name: &str) {
    let enabled = ui.is_enabled();
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
}

/// Every command id the menus use (the tests check that each one exists).
#[cfg(test)]
pub fn menu_ids() -> Vec<&'static str> {
    fn walk(items: &[Item], out: &mut Vec<&'static str>) {
        for i in items {
            match i {
                Cmd(_, id, ..) | Check(_, id, ..) => out.push(id),
                Sub(_, inner) => walk(inner, out),
                Sep | Themes => {}
            }
        }
    }
    let mut out = Vec::new();
    for (_, items) in MENUS {
        walk(items, &mut out);
    }
    out
}
