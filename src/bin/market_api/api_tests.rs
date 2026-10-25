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
    config::{self, AuthMode, Config},
    passwords,
    market::Offer,
    market_store::{ObservationSource, save_offers},
};
use axum::{Form, extract::State as AxumState};
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
const PASSWORD: &str = "admin-test-password";
const FOREIGN: &str = "http://evil.example";
const TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------- harness

enum Mode {
    Oidc,
    /// DEFAULT_AUTH: passwords from public.admins.
    Password,
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
        Self::start_with(pool, mode, Secrets::default(), |_| {}).await
    }

    /// `adjust` changes the configuration that `mode` starts from.
    async fn start_with(
        pool: PgPool,
        mode: Mode,
        secrets: Secrets,
        adjust: impl FnOnce(&mut Config),
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let provider = match mode {
            Mode::Oidc => Some(Provider::start().await),
            _ => None,
        };
        let text = format!("[server]\npublic_url = \"{base}\"\n[auth]\nmode = \"disabled\"");
        let mut config = Config::parse(&text).unwrap();
        config.auth.mode = match mode {
            Mode::Oidc => AuthMode::Oidc,
            Mode::Password => AuthMode::Password,
            Mode::Disabled => AuthMode::Disabled,
        };
        config.auth.oidc = provider.as_ref().map(|provider| config::Oidc {
            issuer: provider.issuer.clone(),
            client_id: CLIENT_ID.into(),
            scopes: vec!["openid".into(), "email".into(), "profile".into()],
        });
        adjust(&mut config);
        let auth = Auth::new(&config, secrets, &pool).await.unwrap();
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

    async fn add_admin_with_password(&self, email: &str, role: &str, password: &str) {
        sqlx::query("INSERT INTO public.admins (email, name, role, password_hash) VALUES ($1, $1, $2, $3)")
            .bind(email)
            .bind(role)
            .bind(passwords::hash(password))
            .execute(&self.pool)
            .await
            .unwrap();
    }

    async fn password_login(&self, browser: &mut Browser, username: &str, password: &str) -> u16 {
        let body = json!({"username": username, "password": password});
        self.post("/api/auth/login", browser, Some(body)).await.status
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
    let method = api.get("/api/auth/method", &mut browser).await.json();
    assert_eq!(method, json!({"method": "oidc"}));
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

// --------------------------------------------------------- password login

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn password_login_checks_the_stored_hash(pool: PgPool) {
    let api = Api::start(pool, Mode::Password).await;
    api.add_admin_with_password("viewer@aeza.test", "viewer", PASSWORD).await;
    api.add_admin_with_password("inactive@aeza.test", "viewer", PASSWORD).await;
    api.set_active("inactive@aeza.test", false).await;
    let mut browser = Browser::default();
    let method = api.get("/api/auth/method", &mut browser).await.json();
    assert_eq!(method, json!({"method": "password"}));
    let reply = api.get("/api/auth/login", &mut browser).await;
    assert_eq!(reply.location, "/", "there is no provider to redirect to");

    for (username, password) in [
        ("viewer@aeza.test", "wrong-password-of-enough-length"),
        ("nobody@aeza.test", PASSWORD),
        ("inactive@aeza.test", PASSWORD),
        // Imported for Keycloak by a migration; it has no password.
        ("analyst@aeza.local", ""),
    ] {
        assert_eq!(api.password_login(&mut browser, username, password).await, 401, "{username}");
    }
    let body = json!({"username": "viewer@aeza.test", "password": PASSWORD});
    let reply = api
        .send(reqwest::Method::POST, "/api/auth/login", &mut browser, Some(FOREIGN), Some(body))
        .await;
    assert_eq!(reply.status, 403, "a login from another site is refused");
    assert!(!browser.signed_in());

    let body = json!({"username": "Viewer@Aeza.Test", "password": PASSWORD});
    let reply = api.post("/api/auth/login", &mut browser, Some(body)).await;
    assert_eq!(reply.status, 204, "email is matched case-insensitively");
    let session = reply.cookie("aeza_session").unwrap();
    assert!(session.contains("HttpOnly") && session.contains("SameSite=Lax"), "{session}");
    let me = api.get("/api/auth/me", &mut browser).await.json();
    assert_eq!((me["email"].as_str(), me["role"].as_str()), (Some("viewer@aeza.test"), Some("viewer")));
    let (sessions, last_login): (i64, Option<DateTime<Utc>>) = sqlx::query_as(
        r#"SELECT (SELECT count(*) FROM public.admin_sessions), "lastLoginAt"
           FROM public.admins WHERE email = 'viewer@aeza.test'"#,
    )
    .fetch_one(&api.pool)
    .await
    .unwrap();
    assert_eq!(sessions, 1);
    assert!(last_login.is_some());
    assert!(api.socket(&browser, Some(&api.base)).await.is_ok());

    let mut stolen = Browser {
        cookies: browser.cookies.clone(),
    };
    let reply = api.post("/api/auth/logout", &mut browser, None).await;
    assert_eq!(reply.json()["logoutUrl"], Value::Null);
    assert!(!browser.signed_in());
    assert_eq!(api.get("/api/auth/me", &mut stolen).await.status, 401);
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn password_sessions_survive_a_restart_but_not_deactivation(pool: PgPool) {
    let api = Api::start(pool.clone(), Mode::Password).await;
    api.add_admin_with_password("viewer@aeza.test", "viewer", PASSWORD).await;
    let mut browser = Browser::default();
    assert_eq!(api.password_login(&mut browser, "viewer@aeza.test", PASSWORD).await, 204);
    drop(api);

    // A new process on the same database knows the session.
    let api = Api::start(pool, Mode::Password).await;
    assert_eq!(api.get("/api/auth/me", &mut browser).await.status, 200);
    let mut socket = api.socket(&browser, Some(&api.base)).await.unwrap();
    next_message(&mut socket).await.unwrap();
    api.set_active("viewer@aeza.test", false).await;
    assert_eq!(api.get("/api/auth/me", &mut browser).await.status, 401);
    while next_message(&mut socket).await.is_some() {}
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn the_default_user_is_created_only_while_no_admin_has_a_password(pool: PgPool) {
    let start = |pool, password: &str| {
        let secrets = Secrets {
            default_user_password: Some(password.into()),
            ..Secrets::default()
        };
        Api::start_with(pool, Mode::Password, secrets, |config| {
            config.auth.default_user = config::DefaultUser {
                enabled: true,
                login: "admin".into(),
            };
        })
    };
    let api = start(pool.clone(), PASSWORD).await;
    let mut browser = Browser::default();
    assert_eq!(api.password_login(&mut browser, "admin", PASSWORD).await, 204);
    assert_eq!(api.get("/api/auth/me", &mut browser).await.json()["role"], "superadmin");
    drop(api);

    // The admin exists now: another LOCAL_ADMIN_PASSWORD changes nothing.
    let api = start(pool, "another-password-of-length").await;
    let mut browser = Browser::default();
    assert_eq!(api.password_login(&mut browser, "admin", "another-password-of-length").await, 401);
    assert_eq!(api.password_login(&mut browser, "admin", PASSWORD).await, 204);
    let admins: i64 = sqlx::query_scalar("SELECT count(*) FROM public.admins WHERE email = 'admin'")
        .fetch_one(&api.pool)
        .await
        .unwrap();
    assert_eq!(admins, 1);
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
        "/api/market/prices/changes",
        "/api/market/prices/sources",
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

// ------------------------------------------------------------ price impact

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn price_impact_routes_answer_on_an_empty_database(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    let mut browser = Browser::default();
    for path in ["/api/client/price-impact", "/api/client/price-impact-demo"] {
        let reply = api.get(path, &mut browser).await;
        assert_eq!(reply.status, 200, "{path}: {}", reply.body);
        assert_eq!(reply.json(), json!([]), "{path}");
    }
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn a_real_customers_price_increase_is_reported(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    api.load_fixture().await;
    // The fixture uses example.test addresses; one customer becomes a real one.
    sqlx::raw_sql(
        "UPDATE public.accounts SET email = 'real@customer.test' WHERE id = 1;
         UPDATE public.services SET price = price * 2
          WHERE id IN (SELECT id FROM public.services WHERE price > 0 AND \"ownerId\" IN (1, 2)
                       ORDER BY \"ownerId\" LIMIT 2);",
    )
    .execute(&api.pool)
    .await
    .unwrap();
    let (service, owner_price): (i64, i64) = sqlx::query_as(
        "SELECT id::bigint, price::bigint FROM public.services WHERE \"ownerId\" = 1 AND price > 0 ORDER BY id LIMIT 1",
    )
    .fetch_one(&api.pool)
    .await
    .unwrap();

    let mut browser = Browser::default();
    let rows = api.get("/api/client/price-impact", &mut browser).await.json();
    let rows = rows.as_array().unwrap();
    assert!(!rows.is_empty(), "the increase is in operation_logs");
    assert!(rows.iter().all(|row| row["accountId"] == "1"), "example.test customers are left out: {rows:?}");
    let row = rows.iter().find(|row| row["serviceId"] == service.to_string()).unwrap();
    assert_eq!(row["currentPriceCents"], owner_price);
    assert_eq!(row["previousPriceCents"].as_i64().unwrap() * 2, owner_price);
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn scenario_events_are_served_as_stored(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    api.load_fixture().await;
    sqlx::raw_sql(
        "INSERT INTO public.price_demo_events (event_id, account_id, service_id, control_account_id, changed_at, event)
         SELECT 'scenario-1', s.\"ownerId\", s.id, 2, now() - interval '75 days',
                jsonb_build_object('eventId', 'scenario-1', 'accountId', s.\"ownerId\"::text)
         FROM public.services s WHERE s.\"ownerId\" = 1 ORDER BY s.id LIMIT 1",
    )
    .execute(&api.pool)
    .await
    .unwrap();
    let mut browser = Browser::default();
    let events = api.get("/api/client/price-impact-demo", &mut browser).await.json();
    assert_eq!(events, json!([{"eventId": "scenario-1", "accountId": "1"}]));
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn the_price_risk_weight_is_part_of_the_settings(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    api.load_fixture().await;
    let mut browser = Browser::default();
    let mut socket = api.socket(&browser, None).await.unwrap();
    let snapshot = next_message(&mut socket).await.unwrap();
    assert_eq!(snapshot["settings"]["weights"]["priceRisk"], 0.0, "off by default");
    let mut settings = snapshot["settings"].clone();
    let risks = api.post("/api/client/snapshot", &mut browser, Some(settings.clone())).await.json();
    let factors = risks[0]["factors"].as_array().unwrap();
    assert!(factors.iter().any(|factor| factor["key"] == "priceRisk" && factor["available"] == false));

    settings["weights"]["priceRisk"] = json!(20);
    let update = json!({"type": "updateSettings", "requestId": "p1", "settings": settings});
    socket.send(tungstenite::Message::Text(update.to_string().into())).await.unwrap();
    assert_eq!(reply_to(&mut socket, "p1").await["type"], "ack");
    let stored: f64 = sqlx::query_scalar("SELECT (settings->'weights'->>'priceRisk')::float8 FROM public.client_settings WHERE id")
        .fetch_one(&api.pool)
        .await
        .unwrap();
    assert_eq!(stored, 20.0);
    settings["weights"]["priceRisk"] = json!(101);
    let reply = api.post("/api/client/snapshot", &mut browser, Some(settings)).await;
    assert_eq!(reply.status, 400, "weights are 0–100");
}

// ------------------------------------------------------- market monitoring

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn competitor_price_changes_and_source_status_are_reported(pool: PgPool) {
    let api = Api::start(pool, Mode::Disabled).await;
    let observation = |days_ago: i64| ObservationSource {
        kind: "live",
        observed_at: Utc::now() - chrono::Duration::days(days_ago),
        source_url: "https://hetzner.example/cloud".into(),
        archive_url: None,
        cdx_digest: None,
        parser_version: "test",
    };
    let priced = |plan: &str, minor: i64| Offer {
        price_minor: Some(minor),
        ..offer(plan)
    };
    // CX22 goes up by 10% within the last 90 days; CX32 changed 120 days ago.
    for (days_ago, offers) in [
        (130, vec![priced("CX32", 80_000)]),
        (120, vec![priced("CX32", 90_000)]),
        (10, vec![priced("CX22", 50_000)]),
        (1, vec![priced("CX22", 55_000)]),
    ] {
        save_offers(&api.pool, "hetzner", &observation(days_ago), &offers)
            .await
            .unwrap();
    }

    let mut browser = Browser::default();
    let changes = api.get("/api/market/prices/changes", &mut browser).await.json();
    let changes = changes.as_array().unwrap();
    assert_eq!(changes.len(), 1, "only the recent change: {changes:?}");
    let change = &changes[0];
    assert_eq!(change["providerCode"], "hetzner");
    assert_eq!(change["planKey"], "CX22");
    assert_eq!((change["previousPriceMinor"].as_i64(), change["currentPriceMinor"].as_i64()), (Some(50_000), Some(55_000)));
    assert!((change["changePercent"].as_f64().unwrap() - 10.0).abs() < 1e-9);
    assert_eq!(change["sourceUrl"], "https://hetzner.example/cloud");

    let sources = api.get("/api/market/prices/sources", &mut browser).await.json();
    let sources = sources.as_array().unwrap();
    let source = |code: &str| sources.iter().find(|source| source["providerCode"] == code).unwrap();
    assert_eq!(source("hetzner")["completeObservations"], 4);
    assert!(source("hetzner")["lastCompleteAt"].is_string());
    assert_eq!(source("selectel")["completeObservations"], 0);
    assert!(source("selectel")["lastRetrievedAt"].is_null());
    assert!(source("selectel")["tariffPageUrls"].as_array().is_some_and(|urls| !urls.is_empty()));
}

// ------------------------------------------------------------------- v1

/// The fixture as uploaded through the API. Its @example.test addresses are
/// real customers to v1: only negative IDs mark synthetic rows.
fn real_snapshot() -> Value {
    serde_json::from_str(include_str!("../../../scripts/fixtures/data.json")).unwrap()
}

async fn v1_login(api: &Api, email: &str) -> Browser {
    let mut browser = Browser::default();
    let body = json!({"username": email, "password": PASSWORD});
    let reply = api.post("/api/v1/auth/login", &mut browser, Some(body)).await;
    assert_eq!(reply.status, 204, "{email}: {}", reply.body);
    browser
}

async fn put_snapshot(api: &Api, browser: &mut Browser, snapshot: Value) -> Reply {
    let origin = api.base.clone();
    api.send(reqwest::Method::PUT, "/api/v1/source/snapshot", browser, Some(&origin), Some(snapshot))
        .await
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn v1_session_endpoints_follow_the_login(pool: PgPool) {
    let api = Api::start(pool, Mode::Password).await;
    api.add_admin_with_password("viewer@aeza.test", "viewer", PASSWORD).await;
    let mut anonymous = Browser::default();
    for path in ["/api/v1/auth/me", "/api/v1/customers", "/api/v1/analytics/summary", "/api/v1/model/settings"] {
        assert_eq!(api.get(path, &mut anonymous).await.status, 401, "{path}");
    }
    assert_eq!(put_snapshot(&api, &mut anonymous, real_snapshot()).await.status, 401);

    let mut browser = v1_login(&api, "viewer@aeza.test").await;
    let me = api.get("/api/v1/auth/me", &mut browser).await.json();
    assert_eq!((me["email"].as_str(), me["role"].as_str()), (Some("viewer@aeza.test"), Some("viewer")));
    let mut stolen = Browser { cookies: browser.cookies.clone() };
    assert_eq!(api.post("/api/v1/auth/logout", &mut browser, None).await.status, 200);
    assert_eq!(api.get("/api/v1/auth/me", &mut stolen).await.status, 401);

    let wrong = json!({"username": "viewer@aeza.test", "password": "wrong-password-of-enough-length"});
    assert_eq!(api.post("/api/v1/auth/login", &mut Browser::default(), Some(wrong)).await.status, 401);
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn v1_snapshot_upload_drives_customers_summary_and_metrics(pool: PgPool) {
    let api = Api::start(pool, Mode::Password).await;
    api.add_admin_with_password("viewer@aeza.test", "viewer", PASSWORD).await;
    api.add_admin_with_password("operator@aeza.test", "operator", PASSWORD).await;
    let mut viewer = v1_login(&api, "viewer@aeza.test").await;
    let mut operator = v1_login(&api, "operator@aeza.test").await;

    assert_eq!(put_snapshot(&api, &mut viewer, real_snapshot()).await.status, 403, "viewers only read");
    let reply = put_snapshot(&api, &mut operator, real_snapshot()).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    assert_eq!(reply.json(), json!({"accounts": 10, "services": 30, "transactions": 100, "rejected": 0}));

    // Customers: every visible one has a risk, sorting, filtering and paging work.
    let list = api.get("/api/v1/customers?limit=500", &mut viewer).await.json();
    assert_eq!(list["total"], 10);
    let scores = |list: &Value| list["items"].as_array().unwrap().iter().map(|item| item["risk"]["score"].as_i64().unwrap()).collect::<Vec<_>>();
    let descending = api.get("/api/v1/customers?sort=score&direction=desc&limit=500", &mut viewer).await.json();
    assert!(scores(&descending).windows(2).all(|pair| pair[0] >= pair[1]), "{:?}", scores(&descending));
    let page = api.get("/api/v1/customers?sort=id&limit=3&page=2", &mut viewer).await.json();
    let ids: Vec<i64> = page["items"].as_array().unwrap().iter().map(|item| item["account"]["id"].as_i64().unwrap()).collect();
    assert_eq!((ids, page["total"].as_i64()), (vec![4, 5, 6], Some(10)));
    let level = descending["items"][0]["risk"]["level"].as_str().unwrap().to_owned();
    let filtered = api.get(&format!("/api/v1/customers?risk={level}&limit=500"), &mut viewer).await.json();
    assert!(filtered["items"].as_array().unwrap().iter().all(|item| item["risk"]["level"] == level.as_str()));
    let found = api.get("/api/v1/customers?search=user-0007", &mut viewer).await.json();
    assert_eq!(found["items"][0]["account"]["email"], "user-0007@example.test");
    for invalid in ["sort=name", "direction=up", "risk=extreme", "page=0", "limit=501"] {
        assert_eq!(api.get(&format!("/api/v1/customers?{invalid}"), &mut viewer).await.status, 400, "{invalid}");
    }

    // One customer with its services, transactions and explained factors.
    let customer = api.get("/api/v1/customers/1", &mut viewer).await.json();
    assert_eq!(customer["account"]["id"], 1);
    let services = customer["services"].as_array().unwrap();
    assert!(!services.is_empty() && services.iter().all(|service| service["ownerId"] == 1));
    assert!(customer["transactions"].is_array());
    assert!(customer["risk"]["factors"].as_array().unwrap().iter().all(|factor| factor["explanation"].is_string()));
    for missing in ["999999", "abc"] {
        assert_eq!(api.get(&format!("/api/v1/customers/{missing}"), &mut viewer).await.status, 404, "{missing}");
    }

    // The summary and the Prometheus gauges count the same customers.
    let summary = api.get("/api/v1/analytics/summary", &mut viewer).await.json();
    assert_eq!(summary["customersTotal"], 10);
    let by_risk: i64 = summary["customersByRisk"].as_object().unwrap().values().map(|count| count.as_i64().unwrap()).sum();
    assert_eq!(by_risk, 10);
    assert_eq!(summary["expectedLossHorizonDays"], 30);
    let metrics = api.get("/metrics", &mut Browser::default()).await;
    assert_eq!(metrics.status, 200);
    assert!(metrics.body.contains("aeza_customers_total 10\n"), "{}", metrics.body);
    assert!(metrics.body.contains(&format!("aeza_expected_loss_30d_eur_cents {}\n", summary["expectedLossEurCents"])));
    assert!(!metrics.body.contains("account"), "no customer IDs in labels");
    let alias = api.get("/metics", &mut Browser::default()).await;
    assert_eq!(alias.body, metrics.body, "/metics is the same endpoint");

    // A second upload replaces the snapshot instead of adding to it.
    let mut smaller = real_snapshot();
    smaller["accounts"].as_array_mut().unwrap().truncate(1);
    smaller["services"].as_array_mut().unwrap().retain(|service| service["ownerId"] == 1);
    let kept: Vec<Value> = smaller["services"].as_array().unwrap().iter().map(|service| service["id"].clone()).collect();
    smaller["transactions"].as_array_mut().unwrap().retain(|transaction| kept.contains(&transaction["serviceId"]));
    assert_eq!(put_snapshot(&api, &mut operator, smaller).await.status, 200);
    assert_eq!(api.get("/api/v1/customers", &mut viewer).await.json()["total"], 1);
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn v1_rejects_invalid_snapshots_and_keeps_the_previous_one(pool: PgPool) {
    let api = Api::start(pool, Mode::Password).await;
    api.add_admin_with_password("operator@aeza.test", "operator", PASSWORD).await;
    let mut operator = v1_login(&api, "operator@aeza.test").await;
    assert_eq!(put_snapshot(&api, &mut operator, real_snapshot()).await.status, 200);

    let mut synthetic = real_snapshot();
    synthetic["accounts"][0]["id"] = json!(-5);
    let mut orphan = real_snapshot();
    orphan["services"][0]["ownerId"] = json!(999_999);
    let mut empty = real_snapshot();
    empty["accounts"] = json!([]);
    for (name, snapshot) in [("synthetic customer", synthetic), ("unknown owner", orphan), ("no accounts", empty)] {
        assert_eq!(put_snapshot(&api, &mut operator, snapshot).await.status, 400, "{name}");
    }
    // Amounts above 10 euros are real amounts, not a constraint violation.
    let mut large = real_snapshot();
    large["transactions"][0]["amount"] = json!(250_000);
    assert_eq!(put_snapshot(&api, &mut operator, large).await.status, 200);
    let amount: i32 = sqlx::query_scalar("SELECT max(amount) FROM public.transactions")
        .fetch_one(&api.pool)
        .await
        .unwrap();
    assert_eq!(amount, 250_000);
    assert_eq!(api.get("/api/v1/customers", &mut operator).await.json()["total"], 10);
}

async fn patch_settings(api: &Api, browser: &mut Browser, settings: Value) -> Reply {
    let origin = api.base.clone();
    api.send(reqwest::Method::PATCH, "/api/v1/model/settings", browser, Some(&origin), Some(settings))
        .await
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn v1_model_settings_are_read_by_all_and_changed_by_operators(pool: PgPool) {
    let api = Api::start(pool, Mode::Password).await;
    api.add_admin_with_password("viewer@aeza.test", "viewer", PASSWORD).await;
    api.add_admin_with_password("operator@aeza.test", "operator", PASSWORD).await;
    let mut viewer = v1_login(&api, "viewer@aeza.test").await;
    let mut operator = v1_login(&api, "operator@aeza.test").await;
    let current = api.get("/api/v1/model/settings", &mut viewer).await.json();
    let version = current["metadata"]["version"].as_i64().unwrap();
    let mut settings = current["settings"].clone();
    settings["highThreshold"] = json!(70);

    let reply = patch_settings(&api, &mut viewer, settings.clone()).await;
    assert_eq!(reply.status, 403);
    let reply = patch_settings(&api, &mut operator, settings.clone()).await;
    assert_eq!(reply.status, 200, "{}", reply.body);
    let updated = reply.json();
    assert_eq!(updated["settings"]["highThreshold"], 70);
    assert_eq!(updated["metadata"]["version"], version + 1);
    assert_eq!(updated["metadata"]["updatedBy"], "operator@aeza.test");
    assert_eq!(api.get("/api/v1/model/settings", &mut viewer).await.json()["settings"]["highThreshold"], 70);

    settings["highThreshold"] = json!(settings["mediumThreshold"]);
    let reply = patch_settings(&api, &mut operator, settings).await;
    assert_eq!(reply.status, 400, "high must stay above medium");
}

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn v1_documentation_is_public(pool: PgPool) {
    let api = Api::start(pool, Mode::Password).await;
    let mut anonymous = Browser::default();
    let docs = api.get("/api/v1/docs", &mut anonymous).await;
    assert_eq!(docs.status, 200);
    assert!(docs.body.contains("SwaggerUIBundle") && docs.body.contains("/api/v1/openapi.json"));
    let spec = api.get("/api/v1/openapi.json", &mut anonymous).await.json();
    assert!(spec["paths"]["/api/v1/customers"]["get"].is_object());
}

// ----------------------------------------------------------- live updates

#[sqlx::test(migrator = "aeza_intelligence::adapter::MIGRATOR")]
async fn data_and_settings_changes_reach_open_sockets(pool: PgPool) {
    PgAdapter::from_pool(pool.clone())
        .load(&Dataset::from_path(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fixtures/data.json")).unwrap())
        .await
        .unwrap();
    let api = Api::start(pool, Mode::Disabled).await;
    let mut watcher = api.socket(&Browser::default(), None).await.unwrap();
    let snapshot = next_message(&mut watcher).await.unwrap();
    assert_eq!(snapshot["type"], "snapshot");

    // A write to the client tables: the dataset and the changed scores follow.
    sqlx::query("UPDATE public.accounts SET balance = 0 WHERE id = 1")
        .execute(&api.pool)
        .await
        .unwrap();
    let mut seen = Vec::new();
    while !(seen.contains(&"dataset".to_owned())) {
        let message = next_message(&mut watcher).await.expect("socket stays open");
        seen.push(message["type"].as_str().unwrap().to_owned());
        if message["type"] == "dataset" {
            let account = message["dataset"]["accounts"].as_array().unwrap().iter().find(|account| account["id"] == "1").unwrap().clone();
            assert_eq!(account["balanceCents"], 0);
        }
    }

    // New coefficients from another session reach this one.
    let mut editor = api.socket(&Browser::default(), None).await.unwrap();
    let mut settings = next_message(&mut editor).await.unwrap()["settings"].clone();
    settings["weights"]["paymentCoverage"] = json!(60);
    let update = json!({"type": "updateSettings", "requestId": "s1", "settings": settings});
    editor.send(tungstenite::Message::Text(update.to_string().into())).await.unwrap();
    loop {
        let message = next_message(&mut watcher).await.expect("socket stays open");
        if message["type"] == "settings" {
            assert_eq!(message["settings"]["weights"]["paymentCoverage"], 60.0);
            break;
        }
    }
}
