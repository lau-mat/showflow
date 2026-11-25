use super::*;
use crate::auth::{self, AuthConfig};
use axum::{
    body::{to_bytes, Body},
    http::{header, Request},
};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

fn app(pool: PgPool) -> Router {
    let config = AuthConfig::new("test-only-signing-key-with-at-least-32-bytes").unwrap();
    Router::new()
        .merge(auth::routes())
        .merge(crate::shows::routes())
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
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

/// Registers a user; returns (user id, access token). Their email is `<name>@example.com`.
async fn user(app: &Router, name: &str) -> (Uuid, String) {
    let request = Request::post("/auth/register").header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({"username": name, "email": format!("{name}@example.com"), "password": PASSWORD}).to_string())).unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body: Value = serde_json::from_slice(&to_bytes(response.into_body(), 1 << 20).await.unwrap()).unwrap();
    (body["user"]["id"].as_str().unwrap().parse().unwrap(), body["access_token"].as_str().unwrap().to_owned())
}

async fn new_show(app: &Router, token: &str) -> Uuid {
    let id = Uuid::new_v4();
    let op = json!({"op_id": Uuid::new_v4(), "entity": "show", "id": id, "kind": "set", "fields": {"name": "Gala", "time": 1}});
    let (status, _) = call(app, "POST", &format!("/shows/{id}/ops"), token, Some(json!({"ops": [op]}))).await;
    assert_eq!(status, StatusCode::OK);
    id
}

async fn try_edit(app: &Router, token: &str, show: Uuid) -> (StatusCode, String) {
    let op = json!({"op_id": Uuid::new_v4(), "entity": "role", "id": Uuid::new_v4(), "kind": "set", "fields": {"name": "Audio"}});
    let (status, body) = call(app, "POST", &format!("/shows/{show}/ops"), token, Some(json!({"ops": [op]}))).await;
    let result = body["results"][0]["status"].as_str().unwrap_or("").to_owned();
    (status, result)
}

