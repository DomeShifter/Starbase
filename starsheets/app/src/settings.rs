//! File › Settings, after Google Sheets' "Settings for this spreadsheet": the locale (how typed
//! dates are read, e.g. 05/08/2026 as 5 August in the UK), the default date, time and number
//! formats new entries get, and how numbers are written (1,234.56 or 1.234,56). Kept in the
//! browser, and handed to GridCraft's input reader and number display (patches 0026, 0033).
//! The Style tab holds look-and-feel choices (to-do 3 row 6): for now the toolbar background.

use gridcraft_engine::core::Value;
use gridcraft_ui_egui::SheetApp;
use serde::{Deserialize, Serialize};

use crate::house;

/// Asks for the Settings window (File › Settings), through egui's memory.
pub const REQUEST: &str = "starsheets_settings";
const STORE_KEY: &str = "starsheets_settings";

/// How numbers are written: label, decimal mark, thousands separator.
const SEPARATORS: &[(&str, char, char)] = &[("1,234.56", '.', ','), ("1.234,56", ',', '.'), ("1 234,56", ',', '\u{202f}'), ("1'234.56", '.', '\'')];

/// Each locale's way of writing numbers (an index into SEPARATORS), in LOCALES order.
const LOCALE_SEPARATORS: &[usize] = &[0, 0, 0, 0, 0, 1, 1, 2, 1, 1, 1, 0];

/// A locale: name, day first?, date, time and date-time formats.
const LOCALES: &[(&str, bool, &str, &str, &str)] = &[
    ("English (United Kingdom)", true, "dd/mm/yyyy", "hh:mm:ss", "dd/mm/yyyy hh:mm:ss"),
    ("English (United States)", false, "m/d/yyyy", "h:mm:ss AM/PM", "m/d/yyyy h:mm:ss"),
    ("English (Ireland)", true, "dd/mm/yyyy", "hh:mm:ss", "dd/mm/yyyy hh:mm:ss"),
    ("English (Australia)", true, "d/mm/yyyy", "h:mm:ss AM/PM", "d/mm/yyyy h:mm:ss"),
    ("English (Canada)", false, "yyyy-mm-dd", "h:mm:ss AM/PM", "yyyy-mm-dd h:mm:ss"),
    ("Deutsch (Deutschland)", true, "dd.mm.yyyy", "hh:mm:ss", "dd.mm.yyyy hh:mm:ss"),
    ("Español (España)", true, "d/m/yyyy", "hh:mm:ss", "d/m/yyyy hh:mm:ss"),
    ("Français (France)", true, "dd/mm/yyyy", "hh:mm:ss", "dd/mm/yyyy hh:mm:ss"),
    ("Italiano (Italia)", true, "dd/mm/yyyy", "hh:mm:ss", "dd/mm/yyyy hh:mm:ss"),
    ("Nederlands (Nederland)", true, "d-m-yyyy", "hh:mm:ss", "d-m-yyyy hh:mm:ss"),
    ("Português (Brasil)", true, "dd/mm/yyyy", "hh:mm:ss", "dd/mm/yyyy hh:mm:ss"),
    ("日本語 (日本)", false, "yyyy/mm/dd", "hh:mm:ss", "yyyy/mm/dd hh:mm:ss"),
];

const DATE_FORMATS: &[&str] = &[
    "dd/mm/yyyy",
    "d/m/yyyy",
    "dd-mm-yyyy",
    "dd.mm.yyyy",
    "m/d/yyyy",
    "mm/dd/yyyy",
    "yyyy-mm-dd",
    "d mmm yyyy",
    "d mmmm yyyy",
    "dd-mmm-yyyy",
    "dddd, d mmmm yyyy",
];
const TIME_FORMATS: &[&str] = &["hh:mm:ss", "hh:mm", "h:mm:ss AM/PM", "h:mm AM/PM"];
/// Default number formats ("" = Automatic, as typed).
const NUMBER_FORMATS: &[(&str, &str)] = &[("Automatic", ""), ("1,000.12", "#,##0.00"), ("1000.12", "0.00"), ("1,000", "#,##0"), ("1000", "0")];
/// The example shown for each choice: 26 September 2008, 15:59:00 (Google's sample).
const SAMPLE: f64 = 39_717.665_972_222_2;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct Prefs {
    pub locale: usize,
    pub date: String,
    pub time: String,
    pub number: String,
    /// How numbers are written (an index into SEPARATORS).
    #[serde(default)]
    pub separators: usize,
    /// Style: the toolbar background (None = Google's pale blue), 0xRRGGBB.
    #[serde(default)]
    pub toolbar: Option<u32>,
}

impl Default for Prefs {
    fn default() -> Prefs {
        let (_, _, date, time, _) = LOCALES[0];
        Prefs { locale: 0, date: date.into(), time: time.into(), number: String::new(), separators: 0, toolbar: None }
    }
}

