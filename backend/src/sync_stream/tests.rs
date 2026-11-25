use super::*;
use crate::auth::{self, AuthConfig};
use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tower::ServiceExt;

#[test]
fn notifications_are_parsed_into_events() {
    let show = Uuid::new_v4();
    let user = Uuid::new_v4();
    assert_eq!(SyncEvent::parse(&json!({"show_id": show, "rev": 7}).to_string()), Some(SyncEvent::Show { show_id: show, rev: 7 }));
    assert_eq!(SyncEvent::parse(&json!({"access": user}).to_string()), Some(SyncEvent::Access { user_id: Some(user) }));
    assert_eq!(SyncEvent::parse(&json!({"access_all": true}).to_string()), Some(SyncEvent::Access { user_id: None }));
    for junk in ["", "nope", "{}", r#"{"show_id": "x", "rev": 1}"#, r#"{"show_id": null}"#] {
        assert_eq!(SyncEvent::parse(junk), None, "{junk}");
    }
}

#[test]
fn a_stream_only_announces_shows_its_user_can_see() {
    let (mine, theirs, me, other) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let visible: HashSet<Uuid> = [mine].into();
    assert_eq!(route(&SyncEvent::Show { show_id: mine, rev: 3 }, &visible, me), Action::Announce { show_id: mine, rev: 3 });
    assert_eq!(route(&SyncEvent::Show { show_id: theirs, rev: 3 }, &visible, me), Action::Ignore);
    assert_eq!(route(&SyncEvent::Access { user_id: Some(me) }, &visible, me), Action::ReloadAccess);
    assert_eq!(route(&SyncEvent::Access { user_id: Some(other) }, &visible, me), Action::Ignore);
    assert_eq!(route(&SyncEvent::Access { user_id: None }, &visible, me), Action::ReloadAccess);
}

#[test]
fn a_stream_lives_exactly_as_long_as_its_access_token() {
    assert_eq!(lifetime(1000, 400), Duration::from_secs(600));
    assert_eq!(lifetime(1000, 1000), Duration::ZERO);
    assert_eq!(lifetime(1000, 5000), Duration::ZERO, "an expired token never opens a long stream");
}

#[tokio::test]
async fn the_hub_reaches_every_subscriber_and_tolerates_having_none() {
    let hub = SyncHub::new();
    hub.publish(SyncEvent::Access { user_id: None });
    let (mut a, mut b) = (hub.subscribe(), hub.subscribe());
    hub.publish(SyncEvent::Show { show_id: Uuid::nil(), rev: 1 });
    assert_eq!(a.recv().await.unwrap(), SyncEvent::Show { show_id: Uuid::nil(), rev: 1 });
    assert_eq!(b.recv().await.unwrap(), SyncEvent::Show { show_id: Uuid::nil(), rev: 1 });
}

// ---- end to end against a database: write through the API, hear about it on a stream

fn app(pool: PgPool) -> Router {
    let config = AuthConfig::new("test-only-signing-key-with-at-least-32-bytes").unwrap();
    let state = auth::state(pool.clone(), config);
    spawn_listener(pool, state.hub.clone());
    Router::new()
        .merge(auth::routes())
        .merge(crate::shows::routes())
        .merge(routes())
        .with_state(state)
}

async fn json_call(app: &Router, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
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
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, if bytes.is_empty() { Value::Null } else { serde_json::from_slice(&bytes).unwrap() })
}

async fn register(app: &Router, name: &str) -> String {
    let (status, body) = json_call(app, "POST", "/auth/register", None, Some(json!({
        "username": name, "email": format!("{name}@example.com"), "password": "correct horse battery staple"
    }))).await;
    assert_eq!(status, StatusCode::CREATED);
    body["access_token"].as_str().unwrap().to_owned()
}

async fn create_show(app: &Router, token: &str) -> Uuid {
    let id = Uuid::new_v4();
    let op = json!({"op_id": Uuid::new_v4(), "entity": "show", "id": id, "kind": "set", "fields": {"name": "Gala", "time": 1}});
    let (status, _) = json_call(app, "POST", &format!("/shows/{id}/ops"), Some(token), Some(json!({"ops": [op]}))).await;
    assert_eq!(status, StatusCode::OK);
    id
}

