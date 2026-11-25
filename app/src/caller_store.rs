//! Account-based caller access on this computer, both sides of it.
//!
//! As a member: the device's own key, the passes the cloud issued to it, and answering a host's
//! challenge (`claim`). As a host: the cloud's public key and, per show, who is a member (may join)
//! and who may call, refreshed by the sync engine so passes can be checked without internet
//! (`AccountVerifier`). A pass only says who someone is; these lists say what they may do.

use std::collections::HashMap;

use protocol::caller_pass::{parse_public_key, public_key_text, sign_proof, signing_key, verify_pass, verify_proof, SigningKey};
use rusqlite::{params, Connection, OptionalExtension};
use protocol::session::AccessKind;
use serde::{Deserialize, Serialize};
use showlogic::{PassContext, PassVerifier, VerifiedUser};
use tauri::{AppHandle, Emitter, State};

use crate::db::DbState;
use crate::session::server::{now, Db};
use crate::session::Sessions;
use crate::store::current_account;

#[derive(Serialize, Debug, PartialEq)]
pub struct HeldPass {
    pub show_id: String,
    pub expires_at: i64,
}

/// What the sync engine needs to know to keep this device's passes fresh.
#[derive(Serialize, Debug)]
pub struct AccessState {
    pub device_key: String,
    /// The signed-in account's passes.
    pub passes: Vec<HeldPass>,
}

/// A claim for one connection to one host.
#[derive(Serialize, Debug)]
pub struct ClaimAnswer {
    pub pass: String,
    pub proof: String,
}

fn err(error: rusqlite::Error) -> String {
    error.to_string()
}

/// This installation's secret key, created the first time it is needed. It never leaves the app;
/// like the login token it is stored unencrypted.
fn device_key(conn: &Connection) -> Result<SigningKey, String> {
    conn.execute("INSERT OR IGNORE INTO caller_device (id, seed) VALUES (1, ?1)", [showlogic::code::random_hex(64)])
        .map_err(err)?;
    let hex: String = conn.query_row("SELECT seed FROM caller_device WHERE id = 1", [], |row| row.get(0)).map_err(err)?;
    let mut seed = [0u8; 32];
    for (index, byte) in seed.iter_mut().enumerate() {
        let pair = hex.get(index * 2..index * 2 + 2).ok_or("The device key is damaged")?;
        *byte = u8::from_str_radix(pair, 16).map_err(|_| "The device key is damaged")?;
    }
    Ok(signing_key(seed))
}

pub fn state(conn: &Connection) -> Result<AccessState, String> {
    let device_key = public_key_text(&device_key(conn)?);
    let passes = match current_account(conn)? {
        None => Vec::new(),
        Some(account) => conn
            .prepare("SELECT show_id, expires_at FROM caller_passes WHERE user_id = ?1 ORDER BY show_id")
            .map_err(err)?
            .query_map([account], |row| Ok(HeldPass { show_id: row.get(0)?, expires_at: row.get(1)? }))
            .map_err(err)?
            .collect::<Result<_, _>>()
            .map_err(err)?,
    };
    Ok(AccessState { device_key, passes })
}

pub fn save_pass(conn: &Connection, show_id: &str, pass: &str, expires_at: i64) -> Result<(), String> {
    let account = current_account(conn)?.ok_or("Not signed in")?;
    conn.execute(
        "INSERT INTO caller_passes (show_id, user_id, pass, expires_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(show_id, user_id) DO UPDATE SET pass = excluded.pass, expires_at = excluded.expires_at",
        params![show_id, account, pass, expires_at],
    )
    .map_err(err)?;
    Ok(())
}

/// The signed-in account's answer to a host's challenge, if it holds a pass for the show.
pub fn claim(conn: &Connection, show_id: &str, session_id: &str, nonce: &str, now: i64) -> Result<Option<ClaimAnswer>, String> {
    let Some(account) = current_account(conn)? else { return Ok(None) };
    let pass: Option<String> = conn
        .query_row(
            "SELECT pass FROM caller_passes WHERE show_id = ?1 AND user_id = ?2 AND expires_at > ?3",
            params![show_id, account, now],
            |row| row.get(0),
        )
        .optional()
        .map_err(err)?;
    pass.map(|pass| Ok(ClaimAnswer { proof: sign_proof(&device_key(conn)?, session_id, nonce), pass })).transpose()
}

/// Who belongs to one show, by account id.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ShowAccess {
    pub members: Vec<String>,
    pub callers: Vec<String>,
}

impl ShowAccess {
    pub fn allows(&self, user_id: &str, kind: AccessKind) -> bool {
        let list = match kind {
            AccessKind::View => &self.members,
            AccessKind::Call => &self.callers,
        };
        list.iter().any(|id| id == user_id)
    }
}

