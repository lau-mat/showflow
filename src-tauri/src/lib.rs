mod db;
mod session;

use std::sync::{Arc, Mutex};
use tauri::State;
use tauri::Manager;
use tauri_plugin_opener;
use serde::{Serialize, Deserialize};
use rusqlite::Connection;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Show {
    pub id: i64,
    pub name: String,
    pub time: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShowRole {
    pub id: i64,
    pub show_id: i64,
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScenarioLineComment {
    pub id: i64,
    pub line_id: i64,
    pub role_id: Option<i64>, // Nullable foreign key
    pub comment: String
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ScenarioLine {
    pub id: i64,
    pub order: i64,
    pub name: String,
    pub comment: String,
    pub time: String,
    pub time_mode: i64
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ShowDetails {
    pub show: Show,
    pub roles: Vec<ShowRole>,
    pub lines: Vec<ScenarioLine>,
    pub comments: Vec<ScenarioLineComment>,
}

pub fn fetch_show_details(db: &Arc<Mutex<Connection>>, show_id: i64) -> Result<ShowDetails, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    // 1. Fetch the main Show
    let show = conn.query_row(
        "SELECT show_name, show_time FROM shows WHERE show_id = ?1",
        [show_id],
        |row| {
            Ok(Show {
                id: show_id,
                name: row.get(0)?,
                time: row.get(1)?,
            })
        },
    ).map_err(|e| format!("Show not found: {}", e))?;

    // 2. Fetch Roles for this show
    let mut roles_stmt = conn
        .prepare("SELECT role_id, show_id, role_name FROM show_roles WHERE show_id = ?1")
        .map_err(|e| e.to_string())?;
    
    let roles = roles_stmt
        .query_map([show_id], |row| {
            Ok(ShowRole {
                id: row.get(0)?,
                show_id: row.get(1)?,
                name: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 3. Fetch Scenario Lines for this show
    let mut lines_stmt = conn
        .prepare("SELECT scenario_line_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_time, scenario_line_time_mode FROM show_scenario_line WHERE show_id = ?1 ORDER BY scenario_line_order ASC")
        .map_err(|e| e.to_string())?;

    let lines = lines_stmt
        .query_map([show_id], |row| {
            Ok(ScenarioLine {
                id: row.get(0)?,
                order: row.get(1)?,
                name: row.get(2)?,
                comment: row.get(3)?,
                time: row.get(4)?,
                time_mode: row.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 4. Fetch Comments for lines in this show
    let mut comments_stmt = conn
        .prepare(
            "SELECT c.id, c.line_id, c.role_id, c.comment 
            FROM show_scenario_line_comment c
            JOIN show_scenario_line l ON c.line_id = l.scenario_line_id
            WHERE l.show_id = ?1"
        )
        .map_err(|e| e.to_string())?;

    let comments = comments_stmt
        .query_map([show_id], |row| {
            Ok(ScenarioLineComment {
                id: row.get(0)?,
                line_id: row.get(1)?,
                role_id: row.get(2)?,
                comment: row.get(3)?
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(ShowDetails {
        show,
        roles,
        lines,
        comments,
    })
}

#[tauri::command]
fn new_show(state: State<'_, db::DbState>, name: &str, time: i64) -> Result<Show, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO shows (show_name, show_time) VALUES (?1, ?2)",
        rusqlite::params![name, time],
    ).map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();

    let show = Show {
        id,
        name: name.to_string(),
        time: time,
    };
    println!("New show created: {} at {}", show.name, show.time);
    Ok(show)
}

#[tauri::command]
fn get_shows(state: State<'_, db::DbState>) -> Result<Vec<Show>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;  
    let mut stmt = conn.prepare("SELECT show_id, show_name, show_time FROM shows").map_err(|e| e.to_string())?;
    let show_iter = stmt.query_map([], |row| {
        Ok(Show {
            id: row.get(0)?,
            name: row.get(1)?,
            time: row.get(2)?,
        })
    }).map_err(|e| e.to_string())?;

    let mut shows = Vec::new();
    for show in show_iter {
        shows.push(show.map_err(|e| e.to_string())?);
    }
    Ok(shows)
}

#[tauri::command]
fn delete_show(state: State<'_, db::DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM shows WHERE show_id = ?1",
        &[&id],
    ).map_err(|e| e.to_string())?;
    println!("Show with ID {} deleted", id);
    Ok(())
}

#[tauri::command]
fn get_full_show_details(state: State<'_, db::DbState>, show_id: i64) -> Result<ShowDetails, String> {
    let response = fetch_show_details(&state.0, show_id)?;
    Ok(response)
}

#[tauri::command]
fn add_role(state: State<'_, db::DbState>, show_id: i64, role_name: &str) -> Result<ShowRole, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO show_roles (show_id, role_name) VALUES (?1, ?2)",
        rusqlite::params![show_id, role_name],
    ).map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();

    let role = ShowRole {
        id,
        show_id,
        name: role_name.to_string(),
    };
    println!("New role created: {} for show ID {}", role.name, role.show_id);
    Ok(role)
}

#[tauri::command]
fn add_scenario_line(state: State<'_, db::DbState>, show_id: i64, line_order: i64, line_name: &str, line_comment: &str, line_time: &str, line_time_mode: i64) -> Result<ScenarioLine, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO show_scenario_line (show_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_time, scenario_line_time_mode) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![show_id, line_order, line_name, line_comment, line_time, line_time_mode],
    ).map_err(|e| e.to_string())?;

    let line_id = conn.last_insert_rowid();

    let scenario_line = ScenarioLine {
        id: line_id,
        order: line_order,
        name: line_name.to_string(),
        comment: line_comment.to_string(),
        time: line_time.to_string(),
        time_mode: line_time_mode,
    };
    println!("New scenario line created: {} for show ID {}", scenario_line.name, show_id);
    Ok(scenario_line)
}

#[tauri::command]
fn delete_scenario_line(state: State<'_, db::DbState>, line_id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM show_scenario_line WHERE scenario_line_id = ?1",
        rusqlite::params![line_id],
    ).map_err(|e| e.to_string())?;
    println!("Scenario line with ID {} deleted", line_id);
    Ok(())
}

#[tauri::command]
fn add_scenario_line_comment(state: State<'_, db::DbState>, line_id: i64, role_id: Option<i64>, comment: &str) -> Result<ScenarioLineComment, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO show_scenario_line_comment (line_id, role_id, comment) VALUES (?1, ?2, ?3)",
        rusqlite::params![line_id, role_id, comment],
    ).map_err(|e| e.to_string())?;

    let comment_id = conn.last_insert_rowid();

    let scenario_line_comment = ScenarioLineComment {
        id: comment_id,
        line_id,
        role_id,
        comment: comment.to_string()
    };
    println!("New comment added to line ID {}: {}", line_id, scenario_line_comment.comment);
    Ok(scenario_line_comment)
}

#[tauri::command]
fn edit_scenario_line_comment(state: State<'_, db::DbState>, comment_id: i64, role_id: Option<i64>, comment: &str) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "UPDATE show_scenario_line_comment SET comment = ?1, role_id = ?2 WHERE id = ?3",
        rusqlite::params![comment, role_id, comment_id],
    ).map_err(|e| e.to_string())?;

    println!("Comment with ID {} updated to: {}", comment_id, comment);
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
                session: Mutex::new(None),
                shutdown_tx: Mutex::new(None),
            });
            
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            new_show,
            get_shows,
            delete_show,
            get_full_show_details,
            add_role,
            add_scenario_line,
            delete_scenario_line,
            add_scenario_line_comment,
            edit_scenario_line_comment,
            session::start_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}