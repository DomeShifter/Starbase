//! Optional two-step sign-in with an authenticator app (TOTP, RFC 6238: 6 digits, 30 s, one step
//! either side) and ten one-time backup codes. Kept in the person's entry in users.json:
//! `totp` (secret, hex), `totp_on`, `totp_step` (the last step accepted: a code is good once),
//! `totp_pending` (a secret shown but not yet confirmed) and `backup` (SHA-256 of each unused
//! backup code). Signing in through single sign-on skips it: the provider does its own.

use std::sync::Mutex;

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Map, Value, json};
use sha1::Sha1;
use subtle::ConstantTimeEq;

use crate::auth;
use crate::store::{Store, now, random_hex};

/// Checks and changes are one at a time, so one code can't be used twice by two requests at once.
static LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn hmac_sha1(key: &[u8], msg: &[u8]) -> [u8; 20] {
    let mut m = <Hmac<Sha1> as KeyInit>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(msg);
    m.finalize().into_bytes().into()
}

/// The 6-digit code for a 30-second step.
pub fn code_at(secret: &[u8], step: u64) -> u32 {
    let h = hmac_sha1(secret, &step.to_be_bytes());
    let o = (h[19] & 15) as usize;
    u32::from_be_bytes([h[o] & 0x7f, h[o + 1], h[o + 2], h[o + 3]]) % 1_000_000
}

pub fn base32(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut out, mut buf, mut bits) = (String::new(), 0u32, 0);
    for b in bytes {
        buf = (buf << 8) | *b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(A[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
        buf &= 31;
    }
    if bits > 0 {
        out.push(A[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// The step a code is good for: right for this step or one either side, and newer than `after`.
fn matching_step(secret: &[u8], code: &str, at: f64, after: u64) -> Option<u64> {
    let typed: u32 = code.parse().ok().filter(|_| code.len() == 6)?;
    let cur = (at / 30.0) as u64;
    (cur.saturating_sub(1)..=cur + 1).find(|&s| s > after && bool::from(code_at(secret, s).to_be_bytes().ct_eq(&typed.to_be_bytes())))
}

fn plain(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_lowercase()
}

fn backup_hash(code: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(plain(code).as_bytes()))
}

fn field<'a>(u: &'a Map<String, Value>, k: &str) -> &'a str {
    u.get(k).and_then(Value::as_str).unwrap_or("")
}

fn entry<'a>(users: &'a mut [Map<String, Value>], username: &str) -> Result<&'a mut Map<String, Value>, String> {
    users.iter_mut().find(|u| field(u, "username") == username).ok_or_else(|| "No such person.".to_string())
}

pub fn is_on(store: &Store, username: &str) -> bool {
    auth::load(store).iter().any(|u| field(u, "username") == username && u.get("totp_on").and_then(Value::as_bool) == Some(true))
}

/// What the person's app needs: the secret (base32), the otpauth address and the QR code as rows of 0/1.
pub struct Setup {
    pub secret: String,
    pub uri: String,
    pub qr: Vec<String>,
}

/// A new secret, kept as pending until `turn_on` sees a right code.
pub fn start(store: &Store, username: &str, issuer: &str) -> Result<Setup, String> {
    let _g = lock();
    let mut users = auth::load(store);
    let u = entry(&mut users, username)?;
    if u.get("totp_on").and_then(Value::as_bool) == Some(true) {
        return Err("Two-step sign-in is already on.".into());
    }
    let secret = random_hex(20);
    u.insert("totp_pending".into(), json!(secret));
    auth::save(store, &users).map_err(|e| e.to_string())?;
    let b32 = base32(&hex::decode(&secret).map_err(|e| e.to_string())?);
    let enc = |s: &str| percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string();
    let uri = format!("otpauth://totp/{}:{}?secret={b32}&issuer={}&algorithm=SHA1&digits=6&period=30", enc(issuer), enc(username), enc(issuer));
    Ok(Setup { secret: b32, qr: qr_rows(&uri), uri })
}

