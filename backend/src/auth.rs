use std::{env, sync::Arc};

pub(crate) mod client;
mod cookie;
mod policy;
mod tokens;

use client::{upsert_device, ClientInfo};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    async_trait,
    extract::{FromRequestParts, Path, State},
    http::{header, request::Parts, HeaderMap, HeaderValue, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

use crate::mail::{LogMailer, Mailer};

const ACCESS_SECONDS: i64 = 15 * 60;
const ISSUER: &str = "showflow";
const AUDIENCE: &str = "showflow-api";

pub struct AuthConfig {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    dummy_password_hash: String,
    /// Browser path of the API, used as the refresh cookie's `Path`.
    cookie_path: String,
    /// `Secure` cookies need https; `AUTH_COOKIE_INSECURE=1` allows plain-http development.
    cookie_secure: bool,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(&env::var("AUTH_JWT_SECRET")?)
    }

    pub(crate) fn new(secret: &str) -> Result<Self, Box<dyn std::error::Error>> {
        if secret.len() < 32 {
            return Err("AUTH_JWT_SECRET must contain at least 32 bytes".into());
        }
        Ok(Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            dummy_password_hash: hash_password(&Uuid::new_v4().to_string())
                .map_err(|_| "Failed to initialize password verification")?,
            cookie_path: env::var("AUTH_COOKIE_PATH").unwrap_or_else(|_| "/api/auth".to_owned()),
            cookie_secure: env::var("AUTH_COOKIE_INSECURE").as_deref() != Ok("1"),
        })
    }

    fn access_token(&self, user_id: Uuid) -> Result<String, AuthError> {
        let now = Utc::now().timestamp();
        let claims = Claims {
            sub: user_id,
            iat: now,
            exp: now + ACCESS_SECONDS,
            iss: ISSUER.to_owned(),
            aud: AUDIENCE.to_owned(),
        };
        encode(&Header::new(Algorithm::HS256), &claims, &self.encoding_key)
            .map_err(AuthError::internal)
    }

    fn verify(&self, token: &str) -> Result<Claims, AuthError> {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[AUDIENCE]);
        validation.set_required_spec_claims(&["sub", "exp", "iat", "iss", "aud"]);
        validation.leeway = 0;
        let claims = decode::<Claims>(token, &self.decoding_key, &validation)
            .map_err(|_| AuthError::Unauthorized)?
            .claims;
        if claims.exp <= Utc::now().timestamp() || claims.iat > Utc::now().timestamp() {
            return Err(AuthError::Unauthorized);
        }
        Ok(claims)
    }
}

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) pool: PgPool,
    pub(crate) auth: Arc<AuthConfig>,
    // Held for the email verification / password reset endpoints that are not built yet.
    #[allow(dead_code)]
    mailer: Arc<dyn Mailer>,
    /// Fan-out of "a show changed" announcements to the connected live streams.
    pub(crate) hub: crate::sync_stream::SyncHub,
}

pub(crate) fn state(pool: PgPool, auth: AuthConfig) -> AppState {
    AppState {
        pool,
        auth: Arc::new(auth),
        mailer: Arc::new(LogMailer),
        hub: crate::sync_stream::SyncHub::new(),
    }
}

/// The auth routes alone; `main` merges them with the other route groups under one state.
pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/auth/devices", get(devices))
        .route("/auth/devices/:client_id", delete(revoke_device))
        .route("/auth/logout-all", post(logout_all))
        .layer(middleware::map_response(no_store))
}

/// Auth routes with their own state (used by the auth tests).
#[cfg(test)]
pub fn router(pool: PgPool, auth: AuthConfig) -> Router {
    routes().with_state(state(pool, auth))
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

#[derive(Debug)]
pub(crate) enum AuthError {
    BadRequest(&'static str),
    Unauthorized,
    Conflict,
    Internal,
}

impl AuthError {
    pub(crate) fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!("Authentication operation failed: {error}");
        Self::Internal
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "Invalid credentials or token"),
            Self::Conflict => (
                StatusCode::CONFLICT,
                "Username or email is already registered",
            ),
            Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authentication unavailable",
            ),
        };
        let mut response = (status, Json(ErrorBody { error })).into_response();
        if status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

#[derive(Debug, Deserialize, Serialize)]
struct Claims {
    sub: Uuid,
    iat: i64,
    exp: i64,
    iss: String,
    aud: String,
}

// Future protected handlers can use this extractor without a database lookup.
pub(crate) struct AuthenticatedUser(pub(crate) Uuid);

