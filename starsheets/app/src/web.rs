//! StarSheets browser shell. Adapted from GridCraft's `apps/gridcraft-web/src/web.rs` (v0.19.1,
//! MIT OR Apache-2.0, (c) ArtCraft Team and the GridCraft contributors): host bridge and page API
//! removed, StarSheets' own menus, toolbar and theme added around the shared `SheetApp`.
//! The browser shell: web `Services`, drag-and-drop, and the eframe web runner.

use gridcraft_engine::Session;
use gridcraft_ui_egui::{Inbox, Services, SheetApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;

pub(crate) const CANVAS_ID: &str = "starsheets_canvas";
const LOADING_ID: &str = "starsheets_loading";

/// Starts the eframe web runner for the StarSheets canvas. It adjusts the WebGPU device limits to the adapter's, optionally selects the WebGL backend from the query string, and builds the shell with the requested sample or host bridge, browser language rules, stored preferences and the loading/error page state.
pub fn start() {
    crate::fonts::install();
    eframe::WebLogger::init(log::LevelFilter::Info).ok();
    wasm_bindgen_futures::spawn_local(async {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            log::error!("no document");
            return;
        };
        let Some(canvas) = document.get_element_by_id(CANVAS_ID).and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok()) else {
            log::error!("missing <canvas id=\"{CANVAS_ID}\">");
            return;
        };
        let mut options = eframe::WebOptions::default();
        if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_options.wgpu_setup {
            let default_descriptor = std::sync::Arc::clone(&setup.device_descriptor);

            setup.device_descriptor = std::sync::Arc::new(move |adapter| {
                let mut descriptor = default_descriptor(adapter);

                descriptor.required_limits = descriptor.required_limits.or_worse_values_from(&adapter.limits());

                descriptor
            });
        }
        if query().contains("webgl")
            && let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup
        {
            create.instance_descriptor.backends = eframe::wgpu::Backends::GL;
        }
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    let inbox: Inbox = Inbox::default();
                    let mut session = Session::new();
                    // In host mode the host sends the workbook to edit; no sample meanwhile.
                    let q = query();
                    if let Some(name) = q.split(['?', '&']).find_map(|kv| kv.strip_prefix("sample=")) {
                        let _ = session.execute("file.new", serde_json::json!({"sample": name}));
                    } else if q.contains("sample") {
                        let _ = session.execute("file.new", serde_json::json!({"sample": "sales"}));
                    }
                    let services = services(inbox.clone(), cc.egui_ctx.clone());
                    let mut app = SheetApp::new(session, services);
                    // Light theme only for now: the Sheets palette is light (dark mode comes later).
                    app.ui.system_theme = false;
                    // StarSheets draws its own title row, menus and toolbar (crate::shell).
                    // Its alerts and questions too (crate::messages, as house dialogs).
                    app.chrome = gridcraft_ui_egui::Chrome { title_bar: false, file_tab: false, ribbon: false, sheet_tabs: false, questions: false };
                    // Follow the browser's language, except a CJK one. CJK coverage comes from
                    // optional craft-fonts input, not a guaranteed web font set, so a CJK browser
                    // cannot be assumed to render localized labels without tofu. Users can still
                    // choose a language explicitly.
                    let browser = gridcraft_ui_egui::i18n::Language::system();
                    if !matches!(
                        browser,
                        gridcraft_ui_egui::i18n::Language::Zh | gridcraft_ui_egui::i18n::Language::Ja | gridcraft_ui_egui::i18n::Language::Ko
                    ) {
                        app.ui.language = browser;
                    }
                    // Preferences from an earlier visit (#1562), as the desktop app reads `ui.json`
                    // and `prefs.json`; a saved language replaces the browser's.
                    let (stored_ui, stored_prefs) = (stored(crate::prefs::UI_KEY), stored(crate::prefs::PREFS_KEY));
                    crate::prefs::restore(&mut app, stored_ui.as_deref(), stored_prefs.as_deref());
                    // Sheets behaviour, applied after stored preferences so they can't turn it back:
                    // no gliding selection outline, Google's cell keys (Enter edits, Backspace only clears).
                    app.ui.animations = false;
                    app.ui.sheets_keys = true;
                    app.ui.paste_pictures_in_cell = true;
                    app.ui.menu_look = Some(crate::shell::MENU_LOOK);
                    crate::settings::Prefs::load().apply(&mut app);
                    gridcraft_ui_egui::spelling::set(crate::spelling::known);
                    let prefs = crate::prefs::PrefsSync::new(stored_ui, stored_prefs);
                    SheetApp::setup_context_for_language(&cc.egui_ctx, false, app.ui.language);
                    crate::theme::install(&cc.egui_ctx);
                    egui_extras::install_image_loaders(&cc.egui_ctx);
                    install_wake(&cc.egui_ctx);
                    // `?test`: window.starsheets for agents and scripts (testhook.rs, TESTING.md).
                    let test = crate::testhook::TestHook::install(&cc.egui_ctx);
                    Ok(Box::new(WebShell { test, ..WebShell::new(app, inbox, prefs) }))
                }),
            )
            .await;
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>Starbase failed to start: {e:?}</p><p>A browser with WebGPU or WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

