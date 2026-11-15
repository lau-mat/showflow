use std::{env, sync::Arc};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    async_trait,
    extract::{FromRequestParts, State},
    http::{header, request::Parts, HeaderValue, StatusCode},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
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

const ACCESS_SECONDS: i64 = 15 * 60;
const REFRESH_DAYS: i64 = 30;
const ISSUER: &str = "showflow";
const AUDIENCE: &str = "showflow-api";

pub struct AuthConfig {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
    dummy_password_hash: String,
}

impl AuthConfig {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(&env::var("AUTH_JWT_SECRET")?)
    }

    fn new(secret: &str) -> Result<Self, Box<dyn std::error::Error>> {
        if secret.len() < 32 {
            return Err("AUTH_JWT_SECRET must contain at least 32 bytes".into());
        }
        Ok(Self {
            encoding_key: EncodingKey::from_secret(secret.as_bytes()),
            decoding_key: DecodingKey::from_secret(secret.as_bytes()),
            dummy_password_hash: hash_password(&Uuid::new_v4().to_string())
                .map_err(|_| "Failed to initialize password verification")?,
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
struct AppState {
    pool: PgPool,
    auth: Arc<AuthConfig>,
}

pub fn router(pool: PgPool, auth: AuthConfig) -> Router {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .layer(middleware::map_response(no_store))
        .with_state(AppState {
            pool,
            auth: Arc::new(auth),
        })
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
enum AuthError {
    BadRequest(&'static str),
    Unauthorized,
    Conflict,
    Internal,
}

impl AuthError {
    fn internal(error: impl std::fmt::Display) -> Self {
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
struct AuthenticatedUser(Uuid);

#[async_trait]
impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
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
        Ok(Self(state.auth.verify(token)?.sub))
    }
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

#[derive(Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    refresh_token: String,
    refresh_expires_at: DateTime<Utc>,
}

#[derive(FromRow)]
struct LoginUser {
    user_id: Uuid,
    user_pass: String,
    user_active: Option<bool>,
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

fn new_refresh_token() -> Result<String, AuthError> {
    let mut bytes = [0; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(AuthError::internal)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn validate_refresh_token(token: &str) -> Result<(), AuthError> {
    if token.len() != 43 || URL_SAFE_NO_PAD.decode(token).map(|v| v.len()) != Ok(32) {
        return Err(AuthError::Unauthorized);
    }
    Ok(())
}

async fn issue_session(
    connection: &mut PgConnection,
    auth: &AuthConfig,
    user_id: Uuid,
) -> Result<TokenResponse, AuthError> {
    let mut response = TokenResponse {
        access_token: auth.access_token(user_id)?,
        token_type: "Bearer",
        expires_in: ACCESS_SECONDS,
        refresh_token: new_refresh_token()?,
        refresh_expires_at: Utc::now() + Duration::days(REFRESH_DAYS),
    };
    response.refresh_expires_at = sqlx::query_scalar(
        "INSERT INTO auth_sessions (user_id, refresh_token_hash, expires_at)
         VALUES ($1, $2, $3) RETURNING expires_at",
    )
    .bind(user_id)
    .bind(token_hash(&response.refresh_token))
    .bind(response.refresh_expires_at)
    .fetch_one(connection)
    .await
    .map_err(AuthError::internal)?;
    Ok(response)
}

async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AuthError> {
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
    .bind(email)
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
    let response = issue_session(&mut tx, &state.auth, user_id).await?;
    tx.commit().await.map_err(AuthError::internal)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, AuthError> {
    let email = normalize_email(&request.email)?;
    if request.password.is_empty() || request.password.len() > 1024 {
        return Err(AuthError::Unauthorized);
    }
    let user = sqlx::query_as::<_, LoginUser>(
        "SELECT user_id, user_pass, user_active FROM users WHERE lower(user_email) = $1",
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
            .bind(user.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(AuthError::internal)?
            .flatten();
    if active != Some(true) {
        return Err(AuthError::Unauthorized);
    }
    sqlx::query("UPDATE users SET user_last_login = CURRENT_TIMESTAMP WHERE user_id = $1")
        .bind(user.user_id)
        .execute(&mut *tx)
        .await
        .map_err(AuthError::internal)?;
    let response = issue_session(&mut tx, &state.auth, user.user_id).await?;
    tx.commit().await.map_err(AuthError::internal)?;
    Ok(Json(response))
}

#[derive(FromRow)]
struct RefreshSession {
    session_id: Uuid,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
}

async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Result<Json<TokenResponse>, AuthError> {
    validate_refresh_token(&request.refresh_token)?;
    let mut tx = state.pool.begin().await.map_err(AuthError::internal)?;
    // Locking the session makes concurrent refreshes single-use.
    let session = sqlx::query_as::<_, RefreshSession>(
        "SELECT s.session_id, s.user_id, s.expires_at
         FROM auth_sessions s JOIN users u ON u.user_id = s.user_id
         WHERE s.refresh_token_hash = $1 AND s.expires_at > CURRENT_TIMESTAMP
           AND u.user_active IS TRUE
         FOR UPDATE OF s",
    )
    .bind(token_hash(&request.refresh_token))
    .fetch_optional(&mut *tx)
    .await
    .map_err(AuthError::internal)?
    .ok_or(AuthError::Unauthorized)?;
    let response = TokenResponse {
        access_token: state.auth.access_token(session.user_id)?,
        token_type: "Bearer",
        expires_in: ACCESS_SECONDS,
        refresh_token: new_refresh_token()?,
        // Rotation does not extend the session's absolute lifetime.
        refresh_expires_at: session.expires_at,
    };
    sqlx::query("UPDATE auth_sessions SET refresh_token_hash = $1 WHERE session_id = $2")
        .bind(token_hash(&response.refresh_token))
        .bind(session.session_id)
        .execute(&mut *tx)
        .await
        .map_err(AuthError::internal)?;
    tx.commit().await.map_err(AuthError::internal)?;
    Ok(Json(response))
}

async fn logout(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Result<StatusCode, AuthError> {
    validate_refresh_token(&request.refresh_token)?;
    sqlx::query("DELETE FROM auth_sessions WHERE refresh_token_hash = $1")
        .bind(token_hash(&request.refresh_token))
        .execute(&state.pool)
        .await
        .map_err(AuthError::internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize)]
struct MeResponse {
    user_id: Uuid,
}

async fn me(AuthenticatedUser(user_id): AuthenticatedUser) -> Json<MeResponse> {
    Json(MeResponse { user_id })
}

#[cfg(test)]
mod tests;
