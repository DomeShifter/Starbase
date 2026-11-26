//! Messages of Starbase Docs live co-editing, shared by the server and the web app.
//!
//! The server keeps one authoritative document per open file and an op log of every change it
//! applied, numbered `seq` from 1. A client says where it is (`Hello { last_seq }`); the server
//! answers `Welcome` with either the log tail after that `seq` or, when the log no longer goes
//! back that far (or the client has nothing), the whole document. Changes carry the client's
//! own counter (`cseq`, for the acknowledgement) and the server `base` they were made on; the
//! server transforms them over what happened since.
//!
//! The editing steps are a type parameter `S` so this crate needs no engine: the server and
//! the app use `stardocs_coedit::Step`.
//!
//! Messages are adjacently tagged (`{"t": "welcome", "c": {..}}`), like the Sheets protocol.

use serde::{Deserialize, Serialize};

/// Bump when anything on the wire (or what is hashed) changes; a client that sends another
/// number in `Hello` gets [`ToClient::Incompatible`].
pub const PROTOCOL: u32 = 1;

/// A caret position on the wire: the story (`None` = body, `Some(id)` = header/footer/note part),
/// the container path (empty = the story's top level, else `[block, row, cell]` per table level),
/// the paragraph (block index in that container) and a byte offset in it.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Pos {
    pub story: Option<u32>,
    pub path: Vec<u32>,
    pub para: u32,
    pub offset: u32,
}

/// Where someone is: the caret, and the other end of the selection when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Presence {
    pub caret: Pos,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<Pos>,
}

/// Someone with the file open: one entry per connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub client: String,
    pub name: String,
    pub colour: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "c", rename_all = "snake_case")]
pub enum ToServer<S> {
    /// First message. `last_seq`: the last server revision this client has applied (0: none, send
    /// the document).
    Hello {
        file: String,
        client: String,
        #[serde(default)]
        protocol: u32,
        #[serde(default)]
        last_seq: u64,
    },
    /// A change made on top of server revision `base`.
    Change { cseq: u64, base: u64, steps: Vec<S> },
    /// Where this client's caret is.
    Presence(Presence),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Applied<S> {
    pub seq: u64,
    pub client: String,
    pub steps: Vec<S>,
}

/// What `Welcome` brings the client up to date with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", content = "c", rename_all = "snake_case")]
pub enum Catchup<S> {
    /// Changes after the client's `last_seq`, in order (possibly none).
    Tail(Vec<Applied<S>>),
    /// The log no longer goes back to `last_seq` (or the client has nothing): the whole file
    /// follows (the transport sends the bytes as a binary frame; `len` lets the client check it
    /// got them all).
    Full { len: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "c", rename_all = "snake_case")]
pub enum ToClient<S> {
    Welcome {
        /// Server revision after the catch-up.
        seq: u64,
        catchup: Catchup<S>,
        /// Fingerprint of the document at `seq` ([`hash`]).
        hash: u64,
        /// This connection's name; `Applied`, `People` and `Presence` use it.
        you: String,
        can_edit: bool,
    },
    /// A change everyone applies (also the sender's acknowledgement, `cseq` is the sender's).
    Applied {
        seq: u64,
        client: String,
        cseq: u64,
        steps: Vec<S>,
    },
    /// The sender's change was dropped (it could not be applied); it must not be re-sent.
    Rejected {
        cseq: u64,
        why: String,
    },
    /// The document's fingerprint at `seq`, sent now and then so a client can notice drift and
    /// ask again with `Hello { last_seq: 0 }`.
    Check {
        seq: u64,
        hash: u64,
    },
    People {
        people: Vec<Person>,
        you: String,
    },
    /// Where someone is, already mapped through every change up to the current `seq`.
    Presence {
        client: String,
        presence: Presence,
    },
    /// The client speaks another protocol: reload.
    Incompatible {
        server: u32,
    },
}

/// FNV-1a 64 of `bytes`: the document fingerprint (both ends must agree on what is hashed, so
/// changing that bumps [`PROTOCOL`]).
pub fn hash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let m: ToServer<String> = ToServer::Change { cseq: 3, base: 7, steps: vec!["a".into()] };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.starts_with(r#"{"t":"change","c":"#), "{j}");
        assert_eq!(serde_json::from_str::<ToServer<String>>(&j).unwrap(), m);
        let w: ToClient<String> = ToClient::Welcome {
            seq: 2,
            catchup: Catchup::Tail(vec![Applied { seq: 2, client: "c1".into(), steps: vec![] }]),
            hash: 9,
            you: "c2".into(),
            can_edit: true,
        };
        assert_eq!(serde_json::from_str::<ToClient<String>>(&serde_json::to_string(&w).unwrap()).unwrap(), w);
    }

    #[test]
    fn old_hello_has_protocol_zero() {
        let m: ToServer<String> = serde_json::from_str(r#"{"t":"hello","c":{"file":"f","client":"c"}}"#).unwrap();
        assert!(matches!(m, ToServer::Hello { protocol: 0, last_seq: 0, .. }));
    }

    #[test]
    fn hash_differs() {
        assert_ne!(hash(b"a"), hash(b"b"));
        assert_eq!(hash(b""), 0xcbf2_9ce4_8422_2325);
    }
}
