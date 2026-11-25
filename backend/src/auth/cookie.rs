//! The web refresh-token cookie. Built by hand: it is one header, not worth a dependency.

use axum::http::{header, HeaderMap};

pub const NAME: &str = "showflow_refresh";

/// `path` is where the browser sees the API (`/api/auth` behind the gateway and the Vite proxy).
pub fn set(token: &str, max_age_seconds: i64, path: &str, secure: bool) -> String {
    format!(
        "{NAME}={token}; HttpOnly; SameSite=Strict; Path={path}; Max-Age={}{}",
        max_age_seconds.max(0),
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear(path: &str, secure: bool) -> String {
    set("", 0, path, secure)
}

pub fn read(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, value)| *name == NAME && !value.is_empty())
        .map(|(_, value)| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn cookie_is_httponly_strict_and_scoped() {
        let cookie = set("tok", 120, "/api/auth", true);
        assert_eq!(
            cookie,
            "showflow_refresh=tok; HttpOnly; SameSite=Strict; Path=/api/auth; Max-Age=120; Secure"
        );
        assert!(!set("tok", 1, "/api/auth", false).contains("Secure"));
        assert!(clear("/api/auth", true).contains("Max-Age=0"));
        assert!(set("tok", -5, "/", true).contains("Max-Age=0"));
    }

    #[test]
    fn reads_the_cookie_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; showflow_refresh=abc123; b=2"),
        );
        assert_eq!(read(&headers).as_deref(), Some("abc123"));
        assert_eq!(read(&HeaderMap::new()), None);
        headers.insert(header::COOKIE, HeaderValue::from_static("showflow_refresh="));
        assert_eq!(read(&headers), None);
    }
}
