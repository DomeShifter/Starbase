//! Files that aren't spreadsheets or documents (photos, PDFs, music, films, notes, anything):
//! downloading them, and looking at them in the browser.
//!
//! - `GET /api/files/ID` of such a file is streamed from disk as a download (never read whole into
//!   memory, never under the store lock), with `Content-Disposition: attachment`.
//! - `GET /api/files/ID/preview` is a small page made here, with no script at all, that shows the
//!   file: a picture as `<img>`, sound and film with the browser's own player, text in `<pre>`
//!   (escaped, at most TEXT_MAX). Its Content-Security-Policy is a sandbox (`sandbox
//!   allow-same-origin allow-downloads`: no scripts, forms, pop-ups or plugins; same-origin only so
//!   the `<img>`/`<video>` requests carry the session cookie, which is `SameSite=Strict`, and
//!   without `allow-scripts` that can't be used for anything) with `default-src 'none'` and only
//!   `img-src`/`media-src 'self'`.
//! - `GET /api/files/ID/raw` is the file's bytes for that page, only for an allow-list of types
//!   the browser shows as media (never HTML, SVG, XML or text), with that exact type, `nosniff`,
//!   `Content-Disposition: inline`, the same sandbox CSP and byte ranges (films seek). A PDF's
//!   preview is its raw bytes: the browser's PDF viewer is a plugin, and a sandbox blocks it, so a
//!   PDF goes without the sandbox; `application/pdf` with `nosniff` is only ever handed to that
//!   viewer, which runs no script on this site.
//! - HTML and SVG are never shown, only downloaded: both are documents that can carry script, and
//!   one mistake (a proxy dropping the CSP header, an old browser) would run it on this site.
//!   An SVG picture isn't worth that.
//!
//! Who may: anyone who may view the file (the same check as a download). Paths come from the
//! index and are opened with `Store::open_safe` (never through a symbolic link). Streams are
//! limited per person ([`Ticket`]).

use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::{LazyLock, Mutex};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Response, header};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use crate::{Shared, auth, blocking, error, sharing, spaces, store};

/// Text shown in a preview, at most (the rest: download it).
pub const TEXT_MAX: u64 = 1024 * 1024;
/// Bytes read from disk at a time while streaming.
const CHUNK: usize = 64 * 1024;
/// Files streamed at once, per person and in all. The request limit (guard.rs) ends when the
/// answer starts, not when a long download ends, so streams have their own.
const STREAMS_PER_PERSON: usize = 16;
const STREAMS_AT_ONCE: usize = 64;
static STREAMS: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Default::default);

/// One running stream of someone's: given back when the stream ends (or is dropped).
pub struct Ticket(String);

impl Ticket {
    /// A ticket for `who` ("" without accounts), or None when they (or everyone) already stream
    /// as many files as allowed.
    pub fn take(who: &str) -> Option<Ticket> {
        let mut m = STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        let all: usize = m.values().sum();
        let mine = m.get(who).copied().unwrap_or(0);
        if all >= STREAMS_AT_ONCE || mine >= STREAMS_PER_PERSON {
            return None;
        }
        *m.entry(who.to_string()).or_default() += 1;
        Some(Ticket(who.to_string()))
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let mut m = STREAMS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(n) = m.get_mut(&self.0) {
            *n -= 1;
            if *n == 0 {
                m.remove(&self.0);
            }
        }
    }
}

/// A PDF's raw bytes: framed only by this site's own pages (the Drive preview), nothing else.
pub const PDF_CSP: &str = "frame-ancestors 'self'; base-uri 'none'";
/// The CSP of the preview page and of raw media.
pub const SANDBOX: &str = "sandbox allow-same-origin allow-downloads; default-src 'none'; img-src 'self'; media-src 'self'; style-src 'unsafe-inline'; form-action 'none'; base-uri 'none'; frame-ancestors 'none'";