impl Prefs {
    pub fn load() -> Prefs {
        #[cfg(target_arch = "wasm32")]
        if let Some(p) = web_sys::window()
            .and_then(|w| w.local_storage().ok().flatten())
            .and_then(|s| s.get_item(STORE_KEY).ok().flatten())
            .and_then(|t| serde_json::from_str::<Prefs>(&t).ok())
        {
            return p;
        }
        let _ = STORE_KEY;
        Prefs::default()
    }

    fn save(&self) {
        #[cfg(target_arch = "wasm32")]
        if let (Some(s), Ok(t)) = (web_sys::window().and_then(|w| w.local_storage().ok().flatten()), serde_json::to_string(self)) {
            let _ = s.set_item(STORE_KEY, &t);
        }
    }

    /// Hands the locale to GridCraft's input reader and number display.
    pub fn apply(&self, app: &mut SheetApp) {
        let (_, day_first, ..) = LOCALES[self.locale.min(LOCALES.len() - 1)];
        let (_, decimal, group) = SEPARATORS[self.separators.min(SEPARATORS.len() - 1)];
        // Through the sync crate, which remembers it: live editing sends it with each change.
        starsheets_sync::InputLocale {
            day_first,
            date: self.date.clone(),
            time: self.time.clone(),
            number: (!self.number.is_empty()).then(|| self.number.clone()),
            decimal: Some(decimal),
            group: Some(group),
        }
        .apply();
        let _ = app;
        self.apply_style();
        DEFAULTS.with(|d| *d.borrow_mut() = (self.date.clone(), self.time.clone()));
    }
}

impl Prefs {
    /// Hands the Style choices to the app's drawing (also while the window is open: live preview).
    pub fn apply_style(&self) {
        crate::theme::set_toolbar(self.toolbar.map(rgb));
    }
}