struct WebShell {
    app: SheetApp,
    inbox: Inbox,
    shell: crate::shell::Shell,
    bottom: crate::bottom::Bottom,
    /// Conditional formatting, data validation, charts, named and protected ranges (panels.rs).
    panels: crate::panels::SidePanels,
    find: crate::findreplace::FindReplace,
    messages: crate::messages::Messages,
    link: crate::linkcard::LinkCard,
    autolink: crate::autolink::AutoLink,
    dates: crate::datepicker::DatePicker,
    custom_date: crate::customdate::CustomDate,
    custom_number: crate::customnumber::CustomNumber,
    settings: crate::settings::SettingsWindow,
    /// Who gets the keyboard each frame: the grid or a dialog (input.rs).
    input: crate::input::Router,
    notes: crate::notes::Notes,
    calc: crate::calcpreview::CalcPreview,
    merge: crate::merge::MergeGuard,
    files: crate::files::Files,
    copyto: crate::copyto::CopyTo,
    /// Spelling suggestions for a clicked misspelt word (spelling.rs).
    spell: crate::spelling::Suggest,
    /// Cut / Copy / Paste on a right-click in the text being edited (editmenu.rs).
    edit_menu: crate::editmenu::EditMenu,
    /// The home screen, shown instead of a spreadsheet when the page has no file (home.rs).
    home: crate::home::Home,
    live: crate::live::Live,
    account: crate::account::Account,
    share: crate::share::Share,
    error_tip: crate::errortip::ErrorTip,
    webfonts: crate::fonts::WebFontLoader,
    /// When preferences were last compared with what's stored, and whether last frame had input.
    prefs_checked: f64,
    prefs_input: bool,
    prefs: crate::prefs::PrefsSync,
    /// `window.starsheets`, only when the URL has `test` (testhook.rs).
    test: Option<crate::testhook::TestHook>,
}

impl WebShell {
    fn new(app: SheetApp, inbox: Inbox, prefs: crate::prefs::PrefsSync) -> WebShell {
        WebShell {
            app,
            inbox,
            prefs,
            home: crate::home::Home::new(),
            error_tip: crate::errortip::ErrorTip,
            shell: Default::default(),
            bottom: Default::default(),
            panels: Default::default(),
            find: Default::default(),
            messages: Default::default(),
            link: Default::default(),
            autolink: Default::default(),
            dates: Default::default(),
            custom_date: Default::default(),
            custom_number: Default::default(),
            settings: Default::default(),
            input: Default::default(),
            notes: Default::default(),
            calc: Default::default(),
            merge: Default::default(),
            files: Default::default(),
            copyto: Default::default(),
            spell: Default::default(),
            edit_menu: Default::default(),
            live: Default::default(),
            account: Default::default(),
            share: Default::default(),
            webfonts: Default::default(),
            prefs_checked: 0.0,
            prefs_input: false,
            test: None,
        }
    }
}

