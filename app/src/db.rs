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
        ),
        M::up(
            "CREATE TABLE auth_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                refresh_token TEXT,
                refresh_expires_at TEXT,
                last_validated_at INTEGER,
                device_id TEXT NOT NULL
            );"
        ),
        M::up(
            // Sync-ready show tables: client-generated lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))) ids (so shows can be created offline
            // and merged), fractional `position` instead of an integer order, soft deletes, plus
            // the outbox of edits not yet pushed to the cloud.
            "CREATE TABLE show_id_map (old INTEGER PRIMARY KEY, new TEXT NOT NULL);
            CREATE TABLE role_id_map (old INTEGER PRIMARY KEY, new TEXT NOT NULL);
            CREATE TABLE line_id_map (old INTEGER PRIMARY KEY, new TEXT NOT NULL);
            CREATE TABLE comment_id_map (old INTEGER PRIMARY KEY, new TEXT NOT NULL);
            INSERT INTO show_id_map SELECT show_id, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))) FROM shows;
            INSERT INTO role_id_map SELECT role_id, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))) FROM show_roles;
            INSERT INTO line_id_map SELECT scenario_line_id, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))) FROM show_scenario_line;
            INSERT INTO comment_id_map SELECT id, lower(hex(randomblob(4)) || '-' || hex(randomblob(2)) || '-4' || substr(hex(randomblob(2)), 2) || '-' || substr('89ab', abs(random()) % 4 + 1, 1) || substr(hex(randomblob(2)), 2) || '-' || hex(randomblob(6))) FROM show_scenario_line_comment;

            CREATE TABLE shows_new (
                show_id TEXT PRIMARY KEY,
                show_name TEXT NOT NULL,
                show_time INTEGER NOT NULL,
                show_status INTEGER DEFAULT 0,
                owner_account TEXT,
                deleted_at INTEGER
            );
            INSERT INTO shows_new (show_id, show_name, show_time, show_status)
                SELECT m.new, s.show_name, s.show_time, s.show_status
                FROM shows s JOIN show_id_map m ON m.old = s.show_id;

            CREATE TABLE show_roles_new (
                role_id TEXT PRIMARY KEY,
                show_id TEXT NOT NULL,
                role_name TEXT NOT NULL,
                deleted_at INTEGER
            );
            INSERT INTO show_roles_new (role_id, show_id, role_name)
                SELECT rm.new, sm.new, r.role_name
                FROM show_roles r
                JOIN role_id_map rm ON rm.old = r.role_id
                JOIN show_id_map sm ON sm.old = r.show_id;

            CREATE TABLE show_scenario_line_new (
                scenario_line_id TEXT PRIMARY KEY,
                show_id TEXT NOT NULL,
                scenario_line_position TEXT NOT NULL,
                scenario_line_name TEXT NOT NULL,
                scenario_line_note TEXT NOT NULL,
                scenario_line_duration INTEGER NOT NULL DEFAULT 0,
                scenario_line_absolute_time INTEGER,
                deleted_at INTEGER
            );
            INSERT INTO show_scenario_line_new
                (scenario_line_id, show_id, scenario_line_position, scenario_line_name,
                 scenario_line_note, scenario_line_duration, scenario_line_absolute_time)
                SELECT lm.new, sm.new,
                       'k' || printf('%06d', row_number() OVER (
                           PARTITION BY l.show_id ORDER BY l.scenario_line_order, l.scenario_line_id)) || 'V',
                       l.scenario_line_name, l.scenario_line_note,
                       l.scenario_line_duration, l.scenario_line_absolute_time
                FROM show_scenario_line l
                JOIN line_id_map lm ON lm.old = l.scenario_line_id
                JOIN show_id_map sm ON sm.old = l.show_id;

            CREATE TABLE show_scenario_line_comment_new (
                id TEXT PRIMARY KEY,
                line_id TEXT NOT NULL,
                role_id TEXT,
                comment TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                deleted_at INTEGER
            );
            INSERT INTO show_scenario_line_comment_new (id, line_id, role_id, comment, created_at)
                SELECT cm.new, lm.new, rm.new, c.comment, c.created_at
                FROM show_scenario_line_comment c
                JOIN comment_id_map cm ON cm.old = c.id
                JOIN line_id_map lm ON lm.old = c.line_id
                LEFT JOIN role_id_map rm ON rm.old = c.role_id;

            DROP TABLE show_scenario_line_comment;
            DROP TABLE show_scenario_line;
            DROP TABLE show_roles;
            DROP TABLE shows;
            ALTER TABLE shows_new RENAME TO shows;
            ALTER TABLE show_roles_new RENAME TO show_roles;
            ALTER TABLE show_scenario_line_new RENAME TO show_scenario_line;
            ALTER TABLE show_scenario_line_comment_new RENAME TO show_scenario_line_comment;
            DROP TABLE show_id_map;
            DROP TABLE role_id_map;
            DROP TABLE line_id_map;
            DROP TABLE comment_id_map;

            CREATE INDEX show_roles_show ON show_roles (show_id);
            CREATE INDEX show_scenario_line_show ON show_scenario_line (show_id, scenario_line_position);
            CREATE INDEX show_scenario_line_comment_line ON show_scenario_line_comment (line_id);

            -- Per-show sync cursor: the last server change applied, and my role in the show.
            CREATE TABLE sync_shows (
                show_id TEXT PRIMARY KEY,
                server_rev INTEGER NOT NULL DEFAULT 0,
                role TEXT NOT NULL DEFAULT 'owner'
            );
            -- Edits not yet acknowledged by the cloud, in the order they were made.
            CREATE TABLE outbox (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                show_id TEXT NOT NULL,
                op_id TEXT NOT NULL UNIQUE,
                op TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
            );
            ALTER TABLE auth_state ADD COLUMN user_id TEXT;"
        ),
        M::up(
            "-- Who may call each show in a live session, remembered between sessions.
            CREATE TABLE session_settings (
                show_id TEXT PRIMARY KEY,
                policy TEXT NOT NULL,
                code TEXT
            );"
        ),
        M::up(
            "-- Account-based caller access (see caller_store.rs). This device's secret key and the
            -- passes issued to it; and, for hosting, the cloud's public key and each show's callers.
            CREATE TABLE caller_device (id INTEGER PRIMARY KEY CHECK (id = 1), seed TEXT NOT NULL);
            CREATE TABLE caller_passes (
                show_id TEXT NOT NULL,
                user_id TEXT NOT NULL,
                pass TEXT NOT NULL,
                expires_at INTEGER NOT NULL,
                PRIMARY KEY (show_id, user_id)
            );
            CREATE TABLE caller_trust (id INTEGER PRIMARY KEY CHECK (id = 1), public_key TEXT NOT NULL);
            CREATE TABLE caller_lists (show_id TEXT PRIMARY KEY, callers TEXT NOT NULL);"
        ),
        M::up(
            "-- Joining a session can be protected like calling it: its own policy and code (unset:
            -- open). Hosts also keep each show's members, who may join by account.
            ALTER TABLE session_settings ADD COLUMN view_policy TEXT;
            ALTER TABLE session_settings ADD COLUMN view_code TEXT;
            ALTER TABLE caller_lists ADD COLUMN members TEXT NOT NULL DEFAULT '[]';"
        )
    ])
}

