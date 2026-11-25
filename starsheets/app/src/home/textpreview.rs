//! Read-only previews of text files in Drive: plain text and logs, Markdown, CSV/TSV, JSON and
//! source code. One entry point, [`show`], picks the view from the file name; [`handles`] says
//! whether a name gets a text preview at all.
//!
//! - Plain text and code: monospace, syntax colours for common languages (egui_extras' built-in
//!   highlighter, no extra crates), a "Wrap lines" switch and line numbers.
//! - Markdown: headings, lists, emphasis, code blocks, links (http/https/mailto only), tables,
//!   block quotes, task lists (pulldown-cmark does the parsing).
//! - CSV/TSV: a table with the first row as header, resizable columns, quoted fields with commas,
//!   quotes and line breaks; only the first [`MAX_ROWS`] rows are shown.
//! - JSON: pretty-printed, or a collapsible tree; invalid JSON is shown as text with a note.
//! - Text that isn't UTF-8 is decoded with replacement characters (UTF-16 with a BOM is decoded
//!   properly); files over [`MAX_BYTES`] show their first part. A note above the view says so.
//!
//! All text is selectable, and "Copy all" copies the whole (decoded) text.

use std::sync::Arc;

use egui::text::LayoutJob;
use egui::{Id, Label, RichText, ScrollArea, TextWrapMode};
use egui_extras::syntax_highlighting::{CodeTheme, highlight};

/// How much of a file is shown: the first 1 MB.
pub const MAX_BYTES: usize = 1 << 20;
/// How many CSV/TSV rows are shown (the header row included).
pub const MAX_ROWS: usize = 1000;
/// How many CSV/TSV columns are shown.
pub const MAX_COLS: usize = 256;
/// How many Markdown blocks are shown.
const MAX_BLOCKS: usize = 10_000;
/// How many JSON tree nodes are drawn.
const MAX_NODES: usize = 5_000;

/// Which view a file gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Markdown,
    /// Delimited table, with its separator.
    Table(char),
    Json,
    /// Source code, with the highlighter's language name ("" = monospace without colours).
    Code(&'static str),
}

fn kind(name: &str) -> Option<Kind> {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name).to_lowercase();
    match base.as_str() {
        "readme" | "license" | "licence" | "copying" | "authors" | "changelog" | "notice" | "todo" => return Some(Kind::Plain),
        "makefile" | "dockerfile" | "containerfile" => return Some(Kind::Code("sh")),
        _ => {}
    }
    let (_, ext) = base.rsplit_once('.')?;
    Some(match ext {
        "txt" | "text" | "log" | "out" | "ini" | "cfg" | "conf" | "properties" | "gitignore" | "gitattributes" | "editorconfig" | "env" | "srt"
        | "vtt" => Kind::Plain,
        "md" | "markdown" | "mdown" | "mkd" => Kind::Markdown,
        "csv" => Kind::Table(','),
        "tsv" | "tab" => Kind::Table('\t'),
        "json" | "geojson" | "webmanifest" => Kind::Json,
        "rs" => Kind::Code("rs"),
        "py" | "pyw" | "pyi" => Kind::Code("py"),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Kind::Code("js"),
        "sh" | "bash" | "zsh" | "fish" | "bashrc" | "zshrc" | "profile" | "rb" | "pl" | "r" | "ps1" => Kind::Code("sh"),
        "yaml" | "yml" => Kind::Code("yaml"),
        "toml" => Kind::Code("toml"),
        "sql" => Kind::Code("sql"),
        "c" | "h" => Kind::Code("c"),
        "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" => Kind::Code("cpp"),
        // C-like enough for the built-in highlighter: comments, strings, numbers.
        "java" | "go" | "cs" | "kt" | "kts" | "swift" | "scala" | "dart" | "php" | "groovy" | "gradle" | "proto" => Kind::Code("cpp"),
        "jsonc" | "json5" => Kind::Code("json"),
        "html" | "htm" | "xhtml" | "xml" | "svg" | "css" | "scss" | "sass" | "less" | "lua" | "vue" | "svelte" | "bat" | "cmd" | "tex" | "diff"
        | "patch" | "nix" | "jsonl" | "ndjson" => Kind::Code(""),
        _ => return None,
    })
}

/// Whether [`show`] has a proper view for this file name (by extension or well-known name).
pub fn handles(name: &str) -> bool {
    kind(name).is_some()
}

/// The file's bytes as text.
#[derive(Debug, Default, PartialEq)]
struct Decoded {
    text: String,
    /// Some bytes weren't valid in the encoding and became U+FFFD.
    lossy: bool,
    utf16: bool,
    /// Only the first [`MAX_BYTES`] were decoded.
    truncated: bool,
}