/// How the browser can show a file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Image(&'static str),
    Audio(&'static str),
    Video(&'static str),
    Pdf,
    Text,
    /// Download only (HTML, SVG and anything not listed).
    None,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Image(_) => "image",
            Kind::Audio(_) => "audio",
            Kind::Video(_) => "video",
            Kind::Pdf => "pdf",
            Kind::Text => "text",
            Kind::None => "none",
        }
    }
    /// The Content-Type raw bytes are served with; None = never served raw.
    fn raw_type(self) -> Option<&'static str> {
        match self {
            Kind::Image(t) | Kind::Audio(t) | Kind::Video(t) => Some(t),
            Kind::Pdf => Some("application/pdf"),
            Kind::Text | Kind::None => None,
        }
    }
}

/// What a file name's extension says it is (an allow-list: unknown types are download only).
pub fn kind(name: &str) -> Kind {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => Kind::Image("image/png"),
        "jpg" | "jpeg" | "jfif" => Kind::Image("image/jpeg"),
        "gif" => Kind::Image("image/gif"),
        "webp" => Kind::Image("image/webp"),
        "avif" => Kind::Image("image/avif"),
        "bmp" => Kind::Image("image/bmp"),
        "ico" => Kind::Image("image/x-icon"),
        "mp3" => Kind::Audio("audio/mpeg"),
        "wav" => Kind::Audio("audio/wav"),
        "ogg" | "oga" | "opus" => Kind::Audio("audio/ogg"),
        "flac" => Kind::Audio("audio/flac"),
        "m4a" => Kind::Audio("audio/mp4"),
        "aac" => Kind::Audio("audio/aac"),
        "mp4" | "m4v" => Kind::Video("video/mp4"),
        "webm" => Kind::Video("video/webm"),
        "ogv" => Kind::Video("video/ogg"),
        "mov" => Kind::Video("video/quicktime"),
        "pdf" => Kind::Pdf,
        "txt" | "text" | "md" | "markdown" | "log" | "json" | "yaml" | "yml" | "toml" | "ini" | "cfg" | "conf" | "env" | "tsv" | "rs" | "py"
        | "js" | "mjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cc" | "cpp" | "hpp" | "java" | "kt" | "go" | "rb" | "php" | "sh" | "bash" | "zsh"
        | "fish" | "ps1" | "bat" | "css" | "scss" | "sql" | "lua" | "pl" | "r" | "swift" | "cs" | "diff" | "patch" | "tex" | "srt" | "vtt"
        | "nfo" => Kind::Text,
        _ => Kind::None,
    }
}

/// A file found for a request: where it is on disk and what it's called.
struct Found {
    /// Opened by `Store::open_safe`: the file checked is the file read.
    file: std::fs::File,
    name: String,
    version: String,
    /// Who asked (their username; "" without accounts), for the stream limits.
    who: String,
    /// A picture someone else's: sent without where it was taken (photos/strip.rs).
    strip: bool,
}

/// Finds file `id` for `me` (who must be able to view it). Err = the reply to send.
async fn find(app: &Shared, me: Option<auth::User>, id: String, opened: bool) -> Result<Found, Response<Body>> {
    blocking(app, move |a| {
        let acc = sharing::Access::new(&a.store, a.auth, me.as_ref());
        let role = acc.file_role(&id);
        if role < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let path = a.store.path_of(&id).ok().flatten().ok_or_else(|| error(404, "no such file"))?;
        let file = a.store.open_safe(&path).ok_or_else(|| error(404, "no such file"))?;
        let version = a.store.version_of(&a.store.full(&path)).map_err(|_| error(404, "no such file"))?;
        if opened && let Some(u) = &me {
            let _ = spaces::note_opened(&a.store, u, &id);
        }
        let name = store::base_of(&path);
        let lower = name.to_ascii_lowercase();
        let photo = [".jpg", ".jpeg", ".png"].iter().any(|e| lower.ends_with(e));
        let strip = photo && role < sharing::Role::Owner && !acc.keeps_location(&id);
        Ok(Found { file, name, version, who: me.map(|u| u.username).unwrap_or_default(), strip })
    })
    .await
    .unwrap_or_else(|| Err(error(500, "server error")))
}

