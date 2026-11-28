//! Shared positions for workspace titles, search, account and the return control.

use egui::{RichText, Ui, vec2};

use super::LEFT;
use crate::{house, theme};

pub(super) fn brand(ui: &mut Ui, logo: Option<egui::Image<'static>>, name: &str, target: Option<&str>) {
    let compact = ui.available_width() < 740.0;
    let width = if compact { 52.0 } else { LEFT - 32.0 };
    ui.allocate_ui_with_layout(vec2(width, 48.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        if let Some(logo) = logo {
            let image = ui.add(logo.fit_to_exact_size(vec2(36.0, 36.0)));
            if let Some(target) = target {
                house::target(ui.ctx(), target, image.rect);
            }
        }
        if !compact {
            ui.add_space(house::S);
            ui.label(RichText::new(name).size(22.0).color(theme::text()));
        }
    });
}

pub(super) fn search(ui: &mut Ui, text: &mut String, id: &'static str, hint: &str, target: &str) {
    let room = ui.available_width();
    let reserve = if room < 770.0 { 218.0 } else { 490.0 };
    let width = (room - reserve).clamp(100.0, 600.0);
    let field = house::field(text, house::SEARCH_TEXT)
        .id(egui::Id::unique(id))
        .hint_text(RichText::new(hint).size(house::SEARCH_TEXT))
        .frame(egui::Frame::NONE)
        .margin(vec2(0.0, 0.0))
        .min_size(vec2(0.0, 24.0))
        .desired_width((width - 57.0).max(40.0));
    let surface = if theme::photo_dark() { theme::menu_hover() } else { theme::drive_surface() };
    let box_ui = ui.allocate_ui_with_layout(vec2(width, 48.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        egui::Frame::NONE.fill(surface).corner_radius(24).inner_margin(egui::Margin::symmetric(16, 12)).show(ui, |ui| {
            if let Some(img) = crate::icons::icon("search") {
                ui.add(img.fit_to_exact_size(vec2(20.0, 20.0)));
            }
            ui.add_space(house::S);
            let input = ui.add(field);
            house::target(ui.ctx(), target, input.rect);
        });
    });
    house::target(ui.ctx(), "workspace:search", box_ui.response.rect);
    if target == "photo-search" {
        house::target(ui.ctx(), target, box_ui.response.rect);
    }
}

pub(super) fn switch(ui: &mut Ui, label: &str, target: &str, enabled: bool) -> bool {
    let button = ui.add_enabled_ui(enabled, |ui| ui.add_sized(vec2(126.0, 34.0), house::quiet_with(None, label, false))).inner;
    house::target(ui.ctx(), target, button.rect);
    button.clicked()
}
