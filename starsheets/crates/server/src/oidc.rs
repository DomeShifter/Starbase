//! Single sign-on through the server owner's own identity provider (OpenID Connect: Authelia,
//! Authentik, Keycloak, Kanidm and others). Setting it up: docs/oidc.md.
//!
//! The flow: GET /api/oidc/login?next=/path sends the browser to the provider (authorization
//! code with PKCE, state and nonce; kept here for ten minutes, used once, and tied to the
//! browser by a cookie). The provider sends it back to GET /api/oidc/callback: the code is
//! exchanged, the ID token checked (signature, issuer, audience, expiry, nonce, access-token
//! hash), and the provider's userinfo is always read too (some providers put groups and email
//! only there). The account is the one linked to that provider's sign-in (issuer and subject,
//! never email or username), or a new one. Then it is an ordinary session (auth.rs).
//!
//! Password sign-in keeps working alongside, unless STARBASE_OIDC_ONLY is on (then only for
//! owners).
//!
//! A session made through single sign-on lasts STARBASE_OIDC_SESSION_HOURS (12 by default),
//! not 30 days: then the app goes back to the provider without asking (`prompt=none`), so
//! someone removed or demoted there is noticed within that time.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use openidconnect::core::{CoreAuthPrompt, CoreAuthenticationFlow, CoreClient, CoreGenderClaim};
use openidconnect::url::Url;
use openidconnect::{
    AccessTokenHash, AdditionalClaims, AuthorizationCode, ClientId, ClientSecret, CsrfToken, HttpRequest, HttpResponse, IssuerUrl, Nonce,
    OAuth2TokenResponse, PkceCodeChallenge, PkceCodeVerifier, ProviderMetadataWithLogout, RedirectUrl, Scope, SyncHttpClient, TokenResponse,
    UserInfoClaims,
};
use serde_json::{Map, Value};

use crate::auth::{self, User};
use crate::store::{Store, random_hex};

/// How long a started sign-in may take at the provider.
pub const PENDING_FOR: Duration = Duration::from_secs(600);
/// Sign-ins waiting at the provider at once; a new one past this pushes out the oldest, so
/// nobody can fill the list and stop everyone else from signing in.
const MAX_PENDING: usize = 10_000;
/// Sign-ins waiting at once from one address (an IPv6 /64 counts as one, guard::ip_prefix); a
/// new one past this pushes out that address's oldest. guard::OIDC_IP lets one address start at
/// most this many in PENDING_FOR.
const MAX_PENDING_PER_ADDRESS: usize = 50;
/// Sign-ins waiting at once from one network (an IPv6 /48, guard::ip_network), so one network
/// can't push everyone else's out.
const MAX_PENDING_PER_NETWORK: usize = 500;
/// A sign-in asked to be fresh (`Start::fresh`) must have happened at the provider this
/// recently.
const FRESH_SECS: i64 = 600;
/// How long the provider's settings and keys are kept before they are read again.
const REFRESH: Duration = Duration::from_secs(3600);
/// Starting a sign-in reads them again when older than this, so a provider that has gone down
/// shows StarSheets' own "couldn't be reached" rather than the browser's error page.
const REFRESH_AT_START: Duration = Duration::from_secs(30);
/// The cookie that ties a started sign-in to the browser that started it: over HTTPS with the
/// `__Host-` prefix (HOST_COOKIE), which a sibling subdomain can't set.
pub const COOKIE: &str = "starsheets_oidc";
pub const HOST_COOKIE: &str = "__Host-starsheets_oidc";
/// How long a session made through single sign-on lasts unless STARBASE_OIDC_SESSION_HOURS
/// says otherwise.
pub const SESSION_HOURS: u64 = 12;
/// Where the provider sends people back to (the end of STARBASE_OIDC_REDIRECT_URL).
pub const CALLBACK: &str = "api/oidc/callback";

// ---------------------------------------------------------------- settings

