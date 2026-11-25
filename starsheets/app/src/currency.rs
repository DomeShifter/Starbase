//! Format › Number › Custom currency, after Google Sheets: a search box, the list of currencies
//! with an example of each, and a choice of how the amount is written (symbol first or last,
//! with or without pence). Apply sets the selection's number format.

use egui::{RichText, Stroke, vec2};
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// Asks for the window (the menus set it in egui's memory).
pub const REQUEST: &str = "starsheets_custom_currency";
const SAMPLE: f64 = 1_235.0;
const WIDTH: f32 = 470.0;

/// Name, ISO code and symbol.
pub const CURRENCIES: &[(&str, &str, &str)] = &[
    ("British pound", "GBP", "£"),
    ("Euro", "EUR", "€"),
    ("US dollar", "USD", "$"),
    ("Japanese yen", "JPY", "¥"),
    ("Chinese yuan", "CNY", "CN¥"),
    ("Swiss franc", "CHF", "CHF"),
    ("Canadian dollar", "CAD", "CA$"),
    ("Australian dollar", "AUD", "A$"),
    ("New Zealand dollar", "NZD", "NZ$"),
    ("Indian rupee", "INR", "₹"),
    ("South Korean won", "KRW", "₩"),
    ("Swedish krona", "SEK", "kr"),
    ("Norwegian krone", "NOK", "kr"),
    ("Danish krone", "DKK", "kr."),
    ("Polish złoty", "PLN", "zł"),
    ("Czech koruna", "CZK", "Kč"),
    ("Hungarian forint", "HUF", "Ft"),
    ("Russian rouble", "RUB", "₽"),
    ("Ukrainian hryvnia", "UAH", "₴"),
    ("Turkish lira", "TRY", "₺"),
    ("Israeli new shekel", "ILS", "₪"),
    ("South African rand", "ZAR", "R"),
    ("Nigerian naira", "NGN", "₦"),
    ("Brazilian real", "BRL", "R$"),
    ("Mexican peso", "MXN", "MX$"),
    ("Singapore dollar", "SGD", "S$"),
    ("Hong Kong dollar", "HKD", "HK$"),
    ("Thai baht", "THB", "฿"),
    ("Philippine peso", "PHP", "₱"),
    ("Vietnamese dong", "VND", "₫"),
    ("Bitcoin", "BTC", "₿"),
];

/// The ways an amount can be written, as format codes for `sym` (quoted, so any symbol works).
pub fn codes(sym: &str, iso: &str) -> [String; 5] {
    [
        format!("\"{sym}\"#,##0.00"),
        format!("\"{sym}\"#,##0"),
        format!("#,##0.00\" {sym}\""),
        format!("\"{sym}\"#,##0.00;(\"{sym}\"#,##0.00)"),
        format!("#,##0.00\" {iso}\""),
    ]
}

#[derive(Default)]
pub struct CustomCurrency {
    open: bool,
    search: String,
    picked: usize,
    style: usize,
}

impl CustomCurrency {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_some() {
            *self = Self { open: true, ..Self::default() };
        }
        if !self.open {
            return;
        }
        let mut closed = false;
        let mut quit = false;
        let mut apply = false;
        house::dialog(ctx, "starsheets_custom_currency", WIDTH, "Custom currencies", Some(&mut closed), |ui| {
            ui.add(house::field(&mut self.search, house::BODY).hint_text("Search").desired_width(WIDTH));
            ui.add_space(house::S);
            let needle = self.search.trim().to_lowercase();
            ui.visuals_mut().widgets.inactive.bg_fill = house::SCROLL_HANDLE;
            egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, false]).show(ui, |ui| {
                crate::shell::menu_style_width(ui, ui.available_width());
                let mut any = false;
                for (i, (name, iso, sym)) in CURRENCIES.iter().enumerate() {
                    if !needle.is_empty() && ![*name, *iso, *sym].iter().any(|t| t.to_lowercase().contains(&needle)) {
                        continue;
                    }
                    any = true;
                    let on = i == self.picked;
                    let r = ui.add(
                        egui::Button::new(RichText::new(format!("{name} ({iso})")).size(house::BODY).color(theme::TEXT))
                            .shortcut_text(RichText::new(sample(&codes(sym, iso)[0])).size(house::NOTE).color(theme::TEXT_DIM))
                            .fill(if on { house::SELECTED } else { egui::Color32::TRANSPARENT })
                            .frame_when_inactive(on)
                            .min_size(vec2(ui.available_width(), house::ROW_HEIGHT)),
                    );
                    if r.clicked() {
                        self.picked = i;
                    }
                    if r.double_clicked() {
                        apply = true;
                    }
                    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, theme::DIVIDER));
                }
                if !any {
                    house::note(ui, "No currencies match your search.");
                }
            });
            house::gap(ui);
            let (_, iso, sym) = CURRENCIES[self.picked];
            let samples: Vec<String> = codes(sym, iso).iter().map(|c| sample(c)).collect();
            let labels: Vec<&str> = samples.iter().map(String::as_str).collect();
            house::label(ui, "Format", "");
            house::select(ui, "starsheets_currency_style", WIDTH, &mut self.style, &labels, &[]);
            match house::actions(ui, &["Cancel", "Apply"]) {
                Some(0) => quit = true,
                Some(_) => apply = true,
                None => {}
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                quit = true;
            }
        });
        if apply {
            let (_, iso, sym) = CURRENCIES[self.picked];
            let code = codes(sym, iso)[self.style.min(4)].clone();
            app.run_or_alert("home.numberFormat", json!({ "code": code }));
            self.open = false;
        } else if closed || quit {
            self.open = false;
        }
    }
}

/// The example amount in format `code`.
fn sample(code: &str) -> String {
    let v = gridcraft_engine::core::Value::number(SAMPLE);
    gridcraft_engine::numfmt::text_function(&v, code, gridcraft_engine::core::date::DateSystem::default()).unwrap_or_else(|_| code.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_currency_format_shows_its_symbol() {
        for (_, iso, sym) in CURRENCIES {
            for code in codes(sym, iso) {
                let s = sample(&code);
                assert!(s.contains(sym) || s.contains(iso), "{code} gave {s}");
                assert!(s.contains("1,235"), "{code} gave {s}");
            }
        }
        assert_eq!(sample(&codes("€", "EUR")[0]), "€1,235.00");
        assert_eq!(sample(&codes("€", "EUR")[2]), "1,235.00 €");
    }

    #[test]
    fn a_currency_format_applies_to_a_cell() {
        let mut s = gridcraft_engine::Session::new();
        s.execute("file.new", json!({})).unwrap();
        s.execute("cell.set", json!({"cell": "A1", "input": "1234.5"})).unwrap();
        s.execute("home.numberFormat", json!({"code": codes("€", "EUR")[0]})).unwrap();
        let d = s.active().unwrap();
        let sh = d.wb.active().unwrap();
        let code = d.wb.styles.get(sh.style_id(gridcraft_engine::core::CellRef::new(0, 0))).num_fmt.as_str().to_string();
        assert!(code.contains('€') && code.contains("#,##0.00"), "{code}");
    }
}