fn decode(bytes: &[u8]) -> Decoded {
    let truncated = bytes.len() > MAX_BYTES;
    let b = &bytes[..bytes.len().min(MAX_BYTES)];
    let mut d = Decoded { truncated, ..Decoded::default() };
    if b.starts_with(&[0xFF, 0xFE]) || b.starts_with(&[0xFE, 0xFF]) {
        let le = b[0] == 0xFF;
        let units = b[2..].as_chunks::<2>().0.iter().map(|&c| if le { u16::from_le_bytes(c) } else { u16::from_be_bytes(c) });
        d.utf16 = true;
        d.text = char::decode_utf16(units)
            .map(|r| {
                r.unwrap_or_else(|_| {
                    d.lossy = true;
                    char::REPLACEMENT_CHARACTER
                })
            })
            .collect();
        // An odd last byte is lost by chunks_exact: only a problem when the file wasn't cut.
        if !truncated && (b.len() - 2) % 2 == 1 {
            d.lossy = true;
            d.text.push(char::REPLACEMENT_CHARACTER);
        }
    } else {
        let mut b = b.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(b);
        // Cutting at 1 MB can split a character: drop the partial one rather than flag it.
        if truncated
            && let Err(e) = std::str::from_utf8(b)
            && e.error_len().is_none()
        {
            b = &b[..e.valid_up_to()];
        }
        let text = String::from_utf8_lossy(b);
        d.lossy = matches!(text, std::borrow::Cow::Owned(_));
        d.text = text.into_owned();
    }
    if d.text.contains('\r') {
        d.text = d.text.replace("\r\n", "\n");
    }
    d
}

/// Splits CSV/TSV text into rows of fields (RFC 4180 quoting), at most `max_rows` rows. The
/// flag says whether rows were left out.
fn parse_delimited(text: &str, sep: char, max_rows: usize) -> (Vec<Vec<String>>, bool) {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    // The field so far came from quotes (so an empty `""` still counts as a field).
    let mut was_quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        if c == '"' && field.is_empty() && !was_quoted {
            quoted = true;
            was_quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut field));
            was_quoted = false;
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            was_quoted = false;
            if rows.len() == max_rows {
                return (rows, true);
            }
            rows.push(std::mem::take(&mut row));
        } else {
            field.push(c);
        }
    }
    if !field.is_empty() || was_quoted || !row.is_empty() {
        row.push(field);
        if rows.len() == max_rows {
            return (rows, true);
        }
        rows.push(row);
    }
    (rows, false)
}

