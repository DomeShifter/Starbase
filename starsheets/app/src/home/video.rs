//! The browser's own `<video>` (with its own controls) laid over the photo viewer's picture area
//! while a video is shown. egui draws no video, so this is a real page element placed over the
//! canvas each frame; the top bar and the Info panel stay egui's and stay clickable, because the
//! element covers only the picture area. It never keeps the keyboard: when it takes focus the
//! canvas gets it back, so the arrow keys, Esc and the viewer's other keys keep working.
//! Native builds (tests) have no page: both calls do nothing.

use egui::Rect;

/// The id of the element, for tests (`document.getElementById`).
#[cfg(target_arch = "wasm32")]
pub(super) const VIDEO_ID: &str = "starbase_photo_video";

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::{Rect, VIDEO_ID};
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::closure::Closure;

    fn document() -> Option<web_sys::Document> {
        web_sys::window()?.document()
    }

    /// Shows the video of file `id` over `area` (egui points), creating the element if needed.
    pub fn show(id: &str, area: Rect, ppp: f32) {
        let (Some(win), Some(doc)) = (web_sys::window(), document()) else { return };
        let src = format!("api/files/{id}/raw");
        let el = match doc.get_element_by_id(VIDEO_ID) {
            Some(e) => e,
            None => {
                let Ok(e) = doc.create_element("video") else { return };
                e.set_id(VIDEO_ID);
                let _ = e.set_attribute("controls", "");
                let _ = e.set_attribute("autoplay", "");
                let _ = e.set_attribute("playsinline", "");
                let _ = e.set_attribute("preload", "metadata");
                // Clicking the controls must not steal the keyboard from the viewer.
                let give_back = Closure::<dyn FnMut()>::new(|| {
                    if let Some(c) =
                        document().and_then(|d| d.get_element_by_id(crate::web::CANVAS_ID)).and_then(|c| c.dyn_into::<web_sys::HtmlElement>().ok())
                    {
                        let _ = c.focus();
                    }
                });
                for ev in ["focus", "pointerup"] {
                    let _ = e.add_event_listener_with_callback(ev, give_back.as_ref().unchecked_ref());
                }
                give_back.forget();
                if let Some(b) = doc.body() {
                    let _ = b.append_child(&e);
                }
                e
            }
        };
        if el.get_attribute("src").as_deref() != Some(src.as_str()) {
            let _ = el.set_attribute("src", &src);
        }
        let Some(html) = el.dyn_ref::<web_sys::HtmlElement>() else { return };
        // egui points to CSS pixels.
        let k = ppp as f64 / win.device_pixel_ratio().max(0.01);
        let origin =
            doc.get_element_by_id(crate::web::CANVAS_ID).map(|c| c.get_bounding_client_rect()).map(|r| (r.left(), r.top())).unwrap_or((0.0, 0.0));
        let st = html.style();
        for (k2, v) in [
            ("position", "fixed".to_string()),
            ("z-index", "5".to_string()),
            ("background", "#000".to_string()),
            ("left", format!("{}px", origin.0 + area.min.x as f64 * k)),
            ("top", format!("{}px", origin.1 + area.min.y as f64 * k)),
            ("width", format!("{}px", area.width() as f64 * k)),
            ("height", format!("{}px", area.height() as f64 * k)),
        ] {
            let _ = st.set_property(k2, &v);
        }
    }

    /// Removes the element (it stops playing with it).
    pub fn hide() {
        if let Some(e) = document().and_then(|d| d.get_element_by_id(VIDEO_ID)) {
            e.remove();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    use super::Rect;
    pub fn show(_id: &str, _area: Rect, _ppp: f32) {}
    pub fn hide() {}
}

pub(super) use imp::{hide, show};
