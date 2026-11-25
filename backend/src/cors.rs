//! Minimal CORS for the app webviews (`tauri://localhost` and friends), which call the cloud
//! API cross-origin. Only listed origins are answered, and credentials are never allowed:
//! apps send their tokens in the request, and the browser web app is same-origin.

use std::sync::Arc;

use axum::{
    extract::{Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::Response,
};

const DEFAULT_ORIGINS: &str = "tauri://localhost,http://tauri.localhost,https://tauri.localhost";
const ALLOWED_HEADERS: &str =
    "authorization, content-type, x-showflow-platform, x-showflow-device-id, x-showflow-app-version";

pub fn parse_origins(value: Option<&str>) -> Arc<Vec<String>> {
    Arc::new(
        value
            .unwrap_or(DEFAULT_ORIGINS)
            .split(',')
            .map(|origin| origin.trim().to_owned())
            .filter(|origin| !origin.is_empty() && origin != "*")
            .collect(),
    )
}

pub async fn layer(State(origins): State<Arc<Vec<String>>>, request: Request, next: Next) -> Response {
    let allowed = request
        .headers()
        .get(header::ORIGIN)
        .filter(|origin| origins.iter().any(|o| o.as_bytes() == origin.as_bytes()))
        .cloned();
    let preflight = request.method() == Method::OPTIONS && allowed.is_some();

    let mut response = if preflight {
        let mut response = Response::new(axum::body::Body::empty());
        *response.status_mut() = StatusCode::NO_CONTENT;
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("GET, POST, DELETE, OPTIONS"));
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static(ALLOWED_HEADERS));
        headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
        response
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.append(header::VARY, HeaderValue::from_static("Origin"));
    if let Some(origin) = allowed {
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, middleware, routing::post, Router};
    use tower::ServiceExt;

    fn app() -> Router {
        Router::new()
            .route("/auth/login", post(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(parse_origins(None), layer))
    }

    async fn send(method: Method, origin: Option<&str>) -> Response {
        let mut request = Request::builder().method(method).uri("/auth/login");
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        app().oneshot(request.body(Body::empty()).unwrap()).await.unwrap()
    }

    #[test]
    fn wildcards_are_never_accepted() {
        assert_eq!(*parse_origins(Some("*, https://a.example ,")), vec!["https://a.example"]);
        assert_eq!(parse_origins(None).len(), 3);
    }

    #[tokio::test]
    async fn preflight_from_an_app_origin_is_answered() {
        let response = send(Method::OPTIONS, Some("tauri://localhost")).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "tauri://localhost");
        assert!(response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
            .to_str().unwrap().contains("x-showflow-device-id"));
        assert!(response.headers().get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none());
    }

    #[tokio::test]
    async fn simple_requests_get_the_header_only_for_listed_origins() {
        let allowed = send(Method::POST, Some("http://tauri.localhost")).await;
        assert_eq!(allowed.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "http://tauri.localhost");
        let other = send(Method::POST, Some("https://evil.example")).await;
        assert!(other.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        let none = send(Method::POST, None).await;
        assert_eq!(none.status(), StatusCode::OK);
        assert!(none.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
        // A disallowed origin's preflight is not answered by the middleware (405 from the router).
        assert_eq!(send(Method::OPTIONS, Some("https://evil.example")).await.status(), StatusCode::METHOD_NOT_ALLOWED);
    }
}