/// Whether file `id` is one this module serves (exists and isn't a spreadsheet or document).
pub async fn is_other(app: &Shared, id: String) -> bool {
    blocking(app, move |a| a.store.path_of(&id).ok().flatten().is_some_and(|p| !store::is_editable(&p))).await.unwrap_or(false)
}

/// `filename*=` for a Content-Disposition header (RFC 6266/5987).
pub fn disposition(kind: &str, name: &str) -> String {
    let enc: String =
        name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("{kind}; filename*=UTF-8''{enc}")
}

/// One `bytes=` range of a file `len` long: Ok(None) = the whole file, Err = can't be satisfied.
pub fn range(h: Option<&str>, len: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(spec) = h.and_then(|h| h.trim().strip_prefix("bytes=")) else { return Ok(None) };
    if spec.contains(',') {
        return Ok(None); // several ranges: the whole file is a fine answer
    }
    let Some((a, b)) = spec.trim().split_once('-') else { return Ok(None) };
    let (a, b) = (a.trim(), b.trim());
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        (len.saturating_sub(n), len.saturating_sub(1))
    } else {
        let s: u64 = a.parse().map_err(|_| ())?;
        let e: u64 = if b.is_empty() { len.saturating_sub(1) } else { b.parse::<u64>().map_err(|_| ())?.min(len.saturating_sub(1)) };
        (s, e)
    };
    if len == 0 || start >= len || start > end {
        return Err(());
    }
    Ok(Some((start, end)))
}

/// A picture that isn't the sender's own: read, location taken out in memory (nothing is written
/// anywhere) and sent whole. Not a picture we can clean: refused, never sent as it is.
async fn stripped(f: Found, extra: &[(header::HeaderName, String)], _ticket: Ticket) -> Response<Body> {
    let mut file = tokio::fs::File::from_std(f.file);
    let mut bytes = Vec::new();
    if file.metadata().await.map_or(true, |m| m.len() > crate::photos::strip::MAX) || file.read_to_end(&mut bytes).await.is_err() {
        return error(413, "That picture is too big to share without its location.");
    }
    let Some(clean) = tokio::task::spawn_blocking(move || crate::photos::strip::without_location(&bytes)).await.ok().flatten() else {
        return error(422, "That picture can't be copied without its location. Ask the owner to download it.");
    };
    let mut b = Response::builder().status(200).header(header::CONTENT_LENGTH, clean.len()).header(header::ETAG, format!("\"{}-nl\"", f.version));
    for (k, v) in extra {
        b = b.header(k, v);
    }
    b.body(Body::from(clean)).unwrap()
}

/// Streams the file from disk: `extra` headers, the Range asked for.
async fn stream(f: Found, headers: &HeaderMap, extra: &[(header::HeaderName, String)]) -> Response<Body> {
    let Some(ticket) = Ticket::take(&f.who) else {
        return error(503, "Too many downloads at once. Wait for one to finish, then try again.");
    };
    if f.strip {
        return stripped(f, extra, ticket).await;
    }
    let mut file = tokio::fs::File::from_std(f.file);
    let len = match file.metadata().await {
        Ok(m) => m.len(),
        Err(_) => return error(404, "no such file"),
    };
    let asked = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let mut b = Response::builder().header(header::ACCEPT_RANGES, "bytes").header(header::ETAG, format!("\"{}\"", f.version));
    for (k, v) in extra {
        b = b.header(k, v);
    }
    let (start, count) = match range(asked, len) {
        Err(()) => return b.status(416).header(header::CONTENT_RANGE, format!("bytes */{len}")).body(Body::empty()).unwrap(),
        Ok(None) => {
            b = b.status(200);
            (0, len)
        }
        Ok(Some((s, e))) => {
            b = b.status(206).header(header::CONTENT_RANGE, format!("bytes {s}-{e}/{len}"));
            (s, e - s + 1)
        }
    };
    if start > 0 && file.seek(SeekFrom::Start(start)).await.is_err() {
        return error(500, "storage error");
    }
    // The ticket rides along with the stream: given back when it ends or the reader goes away.
    let body = futures_util::stream::unfold((file, count, ticket), |(mut file, left, ticket)| async move {
        if left == 0 {
            return None;
        }
        let mut buf = vec![0; CHUNK.min(left as usize)];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<Bytes, std::io::Error>(Bytes::from(buf)), (file, left - n as u64, ticket)))
            }
            Err(e) => Some((Err(e), (file, 0, ticket))),
        }
    });
    b.header(header::CONTENT_LENGTH, count).body(Body::from_stream(body)).unwrap()
}

