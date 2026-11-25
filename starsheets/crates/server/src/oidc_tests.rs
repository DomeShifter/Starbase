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
    /// The provider hasn't checked their email (email_verified false).
    email_unverified: bool,
}

fn person(sub: &str, username: &str, groups: &[&str]) -> Person {
    Person { sub: sub.into(), username: username.into(), groups: groups.iter().map(|g| g.to_string()).collect(), groups_in_userinfo_only: false, email_unverified: false }
}

struct Fake {
    issuer: String,
    key: CoreRsaPrivateSigningKey,
    /// code → (who, nonce, PKCE challenge, redirect URI)
    codes: Mutex<HashMap<String, (Person, String, String, String)>>,
    tokens: Mutex<HashMap<String, Person>>,
    /// Signs ID tokens for another client (a token meant for someone else).
    wrong_audience: Mutex<bool>,
    /// How long ago the person signed in at the provider (the ID token's auth_time), seconds.
    auth_ago: Mutex<i64>,
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
    .set_nonce(Some(Nonce::new(nonce)))
    .set_auth_time(Some(now - chrono::Duration::seconds(*f.auth_ago.lock().unwrap())));
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
            let email = format!("{}@example.org", p.username.to_lowercase().replace(' ', "."));
            axum::Json(json!({ "sub": p.sub, "preferred_username": p.username, "name": format!("{} (full name)", p.username), "email": email, "email_verified": !p.email_unverified, "groups": p.groups }))
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
        auth_ago: Mutex::new(0),
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
        provider: "Example ID".into(),
        auto_create: Some(true),
        groups_claim: "groups".into(),
        owner_group: None,
        allowed_group: None,
        only: false,
        skew: 60,
        session_hours: 12,
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
    let pending = g.cookie(HOST_COOKIE).expect("the sign-in cookie");
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
    g.cookie(crate::HOST_COOKIE).filter(|c| !c.ends_with('=')).unwrap_or_else(|| panic!("no session: {} {:?}", g.location, g.cookies))
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
    assert!(g.cookies.iter().any(|c| c.starts_with("__Host-starsheets_oidc=;") && c.contains("Max-Age=0")), "the sign-in cookie is cleared");
    assert!(g.cookies.iter().any(|c| c.starts_with("starsheets_session=;") && c.contains("Max-Age=0")), "the old session name is cleared");
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
    cfg.auto_create = Some(false);
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let g = sign_in(&app, &f, &person("sub-a", "a", &["other"])).await;
    assert!(g.location.ends_with("sso=not-allowed") && g.cookie(crate::HOST_COOKIE).is_none(), "{}", g.location);
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
    assert_eq!((g.location.as_str(), g.cookie(crate::HOST_COOKIE)), ("/?sso=failed", None));
    // An ID token for another client.
    *f.wrong_audience.lock().unwrap() = true;
    let g = sign_in(&app, &f, &ann).await;
    assert!(g.location.ends_with("sso=failed") && g.cookie(crate::HOST_COOKIE).is_none());
    *f.wrong_audience.lock().unwrap() = false;
    // State: made up, from another browser (wrong cookie), or used twice.
    let g = send(&app, "GET", "/api/oidc/login", "", Value::Null).await;
    let pending = g.cookie(HOST_COOKIE).unwrap();
    let q: HashMap<String, String> = Url::parse(&g.location).unwrap().query_pairs().into_owned().collect();
    let code = random_hex(8);
    f.codes.lock().unwrap().insert(code.clone(), (ann.clone(), q["nonce"].clone(), q["code_challenge"].clone(), q["redirect_uri"].clone()));
    let made_up = send(&app, "GET", &format!("/api/oidc/callback?code={code}&state=madeup"), &pending, Value::Null).await;
    assert_eq!(made_up.location, "/?sso=expired");
    let callback = format!("/api/oidc/callback?code={code}&state={}", q["state"]);
    let other = send(&app, "GET", &callback, &format!("{HOST_COOKIE}=0123"), Value::Null).await;
    assert_eq!(other.location, "/?sso=expired", "another browser's cookie");
    let other = send(&app, "GET", &callback, "", Value::Null).await;
    assert_eq!(other.location, "/?sso=expired", "no cookie");
    // Over HTTPS the plain cookie (which a sibling site can set) isn't read, even with the
    // right value.
    let planted = pending.replacen(HOST_COOKIE, COOKIE, 1);
    let other = send(&app, "GET", &callback, &planted, Value::Null).await;
    assert_eq!(other.location, "/?sso=expired", "a plain binding cookie over HTTPS");
    // Someone who only saw `state` can't cancel the sign-in: the right browser still finishes it.
    let g = send(&app, "GET", &callback, &pending, Value::Null).await;
    assert_eq!(g.location, "/", "{:?}", g.cookies);
    let again = send(&app, "GET", &callback, &pending, Value::Null).await;
    assert_eq!(again.location, "/?sso=expired", "used once");
    // Cancelled at the provider.
    let g = send(&app, "GET", "/api/oidc/login?next=/x", "", Value::Null).await;
    let state = Url::parse(&g.location).unwrap().query_pairs().find(|(k, _)| k == "state").unwrap().1.into_owned();
    let g = send(&app, "GET", &format!("/api/oidc/callback?error=access_denied&state={state}"), &g.cookie(HOST_COOKIE).unwrap(), Value::Null).await;
    assert_eq!(g.location, "/x?sso=denied");
    // Only paths on this server as the page to go back to.
    let g = send(&app, "GET", "/api/oidc/login?next=//evil.example/", "", Value::Null).await;
    let pending = g.cookie(HOST_COOKIE).unwrap();
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
    // Linking is a POST from a signed-in account with its current password (never a GET a
    // sibling site could send the browser to).
    assert_eq!(send(&app, "GET", "/api/oidc/link", &boss, Value::Null).await.status, 404);
    assert_eq!(send(&app, "POST", "/api/oidc/link", "", json!({"current_password": "bosspassword"})).await.status, 401);
    // (Wrong passwords with an account of its own: the per-username limit is shared by every
    // test, and main.rs's tests count boss's.)
    auth::add(&app.store, "linkwrong", "L", "linkpassword").unwrap();
    let l = send(&app, "POST", "/api/login", "", json!({"username": "linkwrong", "password": "linkpassword"})).await.cookie(crate::COOKIE).unwrap();
    assert_eq!(send(&app, "POST", "/api/oidc/link", &l, json!({"current_password": "wrong one"})).await.status, 400);
    assert_eq!(send(&app, "POST", "/api/oidc/link", &l, json!({})).await.status, 400);
    let g = link_trip(&app, &f, &boss, "bosspassword", &person("sub-boss", "james", &[])).await;
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

/// Linking through POST /api/oidc/link, then the provider, then the callback.
async fn link_trip(app: &Shared, f: &Fake, session: &str, password: &str, who: &Person) -> Got {
    let g = send(app, "POST", "/api/oidc/link", session, json!({"current_password": password, "next": "/"})).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    let pending = g.cookie(HOST_COOKIE).expect("the sign-in cookie");
    let url = Url::parse(g.json["url"].as_str().unwrap()).unwrap();
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q.get("prompt").map(String::as_str), Some("login"), "linking lets the person pick who to link");
    let code = random_hex(8);
    f.codes.lock().unwrap().insert(code.clone(), (who.clone(), q["nonce"].clone(), q["code_challenge"].clone(), q["redirect_uri"].clone()));
    send(app, "GET", &format!("/api/oidc/callback?code={code}&state={}", q["state"]), &format!("{pending}; {session}"), Value::Null).await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_password_removes_the_link_and_the_owner_sees_and_removes_links() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    auth::add(&app.store, "ann", "Ann", "annpassword").unwrap();
    let ann = send(&app, "POST", "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await.cookie(crate::COOKIE).unwrap();
    let boss = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.cookie(crate::COOKIE).unwrap();
    link_trip(&app, &f, &ann, "annpassword", &person("sub-ann", "ann", &[])).await;
    assert!(auth::is_linked(&app.store, "ann"));
    // The owner sees the link (others don't), and can remove it.
    let list = send(&app, "GET", "/api/users", &boss, Value::Null).await.json;
    let a = list.as_array().unwrap().iter().find(|u| u["username"] == "ann").unwrap().clone();
    assert_eq!((a["sso"]["issuer"].as_str(), a["sso"]["subject"].as_str()), (Some(f.issuer.as_str()), Some("sub-ann")));
    let list = send(&app, "GET", "/api/users", &ann, Value::Null).await.json;
    assert!(list.as_array().unwrap().iter().all(|u| u.get("sso").is_none()));
    assert_eq!(send(&app, "DELETE", "/api/users/sso?username=ann", &ann, Value::Null).await.status, 403);
    let through_sso = session(&sign_in(&app, &f, &person("sub-ann", "ann", &[])).await);
    assert_eq!(send(&app, "DELETE", "/api/users/sso?username=ann", &boss, Value::Null).await.status, 200);
    assert_eq!(me(&app, &through_sso).await["signed_in"], false, "sessions through the removed link end");
    assert_eq!(me(&app, &ann).await["signed_in"], true, "the password session stays");
    assert!(!auth::is_linked(&app.store, "ann"));
    // Changing the password in a password session removes the link.
    link_trip(&app, &f, &ann, "annpassword", &person("sub-ann", "ann", &[])).await;
    let g = send(&app, "POST", "/api/me", &ann, json!({"current_password": "annpassword", "password": "a new password"})).await;
    assert_eq!(g.status, 200);
    assert!(!auth::is_linked(&app.store, "ann"), "a password change removes the link");
    // ... but not in a session made through single sign-on.
    let ann = g.cookie(crate::COOKIE).unwrap();
    link_trip(&app, &f, &ann, "a new password", &person("sub-ann", "ann", &[])).await;
    let s = session(&sign_in(&app, &f, &person("sub-ann", "ann", &[])).await);
    assert_eq!(send(&app, "POST", "/api/me", &s, json!({"current_password": "a new password", "password": "another password"})).await.status, 200);
    assert!(auth::is_linked(&app.store, "ann"), "changed through single sign-on: the link stays");
    // A password reset by the owner removes it.
    assert_eq!(send(&app, "POST", "/api/users/password", &boss, json!({"username": "ann", "password": "reset password"})).await.status, 200);
    assert!(!auth::is_linked(&app.store, "ann"));
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_with_no_password_links_only_after_a_fresh_sign_in() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let ann = person("sub-ann", "ann", &[]);
    // An ordinary sign-in (the provider may not have asked anything) isn't enough ...
    let s = session(&sign_in(&app, &f, &ann).await);
    assert_eq!(send(&app, "POST", "/api/oidc/link", &s, json!({})).await.status, 403);
    // ... nor a renewal without asking.
    let s = session(&trip(&app, &f, "/api/oidc/login?silent=1", "", &ann, None).await);
    assert_eq!(send(&app, "POST", "/api/oidc/link", &s, json!({})).await.status, 403);
    // A fresh one is: the provider is asked to check again (prompt=login, max_age=0).
    let g = send(&app, "GET", "/api/oidc/login?fresh=1", "", Value::Null).await;
    let q: HashMap<String, String> = Url::parse(&g.location).unwrap().query_pairs().into_owned().collect();
    assert_eq!((q.get("prompt").map(String::as_str), q.get("max_age").map(String::as_str)), (Some("login"), Some("0")));
    let s = session(&trip(&app, &f, "/api/oidc/login?fresh=1", "", &ann, None).await);
    assert_eq!(me(&app, &s).await["sso"]["fresh"], true);
    let g = send(&app, "POST", "/api/oidc/link", &s, json!({})).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    // A provider that didn't really ask again (an old auth_time): the sign-in fails.
    *f.auth_ago.lock().unwrap() = 3600;
    let g = trip(&app, &f, "/api/oidc/login?fresh=1", "", &ann, None).await;
    assert!(g.location.ends_with("sso=failed"), "{}", g.location);
    // Nor one from a few minutes before this sign-in started (still "recent", but the provider
    // ignored prompt=login and max_age=0).
    *f.auth_ago.lock().unwrap() = 300;
    let g = trip(&app, &f, "/api/oidc/login?fresh=1", "", &ann, None).await;
    assert!(g.location.ends_with("sso=failed"), "{}", g.location);
    *f.auth_ago.lock().unwrap() = 0;
    // A fresh sign-in from long ago may not link either.
    age_sessions(&app, 3600.0, false);
    assert_eq!(send(&app, "POST", "/api/oidc/link", &s, json!({})).await.status, 403);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

/// Moves every session `secs` into the past (`expire`: also ends them).
fn age_sessions(app: &Shared, secs: f64, expire: bool) {
    let path = app.store.meta().join("sessions.json");
    let mut all = app.store.load_json(&path);
    for v in all.values_mut() {
        v["made"] = json!(v["made"].as_f64().unwrap() - secs);
        if expire {
            v["expires"] = json!(crate::store::now() - 1.0);
        }
    }
    app.store.save_json(&path, &Value::Object(all)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_sign_on_sessions_are_short_and_renewed_at_the_provider() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.session_hours = 2;
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let s = session(&sign_in(&app, &f, &person("sub-ann", "ann", &[])).await);
    let path = app.store.meta().join("sessions.json");
    let left = app.store.load_json(&path).values().next().unwrap()["expires"].as_f64().unwrap() - crate::store::now();
    assert!((7100.0..=7200.0).contains(&left), "two hours: {left}");
    assert_eq!(me(&app, &s).await["sso"]["renew"], false);
    // Ended: not signed in, but the app may renew it without asking.
    age_sessions(&app, 0.0, true);
    let info = me(&app, &s).await;
    assert_eq!((info["signed_in"].as_bool(), info["sso"]["renew"].as_bool()), (Some(false), Some(true)));
    assert_eq!(me(&app, "").await["sso"]["renew"], false);
    // Renewing: prompt=none at the provider; when it needs to ask, the old session goes (the
    // provider's redirect back doesn't bring the session cookie: SameSite=Strict).
    let g = send(&app, "GET", "/api/oidc/login?silent=1&next=/x", &s, Value::Null).await;
    let q: HashMap<String, String> = Url::parse(&g.location).unwrap().query_pairs().into_owned().collect();
    assert_eq!(q.get("prompt").map(String::as_str), Some("none"));
    let back = format!("/api/oidc/callback?error=login_required&state={}", q["state"]);
    let g = send(&app, "GET", &back, &g.cookie(HOST_COOKIE).unwrap(), Value::Null).await;
    assert_eq!(g.location, "/x?sso=renew");
    assert_eq!(me(&app, &s).await["sso"]["renew"], false, "the ended session is gone");
    assert_eq!(app.store.load_json(&path).len(), 0);
    // A normal sign-in has no prompt=none.
    let g = send(&app, "GET", "/api/oidc/login", "", Value::Null).await;
    assert!(!g.location.contains("prompt="), "{}", g.location);
    // A renewed session works again, and replaces the ended one.
    let s = session(&sign_in(&app, &f, &person("sub-ann", "ann", &[])).await);
    age_sessions(&app, 0.0, true);
    let s2 = session(&trip(&app, &f, "/api/oidc/login?silent=1", &s, &person("sub-ann", "ann", &[]), None).await);
    assert_eq!(me(&app, &s2).await["user"]["username"], "ann");
    assert_eq!(app.store.load_json(&path).len(), 1, "the ended session was removed");
    // Password sessions keep their 30 days.
    let g = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await;
    assert_eq!(g.status, 200);
    let days = app.store.load_json(&path).values().map(|v| v["expires"].as_f64().unwrap() - crate::store::now()).fold(0.0, f64::max);
    assert!(days > 29.0 * 86400.0);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn leaving_the_groups_ends_sessions() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.owner_group = Some("admins".into());
    cfg.allowed_group = Some("sheets".into());
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    // Demoted at a sign-in: the other sessions end.
    let cat = person("sub-cat", "cat", &["admins"]);
    let old = session(&sign_in(&app, &f, &cat).await);
    let g = sign_in(&app, &f, &person("sub-cat", "cat", &["sheets"])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], false);
    assert_eq!(me(&app, &old).await["signed_in"], false, "the owner's session ended");
    // Out of the allowed group: refused, and their single sign-on sessions end.
    let s = session(&g);
    let g = sign_in(&app, &f, &person("sub-cat", "cat", &[])).await;
    assert!(g.location.ends_with("sso=not-allowed"));
    assert_eq!(me(&app, &s).await["signed_in"], false);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_accounts_follow_the_sign_up_setting_unless_set() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.auto_create = None;
    let app = server(cfg.clone());
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let ann = person("sub-ann", "ann", &["sheets"]);
    assert!(sign_in(&app, &f, &ann).await.location.ends_with("sso=no-account"), "sign-up is off by default");
    auth::set_signup(&app.store, true, "a-code").unwrap();
    assert!(sign_in(&app, &f, &ann).await.location.ends_with("sso=no-account"), "sign-up needs a code the provider can't give");
    auth::set_signup(&app.store, true, "").unwrap();
    assert_eq!(me(&app, &session(&sign_in(&app, &f, &ann).await)).await["user"]["username"], "ann");
    // With an allowed group, the provider decides who.
    cfg.allowed_group = Some("sheets".into());
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    assert_eq!(me(&app, &session(&sign_in(&app, &f, &ann).await)).await["user"]["username"], "ann");
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[test]
fn waiting_sign_ins_push_out_the_oldest_never_refuse() {
    let o = Oidc::new(config("https://auth.example.com"));
    let pending_in = |from: &str, net: &str| Pending {
        binding: "b".into(),
        nonce: "n".into(),
        verifier: "v".into(),
        made: Instant::now(),
        start: Start { next: "/".into(), from: from.into(), net: net.into(), ..Default::default() },
    };
    let pending = |from: &str| pending_in(from, from);
    // One address (an IPv6 /64) can keep at most MAX_PENDING_PER_ADDRESS: its oldest go.
    for i in 0..MAX_PENDING_PER_ADDRESS + 5 {
        o.keep(format!("a{i}"), pending("2001:db8::/64"));
        std::thread::sleep(Duration::from_micros(50));
    }
    let w = o.waiting();
    assert_eq!(w.len(), MAX_PENDING_PER_ADDRESS);
    assert!(!w.contains(&"a0".to_string()) && w.contains(&format!("a{}", MAX_PENDING_PER_ADDRESS + 4)));
    // One network (an IPv6 /48) at most MAX_PENDING_PER_NETWORK, however many /64s it uses.
    for i in 0..MAX_PENDING_PER_NETWORK + 20 {
        o.keep(format!("n{i}"), pending_in(&format!("2001:db8:5:{i:x}::/64"), "2001:db8:5::/48"));
    }
    assert_eq!(o.waiting().iter().filter(|k| k.starts_with('n')).count(), MAX_PENDING_PER_NETWORK);
    // Everyone together: the oldest of all go, and a new sign-in is always kept.
    for i in 0..MAX_PENDING {
        o.keep(format!("b{i}"), pending(&format!("10.0.{}.{}", i / 256, i % 256)));
    }
    o.keep("last".into(), pending("192.0.2.1"));
    let w = o.waiting();
    assert_eq!(w.len(), MAX_PENDING);
    assert!(w.contains(&"last".to_string()) && !w.iter().any(|k| k.starts_with('a')));
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
    // A right password for someone who isn't an owner gets exactly what a wrong one gets.
    let right = send(&app, "POST", "/api/login", "", json!({"username": "ann", "password": "annpassword"})).await;
    let wrong = send(&app, "POST", "/api/login", "", json!({"username": "ann", "password": "not-the-password"})).await;
    assert_eq!((right.status, &right.json), (401, &wrong.json));
    assert!(right.cookies.is_empty());
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
        (c.scopes.join(" "), c.label.as_str(), c.auto_create, c.only, c.skew, c.session_hours),
        ("openid profile email groups".into(), "Sign in with SSO", None, false, 60, 12)
    );
    assert!(c.warnings().is_empty());
    assert_eq!(c.provider, "auth.example.com", "the provider's name: its host unless STARSHEETS_OIDC_LABEL says");
    let mut named = base.to_vec();
    named.push(("LABEL", "Keycloak"));
    assert_eq!(env(&named).unwrap().unwrap().provider, "Keycloak");
    let mut on = base.to_vec();
    on.push(("AUTO_CREATE", "on"));
    let c = env(&on).unwrap().unwrap();
    assert_eq!((c.auto_create, c.warnings().len()), (Some(true), 1), "on with no allowed group: a warning in the log");
    on.push(("ALLOWED_GROUP", "sheets"));
    assert!(env(&on).unwrap().unwrap().warnings().is_empty());
    // Plain http:// only on this computer.
    let mut plain = base;
    plain[0].1 = "http://auth.example.com";
    assert!(env(&plain).unwrap_err().contains("https://"));
    let mut plain = base;
    plain[2].1 = "http://sheets.example.com/api/oidc/callback";
    assert!(env(&plain).unwrap_err().contains("https://"));
    for local in ["http://localhost:9000", "http://127.0.0.1:9000", "http://[::1]:9000"] {
        let mut l = base;
        l[0].1 = local;
        l[2].1 = "http://localhost:8080/api/oidc/callback";
        assert!(env(&l).is_ok(), "{local}");
    }
    let mut hours = base.to_vec();
    hours.push(("SESSION_HOURS", "0"));
    assert!(env(&hours).is_err());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linking_never_takes_away_the_setup_owners_ownership() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.owner_group = Some("admins".into());
    cfg.only = true;
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    // Another owner through the group, so boss isn't the last one.
    let cat = person("sub-cat", "cat", &["admins"]);
    assert_eq!(me(&app, &session(&sign_in(&app, &f, &cat).await)).await["user"]["owner"], true);
    // The setup owner links a sign-in that isn't in the owner group: still the owner, and the
    // password still gets in with only single sign-on on.
    let boss = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.cookie(crate::COOKIE).unwrap();
    let g = link_trip(&app, &f, &boss, "bosspassword", &person("sub-boss", "james", &["users"])).await;
    assert_eq!(g.location, "/");
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], true, "linking didn't demote");
    let g = sign_in(&app, &f, &person("sub-boss", "james", &[])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], true);
    assert_eq!(send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.status, 200);
    // An owner through the group still stops being one on leaving it.
    let g = sign_in(&app, &f, &person("sub-cat", "cat", &[])).await;
    assert_eq!(me(&app, &session(&g)).await["user"]["owner"], false);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_provider_gone_down_is_noticed_when_a_sign_in_starts() {
    let f = provider().await;
    let up = Arc::new(Oidc::new(config(&f.issuer)));
    let down = Arc::new(Oidc::new(config("http://127.0.0.1:9")));
    let u = up.clone();
    let meta = tokio::task::spawn_blocking(move || u.provider()).await.unwrap().unwrap();
    // Settings read a moment ago are used as they are ...
    *down.provider.lock().unwrap() = Some((Instant::now(), meta.clone()));
    let d = down.clone();
    let start = || Start { next: "/".into(), from: "x".into(), net: "x".into(), ..Default::default() };
    assert!(tokio::task::spawn_blocking(move || d.start(start())).await.unwrap().is_ok());
    // ... older ones are read again, and a provider that doesn't answer is reported.
    *down.provider.lock().unwrap() = Some((Instant::now() - Duration::from_secs(40), meta));
    let d = down.clone();
    assert!(tokio::task::spawn_blocking(move || d.start(start())).await.unwrap().is_err());
}

