//! Live editing, shared by the server and the web app (docs/live-editing.md).
//!
//! The server holds the true copy of an open file and applies changes in the order they arrive.
//! A change is normally a GridCraft command with the sheet, selection and input locale it ran
//! with ([`Change::Cmd`]); the server runs the same engine, so replaying gives the same result,
//! which the change's fingerprint checks. A command that depends on more than that (a paste reads
//! the tab's own clipboard) travels as the finished state of the sheets it changed
//! ([`Change::State`]).

use std::sync::Arc;

use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::{Cell, Sheet, Workbook};
use gridcraft_engine::selection::Selection;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Tabs and server must run the same engine. CI sets STARSHEETS_BUILD to the commit for both.
/// How tabs and the server talk. Bump it when a message changes shape: only then must open tabs
/// reload. (A tab on another build of the same protocol keeps working: replayed changes are
/// checked by fingerprint, and a difference falls back to sending the finished state.) Also bump
/// it when fingerprints are worked out differently (4: only a change's differing cells, no JSON
/// per cell), or every change from an older tab would fall back to the finished state.
/// 5: Hello says which version of the file the tab already has; Welcome can then leave the
/// workbook out; spilled array values travel with whole workbooks and sheets.
/// 6: a formula whose result alone moved (NOW()) is left out of an edit's fingerprint.
/// 7: messages from `PACK_FROM` bytes on travel compressed as binary frames.
/// 8: the server names each connection (Welcome `you`; Applied, People and Presence use that
/// name), so one tab can't pass itself off as another; `NoAccess` when the file stops being
/// shared with you while it's open.
pub const PROTOCOL: u32 = 8;

pub const BUILD: &str = match option_env!("STARSHEETS_BUILD") {
    Some(b) => b,
    None => "dev",
};

// ---------------------------------------------------------------- messages

// Messages are adjacently tagged (`{"t": "welcome", "c": {…}}`): with the tag first, serde reads
// the content straight from the text, so a whole workbook or sheet can stay raw JSON text
// ([`Whole`]) instead of a `serde_json::Value` tree, which took about 1.5 KB a cell (protocol 3).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", content = "c", rename_all = "snake_case")]
pub enum ToServer {
    /// First message: the file (its id), the tab's build and a random id for this tab. The id
    /// only lets the same person's tab undo its own earlier changes after reconnecting; the
    /// server names the connection itself (Welcome `you`, protocol 8).
    Hello {
        file: String,
        build: String,
        client: String,
        /// PROTOCOL (0 from tabs older than it).
        #[serde(default)]
        protocol: u32,
        /// The version (ETag) of the file this tab opened and hasn't changed since: when the
        /// server's copy is that version, Welcome doesn't send the workbook again.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        have: Option<String>,
    },
    /// A change made on top of the server's revision `base`.
    Change {
        cseq: u64,
        base: u64,
        change: Change,
    },
    Undo,
    Redo,
    /// The tab's copy doesn't match the server's: send the whole file again.
    Resync,
    /// Where this tab is (its sheet and selection), for other people's screens.
    Presence {
        sheet: usize,
        sel: Selection,
    },
}

/// Someone with the file open: one entry per tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub client: String,
    pub username: String,
    pub name: String,
    pub colour: String,
}

/// The last change to a file: who (their display name) and when (Unix seconds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LastEdit {
    pub by: String,
    pub at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t", content = "c", rename_all = "snake_case")]
pub enum ToClient {
    /// The file as the server has it, at revision `seq`. No `workbook` when the tab already has
    /// it (its Hello `have`): then `hash` is its fingerprint, to check that's so.
    Welcome {
        seq: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workbook: Option<Whole>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        spills: Vec<SheetSpill>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hash: Option<String>,
        /// False for someone who may only view the file.
        #[serde(default = "yes")]
        can_edit: bool,
        /// Who changed the file last and when, where known (the title row's save symbol).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_edit: Option<LastEdit>,
        /// This connection's name, made by the server: `Applied`, `People` and `Presence` call
        /// this tab that (protocol 8). A Welcome can come again on an open connection when what
        /// you may do changes (your sharing was changed to view only, or back).
        #[serde(default)]
        you: String,
    },
    /// A change everyone applies (also the sender's acknowledgement).
    Applied { seq: u64, client: String, cseq: u64, change: Change },
    /// The server couldn't apply this change; the tab drops it.
    Rejected { cseq: u64, why: String },
    /// Replaying gave a different result: send this change as its finished state, then the
    /// changes after it again.
    NeedState { cseq: u64 },
    /// Replace the whole file (undo, redo, or a tab that fell out of step).
    Reset {
        seq: u64,
        workbook: Whole,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        spills: Vec<SheetSpill>,
    },
    /// The fingerprint of the whole file at `seq`, to check against.
    Check { seq: u64, hash: String },
    /// Saved to the server's folder (Unix seconds).
    Saved { at: i64 },
    /// Something to tell the person ("Can't undo: …").
    Message { text: String },
    /// The file was renamed or moved (by anyone, here or elsewhere): its name and folder now.
    /// `binned`: it went to the bin (changes aren't saved until it's restored).
    Moved { name: String, folder: String, binned: bool },
    /// The tab runs a different build than the server: reload the page.
    Reload,
    /// Everyone who has the file open now (this tab included).
    People { people: Vec<Person>, you: String },
    /// Where someone is.
    Presence { client: String, sheet: usize, sel: Selection },
    /// You may not open this file (any more): the tab stops connecting and keeps what it shows.
    NoAccess { text: String },
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "k", content = "c", rename_all = "snake_case")]
pub enum Change {
    Cmd { sheet: usize, sel: Selection, id: String, params: Json, locale: InputLocale, fp: String },
    State { patch: Patch },
}

