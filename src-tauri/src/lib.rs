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
fn get_show_content(state: State<'_, db::DbState>, id: i64) -> Result<Show, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn.prepare("SELECT show_id, show_name, show_time FROM shows WHERE show_id = ?1").map_err(|e| e.to_string())?;
    let show_opt = stmt.query_row([id], |row| {
        Ok(Show {
            id: row.get(0)?,
            name: row.get(1)?,
            time: row.get(2)?,
        })
    }).optional().map_err(|e| e.to_string())?;

    match show_opt {
        Some(show) => Ok(show),
        None => Err(format!("Show with ID {} not found", id)),
    }
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
            get_show_content
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}