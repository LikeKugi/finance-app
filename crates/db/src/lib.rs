use std::fs;
use std::path::Path;
use std::str::FromStr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::NaiveDate;
use finance_core::{
    canonical_amount, Bank, Category, Currency, FinancialTransaction, TransactionKind,
};
use rusqlite::{params, types::Type, Connection, OptionalExtension};
use rust_decimal::Decimal;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("../migrations/V1__initial.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("../migrations/V2__transactions_and_directories.sql"),
    },
];

struct Migration {
    version: i64,
    sql: &'static str,
}

/// SQLite connection type used by the desktop application.
pub type Database = Connection;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("Ошибка SQLite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Ошибка файловой системы: {0}")]
    Io(#[from] std::io::Error),
    #[error("Версия схемы базы данных {found} новее поддерживаемой {supported}")]
    UnsupportedSchemaVersion { found: i64, supported: i64 },
    #[error("Сумма должна быть положительным числом")]
    InvalidAmount,
    #[error("Категория не найдена или неактивна")]
    CategoryUnavailable,
    #[error("Запись используется и не может быть удалена")]
    EntityInUse,
    #[error("Перемещение категории создаёт цикл")]
    CategoryCycle,
    #[error("Название не может быть пустым")]
    EmptyName,
}

/// Opens the database, enables foreign keys, and applies pending migrations.
pub fn initialize(path: &Path) -> Result<Connection, DbError> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }

    let mut connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA foreign_keys = ON;")?;
    create_migration_history(&connection)?;
    apply_migrations(&mut connection, path)?;

    Ok(connection)
}