/// A workbook (or sheet) as JSON text, sent and read as it is: never a `serde_json::Value`.
pub type Whole = Box<serde_json::value::RawValue>;

/// The finished state of a change: the workbook's parts other than sheets, and each sheet either
/// unchanged (`keep`: its index in the workbook the change was made on) or in full.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Patch {
    pub rest: Whole,
    pub sheets: Vec<PatchSheet>,
    /// What array formulas spill on the sheets sent whole (JSON leaves it out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spills: Vec<SheetSpill>,
}

/// The values array formulas spill on one sheet (`sheet`: its index), which a workbook's JSON
/// leaves out (it's calculation state). Sent beside a whole workbook or sheet, so the tab that
/// gets it shows them without calculating the whole file again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SheetSpill {
    pub sheet: usize,
    pub values: Vec<(CellRef, gridcraft_engine::core::Value)>,
    pub ranges: Vec<(CellRef, gridcraft_engine::core::RangeRef)>,
}

fn spill_of(i: usize, sh: &Sheet) -> Option<SheetSpill> {
    (!sh.spill.is_empty() || !sh.spill_ranges.is_empty()).then(|| SheetSpill {
        sheet: i,
        values: sh.spill.iter().map(|(k, v)| (*k, v.clone())).collect(),
        ranges: sh.spill_ranges.iter().map(|(k, r)| (*k, *r)).collect(),
    })
}

/// What every sheet of `wb` spills.
pub fn workbook_spills(wb: &Workbook) -> Vec<SheetSpill> {
    wb.sheets.iter().enumerate().filter_map(|(i, sh)| spill_of(i, sh)).collect()
}