impl WebShell {
    /// A right-click menu is showing (a cell's, a header's, an object's or the edit menu): only
    /// it shows, no link card, note or tip beside it (to-do 3 row 22).
    fn menu_open(&self) -> bool {
        let g = &self.app.grid;
        g.context_menu.is_some() || g.header_menu.is_some() || g.object_menu.is_some() || g.fill_menu.is_some() || self.edit_menu.is_open()
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// A value stored on an earlier visit, if the browser allows storage.
fn stored(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok()?
}

impl WebShell {
    /// Opens our side panels in place of GridCraft's dialogs; the newest one wins (panels.rs).
    fn side_panels(&mut self, ctx: &egui::Context) {
        self.panels.intercept(&mut self.app, ctx);
        // The link card isn't a side panel, but takes GridCraft's link dialog the same way.
        self.link.intercept(&mut self.app);
    }
}

impl eframe::App for WebShell {
    /// What shows wherever a frame draws nothing: the page colour. eframe's default is a
    /// near-black grey, which flashed in the grid area each time a file was opened or the page
    /// reloaded (the menus and tabs are drawn before the file arrives; the grid is not).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        crate::theme::PAGE.to_normalized_gamma_f32()
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for f in dropped {
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped.xlsx".into());
                match f.bytes_async().await {
                    Ok(bytes) => {
                        inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
                        ctx.request_repaint();
                    }
                    Err(e) => log::error!("couldn't read dropped file {name}: {e}"),
                }
            });
        }
        // Files first: it takes Ctrl+O for its Open dialog before GridCraft's keymap sees it.
        self.account.logic(ctx);
        if self.home.active {
            // The home screen: no spreadsheet runs; dropped files are uploaded where you are.
            self.home.logic(ctx);
            crate::account::set_typing(false);
            crate::unload_guard::set(false);
            self.share.logic(&self.app, ctx, &self.files);
            let dropped: Vec<(String, Vec<u8>)> = std::mem::take(&mut *self.inbox.lock().unwrap_or_else(|e| e.into_inner()));
            for (name, bytes) in dropped {
                self.home.upload(ctx, name, bytes);
            }
            crate::house::keep(ctx);
            return;
        }
        self.files.logic(&mut self.app, ctx);
        // External content (IMAGE pictures): blocked in spreadsheets that aren't this person's own.
        if let Some(d) = self.app.session.active() {
            let file = self.files.linked().filter(|(uid, _)| *uid == d.uid).map(|(_, id)| id);
            crate::external::note_doc(d.uid, file.as_deref(), d.path.is_some(), ctx);
        }
        // Live editing: send this tab's changes, apply everyone else's (live.rs).
        self.live.logic(&mut self.app, ctx, &self.files);
        self.share.logic(&self.app, ctx, &self.files);
        self.copyto.logic(&self.app, ctx, &self.files);
        take_menu_paste(&mut self.app);
        self.files.take_imports(&mut self.app, &self.inbox, ctx);
        self.webfonts.logic(&self.app, ctx);
        crate::spelling::logic(&self.app, ctx);
        crate::images::logic(&mut self.app, ctx);
        crate::house::keep(ctx);
        self.app.logic(ctx);
        // A cell being typed in: renewing the sign-in waits (account.rs).
        crate::account::set_typing(self.app.editor.is_some());
        // Leaving the tab (Ctrl+W, close, reload) asks first while a cell is being typed in or the
        // open file has changes not saved yet; never when everything is saved.
        let dirty = self.app.session.active().is_some_and(|d| d.is_dirty());
        crate::unload_guard::set(crate::unload_guard::should_guard(self.app.editor.is_some(), dirty));
        // Store changed preferences straight away: a reload or closed tab runs no exit code (#1562).
        // Settings change after input (the frame of a click or key and the one after it, as the
        // change happens in `ui`), so only then, and once a second for anything else.
        let input = ctx.input(|i| !i.events.is_empty() || i.pointer.any_down() || i.pointer.any_released());
        let now = ctx.input(|i| i.time);
        let due = input || self.prefs_input || now - self.prefs_checked >= 1.0;
        self.prefs_input = input;
        let changed = if due {
            self.prefs_checked = now;
            self.prefs.changed(&self.app)
        } else {
            Vec::new()
        };
        if !changed.is_empty() {
            match local_storage() {
                Some(store) => {
                    for (key, json) in changed {
                        if let Err(e) = store.set_item(key, &json) {
                            log::warn!("preferences: cannot save {key}: {e:?}");
                        }
                    }
                }
                None => log::warn!("preferences: browser storage is unavailable"),
            }
        }
    }

    /// Forwards a browser frame's raw input to the shared app before eframe handles it.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        crate::emacs_keys::strip(ctx, raw);
        drop_alt_text(raw);
        self.input.raw_input(raw);
        self.autolink.saw_input(raw);
        crate::textmenu::raw_input(raw);
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if let Some(test) = &self.test {
            let parts = crate::testhook::Parts { app: &mut self.app, home: &mut self.home, files: &self.files, compact: self.shell.compact };
            test.process(ui.ctx(), parts);
        }
        // Signed out, or not yet known: the sign-in page (index.html) or first-time setup and
        // sign-up (account.rs dialogs) on a plain page, with nothing of Drive or a file behind.
        if !crate::account::app_shown() {
            let ctx = ui.ctx().clone();
            ui.painter().rect_filled(ui.max_rect(), 0.0, crate::theme::PAGE);
            self.account.ui(&ctx);
            return;
        }
        let open = (!self.home.active).then(|| self.app.session.active().map(|d| d.display_title())).flatten();
        set_tab_title(open.as_deref());
        if self.home.active {
            let ctx = ui.ctx().clone();
            self.home.ui(ui);
            self.share.ui(&ctx);
            self.account.ui(&ctx);
            crate::house::style_tooltips(&ctx);
            return;
        }
        // Version history takes the whole window (versions.rs); the open file stays underneath.
        if self.files.history_open() {
            let ctx = ui.ctx().clone();
            self.files.history_ui(&mut self.app, ui);
            self.messages.ui(&mut self.app, &ctx);
            self.account.ui(&ctx);
            crate::house::style_tooltips(&ctx);
            return;
        }
        // Which box had the keyboard as this frame began (see the check before the grid).
        let focus_at_start = ui.ctx().memory(|m| m.focused());
        // Our panels first, so they sit above GridCraft's formula bar and grid.
        self.shell.ui(&mut self.app, ui);
        self.bottom.ui(&mut self.app, ui);
        // Conditional formatting opens our Sheets-style side panel, not GridCraft's dialogs.
        // Data validation likewise; only one side panel shows at a time.
        let ctx = ui.ctx().clone();
        self.side_panels(&ctx);
        self.panels.ui(&mut self.app, ui);
        // Before GridCraft draws: its alerts are shown as house dialogs instead.
        self.messages.ui(&mut self.app, &ctx);
        self.spell.before(&ctx);
        // The browser's own menu for text being edited (textmenu.rs); one menu at a time.
        crate::textmenu::install(&ctx);
        if crate::textmenu::before(&mut self.app) {
            self.spell.close();
            self.edit_menu.close();
        }
        if self.menu_open() {
            // A card or tip for the cell the pointer was over would show beside the menu (row 22).
            self.app.grid.hover_cell = None;
        }
        self.edit_menu.before(&mut self.app, &ctx);
        self.find.keys(&self.app, &ctx);
        crate::dvpanel::delete_key(&mut self.app);
        // Whether the grid gets this frame's keys: not when a box just let go of them, nor while
        // a dialog is showing (input.rs has the rules).
        self.input.before_grid(&ctx, focus_at_start);
        self.app.ui(ui);
        self.messages.wake(&self.app, &ctx);
        self.input.after_grid(&ctx);
        browser_double_click(&mut self.app, &ctx);
        self.side_panels(&ctx);
        self.autolink.after_frame(&mut self.app);
        if !self.menu_open() {
            self.link.ui(&mut self.app, ui);
        }
        self.link.confirm_ui(&mut self.app, &ctx);
        // Other people's cursors on the grid, and the account dialogs (sign-in on top of all).
        self.live.draw(&self.app, &ctx);
        self.share.ui(&ctx);
        self.account.ui(&ctx);
        self.dates.ui(&mut self.app, ui);
        self.notes.intercept(&mut self.app, &ctx);
        self.notes.ui(&mut self.app, ui);
        self.error_tip.ui(&self.app, &ctx);
        self.settings.ui(&mut self.app, &ctx);
        self.custom_date.ui(&mut self.app, &ctx);
        self.custom_number.ui(&mut self.app, &ctx);
        self.find.intercept(&mut self.app);
        self.find.ui(&mut self.app, &ctx);
        self.find.bar_ui(&mut self.app, &ctx);
        self.calc.ui(&self.app, ui);
        self.merge.ui(&mut self.app, &ctx);
        self.files.ui(&mut self.app, ui);
        self.copyto.ui(&self.app, &ctx);
        self.panels.protect.warning_ui(&mut self.app, &ctx);
        self.spell.ui(&mut self.app, &ctx);
        self.edit_menu.ui(&mut self.app, &ctx);
        crate::textmenu::after(&self.app, &ctx);
        crate::house::style_tooltips(&ctx);
        note_copies(ui.ctx());
        copy_without_https(ui.ctx());
        self.input.end_frame(&ctx);
    }
}

