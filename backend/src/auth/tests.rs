use super::*;
use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Request},
};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

const SECRET: &str = "test-only-signing-key-with-at-least-32-bytes";
const PASSWORD: &str = "correct horse battery staple";

fn config() -> AuthConfig {
    AuthConfig::new(SECRET).unwrap()
}

fn app(pool: PgPool) -> Router {
    router(pool, config())
}

async fn request(app: &Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, body)
}

const WEB: &[(&str, &str)] = &[("x-showflow-platform", "web/web")];
const DESKTOP: &[(&str, &str)] = &[
    ("x-showflow-platform", "desktop/linux"),
    ("x-showflow-device-id", "desktop-device-0001"),
    ("x-showflow-app-version", "1.0.0"),
];

/// Like `request`, with custom headers and the response headers returned.
async fn send(
    app: &Router,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (StatusCode, HeaderMap, Value) {
    let mut req = Request::builder().method(method).uri(path);
    for (name, value) in headers {
        req = req.header(*name, *value);
    }
    let body = match body {
        Some(body) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(req.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let response_headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let json = if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() };
    (status, response_headers, json)
}

fn cookie_of(headers: &HeaderMap) -> String {
    let set = headers.get(header::SET_COOKIE).expect("Set-Cookie").to_str().unwrap();
    set.split(';').next().unwrap().to_owned() // "showflow_refresh=<token>"
}

fn with_bearer<'a>(token: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, String)> {
    let mut headers = vec![("authorization", format!("Bearer {token}"))];
    headers.extend(extra.iter().map(|(k, v)| (*k, (*v).to_owned())));
    headers
}

async fn authed(app: &Router, method: &str, path: &str, token: &str, extra: &[(&str, &str)]) -> (StatusCode, HeaderMap, Value) {
    let owned = with_bearer(token, extra);
    let borrowed: Vec<(&str, &str)> = owned.iter().map(|(k, v)| (*k, v.as_str())).collect();
    send(app, method, path, &borrowed, None).await
}

async fn register_account(app: &Router) -> Value {
    let (status, body) = request(
        app,
        "/auth/register",
        json!({"username": "alice", "email": " Alice@Example.com ", "password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], 900);
    body
}

async fn me_status(app: &Router, token: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::get("/auth/me")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[test]
fn signing_key_must_be_at_least_32_bytes() {
    assert!(AuthConfig::new("").is_err());
    assert!(AuthConfig::new(&"a".repeat(31)).is_err());
    assert!(AuthConfig::new(&"a".repeat(32)).is_ok());
}

#[test]
fn passwords_use_salted_argon2id() {
    let first = hash_password(PASSWORD).unwrap();
    let second = hash_password(PASSWORD).unwrap();
    assert!(first.starts_with("$argon2id$"));
    assert_ne!(first, second);
    assert!(verify_password(PASSWORD, &first).unwrap());
    assert!(!verify_password("incorrect", &first).unwrap());
    assert!(verify_password(PASSWORD, "plaintext-is-not-supported").is_err());
}

#[test]
fn access_tokens_validate_signature_claims_and_lifetime() {
    let auth = config();
    let user_id = Uuid::new_v4();
    let token = auth.access_token(user_id).unwrap();
    let claims = auth.verify(&token).unwrap();
    assert_eq!(claims.sub, user_id);
    assert_eq!(claims.exp - claims.iat, 900);
    let other = AuthConfig::new("a-different-test-only-secret-of-32-bytes").unwrap();
    assert!(other.verify(&token).is_err());
    assert!(auth.verify("not.a.jwt").is_err());

    let now = Utc::now().timestamp();
    for (exp, iat, issuer, audience, algorithm) in [
        (now - 1, now - 901, ISSUER, AUDIENCE, Algorithm::HS256),
        (now, now - 900, ISSUER, AUDIENCE, Algorithm::HS256),
        (now + 900, now + 60, ISSUER, AUDIENCE, Algorithm::HS256),
        (now + 900, now, "wrong", AUDIENCE, Algorithm::HS256),
        (now + 900, now, ISSUER, "wrong", Algorithm::HS256),
        (now + 900, now, ISSUER, AUDIENCE, Algorithm::HS384),
    ] {
        let claims = Claims {
            sub: user_id,
            iat,
            exp,
            iss: issuer.to_owned(),
            aud: audience.to_owned(),
        };
        let token = encode(&Header::new(algorithm), &claims, &auth.encoding_key).unwrap();
        assert!(auth.verify(&token).is_err());
    }

    let missing_subject = encode(
        &Header::new(Algorithm::HS256),
        &json!({"exp": now + 900, "iat": now, "iss": ISSUER, "aud": AUDIENCE}),
        &auth.encoding_key,
    )
    .unwrap();
    assert!(auth.verify(&missing_subject).is_err());
}

#[test]
fn refresh_tokens_are_random_256_bit_secrets_with_stable_hashes() {
    let first = new_refresh_token().unwrap();
    let second = new_refresh_token().unwrap();
    assert_eq!(URL_SAFE_NO_PAD.decode(&first).unwrap().len(), 32);
    assert_eq!(first.len(), 43);
    assert_ne!(first, second);
    assert_ne!(token_hash(&first), token_hash(&second));
    assert_eq!(token_hash(&first).len(), 64);
    assert!(validate_refresh_token(&first).is_ok());
    for invalid in ["", "short", &"!".repeat(43), &"a".repeat(4096)] {
        assert!(validate_refresh_token(invalid).is_err());
    }
}

#[test]
fn email_validation_and_normalization() {
    assert_eq!(
        normalize_email(" Alice@Example.COM ").unwrap(),
        "alice@example.com"
    );
    for invalid in ["", "@example.com", "alice@", "a@b@c", "a b@example.com"] {
        assert!(normalize_email(invalid).is_err());
    }
    assert!(normalize_email(&format!("{}@example.com", "a".repeat(255))).is_err());
}

#[tokio::test]
async fn bearer_header_parsing_rejects_bad_headers_without_touching_the_database() {
    // This pool cannot connect: every rejection below must happen before any query.
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let auth = config();
    let token = auth.access_token(Uuid::new_v4()).unwrap();
    let app = router(pool, auth);

    for value in [
        None,
        Some("Basic abc"),
        Some("Bearer "),
        Some("Bearer garbage"),
    ] {
        let mut req = Request::get("/auth/me");
        if let Some(value) = value {
            req = req.header(header::AUTHORIZATION, value);
        }
        let response = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let duplicate = Request::get("/auth/me")
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.oneshot(duplicate).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn registration_login_rotation_and_logout(pool: PgPool) {
    let app = app(pool.clone());
    let registered = register_account(&app).await;
    let access = registered["access_token"].as_str().unwrap();
    let refresh_token = registered["refresh_token"].as_str().unwrap();
    let (status, me) = me_status(&app, access).await;
    assert_eq!(status, StatusCode::OK);

    let (email, password_hash): (String, String) =
        sqlx::query_as("SELECT user_email, user_pass FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(email, "alice@example.com");
    assert!(password_hash.starts_with("$argon2id$"));
    let stored: String = sqlx::query_scalar("SELECT refresh_token_hash FROM auth_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, token_hash(refresh_token));
    assert_ne!(stored, refresh_token);

    for (email, password) in [
        ("alice@example.com", "wrong password"),
        ("missing@example.com", PASSWORD),
    ] {
        let (status, _) = request(
            &app,
            "/auth/login",
            json!({"email": email, "password": password}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, login) = request(
        &app,
        "/auth/login",
        json!({"email": "ALICE@example.com", "password": PASSWORD}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(login["refresh_token"], registered["refresh_token"]);
    assert_eq!(
        me_status(&app, login["access_token"].as_str().unwrap())
            .await
            .1,
        me
    );

    let (status, rotated) = request(
        &app,
        "/auth/refresh",
        json!({"refresh_token": refresh_token}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(rotated["refresh_token"], registered["refresh_token"]);
    // Sliding expiry: rotation pushes the expiry out instead of keeping the login-time value.
    let expiry = |value: &Value| value.as_str().unwrap().parse::<DateTime<Utc>>().unwrap();
    assert!(expiry(&rotated["refresh_expires_at"]) >= expiry(&registered["refresh_expires_at"]));
    // The retired token still works for a moment (a lost response can be retried) ...
    let (status, retried) = request(
        &app,
        "/auth/refresh",
        json!({"refresh_token": refresh_token}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let stale = rotated["refresh_token"].clone();
    let rotated = retried;
    // ... but replaying the retired token after the grace window revokes the whole session.
    sqlx::query("UPDATE auth_sessions SET rotated_at = CURRENT_TIMESTAMP - INTERVAL '2 minutes' WHERE previous_refresh_token_hash IS NOT NULL")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "/auth/refresh", json!({"refresh_token": stale})).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(&app, "/auth/refresh", json!({"refresh_token": rotated["refresh_token"]})).await.0,
        StatusCode::UNAUTHORIZED,
        "reuse must revoke the session"
    );
    let (_, relogin, _) = {
        let (status, body) = request(&app, "/auth/login", json!({"email": "alice@example.com", "password": PASSWORD})).await;
        (status, body.clone(), ())
    };
    let rotated = relogin;
    let body = json!({"refresh_token": rotated["refresh_token"]});
    assert_eq!(
        request(&app, "/auth/logout", body.clone()).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "/auth/logout", body.clone()).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        request(&app, "/auth/refresh", body).await.0,
        StatusCode::UNAUTHORIZED
    );
    // Logout does not revoke already-issued stateless JWTs or other devices.
    assert_eq!(me_status(&app, access).await.0, StatusCode::OK);
    assert_eq!(
        request(
            &app,
            "/auth/refresh",
            json!({"refresh_token": login["refresh_token"]})
        )
        .await
        .0,
        StatusCode::OK
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn rejects_duplicate_accounts_and_invalid_registration(pool: PgPool) {
    let app = app(pool);
    register_account(&app).await;
    for (username, email) in [("alice", "bob@example.com"), ("bob", "ALICE@example.com")] {
        assert_eq!(
            request(
                &app,
                "/auth/register",
                json!({"username": username, "email": email, "password": PASSWORD})
            )
            .await
            .0,
            StatusCode::CONFLICT
        );
    }
    for (username, email, password) in [
        (" ", "bob@example.com", PASSWORD),
        ("bob", "invalid", PASSWORD),
        ("bob", "bob@example.com", "short"),
        ("bob", "bob@example.com", &"x".repeat(1025)),
    ] {
        assert_eq!(
            request(
                &app,
                "/auth/register",
                json!({"username": username, "email": email, "password": password})
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn expired_inactive_and_deleted_accounts_cannot_refresh(pool: PgPool) {
    let app = app(pool.clone());
    let registered = register_account(&app).await;
    let token_body = json!({"refresh_token": registered["refresh_token"]});
    sqlx::query("UPDATE users SET user_active = FALSE")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "/auth/refresh", token_body.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "/auth/login",
            json!({"email": "alice@example.com", "password": PASSWORD})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE users SET user_active = TRUE")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE auth_sessions SET created_at = CURRENT_TIMESTAMP - INTERVAL '31 days',
         expires_at = CURRENT_TIMESTAMP - INTERVAL '1 day'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        request(&app, "/auth/refresh", token_body).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, login) = request(
        &app,
        "/auth/login",
        json!({"email": "alice@example.com", "password": PASSWORD}),
    )
    .await;
    sqlx::query("DELETE FROM users")
        .execute(&pool)
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        request(
            &app,
            "/auth/refresh",
            json!({"refresh_token": login["refresh_token"]})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn concurrent_refresh_of_one_token_chains_instead_of_forking(pool: PgPool) {
    let app = app(pool.clone());
    let registered = register_account(&app).await;
    let body = json!({"refresh_token": registered["refresh_token"]});
    let (first, second) = tokio::join!(
        request(&app, "/auth/refresh", body.clone()),
        request(&app, "/auth/refresh", body),
    );
    // The second request is the "lost response" retry: it is served, and both rotations land on
    // the same session (one row), so there is only ever one live current token.
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(second.0, StatusCode::OK);
    assert_ne!(first.1["refresh_token"], second.1["refresh_token"]);
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(sessions, 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn sessions_slide_per_platform_and_stop_at_the_absolute_cap(pool: PgPool) {
    let app = app(pool.clone());
    let (status, _, desktop) = send(
        &app, "POST", "/auth/register", DESKTOP,
        Some(json!({"username": "dora", "email": "dora@example.com", "password": PASSWORD})),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    let days = |value: &Value| {
        (value.as_str().unwrap().parse::<DateTime<Utc>>().unwrap() - Utc::now()).num_days()
    };
    assert!((89..=90).contains(&days(&desktop["refresh_expires_at"])));
    assert!(desktop["refresh_token"].is_string(), "apps get the token in the body");

    // Close to the absolute cap the sliding window is clamped to it.
    sqlx::query("UPDATE auth_sessions SET absolute_expires_at = CURRENT_TIMESTAMP + INTERVAL '3 days'")
        .execute(&pool).await.unwrap();
    let (status, _, refreshed) = send(
        &app, "POST", "/auth/refresh", DESKTOP,
        Some(json!({"refresh_token": desktop["refresh_token"]})),
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert!((2..=3).contains(&days(&refreshed["refresh_expires_at"])));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn browser_refresh_token_lives_in_an_httponly_cookie(pool: PgPool) {
    let app = app(pool.clone());
    let (status, headers, registered) = send(
        &app, "POST", "/auth/register", WEB,
        Some(json!({"username": "web", "email": "web@example.com", "password": PASSWORD})),
    ).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(registered.get("refresh_token").is_none(), "{registered}");
    assert_eq!(registered["user"]["username"], "web");
    assert_eq!(registered["user"]["email"], "web@example.com");
    let set_cookie = headers[header::SET_COOKIE].to_str().unwrap();
    for part in ["HttpOnly", "SameSite=Strict", "Path=/api/auth", "Secure"] {
        assert!(set_cookie.contains(part), "{set_cookie}");
    }
    let cookie = cookie_of(&headers);

    // Refresh works from the cookie alone and rotates it.
    let with_cookie: Vec<(&str, &str)> = vec![WEB[0], ("cookie", cookie.as_str())];
    let (status, headers, refreshed) = send(&app, "POST", "/auth/refresh", &with_cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(refreshed.get("refresh_token").is_none());
    assert_eq!(refreshed["user"]["username"], "web");
    let rotated_cookie = cookie_of(&headers);
    assert_ne!(rotated_cookie, cookie);

    // Without the platform header the cookie is ignored (cross-site requests cannot set it).
    let no_header: Vec<(&str, &str)> = vec![("cookie", rotated_cookie.as_str())];
    assert_eq!(send(&app, "POST", "/auth/refresh", &no_header, None).await.0, StatusCode::UNAUTHORIZED);
    // A browser cannot be talked into a body token either: it must use its cookie.
    let (_, _, login) = send(
        &app, "POST", "/auth/login", &[],
        Some(json!({"email": "web@example.com", "password": PASSWORD})),
    ).await;
    assert!(login["refresh_token"].is_string());
    assert_eq!(
        send(&app, "POST", "/auth/refresh", WEB, Some(json!({"refresh_token": login["refresh_token"]}))).await.0,
        StatusCode::UNAUTHORIZED
    );

    // A dead cookie is cleared by the 401; logout clears it and revokes the session.
    let dead: Vec<(&str, &str)> = vec![WEB[0], ("cookie", "showflow_refresh=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")];
    let (status, headers, _) = send(&app, "POST", "/auth/refresh", &dead, None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers[header::SET_COOKIE].to_str().unwrap().contains("Max-Age=0"));
    let live: Vec<(&str, &str)> = vec![WEB[0], ("cookie", rotated_cookie.as_str())];
    let (status, headers, _) = send(&app, "POST", "/auth/logout", &live, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(headers[header::SET_COOKIE].to_str().unwrap().contains("Max-Age=0"));
    assert_eq!(send(&app, "POST", "/auth/refresh", &live, None).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn devices_are_recorded_listed_and_revocable(pool: PgPool) {
    let app = app(pool.clone());
    let body = || Some(json!({"username": "eve", "email": "eve@example.com", "password": PASSWORD}));
    let (_, _, desktop) = send(&app, "POST", "/auth/register", DESKTOP, body()).await;
    let login = json!({"email": "eve@example.com", "password": PASSWORD});
    let phone = [
        ("x-showflow-platform", "mobile/android"),
        ("x-showflow-device-id", "phone-device-00002"),
        ("x-showflow-app-version", "1.0.1"),
    ];
    let (_, _, mobile) = send(&app, "POST", "/auth/login", &phone, Some(login.clone())).await;
    // Logging in again from the same device reuses its row.
    send(&app, "POST", "/auth/login", DESKTOP, Some(login)).await;
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM client").fetch_one(&pool).await.unwrap();
    assert_eq!(rows, 2);

    let token = desktop["access_token"].as_str().unwrap();
    let (status, _, list) = authed(&app, "GET", "/auth/devices", token, DESKTOP).await;
    assert_eq!(status, StatusCode::OK);
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    let current: Vec<_> = list.iter().filter(|d| d["current"] == true).collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["platform"], "desktop");
    assert_eq!(current[0]["app_version"], "1.0.0");
    assert!(list.iter().all(|d| d.get("identifier").is_none()));

    // Revoking the phone kills its refresh tokens and nothing else.
    let phone_id = list.iter().find(|d| d["platform"] == "mobile").unwrap()["id"].as_str().unwrap().to_owned();
    let (status, _, _) = authed(&app, "DELETE", &format!("/auth/devices/{phone_id}"), token, DESKTOP).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        send(&app, "POST", "/auth/refresh", &phone, Some(json!({"refresh_token": mobile["refresh_token"]}))).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        send(&app, "POST", "/auth/refresh", DESKTOP, Some(json!({"refresh_token": desktop["refresh_token"]}))).await.0,
        StatusCode::OK
    );
    let (_, _, list) = authed(&app, "GET", "/auth/devices", token, DESKTOP).await;
    assert_eq!(list.as_array().unwrap().len(), 1);

    // Other users' devices cannot be revoked; logout-all revokes everything.
    let (_, _, other) = send(&app, "POST", "/auth/register", DESKTOP,
        Some(json!({"username": "mallory", "email": "mallory@example.com", "password": PASSWORD}))).await;
    let desktop_id = list[0]["id"].as_str().unwrap();
    authed(&app, "DELETE", &format!("/auth/devices/{desktop_id}"), other["access_token"].as_str().unwrap(), DESKTOP).await;
    let (_, _, list) = authed(&app, "GET", "/auth/devices", token, DESKTOP).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let (status, _, _) = authed(&app, "POST", "/auth/logout-all", token, DESKTOP).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_sessions s JOIN users u USING (user_id) WHERE u.user_name = 'eve'")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(count, 0);
    assert_eq!(authed(&app, "GET", "/auth/devices", "garbage", &[]).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn verification_and_reset_tokens_are_single_use_and_hashed(pool: PgPool) {
    use super::tokens::{consume_user_token, create_user_token, TokenPurpose};
    let app = app(pool.clone());
    register_account(&app).await;
    let user_id: Uuid = sqlx::query_scalar("SELECT user_id FROM users").fetch_one(&pool).await.unwrap();
    let mut conn = pool.acquire().await.unwrap();

    let raw = create_user_token(&mut conn, user_id, TokenPurpose::PasswordReset, Duration::hours(1)).await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM user_tokens").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, token_hash(&raw));
    assert!(consume_user_token(&mut conn, TokenPurpose::EmailVerify, &raw).await.is_err(), "purpose must match");
    assert_eq!(consume_user_token(&mut conn, TokenPurpose::PasswordReset, &raw).await.unwrap(), user_id);
    assert!(consume_user_token(&mut conn, TokenPurpose::PasswordReset, &raw).await.is_err(), "single use");

    let expired = create_user_token(&mut conn, user_id, TokenPurpose::EmailVerify, Duration::hours(1)).await.unwrap();
    sqlx::query("UPDATE user_tokens SET created_at = CURRENT_TIMESTAMP - INTERVAL '2 hours', expires_at = CURRENT_TIMESTAMP - INTERVAL '1 hour' WHERE used_at IS NULL")
        .execute(&pool).await.unwrap();
    assert!(consume_user_token(&mut conn, TokenPurpose::EmailVerify, &expired).await.is_err());
    super::purge_expired(&pool).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM user_tokens WHERE used_at IS NULL").fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0);
}