fn put_spills(wb: &mut Workbook, spills: &[SheetSpill]) {
    for s in spills {
        if let Some(sh) = wb.sheet_mut(s.sheet) {
            sh.spill = s.values.iter().cloned().collect();
            sh.spill_ranges = s.ranges.iter().copied().collect();
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchSheet {
    Keep(usize),
    Sheet(Whole),
}

fn whole<T: Serialize + ?Sized>(v: &T) -> Whole {
    serde_json::value::to_raw_value(v).unwrap_or_else(|_| serde_json::value::RawValue::from_string("null".into()).expect("null"))
}

/// How typed input is read (dates, number separators): the person's setting, which is
/// thread-local in GridCraft, so it travels with each command.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputLocale {
    pub day_first: bool,
    pub date: String,
    pub time: String,
    pub number: Option<String>,
    pub decimal: Option<char>,
    pub group: Option<char>,
}

thread_local! {
    static LOCALE: std::cell::RefCell<InputLocale> = std::cell::RefCell::new(InputLocale::default());
}

impl InputLocale {
    /// Hands the locale to GridCraft's input reader and remembers it as this thread's.
    pub fn apply(&self) {
        use gridcraft_engine::core::parse;
        let (date, time) = (Some(self.date.as_str()).filter(|s| !s.is_empty()), Some(self.time.as_str()).filter(|s| !s.is_empty()));
        let datetime = date.zip(time).map(|(d, t)| format!("{d} {t}"));
        parse::set_input_locale(self.day_first, date, time, datetime.as_deref(), self.number.as_deref());
        if let (Some(d), Some(g)) = (self.decimal, self.group) {
            parse::set_number_separators(d, g);
        }
        LOCALE.with(|l| *l.borrow_mut() = self.clone());
    }
    /// The locale last applied on this thread.
    pub fn current() -> InputLocale {
        LOCALE.with(|l| l.borrow().clone())
    }
}

// ---------------------------------------------------------------- which commands replay

/// Commands that read more than the workbook, sheet, selection and locale, so they travel as
/// their finished state. Anything missed here is caught by the fingerprint check.
pub fn replayable(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    !["paste", "copied", "formatpainter", "autofit", "picture", "file.", "import", "fromtext", "script", "macro"].iter().any(|w| id.contains(w))
}

/// Whether the server may run this command for a tab: only undoable editing commands, and never
/// one that reaches files, the network, macros, scripts or printing (a tab sends anything else as
/// its finished state, which is only data).
pub fn server_allows(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    let denied = [
        "file.",
        "macro",
        "automate.",
        "script",
        "python",
        "control.",
        "app.",
        "window.",
        "print",
        "export",
        "link.open",
        "data.get",
        "data.refresh",
        "data.from",
        "data.query",
        "picture",
        "object",
        "addin",
        "options",
        "solver",
        "equation",
    ];
    gridcraft_engine::find_command(id).is_some_and(|spec| spec.undoable) && !denied.iter().any(|d| lower.contains(d))
}

// ---------------------------------------------------------------- applying changes

/// Runs a command the way it ran where it was made: on `sheet`, with `sel` and `locale`, without
/// adding an undo step or a sync step here. This tab's own sheet, selection, locale and history
/// come back afterwards.
pub fn apply_cmd(s: &mut Session, sheet: usize, sel: &Selection, id: &str, params: &Json, locale: &InputLocale) -> Result<(), String> {
    let log = s.sync_log.take();
    let requests = s.ui_requests.len();
    let Some(d) = s.active_mut() else {
        s.sync_log = log;
        return Err("no workbook".into());
    };
    let (view_sheet, view_sel, undo, redo) = (d.wb.active_sheet(), d.selection.clone(), d.undo.clone(), d.redo.clone());
    d.wb.set_active_sheet(sheet.min(d.wb.sheets.len().saturating_sub(1)));
    d.selection = sel.clone();
    let own = InputLocale::current();
    let switch = own != *locale;
    if switch {
        locale.apply();
    }
    // Someone else's change is applied in a view-only tab too.
    let read_only = std::mem::take(&mut s.read_only);
    let result = s.execute(id, params.clone());
    s.read_only = read_only;
    if switch {
        own.apply();
    }
    s.ui_requests.truncate(requests);
    if let Some(d) = s.active_mut() {
        d.undo = undo;
        d.redo = redo;
        d.wb.set_active_sheet(view_sheet.min(d.wb.sheets.len().saturating_sub(1)));
        d.selection = view_sel;
    }
    s.sync_log = log;
    result.map(|_| ()).map_err(|e| e.to_string())
}

/// Applies any change to the session's active workbook.
pub fn apply(s: &mut Session, change: &Change) -> Result<(), String> {
    match change {
        Change::Cmd { sheet, sel, id, params, locale, .. } => apply_cmd(s, *sheet, sel, id, params, locale),
        Change::State { patch } => {
            let current = s.active().map(|d| d.wb.clone()).ok_or("no workbook")?;
            // The sender's finished state: values (spilled ones too) as it calculated them, and
            // every sheet whose values its change moved comes whole, so nothing to recalculate.
            let wb = apply_patch(&current, patch)?;
            set_workbook(s, wb);
            Ok(())
        }
    }
}

/// Puts `wb` in place of the active workbook, keeping this tab's sheet and selection.
pub fn set_workbook(s: &mut Session, wb: Workbook) {
    if let Some(d) = s.active_mut() {
        let sheet = d.wb.active_sheet().min(wb.sheets.len().saturating_sub(1));
        wb.set_active_sheet(sheet);
        d.wb = Arc::new(wb);
        d.calc.rebuild(&d.wb);
    }
}

/// Puts an `Arc` workbook back (a copy this side already had), keeping sheet and selection.
pub fn restore_workbook(s: &mut Session, wb: &Arc<Workbook>) {
    if let Some(d) = s.active_mut() {
        let sheet = d.wb.active_sheet().min(wb.sheets.len().saturating_sub(1));
        wb.set_active_sheet(sheet);
        d.wb = wb.clone();
        d.calc.rebuild(&d.wb);
    }
}

// ---------------------------------------------------------------- undo of one change among others

/// Why an undo (or redo) can't be done, in plain words.
pub const UNDO_CHANGED: &str = "Can't undo: someone has changed those cells since.";
pub const UNDO_LAYOUT: &str =
    "Can't undo that change any more: it changed more than cells (rows, columns, sheets or rules) and someone has edited the file since.";

/// Two cells hold the same thing (a formula's result may differ: it follows other cells).
fn same_cell(a: Option<&gridcraft_engine::model::Cell>, b: Option<&gridcraft_engine::model::Cell>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.style == b.style && a.formula == b.formula && a.py == b.py && (a.formula.is_some() || a.value == b.value),
        (None, None) => true,
        (Some(c), None) | (None, Some(c)) => {
            c.formula.is_none()
                && c.py.is_none()
                && matches!(c.value, gridcraft_engine::core::Value::Empty)
                && c.style == gridcraft_engine::model::StyleId::DEFAULT
        }
    }
}

/// Puts back what one change (`from` → `to`) did to cells, on top of later changes by others:
/// each cell it changed goes back to its `from` state, provided nobody has changed that cell
/// since (it still holds its `to` state). Only a change that touched nothing but cells (values,
/// formulas, formats) can be undone this way. Undo is (before, after); redo is (after, before).
pub fn undo_cells(from: &Workbook, to: &Workbook, current: &Workbook) -> Result<Workbook, &'static str> {
    if from.sheets.len() != to.sheets.len() || from.sheets.iter().zip(&to.sheets).any(|(a, b)| a.name != b.name) {
        return Err(UNDO_LAYOUT);
    }
    // Nothing outside the sheets may differ, except styles the change added (they're kept).
    let outside = |w: &Workbook| {
        let mut c = w.clone();
        c.sheets = Vec::new();
        c.styles = from.styles.clone();
        c.active_sheet = Default::default();
        c
    };
    if outside(from) != outside(to) {
        return Err(UNDO_LAYOUT);
    }
    let mut out = current.clone();
    for (a, b) in from.sheets.iter().zip(&to.sheets) {
        if Arc::ptr_eq(a, b) {
            continue;
        }
        // Only cells: the sheet with `from`'s cells (and results that follow them) must equal `from`.
        let mut only_cells = (**b).clone();
        only_cells.cells = a.cells.clone();
        only_cells.spill = a.spill.clone();
        only_cells.spill_ranges = a.spill_ranges.clone();
        if only_cells != **a {
            return Err(UNDO_LAYOUT);
        }
        let Some(j) = current.sheet_index(&b.name) else { return Err(UNDO_LAYOUT) };
        let touched = a.cells.diff(&b.cells);
        let now = &current.sheets[j];
        if touched.iter().any(|c| !same_cell(now.cells.get(*c), b.cells.get(*c))) {
            return Err(UNDO_CHANGED);
        }
        let sh = Arc::make_mut(&mut out.sheets[j]);
        for c in touched {
            match a.cells.get(c) {
                Some(cell) => sh.cells.set(c, cell.clone()),
                None => {
                    sh.cells.remove(c);
                }
            }
        }
    }
    Ok(out)
}

