//! Shows over HTTP: list, snapshot, pull changes, and the one write path, `POST /shows/:id/ops`.
//!
//! Every edit is an op (`protocol::sync`). Ops of one show are applied one after another under
//! a lock on the show row, so each gets the next `rev` and the last writer of a field wins.
//! Each op runs in its own savepoint: a rejected op leaves the others in the batch untouched.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use protocol::position;
use protocol::show::{MemberRole, ScenarioLine, ScenarioLineComment, Show, ShowDetails, ShowRole};
use protocol::sync::{Change, EntityKind, Op, OpKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{postgres::PgRow, PgConnection, PgPool, QueryBuilder, Row};
use uuid::Uuid;

use crate::api_error::ApiError;
use crate::auth::{client::ClientInfo, AppState, AuthenticatedUser};

/// Most ops accepted in one request.
const MAX_OPS_PER_REQUEST: usize = 500;
/// Most changes returned by one pull; the response says when there are more.
const MAX_CHANGES_PER_PULL: i64 = 500;
/// Postgres `NOTIFY` channel announcing that a show moved to a new revision.
pub const NOTIFY_CHANNEL: &str = "showflow_sync";

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/shows", get(list_shows))
        .route("/shows/:id", get(snapshot))
        .route("/shows/:id/ops", post(push_ops))
        .route("/shows/:id/changes", get(pull_changes))
}

// ---------------------------------------------------------------------------------------------
// Entity tables
// ---------------------------------------------------------------------------------------------

struct Table {
    name: &'static str,
    pk: &'static str,
    /// `(op field, column)`.
    columns: &'static [(&'static str, &'static str)],
}

fn table(entity: EntityKind) -> Table {
    match entity {
        EntityKind::Show => Table { name: "shows", pk: "show_id", columns: &[("name", "name"), ("time", "show_time")] },
        EntityKind::Role => Table { name: "show_roles", pk: "role_id", columns: &[("name", "name")] },
        EntityKind::Line => Table {
            name: "show_lines",
            pk: "line_id",
            columns: &[
                ("name", "name"),
                ("note", "note"),
                ("duration", "duration"),
                ("absolute_time", "absolute_time"),
                ("position", "position"),
            ],
        },
        EntityKind::Comment => Table {
            name: "show_comments",
            pk: "comment_id",
            columns: &[("line_id", "line_id"), ("role_id", "role_id"), ("comment", "comment")],
        },
    }
}

/// A validated JSON value, typed for the column it goes into.
enum Bind {
    Text(String),
    Int(i64),
    OptInt(Option<i64>),
    Uuid(Uuid),
    OptUuid(Option<Uuid>),
}

fn bind_for(entity: EntityKind, field: &str, value: &Value) -> Result<Bind, ApiError> {
    let bad = || ApiError::BadRequest(format!("Invalid value for {field}"));
    let uuid = |v: &Value| v.as_str().and_then(|s| Uuid::parse_str(s).ok()).ok_or_else(bad);
    Ok(match (entity, field) {
        (_, "name") | (EntityKind::Line, "note" | "position") | (EntityKind::Comment, "comment") => {
            Bind::Text(value.as_str().ok_or_else(bad)?.to_owned())
        }
        (EntityKind::Show, "time") | (EntityKind::Line, "duration") => Bind::Int(value.as_i64().ok_or_else(bad)?),
        (EntityKind::Line, "absolute_time") => Bind::OptInt(if value.is_null() { None } else { Some(value.as_i64().ok_or_else(bad)?) }),
        (EntityKind::Comment, "line_id") => Bind::Uuid(uuid(value)?),
        (EntityKind::Comment, "role_id") => Bind::OptUuid(if value.is_null() { None } else { Some(uuid(value)?) }),
        _ => return Err(bad()),
    })
}

fn push_bind(builder: &mut QueryBuilder<'_, sqlx::Postgres>, bind: Bind) {
    match bind {
        Bind::Text(v) => builder.push_bind(v),
        Bind::Int(v) => builder.push_bind(v),
        Bind::OptInt(v) => builder.push_bind(v),
        Bind::Uuid(v) => builder.push_bind(v),
        Bind::OptUuid(v) => builder.push_bind(v),
    };
}

