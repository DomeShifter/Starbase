//! Protection around every request (docs/deploy.md, "Security settings"):
//!
//! - Cross-site requests: a request that changes something (anything but GET/HEAD) and the live
//!   editing WebSocket must come from a page of this server. Browsers send an `Origin` header with
//!   those requests; when there is one, it must name the host the request was sent to (or
//!   STARBASE_PUBLIC_URL). No custom header or content type is needed, so StarDocs' plain
//!   same-origin fetches keep working.
//! - Host names (against DNS rebinding): with STARBASE_PUBLIC_URL or STARBASE_ALLOWED_HOSTS
//!   set, the server answers only to those names, IP addresses and localhost. Without them it
//!   answers to any name, so a home server reached by its IP or a proxy name works as before.
//! - How many requests run at once, and the browser safety headers on every answer.
//! - A token-bucket limit on sign-in, sign-up and setup tries (per username and per address).
//!
//! The address of whoever is asking is the connection's own, unless that is a proxy listed in
//! STARBASE_TRUSTED_PROXY: then X-Forwarded-For (and X-Forwarded-Proto/-Host) are believed.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderMap, HeaderValue, Method, Response, header};
use axum::middleware::Next;

/// Settings read once from the environment.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    /// Proxies (addresses or `address/bits` ranges) whose X-Forwarded-* headers are believed.
    pub trusted: Vec<(IpAddr, u8)>,
    /// STARBASE_PUBLIC_URL's host (and port), lower case, e.g. `sheets.example.com`.
    pub public: Option<String>,
    /// STARBASE_ALLOWED_HOSTS: more host names the server answers to.
    pub hosts: Vec<String>,
    /// STARBASE_SECURE_COOKIE: always mark the sign-in cookie Secure (HTTPS only).
    pub secure_cookie: bool,
    /// STARBASE_AUTH=off: no sign-in, so no device tokens either.
    pub no_auth: bool,
}

fn yes(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "on" | "yes")
}

fn list(v: &str) -> impl Iterator<Item = &str> {
    v.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty())
}

fn parse_range(s: &str) -> Option<(IpAddr, u8)> {
    let (ip, bits) = s.split_once('/').map_or((s, None), |(a, b)| (a, Some(b)));
    let ip: IpAddr = ip.parse().ok()?;
    let ip = ip.to_canonical();
    let max = if ip.is_ipv4() { 32 } else { 128 };
    let bits = match bits {
        Some(b) => b.parse::<u8>().ok().filter(|b| *b <= max)?,
        None => max,
    };
    Some((ip, bits))
}

impl Config {
    /// From settings looked up by `get` (the environment, or a test's own).
    pub fn from(get: impl Fn(&str) -> Option<String>) -> Config {
        let trusted = get("STARBASE_TRUSTED_PROXY").map(|v| list(&v).filter_map(parse_range).collect()).unwrap_or_default();
        let public = get("STARBASE_PUBLIC_URL").map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).map(|v| {
            let rest = v.split_once("://").map_or(v.as_str(), |(_, r)| r);
            rest.split('/').next().unwrap_or("").to_lowercase()
        });
        let hosts = get("STARBASE_ALLOWED_HOSTS").map(|v| list(&v).map(|h| host_name(h).to_string()).collect()).unwrap_or_default();
        let secure_cookie = get("STARBASE_SECURE_COOKIE").is_some_and(|v| yes(&v));
        let no_auth = get("STARBASE_AUTH").is_some_and(|v| matches!(v.trim().to_lowercase().as_str(), "off" | "false" | "0" | "no"));
        Config { trusted, public: public.filter(|p| !p.is_empty()), hosts, secure_cookie, no_auth }
    }

    fn trusts(&self, ip: IpAddr) -> bool {
        let ip = ip.to_canonical();
        self.trusted.iter().any(|(net, bits)| in_range(ip, *net, *bits))
    }

    /// Whether the host-name allow-list is on.
    fn checks_hosts(&self) -> bool {
        self.public.is_some() || !self.hosts.is_empty()
    }
}

fn in_range(ip: IpAddr, net: IpAddr, bits: u8) -> bool {
    match (ip, net) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits as u32) };
            u32::from(a) & mask == u32::from(b) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits as u32) };
            u128::from(a) & mask == u128::from(b) & mask
        }
        _ => false,
    }
}

/// The settings from the environment (read once).
pub fn config() -> &'static Config {
    static CONFIG: OnceLock<Config> = OnceLock::new();
    CONFIG.get_or_init(|| Config::from(crate::setting))
}

