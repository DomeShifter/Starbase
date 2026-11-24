//! Format › Number › Custom date and time, after Google Sheets: a bar of parts (Day, Month,
//! Year, Hour…) with the separators between them, each part clickable for its kinds (5 or 05,
//! Aug or August…), ▾ to add a part, a list of ready-made formats, and Apply. The selection
//! shows the format while you build it; closing the window takes it back.

use egui::{RichText, Stroke, vec2};
use gridcraft_engine::core::Value;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

use crate::{house, theme};

/// Asks for the window (the menu sets it in egui's memory).
pub const REQUEST: &str = "starsheets_custom_date";
/// Google's sample: 5 August 1930, 15:59:00.
const SAMPLE: f64 = 11_175.665_972_222_2;

/// A kind of part: name, and its variants (format code, what it looks like).
const KINDS: &[(&str, &[(&str, &str)])] = &[
    ("Day", &[("d", "Day without leading 0 (5)"), ("dd", "Day with leading 0 (05)")]),
    ("Day of the week", &[("ddd", "Short day of the week (Tue)"), ("dddd", "Full day of the week (Tuesday)")]),
    (
        "Month",
        &[
            ("m", "Month as number without leading 0 (8)"),
            ("mm", "Month as number with leading 0 (08)"),
            ("mmm", "Short month (Aug)"),
            ("mmmm", "Full month (August)"),
            ("mmmmm", "First letter of the month (A)"),
        ],
    ),
    ("Year", &[("yy", "2-digit year (30)"), ("yyyy", "Full year (1930)")]),
    ("Hour", &[("h", "Hour without leading 0 (15)"), ("hh", "Hour with leading 0 (15)"), ("[h]", "Elapsed hours")]),
    ("Minute", &[("m", "Minute without leading 0 (59)"), ("mm", "Minute with leading 0 (59)")]),
    ("Second", &[("s", "Second without leading 0 (0)"), ("ss", "Second with leading 0 (00)")]),
    ("AM/PM", &[("AM/PM", "AM/PM (PM)"), ("A/P", "A/P (P)")]),
];

/// Google's ready-made formats.
const PRESETS: &[&str] = &[
    "d-mmm-yyyy",
    "d mmm yyyy",
    "d mmmm yyyy",
    "dd/mm/yy",
    "dd/mm",
    "dd-mm-yyyy",
    "dd-mm-yy",
    "yyyy-mm-dd",
    "mmmm yyyy",
    "dddd, d mmmm yyyy",
    "hh:mm:ss",
    "h:mm:ss AM/PM",
    "hh:mm",
    "h:mm AM/PM",
    "dd/mm/yyyy hh:mm:ss",
    "d mmmm yyyy \"at\" hh:mm",
];

#[derive(Clone, Debug, PartialEq)]
enum Part {
    /// (kind index into KINDS, variant index).
    Token(usize, usize),
    Sep(String),
}

#[derive(Default)]
pub struct CustomDate {
    parts: Option<Vec<Part>>,
    /// The live preview: (code applied, undo steps).
    preview: Option<(String, usize)>,
}

