//! The step set, where steps act (containers), applying them, and the paragraph helpers.

use std::sync::Arc;

use wordcraft_doc::props::{CellProps, PropChange, RowProps, TableProps};
use wordcraft_doc::{
    Block, Blocks, Cell, CharProps, Comment, Document, InlineObject, ParaProps, Paragraph, Part, Revision, Row, Run, SectionProps, StoryRef, Table,
};

/// A list of blocks steps act in: a story's top level (`path` empty) or a table cell's content,
/// `path` = `[block, row, cell]` per table level below the story.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Cont {
    pub story: StoryRef,
    pub path: Vec<u32>,
}

impl Cont {
    pub fn body() -> Cont {
        Cont { story: StoryRef::Body, path: Vec::new() }
    }
    pub fn story(story: StoryRef) -> Cont {
        Cont { story, path: Vec::new() }
    }
    /// The content of cell (`row`, `cell`) of the table at `block` here.
    pub fn cell(&self, block: usize, row: usize, cell: usize) -> Cont {
        let mut path = self.path.clone();
        path.extend([block as u32, row as u32, cell as u32]);
        Cont { story: self.story, path }
    }
    /// Is `self` strictly inside `outer` (a cell of a table in it, at any depth)?
    pub fn inside(&self, outer: &Cont) -> bool {
        self.story == outer.story && self.path.len() > outer.path.len() && self.path.starts_with(&outer.path)
    }
}

/// A paragraph's own formatting.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Attrs {
    pub props: ParaProps,
    pub mark: CharProps,
    pub section: Option<Box<SectionProps>>,
}

impl Attrs {
    pub fn of(p: &Paragraph) -> Attrs {
        Attrs { props: p.props.clone(), mark: p.mark.clone(), section: p.section.clone() }
    }
    pub fn set(&self, p: &mut Paragraph) {
        p.props = self.props.clone();
        p.mark = self.mark.clone();
        p.section = self.section.clone();
    }
}

/// The document fields outside the body, parts, comments, revisions and media, each one
/// last-writer-wins as a whole.
pub const FIELDS: [&str; 11] = [
    "lastSection",
    "styles",
    "numbering",
    "settings",
    "core",
    "sources",
    "customProps",
    "footnoteSeparators",
    "endnoteSeparators",
    "passthrough",
    "glossary",
];

pub fn field_eq(a: &Document, b: &Document, name: &str) -> bool {
    match name {
        "lastSection" => a.last_section == b.last_section,
        "styles" => Arc::ptr_eq(&a.styles, &b.styles) || a.styles == b.styles,
        "numbering" => Arc::ptr_eq(&a.numbering, &b.numbering) || a.numbering == b.numbering,
        "settings" => a.settings == b.settings,
        "core" => a.core == b.core,
        "sources" => a.sources == b.sources,
        "customProps" => a.custom_props == b.custom_props,
        "footnoteSeparators" => a.footnote_separators == b.footnote_separators,
        "endnoteSeparators" => a.endnote_separators == b.endnote_separators,
        "passthrough" => a.passthrough == b.passthrough,
        "glossary" => a.glossary == b.glossary,
        _ => true,
    }
}

pub fn field_copy(dst: &mut Document, src: &Document, name: &str) {
    match name {
        "lastSection" => dst.last_section = src.last_section.clone(),
        "styles" => dst.styles = src.styles.clone(),
        "numbering" => dst.numbering = src.numbering.clone(),
        "settings" => dst.settings = src.settings.clone(),
        "core" => dst.core = src.core.clone(),
        "sources" => dst.sources = src.sources.clone(),
        "customProps" => dst.custom_props = src.custom_props.clone(),
        "footnoteSeparators" => dst.footnote_separators = src.footnote_separators.clone(),
        "endnoteSeparators" => dst.endnote_separators = src.endnote_separators.clone(),
        "passthrough" => dst.passthrough = src.passthrough.clone(),
        "glossary" => dst.glossary = src.glossary.clone(),
        _ => {}
    }
}

