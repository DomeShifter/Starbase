//! A Kanban board uses Drive for file access and conditional saves.

use std::{cell::RefCell, rc::Rc};

use egui::{RichText, Ui, vec2};
use serde::{Deserialize, Serialize};

use super::Home;
use crate::{
    files::{Method, enc, fetch},
    house, theme,
};

pub(super) fn is_board(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("kanban"))
}

#[derive(Serialize, Deserialize)]
struct Board {
    columns: Vec<Column>,
    #[serde(default)]
    numbering: bool,
    #[serde(default)]
    last_number: u64,
    #[serde(default)]
    archive: Vec<Card>,
}

#[derive(Serialize, Deserialize)]
struct Column {
    name: String,
    limit: Option<usize>,
    cards: Vec<Card>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Card {
    title: String,
    #[serde(default)]
    number: Option<u64>,
    description: String,
    label: String,
    due: String,
    checklist: Vec<(bool, String)>,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            numbering: false,
            last_number: 0,
            archive: Vec::new(),
            columns: ["To do", "In progress", "Done"].into_iter().map(|name| Column { name: name.into(), limit: None, cards: Vec::new() }).collect(),
        }
    }
}

fn target(ui: &Ui, label: &str, response: egui::Response) -> bool {
    house::target(ui.ctx(), label, response.rect);
    response.clicked()
}

type CardPosition = (usize, usize);

#[derive(Clone, Copy)]
struct CardDrag(CardPosition);

impl Board {
    fn number_cards(&mut self) {
        let cards = self.columns.iter_mut().flat_map(|column| &mut column.cards).chain(&mut self.archive);
        for card in cards {
            if card.number.is_none()
                && let Some(number) = self.last_number.checked_add(1)
            {
                self.last_number = number;
                card.number = Some(number);
            }
        }
    }

    fn normalize_numbering(&mut self) {
        let cards = self.columns.iter().flat_map(|column| &column.cards).chain(&self.archive);
        self.last_number = self.last_number.max(cards.filter_map(|card| card.number).max().unwrap_or_default());
        if self.numbering {
            self.number_cards();
        }
    }

    fn move_card(&mut self, (from, index): CardPosition, (to, target): CardPosition) {
        if from == to && (index == target || index + 1 == target) {
            return;
        }
        let card = self.columns[from].cards.remove(index);
        let target = if from == to && index < target { target - 1 } else { target };
        self.columns[to].cards.insert(target, card);
    }
}

enum Response {
    Opened(Board, String),
    Created(String, String, Board, String),
    Saved(String),
    Failed(String),
}

#[derive(Default)]
pub(super) struct Kanban {
    name: String,
    id: String,
    version: String,
    writable: bool,
    pending: bool,
    dirty: bool,
    error: Option<String>,
    board: Option<Board>,
    inbox: Rc<RefCell<Option<Response>>>,
    editor: Option<(usize, Option<usize>, usize, Card)>,
    column: Option<(Option<usize>, String, usize)>,
    filter: String,
    create_in: Option<String>,
    confirm_close: bool,
    leave_after_save: bool,
    archive_open: bool,
    delete_from_archive: Option<usize>,
    renaming: Option<(CardPosition, String)>,
}

impl Kanban {
    fn request(
        &mut self,
        ctx: &egui::Context,
        method: Method,
        url: String,
        bytes: Option<Vec<u8>>,
        version: Option<String>,
        finish: impl FnOnce(web_sys::Headers, Vec<u8>) -> Result<Response, String> + 'static,
    ) {
        self.pending = true;
        self.error = None;
        let (inbox, ctx) = (self.inbox.clone(), ctx.clone());
        wasm_bindgen_futures::spawn_local(async move {
            let response = match fetch(method, &url, bytes, version).await {
                Ok((200 | 201, headers, bytes)) => finish(headers, bytes),
                Ok((409, _, _)) => {
                    Err("A newer version was saved elsewhere. Your edits are still here. Reopen the file from Drive to see the newer version.".into())
                }
                Ok((status, _, bytes)) => Err(super::said(&serde_json::from_slice(&bytes).unwrap_or_default(), status)),
                Err(error) => Err(error),
            };
            *inbox.borrow_mut() = Some(response.unwrap_or_else(Response::Failed));
            ctx.request_repaint();
        });
    }