/// On plain http eframe can't copy (it needs a secure context): take this frame's copied text
/// and give it to the page's copy bridge (`__ssCopy` in index.html) instead.
/// Remembers this frame's copied text for menu Paste (editmenu::LAST_COPY), and forgets it once
/// the tab has lost the focus (something else may have been copied).
fn note_copies(ctx: &egui::Context) {
    if web_sys::window().and_then(|w| w.document()).and_then(|d| d.has_focus().ok()) == Some(false) {
        crate::editmenu::forget_copy();
    }
    let last = ctx.output(|o| {
        o.commands.iter().rev().find_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
    });
    if let Some(t) = last {
        crate::editmenu::remember_copy(&t);
    }
}

fn copy_without_https(ctx: &egui::Context) {
    if web_sys::window().is_none_or(|w| w.is_secure_context()) {
        return;
    }
    let mut texts = Vec::new();
    ctx.output_mut(|o| {
        o.commands.retain(|c| match c {
            egui::OutputCommand::CopyText(t) => {
                texts.push(t.clone());
                false
            }
            _ => true,
        });
    });
    if let Some(t) = texts.pop() {
        page_copy(&t, None);
    }
}

/// Calls the page's copy bridge; true when the text reached the clipboard.
fn page_copy(text: &str, html: Option<&str>) -> bool {
    let Some(w) = web_sys::window() else { return false };
    let Ok(f) = js_sys::Reflect::get(&w, &"__ssCopy".into()).and_then(|f| f.dyn_into::<js_sys::Function>()) else {
        return false;
    };
    let html = html.map_or(JsValue::UNDEFINED, JsValue::from_str);
    f.call2(&JsValue::NULL, &JsValue::from_str(text), &html).is_ok_and(|v| v.is_truthy())
}

/// What a page clipboard read gave: text and HTML, or nothing.
type PageClip = Option<(String, Option<String>)>;

thread_local! {
    /// Answers of the page's clipboard reads, waiting for the next frame (`take_menu_paste`).
    static MENU_PASTE: std::cell::RefCell<Vec<PageClip>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Menu Paste (Edit > Paste, a cell's right-click Paste): start the page's async clipboard read
/// (index.html `__ssPasteRead`) inside the click; the answer is queued and `take_menu_paste`
/// pastes it next frame the way Ctrl+V does. False when the page can't read at all.
fn menu_paste_start(ctx: &egui::Context) -> bool {
    let Some(w) = web_sys::window() else { return false };
    if !w.is_secure_context() {
        return false;
    }
    let Some(f) = js_sys::Reflect::get(&w, &"__ssPasteRead".into()).ok().and_then(|f| f.dyn_into::<js_sys::Function>().ok()) else {
        return false;
    };
    let Ok(promise) = f.call0(&JsValue::NULL).and_then(|p| p.dyn_into::<js_sys::Promise>()) else { return false };
    let ctx = ctx.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let got = wasm_bindgen_futures::JsFuture::from(promise).await.ok().and_then(|v| {
            let text = js_sys::Reflect::get(&v, &"text".into()).ok()?.as_string().filter(|t| !t.is_empty())?;
            let html = js_sys::Reflect::get(&v, &"html".into()).ok().and_then(|h| h.as_string());
            Some((text, html))
        });
        MENU_PASTE.with(|q| q.borrow_mut().push(got));
        ctx.request_repaint();
    });
    true
}

/// Applies the clipboard read a menu Paste started: its text and HTML through the same
/// `paste_external` as Ctrl+V; when the page couldn't read, StarSheets' own last copy, else the
/// engine's own clipboard (which says "Nothing to paste." if empty).
fn take_menu_paste(app: &mut gridcraft_ui_egui::SheetApp) {
    let answers: Vec<_> = MENU_PASTE.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for answer in answers {
        match answer {
            Some((text, html)) => {
                if let Some(w) = web_sys::window() {
                    let _ = js_sys::Reflect::set(&w, &"__ssPasteHtml".into(), &html.map_or(JsValue::NULL, |h| JsValue::from_str(&h)));
                }
                app.paste_external(text.replace("\r\n", "\n"));
            }
            None => match crate::editmenu::last_copy() {
                Some(text) => app.paste_external(text),
                None => app.run_or_alert("edit.paste", serde_json::json!({})),
            },
        }
    }
}

