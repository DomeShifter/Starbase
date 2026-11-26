//! The save-state icon on the title row of Sheets, Docs and Slides (to-do 3 row 38).
//!
//! One look for every app: an ochre floppy while a change is not saved yet or is being saved, a
//! green clock once it is saved, a grey floppy with a small red dot while offline, a red floppy
//! when it could not be saved, and a grey clock when the file is view only. Hovering shows a small
//! card of two or three short lines ([`card_lines`]); a click is left to the app (it opens version
//! history where the app has one).

use egui::{Color32, CornerRadius, Pos2, Rect, Response, RichText, Sense, Stroke, Ui, pos2, vec2};

/// Where saving stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SaveKind {
    /// Changed, and the save hasn't started yet.
    Unsaved,
    /// A save is on its way.
    Saving,
    /// Everything is saved.
    #[default]
    Saved,
    /// Not connected (changes are kept and sent later).
    Offline,
    /// Not saved: an error, or no access any more.
    Problem,
    /// This person may only read the file.
    ViewOnly,
}

impl SaveKind {
    /// The word the apps' test hooks report (`saveIcon`).
    pub fn word(self) -> &'static str {
        match self {
            SaveKind::Unsaved => "unsaved",
            SaveKind::Saving => "saving",
            SaveKind::Saved => "saved",
            SaveKind::Offline => "offline",
            SaveKind::Problem => "problem",
            SaveKind::ViewOnly => "viewonly",
        }
    }
}

/// What the icon and its card show.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SaveInfo {
    pub kind: SaveKind,
    /// When it was last saved, in words ("2 minutes ago"); `None` lets the widget time it from
    /// the moment it saw the save finish.
    pub when: Option<String>,
    /// The latest version's number and who made it ("you" for the signed-in person).
    pub version: Option<(u32, String)>,
    /// A short second line when there is no version line (a reason, "Last edit by …").
    pub detail: String,
    /// A file on this computer: "Saved on this computer".
    pub local: bool,
    /// A click opens version history.
    pub history: bool,
}

/// The colours, passed in by each app from its own theme.
#[derive(Clone, Copy, Debug)]
pub struct SavePalette {
    pub ochre: Color32,
    pub green: Color32,
    pub red: Color32,
    pub grey: Color32,
    /// The background (the red dot's ring).
    pub page: Color32,
}

/// Ochre: not saved yet, or saving.
pub const SAVE_OCHRE: Color32 = Color32::from_rgb(0xC9, 0xA2, 0x27);

impl SavePalette {
    /// The house colours: green and red as Sheets' house style, grey as the given dim text.
    pub fn house(grey: Color32, page: Color32) -> SavePalette {
        SavePalette { ochre: SAVE_OCHRE, green: Color32::from_rgb(0x14, 0x6C, 0x2E), red: Color32::from_rgb(0xB3, 0x26, 0x1E), grey, page }
    }
}

const CLICK: &str = "Click for version history";

/// The card's three lines (an empty line is left out when drawn).
pub fn card_lines(info: &SaveInfo) -> [String; 3] {
    let version = info.version.as_ref().map(|(n, by)| if by.is_empty() { format!("Version {n}") } else { format!("Version {n} · by {by}") });
    let second = || version.clone().unwrap_or_else(|| info.detail.clone());
    let (one, two) = match info.kind {
        SaveKind::Unsaved => ("Not saved yet".to_string(), "Saving in a moment".to_string()),
        SaveKind::Saving => {
            ("Saving…".to_string(), info.version.as_ref().map(|(n, _)| format!("Version {n} is the last one")).unwrap_or_else(|| info.detail.clone()))
        }
        SaveKind::Saved => {
            let one = match (&info.when, info.local) {
                (_, true) => "Saved on this computer".to_string(),
                (Some(w), false) if !w.is_empty() => format!("Saved · {w}"),
                _ => "Saved".to_string(),
            };
            (one, second())
        }
        SaveKind::Offline => {
            ("Offline".to_string(), if info.detail.is_empty() { "Changes are kept and will sync".to_string() } else { info.detail.clone() })
        }
        SaveKind::Problem => ("Not saved".to_string(), if info.detail.is_empty() { "Trying again".to_string() } else { info.detail.clone() }),
        SaveKind::ViewOnly => ("View only".to_string(), second()),
    };
    [one, two, if info.history { CLICK.to_string() } else { String::new() }]
}

