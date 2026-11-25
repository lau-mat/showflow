//! Persists the cloud refresh token and device identity in the local sqlite db so the app
//! stays logged in across restarts and can run offline for a limited grace window.
//! The token is stored unencrypted; real licence enforcement stays server-side.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::State;

use crate::db::DbState;

/// How long the app may run offline after the last successful online validation.
pub const GRACE_SECONDS: i64 = 14 * 24 * 60 * 60;

#[derive(Serialize, Debug, PartialEq)]
pub struct StoredAuth {
    pub refresh_token: Option<String>,
    pub refresh_expires_at: Option<String>,
    pub device_id: String,
    pub grace_remaining_seconds: i64,
    /// The signed-in cloud account; local shows are scoped to it.
    pub user_id: Option<String>,
}

fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn ensure_row(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO auth_state (id, device_id) VALUES (1, ?1)",
        params![uuid::Uuid::new_v4().to_string()],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn grace_remaining(last_validated_at: Option<i64>, now: i64) -> i64 {
    match last_validated_at {
        None => 0,
        // A clock set back before the last validation is treated as tampering.
        Some(last) if now < last => 0,
        Some(last) => (GRACE_SECONDS - (now - last)).max(0),
    }
}

pub fn load(conn: &Connection, now: i64) -> Result<StoredAuth, String> {
    ensure_row(conn)?;
    conn.query_row(
        "SELECT refresh_token, refresh_expires_at, last_validated_at, device_id, user_id FROM auth_state WHERE id = 1",
        [],
        |row| {
            let token: Option<String> = row.get(0)?;
            let last_validated_at: Option<i64> = row.get(2)?;
            Ok(StoredAuth {
                grace_remaining_seconds: if token.is_some() { grace_remaining(last_validated_at, now) } else { 0 },
                refresh_token: token,
                refresh_expires_at: row.get(1)?,
                device_id: row.get(3)?,
                user_id: row.get(4)?,
            })
        },
    ).optional().map_err(|e| e.to_string())?.ok_or_else(|| "auth_state row missing".to_string())
}

pub fn save(conn: &Connection, refresh_token: &str, refresh_expires_at: &str, user_id: Option<&str>, now: i64) -> Result<(), String> {
    ensure_row(conn)?;
    conn.execute(
        "UPDATE auth_state SET refresh_token = ?1, refresh_expires_at = ?2, last_validated_at = ?3, user_id = COALESCE(?4, user_id) WHERE id = 1",
        params![refresh_token, refresh_expires_at, now, user_id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn clear(conn: &Connection) -> Result<(), String> {
    ensure_row(conn)?;
    conn.execute(
        "UPDATE auth_state SET refresh_token = NULL, refresh_expires_at = NULL, last_validated_at = NULL, user_id = NULL WHERE id = 1",
        [],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn auth_load(state: State<'_, DbState>) -> Result<StoredAuth, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    load(&conn, now_seconds())
}

#[tauri::command]
pub fn auth_save(state: State<'_, DbState>, refresh_token: &str, refresh_expires_at: &str, user_id: Option<String>) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    save(&conn, refresh_token, refresh_expires_at, user_id.as_deref(), now_seconds())
}

#[tauri::command]
pub fn auth_clear(state: State<'_, DbState>) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    clear(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn grace_counts_down_from_the_last_validation() {
        assert_eq!(grace_remaining(None, 1000), 0);
        assert_eq!(grace_remaining(Some(1000), 1000), GRACE_SECONDS);
        assert_eq!(grace_remaining(Some(1000), 1000 + 100), GRACE_SECONDS - 100);
        assert_eq!(grace_remaining(Some(1000), 1000 + GRACE_SECONDS), 0);
        assert_eq!(grace_remaining(Some(1000), 1000 + GRACE_SECONDS + 5), 0);
    }

    #[test]
    fn a_clock_set_back_before_the_last_validation_expires_the_grace() {
        assert_eq!(grace_remaining(Some(5000), 4999), 0);
    }

    #[test]
    fn device_id_is_generated_once_and_survives_save_and_clear() {
        let conn = conn();
        let first = load(&conn, 100).unwrap();
        assert!(!first.device_id.is_empty());
        assert_eq!(first.refresh_token, None);
        assert_eq!(first.grace_remaining_seconds, 0);

        save(&conn, "token-1", "2027-01-01T00:00:00Z", None, 100).unwrap();
        clear(&conn).unwrap();
        assert_eq!(load(&conn, 200).unwrap().device_id, first.device_id);
    }

    #[test]
    fn saving_stores_the_rotated_token_and_restarts_the_grace_window() {
        let conn = conn();
        save(&conn, "token-1", "2027-01-01T00:00:00Z", None, 100).unwrap();
        save(&conn, "token-2", "2027-02-01T00:00:00Z", None, 500).unwrap();
        let stored = load(&conn, 600).unwrap();
        assert_eq!(stored.refresh_token.as_deref(), Some("token-2"));
        assert_eq!(stored.refresh_expires_at.as_deref(), Some("2027-02-01T00:00:00Z"));
        assert_eq!(stored.grace_remaining_seconds, GRACE_SECONDS - 100);
    }

    #[test]
    fn clearing_removes_the_token_and_the_grace() {
        let conn = conn();
        save(&conn, "token-1", "2027-01-01T00:00:00Z", None, 100).unwrap();
        clear(&conn).unwrap();
        let stored = load(&conn, 101).unwrap();
        assert_eq!(stored.refresh_token, None);
        assert_eq!(stored.grace_remaining_seconds, 0);
    }
}
