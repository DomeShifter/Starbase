//! The toolbar and menu icons: Material Symbols (app/icons, Apache License 2.0) drawn by
//! egui_extras' SVG loader, white in the files and tinted here for the light or dark theme; and
//! Starbase Slides' own logo.

use egui::{Color32, Image, ImageSource, vec2};

macro_rules! icons {
    ($($n:literal),* $(,)?) => { &[$(($n, include_bytes!(concat!("../icons/", $n, ".svg")))),*] };
}

static ICONS: &[(&str, &[u8])] = icons!(
    "add",
    "undo",
    "redo",
    "format_bold",
    "format_italic",
    "format_underlined",
    "format_align_left",
    "format_align_center",
    "format_align_right",
    "format_align_justify",
    "format_list_bulleted",
    "format_list_numbered",
    "format_indent_decrease",
    "format_indent_increase",
    "format_clear",
    "format_color_text",
    "format_color_fill",
    "image",
    "link",
    "add_comment",
    "comment",
    "arrow_drop_down",
    "arrow_right",
    "check",
    "table",
    "zoom_in",
    "zoom_out",
    "palette",
    "content_copy",
    "content_cut",
    "content_paste",
    "delete",
    "download",
    "drive_file_rename_outline",
    "find_replace",
    "select_all",
    "notes",
    "print",
    "share",
    "history",
    "home",
    "person_add",
    "special_character",
    "insert_chart",
    "grid_view",
    "description",
    "folder_open",
    "keyboard",
    "info",
    "search",
    "flip_to_front",
    "flip_to_back",
    "aspect_ratio",
    "close",
    "note_add",
    "text_fields",
    "shapes",
    "arrow_selector_tool",
    "slideshow",
    "play_arrow",
    "wallpaper",
    "dashboard",
    "animation",
);

/// Whether `name` is one of ours (the tests check every name the shell uses).
#[cfg(test)]
pub fn has(name: &str) -> bool {
    ICONS.iter().any(|(n, _)| *n == name)
}

fn source(name: &str) -> Option<ImageSource<'static>> {
    ICONS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(n, bytes)| ImageSource::Bytes { uri: format!("bytes://slides/{n}.svg").into(), bytes: (*bytes).into() })
}

/// The icon at 20 px in `tint`.
pub fn icon(name: &str, tint: Color32) -> Option<Image<'static>> {
    icon_px(name, 20.0, tint)
}

pub fn icon_px(name: &str, px: f32, tint: Color32) -> Option<Image<'static>> {
    source(name).map(|s| Image::new(s).fit_to_exact_size(vec2(px, px)).tint(tint))
}

/// Starbase Slides' logo at `px`.
pub fn logo(px: f32) -> Image<'static> {
    Image::new(ImageSource::Bytes { uri: "bytes://slides/starslides_logo.svg".into(), bytes: include_bytes!("../icons/starslides_logo.svg").into() })
        .fit_to_exact_size(vec2(px, px))
}