// ---------------------------------------------------------------------------------------------
// Access
// ---------------------------------------------------------------------------------------------

pub(crate) async fn member_role(
    conn: &mut PgConnection,
    show_id: Uuid,
    user_id: Uuid,
) -> Result<Option<MemberRole>, ApiError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM show_members WHERE show_id = $1 AND user_id = $2")
            .bind(show_id)
            .bind(user_id)
            .fetch_optional(conn)
            .await?;
    Ok(role.as_deref().and_then(MemberRole::parse))
}

async fn require_member(conn: &mut PgConnection, show_id: Uuid, user_id: Uuid) -> Result<MemberRole, ApiError> {
    member_role(conn, show_id, user_id).await?.ok_or(ApiError::NotFound)
}

// ---------------------------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
struct ShowListItem {
    #[serde(flatten)]
    show: Show,
    rev: i64,
    /// Everyone with access, and those who may call it in a live session. A session host keeps
    /// both to decide offline what the holder of a pass may do.
    members: Vec<Uuid>,
    callers: Vec<Uuid>,
}

async fn list_shows(
    State(state): State<AppState>,
    AuthenticatedUser(user_id): AuthenticatedUser,
) -> Result<Json<Vec<ShowListItem>>, ApiError> {
    let rows = sqlx::query(
        "SELECT s.show_id, s.name, s.show_time, s.rev, m.role,
                ARRAY(SELECT a.user_id FROM show_members a WHERE a.show_id = s.show_id ORDER BY a.user_id) AS members,
                ARRAY(SELECT c.user_id FROM show_members c
                      WHERE c.show_id = s.show_id AND (c.role = 'owner' OR c.can_call) ORDER BY c.user_id) AS callers
         FROM shows s JOIN show_members m ON m.show_id = s.show_id
         WHERE m.user_id = $1 AND s.deleted_at IS NULL
         ORDER BY s.show_time, s.show_id",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await?;
    let items = rows
        .into_iter()
        .map(|row: PgRow| {
            let role: String = row.get("role");
            ShowListItem {
                show: Show {
                    id: row.get::<Uuid, _>("show_id").to_string(),
                    name: row.get("name"),
                    time: row.get("show_time"),
                    role: MemberRole::parse(&role).unwrap_or(MemberRole::Viewer),
                },
                rev: row.get("rev"),
                members: row.get("members"),
                callers: row.get("callers"),
            }
        })
        .collect();
    Ok(Json(items))
}

pub(crate) async fn load_snapshot(pool: &PgPool, show_id: Uuid, user_id: Uuid) -> Result<ShowDetails, ApiError> {
    let mut conn = pool.acquire().await?;
    // One transaction so rev and rows agree even while edits are being applied.
    sqlx::query("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY").execute(&mut *conn).await?;
    let result = load_snapshot_in(&mut conn, show_id, user_id).await;
    sqlx::query("COMMIT").execute(&mut *conn).await?;
    result
}

async fn load_snapshot_in(conn: &mut PgConnection, show_id: Uuid, user_id: Uuid) -> Result<ShowDetails, ApiError> {
    let role = require_member(conn, show_id, user_id).await?;
    let show = sqlx::query("SELECT name, show_time, rev FROM shows WHERE show_id = $1 AND deleted_at IS NULL")
        .bind(show_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ApiError::NotFound)?;

    let roles = sqlx::query("SELECT role_id, name FROM show_roles WHERE show_id = $1 AND deleted_at IS NULL ORDER BY name, role_id")
        .bind(show_id)
        .fetch_all(&mut *conn)
        .await?
        .into_iter()
        .map(|row| ShowRole { id: row.get::<Uuid, _>("role_id").to_string(), show_id: show_id.to_string(), name: row.get("name") })
        .collect();

    let lines = sqlx::query(
        "SELECT line_id, position, name, note, duration, absolute_time FROM show_lines
         WHERE show_id = $1 AND deleted_at IS NULL ORDER BY position, line_id",
    )
    .bind(show_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| ScenarioLine {
        id: row.get::<Uuid, _>("line_id").to_string(),
        position: row.get("position"),
        name: row.get("name"),
        note: row.get("note"),
        duration: row.get("duration"),
        absolute_time: row.get("absolute_time"),
    })
    .collect();

    let comments = sqlx::query(
        "SELECT c.comment_id, c.line_id, c.role_id, c.comment FROM show_comments c
         JOIN show_lines l ON l.line_id = c.line_id
         WHERE c.show_id = $1 AND c.deleted_at IS NULL AND l.deleted_at IS NULL
         ORDER BY c.comment_id",
    )
    .bind(show_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(|row| ScenarioLineComment {
        id: row.get::<Uuid, _>("comment_id").to_string(),
        line_id: row.get::<Uuid, _>("line_id").to_string(),
        role_id: row.get::<Option<Uuid>, _>("role_id").map(|id| id.to_string()),
        comment: row.get("comment"),
    })
    .collect();

    Ok(ShowDetails {
        show: Show { id: show_id.to_string(), name: show.get("name"), time: show.get("show_time"), role },
        roles,
        lines,
        comments,
        rev: show.get("rev"),
    })
}

async fn snapshot(
    State(state): State<AppState>,
    AuthenticatedUser(user_id): AuthenticatedUser,
    Path(show_id): Path<Uuid>,
) -> Result<Json<ShowDetails>, ApiError> {
    Ok(Json(load_snapshot(&state.pool, show_id, user_id).await?))
}

#[derive(Deserialize)]
struct PullQuery {
    #[serde(default)]
    since: i64,
}

#[derive(Serialize)]
struct Pulled {
    changes: Vec<Change>,
    /// The show's current revision (the last change, which may be beyond this page).
    rev: i64,
    more: bool,
    /// True once the show has been deleted: the client should drop its copy.
    deleted: bool,
}

async fn pull_changes(
    State(state): State<AppState>,
    AuthenticatedUser(user_id): AuthenticatedUser,
    Path(show_id): Path<Uuid>,
    Query(query): Query<PullQuery>,
) -> Result<Json<Pulled>, ApiError> {
    let mut conn = state.pool.acquire().await?;
    require_member(&mut conn, show_id, user_id).await?;
    let show = sqlx::query("SELECT rev, deleted_at IS NOT NULL AS deleted FROM shows WHERE show_id = $1")
        .bind(show_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or(ApiError::NotFound)?;
    let rows = sqlx::query(
        "SELECT rev, op_id, entity, entity_id, kind, fields::text AS fields, author_id FROM show_changes
         WHERE show_id = $1 AND rev > $2 ORDER BY rev LIMIT $3",
    )
    .bind(show_id)
    .bind(query.since.max(0))
    .bind(MAX_CHANGES_PER_PULL + 1)
    .fetch_all(&mut *conn)
    .await?;
    let more = rows.len() as i64 > MAX_CHANGES_PER_PULL;
    let changes = rows
        .into_iter()
        .take(MAX_CHANGES_PER_PULL as usize)
        .map(|row| change_from_row(&row))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(Pulled { changes, rev: show.get("rev"), more, deleted: show.get("deleted") }))
}

fn change_from_row(row: &PgRow) -> Result<Change, ApiError> {
    let entity: String = row.get("entity");
    let kind: String = row.get("kind");
    let fields: String = row.get("fields");
    Ok(Change {
        rev: row.get("rev"),
        op_id: row.get("op_id"),
        entity: EntityKind::parse(&entity).ok_or_else(|| ApiError::internal("bad entity in change log"))?,
        id: row.get("entity_id"),
        kind: if kind == "delete" { OpKind::Delete } else { OpKind::Set },
        fields: serde_json::from_str(&fields).map_err(ApiError::internal)?,
        author_id: row.get("author_id"),
    })
}

// ---------------------------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct PushRequest {
    ops: Vec<Op>,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OpResult {
    /// Applied now as change `rev`. `position` is the key a `place` resolved to.
    Applied { op_id: Uuid, rev: i64, #[serde(skip_serializing_if = "Option::is_none")] position: Option<String> },
    /// Seen before (a retry): nothing happened, `rev` is where it was recorded.
    Duplicate { op_id: Uuid, rev: i64 },
    Rejected { op_id: Uuid, error: String },
}

#[derive(Serialize)]
struct Pushed {
    results: Vec<OpResult>,
    rev: i64,
}

async fn push_ops(
    State(state): State<AppState>,
    AuthenticatedUser(user_id): AuthenticatedUser,
    client: ClientInfo,
    Path(show_id): Path<Uuid>,
    Json(request): Json<PushRequest>,
) -> Result<Json<Pushed>, ApiError> {
    if request.ops.is_empty() || request.ops.len() > MAX_OPS_PER_REQUEST {
        return Err(ApiError::BadRequest(format!("Send between 1 and {MAX_OPS_PER_REQUEST} ops")));
    }
    let client_id = match client.device_id.as_deref().filter(|_| client.declared) {
        Some(identifier) => sqlx::query_scalar("SELECT client_id FROM client WHERE user_id = $1 AND client_identifier = $2")
            .bind(user_id)
            .bind(identifier)
            .fetch_optional(&state.pool)
            .await?,
        None => None,
    };
    let mut tx = state.pool.begin().await?;
    let pushed = apply_ops(&mut tx, show_id, user_id, client_id, request.ops).await?;
    tx.commit().await?;
    Ok(Json(pushed))
}

/// Applies a batch of ops to a show inside the caller's transaction.
async fn apply_ops(
    tx: &mut PgConnection,
    show_id: Uuid,
    user_id: Uuid,
    client_id: Option<Uuid>,
    ops: Vec<Op>,
) -> Result<Pushed, ApiError> {
    // Whoever holds this lock is the only one writing the show: that is what makes `rev` gapless.
    let mut show = sqlx::query("SELECT rev, deleted_at IS NOT NULL AS deleted FROM shows WHERE show_id = $1 FOR UPDATE")
        .bind(show_id)
        .fetch_optional(&mut *tx)
        .await?;

    if show.is_none() {
        // The first op of an unknown show creates it, and its creator owns it.
        let first = &ops[0];
        if first.entity != EntityKind::Show || first.kind != OpKind::Set || first.id != show_id {
            return Err(ApiError::NotFound);
        }
        first.validate_create().map_err(|e| ApiError::BadRequest(e.to_string()))?;
        // A show id that is already some other entity's id is not a free id.
        if entity_show(&mut *tx, EntityKind::Role, show_id).await?.is_some()
            || entity_show(&mut *tx, EntityKind::Line, show_id).await?.is_some()
            || entity_show(&mut *tx, EntityKind::Comment, show_id).await?.is_some()
        {
            return Err(ApiError::Conflict("That id is already in use"));
        }
        let inserted = sqlx::query(
            "INSERT INTO shows (show_id, name, show_time, created_by) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
        )
        .bind(show_id)
        .bind(first.fields["name"].as_str().unwrap_or_default())
        .bind(first.fields["time"].as_i64().unwrap_or_default())
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 1 {
            sqlx::query("INSERT INTO show_members (show_id, user_id, role, added_by) VALUES ($1, $2, 'owner', $2)")
                .bind(show_id)
                .bind(user_id)
                .execute(&mut *tx)
                .await?;
            // Live streams of this user learn about the new show before its first change.
            notify_access(&mut *tx, Some(user_id)).await?;
        }
        show = sqlx::query("SELECT rev, deleted_at IS NOT NULL AS deleted FROM shows WHERE show_id = $1 FOR UPDATE")
            .bind(show_id)
            .fetch_optional(&mut *tx)
            .await?;
    }
    let show = show.ok_or(ApiError::NotFound)?;
    let role = require_member(&mut *tx, show_id, user_id).await?;
    if show.get::<bool, _>("deleted") {
        return Err(ApiError::Gone("This show has been deleted"));
    }

    let mut rev: i64 = show.get("rev");
    let start_rev = rev;
    let mut results = Vec::with_capacity(ops.len());
    for op in ops {
        let op_id = op.op_id;
        let mut savepoint = sqlx::Connection::begin(&mut *tx).await?;
        match apply_one(&mut savepoint, show_id, user_id, role, client_id, op, rev + 1).await {
            Ok(Outcome::Applied { position }) => {
                savepoint.commit().await?;
                rev += 1;
                results.push(OpResult::Applied { op_id, rev, position });
            }
            Ok(Outcome::Duplicate { rev }) => {
                savepoint.commit().await?;
                results.push(OpResult::Duplicate { op_id, rev });
            }
            Err(error) => {
                savepoint.rollback().await?;
                results.push(OpResult::Rejected { op_id, error });
            }
        }
    }

    if rev != start_rev {
        sqlx::query("UPDATE shows SET rev = $2 WHERE show_id = $1")
            .bind(show_id)
            .bind(rev)
            .execute(&mut *tx)
            .await?;
        // Delivered when the transaction commits: listeners never hear about unsaved changes.
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(NOTIFY_CHANNEL)
            .bind(serde_json::json!({ "show_id": show_id, "rev": rev }).to_string())
            .execute(&mut *tx)
            .await?;
    }
    Ok(Pushed { results, rev })
}

/// Announces that who may see which shows changed (`None`: for everyone). Like every
/// notification it is only delivered once the surrounding transaction commits.
pub(crate) async fn notify_access(conn: &mut PgConnection, user_id: Option<Uuid>) -> Result<(), sqlx::Error> {
    let payload = match user_id {
        Some(user) => serde_json::json!({ "access": user }),
        None => serde_json::json!({ "access_all": true }),
    };
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(NOTIFY_CHANNEL)
        .bind(payload.to_string())
        .execute(conn)
        .await?;
    Ok(())
}

enum Outcome {
    Applied { position: Option<String> },
    Duplicate { rev: i64 },
}

/// A database error while applying one op rejects that op (its savepoint is rolled back).
fn db(error: sqlx::Error) -> String {
    eprintln!("Applying an op failed: {error}");
    "Could not apply the change".to_owned()
}

/// The show an entity with this id belongs to, if it exists (tombstones included).
async fn entity_show(conn: &mut PgConnection, entity: EntityKind, id: Uuid) -> Result<Option<Uuid>, sqlx::Error> {
    let t = table(entity);
    sqlx::query_scalar(&format!("SELECT show_id FROM {} WHERE {} = $1", t.name, t.pk))
        .bind(id)
        .fetch_optional(conn)
        .await
}

/// Turns a `place` into a concrete position between its neighbours (tombstones count so keys
/// stay distinct).
async fn resolve_place(conn: &mut PgConnection, show_id: Uuid, after: Option<Uuid>) -> Result<String, String> {
    let after_key: Option<String> = match after {
        Some(line) => Some(
            sqlx::query_scalar("SELECT position FROM show_lines WHERE line_id = $1 AND show_id = $2")
                .bind(line)
                .bind(show_id)
                .fetch_optional(&mut *conn)
                .await
                .map_err(db)?
                .ok_or("The line to insert after does not exist")?,
        ),
        None => None,
    };
    let next: Option<String> = match &after_key {
        Some(key) => sqlx::query_scalar(
            "SELECT MIN(position) FROM show_lines WHERE show_id = $1 AND position > $2 COLLATE \"C\"",
        )
        .bind(show_id)
        .bind(key)
        .fetch_one(&mut *conn)
        .await,
        None => sqlx::query_scalar("SELECT MIN(position) FROM show_lines WHERE show_id = $1")
            .bind(show_id)
            .fetch_one(&mut *conn)
            .await,
    }
    .map_err(db)?;
    position::between(after_key.as_deref(), next.as_deref()).map_err(|e| e.to_string())
}

async fn apply_one(
    conn: &mut PgConnection,
    show_id: Uuid,
    user_id: Uuid,
    role: MemberRole,
    client_id: Option<Uuid>,
    mut op: Op,
    rev: i64,
) -> Result<Outcome, String> {
    // A retry of something already applied changes nothing.
    let seen: Option<i64> = sqlx::query_scalar("SELECT rev FROM show_changes WHERE show_id = $1 AND op_id = $2")
        .bind(show_id)
        .bind(op.op_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(db)?;
    if let Some(rev) = seen {
        return Ok(Outcome::Duplicate { rev });
    }

    let existing = entity_show(&mut *conn, op.entity, op.id).await.map_err(db)?;
    match (existing, op.kind) {
        (None, OpKind::Set) => op.validate_create(),
        _ => op.validate(),
    }
    .map_err(|e| e.to_string())?;
    if !role.may_edit(op.entity, op.kind == OpKind::Delete) {
        return Err("You may not change this show".into());
    }
    match (existing, op.kind) {
        (Some(owner), _) if owner != show_id => return Err("That id is already in use".into()),
        (None, OpKind::Delete) => return Err("Nothing to delete".into()),
        _ => {}
    }
    if op.entity == EntityKind::Show && op.id != show_id {
        return Err("A show op must target the show itself".into());
    }

    if op.entity == EntityKind::Comment {
        if let Some(line) = op.fields.get("line_id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()) {
            if entity_show(&mut *conn, EntityKind::Line, line).await.map_err(db)? != Some(show_id) {
                return Err("The line does not belong to this show".into());
            }
        }
        if let Some(role_id) = op.fields.get("role_id").and_then(Value::as_str).and_then(|s| Uuid::parse_str(s).ok()) {
            if entity_show(&mut *conn, EntityKind::Role, role_id).await.map_err(db)? != Some(show_id) {
                return Err("The role does not belong to this show".into());
            }
        }
    }

    let mut resolved = None;
    if let Some(place) = op.place.take() {
        let key = resolve_place(&mut *conn, show_id, place.after).await?;
        op.fields.insert("position".into(), Value::String(key.clone()));
        resolved = Some(key);
    }

    write(&mut *conn, show_id, user_id, &op, existing.is_some()).await?;

    sqlx::query(
        "INSERT INTO show_changes (show_id, rev, op_id, entity, entity_id, kind, fields, author_id, client_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7::jsonb, $8, $9)",
    )
    .bind(show_id)
    .bind(rev)
    .bind(op.op_id)
    .bind(op.entity.as_str())
    .bind(op.id)
    .bind(if op.kind == OpKind::Delete { "delete" } else { "set" })
    .bind(Value::Object(op.fields.clone()).to_string())
    .bind(user_id)
    .bind(client_id)
    .execute(&mut *conn)
    .await
    .map_err(db)?;
    Ok(Outcome::Applied { position: resolved })
}

async fn write(conn: &mut PgConnection, show_id: Uuid, user_id: Uuid, op: &Op, exists: bool) -> Result<(), String> {
    let t = table(op.entity);
    let is_line = op.entity == EntityKind::Line;

    if op.kind == OpKind::Delete {
        let mut sql = format!("UPDATE {} SET deleted_at = COALESCE(deleted_at, now())", t.name);
        if is_line {
            sql.push_str(", updated_by = $2, updated_at = now()");
        }
        sql.push_str(&format!(" WHERE {} = $1", t.pk));
        let query = sqlx::query(&sql).bind(op.id);
        let query = if is_line { query.bind(user_id) } else { query };
        query.execute(&mut *conn).await.map_err(db)?;
        return Ok(());
    }

    let mut present = Vec::new();
    for (field, column) in t.columns {
        if let Some(value) = op.fields.get(*field) {
            present.push((*column, bind_for(op.entity, field, value).map_err(|_| format!("Invalid value for {field}"))?));
        }
    }

    if exists {
        if present.is_empty() {
            return Ok(());
        }
        let mut builder = QueryBuilder::new(format!("UPDATE {} SET ", t.name));
        for (i, (column, bind)) in present.into_iter().enumerate() {
            if i > 0 {
                builder.push(", ");
            }
            builder.push(format!("{column} = "));
            push_bind(&mut builder, bind);
        }
        if is_line {
            builder.push(", updated_by = ").push_bind(user_id).push(", updated_at = now()");
        }
        builder.push(format!(" WHERE {} = ", t.pk)).push_bind(op.id);
        builder.build().execute(&mut *conn).await.map_err(db)?;
    } else {
        let mut columns = vec![t.pk.to_owned(), "show_id".to_owned()];
        columns.extend(present.iter().map(|(column, _)| (*column).to_owned()));
        if is_line {
            columns.push("updated_by".to_owned());
        }
        let mut builder = QueryBuilder::new(format!("INSERT INTO {} ({}) VALUES (", t.name, columns.join(", ")));
        builder.push_bind(op.id).push(", ").push_bind(show_id);
        for (_, bind) in present {
            builder.push(", ");
            push_bind(&mut builder, bind);
        }
        if is_line {
            builder.push(", ").push_bind(user_id);
        }
        builder.push(")");
        builder.build().execute(&mut *conn).await.map_err(db)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
