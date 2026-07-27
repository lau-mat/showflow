use std::sync::Mutex;
use tauri::{AppHandle, Manager, State};
use rusqlite::Connection;

pub struct DbState(pub Mutex<Connection>);

fn init_shared_db(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Get the official app data directory
    let mut db_path = app.path().app_data_dir()?;
    std::fs::create_dir_all(&db_path)?;
    db_path.push("showflow.db");

    // 2. Open connection in Rust
    let conn = Connection::open(&db_path)?;

    // 3. IMPORTANT: Enable WAL Mode & set a busy timeout for concurrency
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "busy_timeout", "5000")?; // Waits up to 5s if locked instead of failing immediately

    // 4. Create base tables if they don't exist
    conn.execute(
        "CREATE TABLE IF NOT EXISTS show (
            show_id INTEGER PRIMARY KEY AUTOINCREMENT,
            show_name TEXT NOT NULL,
            show_time TEXT NOT NULL,
            show_status TEXT DEFAULT 'idle'
        )",
        [],
    )?;

    app.manage(DbState(Mutex::new(conn)));
    Ok(())
}

#[tauri::command]
fn new_show(name: &str, time: &str) -> String {
    println!("{}", format!("New show created: {} at {}", name, time));
    format!("New show created: {} at {}", name, time)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_sql::Builder::default().build())
        .setup(|app| {
            init_shared_db(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            new_show
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}