/// What the cloud said last: its public key and who belongs to each show.
pub fn save_trust(conn: &mut Connection, public_key: &str, lists: &HashMap<String, ShowAccess>) -> Result<(), String> {
    if parse_public_key(public_key).is_none() {
        return Err("The server sent an invalid caller key".into());
    }
    let tx = conn.transaction().map_err(err)?;
    tx.execute(
        "INSERT INTO caller_trust (id, public_key) VALUES (1, ?1) ON CONFLICT(id) DO UPDATE SET public_key = excluded.public_key",
        [public_key],
    )
    .map_err(err)?;
    let json = |ids: &[String]| serde_json::to_string(ids).map_err(|e| e.to_string());
    for (show_id, access) in lists {
        tx.execute(
            "INSERT INTO caller_lists (show_id, callers, members) VALUES (?1, ?2, ?3)
             ON CONFLICT(show_id) DO UPDATE SET callers = excluded.callers, members = excluded.members",
            params![show_id, json(&access.callers)?, json(&access.members)?],
        )
        .map_err(err)?;
    }
    tx.commit().map_err(err)
}

fn trusted_key(conn: &Connection) -> Result<Option<String>, String> {
    conn.query_row("SELECT public_key FROM caller_trust WHERE id = 1", [], |row| row.get(0)).optional().map_err(err)
}

/// `None` until this computer has synced the show's lists.
pub fn show_access(conn: &Connection, show_id: &str) -> Result<Option<ShowAccess>, String> {
    conn.query_row("SELECT members, callers FROM caller_lists WHERE show_id = ?1", [show_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })
    .optional()
    .map_err(err)?
    .map(|(members, callers)| {
        let read = |json: String| serde_json::from_str(&json).map_err(|e| e.to_string());
        Ok(ShowAccess { members: read(members)?, callers: read(callers)? })
    })
    .transpose()
}

/// Can this computer check account passes for the show?
pub fn ready_to_verify(conn: &Connection, show_id: &str) -> Result<bool, String> {
    Ok(trusted_key(conn)?.is_some() && show_access(conn, show_id)?.is_some())
}

/// Checks an account claim offline: signed by the cloud, for this show, still valid, made by the
/// device it was issued to, and by someone who still may do what they ask for.
pub fn verify_claim(conn: &Connection, pass: &str, proof: &str, context: &PassContext<'_>) -> Result<VerifiedUser, String> {
    const NOT_SYNCED: &str = "This host has not received the account details yet. It has to go online once.";
    let key = trusted_key(conn)?.and_then(|text| parse_public_key(&text)).ok_or(NOT_SYNCED)?;
    let access = show_access(conn, context.show_id)?.ok_or(NOT_SYNCED)?;
    let body = verify_pass(&key, pass, context.show_id, context.now as i64).map_err(|e| e.to_string())?;
    verify_proof(&body, proof, context.session_id, context.nonce).map_err(|e| e.to_string())?;
    if !access.allows(&body.user_id, context.kind) {
        return Err(match context.kind {
            AccessKind::View => "You are not a member of this show",
            AccessKind::Call => "You are not a caller of this show",
        }
        .into());
    }
    Ok(VerifiedUser { user_id: body.user_id, name: body.username })
}

/// `verify_claim` against the app database, as the show logic wants it.
pub struct AccountVerifier {
    pub db: Db,
}

impl PassVerifier for AccountVerifier {
    fn verify(&self, pass: &str, proof: &str, context: &PassContext<'_>) -> Result<VerifiedUser, String> {
        let conn = self.db.lock().map_err(|_| "The host could not check the pass")?;
        verify_claim(&conn, pass, proof, context)
    }
}

#[tauri::command]
pub fn caller_state(db: State<'_, DbState>) -> Result<AccessState, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    state(&conn)
}

#[tauri::command]
pub fn caller_save_pass(db: State<'_, DbState>, show_id: &str, pass: &str, expires_at: i64) -> Result<(), String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_pass(&conn, show_id, pass, expires_at)
}

/// Stores what the cloud said, and ends the access of anyone a running session let in who is no
/// longer on its show's list.
#[tauri::command]
pub async fn caller_save_trust(
    public_key: String,
    lists: HashMap<String, ShowAccess>,
    db: State<'_, DbState>,
    sessions: State<'_, Sessions>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut conn = db.0.lock().map_err(|e| e.to_string())?;
        save_trust(&mut conn, &public_key, &lists)?;
    }
    for session in sessions.list().await {
        if let Some(allowed) = lists.get(&session.show_id) {
            session.apply(|logic| logic.retain_accounts(|user, kind| allowed.allows(user, kind)));
        }
        // The panel also shows whether passes can be checked at all.
        let _ = app.emit("session-updated", &session.session_id);
    }
    Ok(())
}

