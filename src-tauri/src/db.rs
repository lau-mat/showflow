use rusqlite_migration::{Migrations, M};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

pub struct DbState(pub Arc<Mutex<Connection>>);

fn get_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE shows (
                show_id INTEGER PRIMARY KEY AUTOINCREMENT,
                show_name TEXT NOT NULL,
                show_time INTEGER NOT NULL,
                show_status INTEGER default 0
            );

            CREATE TABLE show_roles (
                role_id INTEGER PRIMARY KEY AUTOINCREMENT,
                show_id INTEGER NOT NULL,
                role_name TEXT NOT NULL,
                FOREIGN KEY(show_id) REFERENCES shows(show_id) ON DELETE CASCADE
            );

            CREATE TABLE show_scenario_line (
                scenario_line_id INTEGER PRIMARY KEY AUTOINCREMENT,
                show_id INTEGER NOT NULL,
                scenario_line_order INTEGER NOT NULL,
                scenario_line_name TEXT NOT NULL,
                scenario_line_note TEXT NOT NULL,
                scenario_line_time INTEGER NOT NULL,
                scenario_line_time_mode INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY(show_id) REFERENCES shows(show_id) ON DELETE CASCADE
            );

            CREATE TABLE show_scenario_line_comment (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                line_id INTEGER NOT NULL,
                role_id INTEGER, -- Optional: links comment to a specific role
                comment TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(line_id) REFERENCES show_scenario_line(scenario_line_id) ON DELETE CASCADE,
                FOREIGN KEY(role_id) REFERENCES show_roles(role_id) ON DELETE SET NULL
            );"
        )
    ])
}

pub fn init_db() -> Result<Connection, String> {
    let mut conn = Connection::open("shows.db").map_err(|e| e.to_string())?;

    // Apply all pending migrations automatically
    let migrations = get_migrations();
    migrations.to_latest(&mut conn).map_err(|e| e.to_string())?;

    Ok(conn)
}