/// A run of Markdown text with one style.
#[derive(Debug, Clone, Default, PartialEq)]
struct Span {
    text: String,
    bold: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Block {
    /// A paragraph, heading (1–6, 0 = none) or list item (with its bullet or number).
    Text {
        spans: Vec<Span>,
        heading: u8,
        indent: u8,
        quote: bool,
        bullet: Option<String>,
    },
    Code {
        lang: String,
        text: String,
        indent: u8,
    },
    Rule,
    /// Rows of cells of spans; the first `head` rows are the header.
    Table {
        rows: Vec<Vec<Vec<Span>>>,
        head: usize,
    },
}

fn parse_markdown(text: &str) -> (Vec<Block>, bool) {
    use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

    #[derive(Default)]
    struct Md {
        blocks: Vec<Block>,
        cur: Vec<Span>,
        bold: u32,
        italic: u32,
        strike: u32,
        link: Option<String>,
        heading: u8,
        lists: Vec<Option<u64>>,
        quote: u8,
        bullet: Option<String>,
        code: Option<(String, String)>,
        table: Option<(Vec<Vec<Vec<Span>>>, usize)>,
    }
    impl Md {
        fn indent(&self) -> u8 {
            (self.lists.len() as u8).saturating_add(self.quote)
        }
        fn flush(&mut self) {
            if self.cur.is_empty() && self.bullet.is_none() {
                return;
            }
            self.blocks.push(Block::Text {
                spans: std::mem::take(&mut self.cur),
                heading: self.heading,
                indent: self.indent(),
                quote: self.quote > 0,
                bullet: self.bullet.take(),
            });
        }
        fn push(&mut self, text: &str, code: bool) {
            let span =
                Span { text: text.to_owned(), bold: self.bold > 0, italic: self.italic > 0, strike: self.strike > 0, code, link: self.link.clone() };
            if let Some((rows, _)) = &mut self.table {
                if let Some(cell) = rows.last_mut().and_then(|r| r.last_mut()) {
                    cell.push(span);
                }
                return;
            }
            self.cur.push(span);
        }
    }

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut md = Md::default();
    let mut more = false;
    for ev in Parser::new_ext(text, opts) {
        if md.blocks.len() >= MAX_BLOCKS {
            more = true;
            break;
        }
        match ev {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    if !md.cur.is_empty() {
                        md.flush();
                    }
                }
                Tag::Heading { level, .. } => {
                    md.flush();
                    md.heading = level as u8;
                }
                Tag::List(start) => {
                    md.flush();
                    md.lists.push(start);
                }
                Tag::Item => {
                    md.flush();
                    md.bullet = Some(match md.lists.last_mut() {
                        Some(Some(n)) => {
                            *n += 1;
                            format!("{}.", *n - 1)
                        }
                        _ => "•".to_owned(),
                    });
                }
                Tag::BlockQuote { .. } => {
                    md.flush();
                    md.quote = md.quote.saturating_add(1);
                }
                Tag::CodeBlock(k) => {
                    md.flush();
                    let lang = match k {
                        CodeBlockKind::Fenced(l) => l.split([' ', ',']).next().unwrap_or("").to_owned(),
                        CodeBlockKind::Indented => String::new(),
                    };
                    md.code = Some((lang, String::new()));
                }
                Tag::Emphasis => md.italic += 1,
                Tag::Strong => md.bold += 1,
                Tag::Strikethrough => md.strike += 1,
                Tag::Link { dest_url, .. } => md.link = Some(dest_url.to_string()),
                Tag::Image { .. } => md.push("[image: ", false),
                Tag::Table(_) => {
                    md.flush();
                    md.table = Some((Vec::new(), 0));
                }
                Tag::TableHead | Tag::TableRow => {
                    if let Some((rows, _)) = &mut md.table {
                        rows.push(Vec::new());
                    }
                }
                Tag::TableCell => {
                    if let Some(row) = md.table.as_mut().and_then(|(r, _)| r.last_mut()) {
                        row.push(Vec::new());
                    }
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Item => md.flush(),
                TagEnd::Heading { .. } => {
                    md.flush();
                    md.heading = 0;
                }
                TagEnd::List { .. } => {
                    md.flush();
                    md.lists.pop();
                }
                TagEnd::BlockQuote { .. } => {
                    md.flush();
                    md.quote = md.quote.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some((lang, mut text)) = md.code.take() {
                        if text.ends_with('\n') {
                            text.pop();
                        }
                        let indent = md.indent();
                        md.blocks.push(Block::Code { lang, text, indent });
                    }
                }
                TagEnd::Emphasis => md.italic = md.italic.saturating_sub(1),
                TagEnd::Strong => md.bold = md.bold.saturating_sub(1),
                TagEnd::Strikethrough => md.strike = md.strike.saturating_sub(1),
                TagEnd::Link => md.link = None,
                TagEnd::Image => md.push("]", false),
                TagEnd::TableHead => {
                    if let Some((rows, head)) = &mut md.table {
                        *head = rows.len();
                    }
                }
                TagEnd::Table => {
                    if let Some((rows, head)) = md.table.take() {
                        md.blocks.push(Block::Table { rows, head });
                    }
                }
                _ => {}
            },
            Event::Text(t) => match &mut md.code {
                Some((_, code)) => code.push_str(&t),
                None => md.push(&t, false),
            },
            Event::Code(t) | Event::InlineMath(t) | Event::DisplayMath(t) => md.push(&t, true),
            Event::Html(t) | Event::InlineHtml(t) => match &mut md.code {
                Some((_, code)) => code.push_str(&t),
                None => md.push(&t, false),
            },
            Event::SoftBreak => md.push(" ", false),
            Event::HardBreak => md.push("\n", false),
            Event::Rule => {
                md.flush();
                md.blocks.push(Block::Rule);
            }
            Event::TaskListMarker(done) => md.push(if done { "[x] " } else { "[ ] " }, false),
            Event::FootnoteReference(l) => md.push(&format!("[^{l}]"), false),
        }
    }
    md.flush();
    (md.blocks, more)
}

/// Links in a previewed file open only when they are web or mail links.
fn safe_link(url: &str) -> bool {
    let u = url.trim_start().to_ascii_lowercase();
    u.starts_with("http://") || u.starts_with("https://") || u.starts_with("mailto:")
}

