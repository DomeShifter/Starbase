//! Drive's view options, remembered in the local settings: the List view's columns (which show,
//! how wide; drag a header edge) and the Grid view's cards (previews, dates, size). Both are
//! set from the ▾ half of the view buttons, [List | ▾] and [Grid | ▾].

use super::*;

/// The list's columns after Name, left to right: key and default width. Name always shows and
/// takes what is left.
pub(super) const COLS: [(&str, f32); 4] = [("owner", 180.0), ("modified", 160.0), ("type", 130.0), ("size", 100.0)];
/// The narrowest Name and other columns can be dragged.
pub(super) const NAME_MIN: f32 = 160.0;
pub(super) const COL_MIN: f32 = 70.0;
/// Card sizes: Compact, Medium, Large.
pub(super) const SIZES: [&str; 3] = ["Compact", "Medium", "Large"];

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ViewOpts {
    /// List columns hidden (keys of COLS).
    pub hidden: Vec<String>,
    /// List column widths dragged away from the default.
    pub widths: Vec<(String, f32)>,
    /// Grid: preview pictures on the cards, the date line, and the card size (index into SIZES).
    pub previews: bool,
    pub dates: bool,
    pub size: u8,
}

impl Default for ViewOpts {
    fn default() -> Self {
        Self { hidden: Vec::new(), widths: Vec::new(), previews: true, dates: true, size: 1 }
    }
}

impl ViewOpts {
    pub fn load(store: &Backend) -> Self {
        Self::parse(store.setting("starsheets_home_cols").as_deref(), store.setting("starsheets_home_grid").as_deref())
    }

    pub fn save(&self, store: &Backend) {
        store.set_setting("starsheets_home_cols", &self.cols_str());
        store.set_setting("starsheets_home_grid", &self.grid_str());
    }

    /// `cols`: "key" or "key=width", "-" in front when hidden, comma separated.
    /// `grid`: previews, dates, size as three digits.
    pub fn parse(cols: Option<&str>, grid: Option<&str>) -> Self {
        let mut o = Self::default();
        for part in cols.unwrap_or("").split(',').filter(|p| !p.is_empty()) {
            let (hide, part) = match part.strip_prefix('-') {
                Some(p) => (true, p),
                None => (false, part),
            };
            let (key, width) = part.split_once('=').map_or((part, None), |(k, w)| (k, w.parse::<f32>().ok()));
            if !COLS.iter().any(|(k, _)| *k == key) {
                continue;
            }
            if hide {
                o.hidden.push(key.to_string());
            }
            if let Some(w) = width.filter(|w| w.is_finite()) {
                o.widths.push((key.to_string(), w.clamp(COL_MIN, 2000.0)));
            }
        }
        if let Some(g) = grid.map(str::as_bytes).filter(|g| g.len() == 3) {
            o.previews = g[0] != b'0';
            o.dates = g[1] != b'0';
            o.size = g[2].saturating_sub(b'0').min(2);
        }
        o
    }