/// The STARBASE_OIDC_* settings (docs/oidc.md).
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub redirect: String,
    pub scopes: Vec<String>,
    pub label: String,
    /// The provider's name as people know it (STARBASE_OIDC_LABEL, else the issuer's host),
    /// shown in People.
    pub provider: String,
    /// STARBASE_OIDC_AUTO_CREATE when set. Unset: on with STARBASE_OIDC_ALLOWED_GROUP,
    /// else it follows the server's sign-up setting (see `makes_accounts`).
    pub auto_create: Option<bool>,
    pub groups_claim: String,
    pub owner_group: Option<String>,
    pub allowed_group: Option<String>,
    pub only: bool,
    /// Seconds of clock difference allowed with the provider when checking expiry.
    pub skew: u64,
    /// How long a session made through single sign-on lasts, in hours.
    pub session_hours: u64,
}

/// Whether a web address may be used: https://, or plain http:// only on this computer
/// (localhost, 127.0.0.1, ::1), for testing.
fn safe_scheme(url: &Url) -> bool {
    url.scheme() == "https" || (url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]" | "::1")))
}

impl Config {
    /// From settings looked up by `get` (the environment, or a test's own). `Ok(None)`: single
    /// sign-on is off (no STARBASE_OIDC_ISSUER).
    pub fn from(get: impl Fn(&str) -> Option<String>) -> Result<Option<Config>, String> {
        let v = |k: &str| get(&format!("STARBASE_OIDC_{k}")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let yes = |k: &str, default: bool| v(k).map_or(default, |s| matches!(s.to_lowercase().as_str(), "1" | "true" | "on" | "yes"));
        let Some(issuer) = v("ISSUER") else { return Ok(None) };
        let issuer_url = Url::parse(&issuer).map_err(|e| format!("STARBASE_OIDC_ISSUER isn't a web address: {e}"))?;
        if !safe_scheme(&issuer_url) {
            return Err("STARBASE_OIDC_ISSUER must start with https:// (plain http:// only for localhost, while testing).".into());
        }
        IssuerUrl::new(issuer.clone()).map_err(|e| format!("STARBASE_OIDC_ISSUER isn't a web address: {e}"))?;
        let client_id = v("CLIENT_ID").ok_or("STARBASE_OIDC_ISSUER is set, but STARBASE_OIDC_CLIENT_ID isn't.")?;
        let redirect = v("REDIRECT_URL").ok_or("STARBASE_OIDC_ISSUER is set, but STARBASE_OIDC_REDIRECT_URL isn't.")?;
        let url = Url::parse(&redirect).map_err(|e| format!("STARBASE_OIDC_REDIRECT_URL isn't a web address: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") || !url.path().ends_with(&format!("/{CALLBACK}")) || url.query().is_some() {
            return Err(format!(
                "STARBASE_OIDC_REDIRECT_URL must be the address Starbase is opened at followed by {CALLBACK}, for example https://sheets.example.com/{CALLBACK}"
            ));
        }
        if !safe_scheme(&url) {
            return Err("STARBASE_OIDC_REDIRECT_URL must start with https:// (plain http:// only for localhost, while testing).".into());
        }
        let client_secret = match v("CLIENT_SECRET_FILE") {
            Some(path) => Some(
                std::fs::read_to_string(&path).map_err(|e| format!("Can't read STARBASE_OIDC_CLIENT_SECRET_FILE ({path}): {e}"))?.trim().to_string(),
            ),
            None => v("CLIENT_SECRET"),
        }
        .filter(|s| !s.is_empty());
        let mut scopes: Vec<String> = Vec::new();
        for s in v("SCOPES").unwrap_or_else(|| "openid profile email groups".into()).split([' ', ',']).filter(|s| !s.is_empty()) {
            if !scopes.iter().any(|x| x == s) {
                scopes.push(s.to_string());
            }
        }
        let label: String = v("BUTTON_LABEL").unwrap_or_else(|| "Sign in with SSO".into()).chars().filter(|c| !c.is_control()).take(60).collect();
        let skew = match v("CLOCK_SKEW") {
            Some(s) => s.parse::<u64>().ok().filter(|n| *n <= 3600).ok_or("STARBASE_OIDC_CLOCK_SKEW is a number of seconds (0 to 3600).")?,
            None => 60,
        };
        let session_hours = match v("SESSION_HOURS") {
            Some(s) => {
                s.parse::<u64>().ok().filter(|n| (1..=720).contains(n)).ok_or("STARBASE_OIDC_SESSION_HOURS is a number of hours (1 to 720).")?
            }
            None => SESSION_HOURS,
        };
        let auto_create = v("AUTO_CREATE").map(|_| yes("AUTO_CREATE", false));
        let provider: String = v("LABEL").unwrap_or_else(|| host_of(&issuer)).chars().filter(|c| !c.is_control()).take(60).collect();
        Ok(Some(Config {
            issuer,
            client_id,
            client_secret,
            redirect,
            scopes,
            label,
            provider,
            auto_create,
            groups_claim: v("GROUPS_CLAIM").unwrap_or_else(|| "groups".into()),
            owner_group: v("OWNER_GROUP"),
            allowed_group: v("ALLOWED_GROUP"),
            only: yes("ONLY", false),
            skew,
            session_hours,
        }))
    }

    /// Warnings for the server's log at start.
    pub fn warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        if self.auto_create == Some(true) && self.allowed_group.is_none() {
            w.push(
                "Single sign-on: STARBASE_OIDC_AUTO_CREATE is on and STARBASE_OIDC_ALLOWED_GROUP isn't set, so EVERYONE who can sign in at the identity provider gets an account here.".into(),
            );
        }
        w
    }

    /// Whether someone signing in through the provider for the first time gets an account:
    /// STARBASE_OIDC_AUTO_CREATE when set; else yes with STARBASE_OIDC_ALLOWED_GROUP (the
    /// provider decides who), else only when the owner lets people make their own account
    /// (sign-up on, with no code: a code can't be given at the provider).
    pub fn makes_accounts(&self, store: &Store) -> bool {
        self.auto_create.unwrap_or_else(|| {
            let s = auth::signup(store);
            self.allowed_group.is_some() || (s.on && s.code.is_empty())
        })
    }

    /// The address Starbase is opened at (the redirect URL without api/oidc/callback).
    pub fn base(&self) -> &str {
        self.redirect.strip_suffix(CALLBACK).unwrap_or(&self.redirect)
    }

    /// The path of that address (where people land after signing in, unless they asked for
    /// another page), ending in /.
    pub fn base_path(&self) -> String {
        Url::parse(self.base()).map(|u| u.path().to_string()).unwrap_or_else(|_| "/".into())
    }

    /// Whether the cookies of this sign-in must be Secure (the redirect URL is HTTPS).
    pub fn https(&self) -> bool {
        self.redirect.starts_with("https://")
    }
}

/// The host of a web address (its whole text when it has none).
pub fn host_of(url: &str) -> String {
    Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| url.to_string())
}

