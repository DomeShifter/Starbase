//! Google Docs' look for the parts WordCraft draws itself: its dialogs (Page setup, Word count,
//! Find and replace, Insert link, Table properties …) and the Comments pane. WordCraft takes the
//! look through its host style hook (`WordApp::set_host_look`, fork commit "StarDocs 0008");
//! web.rs applies the current browser palette. Colours match the rest of our interface (shell.rs, share.rs).

use egui::{Color32, CornerRadius, Frame, Margin, Shadow, Stroke, vec2};
use wordcraft_ui_egui::theme::{medium, regular};
use wordcraft_ui_egui::{ButtonLook, CommentsLook, DialogLook, HostLook, MarginLook, WordApp};

use crate::theme::palette;

const BLUE: Color32 = Color32::from_rgb(0x0B, 0x57, 0xD0);
/// Field outlines: Google's grey at rest, near-black under the pointer.
pub const FIELD: Color32 = Color32::from_rgb(0x74, 0x77, 0x75);
/// Behind commented text, and behind the selected comment's text (Google Docs' yellows).
const HIGHLIGHT: Color32 = Color32::from_rgb(0xFE, 0xF2, 0xCD);
const HIGHLIGHT_ACTIVE: Color32 = Color32::from_rgb(0xFC, 0xD6, 0x63);

/// Gives WordCraft's dialogs and Comments pane our look.
pub fn apply(app: &mut WordApp) {
    app.set_host_look(look());
}

/// Google's buttons: OK is a filled blue pill with white text, Cancel a blue text button.
fn buttons() -> ButtonLook {
    let accent = palette().accent;
    ButtonLook {
        font: medium(14.0),
        height: 36.0,
        padding: 24.0,
        fill: BLUE,
        on_fill: Color32::WHITE,
        text: accent,
        hover: Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 0x14),
    }
}

/// A white card lifted by a soft shadow, as Google's dialogs and comment cards are.
fn shadow(blur: u8, alpha: u8) -> Shadow {
    Shadow { offset: [0, 2], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
}

pub fn look() -> HostLook {
    let colours = palette();
    let light = colours.card == Color32::WHITE;
    let dialog = DialogLook {
        frame: Frame::NONE
            .fill(palette().card)
            .corner_radius(CornerRadius::same(8))
            .shadow(shadow(24, 60))
            .stroke(Stroke::NONE)
            .inner_margin(Margin::same(24)),
        title_font: regular(22.0),
        title_color: palette().text,
        body_size: 14.0,
        heading_font: medium(14.0),
        item_spacing: vec2(10.0, 10.0),
        row_height: 36.0,
        field_stroke: Stroke::new(1.0, FIELD),
        field_hover: Stroke::new(1.0, palette().text),
        field_focus: Stroke::new(2.0, palette().accent),
        field_radius: 4,
        min_width: 420.0,
        close_color: palette().text_dim,
        buttons: buttons(),
    };
    let comments = CommentsLook {
        panel_fill: colours.card,
        panel_stroke: Stroke::new(1.0, if light { Color32::from_rgb(0xE1, 0xE3, 0xE1) } else { colours.menu_line }),
        header_font: regular(18.0),
        header_color: palette().text,
        card: Frame::NONE
            .fill(palette().card)
            .corner_radius(CornerRadius::same(8))
            .shadow(shadow(6, 40))
            .stroke(Stroke::new(1.0, if light { Color32::from_rgb(0xE8, 0xEA, 0xED) } else { colours.menu_line }))
            .inner_margin(Margin::same(12)),
        card_gap: 12.0,
        author_font: medium(14.0),
        author_color: palette().text,
        meta_font: regular(12.0),
        meta_color: palette().meta,
        text_font: regular(14.0),
        icon_color: palette().text_dim,
        buttons: buttons(),
        // Cards beside the page, level with their text, as Docs shows comments.
        margin: Some(MarginLook {
            card_width: 280.0,
            gap: 20.0,
            highlight: HIGHLIGHT,
            highlight_active: HIGHLIGHT_ACTIVE,
            active_card: Frame::NONE
                .fill(palette().card)
                .corner_radius(CornerRadius::same(8))
                .shadow(Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(70) })
                .stroke(Stroke::NONE)
                .inner_margin(Margin::same(12)),
            icon_color: palette().text_dim,
        }),
    };
    HostLook { dialog: Some(dialog), comments: Some(comments) }
}

