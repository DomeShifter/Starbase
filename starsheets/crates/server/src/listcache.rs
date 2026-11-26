//! A short memory for the Drive's two slow answers: the file list (GET /api/files) and the
//! storage total (GET /api/usage). On a network share each takes 0.2 to 1.7 s because it walks
//! the whole tree; the answer is kept per person for a few seconds.
//!
//! Correctness first:
//! - the key holds who is asking (and the server's folder and the whole query), so nobody is
//!   ever given another person's answer;
//! - every request that can change what the list shows (anything but a plain read, and opening a
//!   file, which feeds "Recent") and every live save bumps a counter when it is done; an answer
//!   stored under an older count is never used again, so a person sees their own change at once;
//! - the count is read BEFORE the answer is worked out, so an answer that raced a change is
//!   stored under the old count and thrown away;
//! - the few seconds (`TTL`) are the backstop for changes made outside the server (files put
//!   into the folder by hand).

use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(5);
const MAX_ENTRIES: usize = 256;

static GEN: AtomicU64 = AtomicU64::new(0);
static CACHE: LazyLock<Mutex<HashMap<String, (u64, Instant, Value)>>> = LazyLock::new(Default::default);

/// Something that may change the list or the storage total has happened (or is about to).
pub fn changed() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// Whether a request of this kind can change what the list shows.
pub fn may_change(method: &str, p: &[String]) -> bool {
    method != "GET" || (p.first().is_some_and(|f| f == "files") && p.len() == 2)
}

/// The key for a cacheable request (the list or the storage total), None for any other.
pub fn key(root: &std::path::Path, viewer: &str, method: &str, p: &[String], q: &HashMap<String, String>) -> Option<String> {
    if method != "GET" || p.len() != 1 || !matches!(p[0].as_str(), "files" | "usage") {
        return None;
    }
    let mut query: Vec<(&String, &String)> = q.iter().collect();
    query.sort();
    Some(format!("{}\u{0}{viewer}\u{0}{}\u{0}{query:?}", root.display(), p[0]))
}

/// The count to read before working an answer out.
pub fn start() -> u64 {
    GEN.load(Ordering::SeqCst)
}

pub fn get(key: &str) -> Option<Value> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (g, at, v) = cache.get(key)?;
    (*g == GEN.load(Ordering::SeqCst) && at.elapsed() < TTL).then(|| v.clone())
}

pub fn put(key: String, gen_at_start: u64, v: Value) {
    if gen_at_start != GEN.load(Ordering::SeqCst) {
        return;
    }
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= MAX_ENTRIES {
        cache.retain(|_, (g, at, _)| *g == gen_at_start && at.elapsed() < TTL);
        if cache.len() >= MAX_ENTRIES {
            cache.clear();
        }
    }
    cache.insert(key, (gen_at_start, Instant::now(), v));
}