async fn invite(app: &Router, token: &str, show: Uuid, body: Value) -> Value {
    let (status, body) = call(app, "POST", &format!("/shows/{show}/invites"), token, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn an_existing_account_is_added_straight_away_and_gets_exactly_that_role(pool: PgPool) {
    let app = app(pool.clone());
    let (_, owner) = user(&app, "owner").await;
    let (bob, bob_token) = user(&app, "bob").await;
    let show = new_show(&app, &owner).await;
    assert_eq!(call(&app, "GET", &format!("/shows/{show}"), &bob_token, None).await.0, StatusCode::NOT_FOUND);

    let added = invite(&app, &owner, show, json!({"role": "viewer", "email": " Bob@Example.com "})).await;
    assert_eq!(added["added"]["user_id"], bob.to_string());
    assert_eq!(added["invite"], Value::Null);
    assert_eq!(added["token"], Value::Null);
    let (status, snapshot) = call(&app, "GET", &format!("/shows/{show}"), &bob_token, None).await;
    assert_eq!((status, snapshot["show"]["role"].as_str()), (StatusCode::OK, Some("viewer")));
    assert_eq!(try_edit(&app, &bob_token, show).await.1, "rejected");

    // The owner promotes bob; the new role applies to his very next write.
    let (status, _) = call(&app, "PATCH", &format!("/shows/{show}/members/{bob}"), &owner, Some(json!({"role": "editor"}))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(try_edit(&app, &bob_token, show).await.1, "applied");

    let (_, members) = call(&app, "GET", &format!("/shows/{show}/members"), &bob_token, None).await;
    let names: Vec<(&str, &str, bool)> = members.as_array().unwrap().iter()
        .map(|m| (m["username"].as_str().unwrap(), m["role"].as_str().unwrap(), m["is_me"].as_bool().unwrap())).collect();
    assert_eq!(names, [("owner", "owner", false), ("bob", "editor", true)]);

    // Inviting someone who is already in never changes their role.
    invite(&app, &owner, show, json!({"role": "viewer", "email": "bob@example.com"})).await;
    let (_, snapshot) = call(&app, "GET", &format!("/shows/{show}"), &bob_token, None).await;
    assert_eq!(snapshot["show"]["role"], "editor");
    invite(&app, &owner, show, json!({"role": "viewer", "email": "owner@example.com"})).await;
    let (_, snapshot) = call(&app, "GET", &format!("/shows/{show}"), &owner, None).await;
    assert_eq!(snapshot["show"]["role"], "owner");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn only_the_owner_manages_access(pool: PgPool) {
    let app = app(pool.clone());
    let (owner_id, owner) = user(&app, "owner").await;
    let (editor_id, editor) = user(&app, "editor").await;
    let (viewer_id, viewer) = user(&app, "viewer").await;
    let (_, stranger) = user(&app, "stranger").await;
    let show = new_show(&app, &owner).await;
    invite(&app, &owner, show, json!({"role": "editor", "email": "editor@example.com"})).await;
    invite(&app, &owner, show, json!({"role": "viewer", "email": "viewer@example.com"})).await;
    let members = format!("/shows/{show}/members");

    // Members who are not the owner are refused (403); strangers do not even learn the show exists (404).
    for token in [&editor, &viewer] {
        assert_eq!(call(&app, "POST", &format!("/shows/{show}/invites"), token, Some(json!({"role": "viewer"}))).await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(&app, "GET", &format!("/shows/{show}/invites"), token, None).await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(&app, "PATCH", &format!("{members}/{viewer_id}"), token, Some(json!({"role": "editor"}))).await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(&app, "DELETE", &format!("{members}/{owner_id}"), token, None).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(call(&app, "DELETE", &format!("{members}/{viewer_id}"), &editor, None).await.0, StatusCode::FORBIDDEN, "an editor cannot remove others");
    for request in [("GET", members.clone(), None), ("POST", format!("/shows/{show}/invites"), Some(json!({"role": "viewer"}))), ("DELETE", format!("{members}/{viewer_id}"), None)] {
        assert_eq!(call(&app, request.0, &request.1, &stranger, request.2).await.0, StatusCode::NOT_FOUND, "{} {}", request.0, request.1);
    }

    // Ownership cannot be handed out, taken, or abandoned through these endpoints.
    assert_eq!(call(&app, "PATCH", &format!("{members}/{editor_id}"), &owner, Some(json!({"role": "owner"}))).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&app, "PATCH", &format!("{members}/{owner_id}"), &owner, Some(json!({"role": "viewer"}))).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "DELETE", &format!("{members}/{owner_id}"), &owner, None).await.0, StatusCode::FORBIDDEN, "the owner deletes the show instead of leaving");
    assert_eq!(call(&app, "POST", &format!("/shows/{show}/invites"), &owner, Some(json!({"role": "owner"}))).await.0, StatusCode::BAD_REQUEST);

    // Removing a member ends their access at once; anyone may leave on their own.
    assert_eq!(call(&app, "DELETE", &format!("{members}/{editor_id}"), &owner, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "GET", &format!("/shows/{show}"), &editor, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(try_edit(&app, &editor, show).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "DELETE", &format!("{members}/{viewer_id}"), &viewer, None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "GET", &format!("/shows/{show}"), &viewer, None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "DELETE", &format!("{members}/{viewer_id}"), &viewer, None).await.0, StatusCode::NOT_FOUND, "already gone");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn an_invite_link_works_once_for_the_right_person(pool: PgPool) {
    let app = app(pool.clone());
    let (_, owner) = user(&app, "owner").await;
    let show = new_show(&app, &owner).await;

    // Nobody has this email yet: the owner gets a link to send.
    let created = invite(&app, &owner, show, json!({"role": "commenter", "email": "newcomer@example.com"})).await;
    let token = created["token"].as_str().unwrap().to_owned();
    assert_eq!(created["added"], Value::Null);
    assert_eq!((created["invite"]["role"].as_str(), created["invite"]["email"].as_str(), created["invite"]["max_uses"].as_i64()), (Some("commenter"), Some("newcomer@example.com"), Some(1)));
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM show_invites").fetch_one(&pool).await.unwrap();
    assert_ne!(stored, token, "only a hash is stored");
    assert_eq!(stored, token_hash(&token));
    let (_, pending) = call(&app, "GET", &format!("/shows/{show}/invites"), &owner, None).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);
    assert!(pending[0].get("token").is_none(), "the link cannot be recovered later");

    // Someone else cannot use a link meant for another email, and it stays usable for the right one.
    let (_, other) = user(&app, "other").await;
    let (status, _) = call(&app, "POST", "/invites/accept", &other, Some(json!({"token": token}))).await;
    assert_eq!(status, StatusCode::GONE);
    assert_eq!(call(&app, "POST", "/invites/preview", &other, Some(json!({"token": token}))).await.0, StatusCode::GONE);

    let (_, newcomer) = user(&app, "newcomer").await;
    let (_, preview) = call(&app, "POST", "/invites/preview", &newcomer, Some(json!({"token": token}))).await;
    assert_eq!((preview["show_name"].as_str(), preview["role"].as_str(), preview["invited_by"].as_str(), preview["already_member"].as_bool()), (Some("Gala"), Some("commenter"), Some("owner"), Some(false)));
    let (status, joined) = call(&app, "POST", "/invites/accept", &newcomer, Some(json!({"token": token}))).await;
    assert_eq!((status, joined["role"].as_str(), joined["show_id"].as_str()), (StatusCode::OK, Some("commenter"), Some(show.to_string().as_str())));
    assert_eq!(call(&app, "GET", &format!("/shows/{show}"), &newcomer, None).await.0, StatusCode::OK);

    // Single use: gone for everyone now, and it no longer shows up as pending.
    assert_eq!(call(&app, "POST", "/invites/accept", &other, Some(json!({"token": token}))).await.0, StatusCode::GONE);
    let (_, pending) = call(&app, "GET", &format!("/shows/{show}/invites"), &owner, None).await;
    assert!(pending.as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn open_links_have_limits_and_can_be_revoked_or_expire(pool: PgPool) {
    let app = app(pool.clone());
    let (_, owner) = user(&app, "owner").await;
    let show = new_show(&app, &owner).await;
    let created = invite(&app, &owner, show, json!({"role": "viewer", "max_uses": 2})).await;
    let (token, invite_id) = (created["token"].as_str().unwrap().to_owned(), created["invite"]["id"].as_str().unwrap().to_owned());

    let mut people = Vec::new();
    for name in ["ann", "ben", "cat"] {
        people.push(user(&app, name).await.1);
    }
    let accept = |who: usize| { let (app, token, who) = (app.clone(), token.clone(), people[who].clone()); async move { call(&app, "POST", "/invites/accept", &who, Some(json!({"token": token}))).await } };
    assert_eq!(accept(0).await.0, StatusCode::OK);
    // Accepting again as a member neither fails nor uses up the link.
    assert_eq!(accept(0).await.0, StatusCode::OK);
    assert_eq!(accept(1).await.0, StatusCode::OK);
    assert_eq!(accept(2).await.0, StatusCode::GONE, "two uses, then it is spent");
    let used: i32 = sqlx::query_scalar("SELECT used_count FROM show_invites").fetch_one(&pool).await.unwrap();
    assert_eq!(used, 2);

    // Revoked and expired links are dead; so are made-up tokens and links to deleted shows.
    let second = invite(&app, &owner, show, json!({"role": "viewer"})).await;
    let revoked = second["token"].as_str().unwrap().to_owned();
    let (status, _) = call(&app, "DELETE", &format!("/shows/{show}/invites/{}", second["invite"]["id"].as_str().unwrap()), &owner, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, "POST", "/invites/accept", &people[2], Some(json!({"token": revoked}))).await.0, StatusCode::GONE);
    let third = invite(&app, &owner, show, json!({"role": "viewer"})).await;
    sqlx::query("UPDATE show_invites SET expires_at = now() - interval '1 minute' WHERE invite_id = $1::uuid").bind(third["invite"]["id"].as_str().unwrap()).execute(&pool).await.unwrap();
    assert_eq!(call(&app, "POST", "/invites/accept", &people[2], Some(json!({"token": third["token"]}))).await.0, StatusCode::GONE);
    assert_eq!(call(&app, "POST", "/invites/accept", &people[2], Some(json!({"token": "x".repeat(43)}))).await.0, StatusCode::GONE);
    assert_eq!(call(&app, "POST", "/invites/accept", &people[2], Some(json!({"token": "short"}))).await.0, StatusCode::GONE);
    let fourth = invite(&app, &owner, show, json!({"role": "viewer"})).await;
    let op = json!({"op_id": Uuid::new_v4(), "entity": "show", "id": show, "kind": "delete"});
    call(&app, "POST", &format!("/shows/{show}/ops"), &owner, Some(json!({"ops": [op]}))).await;
    assert_eq!(call(&app, "POST", "/invites/accept", &people[2], Some(json!({"token": fourth["token"]}))).await.0, StatusCode::GONE);
    let _ = invite_id;
    // Out-of-range settings are refused.
    let fresh = new_show(&app, &owner).await;
    for body in [json!({"role": "viewer", "expires_in_days": 0}), json!({"role": "viewer", "expires_in_days": 1000}), json!({"role": "viewer", "max_uses": 0}), json!({"role": "viewer", "email": "not an email"})] {
        assert_eq!(call(&app, "POST", &format!("/shows/{fresh}/invites"), &owner, Some(body.clone())).await.0, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn access_changes_are_announced_to_the_person_concerned(pool: PgPool) {
    let app = app(pool.clone());
    let (_, owner) = user(&app, "owner").await;
    let (bob, _) = user(&app, "bob").await;
    let show = new_show(&app, &owner).await;
    let mut listener = sqlx::postgres::PgListener::connect_with(&pool).await.unwrap();
    listener.listen(crate::shows::NOTIFY_CHANNEL).await.unwrap();
    invite(&app, &owner, show, json!({"role": "viewer", "email": "bob@example.com"})).await;
    let heard = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv()).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(heard.payload()).unwrap(), json!({"access": bob}));
    call(&app, "PATCH", &format!("/shows/{show}/members/{bob}"), &owner, Some(json!({"role": "editor"}))).await;
    let heard = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv()).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(heard.payload()).unwrap(), json!({"access": bob}));
    call(&app, "DELETE", &format!("/shows/{show}/members/{bob}"), &owner, None).await;
    let heard = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv()).await.unwrap().unwrap();
    assert_eq!(serde_json::from_str::<Value>(heard.payload()).unwrap(), json!({"access": bob}));
}