#[test]
fn certificate_problems_in_plain_words() {
    assert!(certificate_hint("invalid peer certificate: Other(OtherError(CaUsedAsEndEntity))").contains("CA:FALSE"));
    assert!(certificate_hint("invalid peer certificate: UnknownIssuer").contains("SSL_CERT_FILE"));
    assert_eq!(certificate_hint("Connection refused"), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_page_learns_when_to_renew() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let ann = person("sub-ann", "ann", &[]);
    let s = session(&sign_in(&app, &f, &ann).await);
    // The page is told when the session ends, so it can renew shortly before.
    let left = me(&app, &s).await["sso"]["expires_in"].as_f64().unwrap();
    assert!((43000.0..=43200.0).contains(&left), "twelve hours: {left}");
    let boss = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.cookie(crate::COOKIE).unwrap();
    assert!(me(&app, &boss).await["sso"]["expires_in"].is_null(), "a password session isn't renewed");
    // Ended while the page is open: requests answer 401 with renew, so the app renews without
    // asking; never for a password session or no session.
    age_sessions(&app, 0.0, true);
    for path in ["/api/files", "/api/users"] {
        let g = send(&app, "GET", path, &s, Value::Null).await;
        assert_eq!((g.status, g.json["renew"].as_bool()), (401, Some(true)), "{path}");
        assert_eq!(send(&app, "GET", path, &boss, Value::Null).await.json["renew"], false, "{path}");
        assert_eq!(send(&app, "GET", path, "", Value::Null).await.json["renew"], false, "{path}");
    }
    // Renewed before it ends: the new session replaces the old one at once.
    let s = session(&sign_in(&app, &f, &ann).await);
    let s2 = session(&trip(&app, &f, "/api/oidc/login?silent=1&next=/", &s, &ann, None).await);
    assert_eq!(me(&app, &s2).await["user"]["username"], "ann");
    let old = me(&app, &s).await;
    assert_eq!((old["signed_in"].as_bool(), old["sso"]["renew"].as_bool()), (Some(false), Some(false)), "the old session is gone");
    // A renewal before the end that the provider can't do silently leaves the session as it is.
    let g = send(&app, "GET", "/api/oidc/login?silent=1&next=/x", &s2, Value::Null).await;
    let q: HashMap<String, String> = Url::parse(&g.location).unwrap().query_pairs().into_owned().collect();
    let back = format!("/api/oidc/callback?error=login_required&state={}", q["state"]);
    let g = send(&app, "GET", &back, &format!("{}; {s2}", g.cookie(HOST_COOKIE).unwrap()), Value::Null).await;
    assert_eq!(g.location, "/x?sso=renew");
    assert_eq!(me(&app, &s2).await["signed_in"], true);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_account_made_through_single_sign_on_sets_a_password_then_unlinks() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let boss = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.cookie(crate::COOKIE).unwrap();
    let ann = person("sub-ann", "ann", &[]);
    let s = session(&sign_in(&app, &f, &ann).await);
    let info = me(&app, &s).await;
    assert_eq!((info["sso"]["linked"].as_bool(), info["sso"]["password"].as_bool()), (Some(true), Some(false)));
    // No password: unlinking is refused with what to do instead, for the person and the owner.
    let g = send(&app, "POST", "/api/oidc/unlink", &s, Value::Null).await;
    assert_eq!(g.status, 400);
    assert!(g.json["error"].as_str().unwrap().contains("Set a password first"), "{:?}", g.json);
    let g = send(&app, "DELETE", "/api/users/sso?username=ann", &boss, Value::Null).await;
    assert_eq!(g.status, 400);
    assert!(g.json["error"].as_str().unwrap().contains("ann has no password"), "{:?}", g.json);
    let list = send(&app, "GET", "/api/users", &boss, Value::Null).await.json;
    assert_eq!(list.as_array().unwrap().iter().find(|u| u["username"] == "ann").unwrap()["password"], false);
    // Setting the first password needs a fresh sign-in at the provider.
    let g = send(&app, "POST", "/api/me", &s, json!({"password": "a long password"})).await;
    assert_eq!(g.status, 403, "{:?}", g.json);
    let s = session(&trip(&app, &f, "/api/oidc/login?fresh=1", "", &ann, None).await);
    assert_eq!(send(&app, "POST", "/api/me", &s, json!({"password": "short"})).await.status, 400, "still a good password");
    let g = send(&app, "POST", "/api/me", &s, json!({"password": "a long password"})).await;
    assert_eq!(g.status, 200, "{:?}", g.json);
    assert!(auth::has_password(&app.store, "ann") && auth::is_linked(&app.store, "ann"), "set through single sign-on: the link stays");
    let s = g.cookie(crate::COOKIE).unwrap();
    // Now the link can go, and the password signs in.
    assert_eq!(send(&app, "POST", "/api/oidc/unlink", &s, Value::Null).await.status, 200);
    assert!(!auth::is_linked(&app.store, "ann"));
    assert_eq!(send(&app, "POST", "/api/login", "", json!({"username": "ann", "password": "a long password"})).await.status, 200);
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_single_sign_on_keeps_everyone_but_owners_linked() {
    let f = provider().await;
    let mut cfg = config(&f.issuer);
    cfg.only = true;
    let app = server(cfg);
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    auth::add(&app.store, "ann", "Ann", "annpassword").unwrap();
    auth::link(&app.store, "ann", &f.issuer, "sub-ann").unwrap();
    let s = session(&sign_in(&app, &f, &person("sub-ann", "ann", &[])).await);
    assert_eq!(send(&app, "POST", "/api/oidc/unlink", &s, Value::Null).await.status, 400);
    assert!(auth::is_linked(&app.store, "ann"));
    // Nor may the owner take the link away, though ann has a password: she couldn't use it.
    let boss = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.cookie(crate::COOKIE).unwrap();
    let g = send(&app, "DELETE", "/api/users/sso?username=ann", &boss, Value::Null).await;
    assert_eq!(g.status, 400);
    assert!(g.json["error"].as_str().unwrap().contains("must stay linked"), "{:?}", g.json);
    assert!(auth::is_linked(&app.store, "ann"));
    let _ = std::fs::remove_dir_all(&app.store.root);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn people_shows_the_provider_and_what_it_calls_each_person() {
    let f = provider().await;
    let app = server(config(&f.issuer));
    auth::add(&app.store, "boss", "Boss", "bosspassword").unwrap();
    let boss = send(&app, "POST", "/api/login", "", json!({"username": "boss", "password": "bosspassword"})).await.cookie(crate::COOKIE).unwrap();
    sign_in(&app, &f, &person("sub-ann", "ann", &[])).await;
    let sso_of = |list: &Value, who: &str| list.as_array().unwrap().iter().find(|u| u["username"] == who).unwrap()["sso"].clone();
    let sso = sso_of(&send(&app, "GET", "/api/users", &boss, Value::Null).await.json, "ann");
    assert_eq!((sso["provider"].as_str(), sso["who"].as_str()), (Some("Example ID"), Some("ann@example.org")));
    assert_eq!((sso["issuer"].as_str(), sso["subject"].as_str()), (Some(f.issuer.as_str()), Some("sub-ann")), "the raw ids, for a tooltip");
    // Kept up to date at each sign-in (the account is still found by issuer + subject only).
    sign_in(&app, &f, &person("sub-ann", "annie", &[])).await;
    assert_eq!(sso_of(&send(&app, "GET", "/api/users", &boss, Value::Null).await.json, "ann")["who"], "annie@example.org");
    assert_eq!(auth::users(&app.store).len(), 2);
    // Linked from a password account: noted too.
    auth::add(&app.store, "bob", "Bob", "bobpassword").unwrap();
    let bob = send(&app, "POST", "/api/login", "", json!({"username": "bob", "password": "bobpassword"})).await.cookie(crate::COOKIE).unwrap();
    link_trip(&app, &f, &bob, "bobpassword", &person("sub-bob", "Bob B", &[])).await;
    assert_eq!(sso_of(&send(&app, "GET", "/api/users", &boss, Value::Null).await.json, "bob")["who"], "bob.b@example.org");
    // An email the provider hasn't verified is never shown (anyone can type boss@corp): the
    // provider's username is, instead.
    let carl = Person { email_unverified: true, ..person("sub-carl", "carl", &[]) };
    sign_in(&app, &f, &carl).await;
    assert_eq!(sso_of(&send(&app, "GET", "/api/users", &boss, Value::Null).await.json, "carl")["who"], "carl");
    let _ = std::fs::remove_dir_all(&app.store.root);
}
