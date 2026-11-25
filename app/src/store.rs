//! The local show store. Every edit, whether it comes from the user or from the cloud, is an
//! `Op` (see `protocol::sync`) applied to the sqlite tables here. Local edits are also queued
//! in the `outbox` in the same transaction, so the sync engine can push them later and a local
//! edit can never be lost or half-queued.

use protocol::position;
use protocol::show::{MemberRole, ScenarioLine, ScenarioLineComment, Show, ShowDetails, ShowRole};
use protocol::sync::{EntityKind, Fields, Op, OpKind};
use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Transaction};
use serde_json::Value;
use uuid::Uuid;

/// How an entity maps onto its sqlite table.
struct Table {
    name: &'static str,
    pk: &'static str,
    /// Column holding the owning show's id (comments reach their show through the line).
    show_column: Option<&'static str>,
    /// `(op field, column)`.
    columns: &'static [(&'static str, &'static str)],
}

fn table(entity: EntityKind) -> Table {
    match entity {
        EntityKind::Show => Table {
            name: "shows",
            pk: "show_id",
            show_column: None,
            columns: &[("name", "show_name"), ("time", "show_time")],
        },
        EntityKind::Role => Table {
            name: "show_roles",
            pk: "role_id",
            show_column: Some("show_id"),
            columns: &[("name", "role_name")],
        },
        EntityKind::Line => Table {
            name: "show_scenario_line",
            pk: "scenario_line_id",
            show_column: Some("show_id"),
            columns: &[
                ("name", "scenario_line_name"),
                ("note", "scenario_line_note"),
                ("duration", "scenario_line_duration"),
                ("absolute_time", "scenario_line_absolute_time"),
                ("position", "scenario_line_position"),
            ],
        },
        EntityKind::Comment => Table {
            name: "show_scenario_line_comment",
            pk: "id",
            show_column: None,
            columns: &[("line_id", "line_id"), ("role_id", "role_id"), ("comment", "comment")],
        },
    }
}

fn sql_value(value: &Value) -> Result<SqlValue, String> {
    match value {
        Value::Null => Ok(SqlValue::Null),
        Value::String(s) => Ok(SqlValue::Text(s.clone())),
        Value::Bool(b) => Ok(SqlValue::Integer(i64::from(*b))),
        Value::Number(n) => n
            .as_i64()
            .map(SqlValue::Integer)
            .or_else(|| n.as_f64().map(SqlValue::Real))
            .ok_or_else(|| "number out of range".to_string()),
        _ => Err("unsupported value".to_string()),
    }
}

fn err(error: rusqlite::Error) -> String {
    error.to_string()
}

fn row_exists(tx: &Transaction, entity: EntityKind, id: &str) -> Result<bool, String> {
    let t = table(entity);
    tx.query_row(&format!("SELECT 1 FROM {} WHERE {} = ?1", t.name, t.pk), [id], |_| Ok(()))
        .optional()
        .map(|row| row.is_some())
        .map_err(err)
}

/// The show an existing entity belongs to.
fn show_of(tx: &Transaction, entity: EntityKind, id: &str) -> Result<Option<String>, String> {
    let sql = match entity {
        EntityKind::Show => return Ok(row_exists(tx, entity, id)?.then(|| id.to_string())),
        EntityKind::Role => "SELECT show_id FROM show_roles WHERE role_id = ?1",
        EntityKind::Line => "SELECT show_id FROM show_scenario_line WHERE scenario_line_id = ?1",
        EntityKind::Comment => {
            "SELECT l.show_id FROM show_scenario_line_comment c
             JOIN show_scenario_line l ON l.scenario_line_id = c.line_id WHERE c.id = ?1"
        }
    };
    tx.query_row(sql, [id], |row| row.get(0)).optional().map_err(err)
}

fn line_show(tx: &Transaction, line_id: &str) -> Result<Option<String>, String> {
    show_of(tx, EntityKind::Line, line_id)
}

