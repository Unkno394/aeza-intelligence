//! Login and cookie sessions.
//!
//! Admins sign in either through OIDC (Authorization Code + PKCE), where the
//! provider only proves the email, or with DEFAULT_AUTH=true by a password
//! stored in public.admins. Either way access is granted to active rows of
//! public.admins, their role decides what the session may change, and the
//! session lives in public.admin_sessions.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use axum::{
    Json,
    extract::{Query, Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    middleware::Next,
    response::{AppendHeaders, IntoResponse, Redirect, Response},
};
use openidconnect::{
    AuthenticationFlow, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, PkceCodeVerifier,
    RedirectUrl, Scope, TokenResponse,
    core::{CoreClient, CoreProviderMetadata, CoreResponseType},
    url::Url,
};
use serde::{Deserialize, Serialize};
use aeza_intelligence::passwords;
use sqlx::PgPool;
use tokio::sync::Mutex;

const SESSION_COOKIE: &str = "aeza_session";
/// Binds a login in progress to the browser that started it. Each login has
/// its own `aeza_login.<state>` cookie, so logins started in several tabs do
/// not overwrite each other.
const LOGIN_COOKIE: &str = "aeza_login";
const LOGIN_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_PENDING_LOGINS: usize = 10_000;

type OidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[derive(clap::Args)]
pub struct AuthArgs {
    /// OIDC issuer; its /.well-known/openid-configuration is read at startup.
    #[arg(long, env = "OIDC_ISSUER")]
    oidc_issuer: Option<String>,
    #[arg(long, env = "OIDC_CLIENT_ID")]
    oidc_client_id: Option<String>,
    /// Omit for a public client, which relies on PKCE alone.
    #[arg(long, env = "OIDC_CLIENT_SECRET", hide_env_values = true)]
    oidc_client_secret: Option<String>,
    #[arg(long, env = "OIDC_SCOPES", default_value = "openid email profile")]
    oidc_scopes: String,
    /// Address users open in the browser, e.g. http://127.0.0.1:8080. The
    /// redirect URI registered at the provider is <PUBLIC_URL>/api/auth/callback.
    #[arg(long, env = "PUBLIC_URL")]
    public_url: Option<String>,
    #[arg(long, env = "SESSION_HOURS", default_value_t = 12)]
    session_hours: i32,
    /// Serve every request as a local superadmin. Only for development and checks.
    #[arg(long, env = "AUTH_DISABLED")]
    auth_disabled: bool,
    /// Sign in with the email and password of a public.admins row instead of OIDC.
    #[arg(long, env = "DEFAULT_AUTH")]
    default_auth: bool,
    /// With DEFAULT_AUTH: while no admin has a password, create a superadmin
    /// from LOCAL_ADMIN_USER and LOCAL_ADMIN_PASSWORD at startup.
    #[arg(long, env = "ENABLE_DEFAULT_USER")]
    enable_default_user: bool,
    #[arg(long, env = "LOCAL_ADMIN_USER")]
    local_admin_user: Option<String>,
    #[arg(long, env = "LOCAL_ADMIN_PASSWORD", hide_env_values = true)]
    local_admin_password: Option<String>,
}

#[derive(Clone, Serialize, sqlx::FromRow)]
pub struct Admin {
    pub id: i64,
    pub email: String,
    pub name: String,
    pub role: String,
}

impl Admin {
    pub fn can_edit_settings(&self) -> bool {
        matches!(self.role.as_str(), "operator" | "superadmin")
    }
}

/// Request extension added by [`require_admin`].
#[derive(Clone)]
pub struct Session {
    pub admin: Admin,
    /// None when authentication is disabled.
    token: Option<String>,
}

pub struct Auth {
    mode: Mode,
    /// PUBLIC_URL without a trailing slash; empty when authentication is disabled.
    origin: String,
    session_hours: i32,
}

