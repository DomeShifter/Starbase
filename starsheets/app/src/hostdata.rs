//! StarSheets' own section of a file (docs/decisions.md, 2026-11-23): what Excel has no place
//! for, kept as JSON text in GridCraft's `Workbook::host_data`, which is saved inside the .xlsx as
//! a custom XML part that Excel and LibreOffice keep but don't show. It travels to every tab with
//! the workbook and changes through the undoable `workbook.setHostData` command.
//!
//! Everything here describes something that also exists in the spreadsheet itself (a data
//! validation rule, a hidden name). A file can be edited elsewhere (Excel keeps our section but
//! doesn't update it), so an entry is only used while what it describes still matches, and
//! entries that no longer match are dropped on the next write. Older files have no section: the
//! panels then fall back to reading the spreadsheet as before.

use gridcraft_ui_egui::SheetApp;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json, json};

/// The section's format version.
pub const VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HostData {
    #[serde(default)]
    pub v: u32,
    /// Data validation rules: what the panel's criterion was, and which drop-down items have a
    /// colour (kept as conditional-formatting rules so Excel shows them too).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dv: Vec<DvMeta>,
    /// Protected sheets and ranges: each entry's settings, by its hidden name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protect: Vec<ProtectMeta>,
    /// Anything a newer StarSheets wrote, kept as it is.
    #[serde(flatten)]
    pub other: Map<String, Json>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DvMeta {
    /// The sheet's name.
    pub sheet: String,
    /// The rule's ranges, as GridCraft prints them (`B2:B9,D2`).
    pub range: String,
    /// The panel's criterion, by its label (`Text contains`, `Custom formula is`).
    pub crit: String,
    /// Drop-down items with a colour: the item's text and the fill (`#RRGGBB`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colours: Vec<ItemColour>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemColour {
    pub text: String,
    pub colour: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ProtectMeta {
    /// The hidden name that holds the protected range (`_sc_protect<n>`).
    pub name: String,
    /// The entry's settings (the protect panel's own format).
    pub meta: Json,
}

/// The section of the active workbook (empty when it has none or it can't be read).
pub fn read(app: &SheetApp) -> HostData {
    app.session.active().and_then(|d| d.wb.host_data.as_deref()).map(parse).unwrap_or_default()
}

pub fn parse(text: &str) -> HostData {
    serde_json::from_str(text).unwrap_or_default()
}

/// Writes the section, first dropping entries whose rule or name is gone. Does nothing when
/// nothing changes (no undo step).
pub fn write(app: &mut SheetApp, mut h: HostData) {
    let Some(d) = app.session.active() else { return };
    let wb = &d.wb;
    h.v = VERSION;
    h.dv.retain(|m| wb.sheets.iter().find(|s| s.name == m.sheet).is_some_and(|sh| sh.validations.iter().any(|v| ranges_text(&v.ranges) == m.range)));
    h.protect.retain(|p| wb.names.iter().any(|n| n.name.eq_ignore_ascii_case(&p.name)));
    let empty = h.dv.is_empty() && h.protect.is_empty() && h.other.is_empty();
    let text = if empty { None } else { serde_json::to_string(&h).ok() };
    if text.as_deref() == wb.host_data.as_deref() {
        return;
    }
    let _ = app.run("workbook.setHostData", json!({ "data": text }));
}

/// A rule's ranges as one text, the way the section stores them.
pub fn ranges_text(ranges: &[gridcraft_engine::core::RangeRef]) -> String {
    ranges.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",")
}

/// Ranges typed in a panel (`b2:b9, d2`) in the section's form (`B2:B9,D2`).
pub fn normalise_ranges(text: &str) -> String {
    crate::refs::plain(text)
}

impl HostData {
    pub fn dv_for(&self, sheet: &str, range: &str) -> Option<&DvMeta> {
        self.dv.iter().find(|m| m.sheet == sheet && m.range == range)
    }
    pub fn set_dv(&mut self, m: DvMeta) {
        self.dv.retain(|x| !(x.sheet == m.sheet && x.range == m.range));
        self.dv.push(m);
    }
    pub fn remove_dv(&mut self, sheet: &str, range: &str) {
        self.dv.retain(|x| !(x.sheet == sheet && x.range == range));
    }
    pub fn protect_for(&self, name: &str) -> Option<&Json> {
        self.protect.iter().find(|p| p.name.eq_ignore_ascii_case(name)).map(|p| &p.meta)
    }
    pub fn set_protect(&mut self, name: &str, meta: Json) {
        self.protect.retain(|p| !p.name.eq_ignore_ascii_case(name));
        self.protect.push(ProtectMeta { name: name.to_string(), meta });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_parts_are_kept_and_bad_text_reads_as_empty() {
        let h = parse(r#"{"v":1,"dv":[{"sheet":"S","range":"A1","crit":"Custom formula is"}],"future":{"x":1}}"#);
        assert_eq!(h.dv[0].crit, "Custom formula is");
        let back = serde_json::to_string(&h).unwrap();
        assert!(back.contains(r#""future":{"x":1}"#), "{back}");
        assert_eq!(parse("not json"), HostData::default());
        assert_eq!(normalise_ranges("b2:b9, d2"), "B2:B9,D2");
    }
}
