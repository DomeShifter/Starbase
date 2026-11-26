//! `GET /api/zip?files=ID,ID&folders=PATH` (paths separated by a newline): several files and
//! folders as ONE .zip, as Google Drive does. Folders go in with everything below them; a single
//! folder alone is named after itself, anything else `Starbase-<date>.zip`.
//!
//! Who may: the same checks as a single download. Every file named must be viewable (else 403 for
//! the whole request, nothing is silently left out); inside a folder, files the person can't view
//! are left out. Files are opened with `Store::open_safe` (never through a symbolic link) and the
//! zip is made in a temporary file and streamed from it (never held whole in memory, never under the store lock). The
//! total size and the number of entries are capped before anything is sent, and a download counts
//! as one stream of the person's ([`crate::preview::Ticket`]).

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

use axum::body::{Body, Bytes};
use axum::http::{Response, header};

use crate::{Shared, auth, blocking, error, preview, sharing, store};

/// Bytes of file content in one zip, at most.
pub const MAX_TOTAL: u64 = 2 * 1024 * 1024 * 1024;
/// Files in one zip, at most.
pub const MAX_ENTRIES: usize = 20_000;
/// Items named in the request, at most.
const MAX_ITEMS: usize = 500;

struct Entry {
    /// Name inside the zip.
    name: String,
    /// Path in the store.
    path: String,
    /// A picture of someone else's: its location is taken out first (as for a single download).
    strip: bool,
    size: u64,
}

struct Plan {
    name: String,
    entries: Vec<Entry>,
    /// Empty folders (kept, as a zip of a folder would).
    dirs: Vec<String>,
}

/// A name not yet in `taken`: `a.txt`, `a (2).txt`, `a (3).txt`…
fn unique(taken: &mut HashSet<String>, name: &str) -> String {
    let mut n = name.to_string();
    let (stem, ext) = store::split_existing(name);
    let mut i = 2;
    while !taken.insert(n.to_lowercase()) {
        n = if ext.is_empty() { format!("{stem} ({i})") } else { format!("{stem} ({i}){ext}") };
        i += 1;
    }
    n
}

fn plan(app: &crate::App, me: Option<&auth::User>, ids: &[String], folders: &[String]) -> Result<Plan, Response<Body>> {
    let acc = sharing::Access::new(&app.store, app.auth, me);
    let (all_files, all_folders) = app.store.walk();
    let by_path: HashMap<&String, &String> = acc.file_paths.iter().map(|(k, v)| (v, k)).collect();
    let mut taken = HashSet::new();
    let (mut entries, mut dirs) = (Vec::new(), Vec::new());
    let strip_for = |id: &str, path: &str, role: sharing::Role| {
        let lower = path.to_ascii_lowercase();
        let photo = [".jpg", ".jpeg", ".png"].iter().any(|e| lower.ends_with(e));
        photo && role < sharing::Role::Owner && !acc.keeps_location(id)
    };
    let add = |name: String, path: &str, strip: bool, entries: &mut Vec<Entry>| -> Result<(), Response<Body>> {
        let full = app.store.safe_file(path).ok_or_else(|| error(404, "no such file"))?;
        let meta = std::fs::metadata(full).map_err(|_| error(404, "no such file"))?;
        entries.push(Entry { name, path: path.to_string(), strip, size: meta.len() });
        Ok(())
    };
    for id in ids {
        let role = acc.file_role(id);
        if role < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let path = acc.file_paths.get(id).ok_or_else(|| error(404, "no such file"))?;
        let name = unique(&mut taken, &store::base_of(path));
        add(name, path, strip_for(id, path, role), &mut entries)?;
    }
    for f in folders {
        let f = store::rel(Some(f)).map_err(|b| error(b.code, &b.msg))?;
        if f.is_empty() || !all_folders.contains(&f) {
            return Err(error(404, "no such folder"));
        }
        if acc.folder_role(&f) < sharing::Role::View {
            return Err(error(403, "You don't have access to that."));
        }
        let top = unique(&mut taken, &store::base_of(&f));
        let prefix = format!("{f}/");
        let mut any = false;
        for path in all_files.iter().filter(|p| p.starts_with(&prefix)) {
            let Some(id) = by_path.get(path) else { continue };
            let role = acc.file_role(id);
            if role < sharing::Role::View {
                continue;
            }
            any = true;
            add(format!("{top}/{}", &path[prefix.len()..]), path, strip_for(id, path, role), &mut entries)?;
            if entries.len() > MAX_ENTRIES {
                return Err(error(413, "That is too many files for one zip. Download fewer at a time."));
            }
        }
        for d in all_folders.iter().filter(|d| d.starts_with(&prefix)) {
            if acc.folder_role(d) >= sharing::Role::View {
                dirs.push(format!("{top}/{}/", &d[prefix.len()..]));
            }
        }
        if !any {
            dirs.push(format!("{top}/"));
        }
    }
    if entries.len() > MAX_ENTRIES || entries.iter().map(|e| e.size).sum::<u64>() > MAX_TOTAL {
        return Err(error(413, "That is too big for one zip (2 GB of files at most). Download fewer at a time."));
    }
    let name = match (ids.len(), folders) {
        (0, [one]) => format!("{}.zip", store::base_of(&store::rel(Some(one)).unwrap_or_default())),
        _ => format!("Starbase-{}.zip", &store::readable_stamp()[..10]),
    };
    Ok(Plan { name, entries, dirs })
}