impl CustomDate {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if ctx.data_mut(|m| m.remove_temp::<bool>(egui::Id::unique(REQUEST))).is_some() {
            let current = app.session.active().and_then(|d| {
                let sh = d.wb.active()?;
                let code = d.wb.styles.get(sh.style_id(d.selection.active)).num_fmt.as_str().to_string();
                (code.contains('d') || code.contains('y') || code.contains('h')).then_some(code)
            });
            let start = current.unwrap_or_else(|| crate::settings::default_codes().0);
            self.parts = Some(parse(&start));
            self.preview = None;
        }
        let Some(parts) = self.parts.as_mut() else { return };
        let mut done = None;
        let mut closed = false;
        house::dialog(ctx, "starsheets_custom_date", 470.0, "Custom date and time formats", Some(&mut closed), |ui| {
            // The builder bar.
            ui.horizontal(|ui| {
                let bar = ui.available_width() - 36.0 - 88.0 - 2.0 * house::S - 2.0 * house::S;
                egui::Frame::NONE
                    .stroke(Stroke::new(1.0, theme::BOX))
                    .corner_radius(f32::from(house::FIELD_RADIUS))
                    .inner_margin(egui::Margin::same(house::S as i8))
                    .show(ui, |ui| {
                        ui.set_width(bar);
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(2.0, 4.0);
                            let mut remove = None;
                            for (i, p) in parts.iter_mut().enumerate() {
                                match p {
                                    Part::Token(k, v) => {
                                        let (kind, variants) = KINDS[*k];
                                        let label = format!("{kind} ({})", sample(variants[*v].0, *k));
                                        let chip = ui.add(
                                            egui::Button::new(RichText::new(label).size(13.0))
                                                .fill(house::SELECTED)
                                                .stroke(Stroke::NONE)
                                                .corner_radius(f32::from(house::FIELD_RADIUS)),
                                        );
                                        egui::Popup::menu(&chip).id(egui::Id::unique(("date_part", i))).show(|ui| {
                                            let width = crate::shell::menu_style(ui, false);
                                            for (vi, (_, text)) in variants.iter().enumerate() {
                                                let mark = if *v == vi { "✓  " } else { "     " };
                                                if ui.add(crate::shell::menu_row(&format!("{mark}{text}"), "", false)).clicked() {
                                                    *v = vi;
                                                    ui.close();
                                                }
                                            }
                                            crate::shell::menu_sep_width(ui, width);
                                            if ui.add(crate::shell::menu_row("Delete", "", false)).clicked() {
                                                remove = Some(i);
                                                ui.close();
                                            }
                                        });
                                    }
                                    Part::Sep(s) => {
                                        let w = (s.chars().count().max(1) as f32) * 8.0 + 6.0;
                                        ui.add(egui::TextEdit::singleline(s).desired_width(w).frame(egui::Frame::NONE));
                                    }
                                }
                            }
                            if let Some(i) = remove {
                                parts.remove(i);
                            }
                        });
                    });
                let add = ui.add(egui::Button::new("▾").min_size(vec2(36.0, house::FIELD_HEIGHT))).on_hover_text("Add a part");
                egui::Popup::menu(&add).id(egui::Id::unique("date_add")).show(|ui| {
                    crate::shell::menu_style(ui, false);
                    for (k, (kind, _)) in KINDS.iter().enumerate() {
                        if ui.add(crate::shell::menu_row(kind, "", false)).clicked() {
                            if !parts.is_empty() {
                                parts.push(Part::Sep(" ".into()));
                            }
                            parts.push(Part::Token(k, if k == 3 { 1 } else { 0 }));
                            ui.close();
                        }
                    }
                });
                if ui.add(house::primary("Apply")).clicked() {
                    done = Some(true);
                }
            });
            house::gap(ui);
            // The dialog's fields are white; the list's scroll handle keeps the house grey.
            ui.visuals_mut().widgets.inactive.bg_fill = house::SCROLL_HANDLE;
            egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
                crate::shell::menu_style_width(ui, ui.available_width());
                for code in PRESETS {
                    let r = ui.add(
                        egui::Button::new(RichText::new(sample_code(code)).size(house::BODY).color(theme::TEXT))
                            .shortcut_text("")
                            .frame_when_inactive(false)
                            .min_size(vec2(ui.available_width(), house::ROW_HEIGHT)),
                    );
                    if r.clicked() {
                        *parts = parse(code);
                    }
                    ui.painter().hline(r.rect.x_range(), r.rect.bottom(), Stroke::new(1.0, theme::DIVIDER));
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        if closed {
            done = Some(false);
        }
        let code = to_code(parts);
        // Live preview: the selection shows the format as it is built.
        if self.preview.as_ref().is_none_or(|(c, _)| *c != code) && done.is_none() {
            self.undo_preview(app);
            let ok = app.run("home.numberFormat", json!({ "code": code })).is_ok();
            self.preview = Some((code.clone(), usize::from(ok)));
        }
        match done {
            Some(true) => {
                self.preview = None;
                self.parts = None;
            }
            Some(false) => {
                self.undo_preview(app);
                self.parts = None;
            }
            None => {}
        }
    }

    fn undo_preview(&mut self, app: &mut SheetApp) {
        if let Some((_, n)) = self.preview.take() {
            for _ in 0..n {
                let _ = app.run("edit.undo", json!({}));
            }
        }
    }
}

/// A part's look in the sample date (5 August 1930, 15:59:00).
fn sample(code: &str, kind: usize) -> String {
    // Minutes and months share "m": give a minute code its hour so it reads as minutes.
    let full = if KINDS[kind].0 == "Minute" { format!("h:{code}") } else { code.to_string() };
    let s = sample_code(&full);
    if KINDS[kind].0 == "Minute" { s.split(':').nth(1).unwrap_or("").to_string() } else { s }
}