/// For the joined session pages: this device's answer to the host's challenge, if it has a pass.
#[tauri::command]
pub fn caller_claim(db: State<'_, DbState>, show_id: &str, session_id: &str, nonce: &str) -> Result<Option<ClaimAnswer>, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    claim(&conn, show_id, session_id, nonce, now() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use protocol::caller_pass::{sign_pass, PassBody, PASS_LIFETIME_SECONDS, PASS_VERSION};

    const NOW: i64 = 1_800_000_000;

    fn database(account: Option<&str>) -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO auth_state (id, device_id, user_id) VALUES (1, 'dev', ?1)", [account]).unwrap();
        conn
    }

    fn cloud() -> SigningKey {
        signing_key([1; 32])
    }

    /// What the cloud would issue to `user` for `device_key`.
    fn issue(user: &str, device_key: &str) -> String {
        sign_pass(&cloud(), &PassBody {
            v: PASS_VERSION,
            show_id: "show".into(),
            user_id: user.into(),
            username: format!("{user} name"),
            device_key: device_key.into(),
            issued_at: NOW,
            expires_at: NOW + PASS_LIFETIME_SECONDS,
        })
    }

    fn context(nonce: &'static str) -> PassContext<'static> {
        PassContext { show_id: "show", session_id: "session", nonce, now: NOW as f64 + 10.0, kind: AccessKind::Call }
    }

    fn trust(host: &mut Connection, callers: &[&str]) {
        let callers: Vec<String> = callers.iter().map(|c| c.to_string()).collect();
        // Everyone who has ever held a pass here is still a member; "sam" also when not a caller.
        let members = vec!["owner".to_string(), "sam".to_string()];
        let lists = HashMap::from([("show".to_string(), ShowAccess { members, callers })]);
        save_trust(host, &public_key_text(&cloud()), &lists).unwrap();
    }

    /// A member's device holding a pass for "show".
    fn member(user: &str) -> Connection {
        let conn = database(Some(user));
        let key = state(&conn).unwrap().device_key;
        save_pass(&conn, "show", &issue(user, &key), NOW + PASS_LIFETIME_SECONDS).unwrap();
        conn
    }

    #[test]
    fn each_installation_has_one_lasting_key_of_its_own() {
        let (a, b) = (database(None), database(None));
        assert_eq!(state(&a).unwrap().device_key, state(&a).unwrap().device_key);
        assert_ne!(state(&a).unwrap().device_key, state(&b).unwrap().device_key);
    }

    #[test]
    fn a_member_device_proves_itself_to_a_host_that_synced_earlier() {
        let device = member("sam");
        let mut host = database(Some("owner"));
        let answer = claim(&device, "show", "session", "nonce-1", NOW).unwrap().expect("a pass is held");

        assert!(!ready_to_verify(&host, "show").unwrap());
        assert!(verify_claim(&host, &answer.pass, &answer.proof, &context("nonce-1")).unwrap_err().contains("go online"));
        trust(&mut host, &["owner", "sam"]);
        assert!(ready_to_verify(&host, "show").unwrap());
        assert!(!ready_to_verify(&host, "other").unwrap());
        assert_eq!(
            verify_claim(&host, &answer.pass, &answer.proof, &context("nonce-1")),
            Ok(VerifiedUser { user_id: "sam".into(), name: "sam name".into() })
        );

        // The same answer on another connection is a replay.
        assert!(verify_claim(&host, &answer.pass, &answer.proof, &context("nonce-2")).is_err());
        // Taken off the list: refused as soon as the host has heard.
        trust(&mut host, &["owner"]);
        assert!(verify_claim(&host, &answer.pass, &answer.proof, &context("nonce-1")).unwrap_err().contains("not a caller"));
        // Still a member, so joining works.
        let joining = PassContext { kind: AccessKind::View, ..context("nonce-1") };
        assert!(verify_claim(&host, &answer.pass, &answer.proof, &joining).is_ok());
    }

    #[test]
    fn a_copied_pass_is_useless_on_another_device() {
        let device = member("sam");
        let thief = database(Some("sam"));
        let stolen: String = device.query_row("SELECT pass FROM caller_passes", [], |row| row.get(0)).unwrap();
        save_pass(&thief, "show", &stolen, NOW + 100).unwrap();
        let answer = claim(&thief, "show", "session", "nonce-1", NOW).unwrap().unwrap();
        let mut host = database(Some("owner"));
        trust(&mut host, &["sam"]);
        assert!(verify_claim(&host, &answer.pass, &answer.proof, &context("nonce-1")).unwrap_err().contains("its own"));
    }

    #[test]
    fn passes_belong_to_the_signed_in_account_and_expire() {
        let device = member("sam");
        assert_eq!(state(&device).unwrap().passes.len(), 1);
        assert!(claim(&device, "show", "s", "n", NOW + PASS_LIFETIME_SECONDS).unwrap().is_none(), "expired");
        assert!(claim(&device, "other", "s", "n", NOW).unwrap().is_none());
        for account in [Some("kim"), None] {
            device.execute("UPDATE auth_state SET user_id = ?1", [account]).unwrap();
            assert!(claim(&device, "show", "s", "n", NOW).unwrap().is_none(), "{account:?}");
            assert!(state(&device).unwrap().passes.is_empty());
        }
        device.execute("UPDATE auth_state SET user_id = 'sam'", []).unwrap();
        assert!(claim(&device, "show", "s", "n", NOW).unwrap().is_some(), "signed in again");
    }

    #[test]
    fn a_broken_key_from_the_server_does_not_replace_the_trusted_one() {
        let mut host = database(Some("owner"));
        trust(&mut host, &[]);
        assert!(save_trust(&mut host, "garbage", &HashMap::new()).is_err());
        assert_eq!(trusted_key(&host).unwrap(), Some(public_key_text(&cloud())));
    }
}