enum Mode {
    Disabled,
    /// DEFAULT_AUTH: email and password of a public.admins row.
    Password,
    Oidc(Oidc),
}

struct Oidc {
    client: OidcClient,
    http: reqwest::Client,
    scopes: Vec<Scope>,
    end_session: Option<Url>,
    pending: Mutex<HashMap<String, PendingLogin>>,
}

struct PendingLogin {
    verifier: PkceCodeVerifier,
    nonce: Nonce,
    return_to: String,
    started: Instant,
}

impl Auth {
    pub async fn new(args: AuthArgs, pool: &PgPool) -> Result<Arc<Self>> {
        if args.auth_disabled {
            eprintln!("WARNING: authentication is disabled, every request acts as superadmin");
            return Ok(Arc::new(Self {
                mode: Mode::Disabled,
                origin: String::new(),
                session_hours: 0,
            }));
        }
        let required = |value: Option<String>, name: &str| {
            value
                .filter(|value| !value.is_empty())
                .with_context(|| format!("{name} is required"))
        };
        let origin = required(args.public_url, "PUBLIC_URL")?
            .trim_end_matches('/')
            .to_owned();
        anyhow::ensure!(
            (1..=720).contains(&args.session_hours),
            "SESSION_HOURS must be between 1 and 720"
        );
        if args.default_auth {
            if args.enable_default_user {
                create_default_user(pool, args.local_admin_user, args.local_admin_password).await?;
            }
            let with_password: i64 = sqlx::query_scalar(
                r#"SELECT count(*) FROM public.admins WHERE password_hash IS NOT NULL AND "isActive""#,
            )
            .fetch_one(pool)
            .await?;
            if with_password == 0 {
                eprintln!(
                    "WARNING: no active admin has a password yet; add one with \
                     scripts/auth_user.sh add or set ENABLE_DEFAULT_USER=true"
                );
            }
            eprintln!("Password login enabled (DEFAULT_AUTH)");
            return Ok(Arc::new(Self {
                mode: Mode::Password,
                origin,
                session_hours: args.session_hours,
            }));
        }
        anyhow::ensure!(
            !args.enable_default_user,
            "ENABLE_DEFAULT_USER requires DEFAULT_AUTH=true"
        );
        let issuer = args
            .oidc_issuer
            .filter(|issuer| !issuer.is_empty())
            .context("OIDC_ISSUER is required unless DEFAULT_AUTH=true (or AUTH_DISABLED=true for local checks)")?;
        let client_id = required(args.oidc_client_id, "OIDC_CLIENT_ID")?;
        // Redirects are not followed, as the openidconnect crate requires.
        let http = reqwest::ClientBuilder::new()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .build()?;
        let metadata = CoreProviderMetadata::discover_async(IssuerUrl::new(issuer.clone())?, &http)
            .await
            .with_context(|| format!("OIDC discovery failed for {issuer}"))?;
        // end_session_endpoint is not part of the core metadata type.
        let discovery: serde_json::Value = http
            .get(format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            ))
            .send()
            .await?
            .json()
            .await?;
        let end_session = discovery["end_session_endpoint"]
            .as_str()
            .map(Url::parse)
            .transpose()
            .context("invalid end_session_endpoint")?;
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(client_id),
            args.oidc_client_secret
                .filter(|secret| !secret.is_empty())
                .map(ClientSecret::new),
        )
        .set_redirect_uri(RedirectUrl::new(format!("{origin}/api/auth/callback"))?);
        eprintln!("OIDC login enabled with issuer {issuer}");
        Ok(Arc::new(Self {
            mode: Mode::Oidc(Oidc {
                client,
                http,
                scopes: args
                    .oidc_scopes
                    .split_whitespace()
                    // authorize_url always requests openid.
                    .filter(|scope| *scope != "openid")
                    .map(|scope| Scope::new(scope.to_owned()))
                    .collect(),
                end_session,
                pending: Mutex::new(HashMap::new()),
            }),
            origin,
            session_hours: args.session_hours,
        }))
    }

    /// The admin behind the request's session cookie, if the session is live.
    async fn session(&self, pool: &PgPool, headers: &HeaderMap) -> sqlx::Result<Option<Session>> {
        if matches!(self.mode, Mode::Disabled) {
            return Ok(Some(Session {
                admin: Admin {
                    id: 0,
                    email: "local".into(),
                    name: "Local (auth disabled)".into(),
                    role: "superadmin".into(),
                },
                token: None,
            }));
        }
        let Some(token) = cookie(headers, SESSION_COOKIE) else {
            return Ok(None);
        };
        Ok(find_session(pool, token).await?.map(|admin| Session {
            admin,
            token: Some(token.to_owned()),
        }))
    }

    /// Whether a session taken earlier (e.g. by a long-lived WebSocket) is
    /// still unexpired and its admin still active.
    pub async fn is_live(&self, pool: &PgPool, session: &Session) -> bool {
        match &session.token {
            None => true,
            Some(token) => matches!(find_session(pool, token).await, Ok(Some(_))),
        }
    }

    /// Cross-site WebSocket and state-changing requests must come from PUBLIC_URL.
    fn origin_allowed(&self, method: &Method, headers: &HeaderMap) -> bool {
        if matches!(self.mode, Mode::Disabled) {
            return true;
        }
        let upgrade = headers.contains_key(header::UPGRADE);
        if method == Method::GET && !upgrade {
            return true;
        }
        match headers.get(header::ORIGIN) {
            Some(origin) => origin.as_bytes() == self.origin.as_bytes(),
            // Non-browser clients send no Origin; the cookie is still required.
            None => true,
        }
    }

    fn secure(&self) -> bool {
        self.origin.starts_with("https://")
    }

    fn set_cookie(&self, name: &str, value: &str, path: &str, max_age: i64) -> String {
        format!(
            "{name}={value}; Path={path}; Max-Age={max_age}; HttpOnly; SameSite=Lax{}",
            if self.secure() { "; Secure" } else { "" }
        )
    }
}

