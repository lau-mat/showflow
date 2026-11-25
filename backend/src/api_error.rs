//! Errors for the show endpoints. Same wire shape as the auth errors (`{"error": "..."}`) so the
//! frontend handles them identically.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use crate::auth::AuthError;

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    /// The caller may see the show but not do this to it.
    Forbidden(&'static str),
    /// Also used for shows the caller has no access to: their existence is not revealed.
    NotFound,
    Conflict(&'static str),
    Gone(&'static str),
    Internal,
}

impl ApiError {
    pub fn internal(error: impl std::fmt::Display) -> Self {
        eprintln!("Request failed: {error}");
        Self::Internal
    }
}

impl From<AuthError> for ApiError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::BadRequest(message) => Self::BadRequest(message.to_owned()),
            AuthError::Unauthorized => Self::Unauthorized,
            AuthError::Conflict => Self::Conflict("Already exists"),
            AuthError::Internal => Self::Internal,
        }
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        Self::internal(error)
    }
}

#[derive(Serialize)]
struct Body {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Unauthorized => return AuthError::Unauthorized.into_response(),
            Self::Forbidden(message) => (StatusCode::FORBIDDEN, message.to_owned()),
            Self::NotFound => (StatusCode::NOT_FOUND, "Not found".to_owned()),
            Self::Conflict(message) => (StatusCode::CONFLICT, message.to_owned()),
            Self::Gone(message) => (StatusCode::GONE, message.to_owned()),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong".to_owned()),
        };
        (status, Json(Body { error })).into_response()
    }
}
