//! The pictures' "justified rows" layout (Google Photos, Flickr): every picture keeps its own
//! aspect ratio (never cropped), each row is scaled so it fills the width exactly, and the last
//! row is left at the target height instead of being stretched. Pure maths, no egui: the Photos
//! grid and the Timeline use it.

/// A phone-width window (at 390 px the rows are shorter so more fit).
pub(in crate::home) const PHONE_WIDTH: f32 = 520.0;
/// Target row height on a desktop and on a phone, px.
pub(in crate::home) const ROW_DESKTOP: f32 = 180.0;
pub(in crate::home) const ROW_PHONE: f32 = 110.0;
/// A ratio outside this is treated as this (a 1:50 sliver would otherwise own a whole row).
const MIN_RATIO: f32 = 0.2;
const MAX_RATIO: f32 = 5.0;

/// The target row height for a grid `width` wide.
pub(in crate::home) fn target_for(width: f32) -> f32 {
    if width < PHONE_WIDTH { ROW_PHONE } else { ROW_DESKTOP }
}

/// One row: pictures `start..end`, all `height` tall.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::home) struct Row {
    pub start: usize,
    pub end: usize,
    pub height: f32,
}

/// The ratio to lay out for a picture of `w` x `h` (4:3 while unknown).
pub(in crate::home) fn ratio(w: u32, h: u32) -> f32 {
    if w == 0 || h == 0 { 4.0 / 3.0 } else { (w as f32 / h as f32).clamp(MIN_RATIO, MAX_RATIO) }
}

/// Splits pictures with these width / height `ratios` into rows for `width`, rows about `target`
/// tall, `gap` between pictures and rows. A picture's width in its row is `ratio * row.height`.
pub(in crate::home) fn rows(ratios: &[f32], width: f32, target: f32, gap: f32) -> Vec<Row> {
    let mut out = Vec::new();
    let (mut start, mut sum) = (0, 0.0_f32);
    for (i, r) in ratios.iter().enumerate() {
        sum += r.clamp(MIN_RATIO, MAX_RATIO);
        let n = (i + 1 - start) as f32;
        // The row is full when its pictures, at the target height, are as wide as the grid.
        if sum * target + gap * (n - 1.0) >= width {
            // Scaled to fill exactly, but never taller than the target (one narrow picture).
            let height = ((width - gap * (n - 1.0)) / sum).min(target);
            out.push(Row { start, end: i + 1, height });
            start = i + 1;
            sum = 0.0;
        }
    }
    if start < ratios.len() {
        out.push(Row { start, end: ratios.len(), height: target });
    }
    out
}

/// Where each row starts, and the whole grid's height.
pub(in crate::home) fn offsets(rows: &[Row], gap: f32) -> (Vec<f32>, f32) {
    let mut y = 0.0;
    let tops = rows
        .iter()
        .map(|r| {
            let t = y;
            y += r.height + gap;
            t
        })
        .collect();
    (tops, (y - gap).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAP: f32 = 4.0;

    fn width_of(rs: &[f32], row: &Row) -> f32 {
        rs[row.start..row.end].iter().map(|r| r.clamp(MIN_RATIO, MAX_RATIO) * row.height).sum::<f32>() + GAP * (row.end - row.start - 1) as f32
    }

    #[test]
    fn every_full_row_fills_the_width_and_keeps_every_ratio() {
        // portrait, landscape, panorama, square, mixed
        let rs: Vec<f32> = [0.75, 1.5, 3.0, 1.0, 0.66, 1.33, 1.78, 0.5, 2.0, 1.0, 1.5, 0.8, 1.2].to_vec();
        let all = rows(&rs, 1000.0, 180.0, GAP);
        assert_eq!(all.first().unwrap().start, 0);
        assert_eq!(all.last().unwrap().end, rs.len(), "every picture is in a row");
        for w in all.windows(2) {
            assert_eq!(w[0].end, w[1].start, "no gap, no overlap");
        }
        for row in &all[..all.len() - 1] {
            assert!((width_of(&rs, row) - 1000.0).abs() < 0.01, "{row:?} fills the width");
            assert!(row.height <= 180.0 + 0.01);
        }
    }

    #[test]
    fn the_last_row_is_not_stretched() {
        let rs = [1.5, 1.5, 1.5, 1.5, 1.5];
        let all = rows(&rs, 1000.0, 180.0, GAP);
        let last = all.last().unwrap();
        assert_eq!(last.height, 180.0);
        assert!(width_of(&rs, last) < 1000.0);
    }

    #[test]
    fn a_panorama_gets_a_row_to_itself_and_is_never_taller_than_the_target() {
        let all = rows(&[5.0, 1.0], 390.0, 110.0, GAP);
        assert_eq!(all[0], Row { start: 0, end: 1, height: 390.0 / 5.0 });
        assert!(all[0].height < 110.0);
    }

    #[test]
    fn a_lone_narrow_picture_is_not_blown_up() {
        let all = rows(&[0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5], 1000.0, 180.0, GAP);
        assert!(all.iter().all(|r| r.height <= 180.0));
        assert_eq!(rows(&[], 1000.0, 180.0, GAP), vec![]);
        assert_eq!(rows(&[0.5], 1000.0, 180.0, GAP), vec![Row { start: 0, end: 1, height: 180.0 }]);
    }

    #[test]
    fn phone_rows_are_shorter_and_offsets_add_up() {
        assert_eq!((target_for(390.0), target_for(1200.0)), (110.0, 180.0));
        let all = rows(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0], 390.0, 110.0, GAP);
        let (tops, total) = offsets(&all, GAP);
        assert_eq!(tops[0], 0.0);
        let last = all.last().unwrap();
        assert!((tops.last().unwrap() + last.height - total).abs() < 0.01);
        assert_eq!(ratio(0, 5), 4.0 / 3.0);
        assert_eq!(ratio(1000, 10), MAX_RATIO);
    }
}
