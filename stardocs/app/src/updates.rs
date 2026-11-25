//! Update checks in the desktop program (docs/privacy.md): Starbase makes no connection unless
//! asked. WordCraft's updater is pointed at Starbase's own releases (fork change StarDocs 0011),
//! the check at start runs only when Help › Check for updates is ticked, and Help › Check for
//! updates now asks once. The web app has neither: the server's admin updates it.

use serde_json::json;
use wordcraft_ui_egui::WordApp;
use wordcraft_ui_egui::craft_update::app::App;

/// Starbase as the update check knows it: its releases, its name, its settings folder.
pub static APP: App = App {
    developer: "Starbase",
    ..App::new("starbase", "Starbase", "Docs, Sheets and Drive for your own server", starbase_connect::RELEASES_REPO)
};

/// Points the updater at Starbase's releases. No check starts here.
pub fn setup(app: &mut WordApp) {
    app.set_update_app(&APP, starbase_connect::VERSION);
}

/// The check at start, only when the person turned it on.
pub fn start_if_on(app: &mut WordApp) {
    app.set_startup_update_check(starbase_connect::update_check_on());
}

/// Help's two update rows, under About (desktop only).
pub fn menu(app: &mut WordApp, ui: &mut egui::Ui, width: f32, column: bool) {
    let on = starbase_connect::update_check_on();
    if ui.add(crate::shell::menu_row(on.then_some("check"), column, "Check for updates", "", width)).clicked() {
        match starbase_connect::set_update_check(!on) {
            // Turned on: look now, as the next start would.
            Ok(()) => app.set_startup_update_check(!on),
            Err(e) => app.status(e),
        }
        ui.close();
    }
    if ui.add(crate::shell::menu_row(None, column, "Check for updates now", "", width)).clicked() {
        crate::shell::run(app, "help.checkForUpdates", json!({}));
        ui.close();
    }
}