/// GET /api/files/ID for a file that isn't a spreadsheet or document: a download.
pub async fn download(app: &Shared, me: Option<auth::User>, id: String, headers: &HeaderMap) -> Response<Body> {
    let f = match find(app, me, id, false).await {
        Ok(f) => f,
        Err(r) => return r,
    };
    let extra = [
        (header::CONTENT_TYPE, "application/octet-stream".to_string()),
        (header::CONTENT_DISPOSITION, disposition("attachment", &f.name)),
        (header::CACHE_CONTROL, "no-store".to_string()),
        (header::HeaderName::from_static("x-file-name"), crate::json_ascii(&f.name)),
    ];
    stream(f, headers, &extra).await
}

/// GET /api/files/ID/raw: the bytes, for the preview page (media types only).
pub async fn raw(app: &Shared, me: Option<auth::User>, id: String, headers: &HeaderMap) -> Response<Body> {
    let f = match find(app, me, id, false).await {
        Ok(f) => f,
        Err(r) => return r,
    };
    serve_raw(f, headers).await
}

async fn serve_raw(f: Found, headers: &HeaderMap) -> Response<Body> {
    let k = kind(&f.name);
    let Some(ctype) = k.raw_type() else {
        return error(415, "This file can't be shown in the browser: download it.");
    };
    let mut extra = vec![
        (header::CONTENT_TYPE, ctype.to_string()),
        (header::CONTENT_DISPOSITION, disposition("inline", &f.name)),
        // Never kept by the browser or a proxy: access can be taken away.
        (header::CACHE_CONTROL, "no-store".to_string()),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
    ];
    if k == Kind::Pdf {
        // Drive's own preview shows a PDF in a frame over the file list: this site only, never
        // another site's frame.
        extra.push((header::X_FRAME_OPTIONS, "SAMEORIGIN".to_string()));
        extra.push((header::CONTENT_SECURITY_POLICY, PDF_CSP.to_string()));
    } else {
        extra.push((header::CONTENT_SECURITY_POLICY, SANDBOX.to_string()));
    }
    stream(f, headers, &extra).await
}

/// HTML-escapes text.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// GET /api/files/ID/preview: the page that shows the file (a PDF: the PDF itself).
pub async fn preview(app: &Shared, me: Option<auth::User>, id: String, headers: &HeaderMap) -> Response<Body> {
    let f = match find(app, me, id.clone(), true).await {
        Ok(f) => f,
        Err(r) => return r,
    };
    let k = kind(&f.name);
    if k == Kind::Pdf {
        return serve_raw(f, headers).await;
    }
    let raw = format!("raw?v={}", esc(&f.version));
    let body = match k {
        Kind::Image(_) => format!("<img src=\"{raw}\" alt=\"{}\">", esc(&f.name)),
        Kind::Audio(_) => format!("<audio controls preload=\"metadata\" src=\"{raw}\"></audio>"),
        Kind::Video(_) => format!("<video controls preload=\"metadata\" src=\"{raw}\"></video>"),
        Kind::Text => {
            let file = f.file;
            let read = tokio::task::spawn_blocking(move || -> std::io::Result<(Vec<u8>, u64)> {
                use std::io::Read;
                let len = file.metadata()?.len();
                let mut buf = Vec::new();
                file.take(TEXT_MAX).read_to_end(&mut buf)?;
                Ok((buf, len))
            })
            .await;
            let Ok(Ok((bytes, len))) = read else { return error(404, "no such file") };
            let more = if len > TEXT_MAX { "<p class=\"note\">Only the start of this file is shown. Download it to see the rest.</p>" } else { "" };
            format!("{more}<pre>{}</pre>", esc(&String::from_utf8_lossy(&bytes)))
        }
        _ => "<p class=\"note\">There's no preview for this kind of file. Download it to open it on your computer.</p>".to_string(),
    };
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{name}</title><style>{STYLE}</style></head><body data-kind=\"{kind}\"><header><span class=\"name\">{name}</span>\
<a class=\"button\" href=\"../{id}\" download=\"{name}\">Download</a></header><main>{body}</main></body></html>",
        name = esc(&f.name),
        kind = k.name(),
        id = esc(&id),
    );
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::CONTENT_SECURITY_POLICY, SANDBOX)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .body(Body::from(page))
        .unwrap()
}

