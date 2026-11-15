use super::*;
use axum::{
    body::{to_bytes, Body},
    http::Request,
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
async fn bearer_authentication_is_stateless_and_rejects_bad_headers() {
    // This pool cannot connect: successful authentication proves no DB lookup is made.
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let auth = config();
    let user_id = Uuid::new_v4();
    let token = auth.access_token(user_id).unwrap();
    let app = router(pool, auth);
    let (status, body) = me_status(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user_id"], user_id.to_string());

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
    assert_eq!(
        rotated["refresh_expires_at"],
        registered["refresh_expires_at"]
    );
    assert_eq!(
        request(
            &app,
            "/auth/refresh",
            json!({"refresh_token": refresh_token})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
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
async fn concurrent_refresh_is_single_use(pool: PgPool) {
    let app = app(pool);
    let registered = register_account(&app).await;
    let body = json!({"refresh_token": registered["refresh_token"]});
    let (first, second) = tokio::join!(
        request(&app, "/auth/refresh", body.clone()),
        request(&app, "/auth/refresh", body),
    );
    let statuses = [first.0, second.0];
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1);
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::UNAUTHORIZED)
            .count(),
        1
    );
}
