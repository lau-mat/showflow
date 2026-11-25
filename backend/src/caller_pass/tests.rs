use super::*;
use crate::auth::{self, AuthConfig};
use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use protocol::caller_pass::{signing_key, verify_pass, PassError};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

/// 32 bytes of 7, as `CALLER_PASS_KEY` would hold them.
const SEED: &str = "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc=";

fn config() -> AuthConfig {
    AuthConfig::new("test-only-signing-key-with-at-least-32-bytes").unwrap()
}

fn app(pool: PgPool, config: AuthConfig) -> Router {
    Router::new()
        .merge(auth::routes())
        .merge(crate::shows::routes())
        .merge(crate::sharing::routes())
        .merge(routes())
        .with_state(auth::state(pool, config))
}

async fn call(app: &Router, method: &str, path: &str, token: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path).header(header::AUTHORIZATION, format!("Bearer {token}"));
    let body = match body {
        Some(body) => {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app.clone().oneshot(request.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

/// Registers a user; returns (user id, access token). Their email is `<name>@example.com`.
async fn user(app: &Router, name: &str) -> (Uuid, String) {
    let request = Request::post("/auth/register").header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"username": name, "email": format!("{name}@example.com"), "password": "correct horse battery staple"}).to_string())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    (body["user"]["id"].as_str().unwrap().parse().unwrap(), body["access_token"].as_str().unwrap().to_owned())
}

/// A show owned by `owner` with `member` invited as a viewer.
async fn shared_show(app: &Router, owner: &str, member: &str) -> Uuid {
    let id = Uuid::new_v4();
    let op = json!({"op_id": Uuid::new_v4(), "entity": "show", "id": id, "kind": "set", "fields": {"name": "Gala", "time": 1}});
    assert_eq!(call(app, "POST", &format!("/shows/{id}/ops"), owner, Some(json!({"ops": [op]}))).await.0, StatusCode::OK);
    let invite = json!({"role": "viewer", "email": format!("{member}@example.com")});
    assert_eq!(call(app, "POST", &format!("/shows/{id}/invites"), owner, Some(invite)).await.0, StatusCode::CREATED);
    id
}

fn device_key() -> String {
    public_key_text(&signing_key([2; 32]))
}

async fn ask_pass(app: &Router, token: &str, show: Uuid) -> (StatusCode, Value) {
    call(app, "POST", &format!("/shows/{show}/caller-pass"), token, Some(json!({"device_key": device_key()}))).await
}

/// (members, callers) as `GET /shows` reports them.
async fn listed(app: &Router, token: &str, show: Uuid) -> (Vec<String>, Vec<String>) {
    let (_, shows) = call(app, "GET", "/shows", token, None).await;
    let item = shows.as_array().unwrap().iter().find(|item| item["id"] == show.to_string()).unwrap().clone();
    let ids = |key: &str| item[key].as_array().unwrap().iter().map(|id| id.as_str().unwrap().to_owned()).collect();
    (ids("members"), ids("callers"))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn members_get_a_pass_and_the_lists_say_who_may_call(pool: PgPool) {
    let app = app(pool, config().with_caller_key(SEED).unwrap());
    let (ann, ann_token) = user(&app, "ann").await;
    let (bob, bob_token) = user(&app, "bob").await;
    let (_, eve_token) = user(&app, "eve").await;
    let show = shared_show(&app, &ann_token, "bob").await;

    let (status, key) = call(&app, "GET", "/caller-passes/key", &bob_token, None).await;
    assert_eq!(status, StatusCode::OK);
    let trusted = parse_public_key(key["public_key"].as_str().unwrap()).unwrap();

    // The owner's pass verifies with the published key and says who and which device.
    let (status, issued) = ask_pass(&app, &ann_token, show).await;
    assert_eq!(status, StatusCode::OK, "{issued}");
    let now = Utc::now().timestamp();
    let body = verify_pass(&trusted, issued["pass"].as_str().unwrap(), &show.to_string(), now).unwrap();
    assert_eq!((body.user_id.as_str(), body.username.as_str()), (ann.to_string().as_str(), "ann"));
    assert_eq!(body.device_key, device_key());
    assert_eq!(body.expires_at, issued["expires_at"].as_i64().unwrap());
    assert_eq!(verify_pass(&trusted, issued["pass"].as_str().unwrap(), &Uuid::new_v4().to_string(), now), Err(PassError::OtherShow));

    // Every member gets one (it admits them to a protected session); a stranger learns nothing.
    assert_eq!(ask_pass(&app, &bob_token, show).await.0, StatusCode::OK);
    assert_eq!(ask_pass(&app, &eve_token, show).await.0, StatusCode::NOT_FOUND);
    let mut both = vec![ann.to_string(), bob.to_string()];
    both.sort();
    assert_eq!(listed(&app, &bob_token, show).await, (both.clone(), vec![ann.to_string()]), "member, not caller");

    // Only the owner hands out the right to call; the member's role is untouched by it.
    let member = format!("/shows/{show}/members/{bob}");
    assert_eq!(call(&app, "PATCH", &member, &bob_token, Some(json!({"can_call": true}))).await.0, StatusCode::FORBIDDEN);
    assert_eq!(call(&app, "PATCH", &member, &ann_token, Some(json!({}))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&app, "PATCH", &member, &ann_token, Some(json!({"can_call": true}))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(listed(&app, &ann_token, show).await, (both.clone(), both.clone()));
    let (_, members) = call(&app, "GET", &format!("/shows/{show}/members"), &ann_token, None).await;
    let bobs = members.as_array().unwrap().iter().find(|m| m["user_id"] == bob.to_string()).unwrap();
    assert_eq!((bobs["role"].as_str(), bobs["can_call"].as_bool()), (Some("viewer"), Some(true)));

    // Changing the role keeps the right to call, and taking it away is reported to hosts.
    assert_eq!(call(&app, "PATCH", &member, &ann_token, Some(json!({"role": "editor"}))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(listed(&app, &ann_token, show).await.1, both);
    assert_eq!(call(&app, "PATCH", &member, &ann_token, Some(json!({"can_call": false}))).await.0, StatusCode::NO_CONTENT);
    assert_eq!(listed(&app, &ann_token, show).await.1, vec![ann.to_string()]);

    // Removed from the show: off both lists, and no new pass.
    assert_eq!(call(&app, "DELETE", &member, &ann_token, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(listed(&app, &ann_token, show).await, (vec![ann.to_string()], vec![ann.to_string()]));
    assert_eq!(ask_pass(&app, &bob_token, show).await.0, StatusCode::NOT_FOUND);

    let bad = call(&app, "POST", &format!("/shows/{show}/caller-pass"), &ann_token, Some(json!({"device_key": "nope"}))).await;
    assert_eq!(bad.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn without_a_signing_key_account_access_does_not_exist(pool: PgPool) {
    let app = app(pool, config());
    let (_, ann) = user(&app, "ann").await;
    user(&app, "bob").await;
    let show = shared_show(&app, &ann, "bob").await;
    assert_eq!(call(&app, "GET", "/caller-passes/key", &ann, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ask_pass(&app, &ann, show).await.0, StatusCode::NOT_FOUND);
}

#[test]
fn the_signing_key_must_be_32_bytes_of_base64() {
    assert!(config().with_caller_key(SEED).unwrap().caller_key().is_some());
    assert!(config().with_caller_key(&format!(" {SEED}\n")).is_ok());
    for bad in ["", "short", "BwcHBwc=", "not base64 at all, but long enough to look like it"] {
        assert!(config().with_caller_key(bad).is_err(), "{bad}");
    }
    assert!(config().caller_key().is_none());
}
