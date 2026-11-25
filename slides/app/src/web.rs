//! The browser shell: web `Services`, drag-and-drop, the unsaved-changes guard and the eframe web
//! runner. Adapted from DeckCraft's apps/deckcraft-web/src/web.rs (MIT OR Apache-2.0): our window
//! (`shell`) draws the menus and toolbar around DeckCraft's slides, and a presentation opened
//! from the Starbase home screen is read and saved through the server (`server`).

use std::cell::Cell;
use std::rc::Rc;

use deckcraft_engine::Session;
use deckcraft_ui_egui::{Inbox, InboxEvent, InboxFile, Services, SlideApp};
use wasm_bindgen::JsCast as _;

use crate::server::Link;
use crate::shell::{Drive, Shell};

const PRESENTATION_EXTS: &[&str] = &["pptx", "potx", "ppsx", "txt", "md"];
const PICTURE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "m4a", "aac", "flac", "ogg", "opus", "aif", "aiff"];
const VIDEO_EXTS: &[&str] = &["mp4", "m4v", "mov", "webm"];
const CANVAS_ID: &str = "starslides_canvas";
const LOADING_ID: &str = "starslides_loading";

pub fn start() {
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
        // Opened from the Starbase home screen (`?file=` / `?new=`): the server keeps the file
        // (server.rs). Otherwise a page framing the app (`?host=parent`, `data-host="page"`) may
        // keep it (host.rs); the presenter popup is neither.
        let popup = crate::presenter::token().is_some();
        let from_server = !popup && (query_param("file").is_some() || query_param("new").is_some());
        let bridge = if popup || from_server { None } else { crate::host::Bridge::detect(&canvas) };
        let app_bridge = bridge.clone();
        let dirty = Rc::new(Cell::new(false));
        if !popup
            && bridge.is_none()
            && let Err(e) = guard_unload(dirty.clone())
        {
            log::error!("no unsaved-changes guard: {e}");
        }
        // WebGL2 through glow: no WebGPU path, a smaller download (docs/research/wordcraft-trim.md).
        let options = eframe::WebOptions::default();
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                options,
                Box::new(move |cc| {
                    egui_extras::install_image_loaders(&cc.egui_ctx);
                    crate::fonts::add();
                    deckcraft_ui_egui::install_ime_pass_hooks(&cc.egui_ctx);
                    if let Some(token) = crate::presenter::token() {
                        let presenter = crate::presenter::Popup::new(token, cc.egui_ctx.clone()).map_err(std::io::Error::other)?;
                        return Ok(Box::new(presenter) as Box<dyn eframe::App>);
                    }
                    let inbox: Inbox = Inbox::default();
                    let mode = crate::theme::Mode::parse(&crate::prefs::get(crate::theme::KEY).unwrap_or_default());
                    let shell = Shell::new(mode);
                    let mut services = services(inbox.clone(), cc.egui_ctx.clone(), shell.presenter_flag());
                    if let Some(bridge) = &app_bridge {
                        bridge.connect(&mut services, cc.egui_ctx.clone());
                    }
                    let link = if from_server { Link::detect(cc.egui_ctx.clone()) } else { None };
                    if let Some(link) = &link {
                        link.connect(&mut services);
                    }
                    let mut app = SlideApp::new(Session::new(), services).with_chrome(crate::shell::chrome());
                    // The server or the host page sends the presentation; an empty one meanwhile,
                    // which the first file takes the place of.
                    if link.is_some() || app_bridge.is_some() {
                        if let Err(e) = app.start_for_host() {
                            log::error!("new presentation: {e}");
                        }
                    } else if query_has("sample") {
                        if let Err(e) = deckcraft_engine::sample::open_sample(&mut app.session) {
                            log::error!("sample deck: {e}");
                        }
                    } else if let Err(e) = app.run("file.new", serde_json::json!({})) {
                        log::error!("new presentation: {e}");
                    }
                    if let Some(link) = &link {
                        link.open(&mut app);
                    }
                    if app_bridge.is_some()
                        && let Some(author) = query_param("author")
                    {
                        crate::host::Bridge::set_author(&mut app, &author);
                    }
                    crate::api::install(&mut app, cc.egui_ctx.clone());
                    let hook = crate::testhook::TestHook::install(&cc.egui_ctx);
                    let fonts = Rc::new(Cell::new(0));
                    crate::fonts::fetch_more(fonts.clone(), cc.egui_ctx.clone());
                    Ok(Box::new(WebShell {
                        app,
                        inbox,
                        bridge: app_bridge,
                        link,
                        shell,
                        hook,
                        dirty,
                        fonts,
                        fonts_seen: 0,
                        fullscreen: crate::fullscreen::Fullscreen::new(),
                    }))
                }),
            )
            .await;
        if let Some(bridge) = &bridge {
            match &result {
                Ok(()) => bridge.post_ready(),
                Err(e) => bridge.post_failed(&format!("{e:?}")),
            }
        }
        if let Some(el) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => el.remove(),
                Err(e) => el.set_inner_html(&format!("<p>Starbase Slides failed to start: {e:?}</p><p>A browser with WebGL2 is required.</p>")),
            }
        }
    });
}

