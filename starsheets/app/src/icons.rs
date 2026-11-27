//! Toolbar icons: Material Symbols SVGs (Apache-2.0, see app/icons/README.md), embedded at build time.

use egui::{Image, ImageSource, vec2};

const ICONS: &[(&str, &[u8])] = &[
    ("accessibility_new", include_bytes!("../icons/accessibility_new.svg")),
    ("add", include_bytes!("../icons/add.svg")),
    ("add_box", include_bytes!("../icons/add_box.svg")),
    ("add_circle", include_bytes!("../icons/add_circle.svg")),
    ("add_comment", include_bytes!("../icons/add_comment.svg")),
    ("arrow_drop_down", include_bytes!("../icons/arrow_drop_down.svg")),
    ("arrow_right", include_bytes!("../icons/arrow_right.svg")),
    ("arrow_drop_down_circle", include_bytes!("../icons/arrow_drop_down_circle.svg")),
    ("arrow_upward", include_bytes!("../icons/arrow_upward.svg")),
    ("aspect_ratio", include_bytes!("../icons/aspect_ratio.svg")),
    ("border_all", include_bytes!("../icons/border_all.svg")),
    ("border_color", include_bytes!("../icons/border_color.svg")),
    // border_color without its built-in bar: the toolbar draws the bar in the chosen colour.
    ("border_color_pen", include_bytes!("../icons/border_color_pen.svg")),
    ("line_style", include_bytes!("../icons/line_style.svg")),
    ("border_bottom", include_bytes!("../icons/border_bottom.svg")),
    ("border_clear", include_bytes!("../icons/border_clear.svg")),
    ("border_horizontal", include_bytes!("../icons/border_horizontal.svg")),
    ("border_inner", include_bytes!("../icons/border_inner.svg")),
    ("border_left", include_bytes!("../icons/border_left.svg")),
    ("border_outer", include_bytes!("../icons/border_outer.svg")),
    ("border_right", include_bytes!("../icons/border_right.svg")),
    ("border_top", include_bytes!("../icons/border_top.svg")),
    ("border_vertical", include_bytes!("../icons/border_vertical.svg")),
    ("calculate", include_bytes!("../icons/calculate.svg")),
    ("call_split", include_bytes!("../icons/call_split.svg")),
    ("cell_merge", include_bytes!("../icons/cell_merge.svg")),
    ("check", include_bytes!("../icons/check.svg")),
    ("check_box", include_bytes!("../icons/check_box.svg")),
    ("check_box_outline_blank", include_bytes!("../icons/check_box_outline_blank.svg")),
    ("cleaning_services", include_bytes!("../icons/cleaning_services.svg")),
    ("close", include_bytes!("../icons/close.svg")),
    ("content_copy", include_bytes!("../icons/content_copy.svg")),
    ("content_cut", include_bytes!("../icons/content_cut.svg")),
    ("content_paste", include_bytes!("../icons/content_paste.svg")),
    ("decimal_decrease", include_bytes!("../icons/decimal_decrease.svg")),
    ("decimal_increase", include_bytes!("../icons/decimal_increase.svg")),
    ("delete", include_bytes!("../icons/delete.svg")),
    ("description", include_bytes!("../icons/description.svg")),
    ("download", include_bytes!("../icons/download.svg")),
    ("edit", include_bytes!("../icons/edit.svg")),
    ("rotate_left", include_bytes!("../icons/rotate_left.svg")),
    ("rotate_right", include_bytes!("../icons/rotate_right.svg")),
    ("white_rotate_left", include_bytes!("../icons/white_rotate_left.svg")),
    ("white_rotate_right", include_bytes!("../icons/white_rotate_right.svg")),
    ("filter_alt", include_bytes!("../icons/filter_alt.svg")),
    ("filter_list", include_bytes!("../icons/filter_list.svg")),
    ("find_replace", include_bytes!("../icons/find_replace.svg")),
    ("folder_open", include_bytes!("../icons/folder_open.svg")),
    ("format_align_center", include_bytes!("../icons/format_align_center.svg")),
    ("format_align_left", include_bytes!("../icons/format_align_left.svg")),
    ("format_align_right", include_bytes!("../icons/format_align_right.svg")),
    ("format_bold", include_bytes!("../icons/format_bold.svg")),
    ("format_clear", include_bytes!("../icons/format_clear.svg")),
    ("format_color_fill", include_bytes!("../icons/format_color_fill.svg")),
    ("format_color_reset", include_bytes!("../icons/format_color_reset.svg")),
    ("format_text_clip", include_bytes!("../icons/format_text_clip.svg")),
    ("format_color_text", include_bytes!("../icons/format_color_text.svg")),
    ("format_italic", include_bytes!("../icons/format_italic.svg")),
    ("format_paint", include_bytes!("../icons/format_paint.svg")),
    ("format_size", include_bytes!("../icons/format_size.svg")),
    ("functions", include_bytes!("../icons/functions.svg")),
    ("grid_on", include_bytes!("../icons/grid_on.svg")),
    ("history", include_bytes!("../icons/history.svg")),
    ("image", include_bytes!("../icons/image.svg")),
    // Photos (home/photos.rs); white_* are the same symbols in white, for the dark photo viewer.
    ("photo_library", include_bytes!("../icons/photo_library.svg")),
    ("play_arrow", include_bytes!("../icons/play_arrow.svg")),
    ("pause", include_bytes!("../icons/pause.svg")),
    ("shuffle", include_bytes!("../icons/shuffle.svg")),
    ("zoom_out", include_bytes!("../icons/zoom_out.svg")),
    ("white_close", include_bytes!("../icons/white_close.svg")),
    ("white_chevron_left", include_bytes!("../icons/white_chevron_left.svg")),
    ("white_chevron_right", include_bytes!("../icons/white_chevron_right.svg")),
    ("white_play_arrow", include_bytes!("../icons/white_play_arrow.svg")),
    ("white_pause", include_bytes!("../icons/white_pause.svg")),
    ("white_shuffle", include_bytes!("../icons/white_shuffle.svg")),
    ("white_zoom_in", include_bytes!("../icons/white_zoom_in.svg")),
    ("white_zoom_out", include_bytes!("../icons/white_zoom_out.svg")),
    ("white_download", include_bytes!("../icons/white_download.svg")),
    ("white_info", include_bytes!("../icons/white_info.svg")),
    ("white_star", include_bytes!("../icons/white_star.svg")),
    ("white_star_fill", include_bytes!("../icons/white_star_fill.svg")),
    ("white_delete", include_bytes!("../icons/white_delete.svg")),
    ("white_remove", include_bytes!("../icons/white_remove.svg")),
    ("white_photo_library", include_bytes!("../icons/white_photo_library.svg")),
    ("info", include_bytes!("../icons/info.svg")),
    ("insert_chart", include_bytes!("../icons/insert_chart.svg")),
    ("keyboard", include_bytes!("../icons/keyboard.svg")),
    ("keyboard_arrow_down", include_bytes!("../icons/keyboard_arrow_down.svg")),
    ("keyboard_arrow_up", include_bytes!("../icons/keyboard_arrow_up.svg")),
    ("keyboard_tab", include_bytes!("../icons/keyboard_tab.svg")),
    ("label", include_bytes!("../icons/label.svg")),
    ("link", include_bytes!("../icons/link.svg")),
    ("link_off", include_bytes!("../icons/link_off.svg")),
    // A generic globe for the link card: never a site's own icon (fetching one would be a request).
    ("language", include_bytes!("../icons/language.svg")),
    ("open_in_new", include_bytes!("../icons/open_in_new.svg")),
    ("lock", include_bytes!("../icons/lock.svg")),
    ("menu", include_bytes!("../icons/menu.svg")),
    ("note_add", include_bytes!("../icons/note_add.svg")),
    ("numbers_123", include_bytes!("../icons/numbers_123.svg")),
    ("percent", include_bytes!("../icons/percent.svg")),
    ("pivot_table_chart", include_bytes!("../icons/pivot_table_chart.svg")),
    ("print", include_bytes!("../icons/print.svg")),
    ("redo", include_bytes!("../icons/redo.svg")),
    ("remove", include_bytes!("../icons/remove.svg")),
    ("rule", include_bytes!("../icons/rule.svg")),
    ("search", include_bytes!("../icons/search.svg")),
    ("settings", include_bytes!("../icons/settings.svg")),
    ("slideshow", include_bytes!("../icons/slideshow.svg")),
    ("sort", include_bytes!("../icons/sort.svg")),
    ("sort_by_alpha", include_bytes!("../icons/sort_by_alpha.svg")),
    ("spellcheck", include_bytes!("../icons/spellcheck.svg")),
    ("stardocs", include_bytes!("../icons/stardocs.svg")),
    ("stardrive", include_bytes!("../icons/stardrive.svg")),
    ("starphotos", include_bytes!("../icons/starphotos.svg")),
    ("starsheets", include_bytes!("../icons/starsheets.svg")),
    ("starslides", include_bytes!("../icons/starslides.svg")),
    ("sticky_note_2", include_bytes!("../icons/sticky_note_2.svg")),
    ("strikethrough_s", include_bytes!("../icons/strikethrough_s.svg")),
    ("tab", include_bytes!("../icons/tab.svg")),
    ("table", include_bytes!("../icons/table.svg")),
    ("table_rows", include_bytes!("../icons/table_rows.svg")),
    ("text_rotate_up", include_bytes!("../icons/text_rotate_up.svg")),
    ("text_rotate_vertical", include_bytes!("../icons/text_rotate_vertical.svg")),
    ("text_rotation_angledown", include_bytes!("../icons/text_rotation_angledown.svg")),
    ("text_rotation_angleup", include_bytes!("../icons/text_rotation_angleup.svg")),
    ("text_rotation_down", include_bytes!("../icons/text_rotation_down.svg")),
    ("text_rotation_none", include_bytes!("../icons/text_rotation_none.svg")),
    ("undo", include_bytes!("../icons/undo.svg")),
    ("upload", include_bytes!("../icons/upload.svg")),
    ("vertical_align_bottom", include_bytes!("../icons/vertical_align_bottom.svg")),
    ("vertical_align_center", include_bytes!("../icons/vertical_align_center.svg")),
    ("vertical_align_top", include_bytes!("../icons/vertical_align_top.svg")),
    ("view_column", include_bytes!("../icons/view_column.svg")),
    ("visibility", include_bytes!("../icons/visibility.svg")),
    ("visibility_off", include_bytes!("../icons/visibility_off.svg")),
    ("width", include_bytes!("../icons/width.svg")),
    ("wrap_text", include_bytes!("../icons/wrap_text.svg")),
    ("zoom_in", include_bytes!("../icons/zoom_in.svg")),
    ("drive_file_move", include_bytes!("../icons/drive_file_move.svg")),
    // The Drive's icons for files that aren't spreadsheets or documents (home/items.rs).
    ("draft", include_bytes!("../icons/draft.svg")),
    ("movie", include_bytes!("../icons/movie.svg")),
    ("music_note", include_bytes!("../icons/music_note.svg")),
    ("picture_as_pdf", include_bytes!("../icons/picture_as_pdf.svg")),
    ("arrow_back", include_bytes!("../icons/arrow_back.svg")),
    ("arrow_forward", include_bytes!("../icons/arrow_forward.svg")),
    ("create_new_folder", include_bytes!("../icons/create_new_folder.svg")),
    ("folder_fill", include_bytes!("../icons/folder_fill.svg")),
    ("palette", include_bytes!("../icons/palette.svg")),
    ("chevron_left", include_bytes!("../icons/chevron_left.svg")),
    ("chevron_right", include_bytes!("../icons/chevron_right.svg")),
    ("expand_more", include_bytes!("../icons/expand_more.svg")),
    ("folder", include_bytes!("../icons/folder.svg")),
    ("folder_shared", include_bytes!("../icons/folder_shared.svg")),
    ("grid_view", include_bytes!("../icons/grid_view.svg")),
    ("group", include_bytes!("../icons/group.svg")),
    ("groups", include_bytes!("../icons/groups.svg")),
    ("home_storage", include_bytes!("../icons/home_storage.svg")),
    ("more_vert", include_bytes!("../icons/more_vert.svg")),
    ("person_add", include_bytes!("../icons/person_add.svg")),
    ("schedule", include_bytes!("../icons/schedule.svg")),
    ("star", include_bytes!("../icons/star.svg")),
    ("star_fill", include_bytes!("../icons/star_fill.svg")),
    ("view_list", include_bytes!("../icons/view_list.svg")),
    ("photo_mode", include_bytes!("../icons/photo_mode.svg")),
    ("dark_mode", include_bytes!("../icons/dark_mode.svg")),
    ("light_mode", include_bytes!("../icons/light_mode.svg")),
    ("event", include_bytes!("../icons/event.svg")),
];

