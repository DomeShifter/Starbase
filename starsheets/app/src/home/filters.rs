//! Search filter chips (Type, People, Modified) under the search box, as in Google Drive. They
//! narrow the search results on the screen (the search itself runs over the list already loaded).

use super::*;

/// The chips' choices; `None` / 0 means "any".
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Filters {
    /// folder, sheet, doc, slides, image, pdf
    pub kind: Option<&'static str>,
    /// 0 anyone, 1 me, 2 others
    pub who: u8,
    /// 0 any time, 1 today, 2 last 7 days, 3 last 30 days, 4 this year
    pub when: u8,
}

pub(super) const KINDS: [(&str, &str); 6] = [
    ("folder", "Folders"),
    ("sheet", "Spreadsheets"),
    ("doc", "Documents"),
    ("slides", "Presentations"),
    ("image", "Photos & images"),
    ("pdf", "PDFs"),
];
const WHO: [&str; 3] = ["Anyone", "Owned by me", "Owned by others"];
const WHEN: [&str; 5] = ["Any time", "Today", "Last 7 days", "Last 30 days", "This year"];

/// What a file is, for the Type chip.
fn kind_of(name: &str) -> &'static str {
    if crate::store::is_slides(name) {
        "slides"
    } else {
        match items::file_kind(name) {
            "doc" => "doc",
            "sheet" => "sheet",
            "image" => "image",
            "pdf" => "pdf",
            _ => "other",
        }
    }
}

fn now_unix() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Date::now() / 1000.0) as i64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
    }
}

/// Unix time of 1 January of the year `now` falls in (UTC).
fn year_start(now: i64) -> i64 {
    // Days since 1970 -> year (Howard Hinnant's civil-from-days), then days-from-civil of Jan 1.
    let z = now.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let y = yoe + era * 400 + i64::from((5 * doy + 2) / 153 >= 10);
    let y1 = y - 1; // Jan 1 of y is day 306 of the March-based year y - 1
    let (era1, yoe1) = (y1.div_euclid(400), y1.rem_euclid(400));
    let doe1 = yoe1 * 365 + yoe1 / 4 - yoe1 / 100 + 306;
    (era1 * 146_097 + doe1 - 719_468) * 86_400
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_combine() {
        let f = |name: &str, owner: &str, modified| FileRow { name: name.into(), owner: owner.into(), modified, ..Default::default() };
        let now = now_unix();
        let sheet = f("a.xlsx", "ann", now);
        let old_doc = f("b.docx", "bob", 1_000_000);
        let slides = f("c.PPTX", "ann", now);
        let all = |x: &Filters| [&sheet, &old_doc, &slides].map(|r| x.file_ok(r, "ann"));
        assert_eq!(all(&Filters::default()), [true; 3]);
        assert_eq!(all(&Filters { kind: Some("sheet"), ..Default::default() }), [true, false, false]);
        assert_eq!(all(&Filters { kind: Some("slides"), ..Default::default() }), [false, false, true]);
        assert_eq!(all(&Filters { who: 2, ..Default::default() }), [false, true, false]);
        assert_eq!(all(&Filters { when: 2, ..Default::default() }), [true, false, true]);
        assert_eq!(all(&Filters { who: 1, when: 1, kind: Some("slides") }), [false, false, true]);
        assert!(!Filters { kind: Some("sheet"), ..Default::default() }.folder_ok(&FolderRow::default(), "ann", now));
    }

    #[test]
    fn year_starts() {
        assert_eq!(super::year_start(1_764_000_000), 1_767_225_600 - 31_536_000); // 2025-11-24 -> 2025-01-01
        assert_eq!(super::year_start(1_767_225_600), 1_767_225_600); // 2026-01-01 exactly
        assert_eq!(super::year_start(1_767_225_599), 1_735_689_600); // last second of 2025
    }
}

impl Filters {
    pub fn any(&self) -> bool {
        *self != Filters::default()
    }