/// Puts `wb` in place (keeping this side's sheet and selection) and recalculates everything.
pub fn set_workbook_recalc(s: &mut Session, wb: Workbook) {
    set_workbook(s, wb);
    if let Some(d) = s.active_mut() {
        let (wb, calc) = (&mut d.wb, &mut d.calc);
        calc.recalc_all(Arc::make_mut(wb));
    }
}

// ---------------------------------------------------------------- whole workbooks and patches

/// What JSON doesn't carry (unmodelled parts of the .xlsx, macros) comes from `from`.
fn keep_unserialized(wb: &mut Workbook, from: &Workbook) {
    for i in 0..wb.sheets.len() {
        keep_sheet_unserialized(wb, i, from);
    }
    keep_workbook_unserialized(wb, from);
}

fn keep_workbook_unserialized(wb: &mut Workbook, from: &Workbook) {
    wb.preserved = from.preserved.clone();
    wb.source_dir = from.source_dir.clone();
    wb.source_path = from.source_path.clone();
    wb.region = from.region;
    wb.macros = from.macros.clone();
    wb.macros_unread = from.macros_unread.clone();
    wb.vba_project = from.vba_project.clone();
}

/// A sheet that came as JSON gets back what JSON doesn't carry (its unmodelled file content and
/// the file's sheet id) from the sheet of the same name in `from`: without them, saving dropped
/// whatever GridCraft keeps unread for that sheet.
fn keep_sheet_unserialized(wb: &mut Workbook, i: usize, from: &Workbook) {
    let Some(name) = wb.sheets.get(i).map(|sh| sh.name.clone()) else { return };
    let Some(old) = from.sheets.iter().find(|o| o.name == name) else { return };
    if old.preserved.is_none() && old.file_id == 0 {
        return;
    }
    let (preserved, file_id) = (old.preserved.clone(), old.file_id);
    if let Some(sh) = wb.sheet_mut(i) {
        if sh.preserved.is_none() {
            sh.preserved = preserved;
        }
        if sh.file_id == 0 {
            sh.file_id = file_id;
        }
    }
}

pub fn workbook_json(wb: &Workbook) -> Whole {
    whole(wb)
}

