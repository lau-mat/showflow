//! Caller passes (`protocol::caller_pass`): the cloud's side of account-based access to live
//! sessions. A pass proves who someone is, on which device; every member of a show may have one.
//! What it lets them do (join, call) is decided by the lists the host syncs (`GET /shows`).
//! Without `CALLER_PASS_KEY` the feature does not exist here: both endpoints answer 404.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use protocol::caller_pass::{parse_public_key, public_key_text, sign_pass, PassBody, PASS_LIFETIME_SECONDS, PASS_VERSION};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api_error::ApiError;
use crate::auth::{AppState, AuthenticatedUser};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/caller-passes/key", get(public_key))
        .route("/shows/:id/caller-pass", post(issue))
}

#[derive(Serialize)]
struct PublicKey {
    public_key: String,
}

/// What a session host needs to check passes offline.
async fn public_key(State(state): State<AppState>, AuthenticatedUser(_): AuthenticatedUser) -> Result<Json<PublicKey>, ApiError> {
    let key = state.auth.caller_key().ok_or(ApiError::NotFound)?;
    Ok(Json(PublicKey { public_key: public_key_text(key) }))
}

#[derive(Deserialize)]
struct PassRequest {
    device_key: String,
}

#[derive(Serialize)]
struct IssuedPass {
    pass: String,
    expires_at: i64,
}

async fn issue(
    State(state): State<AppState>,
    AuthenticatedUser(me): AuthenticatedUser,
    Path(show_id): Path<Uuid>,
    Json(request): Json<PassRequest>,
) -> Result<Json<IssuedPass>, ApiError> {
    let key = state.auth.caller_key().ok_or(ApiError::NotFound)?;
    if parse_public_key(&request.device_key).is_none() {
        return Err(ApiError::BadRequest("A valid device key is required".into()));
    }
    let username: String = sqlx::query_scalar(
        "SELECT u.user_name
         FROM show_members m
         JOIN users u ON u.user_id = m.user_id AND u.user_active IS TRUE
         JOIN shows s ON s.show_id = m.show_id AND s.deleted_at IS NULL
         WHERE m.show_id = $1 AND m.user_id = $2",
    )
    .bind(show_id)
    .bind(me)
    .fetch_optional(&state.pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    let issued_at = Utc::now().timestamp();
    let expires_at = issued_at + PASS_LIFETIME_SECONDS;
    let pass = sign_pass(key, &PassBody {
        v: PASS_VERSION,
        show_id: show_id.to_string(),
        user_id: me.to_string(),
        username,
        device_key: request.device_key,
        issued_at,
        expires_at,
    });
    Ok(Json(IssuedPass { pass, expires_at }))
}

#[cfg(test)]
mod tests;