/// A page to go back to after signing in: only a path on this server (never `//host` or
/// `/\host`, which browsers treat as another site), also once percent-escapes are decoded (up to
/// three times: `/%2F%2Fhost`, `/%5Chost`, `/%09/host`), in case anything on the way decodes
/// them. The web app checks `?next=` the same way (app/src/continueto.rs).
pub fn safe_next(next: &str) -> Option<String> {
    let mut ok = next.starts_with('/') && next.len() <= 2000 && !next.chars().any(char::is_whitespace);
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

/// `path` with `?key=value` (or `&key=value`) added.
pub fn with_param(path: &str, key: &str, value: &str) -> String {
    let sep = if path.contains('?') { '&' } else { '?' };
    format!("{path}{sep}{key}={value}")
}

// ---------------------------------------------------------------- talking to the provider

#[derive(Debug)]
pub struct HttpError(String);

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HttpError {}

/// The HTTP client for the provider: never follows redirects (a redirect could send the
/// server's requests somewhere else), gives up after 15 seconds, reads at most 1 MiB, and trusts
/// the computer's own certificate store.
struct Http(ureq::Agent);

impl Http {
    fn new() -> Http {
        let tls = ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build();
        let config = ureq::Agent::config_builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(15)))
            .user_agent(concat!("Starbase/", env!("CARGO_PKG_VERSION")))
            .tls_config(tls)
            .build();
        Http(ureq::Agent::new_with_config(config))
    }
}

impl SyncHttpClient for Http {
    type Error = HttpError;

    fn call(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let (parts, body) = request.into_parts();
        let sent = if body.is_empty() {
            self.0.run(ureq::http::Request::from_parts(parts, ureq::SendBody::none()))
        } else {
            self.0.run(ureq::http::Request::from_parts(parts, body))
        };
        let (parts, mut body) = sent.map_err(|e| HttpError(e.to_string()))?.into_parts();
        let bytes = body.with_config().limit(1024 * 1024).read_to_vec().map_err(|e| HttpError(e.to_string()))?;
        Ok(HttpResponse::from_parts(parts, bytes))
    }
}

