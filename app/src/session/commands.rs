//! What the host computer can do with its sessions. These are Tauri commands on purpose: managing
//! a session (who may call, approvals, ending it) is only possible from inside the app, never over
//! the session's network socket.

use std::sync::Arc;

use protocol::discovery::DISCOVERY_PORT;
use protocol::session::{AccessKind, AccessPolicy, AccessSettings};
use qrcode::render::svg;
use qrcode::QrCode;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use showlogic::HostView;
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::discovery::{self, FoundSession};
use super::ports::PortRange;
use super::registry::Sessions;
use super::server::{now, OnChange, RunningSession};
use crate::caller_store;
use crate::db::DbState;
use crate::store;

#[derive(Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub show_id: String,
    pub show_name: String,
    /// The one address everyone joins with.
    pub url: String,
}

#[derive(Serialize)]
pub struct SessionStatus {
    #[serde(flatten)]
    pub info: SessionInfo,
    pub view: HostView,
    /// This computer has what it needs to check account passes for the show.
    pub account_ready: bool,
    /// Other apps on the network can find this session without typing its address.
    pub discoverable: bool,
}

fn join_url(port: u16) -> String {
    // Offline without any network, other devices cannot reach us anyway.
    let ip = local_ip_address::local_ip().map(|ip| ip.to_string()).unwrap_or_else(|_| "127.0.0.1".into());
    format!("http://{ip}:{port}")
}

fn info(session: &RunningSession) -> SessionInfo {
    SessionInfo {
        session_id: session.session_id.clone(),
        show_id: session.show_id.clone(),
        show_name: session.show_name.clone(),
        url: join_url(session.http_port),
    }
}

fn window_label(session_id: &str) -> String {
    format!("session-manager-{session_id}")
}

/// What was last set for this show, or the defaults (anyone joins, calling is approved).
pub fn load_settings(conn: &Connection, show_id: &str) -> Result<AccessSettings, String> {
    let row = conn
        .query_row("SELECT policy, code, view_policy, view_code FROM session_settings WHERE show_id = ?1", [show_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?))
        })
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((caller, caller_code, view, view_code)) = row else { return Ok(AccessSettings::default()) };
    let read = |json: &str| serde_json::from_str::<AccessPolicy>(json).map(AccessPolicy::single).map_err(|e| e.to_string());
    Ok(AccessSettings {
        caller: read(&caller)?,
        caller_code,
        view: view.as_deref().map(read).transpose()?.unwrap_or(AccessSettings::default().view),
        view_code,
    })
}