/// The address the connection came from (axum's ConnectInfo), if known.
pub fn peer(headers_ext: &axum::http::Extensions) -> Option<IpAddr> {
    headers_ext.get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip().to_canonical())
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Who is asking: the connection's address, or (from a trusted proxy) the last address in
/// X-Forwarded-For that isn't one of the trusted proxies.
pub fn client_ip(peer: Option<IpAddr>, headers: &HeaderMap, cfg: &Config) -> Option<IpAddr> {
    let peer = peer?;
    if !cfg.trusts(peer) {
        return Some(peer);
    }
    let forwarded: Vec<IpAddr> = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .filter_map(|s| s.trim().parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical())
        .collect();
    Some(forwarded.into_iter().rev().find(|ip| !cfg.trusts(*ip)).unwrap_or(peer))
}

/// Whether the browser reached us over HTTPS (said by a trusted proxy), or the cookie should be
/// Secure anyway (STARBASE_SECURE_COOKIE).
pub fn secure(peer: Option<IpAddr>, headers: &HeaderMap, cfg: &Config) -> bool {
    cfg.secure_cookie
        || (peer.is_some_and(|p| cfg.trusts(p))
            && header_str(headers, "x-forwarded-proto").is_some_and(|v| v.split(',').next().unwrap_or("").trim().eq_ignore_ascii_case("https")))
}

/// A host name without its port (and without the brackets of an IPv6 address), lower case.
fn host_name(authority: &str) -> String {
    let a = authority.trim().to_lowercase();
    if let Some(rest) = a.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    match a.rsplit_once(':') {
        Some((h, port)) if !h.contains(':') && port.chars().all(|c| c.is_ascii_digit()) => h.to_string(),
        _ => a,
    }
}

/// The host (and port) the request was sent to.
fn request_host(headers: &HeaderMap, uri: &axum::http::Uri) -> Option<String> {
    header_str(headers, "host").map(str::to_string).or_else(|| uri.authority().map(|a| a.as_str().to_string())).map(|h| h.trim().to_lowercase())
}

/// Checks one request; `Err((status, message))` refuses it.
pub fn check(method: &Method, uri: &axum::http::Uri, headers: &HeaderMap, peer: Option<IpAddr>, cfg: &Config) -> Result<(), (u16, &'static str)> {
    let host = request_host(headers, uri);
    if cfg.checks_hosts() && uri.path() != "/healthz" {
        let name = host.as_deref().map(host_name).unwrap_or_default();
        let known = name == "localhost"
            || name.parse::<IpAddr>().is_ok()
            || cfg.public.as_deref().map(host_name).is_some_and(|p| p == name)
            || cfg.hosts.contains(&name);
        if !known {
            return Err((421, "This server doesn't answer to that name. Its owner can add it to STARBASE_ALLOWED_HOSTS."));
        }
    }
    let changes = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    // A desktop app's request: signed in only by the device token it sends itself, with no
    // cookies, so another site can't have made it in someone's name (there is nothing ambient
    // to borrow). Only a Starbase device token counts, and only with sign-in on: a browser
    // resends other Authorization headers by itself (a proxy's Basic password), so those are
    // ambient. With cookies too, the usual check applies (and the cookies aren't read:
    // main.rs current_user).
    let token_only = !cfg.no_auth && device_token(headers).is_some() && !headers.contains_key(header::COOKIE);
    if (changes || uri.path() == "/api/live") && !token_only {
        let Some(origin) = header_str(headers, "origin") else { return Ok(()) };
        let from = origin.split_once("://").map(|(_, rest)| rest.trim_end_matches('/').to_lowercase());
        let forwarded_host = peer
            .filter(|p| cfg.trusts(*p))
            .and_then(|_| header_str(headers, "x-forwarded-host"))
            .map(|h| h.split(',').next().unwrap_or("").trim().to_lowercase());
        let ok = from.is_some_and(|from| {
            host.as_deref() == Some(from.as_str()) || forwarded_host.as_deref() == Some(from.as_str()) || cfg.public.as_deref() == Some(from.as_str())
        });
        if !ok {
            return Err((403, "That request came from another web site, so it was refused."));
        }
    }
    Ok(())
}

/// A Starbase device token sent as `Authorization: Bearer sbd_…` (the desktop apps, devices.rs).
pub fn device_token(headers: &HeaderMap) -> Option<String> {
    bearer(headers).filter(|t| t.starts_with(crate::devices::PREFIX))
}

