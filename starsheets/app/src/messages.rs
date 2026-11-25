//! GridCraft's alerts ("GridCraft: The cell or chart you're trying to change…") shown as house
//! dialogs with StarSheets' wording, one at a time. GridCraft's questions ("There's already data
//! here. Do you want to replace it?") too: its own prompt is off (`Chrome::questions`).

use gridcraft_ui_egui::SheetApp;

use crate::house;

#[derive(Default)]
pub struct Messages {
    showing: Option<(String, String)>,
    /// A formula that can't be entered, while its cell is being edited: Sheets' message with
    /// "Add '" (enter it as text) and "Keep working…".
    formula: Option<String>,
    /// A GridCraft question was drawn as a house dialog (see `wake`).
    question_shown: bool,
}

/// Sheets' message for a formula that can't be entered.
pub fn formula_message(formula: &str) -> String {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut extra_close = false;
    for ch in formula.chars() {
        match ch {
            '"' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => {
                depth -= 1;
                extra_close |= depth < 0;
                depth = depth.max(0);
            }
            _ => {}
        }
    }
    let first = if extra_close {
        "It looks as though your formula is missing one or more opening brackets."
    } else if depth > 0 {
        "It looks as though your formula is missing one or more closing brackets."
    } else if in_string {
        "It looks as though your formula is missing a quotation mark."
    } else {
        "There's a problem with this formula."
    };
    format!("{first} If you don't want to enter a formula, begin your text with an apostrophe (').")
}

/// What's wrong with a formula that can't be read, in plain words: the bracket or quotation
/// mark that doesn't match, with how to fix it.
pub fn formula_hint(formula: &str) -> String {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut extra_close = false;
    for ch in formula.chars() {
        match ch {
            '"' => in_string = !in_string,
            '(' if !in_string => depth += 1,
            ')' if !in_string => {
                depth -= 1;
                if depth < 0 {
                    extra_close = true;
                    depth = 0;
                }
            }
            _ => {}
        }
    }
    let mut hint = if in_string {
        "A quotation mark is missing: text in a formula goes between two \" marks.".to_string()
    } else if extra_close {
        "It has a closing bracket \")\" with no opening bracket to match it: remove one \")\".".to_string()
    } else if depth > 0 {
        format!("A bracket is opened but not closed: add {} at the end.", "\")\"".repeat(depth as usize))
    } else {
        "Check its brackets, commas and quotation marks.".to_string()
    };
    let upper = formula.to_ascii_uppercase();
    if (upper.contains("ISEVEN(") || upper.contains("ISODD(")) && upper.contains(',') {
        hint.push_str(" ISEVEN and ISODD take just one value: for every other row, use =ISEVEN(ROW()).");
    }
    hint
}

/// GridCraft's "invalid … formula `X` — it does not parse; Excel would reject the saved file"
/// in plain words (StarSheets doesn't talk about Excel).
pub fn plain_error(text: &str) -> String {
    if text.contains("does not parse") {
        // Between "formula " and " — it does not parse", without its quotes (` or ").
        let start = text.find("formula ").map_or(0, |i| i + "formula ".len());
        let end = text.find("it does not parse").unwrap_or(text.len()).max(start);
        let formula = text[start..end].trim().trim_end_matches('—').trim().trim_matches(|c| c == '`' || c == '"');
        return format!("Starbase Sheets can't read the formula ={}. {}", formula.trim_start_matches('='), formula_hint(formula));
    }
    text.replace("GridCraft", "Starbase Sheets").replace("; Excel would reject the saved file", "")
}

/// Sheets' words for the messages people meet most.
fn reword(title: &str, text: &str) -> (String, String) {
    // Menu Paste with nothing StarSheets knows of (after a reload, or copied in another app).
    if text.trim_end_matches('.') == "Nothing to paste" {
        return ("Copying and pasting in this browser".into(), crate::editmenu::PASTE_NOTE.into());
    }
    let title = if title == "GridCraft" || title.is_empty() { "Heads up".to_string() } else { title.to_string() };
    let text = if text.contains("protected sheet") {
        "You're trying to edit a protected cell. To change it, remove its protection in Data › Protect sheets and ranges.".to_string()
    } else {
        plain_error(text)
    };
    (title, text)
}

