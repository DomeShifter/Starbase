//! "Continue to" after signing in: a link opened while signed out (a document in Docs, a
//! spreadsheet, a folder) sends the browser to the sign-in page with `?next=<the link>`, and
//! signing in (with a password, a new account or single sign-on) goes on there instead of to
//! Drive. Only a path on this same server is followed, so a crafted link can't send someone to
//! another site after they sign in (an "open redirect"). The server checks single sign-on's
//! `next` the same way (crates/server/src/oidc.rs `safe_next`).

/// `next` if it is a path on this server: it starts with one `/`, and has no second `/` or `\`
/// right after it (`//host` and `/\host` are other sites to a browser), no backslash anywhere,
/// no scheme (`https:`, `javascript:` …, which can't start with `/`), and no spaces or control
/// characters (browsers drop tabs and newlines, which could turn it into `//host`). The same
/// holds once percent-escapes are decoded (up to three times), so `/%2F%2Fhost`, `/%5Chost` and
/// `/%09/host` are refused too, in case anything on the way decodes them. (The server's copy:
/// crates/server/src/oidc.rs `safe_next`.)
pub fn safe_next(next: &str) -> Option<String> {
    // Visible ASCII only, as the server (a Location header can't hold anything else).
    let mut ok = next.starts_with('/') && next.len() <= 2000 && next.chars().all(|c| c.is_ascii_graphic());
    let mut form = next.to_string();
    for _ in 0..4 {
        ok &= !form.starts_with("//") && !form.contains('\\') && !form.chars().any(char::is_control);
        let decoded = percent_decode(&form);
        if !ok || decoded == form {
            break;
        }
        form = decoded;
    }
    ok.then(|| next.to_string())
}

/// `%XX` escapes decoded (others left as they are; bytes that aren't UTF-8 become U+FFFD).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|&c| hex(c)), b.get(i + 2).and_then(|&c| hex(c)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The page's own `?next=` (decoded), when it is safe to follow.
#[cfg(target_arch = "wasm32")]
pub fn from_address() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let params = web_sys::UrlSearchParams::new_with_str(&search).ok()?;
    safe_next(&params.get("next")?)
}

/// Whether `next` itself carries a `next=` (following it could go round in circles).
pub fn loops(next: &str) -> bool {
    next.contains("next=")
}

/// Takes `?next=` out of the address bar (signed in already: it isn't followed, so a page that
/// sent here by mistake can't go round in circles).
#[cfg(target_arch = "wasm32")]
pub fn forget() {
    let Some(w) = web_sys::window() else { return };
    let loc = w.location();
    let Ok(search) = loc.search() else { return };
    let Ok(params) = web_sys::UrlSearchParams::new_with_str(&search) else { return };
    if params.get("next").is_none() {
        return;
    }
    params.delete("next");
    let rest: String = params.to_string().into();
    let url = format!("{}{}{}", loc.pathname().unwrap_or_default(), if rest.is_empty() { "" } else { "?" }, rest) + &loc.hash().unwrap_or_default();
    if let Ok(h) = w.history() {
        let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url));
    }
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    #[test]
    fn paths_on_this_server_are_followed() {
        for good in [
            "/",
            "/docs/?file=abc123",
            "/docs/?file=a%2Fb&x=1",
            "/?dir=ada%2FTeam",
            "/slides/?file=1#s2",
            "/a/b/c",
            "/?dir=ada%2FMy%20files",
            "/docs/?file=a%25b",
        ] {
            assert_eq!(safe_next(good).as_deref(), Some(good), "{good}");
        }
    }

    #[test]
    fn a_link_with_its_own_next_is_a_loop() {
        assert!(super::loops("/?next=/docs/"));
        assert!(!super::loops("/docs/?file=abc"));
    }

    #[test]
    fn other_sites_and_schemes_are_refused() {
        for bad in [
            "",
            "https://evil.example/",
            "http://evil.example",
            "//evil.example/x",
            "///evil.example",
            "/\\evil.example",
            "\\\\evil.example",
            "/docs\\..\\x",
            "javascript:alert(1)",
            "data:text/html,hi",
            "docs/?file=1",
            "evil.example",
            "/\t/evil.example",
            "/\n/evil.example",
            "/ /evil.example",
            " /docs/",
            // Encoded: once, twice, upper and lower case.
            "/%2F%2Fevil.example",
            "/%2f/evil.example",
            "/%252F%252Fevil.example",
            "/%5Cevil.example",
            "/docs/?x=%5c",
            "/%09/evil.example",
            "/%0A/evil.example",
            // Not ASCII (what `/%C3%A9` decodes to).
            "/é",
        ] {
            assert!(safe_next(bad).is_none(), "{bad:?}");
        }
        assert!(safe_next(&format!("/{}", "a".repeat(2000))).is_none());
    }
}
