mod db;

use std::sync::Mutex;
use tauri::State;
use tauri::Manager;
use tauri_plugin_opener;
use rusqlite::OptionalExtension;
use serde::Serialize;

#[derive(Serialize)]
pub struct Show {
    pub id: i64,
    pub name: String,
    pub time: i64,
}

#[derive(Serialize)]
pub struct ShowRole {
    pub id: i64,
    pub show_id: i64,
    pub name: String,
}

#[derive(Serialize)]
pub struct ScenarioLineComment {
    pub id: i64,
    pub line_id: i64,
    pub role_id: Option<i64>, // Nullable foreign key
    pub comment: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct ScenarioLine {
    pub id: i64,
    pub show_id: i64,
    pub line_number: i64,
    pub content: String,
}

// Master struct sent over to JS
#[derive(Serialize)]
pub struct ShowDetails {
    pub show: Show,
    pub roles: Vec<ShowRole>,
    pub lines: Vec<ScenarioLine>,
    pub comments: Vec<ScenarioLineComment>,
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
    let conn = state.0.lock().map_err(|e| e.to_string())?;

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
        .prepare("SELECT scenario_line_id, show_id, scenario_line_order, scenario_line_name FROM show_scenario_line WHERE show_id = ?1 ORDER BY scenario_line_order ASC")
        .map_err(|e| e.to_string())?;

    let lines = lines_stmt
        .query_map([show_id], |row| {
            Ok(ScenarioLine {
                id: row.get(0)?,
                show_id: row.get(1)?,
                line_number: row.get(2)?,
                content: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 4. Fetch Comments for lines in this show
    let mut comments_stmt = conn
        .prepare(
            "SELECT c.id, c.line_id, c.role_id, c.comment, c.created_at 
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
                comment: row.get(3)?,
                created_at: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Return the aggregated struct
    Ok(ShowDetails {
        show,
        roles,
        lines,
        comments,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let conn = db::init_db().expect("Failed to initialize database");
            app.manage(db::DbState(Mutex::new(conn)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            new_show,
            get_shows,
            delete_show,
            get_full_show_details
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}