//! The home screen's bin: what was deleted, to restore or remove for good.

use super::*;

impl Home {
    pub(super) fn bin_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            house::note(ui, "Things in the bin are deleted for good after 30 days.");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let sure = self.confirm_purge == Some(None);
                if ui
                    .add_enabled(
                        !self.bin.is_empty(),
                        egui::Button::new(RichText::new(if sure { "Click again to empty the bin" } else { "Empty bin" }).color(house::ERROR))
                            .frame(false),
                    )
                    .clicked()
                {
                    if sure {
                        call(&self.store, &self.inbox, ui.ctx(), Method::Delete, "api/bin".into(), None, refresh_after);
                        self.confirm_purge = None;
                    } else {
                        self.confirm_purge = Some(None);
                    }
                }
            });
        });
        ui.add_space(house::S);
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if self.loading && self.bin.is_empty() {
                ui.label(RichText::new("Loading…").color(theme::TEXT_DIM));
            } else if self.bin.is_empty() {
                ui.label(RichText::new("The bin is empty.").color(theme::TEXT_DIM));
            }
            let w = ui.available_width() - 8.0;
            for b in self.bin.clone() {
                let (r, _) = ui.allocate_exact_size(vec2(w, ROW + 6.0), Sense::hover());
                let p = ui.painter().clone();
                if let Some(img) = crate::icons::icon(if b.folder {
                    "folder"
                } else if crate::store::is_doc(&b.original) {
                    "stardocs"
                } else {
                    "starsheets"
                }) {
                    img.paint_at(ui, Rect::from_center_size(r.left_center() + vec2(20.0, 0.0), vec2(20.0, 20.0)));
                }
                p.text(r.left_center() + vec2(44.0, -8.0), Align2::LEFT_CENTER, name_of(&b.original), FontId::proportional(14.0), theme::TEXT);
                let from = parent_of(&b.original);
                let from = if from.is_empty() || from == self.space {
                    "My files".to_string()
                } else {
                    from.strip_prefix(&format!("{}/", self.space)).unwrap_or(from).to_string()
                };
                p.text(
                    r.left_center() + vec2(44.0, 10.0),
                    Align2::LEFT_CENTER,
                    format!("from {from} · deleted {}", ago(b.deleted)),
                    FontId::proportional(12.0),
                    theme::TEXT_DIM,
                );
                let restore = Rect::from_min_size(r.right_top() + vec2(-260.0, 6.0), vec2(96.0, ROW - 6.0));
                let purge = Rect::from_min_size(r.right_top() + vec2(-156.0, 6.0), vec2(150.0, ROW - 6.0));
                if ui.put(restore, house::secondary_in("Restore", ACCENT)).clicked() {
                    call(&self.store, &self.inbox, ui.ctx(), Method::Post, format!("api/bin/restore?name={}", enc(&b.name)), None, refresh_after);
                }
                let sure = self.confirm_purge.as_ref() == Some(&Some(b.name.clone()));
                if ui
                    .put(
                        purge,
                        egui::Button::new(RichText::new(if sure { "Sure? Click again" } else { "Delete forever" }).color(house::ERROR)).frame(false),
                    )
                    .clicked()
                {
                    if sure {
                        call(&self.store, &self.inbox, ui.ctx(), Method::Delete, format!("api/bin?name={}", enc(&b.name)), None, refresh_after);
                        self.confirm_purge = None;
                    } else {
                        self.confirm_purge = Some(Some(b.name.clone()));
                    }
                }
            }
        });
    }
}
