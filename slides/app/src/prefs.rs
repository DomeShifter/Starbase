//! Small settings kept in the browser between visits (`localStorage`): the theme choice. Native
//! builds (the window tests) keep nothing.

#[cfg(target_arch = "wasm32")]
fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

#[cfg(target_arch = "wasm32")]
pub fn get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok().flatten()
}

#[cfg(target_arch = "wasm32")]
pub fn set(key: &str, value: &str) {
    if let Some(s) = storage() {
        let _ = s.set_item(key, value);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)] // the same interface as the browser's; the window tests never read it back
pub fn get(_key: &str) -> Option<String> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
pub fn set(_key: &str, _value: &str) {}
