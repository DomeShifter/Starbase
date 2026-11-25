//! The desktop program's stand-in for the browser-only `share` module: local documents have no
//! Starbase server to share through (the title row's Share button is greyed, `Link::can_share`),
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