fn query() -> String {
    web_sys::window().and_then(|w| w.location().search().ok()).unwrap_or_default()
}

fn query_param(key: &str) -> Option<String> {
    web_sys::UrlSearchParams::new_with_str(&query()).ok()?.get(key)
}

/// Whether the URL has the flag `key` (a whole parameter, never a substring).
fn query_has(key: &str) -> bool {
    web_sys::UrlSearchParams::new_with_str(&query()).is_ok_and(|q| q.has(key))
}

struct WebShell {
    app: SlideApp,
    inbox: Inbox,
    bridge: Option<crate::host::Bridge>,
    link: Option<Link>,
    shell: Shell,
    hook: Option<crate::testhook::TestHook>,
    /// Unsaved changes, for the leave-page guard.
    dirty: Rc<Cell<bool>>,
    /// Font faces fetched after start so far (`fonts::fetch_more`), and how many the slides were
    /// last drawn with: when more arrive, the slides are drawn again in them.
    fonts: Rc<Cell<usize>>,
    fonts_seen: usize,
    /// A slideshow fills the screen.
    fullscreen: crate::fullscreen::Fullscreen,
}

impl eframe::App for WebShell {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(bridge) = &self.bridge {
            bridge.open_files(&mut self.app);
        }
        if let Some(link) = &self.link {
            link.before(&mut self.app);
        }
        if self.fonts.get() != self.fonts_seen {
            self.fonts_seen = self.fonts.get();
            self.app.textures.invalidate();
        }
        self.app.logic(ctx);
        if let Some(link) = &self.link {
            link.tick(&mut self.app, ctx);
        }
        if let Some(bridge) = &self.bridge {
            bridge.report_dirty(&self.app);
        }
        self.dirty.set(self.app.session.documents().iter().any(|d| d.is_dirty()));
        self.fullscreen.tick(&mut self.app, ctx);
        if self.app.show.is_none()
            && let Some(host) = self.app.services.presenter.as_mut()
        {
            host.close();
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // Browsers cannot read dropped files synchronously: consume this pass's drops here and
        // deliver them through the inbox once read.
        for f in std::mem::take(&mut raw.dropped_files) {
            let id = self.app.begin_external_file("drop");
            let inbox = self.inbox.clone();
            let ctx = ctx.clone();
            wasm_bindgen_futures::spawn_local(async move {
                let name = f.path().file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "dropped".into());
                match f.bytes_async().await {
                    Ok(bytes) => push_inbox(&inbox, InboxEvent::File(InboxFile { id, name, bytes })),
                    Err(e) => {
                        log::error!("couldn't read dropped file {name}: {e}");
                        push_inbox(&inbox, InboxEvent::Cancelled { id });
                    }
                }
                ctx.request_repaint();
            });
        }
        self.app.raw_input_hook(ctx, raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let drive = self.link.as_ref().map(|l| l as &dyn Drive);
        if let Some(hook) = &self.hook {
            hook.process(&mut self.app, &mut self.shell, drive, ui.ctx());
        }
        self.shell.ui(&mut self.app, ui, drive);
        self.app.ui(ui);
    }
}