/// Seconds as "just now", "a minute ago", "5 min ago", "2 hours ago".
pub fn ago_text(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..=44 => "just now".into(),
        45..=89 => "a minute ago".into(),
        90..=3599 => format!("{} min ago", (s + 30) / 60),
        3600..=5399 => "an hour ago".into(),
        _ => format!("{} hours ago", (s + 1800) / 3600),
    }
}

/// What [`save_icon`] drew: the button, and the card's rectangle while it is shown (for test hooks).
pub struct SaveIcon {
    pub response: Response,
    pub card: Option<Rect>,
}

/// The icon (a 36-point round button) with its hover card. The caller handles the click.
pub fn save_icon(ui: &mut Ui, info: &SaveInfo, pal: &SavePalette) -> SaveIcon {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    // When nobody says when it was saved, time it from the moment the save was seen to finish.
    let id = ui.id().with("starbase_save_icon");
    let (last_kind, saved_at) = ctx.data(|d| d.get_temp::<(SaveKind, Option<f64>)>(id)).unwrap_or((info.kind, None));
    let saved_at = match info.kind {
        SaveKind::Saved if matches!(last_kind, SaveKind::Saving | SaveKind::Unsaved) => Some(now),
        SaveKind::Saved => saved_at,
        _ => None,
    };
    ctx.data_mut(|d| d.insert_temp(id, (info.kind, saved_at)));
    let mut info = info.clone();
    if info.when.is_none() {
        info.when = saved_at.map(|t| ago_text(now - t));
    }

    let (rect, response) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(18), ui.visuals().widgets.hovered.weak_bg_fill);
    }
    let colour = colour(info.kind, pal);
    let icon = Rect::from_center_size(rect.center(), vec2(20.0, 20.0));
    let painter = ui.painter();
    match info.kind {
        SaveKind::Saved | SaveKind::ViewOnly => clock(painter, icon, colour),
        SaveKind::Saving => {
            // A gentle pulse while saving.
            let a = 0.55 + 0.45 * ((now * 4.0).sin() * 0.5 + 0.5) as f32;
            floppy(painter, icon, colour.gamma_multiply(a), pal.page);
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        _ => floppy(painter, icon, colour, pal.page),
    }
    if info.kind == SaveKind::Offline {
        painter.circle(rect.center() + vec2(8.0, -8.0), 4.0, pal.red, Stroke::new(1.5, pal.page));
    }
    if info.kind == SaveKind::Saved {
        // "2 min ago" moves on by itself.
        ctx.request_repaint_after(std::time::Duration::from_secs(30));
    }

    let lines = card_lines(&info);
    let mut card = None;
    let response = response.on_hover_ui(|ui| {
        ui.set_width(220.0);
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 14.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.0, colour);
            ui.label(RichText::new(&lines[0]).size(14.0).strong());
        });
        if !lines[1].is_empty() {
            ui.label(RichText::new(&lines[1]).size(12.0).color(pal.grey));
        }
        if !lines[2].is_empty() {
            ui.label(RichText::new(&lines[2]).size(12.0).color(pal.grey).italics());
        }
        card = Some(ui.min_rect());
    });
    let response = if info.history { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    SaveIcon { response, card }
}

fn colour(kind: SaveKind, pal: &SavePalette) -> Color32 {
    match kind {
        SaveKind::Unsaved | SaveKind::Saving => pal.ochre,
        SaveKind::Saved => pal.green,
        SaveKind::Problem => pal.red,
        SaveKind::Offline | SaveKind::ViewOnly => pal.grey,
    }
}

