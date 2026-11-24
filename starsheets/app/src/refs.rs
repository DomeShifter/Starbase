//! Ranges typed in a panel's range box, read the way formulas read them (to-do 2 row 19):
//! `B2:B9`, `$A$2:$B$9`, `A:A`, Google's open-ended `A2:A`, and with a sheet in front,
//! `Sheet1!A2:A` or `'Query (with dates)'!I2`.

use gridcraft_engine::core::RangeRef;
use gridcraft_engine::formula::Expr;

/// One typed range: the sheet it names (None: none given) and its cells.
pub fn area(text: &str) -> Option<(Option<String>, RangeRef)> {
    match gridcraft_engine::formula::parse(text.trim().trim_start_matches('=')).ok()? {
        Expr::Ref(r) => Some((r.sheet_name().map(str::to_string), r.range())),
        _ => None,
    }
}

/// Comma-separated ranges (`B2:B9, Sheet1!D2`) without their sheet names, as `B2:B9,D2`; parts
/// that aren't ranges are left out.
pub fn plain(text: &str) -> String {
    text.split(',').filter_map(area).map(|(_, r)| r.a1()).collect::<Vec<_>>().join(",")
}

/// The sheet the ranges name, if any names one other than `here` (a rule's own cells must be on
/// the sheet it is made on, as in Google Sheets).
pub fn other_sheet(text: &str, here: &str) -> Option<String> {
    text.split(',').filter_map(area).filter_map(|(s, _)| s).find(|s| !s.eq_ignore_ascii_case(here))
}

/// The top-left cell of the first range, for relative rule formulas.
pub fn top_left(text: &str) -> Option<String> {
    area(text.split(',').next()?).map(|(_, r)| r.start.a1())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheets_dollars_and_open_ranges_are_read() {
        assert_eq!(plain("'Query (with dates)'!$I$2, b3:b9"), "I2,B3:B9");
        assert_eq!(top_left("Sheet1!A2:A"), Some("A2".into()));
        assert_eq!(other_sheet("Sheet1!A2:A,B1", "Sheet1"), None);
        assert_eq!(other_sheet("'Other one'!A1", "Sheet1"), Some("Other one".into()));
        assert_eq!(plain("not a range"), "");
    }
}
