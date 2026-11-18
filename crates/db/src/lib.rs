use std::fs;
use std::path::Path;

use rusqlite::Connection;

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: include_str!("../migrations/V1__initial.sql"),
}];

struct Migration {
    version: i64,
    sql: &'static str,
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("Ошибка SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Ошибка файловой системы: {0}")]
    Io(#[from] std::io::Error),
    #[error("Версия схемы базы данных {found} новее поддерживаемой {supported}")]
    UnsupportedSchemaVersion { found: i64, supported: i64 },
}

/// Opens the database, enables foreign keys, and applies pending migrations.
pub fn initialize(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    let mut connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    create_migration_history(&connection)?;
    apply_migrations(&mut connection)?;

    Ok(connection)
}

fn create_migration_history(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
}

fn apply_migrations(connection: &mut Connection) -> Result<(), DbError> {
    let current_version = connection.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    let latest_version = MIGRATIONS.last().map_or(0, |migration| migration.version);

    if current_version > latest_version {
        return Err(DbError::UnsupportedSchemaVersion {
            found: current_version,
            supported: latest_version,
        });
    }

    for migration in MIGRATIONS
        .iter()
        .filter(|migration| migration.version > current_version)
    {
        let transaction = connection.transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [migration.version],
        )?;
        transaction.commit()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::initialize;

    #[test]
    fn initialize_applies_migration_and_creates_settings_table() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let connection = initialize(&database_path).unwrap();

        let table_count = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'app_settings'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();

        assert_eq!(table_count, 1);
    }

    #[test]
    fn initialize_is_idempotent_and_enables_foreign_keys() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        drop(initialize(&database_path).unwrap());
        let connection = initialize(&database_path).unwrap();

        let migration_count = connection
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let foreign_keys_enabled = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0))
            .unwrap();

        assert_eq!(migration_count, 1);
        assert_eq!(foreign_keys_enabled, 1);
    }
}