/// A floppy disk: the outline with a cut corner, the shutter at the top and the label below.
fn floppy(p: &egui::Painter, r: Rect, c: Color32, page: Color32) {
    let r = r.shrink(1.5);
    let cut = 4.0;
    let pts: Vec<Pos2> = vec![r.left_top(), pos2(r.right() - cut, r.top()), pos2(r.right(), r.top() + cut), r.right_bottom(), r.left_bottom()];
    p.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
    // The shutter (a light window at the top) and the label (light, lower half).
    let w = r.width();
    let shutter = Rect::from_min_size(pos2(r.left() + w * 0.22, r.top() + 1.5), vec2(w * 0.5, r.height() * 0.3));
    p.rect_filled(shutter, CornerRadius::same(1), page);
    p.rect_filled(Rect::from_min_size(pos2(shutter.right() - w * 0.16, shutter.top() + 1.0), vec2(w * 0.1, shutter.height() - 2.0)), 0, c);
    let label = Rect::from_min_max(pos2(r.left() + w * 0.16, r.top() + r.height() * 0.52), pos2(r.right() - w * 0.16, r.bottom() - 1.5));
    p.rect_filled(label, CornerRadius::same(1), page);
}

/// A clock: a ring and two hands.
fn clock(p: &egui::Painter, r: Rect, c: Color32) {
    let centre = r.center();
    let radius = r.width() * 0.42;
    p.circle_stroke(centre, radius, Stroke::new(2.0, c));
    p.line_segment([centre, centre + vec2(0.0, -radius * 0.6)], Stroke::new(2.0, c));
    p.line_segment([centre, centre + vec2(radius * 0.5, radius * 0.3)], Stroke::new(2.0, c));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(kind: SaveKind) -> SaveInfo {
        SaveInfo { kind, history: true, ..Default::default() }
    }

    #[test]
    fn every_row_of_the_spec() {
        let v = Some((14, "James".to_string()));
        let rows: Vec<(SaveInfo, [&str; 3])> = vec![
            (info(SaveKind::Unsaved), ["Not saved yet", "Saving in a moment", CLICK]),
            (SaveInfo { version: Some((13, "you".into())), ..info(SaveKind::Saving) }, ["Saving…", "Version 13 is the last one", CLICK]),
            (
                SaveInfo { when: Some("2 min ago".into()), version: v.clone(), ..info(SaveKind::Saved) },
                ["Saved · 2 min ago", "Version 14 · by James", CLICK],
            ),
            (info(SaveKind::Offline), ["Offline", "Changes are kept and will sync", CLICK]),
            (SaveInfo { detail: "No access".into(), ..info(SaveKind::Problem) }, ["Not saved", "No access", CLICK]),
            (SaveInfo { version: v.clone(), ..info(SaveKind::ViewOnly) }, ["View only", "Version 14 · by James", CLICK]),
            // Desktop: a file on this computer, no versions, no history.
            (
                SaveInfo { kind: SaveKind::Saved, local: true, when: Some("just now".into()), ..Default::default() },
                ["Saved on this computer", "", ""],
            ),
            // No version known: the detail line instead; nothing known: one line.
            (SaveInfo { detail: "Last edit by you".into(), ..info(SaveKind::Saved) }, ["Saved", "Last edit by you", CLICK]),
            (SaveInfo { version: Some((3, String::new())), ..info(SaveKind::ViewOnly) }, ["View only", "Version 3", CLICK]),
        ];
        for (i, want) in rows {
            assert_eq!(card_lines(&i), want.map(str::to_string), "{i:?}");
        }
    }

    #[test]
    fn ago_words() {
        assert_eq!(ago_text(3.0), "just now");
        assert_eq!(ago_text(60.0), "a minute ago");
        assert_eq!(ago_text(120.0), "2 min ago");
        assert_eq!(ago_text(4000.0), "an hour ago");
        assert_eq!(ago_text(7200.0), "2 hours ago");
    }

    #[test]
    fn kind_words_are_distinct() {
        let all = [SaveKind::Unsaved, SaveKind::Saving, SaveKind::Saved, SaveKind::Offline, SaveKind::Problem, SaveKind::ViewOnly];
        let words: std::collections::HashSet<_> = all.iter().map(|k| k.word()).collect();
        assert_eq!(words.len(), all.len());
    }
}