/// Builds the zip into a file made for it in the store's own folder and unlinked at once (so it
/// can't be left behind), and hands that file back, read from the start. (The zip library writes
/// a zip it can seek in; the download then streams from this file.)
fn write_zip(app: &crate::App, plan: Plan) -> Result<std::fs::File, String> {
    use std::io::{Seek, SeekFrom};
    use zip::write::SimpleFileOptions;
    let dir = app.store.meta();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("zip-{}.tmp", store::random_hex(8)));
    let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&tmp).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    let mut z = zip::ZipWriter::new(file);
    let base = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    for d in &plan.dirs {
        z.add_directory(d.as_str(), base).map_err(|e| e.to_string())?;
    }
    for e in &plan.entries {
        let Some(mut f) = app.store.open_safe(&e.path) else { continue };
        let mut bytes = Vec::new();
        if e.strip {
            // Not a picture we can clean: left out rather than sent with its location.
            if e.size > crate::photos::strip::MAX || f.read_to_end(&mut bytes).is_err() {
                continue;
            }
            match crate::photos::strip::without_location(&bytes) {
                Some(clean) => bytes = clean,
                None => continue,
            }
        }
        z.start_file(e.name.as_str(), base).map_err(|e| e.to_string())?;
        if e.strip {
            z.write_all(&bytes).map_err(|e| e.to_string())?;
        } else {
            std::io::copy(&mut f, &mut z).map_err(|e| e.to_string())?;
        }
    }
    let mut file = z.finish().map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    Ok(file)
}

pub async fn zip(app: &Shared, me: Option<auth::User>, q: &HashMap<String, String>) -> Response<Body> {
    let ids: Vec<String> = q.get("files").map(|s| s.split(',').filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    let folders: Vec<String> = q.get("folders").map(|s| s.split('\n').filter(|x| !x.is_empty()).map(str::to_string).collect()).unwrap_or_default();
    if ids.len() + folders.len() > MAX_ITEMS {
        return error(413, "That is too many items for one zip.");
    }
    if ids.is_empty() && folders.is_empty() {
        return error(400, "Nothing to download.");
    }
    let who = me.as_ref().map(|u| u.username.clone()).unwrap_or_default();
    let Some(ticket) = preview::Ticket::take(&who) else {
        return error(503, "Too many downloads at once. Wait for one to finish, then try again.");
    };
    let planned = blocking(app, move |a| plan(a, me.as_ref(), &ids, &folders)).await;
    let plan = match planned {
        Some(Ok(p)) => p,
        Some(Err(r)) => return r,
        None => return error(500, "server error"),
    };
    let name = plan.name.clone();
    let app2 = app.clone();
    let made = tokio::task::spawn_blocking(move || write_zip(&app2, plan)).await;
    let file = match made {
        Ok(Ok(f)) => tokio::fs::File::from_std(f),
        _ => return error(500, "The zip could not be made."),
    };
    let len = file.metadata().await.map_or(0, |m| m.len());
    // The ticket rides along with the stream: given back when it ends or the reader goes away.
    let body = futures_util::stream::unfold((file, ticket), |(mut file, ticket)| async move {
        use tokio::io::AsyncReadExt;
        let mut buf = vec![0; 64 * 1024];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok::<Bytes, std::io::Error>(Bytes::from(buf)), (file, ticket)))
            }
            Err(e) => Some((Err(e), (file, ticket))),
        }
    });
    Response::builder()
        .status(200)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, len)
        .header(header::CONTENT_DISPOSITION, preview::disposition("attachment", &name))
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::HeaderName::from_static("x-file-name"), crate::json_ascii(&name))
        .body(Body::from_stream(body))
        .unwrap()
}