fn services(inbox: Inbox, ctx: egui::Context) -> Services {
    let open_inbox = inbox.clone();
    let picture_ctx = ctx.clone();
    Services {
        copy_html: Some(Box::new(copy_html)),
        // Menu Paste: read the real clipboard when chosen (answered next frame, take_menu_paste);
        // plain http / no clipboard API: StarSheets' own last copy.
        paste_start: Some(Box::new({
            let ctx = ctx.clone();
            move || menu_paste_start(&ctx)
        })),
        paste_text: Some(Box::new(crate::editmenu::last_copy)),
        // The HTML that came with this paste (index.html keeps it from the browser's paste event).
        paste_html: Some(Box::new(take_paste_html)),
        open_async: Some(Box::new(move || {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new()
                    .add_filter("Spreadsheets", &["xlsx", "xlsm", "xls", "xlsb", "ods", "csv", "tsv", "txt", "slk", "json"])
                    .pick_file()
                    .await
                else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(|e| e.into_inner()).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        pick_picture_async: Some(Box::new(move |inbox| {
            let ctx = picture_ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = rfd::AsyncFileDialog::new().add_filter("Pictures (PNG, JPEG)", &["png", "jpg", "jpeg"]).pick_file().await else {
                    return;
                };
                let bytes = file.read().await;
                inbox.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push((file.file_name(), bytes));
                ctx.request_repaint();
            });
        })),
        download: Some(Box::new(|name: &str, bytes: &[u8]| {
            if let Err(e) = download(name, bytes) {
                log::error!("download of {name} failed: {e}");
            }
        })),
        print: Some(Box::new(|bytes: &[u8]| {
            if let Err(e) = print_pdf(bytes) {
                log::error!("print failed: {e}");
            }
        })),
        open_url: Some(Box::new(|url: &str| {
            if let Some(w) = web_sys::window()
                && let gridcraft_engine::links::LinkKind::External(url) = gridcraft_engine::links::classify(url)
            {
                // With `noopener` the new browsing context can't hold a reference back to the
                // editor window and navigate it while the user reads the linked page. It must be a
                // window *feature*: inside the target string it would only name the new window.
                let _ = w.open_with_url_and_target_and_features(&url, "_blank", "noopener,noreferrer");
            }
        })),
        inbox: Some(inbox),
        ..Default::default()
    }
}

/// Clipboard HTML longer than this (UTF-16 units, as the browser counts) is left out of a paste:
/// the cells' data is at most ~2.7 MB of it, and a huge page copied elsewhere would only be
/// copied into the app and scanned for nothing. The plain text still pastes.
const MAX_PASTE_HTML: u32 = 8 * 1024 * 1024;

/// The clipboard HTML of the paste being handled, taken so a later paste never sees it.
fn take_paste_html() -> Option<String> {
    let w = web_sys::window()?;
    let value = js_sys::Reflect::get(&w, &"__ssPasteHtml".into()).ok()?;
    let _ = js_sys::Reflect::set(&w, &"__ssPasteHtml".into(), &JsValue::NULL);
    // Measured on the JavaScript side, before the text is copied into the app's memory.
    let text = value.dyn_into::<js_sys::JsString>().ok()?;
    if text.length() == 0 || text.length() > MAX_PASTE_HTML {
        return None;
    }
    text.as_string()
}

/// Start inside the copy gesture; deferring `write` can lose browser user activation.
fn copy_html(html: &str, text: &str) -> Result<(), String> {
    crate::editmenu::remember_copy(text);
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    // The page's copy bridge first (see index.html), on https too: it fills the browser's own
    // copy event, which keeps the HTML as written. The async clipboard API cleans HTML up on
    // write, which may drop the data another Starbase Sheets tab pastes the cells from.
    if page_copy(text, Some(html)) {
        return Ok(());
    }
    // Plain http: no clipboard API either.
    if !window.is_secure_context() {
        return Err("the browser refused to copy (plain http)".into());
    }
    let clipboard = window.navigator().clipboard();
    if clipboard.is_undefined() || clipboard.is_null() {
        return Err("browser clipboard is unavailable (a secure context is required)".into());
    }
    if !js_sys::Reflect::get(&clipboard, &"write".into()).map_err(js)?.is_function() {
        return Err("browser does not support rich clipboard writes".into());
    }
    let data = js_sys::Object::new();
    for (mime, value) in [("text/html", html), ("text/plain", text)] {
        let opts = web_sys::BlobPropertyBag::new();
        opts.set_type(mime);
        let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(value));
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &opts).map_err(js)?;
        js_sys::Reflect::set(&data, &mime.into(), &blob).map_err(js)?;
    }
    let item = web_sys::ClipboardItem::new_with_record_from_str_to_blob_promise(&data).map_err(js)?;
    let promise = clipboard.write(&js_sys::Array::of1(&item));
    let text = text.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(e) = wasm_bindgen_futures::JsFuture::from(promise).await {
            log::warn!("Rich clipboard write failed; trying plain text: {e:?}");
            if let Err(e) = wasm_bindgen_futures::JsFuture::from(clipboard.write_text(&text)).await {
                log::error!("Plain-text clipboard fallback also failed: {e:?}");
            }
        }
    });
    Ok(())
}

