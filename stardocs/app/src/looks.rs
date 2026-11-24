//! Google Docs' look for the parts WordCraft draws itself: its dialogs (Page setup, Word count,
//! Find and replace, Insert link, Table properties …) and the Comments pane. WordCraft takes the
//! look through its host style hook (`WordApp::set_host_look`, fork commit "StarDocs 0008");
//! web.rs sets it once at startup. Colours match the rest of our interface (shell.rs, share.rs).

use egui::{Color32, CornerRadius, Frame, Margin, Shadow, Stroke, vec2};
use wordcraft_ui_egui::theme::{medium, regular};
use wordcraft_ui_egui::{ButtonLook, CommentsLook, DialogLook, HostLook, WordApp};

const BLUE: Color32 = Color32::from_rgb(0x0B, 0x57, 0xD0);
const TEXT: Color32 = Color32::from_rgb(0x1F, 0x1F, 0x1F);
const TEXT_DIM: Color32 = Color32::from_rgb(0x44, 0x47, 0x46);
const META: Color32 = Color32::from_rgb(0x5F, 0x63, 0x68);

/// Gives WordCraft's dialogs and Comments pane our look.
pub fn apply(app: &mut WordApp) {
    app.set_host_look(look());
}

/// Google's buttons: OK is a filled blue pill with white text, Cancel a blue text button.
fn buttons() -> ButtonLook {
    ButtonLook {
        font: medium(14.0),
        height: 36.0,
        padding: 24.0,
        fill: BLUE,
        on_fill: Color32::WHITE,
        text: BLUE,
        hover: Color32::from_rgba_unmultiplied(0x0B, 0x57, 0xD0, 0x14),
    }
}

/// A white card lifted by a soft shadow, as Google's dialogs and comment cards are.
fn shadow(blur: u8, alpha: u8) -> Shadow {
    Shadow { offset: [0, 2], blur, spread: 0, color: Color32::from_black_alpha(alpha) }
}

pub fn look() -> HostLook {
    let dialog = DialogLook {
        frame: Frame::NONE
            .fill(Color32::WHITE)
            .corner_radius(CornerRadius::same(8))
            .shadow(shadow(24, 60))
            .stroke(Stroke::NONE)
            .inner_margin(Margin::same(24)),
        title_font: regular(22.0),
        title_color: TEXT,
        body_size: 14.0,
        heading_font: medium(14.0),
        item_spacing: vec2(10.0, 10.0),
        row_height: 28.0,
        min_width: 420.0,
        close_color: TEXT_DIM,
        buttons: buttons(),
    };
    let comments = CommentsLook {
        panel_fill: Color32::WHITE,
        panel_stroke: Stroke::new(1.0, Color32::from_rgb(0xE1, 0xE3, 0xE1)),
        header_font: regular(18.0),
        header_color: TEXT,
        card: Frame::NONE
            .fill(Color32::WHITE)
            .corner_radius(CornerRadius::same(8))
            .shadow(shadow(6, 40))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0xE8, 0xEA, 0xED)))
            .inner_margin(Margin::same(12)),
        card_gap: 12.0,
        author_font: medium(14.0),
        author_color: TEXT,
        meta_font: regular(12.0),
        meta_color: META,
        text_font: regular(14.0),
        icon_color: TEXT_DIM,
        buttons: buttons(),
    };
    HostLook { dialog: Some(dialog), comments: Some(comments) }
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
