//! A commenter's comment ids against the server's (server.rs). The server gives each new comment
//! its own id (starsheets' docx_comments.rs) and its save reply says which differ:
//! `"ids": {"<ours>": <server's>}`. The editor's ids are never changed: the pairs are kept here
//! and sent with every later save in the `X-Comment-Ids` header, which the server reads them by.
//! A save already waiting when a reply comes is sent with the new pairs too, so it can't delete
//! the comment under its new id or add it a second time.

use std::collections::BTreeMap;

/// The most pairs kept: past this the document is opened again (its ids are then the server's).
pub const MAX: usize = 300;

/// The `X-Comment-Ids` header: `ours=server` pairs, comma-separated (empty when there are none).
pub fn header(alias: &BTreeMap<u32, u32>) -> String {
    alias.iter().map(|(a, b)| format!("{a}={b}")).collect::<Vec<_>>().join(",")
}

/// Takes a save reply's `ids` into `alias`. `false`: not readable, or more pairs than [`MAX`];
/// the document must then be opened again.
#[must_use]
pub fn apply(alias: &mut BTreeMap<u32, u32>, ids: &serde_json::Value) -> bool {
    let Some(ids) = ids.as_object() else { return ids.is_null() };
    for (ours, server) in ids {
        let (Ok(ours), Some(server)) = (ours.parse::<u32>(), server.as_u64().and_then(|v| u32::try_from(v).ok())) else { return false };
        // One server id names one comment of ours: an older pair for it is stale.
        alias.retain(|a, b| *a == ours || *b != server);
        if ours == server {
            alias.remove(&ours);
        } else {
            alias.insert(ours, server);
        }
    }
    alias.len() <= MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_s_ids_are_kept_and_sent_with_later_saves() {
        let mut alias = BTreeMap::new();
        assert!(apply(&mut alias, &serde_json::Value::Null));
        assert_eq!(header(&alias), "");
        // The first save: our 5 is the server's 0.
        assert!(apply(&mut alias, &serde_json::json!({"5": 0})));
        assert_eq!(header(&alias), "5=0");
        // A later one: our 0 (a newer comment) is the server's 1, our 7 the server's 7.
        assert!(apply(&mut alias, &serde_json::json!({"0": 1, "7": 7})));
        assert_eq!(header(&alias), "0=1,5=0");
        // Our 5 was deleted and put back (undo): now the server's 2; the server's 1 went to our 9.
        assert!(apply(&mut alias, &serde_json::json!({"5": 2, "9": 1})));
        assert_eq!(header(&alias), "5=2,9=1");
        assert!(!apply(&mut alias, &serde_json::json!({"x": 1})));
        assert!(!apply(&mut alias, &serde_json::json!([1])));
        let many: serde_json::Map<String, serde_json::Value> = (0..=MAX as u32).map(|i| (i.to_string(), serde_json::json!(i + 1000))).collect();
        assert!(!apply(&mut BTreeMap::new(), &serde_json::Value::Object(many)));
    }
}
