//! Storage for single-use email-verification and password-reset tokens (`user_tokens`).
//!
//! Nothing issues these yet. Adding the endpoints later means: call `create_user_token`, mail
//! the raw token through `crate::mail::Mailer`, and call `consume_user_token` from the
//! `/auth/verify-email` or `/auth/password-reset` handler. Only hashes are stored.
#![allow(dead_code)]

use chrono::{Duration, Utc};
use sqlx::PgConnection;
use uuid::Uuid;

use super::{new_refresh_token, token_hash, validate_refresh_token, AuthError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenPurpose {
    EmailVerify,
    PasswordReset,
}

impl TokenPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::EmailVerify => "email_verify",
            Self::PasswordReset => "password_reset",
        }
    }
}

/// Returns the raw token (only ever shown to the user, e.g. inside an email link).
pub async fn create_user_token(
    connection: &mut PgConnection,
    user_id: Uuid,
    purpose: TokenPurpose,
    ttl: Duration,
) -> Result<String, AuthError> {
    let raw = new_refresh_token()?;
    sqlx::query(
        "INSERT INTO user_tokens (user_id, purpose, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(purpose.as_str())
    .bind(token_hash(&raw))
    .bind(Utc::now() + ttl)
    .execute(connection)
    .await
    .map_err(AuthError::internal)?;
    Ok(raw)
}

/// Atomically marks the token used and returns its user; unknown, used, expired or
/// wrong-purpose tokens all look the same (`Unauthorized`).
pub async fn consume_user_token(
    connection: &mut PgConnection,
    purpose: TokenPurpose,
    raw: &str,
) -> Result<Uuid, AuthError> {
    validate_refresh_token(raw)?;
    sqlx::query_scalar(
        "UPDATE user_tokens SET used_at = CURRENT_TIMESTAMP
         WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > CURRENT_TIMESTAMP
         RETURNING user_id",
    )
    .bind(token_hash(raw))
    .bind(purpose.as_str())
    .fetch_optional(connection)
    .await
    .map_err(AuthError::internal)?
    .ok_or(AuthError::Unauthorized)
}