fn sample_code(code: &str) -> String {
    gridcraft_engine::numfmt::text_function(&Value::number(SAMPLE), code, gridcraft_engine::core::date::DateSystem::default())
        .unwrap_or_else(|_| code.to_string())
}

/// The format code for the parts; separators with letters are quoted.
fn to_code(parts: &[Part]) -> String {
    parts
        .iter()
        .map(|p| match p {
            Part::Token(k, v) => KINDS[*k].1[*v].0.to_string(),
            Part::Sep(s) if s.chars().any(char::is_alphabetic) => {
                // Spaces stay outside the quotes, as Google writes it: `yyyy "at" hh:mm`.
                let core = s.trim();
                let lead = &s[..s.len() - s.trim_start().len()];
                let trail = &s[s.trim_end().len()..];
                format!("{lead}\"{core}\"{trail}")
            }
            Part::Sep(s) => s.clone(),
        })
        .collect()
}

/// A format code as parts (for a preset or the cell's own format).
fn parse(code: &str) -> Vec<Part> {
    let chars: Vec<char> = code.chars().collect();
    let mut out: Vec<Part> = Vec::new();
    let mut sep = String::new();
    let mut i = 0;
    let push_sep = |out: &mut Vec<Part>, sep: &mut String| {
        if !sep.is_empty() {
            out.push(Part::Sep(std::mem::take(sep)));
        }
    };
    let last_hour = |out: &[Part]| out.iter().rev().find_map(|p| if let Part::Token(k, _) = p { Some(*k) } else { None }) == Some(4);
    while i < chars.len() {
        let c = chars[i];
        let rest: String = chars[i..].iter().collect();
        if rest.to_ascii_uppercase().starts_with("AM/PM") || rest.to_ascii_uppercase().starts_with("A/P") {
            push_sep(&mut out, &mut sep);
            let long = rest.to_ascii_uppercase().starts_with("AM/PM");
            out.push(Part::Token(7, usize::from(!long)));
            i += if long { 5 } else { 3 };
            continue;
        }
        if c == '"' {
            let end = chars[i + 1..].iter().position(|&x| x == '"').map_or(chars.len(), |p| i + 1 + p);
            sep.extend(&chars[i + 1..end.min(chars.len())]);
            i = end + 1;
            continue;
        }
        if c == '[' && rest.to_ascii_lowercase().starts_with("[h]") {
            push_sep(&mut out, &mut sep);
            out.push(Part::Token(4, 2));
            i += 3;
            continue;
        }
        let lower = c.to_ascii_lowercase();
        if "dmyhs".contains(lower) {
            let mut n = 0;
            while i + n < chars.len() && chars[i + n].to_ascii_lowercase() == lower {
                n += 1;
            }
            push_sep(&mut out, &mut sep);
            let next_is_s = chars[i + n..].iter().find(|x| x.is_alphabetic()).is_some_and(|x| x.eq_ignore_ascii_case(&'s'));
            let part = match lower {
                'd' if n >= 3 => Part::Token(1, usize::from(n >= 4)),
                'd' => Part::Token(0, usize::from(n >= 2)),
                'y' => Part::Token(3, usize::from(n > 2)),
                'h' => Part::Token(4, usize::from(n >= 2)),
                's' => Part::Token(6, usize::from(n >= 2)),
                // m: minutes after an hour or before seconds, else the month.
                _ if n <= 2 && (last_hour(&out) || next_is_s) => Part::Token(5, usize::from(n >= 2)),
                _ => Part::Token(2, (n.clamp(1, 5)) - 1),
            };
            out.push(part);
            i += n;
            continue;
        }
        sep.push(c);
        i += 1;
    }
    push_sep(&mut out, &mut sep);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip_through_parts() {
        for code in PRESETS {
            assert_eq!(to_code(&parse(code)), *code, "{code}");
        }
        assert_eq!(parse("hh:mm"), vec![Part::Token(4, 1), Part::Sep(":".into()), Part::Token(5, 1)]);
        assert_eq!(parse("mm/dd"), vec![Part::Token(2, 1), Part::Sep("/".into()), Part::Token(0, 1)]);
    }
}