/// Opens `bytes` (a PDF) as an object URL in a hidden iframe and calls the iframe's own
/// `print()` once it has finished loading — the standard way to drive the browser's native print
/// dialog on a PDF without a download or a popup window the browser might block. The blob URL and
/// the iframe are both cleaned up once the print dialog closes, so repeated prints don't leak
/// them.
fn print_pdf(bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;

    let iframe: web_sys::HtmlIFrameElement = document.create_element("iframe").map_err(js)?.dyn_into().map_err(|_| "not an iframe")?;
    iframe.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;

    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type("application/pdf");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;

    if let Err(e) = body.append_child(&iframe) {
        cleanup_print(&iframe, &url);
        return Err(js(e));
    }

    // `onload` fires once the PDF has actually rendered inside the iframe;
    // printing before that would show a blank page. Set the handler BEFORE
    // `src` so a fast/cached load can't fire before we're listening.
    let iframe_for_load = iframe.clone();
    let url_for_cleanup = url.clone();
    let on_load = wasm_bindgen::closure::Closure::once_into_js(move || {
        iframe_for_load.set_onload(None);
        if let Err(e) = print_loaded_pdf(&iframe_for_load, &url_for_cleanup) {
            // Firefox shows a PDF in a frame with its own viewer, which a page may not ask to
            // print ("Permission denied … cross-origin object"): open the PDF in a new tab
            // instead, where the browser's print button is one click away (to-do row 70).
            iframe_for_load.remove();
            log::info!("printing in the page isn't allowed here ({e:?}); opening the PDF in a new tab");
            let opened = web_sys::window().and_then(|w| w.open_with_url_and_target(&url_for_cleanup, "_blank").ok().flatten()).is_some();
            if !opened {
                log::error!("print failed: the browser blocked the new tab");
            }
            // The new tab has its own copy once loaded; let go of ours a minute later.
            let url = url_for_cleanup.clone();
            let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
                web_sys::Url::revoke_object_url(&url).ok();
            });
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 60_000);
            }
        }
    });
    iframe.set_onload(Some(on_load.unchecked_ref()));
    iframe.set_src(&url);
    Ok(())
}

fn cleanup_print(iframe: &web_sys::HtmlIFrameElement, url: &str) {
    // Both operations are idempotent: print can fail after dispatching `afterprint`.
    web_sys::Url::revoke_object_url(url).ok();
    iframe.remove();
}

fn print_loaded_pdf(iframe: &web_sys::HtmlIFrameElement, url: &str) -> Result<(), JsValue> {
    let content_window = iframe.content_window().ok_or_else(|| JsValue::from_str("no print content window"))?;
    let iframe_for_cleanup = iframe.clone();
    let url_for_cleanup = url.to_owned();
    let on_after_print = wasm_bindgen::closure::Closure::once(move || {
        cleanup_print(&iframe_for_cleanup, &url_for_cleanup);
    });
    // Listen before `print()`, which can dispatch `afterprint` before it returns.
    // A one-shot listener also releases the callback when the dialog closes or is cancelled.
    let options = web_sys::AddEventListenerOptions::new();
    options.set_once(true);
    let callback = on_after_print.as_ref().unchecked_ref();
    content_window.add_event_listener_with_callback_and_add_event_listener_options("afterprint", callback, &options)?;
    if let Err(e) = content_window.print() {
        let _ = content_window.remove_event_listener_with_callback("afterprint", callback);
        return Err(e);
    }
    let _ = on_after_print.into_js_value();
    Ok(())
}

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("xlsm") => "application/vnd.ms-excel.sheet.macroEnabled.12",
        Some("xlsb") => "application/vnd.ms-excel.sheet.binary.macroEnabled.12",
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet",
        Some("csv") => "text/csv",
        Some("tsv" | "txt") => "text/plain",
        Some("pdf") => "application/pdf",
        Some("html" | "htm") => "text/html",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// The browser's file picker for any kind of file: a hidden `<input type=file multiple>` clicked
/// at once (inside the user's click), read with `Blob.arrayBuffer`. `got` is called for each file.
/// (The `rfd` crate's picker opened in Firefox but never delivered the files.)
pub(crate) fn pick_any(got: Box<dyn Fn(String, Vec<u8>)>) {
    use wasm_bindgen::closure::Closure;
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else { return };
    let Some(input) = doc.create_element("input").ok().and_then(|e| e.dyn_into::<web_sys::HtmlInputElement>().ok()) else { return };
    input.set_type("file");
    input.set_multiple(true);
    let _ = input.set_attribute("style", "display:none");
    if let Some(body) = doc.body() {
        let _ = body.append_child(&input);
    }
    let got: std::rc::Rc<dyn Fn(String, Vec<u8>)> = std::rc::Rc::from(got);
    let i2 = input.clone();
    let done = Closure::<dyn FnMut()>::new(move || {
        if let Some(files) = i2.files() {
            for n in 0..files.length() {
                let Some(file) = files.get(n) else { continue };
                let got = got.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    if let Ok(buf) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                        got(file.name(), js_sys::Uint8Array::new(&buf).to_vec());
                    }
                });
            }
        }
        i2.remove();
    });
    let _ = input.add_event_listener_with_callback("change", done.as_ref().unchecked_ref());
    let _ = input.add_event_listener_with_callback("cancel", done.as_ref().unchecked_ref());
    done.forget();
    input.click();
}

