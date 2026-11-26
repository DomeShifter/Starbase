//! Link hover previews (To-do 3 row 19): the page title, site name, small icon and a small picture
//! for a web link, shown on Sheets' link card. **Off by default**: the owner turns it on in People
//! (auth::link_preview). The browser never contacts the other site: it asks
//! `GET /api/link-preview?url=…` and `GET /api/link-preview/image?url=…&kind=icon|picture`, and
//! this server does the fetching, under these limits:
//!
//! - only http and https; never a private, loopback, link-local, carrier-NAT, multicast or other
//!   non-public address (also IPv4 inside IPv6). Names are looked up by [`GuardResolver`], which
//!   refuses the whole name if any address is private, and ureq connects to exactly the addresses
//!   it returned (no DNS rebinding between the check and the connect);
//! - redirects followed by hand, at most 3, each hop checked again;
//! - 2 s to connect, 5 s in total for the page and its pictures;
//! - at most 256 KB of the page, 300 KB per picture (png, jpeg, webp, gif, ico; never SVG),
//!   re-encoded here as a small PNG (picture ≤ 320 px, icon ≤ 32 px);
//! - no cookies, no referrer, a fixed user-agent;
//! - 4 fetches at once (more answer "busy"); a per-person rate limit (guard::LINK_PREVIEW_USER);
//! - title and site name are plain text, cut to 120 characters;
//! - a disk cache keyed by the address: 7 days for a preview, 1 hour for a failure, 200 MB at
//!   most (oldest removed first). A stale preview is still served when a fresh fetch fails.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

pub const MAX_PAGE: u64 = 256 * 1024;
pub const MAX_PICTURE: u64 = 300 * 1024;
pub const MAX_HOPS: usize = 3;
pub const TOTAL: Duration = Duration::from_secs(5);
pub const CONNECT: Duration = Duration::from_secs(2);
pub const MAX_TEXT: usize = 120;
pub const PICTURE_PX: u32 = 320;
pub const ICON_PX: u32 = 32;
const KEEP_OK: Duration = Duration::from_secs(7 * 86_400);
const KEEP_FAILED: Duration = Duration::from_secs(3600);
const CACHE_MAX: u64 = 200 * 1024 * 1024;
const MAX_AT_ONCE: usize = 4;
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------- the address guard

/// Whether `ip` is an ordinary public internet address (the only kind fetched).
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v6.segments();
            // IPv4-compatible (::a.b.c.d), NAT64 (64:ff9b::/96) and 6to4 (2002::/16) carry an IPv4 inside.
            if s[..6] == [0; 6] || (s[0] == 0x64 && s[1] == 0xff9b) {
                let v4 = Ipv4Addr::new((s[6] >> 8) as u8, s[6] as u8, (s[7] >> 8) as u8, s[7] as u8);
                return !v6.is_unspecified() && !v6.is_loopback() && public_v4(v4);
            }
            if s[0] == 0x2002 {
                return public_v4(Ipv4Addr::new((s[1] >> 8) as u8, s[1] as u8, (s[2] >> 8) as u8, s[2] as u8));
            }
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local
                || (s[0] & 0xffc0) == 0xfe80 // link-local
                || (s[0] & 0xffc0) == 0xfec0 // old site-local
                || (s[0] == 0x2001 && s[1] == 0x0db8)) // documentation
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_documentation()
        || o[0] == 0 // "this network"
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // carrier-grade NAT 100.64/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // IETF protocol assignments
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // benchmarking 198.18/15
        || o[0] >= 240) // reserved
}

/// ureq's own name lookup, refusing the whole name when any address it gives is not public
/// (ureq then connects only to the addresses returned here).
#[derive(Debug, Default)]
pub struct GuardResolver {
    inner: DefaultResolver,
    /// Tests only: lets a test page on 127.0.0.1 through.
    allow_private: bool,
}

impl Resolver for GuardResolver {
    fn resolve(&self, uri: &Uri, config: &Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let got = self.inner.resolve(uri, config, timeout)?;
        if !self.allow_private && got.iter().any(|a: &SocketAddr| !is_public(a.ip())) {
            return Err(ureq::Error::Other("that address is on a private network".into()));
        }
        Ok(got)
    }
}