#[async_trait]
impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        Ok(Self(bearer_claims(parts, state)?.sub))
    }
}

/// Like `AuthenticatedUser`, plus when the access token stops being valid. Long-lived responses
/// (the live stream) end then, so a client has to come back with a fresh token.
pub(crate) struct AuthenticatedSession {
    pub(crate) user_id: Uuid,
    pub(crate) expires_at: i64,
}

#[async_trait]
impl FromRequestParts<AppState> for AuthenticatedSession {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let claims = bearer_claims(parts, state)?;
        Ok(Self { user_id: claims.sub, expires_at: claims.exp })
    }
}

fn bearer_claims(parts: &Parts, state: &AppState) -> Result<Claims, AuthError> {
    let mut values = parts.headers.get_all(header::AUTHORIZATION).iter();
    let value = values
        .next()
        .and_then(|value| value.to_str().ok())
        .ok_or(AuthError::Unauthorized)?;
    if values.next().is_some() {
        return Err(AuthError::Unauthorized);
    }
    let (scheme, token) = value.split_once(' ').ok_or(AuthError::Unauthorized)?;
    if !scheme.eq_ignore_ascii_case("Bearer") || token.is_empty() {
        return Err(AuthError::Unauthorized);
    }
    state.auth.verify(token)
}

#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    email: String,
    password: String,
}

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

/// Apps send the refresh token in the body; browsers send it as a cookie instead.
#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Clone, Serialize, FromRow)]
struct UserView {
    #[sqlx(rename = "user_id")]
    id: Uuid,
    #[sqlx(rename = "user_name")]
    username: String,
    #[sqlx(rename = "user_email")]
    email: String,
    #[sqlx(rename = "user_email_verified")]
    email_verified: bool,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    /// Absent for browsers: theirs is in an HttpOnly cookie.
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    refresh_expires_at: DateTime<Utc>,
    user: UserView,
}

struct Issued {
    access_token: String,
    refresh_token: String,
    refresh_expires_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct LoginUser {
    user_pass: String,
    user_active: Option<bool>,
    #[sqlx(flatten)]
    view: UserView,
}

fn normalize_email(email: &str) -> Result<String, AuthError> {
    let email = email.trim().to_lowercase();
    let parts: Vec<_> = email.split('@').collect();
    if email.len() > 254
        || parts.len() != 2
        || parts.iter().any(|part| part.is_empty())
        || email.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(AuthError::BadRequest("A valid email address is required"));
    }
    Ok(email)
}

fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(AuthError::internal)
}

fn verify_password(password: &str, hash: &str) -> Result<bool, AuthError> {
    let parsed = PasswordHash::new(hash).map_err(AuthError::internal)?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(error) => Err(AuthError::internal(error)),
    }
}

