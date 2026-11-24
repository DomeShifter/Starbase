//! Single sign-on against a pretend identity provider on a local port: discovery, keys (JWKS),
//! the token endpoint (client secret, PKCE) and userinfo, with ID tokens signed by a key made
//! for the run. The browser's part (following redirects, keeping cookies) is done by hand.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use openidconnect::core::{CoreJsonWebKey, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey};
use openidconnect::{
    AccessToken, Audience, EndUserUsername, IdToken, IdTokenClaims, JsonWebKeyId, JsonWebKeySet, PrivateSigningKey, StandardClaims, SubjectIdentifier,
};
use serde_json::json;
use tower::ServiceExt;

use super::*;
use crate::{App, Shared};

const CLIENT: &str = "starsheets";
const SECRET: &str = "a-client-secret-for-tests";

#[derive(Clone, Default)]
struct Person {
    sub: String,
    username: String,
    groups: Vec<String>,
    /// Groups only in userinfo, as Authelia does.
    groups_in_userinfo_only: bool,
}

fn person(sub: &str, username: &str, groups: &[&str]) -> Person {
    Person { sub: sub.into(), username: username.into(), groups: groups.iter().map(|g| g.to_string()).collect(), groups_in_userinfo_only: false }
}

struct Fake {
    issuer: String,
    key: CoreRsaPrivateSigningKey,
    /// code → (who, nonce, PKCE challenge, redirect URI)
    codes: Mutex<HashMap<String, (Person, String, String, String)>>,
    tokens: Mutex<HashMap<String, Person>>,
    /// Signs ID tokens for another client (a token meant for someone else).
    wrong_audience: Mutex<bool>,
}

type F = Arc<Fake>;

async fn discovery(State(f): State<F>) -> axum::Json<Value> {
    let i = &f.issuer;
    axum::Json(json!({
        "issuer": i,
        "authorization_endpoint": format!("{i}/authorize"),
        "token_endpoint": format!("{i}/token"),
        "userinfo_endpoint": format!("{i}/userinfo"),
        "jwks_uri": format!("{i}/jwks"),
        "end_session_endpoint": format!("{i}/end"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "code_challenge_methods_supported": ["S256"],
    }))
}

async fn jwks(State(f): State<F>) -> axum::Json<Value> {
    let set: JsonWebKeySet<CoreJsonWebKey> = JsonWebKeySet::new(vec![f.key.as_verification_key()]);
    axum::Json(serde_json::to_value(set).unwrap())
}

fn refuse(why: &str) -> axum::response::Response {
    axum::response::Response::builder()
        .status(400)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "error": "invalid_grant", "error_description": why }).to_string()))
        .unwrap()
}

async fn token(State(f): State<F>, headers: HeaderMap, body: String) -> axum::response::Response {
    use sha2::{Digest, Sha256};
    let form: HashMap<String, String> = form_urlencoded::parse(body.as_bytes()).into_owned().collect();
    let basic = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{CLIENT}:{SECRET}")));
    if headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) != Some(basic.as_str()) {
        return refuse("client");
    }
    let Some((who, nonce, challenge, redirect)) = f.codes.lock().unwrap().remove(form.get("code").map_or("", String::as_str)) else {
        return refuse("code");
    };
    let verifier = form.get("code_verifier").cloned().unwrap_or_default();
    if base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())) != challenge {
        return refuse("pkce");
    }
    if form.get("redirect_uri") != Some(&redirect) || form.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return refuse("redirect");
    }
    let mut extra = Map::new();
    if !who.groups_in_userinfo_only {
        extra.insert("groups".into(), json!(who.groups));
    }
    let aud = if *f.wrong_audience.lock().unwrap() { "someone-else" } else { CLIENT };
    let now = chrono::Utc::now();
    let claims = IdTokenClaims::<Extra, CoreGenderClaim>::new(
        IssuerUrl::new(f.issuer.clone()).unwrap(),
        vec![Audience::new(aud.into())],
        now + chrono::Duration::seconds(300),
        now,
        StandardClaims::new(SubjectIdentifier::new(who.sub.clone())).set_preferred_username(Some(EndUserUsername::new(who.username.clone()))),
        Extra { claims: extra },
    )
    .set_nonce(Some(Nonce::new(nonce)));
    let access = AccessToken::new(random_hex(16));
    let id = IdToken::<Extra, CoreGenderClaim, CoreJweContentEncryptionAlgorithm, CoreJwsSigningAlgorithm>::new(
        claims,
        &f.key,
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        Some(&access),
        None,
    )
    .unwrap();
    f.tokens.lock().unwrap().insert(access.secret().clone(), who);
    axum::Json(json!({ "access_token": access.secret(), "token_type": "Bearer", "expires_in": 300, "id_token": id.to_string() })).into_response()
}