fn login_cookie(state: &str) -> String {
    format!("{LOGIN_COOKIE}.{state}")
}

/// States issued by [`login`] are URL-safe base64; anything else cannot name a
/// login cookie and must not reach a Set-Cookie header.
fn valid_state(state: &str) -> bool {
    !state.is_empty()
        && state.len() <= 128
        && state
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Only same-site paths outside /api, so a login cannot redirect elsewhere.
fn safe_return_to(value: Option<&str>) -> String {
    match value {
        Some(path)
            if path.starts_with('/')
                && !path.starts_with("//")
                && !path.contains('\\')
                && !path.starts_with("/api/") =>
        {
            path.to_owned()
        }
        _ => "/".to_owned(),
    }
}

async fn find_session(pool: &PgPool, token: &str) -> sqlx::Result<Option<Admin>> {
    sqlx::query_as(
        "SELECT a.id, a.email, a.name, a.role FROM public.admin_sessions s \
         JOIN public.admins a ON a.id = s.admin_id \
         WHERE s.token_hash = sha256(convert_to($1, 'UTF8')) \
           AND s.expires_at > now() AND a.\"isActive\"",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
}

pub async fn require_admin(
    State(auth): State<Arc<Auth>>,
    State(pool): State<PgPool>,
    mut request: Request,
    next: Next,
) -> Response {
    if !auth.origin_allowed(request.method(), request.headers()) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "origin"})),
        )
            .into_response();
    }
    match auth.session(&pool, request.headers()).await {
        Ok(Some(session)) => {
            request.extensions_mut().insert(session);
            next.run(request).await
        }
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        )
            .into_response(),
        Err(error) => {
            eprintln!("session lookup failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub async fn me(axum::Extension(session): axum::Extension<Session>) -> Json<Admin> {
    Json(session.admin)
}

pub async fn methods(State(auth): State<Arc<Auth>>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "sso": matches!(auth.mode, Mode::Oidc(_)),
        "local": matches!(auth.mode, Mode::Password),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginQuery {
    return_to: Option<String>,
}

pub async fn login(State(auth): State<Arc<Auth>>, Query(query): Query<LoginQuery>) -> Response {
    let return_to = safe_return_to(query.return_to.as_deref());
    let Mode::Oidc(oidc) = &auth.mode else {
        return Redirect::to(&return_to).into_response();
    };
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = oidc
        .client
        .authorize_url(
            AuthenticationFlow::<CoreResponseType>::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .set_pkce_challenge(challenge);
    for scope in &oidc.scopes {
        request = request.add_scope(scope.clone());
    }
    let (url, state, nonce) = request.url();
    {
        let mut pending = oidc.pending.lock().await;
        pending.retain(|_, login| login.started.elapsed() < LOGIN_TIMEOUT);
        if pending.len() >= MAX_PENDING_LOGINS {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        pending.insert(
            state.secret().clone(),
            PendingLogin {
                verifier,
                nonce,
                return_to,
                started: Instant::now(),
            },
        );
    }
    (
        AppendHeaders([(
            header::SET_COOKIE,
            auth.set_cookie(
                &login_cookie(state.secret()),
                "1",
                "/api/auth",
                LOGIN_TIMEOUT.as_secs() as i64,
            ),
        )]),
        Redirect::to(url.as_str()),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct Credentials {
    /// The admin's email (public.admins.email), compared case-insensitively.
    username: String,
    password: String,
}

/// DEFAULT_AUTH login: an active admin whose password matches its stored hash.
pub async fn password_login(
    State(auth): State<Arc<Auth>>,
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Json(credentials): Json<Credentials>,
) -> Response {
    if !matches!(auth.mode, Mode::Password) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !auth.origin_allowed(&Method::POST, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let admin: Option<(i64, Option<String>)> = match sqlx::query_as(
        r#"SELECT id, password_hash FROM public.admins WHERE lower(email) = lower($1) AND "isActive""#,
    )
    .bind(&credentials.username)
    .fetch_optional(&pool)
    .await
    {
        Ok(admin) => admin,
        Err(error) => {
            eprintln!("admin lookup failed: {error}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let (admin_id, hash) = admin.map_or((None, None), |(id, hash)| (Some(id), hash));
    // Argon2 takes tens of milliseconds of CPU; keep it off the async workers.
    let matches =
        tokio::task::spawn_blocking(move || passwords::verify(&credentials.password, hash.as_deref()))
            .await
            .unwrap_or(false);
    let Some(admin_id) = admin_id.filter(|_| matches) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "invalid_credentials"})),
        )
            .into_response();
    };
    let session = async {
        sqlx::query(r#"UPDATE public.admins SET "lastLoginAt" = CURRENT_TIMESTAMP WHERE id = $1"#)
            .bind(admin_id)
            .execute(&pool)
            .await?;
        start_session(&pool, admin_id, auth.session_hours).await
    };
    match session.await {
        Ok(token) => (
            AppendHeaders([(
                header::SET_COOKIE,
                auth.set_cookie(SESSION_COOKIE, &token, "/", i64::from(auth.session_hours) * 3600),
            )]),
            StatusCode::NO_CONTENT,
        )
            .into_response(),
        Err(error) => {
            eprintln!("session insert failed: {error}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// A new session of `admin_id`; the returned token goes into the cookie and
/// only its SHA-256 hash into the database.
async fn start_session(pool: &PgPool, admin_id: i64, hours: i32) -> sqlx::Result<String> {
    let token = CsrfToken::new_random_len(32).secret().clone();
    sqlx::query("DELETE FROM public.admin_sessions WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO public.admin_sessions (token_hash, admin_id, expires_at) \
         VALUES (sha256(convert_to($1, 'UTF8')), $2, now() + make_interval(hours => $3))",
    )
    .bind(&token)
    .bind(admin_id)
    .bind(hours)
    .execute(pool)
    .await?;
    Ok(token)
}

/// ENABLE_DEFAULT_USER: the first superadmin, created only while no admin has
/// a password, so changing or removing it later is not undone by a restart.
async fn create_default_user(
    pool: &PgPool,
    user: Option<String>,
    password: Option<String>,
) -> Result<()> {
    let (Some(user), Some(password)) = (
        user.filter(|value| !value.is_empty()),
        password.filter(|value| !value.is_empty()),
    ) else {
        anyhow::bail!("ENABLE_DEFAULT_USER requires LOCAL_ADMIN_USER and LOCAL_ADMIN_PASSWORD");
    };
    anyhow::ensure!(
        password.chars().count() >= passwords::MIN_LENGTH,
        "LOCAL_ADMIN_PASSWORD must be at least {} characters",
        passwords::MIN_LENGTH
    );
    let has_password: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM public.admins WHERE password_hash IS NOT NULL)",
    )
    .fetch_one(pool)
    .await?;
    if has_password {
        return Ok(());
    }
    let hash = tokio::task::spawn_blocking(move || passwords::hash(&password)).await?;
    sqlx::query(
        r#"INSERT INTO public.admins (email, name, role, password_hash)
           VALUES ($1, $1, 'superadmin', $2)
           ON CONFLICT (lower(email)) DO UPDATE
             SET role = 'superadmin', "isActive" = true, password_hash = EXCLUDED.password_hash"#,
    )
    .bind(&user)
    .bind(hash)
    .execute(pool)
    .await?;
    eprintln!("Created the default superadmin {user} (ENABLE_DEFAULT_USER)");
    Ok(())
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Why a login failed, shown by the frontend login screen as ?authError=.
#[derive(Debug, Clone, Copy)]
enum LoginError {
    Denied,
    Expired,
    Failed,
    Unverified,
    Forbidden,
}

impl LoginError {
    fn code(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Failed => "failed",
            Self::Unverified => "unverified",
            Self::Forbidden => "forbidden",
        }
    }
}

pub async fn callback(
    State(auth): State<Arc<Auth>>,
    State(pool): State<PgPool>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let mut cookies = Vec::new();
    if let Some(state) = query.state.as_deref().filter(|state| valid_state(state)) {
        cookies.push((
            header::SET_COOKIE,
            auth.set_cookie(&login_cookie(state), "", "/api/auth", 0),
        ));
    }
    match finish_login(&auth, &pool, &headers, query).await {
        Ok((token, return_to)) => {
            let max_age = i64::from(auth.session_hours) * 3600;
            cookies.push((
                header::SET_COOKIE,
                auth.set_cookie(SESSION_COOKIE, &token, "/", max_age),
            ));
            (AppendHeaders(cookies), Redirect::to(&return_to)).into_response()
        }
        // A stale callback (e.g. from a tab whose login another tab already
        // finished) is harmless when this browser is signed in by now.
        Err(LoginError::Expired)
            if matches!(auth.session(&pool, &headers).await, Ok(Some(_))) =>
        {
            (AppendHeaders(cookies), Redirect::to("/")).into_response()
        }
        Err(error) => (
            AppendHeaders(cookies),
            Redirect::to(&format!("/?authError={}", error.code())),
        )
            .into_response(),
    }
}

async fn finish_login(
    auth: &Auth,
    pool: &PgPool,
    headers: &HeaderMap,
    query: CallbackQuery,
) -> Result<(String, String), LoginError> {
    let Mode::Oidc(oidc) = &auth.mode else {
        return Err(LoginError::Failed);
    };
    if let Some(error) = query.error {
        eprintln!("OIDC provider returned error: {error}");
        return Err(LoginError::Denied);
    }
    let (Some(code), Some(state)) = (query.code, query.state) else {
        return Err(LoginError::Failed);
    };
    // The state must belong to this browser, not only to some pending login.
    if !valid_state(&state) || cookie(headers, &login_cookie(&state)).is_none() {
        return Err(LoginError::Expired);
    }
    let login = oidc
        .pending
        .lock()
        .await
        .remove(&state)
        .filter(|login| login.started.elapsed() < LOGIN_TIMEOUT)
        .ok_or(LoginError::Expired)?;
    let failed = |stage: &str, error: &dyn std::fmt::Display| {
        eprintln!("OIDC login failed at {stage}: {error}");
        LoginError::Failed
    };
    let token = oidc
        .client
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|error| failed("token endpoint", &error))?
        .set_pkce_verifier(login.verifier)
        .request_async(&oidc.http)
        .await
        .map_err(|error| failed("code exchange", &error))?;
    let id_token = token
        .id_token()
        .ok_or_else(|| failed("token response", &"no id_token"))?;
    let claims = id_token
        .claims(&oidc.client.id_token_verifier(), &login.nonce)
        .map_err(|error| failed("id_token verification", &error))?;
    let email = claims
        .email()
        .ok_or_else(|| failed("id_token claims", &"no email claim"))?;
    if claims.email_verified() == Some(false) {
        return Err(LoginError::Unverified);
    }
    let admin: Option<(i64,)> = sqlx::query_as(
        "UPDATE public.admins SET \"lastLoginAt\" = CURRENT_TIMESTAMP \
         WHERE lower(email) = lower($1) AND \"isActive\" RETURNING id",
    )
    .bind(email.as_str())
    .fetch_optional(pool)
    .await
    .map_err(|error| failed("admin lookup", &error))?;
    let Some((admin_id,)) = admin else {
        eprintln!(
            "OIDC login rejected: {} is not an active admin",
            email.as_str()
        );
        return Err(LoginError::Forbidden);
    };
    let session = start_session(pool, admin_id, auth.session_hours)
        .await
        .map_err(|error| failed("session insert", &error))?;
    Ok((session, login.return_to))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogoutResponse {
    /// Provider logout page to open next, when the provider has one.
    logout_url: Option<String>,
}

pub async fn logout(
    State(auth): State<Arc<Auth>>,
    State(pool): State<PgPool>,
    headers: HeaderMap,
) -> Response {
    if matches!(auth.mode, Mode::Disabled) {
        return Json(LogoutResponse { logout_url: None }).into_response();
    }
    if !auth.origin_allowed(&Method::POST, &headers) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(token) = cookie(&headers, SESSION_COOKIE)
        && let Err(error) = sqlx::query(
            "DELETE FROM public.admin_sessions WHERE token_hash = sha256(convert_to($1, 'UTF8'))",
        )
        .bind(token)
        .execute(&pool)
        .await
    {
        eprintln!("session delete failed: {error}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let logout_url = match &auth.mode {
        Mode::Oidc(oidc) => oidc.end_session.clone().map(|mut url| {
            url.query_pairs_mut()
                .append_pair("client_id", oidc.client.client_id().as_str())
                .append_pair("post_logout_redirect_uri", &format!("{}/", auth.origin));
            url.to_string()
        }),
        _ => None,
    };
    (
        AppendHeaders([(
            header::SET_COOKIE,
            auth.set_cookie(SESSION_COOKIE, "", "/", 0),
        )]),
        Json(LogoutResponse { logout_url }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_stays_on_this_site() {
        assert_eq!(
            safe_return_to(Some("/snapshot?account=1")),
            "/snapshot?account=1"
        );
        assert_eq!(safe_return_to(Some("//evil.example")), "/");
        assert_eq!(safe_return_to(Some("https://evil.example")), "/");
        assert_eq!(safe_return_to(Some("/\\evil.example")), "/");
        assert_eq!(safe_return_to(Some("/api/auth/logout")), "/");
        assert_eq!(safe_return_to(None), "/");
    }

    #[test]
    fn reads_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.append(header::COOKIE, "a=1; aeza_session=tok".parse().unwrap());
        headers.append(header::COOKIE, "aeza_login.st=1".parse().unwrap());
        assert_eq!(cookie(&headers, SESSION_COOKIE), Some("tok"));
        assert_eq!(cookie(&headers, &login_cookie("st")), Some("1"));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn only_issued_states_name_a_login_cookie() {
        assert!(valid_state(CsrfToken::new_random().secret()));
        assert!(!valid_state(""));
        assert!(!valid_state("a; Path=/"));
        assert!(!valid_state("a\r\nSet-Cookie: x=1"));
    }

    #[test]
    fn only_operators_and_superadmins_edit_settings() {
        let admin = |role: &str| Admin {
            id: 1,
            email: "a@example.test".into(),
            name: "A".into(),
            role: role.into(),
        };
        assert!(!admin("viewer").can_edit_settings());
        assert!(admin("operator").can_edit_settings());
        assert!(admin("superadmin").can_edit_settings());
    }

    /// Auth from command-line flags only, ignoring the environment. The pool
    /// never connects: these configurations fail before the first query.
    async fn auth(flags: &[&str]) -> Result<Arc<Auth>> {
        use clap::{CommandFactory, FromArgMatches};
        #[derive(clap::Parser)]
        struct Args {
            #[command(flatten)]
            auth: AuthArgs,
        }
        let matches = Args::command()
            .mut_args(|arg| arg.env(None::<&'static str>))
            .try_get_matches_from(std::iter::once("market_api").chain(flags.iter().copied()))?;
        let pool = PgPool::connect_lazy("postgres://unused@127.0.0.1:1/unused")?;
        Auth::new(Args::from_arg_matches(&matches)?.auth, &pool).await
    }

    #[tokio::test]
    async fn refuses_to_start_without_a_complete_login_method() {
        let error = |result: Result<Arc<Auth>>| result.err().map(|error| error.to_string());
        let url = ["--public-url", "http://127.0.0.1:8080"];
        assert_eq!(error(auth(&[]).await).as_deref(), Some("PUBLIC_URL is required"));
        assert!(
            error(auth(&url).await).is_some_and(|e| e.starts_with("OIDC_ISSUER is required unless DEFAULT_AUTH")),
        );
        let default_user = [&url[..], &["--enable-default-user"]].concat();
        assert_eq!(
            error(auth(&default_user).await).as_deref(),
            Some("ENABLE_DEFAULT_USER requires DEFAULT_AUTH=true")
        );
        // Compose passes unset variables as empty strings.
        let unset = [&default_user[..], &["--default-auth", "--local-admin-user=", "--local-admin-password="]].concat();
        assert_eq!(
            error(auth(&unset).await).as_deref(),
            Some("ENABLE_DEFAULT_USER requires LOCAL_ADMIN_USER and LOCAL_ADMIN_PASSWORD")
        );
        let short = [&default_user[..], &["--default-auth", "--local-admin-user", "admin", "--local-admin-password", "short"]].concat();
        assert_eq!(
            error(auth(&short).await).as_deref(),
            Some("LOCAL_ADMIN_PASSWORD must be at least 16 characters")
        );
        assert!(auth(&["--auth-disabled"]).await.is_ok());
    }

    #[test]
    fn cookies_are_secure_behind_https() {
        let auth = |origin: &str| Auth {
            mode: Mode::Password,
            origin: origin.into(),
            session_hours: 12,
        };
        assert_eq!(
            auth("https://intel.example").set_cookie("a", "1", "/", 60),
            "a=1; Path=/; Max-Age=60; HttpOnly; SameSite=Lax; Secure"
        );
        assert_eq!(
            auth("http://127.0.0.1:8080").set_cookie("a", "1", "/", 60),
            "a=1; Path=/; Max-Age=60; HttpOnly; SameSite=Lax"
        );
    }
}