/// Returns transactions of one type, optionally filtered by inclusive dates, category, and text.
pub fn list_transactions(
    connection: &Connection,
    kind: TransactionKind,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    category_id: Option<i64>,
    search: &str,
) -> Result<Vec<FinancialTransaction>, DbError> {
    let (table, categories) = transaction_tables(kind);
    let sql = format!(
        "SELECT t.id, t.date, t.category_id, c.name, t.amount, t.currency, t.comment
         FROM {table} t JOIN {categories} c ON c.id = t.category_id
         WHERE (?1 IS NULL OR t.date >= ?1)
           AND (?2 IS NULL OR t.date <= ?2)
           AND (?3 IS NULL OR t.category_id = ?3)
           AND (?4 = '' OR instr(lower(t.comment), lower(?4)) > 0
                OR instr(lower(c.name), lower(?4)) > 0)
         ORDER BY t.date DESC, t.id DESC"
    );
    let from = from.map(|date| date.format("%Y-%m-%d").to_string());
    let to = to.map(|date| date.format("%Y-%m-%d").to_string());
    let mut statement = connection.prepare(&sql)?;
    let transactions = statement
        .query_map(params![from, to, category_id, search.trim()], |row| {
            let date_text = row.get::<_, String>(1)?;
            let date = NaiveDate::parse_from_str(&date_text, "%Y-%m-%d").map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(1, Type::Text, Box::new(error))
            })?;
            let amount_text = row.get::<_, String>(4)?;
            let amount = Decimal::from_str(&amount_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(4, Type::Text, Box::new(error))
            })?;
            let currency_text = row.get::<_, String>(5)?;
            let currency = Currency::from_str(&currency_text).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(5, Type::Text, Box::new(error))
            })?;
            Ok(FinancialTransaction {
                id: row.get(0)?,
                date,
                category_id: row.get(2)?,
                category_name: row.get(3)?,
                amount,
                currency,
                comment: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(transactions)
}

/// Creates or updates an income or expense, retaining the decimal as canonical text.
pub fn save_transaction(
    connection: &Connection,
    kind: TransactionKind,
    id: Option<i64>,
    date: NaiveDate,
    category_id: i64,
    amount: Decimal,
    currency: Currency,
    comment: &str,
) -> Result<i64, DbError> {
    if amount <= Decimal::ZERO {
        return Err(DbError::InvalidAmount);
    }
    let (table, categories) = transaction_tables(kind);
    let current_category = id
        .map(|id| {
            connection
                .query_row(
                    &format!("SELECT category_id FROM {table} WHERE id = ?1"),
                    [id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
        })
        .transpose()?
        .flatten();
    let category_is_active = connection
        .query_row(
            &format!("SELECT is_active FROM {categories} WHERE id = ?1"),
            [category_id],
            |row| row.get::<_, bool>(0),
        )
        .optional()?
        .unwrap_or(false);
    if !category_is_active && current_category != Some(category_id) {
        return Err(DbError::CategoryUnavailable);
    }

    let amount = canonical_amount(amount);
    let date = date.format("%Y-%m-%d").to_string();
    if let Some(id) = id {
        let changed = connection.execute(
            &format!(
                "UPDATE {table} SET date = ?1, category_id = ?2, amount = ?3,
                 currency = ?4, comment = ?5 WHERE id = ?6"
            ),
            params![date, category_id, amount, currency.code(), comment, id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        Ok(id)
    } else {
        connection.execute(
            &format!(
                "INSERT INTO {table} (date, category_id, amount, currency, comment)
                 VALUES (?1, ?2, ?3, ?4, ?5)"
            ),
            params![date, category_id, amount, currency.code(), comment],
        )?;
        Ok(connection.last_insert_rowid())
    }
}

/// Deletes a transaction by its database identifier.
pub fn delete_transaction(
    connection: &Connection,
    kind: TransactionKind,
    id: i64,
) -> Result<(), DbError> {
    let (table, _) = transaction_tables(kind);
    let deleted = connection.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
    if deleted == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    Ok(())
}

/// Lists categories in display order, optionally including inactive entries.
pub fn list_categories(
    connection: &Connection,
    kind: TransactionKind,
    include_inactive: bool,
) -> Result<Vec<Category>, DbError> {
    let table = category_table(kind);
    let mut statement = connection.prepare(&format!(
        "SELECT id, name, parent_id, is_active FROM {table}
         WHERE (?1 = 1 OR is_active = 1) ORDER BY name COLLATE NOCASE, id"
    ))?;
    let rows = statement.query_map([include_inactive], |row| {
        Ok(Category {
            id: row.get(0)?,
            name: row.get(1)?,
            parent_id: row.get(2)?,
            is_active: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Creates or updates a category after checking its parent and hierarchy.
pub fn save_category(
    connection: &Connection,
    kind: TransactionKind,
    id: Option<i64>,
    name: &str,
    parent_id: Option<i64>,
    is_active: bool,
) -> Result<i64, DbError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DbError::EmptyName);
    }
    let table = category_table(kind);
    let existing_parent_id = if let Some(id) = id {
        connection
            .query_row(
                &format!("SELECT parent_id FROM {table} WHERE id = ?1"),
                [id],
                |row| row.get::<_, Option<i64>>(0),
            )
            .optional()?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)?
    } else {
        None
    };
    if let Some(parent_id) = parent_id {
        let parent_active = connection
            .query_row(
                &format!("SELECT is_active FROM {table} WHERE id = ?1"),
                [parent_id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .unwrap_or(false);
        if (!parent_active && existing_parent_id != Some(parent_id)) || id == Some(parent_id) {
            return Err(DbError::CategoryUnavailable);
        }
        if let Some(id) = id {
            let creates_cycle = connection.query_row(
                &format!(
                    "WITH RECURSIVE descendants(id) AS (
                        SELECT id FROM {table} WHERE parent_id = ?1
                        UNION ALL
                        SELECT child.id FROM {table} child
                        JOIN descendants parent ON child.parent_id = parent.id
                    ) SELECT EXISTS(SELECT 1 FROM descendants WHERE id = ?2)"
                ),
                params![id, parent_id],
                |row| row.get::<_, bool>(0),
            )?;
            if creates_cycle {
                return Err(DbError::CategoryCycle);
            }
        }
    }

    if let Some(id) = id {
        let changed = connection.execute(
            &format!(
                "UPDATE {table} SET name = ?1, parent_id = ?2, is_active = ?3 WHERE id = ?4"
            ),
            params![name, parent_id, is_active, id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        Ok(id)
    } else {
        connection.execute(
            &format!(
                "INSERT INTO {table} (name, parent_id, is_active) VALUES (?1, ?2, ?3)"
            ),
            params![name, parent_id, is_active],
        )?;
        Ok(connection.last_insert_rowid())
    }
}

/// Deletes a category only when it has no child categories or transactions.
pub fn delete_category(
    connection: &Connection,
    kind: TransactionKind,
    id: i64,
) -> Result<(), DbError> {
    let (table, transactions) = transaction_tables(kind);
    let children = connection.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE parent_id = ?1)"),
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    let used = connection.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {transactions} WHERE category_id = ?1)"),
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    if children || used {
        return Err(DbError::EntityInUse);
    }
    let deleted = match connection.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id]) {
        Ok(deleted) => deleted,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(DbError::EntityInUse);
        }
        Err(error) => return Err(error.into()),
    };
    if deleted == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    Ok(())
}

/// Lists all banks alphabetically.
pub fn list_banks(connection: &Connection) -> Result<Vec<Bank>, DbError> {
    let mut statement = connection.prepare(
        "SELECT id, name, comment FROM banks ORDER BY name COLLATE NOCASE, id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(Bank {
            id: row.get(0)?,
            name: row.get(1)?,
            comment: row.get(2)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Creates or updates a bank.
pub fn save_bank(
    connection: &Connection,
    id: Option<i64>,
    name: &str,
    comment: &str,
) -> Result<i64, DbError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(DbError::EmptyName);
    }
    if let Some(id) = id {
        let changed = connection.execute(
            "UPDATE banks SET name = ?1, comment = ?2 WHERE id = ?3",
            params![name, comment, id],
        )?;
        if changed == 0 {
            return Err(rusqlite::Error::QueryReturnedNoRows.into());
        }
        Ok(id)
    } else {
        connection.execute(
            "INSERT INTO banks (name, comment) VALUES (?1, ?2)",
            params![name, comment],
        )?;
        Ok(connection.last_insert_rowid())
    }
}

/// Deletes a bank. Foreign-key restrictions protect banks used by later modules.
pub fn delete_bank(connection: &Connection, id: i64) -> Result<(), DbError> {
    let deleted = match connection.execute("DELETE FROM banks WHERE id = ?1", [id]) {
        Ok(deleted) => deleted,
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            return Err(DbError::EntityInUse);
        }
        Err(error) => return Err(error.into()),
    };
    if deleted == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows.into());
    }
    Ok(())
}

fn category_table(kind: TransactionKind) -> &'static str {
    match kind {
        TransactionKind::Income => "income_categories",
        TransactionKind::Expense => "expense_categories",
    }
}