/// One editing step. Paragraph and table steps act in a container [`Cont`] at a block index;
/// byte offsets are UTF-8 offsets into a paragraph's text.
// Spike: some variants are big; the real build boxes them (steps are few per command).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Replace bytes `from..to` of paragraph `block` with `text` (`runs` cover exactly `text`,
    /// `objects` are its U+FFFC characters' objects).
    Splice {
        c: Cont,
        block: usize,
        from: usize,
        to: usize,
        text: String,
        runs: Vec<Run>,
        objects: Vec<InlineObject>,
    },
    /// Set the character formatting of byte ranges of paragraph `block` (text unchanged).
    Format {
        c: Cont,
        block: usize,
        segs: Vec<(usize, usize, CharProps)>,
    },
    /// Set paragraph `block`'s own formatting (last writer wins).
    ParaProps {
        c: Cont,
        block: usize,
        attrs: Box<Attrs>,
    },
    /// Split paragraph `block` at `off`: the head keeps its formatting, the tail gets `tail`.
    Split {
        c: Cont,
        block: usize,
        off: usize,
        tail: Box<Attrs>,
    },
    /// Join paragraph `block + 1` (whose formatting was `gone`) onto paragraph `block` (length
    /// `at`), which keeps its own formatting.
    Join {
        c: Cont,
        block: usize,
        at: usize,
        gone: Box<Attrs>,
    },
    /// Replace blocks `from..to` (which are `old`) with `blocks`.
    Blocks {
        c: Cont,
        from: usize,
        to: usize,
        blocks: Vec<Arc<Block>>,
        old: Vec<Arc<Block>>,
    },
    /// Table at `block`: insert `row` before row `at`.
    InsertRow {
        c: Cont,
        block: usize,
        at: usize,
        row: Box<Row>,
    },
    DeleteRow {
        c: Cont,
        block: usize,
        at: usize,
    },
    /// Table at `block`: insert a column before cell `at` of every row (`cells`, one per row)
    /// and grid column `width`. Tables without merged cells only.
    InsertCol {
        c: Cont,
        block: usize,
        at: usize,
        width: f32,
        cells: Vec<Cell>,
    },
    DeleteCol {
        c: Cont,
        block: usize,
        at: usize,
    },
    /// Table formatting and grid (last writer wins).
    TableAttrs {
        c: Cont,
        block: usize,
        props: Box<TableProps>,
        grid: Vec<f32>,
    },
    RowAttrs {
        c: Cont,
        block: usize,
        row: usize,
        props: Box<RowProps>,
    },
    CellAttrs {
        c: Cont,
        block: usize,
        row: usize,
        cell: usize,
        props: Box<CellProps>,
    },
    /// A new story (header, footer, note, comment text, text box) with its content.
    PartAdd {
        id: u32,
        part: Box<Part>,
    },
    PartRemove {
        id: u32,
    },
    /// A story's own settings (kind, text body); its blocks are left alone.
    PartAttrs {
        id: u32,
        part: Box<Part>,
    },
    /// Add, change, resolve or delete (`None`) comment `id` (last writer wins per comment).
    Comment {
        id: u32,
        comment: Option<Box<Comment>>,
    },
    /// Insert a revision record at index `at`; references to `at` and later move up by one.
    RevInsert {
        at: usize,
        rev: Box<Revision>,
    },
    /// The whole revision list (anything other than additions; last writer wins).
    Revisions {
        all: Vec<Revision>,
    },
    Media {
        key: String,
        bytes: Option<Arc<Vec<u8>>>,
    },
    /// Copy field `name` (one of [`FIELDS`]) from `from`.
    Field {
        name: &'static str,
        from: Box<Document>,
    },
}