/// Trigger a browser save dialog or download of `bytes` named after `name`.
pub(crate) fn download_file(name: &str, bytes: &[u8]) -> Result<(), String> {
    download(name, bytes)
}

fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let window = web_sys::window().ok_or("no window")?;

    let show_picker_fn: Option<js_sys::Function> =
        js_sys::Reflect::get(&window, &JsValue::from_str("showSaveFilePicker")).ok().and_then(|v| v.dyn_into().ok());

    if let Some(picker_fn) = show_picker_fn {
        let ext = std::path::Path::new(name).extension().and_then(std::ffi::OsStr::to_str).unwrap_or("xlsx").to_ascii_lowercase();
        let dot_ext = format!(".{ext}");
        let mime = mime_for(name);

        let accept_obj = js_sys::Object::new();
        let exts_arr = js_sys::Array::new();
        exts_arr.push(&JsValue::from_str(&dot_ext));
        let _ = js_sys::Reflect::set(&accept_obj, &JsValue::from_str(mime), &exts_arr);

        let type_desc = match ext.as_str() {
            "xlsx" => "Excel Workbook (*.xlsx)",
            "xlsm" => "Excel Macro-Enabled Workbook (*.xlsm)",
            "xlsb" => "Excel Binary Workbook (*.xlsb)",
            "ods" => "OpenDocument Spreadsheet (*.ods)",
            "csv" => "CSV (Comma delimited) (*.csv)",
            "tsv" => "Tab Delimited Text (*.tsv)",
            "txt" => "Plain Text (*.txt)",
            "pdf" => "PDF Document (*.pdf)",
            "html" | "htm" => "HTML Document (*.html)",
            "json" => "JSON Document (*.json)",
            "png" => "PNG Image (*.png)",
            "jpg" | "jpeg" => "JPEG Image (*.jpeg)",
            _ => "Spreadsheet",
        };

        let type_obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&type_obj, &JsValue::from_str("description"), &JsValue::from_str(type_desc));
        let _ = js_sys::Reflect::set(&type_obj, &JsValue::from_str("accept"), &accept_obj);

        let types_arr = js_sys::Array::new();
        types_arr.push(&type_obj);

        let picker_opts = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&picker_opts, &JsValue::from_str("suggestedName"), &JsValue::from_str(name));
        let _ = js_sys::Reflect::set(&picker_opts, &JsValue::from_str("types"), &types_arr);

        let prom_val = picker_fn.call1(&window, &picker_opts).map_err(js)?;
        let prom: js_sys::Promise = prom_val.dyn_into().map_err(|_| "not a promise".to_string())?;

        let bytes_vec = bytes.to_vec();
        let name_owned = name.to_string();
        // Anything but the user cancelling (no user gesture, a denied permission, a failed write)
        // falls back to an ordinary download, so a save never fails silently.
        let fallback = move |bytes: &[u8]| {
            if let Some(w) = web_sys::window()
                && let Err(e) = trigger_anchor_download(&w, &name_owned, bytes)
            {
                log::error!("download fallback failed: {e}");
            }
        };
        wasm_bindgen_futures::spawn_local(async move {
            match wasm_bindgen_futures::JsFuture::from(prom).await {
                Ok(handle_val) => {
                    if let Err(e) = write_file_handle(&handle_val, &bytes_vec).await {
                        log::error!("failed writing to file handle: {e}");
                        fallback(&bytes_vec);
                    } else {
                        log::info!("Successfully saved file through save file picker");
                    }
                }
                Err(e) => {
                    let err_name = js_sys::Reflect::get(&e, &JsValue::from_str("name")).ok().and_then(|v| v.as_string()).unwrap_or_default();
                    if err_name == "AbortError" {
                        log::info!("Save file dialog cancelled by user");
                    } else {
                        log::warn!("showSaveFilePicker refused ({err_name}); downloading instead");
                        fallback(&bytes_vec);
                    }
                }
            }
        });
        return Ok(());
    }

    // Fallback for browsers without showSaveFilePicker (e.g. Firefox, Safari):
    trigger_anchor_download(&window, name, bytes)
}

async fn write_file_handle(handle: &JsValue, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let create_fn: js_sys::Function = js_sys::Reflect::get(handle, &JsValue::from_str("createWritable"))
        .map_err(js)?
        .dyn_into()
        .map_err(|_| "createWritable is not a function".to_string())?;
    let stream_prom: js_sys::Promise = create_fn.call0(handle).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    let stream = wasm_bindgen_futures::JsFuture::from(stream_prom).await.map_err(js)?;

    let write_fn: js_sys::Function =
        js_sys::Reflect::get(&stream, &JsValue::from_str("write")).map_err(js)?.dyn_into().map_err(|_| "write is not a function".to_string())?;
    let uint8 = js_sys::Uint8Array::from(bytes);
    let write_prom: js_sys::Promise = write_fn.call1(&stream, &uint8).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(write_prom).await.map_err(js)?;

    let close_fn: js_sys::Function =
        js_sys::Reflect::get(&stream, &JsValue::from_str("close")).map_err(js)?.dyn_into().map_err(|_| "close is not a function".to_string())?;
    let close_prom: js_sys::Promise = close_fn.call0(&stream).map_err(js)?.dyn_into().map_err(|_| "not a promise".to_string())?;
    wasm_bindgen_futures::JsFuture::from(close_prom).await.map_err(js)?;
    Ok(())
}