/// Web services: the browser file picker (bytes arrive through the inbox) and downloads.
fn services(inbox: Inbox, ctx: egui::Context, asked_presenter: Rc<Cell<bool>>) -> Services {
    let open_inbox = inbox.clone();
    let presenter = match crate::presenter::BrowserHost::new(ctx.clone()) {
        Ok(host) => Some(Box::new(host) as Box<dyn deckcraft_ui_egui::presenter::Host>),
        Err(error) => {
            log::error!("Presenter window: {error}");
            None
        }
    };
    Services {
        presenter,
        open_async: Some(Box::new(move |purpose: &str, request: u64| {
            let inbox = open_inbox.clone();
            let ctx = ctx.clone();
            let dialog = match purpose {
                "picture" => rfd::AsyncFileDialog::new().add_filter("Pictures", PICTURE_EXTS),
                "audio" => rfd::AsyncFileDialog::new().add_filter("Audio", AUDIO_EXTS),
                "video" => rfd::AsyncFileDialog::new().add_filter("Video", VIDEO_EXTS),
                _ => rfd::AsyncFileDialog::new().add_filter("Presentations", PRESENTATION_EXTS),
            };
            wasm_bindgen_futures::spawn_local(async move {
                let Some(file) = dialog.pick_file().await else {
                    push_inbox(&inbox, InboxEvent::Cancelled { id: request });
                    ctx.request_repaint();
                    return;
                };
                let bytes = file.read().await;
                push_inbox(&inbox, InboxEvent::File(InboxFile { id: request, name: file.file_name(), bytes }));
                ctx.request_repaint();
            });
        })),
        write: Some(Box::new(|name: &str, bytes: &[u8]| download(name, bytes))),
        download: Some(Box::new(|name: &str, bytes: &[u8]| {
            if let Err(e) = download(name, bytes) {
                log::error!("download of {name} failed: {e}");
            }
        })),
        inbox: Some(inbox),
        // DeckCraft opens its presenter window when there are two screens. A browser tells only
        // through `screen.isExtended` (Chromium); otherwise only View › Presenter view asks for it,
        // and Slideshow shows the slides alone, full screen, as Google Slides does.
        displays: Some(Box::new(move || Some(if asked_presenter.get() || screen_extended() { 2 } else { 1 }))),
        ..Default::default()
    }
}

/// Whether the browser says the screen is part of a multi-screen setup (`screen.isExtended`).
fn screen_extended() -> bool {
    web_sys::window()
        .and_then(|w| w.screen().ok())
        .and_then(|s| js_sys::Reflect::get(&s, &"isExtended".into()).ok())
        .is_some_and(|v| v.as_bool() == Some(true))
}

fn push_inbox(inbox: &Inbox, event: InboxEvent) {
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push(event);
}

/// Asks before leaving the page while something isn't saved (the server link's saves, or
/// changes in a presentation that has no server behind it).
fn guard_unload(dirty: Rc<Cell<bool>>) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;
    let guard = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(move |e: web_sys::BeforeUnloadEvent| {
        if crate::server::unsaved() || dirty.get() {
            e.prevent_default();
            e.set_return_value("unsaved");
        }
    });
    window.add_event_listener_with_callback("beforeunload", guard.as_ref().unchecked_ref()).map_err(|e| format!("{e:?}"))?;
    guard.forget();
    Ok(())
}

/// Trigger a browser download of `bytes` named after the last component of `path`.
fn download(path: &str, bytes: &[u8]) -> Result<(), String> {
    let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "presentation".into());
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime_for(&name));
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(js)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(js)?.dyn_into().map_err(|_| "not an anchor")?;
    a.set_href(&url);
    a.set_download(&name);
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

fn mime_for(name: &str) -> &'static str {
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("gif") => "image/gif",
        Some("txt") => "text/plain",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => "application/octet-stream",
    }
}