impl Step {
    pub fn kind(&self) -> &'static str {
        match self {
            Step::Splice { .. } => "splice",
            Step::Format { .. } => "format",
            Step::ParaProps { .. } => "paraProps",
            Step::Split { .. } => "split",
            Step::Join { .. } => "join",
            Step::Blocks { .. } => "blocks",
            Step::InsertRow { .. } => "insertRow",
            Step::DeleteRow { .. } => "deleteRow",
            Step::InsertCol { .. } => "insertCol",
            Step::DeleteCol { .. } => "deleteCol",
            Step::TableAttrs { .. } => "tableAttrs",
            Step::RowAttrs { .. } => "rowAttrs",
            Step::CellAttrs { .. } => "cellAttrs",
            Step::PartAdd { .. } => "partAdd",
            Step::PartRemove { .. } => "partRemove",
            Step::PartAttrs { .. } => "partAttrs",
            Step::Comment { .. } => "comment",
            Step::RevInsert { .. } => "revInsert",
            Step::Revisions { .. } => "revisions",
            Step::Media { .. } => "media",
            Step::Field { .. } => "field",
        }
    }

    /// The container a paragraph or table step acts in.
    pub fn cont(&self) -> Option<&Cont> {
        match self {
            Step::Splice { c, .. }
            | Step::Format { c, .. }
            | Step::ParaProps { c, .. }
            | Step::Split { c, .. }
            | Step::Join { c, .. }
            | Step::Blocks { c, .. }
            | Step::InsertRow { c, .. }
            | Step::DeleteRow { c, .. }
            | Step::InsertCol { c, .. }
            | Step::DeleteCol { c, .. }
            | Step::TableAttrs { c, .. }
            | Step::RowAttrs { c, .. }
            | Step::CellAttrs { c, .. } => Some(c),
            _ => None,
        }
    }

    pub fn cont_mut(&mut self) -> Option<&mut Cont> {
        match self {
            Step::Splice { c, .. }
            | Step::Format { c, .. }
            | Step::ParaProps { c, .. }
            | Step::Split { c, .. }
            | Step::Join { c, .. }
            | Step::Blocks { c, .. }
            | Step::InsertRow { c, .. }
            | Step::DeleteRow { c, .. }
            | Step::InsertCol { c, .. }
            | Step::DeleteCol { c, .. }
            | Step::TableAttrs { c, .. }
            | Step::RowAttrs { c, .. }
            | Step::CellAttrs { c, .. } => Some(c),
            _ => None,
        }
    }

    /// The block a step addresses (a paragraph, or a table for the table steps; `from` for Blocks).
    pub fn block_mut(&mut self) -> Option<&mut usize> {
        match self {
            Step::Splice { block, .. }
            | Step::Format { block, .. }
            | Step::ParaProps { block, .. }
            | Step::Split { block, .. }
            | Step::Join { block, .. }
            | Step::InsertRow { block, .. }
            | Step::DeleteRow { block, .. }
            | Step::InsertCol { block, .. }
            | Step::DeleteCol { block, .. }
            | Step::TableAttrs { block, .. }
            | Step::RowAttrs { block, .. }
            | Step::CellAttrs { block, .. } => Some(block),
            _ => None,
        }
    }

    pub fn is_table_step(&self) -> bool {
        matches!(
            self,
            Step::InsertRow { .. }
                | Step::DeleteRow { .. }
                | Step::InsertCol { .. }
                | Step::DeleteCol { .. }
                | Step::TableAttrs { .. }
                | Step::RowAttrs { .. }
                | Step::CellAttrs { .. }
        )
    }

    /// A one-line description for failure reports (character formatting shown as b/i only).
    pub fn short(&self) -> String {
        let cp = |p: &CharProps| format!("{}{}", if p.bold == Some(true) { "b" } else { "" }, if p.italic == Some(true) { "i" } else { "" });
        let at = |c: &Cont, b: &usize| format!("{:?}{:?}#{b}", c.story, c.path);
        match self {
            Step::Splice { c, block, from, to, text, runs, .. } => {
                format!(
                    "Splice({}:{from}..{to} {text:?} [{}])",
                    at(c, block),
                    runs.iter().map(|r| format!("{}{}", r.len, cp(&r.props))).collect::<Vec<_>>().join(",")
                )
            }
            Step::Format { c, block, segs } => {
                format!("Format({}: {})", at(c, block), segs.iter().map(|(f, t, p)| format!("{f}..{t}={}", cp(p))).collect::<Vec<_>>().join(","))
            }
            Step::ParaProps { c, block, attrs } => format!("ParaProps({}: {:?})", at(c, block), attrs.props.style),
            Step::Split { c, block, off, tail } => format!("Split({}@{off} tail {:?})", at(c, block), tail.props.style),
            Step::Join { c, block, at: a, .. } => format!("Join({}@{a})", at(c, block)),
            Step::Blocks { c, from, to, blocks, old } => format!("Blocks({}..{to} -> {} blocks, old {})", at(c, from), blocks.len(), old.len()),
            Step::InsertRow { c, block, at: a, .. } => format!("InsertRow({} @{a})", at(c, block)),
            Step::DeleteRow { c, block, at: a } => format!("DeleteRow({} @{a})", at(c, block)),
            Step::InsertCol { c, block, at: a, .. } => format!("InsertCol({} @{a})", at(c, block)),
            Step::DeleteCol { c, block, at: a } => format!("DeleteCol({} @{a})", at(c, block)),
            Step::TableAttrs { c, block, grid, .. } => format!("TableAttrs({} grid {})", at(c, block), grid.len()),
            Step::RowAttrs { c, block, row, .. } => format!("RowAttrs({} r{row})", at(c, block)),
            Step::CellAttrs { c, block, row, cell, .. } => format!("CellAttrs({} r{row}c{cell})", at(c, block)),
            Step::PartAdd { id, .. } => format!("PartAdd({id})"),
            Step::PartRemove { id } => format!("PartRemove({id})"),
            Step::PartAttrs { id, .. } => format!("PartAttrs({id})"),
            Step::Comment { id, comment } => format!("Comment({id} {})", if comment.is_some() { "set" } else { "delete" }),
            Step::RevInsert { at: a, .. } => format!("RevInsert({a})"),
            Step::Revisions { all } => format!("Revisions({})", all.len()),
            Step::Media { key, bytes } => format!("Media({key} {})", bytes.is_some()),
            Step::Field { name, .. } => format!("Field({name})"),
        }
    }
}