    /// Whether something modified at `modified` passes the Modified chip.
    fn when_ok(&self, modified: i64, now: i64) -> bool {
        let since = match self.when {
            1 => now - now.rem_euclid(86_400),
            2 => now - 7 * 86_400,
            3 => now - 30 * 86_400,
            4 => year_start(now),
            _ => return true,
        };
        modified >= since
    }

    fn who_ok(&self, owner: &str, me: &str) -> bool {
        let mine = owner.is_empty() || owner == me;
        match self.who {
            1 => mine,
            2 => !mine,
            _ => true,
        }
    }

    /// Whether a file passes all three chips (`me` is my own folder / username).
    pub fn file_ok(&self, f: &FileRow, me: &str) -> bool {
        self.kind.is_none_or(|k| k == kind_of(&f.name)) && self.who_ok(&f.owner, me) && self.when_ok(f.modified, now_unix())
    }

    /// Whether a folder passes (its date is its newest file's; none passes only with any time).
    pub fn folder_ok(&self, f: &FolderRow, me: &str, newest: i64) -> bool {
        self.kind.is_none_or(|k| k == "folder") && self.who_ok(&f.owner, me) && self.when_ok(newest, now_unix())
    }
}

impl Home {
    /// The three chips, shown while a search is on.
    pub(super) fn filter_chips(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let kind_label = KINDS.iter().find(|(k, _)| Some(*k) == self.filters.kind).map_or("Type", |(_, l)| *l);
            chip(
                ui,
                kind_label,
                self.filters.kind.is_some(),
                |ui, f| {
                    let mut hit = false;
                    for (k, l) in KINDS {
                        if house::menu_row(ui, Some(if f.kind == Some(k) { "check" } else { "" }), l, "", true) {
                            f.kind = if f.kind == Some(k) { None } else { Some(k) };
                            hit = true;
                        }
                    }
                    hit
                },
                &mut self.filters,
            );
            if self.accounts {
                let l = if self.filters.who == 0 { "People" } else { WHO[self.filters.who as usize] };
                chip(ui, l, self.filters.who != 0, |ui, f| pick(ui, &WHO, &mut f.who), &mut self.filters);
            }
            let l = if self.filters.when == 0 { "Modified" } else { WHEN[self.filters.when as usize] };
            chip(ui, l, self.filters.when != 0, |ui, f| pick(ui, &WHEN, &mut f.when), &mut self.filters);
            if self.filters.any() && ui.add(house::link("Clear filters")).clicked() {
                self.filters = Filters::default();
            }
        });
        ui.add_space(house::S);
    }
}

/// A menu of exclusive choices; picking the chosen one again resets to the first ("any").
fn pick(ui: &mut Ui, names: &[&str], value: &mut u8) -> bool {
    let mut hit = false;
    for (i, l) in names.iter().enumerate() {
        if house::menu_row(ui, Some(if *value == i as u8 { "check" } else { "" }), l, "", true) {
            *value = if *value == i as u8 { 0 } else { i as u8 };
            hit = true;
        }
    }
    hit
}

/// One chip: a rounded button (tinted when it filters) that opens its menu.
fn chip(ui: &mut Ui, label: &str, on: bool, menu: impl FnOnce(&mut Ui, &mut Filters) -> bool, f: &mut Filters) -> bool {
    let text = RichText::new(format!("{label} ▾")).size(house::NOTE).color(theme::text());
    let r = ui.add(
        egui::Button::new(text)
            .fill(if on { theme::drive_selection() } else { theme::card() })
            .stroke(Stroke::new(1.0, theme::BOX))
            .corner_radius(14)
            .min_size(vec2(0.0, 28.0)),
    );
    let mut hit = false;
    egui::Popup::menu(&r).show(|ui| {
        house::menu(ui, 220.0);
        hit = menu(ui, f);
        if hit {
            ui.close();
        }
    });
    hit
}
