//! Which client is talking: platform, device id and app version, from the `X-ShowFlow-*`
//! headers the frontend attaches to every request. These are client-asserted, so they drive
//! device bookkeeping and session policy (advisory licensing data) but never authorization.

use std::convert::Infallible;

use axum::{async_trait, extract::FromRequestParts, http::{header::HeaderMap, request::Parts}};
use sqlx::PgConnection;
use uuid::Uuid;

use super::AuthError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Web,
    Desktop,
    Mobile,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
        }
    }

    /// Accepts `web`, `desktop/linux`, `mobile/android`, ...: only the kind matters.
    fn parse(value: &str) -> Option<Self> {
        match value.split('/').next()?.trim().to_ascii_lowercase().as_str() {
            "web" => Some(Self::Web),
            "desktop" => Some(Self::Desktop),
            "mobile" => Some(Self::Mobile),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ClientInfo {
    pub platform: Platform,
    /// False when the request did not name a valid platform (plain API clients, curl).
    pub declared: bool,
    pub device_id: Option<String>,
    pub app_version: Option<String>,
}

impl ClientInfo {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let platform = text("x-showflow-platform").and_then(Platform::parse);
        Self {
            platform: platform.unwrap_or(Platform::Web),
            declared: platform.is_some(),
            device_id: text("x-showflow-device-id")
                .filter(|id| valid_device_id(id))
                .map(str::to_owned),
            app_version: text("x-showflow-app-version")
                .filter(|v| !v.is_empty() && v.len() <= 32 && v.chars().all(|c| c.is_ascii_graphic()))
                .map(str::to_owned),
        }
    }

    /// Browsers get their refresh token as an HttpOnly cookie instead of in the JSON body. It
    /// requires the custom platform header, which a cross-site form or image request cannot
    /// set (and a cross-origin fetch would need a CORS preflight the API never grants).
    pub fn cookie_mode(&self) -> bool {
        self.declared && self.platform == Platform::Web
    }
}

fn valid_device_id(id: &str) -> bool {
    (8..=64).contains(&id.len())
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for ClientInfo {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::from_headers(&parts.headers))
    }
}

/// Registers (or refreshes) the device row for a user and returns its id. Clients that send no
/// device id get a throwaway one, so each of their logins is its own device.
pub async fn upsert_device(
    connection: &mut PgConnection,
    user_id: Uuid,
    client: &ClientInfo,
) -> Result<Uuid, AuthError> {
    let identifier = client
        .device_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    sqlx::query_scalar(
        "INSERT INTO client (user_id, client_platform, client_identifier, client_app_version)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, client_identifier) DO UPDATE SET
             client_platform = EXCLUDED.client_platform,
             client_app_version = EXCLUDED.client_app_version,
             client_last_connected = CURRENT_TIMESTAMP
         RETURNING client_id",
    )
    .bind(user_id)
    .bind(client.platform.as_str())
    .bind(identifier)
    .bind(&client.app_version)
    .fetch_one(connection)
    .await
    .map_err(AuthError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn parses_the_frontend_headers() {
        let info = ClientInfo::from_headers(&headers(&[
            ("x-showflow-platform", "desktop/linux"),
            ("x-showflow-device-id", "3f2c9a10-aaaa-bbbb-cccc-1234567890ab"),
            ("x-showflow-app-version", "1.2.3"),
        ]));
        assert_eq!(info.platform, Platform::Desktop);
        assert!(info.declared && !info.cookie_mode());
        assert_eq!(info.app_version.as_deref(), Some("1.2.3"));
        assert!(info.device_id.is_some());
    }

    #[test]
    fn only_a_declared_web_client_uses_cookies() {
        assert!(ClientInfo::from_headers(&headers(&[("x-showflow-platform", "web/web")])).cookie_mode());
        let none = ClientInfo::from_headers(&HeaderMap::new());
        assert_eq!(none.platform, Platform::Web);
        assert!(!none.declared && !none.cookie_mode());
        let bogus = ClientInfo::from_headers(&headers(&[("x-showflow-platform", "toaster")]));
        assert!(!bogus.declared && !bogus.cookie_mode());
    }

    #[test]
    fn rejects_malformed_device_ids_and_versions() {
        for id in ["short", "has spaces in it!", &"a".repeat(65)] {
            assert!(!valid_device_id(id), "{id}");
        }
        let info = ClientInfo::from_headers(&headers(&[
            ("x-showflow-device-id", "bad id with spaces"),
            ("x-showflow-app-version", &"9".repeat(40)),
        ]));
        assert!(info.device_id.is_none() && info.app_version.is_none());
    }
}
