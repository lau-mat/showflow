mod auth_store;
mod db;
mod platform;
mod session;
mod store;
mod sync_store;

use std::sync::{Arc, Mutex};
use tauri::{State, Manager, Emitter};
use tauri_plugin_opener;
use protocol::show::{MemberRole, ScenarioLine, ScenarioLineComment, Show, ShowDetails, ShowRole};
use protocol::sync::{EntityKind, OpKind, Place};
use serde_json::json;
use uuid::Uuid;

use store::{apply_local, new_op};

fn parse_id(id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(id).map_err(|_| format!("Invalid id: {id}"))
}

#[tauri::command]
fn new_show(state: State<'_, db::DbState>, name: &str, time: i64) -> Result<Show, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4();
    apply_local(&mut conn, &id.to_string(), new_op(EntityKind::Show, id, OpKind::Set, json!({ "name": name, "time": time })))?;
    Ok(Show { id: id.to_string(), name: name.to_string(), time, role: MemberRole::Owner })
}

/// Changes the show's `name` and/or `time`; only the fields given are touched.
#[tauri::command]
fn edit_show(state: State<'_, db::DbState>, show_id: &str, fields: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut op = new_op(EntityKind::Show, parse_id(show_id)?, OpKind::Set, serde_json::Value::Null);
    op.fields = fields;
    apply_local(&mut conn, show_id, op)?;
    Ok(())
}

#[tauri::command]
fn get_shows(state: State<'_, db::DbState>) -> Result<Vec<Show>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let account = store::current_account(&conn)?;
    store::list_shows(&conn, account.as_deref())
}

#[tauri::command]
fn delete_show(state: State<'_, db::DbState>, id: &str) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    apply_local(&mut conn, id, new_op(EntityKind::Show, parse_id(id)?, OpKind::Delete, json!({})))?;
    Ok(())
}

#[tauri::command]
fn get_full_show_details(state: State<'_, db::DbState>, show_id: &str) -> Result<ShowDetails, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    store::fetch_show_details(&conn, show_id)
}

#[tauri::command]
fn add_role(state: State<'_, db::DbState>, show_id: &str, role_name: &str) -> Result<ShowRole, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4();
    apply_local(&mut conn, show_id, new_op(EntityKind::Role, id, OpKind::Set, json!({ "name": role_name })))?;
    Ok(ShowRole { id: id.to_string(), show_id: show_id.to_string(), name: role_name.to_string() })
}

/// `after` is the line to insert behind (`None`: at the start of the script).
#[tauri::command]
fn add_scenario_line(state: State<'_, db::DbState>, show_id: &str, after: Option<String>, line_name: &str, line_note: &str, duration: i64, absolute_time: Option<i64>) -> Result<ScenarioLine, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4();
    let mut op = new_op(EntityKind::Line, id, OpKind::Set, json!({
        "name": line_name, "note": line_note, "duration": duration, "absolute_time": absolute_time
    }));
    op.place = Some(Place { after: after.as_deref().map(parse_id).transpose()? });
    let queued = apply_local(&mut conn, show_id, op)?;
    Ok(ScenarioLine {
        id: id.to_string(),
        position: queued.fields["position"].as_str().unwrap_or_default().to_string(),
        name: line_name.to_string(),
        note: line_note.to_string(),
        duration,
        absolute_time,
    })
}

/// Changes some fields of a cue (`name`, `note`, `duration`, `absolute_time`); only those given are
/// touched. Moving a cue is not an edit: its position is not accepted here.
#[tauri::command]
fn edit_scenario_line(state: State<'_, db::DbState>, line_id: &str, fields: serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    if fields.keys().any(|key| key == "position") {
        return Err("A cue cannot be moved with an edit".into());
    }
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let show_id = store::show_for(&mut conn, EntityKind::Line, line_id)?;
    let mut op = new_op(EntityKind::Line, parse_id(line_id)?, OpKind::Set, serde_json::Value::Null);
    op.fields = fields;
    apply_local(&mut conn, &show_id, op)?;
    Ok(())
}

#[tauri::command]
fn delete_scenario_line(state: State<'_, db::DbState>, line_id: &str) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let show_id = store::show_for(&mut conn, EntityKind::Line, line_id)?;
    apply_local(&mut conn, &show_id, new_op(EntityKind::Line, parse_id(line_id)?, OpKind::Delete, json!({})))?;
    Ok(())
}

#[tauri::command]
fn add_scenario_line_comment(state: State<'_, db::DbState>, line_id: &str, role_id: Option<String>, comment: &str) -> Result<ScenarioLineComment, String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let show_id = store::show_for(&mut conn, EntityKind::Line, line_id)?;
    let id = Uuid::new_v4();
    apply_local(&mut conn, &show_id, new_op(EntityKind::Comment, id, OpKind::Set, json!({
        "line_id": line_id, "role_id": role_id, "comment": comment
    })))?;
    Ok(ScenarioLineComment { id: id.to_string(), line_id: line_id.to_string(), role_id, comment: comment.to_string() })
}

#[tauri::command]
fn edit_scenario_line_comment(state: State<'_, db::DbState>, comment_id: &str, role_id: Option<String>, comment: &str) -> Result<(), String> {
    let mut conn = state.0.lock().map_err(|e| e.to_string())?;
    let show_id = store::show_for(&mut conn, EntityKind::Comment, comment_id)?;
    apply_local(&mut conn, &show_id, new_op(EntityKind::Comment, parse_id(comment_id)?, OpKind::Set, json!({
        "role_id": role_id, "comment": comment
    })))?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let conn = db::init_db().expect("Failed to initialize database");
            app.manage(db::DbState(Arc::new(Mutex::new(conn))));

            app.manage(session::AppState {
            session: Arc::new(Mutex::new(None)),
                shutdown_tx: Mutex::new(None),  
            });
            
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // Check if the closed window is the master window
                if window.label() == "session-manager" {
                    let state = window.state::<session::AppState>();

                    if let Ok(mut session_guard) = state.session.lock() {
                        *session_guard = None;
                    }
                    
                    // Take the channel out and send the broadcast shutdown signal
                    if let Some(tx) = state.shutdown_tx.lock().unwrap().take() {
                        let _ = tx.send(()); // Triggers both rx_http and rx_ws
                    };

                    let _ = window.emit("session-stopped", ());
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            new_show,
            get_shows,
            edit_show,
            delete_show,
            get_full_show_details,
            add_role,
            add_scenario_line,
            edit_scenario_line,
            delete_scenario_line,
            add_scenario_line_comment,
            edit_scenario_line_comment,
            session::start_session,
            session::generate_server_qr,
            platform::platform_info,
            auth_store::auth_load,
            auth_store::auth_save,
            auth_store::auth_clear,
            sync_store::sync_state,
            sync_store::sync_pending,
            sync_store::sync_ack,
            sync_store::sync_apply_changes,
            sync_store::sync_forget_show,
            sync_store::sync_adopt_account
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}