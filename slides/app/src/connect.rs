//! File › Connect to a server… in the desktop program (from StarDocs' connect.rs): the server's address, then the code to
//! check in the browser while the app waits for the person to allow it there (starbase-connect
//! does the sign-in on a thread of its own), then "Connected". Disconnect forgets the token here
//! and removes it on the server, also on a thread.

use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use egui::{Button, RichText};
use starbase_connect::{Server, Step};

use crate::theme::Palette;

/// The app's name on the server's "Your devices" list.
const APP: &str = "Starbase Slides";

enum Stage {
    /// Typing the address (and why the last try failed).
    Address(Option<String>),
    /// Waiting for the server's code.
    Starting,
    /// The code to check in the browser; `opened`: why the browser couldn't be opened.
    Code { code: String, url: String, opened: Option<String> },
    /// Connected as this person.
    Done(String),
}

/// The dialog, while open.
#[derive(Default)]
pub struct ConnectDialog {
    stage: Option<Stage>,
    address: String,
    focus: bool,
    signing_in: Option<Receiver<Step>>,
    /// A disconnect on its way: what it says when done.
    disconnecting: Option<Receiver<Result<(), String>>>,
}

impl ConnectDialog {
    /// Opens the dialog at the address box.
    pub fn open(&mut self) {
        self.stage = Some(Stage::Address(None));
        self.focus = true;
    }

    /// Disconnects from `server` (a thread: the server is told, which can take a moment).
    pub fn disconnect(&mut self, server: Server) {
        let (tx, rx) = channel();
        let spawned = std::thread::Builder::new().name("disconnect".into()).spawn(move || {
            let _ = tx.send(server.disconnect());
        });
        if spawned.is_ok() {
            self.disconnecting = Some(rx);
        }
    }

    /// Each frame. Returns a server just connected to (`Some(Some(…))`), or a disconnect done
    /// (`Some(None)`); `status` gets what to say in the status line.
    pub fn ui(&mut self, ctx: &egui::Context, pal: &Palette, status: &mut dyn FnMut(String)) -> Option<Option<Server>> {
        let mut changed = None;
        if let Some(rx) = &self.disconnecting {
            match rx.try_recv() {
                Ok(r) => {
                    status(r.err().unwrap_or_else(|| "Disconnected from the server.".to_string()));
                    self.disconnecting = None;
                    changed = Some(None);
                }
                Err(TryRecvError::Disconnected) => self.disconnecting = None,
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(200)),
            }
        }
        if let Some(rx) = &self.signing_in {
            loop {
                match rx.try_recv() {
                    Ok(Step::Code { user_code, verify_url, opened }) => {
                        self.stage = Some(Stage::Code { code: user_code, url: verify_url, opened: opened.err() });
                    }
                    Ok(Step::Waiting) => {}
                    Ok(Step::Done(server)) => {
                        self.stage = Some(Stage::Done(server.name()));
                        status(format!("Connected to {} as {}.", server.url, server.name()));
                        changed = Some(Some(server));
                        self.signing_in = None;
                        break;
                    }
                    Ok(Step::Failed(why)) => {
                        self.stage = Some(Stage::Address(Some(why)));
                        self.signing_in = None;
                        break;
                    }
                    Err(TryRecvError::Empty) => {
                        ctx.request_repaint_after(Duration::from_millis(250));
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.signing_in = None;
                        break;
                    }
                }
            }
        }
        if self.stage.is_some() {
            self.dialog(ctx, pal);
        }
        changed
    }

    fn dialog(&mut self, ctx: &egui::Context, pal: &Palette) {
        let (ink, text_dim, blue, on_blue) = (pal.text, pal.text_dim, pal.link, pal.on_link);
        let (mut close, mut start) = (false, false);
        let modal = egui::Modal::new(egui::Id::unique("starslides_connect"))
            .frame(egui::Frame::popup(&ctx.global_style()).fill(pal.menu).corner_radius(8.0).inner_margin(20.0));
        let shown = modal.show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Connect to a server").size(18.0).color(ink));
            ui.add_space(8.0);
            match &self.stage {
                Some(Stage::Address(why)) => {
                    ui.label(RichText::new("The address of your Starbase server, as you open it in the browser.").size(14.0).color(text_dim));
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(&mut self.address).hint_text("https://files.example.org").desired_width(f32::INFINITY));
                    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Server address"));
                    // Asked until it has it (the first frames of a new dialog can't take it yet).
                    if self.focus {
                        r.request_focus();
                        self.focus = !r.has_focus();
                    }
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if self.address.trim().to_lowercase().starts_with("http://") {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("http:// isn't encrypted: it works only for a server on your own network, never one on the internet.")
                                .size(13.0)
                                .color(text_dim),
                        );
                    }
                    if let Some(why) = why {
                        ui.add_space(6.0);
                        ui.label(RichText::new(why).size(13.0).color(pal.error));
                    }
                    ui.add_space(10.0);
                    let mut go = enter;
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui.add(Button::new(RichText::new("Connect").color(on_blue)).fill(blue)).clicked() {
                            go = true;
                        }
                    });
                    start = go && !self.address.trim().is_empty();
                }
                Some(Stage::Starting) => {
                    ui.label(RichText::new("Asking the server…").size(14.0).color(text_dim));
                    ui.add_space(10.0);
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
                Some(Stage::Code { code, url, opened }) => {
                    let text = match opened {
                        None => "We opened your browser. Sign in there if it asks, type this code, and allow Starbase Slides.",
                        Some(_) => "Open this address in your browser, sign in if it asks, type this code, and allow Starbase Slides.",
                    };
                    ui.label(RichText::new(text).size(14.0).color(text_dim));
                    ui.add_space(10.0);
                    ui.vertical_centered(|ui| ui.label(RichText::new(code.as_str()).size(30.0).monospace().color(ink)));
                    if opened.is_some() {
                        ui.add_space(6.0);
                        ui.add(egui::Label::new(RichText::new(url.as_str()).size(13.0).color(blue)).selectable(true));
                    }
                    ui.add_space(10.0);
                    ui.label(RichText::new("Waiting for you to allow it…").size(13.0).color(text_dim));
                    ui.add_space(10.0);
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
                Some(Stage::Done(name)) => {
                    ui.label(
                        RichText::new(format!("Connected as {name}. Presentations from the server's Drive now open here."))
                            .size(14.0)
                            .color(text_dim),
                    );
                    ui.add_space(10.0);
                    if ui.add(Button::new(RichText::new("Done").color(on_blue)).fill(blue)).clicked() {
                        close = true;
                    }
                }
                None => {}
            }
        });
        if shown.should_close() {
            close = true;
        }
        if start {
            self.signing_in = Some(starbase_connect::connect(&self.address, APP));
            self.stage = Some(Stage::Starting);
        }
        if close {
            // Dropping the receiver stops the sign-in thread at its next check.
            self.signing_in = None;
            self.stage = None;
        }
    }
}