    fn accept_reply(&mut self) {
        let response = self.inbox.borrow_mut().take();
        if let Some(response) = response {
            self.pending = false;
            match response {
                Response::Opened(board, version) => {
                    self.board = Some(board);
                    self.version = version;
                }
                Response::Created(id, name, board, version) => {
                    self.id = id;
                    self.name = name;
                    self.board = Some(board);
                    self.version = version;
                    self.create_in = None;
                }
                Response::Saved(version) => {
                    self.version = version;
                    self.dirty = false;
                }
                Response::Failed(error) => self.error = Some(error),
            }
        }
    }

    fn open(ctx: &egui::Context, id: &str, name: &str, writable: bool) -> Self {
        let mut app = Self { id: id.into(), name: name.into(), writable, ..Self::default() };
        app.request(ctx, Method::Get, format!("api/files/{}", enc(id)), None, None, |headers, bytes| {
            let version = headers.get("ETag").ok().flatten().ok_or("The board version is unavailable.")?;
            let mut board: Board = serde_json::from_slice(&bytes).map_err(|_| "This file is not a valid Kanban board.")?;
            board.normalize_numbering();
            Ok(Response::Opened(board, version.trim_matches('"').into()))
        });
        app
    }

    fn get_string(value: &serde_json::Value, key: &str) -> Result<String, String> {
        value[key].as_str().filter(|s| !s.is_empty()).map(str::to_string).ok_or_else(|| format!("Missing board {key}."))
    }

    fn create(&mut self, ctx: &egui::Context, folder: &str) {
        let board = Board::default();
        let name = format!("{}.kanban", self.name.trim());
        let url = format!("api/files?name={}&folder={}&as_is=1", enc(&name), enc(folder));
        self.request(ctx, Method::Post, url, serde_json::to_vec(&board).ok(), None, move |_, bytes| {
            let value = serde_json::from_slice(&bytes).map_err(|_| "Could not read the new board details.")?;
            Ok(Response::Created(Self::get_string(&value, "id")?, Self::get_string(&value, "name")?, board, Self::get_string(&value, "version")?))
        });
    }

    fn save(&mut self, ctx: &egui::Context) {
        let Some(board) = &self.board else { return };
        let Ok(bytes) = serde_json::to_vec(board) else { return };
        self.request(ctx, Method::Put, format!("api/files/{}", enc(&self.id)), Some(bytes), Some(self.version.clone()), |_, bytes| {
            let value = serde_json::from_slice(&bytes).map_err(|_| "Could not read the saved board version.")?;
            Ok(Response::Saved(Self::get_string(&value, "version")?))
        });
    }

    fn save_indicator(&self, ui: &mut Ui) -> bool {
        use starbase_savestate::{SaveInfo, SaveKind, SavePalette, disk_save_icon};
        let kind = if !self.writable {
            SaveKind::ViewOnly
        } else if self.pending {
            SaveKind::Saving
        } else if self.error.is_some() {
            SaveKind::Problem
        } else if self.dirty {
            SaveKind::Unsaved
        } else {
            SaveKind::Saved
        };
        let info =
            SaveInfo { kind, detail: if self.error.is_some() { "Click to retry the save".into() } else { String::new() }, ..Default::default() };
        let palette = SavePalette::house(theme::text_dim(), theme::page());
        let icon = disk_save_icon(ui, &info, &palette);
        house::target(ui.ctx(), &format!("kanban:save:{}", kind.word()), icon.response.rect);
        icon.response.clicked() && self.writable && self.dirty && !self.pending
    }

