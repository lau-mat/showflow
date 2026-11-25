use super::*;
use crate::auth::{self, AuthConfig};
use axum::{
    body::{to_bytes, Body},
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery staple";

fn app(pool: PgPool) -> Router {
    let config = AuthConfig::new("test-only-signing-key-with-at-least-32-bytes").unwrap();
    Router::new()
        .merge(auth::routes())
        .merge(routes())
        .with_state(auth::state(pool, config))
}

async fn call(app: &Router, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
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

/// Registers a user; returns (user id, access token).
async fn user(app: &Router, name: &str) -> (Uuid, String) {
    let (status, body) = call(
        app, "POST", "/auth/register", None,
        Some(json!({"username": name, "email": format!("{name}@example.com"), "password": PASSWORD})),
    ).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (body["user"]["id"].as_str().unwrap().parse().unwrap(), body["access_token"].as_str().unwrap().to_owned())
}

fn set(entity: &str, id: Uuid, fields: Value) -> Value {
    json!({"op_id": Uuid::new_v4(), "entity": entity, "id": id, "kind": "set", "fields": fields})
}

fn delete(entity: &str, id: Uuid) -> Value {
    json!({"op_id": Uuid::new_v4(), "entity": entity, "id": id, "kind": "delete"})
}

fn line_after(id: Uuid, name: &str, after: Option<Uuid>) -> Value {
    let mut op = set("line", id, json!({"name": name, "note": "", "duration": 60}));
    op["place"] = json!({"after": after});
    op
}

async fn push(app: &Router, token: &str, show: Uuid, ops: Vec<Value>) -> (StatusCode, Value) {
    call(app, "POST", &format!("/shows/{show}/ops"), Some(token), Some(json!({"ops": ops}))).await
}

async fn new_show(app: &Router, token: &str, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let (status, body) = push(app, token, id, vec![set("show", id, json!({"name": name, "time": 1_800_000_000}))]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["results"][0]["status"], "applied", "{body}");
    id
}

async fn snapshot(app: &Router, token: &str, show: Uuid) -> Value {
    let (status, body) = call(app, "GET", &format!("/shows/{show}"), Some(token), None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body
}

fn line_names(snapshot: &Value) -> Vec<String> {
    snapshot["lines"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().to_owned()).collect()
}

async fn add_member(pool: &PgPool, show: Uuid, user: Uuid, role: &str) {
    sqlx::query("INSERT INTO show_members (show_id, user_id, role) VALUES ($1, $2, $3)")
        .bind(show).bind(user).bind(role).execute(pool).await.unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn creating_a_show_makes_the_caller_its_owner(pool: PgPool) {
    let app = app(pool.clone());
    let (alice, token) = user(&app, "alice").await;
    let show = new_show(&app, &token, "Gala").await;

    let snap = snapshot(&app, &token, show).await;
    assert_eq!(snap["show"]["name"], "Gala");
    assert_eq!(snap["show"]["role"], "owner");
    assert_eq!(snap["rev"], 1);
    let (_, list) = call(&app, "GET", "/shows", Some(&token), None).await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["id"], show.to_string());
    assert_eq!(list[0]["rev"], 1);
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM show_members WHERE show_id = $1 AND role = 'owner'")
        .bind(show).fetch_one(&pool).await.unwrap();
    assert_eq!(owner, alice);

    // Only a show-creating op may open an unknown show, and only with the show's own id.
    let unknown = Uuid::new_v4();
    assert_eq!(push(&app, &token, unknown, vec![set("role", Uuid::new_v4(), json!({"name": "Audio"}))]).await.0, StatusCode::NOT_FOUND);
    assert_eq!(push(&app, &token, unknown, vec![set("show", Uuid::new_v4(), json!({"name": "x", "time": 1}))]).await.0, StatusCode::NOT_FOUND);
    assert_eq!(push(&app, &token, unknown, vec![set("show", unknown, json!({"name": "x"}))]).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(call(&app, "GET", "/shows", None, None).await.0, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn ops_are_idempotent_and_revs_are_gapless(pool: PgPool) {
    let app = app(pool.clone());
    let (_, token) = user(&app, "alice").await;
    let show = new_show(&app, &token, "Gala").await;
    let role = Uuid::new_v4();
    let line = Uuid::new_v4();
    let batch = vec![set("role", role, json!({"name": "Audio"})), line_after(line, "Intro", None)];

    let (_, first) = push(&app, &token, show, batch.clone()).await;
    assert_eq!(first["results"][0], json!({"status": "applied", "op_id": batch[0]["op_id"], "rev": 2}));
    assert_eq!(first["results"][1]["rev"], 3);
    assert!(first["results"][1]["position"].is_string(), "the resolved position is reported");
    assert_eq!(first["rev"], 3);

    // A retry (lost response) changes nothing and points at where the ops were recorded.
    let (_, again) = push(&app, &token, show, batch.clone()).await;
    assert_eq!(again["results"][0]["status"], "duplicate");
    assert_eq!(again["results"][0]["rev"], 2);
    assert_eq!(again["results"][1]["rev"], 3);
    assert_eq!(again["rev"], 3);
    let lines: i64 = sqlx::query_scalar("SELECT count(*) FROM show_lines").fetch_one(&pool).await.unwrap();
    assert_eq!(lines, 1);

    let (status, pulled) = call(&app, "GET", &format!("/shows/{show}/changes?since=1"), Some(&token), None).await;
    assert_eq!(status, StatusCode::OK);
    let revs: Vec<i64> = pulled["changes"].as_array().unwrap().iter().map(|c| c["rev"].as_i64().unwrap()).collect();
    assert_eq!(revs, [2, 3]);
    assert_eq!(pulled["rev"], 3);
    assert_eq!(pulled["more"], false);
    // The recorded change carries the concrete position, never a `place`.
    assert!(pulled["changes"][1]["fields"]["position"].is_string());
    assert!(pulled["changes"][1].get("place").is_none());
    let (_, nothing) = call(&app, "GET", &format!("/shows/{show}/changes?since=3"), Some(&token), None).await;
    assert!(nothing["changes"].as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn concurrent_edits_merge_per_field_and_the_latest_write_wins(pool: PgPool) {
    let app = app(pool.clone());
    let (_, alice) = user(&app, "alice").await;
    let (bob_id, bob) = user(&app, "bob").await;
    let show = new_show(&app, &alice, "Gala").await;
    add_member(&pool, show, bob_id, "editor").await;
    let line = Uuid::new_v4();
    push(&app, &alice, show, vec![line_after(line, "Intro", None)]).await;

    // Both start from the same state and edit different fields: neither loses anything.
    push(&app, &alice, show, vec![set("line", line, json!({"note": "alice's note"}))]).await;
    push(&app, &bob, show, vec![set("line", line, json!({"duration": 99}))]).await;
    let snap = snapshot(&app, &alice, show).await;
    assert_eq!(snap["lines"][0]["note"], "alice's note");
    assert_eq!(snap["lines"][0]["duration"], 99);
    assert_eq!(snap["lines"][0]["name"], "Intro");

    // Same field: the one applied last wins.
    push(&app, &alice, show, vec![set("line", line, json!({"name": "Alice's title"}))]).await;
    push(&app, &bob, show, vec![set("line", line, json!({"name": "Bob's title"}))]).await;
    assert_eq!(snapshot(&app, &alice, show).await["lines"][0]["name"], "Bob's title");

    // Authors are recorded.
    let (_, pulled) = call(&app, "GET", &format!("/shows/{show}/changes?since=0"), Some(&alice), None).await;
    assert_eq!(pulled["changes"].as_array().unwrap().last().unwrap()["author_id"], bob_id.to_string());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn deletes_win_over_concurrent_edits_and_hide_the_children(pool: PgPool) {
    let app = app(pool.clone());
    let (_, token) = user(&app, "alice").await;
    let show = new_show(&app, &token, "Gala").await;
    let line = Uuid::new_v4();
    let comment = Uuid::new_v4();
    push(&app, &token, show, vec![
        line_after(line, "Intro", None),
        set("comment", comment, json!({"line_id": line, "comment": "careful", "role_id": null})),
    ]).await;
    assert_eq!(snapshot(&app, &token, show).await["comments"].as_array().unwrap().len(), 1);

    push(&app, &token, show, vec![delete("line", line)]).await;
    // An edit that was made before the delete was seen still applies (and is logged) ...
    let (_, late) = push(&app, &token, show, vec![set("line", line, json!({"note": "late edit"}))]).await;
    assert_eq!(late["results"][0]["status"], "applied");
    // ... but the line stays deleted, and so do its comments.
    let snap = snapshot(&app, &token, show).await;
    assert!(snap["lines"].as_array().unwrap().is_empty());
    assert!(snap["comments"].as_array().unwrap().is_empty());
    let tombstone: Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar("SELECT deleted_at FROM show_lines WHERE line_id = $1")
        .bind(line).fetch_one(&pool).await.unwrap();
    assert!(tombstone.is_some());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn lines_keep_their_order_when_people_insert_at_the_same_spot(pool: PgPool) {
    let app = app(pool.clone());
    let (alice_id, alice) = user(&app, "alice").await;
    let (bob_id, bob) = user(&app, "bob").await;
    let _ = alice_id;
    let show = new_show(&app, &alice, "Gala").await;
    add_member(&pool, show, bob_id, "editor").await;
    let (a, c) = (Uuid::new_v4(), Uuid::new_v4());
    push(&app, &alice, show, vec![line_after(a, "A", None), line_after(c, "C", Some(a))]).await;

    // Both insert right after A before seeing each other's line.
    push(&app, &alice, show, vec![line_after(Uuid::new_v4(), "B-alice", Some(a))]).await;
    push(&app, &bob, show, vec![line_after(Uuid::new_v4(), "B-bob", Some(a))]).await;
    push(&app, &bob, show, vec![line_after(Uuid::new_v4(), "Start", None)]).await;
    let names = line_names(&snapshot(&app, &alice, show).await);
    assert_eq!((names.first().unwrap().as_str(), names.last().unwrap().as_str()), ("Start", "C"));
    assert_eq!(names.len(), 5);
    assert_eq!(names[1], "A");
    assert!(names[2..4].contains(&"B-alice".to_owned()) && names[2..4].contains(&"B-bob".to_owned()));
    // The order is a pure function of the stored keys: asking again gives the same answer.
    assert_eq!(names, line_names(&snapshot(&app, &bob, show).await));

    // Placing after a line that is not in this show is rejected, not guessed.
    let (_, body) = push(&app, &alice, show, vec![line_after(Uuid::new_v4(), "Lost", Some(Uuid::new_v4()))]).await;
    assert_eq!(body["results"][0]["status"], "rejected");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn roles_decide_what_a_member_may_change(pool: PgPool) {
    let app = app(pool.clone());
    let (_, owner) = user(&app, "owner").await;
    let show = new_show(&app, &owner, "Gala").await;
    let line = Uuid::new_v4();
    push(&app, &owner, show, vec![line_after(line, "Intro", None)]).await;

    let mut members = Vec::new();
    for role in ["editor", "commenter", "viewer"] {
        let (id, token) = user(&app, role).await;
        add_member(&pool, show, id, role).await;
        members.push((role, token));
    }
    let status = |body: &Value| body["results"][0]["status"].as_str().unwrap().to_owned();
    for (role, token) in &members {
        let (code, body) = push(&app, token, show, vec![set("comment", Uuid::new_v4(), json!({"line_id": line, "comment": "hi"}))]).await;
        assert_eq!(code, StatusCode::OK);
        assert_eq!(status(&body), if *role == "viewer" { "rejected" } else { "applied" }, "{role} comment");

        let (_, body) = push(&app, token, show, vec![set("line", line, json!({"note": role}))]).await;
        assert_eq!(status(&body), if *role == "editor" { "applied" } else { "rejected" }, "{role} line edit");

        let (_, body) = push(&app, token, show, vec![set("role", Uuid::new_v4(), json!({"name": role}))]).await;
        assert_eq!(status(&body), if *role == "editor" { "applied" } else { "rejected" }, "{role} role create");
        assert_eq!(snapshot(&app, token, show).await["show"]["role"], *role, "everyone can read");

        let (_, body) = push(&app, token, show, vec![delete("show", show)]).await;
        assert_eq!(status(&body), "rejected", "{role} may not delete the show");
    }
    assert!(snapshot(&app, &owner, show).await["show"]["name"].is_string());

    // Strangers learn nothing: not the show, not its changes, not that it exists.
    let (_, stranger) = user(&app, "stranger").await;
    assert_eq!(call(&app, "GET", &format!("/shows/{show}"), Some(&stranger), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(call(&app, "GET", &format!("/shows/{show}/changes"), Some(&stranger), None).await.0, StatusCode::NOT_FOUND);
    assert_eq!(push(&app, &stranger, show, vec![set("show", show, json!({"name": "mine now"}))]).await.0, StatusCode::NOT_FOUND);
    let (_, list) = call(&app, "GET", "/shows", Some(&stranger), None).await;
    assert!(list.as_array().unwrap().is_empty());

    // The owner can delete; afterwards nobody can write and the list drops it.
    let (_, body) = push(&app, &owner, show, vec![delete("show", show)]).await;
    assert_eq!(status(&body), "applied");
    assert_eq!(push(&app, &owner, show, vec![set("show", show, json!({"name": "again"}))]).await.0, StatusCode::GONE);
    assert_eq!(call(&app, "GET", &format!("/shows/{show}"), Some(&owner), None).await.0, StatusCode::NOT_FOUND);
    let (_, pulled) = call(&app, "GET", &format!("/shows/{show}/changes?since=0"), Some(&owner), None).await;
    assert_eq!(pulled["deleted"], true, "members still learn that it is gone");
    let (_, list) = call(&app, "GET", "/shows", Some(&owner), None).await;
    assert!(list.as_array().unwrap().is_empty());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn ids_and_references_cannot_reach_into_other_shows(pool: PgPool) {
    let app = app(pool.clone());
    let (_, alice) = user(&app, "alice").await;
    let (_, bob) = user(&app, "bob").await;
    let alices = new_show(&app, &alice, "Alice's show").await;
    let bobs = new_show(&app, &bob, "Bob's show").await;
    let (alice_line, bob_line, alice_role) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    push(&app, &alice, alices, vec![line_after(alice_line, "secret", None), set("role", alice_role, json!({"name": "Audio"}))]).await;
    push(&app, &bob, bobs, vec![line_after(bob_line, "mine", None)]).await;
    let status = |body: &Value| body["results"][0]["status"].as_str().unwrap().to_owned();

    // Taking over an id that exists in someone else's show.
    let (_, body) = push(&app, &bob, bobs, vec![set("line", alice_line, json!({"name": "hijacked"}))]).await;
    assert_eq!(status(&body), "rejected");
    let (_, body) = push(&app, &bob, bobs, vec![delete("line", alice_line)]).await;
    assert_eq!(status(&body), "rejected");
    assert_eq!(snapshot(&app, &alice, alices).await["lines"][0]["name"], "secret");

    // Comments only attach to lines and roles of the same show.
    let (_, body) = push(&app, &bob, bobs, vec![set("comment", Uuid::new_v4(), json!({"line_id": alice_line, "comment": "x"}))]).await;
    assert_eq!(status(&body), "rejected");
    let (_, body) = push(&app, &bob, bobs, vec![set("comment", Uuid::new_v4(), json!({"line_id": bob_line, "comment": "x", "role_id": alice_role}))]).await;
    assert_eq!(status(&body), "rejected");
    // A show op may only target its own show.
    let (_, body) = push(&app, &bob, bobs, vec![set("show", alices, json!({"name": "mine"}))]).await;
    assert_eq!(status(&body), "rejected");
    assert_eq!(snapshot(&app, &alice, alices).await["show"]["name"], "Alice's show");
    // An id already used by another entity cannot become a show.
    assert_eq!(push(&app, &bob, alice_line, vec![set("show", alice_line, json!({"name": "x", "time": 1}))]).await.0, StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn one_bad_op_does_not_spoil_the_rest_of_the_batch(pool: PgPool) {
    let app = app(pool.clone());
    let (_, token) = user(&app, "alice").await;
    let show = new_show(&app, &token, "Gala").await;
    let (good, bad) = (Uuid::new_v4(), Uuid::new_v4());
    let (status, body) = push(&app, &token, show, vec![
        line_after(good, "good", None),
        set("line", bad, json!({"name": "bad", "duration": -5, "note": ""})),
        set("role", Uuid::new_v4(), json!({"bogus": 1})),
        delete("line", Uuid::new_v4()),
        set("role", Uuid::new_v4(), json!({"name": "Audio"})),
    ]).await;
    assert_eq!(status, StatusCode::OK);
    let statuses: Vec<&str> = body["results"].as_array().unwrap().iter().map(|r| r["status"].as_str().unwrap()).collect();
    assert_eq!(statuses, ["applied", "rejected", "rejected", "rejected", "applied"]);
    assert_eq!(body["rev"], 3, "only applied ops take a revision: 1 (show) + 2");
    assert_eq!(body["results"][4]["rev"], 3);
    assert_eq!(line_names(&snapshot(&app, &token, show).await), ["good"]);
    assert_eq!(push(&app, &token, show, vec![]).await.0, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn parallel_pushes_to_one_show_get_distinct_consecutive_revisions(pool: PgPool) {
    let app = app(pool.clone());
    let (_, token) = user(&app, "alice").await;
    let show = new_show(&app, &token, "Gala").await;
    let mut tasks = Vec::new();
    for i in 0..8 {
        let (app, token) = (app.clone(), token.clone());
        tasks.push(tokio::spawn(async move {
            push(&app, &token, show, vec![set("role", Uuid::new_v4(), json!({"name": format!("role {i}")}))]).await
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().1["results"][0]["status"], "applied");
    }
    let revs: Vec<i64> = sqlx::query_scalar("SELECT rev FROM show_changes WHERE show_id = $1 ORDER BY rev")
        .bind(show).fetch_all(&pool).await.unwrap();
    assert_eq!(revs, (1..=9).collect::<Vec<_>>());
    let stored: i64 = sqlx::query_scalar("SELECT rev FROM shows WHERE show_id = $1").bind(show).fetch_one(&pool).await.unwrap();
    assert_eq!(stored, 9);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn committed_changes_are_announced_and_rejected_ones_are_not(pool: PgPool) {
    let app = app(pool.clone());
    let (_, token) = user(&app, "alice").await;
    let mut listener = sqlx::postgres::PgListener::connect_with(&pool).await.unwrap();
    listener.listen(NOTIFY_CHANNEL).await.unwrap();

    let show = new_show(&app, &token, "Gala").await;
    // Creating a show first announces that the creator's access changed, then the change itself.
    let mut payloads = Vec::new();
    for _ in 0..2 {
        let notification = tokio::time::timeout(std::time::Duration::from_secs(5), listener.recv()).await.unwrap().unwrap();
        payloads.push(serde_json::from_str::<Value>(notification.payload()).unwrap());
    }
    assert!(payloads[0].get("access").is_some(), "{payloads:?}");
    assert_eq!(payloads[1], json!({"show_id": show, "rev": 1}));

    push(&app, &token, show, vec![set("role", Uuid::new_v4(), json!({"bogus": true}))]).await;
    assert!(tokio::time::timeout(std::time::Duration::from_millis(500), listener.recv()).await.is_err(),
        "a batch where nothing applied must not wake anyone");
}

#[test]
fn bind_types_follow_the_columns() {
    assert!(matches!(bind_for(EntityKind::Line, "absolute_time", &Value::Null), Ok(Bind::OptInt(None))));
    assert!(matches!(bind_for(EntityKind::Line, "duration", &json!(5)), Ok(Bind::Int(5))));
    assert!(matches!(bind_for(EntityKind::Comment, "role_id", &Value::Null), Ok(Bind::OptUuid(None))));
    assert!(bind_for(EntityKind::Comment, "line_id", &json!("nope")).is_err());
    assert!(bind_for(EntityKind::Show, "bogus", &json!(1)).is_err());
}