enum Body {
    /// Monospace text with this highlighter language ("" = no colours).
    Text(&'static str),
    Markdown(Vec<Block>),
    Table(Vec<Vec<String>>),
    Json(serde_json::Value, String),
}

/// Everything [`show`] draws, worked out once per file.
struct Prepared {
    text: String,
    /// "1\n2\n…", right-aligned, for the line-number column.
    numbers: String,
    notes: Vec<String>,
    body: Body,
}

fn prepare(name: &str, bytes: &[u8]) -> Prepared {
    let d = decode(bytes);
    let mut notes = Vec::new();
    if d.utf16 {
        notes.push("Decoded from UTF-16.".to_owned());
    }
    if d.lossy {
        notes.push("Not valid UTF-8: some characters could not be read.".to_owned());
    }
    if d.truncated {
        notes.push("Large file: showing the first 1 MB.".to_owned());
    }
    let body = match kind(name).unwrap_or(Kind::Plain) {
        Kind::Plain => Body::Text(""),
        Kind::Code(lang) => Body::Text(lang),
        Kind::Markdown => {
            let (blocks, more) = parse_markdown(&d.text);
            if more {
                notes.push(format!("Showing the first {MAX_BLOCKS} blocks."));
            }
            Body::Markdown(blocks)
        }
        Kind::Table(sep) => {
            let (rows, more) = parse_delimited(&d.text, sep, MAX_ROWS);
            if more {
                notes.push(format!("Showing the first {MAX_ROWS} rows."));
            }
            if rows.iter().any(|r| r.len() > MAX_COLS) {
                notes.push(format!("Showing the first {MAX_COLS} columns."));
            }
            Body::Table(rows)
        }
        Kind::Json => match serde_json::from_str::<serde_json::Value>(&d.text) {
            Ok(v) => {
                let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
                Body::Json(v, pretty)
            }
            Err(e) => {
                notes.push(format!("Not valid JSON ({e}): shown as text."));
                Body::Text("json")
            }
        },
    };
    let numbers = match body {
        Body::Text(_) => {
            let n = d.text.lines().count().max(1);
            let w = n.to_string().len();
            let mut s = String::with_capacity(n * (w + 1));
            for i in 1..=n {
                if i > 1 {
                    s.push('\n');
                }
                s.push_str(&format!("{i:>w$}"));
            }
            s
        }
        _ => String::new(),
    };
    Prepared { text: d.text, numbers, notes, body }
}

/// The view switches, shared by every preview (remembered while the app runs).
#[derive(Clone, Copy)]
struct Opts {
    wrap: bool,
    numbers: bool,
    tree: bool,
}

/// Shows `bytes` (the file `name`) as a read-only, scrollable, selectable preview.
pub fn show(ui: &mut egui::Ui, name: &str, bytes: &[u8]) {
    if bytes.is_empty() {
        ui.weak("This file is empty.");
        return;
    }
    // Only the last file's preparation is kept, so big files don't pile up in memory.
    let cache_id = Id::unique("textpreview.cache");
    let key = Id::unique(("textpreview", name, bytes));
    let cached = ui.data(|d| d.get_temp::<(Id, Arc<Prepared>)>(cache_id));
    let p = match cached {
        Some((k, p)) if k == key => p,
        _ => {
            let p = Arc::new(prepare(name, bytes));
            ui.data_mut(|d| d.insert_temp(cache_id, (key, p.clone())));
            p
        }
    };
    let opts_id = Id::unique("textpreview.opts");
    let mut o = ui.data(|d| d.get_temp::<Opts>(opts_id)).unwrap_or(Opts { wrap: false, numbers: true, tree: false });

    ui.horizontal_wrapped(|ui| {
        match &p.body {
            Body::Text(_) => {
                ui.checkbox(&mut o.wrap, "Wrap lines");
                ui.add_enabled(!o.wrap, egui::Checkbox::new(&mut o.numbers, "Line numbers"));
            }
            Body::Json(..) => {
                ui.selectable_value(&mut o.tree, false, "Text");
                ui.selectable_value(&mut o.tree, true, "Tree");
                if !o.tree {
                    ui.checkbox(&mut o.wrap, "Wrap lines");
                }
            }
            _ => {}
        }
        if ui.button("Copy all").clicked() {
            ui.ctx().copy_text(p.text.clone());
        }
        for n in &p.notes {
            ui.weak(n);
        }
    });
    ui.separator();

    match &p.body {
        Body::Text(lang) => text_view(ui, &p.text, &p.numbers, lang, o),
        Body::Json(_, pretty) if !o.tree => text_view(ui, pretty, "", "json", o),
        Body::Json(v, _) => {
            ScrollArea::both().id_salt("textpreview.json").auto_shrink([false, false]).show(ui, |ui| {
                let mut budget = MAX_NODES;
                json_tree(ui, "root", v, Id::unique("textpreview.json.root"), 0, &mut budget);
                if budget == 0 {
                    ui.weak(format!("Showing the first {MAX_NODES} values."));
                }
            });
        }
        Body::Markdown(blocks) => markdown_view(ui, blocks),
        Body::Table(rows) => table_view(ui, rows),
    }
    ui.data_mut(|d| d.insert_temp(opts_id, o));
}

fn mono(ui: &egui::Ui) -> egui::FontId {
    egui::TextStyle::Monospace.resolve(ui.style())
}

/// Monospace text with colours for `lang`, line numbers when not wrapping.
fn text_view(ui: &mut egui::Ui, text: &str, numbers: &str, lang: &str, o: Opts) {
    let font = mono(ui);
    let theme = CodeTheme::from_style(ui.style());
    let job = highlight(ui.ctx(), ui.style(), &theme, text, lang);
    let area = if o.wrap { ScrollArea::vertical() } else { ScrollArea::both() };
    area.id_salt("textpreview.text").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            if o.numbers && !o.wrap && !numbers.is_empty() {
                let color = ui.visuals().weak_text_color();
                let nums = LayoutJob::simple(numbers.to_owned(), font, color, f32::INFINITY);
                ui.add(Label::new(nums).selectable(false));
            }
            let mode = if o.wrap { TextWrapMode::Wrap } else { TextWrapMode::Extend };
            ui.add(Label::new(job).selectable(true).wrap_mode(mode));
        });
    });
}