/// The icon called `name`, 18 px, or None if it isn't embedded (the test below checks the names
/// the toolbar uses).
pub fn icon(name: &str) -> Option<Image<'static>> {
    // Photo Mode's dark look: the grey outline icons turn light.
    if crate::theme::photo_dark() {
        return icon_in(name, crate::theme::text_dim());
    }
    let (n, bytes) = ICONS.iter().find(|(n, _)| *n == name)?;
    Some(
        Image::new(ImageSource::Bytes { uri: format!("bytes://starsheets/{n}.svg").into(), bytes: (*bytes).into() })
            .fit_to_exact_size(vec2(18.0, 18.0)),
    )
}

/// The icon called `name` filled with `colour` (white on the dark filter view bar, blue for a
/// ticked box). The SVGs are filled `#444746`, and a tint only multiplies colours, so it can
/// darken an icon but never lighten it or turn it blue: the fill itself is swapped instead.
pub fn icon_in(name: &str, colour: egui::Color32) -> Option<Image<'static>> {
    let (n, bytes) = ICONS.iter().find(|(n, _)| *n == name)?;
    let hex = format!("#{:02X}{:02X}{:02X}", colour.r(), colour.g(), colour.b());
    let svg = String::from_utf8_lossy(bytes).replace("#444746", &hex);
    let bytes: std::sync::Arc<[u8]> = svg.into_bytes().into();
    Some(
        Image::new(ImageSource::Bytes { uri: format!("bytes://starsheets/{n}-{}.svg", &hex[1..]).into(), bytes: egui::load::Bytes::Shared(bytes) })
            .fit_to_exact_size(vec2(18.0, 18.0)),
    )
}