pub fn short(steps: &[Step]) -> String {
    steps.iter().map(Step::short).collect::<Vec<_>>().join(" ")
}

// ------------------------------------------------------------------------------------------
// Paragraph helpers

pub const OBJ: char = '\u{FFFC}';

pub fn objs_in(s: &str) -> usize {
    s.matches(OBJ).count()
}

/// The runs covering bytes `a..b` of a paragraph whose runs cover its text.
pub fn runs_slice(runs: &[Run], a: usize, b: usize) -> Vec<Run> {
    let mut out = Vec::new();
    let mut pos = 0;
    for r in runs {
        let (s, e) = (pos, pos + r.len);
        pos = e;
        let (lo, hi) = (s.max(a), e.min(b));
        if lo < hi {
            out.push(Run { len: hi - lo, props: r.props.clone() });
        }
    }
    out
}

pub fn bad(what: &str) -> String {
    format!("step does not apply: {what}")
}

fn check_range(p: &Paragraph, from: usize, to: usize) -> Result<(), String> {
    if from > to || to > p.text.len() || !p.text.is_char_boundary(from) || !p.text.is_char_boundary(to) {
        return Err(bad(&format!("range {from}..{to} in a paragraph of {} bytes", p.text.len())));
    }
    if p.objects.len() != objs_in(&p.text) {
        return Err(bad("paragraph objects don't match its U+FFFC count"));
    }
    Ok(())
}

pub fn splice_para(p: &mut Paragraph, from: usize, to: usize, text: &str, runs: &[Run], objects: &[InlineObject]) -> Result<(), String> {
    p.normalize();
    check_range(p, from, to)?;
    if objs_in(text) != objects.len() || runs.iter().map(|r| r.len).sum::<usize>() != text.len() {
        return Err(bad("splice payload is inconsistent"));
    }
    let k0 = objs_in(&p.text[..from]);
    let kd = objs_in(&p.text[from..to]);
    p.objects.splice(k0..k0 + kd, objects.iter().cloned());
    let mut nr = runs_slice(&p.runs, 0, from);
    nr.extend(runs.iter().cloned());
    nr.extend(runs_slice(&p.runs, to, usize::MAX));
    p.text.replace_range(from..to, text);
    p.runs = nr;
    p.normalize();
    p.touch();
    Ok(())
}

pub fn format_para(p: &mut Paragraph, segs: &[(usize, usize, CharProps)]) -> Result<(), String> {
    p.normalize();
    for (f, t, props) in segs {
        check_range(p, *f, *t)?;
        let mut nr = runs_slice(&p.runs, 0, *f);
        if t > f {
            nr.push(Run { len: t - f, props: props.clone() });
        }
        nr.extend(runs_slice(&p.runs, *t, usize::MAX));
        p.runs = nr;
        p.normalize();
    }
    p.touch();
    Ok(())
}