pub fn workbook_from_json(v: &Whole, current: Option<&Workbook>) -> Result<Workbook, String> {
    let mut wb: Workbook = serde_json::from_str(v.get()).map_err(|e| format!("bad workbook: {e}"))?;
    if let Some(c) = current {
        keep_unserialized(&mut wb, c);
    }
    Ok(wb)
}

/// A whole workbook from the server with what its formulas spill: complete, nothing to
/// recalculate (protocol 5; before, every tab recalculated the whole file on joining).
pub fn workbook_from_parts(v: &Whole, spills: &[SheetSpill], current: Option<&Workbook>) -> Result<Workbook, String> {
    let mut wb = workbook_from_json(v, current)?;
    put_spills(&mut wb, spills);
    Ok(wb)
}

/// [`workbook_from_json`], recalculated: what a tab shows when the server sends it the whole
/// file. JSON doesn't carry the values array formulas spill (`{A2:C2;SORTN(…)}` would show only
/// its first cell until an edit reached it), so they are worked out again here, as opening a
/// file does. Formula results never count in fingerprints, so this can't upset the check.
pub fn workbook_from_json_recalc(v: &Whole, current: Option<&Workbook>) -> Result<Workbook, String> {
    let mut wb = workbook_from_json(v, current)?;
    gridcraft_engine::calc::Calc::new().recalc_all(&mut wb);
    Ok(wb)
}

/// The finished state of what a change altered: sheets it changed in full, the others as
/// `{"keep": i}` (sheet `i` of the workbook it was made on), and the workbook's other parts.
pub fn make_patch(before: &Workbook, after: &Workbook) -> Patch {
    let mut rest = after.clone();
    rest.sheets = Vec::new();
    let mut spills = Vec::new();
    let sheets = after
        .sheets
        .iter()
        .enumerate()
        .map(|(i, sh)| match before.sheets.iter().position(|b| Arc::ptr_eq(b, sh)) {
            Some(k) => PatchSheet::Keep(k),
            None => {
                spills.extend(spill_of(i, sh));
                PatchSheet::Sheet(whole(&**sh))
            }
        })
        .collect();
    Patch { rest: whole(&rest), sheets, spills }
}

pub fn apply_patch(current: &Workbook, patch: &Patch) -> Result<Workbook, String> {
    let mut wb: Workbook = serde_json::from_str(patch.rest.get()).map_err(|e| format!("bad change: {e}"))?;
    let mut sheets = Vec::new();
    for e in &patch.sheets {
        match e {
            PatchSheet::Keep(i) => sheets.push(current.sheets.get(*i).cloned().ok_or("the change refers to a sheet that isn't there")?),
            PatchSheet::Sheet(raw) => {
                let sh: Sheet = serde_json::from_str(raw.get()).map_err(|e| format!("bad sheet: {e}"))?;
                sheets.push(Arc::new(sh));
            }
        }
    }
    if sheets.is_empty() {
        return Err("a workbook needs a sheet".into());
    }
    wb.sheets = sheets;
    let kept = |i: usize| matches!(patch.sheets.get(i), Some(PatchSheet::Keep(_)));
    for i in (0..wb.sheets.len()).filter(|i| !kept(*i)) {
        keep_sheet_unserialized(&mut wb, i, current);
    }
    keep_workbook_unserialized(&mut wb, current);
    put_spills(&mut wb, &patch.spills);
    Ok(wb)
}

// ---------------------------------------------------------------- big messages on the wire

/// Messages at least this long travel compressed, as a binary WebSocket frame (protocol 7);
/// shorter ones stay text frames. A whole workbook is mostly repeated JSON keys, so it shrinks a
/// lot (140 MB to a few MB for two million cells), which matters most away from home.
pub const PACK_FROM: usize = 64 * 1024;

/// `text` compressed for a binary frame, when it is long enough to be worth it.
pub fn pack(text: &str) -> Option<Vec<u8>> {
    use std::io::Write as _;
    if text.len() < PACK_FROM {
        return None;
    }
    let mut e = flate2::write::DeflateEncoder::new(Vec::with_capacity(text.len() / 8), flate2::Compression::fast());
    e.write_all(text.as_bytes()).ok()?;
    e.finish().ok()
}

/// The biggest frame the server takes from a tab (compressed or not); bigger ones end the
/// connection. A tab's biggest message is a change sent as its finished state: the sheets it
/// changed, in full. Two million cells are 140 MB of JSON and 15.5 MB packed, so 16 MB carries a
/// change to a sheet of about two million cells, more than a browser tab edits comfortably.
/// (The server's own messages to tabs, such as a whole workbook, aren't limited by this.)
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// The longest message a packed frame may unpack to. Deflate can grow up to about a thousand
/// times, so a 16 MB frame could otherwise become 16 GB on the server. 256 MB is about 3.5
/// million cells of JSON, above what a full 16 MB frame of ordinary cells unpacks to (about
/// 145 MB); only very repetitive sheets pack smaller, and the tab checks both limits first.
pub const MAX_UNPACKED: usize = 256 * 1024 * 1024;
// A full frame of ordinary cells must unpack within the limit.
const _: () = assert!(MAX_UNPACKED > MAX_FRAME * 9);