fn qr_rows(text: &str) -> Vec<String> {
    use qrcode::{Color, QrCode};
    let Ok(q) = QrCode::new(text.as_bytes()) else { return Vec::new() };
    let w = q.width();
    q.to_colors().chunks(w).map(|r| r.iter().map(|c| if *c == Color::Dark { '1' } else { '0' }).collect()).collect()
}

/// Turns it on when `code` is right for the pending secret; gives the ten backup codes (shown once).
pub fn turn_on(store: &Store, username: &str, code: &str) -> Result<Vec<String>, String> {
    let _g = lock();
    let mut users = auth::load(store);
    let u = entry(&mut users, username)?;
    let secret = hex::decode(field(u, "totp_pending")).ok().filter(|s| !s.is_empty()).ok_or("Start again: ask for a new QR code first.")?;
    let step = matching_step(&secret, &plain(code), now(), 0).ok_or("That code isn't right. Check the code in your app and try again.")?;
    let codes: Vec<String> = (0..10).map(|_| random_hex(8)).map(|c| format!("{}-{}", &c[..8], &c[8..])).collect();
    u.remove("totp_pending");
    u.insert("totp".into(), json!(hex::encode(&secret)));
    u.insert("totp_on".into(), json!(true));
    u.insert("totp_step".into(), json!(step));
    u.insert("backup".into(), json!(codes.iter().map(|c| backup_hash(c)).collect::<Vec<_>>()));
    auth::save(store, &users).map_err(|e| e.to_string())?;
    Ok(codes)
}

/// Checks a code at sign-in or before turning off: an authenticator code (once per step) or an
/// unused backup code (used up by this).
pub fn check(store: &Store, username: &str, code: &str) -> bool {
    let _g = lock();
    let mut users = auth::load(store);
    let Ok(u) = entry(&mut users, username) else { return false };
    let code = plain(code);
    if code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()) {
        let secret = hex::decode(field(u, "totp")).unwrap_or_default();
        let after = u.get("totp_step").and_then(Value::as_u64).unwrap_or(0);
        let Some(step) = (!secret.is_empty()).then(|| matching_step(&secret, &code, now(), after)).flatten() else { return false };
        u.insert("totp_step".into(), json!(step));
        return auth::save(store, &users).is_ok();
    }
    let h = backup_hash(&code);
    let mut left: Vec<Value> = u.get("backup").and_then(Value::as_array).cloned().unwrap_or_default();
    let Some(i) = left.iter().position(|v| v.as_str().is_some_and(|v| v.len() == h.len() && bool::from(v.as_bytes().ct_eq(h.as_bytes())))) else {
        return false;
    };
    left.remove(i);
    u.insert("backup".into(), json!(left));
    auth::save(store, &users).is_ok()
}

/// Turns it off (the person's own, after a code; or the owner's for someone who lost their phone).
pub fn turn_off(store: &Store, username: &str) -> Result<(), String> {
    let _g = lock();
    let mut users = auth::load(store);
    let u = entry(&mut users, username)?;
    for k in ["totp", "totp_on", "totp_step", "totp_pending", "backup"] {
        u.remove(k);
    }
    auth::save(store, &users).map_err(|e| e.to_string())
}

#[cfg(test)]
pub fn unbase32(s: &str) -> Vec<u8> {
    let (mut out, mut buf, mut bits) = (Vec::new(), 0u32, 0);
    for c in s.bytes() {
        let v = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".iter().position(|a| *a == c).unwrap() as u32;
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
            buf &= (1 << bits) - 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6238_vector() {
        // RFC 6238 appendix B, SHA-1, T = 59 s: 94287082, so 287082 as 6 digits.
        assert_eq!(code_at(b"12345678901234567890", 59 / 30), 287082);
        // The full 8-digit vector from the RFC (94287082).
        let h = hmac_sha1(b"12345678901234567890", &1u64.to_be_bytes());
        let o = (h[19] & 15) as usize;
        let n = u32::from_be_bytes([h[o] & 0x7f, h[o + 1], h[o + 2], h[o + 3]]);
        assert_eq!(n % 100_000_000, 94287082);
        assert_eq!(base32(b"12345678901234567890"), "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(unbase32("GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"), b"12345678901234567890");
    }
}
