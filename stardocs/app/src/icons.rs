//! The toolbar icons: Material Symbols (app/icons, Apache License 2.0), drawn by egui_extras' SVG loader,
//! and StarDocs' own logo.

use egui::{Image, ImageSource, vec2};

macro_rules! icons {
    ($($n:literal),* $(,)?) => { &[$(($n, include_bytes!(concat!("../icons/", $n, ".svg")))),*] };
}

static ICONS: &[(&str, &[u8])] = icons!(
    "undo",
    "redo",
    "print",
    "spellcheck",
    "format_bold",
    "format_italic",
    "format_underlined",
    "format_color_text",
    "ink_highlighter",
    "link",
    "add_comment",
    "image",
    "format_align_left",
    "format_align_center",
    "format_align_right",
    "format_align_justify",
    "format_line_spacing",
    "checklist",
    "format_list_bulleted",
    "format_list_numbered",
    "format_indent_decrease",
    "format_indent_increase",
    "format_clear",
    "add",
    "remove",
    "arrow_drop_down",
    "format_paint",
    "edit",
    "rate_review",
    "visibility",
    "comment",
    "keyboard_arrow_up",
    "keyboard_arrow_down",
    "check",
    "more_vert",
    // The menus' icon column, and the submenu arrow.
    "arrow_right",
    "arrow_back",
    "close",
    "article",
    "bookmark",
    "border_all",
    "calendar_today",
    "content_copy",
    "content_cut",
    "content_paste",
    "content_paste_go",
    "crop",
    "description",
    "download",
    "drive_file_rename_outline",
    "find_replace",
    "fit_page_width",
    "folder_open",
    "format_quote",
    "format_size",
    "fullscreen",
    "functions",
    "horizontal_rule",
    "info",
    "insert_page_break",
    "keyboard",
    "match_case",
    "notes",
    "numbers_123",
    "page_header",
    "record_voice_over",
    "search",
    "select_all",
    "special_character",
    "strikethrough_s",
    "subscript",
    "superscript",
    "table",
    "tag",
    "toc",
    "view_column",
    "zoom_in",
    "zoom_out",
    // The toolbar's picture tools, while a picture is selected.
    "tune",
    "swap_horiz",
    "restart_alt",
    "art_track",
    "format_image_left",
    "horizontal_split",
    "flip_to_back",
    "flip_to_front",
);

/// Each menu entry's icon, as Docs shows them (Material Symbols names), by its wording.
static MENU_ICONS: &[(&str, &str)] = &[
    // File
    ("New document", "description"),
    ("Open", "folder_open"),
    ("Download", "download"),
    ("Rename", "drive_file_rename_outline"),
    ("Page setup", "article"),
    ("Print", "print"),
    // Edit
    ("Undo", "undo"),
    ("Redo", "redo"),
    ("Cut", "content_cut"),
    ("Copy", "content_copy"),
    ("Paste", "content_paste"),
    ("Paste without formatting", "content_paste_go"),
    ("Select all", "select_all"),
    ("Find and replace", "find_replace"),
    // View
    ("Mode", "edit"),
    ("Zoom in", "zoom_in"),
    ("Zoom out", "zoom_out"),
    ("Fit page width", "fit_page_width"),
    ("Full screen", "fullscreen"),
    // Insert
    ("Image", "image"),
    ("Table", "table"),
    ("Date", "calendar_today"),
    ("Link", "link"),
    ("Comment", "add_comment"),
    ("Horizontal line", "horizontal_rule"),
    ("Break", "insert_page_break"),
    ("Headers and footers", "page_header"),
    ("Page numbers", "tag"),
    ("Footnote", "notes"),
    ("Special characters", "special_character"),
    ("Equation", "functions"),
    ("Bookmark", "bookmark"),
    ("Table of contents", "toc"),
    // Format
    ("Text", "format_bold"),
    ("Paragraph styles", "format_size"),
    ("Align and indent", "format_align_left"),
    ("Line and paragraph spacing", "format_line_spacing"),
    ("Columns", "view_column"),
    ("Bullets and numbering", "format_list_numbered"),
    ("Borders and shading", "border_all"),
    ("Clear formatting", "format_clear"),
    ("Bold", "format_bold"),
    ("Italic", "format_italic"),
    ("Underline", "format_underlined"),
    ("Strikethrough", "strikethrough_s"),
    ("Superscript", "superscript"),
    ("Subscript", "subscript"),
    ("Size", "format_size"),
    ("Capitalisation", "match_case"),
    ("Left", "format_align_left"),
    ("Centre", "format_align_center"),
    ("Right", "format_align_right"),
    ("Justified", "format_align_justify"),
    ("Increase indent", "format_indent_increase"),
    ("Decrease indent", "format_indent_decrease"),
    ("Bulleted list", "format_list_bulleted"),
    ("Numbered list", "format_list_numbered"),
    ("Checklist", "checklist"),
    ("Crop image", "crop"),
    // Tools
    ("Spelling and grammar", "spellcheck"),
    ("Word count", "numbers_123"),
    ("Review suggested edits", "rate_review"),
    ("Citations", "format_quote"),
    ("Read aloud", "record_voice_over"),
    // Help
    ("Search the menus", "search"),
    ("Keyboard shortcuts", "keyboard"),
    ("About Starbase Docs", "info"),
];

/// The Material Symbols name of the menu entry worded `label`, if Docs shows it with an icon.
pub fn menu_icon_name(label: &str) -> Option<&'static str> {
    MENU_ICONS.iter().find(|(l, _)| *l == label).map(|(_, n)| *n)
}

/// The icon called `name` at `px` pixels square, or none for an unknown name.
pub fn icon_px(name: &str, px: f32) -> Option<Image<'static>> {
    icon(name).map(|i| i.fit_to_exact_size(vec2(px, px)))
}

/// The icon called `name` at 18 px, or none for an unknown name.
pub fn icon(name: &str) -> Option<Image<'static>> {
    let (n, bytes) = ICONS.iter().find(|(n, _)| *n == name)?;
    Some(
        Image::new(ImageSource::Bytes { uri: format!("bytes://stardocs/{n}.svg").into(), bytes: (*bytes).into() })
            .fit_to_exact_size(vec2(18.0, 18.0)),
    )
}

/// StarDocs' own logo (app/icons/stardocs_logo.svg, square), `height` pixels tall and as wide.
pub fn logo(height: f32) -> Image<'static> {
    let bytes: &'static [u8] = include_bytes!("../icons/stardocs_logo.svg");
    Image::new(ImageSource::Bytes { uri: "bytes://stardocs/stardocs_logo.svg".into(), bytes: bytes.into() }).fit_to_exact_size(vec2(height, height))
}