/// Turns a `place` into a concrete `position`: between the line it goes after and the next
/// line (tombstones count, so keys stay distinct).
fn resolve_place(tx: &Transaction, show_id: &str, op: &mut Op) -> Result<(), String> {
    let Some(place) = op.place.take() else { return Ok(()) };
    let after_key: Option<String> = match place.after {
        Some(after) => Some(
            tx.query_row(
                "SELECT scenario_line_position FROM show_scenario_line WHERE scenario_line_id = ?1 AND show_id = ?2",
                params![after.to_string(), show_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(err)?
            .ok_or("The line to insert after does not exist")?,
        ),
        None => None,
    };
    let next: Option<String> = match &after_key {
        Some(key) => tx.query_row(
            "SELECT MIN(scenario_line_position) FROM show_scenario_line WHERE show_id = ?1 AND scenario_line_position > ?2",
            params![show_id, key],
            |row| row.get(0),
        ),
        None => tx.query_row(
            "SELECT MIN(scenario_line_position) FROM show_scenario_line WHERE show_id = ?1",
            [show_id],
            |row| row.get(0),
        ),
    }
    .map_err(err)?;
    let key = position::between(after_key.as_deref(), next.as_deref()).map_err(|e| e.to_string())?;
    op.fields.insert("position".into(), Value::String(key));
    Ok(())
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Writes one patch into the tables. Used for local ops and for changes pulled from the cloud.
pub(crate) fn write(
    tx: &Transaction,
    show_id: &str,
    entity: EntityKind,
    id: &str,
    kind: OpKind,
    fields: &Fields,
    account: Option<&str>,
) -> Result<(), String> {
    let t = table(entity);
    if kind == OpKind::Delete {
        tx.execute(
            &format!("UPDATE {} SET deleted_at = COALESCE(deleted_at, ?1) WHERE {} = ?2", t.name, t.pk),
            params![now(), id],
        )
        .map_err(err)?;
        return Ok(());
    }

    let present: Vec<(&str, SqlValue)> = t
        .columns
        .iter()
        .filter_map(|(field, column)| fields.get(*field).map(|value| (*column, value)))
        .map(|(column, value)| sql_value(value).map(|v| (column, v)))
        .collect::<Result<_, _>>()?;

    if row_exists(tx, entity, id)? {
        // A patch on an existing row. (An upsert would trip NOT NULL on the columns the patch
        // leaves out, because SQLite checks the proposed insert row first.)
        if !present.is_empty() {
            let sets = present.iter().enumerate().map(|(i, (column, _))| format!("{column} = ?{}", i + 1)).collect::<Vec<_>>().join(", ");
            let mut values: Vec<SqlValue> = present.into_iter().map(|(_, v)| v).collect();
            values.push(SqlValue::Text(id.to_string()));
            let id_slot = values.len();
            tx.execute(&format!("UPDATE {} SET {sets} WHERE {} = ?{id_slot}", t.name, t.pk), params_from_iter(values)).map_err(err)?;
        }
    } else {
        let mut columns = vec![t.pk.to_string()];
        let mut values = vec![SqlValue::Text(id.to_string())];
        if let Some(show_column) = t.show_column {
            columns.push(show_column.to_string());
            values.push(SqlValue::Text(show_id.to_string()));
        }
        for (column, value) in present {
            columns.push(column.to_string());
            values.push(value);
        }
        let placeholders = (1..=columns.len()).map(|i| format!("?{i}")).collect::<Vec<_>>().join(", ");
        tx.execute(
            &format!("INSERT INTO {} ({}) VALUES ({})", t.name, columns.join(", "), placeholders),
            params_from_iter(values),
        )
        .map_err(err)?;
    }
    if entity == EntityKind::Show {
        tx.execute(
            "UPDATE shows SET owner_account = COALESCE(owner_account, ?1) WHERE show_id = ?2",
            params![account, id],
        )
        .map_err(err)?;
    }
    Ok(())
}

pub fn current_account(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row("SELECT user_id FROM auth_state WHERE id = 1", [], |row| row.get(0))
        .optional()
        .map(Option::flatten)
        .map_err(err)
}

fn role_in(tx: &Transaction, show_id: &str) -> Result<MemberRole, String> {
    let role: Option<String> = tx
        .query_row("SELECT role FROM sync_shows WHERE show_id = ?1", [show_id], |row| row.get(0))
        .optional()
        .map_err(err)?;
    // A show that never synced is purely local: its creator owns it.
    Ok(role.as_deref().and_then(MemberRole::parse).unwrap_or(MemberRole::Owner))
}

/// Applies a local edit and queues it for the cloud, atomically. Returns the op as queued
/// (a `place` replaced by the concrete `position`).
pub fn apply_local(conn: &mut Connection, show_id: &str, mut op: Op) -> Result<Op, String> {
    let account = current_account(conn)?;
    let tx = conn.transaction().map_err(err)?;
    let id = op.id.to_string();
    let existing_show = show_of(&tx, op.entity, &id)?;

    match (&existing_show, op.kind) {
        (None, OpKind::Set) => op.validate_create(),
        (_, _) => op.validate(),
    }
    .map_err(|e| e.to_string())?;
    if op.entity != EntityKind::Show && !row_exists(&tx, EntityKind::Show, show_id)? {
        return Err("Show not found".into());
    }
    match (&existing_show, op.kind) {
        (None, OpKind::Delete) => return Err("Nothing to delete".into()),
        (Some(owner), _) if owner != show_id => return Err("That item belongs to another show".into()),
        _ => {}
    }
    if !role_in(&tx, show_id)?.may_edit(op.entity, op.kind == OpKind::Delete) {
        return Err("You may not change this show".into());
    }
    if op.entity == EntityKind::Comment {
        if let Some(line_id) = op.fields.get("line_id").and_then(Value::as_str) {
            if line_show(&tx, line_id)?.as_deref() != Some(show_id) {
                return Err("The line does not belong to this show".into());
            }
        }
    }
    resolve_place(&tx, show_id, &mut op)?;

    write(&tx, show_id, op.entity, &id, op.kind, &op.fields, account.as_deref())?;
    tx.execute(
        "INSERT INTO outbox (show_id, op_id, op) VALUES (?1, ?2, ?3)",
        params![show_id, op.op_id.to_string(), serde_json::to_string(&op).map_err(|e| e.to_string())?],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(op)
}

/// Looks up which show an entity belongs to, for commands that only receive the entity id.
pub fn show_for(conn: &mut Connection, entity: EntityKind, id: &str) -> Result<String, String> {
    let tx = conn.transaction().map_err(err)?;
    show_of(&tx, entity, id)?.ok_or_else(|| "Not found".to_string())
}

pub fn new_op(entity: EntityKind, id: Uuid, kind: OpKind, fields: Value) -> Op {
    Op {
        op_id: Uuid::new_v4(),
        entity,
        id,
        kind,
        fields: fields.as_object().cloned().unwrap_or_default(),
        place: None,
    }
}

pub fn list_shows(conn: &Connection, account: Option<&str>) -> Result<Vec<Show>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.show_id, s.show_name, s.show_time, COALESCE(y.role, 'owner')
             FROM shows s LEFT JOIN sync_shows y ON y.show_id = s.show_id
             WHERE s.deleted_at IS NULL AND (s.owner_account IS NULL OR s.owner_account = ?1)
             ORDER BY s.show_time, s.show_id",
        )
        .map_err(err)?;
    let rows = stmt
        .query_map([account], |row| {
            let role: String = row.get(3)?;
            Ok(Show {
                id: row.get(0)?,
                name: row.get(1)?,
                time: row.get(2)?,
                role: MemberRole::parse(&role).unwrap_or(MemberRole::Viewer),
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(rows)
}

pub fn fetch_show_details(conn: &Connection, show_id: &str) -> Result<ShowDetails, String> {
    let show = conn
        .query_row(
            "SELECT s.show_name, s.show_time, COALESCE(y.role, 'owner'), COALESCE(y.server_rev, 0)
             FROM shows s LEFT JOIN sync_shows y ON y.show_id = s.show_id
             WHERE s.show_id = ?1 AND s.deleted_at IS NULL",
            [show_id],
            |row| {
                let role: String = row.get(2)?;
                Ok((
                    Show {
                        id: show_id.to_string(),
                        name: row.get(0)?,
                        time: row.get(1)?,
                        role: MemberRole::parse(&role).unwrap_or(MemberRole::Viewer),
                    },
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(err)?
        .ok_or("Show not found")?;

    let roles = conn
        .prepare("SELECT role_id, show_id, role_name FROM show_roles WHERE show_id = ?1 AND deleted_at IS NULL ORDER BY rowid")
        .map_err(err)?
        .query_map([show_id], |row| {
            Ok(ShowRole { id: row.get(0)?, show_id: row.get(1)?, name: row.get(2)? })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;

    let lines = conn
        .prepare(
            "SELECT scenario_line_id, scenario_line_position, scenario_line_name, scenario_line_note,
                    scenario_line_duration, scenario_line_absolute_time
             FROM show_scenario_line WHERE show_id = ?1 AND deleted_at IS NULL
             ORDER BY scenario_line_position, scenario_line_id",
        )
        .map_err(err)?
        .query_map([show_id], |row| {
            Ok(ScenarioLine {
                id: row.get(0)?,
                position: row.get(1)?,
                name: row.get(2)?,
                note: row.get(3)?,
                duration: row.get(4)?,
                absolute_time: row.get(5)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;

    let comments = conn
        .prepare(
            "SELECT c.id, c.line_id, c.role_id, c.comment
             FROM show_scenario_line_comment c
             JOIN show_scenario_line l ON l.scenario_line_id = c.line_id
             WHERE l.show_id = ?1 AND l.deleted_at IS NULL AND c.deleted_at IS NULL
             ORDER BY c.rowid",
        )
        .map_err(err)?
        .query_map([show_id], |row| {
            Ok(ScenarioLineComment {
                id: row.get(0)?,
                line_id: row.get(1)?,
                role_id: row.get(2)?,
                comment: row.get(3)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;

    Ok(ShowDetails { show: show.0, roles, lines, comments, rev: show.1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use protocol::sync::Place;
    use serde_json::json;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        conn
    }

    fn create_show(conn: &mut Connection) -> String {
        let id = Uuid::new_v4();
        apply_local(conn, &id.to_string(), new_op(EntityKind::Show, id, OpKind::Set, json!({"name": "Gala", "time": 1_800_000_000}))).unwrap();
        id.to_string()
    }

    fn add_line(conn: &mut Connection, show: &str, name: &str, after: Option<Uuid>) -> Uuid {
        let id = Uuid::new_v4();
        let mut op = new_op(EntityKind::Line, id, OpKind::Set, json!({"name": name, "note": "", "duration": 60}));
        op.place = Some(Place { after });
        apply_local(conn, show, op).unwrap();
        id
    }

    fn names(conn: &Connection, show: &str) -> Vec<String> {
        fetch_show_details(conn, show).unwrap().lines.into_iter().map(|l| l.name).collect()
    }

    #[test]
    fn lines_are_ordered_by_where_they_were_placed() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        let first = add_line(&mut conn, &show, "first", None);
        let third = add_line(&mut conn, &show, "third", Some(first));
        add_line(&mut conn, &show, "second", Some(first));
        add_line(&mut conn, &show, "zero", None);
        let _ = third;
        assert_eq!(names(&conn, &show), ["zero", "first", "second", "third"]);
    }

    #[test]
    fn every_local_edit_is_queued_with_a_concrete_position() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        add_line(&mut conn, &show, "cue", None);
        let queued: Vec<String> = conn.prepare("SELECT op FROM outbox ORDER BY seq").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(queued.len(), 2);
        let line: Op = serde_json::from_str(&queued[1]).unwrap();
        assert!(line.place.is_none());
        assert!(protocol::position::is_valid_key(line.fields["position"].as_str().unwrap()));
    }

    #[test]
    fn a_rejected_edit_leaves_neither_data_nor_outbox_rows() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        let before: i64 = conn.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0)).unwrap();
        // Missing required fields.
        let bad = new_op(EntityKind::Line, Uuid::new_v4(), OpKind::Set, json!({"name": "x"}));
        assert!(apply_local(&mut conn, &show, bad).is_err());
        // Comment on a line that is not in this show.
        let stray = new_op(EntityKind::Comment, Uuid::new_v4(), OpKind::Set, json!({"line_id": Uuid::new_v4(), "comment": "hi"}));
        assert!(apply_local(&mut conn, &show, stray).is_err());
        // Deleting what does not exist.
        assert!(apply_local(&mut conn, &show, new_op(EntityKind::Line, Uuid::new_v4(), OpKind::Delete, json!({}))).is_err());
        let after: i64 = conn.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0)).unwrap();
        assert_eq!(before, after);
        assert!(names(&conn, &show).is_empty());
    }

    #[test]
    fn deleting_a_line_hides_it_and_its_comments_but_keeps_a_tombstone() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        let line = add_line(&mut conn, &show, "cue", None);
        let comment = Uuid::new_v4();
        apply_local(&mut conn, &show, new_op(EntityKind::Comment, comment, OpKind::Set, json!({"line_id": line, "comment": "careful", "role_id": null}))).unwrap();
        assert_eq!(fetch_show_details(&conn, &show).unwrap().comments.len(), 1);

        apply_local(&mut conn, &show, new_op(EntityKind::Line, line, OpKind::Delete, json!({}))).unwrap();
        let details = fetch_show_details(&conn, &show).unwrap();
        assert!(details.lines.is_empty() && details.comments.is_empty());
        let tombstoned: i64 = conn.query_row("SELECT count(*) FROM show_scenario_line WHERE deleted_at IS NOT NULL", [], |r| r.get(0)).unwrap();
        assert_eq!(tombstoned, 1);
    }

    #[test]
    fn patches_only_touch_the_fields_they_carry() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        let line = add_line(&mut conn, &show, "cue", None);
        apply_local(&mut conn, &show, new_op(EntityKind::Line, line, OpKind::Set, json!({"note": "new note"}))).unwrap();
        apply_local(&mut conn, &show, new_op(EntityKind::Line, line, OpKind::Set, json!({"duration": 5, "absolute_time": 1_800_000_100}))).unwrap();
        let line = &fetch_show_details(&conn, &show).unwrap().lines[0];
        assert_eq!((line.name.as_str(), line.note.as_str(), line.duration, line.absolute_time), ("cue", "new note", 5, Some(1_800_000_100)));
    }

    #[test]
    fn viewers_cannot_edit_but_the_owner_of_a_local_show_can() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        conn.execute("INSERT INTO sync_shows (show_id, server_rev, role) VALUES (?1, 3, 'viewer')", [&show]).unwrap();
        let attempt = new_op(EntityKind::Role, Uuid::new_v4(), OpKind::Set, json!({"name": "Audio"}));
        assert!(apply_local(&mut conn, &show, attempt).is_err());
        let details = fetch_show_details(&conn, &show).unwrap();
        assert_eq!((details.show.role, details.rev), (MemberRole::Viewer, 3));
    }

    #[test]
    fn shows_belong_to_the_account_that_created_them() {
        let mut conn = conn();
        conn.execute("INSERT INTO auth_state (id, device_id, user_id) VALUES (1, 'dev', 'user-a')", []).unwrap();
        let mine = create_show(&mut conn);
        conn.execute("UPDATE auth_state SET user_id = 'user-b'", []).unwrap();
        let theirs = create_show(&mut conn);
        let visible = |account: &str| list_shows(&conn, Some(account)).unwrap().into_iter().map(|s| s.id).collect::<Vec<_>>();
        assert_eq!(visible("user-a"), vec![mine]);
        assert_eq!(visible("user-b"), vec![theirs]);
    }

    #[test]
    fn deleting_a_show_removes_it_from_the_list() {
        let mut conn = conn();
        let show = create_show(&mut conn);
        let id: Uuid = show.parse().unwrap();
        apply_local(&mut conn, &show, new_op(EntityKind::Show, id, OpKind::Delete, json!({}))).unwrap();
        assert!(list_shows(&conn, None).unwrap().is_empty());
        assert!(fetch_show_details(&conn, &show).is_err());
    }
}