    pub fn cols_str(&self) -> String {
        COLS.iter()
            .filter_map(|(k, _)| {
                let hide = if self.shown(k) { "" } else { "-" };
                let w = self.widths.iter().find(|(x, _)| x == k).map(|(_, w)| format!("={}", w.round()));
                (!hide.is_empty() || w.is_some()).then(|| format!("{hide}{k}{}", w.unwrap_or_default()))
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    pub fn grid_str(&self) -> String {
        format!("{}{}{}", u8::from(self.previews), u8::from(self.dates), self.size)
    }

    pub fn shown(&self, key: &str) -> bool {
        !self.hidden.iter().any(|h| h == key)
    }

    pub fn toggle(&mut self, key: &str) {
        if self.shown(key) {
            self.hidden.push(key.to_string());
        } else {
            self.hidden.retain(|h| h != key);
        }
    }

    pub fn width(&self, key: &str) -> f32 {
        self.widths.iter().find(|(k, _)| k == key).map_or_else(|| COLS.iter().find(|(k, _)| *k == key).map_or(100.0, |c| c.1), |c| c.1)
    }

    pub fn set_width(&mut self, key: &str, w: f32) {
        self.widths.retain(|(k, _)| k != key);
        self.widths.push((key.to_string(), w));
    }

    /// The visible columns of a list `w` wide, of those that exist here (`have`): (key, x, width),
    /// Name ("name") first at 0 taking the room the others leave.
    pub fn layout(&self, have: &[&'static str], w: f32) -> Vec<(&'static str, f32, f32)> {
        let rest: Vec<(&'static str, f32)> =
            COLS.iter().filter(|(k, _)| have.contains(k) && self.shown(k)).map(|(k, _)| (*k, self.width(k))).collect();
        let name_w = (w - rest.iter().map(|c| c.1).sum::<f32>()).max(NAME_MIN);
        let mut out = vec![("name", 0.0, name_w)];
        let mut x = name_w;
        for (k, cw) in rest {
            out.push((k, x, cw));
            x += cw;
        }
        out
    }

    /// Card width, height and picture height for the chosen size.
    pub fn card(&self) -> (f32, f32, f32) {
        self.card_with(self.dates)
    }

    /// Like `card`, for a grid where a second line is shown (`sub`) or not.
    pub fn card_with(&self, sub: bool) -> (f32, f32, f32) {
        let f = [0.78, 1.0, 1.32][usize::from(self.size.min(2))];
        let pic = (PIC_H * f).round();
        // Without a second line (date, owner or snippet) the card loses that line's height too, instead of leaving a gap.
        let tail = if sub { CARD_H - PIC_H } else { CARD_H - PIC_H - SUB_H };
        ((CARD_W * f).round(), pic + tail, pic)
    }
}

/// A column's header text.
pub(super) fn col_label(key: &str, storage: bool) -> &'static str {
    match key {
        "name" => "Name",
        "owner" if storage => "Location",
        "owner" => "Owner",
        "modified" => "Last modified",
        "type" => "Type",
        "size" => "Size",
        _ => "",
    }
}

/// What kind of thing a list row is, in words.
pub(super) fn type_label(item: &Item, name: &str) -> &'static str {
    if matches!(item, Item::Folder(_)) {
        return "Folder";
    }
    match super::items::file_kind(name) {
        "sheet" => "Spreadsheet",
        "doc" => "Document",
        "image" => "Image",
        "audio" => "Audio",
        "video" => "Video",
        "pdf" => "PDF",
        "text" => "Text",
        _ if name.to_ascii_lowercase().ends_with(".pptx") => "Presentation",
        _ => "File",
    }
}

impl Home {
    /// One half-pair of the segmented view buttons, [List | ▾] / [Grid | ▾], drawn right to left
    /// (the ▾ first). Returns (main clicked, the ▾'s response).
    pub(super) fn view_button(&mut self, ui: &mut Ui, icon: &str, on: bool, tip: &str) -> (bool, egui::Response) {
        let fill = if on { theme::SHARE } else { Color32::WHITE };
        let mut out = (false, None);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let more = match crate::icons::icon("arrow_drop_down") {
                Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(18.0, 18.0))),
                None => egui::Button::new("▾"),
            };
            let more = ui
                .add(more.fill(fill).corner_radius(egui::CornerRadius { nw: 0, sw: 0, ne: 16, se: 16 }).min_size(vec2(26.0, 32.0)))
                .on_hover_text(format!("{tip} options"));
            house::target(ui.ctx(), &format!("{tip} options"), more.rect);
            let b = match crate::icons::icon(icon) {
                Some(img) => egui::Button::image(img.fit_to_exact_size(vec2(20.0, 20.0))),
                None => egui::Button::new(tip),
            };
            let main =
                ui.add(b.fill(fill).corner_radius(egui::CornerRadius { nw: 16, sw: 16, ne: 0, se: 0 }).min_size(vec2(40.0, 32.0))).on_hover_text(tip);
            house::target(ui.ctx(), tip, main.rect);
            out = (main.clicked(), Some(more));
        });
        (out.0, out.1.expect("drawn"))
    }

    /// The List ▾ menu: a tick box per column.
    pub(super) fn list_menu(&mut self, more: &egui::Response) {
        let storage = self.view == Some(View::Storage);
        let have = self.list_have();
        egui::Popup::menu(more).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            house::menu(ui, 220.0);
            house::menu_inset(ui, |ui| house::caps(ui, "Columns"));
            house::menu_row(ui, Some("check_box"), "Name", "", false);
            for (k, _) in COLS {
                if !have.contains(&k) {
                    continue;
                }
                let icon = if self.vo.shown(k) { "check_box" } else { "check_box_outline_blank" };
                if house::menu_row(ui, Some(icon), col_label(k, storage), "", true) {
                    self.vo.toggle(k);
                    self.vo.save(&self.store);
                }
            }
            house::menu_sep(ui);
            if house::menu_row(ui, Some(""), "Reset columns", "", true) {
                self.vo.hidden.clear();
                self.vo.widths.clear();
                self.vo.save(&self.store);
            }
        });
    }

    /// The Grid ▾ menu: previews, dates, card size. No sort here: Drive's own Sort menu orders the grid too.
    pub(super) fn grid_menu(&mut self, more: &egui::Response) {
        egui::Popup::menu(more).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            house::menu(ui, 240.0);
            let boxed = |on: bool| Some(if on { "check_box" } else { "check_box_outline_blank" });
            let tick = |on: bool| Some(if on { "check" } else { "" });
            if house::menu_row(ui, boxed(self.vo.previews), "Show previews", "", true) {
                self.vo.previews = !self.vo.previews;
                self.vo.save(&self.store);
            }
            if house::menu_row(ui, boxed(self.vo.dates), "Show date modified", "", true) {
                self.vo.dates = !self.vo.dates;
                self.vo.save(&self.store);
            }
            house::menu_sep(ui);
            house::menu_inset(ui, |ui| house::caps(ui, "Card size"));
            for (i, l) in SIZES.iter().enumerate() {
                if house::menu_row(ui, tick(usize::from(self.vo.size) == i), l, "", true) {
                    self.vo.size = i as u8;
                    self.vo.save(&self.store);
                }
            }
        });
    }

    /// The columns this list can show besides Name (Owner / Location only where there is one).
    pub(super) fn list_have(&self) -> Vec<&'static str> {
        let storage = self.view == Some(View::Storage);
        COLS.iter().map(|(k, _)| *k).filter(|k| *k != "owner" || storage || self.accounts).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_without_dates_is_shorter_and_keeps_the_picture() {
        let mut o = ViewOpts::default();
        for size in 0..3 {
            o.size = size;
            o.dates = true;
            let (w, h, pic) = o.card();
            o.dates = false;
            let (w2, h2, pic2) = o.card();
            assert_eq!((w, pic), (w2, pic2));
            assert_eq!(h - h2, SUB_H);
            // Owner or snippet lines keep the line (and the height) even with dates off.
            assert_eq!(o.card_with(true), (w, h, pic));
        }
    }

    #[test]
    fn view_options_survive_a_save_and_load() {
        let mut o = ViewOpts::default();
        assert_eq!((o.cols_str().as_str(), o.grid_str().as_str()), ("", "111"));
        o.toggle("size");
        o.set_width("modified", 240.4);
        o.previews = false;
        o.size = 2;
        let back = ViewOpts::parse(Some(&o.cols_str()), Some(&o.grid_str()));
        assert!(!back.shown("size") && back.shown("type"));
        assert_eq!(back.width("modified"), 240.0);
        assert_eq!((back.previews, back.dates, back.size), (false, true, 2));
        // Rubbish and unknown columns are ignored, sizes clamped.
        let odd = ViewOpts::parse(Some("-name,bogus=5,type=NaN,owner=1"), Some("009"));
        assert!(odd.shown("owner") && odd.hidden.is_empty());
        assert_eq!(odd.width("owner"), COL_MIN);
        assert_eq!(odd.width("type"), 130.0);
        assert_eq!((odd.previews, odd.size), (false, 2));
    }

    #[test]
    fn every_card_size_and_column_lays_out() {
        for size in 0..3 {
            let o = ViewOpts { size, ..Default::default() };
            let (w, h, pic) = o.card();
            assert!(w > 100.0 && h > pic && pic > 80.0, "{size}");
        }
        let mut o = ViewOpts::default();
        let all = ["owner", "modified", "type", "size"];
        let l = o.layout(&all, 1000.0);
        assert_eq!(l.iter().map(|c| c.0).collect::<Vec<_>>(), ["name", "owner", "modified", "type", "size"]);
        assert_eq!(l[0].2, 1000.0 - 570.0);
        o.toggle("owner");
        let l = o.layout(&all, 1000.0);
        assert_eq!(l[1], ("modified", 1000.0 - 390.0, 160.0));
        // Never squeezes Name below its minimum.
        assert_eq!(o.layout(&all, 300.0)[0].2, NAME_MIN);
        for (k, _) in COLS {
            assert!(!col_label(k, false).is_empty() && !col_label(k, true).is_empty());
        }
    }
}