/// Our own dialogs (Name version, Keyboard shortcuts) in the same look as WordCraft's under
/// [`look`]: a white card with 8 px corners, a soft shadow and 24 px inside.
pub fn dialog_frame() -> Frame {
    Frame::NONE.fill(palette().card).corner_radius(CornerRadius::same(8)).shadow(shadow(24, 60)).inner_margin(Margin::same(24))
}

/// A dialog's title: Google's 22 px.
pub fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(regular(22.0)).color(palette().text));
}

/// Text fields the Google way: 14 px text, 40 px tall, a grey outline with 4 px corners, 2 px
/// blue while typing.
pub fn field_style(ui: &mut egui::Ui) {
    let st = ui.style_mut();
    st.spacing.interact_size.y = 40.0;
    st.spacing.button_padding = vec2(12.0, 10.0);
    st.text_styles.insert(egui::TextStyle::Body, regular(14.0));
    let w = &mut st.visuals.widgets;
    for (state, stroke) in
        [(&mut w.inactive, FIELD), (&mut w.hovered, palette().text), (&mut w.active, palette().accent), (&mut w.open, palette().accent)]
    {
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.corner_radius = CornerRadius::same(4);
        state.expansion = 0.0;
    }
    st.visuals.selection.stroke = Stroke::new(2.0, palette().accent);
}