#[cfg(test)]
pub fn has(name: &str) -> bool {
    ICONS.iter().any(|(n, _)| *n == name)
}

/// The StarSheets logo (app/icons/starsheets.svg; app/favicon.svg is a simplified copy for 16-32 px) at `size` px, in colour.
pub fn logo(size: f32) -> Option<Image<'static>> {
    let (_, bytes) = ICONS.iter().find(|(n, _)| *n == "starsheets")?;
    Some(Image::new(ImageSource::Bytes { uri: "bytes://starsheets/logo.svg".into(), bytes: (*bytes).into() }).fit_to_exact_size(vec2(size, size)))
}

/// The StarDrive logo (app/icons/stardrive.svg; app/stardrive.svg is the same, as the home screen's tab icon) at `size` px, in colour.
pub fn drive_logo(size: f32) -> Option<Image<'static>> {
    let (_, bytes) = ICONS.iter().find(|(n, _)| *n == "stardrive")?;
    Some(
        Image::new(ImageSource::Bytes { uri: "bytes://starsheets/stardrive_logo.svg".into(), bytes: (*bytes).into() })
            .fit_to_exact_size(vec2(size, size)),
    )
}

/// The Starbase Photos logo (app/icons/starphotos.svg; app/starphotos.svg is the same, as Photo Mode's tab icon) at `size` px, in colour.
pub fn photos_logo(size: f32) -> Option<Image<'static>> {
    let (_, bytes) = ICONS.iter().find(|(n, _)| *n == "starphotos")?;
    Some(
        Image::new(ImageSource::Bytes { uri: "bytes://starsheets/starphotos_logo.svg".into(), bytes: (*bytes).into() })
            .fit_to_exact_size(vec2(size, size)),
    )
}