    fn draw_board(&mut self, ui: &mut Ui) {
        let Some(board) = self.board.as_mut() else { return };
        let writable = self.writable && !self.pending;
        let query = self.filter.trim().to_lowercase();
        let mut movement = None;
        let mut rename = None;
        let width = ((ui.available_width() - 72.0) / 3.0).clamp(286.0, 380.0);
        egui::ScrollArea::horizontal().id_salt("kanban_columns").show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for (index, column) in board.columns.iter().enumerate() {
                    ui.push_id(index, |ui| {
                        let panel = egui::Frame::NONE.fill(theme::drive_surface()).corner_radius(12).inner_margin(12).show(ui, |ui| {
                            ui.set_width(width);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&column.name).strong());
                                let count =
                                    column.limit.map_or_else(|| column.cards.len().to_string(), |limit| format!("{}/{limit}", column.cards.len()));
                                let over = column.limit.is_some_and(|limit| column.cards.len() > limit);
                                let count = RichText::new(count);
                                ui.label(if over { count.color(house::ERROR) } else { count });
                            });
                            if writable {
                                ui.horizontal(|ui| {
                                    if ui.small_button("Edit column").clicked() {
                                        self.column = Some((Some(index), column.name.clone(), column.limit.unwrap_or_default()));
                                    }
                                    let add = ui.small_button("Add card");
                                    if target(ui, &format!("kanban:add:{index}"), add) {
                                        self.editor = Some((index, None, index, Card::default()));
                                    }
                                });
                            }
                            let height = (ui.available_height() - 18.0).max(112.0);
                            egui::ScrollArea::vertical().min_scrolled_height(height).max_height(height).show(ui, |ui| {
                                ui.set_min_height(height);
                                ui.horizontal_wrapped(|ui| {
                                    let tile_width = (width - ui.spacing().item_spacing.x) / 2.0 - 22.0;
                                    for (card_index, card) in column.cards.iter().enumerate() {
                                        if !query.is_empty()
                                            && ![&card.title, &card.description, &card.label].iter().any(|s| s.to_lowercase().contains(&query))
                                        {
                                            continue;
                                        }
                                        let position = (index, card_index);
                                        let can_drag = writable && query.is_empty() && self.renaming.is_none();
                                        let mut body = |ui: &mut Ui| {
                                            egui::Frame::NONE.fill(theme::card()).corner_radius(8).inner_margin(10).show(ui, |ui| {
                                                ui.set_width(tile_width);
                                                ui.horizontal(|ui| {
                                                    ui.allocate_ui_with_layout(
                                                        vec2((tile_width - 38.0).max(40.0), 34.0),
                                                        egui::Layout::top_down(egui::Align::Min),
                                                        |ui| {
                                                            if let Some((editing, text)) = self.renaming.as_mut().filter(|(p, _)| *p == position) {
                                                                let edit = ui.add(
                                                                    egui::TextEdit::singleline(text).desired_width((tile_width - 40.0).max(36.0)),
                                                                );
                                                                house::target(ui.ctx(), "kanban:inline-title", edit.rect);
                                                                if !edit.has_focus() && !edit.lost_focus() {
                                                                    edit.request_focus();
                                                                }
                                                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                                                    rename = Some((*editing, None));
                                                                } else if edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                                                    rename = Some((*editing, Some(text.trim().to_string())));
                                                                }
                                                            } else {
                                                                let title = if let Some(number) = card.number.filter(|_| board.numbering) {
                                                                    format!("#{number} {}", card.title)
                                                                } else {
                                                                    card.title.clone()
                                                                };
                                                                let title = ui.add(egui::Label::new(title).wrap().sense(egui::Sense::click()));
                                                                house::target(ui.ctx(), &format!("kanban:card:{index}:{card_index}"), title.rect);
                                                                if writable && title.double_clicked() {
                                                                    self.renaming = Some((position, card.title.clone()));
                                                                }
                                                            }
                                                        },
                                                    );
                                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                        let expand = house::icon_button(ui, "expand_more", "Expand card", false);
                                                        if target(ui, &format!("kanban:expand:{index}:{card_index}"), expand) {
                                                            self.editor = Some((index, Some(card_index), index, card.clone()));
                                                        }
                                                    });
                                                });
                                                if !card.label.is_empty() {
                                                    ui.label(&card.label);
                                                }
                                                if !card.due.is_empty() {
                                                    ui.label(&card.due);
                                                }
                                                if !card.checklist.is_empty() {
                                                    ui.label(format!(
                                                        "{}/{} checklist",
                                                        card.checklist.iter().filter(|(done, _)| *done).count(),
                                                        card.checklist.len()
                                                    ));
                                                }
                                            });
                                        };
                                        let tile = if can_drag {
                                            ui.dnd_drag_source(ui.scope_id().with(card_index), CardDrag(position), body).response
                                        } else {
                                            ui.vertical(&mut body).response
                                        };
                                        if writable && let Some(source) = tile.dnd_release_payload::<CardDrag>() {
                                            movement = Some((source.0, position));
                                        }
                                    }
                                });
                            });
                        });
                        if writable && query.is_empty() {
                            let zone = panel.response;
                            house::target(ui.ctx(), &format!("kanban:drop:{index}"), zone.rect);
                            if zone.dnd_hover_payload::<CardDrag>().is_some() {
                                ui.painter().rect_stroke(zone.rect, 12, egui::Stroke::new(2.0, theme::text_dim()), egui::StrokeKind::Inside);
                            }
                            if let Some(source) = zone.dnd_release_payload::<CardDrag>() {
                                movement = Some((source.0, (index, column.cards.len())));
                            }
                        }
                    });
                }
                if writable && ui.button("Add column").clicked() {
                    self.column = Some((None, String::new(), 0));
                }
            });
        });
        if let Some((source, target)) = movement {
            board.move_card(source, target);
            self.dirty = true;
        }
        if let Some(((column, card), title)) = rename {
            if let Some(title) = title.filter(|title| !title.is_empty()) {
                board.columns[column].cards[card].title = title;
                self.dirty = true;
            }
            self.renaming = None;
        }
    }

    fn edit_ui(&mut self, ctx: &egui::Context) {
        let (Some(board), Some((from, index, to, card))) = (&mut self.board, &mut self.editor) else { return };
        let mut close = false;
        let mut save = false;
        let mut trash = false;
        house::dialog(ctx, "kanban_card", 420.0, if index.is_some() { "Card" } else { "New card" }, Some(&mut close), |ui| {
            ui.add_enabled_ui(self.writable, |ui| {
                ui.label("Title");
                let title = ui.text_edit_singleline(&mut card.title);
                house::target(ui.ctx(), "kanban:card-title", title.rect);
                ui.label("Description");
                ui.text_edit_multiline(&mut card.description);
                ui.label("Label");
                ui.text_edit_singleline(&mut card.label);
                ui.label("Due date");
                ui.add(egui::TextEdit::singleline(&mut card.due).hint_text("YYYY-MM-DD"));
                egui::ComboBox::from_id_salt("kanban_card_column").selected_text(&board.columns[*to].name).show_ui(ui, |ui| {
                    for (i, column) in board.columns.iter().enumerate() {
                        ui.selectable_value(to, i, &column.name);
                    }
                });
                ui.label("Checklist");
                let mut remove = None;
                egui::ScrollArea::vertical().max_height(145.0).show(ui, |ui| {
                    for (i, (done, text)) in card.checklist.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.checkbox(done, "");
                            ui.text_edit_singleline(text);
                            if ui.small_button("Remove").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                });
                if let Some(i) = remove {
                    card.checklist.remove(i);
                }
                if ui.button("Add checklist item").clicked() {
                    card.checklist.push((false, String::new()));
                }
                ui.horizontal(|ui| {
                    let apply = ui.add_enabled(!card.title.trim().is_empty(), egui::Button::new("Apply card"));
                    save = target(ui, "kanban:apply-card", apply);
                    if index.is_some() {
                        let archive = ui.button("Archive card");
                        trash = target(ui, "kanban:archive-card", archive);
                    }
                });
            });
        });
        if save {
            if let Some(index) = *index {
                if *from == *to {
                    board.columns[*from].cards[index] = card.clone();
                } else {
                    board.columns[*from].cards.remove(index);
                    board.columns[*to].cards.push(card.clone());
                }
            } else {
                board.columns[*to].cards.push(card.clone());
            }
            if board.numbering {
                board.number_cards();
            }
            self.dirty = true;
        }
        if trash && let Some(i) = *index {
            board.archive.push(board.columns[*from].cards.remove(i));
            self.dirty = true;
        }
        if close || save || trash || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.editor = None;
        }
    }

    fn column_ui(&mut self, ctx: &egui::Context) {
        let (Some(board), Some((index, name, limit))) = (&mut self.board, &mut self.column) else { return };
        let mut close = false;
        let mut save = false;
        let mut remove = false;
        let mut shift = None;
        house::dialog(ctx, "kanban_column", 360.0, "Column", Some(&mut close), |ui| {
            ui.label("Name");
            ui.text_edit_singleline(name);
            ui.horizontal(|ui| {
                ui.label("Work in progress limit");
                ui.add(egui::DragValue::new(limit).range(0..=999));
            });
            house::note(ui, "Use 0 for no limit.");
            ui.horizontal(|ui| {
                save = ui.add_enabled(!name.trim().is_empty(), egui::Button::new("Apply column")).clicked();
                if let Some(i) = *index {
                    if ui.add_enabled(i > 0, egui::Button::new("Move left")).clicked() {
                        shift = Some(i - 1);
                    }
                    if ui.add_enabled(i + 1 < board.columns.len(), egui::Button::new("Move right")).clicked() {
                        shift = Some(i + 1);
                    }
                    remove = ui
                        .add_enabled(board.columns.len() > 1 && board.columns[i].cards.is_empty(), egui::Button::new("Remove empty column"))
                        .clicked();
                }
            });
        });
        if save {
            let limit = (*limit != 0).then_some(*limit);
            if let Some(i) = *index {
                board.columns[i].name = name.trim().into();
                board.columns[i].limit = limit;
            } else {
                board.columns.push(Column { name: name.trim().into(), limit, cards: Vec::new() });
            }
        }
        if let Some(i) = *index {
            if let Some(target) = shift {
                board.columns.swap(i, target);
            }
            if remove {
                board.columns.remove(i);
            }
        }
        self.dirty |= save || remove || shift.is_some();
        if close || save || remove || shift.is_some() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.column = None;
        }
    }

    fn archive_ui(&mut self, ui: &mut Ui) {
        let Some(board) = &mut self.board else { return };
        if board.archive.is_empty() {
            ui.label("No archived cards");
            return;
        }
        let mut restore = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for (index, card) in board.archive.iter().enumerate() {
                ui.horizontal(|ui| {
                    if let Some(number) = card.number.filter(|_| board.numbering) {
                        ui.label(format!("#{number}"));
                    }
                    ui.label(&card.title);
                    if self.writable && !self.pending {
                        if let Some(first) = board.columns.first()
                            && ui.button(format!("Restore to {}", first.name)).clicked()
                        {
                            restore = Some(index);
                        }
                        let delete = ui.button("Delete permanently");
                        if target(ui, &format!("kanban:delete:{index}"), delete) {
                            self.delete_from_archive = Some(index);
                        }
                    }
                });
            }
        });
        if let Some(index) = restore {
            board.columns[0].cards.push(board.archive.remove(index));
            self.dirty = true;
        }
    }

    fn confirm_delete(&mut self, ctx: &egui::Context) {
        let (Some(index), Some(board)) = (self.delete_from_archive, &mut self.board) else { return };
        let mut close = false;
        let mut confirmed = false;
        house::dialog(ctx, "kanban_delete", 380.0, "Delete archived card?", Some(&mut close), |ui| {
            ui.label(&board.archive[index].title);
            let button = ui.button("Delete permanently");
            confirmed = target(ui, "kanban:confirm-delete", button);
        });
        if confirmed {
            board.archive.remove(index);
            self.dirty = true;
        }
        if close || confirmed || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.delete_from_archive = None;
        }
    }

    /// True when returning to Drive.
    pub(super) fn ui(&mut self, ui: &mut Ui) -> bool {
        self.accept_reply();
        let ctx = ui.ctx().clone();
        let mut exit = false;
        let mut save = false;
        egui::Panel::top("kanban_header")
            .show_separator_line(false)
            .exact_size(super::TOP)
            .frame(egui::Frame::NONE.fill(theme::page()).inner_margin(egui::Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    super::header::brand(ui, crate::icons::icon("view_column"), "Starbase Kanban", None);
                    if self.board.is_some() {
                        super::header::search(ui, &mut self.filter, "kanban_search", "Filter cards", "kanban:search");
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = house::S;
                        crate::account::me_ui(ui);
                        exit = super::header::switch(ui, "Return to Drive", "kanban:back", !self.pending);
                        if self.board.is_some() {
                            save = self.save_indicator(ui);
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::card()).corner_radius(16).inner_margin(egui::Margin::symmetric(24, 16)))
            .show(ui, |ui| {
                if let Some(error) = &self.error {
                    ui.colored_label(house::ERROR, error);
                }
                if self.pending {
                    ui.spinner();
                }
                if let Some(folder) = self.create_in.clone() {
                    ui.label("Board name");
                    let title = ui.text_edit_singleline(&mut self.name);
                    house::target(ui.ctx(), "kanban:board-name", title.rect);
                    let create = ui.add_enabled(!self.pending && !self.name.trim().is_empty(), egui::Button::new("Create board"));
                    if target(ui, "kanban:create", create) {
                        self.create(&ctx, &folder);
                    }
                }
                if self.board.is_some() {
                    ui.horizontal(|ui| {
                        ui.heading(self.name.strip_suffix(".kanban").unwrap_or(&self.name));
                        let board = self.board.as_mut().unwrap();
                        if !self.pending && self.writable {
                            let numbering = ui.checkbox(&mut board.numbering, "Number cards");
                            house::target(ui.ctx(), "kanban:numbering", numbering.rect);
                            if numbering.changed() {
                                board.normalize_numbering();
                                self.dirty = true;
                            }
                        }
                        let archive = ui.selectable_label(self.archive_open, format!("Archive ({})", board.archive.len()));
                        if target(ui, "kanban:archive", archive) {
                            self.archive_open = !self.archive_open;
                        }
                    });
                }
                if self.archive_open {
                    self.archive_ui(ui);
                } else {
                    self.draw_board(ui);
                }
            });
        self.edit_ui(&ctx);
        self.column_ui(&ctx);
        self.confirm_delete(&ctx);
        if exit && self.dirty {
            exit = false;
            self.leave_after_save = self.error.is_none();
            self.confirm_close = !self.leave_after_save;
        }
        if self.leave_after_save && !self.pending {
            if !self.dirty {
                return true;
            }
            if self.error.is_some() {
                self.leave_after_save = false;
                self.confirm_close = true;
            }
        }
        if self.dirty && !self.pending && (self.error.is_none() || save) {
            self.save(&ctx);
        }
        if self.confirm_close {
            let mut close = false;
            let mut keep = false;
            house::dialog(&ctx, "kanban_leave", 360.0, "Unsaved board changes", Some(&mut close), |ui| {
                ui.label("Return to Drive and discard your unsaved changes?");
                exit = ui.button("Discard changes").clicked();
                keep = ui.button("Keep editing").clicked();
            });
            self.confirm_close = !close && !exit && !keep;
        }
        exit
    }
}

impl Home {
    pub(super) fn new_kanban(&mut self, folder: &str) {
        self.kanban = Some(Kanban { create_in: Some(folder.into()), writable: true, ..Kanban::default() });
    }

    pub(super) fn open_kanban(&mut self, ctx: &egui::Context, id: &str) -> bool {
        let Some(file) = self.file(id).filter(|file| is_board(&file.name)) else { return false };
        self.kanban = Some(Kanban::open(ctx, id, &file.name, super::may_edit(&file.role)));
        true
    }
}