fn json_tree(ui: &mut egui::Ui, key: &str, v: &serde_json::Value, id: Id, depth: usize, budget: &mut usize) {
    use serde_json::Value;
    if *budget == 0 {
        return;
    }
    *budget -= 1;
    match v {
        Value::Object(m) => {
            egui::CollapsingHeader::new(format!("{key}  {{…}} {} keys", m.len())).id_salt(id).default_open(depth < 2).show(ui, |ui| {
                for (k, c) in m {
                    json_tree(ui, k, c, id.with(k), depth + 1, budget);
                }
            });
        }
        Value::Array(a) => {
            egui::CollapsingHeader::new(format!("{key}  […] {} items", a.len())).id_salt(id).default_open(depth < 2).show(ui, |ui| {
                for (i, c) in a.iter().enumerate() {
                    json_tree(ui, &i.to_string(), c, id.with(i), depth + 1, budget);
                }
            });
        }
        _ => {
            ui.horizontal(|ui| {
                ui.weak(format!("{key}:"));
                ui.label(RichText::new(v.to_string()).monospace());
            });
        }
    }
}

fn markdown_view(ui: &mut egui::Ui, blocks: &[Block]) {
    ScrollArea::vertical().id_salt("textpreview.md").auto_shrink([false, false]).show(ui, |ui| {
        let body = egui::TextStyle::Body.resolve(ui.style()).size;
        for (i, b) in blocks.iter().enumerate() {
            match b {
                Block::Text { spans, heading, indent, quote, bullet } => {
                    if *heading > 0 {
                        ui.add_space(6.0);
                    }
                    let size = body
                        * match heading {
                            1 => 1.9,
                            2 => 1.55,
                            3 => 1.3,
                            4 => 1.15,
                            5 => 1.05,
                            _ => 1.0,
                        };
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(f32::from(*indent) * 18.0);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        if let Some(b) = bullet {
                            ui.label(format!("{b} "));
                        }
                        for s in spans {
                            span(ui, s, size, *heading > 0, *quote);
                        }
                    });
                    if *heading == 1 || *heading == 2 {
                        ui.separator();
                    }
                }
                Block::Code { lang, text, indent } => {
                    ui.horizontal(|ui| {
                        ui.add_space(f32::from(*indent) * 18.0);
                        egui::Frame::new().fill(ui.visuals().extreme_bg_color).inner_margin(6.0).corner_radius(4.0).show(ui, |ui| {
                            let theme = CodeTheme::from_style(ui.style());
                            let job = highlight(ui.ctx(), ui.style(), &theme, text, lang);
                            ScrollArea::horizontal().id_salt(("md.code", i)).show(ui, |ui| {
                                ui.add(Label::new(job).selectable(true).wrap_mode(TextWrapMode::Extend));
                            });
                        });
                    });
                }
                Block::Rule => {
                    ui.separator();
                }
                Block::Table { rows, head } => {
                    egui::Grid::new(("md.table", i)).striped(true).show(ui, |ui| {
                        for (r, row) in rows.iter().enumerate() {
                            for cell in row {
                                ui.horizontal_wrapped(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    for s in cell {
                                        span(ui, s, body, r < *head, false);
                                    }
                                });
                            }
                            ui.end_row();
                        }
                    });
                }
            }
            ui.add_space(4.0);
        }
    });
}

fn span(ui: &mut egui::Ui, s: &Span, size: f32, strong: bool, weak: bool) {
    let mut t = RichText::new(&s.text).size(size);
    if s.bold || strong {
        t = t.strong();
    }
    if s.italic {
        t = t.italics();
    }
    if s.strike {
        t = t.strikethrough();
    }
    if s.code {
        t = t.code();
    }
    if weak {
        t = t.weak();
    }
    match &s.link {
        Some(url) if safe_link(url) => {
            ui.hyperlink_to(t, url).on_hover_text(url);
        }
        _ => {
            ui.label(t);
        }
    }
}