/// An error and what caused it, in one line.
fn why(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut src = e.source();
    while let Some(c) = src {
        s.push_str(": ");
        s.push_str(&c.to_string());
        src = c.source();
    }
    s
}

/// What to do about a certificate problem in `why` (an error reading the provider), in plain
/// words; "" for other problems.
pub fn certificate_hint(why: &str) -> &'static str {
    if why.contains("CaUsedAsEndEntity") {
        " (The identity provider's certificate is marked as a certificate authority (CA:TRUE), as the usual `openssl req -x509` one-liner makes it. Starbase refuses such a certificate even when you trust it, while browsers and curl accept it. Make one with `basicConstraints=critical,CA:FALSE` instead: see docs/oidc.md, \"Your own certificate\".)"
    } else if why.contains("UnknownIssuer") {
        " (Starbase doesn't trust the identity provider's certificate. If you made it yourself, add it to the list Starbase trusts with SSL_CERT_FILE: see docs/oidc.md, \"Your own certificate\".)"
    } else {
        ""
    }
}

/// Claims beyond the standard ones (the groups claim, whatever it is called).
#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
pub struct Extra {
    #[serde(flatten)]
    pub claims: Map<String, Value>,
}

impl AdditionalClaims for Extra {}

/// The groups in a groups claim: a list of names, or an object whose keys are the names
/// (Zitadel), or one name.
pub fn groups(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|g| g.as_str().map(str::to_string)).collect(),
        Some(Value::Object(o)) => o.keys().cloned().collect(),
        Some(Value::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// Whether `want` is among `groups`. Keycloak can send full paths ("/admins"): either form
/// matches.
pub fn in_group(groups: &[String], want: &str) -> bool {
    let want = want.trim().trim_start_matches('/');
    !want.is_empty() && groups.iter().any(|g| g.trim().trim_start_matches('/') == want)
}

/// The claims inside a JWT (whose signature was already checked).
fn jwt_claims(jwt: &str) -> Map<String, Value> {
    jwt.split('.')
        .nth(1)
        .and_then(|p| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(p.trim_end_matches('=')).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Who signed in, as the provider says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Identity {
    pub issuer: String,
    pub subject: String,
    /// What they are called at the provider (preferred_username, else the start of their email).
    pub username: String,
    pub name: String,
    /// Their email at the provider ("" when it gives none or hasn't verified it). Only shown,
    /// never used to match.
    pub email: String,
    pub groups: Vec<String>,
}

/// What kind of sign-in to start.
#[derive(Clone, Debug, Default)]
pub struct Start {
    /// Where to go afterwards.
    pub next: String,
    /// Linking an account that is already signed in (its username), rather than signing in.
    pub link: Option<String>,
    /// Renewing an ended session without asking (`prompt=none`): if the provider needs to ask
    /// something, the sign-in box is shown instead of an error.
    pub silent: bool,
    /// The provider must check who it is again now (`prompt=login`, `max_age=0`, and the ID
    /// token's `auth_time` checked): needed before an account with no password links another
    /// sign-in.
    pub fresh: bool,
    /// The ended session being renewed (its cookie value), removed whatever the answer: the
    /// provider's redirect back doesn't bring the session cookie (SameSite=Strict).
    pub renew: Option<String>,
    /// Who started it (guard::ip_prefix) and their network (guard::ip_network), for the
    /// limits on waiting sign-ins.
    pub from: String,
    pub net: String,
}

/// A sign-in waiting at the provider.
pub struct Pending {
    binding: String,
    nonce: String,
    verifier: String,
    made: Instant,
    /// When it started, on the wall clock (seconds): a fresh sign-in must come after it.
    made_at: i64,
    pub start: Start,
}

/// Single sign-on for one server: its settings, the sign-ins under way and the provider's
/// settings (read when first needed, then once an hour, or again after a failure).
pub struct Oidc {
    pub cfg: Config,
    http: Http,
    pending: Mutex<HashMap<String, Pending>>,
    provider: Mutex<Option<(Instant, ProviderMetadataWithLogout)>>,
}

macro_rules! client {
    ($self:ident, $meta:expr) => {
        CoreClient::from_provider_metadata($meta, ClientId::new($self.cfg.client_id.clone()), $self.cfg.client_secret.clone().map(ClientSecret::new))
            .set_redirect_uri(RedirectUrl::new($self.cfg.redirect.clone()).map_err(|e| e.to_string())?)
    };
}

impl Oidc {
    pub fn new(cfg: Config) -> Oidc {
        Oidc { cfg, http: Http::new(), pending: Mutex::default(), provider: Mutex::default() }
    }

    /// The provider's settings and keys (discovery), read again when older than `fresh`.
    /// Blocking: call it off the async threads.
    fn provider_within(&self, fresh: Duration) -> Result<ProviderMetadataWithLogout, String> {
        if let Some((at, m)) = &*self.provider.lock().unwrap_or_else(|e| e.into_inner())
            && at.elapsed() < fresh
        {
            return Ok(m.clone());
        }
        let issuer = IssuerUrl::new(self.cfg.issuer.clone()).map_err(|e| e.to_string())?;
        let m = ProviderMetadataWithLogout::discover(&issuer, &self.http).map_err(|e| {
            let why = why(&e);
            format!("couldn't read the identity provider's settings at {}: {why}{}", self.cfg.issuer, certificate_hint(&why))
        })?;
        *self.provider.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), m.clone()));
        Ok(m)
    }

    fn provider(&self) -> Result<ProviderMetadataWithLogout, String> {
        self.provider_within(REFRESH)
    }

    /// Reads the provider's settings once at start, so a mistake (a wrong address, a certificate
    /// that isn't trusted) shows in the log straight away. Blocking.
    pub fn check(&self) -> Result<(), String> {
        self.provider().map(|_| ())
    }

    /// Reads the provider's settings again next time (its keys may have changed).
    fn forget_provider(&self) {
        *self.provider.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Starts a sign-in: (the provider's address to send the browser to, the cookie value that
    /// ties it to this browser). Blocking.
    pub fn start(&self, start: Start) -> Result<(String, String), String> {
        let meta = self.provider_within(REFRESH_AT_START)?;
        let client = client!(self, meta);
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let mut req =
            client.authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random).set_pkce_challenge(challenge);
        for s in self.cfg.scopes.iter().filter(|s| *s != "openid") {
            req = req.add_scope(Scope::new(s.clone()));
        }
        if start.silent {
            req = req.add_prompt(CoreAuthPrompt::None);
        } else if start.fresh {
            req = req.add_prompt(CoreAuthPrompt::Login).add_extra_param("max_age", "0");
        } else if start.link.is_some() {
            // Linking: the provider asks who to link (otherwise it silently hands back whoever is
            // signed in there, and "Link another sign-in" couldn't pick another one).
            req = req.add_prompt(CoreAuthPrompt::Login);
        }
        let (url, state, nonce) = req.url();
        let binding = random_hex(32);
        let p = Pending {
            binding: binding.clone(),
            nonce: nonce.secret().clone(),
            verifier: verifier.secret().clone(),
            made: Instant::now(),
            made_at: chrono::Utc::now().timestamp(),
            start,
        };
        self.keep(state.secret().clone(), p);
        Ok((url.to_string(), binding))
    }

    /// Keeps a started sign-in. A full list never refuses: the oldest from the same address
    /// goes first (past MAX_PENDING_PER_ADDRESS), then from the same network (past
    /// MAX_PENDING_PER_NETWORK), then the oldest of all (past MAX_PENDING).
    fn keep(&self, state: String, p: Pending) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.retain(|_, p| p.made.elapsed() < PENDING_FOR);
        fn trim(pending: &mut HashMap<String, Pending>, max: usize, of: impl Fn(&Pending) -> bool) {
            while pending.values().filter(|q| of(q)).count() >= max {
                let Some(k) = pending.iter().filter(|(_, q)| of(q)).min_by_key(|(_, q)| q.made).map(|(k, _)| k.clone()) else { break };
                pending.remove(&k);
            }
        }
        trim(&mut pending, MAX_PENDING_PER_ADDRESS, |q| q.start.from == p.start.from);
        trim(&mut pending, MAX_PENDING_PER_NETWORK, |q| q.start.net == p.start.net);
        trim(&mut pending, MAX_PENDING, |_| true);
        pending.insert(state, p);
    }

    /// The states of the sign-ins waiting (for tests).
    #[cfg(test)]
    pub fn waiting(&self) -> Vec<String> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).keys().cloned().collect()
    }

    /// The sign-in started with `state`, if this browser (`binding`, from its cookie) started it
    /// and it hasn't expired. Used up only by the browser that started it, so someone who only
    /// saw `state` (in an address or a log) can't cancel it.
    pub fn take(&self, state: &str, binding: &str) -> Option<Pending> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let p = pending.get(state)?;
        if p.made.elapsed() >= PENDING_FOR {
            pending.remove(state);
            return None;
        }
        if !crate::same(p.binding.as_bytes(), binding.as_bytes()) {
            return None;
        }
        pending.remove(state)
    }

    /// Finishes a sign-in with the code the provider sent back: who it is, and the raw ID token
    /// (for signing out at the provider). Blocking.
    pub fn finish(&self, p: &Pending, code: &str) -> Result<(Identity, String), String> {
        let r = self.finish_inner(p, code);
        if r.is_err() {
            self.forget_provider();
        }
        r
    }

    fn finish_inner(&self, p: &Pending, code: &str) -> Result<(Identity, String), String> {
        let meta = self.provider()?;
        let issuer = meta.issuer().to_string();
        let client = client!(self, meta);
        let token = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| format!("the identity provider has no token address: {e}"))?
            .set_pkce_verifier(PkceCodeVerifier::new(p.verifier.clone()))
            .request(&self.http)
            .map_err(|e| format!("the identity provider didn't accept the sign-in code: {}", why(&e)))?;
        let id_token = token.id_token().ok_or("the identity provider sent no ID token (is the openid scope allowed for this client?)")?;
        let skew = chrono::Duration::seconds(self.cfg.skew as i64);
        let verifier = client.id_token_verifier().set_time_fn(move || chrono::Utc::now() - skew);
        let claims =
            id_token.claims(&verifier, &Nonce::new(p.nonce.clone())).map_err(|e| format!("the ID token didn't pass the checks: {}", why(&e)))?;
        if p.start.fresh {
            // Asked for a fresh sign-in: the provider must say when it checked who this is.
            let at = claims
                .auth_time()
                .ok_or("the identity provider didn't say when the person signed in (auth_time), so a fresh sign-in can't be confirmed")?;
            // After this sign-in started (allowing for clock skew), not merely recently: a
            // provider that ignored prompt=login and max_age=0 must not count as fresh.
            let skew = self.cfg.skew as i64;
            if at.timestamp() < p.made_at - skew || (chrono::Utc::now() - at).num_seconds() > FRESH_SECS + skew {
                return Err("the identity provider didn't ask the person to sign in again (auth_time is before this sign-in started)".into());
            }
        }
        if let Some(expected) = claims.access_token_hash() {
            let alg = id_token.signing_alg().map_err(|e| why(&e))?;
            let key = id_token.signing_key(&verifier).map_err(|e| why(&e))?;
            let actual = AccessTokenHash::from_token(token.access_token(), alg, key).map_err(|e| why(&e))?;
            if actual != *expected {
                return Err("the access token doesn't belong to the ID token".into());
            }
        }
        let subject = claims.subject().clone();
        let raw = id_token.to_string();
        let from_id = jwt_claims(&raw);
        let id_str = |k: &str| from_id.get(k).and_then(Value::as_str).map(str::to_string);
        // Userinfo, always: Authelia (and others) put groups, email and name only there.
        let info: Option<UserInfoClaims<Extra, CoreGenderClaim>> = match client.user_info(token.access_token().clone(), Some(subject.clone())) {
            Ok(req) => Some(req.request(&self.http).map_err(|e| format!("couldn't read the person's details (userinfo): {}", why(&e)))?),
            Err(_) => None,
        };
        let std = info.as_ref().map(|i| i.standard_claims());
        let email = std.and_then(|s| s.email()).map(|e| e.to_string()).or_else(|| id_str("email"));
        // Shown in People only when the provider vouches for it: anyone can type any address.
        let verified = std
            .and_then(|s| s.email_verified())
            .or_else(|| from_id.get("email_verified").and_then(|v| v.as_bool().or_else(|| v.as_str().map(|s| s.eq_ignore_ascii_case("true")))))
            == Some(true);
        let username = std
            .and_then(|s| s.preferred_username())
            .map(|u| u.to_string())
            .or_else(|| id_str("preferred_username"))
            .or_else(|| email.as_ref().and_then(|e| e.split('@').next()).map(str::to_string))
            .unwrap_or_else(|| "user".into());
        let name = std
            .and_then(|s| s.name())
            .and_then(|n| n.get(None))
            .map(|n| n.to_string())
            .or_else(|| id_str("name"))
            .unwrap_or_else(|| username.clone());
        let claim = &self.cfg.groups_claim;
        let groups = match info.as_ref().and_then(|i| i.additional_claims().claims.get(claim)) {
            Some(g) => groups(Some(g)),
            None => groups(from_id.get(claim)),
        };
        let email = email.filter(|_| verified).unwrap_or_default();
        Ok((Identity { issuer, subject: subject.to_string(), username, name, email, groups }, raw))
    }

    /// Where to send the browser to sign out at the provider too, when it has an address for
    /// that (Authelia has none: then only StarSheets' own session ends). Blocking.
    pub fn logout_url(&self, id_token: &str) -> Option<String> {
        let meta = self.provider().ok()?;
        let mut url = meta.additional_metadata().end_session_endpoint.as_ref()?.url().clone();
        url.query_pairs_mut()
            .append_pair("id_token_hint", id_token)
            .append_pair("client_id", &self.cfg.client_id)
            .append_pair("post_logout_redirect_uri", self.cfg.base());
        Some(url.to_string())
    }
}

