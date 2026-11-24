//! Spell check while typing in a cell (Google underlines unknown words in red, and only while you
//! edit). Capitalised words count as names and are never marked. The British English word list (SCOWL, dictionary/en-GB.txt, ~300 KB compressed) is
//! fetched the first time a cell is edited; until it arrives nothing is underlined.
//!
//! Clicking on an underlined word while editing opens suggestions under the click (`Suggest`):
//! up to five words from the same list that are one or two letters away, and "Add to dictionary"
//! (your own words, kept in this browser). Browser checkers such as Harper can't work inside
//! cells: StarSheets draws its cells itself (on a canvas), so there is no text box for them to read.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

static WORDS: OnceLock<HashSet<String>> = OnceLock::new();
static ASKED: AtomicBool = AtomicBool::new(false);
/// Words added with "Add to dictionary" (this browser's localStorage, `starsheets_words`).
static MINE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn mine_has(w: &str) -> bool {
    let mut g = MINE.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(|| stored_words().into_iter().collect());
    set.contains(&w.to_lowercase())
}

fn stored_words() -> Vec<String> {
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(t) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()).and_then(|s| s.get_item("starsheets_words").ok().flatten())
        {
            return t.lines().map(str::to_string).collect();
        }
    }
    Vec::new()
}

/// Adds a word to your own dictionary (this browser).
pub fn add_word(word: &str) {
    let w = word.to_lowercase();
    let mut g = MINE.lock().unwrap_or_else(|e| e.into_inner());
    let set = g.get_or_insert_with(|| stored_words().into_iter().collect());
    set.insert(w);
    #[cfg(target_arch = "wasm32")]
    {
        let mut all: Vec<&String> = set.iter().collect();
        all.sort();
        if let Some(s) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
            let _ = s.set_item("starsheets_words", &all.into_iter().cloned().collect::<Vec<_>>().join("\n"));
        }
    }
}

/// Whether `word` is a word (true while the list hasn't loaded, so nothing is marked).
pub fn known(word: &str) -> bool {
    let Some(words) = WORDS.get() else { return true };
    let w = word.trim_matches('\'');
    let w = w.strip_suffix("'s").or_else(|| w.strip_suffix("’s")).unwrap_or(w);
    // Capitalised words are taken as names (London, Priya): the list has no proper nouns, and
    // spreadsheets are full of them. That covers abbreviations (UK, NHS) too.
    if w.chars().next().is_some_and(char::is_uppercase) {
        return true;
    }
    words.contains(w) || words.contains(&w.to_lowercase()) || mine_has(w)
}

/// Letters apart (insert, delete, change, or swap two next to each other), stopping past `max`.
fn distance(a: &[char], b: &[char], max: usize) -> usize {
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate().take(m + 1) {
        *cell = j;
    }
    for i in 1..=n {
        let mut best = usize::MAX;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
            best = best.min(v);
        }
        if best > max {
            return max + 1;
        }
    }
    d[n][m]
}

/// Whether `typed` is `word` with letters left out (its letters appear in `word`, in order).
fn missing_letters(typed: &[char], word: &[char]) -> bool {
    let mut it = word.iter();
    word.len() > typed.len() && typed.iter().all(|c| it.any(|w| w == c))
}

/// Whether `typed` is `word` with two letters next to each other swapped ("teh").
fn swapped(typed: &[char], word: &[char]) -> bool {
    typed.len() == word.len() && {
        let diff: Vec<usize> = (0..typed.len()).filter(|&i| typed[i] != word[i]).collect();
        diff.len() == 2 && diff[1] == diff[0] + 1 && typed[diff[0]] == word[diff[1]] && typed[diff[1]] == word[diff[0]]
    }
}

/// Up to five known words close to `word`, the likeliest typos first (letters left out, two
/// letters swapped), then by how many letters differ, same first letter, similar length. In the
/// word's own case pattern.
/// A suggestion's sort key, in the order `suggestions` explains, then the word itself.
type Ranked<'a> = (bool, usize, usize, std::cmp::Reverse<usize>, usize, &'a String);

pub fn suggestions(word: &str) -> Vec<String> {
    let Some(words) = WORDS.get() else { return Vec::new() };
    let w: Vec<char> = word.to_lowercase().chars().collect();
    // Ranked: likely slips first (left-out letters, two swapped), then fewest changes, then the
    // fewest extra consonants (vowels are what people leave out: "colr" → "colour" before
    // "colder"), then the longest shared start ("choler" last), then nearest length.
    let consonants = |v: &[char]| v.iter().filter(|c| c.is_alphabetic() && !"aeiouy".contains(**c)).count();
    let mine = consonants(&w);
    let mut found: Vec<Ranked> = words
        .iter()
        .filter_map(|c| {
            let cc: Vec<char> = c.chars().collect();
            let d = distance(&w, &cc, 2);
            let likely = missing_letters(&w, &cc) || swapped(&w, &cc);
            let shared = w.iter().zip(&cc).take_while(|(a, b)| a == b).count();
            (d <= 2 && (shared > 0 || likely))
                .then(|| (!likely, d, consonants(&cc).abs_diff(mine), std::cmp::Reverse(shared), cc.len().abs_diff(w.len()), c))
        })
        .collect();
    found.sort();
    let upper_first = word.chars().next().is_some_and(char::is_uppercase);
    found
        .into_iter()
        .take(5)
        .map(|(_, _, _, _, _, s)| {
            if upper_first {
                let mut c = s.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            } else {
                s.clone()
            }
        })
        .collect()
}

