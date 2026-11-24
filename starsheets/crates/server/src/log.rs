//! A plain activity log: who opened and saved what, conflicts, outside changes, reconnections
//! and refusals, one line each with the time (UTC). Printed (docker logs) and kept in
//! STORE/.starsheets/log.txt, which rolls over to log.1.txt at 1 MB, so it survives restarts and
//! sits with the files.

use std::io::Write as _;

use crate::store::Store;

const MAX: u64 = 1024 * 1024;

pub fn note(store: &Store, msg: &str) {
    let line = format!("{} {msg}", crate::store::log_stamp());
    println!("{line}");
    let path = store.meta().join("log.txt");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX) {
        let _ = std::fs::rename(&path, store.meta().join("log.1.txt"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}
