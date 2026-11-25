//! Small settings kept between visits: the theme choice. In the browser in `localStorage`; in
//! the desktop program in `$XDG_CONFIG_HOME/starbase/slides.json` (a flat JSON object). The
//! window tests keep nothing.

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

/// The desktop program's settings file.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty()).map(std::path::PathBuf::from)?;
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()).map_or_else(|| home.join(".config"), std::path::PathBuf::from);
    Some(config.join("starbase").join("slides.json"))
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
fn all() -> serde_json::Map<String, serde_json::Value> {
    path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
pub fn get(key: &str) -> Option<String> {
    all().get(key).and_then(|v| v.as_str()).map(str::to_string)
}

#[cfg(all(not(target_arch = "wasm32"), not(test)))]
pub fn set(key: &str, value: &str) {
    let Some(path) = path() else { return };
    let mut map = all();
    map.insert(key.to_string(), serde_json::Value::String(value.to_string()));
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| serde_json::to_vec_pretty(&map).map_err(std::io::Error::other))
        .and_then(|json| crate::localfile::write_atomic(&path, &json));
    if let Err(e) = written {
        log::warn!("settings: {}: {e}", path.display());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[allow(dead_code, reason = "the same interface; the window tests keep nothing")]
pub fn get(_key: &str) -> Option<String> {
    None
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub fn set(_key: &str, _value: &str) {}