/// The http(s) address, checked: scheme, a host, and (for a written-out IP) a public one.
pub fn check_url(url: &str, allow_private: bool) -> Result<Uri, String> {
    let uri: Uri = url.trim().parse().map_err(|_| "not an address".to_string())?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) {
        return Err("only web addresses are previewed".into());
    }
    let host = uri.host().filter(|h| !h.is_empty()).ok_or("no site in the address")?;
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>()
        && !allow_private
        && !is_public(ip)
    {
        return Err("that address is on a private network".into());
    }
    if !allow_private && (bare.eq_ignore_ascii_case("localhost") || bare.to_ascii_lowercase().ends_with(".localhost")) {
        return Err("that address is on a private network".into());
    }
    Ok(uri)
}

// ---------------------------------------------------------------- fetching

struct Fetcher {
    deadline: Instant,
    allow_private: bool,
}

impl Fetcher {
    fn new(allow_private: bool) -> Self {
        Fetcher { deadline: Instant::now() + TOTAL, allow_private }
    }

    fn agent(&self) -> Result<ureq::Agent, String> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("too slow".into());
        }
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(left))
            .timeout_connect(Some(CONNECT.min(left)))
            .user_agent(concat!("Starbase/", env!("CARGO_PKG_VERSION")))
            .build();
        Ok(ureq::Agent::with_parts(
            config,
            DefaultConnector::default(),
            GuardResolver { inner: DefaultResolver::default(), allow_private: self.allow_private },
        ))
    }

    /// GETs `url`, following up to MAX_HOPS redirects by hand (each checked), reading at most `max`
    /// bytes. Returns the final address, the content type and the bytes.
    fn get(&self, url: &str, max: u64) -> Result<(String, String, Vec<u8>), String> {
        let mut url = url.to_string();
        for _ in 0..=MAX_HOPS {
            check_url(&url, self.allow_private)?;
            let mut r = self.agent()?.get(&url).call().map_err(|e| e.to_string())?;
            let status = r.status().as_u16();
            if (300..400).contains(&status) {
                let to = r.headers().get("location").and_then(|v| v.to_str().ok()).ok_or("a redirect with nowhere to go")?;
                url = join(&url, to).ok_or("a redirect to a bad address")?;
                continue;
            }
            if status >= 400 {
                return Err(format!("the site said {status}"));
            }
            let kind = r.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
            // Only the first `max` bytes are read: a page is cut (its title sits at the top); a
            // picture that doesn't fit whole is refused.
            use std::io::Read;
            let mut bytes = Vec::new();
            r.body_mut().as_reader().take(max + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if bytes.len() as u64 > max {
                if max != MAX_PAGE {
                    return Err("too big".into());
                }
                bytes.truncate(max as usize);
            }
            return Ok((url, kind, bytes));
        }
        Err("too many redirects".into())
    }
}

/// `to` (an href or a Location) made absolute against `base`. Only http(s) results.
pub fn join(base: &str, to: &str) -> Option<String> {
    let to = to.trim();
    if to.is_empty() || to.chars().any(char::is_control) {
        return None;
    }
    let lower = to.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(to.to_string());
    }
    if to.contains(':') && !to.starts_with('/') && to.find(':') < to.find('/').or(Some(to.len())) {
        return None; // another scheme (data:, javascript:, …)
    }
    let (scheme, rest) = base.split_once("://")?;
    if let Some(r) = to.strip_prefix("//") {
        return Some(format!("{scheme}://{r}"));
    }
    let host_end = rest.find('/').unwrap_or(rest.len());
    let origin = format!("{scheme}://{}", &rest[..host_end]);
    if to.starts_with('/') {
        return Some(format!("{origin}{to}"));
    }
    let path = &rest[host_end..];
    let path = path.split(['?', '#']).next().unwrap_or("");
    let dir = path.rfind('/').map_or("/", |i| &path[..=i]);
    Some(format!("{origin}{dir}{to}"))
}

// ---------------------------------------------------------------- reading the page

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub site: String,
    pub image: Option<String>,
    pub icon: Option<String>,
}