/// The text of a binary frame made by [`pack`]; refused when it unpacks to more than
/// [`MAX_UNPACKED`] bytes.
pub fn unpack(bytes: &[u8]) -> Result<String, String> {
    unpack_at_most(bytes, MAX_UNPACKED)
}

fn unpack_at_most(bytes: &[u8], limit: usize) -> Result<String, String> {
    use std::io::Read as _;
    let mut out = String::new();
    flate2::read::DeflateDecoder::new(bytes).take(limit as u64 + 1).read_to_string(&mut out).map_err(|e| format!("unreadable message: {e}"))?;
    if out.len() > limit {
        return Err(format!("message too big: it unpacks to more than {} MB", limit / (1024 * 1024)));
    }
    Ok(out)
}

/// Whether a message of this text fits what the server takes (see [`MAX_FRAME`]): the tab
/// checks before sending, so a change too big to send is dropped and said, not sent over and
/// over (the server would end the connection each time, and the tab would send it again).
pub fn fits(text: &str, packed: Option<&[u8]>) -> bool {
    text.len() <= MAX_UNPACKED && packed.map_or(text.len(), <[u8]>::len) <= MAX_FRAME
}

// ---------------------------------------------------------------- fingerprints

struct Fnv(u64);

impl Fnv {
    fn new() -> Fnv {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn bytes(&mut self, b: &[u8]) {
        for x in b {
            self.0 ^= u64::from(*x);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn json<T: Serialize + ?Sized>(&mut self, v: &T) {
        self.bytes(&serde_json::to_vec(v).unwrap_or_default());
    }
}

impl Fnv {
    fn u32(&mut self, x: u32) {
        self.bytes(&x.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.bytes(&(s.len() as u64).to_le_bytes());
        self.bytes(s.as_bytes());
    }
}

/// One cell, hashed field by field (no JSON per cell: a big sheet has millions). A formula's
/// result can differ from tab to tab (NOW, RAND): only its text counts.
fn hash_cell(h: &mut Fnv, at: CellRef, cell: Option<&Cell>) {
    use gridcraft_engine::core::Value;
    h.u32(at.row);
    h.u32(at.col);
    let Some(cell) = cell else {
        h.bytes(b"-");
        return;
    };
    h.u32(cell.style.0);
    if let Some(f) = &cell.formula {
        h.bytes(b"=");
        h.str(&f.text);
        if let Some(r) = &f.array {
            h.json(r);
        }
        return;
    }
    if let Some(py) = &cell.py {
        h.bytes(b"p");
        h.json(&**py);
        return;
    }
    match &cell.value {
        Value::Empty => h.bytes(b"e"),
        Value::Number(n) => {
            h.bytes(b"n");
            h.bytes(&n.to_bits().to_le_bytes());
        }
        Value::Text(t) => {
            h.bytes(b"t");
            h.str(t);
        }
        Value::Bool(b) => h.bytes(if *b { b"T" } else { b"F" }),
        Value::Error(e) => {
            h.bytes(b"!");
            h.json(e);
        }
        Value::Array(_) => h.json(&cell.value),
    }
}

/// A sheet's name, merges and row and column sizes.
fn hash_sheet_shape(h: &mut Fnv, i: usize, sh: &Sheet) {
    h.bytes(&(i as u64).to_le_bytes());
    h.str(&sh.name);
    h.json(&sh.merges);
    h.json(&sh.cols);
    h.json(&sh.rows);
}

fn hash_sheet(h: &mut Fnv, i: usize, sh: &Sheet) {
    hash_sheet_shape(h, i, sh);
    for (at, cell) in sh.cells.iter() {
        hash_cell(h, at, Some(cell));
    }
}

/// A sheet a change altered, given the sheet it was before: only the cells that differ, which
/// `CellStore::diff` finds by visiting just the 64-row bands the change copied.
fn hash_sheet_change(h: &mut Fnv, i: usize, before: &Sheet, after: &Sheet) {
    h.bytes(&(i as u64).to_le_bytes());
    h.str(&after.name);
    // Only what changed: hashing a big sheet's row sizes on every edit would cost as much as
    // the cells did.
    if after.merges != before.merges {
        h.json(&after.merges);
    }
    if after.cols != before.cols {
        h.json(&after.cols);
    }
    if after.rows != before.rows {
        h.json(&after.rows);
    }
    for at in after.cells.diff(&before.cells) {
        let (b, a) = (before.cells.get(at), after.cells.get(at));
        // A formula whose result alone moved (NOW(), RAND(), or recalculated after the edit)
        // isn't part of the change: results differ from tab to tab with the clock.
        if let (Some(b), Some(a)) = (b, a)
            && b.style == a.style
            && let (Some(fb), Some(fa)) = (&b.formula, &a.formula)
            && fb.text == fa.text
            && fb.array == fa.array
        {
            continue;
        }
        hash_cell(h, at, a);
    }
}

/// Fingerprint of what a change altered: in each sheet it changed, the cells that differ (a
/// sheet it added, in full), the sheet names, the defined names and the size of the style table.
/// The tab and the server each work it out from their own copy; equal means the change did the
/// same thing on both.
pub fn change_fingerprint(before: &Workbook, after: &Workbook) -> String {
    let mut h = Fnv::new();
    for (i, sh) in after.sheets.iter().enumerate() {
        h.str(&sh.name);
        if before.sheets.iter().any(|b| Arc::ptr_eq(b, sh)) {
            continue;
        }
        match before.sheets.iter().find(|b| b.uid.get() == sh.uid.get()) {
            Some(b) => hash_sheet_change(&mut h, i, b, sh),
            None => hash_sheet(&mut h, i, sh),
        }
    }
    h.json(&after.names);
    h.bytes(&(after.styles.len() as u64).to_le_bytes());
    format!("{:016x}", h.0)
}

/// Fingerprint of the whole workbook.
pub fn fingerprint(wb: &Workbook) -> String {
    let mut h = Fnv::new();
    for (i, sh) in wb.sheets.iter().enumerate() {
        hash_sheet(&mut h, i, sh);
    }
    h.json(&wb.names);
    h.bytes(&(wb.styles.len() as u64).to_le_bytes());
    format!("{:016x}", h.0)
}

#[cfg(test)]
mod tests;

#[cfg(feature = "thumbs")]
pub mod thumb;

/// Copies sheet `index` of `src` to the end of `dst` (another workbook): cell, row and column
/// formats are added to `dst`'s own style list, the copy gets a free name ("Name (2)" when taken),
/// a new id, and table names that don't clash. Formulas that point at other sheets keep their
/// text (they may show #REF! where those sheets don't exist), as in Sheets. Returns its name.
pub fn copy_sheet_into(src: &Workbook, index: usize, dst: &mut Workbook) -> Option<String> {
    use gridcraft_engine::model::StyleId;
    let mut sheet: Sheet = (**src.sheets.get(index)?).clone();
    let mut map: std::collections::HashMap<StyleId, StyleId> = std::collections::HashMap::new();
    let mut remap = |id: StyleId, dst: &mut Workbook| {
        *map.entry(id).or_insert_with(|| if id == StyleId::DEFAULT { StyleId::DEFAULT } else { dst.styles.intern(src.styles.get(id).clone()) })
    };
    let styled: Vec<(gridcraft_engine::core::CellRef, StyleId)> =
        sheet.cells.iter().filter(|(_, c)| c.style != StyleId::DEFAULT).map(|(r, c)| (r, c.style)).collect();
    for (r, id) in styled {
        let new = remap(id, dst);
        sheet.set_style(r, new);
    }
    for line in sheet.rows.values_mut().chain(sheet.cols.values_mut()) {
        if let Some(id) = line.style {
            line.style = Some(remap(id, dst));
        }
    }
    let taken = |n: &str, dst: &Workbook| dst.sheets.iter().any(|s| s.name.eq_ignore_ascii_case(n));
    let base: String = sheet.name.chars().take(26).collect();
    let mut name = sheet.name.clone();
    let mut k = 2;
    while taken(&name, dst) {
        name = format!("{base} ({k})");
        k += 1;
    }
    sheet.name = name.clone();
    sheet.uid = gridcraft_engine::model::SheetUid::fresh();
    let tables: Vec<String> = dst.sheets.iter().flat_map(|s| s.tables.iter().map(|t| t.name.to_lowercase())).collect();
    for t in &mut sheet.tables {
        let mut n = t.name.clone();
        let mut k = 2;
        while tables.contains(&n.to_lowercase()) {
            n = format!("{}_{k}", t.name);
            k += 1;
        }
        t.name = n;
    }
    dst.sheets.push(Arc::new(sheet));
    Some(name)
}

// ---------------------------------------------------------------- the sheet a file reopens on

/// The sheet to open a file on for someone who was last on the sheet with file id `file_id`
/// (0: none known) and name `name`: that sheet by its id, else by its name (a sheet added since
/// the file was opened has no id yet), else the first sheet shown. Hidden sheets are skipped.
pub fn pick_sheet(wb: &Workbook, file_id: u32, name: &str) -> usize {
    let shown = |sh: &Sheet| sh.visibility == gridcraft_engine::model::Visibility::Visible;
    let find = |f: &dyn Fn(&Sheet) -> bool| wb.sheets.iter().position(|sh| shown(sh) && f(sh));
    (if file_id != 0 { find(&|sh| sh.file_id == file_id) } else { None })
        .or_else(|| find(&|sh| sh.name == name))
        .or_else(|| find(&|_| true))
        .unwrap_or(0)
}

// ---------------------------------------------------------------- Clip in StarSheets' section

/// The key of Google Sheets' "Clip" text wrapping in StarSheets' section of a file: Excel has no
/// such setting, so the cells that have it are listed there (`[{"sheet", "ranges"}]`).
const CLIP_KEY: &str = "clip";

/// Before writing a file: lists the cells whose style clips their text in the section (and drops
/// the list when there are none).
pub fn clip_into_host(wb: &mut Workbook) {
    let mut entries = Vec::new();
    for sh in &wb.sheets {
        let cells: Vec<gridcraft_engine::core::CellRef> =
            sh.cells.iter().filter(|(_, c)| wb.styles.get(c.style).align.clip).map(|(at, _)| at).collect();
        if !cells.is_empty() {
            entries.push(serde_json::json!({ "sheet": sh.name, "ranges": runs(&cells) }));
        }
    }
    let mut host: serde_json::Map<String, Json> = wb.host_data.as_deref().and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
    if entries.is_empty() {
        if host.remove(CLIP_KEY).is_none() {
            return;
        }
    } else {
        host.insert(CLIP_KEY.into(), Json::Array(entries));
        host.entry("v").or_insert(Json::from(1));
    }
    let only_version = host.keys().all(|k| k == "v");
    wb.host_data = if only_version { None } else { serde_json::to_string(&host).ok() };
}

/// After reading a file: gives the listed cells the clip style again, and takes the list out of
/// the section (while the file is open the cells' styles say it).
pub fn clip_from_host(wb: &mut Workbook) {
    let Some(mut host) = wb.host_data.as_deref().and_then(|t| serde_json::from_str::<serde_json::Map<String, Json>>(t).ok()) else { return };
    let Some(entries) = host.remove(CLIP_KEY) else { return };
    for e in entries.as_array().into_iter().flatten() {
        let (Some(name), Some(ranges)) = (e.get("sheet").and_then(Json::as_str), e.get("ranges").and_then(Json::as_str)) else { continue };
        let Some(i) = wb.sheets.iter().position(|s| s.name == name) else { continue };
        let mut sh = (*wb.sheets[i]).clone();
        for r in ranges.split(',').filter_map(|r| gridcraft_engine::core::RangeRef::parse(r.trim())) {
            // A list from an edited copy may point anywhere: only cells that exist, and not too many.
            for at in r.iter().take(1_000_000) {
                let Some(cell) = sh.cells.get(at).cloned() else { continue };
                let mut st = wb.styles.get(cell.style).clone();
                if st.align.clip {
                    continue;
                }
                st.align.clip = true;
                st.align.wrap = false;
                let style = wb.styles.intern(st);
                sh.cells.set(at, gridcraft_engine::model::Cell { style, ..cell });
            }
        }
        wb.sheets[i] = Arc::new(sh);
    }
    let only_version = host.keys().all(|k| k == "v");
    wb.host_data = if only_version { None } else { serde_json::to_string(&host).ok() };
}

/// Cells as few ranges as simply found: runs along each row, then runs of equal row runs.
fn runs(cells: &[gridcraft_engine::core::CellRef]) -> String {
    use gridcraft_engine::core::{CellRef, RangeRef};
    let mut sorted = cells.to_vec();
    sorted.sort_by_key(|c| (c.row, c.col));
    let mut row_runs: Vec<(u32, u32, u32)> = Vec::new();
    for c in sorted {
        match row_runs.last_mut() {
            Some((r, _, end)) if *r == c.row && *end + 1 == c.col => *end = c.col,
            _ => row_runs.push((c.row, c.col, c.col)),
        }
    }
    let mut blocks: Vec<RangeRef> = Vec::new();
    for (r, a, b) in row_runs {
        match blocks.iter_mut().find(|x| x.end.row + 1 == r && x.start.col == a && x.end.col == b) {
            Some(x) => x.end.row = r,
            None => blocks.push(RangeRef::new(CellRef::new(r, a), CellRef::new(r, b))),
        }
    }
    blocks.iter().map(|r| r.a1()).collect::<Vec<_>>().join(",")
}