/// Split `p` at `off`; returns the tail paragraph, formatted `tail`. The head keeps its own.
pub fn split_para(p: &mut Paragraph, off: usize, tail: &Attrs) -> Result<Paragraph, String> {
    p.normalize();
    check_range(p, off, off)?;
    let k = objs_in(&p.text[..off]);
    let mut t = Paragraph {
        text: p.text[off..].to_string(),
        runs: runs_slice(&p.runs, off, usize::MAX),
        objects: p.objects.split_off(k),
        ..Paragraph::default()
    };
    tail.set(&mut t);
    t.normalize();
    p.text.truncate(off);
    p.runs = runs_slice(&p.runs, 0, off);
    p.normalize();
    p.touch();
    Ok(t)
}

/// Join `q` onto `p`; `p` keeps its own formatting.
pub fn join_para(p: &mut Paragraph, q: &Paragraph) {
    p.normalize();
    let mut q = q.clone();
    q.normalize();
    p.text.push_str(&q.text);
    p.runs.extend(q.runs);
    p.objects.extend(q.objects);
    p.normalize();
    p.touch();
}

// ------------------------------------------------------------------------------------------
// Containers

fn story_blocks_mut(doc: &mut Document, s: StoryRef) -> Result<&mut Blocks, String> {
    match s {
        StoryRef::Body => Ok(&mut doc.body),
        StoryRef::Part(id) => doc.parts.get_mut(&id).map(|p| &mut p.blocks).ok_or_else(|| bad(&format!("no story {id}"))),
    }
}

pub fn story_blocks(doc: &Document, s: StoryRef) -> Option<&Blocks> {
    match s {
        StoryRef::Body => Some(&doc.body),
        StoryRef::Part(id) => doc.parts.get(&id).map(|p| &p.blocks),
    }
}

pub fn blocks_mut<'a>(doc: &'a mut Document, c: &Cont) -> Result<&'a mut Blocks, String> {
    let mut cur = story_blocks_mut(doc, c.story)?;
    for t in c.path.chunks(3) {
        let [b, r, k] = t else { return Err(bad("container path")) };
        let blk = cur.get_mut(*b as usize).ok_or_else(|| bad("container block"))?;
        cur = match Arc::make_mut(blk) {
            Block::Table(tb) => {
                &mut tb
                    .rows
                    .get_mut(*r as usize)
                    .ok_or_else(|| bad("container row"))?
                    .cells
                    .get_mut(*k as usize)
                    .ok_or_else(|| bad("container cell"))?
                    .blocks
            }
            Block::Para(_) => return Err(bad("container block is not a table")),
        };
    }
    Ok(cur)
}

pub fn blocks_ref<'a>(doc: &'a Document, c: &Cont) -> Option<&'a Blocks> {
    let mut cur = story_blocks(doc, c.story)?;
    for t in c.path.chunks(3) {
        let [b, r, k] = t else { return None };
        cur = &cur.get(*b as usize)?.as_table()?.rows.get(*r as usize)?.cells.get(*k as usize)?.blocks;
    }
    Some(cur)
}

pub fn para_mut<'a>(doc: &'a mut Document, c: &Cont, k: usize) -> Result<&'a mut Paragraph, String> {
    match blocks_mut(doc, c)?.get_mut(k).map(Arc::make_mut) {
        Some(Block::Para(p)) => Ok(p),
        Some(_) => Err(bad(&format!("block {k} is not a paragraph"))),
        None => Err(bad(&format!("no block {k}"))),
    }
}

pub fn para_ref<'a>(doc: &'a Document, c: &Cont, k: usize) -> Option<&'a Paragraph> {
    blocks_ref(doc, c)?.get(k)?.as_para()
}

pub fn table_mut<'a>(doc: &'a mut Document, c: &Cont, k: usize) -> Result<&'a mut Table, String> {
    match blocks_mut(doc, c)?.get_mut(k).map(Arc::make_mut) {
        Some(Block::Table(t)) => Ok(t),
        _ => Err(bad(&format!("block {k} is not a table"))),
    }
}

pub fn table_ref<'a>(doc: &'a Document, c: &Cont, k: usize) -> Option<&'a Table> {
    blocks_ref(doc, c)?.get(k)?.as_table()
}

// ------------------------------------------------------------------------------------------
// Apply