impl Messages {
    pub fn ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        if self.showing.is_none()
            && self.formula.is_none()
            && let Some((t, m)) = app.message.take()
        {
            // While a formula is being typed, a message is about that formula.
            match app.editor.as_ref().filter(|e| e.text.trim_start().starts_with('=') && !m.contains("protect")) {
                Some(e) => self.formula = Some(e.text.clone()),
                None => self.showing = Some(reword(&t, &m)),
            }
        }
        if let Some(formula) = self.formula.clone() {
            let mut choice = None;
            let mut closed = false;
            house::dialog(ctx, "starsheets_formula_problem", 440.0, "There's a problem with this formula", Some(&mut closed), |ui| {
                house::text(ui, &formula_message(&formula));
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    choice = Some(1);
                }
                choice = choice.or(house::actions(ui, &["Add \"'\"", "Keep working…"]));
            });
            match (closed, choice) {
                (_, Some(0)) => {
                    // Entered as text, exactly as typed.
                    self.formula = None;
                    if let Some(e) = app.editor.as_mut() {
                        e.text = format!("'{}", e.text);
                    }
                    app.commit_edit(1, 0, false, false);
                }
                (true, _) | (_, Some(_)) => {
                    self.formula = None;
                    if let Some(e) = app.editor.as_mut() {
                        e.request_focus = true;
                    }
                }
                _ => {}
            }
            return;
        }
        if self.showing.is_none() {
            self.question_ui(app, ctx);
        }
        let Some((title, text)) = self.showing.clone() else { return };
        let mut closed = false;
        let mut ok = false;
        house::dialog(ctx, "starsheets_message", 420.0, &title, Some(&mut closed), |ui| {
            house::text(ui, &text);
            ok = house::actions(ui, &["OK"]).is_some() || ui.input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape));
        });
        if closed || ok {
            self.showing = None;
        }
    }
}

impl Messages {
    /// After the grid: a question it asked this frame shows on the next one (this runs before
    /// GridCraft draws), so ask for that frame.
    pub fn wake(&mut self, app: &SheetApp, ctx: &egui::Context) {
        if app.macros.question.is_none() {
            self.question_shown = false;
        } else if !self.question_shown {
            ctx.request_repaint();
        }
    }

    /// GridCraft's question (replace data, run a macro…) with its choices and Cancel. Enter picks
    /// the first choice, Esc cancels.
    fn question_ui(&mut self, app: &mut SheetApp, ctx: &egui::Context) {
        let Some(q) = app.macros.question.clone() else {
            self.question_shown = false;
            return;
        };
        self.question_shown = true;
        let title = if q.title.is_empty() || q.title == "GridCraft" {
            "Heads up".to_string()
        } else {
            q.title.replace("GridCraft", "Starbase Sheets")
        };
        let mut labels = vec!["Cancel"];
        labels.extend(q.choices.iter().map(|(l, _)| l.as_str()));
        let mut closed = false;
        let mut pick = None;
        house::dialog(ctx, "starsheets_question", 420.0, &title, Some(&mut closed), |ui| {
            house::text(ui, &plain_error(&q.text));
            pick = house::actions(ui, &labels);
            // Consumed, so the grid (drawn after this) doesn't also take the key.
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                pick = Some(0);
            } else if labels.len() > 1 && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                pick = Some(1);
            }
        });
        match (closed, pick) {
            (true, _) | (_, Some(0)) => gridcraft_ui_egui::macro_ui::answer(app, None),
            (_, Some(k)) => gridcraft_ui_egui::macro_ui::answer(app, Some(k - 1)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn formula_problems_in_plain_words() {
        let m =
            super::plain_error("Bad parameter: invalid rule formula `ISEVEN(ROW(),TRUE))` — it does not parse; Excel would reject the saved file");
        assert!(
            m.contains("=ISEVEN(ROW(),TRUE))") && m.contains("no opening bracket") && m.contains("=ISEVEN(ROW())") && !m.contains("Excel"),
            "{m}"
        );
        let m = super::plain_error("invalid rule formula \"ISEVEN(ROW(), TRUE))\" — it does not parse; Excel would reject the saved file");
        assert!(m.starts_with("Starbase Sheets can't read the formula =ISEVEN(ROW(), TRUE)). "), "{m}");
        assert!(super::formula_hint("SUM(A1:A3").contains("add \")\""));
        assert!(super::formula_hint("IF(A1=\"x,1)").contains("quotation mark"));
    }

    #[test]
    fn sheets_words_for_a_broken_formula() {
        assert!(super::formula_message("=\"THIS\")").starts_with("It looks as though your formula is missing one or more opening brackets."));
        assert!(super::formula_message("=SUM(A1").contains("closing brackets"));
        assert!(super::formula_message("=1+").starts_with("There's a problem"));
    }

    #[test]
    fn gridcraft_messages_are_reworded() {
        let (t, m) = super::reword("GridCraft", "The cell or chart you're trying to change is on a protected sheet.");
        assert_eq!(t, "Heads up");
        assert!(m.contains("Protect sheets and ranges"));
        assert_eq!(super::reword("Version history", "x").0, "Version history");
    }
}