/// Applies all pending migrations.
pub fn migrate(conn: &mut Connection) -> Result<(), String> {
    get_migrations().to_latest(conn).map_err(|e| e.to_string())
}

pub fn init_db() -> Result<Connection, String> {
    let mut conn = Connection::open("shows.db").map_err(|e| e.to_string())?;
    migrate(&mut conn)?;
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
             FROM show_scenario_line ORDER BY scenario_line_position",
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

    #[test]
    fn migrates_integer_ids_and_order_to_uuids_and_positions() {
        let mut conn = Connection::open_in_memory().unwrap();
        let migrations = get_migrations();
        migrations.to_version(&mut conn, 3).unwrap();

        conn.execute_batch(
            "INSERT INTO shows (show_id, show_name, show_time) VALUES (7, 'Gala', 1800000000), (9, 'Other', 1800000100);
             INSERT INTO show_roles (role_id, show_id, role_name) VALUES (3, 7, 'Audio');
             INSERT INTO show_scenario_line (scenario_line_id, show_id, scenario_line_order, scenario_line_name, scenario_line_note, scenario_line_duration)
                VALUES (11, 7, 2, 'second', 'n2', 20), (12, 7, 1, 'first', 'n1', 10), (13, 9, 1, 'solo', '', 5);
             INSERT INTO show_scenario_line_comment (id, line_id, role_id, comment)
                VALUES (21, 12, 3, 'with role'), (22, 12, NULL, 'general');",
        ).unwrap();

        migrations.to_latest(&mut conn).unwrap();

        let is_uuid = |s: &str| s.len() == 36 && uuid::Uuid::parse_str(s).is_ok();
        let shows: Vec<String> = conn.prepare("SELECT show_id FROM shows").unwrap()
            .query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(shows.len(), 2);
        assert!(shows.iter().all(|id| is_uuid(id)), "{shows:?}");

        // Lines keep their order within each show, through the new positions.
        let gala: String = conn.query_row("SELECT show_id FROM shows WHERE show_name = 'Gala'", [], |r| r.get(0)).unwrap();
        let lines: Vec<(String, String)> = conn.prepare(
            "SELECT scenario_line_name, scenario_line_position FROM show_scenario_line WHERE show_id = ?1 ORDER BY scenario_line_position, scenario_line_id",
        ).unwrap().query_map([&gala], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>(), ["first", "second"]);
        assert!(lines.iter().all(|l| protocol::position::is_valid_key(&l.1)));
        // New inserts still fit anywhere around the migrated positions.
        assert!(protocol::position::between(None, Some(&lines[0].1)).is_ok());
        assert!(protocol::position::between(Some(&lines[0].1), Some(&lines[1].1)).is_ok());
        assert!(protocol::position::between(Some(&lines[1].1), None).is_ok());

        // Relations survive: roles and comments point at the new ids.
        let (role_id, role_show): (String, String) = conn.query_row("SELECT role_id, show_id FROM show_roles", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!(role_show, gala);
        let comments: Vec<(String, Option<String>, String)> = conn.prepare(
            "SELECT l.scenario_line_name, c.role_id, c.comment FROM show_scenario_line_comment c
             JOIN show_scenario_line l ON l.scenario_line_id = c.line_id ORDER BY c.comment",
        ).unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap();
        assert_eq!(comments, vec![
            ("first".to_string(), None, "general".to_string()),
            ("first".to_string(), Some(role_id), "with role".to_string()),
        ]);

        // The tables the sync engine needs exist and the old id maps are gone.
        for table in ["sync_shows", "outbox"] {
            assert!(conn.prepare(&format!("SELECT * FROM {table}")).is_ok(), "{table}");
        }
        assert!(conn.prepare("SELECT * FROM show_id_map").is_err());
    }
}