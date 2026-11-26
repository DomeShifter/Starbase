//! The starter templates of "New › From a template…" (Drive and each app's File menu).
//!
//! Each one is an ordinary Office file in `files/`, written by `scripts/make-templates.py`
//! (original content). Choosing one makes "<name> (copy).<ext>" in the folder: the server and
//! this computer's own files both answer `POST api/templates/<key>?folder=…` with
//! [`upload_name`] and [`bytes`]. The web app only needs the list, so the file bytes sit behind
//! [`bytes`] and stay out of it.

/// Which app a template opens in (the gallery's groups).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Document,
    Spreadsheet,
    Presentation,
}

impl Kind {
    /// The gallery's heading for the group.
    pub fn heading(self) -> &'static str {
        match self {
            Kind::Document => "Documents",
            Kind::Spreadsheet => "Spreadsheets",
            Kind::Presentation => "Presentations",
        }
    }

    /// The file ending its copies get.
    pub fn ext(self) -> &'static str {
        match self {
            Kind::Document => "docx",
            Kind::Spreadsheet => "xlsx",
            Kind::Presentation => "pptx",
        }
    }
}

/// One starter template.
#[derive(Clone, Copy, Debug)]
pub struct Template {
    /// Its id in `api/templates/<key>` (the file's name in `files/` without the ending).
    pub key: &'static str,
    /// The name shown on its card; the copy is "<name> (copy)".
    pub name: &'static str,
    pub kind: Kind,
    /// The icon on its card (one the apps ship: app/src/icons.rs).
    pub icon: &'static str,
    /// One line under the name.
    pub blurb: &'static str,
}

/// Every template, grouped by kind in this order.
pub const ALL: &[Template] = &[
    Template { key: "cv", name: "CV", kind: Kind::Document, icon: "description", blurb: "Profile, experience, education, skills" },
    Template { key: "letter", name: "Letter", kind: Kind::Document, icon: "description", blurb: "A formal letter with both addresses" },
    Template { key: "meeting-notes", name: "Meeting notes", kind: Kind::Document, icon: "description", blurb: "Agenda, notes and action items" },
    Template {
        key: "monthly-budget",
        name: "Monthly budget",
        kind: Kind::Spreadsheet,
        icon: "note_add",
        blurb: "Planned against actual, with totals",
    },
    Template { key: "todo-tracker", name: "To-do tracker", kind: Kind::Spreadsheet, icon: "note_add", blurb: "Tasks, who, due date and status" },
    Template { key: "plain-deck", name: "Plain deck", kind: Kind::Presentation, icon: "slideshow", blurb: "Title, two points slides, questions" },
];

/// The template with this key.
pub fn find(key: &str) -> Option<&'static Template> {
    ALL.iter().find(|t| t.key == key)
}

/// The templates of one kind (an app's own File › New › From a template…).
pub fn of(kind: Kind) -> impl Iterator<Item = &'static Template> {
    ALL.iter().filter(move |t| t.kind == kind)
}

/// The name its copy is made with: "Monthly budget (copy).xlsx" (the store numbers it if taken).
pub fn upload_name(t: &Template) -> String {
    format!("{} (copy).{}", t.name, t.kind.ext())
}

/// The template's file.
pub fn bytes(key: &str) -> Option<&'static [u8]> {
    Some(match key {
        "cv" => include_bytes!("../files/cv.docx"),
        "letter" => include_bytes!("../files/letter.docx"),
        "meeting-notes" => include_bytes!("../files/meeting-notes.docx"),
        "monthly-budget" => include_bytes!("../files/monthly-budget.xlsx"),
        "todo-tracker" => include_bytes!("../files/todo-tracker.xlsx"),
        "plain-deck" => include_bytes!("../files/plain-deck.pptx"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    /// Every template (table test): its file exists, is a zip with the main part of its kind,
    /// and its copy's name ends in the right type.
    #[test]
    fn every_template_is_a_real_office_file() {
        assert_eq!(ALL.len(), 6);
        for t in ALL {
            let b = bytes(t.key).unwrap_or_else(|| panic!("{}: no file", t.key));
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(b)).unwrap_or_else(|e| panic!("{}: {e}", t.key));
            let main = match t.kind {
                Kind::Document => "word/document.xml",
                Kind::Spreadsheet => "xl/workbook.xml",
                Kind::Presentation => "ppt/presentation.xml",
            };
            let mut text = String::new();
            z.by_name(main).unwrap_or_else(|e| panic!("{}: {main}: {e}", t.key)).read_to_string(&mut text).unwrap();
            assert!(text.len() > 100, "{}", t.key);
            assert!(upload_name(t).ends_with(&format!(" (copy).{}", t.kind.ext())), "{}", t.key);
            assert!(find(t.key).is_some());
        }
        assert!(bytes("../secret").is_none() && find("nope").is_none());
        assert_eq!(of(Kind::Document).count(), 3);
        assert_eq!(of(Kind::Spreadsheet).count(), 2);
        assert_eq!(of(Kind::Presentation).count(), 1);
    }
}
