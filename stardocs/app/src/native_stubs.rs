//! Stand-ins for the browser-only `server` and `share` modules, so the window (`shell`) builds
//! natively for its tests (`shell_tests`). The tests have no Starbase server: they pass no `Link`,
//! so `Link` here has no values at all, and the Share dialog never opens.

pub mod server {
    /// What the title row says about saving (as `server::Saved`).
    #[allow(dead_code, reason = "made only by the browser's server link")]
    #[derive(Clone, Default, PartialEq)]
    pub enum Saved {
        #[default]
        Nothing,
        Saving,
        Done,
        Failed(String),
    }

    /// A Starbase server document; never made in native tests.
    pub enum Link {}

    impl Link {
        pub fn view_only(&self) -> bool {
            match *self {}
        }
        pub fn home(&self) {
            match *self {}
        }
        pub fn new_document(&self) {
            match *self {}
        }
        pub fn id(&self) -> Option<String> {
            match *self {}
        }
        pub fn name(&self) -> String {
            match *self {}
        }
        pub fn notice(&self) -> Option<String> {
            match *self {}
        }
        pub fn saved(&self) -> Saved {
            match *self {}
        }
        pub fn download_next(&self) {
            match *self {}
        }
        pub fn rename(&self, _stem: &str) {
            match *self {}
        }
    }
}

pub mod share {
    use super::server::Link;
    use wordcraft_ui_egui::WordApp;

    /// The Share dialog (as `share::Share`); without a server it never opens.
    #[derive(Default)]
    pub struct Share;

    impl Share {
        pub fn ask(&mut self, _app: &mut WordApp, link: &Link, _ctx: &egui::Context) {
            match *link {}
        }
        pub fn logic(&mut self, _link: Option<&Link>, _ctx: &egui::Context) {}
        pub fn is_open(&self) -> bool {
            false
        }
        pub fn waiting(&self) -> bool {
            false
        }
        pub fn ui(&mut self, _ctx: &egui::Context) {}
    }
}