const STYLE: &str = ":root{color-scheme:light dark;--bg:#f8fafd;--fg:#1f1f1f;--bar:#fff;--line:#dadce0;--accent:#0b57d0}\
@media (prefers-color-scheme:dark){:root{--bg:#1b1b1b;--fg:#e3e3e3;--bar:#262626;--line:#3c3c3c;--accent:#a8c7fa}}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:14px system-ui,sans-serif}\
header{display:flex;align-items:center;gap:16px;padding:12px 16px;background:var(--bar);border-bottom:1px solid var(--line);position:sticky;top:0}\
.name{flex:1;font-size:16px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}\
.button{color:var(--accent);border:1px solid var(--line);border-radius:18px;padding:8px 16px;text-decoration:none}\
main{padding:16px;display:flex;flex-direction:column;align-items:center}\
img,video{max-width:100%;max-height:calc(100vh - 100px)}audio{width:min(600px,100%);margin-top:48px}\
pre{align-self:stretch;margin:0;padding:16px;background:var(--bar);border:1px solid var(--line);border-radius:8px;white-space:pre-wrap;overflow-wrap:anywhere;font:13px ui-monospace,monospace}\
.note{color:var(--fg);opacity:.75}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_limited_per_person() {
        let mine: Vec<Ticket> = (0..STREAMS_PER_PERSON).map_while(|_| Ticket::take("streams-test-person")).collect();
        assert_eq!(mine.len(), STREAMS_PER_PERSON);
        assert!(Ticket::take("streams-test-person").is_none(), "one too many");
        let other = Ticket::take("streams-test-other");
        assert!(other.is_some(), "someone else still can");
        drop(mine);
        assert!(Ticket::take("streams-test-person").is_some(), "given back when a stream ends");
    }

    #[test]
    fn kinds_and_ranges() {
        assert_eq!(kind("a.PNG"), Kind::Image("image/png"));
        assert_eq!(kind("x.md"), Kind::Text);
        assert_eq!(kind("x.pdf"), Kind::Pdf);
        for never in ["page.html", "page.htm", "pic.svg", "x.xhtml", "x.xml", "noext", "x.exe"] {
            assert_eq!(kind(never), Kind::None, "{never}");
        }
        assert_eq!(range(None, 10), Ok(None));
        assert_eq!(range(Some("bytes=0-"), 10), Ok(Some((0, 9))));
        assert_eq!(range(Some("bytes=2-4"), 10), Ok(Some((2, 4))));
        assert_eq!(range(Some("bytes=5-100"), 10), Ok(Some((5, 9))));
        assert_eq!(range(Some("bytes=-3"), 10), Ok(Some((7, 9))));
        assert_eq!(range(Some("bytes=10-"), 10), Err(()));
        assert_eq!(range(Some("bytes=0-1,3-4"), 10), Ok(None));
        assert_eq!(esc("<a href='x'>&\""), "&lt;a href=&#39;x&#39;&gt;&amp;&quot;");
        assert_eq!(disposition("inline", "a b é.png"), "inline; filename*=UTF-8''a%20b%20%C3%A9.png");
    }
}