/// Whatever is sent as `Authorization: Bearer …`.
pub fn bearer(headers: &HeaderMap) -> Option<String> {
    let (scheme, token) = header_str(headers, "authorization")?.trim().split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| token.to_string())
}

// ---------------------------------------------------------------- the middleware

/// Requests handled at once; more wait (up to QUEUE_WAIT) for a turn.
const MAX_AT_ONCE: usize = 128;
const QUEUE_WAIT: Duration = Duration::from_secs(30);
static AT_ONCE: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(MAX_AT_ONCE));

/// Headers that keep browsers safe with every answer: no guessing types, never shown inside
/// another site's frame, no addresses leaked to other sites. (A fuller Content-Security-Policy
/// isn't set: index.html has inline scripts, the app needs WebAssembly, and printing shows a PDF
/// from a blob: URL in a frame, which `object-src 'none'` could block.)
pub fn safety_headers(h: &mut HeaderMap) {
    let set = |h: &mut HeaderMap, name: header::HeaderName, v: &'static str| {
        h.entry(name).or_insert(HeaderValue::from_static(v));
    };
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::X_FRAME_OPTIONS, "DENY");
    set(h, header::REFERRER_POLICY, "same-origin");
    set(h, header::CONTENT_SECURITY_POLICY, "frame-ancestors 'none'; base-uri 'self'");
}

fn refuse(code: u16, msg: &str) -> Response<Body> {
    crate::error(code, msg)
}

/// Runs around every request (main.rs: router()).
pub async fn middleware(mut req: Request, next: Next) -> Response<Body> {
    let cfg = config();
    let peer = peer(req.extensions());
    if secure(peer, req.headers(), cfg) {
        drop_plain_cookies(req.headers_mut());
    }
    let mut resp = match check(req.method(), req.uri(), req.headers(), peer, cfg) {
        Err((code, msg)) => refuse(code, msg),
        Ok(()) => match tokio::time::timeout(QUEUE_WAIT, AT_ONCE.acquire()).await {
            Ok(Ok(_turn)) => next.run(req).await,
            _ => refuse(503, "The server is busy. Try again in a moment."),
        },
    };
    safety_headers(resp.headers_mut());
    resp
}

// ---------------------------------------------------------------- how often people may try

/// A token bucket: up to `burst` tries at once, then one more every `every`.
#[derive(Clone, Copy, Debug)]
pub struct Bucket {
    pub burst: f64,
    pub every: Duration,
}

/// Sign-in tries from one address (every try counts).
pub const LOGIN_IP: Bucket = Bucket { burst: 20.0, every: Duration::from_secs(6) };
/// Wrong passwords for one username.
pub const LOGIN_USER: Bucket = Bucket { burst: 10.0, every: Duration::from_secs(30) };
/// New accounts (sign-up) from one address.
pub const SIGNUP_IP: Bucket = Bucket { burst: 5.0, every: Duration::from_secs(120) };
/// Setup-code tries from one address.
pub const SETUP_IP: Bucket = Bucket { burst: 10.0, every: Duration::from_secs(10) };
/// Single sign-ons started from one address: at most 50 in ten minutes (10 + 600 s / 15 s), no
/// more than oidc.rs keeps waiting for one address.
pub const OIDC_IP: Bucket = Bucket { burst: 10.0, every: Duration::from_secs(15) };

/// Desktop sign-ins started from one address.
pub const DEVICE_IP: Bucket = Bucket { burst: 10.0, every: Duration::from_secs(30) };
/// A desktop app asking whether it was allowed yet (every 5 seconds), per address.
pub const DEVICE_POLL_IP: Bucket = Bucket { burst: 30.0, every: Duration::from_secs(2) };
/// Wrong device codes typed by one account.
/// Link previews a person may ask the server to fetch: 30 a minute (linkpreview.rs).
pub const LINK_PREVIEW_USER: Bucket = Bucket { burst: 30.0, every: Duration::from_secs(2) };
pub const DEVICE_CODE_USER: Bucket = Bucket { burst: 10.0, every: Duration::from_secs(30) };

#[derive(Default)]
pub struct Limiter {
    buckets: Mutex<HashMap<String, (f64, Instant)>>,
}

impl Limiter {
    fn level(map: &HashMap<String, (f64, Instant)>, key: &str, b: Bucket, now: Instant) -> f64 {
        let (tokens, at) = map.get(key).copied().unwrap_or((b.burst, now));
        (tokens + now.saturating_duration_since(at).as_secs_f64() / b.every.as_secs_f64()).min(b.burst)
    }