/// Title, site name, picture and icon from the top of an HTML page (og: tags first).
pub fn read_meta(html: &str, base: &str) -> Meta {
    let mut og_title = None;
    let mut site = None;
    let mut image = None;
    let mut icon = None;
    let mut title = None;
    let lower = html.to_ascii_lowercase();
    let mut at = 0;
    while let Some(i) = lower[at..].find('<') {
        let start = at + i;
        let Some(len) = lower[start..].find('>') else { break };
        let tag = &html[start..start + len + 1];
        let tag_l = &lower[start..start + len + 1];
        at = start + len + 1;
        if tag_l.starts_with("<meta") {
            let key = attr(tag, "property").or_else(|| attr(tag, "name")).unwrap_or_default().to_ascii_lowercase();
            let content = attr(tag, "content");
            match key.as_str() {
                "og:title" | "twitter:title" if og_title.is_none() => og_title = content,
                "og:site_name" if site.is_none() => site = content,
                "og:image" | "og:image:url" | "og:image:secure_url" | "twitter:image" if image.is_none() => image = content,
                _ => {}
            }
        } else if tag_l.starts_with("<link") {
            let rel = attr(tag, "rel").unwrap_or_default().to_ascii_lowercase();
            if icon.is_none() && rel.split_whitespace().any(|r| r == "icon" || r == "apple-touch-icon") {
                let href = attr(tag, "href");
                if href.as_deref().is_some_and(|h| !h.to_ascii_lowercase().ends_with(".svg")) {
                    icon = href;
                }
            }
        } else if tag_l.starts_with("<title") && title.is_none() {
            let end = lower[at..].find("</title").map_or(lower.len(), |e| at + e);
            title = Some(html[at..end].to_string());
        } else if tag_l.starts_with("</head") || tag_l.starts_with("<body") {
            break;
        }
    }
    let clean = |s: String| short(&unescape(&s));
    Meta {
        title: og_title.or(title).map(clean).unwrap_or_default(),
        site: site.map(clean).unwrap_or_default(),
        image: image.and_then(|i| join(base, &unescape(&i))),
        icon: icon.and_then(|i| join(base, &unescape(&i))).or_else(|| join(base, "/favicon.ico")),
    }
}

/// An attribute's value in one tag (quoted or not).
fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        let before_ok = lower[..at].ends_with(|c: char| c.is_whitespace());
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let vstart = from + (lower[from..].len() - rest.len()) + 1;
        let v = tag[vstart..].trim_start();
        return Some(match v.chars().next() {
            Some(q @ ('"' | '\'')) => v[1..].split(q).next().unwrap_or("").to_string(),
            _ => v.split(|c: char| c.is_whitespace() || c == '>').next().unwrap_or("").trim_end_matches('/').to_string(),
        });
    }
    None
}

/// HTML character references to text (the common named ones and numeric ones).
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let ch = end.and_then(|e| {
            let name = &rest[1..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            c.map(|c| (c, e))
        });
        match ch {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Plain one-line text, at most MAX_TEXT characters.
fn short(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let one: String = one.chars().filter(|c| !c.is_control()).collect();
    if one.chars().count() > MAX_TEXT { one.chars().take(MAX_TEXT - 1).collect::<String>() + "…" } else { one }
}

/// A picture's bytes made into a small PNG (never SVG or anything unknown).
pub fn small_png(bytes: &[u8], px: u32) -> Option<Vec<u8>> {
    let png = bytes.starts_with(b"\x89PNG");
    let jpg = bytes.starts_with(&[0xFF, 0xD8]);
    let webp = bytes.len() > 12 && &bytes[8..12] == b"WEBP";
    let gif = bytes.starts_with(b"GIF8");
    let ico = bytes.starts_with(&[0, 0, 1, 0]);
    let format = if png {
        image::ImageFormat::Png
    } else if jpg {
        image::ImageFormat::Jpeg
    } else if webp {
        image::ImageFormat::WebP
    } else if gif {
        image::ImageFormat::Gif
    } else if ico {
        image::ImageFormat::Ico
    } else {
        return None;
    };
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8000);
    limits.max_image_height = Some(8000);
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let mut img = reader.decode().ok()?;
    if img.width().max(img.height()) > px {
        img = img.thumbnail(px, px);
    }
    let mut out = Vec::new();
    img.into_rgba8().write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

// ---------------------------------------------------------------- the preview + cache

/// What the card gets: `{"state": "ok"|"none"|"off"|"busy", "title", "site", "icon", "picture"}`.
/// The preview JSON, then the site icon and the picture bytes when fetched.
pub type Fetched = (Value, Option<Vec<u8>>, Option<Vec<u8>>);

pub fn fetch_preview(url: &str, allow_private: bool) -> Result<Fetched, String> {
    let f = Fetcher::new(allow_private);
    let (final_url, kind, bytes) = f.get(url, MAX_PAGE)?;
    if !(kind.is_empty() || kind.contains("html")) {
        return Err("not a web page".into());
    }
    let html = String::from_utf8_lossy(&bytes);
    let meta = read_meta(&html, &final_url);
    let picture = meta.image.as_deref().and_then(|u| f.get(u, MAX_PICTURE).ok()).and_then(|(_, _, b)| small_png(&b, PICTURE_PX));
    let icon = meta.icon.as_deref().and_then(|u| f.get(u, MAX_PICTURE).ok()).and_then(|(_, _, b)| small_png(&b, ICON_PX));
    let v = json!({
        "state": "ok",
        "title": meta.title,
        "site": meta.site,
        "icon": icon.is_some(),
        "picture": picture.is_some(),
    });
    Ok((v, icon, picture))
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("STARBASE_LINK_PREVIEW_CACHE").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("starbase-link-previews"))
}

fn entry(url: &str) -> PathBuf {
    cache_dir().join(format!("{:016x}", crate::photos::tiles::hash(url.trim())))
}

fn age(p: &Path) -> Option<Duration> {
    std::fs::metadata(p).ok()?.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok())
}