fn rgb(c: u32) -> egui::Color32 {
    egui::Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

thread_local! {
    /// The default date and time formats, for Format › Number › Date / Time / Date time.
    static DEFAULTS: std::cell::RefCell<(String, String)> = std::cell::RefCell::new(("dd/mm/yyyy".into(), "hh:mm:ss".into()));
}

/// The default date, time and date-time format codes (Format › Number uses them).
pub fn default_codes() -> (String, String, String) {
    DEFAULTS.with(|d| {
        let (date, time) = d.borrow().clone();
        let dt = format!("{date} {time}");
        (date, time, dt)
    })
}

fn sample(code: &str) -> String {
    if code.is_empty() {
        return "1000.12".into();
    }
    let v = Value::number(if code.contains('#') || code.starts_with('0') { 1000.12 } else { SAMPLE });
    gridcraft_engine::numfmt::text_function(&v, code, gridcraft_engine::core::date::DateSystem::default()).unwrap_or_else(|_| code.to_string())
}

#[derive(Default)]
pub struct SettingsWindow {
    draft: Option<Prefs>,
    /// The tab shown: 0 General, 1 Style.
    tab: usize,
}

impl SettingsWindow {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_some() {
            self.draft = Some(Prefs::load());
            self.tab = 0;
        }
        let Some(d) = self.draft.as_mut() else { return };
        let tab = &mut self.tab;
        let mut done = None;
        let mut closed = false;
        house::dialog(ctx, "starsheets_settings", 520.0, "Settings", Some(&mut closed), |ui| {
            house::tabs(ui, tab, &["General", "Style"]);
            if *tab == 1 {
                style_tab(ui, d);
            } else {
                general_tab(ui, d);
            }
            match house::actions(ui, &["Cancel", "Save settings"]) {
                Some(0) => done = Some(false),
                Some(_) => done = Some(true),
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        // Style choices show straight away, before Save (house rule: live preview).
        if let Some(d) = &self.draft {
            d.apply_style();
        }
        if closed {
            done = Some(false);
        }
        match done {
            Some(true) => {
                if let Some(p) = self.draft.take() {
                    p.save();
                    p.apply(app);
                }
            }
            Some(false) => {
                self.draft = None;
                // Back to what was saved.
                Prefs::load().apply_style();
            }
            None => {}
        }
    }
}

/// The Style tab: look-and-feel choices, kept in this browser.
fn style_tab(ui: &mut egui::Ui, d: &mut Prefs) {
    house::note(ui, "How Starbase looks in this browser. Changes show straight away; Cancel puts them back.");
    house::gap(ui);
    house::label(ui, "Toolbar background", "The rounded bar of buttons under the menus.");
    let now = d.toolbar.map(rgb).unwrap_or(crate::theme::TOOLBAR);
    let shown = match d.toolbar {
        Some(_) => house::hex(now),
        None => "Default (pale blue)".to_string(),
    };
    let field = egui::Button::new(egui::RichText::new(format!("      {shown}")).size(house::BODY).color(crate::theme::TEXT))
        .right_text(egui::RichText::new("▾").size(house::BODY).color(crate::theme::TEXT_DIM))
        .fill(egui::Color32::WHITE)
        .stroke(egui::Stroke::new(1.0, crate::theme::BOX))
        .corner_radius(egui::CornerRadius::same(house::FIELD_RADIUS))
        .min_size(egui::vec2(240.0, house::FIELD_HEIGHT));
    let r = ui.add(field);
    let swatch = egui::Rect::from_center_size(r.rect.left_center() + egui::vec2(house::M + 9.0, 0.0), egui::vec2(18.0, 18.0));
    ui.painter().rect_filled(swatch, 4, now);
    ui.painter().rect_stroke(swatch, 4, egui::Stroke::new(1.0, crate::theme::DIVIDER), egui::StrokeKind::Inside);
    egui::Popup::menu(&r).id(egui::Id::unique("set_toolbar_colour")).show(|ui| match crate::colourpicker::palette(ui, d.toolbar, false, false) {
        Some(crate::colourpicker::Choice::Colour(c)) => d.toolbar = Some(c),
        Some(crate::colourpicker::Choice::Reset) => d.toolbar = None,
        _ => {}
    });
}

/// The General tab: locale and default formats.
fn general_tab(ui: &mut egui::Ui, d: &mut Prefs) {
    house::note(ui, "These apply to everything you open in this browser.");
    house::gap(ui);
    house::label(ui, "Locale", "How typed dates are read (05/08/2026 is 5 August in the UK, May 8 in the US), and the formats below.");
    let before = d.locale;
    let locales: Vec<&str> = LOCALES.iter().map(|l| l.0).collect();
    house::select(ui, "set_locale", ui.available_width(), &mut d.locale, &locales, &[]);
    if d.locale != before {
        let (_, _, date, time, _) = LOCALES[d.locale];
        d.date = date.into();
        d.time = time.into();
        d.separators = LOCALE_SEPARATORS.get(d.locale).copied().unwrap_or(0);
    }
    house::gap(ui);
    ui.columns(3, |cols| {
        for c in cols.iter_mut() {
            house::form_style(c);
        }
        let w = cols[0].available_width();
        // Each list shows the sample date, time or number in that format.
        let pick = |ui: &mut egui::Ui, id: &str, label: &str, codes: &[&str], now: &mut String, shown: &dyn Fn(&str) -> String| {
            house::label(ui, label, "");
            let labels: Vec<String> = codes.iter().map(|c| shown(c)).collect();
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let mut k = codes.iter().position(|c| *c == now.as_str()).unwrap_or(0);
            if house::select(ui, id, w, &mut k, &refs, &[]) {
                *now = codes[k].to_string();
            }
        };
        pick(&mut cols[0], "set_date", "Date", DATE_FORMATS, &mut d.date, &sample);
        pick(&mut cols[1], "set_time", "Time", TIME_FORMATS, &mut d.time, &sample);
        let codes: Vec<&str> = NUMBER_FORMATS.iter().map(|(_, c)| *c).collect();
        pick(&mut cols[2], "set_number", "Numbers", &codes, &mut d.number, &|c: &str| {
            NUMBER_FORMATS.iter().find(|(_, x)| *x == c).map_or("Automatic", |(l, _)| l).to_string()
        });
    });
    house::note(ui, "New dates, times and numbers you type get these formats; cells already formatted keep theirs.");
    house::gap(ui);
    house::label(ui, "Numbers are written", "The decimal mark and thousands separator in cells (formulas keep 1.5 and commas).");
    let labels: Vec<&str> = SEPARATORS.iter().map(|x| x.0).collect();
    house::select(ui, "set_separators", ui.available_width(), &mut d.separators, &labels, &[]);
}

#[cfg(test)]
mod tests {
    use gridcraft_engine::core::date::DateSystem;
    use gridcraft_engine::core::parse::{parse_input, set_input_locale};

    #[test]
    fn a_day_first_locale_reads_dates_day_first() {
        set_input_locale(true, Some("dd/mm/yyyy"), Some("hh:mm:ss"), Some("dd/mm/yyyy hh:mm:ss"), None);
        let p = parse_input("05/08/1930", DateSystem::default());
        // 5 August 1930 is serial 11175 (8 May would be 11086).
        assert_eq!(p.value.as_f64(), Some(11_175.0));
        assert_eq!(p.format, Some("dd/mm/yyyy"));
        let t = parse_input("15:59", DateSystem::default());
        assert_eq!(t.format, Some("hh:mm:ss"));
        set_input_locale(false, None, None, None, None);
        assert_eq!(parse_input("05/08/1930", DateSystem::default()).value.as_f64(), Some(11_086.0));
    }

    #[test]
    fn a_decimal_comma_reads_and_shows_numbers_the_local_way() {
        use gridcraft_engine::core::parse::{localize_number_text, set_number_separators};
        set_number_separators(',', '.');
        assert_eq!(parse_input("1.234,5", DateSystem::default()).value.as_f64(), Some(1234.5));
        assert_eq!(parse_input("2,25", DateSystem::default()).value.as_f64(), Some(2.25));
        assert_eq!(parse_input("1,5E+20", DateSystem::default()).value.as_f64(), Some(1.5e20));
        assert_eq!(localize_number_text("1,234.50"), "1.234,50");
        set_number_separators('.', ',');
        assert_eq!(localize_number_text("1,234.50"), "1,234.50");
    }
}