/// The word around char position `at` in `text`: (start, end) in chars, if `at` is in or just
/// after a word.
pub fn word_at(text: &str, at: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_alphabetic() || c == '\'' || c == '’';
    let mut start = at.min(chars.len());
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = at.min(chars.len());
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    (end > start).then_some((start, end))
}

/// Fills the list from the file's text.
fn load(text: &str) {
    let _ = WORDS.set(text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect());
}

/// Call each frame: fetches the list once, when a cell is first edited.
pub fn logic(app: &gridcraft_ui_egui::SheetApp, ctx: &egui::Context) {
    if app.editor.is_none() || ASKED.swap(true, Ordering::Relaxed) {
        return;
    }
    #[cfg(target_arch = "wasm32")]
    {
        let ctx = ctx.clone();
        wasm_bindgen_futures::spawn_local(async move {
            if let Ok((200, _, bytes)) = crate::files::get_bytes("dictionary/en-GB.txt").await {
                load(&String::from_utf8_lossy(&bytes));
                ctx.request_repaint();
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = ctx;
}

/// Suggestions for a misspelt word clicked while editing: where to show them, which letters of
/// the cell's text they replace, and the words.
#[derive(Default)]
pub struct Suggest {
    open: Option<(egui::Pos2, usize, usize, String, Vec<String>)>,
    /// Where the suggestions were drawn last frame (a click there is a pick, not a new word).
    rect: Option<egui::Rect>,
    /// Esc was pressed while the suggestions showed (taken before the grid sees it).
    escape: bool,
}

impl Suggest {
    /// Call before the grid: Esc closes the suggestions only, it must not cancel the cell.
    pub fn before(&mut self, ctx: &egui::Context) {
        if self.open.is_some() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.escape = true;
        }
    }

    /// Call after the grid (it has moved the caret to where you clicked).
    pub fn ui(&mut self, app: &mut gridcraft_ui_egui::SheetApp, ctx: &egui::Context) {
        let Some(e) = app.editor.as_ref() else {
            self.open = None;
            return;
        };
        let clicked = ctx.input(|i| i.pointer.primary_clicked());
        // A click in the cell being edited, on an underlined word: offer suggestions.
        let on_box = ctx.input(|i| i.pointer.interact_pos()).zip(self.rect).is_some_and(|(p, r)| r.contains(p));
        if clicked && !on_box && !e.text.trim_start().starts_with('=') {
            self.open = None;
            if let (Some((a, b)), Some(pos)) = (word_at(&e.text, e.caret), ctx.input(|i| i.pointer.interact_pos())) {
                let word: String = e.text.chars().skip(a).take(b - a).collect();
                if word.chars().count() > 1 && !known(&word) {
                    self.open = Some((pos, a, b, word.clone(), suggestions(&word)));
                }
            }
        }
        let Some((pos, a, b, word, list)) = self.open.clone() else { return };
        let mut pick: Option<String> = None;
        let mut add = false;
        let mut close = false;
        let shown = egui::Area::new(egui::Id::unique("starsheets_spelling"))
            .fixed_pos(pos + egui::vec2(0.0, 14.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(0, crate::house::S as i8)).show(ui, |ui| {
                    crate::house::menu(ui, 220.0);
                    if list.is_empty() {
                        crate::house::menu_inset(ui, |ui| crate::house::note(ui, "No suggestions"));
                    }
                    for s in &list {
                        if crate::house::menu_row(ui, None, s, "", true) {
                            pick = Some(s.clone());
                        }
                    }
                    crate::house::menu_sep(ui);
                    if crate::house::menu_row(ui, None, &format!("Add \u{201c}{word}\u{201d} to dictionary"), "", true) {
                        add = true;
                    }
                    if crate::house::menu_row(ui, None, "Ignore", "Esc", true) {
                        close = true;
                    }
                });
            });
        self.rect = Some(shown.response.rect);
        if std::mem::take(&mut self.escape) {
            close = true;
        }
        if let Some(s) = pick
            && let Some(e) = app.editor.as_mut()
        {
            let chars: Vec<char> = e.text.chars().collect();
            if b <= chars.len() {
                let new: String = chars[..a].iter().collect::<String>() + &s + &chars[b..].iter().collect::<String>();
                e.text = new;
                e.caret = a + s.chars().count();
                e.anchor = e.caret;
            }
            close = true;
        }
        if add {
            add_word(&word);
            close = true;
        }
        if close {
            self.open = None;
            self.rect = None;
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn words_are_checked_without_case_or_possessives() {
        super::load("colour\nlondon\nthe\ncat\ncoir\ncola\ncold\ncole\ncoli\ncolt\ncholer\ncolder\ncollar\n");
        assert!(super::known("colour") && super::known("cat's") && super::known("NHS") && super::known("Priya"));
        assert!(!super::known("colr"));
        assert_eq!(super::suggestions("colr").first().map(String::as_str), Some("colour"));
        assert_eq!(super::suggestions("teh").first().map(String::as_str), Some("the"), "swapped letters");
        assert_eq!(super::suggestions("hte").first().map(String::as_str), Some("the"), "swapped first letters");
        assert_eq!(super::word_at("a colr here", 5), Some((2, 6)));
    }
}
