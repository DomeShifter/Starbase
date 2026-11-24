//! Roboto (OFL 1.1, app/fonts/OFL.txt) for the interface and for sans-serif cells. A web build has
//! no system fonts, so these lead GridCraft's font roles (patch 0005, `set_host_faces`).

static REGULAR: &[u8] = include_bytes!("../fonts/Roboto-Regular.ttf");
static MEDIUM: &[u8] = include_bytes!("../fonts/Roboto-Medium.ttf");
static BOLD: &[u8] = include_bytes!("../fonts/Roboto-Bold.ttf");
static ITALIC: &[u8] = include_bytes!("../fonts/Roboto-Italic.ttf");
static BOLD_ITALIC: &[u8] = include_bytes!("../fonts/Roboto-BoldItalic.ttf");

/// Call before the app builds its fonts.
pub fn install() {
    gridcraft_ui_egui::theme::set_host_faces(vec![
        ("ui", REGULAR),
        ("ui-bold", MEDIUM),
        ("cell", REGULAR),
        ("cell-bold", BOLD),
        ("cell-italic", ITALIC),
        ("cell-bold-italic", BOLD_ITALIC),
    ]);
}

/// The web fonts (menu name, Google Fonts family), from app/webfonts.txt.
pub static WEB_FONTS: std::sync::LazyLock<Vec<(&'static str, &'static str)>> = std::sync::LazyLock::new(|| {
    include_str!("../webfonts.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('|').map(|(a, b)| (a.trim(), b.trim())))
        .collect()
});

/// The folder a web font's files are served from (as scripts/fetch-fonts.py names it).
pub fn slug(menu: &str) -> String {
    let mut out = String::new();
    for c in menu.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Fetches a web font the first time an open workbook uses it, and hands its faces to GridCraft
/// (patch 0021), which then draws those cells in it.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
pub struct WebFontLoader {
    requested: std::collections::HashSet<String>,
    previews: bool,
    /// The open workbooks (uid, revision) last looked at: the fonts they use change only with it.
    seen: Option<Vec<(u64, u64)>>,
}

#[cfg(target_arch = "wasm32")]
impl WebFontLoader {
    pub fn logic(&mut self, app: &gridcraft_ui_egui::SheetApp, ctx: &egui::Context) {
        if !self.previews {
            self.previews = true;
            load_previews(ctx);
        }
        // Looking through every style of every open workbook each frame is wasted work on a big
        // file: only when a workbook changed (or another was opened).
        let key: Vec<(u64, u64)> = app.session.documents().iter().map(|d| (d.uid, d.revision)).collect();
        if self.seen.as_ref() == Some(&key) {
            return;
        }
        self.seen = Some(key);
        for name in app.used_fonts() {
            if self.requested.contains(&name) {
                continue;
            }
            let Some((menu, _)) = WEB_FONTS.iter().find(|(m, _)| m.eq_ignore_ascii_case(&name)) else { continue };
            self.requested.insert(name.clone());
            let folder = slug(menu);
            let ctx = ctx.clone();
            // All four faces are handed over together: each addition makes GridCraft rebuild its
            // font set (and its glyph atlas), so one at a time cost four rebuilds per font.
            wasm_bindgen_futures::spawn_local(async move {
                let mut got = Vec::new();
                for (file, bold, italic) in [("regular", false, false), ("bold", true, false), ("italic", false, true), ("bolditalic", true, true)] {
                    if let Ok((200, _, bytes)) = crate::files::get_bytes(&format!("webfonts/{folder}/{file}.ttf")).await {
                        got.push((bold, italic, bytes));
                    }
                }
                for (bold, italic, bytes) in got {
                    gridcraft_ui_egui::fonts::add_web_face(&name, bold, italic, bytes);
                }
                ctx.request_repaint();
            });
        }
    }
}

/// The font menu's previews: each web font's name drawn in that font, at a size that makes the
/// names look about as big as each other (scaled by cap height to match Roboto's), as in Sheets.
static PREVIEW_SCALE: std::sync::RwLock<Vec<(String, f32)>> = std::sync::RwLock::new(Vec::new());

/// Roboto's cap height over its em (1456 / 2048).
const ROBOTO_CAP: f32 = 0.711;

/// The font to draw `menu` in, in the font menu, at about `size` px; `None` until it has loaded.
pub fn preview_font(ctx: &egui::Context, menu: &str, size: f32) -> Option<egui::FontId> {
    let scale = PREVIEW_SCALE.read().ok()?.iter().find(|(m, _)| m == menu).map(|p| p.1)?;
    let family = egui::FontFamily::Name(gridcraft_ui_egui::fonts::role_key(&preview_family(menu), false, false).into());
    // Only once egui has the family (a font set applies a frame late; an unknown family panics).
    ctx.fonts(|f| f.definitions().families.contains_key(&family)).then(|| egui::FontId::new(size * scale, family))
}

fn preview_family(menu: &str) -> String {
    format!("menu:{menu}")
}

/// How much to scale a font so its capitals stand as tall as Roboto's (1.0 when the font doesn't
/// say), kept between 0.8 and 1.4 so nothing ends up tiny or huge.
fn cap_scale(ttf: &[u8]) -> f32 {
    let be16 = |o: usize| ttf.get(o..o + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let be32 = |o: usize| ttf.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize);
    let table = |tag: &[u8; 4]| {
        (0..usize::from(be16(4).unwrap_or(0))).map(|i| 12 + 16 * i).find(|o| ttf.get(*o..*o + 4) == Some(tag)).and_then(|o| be32(o + 8))
    };
    let cap = || {
        let os2 = table(b"OS/2")?;
        let upm = f32::from(be16(table(b"head")? + 18)?);
        let cap = f32::from(be16(os2 + 88)? as i16);
        (be16(os2)? >= 2 && cap > 0.0 && upm > 0.0).then(|| cap / upm)
    };
    cap().map_or(1.0, |c| (ROBOTO_CAP / c).clamp(0.8, 1.4))
}

/// Fetches every web font's menu face (a few KB each) once, then adds them all together, so
/// GridCraft rebuilds its fonts only once.
#[cfg(target_arch = "wasm32")]
pub fn load_previews(ctx: &egui::Context) {
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let mut got = Vec::new();
        for (menu, _) in WEB_FONTS.iter() {
            if let Ok((200, _, bytes)) = crate::files::get_bytes(&format!("webfonts/{}/menu.ttf", slug(menu))).await {
                got.push((menu.to_string(), bytes));
            }
        }
        let mut scales = Vec::new();
        for (menu, bytes) in got {
            scales.push((menu.clone(), cap_scale(&bytes)));
            gridcraft_ui_egui::fonts::add_web_face(&preview_family(&menu), false, false, bytes);
        }
        gridcraft_ui_egui::fonts::keep_families(scales.iter().map(|(m, _)| preview_family(m)).collect());
        if let Ok(mut p) = PREVIEW_SCALE.write() {
            *p = scales;
        }
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_font_without_metrics_keeps_its_size() {
        assert_eq!(super::cap_scale(b"not a font"), 1.0);
    }
}