// ---------------------------------------------------------------- accounts

/// The account for someone who signed in at the provider, or why there is none (a short code
/// the sign-in box explains: not-allowed, no-account, setup-first, linked-elsewhere, failed).
/// Run under the store lock.
///
/// Found by (issuer, subject) only. Made when there is none and `Config::makes_accounts`, with a username from the provider (made unique) and no password. Owner only through
/// STARBASE_OIDC_OWNER_GROUP, checked again at every sign-in (the last owner always stays
/// one). On a new server the owner is still made with the setup code, unless an owner group
/// is set and this person is in it.
pub fn account(store: &Store, cfg: &Config, who: &Identity, link: Option<&str>) -> Result<User, &'static str> {
    let owner_member = cfg.owner_group.as_deref().is_some_and(|g| in_group(&who.groups, g));
    if let Some(g) = &cfg.allowed_group
        && !owner_member
        && !in_group(&who.groups, g)
    {
        // Taken out of the allowed group: their sessions made through single sign-on end now.
        if let Some(u) = auth::linked(store, &who.issuer, &who.subject) {
            auth::end_sso_sessions(store, &u.username);
        }
        return Err("not-allowed");
    }
    let user = if let Some(username) = link {
        if auth::linked(store, &who.issuer, &who.subject).is_some_and(|u| u.username != username) {
            return Err("linked-elsewhere");
        }
        auth::link(store, username, &who.issuer, &who.subject).map_err(|_| "failed")?
    } else if let Some(u) = auth::linked(store, &who.issuer, &who.subject) {
        u
    } else if !auth::any_users(store) {
        if !owner_member {
            return Err("setup-first");
        }
        let u = auth::add_linked(store, &who.username, &who.name, &who.issuer, &who.subject, true).map_err(|_| "failed")?;
        // Files already in the folder become the new owner's (as with the setup code).
        crate::spaces::migrate(store).map_err(|_| "failed")?;
        u
    } else if !cfg.makes_accounts(store) {
        return Err("no-account");
    } else {
        let u = auth::add_linked(store, &who.username, &who.name, &who.issuer, &who.subject, owner_member).map_err(|_| "failed")?;
        let _ = crate::spaces::ensure_space(store, &u.username);
        u
    };
    // What the provider calls them, for People (never used to find the account).
    let _ = auth::note_link_names(store, &user.username, &who.issuer, &who.subject, &who.email, &who.username);
    if cfg.owner_group.is_none() {
        return Ok(user);
    }
    let was_owner = user.owner;
    let user = auth::set_owner_by_group(store, &user.username, owner_member).map_err(|_| "failed")?;
    // Left the owner group: their other sessions end too (this sign-in makes a new one).
    if was_owner && !user.owner {
        auth::end_all_sessions(store, &user.username);
    }
    Ok(user)
}

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod tests;
