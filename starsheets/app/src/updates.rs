//! Update checks in the desktop program (docs/privacy.md): Starbase makes no connection unless
//! asked. GridCraft's updater is pointed at Starbase's own releases (fork change StarSheets 0099),
//! the check at start runs only when File › Settings › Check for updates is on, and Settings'
//! "Check for updates now" asks once. The web app has neither: the server's admin updates it.

use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::craft_update::app::App;

/// Starbase as the update check knows it: its releases, its name, its settings folder.
pub static APP: App = App {
    developer: "Starbase",
    ..App::new("starbase", "Starbase", "Docs, Sheets and Drive for your own server", starbase_connect::RELEASES_REPO)
};

/// Points the updater at Starbase's releases, and starts the check at start only when `on`
/// (the person's setting, [`starbase_connect::update_check_on`]).
pub fn setup(app: &mut SheetApp, on: bool) {
    app.set_update_app(&APP, starbase_connect::VERSION);
    if on {
        app.start_update_check();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Launched as native.rs launches it with the setting off: frames go by, no check starts.
    #[test]
    fn no_update_check_starts_at_launch() {
        let mut app = SheetApp::new(gridcraft_engine::Session::new(), gridcraft_ui_egui::Services::default());
        setup(&mut app, false);
        let ctx = egui::Context::default();
        for _ in 0..5 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.logic(ui.ctx()));
            out.textures_delta.clear();
        }
        assert_eq!(app.update_checks_started(), 0);
    }
}