fn transaction_tables(kind: TransactionKind) -> (&'static str, &'static str) {
    match kind {
        TransactionKind::Income => ("incomes", "income_categories"),
        TransactionKind::Expense => ("expenses", "expense_categories"),
    }
}

fn create_migration_history(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )
}

fn apply_migrations(connection: &mut Connection, database_path: &Path) -> Result<(), DbError> {
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

    if current_version > 0 && current_version < latest_version {
        backup_before_migration(connection, database_path, latest_version)?;
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

fn backup_before_migration(
    connection: &Connection,
    database_path: &Path,
    target_version: i64,
) -> Result<(), DbError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_nanos();
    let stem = database_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("database");
    let backup_path = database_path.with_file_name(format!(
        "{stem}.before-migration-v{target_version}-{timestamp}.db"
    ));
    connection.execute("VACUUM INTO ?1", [backup_path.to_string_lossy().as_ref()])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use finance_core::{Currency, TransactionKind};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    use super::{
        delete_category, initialize, list_transactions, save_category, save_transaction, DbError,
    };

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

        assert_eq!(migration_count, 2);
        assert_eq!(foreign_keys_enabled, 1);
    }

    #[test]
    fn save_and_list_transaction_preserves_exact_decimal_value() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let category_id = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Зарплата",
            None,
            true,
        )
        .unwrap();
        let amount = Decimal::from_str("12345678901234567890.1234").unwrap();
        save_transaction(
            &connection,
            TransactionKind::Income,
            None,
            NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(),
            category_id,
            amount,
            Currency::Rub,
            "Оклад",
        )
        .unwrap();

        let transactions = list_transactions(
            &connection,
            TransactionKind::Income,
            Some(NaiveDate::from_ymd_opt(2026, 11, 18).unwrap()),
            None,
            Some(category_id),
            "оклад",
        )
        .unwrap();

        assert_eq!(transactions[0].amount, amount);
    }

    #[test]
    fn delete_category_is_rejected_when_transactions_reference_it() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let category_id = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Продукты",
            None,
            true,
        )
        .unwrap();
        save_transaction(
            &connection,
            TransactionKind::Expense,
            None,
            NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(),
            category_id,
            Decimal::ONE,
            Currency::Rub,
            "",
        )
        .unwrap();

        let error =
            delete_category(&connection, TransactionKind::Expense, category_id).unwrap_err();

        assert!(matches!(error, DbError::EntityInUse));
    }

    #[test]
    fn save_transaction_rejects_non_positive_amount() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let category_id = save_category(
            &connection,
            TransactionKind::Income,
            None,
            "Подарок",
            None,
            true,
        )
        .unwrap();

        let result = save_transaction(
            &connection,
            TransactionKind::Income,
            None,
            NaiveDate::from_ymd_opt(2026, 11, 18).unwrap(),
            category_id,
            Decimal::ZERO,
            Currency::Rub,
            "",
        );

        assert!(matches!(result, Err(DbError::InvalidAmount)));
    }

    #[test]
    fn category_update_rejects_hierarchy_cycles() {
        let directory = tempfile::tempdir().unwrap();
        let connection = initialize(&directory.path().join("data.db")).unwrap();
        let parent_id = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Дом",
            None,
            true,
        )
        .unwrap();
        let child_id = save_category(
            &connection,
            TransactionKind::Expense,
            None,
            "Счета",
            Some(parent_id),
            true,
        )
        .unwrap();

        let result = save_category(
            &connection,
            TransactionKind::Expense,
            Some(parent_id),
            "Дом",
            Some(child_id),
            true,
        );

        assert!(matches!(result, Err(DbError::CategoryCycle)));
    }

    #[test]
    fn migration_from_previous_schema_creates_a_sqlite_backup() {
        let directory = tempfile::tempdir().unwrap();
        let database_path = directory.path().join("data.db");
        let previous_database = rusqlite::Connection::open(&database_path).unwrap();
        previous_database
            .execute_batch(include_str!("../migrations/V1__initial.sql"))
            .unwrap();
        previous_database
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
                INSERT INTO schema_migrations (version) VALUES (1);
                INSERT INTO app_settings (key, value) VALUES ('theme', 'dark');",
            )
            .unwrap();
        drop(previous_database);

        let connection = initialize(&database_path).unwrap();
        let preserved_setting = connection
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'theme'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let backup_count = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("before-migration-v2")
            })
            .count();

        assert_eq!(preserved_setting, "dark");
        assert_eq!(backup_count, 1);
    }
}