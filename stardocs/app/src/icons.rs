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
);

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
    Image::new(ImageSource::Bytes { uri: "bytes://stardocs/stardocs_logo.svg".into(), bytes: bytes.into() })
        .fit_to_exact_size(vec2(height, height))
}
