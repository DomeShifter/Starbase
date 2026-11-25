//! The desktop program's stand-in for the browser-only `share` module: local documents have no
//! Starbase server to share through (the title row has no Share button, `Link::can_share`),
//! so the Share dialog never opens.

pub mod share {
    use crate::server::Link;
    use wordcraft_ui_egui::WordApp;

    /// The Share dialog (as `share::Share`); without a server it never opens.
    #[derive(Default)]
    pub struct Share;

    impl Share {
        pub fn ask(&mut self, app: &mut WordApp, _link: &Link, _ctx: &egui::Context) {
            app.status("Sharing needs a Starbase server");
        }
        pub fn logic(&mut self, _link: Option<&Link>, _ctx: &egui::Context) {}
        #[cfg_attr(not(test), allow(dead_code, reason = "the test state only"))]
        pub fn is_open(&self) -> bool {
            false
        }
        pub fn waiting(&self) -> bool {
            false
        }
        pub fn ui(&mut self, _ctx: &egui::Context) {}
    }
}

/// The desktop program's stand-in for the browser-only `filedlg` module (File › Move, Move to bin,
/// Details): they need the Starbase server's files.
pub mod filedlg {
    use crate::server::Link;
    use wordcraft_ui_egui::WordApp;

    #[derive(Default)]
    pub struct FileDlg;

    impl FileDlg {
        pub fn ask(&mut self, _kind: &str, app: &mut WordApp, _link: Option<&Link>, _ctx: &egui::Context) {
            app.status("This needs a Starbase server: open the document from the home screen");
        }
        pub fn logic(&mut self, _link: Option<&Link>, _ctx: &egui::Context) {}
        pub fn ui(&mut self, _ctx: &egui::Context) {}
    }
}
