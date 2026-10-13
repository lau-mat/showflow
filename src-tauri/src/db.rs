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
        ),
        M::up(
            "ALTER TABLE show_scenario_line
                ADD COLUMN scenario_line_duration INTEGER NOT NULL DEFAULT 0;
            ALTER TABLE show_scenario_line
                ADD COLUMN scenario_line_absolute_time INTEGER;

            WITH legacy AS (
                SELECT
                    scenario_line_id,
                    scenario_line_time_mode,
                    CAST(scenario_line_time AS TEXT) AS time_text,
                    instr(CAST(scenario_line_time AS TEXT), ':') AS first_colon
                FROM show_scenario_line
            )
            UPDATE show_scenario_line
            SET scenario_line_duration = (
                SELECT CASE
                    WHEN legacy.scenario_line_time_mode != 0 THEN 0
                    WHEN legacy.first_colon = 0 THEN CAST(legacy.time_text AS INTEGER)
                    WHEN instr(substr(legacy.time_text, legacy.first_colon + 1), ':') = 0 THEN
                        CAST(substr(legacy.time_text, 1, legacy.first_colon - 1) AS INTEGER) * 60 +
                        CAST(substr(legacy.time_text, legacy.first_colon + 1) AS INTEGER)
                    ELSE
                        CAST(substr(legacy.time_text, 1, legacy.first_colon - 1) AS INTEGER) * 3600 +
                        CAST(substr(legacy.time_text, legacy.first_colon + 1,
                            instr(substr(legacy.time_text, legacy.first_colon + 1), ':') - 1) AS INTEGER) * 60 +
                        CAST(substr(legacy.time_text, legacy.first_colon +
                            instr(substr(legacy.time_text, legacy.first_colon + 1), ':') + 1) AS INTEGER)
                END
                FROM legacy
                WHERE legacy.scenario_line_id = show_scenario_line.scenario_line_id
            );

            UPDATE show_scenario_line
            SET scenario_line_absolute_time = CASE
                WHEN CAST(scenario_line_time AS INTEGER) > 1000000000 THEN CAST(scenario_line_time AS INTEGER)
                ELSE (
                    SELECT CAST(strftime(
                        '%s',
                        date(shows.show_time, 'unixepoch', 'localtime') || ' ' ||
                            CAST(show_scenario_line.scenario_line_time AS TEXT),
                        'utc'
                    ) AS INTEGER)
                    FROM shows
                    WHERE shows.show_id = show_scenario_line.show_id
                )
            END
            WHERE scenario_line_time_mode = 1;

            ALTER TABLE show_scenario_line DROP COLUMN scenario_line_time_mode;
            ALTER TABLE show_scenario_line DROP COLUMN scenario_line_time;"
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_scenario_line_times_to_duration_and_absolute_time() {
        let mut conn = Connection::open_in_memory().unwrap();
        let migrations = get_migrations();
        migrations.to_version(&mut conn, 1).unwrap();

        conn.execute(
            "INSERT INTO shows (show_name, show_time) VALUES (?1, ?2)",
            rusqlite::params!["Migration test", 1_800_000_000_i64],
        ).unwrap();
        let show_id = conn.last_insert_rowid();

        conn.execute(
            "INSERT INTO show_scenario_line (show_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_time, scenario_line_time_mode)
             VALUES (?1, 1, 'Absolute epoch', '', 1900000000, 1)",
            [show_id],
        ).unwrap();
        conn.execute(
            "INSERT INTO show_scenario_line (show_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_time, scenario_line_time_mode)
             VALUES (?1, 2, 'Duration seconds', '', 125, 0)",
            [show_id],
        ).unwrap();
        conn.execute(
            "INSERT INTO show_scenario_line (show_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_time, scenario_line_time_mode)
             VALUES (?1, 3, 'Duration clock', '', '01:02:03', 0)",
            [show_id],
        ).unwrap();
        conn.execute(
            "INSERT INTO show_scenario_line (show_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_time, scenario_line_time_mode)
             VALUES (?1, 4, 'Absolute clock', '', '09:45:00', 1)",
            [show_id],
        ).unwrap();

        migrations.to_latest(&mut conn).unwrap();

        let migrated_lines = conn.prepare(
            "SELECT scenario_line_duration, scenario_line_absolute_time
             FROM show_scenario_line ORDER BY scenario_line_order",
        ).unwrap().query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?))
        }).unwrap().collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(migrated_lines[0], (0, Some(1_900_000_000)));
        assert_eq!(migrated_lines[1], (125, None));
        assert_eq!(migrated_lines[2], (3_723, None));
        assert!(migrated_lines[3].1.is_some());

        let columns = conn.prepare("PRAGMA table_info(show_scenario_line)").unwrap()
            .query_map([], |row| row.get::<_, String>(1)).unwrap()
            .collect::<Result<Vec<_>, _>>().unwrap();
        assert!(columns.contains(&"scenario_line_duration".to_string()));
        assert!(columns.contains(&"scenario_line_absolute_time".to_string()));
        assert!(!columns.contains(&"scenario_line_time".to_string()));
        assert!(!columns.contains(&"scenario_line_time_mode".to_string()));
    }
}