/// Remembers who may join and call this show. Codes are stored unencrypted, like the app's login
/// token.
pub fn save_settings(conn: &Connection, show_id: &str, settings: &AccessSettings) -> Result<(), String> {
    let json = |policy: AccessPolicy| serde_json::to_string(&policy).map_err(|e| e.to_string());
    conn.execute(
        "INSERT INTO session_settings (show_id, policy, code, view_policy, view_code) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(show_id) DO UPDATE SET policy = excluded.policy, code = excluded.code,
             view_policy = excluded.view_policy, view_code = excluded.view_code",
        params![show_id, json(settings.caller)?, settings.caller_code, json(settings.view)?, settings.view_code],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn remember(db: &DbState, session: &RunningSession) -> Result<(), String> {
    let settings = session.inspect(|logic| logic.settings());
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    save_settings(&conn, &session.show_id, &settings)
}

/// Starts at once, with the show's settings as they are.
#[tauri::command]
pub async fn start_session(show_id: String, sessions: State<'_, Sessions>, db: State<'_, DbState>, app: AppHandle) -> Result<SessionInfo, String> {
    if cfg!(mobile) {
        return Err("Hosting a session is not available on this platform".into());
    }
    let (show_name, settings) = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        (store::fetch_show_details(&conn, &show_id)?.show.name, load_settings(&conn, &show_id)?)
    };
    let on_change: OnChange = Arc::new(move |session_id: &str| {
        let _ = app.emit("session-updated", session_id);
    });
    let session = sessions
        .start(&show_id, show_name, settings, db.0.clone(), PortRange::from_env(), on_change.clone())
        .await?;
    // Generated codes are kept, so they stay the same next time.
    remember(&db, &session)?;
    on_change(&session.session_id);
    Ok(info(&session))
}

#[tauri::command]
pub async fn list_sessions(sessions: State<'_, Sessions>) -> Result<Vec<SessionInfo>, String> {
    Ok(sessions.list().await.iter().map(|s| info(s)).collect())
}

#[tauri::command]
pub async fn session_status(session_id: String, sessions: State<'_, Sessions>, db: State<'_, DbState>) -> Result<SessionStatus, String> {
    let session = sessions.get(&session_id).await?;
    let view = session.inspect(|logic| logic.host_view(now()));
    let account_ready = {
        let conn = db.0.lock().map_err(|e| e.to_string())?;
        caller_store::ready_to_verify(&conn, &session.show_id)?
    };
    Ok(SessionStatus { info: info(&session), view, account_ready, discoverable: sessions.is_discoverable().await })
}

#[tauri::command]
pub async fn stop_session(session_id: String, sessions: State<'_, Sessions>, app: AppHandle) -> Result<(), String> {
    sessions.stop(&session_id).await?;
    if let Some(window) = app.get_webview_window(&window_label(&session_id)) {
        let _ = window.close();
    }
    let _ = app.emit("session-updated", &session_id);
    Ok(())
}

/// A typed code, or (without one) a generated hexadecimal code of `length` characters.
#[tauri::command]
pub async fn set_access_code(
    session_id: String,
    kind: AccessKind,
    code: Option<String>,
    length: Option<usize>,
    sessions: State<'_, Sessions>,
    db: State<'_, DbState>,
) -> Result<(), String> {
    let session = sessions.get(&session_id).await?;
    session.act(|logic| match logic.set_code(kind, code, length) {
        Ok(effects) => (effects, Ok(())),
        Err(error) => (Vec::new(), Err(error.to_string())),
    })?;
    remember(&db, &session)
}

/// Approves or denies a device waiting to join or to call.
#[tauri::command]
pub async fn answer_access_request(
    session_id: String,
    client_id: String,
    kind: AccessKind,
    approve: bool,
    sessions: State<'_, Sessions>,
) -> Result<(), String> {
    sessions.get(&session_id).await?.apply(|logic| logic.answer_request(&client_id, kind, approve));
    Ok(())
}

/// Puts a device out of a protected session.
#[tauri::command]
pub async fn remove_device(session_id: String, client_id: String, sessions: State<'_, Sessions>) -> Result<(), String> {
    sessions.get(&session_id).await?.apply(|logic| logic.remove_device(&client_id));
    Ok(())
}

#[tauri::command]
pub async fn revoke_caller(session_id: String, grant_id: String, sessions: State<'_, Sessions>) -> Result<(), String> {
    sessions.get(&session_id).await?.apply(|logic| logic.revoke(&grant_id));
    Ok(())
}

/// The caller view on this computer. Closing it closes only the window; the session goes on.
#[tauri::command]
pub async fn open_caller_window(session_id: String, sessions: State<'_, Sessions>, app: AppHandle) -> Result<(), String> {
    let session = sessions.get(&session_id).await?;
    let label = window_label(&session_id);
    if let Some(existing) = app.get_webview_window(&label) {
        return existing.set_focus().map_err(|e| e.to_string());
    }
    let config = serde_json::json!({ "sessionId": session.session_id, "host": "127.0.0.1", "ws": session.ws_port, "isHost": true });
    WebviewWindowBuilder::new(&app, label, WebviewUrl::App("session/session.html".into()))
        .title(format!("Show caller — {}", session.show_name))
        .inner_size(1024.0, 768.0)
        // Tells the page where its session is; the secret that makes it the caller is fetched
        // separately through IPC.
        .initialization_script(&format!("window.__SHOWFLOW_SESSION__ = {config};"))
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The secret that lets this computer's caller window call. Only that window may ask for it.
#[tauri::command]
pub async fn caller_host_token(session_id: String, window: WebviewWindow, sessions: State<'_, Sessions>) -> Result<String, String> {
    if window.label() != window_label(&session_id) {
        return Err("Only the session's caller window may ask for this".into());
    }
    Ok(sessions.get(&session_id).await?.inspect(|logic| logic.host_token().to_owned()))
}

/// The QR code everyone joins with.
#[tauri::command]
pub async fn session_qr(session_id: String, sessions: State<'_, Sessions>) -> Result<String, String> {
    let session = sessions.get(&session_id).await?;
    let code = QrCode::new(join_url(session.http_port).as_bytes()).map_err(|e| e.to_string())?;
    Ok(code.render::<svg::Color>().min_dimensions(200, 200).build())
}

/// Sets who may join and call a show, running or not. A running session follows at once.
#[tauri::command]
pub async fn save_session_settings(
    show_id: String,
    settings: AccessSettings,
    sessions: State<'_, Sessions>,
    db: State<'_, DbState>,
) -> Result<(), String> {
    for code in [&settings.caller_code, &settings.view_code].into_iter().flatten() {
        showlogic::code::validate(code).map_err(|e| e.to_string())?;
    }
    let settings = AccessSettings { caller: settings.caller.normalized(), view: settings.view.normalized(), ..settings };
    match sessions.list().await.into_iter().find(|session| session.show_id == show_id) {
        Some(session) => {
            session.act(|logic| match logic.apply_settings(settings) {
                Ok(effects) => (effects, Ok(())),
                Err(error) => (Vec::new(), Err(error.to_string())),
            })?;
            remember(&db, &session)
        }
        None => {
            let conn = db.0.lock().map_err(|e| e.to_string())?;
            save_settings(&conn, &show_id, &settings)
        }
    }
}

/// Sessions other computers on the network are running right now.
#[tauri::command]
pub async fn discover_sessions(sessions: State<'_, Sessions>) -> Result<Vec<FoundSession>, String> {
    let mut found = discovery::search(DISCOVERY_PORT, discovery::SEARCH_TIME).await;
    // Our own are managed from the show list.
    let own: Vec<String> = sessions.list().await.iter().map(|s| s.session_id.clone()).collect();
    found.retain(|session| !own.contains(&session.session_id));
    Ok(found)
}

#[tauri::command]
pub fn session_settings(show_id: String, db: State<'_, DbState>) -> Result<AccessSettings, String> {
    let conn = db.0.lock().map_err(|e| e.to_string())?;
    load_settings(&conn, &show_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn access_settings_are_remembered_per_show_and_default_to_open_joining_with_approved_callers() {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        assert_eq!(load_settings(&conn, "a").unwrap(), AccessSettings::default());
        let settings = AccessSettings {
            caller: AccessPolicy { code: true, ..Default::default() },
            caller_code: Some("P@ssw0rd!".into()),
            view: AccessPolicy { request: true, ..Default::default() },
            view_code: Some("AABBCCDD".into()),
        };
        save_settings(&conn, "a", &settings).unwrap();
        assert_eq!(load_settings(&conn, "a").unwrap(), settings);
        save_settings(&conn, "a", &AccessSettings::default()).unwrap();
        assert_eq!(load_settings(&conn, "a").unwrap(), AccessSettings::default());
        assert_eq!(load_settings(&conn, "b").unwrap(), AccessSettings::default());
    }

    #[test]
    fn settings_saved_before_joining_could_be_protected_keep_it_open() {
        let mut conn = Connection::open_in_memory().unwrap();
        db::migrate(&mut conn).unwrap();
        conn.execute("INSERT INTO session_settings (show_id, policy, code) VALUES ('a', '{\"code\":true}', 'abc123')", []).unwrap();
        let settings = load_settings(&conn, "a").unwrap();
        assert!(settings.view.open && settings.caller.code);
        assert_eq!((settings.caller_code.as_deref(), settings.view_code), (Some("abc123"), None));
    }
}