    /// Whether a try would be allowed now (takes nothing).
    pub fn can(&self, key: &str, b: Bucket) -> bool {
        let map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        Self::level(&map, key, b, Instant::now()) >= 1.0
    }

    /// Takes a try; false (and nothing taken) when there are none left.
    pub fn take(&self, key: &str, b: Bucket) -> bool {
        let now = Instant::now();
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            // Forget buckets nobody has used for an hour (every bucket here is full again by then).
            map.retain(|_, (_, at)| now.saturating_duration_since(*at) < Duration::from_secs(3600));
        }
        let level = Self::level(&map, key, b, now);
        if level < 1.0 {
            map.insert(key.to_string(), (level, now));
            return false;
        }
        map.insert(key.to_string(), (level - 1.0, now));
        true
    }
}

pub static LIMITS: LazyLock<Limiter> = LazyLock::new(Limiter::default);

/// Who an address belongs to, for the try limits: an IPv4 address itself, but for IPv6 its /64
/// network (one home or one server is usually given a whole /64, so counting each address on
/// its own would let one machine pretend to be millions of people).
pub fn ip_prefix(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(a) => a.to_string(),
        IpAddr::V6(a) => {
            let net = std::net::Ipv6Addr::from(u128::from(a) & (u128::MAX << 64));
            format!("{net}/64")
        }
    }
}

/// The network an address is in, for limits on a whole network: an IPv6 /48 (what one site is
/// usually given), an IPv4 address itself.
pub fn ip_network(ip: IpAddr) -> String {
    match ip.to_canonical() {
        IpAddr::V4(a) => a.to_string(),
        IpAddr::V6(a) => {
            let net = std::net::Ipv6Addr::from(u128::from(a) & (u128::MAX << 80));
            format!("{net}/48")
        }
    }
}

/// Over HTTPS, the cookies without the `__Host-` prefix are dropped before anything reads
/// them: a site on a sibling or parent domain can set those (never the prefixed ones), and
/// could otherwise plant a session or a sign-in of its own (session fixation, login CSRF).
pub fn drop_plain_cookies(headers: &mut HeaderMap) {
    let plain = [crate::COOKIE, crate::oidc::COOKIE];
    let kept: Vec<String> = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .map(str::trim)
        .filter(|c| !c.is_empty() && !plain.contains(&c.split('=').next().unwrap_or("").trim()))
        .map(str::to_string)
        .collect();
    headers.remove(header::COOKIE);
    if let Ok(v) = HeaderValue::from_str(&kept.join("; "))
        && !kept.is_empty()
    {
        headers.insert(header::COOKIE, v);
    }
}

