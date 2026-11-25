//! IPC surface for the sync engine. The engine itself lives in the frontend (it already has the
//! tokens and the refresh logic, so the app needs no HTTP client); these commands are its
//! window onto the local database: what is waiting to be pushed, applying what was pulled,
//! and dropping shows the account lost.

use protocol::sync::{Change, Op, OpKind};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;
use tauri::State;

use crate::db::DbState;
use crate::store::{current_account, write};

#[derive(Serialize, Debug, PartialEq)]
pub struct SyncShowState {
    pub show_id: String,
    /// Last server change applied locally (0: nothing pulled yet).
    pub server_rev: i64,
    /// False for a show created here that the cloud has not seen.
    pub synced: bool,
    pub role: String,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct SyncState {
    pub account: Option<String>,
    pub shows: Vec<SyncShowState>,
    pub pending_total: i64,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct PendingOp {
    pub show_id: String,
    pub op: Op,
}

fn err(error: rusqlite::Error) -> String {
    error.to_string()
}

pub fn state(conn: &Connection) -> Result<SyncState, String> {
    let account = current_account(conn)?;
    let Some(account_id) = account.clone() else {
        return Ok(SyncState { account, shows: Vec::new(), pending_total: 0 });
    };
    let shows = conn
        .prepare(
            "SELECT s.show_id, y.server_rev, y.show_id IS NOT NULL, COALESCE(y.role, 'owner')
             FROM shows s LEFT JOIN sync_shows y ON y.show_id = s.show_id
             WHERE s.owner_account = ?1 ORDER BY s.show_id",
        )
        .map_err(err)?
        .query_map([&account_id], |row| {
            Ok(SyncShowState {
                show_id: row.get(0)?,
                server_rev: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                synced: row.get(2)?,
                role: row.get(3)?,
            })
        })
        .map_err(err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(err)?;
    let pending_total = conn
        .query_row(
            "SELECT count(*) FROM outbox o JOIN shows s ON s.show_id = o.show_id WHERE s.owner_account = ?1",
            [&account_id],
            |row| row.get(0),
        )
        .map_err(err)?;
    Ok(SyncState { account, shows, pending_total })
}

/// The oldest unsent edits of the signed-in account, in the order they were made. (Another
/// account's edits on this device stay put until that account signs in again.)
pub fn pending(conn: &Connection, limit: i64) -> Result<Vec<PendingOp>, String> {
    let Some(account) = current_account(conn)? else { return Ok(Vec::new()) };
    conn.prepare(
        "SELECT o.show_id, o.op FROM outbox o JOIN shows s ON s.show_id = o.show_id
         WHERE s.owner_account = ?1 ORDER BY o.seq LIMIT ?2",
    )
    .map_err(err)?
    .query_map(params![account, limit], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
    .map_err(err)?
    .map(|row| {
        let (show_id, op) = row.map_err(err)?;
        Ok(PendingOp { show_id, op: serde_json::from_str(&op).map_err(|e| e.to_string())? })
    })
    .collect()
}

/// The cloud has seen these ops (applied, already known, or finally rejected): stop sending them.
pub fn ack(conn: &mut Connection, show_id: &str, op_ids: &[String]) -> Result<(), String> {
    let tx = conn.transaction().map_err(err)?;
    for op_id in op_ids {
        tx.execute("DELETE FROM outbox WHERE op_id = ?1 AND show_id = ?2", params![op_id, show_id]).map_err(err)?;
    }
    // A show created here is now known to the cloud; its history is pulled from revision 0.
    tx.execute("INSERT OR IGNORE INTO sync_shows (show_id, server_rev, role) VALUES (?1, 0, 'owner')", [show_id]).map_err(err)?;
    tx.commit().map_err(err)
}

/// Re-applies the edits still waiting to be pushed on top of the (freshly updated) tables, so a
/// pull never undoes something the user did offline.
fn reapply_pending(tx: &Transaction, show_id: &str, account: Option<&str>) -> Result<(), String> {
    let ops: Vec<String> = tx
        .prepare("SELECT op FROM outbox WHERE show_id = ?1 ORDER BY seq")
        .map_err(err)?
        .query_map([show_id], |row| row.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for raw in ops {
        let op: Op = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if let Err(error) = write(tx, show_id, op.entity, &op.id.to_string(), op.kind, &op.fields, account) {
            eprintln!("Could not re-apply a pending edit: {error}");
        }
    }
    Ok(())
}

/// Applies changes pulled from the cloud and moves the show's cursor to `rev`.
pub fn apply_changes(conn: &mut Connection, show_id: &str, changes: &[Change], rev: i64, role: Option<&str>) -> Result<(), String> {
    let account = current_account(conn)?;
    let tx = conn.transaction().map_err(err)?;
    for change in changes {
        let applied = write(&tx, show_id, change.entity, &change.id.to_string(), change.kind, &change.fields, account.as_deref());
        if let Err(error) = applied {
            // A patch for something we never saw (it was created before this device could see
            // the show). Skipping is safe: the next pull from the right revision brings it.
            eprintln!("Skipped change {} of show {show_id}: {error}", change.rev);
        }
    }
    tx.execute(
        "INSERT INTO sync_shows (show_id, server_rev, role) VALUES (?1, ?2, COALESCE(?3, 'owner'))
         ON CONFLICT(show_id) DO UPDATE SET
             server_rev = MAX(server_rev, excluded.server_rev),
             role = COALESCE(?3, role)",
        params![show_id, rev, role],
    )
    .map_err(err)?;
    reapply_pending(&tx, show_id, account.as_deref())?;
    tx.commit().map_err(err)
}

/// Drops a show from this device: the account lost access, it was deleted in the cloud, or the
/// cloud rejected an edit and the local copy has to be re-downloaded from scratch.
pub fn forget_show(conn: &mut Connection, show_id: &str) -> Result<(), String> {
    let tx = conn.transaction().map_err(err)?;
    tx.execute("DELETE FROM show_scenario_line_comment WHERE line_id IN (SELECT scenario_line_id FROM show_scenario_line WHERE show_id = ?1)", [show_id]).map_err(err)?;
    tx.execute("DELETE FROM show_scenario_line WHERE show_id = ?1", [show_id]).map_err(err)?;
    tx.execute("DELETE FROM show_roles WHERE show_id = ?1", [show_id]).map_err(err)?;
    tx.execute("DELETE FROM shows WHERE show_id = ?1", [show_id]).map_err(err)?;
    tx.execute("DELETE FROM outbox WHERE show_id = ?1", [show_id]).map_err(err)?;
    tx.execute("DELETE FROM sync_shows WHERE show_id = ?1", [show_id]).map_err(err)?;
    tx.commit().map_err(err)
}

/// First sign-in on a device that already holds shows made before accounts existed: they become
/// the account's, and are queued for upload as if they had just been created.
pub fn adopt_account(conn: &mut Connection, user_id: &str) -> Result<usize, String> {
    let tx = conn.transaction().map_err(err)?;
    let unowned: Vec<String> = tx
        .prepare("SELECT show_id FROM shows WHERE owner_account IS NULL AND deleted_at IS NULL")
        .map_err(err)?
        .query_map([], |row| row.get(0))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    // Shows deleted before they ever belonged to anyone have nothing to upload.
    tx.execute("DELETE FROM outbox WHERE show_id IN (SELECT show_id FROM shows WHERE owner_account IS NULL)", []).map_err(err)?;
    for show_id in &unowned {
        tx.execute("UPDATE shows SET owner_account = ?1 WHERE show_id = ?2", params![user_id, show_id]).map_err(err)?;
        enqueue_snapshot(&tx, show_id)?;
    }
    tx.commit().map_err(err)?;
    Ok(unowned.len())
}

/// Queues create ops that rebuild a show's current content.
fn enqueue_snapshot(tx: &Transaction, show_id: &str) -> Result<(), String> {
    use protocol::sync::EntityKind;
    use serde_json::json;
    let queue = |entity: EntityKind, id: &str, fields: serde_json::Value| -> Result<(), String> {
        let op = Op {
            op_id: uuid::Uuid::new_v4(),
            entity,
            id: id.parse().map_err(|_| format!("bad id {id}"))?,
            kind: OpKind::Set,
            fields: fields.as_object().cloned().unwrap_or_default(),
            place: None,
        };
        tx.execute(
            "INSERT INTO outbox (show_id, op_id, op) VALUES (?1, ?2, ?3)",
            params![show_id, op.op_id.to_string(), serde_json::to_string(&op).map_err(|e| e.to_string())?],
        )
        .map_err(err)?;
        Ok(())
    };

    let (name, time): (String, i64) = tx
        .query_row("SELECT show_name, show_time FROM shows WHERE show_id = ?1", [show_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .map_err(err)?
        .ok_or("Show not found")?;
    queue(EntityKind::Show, show_id, json!({ "name": name, "time": time }))?;

    let roles: Vec<(String, String)> = tx
        .prepare("SELECT role_id, role_name FROM show_roles WHERE show_id = ?1 AND deleted_at IS NULL ORDER BY rowid")
        .map_err(err)?
        .query_map([show_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for (id, name) in roles {
        queue(EntityKind::Role, &id, json!({ "name": name }))?;
    }

    type LineRow = (String, String, String, String, i64, Option<i64>);
    let lines: Vec<LineRow> = tx
        .prepare(
            "SELECT scenario_line_id, scenario_line_position, scenario_line_name, scenario_line_note,
                    scenario_line_duration, scenario_line_absolute_time
             FROM show_scenario_line WHERE show_id = ?1 AND deleted_at IS NULL
             ORDER BY scenario_line_position, scenario_line_id",
        )
        .map_err(err)?
        .query_map([show_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for (id, position, name, note, duration, absolute_time) in lines {
        queue(EntityKind::Line, &id, json!({
            "position": position, "name": name, "note": note, "duration": duration, "absolute_time": absolute_time
        }))?;
    }

    let comments: Vec<(String, String, Option<String>, String)> = tx
        .prepare(
            "SELECT c.id, c.line_id, c.role_id, c.comment FROM show_scenario_line_comment c
             JOIN show_scenario_line l ON l.scenario_line_id = c.line_id
             WHERE l.show_id = ?1 AND l.deleted_at IS NULL AND c.deleted_at IS NULL ORDER BY c.rowid",
        )
        .map_err(err)?
        .query_map([show_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .map_err(err)?
        .collect::<Result<_, _>>()
        .map_err(err)?;
    for (id, line_id, role_id, comment) in comments {
        queue(EntityKind::Comment, &id, json!({ "line_id": line_id, "role_id": role_id, "comment": comment }))?;
    }
    Ok(())
}

#[tauri::command]
pub fn sync_state(db: State<'_, DbState>) -> Result<SyncState, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    state(&conn)
}

#[tauri::command]
pub fn sync_pending(db: State<'_, DbState>, limit: i64) -> Result<Vec<PendingOp>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    pending(&conn, limit.clamp(1, 500))
}

#[tauri::command]
pub fn sync_ack(db: State<'_, DbState>, show_id: &str, op_ids: Vec<String>) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    ack(&mut conn, show_id, &op_ids)
}

#[tauri::command]
pub fn sync_apply_changes(db: State<'_, DbState>, show_id: &str, changes: Vec<Change>, rev: i64, role: Option<String>) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    apply_changes(&mut conn, show_id, &changes, rev, role.as_deref())
}

#[tauri::command]
pub fn sync_forget_show(db: State<'_, DbState>, show_id: &str) -> Result<(), String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    forget_show(&mut conn, show_id)
}

#[tauri::command]
pub fn sync_adopt_account(db: State<'_, DbState>, user_id: &str) -> Result<usize, String> {
    let mut conn = db.0.lock().map_err(|e| e.to_string())?;
    adopt_account(&mut conn, user_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::store::{apply_local, fetch_show_details, new_op};
    use protocol::sync::{EntityKind, Place};
    use serde_json::json;
    use uuid::Uuid;

    fn conn(account: Option<&str>) -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO auth_state (id, device_id, user_id) VALUES (1, 'dev', ?1)", [account]).unwrap();
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

    fn change(rev: i64, entity: EntityKind, id: Uuid, kind: OpKind, fields: serde_json::Value) -> Change {
        Change { rev, op_id: Uuid::new_v4(), entity, id, kind, fields: fields.as_object().cloned().unwrap_or_default(), author_id: None }
    }

    #[test]
    fn pending_edits_come_out_in_order_and_only_for_the_signed_in_account() {
        let mut conn = conn(Some("alice"));
        let show = create_show(&mut conn);
        add_line(&mut conn, &show, "cue", None);
        let pending_ops = pending(&conn, 100).unwrap();
        assert_eq!(pending_ops.len(), 2);
        assert_eq!((pending_ops[0].op.entity, pending_ops[1].op.entity), (EntityKind::Show, EntityKind::Line));
        assert_eq!(state(&conn).unwrap().pending_total, 2);
        assert_eq!(pending(&conn, 1).unwrap().len(), 1);

        // Someone else signs in on this device: Alice's unsent edits are not theirs to push.
        conn.execute("UPDATE auth_state SET user_id = 'bob'", []).unwrap();
        assert!(pending(&conn, 100).unwrap().is_empty());
        assert!(state(&conn).unwrap().shows.is_empty());
        conn.execute("UPDATE auth_state SET user_id = NULL", []).unwrap();
        assert!(pending(&conn, 100).unwrap().is_empty());
    }

    #[test]
    fn acknowledged_edits_leave_the_outbox_and_the_show_becomes_known_to_the_cloud() {
        let mut conn = conn(Some("alice"));
        let show = create_show(&mut conn);
        add_line(&mut conn, &show, "cue", None);
        assert!(!state(&conn).unwrap().shows[0].synced);
        let ids: Vec<String> = pending(&conn, 100).unwrap().iter().map(|p| p.op.op_id.to_string()).collect();
        ack(&mut conn, &show, &ids[..1]).unwrap();
        assert_eq!(pending(&conn, 100).unwrap().len(), 1);
        let shows = state(&conn).unwrap().shows;
        assert!(shows[0].synced);
        assert_eq!(shows[0].server_rev, 0, "history is pulled from 0, our own ops included");
        ack(&mut conn, &show, &ids).unwrap();
        assert!(pending(&conn, 100).unwrap().is_empty());
    }

    #[test]
    fn pulled_changes_create_a_shared_show_and_set_my_role() {
        let mut conn = conn(Some("bob"));
        let (show, line) = (Uuid::new_v4(), Uuid::new_v4());
        let changes = vec![
            change(1, EntityKind::Show, show, OpKind::Set, json!({"name": "Alice's gala", "time": 1_800_000_000})),
            change(2, EntityKind::Line, line, OpKind::Set, json!({"name": "Intro", "note": "", "duration": 60, "position": "V"})),
        ];
        apply_changes(&mut conn, &show.to_string(), &changes, 2, Some("commenter")).unwrap();
        let details = fetch_show_details(&conn, &show.to_string()).unwrap();
        assert_eq!((details.show.name.as_str(), details.lines.len(), details.rev), ("Alice's gala", 1, 2));
        assert_eq!(details.show.role, protocol::show::MemberRole::Commenter);
        let shows = state(&conn).unwrap().shows;
        assert_eq!((shows[0].show_id.as_str(), shows[0].server_rev, shows[0].synced), (show.to_string().as_str(), 2, true));
        // The show belongs to the account that pulled it: nobody else on this device sees it.
        assert_eq!(crate::store::list_shows(&conn, Some("bob")).unwrap().len(), 1);
        assert!(crate::store::list_shows(&conn, Some("alice")).unwrap().is_empty());

        // The cursor only moves forward, and a role change arrives with an empty pull.
        apply_changes(&mut conn, &show.to_string(), &[], 1, Some("editor")).unwrap();
        let shows = state(&conn).unwrap().shows;
        assert_eq!(shows[0].server_rev, 2);
        assert_eq!(shows[0].role, "editor");
    }

    #[test]
    fn a_pull_never_undoes_an_edit_that_has_not_been_sent_yet() {
        let mut conn = conn(Some("alice"));
        let show = create_show(&mut conn);
        let line = add_line(&mut conn, &show, "Intro", None);
        let ids: Vec<String> = pending(&conn, 100).unwrap().iter().map(|p| p.op.op_id.to_string()).collect();
        ack(&mut conn, &show, &ids).unwrap();

        // Offline, the user retitles the line; meanwhile someone else changed the note and the title.
        apply_local(&mut conn, &show, new_op(EntityKind::Line, line, OpKind::Set, json!({"name": "My title"}))).unwrap();
        let remote = vec![change(5, EntityKind::Line, line, OpKind::Set, json!({"name": "Their title", "note": "their note"}))];
        apply_changes(&mut conn, &show, &remote, 5, None).unwrap();

        let line = &fetch_show_details(&conn, &show).unwrap().lines[0];
        assert_eq!(line.name, "My title", "the unsent edit stays on top");
        assert_eq!(line.note, "their note", "everything else follows the cloud");
        assert_eq!(pending(&conn, 100).unwrap().len(), 1, "and it is still queued");
    }

    #[test]
    fn a_remote_delete_beats_a_pending_edit() {
        let mut conn = conn(Some("alice"));
        let show = create_show(&mut conn);
        let line = add_line(&mut conn, &show, "Intro", None);
        let ids: Vec<String> = pending(&conn, 100).unwrap().iter().map(|p| p.op.op_id.to_string()).collect();
        ack(&mut conn, &show, &ids).unwrap();
        apply_local(&mut conn, &show, new_op(EntityKind::Line, line, OpKind::Set, json!({"note": "edit"}))).unwrap();
        apply_changes(&mut conn, &show, &[change(4, EntityKind::Line, line, OpKind::Delete, json!({}))], 4, None).unwrap();
        assert!(fetch_show_details(&conn, &show).unwrap().lines.is_empty());
    }

    #[test]
    fn forgetting_a_show_removes_every_trace_of_it() {
        let mut conn = conn(Some("alice"));
        let show = create_show(&mut conn);
        let other = create_show(&mut conn);
        let line = add_line(&mut conn, &show, "Intro", None);
        apply_local(&mut conn, &show, new_op(EntityKind::Comment, Uuid::new_v4(), OpKind::Set, json!({"line_id": line, "comment": "c"}))).unwrap();
        ack(&mut conn, &show, &[]).unwrap();
        forget_show(&mut conn, &show).unwrap();
        for table in ["shows", "show_roles", "show_scenario_line", "outbox", "sync_shows"] {
            let own: i64 = conn.query_row(&format!("SELECT count(*) FROM {table} WHERE show_id = ?1"), [&show], |r| r.get(0)).unwrap();
            assert_eq!(own, 0, "{table}");
        }
        let comments: i64 = conn.query_row("SELECT count(*) FROM show_scenario_line_comment WHERE line_id = ?1", [line.to_string()], |r| r.get(0)).unwrap();
        assert_eq!(comments, 0, "comments");
        assert!(fetch_show_details(&conn, &other).is_ok(), "other shows are untouched");
    }

    #[test]
    fn adopting_an_account_uploads_shows_made_before_accounts_existed() {
        let mut conn = conn(None);
        // A show from the standalone days: it exists, but nothing queued it.
        let show = Uuid::new_v4().to_string();
        let (line, role) = (Uuid::new_v4().to_string(), Uuid::new_v4().to_string());
        conn.execute("INSERT INTO shows (show_id, show_name, show_time) VALUES (?1, 'Old show', 1700000000)", [&show]).unwrap();
        conn.execute("INSERT INTO show_roles (role_id, show_id, role_name) VALUES (?1, ?2, 'Audio')", [&role, &show]).unwrap();
        conn.execute("INSERT INTO show_scenario_line (scenario_line_id, show_id, scenario_line_position, scenario_line_name, scenario_line_note, scenario_line_duration) VALUES (?1, ?2, 'k000001V', 'Opening', 'n', 30)", [&line, &show]).unwrap();
        conn.execute("INSERT INTO show_scenario_line_comment (id, line_id, role_id, comment) VALUES (?1, ?2, ?3, 'hi')", [&Uuid::new_v4().to_string(), &line, &role]).unwrap();
        // And one that was already deleted: not worth uploading.
        let gone = Uuid::new_v4().to_string();
        conn.execute("INSERT INTO shows (show_id, show_name, show_time, deleted_at) VALUES (?1, 'Gone', 1, 5)", [&gone]).unwrap();

        assert_eq!(adopt_account(&mut conn, "alice").unwrap(), 1);
        conn.execute("UPDATE auth_state SET user_id = 'alice'", []).unwrap();
        let queued = pending(&conn, 100).unwrap();
        let kinds: Vec<EntityKind> = queued.iter().map(|p| p.op.entity).collect();
        assert_eq!(kinds, [EntityKind::Show, EntityKind::Role, EntityKind::Line, EntityKind::Comment]);
        assert!(queued.iter().all(|p| p.show_id == show && p.op.validate_create().is_ok()));
        assert_eq!(queued[2].op.fields["position"], "k000001V");
        assert_eq!(crate::store::list_shows(&conn, Some("alice")).unwrap().len(), 1);

        // Signing in again changes nothing: the shows are owned now.
        assert_eq!(adopt_account(&mut conn, "alice").unwrap(), 0);
        assert_eq!(pending(&conn, 100).unwrap().len(), 4);
    }
}
