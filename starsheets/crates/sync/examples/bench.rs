//! Speed and memory check for large, many-tab workbooks: how long the work behind opening,
//! editing and saving a big file takes, and the most memory it used.
//!
//!     cargo run --release -p starsheets-sync --example bench
//!
//! Size: BENCH_SHEETS tabs (default 30) × BENCH_ROWS rows (default 3,400) × 10 columns, about a
//! million cells, plus one big tab of BENCH_BIG_ROWS rows (default 100,000: another million).
//! Each row: 8 numbers, a text and a formula (the row's sum). CI runs it and
//! puts the table in the job summary, so a slowdown shows up next to the change that caused it.
//! The numbers are for comparing runs on the same machine, not a pass/fail test. BENCH_SAVE=path
//! also writes the .xlsx there, to open it in the app.

use std::sync::Arc;
use std::time::Instant;

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::{Cell, Formula, Sheet, Workbook};

fn env(name: &str, default: u32) -> u32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Most memory this process has held so far (Linux; 0 elsewhere), in MB.
fn peak_mb() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()))
        .map_or(0, |kb| kb / 1024)
}

struct Report(Vec<(String, f64, String)>);

impl Report {
    fn time<T>(&mut self, what: &str, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let out = f();
        self.0.push((what.to_string(), t.elapsed().as_secs_f64() * 1000.0, String::new()));
        out
    }
    fn note(&mut self, text: String) {
        if let Some(last) = self.0.last_mut() {
            last.2 = text;
        }
    }
}

fn build(sheets: u32, rows: u32, big: u32) -> Workbook {
    let mut wb = Workbook::new();
    wb.sheets.clear();
    for s in 0..=sheets {
        let (name, rows) = if s == sheets { ("Big tab".to_string(), big) } else { (format!("Tab {}", s + 1), rows) };
        if rows == 0 {
            continue;
        }
        let mut sh = Sheet::new(name);
        for r in 0..rows {
            for c in 0..8 {
                let v = f64::from((r * 31 + c * 7 + s) % 1000) / 10.0;
                sh.cells.set(CellRef::new(r, c), Cell::value(Value::Number(v)));
            }
            sh.cells.set(CellRef::new(r, 8), Cell::value(Value::Text(Arc::from(format!("Item {r} on tab {s}")))));
            sh.cells.set(CellRef::new(r, 9), Cell::formula(Formula::new(&format!("=SUM(A{n}:H{n})", n = r + 1))));
        }
        wb.sheets.push(Arc::new(sh));
    }
    wb
}

fn main() {
    let (sheets, rows, big) = (env("BENCH_SHEETS", 30), env("BENCH_ROWS", 3_400), env("BENCH_BIG_ROWS", 100_000));
    let cells = (u64::from(sheets) * u64::from(rows) + u64::from(big)) * 10;
    let mut rep = Report(Vec::new());

    let mut wb = rep.time("Build the workbook in memory", || build(sheets, rows, big));
    rep.time("Calculate every formula", || gridcraft_engine::calc::Calc::new().recalc_all(&mut wb));
    let xlsx = rep.time("Save as .xlsx", || gridcraft_engine::io::save_bytes(&wb, "bench.xlsx").expect("save"));
    rep.note(format!("{:.1} MB", xlsx.len() as f64 / 1e6));
    // BENCH_SAVE=path keeps the file, to try it in the app.
    if let Ok(path) = std::env::var("BENCH_SAVE") {
        let _ = std::fs::write(path, &xlsx);
    }
    let opened = rep.time("Open the .xlsx (as the browser and server do)", || gridcraft_engine::io::open_bytes("bench.xlsx", &xlsx).expect("open").0);
    let json = rep.time("Whole workbook as JSON (the server's Welcome)", || starsheets_sync::workbook_json(&opened));
    rep.note(format!("{:.1} MB", json.get().len() as f64 / 1e6));
    let packed = rep.time("Compress it for the wire", || starsheets_sync::pack(json.get()).expect("big"));
    rep.note(format!("{:.1} MB", packed.len() as f64 / 1e6));
    rep.time("Uncompress it (the tab)", || starsheets_sync::unpack(&packed).expect("unpack"));
    drop(packed);
    let spills = starsheets_sync::workbook_spills(&opened);
    let back = rep.time("Read that JSON back (a tab that gets it whole)", || {
        starsheets_sync::workbook_from_parts(&json, &spills, Some(&opened)).expect("json")
    });
    rep.time("Fingerprint of the whole file (a tab that opened the same version)", || starsheets_sync::fingerprint(&back));
    drop(json);

    // One edit, as GridCraft makes it (only that cell's band is copied): on a middle tab, then on
    // the big tab.
    let before = Arc::new(back);
    let edit = |i: usize, row: u32| {
        let mut after = (*before).clone();
        if let Some(sh) = after.sheet_mut(i) {
            sh.cells.set(CellRef::new(row, 0), Cell::value(Value::Number(42.0)));
        }
        Arc::new(after)
    };
    let after = edit((sheets / 2) as usize, rows / 2);
    let fp = rep.time("Fingerprint of one edit (every edit, browser + server)", || starsheets_sync::change_fingerprint(&before, &after));
    if big > 0 {
        let on_big = edit(before.sheets.len() - 1, big / 2);
        rep.time("Fingerprint of one edit on the big tab", || starsheets_sync::change_fingerprint(&before, &on_big));
    }
    rep.time("Fingerprint of the whole file (every 25 edits, every tab)", || starsheets_sync::fingerprint(&after));
    let patch = rep.time("Patch for an undo of that edit (one tab sent whole)", || starsheets_sync::make_patch(&before, &after));
    rep.time("Apply that patch (undo in each tab)", || starsheets_sync::apply_patch(&before, &patch).expect("patch"));

    let tabs = sheets + u32::from(big > 0);
    let mut out = format!("### StarSheets speed check: {tabs} tabs, {cells} cells\n\n| Step | ms | Size |\n|---|---:|---|\n");
    for (what, ms, note) in &rep.0 {
        out.push_str(&format!("| {what} | {ms:.1} | {note} |\n"));
    }
    out.push_str(&format!("\nMost memory used: **{} MB**. Edit fingerprint {fp}.\n", peak_mb()));
    println!("{out}");
    if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = f.write_all(out.as_bytes());
        }
    }
}
