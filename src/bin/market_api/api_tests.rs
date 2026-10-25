//! HTTP and WebSocket tests against a real market_api on a random port.
//!
//! Each test gets a fresh database from `#[sqlx::test]` (DATABASE_URL must
//! point to a PostgreSQL server where the user may create databases). OIDC
//! logins go through [`Provider`], an in-process OpenID provider that signs
//! id_tokens with a fixed test key and checks PKCE, so no Keycloak is needed.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use aeza_intelligence::{
    Dataset,
    market::Offer,
    market_store::{ObservationSource, save_offers},
};
use axum::{Form, extract::State as AxumState};
use clap::{CommandFactory, FromArgMatches};
use futures_util::{SinkExt, StreamExt};
use openidconnect::{
    Audience, EmptyAdditionalClaims, EndUserEmail, IssuerUrl, JsonWebKeyId, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, PrivateSigningKey, StandardClaims, SubjectIdentifier,
    core::{
        CoreIdToken, CoreIdTokenClaims, CoreJsonWebKeySet, CoreJwsSigningAlgorithm,
        CoreRsaPrivateSigningKey,
    },
    url::{Url, form_urlencoded},
};
use serde_json::{Value, json};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{self, client::IntoClientRequest},
};

use super::*;

const CLIENT_ID: &str = "aeza-intelligence";
const LOCAL_PASSWORD: &str = "local-admin-test-password";
const FOREIGN: &str = "http://evil.example";
const TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- harness

#[derive(clap::Parser)]
struct Args {
    #[command(flatten)]
    auth: AuthArgs,
}

enum Mode {
    Oidc,
    Local,
    Disabled,
}

struct Api {
    base: String,
    pool: PgPool,
    http: reqwest::Client,
    provider: Option<Provider>,
    /// The server and the live refresh, stopped with the test so they do not
    /// hold connections while sqlx::test drops the database.
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Api {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// One HTTP response, read in full.
struct Reply {
    status: u16,
    location: String,
    set_cookies: Vec<String>,
    body: String,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("HTTP {}: {error}: {}", self.status, self.body))
    }

    fn cookie(&self, name: &str) -> Option<&str> {
        self.set_cookies
            .iter()
            .find(|line| line.starts_with(&format!("{name}=")))
            .map(String::as_str)
    }
}

/// The cookie jar of one browser.
#[derive(Default)]
struct Browser {
    cookies: BTreeMap<String, String>,
}