/// The preview for `url` (cached, or fetched now). Blocking: call from spawn_blocking.
pub fn preview(url: &str) -> Value {
    let none = json!({"state": "none"});
    if check_url(url, false).is_err() {
        return none;
    }
    let dir = entry(url);
    let meta = dir.join("meta.json");
    let cached: Option<Value> = std::fs::read(&meta).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let fresh = |v: &Value| {
        let keep = if v["state"] == "ok" { KEEP_OK } else { KEEP_FAILED };
        age(&meta).is_some_and(|a| a < keep)
    };
    if let Some(v) = cached.as_ref().filter(|v| fresh(v)) {
        return v.clone();
    }
    if IN_FLIGHT.fetch_add(1, Ordering::SeqCst) >= MAX_AT_ONCE {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        return cached.filter(|v| v["state"] == "ok").unwrap_or_else(|| json!({"state": "busy"}));
    }
    let got = fetch_preview(url, false);
    IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    let _ = std::fs::create_dir_all(&dir);
    match got {
        Ok((v, icon, picture)) => {
            for (name, b) in [("icon.png", icon), ("picture.png", picture)] {
                match b {
                    Some(b) => _ = std::fs::write(dir.join(name), b),
                    None => _ = std::fs::remove_file(dir.join(name)),
                }
            }
            let _ = std::fs::write(&meta, v.to_string());
            prune();
            v
        }
        Err(e) => {
            eprintln!("link preview: {e}");
            // Site down: an older good preview is still better than nothing.
            if let Some(v) = cached.filter(|v| v["state"] == "ok") {
                return v;
            }
            let _ = std::fs::write(&meta, none.to_string());
            none
        }
    }
}

/// The cached icon or picture for `url` (after preview() made it).
pub fn image(url: &str, kind: &str) -> Option<Vec<u8>> {
    let name = match kind {
        "icon" => "icon.png",
        "picture" => "picture.png",
        _ => return None,
    };
    std::fs::read(entry(url).join(name)).ok()
}

/// Keeps the cache under CACHE_MAX, removing the oldest previews first.
fn prune() {
    let Ok(rd) = std::fs::read_dir(cache_dir()) else { return };
    let mut all: Vec<(SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let size: u64 = std::fs::read_dir(&p).ok()?.flatten().filter_map(|f| f.metadata().ok()).map(|m| m.len()).sum();
            let when = std::fs::metadata(p.join("meta.json")).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            Some((when, size, p))
        })
        .collect();
    let mut total: u64 = all.iter().map(|a| a.1).sum();
    all.sort();
    for (_, size, p) in all {
        if total <= CACHE_MAX {
            break;
        }
        let _ = std::fs::remove_dir_all(p);
        total = total.saturating_sub(size);
    }
}

#[cfg(test)]
#[path = "linkpreview_tests.rs"]
mod tests;