pub fn apply(doc: &mut Document, s: &Step) -> Result<(), String> {
    match s {
        Step::Splice { c, block, from, to, text, runs, objects } => splice_para(para_mut(doc, c, *block)?, *from, *to, text, runs, objects),
        Step::Format { c, block, segs } => format_para(para_mut(doc, c, *block)?, segs),
        Step::ParaProps { c, block, attrs } => {
            let p = para_mut(doc, c, *block)?;
            attrs.set(p);
            p.touch();
            Ok(())
        }
        Step::Split { c, block, off, tail } => {
            let t = split_para(para_mut(doc, c, *block)?, *off, tail)?;
            blocks_mut(doc, c)?.insert(block + 1, Arc::new(Block::Para(t)));
            Ok(())
        }
        Step::Join { c, block, at, .. } => {
            let q = para_ref(doc, c, block + 1).ok_or_else(|| bad("join: no next paragraph"))?.clone();
            let p = para_mut(doc, c, *block)?;
            if p.text.len() != *at {
                return Err(bad(&format!("join expected a paragraph of {at} bytes, found {}", p.text.len())));
            }
            join_para(p, &q);
            blocks_mut(doc, c)?.remove(block + 1);
            Ok(())
        }
        Step::Blocks { c, from, to, blocks, .. } => {
            let list = blocks_mut(doc, c)?;
            if from > to || *to > list.len() {
                return Err(bad("block range"));
            }
            list.splice(*from..*to, blocks.iter().cloned());
            Ok(())
        }
        Step::InsertRow { c, block, at, row } => {
            let t = table_mut(doc, c, *block)?;
            if *at > t.rows.len() {
                return Err(bad("row index"));
            }
            t.rows.insert(*at, (**row).clone());
            Ok(())
        }
        Step::DeleteRow { c, block, at } => {
            let t = table_mut(doc, c, *block)?;
            if *at >= t.rows.len() {
                return Err(bad("row index"));
            }
            t.rows.remove(*at);
            Ok(())
        }
        Step::InsertCol { c, block, at, width, cells } => {
            let t = table_mut(doc, c, *block)?;
            if cells.len() != t.rows.len() || t.rows.iter().any(|r| *at > r.cells.len()) {
                return Err(bad("column insert doesn't fit the table"));
            }
            for (r, cell) in t.rows.iter_mut().zip(cells) {
                r.cells.insert(*at, cell.clone());
            }
            let g = (*at).min(t.grid.len());
            t.grid.insert(g, *width);
            Ok(())
        }
        Step::DeleteCol { c, block, at } => {
            let t = table_mut(doc, c, *block)?;
            if t.rows.iter().any(|r| *at >= r.cells.len()) {
                return Err(bad("column index"));
            }
            for r in &mut t.rows {
                r.cells.remove(*at);
            }
            if *at < t.grid.len() {
                t.grid.remove(*at);
            }
            Ok(())
        }
        Step::TableAttrs { c, block, props, grid } => {
            let t = table_mut(doc, c, *block)?;
            t.props = (**props).clone();
            t.grid = grid.clone();
            Ok(())
        }
        Step::RowAttrs { c, block, row, props } => {
            let r = table_mut(doc, c, *block)?.rows.get_mut(*row).ok_or_else(|| bad("row index"))?;
            r.props = (**props).clone();
            Ok(())
        }
        Step::CellAttrs { c, block, row, cell, props } => {
            let r = table_mut(doc, c, *block)?.rows.get_mut(*row).ok_or_else(|| bad("row index"))?;
            r.cells.get_mut(*cell).ok_or_else(|| bad("cell index"))?.props = (**props).clone();
            Ok(())
        }
        Step::PartAdd { id, part } => {
            if doc.parts.contains_key(id) {
                return Err(bad(&format!("story {id} exists")));
            }
            doc.parts.insert(*id, (**part).clone());
            Ok(())
        }
        Step::PartRemove { id } => {
            doc.parts.remove(id);
            Ok(())
        }
        Step::PartAttrs { id, part } => {
            let p = doc.parts.get_mut(id).ok_or_else(|| bad(&format!("no story {id}")))?;
            let blocks = std::mem::take(&mut p.blocks);
            *p = (**part).clone();
            p.blocks = blocks;
            Ok(())
        }
        Step::Comment { id, comment } => {
            match comment {
                Some(c) => doc.comments.insert(*id, (**c).clone()),
                None => doc.comments.remove(id),
            };
            Ok(())
        }
        Step::RevInsert { at, rev } => {
            if *at > doc.revisions.len() {
                return Err(bad("revision index"));
            }
            if *at < doc.revisions.len() {
                shift_revs_doc(doc, *at);
            }
            doc.revisions.insert(*at, (**rev).clone());
            Ok(())
        }
        Step::Revisions { all } => {
            doc.revisions = all.clone();
            Ok(())
        }
        Step::Media { key, bytes } => {
            match bytes {
                Some(b) => doc.media.insert(key.clone(), b.clone()),
                None => doc.media.remove(key),
            };
            Ok(())
        }
        Step::Field { name, from } => {
            field_copy(doc, from, name);
            Ok(())
        }
    }
}