fn table_view(ui: &mut egui::Ui, rows: &[Vec<String>]) {
    use egui_extras::{Column, TableBuilder};
    let Some((header, body)) = rows.split_first() else {
        return;
    };
    let cols = rows.iter().map(Vec::len).max().unwrap_or(0).min(MAX_COLS);
    if cols == 0 {
        return;
    }
    let row_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    let cell = |ui: &mut egui::Ui, text: &str, strong: bool| {
        let flat = text.replace('\n', " ");
        let t = if strong { RichText::new(flat).strong() } else { RichText::new(flat) };
        let r = ui.add(Label::new(t).truncate());
        if text.len() > 40 || text.contains('\n') {
            r.on_hover_text(text);
        }
    };
    ScrollArea::horizontal().id_salt("textpreview.table.h").show(ui, |ui| {
        TableBuilder::new(ui)
            .id_salt("textpreview.table")
            .striped(true)
            .resizable(true)
            .auto_shrink([false, false])
            .columns(Column::initial(140.0).at_least(40.0).clip(true).resizable(true), cols)
            .header(row_h, |mut h| {
                for c in 0..cols {
                    h.col(|ui| cell(ui, header.get(c).map_or("", String::as_str), true));
                }
            })
            .body(|b| {
                b.rows(row_h, body.len(), |mut row| {
                    let r = &body[row.index()];
                    for c in 0..cols {
                        row.col(|ui| cell(ui, r.get(c).map_or("", String::as_str), false));
                    }
                });
            });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_by_name() {
        let table: &[(&str, Option<Kind>)] = &[
            ("notes.txt", Some(Kind::Plain)),
            ("NOTES.TXT", Some(Kind::Plain)),
            ("server.log", Some(Kind::Plain)),
            ("a.text", Some(Kind::Plain)),
            ("x.ini", Some(Kind::Plain)),
            ("x.cfg", Some(Kind::Plain)),
            ("x.conf", Some(Kind::Plain)),
            (".gitignore", Some(Kind::Plain)),
            ("subs.srt", Some(Kind::Plain)),
            ("README", Some(Kind::Plain)),
            ("dir/LICENSE", Some(Kind::Plain)),
            ("Makefile", Some(Kind::Code("sh"))),
            ("Dockerfile", Some(Kind::Code("sh"))),
            ("README.md", Some(Kind::Markdown)),
            ("a.markdown", Some(Kind::Markdown)),
            ("data.csv", Some(Kind::Table(','))),
            ("data.tsv", Some(Kind::Table('\t'))),
            ("data.tab", Some(Kind::Table('\t'))),
            ("a.json", Some(Kind::Json)),
            ("a.geojson", Some(Kind::Json)),
            ("a.jsonc", Some(Kind::Code("json"))),
            ("a.jsonl", Some(Kind::Code(""))),
            ("main.rs", Some(Kind::Code("rs"))),
            ("a.py", Some(Kind::Code("py"))),
            ("a.js", Some(Kind::Code("js"))),
            ("a.mjs", Some(Kind::Code("js"))),
            ("a.ts", Some(Kind::Code("js"))),
            ("a.tsx", Some(Kind::Code("js"))),
            ("a.sh", Some(Kind::Code("sh"))),
            ("a.bash", Some(Kind::Code("sh"))),
            (".bashrc", Some(Kind::Code("sh"))),
            ("a.rb", Some(Kind::Code("sh"))),
            ("a.ps1", Some(Kind::Code("sh"))),
            ("a.yaml", Some(Kind::Code("yaml"))),
            ("a.yml", Some(Kind::Code("yaml"))),
            ("Cargo.toml", Some(Kind::Code("toml"))),
            ("a.sql", Some(Kind::Code("sql"))),
            ("a.c", Some(Kind::Code("c"))),
            ("a.h", Some(Kind::Code("c"))),
            ("a.cpp", Some(Kind::Code("cpp"))),
            ("a.cc", Some(Kind::Code("cpp"))),
            ("a.hpp", Some(Kind::Code("cpp"))),
            ("A.java", Some(Kind::Code("cpp"))),
            ("a.go", Some(Kind::Code("cpp"))),
            ("a.cs", Some(Kind::Code("cpp"))),
            ("a.kt", Some(Kind::Code("cpp"))),
            ("a.swift", Some(Kind::Code("cpp"))),
            ("a.php", Some(Kind::Code("cpp"))),
            ("index.html", Some(Kind::Code(""))),
            ("a.htm", Some(Kind::Code(""))),
            ("a.xml", Some(Kind::Code(""))),
            ("a.css", Some(Kind::Code(""))),
            ("a.scss", Some(Kind::Code(""))),
            ("a.lua", Some(Kind::Code(""))),
            ("a.bat", Some(Kind::Code(""))),
            ("a.diff", Some(Kind::Code(""))),
            ("photo.jpg", None),
            ("doc.docx", None),
            ("sheet.xlsx", None),
            ("a.tar.gz", None),
            ("a.pdf", None),
            ("noextension", None),
            ("", None),
            ("dir.md/file", None),
        ];
        for (name, want) in table {
            assert_eq!(kind(name), *want, "{name}");
            assert_eq!(handles(name), want.is_some(), "{name}");
        }
    }

    #[test]
    fn decoding() {
        // (bytes, text, lossy, utf16, truncated)
        let cases: &[(&[u8], &str, bool, bool, bool)] = &[
            (b"", "", false, false, false),
            (b"hello", "hello", false, false, false),
            (b"\xEF\xBB\xBFbom", "bom", false, false, false),
            (b"a\r\nb\r\n", "a\nb\n", false, false, false),
            (b"lone\rcr", "lone\rcr", false, false, false),
            (b"caf\xE9", "caf\u{FFFD}", true, false, false),
            ("é ✓".as_bytes(), "é ✓", false, false, false),
            (b"\xFF\xFEh\x00i\x00", "hi", false, true, false),
            (b"\xFE\xFF\x00h\x00i", "hi", false, true, false),
            (b"\xFF\xFEh\x00\r\x00\n\x00", "h\n", false, true, false),
            (b"\xFF\xFE\x00\xD8", "\u{FFFD}", true, true, false),
            (b"\xFF\xFEh\x00i", "h\u{FFFD}", true, true, false),
        ];
        for (bytes, text, lossy, utf16, truncated) in cases {
            let d = decode(bytes);
            assert_eq!(d.text, *text, "{bytes:?}");
            assert_eq!((d.lossy, d.utf16, d.truncated), (*lossy, *utf16, *truncated), "{bytes:?}");
        }
    }

    #[test]
    fn big_files_are_cut_at_a_char_boundary() {
        // 'é' is two bytes: put one across the 1 MB line.
        let mut b = vec![b'a'; MAX_BYTES - 1];
        b.extend_from_slice("é and more".as_bytes());
        let d = decode(&b);
        assert!(d.truncated && !d.lossy);
        assert_eq!(d.text.len(), MAX_BYTES - 1);
        // One huge line, no line breaks, still fine.
        let p = prepare("huge.txt", &b);
        assert_eq!(p.numbers, "1");
        assert!(p.notes.iter().any(|n| n.contains("1 MB")));
    }

    #[test]
    fn delimited() {
        type Rows = Vec<Vec<&'static str>>;
        let cases: Vec<(&str, char, Rows)> = vec![
            ("", ',', vec![]),
            ("a,b\n1,2\n", ',', vec![vec!["a", "b"], vec!["1", "2"]]),
            ("a,b\r\n1,2", ',', vec![vec!["a", "b"], vec!["1", "2"]]),
            ("\"x,y\",z", ',', vec![vec!["x,y", "z"]]),
            ("\"say \"\"hi\"\"\"", ',', vec![vec!["say \"hi\""]]),
            ("\"two\nlines\",b\nc,d", ',', vec![vec!["two\nlines", "b"], vec!["c", "d"]]),
            ("a,,c,", ',', vec![vec!["a", "", "c", ""]]),
            ("\"\",x", ',', vec![vec!["", "x"]]),
            ("a\tb,c\n", '\t', vec![vec!["a", "b,c"]]),
            ("ab\"c,d", ',', vec![vec!["ab\"c", "d"]]),
            ("a\n\nb", ',', vec![vec!["a"], vec![""], vec!["b"]]),
        ];
        for (text, sep, want) in cases {
            let (rows, more) = parse_delimited(text, sep, MAX_ROWS);
            assert!(!more, "{text:?}");
            assert_eq!(rows, want, "{text:?}");
        }
        let many = "x\n".repeat(MAX_ROWS + 5);
        let (rows, more) = parse_delimited(&many, ',', MAX_ROWS);
        assert_eq!((rows.len(), more), (MAX_ROWS, true));
        let exact = "x\n".repeat(MAX_ROWS);
        let (rows, more) = parse_delimited(&exact, ',', MAX_ROWS);
        assert_eq!((rows.len(), more), (MAX_ROWS, false));
    }

    #[test]
    fn markdown_blocks() {
        let md = "# Title\n\nSome *em* and **bold** and [web](https://x.org) and [bad](javascript:x).\n\n\
                  - one\n- two\n  1. nested\n\n> quoted\n\n```rust\nfn main() {}\n```\n\n---\n\n\
                  | h1 | h2 |\n|----|----|\n| a | `b` |\n\n- [x] done\n";
        let (blocks, more) = parse_markdown(md);
        assert!(!more);
        let Block::Text { heading, spans, .. } = &blocks[0] else {
            panic!("{:?}", blocks[0]);
        };
        assert_eq!((*heading, spans[0].text.as_str()), (1, "Title"));
        let Block::Text { spans, .. } = &blocks[1] else { panic!() };
        assert!(spans.iter().any(|s| s.italic && s.text == "em"));
        assert!(spans.iter().any(|s| s.bold && s.text == "bold"));
        let links: Vec<_> = spans.iter().filter_map(|s| s.link.as_deref()).collect();
        assert_eq!(links, ["https://x.org", "javascript:x"]);
        assert!(safe_link(links[0]) && !safe_link(links[1]));
        let bullets: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Text { bullet: Some(b), indent, .. } => Some((b.as_str(), *indent)),
                _ => None,
            })
            .collect();
        assert_eq!(bullets, [("•", 1), ("•", 1), ("1.", 2), ("•", 1)]);
        assert!(blocks.iter().any(|b| matches!(b, Block::Text { quote: true, .. })));
        assert!(blocks.iter().any(|b| matches!(b, Block::Code { lang, text, .. } if lang == "rust" && text == "fn main() {}")));
        assert!(blocks.contains(&Block::Rule));
        let Some(Block::Table { rows, head }) = blocks.iter().find(|b| matches!(b, Block::Table { .. })) else { panic!() };
        assert_eq!((*head, rows.len(), rows[1][1][0].code), (1, 2, true));
        let last = blocks.last().unwrap();
        assert!(matches!(last, Block::Text { spans, .. } if spans[0].text == "[x] "));
    }

    #[test]
    fn json_valid_and_invalid() {
        let p = prepare("a.json", br#"{"a":[1,2,{"b":null}]}"#);
        assert!(matches!(&p.body, Body::Json(_, pretty) if pretty.contains("\n  \"a\": [")));
        assert!(p.notes.is_empty());
        let p = prepare("a.json", b"{not json");
        assert!(matches!(p.body, Body::Text("json")));
        assert!(p.notes[0].starts_with("Not valid JSON"));
        // A BOM is fine.
        let p = prepare("a.json", b"\xEF\xBB\xBF[1]");
        assert!(matches!(p.body, Body::Json(..)));
    }

    #[test]
    fn line_numbers() {
        let cases: &[(&[u8], &str)] =
            &[(b"x", "1"), (b"a\nb\nc\n", "1\n2\n3"), (b"a\r\nb", "1\n2"), (&[b'\n'; 10], " 1\n 2\n 3\n 4\n 5\n 6\n 7\n 8\n 9\n10")];
        for (bytes, want) in cases {
            assert_eq!(prepare("a.txt", bytes).numbers, *want, "{bytes:?}");
        }
    }

    /// Every view draws without panicking, in light and dark, with its controls.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn renders_every_kind() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut big_csv = String::from("name,qty\n");
        for i in 0..2000 {
            big_csv.push_str(&format!("\"item, {i}\",{i}\n"));
        }
        let cases: Vec<(&str, Vec<u8>, &str)> = vec![
            ("a.txt", b"hello\r\nworld".to_vec(), "Line numbers"),
            ("README.md", b"# Title\n\n- item\n\n| a | b |\n|---|---|\n| 1 | 2 |\n".to_vec(), "Title"),
            ("d.csv", big_csv.into_bytes(), "name"),
            ("d.tsv", b"x\ty\n1\t2".to_vec(), "y"),
            ("a.json", br#"{"k":[1,2]}"#.to_vec(), "Tree"),
            ("bad.json", b"{".to_vec(), "Line numbers"),
            ("m.rs", b"fn main() { let x = 1; } // hi".to_vec(), "Wrap lines"),
            ("m.go", b"package main".to_vec(), "Copy all"),
            ("e.txt", Vec::new(), "This file is empty."),
            // kittest panics on glyphs no font has (U+FFFD from lossy decoding): the app draws a box.
            ("bom.log", b"\xEF\xBB\xBFbom\r\nline".to_vec(), "Line numbers"),
            ("big.txt", vec![b'z'; MAX_BYTES + 10], "Large file: showing the first 1 MB."),
        ];
        for dark in [false, true] {
            for (name, bytes, label) in &cases {
                let mut h = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui(|ui| {
                    ui.ctx().set_visuals(if dark { egui::Visuals::dark() } else { egui::Visuals::light() });
                    show(ui, name, bytes);
                });
                h.run();
                assert!(h.query_by_label(label).is_some(), "{name}: no {label:?}");
            }
        }
    }
}
