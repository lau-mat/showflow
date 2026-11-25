//! Live updates: `GET /sync/stream` is a server-sent-events stream that tells a client *that*
//! a show it can see has a new revision (and when its list of shows changed). It never carries
//! content: the client then pulls through the normal endpoints, so authorization lives in one
//! place and a missed event costs nothing (clients also poll slowly and pull on reconnect).
//!
//! Instances of the backend find out about each other's writes through Postgres
//! `LISTEN/NOTIFY` (`pg_notify` runs inside the writing transaction, so listeners only hear
//! about committed changes), and rebroadcast to their own connections through `SyncHub`.

use std::{collections::HashSet, convert::Infallible, time::Duration};

use axum::{
    extract::State,
    http::{header, HeaderName, HeaderValue},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::get,
    Router,
};
use chrono::Utc;
use futures_util::stream;
use serde::Deserialize;
use sqlx::{postgres::PgListener, PgPool};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::api_error::ApiError;
use crate::auth::{AppState, AuthenticatedSession};
use crate::shows::NOTIFY_CHANNEL;

/// What the database announced.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncEvent {
    /// A show moved to a new revision.
    Show { show_id: Uuid, rev: i64 },
    /// Who may see which shows changed (`None`: for some set of users, re-check everyone).
    Access { user_id: Option<Uuid> },
}

#[derive(Deserialize)]
struct Notification {
    show_id: Option<Uuid>,
    rev: Option<i64>,
    access: Option<Uuid>,
    #[serde(default)]
    access_all: bool,
}

impl SyncEvent {
    pub fn parse(payload: &str) -> Option<Self> {
        let n: Notification = serde_json::from_str(payload).ok()?;
        match (n.show_id, n.rev, n.access, n.access_all) {
            (Some(show_id), Some(rev), _, _) => Some(Self::Show { show_id, rev }),
            (_, _, Some(user), _) => Some(Self::Access { user_id: Some(user) }),
            (_, _, _, true) => Some(Self::Access { user_id: None }),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct SyncHub {
    sender: broadcast::Sender<SyncEvent>,
}

impl SyncHub {
    pub fn new() -> Self {
        Self { sender: broadcast::channel(1024).0 }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SyncEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: SyncEvent) {
        // Nobody listening is fine.
        let _ = self.sender.send(event);
    }
}

/// Runs for the life of the process: forwards database notifications to the hub, reconnecting
/// (with a pause) whenever the connection drops. Events missed meanwhile are covered by the
/// clients' slow poll and by the pull each of them does when its stream reconnects.
pub fn spawn_listener(pool: PgPool, hub: SyncHub) {
    tokio::spawn(async move {
        loop {
            match listen(&pool, &hub).await {
                Ok(()) => {}
                Err(error) => eprintln!("Sync listener stopped: {error}"),
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn listen(pool: &PgPool, hub: &SyncHub) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.listen(NOTIFY_CHANNEL).await?;
    loop {
        let notification = listener.recv().await?;
        if let Some(event) = SyncEvent::parse(notification.payload()) {
            hub.publish(event);
        }
    }
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/sync/stream", get(stream_events))
}

/// What a stream should do with an event, given what its user may see.
#[derive(Debug, PartialEq)]
pub enum Action {
    Ignore,
    /// Tell the client this show moved to `rev`.
    Announce { show_id: Uuid, rev: i64 },
    /// The user's visible shows changed: reload the access set and tell the client.
    ReloadAccess,
}

pub fn route(event: &SyncEvent, visible: &HashSet<Uuid>, user_id: Uuid) -> Action {
    match event {
        SyncEvent::Show { show_id, rev } if visible.contains(show_id) => Action::Announce { show_id: *show_id, rev: *rev },
        SyncEvent::Show { .. } => Action::Ignore,
        SyncEvent::Access { user_id: None } => Action::ReloadAccess,
        SyncEvent::Access { user_id: Some(id) } if *id == user_id => Action::ReloadAccess,
        SyncEvent::Access { .. } => Action::Ignore,
    }
}

/// How long a stream may live: until its access token expires (the client then reconnects with a
/// fresh one), so a revoked login cannot keep listening.
pub fn lifetime(expires_at: i64, now: i64) -> Duration {
    Duration::from_secs((expires_at - now).max(0) as u64)
}

async fn visible_shows(pool: &PgPool, user_id: Uuid) -> Result<HashSet<Uuid>, sqlx::Error> {
    // Deleted shows stay visible so that members hear about the deletion.
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT show_id FROM show_members WHERE user_id = $1")
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(ids.into_iter().collect())
}

struct StreamState {
    receiver: broadcast::Receiver<SyncEvent>,
    pool: PgPool,
    user_id: Uuid,
    visible: HashSet<Uuid>,
    deadline: tokio::time::Instant,
    sent_ready: bool,
}

async fn stream_events(
    State(state): State<AppState>,
    session: AuthenticatedSession,
) -> Result<impl IntoResponse, ApiError> {
    // Subscribe first, then read the access set: a change in between is seen either way.
    let receiver = state.hub.subscribe();
    let visible = visible_shows(&state.pool, session.user_id).await?;
    let initial = StreamState {
        receiver,
        pool: state.pool.clone(),
        user_id: session.user_id,
        visible,
        deadline: tokio::time::Instant::now() + lifetime(session.expires_at, Utc::now().timestamp()),
        sent_ready: false,
    };

    let events = stream::unfold(initial, |mut st| async move {
        if !st.sent_ready {
            st.sent_ready = true;
            // The client syncs once on every (re)connect, so nothing that happened while it was
            // disconnected is missed.
            return Some((Ok::<_, Infallible>(Event::default().event("ready").data("{}")), st));
        }
        loop {
            let received = tokio::select! {
                received = st.receiver.recv() => received,
                _ = tokio::time::sleep_until(st.deadline) => return None,
            };
            match received {
                Ok(event) => match route(&event, &st.visible, st.user_id) {
                    Action::Ignore => continue,
                    Action::Announce { show_id, rev } => {
                        let data = serde_json::json!({ "show_id": show_id, "rev": rev }).to_string();
                        return Some((Ok(Event::default().event("show").data(data)), st));
                    }
                    Action::ReloadAccess => {
                        match visible_shows(&st.pool, st.user_id).await {
                            Ok(visible) => st.visible = visible,
                            Err(error) => eprintln!("Could not reload stream access: {error}"),
                        }
                        return Some((Ok(Event::default().event("access").data("{}")), st));
                    }
                },
                // Fell behind: tell the client to do a full sync instead of guessing what it missed.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    return Some((Ok(Event::default().event("resync").data("{}")), st));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let sse = Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(25)).text("ping"));
    let headers = [
        (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        // Tell nginx not to hold the events back.
        (HeaderName::from_static("x-accel-buffering"), HeaderValue::from_static("no")),
    ];
    Ok((headers, sse))
}

#[cfg(test)]
mod tests;