fn trigger_anchor_download(window: &web_sys::Window, name: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor".to_string())?;
    a.set_href(&url);
    a.set_download(name);
    a.style().set_property("display", "none").map_err(js)?;
    let body = document.body().ok_or("no body")?;
    body.append_child(&a).map_err(js)?;
    a.click();
    a.remove();
    let revoke = wasm_bindgen::closure::Closure::once_into_js(move || {
        web_sys::Url::revoke_object_url(&url).ok();
    });
    window.set_timeout_with_callback_and_timeout_and_arguments_0(revoke.unchecked_ref(), 10_000).map_err(js)?;
    Ok(())
}

/// A double-click on a cell that GridCraft's own timer missed (a slow touchpad double-tap) still
/// starts editing it: the browser reports its `dblclick` (index.html), which follows the
/// computer's double-click speed. The first click has already selected the cell.
fn browser_double_click(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(w) = web_sys::window() else { return };
    let v = js_sys::Reflect::get(&w, &"__ssDbl".into()).unwrap_or(JsValue::NULL);
    if v.is_null() || v.is_undefined() {
        return;
    }
    let _ = js_sys::Reflect::set(&w, &"__ssDbl".into(), &JsValue::NULL);
    let arr = js_sys::Array::from(&v);
    let (Some(x), Some(y)) = (arr.get(0).as_f64(), arr.get(1).as_f64()) else { return };
    let p = egui::pos2(x as f32, y as f32) / ctx.zoom_factor();
    let on_grid = app.grid.cells_rect.is_some_and(|r| r.contains(p)) && ctx.layer_id_at(p).is_none_or(|l| l.order == egui::Order::Background);
    if on_grid && app.editor.is_none() && app.dialog.is_none() && app.selected_object.is_none() && app.grid.list_picker.is_none() {
        app.begin_edit(None, false);
        ctx.request_repaint();
    }
}

/// For Alt (without Ctrl) the browser layer sends the typed character before the key press, so
/// the grid started editing with "5" or "%" and Alt+Shift+5 (strikethrough) never ran (to-do row
/// 67). The character right before an Alt shortcut key is dropped. AltGr typing (é, €) is left
/// alone: browsers report it with Ctrl+Alt or as AltGraph, not as Alt alone.
fn drop_alt_text(raw: &mut egui::RawInput) {
    let mut i = 1;
    while i < raw.events.len() {
        let alt_key = matches!(&raw.events[i], egui::Event::Key { pressed: true, modifiers: m, .. } if m.alt && !m.ctrl && !m.command);
        if alt_key && matches!(raw.events[i - 1], egui::Event::Text(_)) {
            raw.events.remove(i - 1);
        } else {
            i += 1;
        }
    }
}

/// `window.__ssWake()` asks for a frame at once. index.html calls it after counting something the
/// app reads each frame (the mouse's back and forward buttons, the sign-up link): a synthetic
/// window event doesn't wake eframe, so those waited for the next timed repaint, up to a second
/// (to-do row 81: Back was sometimes slow, sometimes quick).
fn install_wake(ctx: &egui::Context) {
    let ctx = ctx.clone();
    let wake = wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || ctx.request_repaint());
    if let Some(w) = web_sys::window() {
        let _ = js_sys::Reflect::set(&w, &"__ssWake".into(), wake.as_ref());
    }
    // Lives as long as the page.
    wake.forget();
}

/// The browser tab's title and icon: the open file's name only (no ".xlsx", no "Starbase Sheets") with
/// the StarSheets icon, as Google Sheets' tab shows the file's name; "Starbase Drive" with the
/// Starbase Drive icon on the home screen. eframe's web runner ignores `ViewportCommand::Title`, so the
/// page's own `document.title` and icon links (index.html) are set here, and only when they change.
fn set_tab_title(file: Option<&str>) {
    thread_local!(static SHOWN: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) });
    let title = match file {
        Some(name) => {
            let lower = name.to_ascii_lowercase();
            let stem = [".xlsx", ".xlsm", ".xls", ".csv", ".tsv", ".ods"]
                .iter()
                .find(|e| lower.ends_with(*e))
                .map_or(name, |e| &name[..name.len() - e.len()]);
            if stem.trim().is_empty() { "Untitled spreadsheet".to_string() } else { stem.to_string() }
        }
        None => "Starbase Drive".to_string(),
    };
    let icon = if file.is_some() { "favicon" } else { "stardrive" };
    let shown = format!("{icon}\n{title}");
    SHOWN.with(|s| {
        if *s.borrow() != shown
            && let Some(doc) = web_sys::window().and_then(|w| w.document())
        {
            doc.set_title(&title);
            for (id, ext) in [("tab_icon_ico", "ico"), ("tab_icon_svg", "svg")] {
                if let Some(link) = doc.get_element_by_id(id) {
                    let _ = link.set_attribute("href", &format!("{icon}.{ext}"));
                }
            }
            *s.borrow_mut() = shown;
        }
    });
}