impl Browser {
    fn header(&self) -> String {
        self.cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn absorb(&mut self, set_cookies: &[String]) {
        for line in set_cookies {
            let (pair, attributes) = line.split_once(';').unwrap_or((line, ""));
            let (name, value) = pair.split_once('=').unwrap();
            if value.is_empty() || attributes.contains("Max-Age=0") {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
    }

    fn signed_in(&self) -> bool {
        self.cookies.contains_key("aeza_session")
    }
}

/// Query of the provider authorization URL that /api/auth/login redirects to.
struct Authorize {
    query: HashMap<String, String>,
}

impl Authorize {
    fn get(&self, key: &str) -> &str {
        self.query.get(key).map_or("", String::as_str)
    }
}

type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

impl Api {
    async fn start(pool: PgPool, mode: Mode) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let provider = match mode {
            Mode::Oidc => Some(Provider::start().await),
            _ => None,
        };
        let flags: Vec<String> = match mode {
            Mode::Oidc => vec![
                "--oidc-issuer".into(),
                provider.as_ref().unwrap().issuer.clone(),
                "--oidc-client-id".into(),
                CLIENT_ID.into(),
                "--public-url".into(),
                base.clone(),
            ],
            Mode::Local => vec![
                "--local-admin-user".into(),
                "admin".into(),
                "--local-admin-password".into(),
                LOCAL_PASSWORD.into(),
                "--public-url".into(),
                base.clone(),
            ],
            Mode::Disabled => vec!["--auth-disabled".into()],
        };
        // sqlx::test loads .env; the variables there must not change the test setup.
        let matches = Args::command()
            .mut_args(|arg| arg.env(None::<&'static str>))
            .try_get_matches_from(std::iter::once("market_api".to_owned()).chain(flags))
            .unwrap();
        let args = Args::from_arg_matches(&matches).unwrap();
        let auth = Auth::new(args.auth).await.unwrap();
        let (live, watcher) = Live::start(pool.clone()).await.unwrap();
        let app = router(AppState {
            pool: pool.clone(),
            live,
            auth,
        });
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            tasks: vec![watcher, server],
            base,
            pool,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
            provider,
        }
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        browser: &mut Browser,
        origin: Option<&str>,
        body: Option<Value>,
    ) -> Reply {
        let mut request = self.http.request(method, format!("{}{path}", self.base));
        if !browser.cookies.is_empty() {
            request = request.header("cookie", browser.header());
        }
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        let location = header("location");
        let set_cookies: Vec<String> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .map(|value| value.to_str().unwrap().to_owned())
            .collect();
        browser.absorb(&set_cookies);
        Reply {
            status: response.status().as_u16(),
            location,
            set_cookies,
            body: response.text().await.unwrap(),
        }
    }

    async fn get(&self, path: &str, browser: &mut Browser) -> Reply {
        self.send(reqwest::Method::GET, path, browser, None, None).await
    }

    /// A same-origin POST, as the frontend sends it.
    async fn post(&self, path: &str, browser: &mut Browser, body: Option<Value>) -> Reply {
        let origin = self.base.clone();
        self.send(reqwest::Method::POST, path, browser, Some(&origin), body)
            .await
    }

    fn provider(&self) -> &Provider {
        self.provider.as_ref().expect("started with Mode::Oidc")
    }

    async fn begin_login(&self, browser: &mut Browser, return_to: &str) -> Authorize {
        let return_to: String = form_urlencoded::byte_serialize(return_to.as_bytes()).collect();
        let reply = self
            .get(&format!("/api/auth/login?returnTo={return_to}"), browser)
            .await;
        assert_eq!(reply.status, 303, "{}", reply.body);
        let url = Url::parse(&reply.location).unwrap();
        assert!(
            reply
                .location
                .starts_with(&format!("{}/authorize?", self.provider().issuer)),
            "{}",
            reply.location
        );
        Authorize {
            query: url.query_pairs().into_owned().collect(),
        }
    }

    /// The provider authenticated `email`; the browser follows the callback.
    async fn finish_login(
        &self,
        browser: &mut Browser,
        authorize: &Authorize,
        email: &str,
        email_verified: Option<bool>,
    ) -> Reply {
        let code = self.provider().grant(Grant {
            email: email.to_owned(),
            email_verified,
            nonce: authorize.get("nonce").to_owned(),
            challenge: authorize.get("code_challenge").to_owned(),
        });
        let state = authorize.get("state").to_owned();
        self.get(&format!("/api/auth/callback?code={code}&state={state}"), browser)
            .await
    }

    /// A fresh browser signed in as `email`.
    async fn login(&self, email: &str) -> Browser {
        let mut browser = Browser::default();
        let authorize = self.begin_login(&mut browser, "/").await;
        let reply = self
            .finish_login(&mut browser, &authorize, email, Some(true))
            .await;
        assert_eq!(reply.location, "/", "login of {email} failed");
        assert!(browser.signed_in());
        browser
    }

    async fn socket(&self, browser: &Browser, origin: Option<&str>) -> Result<Socket, u16> {
        let mut request = format!("{}/api/client/ws", self.base.replacen("http", "ws", 1))
            .into_client_request()
            .unwrap();
        if let Some(origin) = origin {
            request
                .headers_mut()
                .insert("origin", origin.parse().unwrap());
        }
        if !browser.cookies.is_empty() {
            request
                .headers_mut()
                .insert("cookie", browser.header().parse().unwrap());
        }
        match tokio_tungstenite::connect_async(request).await {
            Ok((socket, _)) => Ok(socket),
            Err(tungstenite::Error::Http(response)) => Err(response.status().as_u16()),
            Err(error) => panic!("WebSocket connect failed: {error}"),
        }
    }

    async fn add_admin(&self, email: &str, role: &str) {
        sqlx::query(
            r#"INSERT INTO public.admins (email, name, role) VALUES ($1, $1, $2)"#,
        )
        .bind(email)
        .bind(role)
        .execute(&self.pool)
        .await
        .unwrap();
    }

    async fn set_active(&self, email: &str, active: bool) {
        sqlx::query(r#"UPDATE public.admins SET "isActive" = $2 WHERE email = $1"#)
            .bind(email)
            .bind(active)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn load_fixture(&self) {
        let data =
            Dataset::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fixtures/data.json"))
                .unwrap();
        PgAdapter::from_pool(self.pool.clone())
            .load(&data)
            .await
            .unwrap();
    }
}

/// Next JSON text message, or None once the server closes the socket.
async fn next_message(socket: &mut Socket) -> Option<Value> {
    loop {
        let message = tokio::time::timeout(TIMEOUT, socket.next())
            .await
            .expect("no WebSocket message within the timeout")?;
        match message {
            Ok(tungstenite::Message::Text(text)) => return Some(serde_json::from_str(&text).unwrap()),
            Ok(tungstenite::Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        }
    }
}

async fn reply_to(socket: &mut Socket, request_id: &str) -> Value {
    loop {
        let message = next_message(socket).await.expect("socket closed before the reply");
        if message["requestId"] == request_id {
            return message;
        }
    }
}

// ---------------------------------------------------------- OIDC provider

struct Grant {
    email: String,
    email_verified: Option<bool>,
    nonce: String,
    challenge: String,
}

#[derive(Clone)]
struct Provider {
    issuer: String,
    grants: Arc<std::sync::Mutex<HashMap<String, Grant>>>,
    codes: Arc<AtomicU64>,
}

#[derive(Clone)]
struct ProviderState {
    provider: Provider,
    key: Arc<CoreRsaPrivateSigningKey>,
}

impl Provider {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let provider = Self {
            issuer: format!("http://{}", listener.local_addr().unwrap()),
            grants: Default::default(),
            codes: Default::default(),
        };
        let key = CoreRsaPrivateSigningKey::from_pem(
            include_str!("../../../tests/fixtures/oidc-test-key.pem"),
            Some(JsonWebKeyId::new("test".into())),
        )
        .unwrap();
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/token", post(token))
            .with_state(ProviderState {
                provider: provider.clone(),
                key: Arc::new(key),
            });
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        provider
    }

    /// Issues an authorization code, as after a successful sign-in form.
    fn grant(&self, grant: Grant) -> String {
        let code = format!("code-{}", self.codes.fetch_add(1, Ordering::Relaxed));
        self.grants.lock().unwrap().insert(code.clone(), grant);
        code
    }
}

async fn discovery(AxumState(state): AxumState<ProviderState>) -> Json<Value> {
    let issuer = &state.provider.issuer;
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/authorize"),
        "token_endpoint": format!("{issuer}/token"),
        "jwks_uri": format!("{issuer}/jwks"),
        "end_session_endpoint": format!("{issuer}/logout"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
    }))
}

async fn jwks(AxumState(state): AxumState<ProviderState>) -> Json<CoreJsonWebKeySet> {
    Json(CoreJsonWebKeySet::new(vec![state.key.as_verification_key()]))
}

async fn token(
    AxumState(state): AxumState<ProviderState>,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    use axum::response::IntoResponse;
    let grant = state
        .provider
        .grants
        .lock()
        .unwrap()
        .remove(form.get("code").map_or("", String::as_str));
    let verifier = PkceCodeVerifier::new(form.get("code_verifier").cloned().unwrap_or_default());
    let Some(grant) = grant.filter(|grant| {
        PkceCodeChallenge::from_code_verifier_sha256(&verifier).as_str() == grant.challenge
    }) else {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
    };
    let now = Utc::now();
    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(state.provider.issuer.clone()).unwrap(),
        vec![Audience::new(CLIENT_ID.into())],
        now + chrono::Duration::minutes(5),
        now,
        StandardClaims::new(SubjectIdentifier::new(grant.email.clone()))
            .set_email(Some(EndUserEmail::new(grant.email)))
            .set_email_verified(grant.email_verified),
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(grant.nonce)));
    let id_token = CoreIdToken::new(
        claims,
        state.key.as_ref(),
        CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        None,
        None,
    )
    .unwrap();
    Json(json!({
        "access_token": "test-access-token",
        "token_type": "bearer",
        "expires_in": 300,
        "id_token": id_token,
    }))
    .into_response()
}

// ------------------------------------------------------------- OIDC login

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn anonymous_requests_are_rejected(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    let mut browser = Browser::default();
    for path in ["/api/auth/me", "/api/client/dataset", "/api/market/prices"] {
        assert_eq!(api.get(path, &mut browser).await.status, 401, "{path}");
    }
    let reply = api.post("/api/client/snapshot", &mut browser, Some(json!({}))).await;
    assert_eq!(reply.status, 401);
    assert_eq!(api.socket(&browser, None).await.err(), Some(401));
    let methods = api.get("/api/auth/methods", &mut browser).await.json();
    assert_eq!(methods, json!({"sso": true, "local": false}));
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn oidc_login_starts_a_session(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("viewer@aeza.test", "viewer").await;
    let mut browser = Browser::default();
    let authorize = api.begin_login(&mut browser, "/snapshot?account=1").await;
    assert_eq!(authorize.get("client_id"), CLIENT_ID);
    assert_eq!(authorize.get("code_challenge_method"), "S256");
    assert_eq!(
        authorize.get("redirect_uri"),
        format!("{}/api/auth/callback", api.base)
    );
    assert!(authorize.get("scope").split(' ').any(|scope| scope == "email"));
    let login_cookie = format!("aeza_login.{}", authorize.get("state"));
    assert!(browser.cookies.contains_key(&login_cookie));

    let reply = api
        .finish_login(&mut browser, &authorize, "Viewer@Aeza.Test", Some(true))
        .await;
    assert_eq!(reply.location, "/snapshot?account=1", "email is matched case-insensitively");
    let session = reply.cookie("aeza_session").unwrap();
    assert!(session.contains("HttpOnly") && session.contains("SameSite=Lax"), "{session}");
    assert!(!session.contains("Secure"), "PUBLIC_URL is http");
    assert!(!browser.cookies.contains_key(&login_cookie));

    let me = api.get("/api/auth/me", &mut browser).await.json();
    assert_eq!(me["email"], "viewer@aeza.test");
    assert_eq!(me["role"], "viewer");
    for path in ["/api/client/dataset", "/api/market/prices"] {
        assert_eq!(api.get(path, &mut browser).await.status, 200, "{path}");
    }

    // Only a hash of the cookie token is stored.
    let token = &browser.cookies["aeza_session"];
    let (by_hash, by_value): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE token_hash = sha256(convert_to($1, 'UTF8'))), \
                count(*) FILTER (WHERE token_hash = convert_to($1, 'UTF8')) \
         FROM public.admin_sessions",
    )
    .bind(token)
    .fetch_one(&api.pool)
    .await
    .unwrap();
    assert_eq!((by_hash, by_value), (1, 0));
    let last_login: Option<DateTime<Utc>> =
        sqlx::query_scalar(r#"SELECT "lastLoginAt" FROM public.admins WHERE email = $1"#)
            .bind("viewer@aeza.test")
            .fetch_one(&api.pool)
            .await
            .unwrap();
    assert!(last_login.is_some());
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn oidc_login_is_refused_without_access(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("inactive@aeza.test", "viewer").await;
    api.set_active("inactive@aeza.test", false).await;
    api.add_admin("unverified@aeza.test", "viewer").await;
    for (email, verified, expected) in [
        ("nobody@aeza.test", Some(true), "forbidden"),
        ("inactive@aeza.test", Some(true), "forbidden"),
        ("unverified@aeza.test", Some(false), "unverified"),
    ] {
        let mut browser = Browser::default();
        let authorize = api.begin_login(&mut browser, "/").await;
        let reply = api.finish_login(&mut browser, &authorize, email, verified).await;
        assert_eq!(reply.location, format!("/?authError={expected}"), "{email}");
        assert!(!browser.signed_in(), "{email}");
    }
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn oidc_callback_rejects_forged_and_failed_logins(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("viewer@aeza.test", "viewer").await;

    // The state belongs to another browser: this one has no login cookie.
    let authorize = api.begin_login(&mut Browser::default(), "/").await;
    let mut attacker = Browser::default();
    let reply = api
        .finish_login(&mut attacker, &authorize, "viewer@aeza.test", Some(true))
        .await;
    assert_eq!(reply.location, "/?authError=expired");
    assert!(!attacker.signed_in());

    let mut browser = Browser::default();
    let reply = api
        .get("/api/auth/callback?error=access_denied&state=x", &mut browser)
        .await;
    assert_eq!(reply.location, "/?authError=denied");

    // A code the provider refuses (here: a PKCE verifier that does not match).
    let authorize = api.begin_login(&mut browser, "/").await;
    let code = api.provider().grant(Grant {
        email: "viewer@aeza.test".into(),
        email_verified: Some(true),
        nonce: authorize.get("nonce").into(),
        challenge: "another-challenge".into(),
    });
    let state = authorize.get("state");
    let reply = api
        .get(&format!("/api/auth/callback?code={code}&state={state}"), &mut browser)
        .await;
    assert_eq!(reply.location, "/?authError=failed");

    // An id_token whose nonce belongs to another login.
    let authorize = api.begin_login(&mut browser, "/").await;
    let code = api.provider().grant(Grant {
        email: "viewer@aeza.test".into(),
        email_verified: Some(true),
        nonce: "another-nonce".into(),
        challenge: authorize.get("code_challenge").into(),
    });
    let state = authorize.get("state");
    let reply = api
        .get(&format!("/api/auth/callback?code={code}&state={state}"), &mut browser)
        .await;
    assert_eq!(reply.location, "/?authError=failed");
    assert!(!browser.signed_in());

    for return_to in ["//evil.example", "https://evil.example", "/api/auth/logout"] {
        let mut browser = Browser::default();
        let authorize = api.begin_login(&mut browser, return_to).await;
        let reply = api
            .finish_login(&mut browser, &authorize, "viewer@aeza.test", Some(true))
            .await;
        assert_eq!(reply.location, "/", "returnTo={return_to}");
    }
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn oidc_logins_in_two_tabs_both_finish(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("viewer@aeza.test", "viewer").await;
    let mut browser = Browser::default();
    let first = api.begin_login(&mut browser, "/snapshot").await;
    let second = api.begin_login(&mut browser, "/historical").await;
    let reply = api
        .finish_login(&mut browser, &first, "viewer@aeza.test", Some(true))
        .await;
    assert_eq!(reply.location, "/snapshot");
    let reply = api
        .finish_login(&mut browser, &second, "viewer@aeza.test", Some(true))
        .await;
    assert_eq!(reply.location, "/historical");
    // A stale callback in a browser that is signed in by now just opens the app.
    let reply = api
        .get("/api/auth/callback?code=x&state=stale", &mut browser)
        .await;
    assert_eq!(reply.location, "/");
    assert!(browser.signed_in());
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn only_operators_change_risk_settings(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("viewer@aeza.test", "viewer").await;
    api.add_admin("operator@aeza.test", "operator").await;
    let viewer = api.login("viewer@aeza.test").await;
    let operator = api.login("operator@aeza.test").await;

    assert_eq!(api.socket(&viewer, Some(FOREIGN)).await.err(), Some(403));
    let mut viewer_socket = api.socket(&viewer, Some(&api.base)).await.unwrap();
    let snapshot = next_message(&mut viewer_socket).await.unwrap();
    assert_eq!(snapshot["type"], "snapshot");
    let settings = snapshot["settings"].clone();
    let update = |id: &str| {
        tungstenite::Message::Text(
            json!({"type": "updateSettings", "requestId": id, "settings": settings})
                .to_string()
                .into(),
        )
    };

    viewer_socket.send(update("v1")).await.unwrap();
    assert_eq!(reply_to(&mut viewer_socket, "v1").await["type"], "error");

    let mut operator_socket = api.socket(&operator, Some(&api.base)).await.unwrap();
    next_message(&mut operator_socket).await.unwrap();
    operator_socket.send(update("o1")).await.unwrap();
    let ack = reply_to(&mut operator_socket, "o1").await;
    assert_eq!(ack["type"], "ack", "{ack}");
    let updated_by: String =
        sqlx::query_scalar("SELECT updated_by FROM public.client_settings WHERE id")
            .fetch_one(&api.pool)
            .await
            .unwrap();
    assert_eq!(updated_by, "operator@aeza.test");
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn logout_ends_the_session(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("viewer@aeza.test", "viewer").await;
    let mut browser = api.login("viewer@aeza.test").await;
    let mut stolen = Browser {
        cookies: browser.cookies.clone(),
    };

    let origin = Some(FOREIGN);
    let reply = api
        .send(reqwest::Method::POST, "/api/auth/logout", &mut browser, origin, None)
        .await;
    assert_eq!(reply.status, 403);

    let reply = api.post("/api/auth/logout", &mut browser, None).await;
    assert_eq!(reply.status, 200);
    let logout_url = reply.json()["logoutUrl"].as_str().unwrap().to_owned();
    assert!(logout_url.starts_with(&format!("{}/logout", api.provider().issuer)));
    assert!(!browser.signed_in(), "the session cookie is cleared");
    assert_eq!(api.get("/api/auth/me", &mut stolen).await.status, 401);
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn deactivation_and_expiry_end_sessions(pool: PgPool) {
    let api = Api::start(pool, Mode::Oidc).await;
    api.add_admin("viewer@aeza.test", "viewer").await;
    api.add_admin("expired@aeza.test", "viewer").await;
    let mut viewer = api.login("viewer@aeza.test").await;
    let mut socket = api.socket(&viewer, Some(&api.base)).await.unwrap();
    next_message(&mut socket).await.unwrap();

    api.set_active("viewer@aeza.test", false).await;
    assert_eq!(api.get("/api/auth/me", &mut viewer).await.status, 401);
    // The socket rechecks its session on every ping (PING_INTERVAL).
    while next_message(&mut socket).await.is_some() {}

    let mut expired = api.login("expired@aeza.test").await;
    sqlx::query("UPDATE public.admin_sessions SET expires_at = now() - interval '1 second'")
        .execute(&api.pool)
        .await
        .unwrap();
    assert_eq!(api.get("/api/auth/me", &mut expired).await.status, 401);
}

// ------------------------------------------------------------ local login

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn local_admin_signs_in_with_the_configured_password(pool: PgPool) {
    let api = Api::start(pool, Mode::Local).await;
    let mut browser = Browser::default();
    let methods = api.get("/api/auth/methods", &mut browser).await.json();
    assert_eq!(methods, json!({"sso": false, "local": true}));

    for (username, password) in [("admin", "wrong-password-of-enough-length"), ("root", LOCAL_PASSWORD)] {
        let body = json!({"username": username, "password": password});
        let reply = api.post("/api/auth/login", &mut browser, Some(body)).await;
        assert_eq!(reply.status, 401, "{username}");
    }
    let body = json!({"username": "admin", "password": LOCAL_PASSWORD});
    let reply = api
        .send(reqwest::Method::POST, "/api/auth/login", &mut browser, Some(FOREIGN), Some(body.clone()))
        .await;
    assert_eq!(reply.status, 403, "a login from another site is refused");
    assert!(!browser.signed_in());

    let reply = api.post("/api/auth/login", &mut browser, Some(body)).await;
    assert_eq!(reply.status, 204);
    let me = api.get("/api/auth/me", &mut browser).await.json();
    assert_eq!(me["role"], "superadmin");
    let socket = api.socket(&browser, Some(&api.base)).await;
    assert!(socket.is_ok(), "the local session also opens the WebSocket");

    let mut stolen = Browser {
        cookies: browser.cookies.clone(),
    };
    let reply = api.post("/api/auth/logout", &mut browser, None).await;
    assert_eq!(reply.json()["logoutUrl"], Value::Null);
    assert_eq!(api.get("/api/auth/me", &mut stolen).await.status, 401);
}

// ------------------------------------------------------------------- data

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn client_endpoints_serve_the_loaded_fixture(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    api.load_fixture().await;
    let mut browser = Browser::default();

    let dataset = api.get("/api/client/dataset", &mut browser).await.json();
    let counts = ["accounts", "services", "transactions"].map(|key| dataset[key].as_array().unwrap().len());
    assert_eq!(counts, [10, 30, 100]);

    let analysis = api.get("/api/client/analysis", &mut browser).await.json();
    assert_eq!(analysis["clients"].as_array().unwrap().len(), 10);

    let settings = serde_json::to_value(SnapshotSettings::default()).unwrap();
    let risks = api
        .post("/api/client/snapshot", &mut browser, Some(settings.clone()))
        .await
        .json();
    let risks = risks.as_array().unwrap();
    assert_eq!(risks.len(), 10);
    assert!(risks.iter().all(|risk| (0..=100).contains(&risk["score"].as_i64().unwrap())));
    let mut invalid = settings;
    invalid["highThreshold"] = json!(10);
    let reply = api.post("/api/client/snapshot", &mut browser, Some(invalid)).await;
    assert_eq!(reply.status, 400);

    let account = dataset["accounts"][0]["id"].as_str().unwrap().to_owned();
    let cashflow = api
        .get(&format!("/api/client/cashflow/{account}"), &mut browser)
        .await
        .json();
    let windows = cashflow["windows"].as_array().unwrap();
    let days: Vec<i64> = windows.iter().map(|w| w["days"].as_i64().unwrap()).collect();
    assert_eq!(days, [7, 30, 60, 90]);
    let outflow: Vec<i64> = windows
        .iter()
        .map(|w| w["projectedOutflowCents"].as_i64().unwrap())
        .collect();
    assert!(outflow.iter().all(|cents| *cents >= 0));
    assert!(outflow.windows(2).all(|pair| pair[0] <= pair[1]), "{outflow:?}");
    assert!(cashflow["startingBalanceCents"].is_i64());

    let history = api
        .get(&format!("/api/client/historical/{account}"), &mut browser)
        .await
        .json();
    assert!(history.is_array());
    let patterns = api.get("/api/client/historical/patterns", &mut browser).await.json();
    for pattern in patterns.as_array().unwrap() {
        let support = pattern["support"].as_u64().unwrap();
        assert!(support >= 2 && support == pattern["clientIds"].as_array().unwrap().len() as u64);
    }

    let quality = api.get("/api/client/data-quality", &mut browser).await.json();
    assert!(quality["unlinkedServiceNames"].is_array());
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn market_endpoints_answer_without_observations(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    let mut browser = Browser::default();
    for path in [
        "/api/market/search-interest",
        "/api/market/geography",
        "/api/market/prices",
        "/api/market/prices/monitor",
        "/api/market/prices/history",
        "/api/market/prices/benchmarks/history",
        "/api/market/prices/client-cohorts",
        "/api/market/prices/behavior",
        "/api/market/prices/quality",
    ] {
        let reply = api.get(path, &mut browser).await;
        assert_eq!(reply.status, 200, "{path}: {}", reply.body);
        assert!(reply.json().is_array(), "{path}");
    }
}

fn offer(plan_key: &str) -> Offer {
    Offer {
        plan_key: plan_key.into(),
        plan_name: plan_key.into(),
        cpu_cores: Some(2),
        ram_mib: Some(4096),
        disk_gib: Some(40),
        disk_type: None,
        region: Some("de".into()),
        price_minor: Some(50_000),
        currency: Some("RUB".into()),
        billing_period: Some("month".into()),
        price_kind: "official_catalog".into(),
        vat_included: None,
        raw_excerpt: String::new(),
    }
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn services_link_only_to_aeza_tariffs(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    api.load_fixture().await;
    let source = ObservationSource {
        kind: "live",
        observed_at: Utc::now(),
        source_url: "https://example.test".into(),
        archive_url: None,
        cdx_digest: None,
        parser_version: "test",
    };
    // A competitor plan with the same name as a client service must not take it.
    save_offers(&api.pool, "hetzner", &source, &[offer("service-0002")])
        .await
        .unwrap();
    save_offers(&api.pool, "aeza", &source, &[offer("service-0001"), offer("SERVICE-0003")])
        .await
        .unwrap();

    let linked: Vec<(String, String)> = sqlx::query_as(
        "SELECT s.name, p.code FROM public.services s \
         JOIN market_intelligence.tariff_products t ON t.id = s.tariff_product_id \
         JOIN market_intelligence.providers p ON p.id = t.provider_id ORDER BY s.name",
    )
    .fetch_all(&api.pool)
    .await
    .unwrap();
    assert_eq!(
        linked,
        [("service-0001".into(), "aeza".into()), ("service-0003".into(), "aeza".into())]
    );

    // The cohorts match the database, service by service.
    let mut browser = Browser::default();
    let cohorts = api
        .get("/api/market/prices/client-cohorts", &mut browser)
        .await
        .json();
    let cohorts = cohorts.as_array().unwrap();
    let total = |key: &str| cohorts.iter().map(|c| c[key].as_i64().unwrap()).sum::<i64>();
    let (services, awaiting, inactive): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE status = 'prolong_wait'), \
                count(*) FILTER (WHERE status IN ('deleted', 'blocked', 'suspended')) \
         FROM public.services WHERE tariff_product_id IS NOT NULL",
    )
    .fetch_one(&api.pool)
    .await
    .unwrap();
    assert_eq!(
        (total("linkedServices"), total("awaitingRenewal"), total("inactiveServices")),
        (services, awaiting, inactive)
    );
    for cohort in cohorts {
        let linked = cohort["linkedServices"].as_i64().unwrap();
        for key in ["awaitingRenewal", "inactiveServices", "accounts"] {
            assert!((0..=linked).contains(&cohort[key].as_i64().unwrap()), "{key}: {cohort}");
        }
    }
}
