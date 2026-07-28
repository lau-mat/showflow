use std::sync::Mutex;
use rusqlite::Connection;
use tauri::State;
use tauri::Manager;
use serde::Serialize;

pub struct DbState(pub Mutex<Connection>);

#[derive(Serialize)]
pub struct Show {
    pub id: i64,
    pub name: String,
    pub time: String,
}

fn init_db() -> Result<Connection, String> {
    let conn = Connection::open("shows.db").map_err(|e| e.to_string())?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS shows (
            id INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            time TEXT NOT NULL
        )",
        [],
    ).map_err(|e| e.to_string())?;
    Ok(conn)
}

#[tauri::command]
fn new_show(state: State<'_, DbState>, name: &str, time: &str) -> Result<Show, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;

    conn.execute(
        "INSERT INTO shows (name, time) VALUES (?1, ?2)",
        &[name, time],
    ).map_err(|e| e.to_string())?;

    let id = conn.last_insert_rowid();

    let show = Show {
        id,
        name: name.to_string(),
        time: time.to_string(),
    };
    println!("New show created: {} at {}", show.name, show.time);
    Ok(show)
}

#[tauri::command]
fn get_shows(state: State<'_, DbState>) -> Result<Vec<Show>, String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;  
    let mut stmt = conn.prepare("SELECT id, name, time FROM shows").map_err(|e| e.to_string())?;
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
fn delete_show(state: State<'_, DbState>, id: i64) -> Result<(), String> {
    let conn = state.0.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM shows WHERE id = ?1",
        &[&id],
    ).map_err(|e| e.to_string())?;
    println!("Show with ID {} deleted", id);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let conn = init_db().expect("Failed to initialize database");
            app.manage(DbState(Mutex::new(conn)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            new_show,
            get_shows,
            delete_show
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}