async fn userinfo(State(f): State<F>, headers: HeaderMap) -> axum::response::Response {
    let bearer = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    match f.tokens.lock().unwrap().get(bearer) {
        Some(p) => {
            axum::Json(json!({ "sub": p.sub, "preferred_username": p.username, "name": format!("{} (full name)", p.username), "groups": p.groups }))
                .into_response()
        }
        None => axum::response::Response::builder().status(401).body(Body::empty()).unwrap(),
    }
}

/// Starts the pretend provider: (it, its issuer address).
async fn provider() -> F {
    use rsa::pkcs1::EncodeRsaPrivateKey;
    let key = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let pem = key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap();
    let key = CoreRsaPrivateSigningKey::from_pem(&pem, Some(JsonWebKeyId::new("test-key".into()))).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let f = Arc::new(Fake {
        issuer: format!("http://127.0.0.1:{port}"),
        key,
        codes: Mutex::default(),
        tokens: Mutex::default(),
        wrong_audience: Mutex::new(false),
    });
    let router = Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/token", post(token))
        .route("/userinfo", get(userinfo))
        .with_state(f.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    f
}

fn config(issuer: &str) -> Config {
    Config {
        issuer: issuer.into(),
        client_id: CLIENT.into(),
        client_secret: Some(SECRET.into()),
        redirect: format!("https://sheets.example.com/{CALLBACK}"),
        scopes: vec!["openid".into(), "profile".into(), "groups".into()],
        label: "Sign in with Example".into(),
        auto_create: true,
        groups_claim: "groups".into(),
        owner_group: None,
        allowed_group: None,
        only: false,
        skew: 60,
    }
}

fn server(cfg: Config) -> Shared {
    let dir = std::env::temp_dir().join(format!("starsheets-oidc-{}", random_hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store { root: dir.clone(), keep: 3, spaces: true };
    std::fs::create_dir_all(store.meta()).unwrap();
    Arc::new(App {
        store,
        base: dir,
        lock: Mutex::new(()),
        location: "here".into(),
        auth: true,
        site: None,
        rooms: crate::live::Rooms::default(),
        oidc: Some(Arc::new(Oidc::new(cfg))),
    })
}

/// A made-up address per sign-in, so the try limits don't get in the way.
fn address() -> String {
    let r = random_hex(3);
    let b = |i: usize| u8::from_str_radix(&r[i..i + 2], 16).unwrap();
    format!("10.{}.{}.{}", b(0), b(2), b(4))
}

struct Got {
    status: u16,
    location: String,
    cookies: Vec<String>,
    json: Value,
}

impl Got {
    /// The `name=value` of a cookie set by this answer.
    fn cookie(&self, name: &str) -> Option<String> {
        self.cookies.iter().find(|c| c.starts_with(&format!("{name}="))).map(|c| c.split(';').next().unwrap().to_string())
    }
}

async fn send(app: &Shared, method: &str, uri: &str, cookie: &str, body: Value) -> Got {
    let mut b = axum::http::Request::builder().method(method).uri(uri).header(header::HOST, "sheets.example.com");
    if !cookie.is_empty() {
        b = b.header(header::COOKIE, cookie);
    }
    let body = if body.is_null() { Body::empty() } else { Body::from(body.to_string()) };
    let mut req = b.body(body).unwrap();
    req.extensions_mut().insert(axum::extract::ConnectInfo(SocketAddr::new(address().parse().unwrap(), 50000)));
    let resp = crate::router(app.clone()).oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let location = resp.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let cookies = resp.headers().get_all(header::SET_COOKIE).iter().filter_map(|v| v.to_str().ok().map(str::to_string)).collect();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Got { status, location, cookies, json: serde_json::from_slice(&bytes).unwrap_or(Value::Null) }
}

/// The browser's trip: StarSheets → the provider (which signs `who` in, or hands back
/// `nonce` instead of the right one) → StarSheets' callback. The callback's answer.
async fn trip(app: &Shared, f: &Fake, start: &str, cookie: &str, who: &Person, nonce: Option<&str>) -> Got {
    let g = send(app, "GET", start, cookie, Value::Null).await;
    assert_eq!(g.status, 303, "{:?}", g.json);
    let pending = g.cookie(COOKIE).expect("the sign-in cookie");
    assert!(g.cookies[0].contains("HttpOnly") && g.cookies[0].contains("SameSite=Lax") && g.cookies[0].contains("Secure"), "{:?}", g.cookies);
    let url = Url::parse(&g.location).unwrap();
    assert!(g.location.starts_with(&format!("{}/authorize?", f.issuer)), "{}", g.location);
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["client_id"], CLIENT);
    assert!(q["scope"].split(' ').any(|s| s == "openid") && q["scope"].contains("groups"), "{}", q["scope"]);
    let code = random_hex(8);
    let nonce = nonce.map_or_else(|| q["nonce"].clone(), str::to_string);
    f.codes.lock().unwrap().insert(code.clone(), (who.clone(), nonce, q["code_challenge"].clone(), q["redirect_uri"].clone()));
    let both = if cookie.is_empty() { pending } else { format!("{pending}; {cookie}") };
    send(app, "GET", &format!("/api/oidc/callback?code={code}&state={}", q["state"]), &both, Value::Null).await
}

async fn sign_in(app: &Shared, f: &Fake, who: &Person) -> Got {
    trip(app, f, "/api/oidc/login?next=/folder%3Fx%3D1", "", who, None).await
}

fn session(g: &Got) -> String {
    g.cookie(crate::COOKIE).filter(|c| !c.ends_with('=')).unwrap_or_else(|| panic!("no session: {} {:?}", g.location, g.cookies))
}

async fn me(app: &Shared, cookie: &str) -> Value {
    send(app, "GET", "/api/info", cookie, Value::Null).await.json
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn first_sign_in_makes_an_account_and_the_next_reuses_it() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let g = sign_in(&app, &f, &person("sub-ann", "Ann Smith", &[])).await;
    assert_eq!((g.status, g.location.as_str()), (303, "/folder?x=1"));
    assert!(g.cookies.iter().any(|c| c.starts_with("starsheets_oidc=;") && c.contains("Max-Age=0")), "the sign-in cookie is cleared");
    let s = session(&g);
    assert!(g.cookies.iter().any(|c| c.starts_with(&s) && c.contains("SameSite=Strict") && c.contains("Secure")));
    let info = me(&app, &s).await;
    assert_eq!(info["user"]["username"], "ann-smith");
    assert_eq!(info["user"]["name"], "Ann Smith (full name)", "userinfo wins over the ID token");
    assert_eq!(info["user"]["owner"], false, "the first single sign-on account isn't the owner");
    assert_eq!(info["sso"]["linked"], true);
    assert_eq!(info["sso"]["label"], "Sign in with Example");
    // Next time, with a new name at the provider: the same account (found by issuer + subject).
    let g = sign_in(&app, &f, &person("sub-ann", "annie", &[])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["username"], "ann-smith");
    assert_eq!(auth::users(&app.store).len(), 2);
    // No password works for it; files work with the session.
    assert!(auth::check(&app.store, "ann-smith", "").is_none());
    assert_eq!(send(&app, "GET", "/api/files", &session(&g), Value::Null).await.status, 200);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_account_with_the_same_name_is_never_taken_over() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    auth::add(&app.store, "ann", "Ann", "annpassword").unwrap();
    let g = sign_in(&app, &f, &person("sub-x", "ann", &[])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["username"], "ann2");
    assert!(!auth::is_linked(&app.store, "ann"));
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn owner_group_promotes_and_demotes_but_keeps_the_last_owner() {
    let f = provider().await;
    // A new server: someone outside the owner group can't claim it; someone in it can.
    let mut cfg = config(&f.issuer);
    cfg.owner_group = Some("admins".into());
    let app = server(cfg);
    let g = sign_in(&app, &f, &person("sub-x", "x", &["users"])).await;
    assert_eq!(g.location, "/folder?x=1&sso=setup-first");
    assert!(!auth::any_users(&app.store));
    // Keycloak's full group path, and groups only in userinfo (Authelia).
    let boss = Person { groups_in_userinfo_only: true, ..person("sub-boss", "boss", &["/admins"]) };
    let g = sign_in(&app, &f, &boss).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], true);
    // Another admin, then dropped from the group: owner, then not.
    let g = sign_in(&app, &f, &person("sub-cat", "cat", &["admins"])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], true);
    let g = sign_in(&app, &f, &person("sub-cat", "cat", &[])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], false);
    // The last owner leaves the group: still the owner (somebody must run the server).
    let g = sign_in(&app, &f, &person("sub-boss", "boss", &[])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], true);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn allowed_group_and_no_automatic_accounts() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.allowed_group = Some("sheets".into());
    cfg.auto_create = false;
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let g = sign_in(&app, &f, &person("sub-a", "a", &["other"])).await;
    assert!(g.location.ends_with("sso=not-allowed") && g.cookie(crate::COOKIE).is_none(), "{}", g.location);
    let g = sign_in(&app, &f, &person("sub-a", "a", &["sheets"])).await;
    assert!(g.location.ends_with("sso=no-account"), "{}", g.location);
    assert_eq!(auth::users(&app.store).len(), 1);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_nonce_and_audience_are_checked() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let ann = person("sub-ann", "ann", &[]);
    // A wrong nonce in the ID token.
    let g = trip(&app, &f, "/api/oidc/login", "", &ann, Some("not-the-nonce")).await;
    assert_eq!((g.location.as_str(), g.cookie(crate::COOKIE)), ("/?sso=failed", None));
    // An ID token for another client.
    *f.wrong_audience.lock().unwrap() = true;
    let g = sign_in(&app, &f, &ann).await;
    assert!(g.location.ends_with("sso=failed") && g.cookie(crate::COOKIE).is_none());
    *f.wrong_audience.lock().unwrap() = false;
    // State: made up, from another browser (wrong cookie), or used twice.
    let g = send(&app, "GET", "/api/oidc/login", "", Value::Null).await;
    let pending = g.cookie(COOKIE).unwrap();
    let q: HashMap<String, String> = Url::parse(&g.location).unwrap().query_pairs().into_owned().collect();
    let code = random_hex(8);
    f.codes.lock().unwrap().insert(code.clone(), (ann.clone(), q["nonce"].clone(), q["code_challenge"].clone(), q["redirect_uri"].clone()));
    let made_up = send(&app, "GET", &format!("/api/oidc/callback?code={code}&state=madeup"), &pending, Value::Null).await;
    assert_eq!(made_up.location, "/?sso=expired");
    let callback = format!("/api/oidc/callback?code={code}&state={}", q["state"]);
    let other = send(&app, "GET", &callback, &format!("{COOKIE}=0123"), Value::Null).await;
    assert_eq!(other.location, "/?sso=expired", "another browser's cookie");
    let again = send(&app, "GET", &callback, &pending, Value::Null).await;
    assert_eq!(again.location, "/?sso=expired", "the wrong cookie used the sign-in up");
    // Cancelled at the provider.
    let g = send(&app, "GET", "/api/oidc/login?next=/x", "", Value::Null).await;
    let state = Url::parse(&g.location).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let g = send(&app, "GET", &format!("/api/oidc/callback?error=access_denied&state={state}"), &g.cookie(COOKIE).unwrap(), Value::Null).await;
    assert_eq!(g.location, "/x?sso=denied");
    // Only paths on this server as the page to go back to.
    let g = send(&app, "GET", "/api/oidc/login?next=//evil.example/", "", Value::Null).await;
    let pending = g.cookie(COOKIE).unwrap();
    let state = Url::parse(&g.location).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let g = send(&app, "GET", &format!("/api/oidc/callback?error=x&state={state}"), &pending, Value::Null).await;
    assert_eq!(g.location, "/?sso=failed");
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linking_an_account_and_signing_out_at_the_provider() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let g = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await;
    let boss = g.cookie(crate::COOKIE).unwrap();
    assert_eq!(send(&app, "GET", "/api/oidc/link", "", Value::Null).await.location, "/?sso=signin-first");
    let g = trip(&app, &f, "/api/oidc/link?next=/", &boss, &person("sub-boss", "james", &[]), None).await;
    assert_eq!(g.location, "/");
    assert!(auth::is_linked(&app.store, "boss"));
    // Signing in through the provider now gives the linked account, and signing out sends the
    // browser to the provider's sign-out page.
    let g = sign_in(&app, &f, &person("sub-boss", "james", &[])).await;
    let s = session(&g);
    assert_eq!(me(&app, &s).await["user"]["username"], "boss");
    let out = send(&app, "POST", "/api/logout", &s, Value::Null).await;
    let url = out.json["logout_url"].as_str().unwrap();
    assert!(url.starts_with(&format!("{}/end?id_token_hint=", f.issuer)) && url.contains("post_logout_redirect_uri=https"), "{url}");
    assert_eq!(me(&app, &s).await["signed_in"], false);
    // A password sign-out has no provider to visit.
    assert!(send(&app, "POST", "/api/logout", &boss, Value::Null).await.json.get("logout_url").is_none());
    // Unlinking (boss has a password, so may).
    let g = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await;
    assert_eq!(send(&app, "POST", "/api/oidc/unlink", &g.cookie(crate::COOKIE).unwrap(), Value::Null).await.status, 200);
    assert!(!auth::is_linked(&app.store, "boss"));
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_single_sign_on_leaves_passwords_to_owners() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.only = true;
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    auth::add(&app.store, "ann", "Ann", "annpassword").unwrap();
    auth::set_signup(&app.store, true, "").unwrap();
    assert_eq!(send(&app, "POST", "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await.status, 403);
    assert_eq!(send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.status, 200);
    let info = me(&app, "").await;
    assert_eq!((info["sso"]["only"].as_bool(), info["signup"].as_bool()), (Some(true), Some(false)));
    let g = send(&app, "POST", "/api/signup", "", json!({"username": "new", "name": "N", "password": "longenough"})).await;
    assert_eq!(g.status, 403);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_unreachable() {
    // Nothing listens on port 9 (discard) here.
    let mut cfg = config("http://127.0.0.1:9");
    cfg.redirect = format!("http://sheets.example.com/sub/{CALLBACK}");
    let app = server(cfg);
    let g = send(&app, "GET", "/api/oidc/login", "", Value::Null).await;
    assert_eq!(g.location, "/sub/?sso=unreachable");
}

#[test]
fn settings_groups_and_pages() {
    let env = |pairs: &[(&str, &str)]| {
        let m: HashMap<String, String> = pairs.iter().map(|(k, v)| (format!("STARSHEETS_OIDC_{k}"), v.to_string())).collect();
        Config::from(move |k| m.get(k).cloned())
    };
    assert_eq!(env(&[]), Ok(None));
    assert!(env(&[("ISSUER", "https://auth.example.com")]).is_err(), "a client id is needed");
    let base = [("ISSUER", "https://auth.example.com"), ("CLIENT_ID", "sheets"), ("REDIRECT_URL", "https://sheets.example.com/api/oidc/callback")];
    let c = env(&base).unwrap().unwrap();
    assert_eq!(
        (c.scopes.join(" "), c.label.as_str(), c.auto_create, c.only, c.skew),
        ("openid profile email groups".into(), "Sign in with SSO", true, false, 60)
    );
    assert_eq!((c.base(), c.base_path(), c.https()), ("https://sheets.example.com/", "/".to_string(), true));
    let mut wrong = base;
    wrong[2].1 = "https://sheets.example.com/somewhere";
    assert!(env(&wrong).is_err(), "the redirect URL must end in api/oidc/callback");
    // Groups: a list, an object (Zitadel), one name; Keycloak's full paths match either way.
    assert_eq!(groups(Some(&json!(["a", "b", 3]))), vec!["a", "b"]);
    assert_eq!(groups(Some(&json!({"admins": {"id": 1}}))), vec!["admins"]);
    assert_eq!(groups(Some(&json!("solo"))), vec!["solo"]);
    assert!(groups(None).is_empty());
    assert!(in_group(&["/admins".into()], "admins") && in_group(&["admins".into()], "/admins") && !in_group(&["admin".into()], "admins"));
    assert!(!in_group(&["".into()], ""));
    // Only pages on this server.
    assert_eq!(safe_next("/a?b=c").as_deref(), Some("/a?b=c"));
    for bad in ["//evil.example", "/\\evil.example", "https://evil.example", "evil", "/a b"] {
        assert!(safe_next(bad).is_none(), "{bad}");
    }
}

#[test]
fn usernames_from_the_provider() {
    let dir = std::env::temp_dir().join(format!("starsheets-oidc-{}", random_hex(6)));
    std::fs::create_dir_all(&dir).unwrap();
    let s = Store { root: dir, keep: 2, spaces: false };
    assert_eq!(auth::free_username(&s, "Ann Smith"), "ann-smith");
    assert_eq!(auth::free_username(&s, "x"), "userx");
    assert_eq!(auth::free_username(&s, ".hidden"), "hidden");
    let u = auth::add_linked(&s, "ann@example.com", "", "iss", "sub", false).unwrap();
    assert_eq!((u.username.as_str(), u.name.as_str()), ("ann-example.com", "ann-example.com"));
    assert!(auth::add_linked(&s, "other", "Other", "iss", "sub", false).is_err(), "one account per sign-in");
    assert_eq!(auth::free_username(&s, "ann-example.com"), "ann-example.com2");
    assert!(auth::unlink(&s, "ann-example.com").is_err(), "no password: the link stays");
    let _ = std::fs::remove_dir_all(&s.root);
}