/// The key for an address in the limiter ("?" when unknown; an IPv6 address counts by its /64).
pub fn ip_key(kind: &str, ip: Option<IpAddr>) -> String {
    format!("{kind}:{}", ip.map_or_else(|| "?".to_string(), ip_prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> Config {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        Config::from(|k| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
    }

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(header::HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn ip(s: &str) -> Option<IpAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn settings() {
        let c = cfg(&[
            ("STARBASE_TRUSTED_PROXY", "10.0.0.1, 172.16.0.0/12"),
            ("STARBASE_PUBLIC_URL", "https://Sheets.Example.com/"),
            ("STARBASE_ALLOWED_HOSTS", "nas.local,files:8080"),
            ("STARBASE_SECURE_COOKIE", "1"),
        ]);
        assert_eq!(c.public.as_deref(), Some("sheets.example.com"));
        assert_eq!(c.hosts, vec!["nas.local".to_string(), "files".to_string()]);
        assert!(c.secure_cookie && c.trusts("10.0.0.1".parse().unwrap()) && c.trusts("172.20.1.2".parse().unwrap()));
        assert!(!c.trusts("10.0.0.2".parse().unwrap()));
        assert_eq!(cfg(&[]), Config::default());
    }

    #[test]
    fn who_is_asking_and_over_what() {
        let c = cfg(&[("STARBASE_TRUSTED_PROXY", "10.0.0.1")]);
        let h = headers(&[("x-forwarded-for", "6.6.6.6, 1.2.3.4"), ("x-forwarded-proto", "https")]);
        assert_eq!(client_ip(ip("10.0.0.1"), &h, &c), ip("1.2.3.4"), "from a trusted proxy: the last address it added");
        assert_eq!(client_ip(ip("9.9.9.9"), &h, &c), ip("9.9.9.9"), "anyone else can't choose their address");
        assert!(secure(ip("10.0.0.1"), &h, &c));
        assert!(!secure(ip("9.9.9.9"), &h, &c), "X-Forwarded-Proto only from a trusted proxy");
        assert!(!secure(ip("10.0.0.1"), &headers(&[]), &c), "plain HTTP on a home network");
        assert!(secure(None, &headers(&[]), &cfg(&[("STARBASE_SECURE_COOKIE", "yes")])));
    }

    #[test]
    fn origins_and_host_names() {
        let open = Config::default();
        let uri: axum::http::Uri = "/api/files".parse().unwrap();
        let live: axum::http::Uri = "/api/live".parse().unwrap();
        let same = headers(&[("host", "192.168.1.5:8080"), ("origin", "http://192.168.1.5:8080")]);
        let other = headers(&[("host", "192.168.1.5:8080"), ("origin", "https://evil.example")]);
        let none = headers(&[("host", "192.168.1.5:8080")]);
        assert!(check(&Method::POST, &uri, &same, None, &open).is_ok());
        assert_eq!(check(&Method::PUT, &uri, &other, None, &open).unwrap_err().0, 403);
        assert_eq!(check(&Method::GET, &live, &other, None, &open).unwrap_err().0, 403, "the live WebSocket too");
        assert!(check(&Method::GET, &uri, &other, None, &open).is_ok(), "reading is left to sign-in");
        assert!(check(&Method::POST, &uri, &none, None, &open).is_ok(), "no Origin: not a cross-site browser request");
        assert_eq!(check(&Method::POST, &uri, &headers(&[("host", "a"), ("origin", "null")]), None, &open).unwrap_err().0, 403);
        // Behind a proxy that changes Host: the public address, or X-Forwarded-Host from a trusted proxy.
        let proxied = headers(&[("host", "127.0.0.1:8080"), ("origin", "https://sheets.example.com"), ("x-forwarded-host", "sheets.example.com")]);
        assert_eq!(check(&Method::POST, &uri, &proxied, ip("10.0.0.1"), &open).unwrap_err().0, 403);
        let c = cfg(&[("STARBASE_TRUSTED_PROXY", "10.0.0.1")]);
        assert!(check(&Method::POST, &uri, &proxied, ip("10.0.0.1"), &c).is_ok());
        let c = cfg(&[("STARBASE_PUBLIC_URL", "https://sheets.example.com")]);
        assert!(check(&Method::POST, &uri, &proxied, None, &c).is_ok());
        // Host names: open by default; with an allow-list, the listed names, IPs and localhost.
        let rebound = headers(&[("host", "attacker.example")]);
        assert!(check(&Method::GET, &uri, &rebound, None, &open).is_ok());
        assert_eq!(check(&Method::GET, &uri, &rebound, None, &c).unwrap_err().0, 421);
        for h in ["sheets.example.com", "192.168.1.5:8080", "[::1]:8080", "localhost:8080"] {
            assert!(check(&Method::GET, &uri, &headers(&[("host", h)]), None, &c).is_ok(), "{h}");
        }
        let c = cfg(&[("STARBASE_ALLOWED_HOSTS", "nas.local")]);
        assert!(check(&Method::GET, &uri, &headers(&[("host", "NAS.local:8080")]), None, &c).is_ok());
        assert!(check(&Method::GET, &"/healthz".parse().unwrap(), &rebound, None, &c).is_ok());
    }

    #[test]
    fn device_tokens_skip_the_origin_check_only_without_cookies() {
        let open = Config::default();
        let uri: axum::http::Uri = "/api/files".parse().unwrap();
        let live: axum::http::Uri = "/api/live".parse().unwrap();
        let token = headers(&[("host", "a:1"), ("origin", "https://evil.example"), ("authorization", "Bearer sbd_x")]);
        assert!(check(&Method::POST, &uri, &token, None, &open).is_ok(), "nothing ambient: sign-in decides");
        assert!(check(&Method::GET, &live, &token, None, &open).is_ok());
        // Anything a browser could send by itself is refused cross-site as before: a proxy's
        // Basic password, another Bearer token, or any token with sign-in off.
        for a in ["Basic dXNlcjpwYXNz", "Bearer something-else", "Bearer"] {
            let h = headers(&[("host", "a:1"), ("origin", "https://evil.example"), ("authorization", a)]);
            assert_eq!(check(&Method::POST, &uri, &h, None, &open).unwrap_err().0, 403, "{a}");
            assert_eq!(check(&Method::GET, &live, &h, None, &open).unwrap_err().0, 403, "{a}");
        }
        let off = cfg(&[("STARBASE_AUTH", "off")]);
        assert!(off.no_auth);
        assert_eq!(check(&Method::POST, &uri, &token, None, &off).unwrap_err().0, 403, "sign-in off");
        let with_cookie =
            headers(&[("host", "a:1"), ("origin", "https://evil.example"), ("authorization", "Bearer sbd_x"), ("cookie", "starsheets_session=s")]);
        assert_eq!(check(&Method::POST, &uri, &with_cookie, None, &open).unwrap_err().0, 403);
        assert_eq!(check(&Method::GET, &live, &with_cookie, None, &open).unwrap_err().0, 403);
        // The host-name allow-list still applies.
        let c = cfg(&[("STARBASE_ALLOWED_HOSTS", "nas.local")]);
        let rebound = headers(&[("host", "attacker.example"), ("authorization", "Bearer sbd_x")]);
        assert_eq!(check(&Method::GET, &uri, &rebound, None, &c).unwrap_err().0, 421);
        assert_eq!(bearer(&headers(&[("authorization", "bearer  sbd_abc ")])).as_deref(), Some("sbd_abc"));
        assert!(bearer(&headers(&[("authorization", "Basic abc")])).is_none());
        assert!(bearer(&headers(&[("authorization", "Bearer ")])).is_none());
    }

    #[test]
    fn buckets() {
        let l = Limiter::default();
        let b = Bucket { burst: 3.0, every: Duration::from_secs(3600) };
        assert!(l.take("k", b) && l.take("k", b) && l.take("k", b));
        assert!(!l.can("k", b) && !l.take("k", b), "empty");
        assert!(l.take("other", b), "each key has its own");
        let quick = Bucket { burst: 1.0, every: Duration::from_millis(20) };
        assert!(l.take("q", quick) && !l.take("q", quick));
        std::thread::sleep(Duration::from_millis(30));
        assert!(l.take("q", quick), "fills up again");
    }

    #[test]
    fn ipv6_addresses_count_by_their_64() {
        let a = ip("2001:db8:1:2:aaaa::1");
        let b = ip("2001:db8:1:2:ffff:ffff:ffff:ffff");
        let other = ip("2001:db8:1:3::1");
        assert_eq!(ip_key("login", a), ip_key("login", b), "the same /64 is one address");
        assert_ne!(ip_key("login", a), ip_key("login", other));
        assert_eq!(ip_key("login", a), "login:2001:db8:1:2::/64");
        assert_eq!(ip_key("login", ip("1.2.3.4")), "login:1.2.3.4");
        assert_eq!(ip_key("login", ip("::ffff:1.2.3.4")), "login:1.2.3.4", "IPv4 written as IPv6");
        assert_ne!(ip_key("login", ip("1.2.3.4")), ip_key("login", ip("1.2.3.5")));
        // A whole /64 shares one bucket.
        let l = Limiter::default();
        let b1 = Bucket { burst: 2.0, every: Duration::from_secs(3600) };
        assert!(l.take(&ip_key("t", a), b1) && l.take(&ip_key("t", ip("2001:db8:1:2::77")), b1));
        assert!(!l.take(&ip_key("t", ip("2001:db8:1:2::78")), b1));
        assert_eq!(ip_network(ip("2001:db8:1:2::1").unwrap()), ip_network(ip("2001:db8:1:ffff::1").unwrap()));
        assert_ne!(ip_network(ip("2001:db8:1:2::1").unwrap()), ip_network(ip("2001:db8:2::1").unwrap()));
        assert_eq!(ip_network(ip("2001:db8:1:2::1").unwrap()), "2001:db8:1::/48");
    }

    #[test]
    fn plain_cookies_are_dropped_over_https() {
        let mut h = headers(&[("cookie", "starsheets_session=planted; theme=dark"), ("cookie", "__Host-starsheets_session=real; starsheets_oidc=x")]);
        drop_plain_cookies(&mut h);
        assert_eq!(h.get_all(header::COOKIE).iter().count(), 1);
        assert_eq!(h.get(header::COOKIE).unwrap(), "theme=dark; __Host-starsheets_session=real");
        let mut h = headers(&[("cookie", "starsheets_session=planted")]);
        drop_plain_cookies(&mut h);
        assert!(h.get(header::COOKIE).is_none());
        // The single sign-on start: at most as many from one address as are kept waiting.
        assert!(OIDC_IP.burst + 600.0 / OIDC_IP.every.as_secs_f64() <= 50.0);
    }
}