/// A dialog button: the default action filled blue, the others blue text, both pill-shaped and
/// 36 px tall.
pub fn pill(ui: &mut egui::Ui, text: &str, filled: bool) -> egui::Response {
    let b = buttons();
    let (fill, colour) = if filled { (b.fill, b.on_fill) } else { (Color32::TRANSPARENT, b.text) };
    let button = egui::Button::new(egui::RichText::new(text).font(b.font.clone()).color(colour))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(255))
        .min_size(vec2(72.0, b.height));
    ui.add(button)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, reason = "tests")]

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use wordcraft_engine::Session;
    use wordcraft_ui_egui::{HostChrome, Services, WordApp};

    fn window() -> Harness<'static, WordApp> {
        let doc = wordcraft_engine::sample::sample_document();
        let mut app =
            WordApp::new(Session::new(doc), Services::default()).with_chrome(HostChrome { title_bar: false, ribbon: false, status_bar: false });
        super::apply(&mut app);
        let mut h = Harness::builder().with_size(egui::vec2(1300.0, 800.0)).build_ui_state(
            |ui, app: &mut WordApp| {
                app.logic(ui.ctx());
                app.ui(ui);
            },
            app,
        );
        h.run_steps(3);
        h
    }

    /// Page setup under our look: our title row and Google's button order, Cancel then OK.
    #[test]
    fn page_setup_takes_our_look() {
        let mut h = window();
        h.state_mut().run("ui.dialog", json!({"name": "pageSetup"})).unwrap();
        h.run_steps(4);
        assert!(h.query_all_by_label("Page Setup").next().is_some(), "Page Setup is open");
        let ok = h.get_by_label("OK").rect();
        let cancel = h.get_by_label("Cancel").rect();
        assert!(cancel.right() <= ok.left(), "Cancel {cancel:?} should sit before OK {ok:?}");
        h.get_by_label("Close").click();
        h.run_steps(3);
        assert!(h.query_all_by_label("Page Setup").next().is_none(), "the round ✕ closes the dialog");
    }

    /// Comments show as Google's cards beside the page (fork StarDocs 0009): no markup column
    /// beside the page, a card per thread, and a new comment opens its card there, not the
    /// Comments pane.
    #[test]
    fn comments_show_as_cards_beside_the_page() {
        let mut h = window();
        let new = h.state_mut().run("review.newComment", json!({"text": "Is the hall free?"})).unwrap();
        h.run_steps(6);
        assert!(!h.state().session.view.comments_pane, "the pane stays closed");
        assert_eq!(wordcraft_ui_egui::canvas::markup_width(h.state()), 0.0, "no Word-style markup column");
        let id = new["id"].as_u64().unwrap();
        let cards = &h.state().canvas.balloon_rects;
        assert!(cards.iter().any(|(c, _)| u64::from(*c) == id), "the new comment has a card: {cards:?}");
        let page = h.state().canvas.page_rects[0];
        let card = cards[0].1;
        assert!(card.left() >= page.right(), "the card {card:?} is beside the page {page:?}, not over it");
    }

    /// Pages side by side keep the comment cards in a column of their own (StarDocs 0017): a
    /// card on page 1 never covers page 2.
    #[test]
    fn side_by_side_pages_keep_comment_cards_off_the_next_page() {
        let mut h = window();
        h.state_mut().session.view.multi_page = true;
        h.state_mut().run("review.newComment", json!({"text": "Is the hall free?"})).unwrap();
        h.run_steps(6);
        let pages = h.state().canvas.page_rects.clone();
        if pages.len() < 2 {
            return; // the sample document must run to two pages for this to mean anything
        }
        assert!(pages[1].min.y < pages[0].max.y, "pages are side by side: {pages:?}");
        let cards = h.state().canvas.balloon_rects.clone();
        assert!(
            !cards.is_empty(),
            "the comment has a card; pages {pages:?} avail {:?} comments {:?}",
            h.state().canvas.page_rects,
            h.state().session.doc.comments.keys().collect::<Vec<_>>()
        );
        for (_, card) in &cards {
            for p in &pages {
                assert!(!card.intersects(*p), "card {card:?} covers page {p:?}; all pages {pages:?}");
            }
        }
    }

    /// A comment made on selected text: the selection keeps its text, and what is typed into
    /// the new comment stays in the comment and out of the page (T1: after the first letter the
    /// page took the keyboard, so the rest replaced the selected text).
    #[test]
    fn typing_in_a_new_comment_leaves_the_page_and_the_selection_alone() {
        use wordcraft_doc::StoryRef;
        let mut h = window();
        let body = h.state().session.doc.plain_text(StoryRef::Body);
        let mut sel = h.state().session.sel.clone();
        sel.anchor.off = 4;
        sel.focus.off = 8;
        h.state_mut().session.sel = sel.clone();
        let selected = h.state().session.selected_text();
        assert!(!selected.is_empty());
        h.state_mut().run("review.newComment", json!({"text": ""})).unwrap();
        // As a click on the toolbar button does: the page asks for the keyboard back.
        h.state_mut().canvas.want_focus = true;
        h.run_steps(5);
        let after = h.state().session.sel.clone();
        assert_eq!(h.state().session.selected_text(), selected, "the selection keeps its text: {sel:?} became {after:?}");
        for letter in ["a", "b", "c", "d"] {
            h.event(egui::Event::Text(letter.into()));
            h.run_steps(3);
        }
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Body), body, "nothing typed reached the page");
        let part = h.state().session.doc.comments.values().next().expect("the comment is still there").part;
        assert_eq!(h.state().session.doc.plain_text(StoryRef::Part(part)), "abcd");
    }

    /// The Comments pane under our look: our header, the empty message, and a card with
    /// Comment / Cancel for a new comment; Cancel removes it.
    #[test]
    fn comments_pane_takes_our_look() {
        let mut h = window();
        h.state_mut().run("view.commentsPane", json!({"value": true})).unwrap();
        h.run_steps(3);
        h.get_by_label("No comments yet");
        h.get_by_label("Add a comment").click();
        h.run_steps(4);
        h.get_by_label("Comment");
        h.get_by_label("Cancel").click();
        h.run_steps(4);
        let left = h.state_mut().session.run("review.comments", &json!({})).unwrap();
        assert_eq!(left.as_array().map(Vec::len), Some(0), "Cancel removes the new comment");
        h.get_by_label("Close").click();
        h.run_steps(3);
        assert!(h.query_by_label("No comments yet").is_none(), "the header's ✕ closes the pane");
    }
}
