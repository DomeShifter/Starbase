//! Optional detailed map tiles for Photos → Places (the built-in map is offline Natural Earth).
//!
//! The person who runs the server may set `STARBASE_MAP_TILES` to a raster tile address with
//! `{z}`, `{x}` and `{y}` (e.g. `https://tiles.example.org/{z}/{x}/{y}.png`) and
//! `STARBASE_MAP_ATTRIBUTION` to the credit line the tile host asks for. Tiles are fetched by the
//! server and kept in a small disk cache, so viewers' browsers never contact the tile host.

use std::path::PathBuf;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Response, header};
use serde_json::{Value, json};

use crate::error;

/// Biggest tile accepted from the tile host.
const MAX_TILE: u64 = 1024 * 1024;
/// Deepest zoom asked for.
pub const MAX_Z: u32 = 19;

/// The configured tile address, when it is a plain https (or http) address with all three holes.
pub fn source() -> Option<String> {
    let s = std::env::var("STARBASE_MAP_TILES").ok()?;
    let s = s.trim().to_string();
    let ok = (s.starts_with("https://") || s.starts_with("http://")) && ["{z}", "{x}", "{y}"].iter().all(|h| s.contains(h));
    if !ok {
        eprintln!("STARBASE_MAP_TILES must be an http(s) address with {{z}}, {{x}} and {{y}}: ignored");
    }
    ok.then_some(s)
}

/// What the app learns about the tiles: `null` when there are none.
pub fn info() -> Value {
    match source() {
        Some(_) => json!({"max_z": MAX_Z, "attribution": std::env::var("STARBASE_MAP_ATTRIBUTION").unwrap_or_default()}),
        None => Value::Null,
    }
}

/// `z/x/y(.png)` checked: a real tile of the world, not a path.
pub fn parse(z: &str, x: &str, y: &str) -> Option<(u32, u32, u32)> {
    let y = y.strip_suffix(".png").unwrap_or(y);
    let all_digits = |s: &str| !s.is_empty() && s.len() <= 7 && s.bytes().all(|b| b.is_ascii_digit());
    if !(all_digits(z) && all_digits(x) && all_digits(y)) {
        return None;
    }
    let (z, x, y) = (z.parse().ok()?, x.parse().ok()?, y.parse().ok()?);
    (z <= MAX_Z && x < (1u32 << z) && y < (1u32 << z)).then_some((z, x, y))
}

fn cache_dir() -> PathBuf {
    std::env::var_os("STARBASE_MAP_TILES_CACHE").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("starbase-map-tiles"))
}

fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(2)
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(concat!("Starbase/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut r = agent.get(url).call().map_err(|e| e.to_string())?;
    r.body_mut().with_config().limit(MAX_TILE).read_to_vec().map_err(|e| e.to_string())
}

/// `GET /api/photos/tiles/{z}/{x}/{y}.png` for signed-in people.
pub async fn tile(z: &str, x: &str, y: &str) -> Response<Body> {
    let Some(src) = source() else { return error(404, "no map tiles on this server") };
    let Some((z, x, y)) = parse(z, x, y) else { return error(400, "not a map tile") };
    let got = tokio::task::spawn_blocking(move || {
        let path = cache_dir().join(format!("{:016x}", hash(&src))).join(format!("{z}/{x}/{y}"));
        if let Ok(b) = std::fs::read(&path) {
            return Ok(b);
        }
        let url = src.replace("{z}", &z.to_string()).replace("{x}", &x.to_string()).replace("{y}", &y.to_string());
        let b = fetch(&url)?;
        if let Some(d) = path.parent()
            && std::fs::create_dir_all(d).is_ok()
        {
            let _ = std::fs::write(&path, &b);
        }
        Ok::<_, String>(b)
    })
    .await
    .unwrap_or_else(|e| Err(e.to_string()));
    match got {
        Ok(b) => {
            let png = b.starts_with(b"\x89PNG");
            let jpg = b.starts_with(&[0xFF, 0xD8]);
            let webp = b.len() > 12 && &b[8..12] == b"WEBP";
            let kind = if png {
                "image/png"
            } else if jpg {
                "image/jpeg"
            } else if webp {
                "image/webp"
            } else {
                return error(502, "the tile host sent something that isn't a picture");
            };
            Response::builder()
                .status(200)
                .header(header::CONTENT_TYPE, kind)
                .header(header::CACHE_CONTROL, "private, max-age=86400")
                .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
                .body(Body::from(b))
                .unwrap()
        }
        Err(e) => {
            eprintln!("map tile {z}/{x}/{y}: {e}");
            error(502, "the map tile host didn't answer")
        }
    }
}

/// A stable short name for the tile address (a cache folder per source).
pub fn hash(s: &str) -> u64 {
    // FNV-1a: stable across runs and builds, unlike the std hasher.
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_addresses_are_checked() {
        assert_eq!(parse("0", "0", "0.png"), Some((0, 0, 0)));
        assert_eq!(parse("3", "7", "7"), Some((3, 7, 7)));
        for bad in [("3", "8", "0"), ("20", "0", "0"), ("1", "..", "0"), ("1", "-1", "0"), ("", "0", "0"), ("1", "0", "0.jpg")] {
            assert_eq!(parse(bad.0, bad.1, bad.2), None, "{bad:?}");
        }
    }
}