pub fn apply_all(doc: &mut Document, steps: &[Step]) -> Result<(), String> {
    steps.iter().try_for_each(|s| apply(doc, s))
}

/// A copy with every paragraph's runs normalised, in every story (the engine sometimes leaves
/// adjacent runs with equal formatting unmerged; the steps always merge them).
pub fn normalized(doc: &Document) -> Document {
    fn norm(blocks: &mut Blocks) {
        for b in blocks.iter_mut() {
            match &**b {
                Block::Para(p) => {
                    let mut q = p.clone();
                    q.normalize();
                    if q.runs != p.runs {
                        *b = Arc::new(Block::Para(q));
                    }
                }
                Block::Table(t) => {
                    let mut t2 = t.clone();
                    for r in &mut t2.rows {
                        for c in &mut r.cells {
                            norm(&mut c.blocks);
                        }
                    }
                    if &t2 != t {
                        *b = Arc::new(Block::Table(t2));
                    }
                }
            }
        }
    }
    let mut d = doc.clone();
    norm(&mut d.body);
    for p in d.parts.values_mut() {
        norm(&mut p.blocks);
    }
    d
}

// ------------------------------------------------------------------------------------------
// Revision references (indexes into `Document::revisions`)

fn up(r: &mut u32, at: usize) {
    if *r as usize >= at {
        *r += 1;
    }
}

fn shift_change<T>(c: &mut Option<Box<PropChange<T>>>, at: usize) {
    if let Some(c) = c {
        up(&mut c.rev, at);
    }
}

pub fn shift_char(c: &mut CharProps, at: usize) {
    if let Some(r) = c.ins.as_mut() {
        up(r, at);
    }
    if let Some(r) = c.del.as_mut() {
        up(r, at);
    }
    shift_change(&mut c.fmt_change, at);
}

pub fn shift_attrs(a: &mut Attrs, at: usize) {
    shift_change(&mut a.props.fmt_change, at);
    if let Some(n) = a.props.num_change.as_mut() {
        up(&mut n.rev, at);
    }
    shift_char(&mut a.mark, at);
    if let Some(s) = a.section.as_deref_mut() {
        shift_change(&mut s.fmt_change, at);
    }
}

fn refs_para(p: &Paragraph, at: usize) -> bool {
    let rc = |c: &CharProps| {
        c.ins.is_some_and(|r| r as usize >= at)
            || c.del.is_some_and(|r| r as usize >= at)
            || c.fmt_change.as_ref().is_some_and(|f| f.rev as usize >= at)
    };
    p.runs.iter().any(|r| rc(&r.props))
        || rc(&p.mark)
        || p.props.fmt_change.as_ref().is_some_and(|f| f.rev as usize >= at)
        || p.props.num_change.as_ref().is_some_and(|f| f.rev as usize >= at)
        || p.section.as_ref().and_then(|s| s.fmt_change.as_ref()).is_some_and(|f| f.rev as usize >= at)
}

pub fn shift_para(p: &mut Paragraph, at: usize) {
    for r in &mut p.runs {
        shift_char(&mut r.props, at);
    }
    let mut a = Attrs::of(p);
    shift_attrs(&mut a, at);
    a.set(p);
}

pub fn shift_cell(c: &mut Cell, at: usize) {
    shift_change(&mut c.props.fmt_change, at);
    shift_blocks(&mut c.blocks, at);
}

