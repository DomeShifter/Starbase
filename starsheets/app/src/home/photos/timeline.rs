//! The Timeline view of Photo Mode: the picture set as one long column, newest first, under
//! month and day headers, with a date scrubber on the right edge.
//!
//! - Pictures keep their shape (justified rows, no cropping). TODO: switch `justify` to the
//!   shared layout in photos/justify.rs once it has merged.
//! - Only the rows in view are drawn (the layout is built once per set and width, and the
//!   visible part found by binary search), so tens of thousands of pictures scroll smoothly.
//! - Dates: the EXIF taken date the index returns, else the file's modified time.
//! - The scrubber's ticks are spaced by how many pictures each month has; dragging or clicking
//!   jumps there, and a bubble names the month.

use egui::{Align2, Color32, FontId, Rect, Sense, Ui, pos2, vec2};

use super::{GAP, Pic, SMALL};
use crate::house;
use crate::store::Backend;
use crate::theme;

const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const MONTH_H: f32 = 46.0;
const DAY_H: f32 = 30.0;
const SCRUB_W: f32 = 36.0;

/// (year, month 1-12, day 1-31, seconds into the day) of a picture: taken date, else modified.
pub(super) fn when(p: &Pic) -> (i32, u32, u32, u32) {
    if let Some(t) = p.taken.as_deref()
        && t.len() >= 10
        && let (Ok(y), Ok(m), Ok(d)) = (t[0..4].parse::<i32>(), t[5..7].parse::<u32>(), t[8..10].parse::<u32>())
        && (1..=12).contains(&m)
        && (1..=31).contains(&d)
    {
        let part = |a: usize| t.get(a..a + 2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
        let secs = if t.len() >= 19 { part(11) * 3600 + part(14) * 60 + part(17) } else { 0 };
        return (y, m, d, secs);
    }
    let (y, m, d) = civil(p.modified.div_euclid(86400));
    (y, m, d, p.modified.rem_euclid(86400) as u32)
}

/// Year, month, day of a day count since 1970-01-01 (proleptic Gregorian).
fn civil(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((yoe + era * 400 + i64::from(m <= 2)) as i32, m, d)
}

/// Days since 1970-01-01 of a date.
fn days(y: i32, m: u32, d: u32) -> i64 {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Sort key: newer is larger.
fn key(p: &Pic) -> i64 {
    let (y, m, d, s) = when(p);
    days(y, m, d) * 86400 + i64::from(s)
}

/// "Wed 25 Nov".
fn day_label(y: i32, m: u32, d: u32) -> String {
    let wd = (days(y, m, d) + 3).rem_euclid(7) as usize; // 1970-01-01 was a Thursday
    format!("{} {d} {}", WEEKDAYS[wd], &MONTHS[m as usize - 1][..3])
}

enum Kind {
    Month(String),
    Day(String),
    /// Pictures `a..b` (indexes into the sorted set) at height `h`.
    Row(usize, usize, f32),
}

struct Item {
    y: f32,
    h: f32,
    kind: Kind,
}

/// A month on the scrubber: label, year, offset of its header, first picture, how many pictures.
struct Month {
    label: String,
    year: i32,
    y: f32,
    first: usize,
    count: usize,
}

#[derive(Default)]
struct Built {
    width: f32,
    sig: (usize, String),
    items: Vec<Item>,
    months: Vec<Month>,
    total: f32,
    /// y of each picture's row, by picture index.
    row_y: Vec<f32>,
}

#[derive(Default)]
pub(in crate::home) struct Timeline {
    built: Built,
    scroll_to: Option<f32>,
    offset: f32,
    /// The scrubber is being dragged: where the bubble shows.
    bubble: Option<(f32, String)>,
    /// Where the scrubber and the first tile on screen were drawn (points), for the test hook.
    scrub: Option<Rect>,
    tile0: Option<Rect>,
}

/// Justified rows over `aspects` (width / height), as (start, end, height) of each row.
fn justify(aspects: &[f32], width: f32, target: f32) -> Vec<(usize, usize, f32)> {
    let mut rows = Vec::new();
    let (mut a, mut sum) = (0, 0.0);
    for (i, &r) in aspects.iter().enumerate() {
        sum += r;
        let n = (i + 1 - a) as f32;
        if sum * target + GAP * (n - 1.0) >= width {
            rows.push((a, i + 1, ((width - GAP * (n - 1.0)) / sum).min(target * 1.6)));
            a = i + 1;
            sum = 0.0;
        }
    }
    if a < aspects.len() {
        rows.push((a, aspects.len(), target));
    }
    rows
}

fn aspect(p: &Pic) -> f32 {
    if p.width > 0 && p.height > 0 { p.width as f32 / p.height as f32 } else { 1.5 }
}

fn build(pics: &[Pic], width: f32) -> Built {
    let target = if width < 520.0 { 110.0 } else { 180.0 };
    let mut b = Built { width, sig: sig(pics), row_y: vec![0.0; pics.len()], ..Default::default() };
    let mut y = 0.0;
    let mut i = 0;
    let mut cur_month = (0, 0);
    while i < pics.len() {
        let (yr, mo, dy, _) = when(&pics[i]);
        let mut j = i;
        while j < pics.len() && when(&pics[j]).0 == yr && when(&pics[j]).1 == mo && when(&pics[j]).2 == dy {
            j += 1;
        }
        if cur_month != (yr, mo) {
            cur_month = (yr, mo);
            let label = format!("{} {yr}", MONTHS[mo as usize - 1]);
            b.months.push(Month { label: label.clone(), year: yr, y, first: i, count: 0 });
            b.items.push(Item { y, h: MONTH_H, kind: Kind::Month(label) });
            y += MONTH_H;
        }
        if let Some(m) = b.months.last_mut() {
            m.count += j - i;
        }
        let mut label = day_label(yr, mo, dy);
        if let Some(pl) = pics[i..j].iter().find_map(|p| p.place.clone()) {
            label = format!("{label} · {pl}");
        }
        b.items.push(Item { y, h: DAY_H, kind: Kind::Day(label) });
        y += DAY_H;
        let aspects: Vec<f32> = pics[i..j].iter().map(aspect).collect();
        for (s, e, h) in justify(&aspects, width, target) {
            for k in s..e {
                b.row_y[i + k] = y;
            }
            b.items.push(Item { y, h, kind: Kind::Row(i + s, i + e, h) });
            y += h + GAP;
        }
        y += GAP * 2.0;
        i = j;
    }
    b.total = y;
    b
}

fn sig(pics: &[Pic]) -> (usize, String) {
    (pics.len(), pics.first().map(|p| format!("{}{}{}", p.id, p.width, p.taken.as_deref().unwrap_or(""))).unwrap_or_default())
}

impl super::PhotoMode {
    /// The timeline for the pictures listed. Newest first, whatever the Photos order is.
    pub(super) fn timeline_ui(&mut self, ui: &mut Ui, store: &Backend) {
        let ctx = ui.ctx().clone();
        if !self.pics.windows(2).all(|w| key(&w[0]) >= key(&w[1])) {
            self.pics.sort_by_key(|p| std::cmp::Reverse(key(p)));
        }
        let avail = ui.available_rect_before_wrap();
        let width = (avail.width() - house::SCROLL_ROOM - SCRUB_W).max(120.0);
        if self.tl.built.width != width || self.tl.built.sig != sig(&self.pics) {
            self.tl.built = build(&self.pics, width);
        }
        let total = self.tl.built.total;
        let view_h = avail.height();
        let mut sa = egui::ScrollArea::vertical().id_salt("timeline").auto_shrink([false, false]).max_width(width + house::SCROLL_ROOM);
        if let Some(o) = self.tl.scroll_to.take() {
            sa = sa.vertical_scroll_offset(o);
        }
        let mut open = None;
        let mut first_tile = None;
        let out = sa.show_viewport(ui, |ui, vp| {
            let (rect, _) = ui.allocate_exact_size(vec2(width, total.max(1.0)), Sense::hover());
            let (top, bottom) = (vp.min.y - 200.0, vp.max.y + 200.0);
            let items = &self.tl.built.items;
            let from = items.partition_point(|it| it.y + it.h < top);
            let to = items.partition_point(|it| it.y <= bottom);
            let mut shown = Vec::new();
            for it in &items[from..to] {
                let y = rect.min.y + it.y;
                match &it.kind {
                    Kind::Month(l) => {
                        ui.painter().text(pos2(rect.min.x + 2.0, y + 10.0), Align2::LEFT_TOP, l, FontId::proportional(house::TITLE), theme::text());
                    }
                    Kind::Day(l) => {
                        ui.painter().text(pos2(rect.min.x + 2.0, y + 6.0), Align2::LEFT_TOP, l, FontId::proportional(house::BODY), theme::text_dim());
                    }
                    Kind::Row(a, b, h) => shown.push((y, *a, *b, *h)),
                }
            }
            for (y, a, b, h) in shown {
                let mut x = rect.min.x;
                let aspects: Vec<f32> = (a..b).map(|i| aspect(&self.pics[i])).collect();
                let sum: f32 = aspects.iter().sum();
                // Rows that fill the width are justified; the last row of a day keeps its height.
                let full = sum * h + GAP * (b - a - 1) as f32 >= width - 1.0;
                for (k, i) in (a..b).enumerate() {
                    let w = if full && k + 1 == b - a { (rect.max.x - x).max(1.0) } else { aspects[k] * h };
                    let r = Rect::from_min_size(pos2(x, y), vec2(w, h));
                    x += w + GAP;
                    self.want(&ctx, store, i, SMALL);
                    let resp = ui.interact(r, ui.scope_id().with(("tl", i)), Sense::click());
                    let painter = ui.painter_at(r);
                    painter.rect_filled(r, 0, Color32::from_rgb(0x2A, 0x2D, 0x31));
                    if let Some((t, _)) = self.best(i, SMALL) {
                        painter.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    if self.pics[i].is_video() {
                        super::video_badge(&painter, r, self.pics[i].duration);
                    }
                    if first_tile.is_none() && r.min.y >= ui.clip_rect().min.y {
                        first_tile = Some(r);
                    }
                    if resp.hovered() {
                        painter.rect_filled(r, 0, Color32::from_black_alpha(40));
                    }
                    if resp.clicked() {
                        open = Some(i);
                    }
                }
            }
            vp.min.y
        });
        self.tl.offset = out.inner;
        self.tl.tile0 = first_tile;
        self.tl.scrub = Some(Rect::from_min_max(pos2(avail.max.x - SCRUB_W, avail.min.y), pos2(avail.max.x, avail.max.y)));
        let scrub = Rect::from_min_max(pos2(avail.max.x - SCRUB_W, avail.min.y), pos2(avail.max.x, avail.max.y));
        self.scrubber(ui, scrub, total, view_h);
        if let Some(i) = open {
            self.viewer = Some(super::Viewer::new(i, self.pics.len()));
        }
    }

    /// The right edge: ticks per month spaced by picture count; click or drag to jump.
    fn scrubber(&mut self, ui: &mut Ui, rect: Rect, total: f32, view_h: f32) {
        let b = &self.tl.built;
        let n: usize = b.months.iter().map(|m| m.count).sum();
        if n == 0 {
            return;
        }
        let resp = ui.interact(rect, ui.scope_id().with("tl-scrub"), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        let pad = 10.0;
        let (y0, span) = (rect.min.y + pad, rect.height() - 2.0 * pad);
        let mut last_year = i32::MIN;
        let mut seen = 0usize;
        for m in &b.months {
            let y = y0 + span * seen as f32 / n as f32;
            seen += m.count;
            if m.year != last_year {
                last_year = m.year;
                painter.text(pos2(rect.max.x - 4.0, y), Align2::RIGHT_TOP, m.year.to_string(), FontId::proportional(house::NOTE), theme::text_dim());
            } else {
                painter.line_segment([pos2(rect.max.x - 10.0, y + 1.0), pos2(rect.max.x - 4.0, y + 1.0)], (1.0, theme::text_dim()));
            }
        }
        // Where the view is now.
        let cur = (self.tl.offset / total.max(1.0)).clamp(0.0, 1.0);
        painter.rect_filled(Rect::from_center_size(pos2(rect.max.x - 2.0, y0 + span * cur + 8.0), vec2(4.0, 16.0)), 2, theme::share());
        if (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
            && let Some(p) = resp.interact_pointer_pos()
        {
            let f = ((p.y - y0) / span.max(1.0)).clamp(0.0, 0.9999);
            let idx = ((f * n as f32) as usize).min(n - 1);
            if let Some(m) = b.months.iter().rev().find(|m| m.first <= idx).or(b.months.first()) {
                let to = (b.row_y.get(idx).copied().unwrap_or(m.y) - 40.0).clamp(0.0, (total - view_h).max(0.0));
                self.tl.bubble = Some((p.y, m.label.clone()));
                self.tl.scroll_to = Some(to);
                ui.ctx().request_repaint();
            }
        } else {
            self.tl.bubble = None;
        }
        if let Some((y, label)) = &self.tl.bubble {
            let galley = ui.painter().layout_no_wrap(label.clone(), FontId::proportional(house::BODY), Color32::WHITE);
            let r = Rect::from_min_size(pos2(rect.min.x - galley.size().x - 24.0, y - 14.0), galley.size() + vec2(16.0, 10.0));
            ui.painter().rect_filled(r, 8, Color32::from_black_alpha(220));
            ui.painter().galley(r.min + vec2(8.0, 5.0), galley, Color32::WHITE);
        }
    }
}

impl Timeline {
    /// For the test hook: where things are drawn, the scroll offset and the months.
    pub(in crate::home) fn test_state(&self) -> serde_json::Value {
        let r = |r: &Option<Rect>| r.map(|r| [r.min.x, r.min.y, r.max.x, r.max.y]);
        serde_json::json!({
            "offset": self.offset, "total": self.built.total, "scrub": r(&self.scrub), "tile0": r(&self.tile0),
            "bubble": self.bubble.as_ref().map(|b| &b.1),
            "months": self.built.months.iter().map(|m| serde_json::json!([m.label, m.count])).collect::<Vec<_>>(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pic(taken: Option<&str>, modified: i64) -> Pic {
        Pic { taken: taken.map(str::to_string), modified, width: 300, height: 200, ..Default::default() }
    }

    #[test]
    fn dates_come_from_exif_then_modified() {
        assert_eq!(when(&pic(Some("2026-11-25T18:30:05+00:00"), 0)), (2026, 11, 25, 18 * 3600 + 30 * 60 + 5));
        assert_eq!(when(&pic(None, 86400 * 365)), (1971, 1, 1, 0));
        assert_eq!(day_label(2026, 11, 25), "Wed 25 Nov");
    }

    #[test]
    fn rows_fill_the_width_and_keep_shape() {
        let a = vec![1.5; 10];
        let rows = justify(&a, 800.0, 180.0);
        assert_eq!(rows.iter().map(|r| r.1 - r.0).sum::<usize>(), 10);
        let (s, e, h) = rows[0];
        let w: f32 = (s..e).map(|i| a[i] * h).sum::<f32>() + GAP * (e - s - 1) as f32;
        assert!((w - 800.0).abs() < 1.0 || h >= 180.0 * 1.6 - 0.1);
    }

    #[test]
    fn layout_groups_by_month_and_day() {
        let pics = vec![pic(Some("2026-11-25T10:00:00"), 0), pic(Some("2026-11-25T09:00:00"), 0), pic(Some("2026-10-02T09:00:00"), 0)];
        let b = build(&pics, 800.0);
        assert_eq!(b.months.len(), 2);
        assert_eq!(b.months[0].count, 2);
        assert_eq!(b.months[1].label, "October 2026");
        assert!(b.total > 0.0);
    }
}