/// Reads server-sent events until one named `name` arrives.
async fn next_event(body: &mut (impl futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin), name: &str, wait: Duration) -> Option<String> {
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let chunk = tokio::time::timeout_at(deadline, body.next()).await.ok()??.ok()?;
        seen.push_str(&String::from_utf8_lossy(&chunk));
        for block in seen.split("\n\n") {
            if block.lines().any(|line| line == format!("event: {name}")) {
                return block.lines().find_map(|l| l.strip_prefix("data: ")).map(str::to_owned);
            }
        }
    }
}

/// The first `show` event announcing at least `rev` of `show` (earlier announcements may still be in flight).
async fn show_at_least(body: &mut (impl futures_util::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin), show: Uuid, rev: i64) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.checked_duration_since(tokio::time::Instant::now())?;
        let data: Value = serde_json::from_str(&next_event(body, "show", left).await?).ok()?;
        if data["show_id"] == show.to_string() && data["rev"].as_i64()? >= rev {
            return Some(data);
        }
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires a disposable PostgreSQL DATABASE_URL with CREATE DATABASE privilege"]
async fn a_change_is_announced_to_members_only(pool: PgPool) {
    let app = app(pool.clone());
    let alice = register(&app, "alice").await;
    let bob = register(&app, "bob").await;
    let show = create_show(&app, &alice).await;

    let open = |token: String| {
        let app = app.clone();
        async move {
            let response = app.oneshot(Request::get("/sync/stream").header(header::AUTHORIZATION, format!("Bearer {token}")).body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "text/event-stream");
            assert_eq!(response.headers()["x-accel-buffering"], "no");
            response.into_body().into_data_stream()
        }
    };
    let mut alice_stream = open(alice.clone()).await;
    let mut bob_stream = open(bob.clone()).await;
    assert!(next_event(&mut alice_stream, "ready", Duration::from_secs(5)).await.is_some(), "a stream starts with a ready event");
    assert!(next_event(&mut bob_stream, "ready", Duration::from_secs(5)).await.is_some());
    // Let the listener connect before writing.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let role = json!({"op_id": Uuid::new_v4(), "entity": "role", "id": Uuid::new_v4(), "kind": "set", "fields": {"name": "Audio"}});
    json_call(&app, "POST", &format!("/shows/{show}/ops"), Some(&alice), Some(json!({"ops": [role]}))).await;
    let data = show_at_least(&mut alice_stream, show, 2).await.expect("the writer's stream hears it too");
    assert_eq!(data, json!({"show_id": show, "rev": 2}));
    assert!(next_event(&mut bob_stream, "show", Duration::from_millis(800)).await.is_none(), "bob is not a member, so he hears nothing");

    // Make bob a member: the access event arrives only after something announces it, then he hears changes.
    sqlx::query("INSERT INTO show_members (show_id, user_id, role) SELECT $1, user_id, 'viewer' FROM users WHERE user_name = 'bob'")
        .bind(show).execute(&pool).await.unwrap();
    let bob_id: Uuid = sqlx::query_scalar("SELECT user_id FROM users WHERE user_name = 'bob'").fetch_one(&pool).await.unwrap();
    sqlx::query("SELECT pg_notify($1, $2)").bind(NOTIFY_CHANNEL).bind(json!({"access": bob_id}).to_string()).execute(&pool).await.unwrap();
    assert!(next_event(&mut bob_stream, "access", Duration::from_secs(5)).await.is_some());
    let role = json!({"op_id": Uuid::new_v4(), "entity": "role", "id": Uuid::new_v4(), "kind": "set", "fields": {"name": "Video"}});
    json_call(&app, "POST", &format!("/shows/{show}/ops"), Some(&alice), Some(json!({"ops": [role]}))).await;
    let data = show_at_least(&mut bob_stream, show, 3).await.expect("bob now hears changes");
    assert_eq!(data["rev"], 3);

    // A show created while the stream is open is announced too (its creator was not a member when
    // the stream started).
    let second = create_show(&app, &alice).await;
    assert!(next_event(&mut alice_stream, "access", Duration::from_secs(5)).await.is_some(), "membership changed");
    let data = show_at_least(&mut alice_stream, second, 1).await.expect("and so is the new show's first change");
    assert_eq!(data["show_id"], second.to_string());

    // No token, no stream.
    let rejected = app.clone().oneshot(Request::get("/sync/stream").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
}