pub fn shift_row(r: &mut Row, at: usize) {
    shift_change(&mut r.props.fmt_change, at);
    for c in &mut r.cells {
        shift_cell(c, at);
    }
}

fn block_refs(b: &Block, at: usize) -> bool {
    match b {
        Block::Para(p) => refs_para(p, at),
        Block::Table(t) => {
            t.props.fmt_change.as_ref().is_some_and(|f| f.rev as usize >= at)
                || t.rows.iter().any(|r| {
                    r.props.fmt_change.as_ref().is_some_and(|f| f.rev as usize >= at)
                        || r.cells
                            .iter()
                            .any(|c| c.props.fmt_change.as_ref().is_some_and(|f| f.rev as usize >= at) || c.blocks.iter().any(|b| block_refs(b, at)))
                })
        }
    }
}

/// Move every revision reference at or after `at` up by one (only blocks that have one are
/// copied, so untouched blocks stay shared).
pub fn shift_blocks(blocks: &mut Blocks, at: usize) {
    for b in blocks.iter_mut() {
        if !block_refs(b, at) {
            continue;
        }
        match Arc::make_mut(b) {
            Block::Para(p) => shift_para(p, at),
            Block::Table(t) => {
                shift_change(&mut t.props.fmt_change, at);
                for r in &mut t.rows {
                    shift_row(r, at);
                }
            }
        }
    }
}

pub fn shift_revs_doc(doc: &mut Document, at: usize) {
    shift_blocks(&mut doc.body, at);
    for p in doc.parts.values_mut() {
        shift_blocks(&mut p.blocks, at);
    }
    shift_change(&mut doc.last_section.fmt_change, at);
}

/// The same for what a step carries.
pub fn shift_revs_step(s: &mut Step, at: usize) {
    match s {
        Step::Splice { runs, .. } => runs.iter_mut().for_each(|r| shift_char(&mut r.props, at)),
        Step::Format { segs, .. } => segs.iter_mut().for_each(|(_, _, p)| shift_char(p, at)),
        Step::ParaProps { attrs, .. } => shift_attrs(attrs, at),
        Step::Split { tail, .. } => shift_attrs(tail, at),
        Step::Join { gone, .. } => shift_attrs(gone, at),
        Step::Blocks { blocks, old, .. } => {
            shift_blocks(blocks, at);
            shift_blocks(old, at);
        }
        Step::InsertRow { row, .. } => shift_row(row, at),
        Step::InsertCol { cells, .. } => cells.iter_mut().for_each(|c| shift_cell(c, at)),
        Step::TableAttrs { props, .. } => shift_change(&mut props.fmt_change, at),
        Step::RowAttrs { props, .. } => shift_change(&mut props.fmt_change, at),
        Step::CellAttrs { props, .. } => shift_change(&mut props.fmt_change, at),
        Step::PartAdd { part, .. } => shift_blocks(&mut part.blocks, at),
        Step::Field { name: "lastSection", from } => shift_change(&mut from.last_section.fmt_change, at),
        _ => {}
    }
}

/// Is a step a no-op?
pub fn is_noop(s: &Step) -> bool {
    match s {
        Step::Blocks { from, to, blocks, .. } => from == to && blocks.is_empty(),
        Step::Splice { from, to, text, .. } => from == to && text.is_empty(),
        Step::Format { segs, .. } => segs.iter().all(|(f, t, _)| f >= t),
        _ => false,
    }
}

/// A blank cell for a row inserted next to a new column (both orders make the same one).
pub fn blank_cell() -> Cell {
    Cell::empty()
}

/// Do any cells span columns or rows (column steps can't be used then)?
pub fn has_merges(t: &Table) -> bool {
    t.rows.iter().any(|r| {
        r.props.grid_before != 0 || r.props.grid_after != 0 || r.cells.iter().any(|c| c.props.span != 1 || c.props.vmerge != Default::default())
    })
}

pub fn meta_comment(c: &Comment) -> Box<Comment> {
    Box::new(c.clone())
}

/// A copy of everything outside the body, parts, comments and revisions (for [`Step::Field`]).
pub fn fields_of(d: &Document) -> Document {
    let mut m = d.clone();
    m.body = Vec::new();
    m.parts.clear();
    m.comments.clear();
    m.revisions.clear();
    m.media.clear();
    m
}