pub(crate) fn new_refresh_token() -> Result<String, AuthError> {
    let mut bytes = [0; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(AuthError::internal)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn validate_refresh_token(token: &str) -> Result<(), AuthError> {
    if token.len() != 43 || URL_SAFE_NO_PAD.decode(token).map(|v| v.len()) != Ok(32) {
        return Err(AuthError::Unauthorized);
    }
    Ok(())
}

/// Browsers present the cookie, everything else the body; the other source is ignored so a
/// cross-site request that cannot set the platform header can never ride on the cookie.
fn presented_token(
    headers: &HeaderMap,
    client: &ClientInfo,
    body: Option<Json<RefreshRequest>>,
) -> Result<String, AuthError> {
    let token = if client.cookie_mode() {
        cookie::read(headers)
    } else {
        body.map(|Json(request)| request.refresh_token)
    };
    let token = token.ok_or(AuthError::Unauthorized)?;
    validate_refresh_token(&token)?;
    Ok(token)
}

async fn issue_session(
    connection: &mut PgConnection,
    auth: &AuthConfig,
    user_id: Uuid,
    client_id: Uuid,
    client: &ClientInfo,
) -> Result<Issued, AuthError> {
    let now = Utc::now();
    let absolute = now + Duration::days(policy::ABSOLUTE_DAYS);
    let refresh_token = new_refresh_token()?;
    let refresh_expires_at = sqlx::query_scalar(
        "INSERT INTO auth_sessions (user_id, client_id, refresh_token_hash, expires_at, absolute_expires_at)
         VALUES ($1, $2, $3, $4, $5) RETURNING expires_at",
    )
    .bind(user_id)
    .bind(client_id)
    .bind(token_hash(&refresh_token))
    .bind(policy::next_expiry(now, client.platform, absolute))
    .bind(absolute)
    .fetch_one(connection)
    .await
    .map_err(AuthError::internal)?;
    Ok(Issued {
        access_token: auth.access_token(user_id)?,
        refresh_token,
        refresh_expires_at,
    })
}

/// Builds the JSON response; for browsers the refresh token moves into a Set-Cookie header.
fn token_reply(
    auth: &AuthConfig,
    status: StatusCode,
    issued: Issued,
    user: UserView,
    client: &ClientInfo,
) -> Response {
    let cookie_mode = client.cookie_mode();
    let max_age = (issued.refresh_expires_at - Utc::now()).num_seconds();
    let set_cookie = cookie_mode
        .then(|| cookie::set(&issued.refresh_token, max_age, &auth.cookie_path, auth.cookie_secure));
    let body = TokenResponse {
        access_token: issued.access_token,
        token_type: "Bearer",
        expires_in: ACCESS_SECONDS,
        refresh_token: (!cookie_mode).then_some(issued.refresh_token),
        refresh_expires_at: issued.refresh_expires_at,
        user,
    };
    let mut response = (status, Json(body)).into_response();
    if let Some(value) = set_cookie.and_then(|v| HeaderValue::from_str(&v).ok()) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

fn with_cleared_cookie(mut response: Response, auth: &AuthConfig) -> Response {
    if let Ok(value) = HeaderValue::from_str(&cookie::clear(&auth.cookie_path, auth.cookie_secure)) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

async fn register(
    State(state): State<AppState>,
    client: ClientInfo,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, AuthError> {
    let username = request.username.trim();
    if username.is_empty()
        || username.chars().count() > 255
        || username.chars().any(char::is_control)
    {
        return Err(AuthError::BadRequest(
            "Username must contain 1 to 255 characters",
        ));
    }
    let email = normalize_email(&request.email)?;
    if !(12..=1024).contains(&request.password.len()) {
        return Err(AuthError::BadRequest(
            "Password must contain 12 to 1024 bytes",
        ));
    }
    let hash = tokio::task::spawn_blocking(move || hash_password(&request.password))
        .await
        .map_err(AuthError::internal)??;
    let mut tx = state.pool.begin().await.map_err(AuthError::internal)?;
    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (user_name, user_email, user_pass)
         VALUES ($1, $2, $3) RETURNING user_id",
    )
    .bind(username)
    .bind(&email)
    .bind(hash)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| {
        if error
            .as_database_error()
            .is_some_and(|db| db.is_unique_violation())
        {
            AuthError::Conflict
        } else {
            AuthError::internal(error)
        }
    })?;
    let client_id = upsert_device(&mut tx, user_id, &client).await?;
    let issued = issue_session(&mut tx, &state.auth, user_id, client_id, &client).await?;
    tx.commit().await.map_err(AuthError::internal)?;
    let user = UserView {
        id: user_id,
        username: username.to_owned(),
        email,
        email_verified: false,
    };
    Ok(token_reply(&state.auth, StatusCode::CREATED, issued, user, &client))
}

async fn login(
    State(state): State<AppState>,
    client: ClientInfo,
    Json(request): Json<LoginRequest>,
) -> Result<Response, AuthError> {
    let email = normalize_email(&request.email)?;
    if request.password.is_empty() || request.password.len() > 1024 {
        return Err(AuthError::Unauthorized);
    }
    let user = sqlx::query_as::<_, LoginUser>(
        "SELECT user_id, user_name, user_email, COALESCE(user_email_verified, FALSE) AS user_email_verified,
                user_pass, user_active
         FROM users WHERE lower(user_email) = $1",
    )
    .bind(email)
    .fetch_optional(&state.pool)
    .await
    .map_err(AuthError::internal)?;
    let hash = user
        .as_ref()
        .map(|user| user.user_pass.clone())
        .unwrap_or_else(|| state.auth.dummy_password_hash.clone());
    let valid = tokio::task::spawn_blocking(move || verify_password(&request.password, &hash))
        .await
        .map_err(AuthError::internal)??;
    let user = user
        .filter(|user| valid && user.user_active == Some(true))
        .ok_or(AuthError::Unauthorized)?;
    let mut tx = state.pool.begin().await.map_err(AuthError::internal)?;
    // Recheck account status under a lock before issuing a session.
    let active: Option<bool> =
        sqlx::query_scalar("SELECT user_active FROM users WHERE user_id = $1 FOR UPDATE")
            .bind(user.view.id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AuthError::internal)?
            .flatten();
    if active != Some(true) {
        return Err(AuthError::Unauthorized);
    }
    sqlx::query("UPDATE users SET user_last_login = CURRENT_TIMESTAMP WHERE user_id = $1")
        .bind(user.view.id)
        .execute(&mut *tx)
        .await
        .map_err(AuthError::internal)?;
    let client_id = upsert_device(&mut tx, user.view.id, &client).await?;
    let issued = issue_session(&mut tx, &state.auth, user.view.id, client_id, &client).await?;
    tx.commit().await.map_err(AuthError::internal)?;
    Ok(token_reply(&state.auth, StatusCode::OK, issued, user.view, &client))
}

#[derive(FromRow)]
struct RefreshSession {
    session_id: Uuid,
    client_id: Option<Uuid>,
    absolute_expires_at: DateTime<Utc>,
    refresh_token_hash: String,
    rotated_at: Option<DateTime<Utc>>,
    #[sqlx(flatten)]
    user: UserView,
}

async fn refresh(
    State(state): State<AppState>,
    client: ClientInfo,
    headers: HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> Response {
    match refresh_session(&state, &client, &headers, body).await {
        Ok(response) => response,
        // A dead cookie is useless: have the browser drop it so it stops being sent.
        Err(AuthError::Unauthorized) if client.cookie_mode() => {
            with_cleared_cookie(AuthError::Unauthorized.into_response(), &state.auth)
        }
        Err(error) => error.into_response(),
    }
}

async fn refresh_session(
    state: &AppState,
    client: &ClientInfo,
    headers: &HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> Result<Response, AuthError> {
    let token = presented_token(headers, client, body)?;
    let presented = token_hash(&token);
    let mut tx = state.pool.begin().await.map_err(AuthError::internal)?;
    // Locking the session serializes concurrent refreshes of the same session.
    let session = sqlx::query_as::<_, RefreshSession>(
        "SELECT s.session_id, s.client_id, s.absolute_expires_at, s.refresh_token_hash, s.rotated_at,
                u.user_id, u.user_name, u.user_email,
                COALESCE(u.user_email_verified, FALSE) AS user_email_verified
         FROM auth_sessions s JOIN users u ON u.user_id = s.user_id
         WHERE (s.refresh_token_hash = $1 OR s.previous_refresh_token_hash = $1)
           AND s.expires_at > CURRENT_TIMESTAMP AND u.user_active IS TRUE
         FOR UPDATE OF s",
    )
    .bind(&presented)
    .fetch_optional(&mut *tx)
    .await
    .map_err(AuthError::internal)?
    .ok_or(AuthError::Unauthorized)?;
    let now = Utc::now();
    if session.refresh_token_hash != presented && !policy::within_rotation_grace(session.rotated_at, now) {
        // An already-rotated token came back after the retry window: it was copied or replayed.
        // Revoke the whole session; the real owner has to log in again.
        sqlx::query("DELETE FROM auth_sessions WHERE session_id = $1")
            .bind(session.session_id)
            .execute(&mut *tx)
            .await
            .map_err(AuthError::internal)?;
        tx.commit().await.map_err(AuthError::internal)?;
        return Err(AuthError::Unauthorized);
    }
    let refresh_token = new_refresh_token()?;
    let refresh_expires_at: DateTime<Utc> = sqlx::query_scalar(
        "UPDATE auth_sessions SET previous_refresh_token_hash = refresh_token_hash,
             refresh_token_hash = $1, rotated_at = CURRENT_TIMESTAMP,
             last_used_at = CURRENT_TIMESTAMP, expires_at = $2
         WHERE session_id = $3 RETURNING expires_at",
    )
    .bind(token_hash(&refresh_token))
    .bind(policy::next_expiry(now, client.platform, session.absolute_expires_at))
    .bind(session.session_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(AuthError::internal)?;
    if let Some(client_id) = session.client_id {
        sqlx::query(
            "UPDATE client SET client_last_connected = CURRENT_TIMESTAMP,
                 client_app_version = COALESCE($2, client_app_version)
             WHERE client_id = $1",
        )
        .bind(client_id)
        .bind(&client.app_version)
        .execute(&mut *tx)
        .await
        .map_err(AuthError::internal)?;
    }
    tx.commit().await.map_err(AuthError::internal)?;
    let issued = Issued {
        access_token: state.auth.access_token(session.user.id)?,
        refresh_token,
        refresh_expires_at,
    };
    Ok(token_reply(&state.auth, StatusCode::OK, issued, session.user, client))
}

async fn logout(
    State(state): State<AppState>,
    client: ClientInfo,
    headers: HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> Result<Response, AuthError> {
    let token = presented_token(&headers, &client, body);
    // Without a usable token there is nothing to revoke, but a browser still gets its cookie cleared.
    if let Ok(token) = token {
        let hash = token_hash(&token);
        sqlx::query(
            "DELETE FROM auth_sessions WHERE refresh_token_hash = $1 OR previous_refresh_token_hash = $1",
        )
        .bind(hash)
        .execute(&state.pool)
        .await
        .map_err(AuthError::internal)?;
    } else if !client.cookie_mode() {
        return Err(AuthError::Unauthorized);
    }
    let response = StatusCode::NO_CONTENT.into_response();
    Ok(if client.cookie_mode() {
        with_cleared_cookie(response, &state.auth)
    } else {
        response
    })
}

#[derive(Serialize)]
struct MeResponse {
    user_id: Uuid,
    user: UserView,
}

async fn me(
    State(state): State<AppState>,
    AuthenticatedUser(user_id): AuthenticatedUser,
) -> Result<Json<MeResponse>, AuthError> {
    let user = sqlx::query_as::<_, UserView>(
        "SELECT user_id, user_name, user_email, COALESCE(user_email_verified, FALSE) AS user_email_verified
         FROM users WHERE user_id = $1 AND user_active IS TRUE",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(AuthError::internal)?
    .ok_or(AuthError::Unauthorized)?;
    Ok(Json(MeResponse { user_id, user }))
}

#[derive(Serialize, FromRow)]
struct DeviceView {
    id: Uuid,
    platform: String,
    app_version: Option<String>,
    last_seen: DateTime<Utc>,
    #[sqlx(skip)]
    current: bool,
    #[serde(skip)]
    identifier: String,
}

/// Devices that still hold a live session. Revoking one only affects its refresh sessions:
/// already-issued access tokens (15 minutes) keep working until they expire.
async fn devices(
    State(state): State<AppState>,
    client: ClientInfo,
    AuthenticatedUser(user_id): AuthenticatedUser,
) -> Result<Json<Vec<DeviceView>>, AuthError> {
    let mut devices = sqlx::query_as::<_, DeviceView>(
        "SELECT c.client_id AS id, c.client_platform AS platform, c.client_app_version AS app_version,
                c.client_last_connected AS last_seen, c.client_identifier AS identifier
         FROM client c
         WHERE c.user_id = $1 AND EXISTS (
             SELECT 1 FROM auth_sessions s
             WHERE s.client_id = c.client_id AND s.expires_at > CURRENT_TIMESTAMP)
         ORDER BY c.client_last_connected DESC",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(AuthError::internal)?;
    for device in &mut devices {
        device.current = client.device_id.as_deref() == Some(device.identifier.as_str());
    }
    Ok(Json(devices))
}

async fn revoke_device(
    State(state): State<AppState>,
    AuthenticatedUser(user_id): AuthenticatedUser,
    Path(client_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("DELETE FROM auth_sessions WHERE client_id = $1 AND user_id = $2")
        .bind(client_id)
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(AuthError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn logout_all(
    State(state): State<AppState>,
    client: ClientInfo,
    AuthenticatedUser(user_id): AuthenticatedUser,
) -> Result<Response, AuthError> {
    sqlx::query("DELETE FROM auth_sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(AuthError::internal)?;
    let response = StatusCode::NO_CONTENT.into_response();
    Ok(if client.cookie_mode() {
        with_cleared_cookie(response, &state.auth)
    } else {
        response
    })
}

/// Housekeeping, run periodically from `main`: drops expired sessions and tokens, and devices
/// that have no session left and have not connected for a quarter.
pub async fn purge_expired(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM auth_sessions WHERE expires_at <= CURRENT_TIMESTAMP")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM user_tokens WHERE expires_at <= CURRENT_TIMESTAMP")
        .execute(pool)
        .await?;
    sqlx::query(
        "DELETE FROM client c WHERE c.client_last_connected < CURRENT_TIMESTAMP - INTERVAL '90 days'
           AND NOT EXISTS (SELECT 1 FROM auth_sessions s WHERE s.client_id = c.client_id)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
